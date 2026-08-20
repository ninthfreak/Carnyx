//! The JNI binding to `CarnyxLocation`.
//!
//! Same shape as [`super::nwd`], for the same reasons: the class is loaded from
//! a dex at run time, so JNI's automatic `Java_…` symbol lookup would never find
//! an exported symbol — it searches only the libraries registered against the
//! DEFINING class's loader, and a run-time dex has none. `RegisterNatives` is
//! the only route in.
//!
//! ## What decides what
//!
//! Java reports; Rust decides. `CarnyxLocation` passes on whatever the platform
//! handed it, and every judgement about those numbers — whether a coordinate
//! pair is possible at all, whether a (0, 0) from an empty provider counts as a
//! fix — is made by [`super::ingest_position`], where it is testable. This file
//! is only the wire.
//!
//! ## Permission is a state, not an error
//!
//! [`start`] returning `false` is the ordinary answer on a unit where nobody
//! tapped Allow, and the caller keeps whatever position it already had. Nothing
//! here throws, blocks, or asks for a dialog: there is no Activity on the far
//! side of `android_main` to show one, and a modal permission prompt over a
//! moving car's radio would be the wrong thing even if there were.
//!
//! NONE OF THIS HAS RUN. It compiles for `armv7-linux-androideabi`, and that is
//! the entire claim.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::errors::Error;
use jni::objects::{JClass, JObject, JString};
use jni::refs::Global;
use jni::sys::{jboolean, jdouble, jfloat};
use jni::{jni_sig, jni_str, Env, EnvUnowned, JavaVM, NativeMethod};

use super::TunerError;

const CLASS: &jni::strings::JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxLocation");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Java → Rust. One method, because one fact crosses: where the car is.
///
/// `extern "system"` and registered by hand below; see the module header for why
/// an exported symbol would not be found.
extern "system" fn native_position<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    lat: jdouble,
    lon: jdouble,
    fix: jboolean,
    speed_mps: jfloat,
    has_speed: jboolean,
) {
    // Through the same guard the tuner's natives use. A panic unwinding across
    // the JNI boundary is undefined behaviour, so it is turned into a Java
    // exception instead.
    guard(&mut env, |_env| {
        super::ingest_position(lat, lon, fix, speed_mps, has_speed);
        Ok(())
    });
}

/// Identical to `nwd`'s: run the body, and turn any panic or error into a thrown
/// RuntimeException rather than letting it unwind into the JVM.
fn guard<'a>(unowned: &mut EnvUnowned<'a>, body: impl FnOnce(&mut Env) -> Result<(), Error>) {
    unowned
        .with_env(body)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// Java → Rust, the second fact that crosses: WHICH providers actually
/// registered.
///
/// The unit has no adb, so `Log.i` reaches nobody. Time-to-first-fix is the
/// thing this file gets wrong in ways that are invisible from the outside — a
/// skipped provider looks exactly like a slow one — so the answer has to land
/// somewhere the driver can read it, which is the diagnostics panel.
extern "system" fn native_note<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    line: JString<'a>,
) {
    // Through `guard`, like every other native here: a panic unwinding across
    // the JNI boundary is undefined behaviour.
    //
    // `try_to_string` and not `get_string`: this crate's `JString` is the
    // borrowed reference and the conversion hangs off it, which is the shape
    // `nwd::text` uses and the one that compiles. The first cut of this reached
    // for the older `Env::get_string(&JString)` API and would not have built for
    // the target — worth the note, because this file is `cfg(target_os =
    // "android")` and the HOST BUILD NEVER COMPILES IT.
    guard(&mut env, |env| {
        let text = if line.is_null() {
            String::new()
        } else {
            line.try_to_string(env).unwrap_or_default()
        };
        super::ingest_note(format!("location: {text}"));
        Ok(())
    });
}

fn natives() -> Vec<NativeMethod<'static>> {
    // SAFETY: the signature matches both the Java declaration
    // (`private static native void nativePosition(double, double, boolean,
    // float, boolean)`) and this function's parameter list. The three are
    // written together and must be changed together — a mismatch is not a
    // compile error on either side, it is a crash at the first fix.
    unsafe {
        vec![
            NativeMethod::from_raw_parts(
                jni_str!("nativePosition"),
                jni_str!("(DDZFZ)V"),
                native_position as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeNote"),
                jni_str!("(Ljava/lang/String;)V"),
                native_note as *mut c_void,
            ),
        ]
    }
}

/// Load the class, bind the callback, and hand it the app context.
///
/// # Safety
///
/// `vm` must be the process's `JavaVM` and `activity` the `NativeActivity`
/// object reference, exactly as `AndroidApp::vm_as_ptr` and
/// `activity_as_ptr` return them, with the activity still alive.
pub unsafe fn init(vm: *mut c_void, activity: *mut c_void) -> Result<(), TunerError> {
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

        let global = env
            .new_global_ref(&class)
            .map_err(|e| TunerError::Java(e.to_string()))?;
        let _ = CLASS_REF.set(global);

        env.call_static_method(
            &class,
            jni_str!("attach"),
            jni_sig!("(Landroid/content/Context;)V"),
            &[(&context).into()],
        )
        .map_err(|e| TunerError::Java(format!("attach: {e}")))?;
        Ok(())
    })
}

fn with_class<R>(f: impl FnOnce(&mut Env, &JClass) -> Result<R, jni::errors::Error>) -> Option<R> {
    let class = CLASS_REF.get()?;
    let jvm = JavaVM::singleton().ok()?;
    jvm.attach_current_thread(|env: &mut Env| f(env, class)).ok()
}

/// Begin listening for fixes.
///
/// `false` means there is nothing to listen to — no permission, no provider, or
/// `init` never ran. All three are ordinary states on a head unit, and the
/// caller should keep the position it already had rather than treating this as a
/// failure.
pub fn start() -> bool {
    with_class(|env, class| {
        env.call_static_method(class, jni_str!("start"), jni_sig!("()Z"), &[])?
            .z()
    })
    .unwrap_or(false)
}

/// Whether the runtime grant actually exists. Declaring the permission in the
/// manifest is not the same thing, and on a head unit nobody is there to tap
/// Allow.
pub fn has_permission() -> bool {
    with_class(|env, class| {
        env.call_static_method(class, jni_str!("hasPermission"), jni_sig!("()Z"), &[])?
            .z()
    })
    .unwrap_or(false)
}

/// Stop listening. Safe to call when nothing was started.
pub fn stop() {
    let _ = with_class(|env, class| {
        env.call_static_method(class, jni_str!("stop"), jni_sig!("()V"), &[])?;
        Ok(())
    });
}
