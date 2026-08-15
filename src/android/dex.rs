//! Getting Java into a process that has none.
//!
//! Carnyx is a pure NativeActivity built by cargo-apk, which packages no Java
//! and no dex. The tuner is an AIDL binder service and there is no NDK path to
//! binder, so the Java in `java/` has to arrive some other way: `build.rs`
//! compiles and dexes it, the library embeds the dex with `include_bytes!`, and
//! this module hands it to a `dalvik.system.InMemoryDexClassLoader` at run time.
//!
//! The mechanism is Slint's, copied deliberately rather than invented — see
//! `i-slint-backend-android-activity-1.17.1`'s `javahelper.rs`, which ships this
//! on this device.
//!
//! ONE DELIBERATE SIMPLIFICATION. Slint carries a `DexClassLoader` fallback that
//! writes the dex into the code cache, because `InMemoryDexClassLoader` is API
//! 26+ and Slint supports older devices. Carnyx cannot: its floor is API 26
//! already, forced by skia-bindings, which hardcodes that level (see Cargo.toml).
//! A fallback for a device this APK will not install on would be untestable code
//! guarding an unreachable case, so instead the level is checked once and a
//! device below 26 gets a legible refusal.

use std::sync::OnceLock;

use jni::errors::Error;
use jni::objects::{JClass, JClassLoader, JObject, JValue, LoaderContext};
use jni::refs::Global;
use jni::strings::JNIStr;
use jni::{jni_sig, jni_str, Env};

/// The dex `build.rs` produced. Empty when the build ran with
/// `CARNYX_ALLOW_MISSING_DEX` set on a machine with no Android SDK — see
/// [`check`], which is where that becomes a legible error rather than a
/// `ClassNotFoundException` in a car.
const DEX_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/classes.dex"));

/// `InMemoryDexClassLoader` arrived in API 26, and so did the level Carnyx is
/// built against.
const MIN_API: i32 = 26;

static LOADER: OnceLock<Global<JClassLoader<'static>>> = OnceLock::new();

/// Everything that has to be true before a class can be loaded, checked once so
/// the failure names its own cause.
pub fn check(env: &mut Env) -> Result<(), String> {
    if DEX_DATA.is_empty() {
        return Err(
            "this library was built with CARNYX_ALLOW_MISSING_DEX and carries no Java: the NWD \
             tuner cannot run. Rebuild with an Android SDK on the path."
                .into(),
        );
    }
    let sdk = env
        .get_static_field(
            jni_str!("android/os/Build$VERSION"),
            jni_str!("SDK_INT"),
            jni_sig!("I"),
        )
        .and_then(|v| v.i())
        .map_err(|e| format!("could not read Build.VERSION.SDK_INT: {e}"))?;
    if sdk < MIN_API {
        return Err(format!(
            "API {sdk} is below {MIN_API}, so InMemoryDexClassLoader is unavailable and the tuner \
             cannot be loaded"
        ));
    }
    Ok(())
}

/// Load one of our classes out of the embedded dex.
///
/// `context` is any `android.content.Context` — the NativeActivity will do. Its
/// class loader becomes the PARENT of the dex loader, which is what lets the
/// embedded classes resolve `android.*` and, on a real unit, the vendor's own
/// framework classes.
///
/// `name` is the internal (slash-separated) form, e.g.
/// `com/ninthfreak/carnyx/NwdBridge`.
pub fn load_class<'local>(
    env: &mut Env<'local>,
    context: &JObject<'_>,
    name: &JNIStr,
) -> Result<JClass<'local>, Error> {
    if LOADER.get().is_none() {
        let built = build_loader(env, context)?;
        let global = env.new_global_ref(built)?;
        let _ = LOADER.set(global);
    }
    let loader = LOADER.get().expect("just set");
    // `true` initialises the class. It has to be initialised before its natives
    // are registered against it, and there is nothing in these classes' static
    // initialisers that can fail.
    LoaderContext::Loader(loader.as_ref()).find_class(env, name, true)
}

fn build_loader<'local>(
    env: &mut Env<'local>,
    context: &JObject<'_>,
) -> Result<JClassLoader<'local>, Error> {
    let parent = env
        .call_method(
            context,
            jni_str!("getClassLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        )?
        .l()?;

    // SAFETY: `DEX_DATA` is `'static` and `InMemoryDexClassLoader` does not
    // mutate the buffer it is given — the same argument Slint's backend makes
    // for the identical call.
    let buffer =
        unsafe { env.new_direct_byte_buffer(DEX_DATA.as_ptr() as *mut u8, DEX_DATA.len()) }?;

    let loader = env.new_object(
        jni_str!("dalvik/system/InMemoryDexClassLoader"),
        jni_sig!("(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V"),
        &[JValue::Object(&buffer), JValue::Object(&parent)],
    )?;

    JClassLoader::cast_local(env, loader)
}
