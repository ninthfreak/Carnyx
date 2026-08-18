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
//! The service class CANNOT live in the dex this module loads. Android constructs
//! a manifest-declared component through the application's own class loader,
//! which knows nothing about the `InMemoryDexClassLoader` Rust builds after
//! start-up — so `CarnyxService` is in the GRADLE source set, compiled by AGP
//! into the APK's own dex, and only the starter (`CarnyxProcess`) is in the
//! runtime dex beside `CarnyxNet` and `CarnyxLocation`. The starter names the
//! service by string through a `ComponentName`, so nothing has to resolve across
//! the two trees at compile time.
//!
//! That split is the whole of #67's packager problem: cargo-apk packages no Java
//! and its manifest schema has no `service` field, so under the DEFAULT build
//! this call finds no such component and logs that it did. Build with
//! `tools/build-apk-gradle.sh` to get the service.
//!
//! NONE OF THIS HAS RUN, and the caveat is stronger than `net.rs`'s. That file
//! says "it compiles for `armv7-linux-androideabi`"; this one cannot say even
//! that, because it was written on a machine with no Android SDK and no NDK.
//!
//! WHAT WAS ACTUALLY CHECKED: both Java files compile clean against a real
//! API-34 framework jar, and `javap` was used to confirm that the descriptors
//! below — `(Landroid/content/Context;)V` and `(Ljava/lang/String;)Z` — are the
//! ones the compiled class carries. Every JNI construct here is copied verbatim
//! from `nwd.rs` or `net.rs` rather than composed, which is the most this can be
//! held to without a device. Treat the first run on the unit as the first test.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::objects::{JClass, JObject, JValue};
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
