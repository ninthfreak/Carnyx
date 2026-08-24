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

/// Load the class and hand it the context.
///
/// # Safety
///
/// As [`super::service::init`]: `vm` and `activity` must be what `AndroidApp`
/// handed out, with the activity still alive.
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

/// The report, one line per entry.
///
/// Empty when the class never loaded, which on a host build is always — the
/// caller says so rather than printing nothing.
pub fn report() -> Vec<String> {
    let Some(class) = CLASS_REF.get() else {
        return Vec::new();
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return Vec::new();
    };
    let text = jvm
        .attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
            let s = env
                .call_static_method(class, jni_str!("report"), jni_sig!("()Ljava/lang/String;"), &[])?
                .l()?;
            let s: JString = s.into();
            Ok(env.get_string(&s)?.into())
        })
        .unwrap_or_default();
    text.lines().map(str::to_string).collect()
}
