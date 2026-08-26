//! Keeping the process alive, over the platform.
//!
//! ## The fault this exists for
//!
//! > "When switching to a different app and then switching back, it looked like
//! > the app was starting fresh, having to draw elements and wait for things like
//! > radio text to be decoded."
//!
//! [`crate::session`] answers that by SURVIVING the restart — the dial and the
//! decoded RDS are written out and read back, so a cold start looks warm. This
//! module answers it the other way, by making the restart not happen: a process
//! with a foreground service is not a candidate for the launcher's cleaner or the
//! low-memory killer, which is exactly why CarFM never had the fault at all
//! (`VibeStreamService` runs foreground even on the built-in tuner, carrying no
//! audio).
//!
//! THE TWO ARE NOT REDUNDANT AND NEITHER REPLACES THE OTHER. A foreground service
//! makes the process expensive to kill, not unkillable — the unit still sleeps on
//! ACC-off, and a driver can still stop the app — so the restore path stays the
//! backstop for every restart the service does not prevent.
//!
//! ## Why this is three files
//!
//! `CarnyxService` is in the GRADLE source set — `android/app/src/main/java/` —
//! and only the starter (`CarnyxProcess`) is in `java/`, the runtime dex, beside
//! `CarnyxNet` and `CarnyxLocation`. The starter names the service by string
//! through a `ComponentName`, so neither tree has to resolve into the other at
//! compile time.
//!
//! THE RULE THAT FORCES THIS, stated exactly. A class that exists ONLY in the
//! embedded dex cannot be a manifest component: Android constructs one through
//! the application's own class loader, which knows nothing about the
//! `InMemoryDexClassLoader` this module builds after start-up, and the failure
//! is a `ClassNotFoundException` when the platform tries to construct it.
//!
//! An earlier version of this comment went further and said the service could
//! not live in `java/` AT ALL. That was wrong, and worth correcting rather than
//! quietly fixing: `android/app/build.gradle.kts` puts `../../java` in the Gradle
//! JAVA source set as well as the aidl one, so AGP compiles that whole tree into
//! the APK's own dex too, and a service class there would in fact be
//! constructible under Gradle. What is true is narrower — it would be compiled
//! TWICE, dead weight in the embedded dex, and invisible under cargo-apk, which
//! packages no Java at all. Keeping Gradle-only code in the Gradle source set is
//! a choice for clarity; the class-loader rule is the constraint behind it.
//!
//! Either way cargo-apk cannot declare the `<service>` — its manifest schema has
//! no field for one — so under the DEFAULT build this call finds no such
//! component and logs that it did. Build with `tools/build-apk-gradle.sh` to get
//! the service.
//!
//! ## What is confirmed
//!
//! IT BUILDS, INSTALLS AND RUNS ON THE UNIT. The Gradle APK with the service
//! declared was built and started on the head unit — so this module's `init` and
//! `start` are on a path that executes, AGP compiles `CarnyxService`, and the
//! manifest merges.
//!
//! WHETHER THE SERVICE ENTERS THE FOREGROUND IS NOT YET CONFIRMED, and an app
//! that runs does not say. `start` returns whether the platform ACCEPTED the
//! start, not whether `startForeground` later succeeded on the main thread —
//! `CarnyxService` logs its own failure there. `android_main` writes the return
//! value into the settings log as `service: started` or `service: none`, because
//! the unit has no adb and logcat reaches nobody; the `session:` line's
//! `app #N in this process` is what says whether the process then survived.
//!
//! Written on a machine with no Android SDK and no NDK, so the Rust here was
//! never compiled for the target before it shipped. What was checked off-device:
//! both Java files compile clean against a real API-34 framework jar, and `javap`
//! confirmed that the descriptors below — `(Landroid/content/Context;)V` and
//! `(Ljava/lang/String;)Z` — are the ones the compiled class carries. Every JNI
//! construct is copied verbatim from `nwd.rs` or `net.rs` rather than composed.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::strings::JNIStr;
use jni::{jni_sig, jni_str, Env, JavaVM};

const CLASS: &JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxProcess");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Load the starter class and hand it the context.
///
/// THE CONTEXT STAYS ON THE JAVA SIDE, exactly as `NwdBridge.attach` keeps it:
/// `CarnyxProcess.attach` takes `getApplicationContext()` and holds it in a
/// static, so Rust never has to keep a Java object reference alive between
/// calls. The activity reference `AndroidApp` hands out is a LOCAL one, valid
/// only for the frame it arrived on, and the classic JNI use-after-free is
/// holding one past that.
///
/// # Safety
///
/// As [`super::nwd::init`]: `vm` and `activity` must be what `AndroidApp` handed
/// out, with the activity still alive.
pub unsafe fn init(vm: *mut c_void, activity: *mut c_void) -> Result<(), super::TunerError> {
    use super::TunerError;
    if activity.is_null() {
        return Err(TunerError::Unavailable("null activity".into()));
    }
    let jvm = JavaVM::singleton()
        .or_else(|_| -> Result<JavaVM, jni::errors::Error> {
            Ok(unsafe { JavaVM::from_raw(vm.cast()) })
        })
        .map_err(|e| TunerError::Java(e.to_string()))?;

    jvm.attach_current_thread(|env: &mut Env| -> Result<(), TunerError> {
        super::dex::check(env).map_err(TunerError::Unavailable)?;
        let context = unsafe { JObject::from_raw(env, activity.cast()) };
        let class = super::dex::load_class(env, &context, CLASS)
            .map_err(|e| TunerError::Java(format!("loading {CLASS:?}: {e}")))?;
        env.call_static_method(
            &class,
            jni_str!("attach"),
            jni_sig!("(Landroid/content/Context;)V"),
            &[JValue::Object(&context)],
        )
        .map_err(|e| TunerError::Java(format!("attach: {e}")))?;
        let class_ref = env
            .new_global_ref(&class)
            .map_err(|e| TunerError::Java(e.to_string()))?;
        let _ = CLASS_REF.set(class_ref);
        Ok(())
    })
}

/// Ask the platform to run the service in the foreground.
///
/// CALL THIS WHILE THE ACTIVITY IS ON SCREEN. From Android 12 a background
/// `startForegroundService` throws `ForegroundServiceStartNotAllowedException`,
/// so start-up — where the app is indisputably in front — is the moment for it.
///
/// `text` is the notification's second line; the frequency is the honest thing
/// to put there, because it is true the instant the app starts and needs no RDS.
/// The notification itself may never be seen: on API 33+ posting needs
/// POST_NOTIFICATIONS, which this app deliberately never requests, and the
/// platform then suppresses the line while STILL running the service in the
/// foreground — which is the half that matters.
///
/// Returns false when the platform refused, which is the ordinary answer on a
/// cargo-apk build where the service class is not in the APK at all. There is
/// nothing for a caller to do about it; the reason is in logcat.
pub fn start(text: &str) -> bool {
    let Some(class) = CLASS_REF.get() else {
        return false;
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return false;
    };
    jvm.attach_current_thread(|env: &mut Env| -> Result<bool, jni::errors::Error> {
        let t = env.new_string(text)?;
        env.call_static_method(
            class,
            jni_str!("start"),
            jni_sig!("(Ljava/lang/String;)Z"),
            &[(&t).into()],
        )?
        .z()
    })
    .unwrap_or(false)
}

/// The clock's two facts (§4.8): the local time and the system's 12/24 setting.
///
/// ONE CALL FOR THE TIME so both fields come from one reading — see
/// `CarnyxProcess.clockHourMinute` for why two calls would show 09:00 for a
/// minute at ten o'clock.
///
/// `None` when the class never loaded, which is every host build. The caller
/// draws nothing rather than a zero, because `00:00` is a real time.
pub fn clock_now() -> Option<(u32, u32, bool)> {
    let class = CLASS_REF.get()?;
    let jvm = JavaVM::singleton().ok()?;
    jvm.attach_current_thread(|env: &mut Env| -> Result<Option<(u32, u32, bool)>, jni::errors::Error> {
        let hm = env
            .call_static_method(class, jni_str!("clockHourMinute"), jni_sig!("()I"), &[])?
            .i()?;
        if hm < 0 {
            return Ok(None);
        }
        let is24 = env
            .call_static_method(class, jni_str!("clockIs24Hour"), jni_sig!("()Z"), &[])?
            .z()?;
        Ok(Some(((hm / 100) as u32, (hm % 100) as u32, is24)))
    })
    .ok()
    .flatten()
}

/// The device's ISO 3166-1 alpha-2 country, or `""` (§4.9's units).
///
/// READ ONCE AT START-UP by the caller, unlike the clock beside it. A driver
/// does not cross a border mid-drive often enough to poll for it, and the units
/// changing under a countdown would be worse than being a launch behind.
pub fn country_code() -> String {
    let Some(class) = CLASS_REF.get() else {
        return String::new();
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return String::new();
    };
    jvm.attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
        let out = env
            .call_static_method(class, jni_str!("countryCode"), jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        JString::cast_local(env, out)?.try_to_string(env)
    })
    .unwrap_or_default()
}
