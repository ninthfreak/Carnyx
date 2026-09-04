//! Turn-by-turn navigation from OsmAnd, over the platform.
//!
//! The seam and nothing else. `CarnyxNav.java` binds OsmAnd's AIDL service and
//! calls the three natives below; `crate::nav` decides what any of it means.
//! Read that module's header for the split and `CarnyxNav.java`'s for the API.
//!
//! ## Why the callbacks are registered by hand
//!
//! For [`super::location`]'s reason, and it is the same mechanism: the class
//! lives in the EMBEDDED DEX, loaded at run time by an `InMemoryDexClassLoader`,
//! so the JVM never resolves `Java_com_ninthfreak_carnyx_CarnyxNav_nativeNav` by
//! symbol lookup — that path only works for classes the platform loader knows.
//! `RegisterNatives` binds them explicitly against the class we loaded.
//!
//! ## What is confirmed
//!
//! NOTHING HERE HAS RUN AGAINST OSMAND, and no OsmAnd was installed anywhere in
//! this container. What is checked is that it compiles against this crate's jni
//! (`tools/check-jni.sh`) and that the AIDL it is built on still matches
//! upstream (`tools/check-osmand-aidl.sh`). Both are readings.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::errors::Error;
use jni::objects::{JClass, JObject, JObjectArray, JString};
use jni::refs::Global;
use jni::sys::{jboolean, jint, jlong};
use jni::{jni_sig, jni_str, Env, EnvUnowned, JavaVM, NativeMethod};

use super::TunerError;

const CLASS: &jni::strings::JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxNav");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Java → Rust: one navigation update, exactly as OsmAnd sent it.
///
/// THE SENTINELS CROSS UNTOUCHED. `(-1, -1)` is OsmAnd's "navigating, nothing to
/// say" and `12` is off-route; neither is filtered here, because deciding which
/// is which is [`crate::nav::Nav::state`]'s and it is tested there.
extern "system" fn native_nav<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    distance_to: jint,
    turn_type: jint,
    left_side: jboolean,
) {
    guard(&mut env, |_env| {
        super::ingest_nav(distance_to, turn_type, left_side);
        Ok(())
    });
}

/// Java → Rust: the refused edge — OsmAnd's gate closed (true) or opened
/// (false).
///
/// A STATE, NOT A LOG LINE. The note beside it goes to the diagnostics ring;
/// this goes to the settings row, which has to tell bound-but-refused apart
/// from bound-but-idle because only one of them has a fix the driver can
/// perform. See `CarnyxNav.afterPoll` for how the edges are found.
extern "system" fn native_nav_refused<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    refused: jboolean,
) {
    guard(&mut env, |_env| {
        super::ingest_nav_refused(refused);
        Ok(())
    });
}

/// Java → Rust: one voice-router announcement, as its two lists.
///
/// BOTH LISTS, unjoined. Which one to show is a decision and it is made in
/// `Nav::speak`; joining them here would make that choice at the seam where
/// nothing can test it.
extern "system" fn native_nav_voice<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    cmds: JObjectArray<'a>,
    played: JObjectArray<'a>,
) {
    guard(&mut env, |env| {
        let cmds = read_strings(env, &cmds)?;
        let played = read_strings(env, &played)?;
        super::ingest_nav_voice(cmds, played);
        Ok(())
    });
}

/// Java → Rust: one poll answer, field by field.
///
/// A NULL STRING IS A REAL ANSWER — "this route has no street name" and "the poll
/// has not landed" are both `None` and both collapse the element, which is the
/// handoff's rule. `JString::try_to_string` on a null throws, so each is read
/// through `opt_string`, which turns null into `None` rather than an exception.
#[allow(clippy::too_many_arguments)]
extern "system" fn native_nav_info<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    arrival_ms: jlong,
    left_seconds: jint,
    left_metres: jint,
    map_visible: jboolean,
    turn_name: JString<'a>,
    turn_type: JString<'a>,
    turn_metres: jint,
    imminent: jint,
    after_name: JString<'a>,
    after_type: JString<'a>,
) {
    guard(&mut env, |env| {
        // ZERO IS "NOT NAVIGATING" AND NOT A VALUE. OsmAnd answers `getAppInfo`
        // whether or not a route is running and zeroes these when it is not, so
        // the zero is filtered here — at the seam, where the encoding is known —
        // rather than in `crate::nav`, which would then have to know it too.
        let route = super::NavRoute {
            arrival_ms: (arrival_ms > 0).then_some(arrival_ms),
            left_seconds: (left_seconds > 0).then_some(left_seconds),
            left_metres: (left_metres > 0).then_some(left_metres),
            // NO `!= 0`: `jboolean` is `bool` in this crate's jni. See
            // `super::ingest_nav`, where the same mistake was made and caught.
            map_visible,
            street: opt_string(env, &turn_name),
            turn_xml: opt_string(env, &turn_type),
            turn_metres: (turn_metres > 0).then_some(turn_metres),
            // NOT FILTERED, because its scale is unknown and a filter would be a
            // guess about it. See `crate::nav::Route::imminent`.
            imminent: Some(imminent),
            after_street: opt_string(env, &after_name),
            after_turn_xml: opt_string(env, &after_type),
        };
        super::ingest_nav_info(route);
        Ok(())
    });
}

/// A Java string that may be null, and an empty one counts as absent.
fn opt_string(env: &mut Env, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    s.try_to_string(env).ok().filter(|t| !t.trim().is_empty())
}

/// One line into the diagnostics log, from the bind and the subscribe.
extern "system" fn native_nav_note<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    line: JString<'a>,
) {
    guard(&mut env, |env| {
        let text = line.try_to_string(env)?;
        super::ingest_note(text);
        Ok(())
    });
}

/// A Java `String[]` as a `Vec<String>`.
///
/// A NULL ELEMENT BECOMES AN EMPTY STRING rather than ending the read.
/// `CarnyxNav.toArray` already replaces nulls, so this is the second guard on
/// the same thing — and the cost of being wrong is a dropped announcement, not
/// a crash, only because of this.
fn read_strings(env: &mut Env, array: &JObjectArray) -> Result<Vec<String>, Error> {
    // ON THE ARRAY, NOT ON `Env`. The two `Env` methods this used —
    // `get_array_length` and `get_object_array_element` — are deprecated in jni
    // 0.22 and are documented as going away; `tools/check-jni.sh` is what says
    // so here, because nothing else in this container compiles this file. The
    // array's own `len` answers a `usize`, so the `max(0) as usize` that guarded
    // against a negative `jsize` has nothing left to guard.
    let len = array.len(env)?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let item = array.get_element(env, i)?;
        let s = JString::cast_local(env, item)?;
        out.push(s.try_to_string(env).unwrap_or_default());
    }
    Ok(out)
}

/// As `location`'s: a panic unwinding across the JNI boundary is undefined
/// behaviour, so it becomes a thrown `RuntimeException` instead.
fn guard<'a>(unowned: &mut EnvUnowned<'a>, body: impl FnOnce(&mut Env) -> Result<(), Error>) {
    unowned
        .with_env(body)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

fn natives() -> Vec<NativeMethod<'static>> {
    // SAFETY: each signature matches BOTH the Java declaration in
    // `CarnyxNav.java` and the parameter list of the function beside it. The
    // three are written together and must be changed together — a mismatch is
    // not a compile error on either side, it is a crash at the first update.
    unsafe {
        vec![
            NativeMethod::from_raw_parts(
                jni_str!("nativeNav"),
                jni_str!("(IIZ)V"),
                native_nav as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeNavVoice"),
                jni_str!("([Ljava/lang/String;[Ljava/lang/String;)V"),
                native_nav_voice as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeNavInfo"),
                jni_str!("(JIIZLjava/lang/String;Ljava/lang/String;IILjava/lang/String;Ljava/lang/String;)V"),
                native_nav_info as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeNavNote"),
                jni_str!("(Ljava/lang/String;)V"),
                native_nav_note as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeNavRefused"),
                jni_str!("(Z)V"),
                native_nav_refused as *mut c_void,
            ),
        ]
    }
}

/// Load the class, bind the three callbacks, and hand it the app context.
///
/// NOTHING IS SUBSCRIBED HERE. Binding OsmAnd is [`start`], which the app calls
/// only when the driver's switch is on — a feature that is off must not start
/// another app, and `BIND_AUTO_CREATE` would.
///
/// # Safety
///
/// As [`super::location::init`]: `vm` and `activity` must be what `AndroidApp`
/// handed out, with the activity still alive.
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

fn with_class<R>(f: impl FnOnce(&mut Env, &JClass) -> Result<R, Error>) -> Option<R> {
    let class = CLASS_REF.get()?;
    let jvm = JavaVM::singleton().ok()?;
    jvm.attach_current_thread(|env: &mut Env| f(env, class)).ok()
}

/// Which OsmAnd is installed, or `""` — WITHOUT binding it.
///
/// The settings row reads this so a driver can see whether the switch has
/// anything to talk to before they turn it on.
pub fn installed_package() -> String {
    with_class(|env, class| {
        let s = env
            .call_static_method(
                class,
                jni_str!("installedPackage"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        JString::cast_local(env, s)?.try_to_string(env)
    })
    .unwrap_or_default()
}

/// Which units OsmAnd's own settings say the driver chose, encoded.
///
/// 0 when nothing is known — OsmAnd not bound, the read declined, or a build
/// without the call. See `CarnyxNav.metricSystem` for the encoding and
/// [`crate::units::Units::from_osmand`] for what each number means here.
///
/// A `jint` and no allocation, because the caller reads this on every navigation
/// publish rather than caching an event.
pub fn metric_system() -> i32 {
    with_class(|env, class| {
        env.call_static_method(class, jni_str!("metricSystem"), jni_sig!("()I"), &[])?
            .i()
    })
    .unwrap_or(0)
}

/// Bind OsmAnd and subscribe. Returns a line for the diagnostics log.
pub fn start() -> String {
    with_class(|env, class| {
        let s = env
            .call_static_method(class, jni_str!("start"), jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        JString::cast_local(env, s)?.try_to_string(env)
    })
    .unwrap_or_else(|| "navigation is unavailable in this build".into())
}

/// Unsubscribe and let go. Returns a line for the diagnostics log.
pub fn stop() -> String {
    with_class(|env, class| {
        let s = env
            .call_static_method(class, jni_str!("stop"), jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        JString::cast_local(env, s)?.try_to_string(env)
    })
    .unwrap_or_default()
}

/// Bring OsmAnd to the front — the status-bar mark's tap. Returns a line for
/// the diagnostics log.
pub fn launch() -> String {
    with_class(|env, class| {
        let s = env
            .call_static_method(class, jni_str!("launch"), jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        JString::cast_local(env, s)?.try_to_string(env)
    })
    .unwrap_or_else(|| "navigation is unavailable in this build".into())
}
