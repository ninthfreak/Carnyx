//! Coming back when the unit does, over the platform.
//!
//! ## Two halves that never meet
//!
//! `WakeReceiver` — in the GRADLE source set, because a manifest component is
//! constructed by the application's class loader and this one runs in a process
//! that has no Rust in it yet — decides whether to bring the face forward when
//! `com.nwd.ACTION_OS_WAKE_UP` arrives. It cannot ask this process anything: the
//! process is dead, which is the entire reason it exists.
//!
//! So the two halves pass notes through the platform's own SharedPreferences,
//! by NAME rather than through a shared class, the same way [`super::service`]
//! names its service by string:
//!
//! * DOWN — [`set_foreground`] keeps `was_foreground` current, so whatever it
//!   holds at the moment of the kill is the honest answer to what the driver was
//!   last looking at. The receiver reads it and leaves the face alone if they
//!   were somewhere else.
//! * UP — [`take_last_wake`] reads back the one line the receiver wrote about
//!   what it did, and clears it.
//!
//! ## The line back up is the only evidence this feature can produce
//!
//! Everything the receiver does happens with no face on screen, on a unit with
//! no adb. Whether the vendor broadcast arrives at all, whether the flag said
//! what was expected, and whether Android 10's background-activity-start
//! restriction refused the launch are three different outcomes that look
//! identical from the driver's seat — the app is simply not there. The note
//! makes them three different lines in the diagnostics log.
//!
//! ## What is confirmed
//!
//! NOTHING HERE HAS RUN ON THE UNIT, and the JNI constructs are copied from
//! `service.rs` rather than composed. The receiver is also absent from a
//! cargo-apk build, which packages no Java and has no `<receiver>` field in its
//! manifest schema — there, `take_last_wake` answers `""` forever and nothing
//! else changes.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::strings::JNIStr;
use jni::{jni_sig, jni_str, Env, JavaVM};

const CLASS: &JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxWake");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Load the class, hand it the context, and write down where the driver is now.
///
/// THE FLAG IS SEEDED HERE, not left to the first lifecycle callback. `Resume`
/// arrives after the first frames rather than before them, and a unit that
/// slept between this call and that one would have the receiver reading a flag
/// from the PREVIOUS run. Start-up is unambiguously in front, and
/// [`super::is_foreground`] says so.
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
    })?;
    set_foreground(super::is_foreground());
    Ok(())
}

/// Record whether the face is in front, for the receiver to read after the kill.
///
/// Called from [`super::set_foreground`] on every lifecycle edge, so there is no
/// second place that has to remember to. Silently does nothing before [`init`]
/// has run and on a build with no class, which are the same case as far as a
/// caller is concerned.
pub fn set_foreground(front: bool) {
    let Some(class) = CLASS_REF.get() else {
        return;
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return;
    };
    let _ = jvm.attach_current_thread(|env: &mut Env| -> Result<(), jni::errors::Error> {
        env.call_static_method(
            class,
            jni_str!("setForeground"),
            jni_sig!("(Z)V"),
            &[JValue::Bool(front)],
        )?;
        Ok(())
    });
}

/// What the receiver did last time, and forget it.
///
/// Empty when it has said nothing since the app last asked — an ordinary
/// launcher tap, or a build with no receiver in it. See [`take_last_wake`]'s
/// counterpart `CarnyxWake.takeLastWake` for why this takes rather than reads.
pub fn take_last_wake() -> String {
    let Some(class) = CLASS_REF.get() else {
        return String::new();
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return String::new();
    };
    jvm.attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
        let note = env
            .call_static_method(
                class,
                jni_str!("takeLastWake"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        // See `probe::report`. A null comes back as `Error::NullPtr` from
        // `try_to_string` rather than as an empty string, which the caller's
        // `unwrap_or_default` turns into the same thing — and `takeLastWake`
        // returns `""` for "nothing to say" rather than null anyway.
        let note = JString::cast_local(env, note)?;
        note.try_to_string(env)
    })
    .unwrap_or_default()
}
