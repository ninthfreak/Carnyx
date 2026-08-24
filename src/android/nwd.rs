//! The JNI binding to the NWD radio bridge.
//!
//! Two directions cross here and they are asymmetric on purpose.
//!
//! OUT (Rust → Java) is a handful of static calls on
//! `com.ninthfreak.carnyx.NwdBridge`, all of them primitives, strings and `int[]`
//! — no parcelable ever crosses this boundary. The AIDL marshalling stays on the
//! Java side, where the generated `Stub` does it, and Rust never has to know
//! what a `Frequency` looks like on the wire.
//!
//! IN (Java → Rust) is thirteen static native methods, bound with
//! `RegisterNatives`. NOT with exported `Java_…` symbols: the bridge class is
//! loaded by a fresh `InMemoryDexClassLoader`, and JNI's automatic symbol lookup
//! searches only the native libraries registered against the DEFINING class's
//! loader — which for a dex loaded at run time is none of them. An exported
//! symbol would simply never be found. Slint's backend reaches the same
//! conclusion for the same reason.
//!
//! NONE OF THIS HAS RUN. There is no head unit, no NWD service and no Android
//! device in the container this was written in.
//!
//! AND IT IS NOT EVEN COMPILED THERE, which this note used to claim it was: "it
//! compiles for `armv7-linux-androideabi`, and that is the entire claim". That
//! was false and a Gradle build proved it — `write_log` shipped with a parameter
//! that shadowed the `text` helper below, a plain type error, and nothing in the
//! container caught it. `cargo check --target armv7-linux-androideabi` cannot run
//! without an NDK, because Slint's Android backend pulls in `skia-bindings` whose
//! build script demands one, so EVERY line in this file is unchecked until
//! someone builds the APK.
//!
//! What that means for anyone editing here: the host test suite proves nothing
//! about this file. Either build the APK, or type-check the changed function in a
//! throwaway crate against `jni` alone with local stubs for `TunerError`,
//! `with_bridge` and `text` — that reproduces this class of error exactly.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::errors::Error;
use jni::objects::{JClass, JIntArray, JObject, JString, JValue};
use jni::refs::Global;
use jni::sys::{jboolean, jint};
use jni::{jni_sig, jni_str, Env, EnvUnowned, JavaVM, NativeMethod};

use super::{BandPoint, Tuner, TunerError, TunerSnapshot};

/// The bridge class, as the dex loader knows it.
const BRIDGE_CLASS: &jni::strings::JNIStr = jni_str!("com/ninthfreak/carnyx/NwdBridge");

static BRIDGE: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// `attach_current_thread` propagates whatever error type the body returns, so
/// long as it can carry a JNI error. Everything from the JVM lands in
/// [`TunerError::Java`]: a JNI error is a fault in the plumbing, not a state the
/// app can act on differently.
impl From<Error> for TunerError {
    fn from(e: Error) -> Self {
        TunerError::Java(e.to_string())
    }
}

/// A handle to the head unit's tuner.
///
/// Zero-sized: all the state lives in the Java bridge and in this module's
/// process globals, because there is exactly one tuner in one process and the
/// vendor's callbacks are static.
#[derive(Debug, Clone, Copy)]
pub struct NwdTuner {
    _private: (),
}

/// Wire up the tuner: load the dex, bind the native callbacks, hand the bridge
/// the app context.
///
/// Call once, from `android_main`, with the pointers `AndroidApp` hands out:
///
/// ```ignore
/// let tuner = unsafe { carnyx::android::init(app.vm_as_ptr(), app.activity_as_ptr()) }?;
/// ```
///
/// Raw pointers rather than an `AndroidApp` deliberately: it keeps this module
/// free of both `slint` and `android-activity`, so the whole seam compiles and
/// its tests run on the host.
///
/// # Safety
///
/// `vm` must be the process's `JavaVM` and `activity` the `NativeActivity`
/// object reference, exactly as `AndroidApp::vm_as_ptr` and
/// `AndroidApp::activity_as_ptr` return them, and the activity reference must
/// still be alive.
pub unsafe fn init(vm: *mut c_void, activity: *mut c_void) -> Result<NwdTuner, TunerError> {
    if activity.is_null() {
        return Err(TunerError::Unavailable("null activity".into()));
    }
    // Slint's backend initialises the singleton during its own start-up, so this
    // usually finds it already there; `from_raw` covers being called first.
    let jvm = JavaVM::singleton()
        .or_else(|_| -> Result<JavaVM, Error> { Ok(unsafe { JavaVM::from_raw(vm.cast()) }) })
        .map_err(|e| TunerError::Java(e.to_string()))?;

    jvm.attach_current_thread(|env: &mut Env| -> Result<NwdTuner, TunerError> {
        super::dex::check(env).map_err(TunerError::Unavailable)?;

        let context = unsafe { JObject::from_raw(env, activity.cast()) };

        let class = super::dex::load_class(env, &context, BRIDGE_CLASS)
            .map_err(|e| TunerError::Java(format!("loading {BRIDGE_CLASS:?}: {e}")))?;

        // SAFETY: every entry pairs a `#[no_mangle]`-free `extern "system"` fn
        // below with the signature of the identically-named `private static
        // native` method in NwdBridge.java. The two lists are written together
        // and must be changed together.
        unsafe { env.register_native_methods(&class, &natives()) }
            .map_err(|e| TunerError::Java(format!("RegisterNatives: {e}")))?;

        let global = env
            .new_global_ref(&class)
            .map_err(|e| TunerError::Java(e.to_string()))?;
        let _ = BRIDGE.set(global);

        // Only now, with the callbacks bound, is it safe to let the bridge run:
        // `attach` is what makes every other entry point usable.
        env.call_static_method(
            &class,
            jni_str!("attach"),
            jni_sig!("(Landroid/content/Context;)V"),
            &[JValue::Object(&context)],
        )
        .map_err(|e| TunerError::Java(format!("attach: {e}")))?;

        Ok(NwdTuner { _private: () })
    })
}

/// Run `f` with the JNI environment and the bridge class.
///
/// Every call out to Java goes through here, so thread attachment and the
/// "did init run?" check exist in exactly one place.
fn with_bridge<R>(
    f: impl FnOnce(&mut Env, &Global<JClass<'static>>) -> Result<R, Error>,
) -> Result<R, TunerError> {
    let class = BRIDGE
        .get()
        .ok_or_else(|| TunerError::Unavailable("android::init has not run".into()))?;
    let jvm = JavaVM::singleton().map_err(|e| TunerError::Java(e.to_string()))?;
    jvm.attach_current_thread(|env: &mut Env| f(env, class))
        .map_err(|e: Error| TunerError::Java(e.to_string()))
}

/// Same, for the calls whose failure is not worth propagating — a broadcast that
/// did not send, a watch that did not start. The reason goes to logcat, which is
/// the only channel a head unit has.
fn call_void(what: &str, f: impl FnOnce(&mut Env, &Global<JClass<'static>>) -> Result<(), Error>) {
    if let Err(e) = with_bridge(f) {
        warn(&format!("{what}: {e}"));
    }
}

/// Write to logcat from Rust.
///
/// Goes through `android.util.Log` rather than `eprintln!` because a
/// NativeActivity's stderr is not connected to anything a driver or a developer
/// can read. `android.util.Log` is on the system class loader, so this works
/// even when the dex load itself is what failed.
fn warn(message: &str) {
    let _ = JavaVM::singleton().map(|jvm| {
        jvm.attach_current_thread(|env: &mut Env| -> Result<(), Error> {
            let tag = JString::new(env, "CarnyxNwd")?;
            let msg = JString::new(env, message)?;
            env.call_static_method(
                jni_str!("android/util/Log"),
                jni_str!("w"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)I"),
                &[JValue::Object(&tag), JValue::Object(&msg)],
            )?;
            Ok(())
        })
    });
}

impl Tuner for NwdTuner {
    fn is_available(&self) -> bool {
        with_bridge(|env, class| {
            env.call_static_method(class, jni_str!("isAvailable"), jni_sig!("()Z"), &[])?
                .z()
        })
        .unwrap_or(false)
    }

    fn connect(&self) -> Result<(), TunerError> {
        let accepted = with_bridge(|env, class| {
            env.call_static_method(class, jni_str!("connect"), jni_sig!("()Z"), &[])?
                .z()
        })?;
        if accepted {
            Ok(())
        } else {
            // The real outcome still arrives as ConnectFailed; this is only the
            // synchronous half, for a caller that wants to know the request
            // itself was refused.
            Err(TunerError::Unavailable("bindService was refused".into()))
        }
    }

    fn disconnect(&self) {
        call_void("disconnect", |env, class| {
            env.call_static_method(class, jni_str!("disconnect"), jni_sig!("()V"), &[])?;
            Ok(())
        });
    }

    fn tune(&self, mhz: f32) -> Result<(), TunerError> {
        // The MHz-to-raw conversion and the band are Rust's, not Java's: the
        // scale is a property of the unit that is learned at run time, and the
        // band must be the one the tuner last reported. See Calibration.
        let cal = super::calibration();
        let raw = cal.raw_for(mhz).ok_or(TunerError::NotCalibrated)?;
        let band = cal.band() as jint;
        let ok = with_bridge(|env, class| {
            env.call_static_method(
                class,
                jni_str!("tune"),
                jni_sig!("(II)Z"),
                &[JValue::Int(raw), JValue::Int(band)],
            )?
            .z()
        })?;
        if ok {
            Ok(())
        } else {
            Err(TunerError::NotConnected)
        }
    }

    fn seek(&self, up: bool) {
        call_void("seek", |env, class| {
            env.call_static_method(
                class,
                jni_str!("seek"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(up)],
            )?;
            Ok(())
        });
    }

    fn snapshot(&self) -> Option<TunerSnapshot> {
        with_bridge(|env, class| {
            let numbers = env
                .call_static_method(class, jni_str!("pollNumbers"), jni_sig!("()[I"), &[])?
                .l()?;
            if numbers.is_null() {
                return Ok(None);
            }
            let array = JIntArray::cast_local(env, numbers)?;
            let mut buf = [0i32; 5];
            array.get_region(env, 0, &mut buf)?;

            let ps = env
                .call_static_method(class, jni_str!("pollPs"), jni_sig!("()Ljava/lang/String;"), &[])?
                .l()?;
            let ps = JString::cast_local(env, ps)?;
            let rt = env
                .call_static_method(class, jni_str!("pollRt"), jni_sig!("()Ljava/lang/String;"), &[])?
                .l()?;
            let rt = JString::cast_local(env, rt)?;

            Ok(Some((buf, text(env, &ps), text(env, &rt))))
        })
        .ok()
        .flatten()
        .map(|(buf, ps, rt)| {
            let [raw, band, pty, stereo, source] = buf;
            let band = band as i8;
            TunerSnapshot {
                raw,
                mhz: super::calibration().mhz_for(raw),
                band,
                ps,
                rt,
                pty,
                stereo_getter_stuck: stereo != 0,
                mcu_source: (source >= 0).then_some(source),
            }
        })
    }

    fn set_audio_enabled(&self, on: bool) {
        call_void("setAudioEnabled", |env, class| {
            env.call_static_method(
                class,
                jni_str!("setAudioEnabled"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(on)],
            )?;
            Ok(())
        });
    }

    fn set_rds_enabled(&self, on: bool) {
        call_void("setRdsEnabled", |env, class| {
            env.call_static_method(
                class,
                jni_str!("setRdsEnabled"),
                jni_sig!("(Z)V"),
                &[JValue::Bool(on)],
            )?;
            Ok(())
        });
    }

    fn send_panel_key(&self, code: i32) {
        call_void("sendPanelKey", |env, class| {
            env.call_static_method(
                class,
                jni_str!("sendPanelKey"),
                jni_sig!("(I)V"),
                &[JValue::Int(code)],
            )?;
            Ok(())
        });
    }

    fn start_level_watch(&self, interval_ms: i64) {
        call_void("startLevelWatch", |env, class| {
            env.call_static_method(
                class,
                jni_str!("startLevelWatch"),
                jni_sig!("(J)V"),
                &[JValue::Long(interval_ms)],
            )?;
            Ok(())
        });
    }

    fn stop_level_watch(&self) {
        call_void("stopLevelWatch", |env, class| {
            env.call_static_method(class, jni_str!("stopLevelWatch"), jni_sig!("()V"), &[])?;
            Ok(())
        });
    }

    fn read_level_now(&self) {
        call_void("readLevelNow", |env, class| {
            env.call_static_method(class, jni_str!("readLevelNow"), jni_sig!("()V"), &[])?;
            Ok(())
        });
    }

    fn band_plan(&self) -> Vec<BandPoint> {
        let flat = with_bridge(|env, class| {
            let array = env
                .call_static_method(class, jni_str!("bandPlan"), jni_sig!("()[I"), &[])?
                .l()?;
            if array.is_null() {
                return Ok(Vec::new());
            }
            let array = JIntArray::cast_local(env, array)?;
            let mut buf = vec![0i32; array.len(env)?];
            array.get_region(env, 0, &mut buf)?;
            Ok(buf)
        })
        .unwrap_or_default();
        super::band_plan_from_raw(&flat)
    }

    fn start_illumination_watch(&self) {
        call_void("startIlluminationWatch", |env, class| {
            env.call_static_method(
                class,
                jni_str!("startIlluminationWatch"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
    }

    /// See `Tuner::write_log`. The only call out of here that passes a string IN.
    ///
    /// THE FAILURE COMES BACK AS A VALUE, not as a Java exception, and the "!"
    /// prefix is that convention — `NwdBridge.writeLog` documents why. A thrown
    /// exception reaches jni-rs as a bare `Error::JavaException` with the message
    /// still pending on the JVM, and the message is the entire value of the call
    /// on a unit whose only readable channel is the panel that asked for the save.
    /// `body` RATHER THAN `text`, and the name is not cosmetic: this module has a
    /// free function called `text` that turns a `JString` into a `String`, and a
    /// parameter of that name shadows it — `text(env, &out)` below then reads as a
    /// call on a `&str` and does not compile. It got here because this file is
    /// `#[cfg(target_os = "android")]` and the container it was written in has no
    /// NDK, so `cargo check` never reaches this line. The Gradle build did.
    fn write_log(&self, body: &str) -> Result<String, TunerError> {
        let out = with_bridge(|env, class| {
            let arg = env.new_string(body)?;
            let out = env
                .call_static_method(
                    class,
                    jni_str!("writeLog"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[JValue::Object(&arg)],
                )?
                .l()?;
            let out = JString::cast_local(env, out)?;
            Ok(text(env, &out))
        })?;
        match out.strip_prefix('!') {
            Some(why) => Err(TunerError::Java(why.to_string())),
            // `text` turns a null into an empty string, so this is the one shape
            // the sentinel cannot describe: the method returned nothing at all.
            None if out.is_empty() => Err(TunerError::Java("writeLog returned nothing".into())),
            None => Ok(out),
        }
    }
}

/// A Java string as a Rust one. A null or unreadable string is empty rather than
/// an error: every one of these is a display field, and a missing PS name is a
/// blank plate, not a fault.
///
/// Java is allowed to send null here and does — `getRtMessage()` is declared to
/// return `String` and this firmware's radio manager hardcodes it to `""`, while
/// `psName` is genuinely null for a passive client.
fn text(env: &Env, s: &JString<'_>) -> String {
    if s.is_null() {
        return String::new();
    }
    s.try_to_string(env).unwrap_or_default()
}

// ── Native callbacks ─────────────────────────────────────────────────────────
//
// One per `private static native` method in NwdBridge.java. Each one does the
// least it can: pull the arguments across and hand them to the ingest edge in
// `super`, which is where every decision about them is made and where the tests
// live.

/// Bind the callbacks. Built at run time rather than as a `const` because
/// casting a function item to a raw pointer is not a const operation.
fn natives() -> Vec<NativeMethod<'static>> {
    // SAFETY for every entry: the signature string matches both the Java
    // declaration and the Rust function's parameter list, and each function is a
    // static native (second parameter `JClass`).
    unsafe {
        vec![
            NativeMethod::from_raw_parts(
                jni_str!("nativeConnected"),
                jni_str!("(IILjava/lang/String;Ljava/lang/String;IZ)V"),
                native_connected as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeConnectFailed"),
                jni_str!("(Ljava/lang/String;)V"),
                native_connect_failed as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeDisconnected"),
                jni_str!("()V"),
                native_disconnected as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeFrequency"),
                jni_str!("(IILjava/lang/String;I)V"),
                native_frequency as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeRadioText"),
                jni_str!("(Ljava/lang/String;)V"),
                native_radio_text as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeRdsGroup"),
                jni_str!("(Ljava/lang/String;)V"),
                native_rds_group as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeStereo"),
                jni_str!("(Z)V"),
                native_stereo as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativePty"),
                jni_str!("(I)V"),
                native_pty as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeScanState"),
                jni_str!("(I)V"),
                native_scan_state as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeRadioState"),
                jni_str!("(I)V"),
                native_radio_state as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeLevel"),
                jni_str!("(IIIZLjava/lang/String;)V"),
                native_level as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativePanelKey"),
                jni_str!("(ILjava/lang/String;)V"),
                native_panel_key as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeIllumination"),
                jni_str!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V"),
                native_illumination as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("nativeSleep"),
                jni_str!("(Ljava/lang/String;)V"),
                native_sleep as *mut c_void,
            ),
        ]
    }
}

/// Upgrade the raw environment, run the body, and never let a panic or an
/// exception cross back into the vendor's binder thread.
fn guard<'a>(unowned: &mut EnvUnowned<'a>, body: impl FnOnce(&mut Env) -> Result<(), Error>) {
    unowned
        .with_env(body)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

extern "system" fn native_connected<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    band: jint,
    raw: jint,
    ps: JString<'a>,
    rt: JString<'a>,
    pty: jint,
    registered: jboolean,
) {
    guard(&mut env, |env| {
        super::ingest_connected(
            band,
            raw,
            text(env, &ps),
            text(env, &rt),
            pty,
            registered,
        );
        Ok(())
    });
}

extern "system" fn native_connect_failed<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    reason: JString<'a>,
) {
    guard(&mut env, |env| {
        super::ingest_connect_failed(text(env, &reason));
        Ok(())
    });
}

extern "system" fn native_disconnected<'a>(mut env: EnvUnowned<'a>, _class: JClass<'a>) {
    guard(&mut env, |_env| {
        super::ingest_disconnected();
        Ok(())
    });
}

extern "system" fn native_frequency<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    band: jint,
    raw: jint,
    ps: JString<'a>,
    slot: jint,
) {
    guard(&mut env, |env| {
        super::ingest_frequency(band, raw, text(env, &ps), slot);
        Ok(())
    });
}

extern "system" fn native_radio_text<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    rt: JString<'a>,
) {
    guard(&mut env, |env| {
        super::ingest_radio_text(text(env, &rt));
        Ok(())
    });
}

extern "system" fn native_rds_group<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    hex: JString<'a>,
) {
    guard(&mut env, |env| {
        super::ingest_rds_group(&text(env, &hex));
        Ok(())
    });
}

extern "system" fn native_stereo<'a>(mut env: EnvUnowned<'a>, _class: JClass<'a>, on: jboolean) {
    guard(&mut env, |_env| {
        super::ingest_stereo(on);
        Ok(())
    });
}

extern "system" fn native_pty<'a>(mut env: EnvUnowned<'a>, _class: JClass<'a>, pty: jint) {
    guard(&mut env, |_env| {
        super::ingest_pty(pty);
        Ok(())
    });
}

extern "system" fn native_scan_state<'a>(mut env: EnvUnowned<'a>, _class: JClass<'a>, state: jint) {
    guard(&mut env, |_env| {
        super::ingest_scan_state(state);
        Ok(())
    });
}

extern "system" fn native_radio_state<'a>(mut env: EnvUnowned<'a>, _class: JClass<'a>, state: jint) {
    guard(&mut env, |_env| {
        super::ingest_radio_state(state);
        Ok(())
    });
}

extern "system" fn native_level<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    level: jint,
    asked: jint,
    landed: jint,
    ok: jboolean,
    err: JString<'a>,
) {
    guard(&mut env, |env| {
        let err = if err.is_null() { None } else { Some(text(env, &err)) };
        super::ingest_level(level, asked, landed, ok, err);
        Ok(())
    });
}

extern "system" fn native_panel_key<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    key: jint,
    action: JString<'a>,
) {
    guard(&mut env, |env| {
        super::ingest_panel_key(key, text(env, &action));
        Ok(())
    });
}

extern "system" fn native_sleep<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    action: JString<'a>,
) {
    guard(&mut env, |env| {
        super::ingest_sleep(text(env, &action));
        Ok(())
    });
}

extern "system" fn native_illumination<'a>(
    mut env: EnvUnowned<'a>,
    _class: JClass<'a>,
    action: JString<'a>,
    extras: JString<'a>,
    ui_mode: JString<'a>,
) {
    guard(&mut env, |env| {
        super::ingest_illumination(
            text(env, &action),
            text(env, &extras),
            text(env, &ui_mode),
        );
        Ok(())
    });
}
