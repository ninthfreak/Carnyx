//! `LogoNet` and `ImageCodec`, over the platform.
//!
//! ## Why there is no TLS crate in this tree
//!
//! [`crate::logos::LogoNet`]'s own doc recommends `ureq` + `rustls` +
//! `webpki-roots`, and that recommendation is deliberately not taken. The head
//! unit is 32-bit ARM; a TLS stack there is a C dependency that must be
//! cross-compiled and verified, and `rusqlite` has already left this project with
//! one whose armv7 build has never been exercised. Against that, this app
//! already dexes Java and binds it over JNI twice — for the tuner and for
//! location — so one more class costs almost nothing.
//!
//! Platform HTTPS is also simply more correct here: it uses the system trust
//! store, which is what the rest of the device trusts and which tracks OS
//! updates. Bundled roots go stale on a machine that may never be updated again.
//!
//! Decoding follows the same logic. `BitmapFactory` reads PNG, JPEG, WebP and
//! GIF, and has been hardened against malformed input for a decade — which is the
//! point, because every byte it sees came off an image search.
//!
//! ## The division of labour
//!
//! Java moves bytes and pixels. Every decision stays in `src/logos.rs`: the DDG
//! two-step, the token, the parsing, the ranking, the six error strings a driver
//! reads. That half is tested; this half cannot be.
//!
//! NONE OF THIS HAS RUN. It compiles for `armv7-linux-androideabi`.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::refs::Global;
use jni::{jni_sig, jni_str, Env, JavaVM};

use crate::logos::ddg::{self, DdgImage};
use crate::logos::query;
use crate::logos::resolver::{fetch_error as err, MAX_LOGO_BYTES};
use crate::logos::{FetchedImage, ImageCodec, LogoNet, Raster};

const CLASS: &jni::strings::JNIStr = jni_str!("com/ninthfreak/carnyx/CarnyxNet");

static CLASS_REF: OnceLock<Global<JClass<'static>>> = OnceLock::new();

/// The cap on a search response. DuckDuckGo's HTML landing page — the one
/// carrying the `vqd` token — is a few hundred KB of markup; 2 MiB leaves room
/// for it to grow without letting a hostile or broken server stream forever into
/// a head unit's memory.
const MAX_TEXT_BYTES: i32 = 2 * 1024 * 1024;

/// Load the class. Unlike the tuner and location there are no callbacks to
/// register — every call here is Rust asking Java a question and waiting.
///
/// # Safety
///
/// As [`super::nwd::init`]: `vm` and `activity` must be what `AndroidApp` handed
/// out, with the activity still alive.
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
        let global = env
            .new_global_ref(&class)
            .map_err(|e| TunerError::Java(e.to_string()))?;
        let _ = CLASS_REF.set(global);
        Ok(())
    })
}

fn with_class<R>(f: impl FnOnce(&mut Env, &JClass) -> Result<R, jni::errors::Error>) -> Option<R> {
    let class = CLASS_REF.get()?;
    let jvm = JavaVM::singleton().ok()?;
    jvm.attach_current_thread(|env: &mut Env| f(env, class)).ok()
}

/// What the last Java call reported, read back together so the six driver-facing
/// error strings can be chosen correctly.
struct Outcome {
    status: u16,
    content_type: Option<String>,
    error: Option<String>,
}

/// Read a static method returning a possibly-null String.
///
/// The null check is not belt-and-braces: all three of these getters return null
/// as their ordinary "nothing to report" answer, and `try_to_string` on a null
/// `JString` is an error rather than an empty string.
fn static_string(env: &mut Env, class: &JClass, name: &jni::strings::JNIStr) -> Option<String> {
    let v = env
        .call_static_method(class, name, jni_sig!("()Ljava/lang/String;"), &[])
        .ok()?
        .l()
        .ok()?;
    if v.is_null() {
        return None;
    }
    JString::cast_local(env, v).ok()?.try_to_string(env).ok()
}

fn outcome(env: &mut Env, class: &JClass) -> Outcome {
    let status = env
        .call_static_method(class, jni_str!("lastStatus"), jni_sig!("()I"), &[])
        .and_then(|v| v.i())
        .unwrap_or(0);
    Outcome {
        status: status.clamp(0, u16::MAX as i32) as u16,
        content_type: static_string(env, class, jni_str!("lastContentType")),
        error: static_string(env, class, jni_str!("lastError")),
    }
}

/// The platform transport.
#[derive(Debug, Clone, Copy, Default)]
pub struct AndroidNet;

impl AndroidNet {
    fn get_text(&self, url: &str) -> Result<String, String> {
        with_class(|env, class| {
            let u = env.new_string(url)?;
            let ua = env.new_string(query::DDG_UA)?;
            let v = env.call_static_method(
                class,
                jni_str!("getText"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;I)Ljava/lang/String;"),
                &[(&u).into(), (&ua).into(), MAX_TEXT_BYTES.into()],
            )?;
            let obj = v.l()?;
            let out = if obj.is_null() {
                None
            } else {
                Some(JString::cast_local(env, obj)?.try_to_string(env)?)
            };
            Ok((out, outcome(env, class)))
        })
        .ok_or_else(|| err::unreachable("no network binding"))
        .and_then(|(body, o)| match body {
            Some(b) => Ok(b),
            None if o.status >= 400 => Err(err::http_status(o.status)),
            None => Err(err::unreachable(o.error.as_deref().unwrap_or("no response"))),
        })
    }
}

impl LogoNet for AndroidNet {
    /// The DuckDuckGo two-step. Both round trips are here; both PARSES are in
    /// `ddg`, where they are unit-tested against captured bodies.
    ///
    /// Every failure yields an empty vector rather than an error, which is the
    /// contract and matches CarFM: a search that finds nothing and a search that
    /// could not run look the same to the driver, and "no logos found" is the
    /// honest wording for both.
    fn search(&self, query: &str, n: usize) -> Result<Vec<DdgImage>, String> {
        let page = match self.get_text(&query::vqd_url(query)) {
            Ok(p) => p,
            Err(_) => return Ok(Vec::new()),
        };
        let Some(vqd) = ddg::parse_vqd(&page) else {
            return Ok(Vec::new());
        };
        let body = match self.get_text(&query::results_url(query, &vqd)) {
            Ok(b) => b,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(ddg::parse_results(&body, n))
    }

    fn fetch_image(&self, url: &str) -> Result<FetchedImage, String> {
        if !url.starts_with("https://") {
            // http:// included: the transport refuses it, and saying so plainly
            // is better than "couldn't reach".
            return Err(err::not_a_web_address());
        }
        let got = with_class(|env, class| {
            let u = env.new_string(url)?;
            let ua = env.new_string(query::DDG_UA)?;
            let v = env.call_static_method(
                class,
                jni_str!("getBytes"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;I)[B"),
                &[(&u).into(), (&ua).into(), (MAX_LOGO_BYTES as i32).into()],
            )?;
            let obj = v.l()?;
            let bytes = if obj.is_null() {
                None
            } else {
                let a: JByteArray = JByteArray::cast_local(env, obj)?;
                Some(env.convert_byte_array(&a)?)
            };
            Ok((bytes, outcome(env, class)))
        });

        let Some((bytes, o)) = got else {
            return Err(err::unreachable("no network binding"));
        };
        match bytes {
            Some(b) if b.is_empty() => Err(err::empty()),
            Some(b) => Ok(FetchedImage {
                mime: err::mime_from_content_type(o.content_type.as_deref()),
                bytes: b,
            }),
            None if o.error.as_deref() == Some("over the size cap") => {
                // The cap was hit mid-stream, so the true length is unknown; the
                // cap itself is the honest number to report.
                Err(err::too_large(MAX_LOGO_BYTES + 1))
            }
            None if o.status >= 400 => Err(err::http_status(o.status)),
            None => Err(err::unreadable(o.error.as_deref().unwrap_or("no response"))),
        }
    }

    fn fetch_text(&self, url: &str) -> Result<String, String> {
        self.get_text(url)
    }
}

/// The platform decoder.
#[derive(Debug, Clone, Copy, Default)]
pub struct AndroidCodec;

impl ImageCodec for AndroidCodec {
    /// One JNI crossing, not three: Java returns a big-endian `i32` width, a
    /// big-endian `i32` height, then straight RGBA, in a single BYTE array.
    ///
    /// Bytes rather than ints, and it matters on this hardware. A 1024×1024 logo
    /// as `int[]` components is 16 MB in Java and another 16 MB in the `Vec<i32>`
    /// it lands in here; with the `Bitmap` and the intermediate ARGB array still
    /// alive that is upwards of 40 MB in flight for one image, on a 32-bit head
    /// unit. As bytes it is 4 MB a side, and the pixels move straight into the
    /// `Raster` with no per-component conversion at all.
    fn decode(&self, bytes: &[u8]) -> Option<Raster> {
        /// Must match `CarnyxNet.HEADER`.
        const HEADER: usize = 8;

        let mut flat: Vec<u8> = with_class(|env, class| {
            let arr = env.byte_array_from_slice(bytes)?;
            let v = env.call_static_method(
                class,
                jni_str!("decode"),
                jni_sig!("([BI)[B"),
                &[(&arr).into(), (crate::logos::prep::DECODE_MAX_EDGE as i32).into()],
            )?;
            let obj = v.l()?;
            if obj.is_null() {
                return Ok(Vec::new());
            }
            let bytes: JByteArray = JByteArray::cast_local(env, obj)?;
            env.convert_byte_array(&bytes)
        })?;

        if flat.len() < HEADER {
            return None;
        }
        let w = i32::from_be_bytes([flat[0], flat[1], flat[2], flat[3]]);
        let h = i32::from_be_bytes([flat[4], flat[5], flat[6], flat[7]]);
        if w <= 0 || h <= 0 {
            return None;
        }
        let (w, h) = (w as u32, h as u32);
        // Guard the arithmetic rather than trusting the header: this is decoded
        // from an image off a search engine.
        let want = (w as usize)
            .checked_mul(h as usize)
            .and_then(|p| p.checked_mul(4))?;
        if flat.len() != want + HEADER {
            return None;
        }
        // `drain` rather than `flat[HEADER..].to_vec()`: the copy would hold both
        // buffers at once, which is the allocation this whole change exists to
        // avoid.
        flat.drain(..HEADER);
        let raster = Raster { w, h, rgba: flat };
        // The exact cap is applied here so the ladder maths lives in one place;
        // Java's inSampleSize only gets it into the right order of magnitude.
        Some(crate::logos::prep::resample_raster(
            &raster,
            crate::logos::prep::DECODE_MAX_EDGE,
        ))
    }

    fn encode_png(&self, raster: &Raster) -> Option<Vec<u8>> {
        if !raster.is_valid() {
            return None;
        }
        with_class(|env, class| {
            let arr = env.byte_array_from_slice(&raster.rgba)?;
            let v = env.call_static_method(
                class,
                jni_str!("encodePng"),
                jni_sig!("(II[B)[B"),
                &[
                    (raster.w as i32).into(),
                    (raster.h as i32).into(),
                    (&arr).into(),
                ],
            )?;
            let obj = v.l()?;
            if obj.is_null() {
                return Ok(Vec::new());
            }
            let out: JByteArray = JByteArray::cast_local(env, obj)?;
            env.convert_byte_array(&out)
        })
        .filter(|v| !v.is_empty())
    }
}
