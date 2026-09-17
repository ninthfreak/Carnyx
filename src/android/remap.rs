//! Install the kernel source remap through the vendor's own file copier.
//!
//! See `CarnyxRemap.java` for the mechanism: stage a payload in the app's
//! external files dir and ask `com.nwd.factory.setting`'s exported, permissionless
//! `CopyFileService` to copy it into `/config/app`, where the kernel reads
//! `replace_source_list.xml` and launches Carnyx in the stock radio app's place.
//! This side is the seam and nothing more: load the class once, call it on the
//! two explicit taps, hand back the line.
//!
//! ON DEMAND, NEVER AT START-UP. Both calls change or read a system partition on
//! an explicit tap in a settings panel; neither belongs on any automatic path.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::errors::Error;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::strings::JNIStr;
use jni::{jni_sig, jni_str, Env, EnvUnowned, JavaVM, NativeMethod};

const CLASS: &JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxRemap");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Why the class did not load, when it did not. See `stock::INIT_ERR` for the
/// same reasoning: without it every failure prints the host's "only on the unit"
/// line, which is a lie on the unit where `build.rs` dexes this class.
static INIT_ERR: OnceLock<String> = OnceLock::new();

/// Java → Rust. One method, because one fact crosses: what the copier did.
///
/// `extern "system"` and registered by hand below — an exported symbol would not
/// be found, for `CarnyxLocation`'s reason. The watcher thread that calls this
/// finishes long after the settings tap returned, so there is no return value for
/// its verdict to travel back on and this is the channel.
extern "system" fn native_note<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    line: JString<'a>,
) {
    // Through `guard`, like every other native in this tree: a panic unwinding
    // across the JNI boundary is undefined behaviour.
    guard(&mut env, |env| {
        let text = if line.is_null() {
            String::new()
        } else {
            line.try_to_string(env).unwrap_or_default()
        };
        super::ingest_note(text);
        Ok(())
    });
}

/// Identical to `location`'s: run the body, and turn any panic or error into a
/// thrown RuntimeException rather than letting it unwind into the JVM.
fn guard<'a>(unowned: &mut EnvUnowned<'a>, body: impl FnOnce(&mut Env) -> Result<(), Error>) {
    unowned
        .with_env(body)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

fn natives() -> Vec<NativeMethod<'static>> {
    // SAFETY: the signature matches both the Java declaration
    // (`private static native void nativeRemapNote(String)`) and this function's
    // parameter list. The three are written together and must be changed
    // together — a mismatch is not a compile error on either side, it is a crash
    // the first time the watcher reports.
    unsafe {
        vec![NativeMethod::from_raw_parts(
            jni_str!("nativeRemapNote"),
            jni_str!("(Ljava/lang/String;)V"),
            native_note as *mut c_void,
        )]
    }
}

/// Load the class, bind the callback, and hand it the context.
///
/// # Safety
///
/// As [`super::stock::init`]: `vm` and `activity` must be what `AndroidApp`
/// handed out, with the activity still alive.
pub unsafe fn init(vm: *mut c_void, activity: *mut c_void) -> Result<(), super::TunerError> {
    let outcome = unsafe { load(vm, activity) };
    if let Err(e) = &outcome {
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
        unsafe { env.register_native_methods(&class, &natives()) }
            .map_err(|e| TunerError::Java(format!("RegisterNatives: {e}")))?;
        env.call_static_method(
            &class,
            jni_str!("attach"),
            jni_sig!("(Landroid/content/Context;)Ljava/lang/String;"),
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

/// Stage the payload and ask the factory copier to write the remap. One line.
pub fn install() -> String {
    call("install")
}

/// Read `/config/app/replace_source_list.xml` back and report. One line.
pub fn verify() -> String {
    call("verify")
}

/// Call a no-argument `String`-returning static, or say why it could not.
fn call(method: &'static str) -> String {
    let Some(class) = CLASS_REF.get() else {
        return match INIT_ERR.get() {
            Some(e) => format!("radio takeover: class did not load — {e}"),
            None => "radio takeover: only on the unit".into(),
        };
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return "radio takeover: no JVM".into();
    };
    // The method name reaches JNI as an interface string. It is one of two
    // literals from `install`/`verify`, never caller data, but `jni_str!` needs a
    // literal, so the two are spelled out rather than formatted in.
    let name: &JNIStr = match method {
        "install" => jni_str!("install"),
        _ => jni_str!("verify"),
    };
    jvm.attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
        let s = env
            .call_static_method(class, name, jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        // Same shape as `stock::report`: `cast_local` + `try_to_string`.
        let s = JString::cast_local(env, s)?;
        s.try_to_string(env)
    })
    .unwrap_or_else(|e| format!("radio takeover: the {method} call failed — {e}"))
}
