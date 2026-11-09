//! Turn-by-turn navigation from CoMaps, over the platform (#134).
//!
//! The seam and nothing else, on [`super::nav`]'s pattern. `CarnyxComaps.java`
//! watches CoMaps' navigation provider and calls the three natives below;
//! `crate::comaps` decides what any of it means. Read that module's header for
//! what the provider is and why it is watched rather than polled, and the Java
//! class's for the permission.
//!
//! ## Why the callbacks are registered by hand
//!
//! [`super::nav`]'s reason exactly: the class lives in the EMBEDDED DEX, loaded
//! by an `InMemoryDexClassLoader`, so the JVM never resolves
//! `Java_com_ninthfreak_carnyx_CarnyxComaps_nativeComapsRow` by symbol lookup.
//! `RegisterNatives` binds them against the class we loaded.
//!
//! ## What is confirmed
//!
//! NOTHING HERE HAS RUN AGAINST COMAPS. What is checked in this container is
//! that it compiles against this crate's jni (`tools/check-jni.sh`).

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::errors::Error;
use jni::objects::{JClass, JObject, JObjectArray, JString};
use jni::refs::Global;
use jni::sys::jint;
use jni::{jni_sig, jni_str, Env, EnvUnowned, JavaVM, NativeMethod};

use super::TunerError;

const CLASS: &jni::strings::JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxComaps");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// Java → Rust: one `/live` row, its column names and its cells.
///
/// A NULL CELL STAYS `None`. The provider writes `null` for a destination it
/// does not have, and `crate::comaps::Row::get` reads null and empty alike as
/// absent — but that is its decision, not this seam's, so the null crosses.
extern "system" fn native_comaps_row<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    names: JObjectArray<'a>,
    values: JObjectArray<'a>,
) {
    guard(&mut env, |env| {
        let names = read_strings(env, &names)?.into_iter().map(Option::unwrap_or_default).collect();
        let values = read_strings(env, &values)?;
        super::ingest_comaps_row(names, values);
        Ok(())
    });
}

/// Java → Rust: the link state, one of `CarnyxComaps.LINK_*`, on a change.
extern "system" fn native_comaps_link<'a>(mut env: EnvUnowned<'a>, _class: JClass<'a>, state: jint) {
    guard(&mut env, |_env| {
        super::ingest_comaps_link(state);
        Ok(())
    });
}

/// One line into the diagnostics log.
extern "system" fn native_comaps_note<'a>(
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

/// A Java `String[]`, each element `None` when it is null.
///
/// ON THE ARRAY AND NOT ON `Env`, for the deprecation `super::nav::read_strings`
/// records — `tools/check-jni.sh` is what says so.
fn read_strings(env: &mut Env, array: &JObjectArray) -> Result<Vec<Option<String>>, Error> {
    let len = array.len(env)?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let item = array.get_element(env, i)?;
        if item.is_null() {
            out.push(None);
            continue;
        }
        let s = JString::cast_local(env, item)?;
        out.push(s.try_to_string(env).ok());
    }
    Ok(out)
}

/// As `nav`'s: a panic unwinding across the JNI boundary is undefined
/// behaviour, so it becomes a thrown `RuntimeException` instead.
fn guard<'a>(unowned: &mut EnvUnowned<'a>, body: impl FnOnce(&mut Env) -> Result<(), Error>) {
    unowned
        .with_env(body)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

fn natives() -> Vec<NativeMethod<'static>> {
    // SAFETY: each signature matches BOTH the Java declaration in
    // `CarnyxComaps.java` and the parameter list of the function beside it.
    // Written together and to be changed together — a mismatch is not a
    // compile error on either side, it is a crash at the first row.
    unsafe {
        vec![
            NativeMethod::from_raw_parts(
                jni_str!("nativeComapsRow"),
                jni_str!("([Ljava/lang/String;[Ljava/lang/String;)V"),
                native_comaps_row as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeComapsLink"),
                jni_str!("(I)V"),
                native_comaps_link as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeComapsNote"),
                jni_str!("(Ljava/lang/String;)V"),
                native_comaps_note as *mut c_void,
            ),
        ]
    }
}

/// Load the class, bind the three callbacks, and hand it the Activity.
///
/// NOTHING IS WATCHED HERE. That is [`start`], which the app calls only when
/// the driver's switch is on and CoMaps is the chosen app.
///
/// # Safety
///
/// As [`super::nav::init`]: `vm` and `activity` must be what `AndroidApp`
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

/// One `()Ljava/lang/String;` static call, or `None` when the class is not loaded.
fn call_string(name: &'static jni::strings::JNIStr) -> Option<String> {
    with_class(|env, class| {
        let s = env
            .call_static_method(class, name, jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        JString::cast_local(env, s)?.try_to_string(env)
    })
}

/// Which CoMaps has the navigation provider, or `""` — WITHOUT watching it.
pub fn installed_package() -> String {
    call_string(jni_str!("installedPackage")).unwrap_or_default()
}

/// A CoMaps installed without the provider, or `""`.
pub fn outdated_package() -> String {
    call_string(jni_str!("outdatedPackage")).unwrap_or_default()
}

/// Start watching. Returns a line for the diagnostics log.
pub fn start() -> String {
    call_string(jni_str!("start")).unwrap_or_else(|| "CoMaps is unavailable in this build".into())
}

/// Stop watching. Returns a line for the diagnostics log.
pub fn stop() -> String {
    call_string(jni_str!("stop")).unwrap_or_default()
}

/// Ask for the permission. Returns a line for the diagnostics log.
pub fn request_permission() -> String {
    call_string(jni_str!("requestPermission"))
        .unwrap_or_else(|| "CoMaps is unavailable in this build".into())
}

/// Bring CoMaps to the front. Returns a line for the diagnostics log.
pub fn launch() -> String {
    call_string(jni_str!("launch")).unwrap_or_else(|| "CoMaps is unavailable in this build".into())
}
