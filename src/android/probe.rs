//! What could keep this app alive while the head unit sleeps, over the platform.
//!
//! See `CarnyxKeepAlive.java` for the question and for what it will and will not
//! read. This side is the seam and nothing more: load the class once, call it on
//! demand, hand back the lines.
//!
//! ON DEMAND, NEVER AT START-UP. The report walks three settings tables and the
//! package list, which is binder work measured in tens of milliseconds and is of
//! no use to a driver. It runs when a DIAGNOSTICS row is tapped, which is a
//! moment nobody is driving through.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::strings::JNIStr;
use jni::{jni_sig, jni_str, Env, JavaVM};

const CLASS: &JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxKeepAlive");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Why the class did not load, when it did not.
///
/// WITHOUT IT EVERY FAILURE PRINTS THE SAME FALSE SENTENCE. `report` returned an
/// empty vector for "no class", and the caller turns that into "unavailable in
/// this build" — which is TRUE on the host and a LIE on the unit, where
/// `build.rs` puts this class in the embedded dex. A driver reading that line
/// after tapping the row would conclude the feature was never built, when what
/// actually happened was a dex that would not load or a JNI call that threw.
static INIT_ERR: OnceLock<String> = OnceLock::new();

/// What this probe calls itself in the log, matching `app::probe_name`.
const NAME: &str = "keep-alive probe";

/// Load the class and hand it the context.
///
/// # Safety
///
/// As [`super::service::init`]: `vm` and `activity` must be what `AndroidApp`
/// handed out, with the activity still alive.
pub unsafe fn init(vm: *mut c_void, activity: *mut c_void) -> Result<(), super::TunerError> {
    use super::TunerError;
    let outcome = unsafe { load(vm, activity) };
    if let Err(e) = &outcome {
        // FIRST REASON WINS, and there is only ever one: `init` runs once from
        // `android_main`. Recorded so `report` can say which failure this was.
        let _ = INIT_ERR.set(e.to_string());
    }
    outcome
}

unsafe fn load(vm: *mut c_void, activity: *mut c_void) -> Result<(), super::TunerError> {
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

/// The report, one line per entry.
///
/// Empty when the class never loaded, which on a host build is always — the
/// caller says so rather than printing nothing.
pub fn report() -> Vec<String> {
    let Some(class) = CLASS_REF.get() else {
        // NOT "unavailable in this build" — see `INIT_ERR`. On a host build
        // there genuinely is no class and no error, and the caller's own line is
        // the honest one; on the unit there is a reason and this is it.
        return match INIT_ERR.get() {
            Some(e) => vec![format!("{NAME}: class did not load — {e}")],
            None => Vec::new(),
        };
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return vec![format!("{NAME}: no JVM")];
    };
    let text = jvm
        .attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
            let s = env
                .call_static_method(class, jni_str!("report"), jni_sig!("()Ljava/lang/String;"), &[])?
                .l()?;
            // `cast_local` + `try_to_string`, NOT `.into()` + `Env::get_string`.
            // This crate's `JString` is a borrowed reference with no
            // `From<JObject>`, and the conversion hangs off the string rather
            // than off the env — where `Env::get_string` still exists it is
            // deprecated. Written the wrong way first because this file is
            // `cfg(target_os = "android")` and the container it was written in
            // has no NDK, so nothing here was ever compiled; `location.rs:88`
            // and `nwd.rs:386` both already carry a note about this exact trap.
            let s = JString::cast_local(env, s)?;
            s.try_to_string(env)
        })
        .unwrap_or_else(|e| format!("{NAME}: the report call failed — {e}"));
    text.lines().map(str::to_string).collect()
}
