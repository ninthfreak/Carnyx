//! The station pop-up, over the platform.
//!
//! ## What it is for
//!
//! The steering wheel changes station whether or not the face is on screen — the
//! MCU broadcasts `com.nwd.action.ACTION_KEY_VALUE`, `NwdBridge` hears it, and
//! `State::reassert` makes this app's choice the one that plays. A driver in
//! another app therefore gets a station change with NOTHING TO SEE. This posts a
//! heads-up notification saying what is now tuned, and nothing else.
//!
//! ## Why it does not go through the service
//!
//! [`super::service`] pins the process and lives in the Gradle source set, so it
//! does not exist under cargo-apk at all. A notification needs none of that —
//! any component with a `Context` can post one — so `CarnyxAlert` is in `java/`,
//! the runtime dex, and both packagers get it.
//!
//! It also sidesteps a rule that would have bitten immediately. Reaching the
//! service means `startService` FROM THE BACKGROUND, which API 26 forbids and
//! which API 31 forbids again for `startForegroundService` — and the background
//! is the only time this feature fires at all. Posting directly has no such rule.
//!
//! ## What is confirmed, and what is not
//!
//! NOTHING HERE HAS RUN ON THE UNIT. It is written the way `service.rs` was: the
//! JNI constructs are copied from that module rather than composed, and the
//! descriptors below match what the Java declares. The unit is Android 10, where
//! posting needs no permission — `POST_NOTIFICATIONS` arrived in API 33 — so the
//! failure mode this cannot check for here is a NEWER head unit, and
//! `CarnyxAlert.post` answers that by reporting it rather than returning as
//! though it had worked.
//!
//! THE PREREQUISITE IS NOT CONFIRMED EITHER. A wheel press reaches Rust through
//! `ingest_panel_key`, which queues a `TunerEvent` drained by
//! `slint::invoke_from_event_loop` — the SLINT event loop. Whether that loop
//! pumps while the activity is stopped decides whether a backgrounded wheel
//! press retunes at all, and so whether there is ever a station change for this
//! module to announce. `android-activity`'s main thread keeps polling, so it
//! should; that is a reading of the crate, not a measurement on the unit. The
//! diagnostics log records each tune with the foreground flag beside it, which is
//! what one drive will settle.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::objects::{JClass, JObject, JString, JValue};
use jni::refs::Global;
use jni::strings::JNIStr;
use jni::{jni_sig, jni_str, Env, JavaVM};

const CLASS: &JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxAlert");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Load the class and hand it the context.
///
/// THE CONTEXT STAYS ON THE JAVA SIDE, as `CarnyxProcess.attach` keeps it: Rust
/// never holds a Java object reference between calls, because the activity
/// reference `AndroidApp` hands out is a LOCAL one, valid only for the frame it
/// arrived on.
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

/// Say what is tuned now.
///
/// `title` is the call sign, or the dial when no call sign has resolved; `text`
/// is the second line. Both are ALWAYS shown — a logo-only banner was tried and
/// the platform draws a landscape wordmark into a square slot too small to read.
/// `logo` is the station's saved picture, drawn beside the words when there is
/// one, and an EMPTY STRING is "there is none" rather than an `Option` — 
/// there is no null String to hand `new_string`, and the Java side tests for
/// empty anyway.
///
/// A PATH, NOT PIXELS. Android decodes the file itself, and at a size it picks;
/// marshalling a decoded bitmap across this seam would be more code and a copy
/// of an image the platform is about to resample anyway.
///
/// ONE NOTIFICATION ID BEHIND THIS, so stepping four presets updates one banner
/// rather than stacking four — the driver wants to know where they landed, not
/// where they have been.
///
/// RETURNS WHAT HAPPENED, not whether it worked. Every way of failing used to
/// come back as `false` with the reason in logcat, which on a unit with no adb
/// reaches nobody — and the ways differ in what a person has to do about them:
/// notifications off is a Settings toggle, a downgraded channel needs a new
/// channel id and cannot be fixed from code, and a clean "posted" with no banner
/// on screen is SystemUI's.
#[allow(clippy::too_many_arguments)]
pub fn post(
    title: &str,
    text: &str,
    logo: &str,
    brand: i32,
    ground: i32,
    ink: i32,
    edge: i32,
    logo_fallback: i32,
    logo_plate: i32,
    plate: i32,
) -> String {
    let Some(class) = CLASS_REF.get() else {
        return "no alert class in this build".into();
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return "no JVM".into();
    };
    jvm.attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
        let t = env.new_string(title)?;
        let x = env.new_string(text)?;
        let l = env.new_string(logo)?;
        let out = env
            .call_static_method(
                class,
                jni_str!("post"),
                jni_sig!(
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;IIIIIII)\
                     Ljava/lang/String;"
                ),
                &[
                    (&t).into(),
                    (&x).into(),
                    (&l).into(),
                    brand.into(),
                    ground.into(),
                    ink.into(),
                    edge.into(),
                    logo_fallback.into(),
                    logo_plate.into(),
                    plate.into(),
                ],
            )?
            .l()?;
        let out = JString::cast_local(env, out)?;
        out.try_to_string(env)
    })
    .unwrap_or_else(|e| format!("JNI failure: {e}"))
}

/// Ask for the notification permission API 33 introduced.
///
/// ON DEMAND AND NEVER AT START-UP. `CarnyxAlert.requestPostNotifications` has
/// the reasoning: the standing rule against raising a dialog on a dashboard is
/// about asking unprompted, mid-drive, and this is a settings row a parked
/// driver taps. Below API 33 the Java side answers "not needed" without doing
/// anything, so calling it on any unit is safe.
pub fn request_post_notifications() -> String {
    let Some(class) = CLASS_REF.get() else {
        return "no alert class in this build".into();
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return "no JVM".into();
    };
    jvm.attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
        let out = env
            .call_static_method(
                class,
                jni_str!("requestPostNotifications"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let out = JString::cast_local(env, out)?;
        out.try_to_string(env)
    })
    .unwrap_or_else(|e| format!("JNI failure: {e}"))
}

/// Send the driver to Android's "Display over other apps" screen.
///
/// ON DEMAND, from a settings row. `CarnyxOverlay` has the reasoning: this is a
/// "special" permission that no dialog can request, so the only way to get it is
/// to open the screen and let the driver switch the app on. A ROM with no such
/// screen answers with a line saying so rather than throwing.
pub fn request_overlay_permission() -> String {
    let Some(class) = CLASS_REF.get() else {
        return "no alert class in this build".into();
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return "no JVM".into();
    };
    jvm.attach_current_thread(|env: &mut Env| -> Result<String, jni::errors::Error> {
        let out = env
            .call_static_method(
                class,
                jni_str!("requestOverlayPermission"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let out = JString::cast_local(env, out)?;
        out.try_to_string(env)
    })
    .unwrap_or_else(|e| format!("JNI failure: {e}"))
}

/// Take the pop-up down — the driver is back on the face and can see the dial.
pub fn clear() {
    let Some(class) = CLASS_REF.get() else {
        return;
    };
    let Ok(jvm) = JavaVM::singleton() else {
        return;
    };
    let _ = jvm.attach_current_thread(|env: &mut Env| -> Result<(), jni::errors::Error> {
        env.call_static_method(class, jni_str!("clear"), jni_sig!("()V"), &[])?;
        Ok(())
    });
}
