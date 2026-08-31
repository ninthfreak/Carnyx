#!/usr/bin/env bash
# Type-check the JNI seam modules on the HOST, without an NDK.
#
# ── THE HOLE THIS FILLS ───────────────────────────────────────────────────────
#
# Everything in src/android/ is `#[cfg(target_os = "android")]`, and the only
# thing that compiles it is a build for the target. That build needs an NDK,
# because slint pulls in skia-bindings, which compiles Skia from source for
# armv7. A container without one cannot run `cargo check --target
# armv7-linux-androideabi` at all — it panics in skia's build script long before
# reaching our code.
#
# So `cargo check`, `cargo test` and `cargo clippy` all pass over this code
# without reading a line of it, and a JNI mistake is found by the owner, on their
# machine, at the end of a Gradle build. That has now happened three times with
# the SAME mistake: `Env::get_string` and a `JObject -> JString` `.into()`, which
# this crate's jni does not have. `location.rs:88` and `probe.rs:106` each carry
# a note about it.
# Notes did not stop the third one; this does.
#
# ── HOW ───────────────────────────────────────────────────────────────────────
#
# `jni` is ordinary Rust and builds fine for the host — it is only a
# target-specific DEPENDENCY of this crate, not a target-specific crate. So the
# module bodies are copied verbatim into a throwaway crate that depends on the
# same jni version, with stubs for the few `super::` items they reach, and
# checked there. Nothing runs; it only has to compile.
#
# ── THE IMPORTS ARE THE MODULE'S OWN, AND THAT IS NOT A DETAIL ────────────────
#
# The first version of this script DELETED every `use jni::…` and `use std::…`
# line out of each module and handed all of them one generous prelude at the
# crate root — `JByteArray, JClass, JIntArray, JObject, JString, JValue` — which
# each generated module then pulled in with `use super::*`.
#
# So it passed a module that USED `JString` WITHOUT IMPORTING IT, which is what
# `alert.rs` did, and the owner's Gradle build found it: "cannot find type
# `JString` in this scope". A harness whose whole job is to catch a missing or
# wrong JNI name cannot supply the names itself. Now the module's own `use`
# lines are copied along with everything else, the stubs keep their imports
# inside their own scope, and there is no crate-root glob to fall back on.
#
# UNUSED IMPORTS ARE DENIED for the same reason and in the same pass: the same
# build reported two, in `probe.rs` and `stock.rs`. A warning the owner reads on
# their machine is a warning this script should have read here.
#
# WHAT THIS DOES NOT CHECK: that the descriptors match the Java, that the classes
# load, or that any of it behaves. It checks the Rust — which is the half that
# was breaking the build.
set -euo pipefail

cd "$(dirname "$0")/.."

# ── FIRST: EVERY init IS CALLED, NOT JUST CORRECT ────────────────────────────
#
# Type-checking the seam says a module WOULD work; it says nothing about
# whether `android_main` ever runs it. `nav::init` shipped complete, correct,
# type-checked by this very script — and uncalled, so `installedPackage`
# answered "" and the settings row reported OsmAnd "not installed" on a unit
# with OsmAnd mid-route. `android_main` is the one function no container check
# can compile (skia's build script needs an NDK), so the wiring is held by
# grep: every src/android module that exports an `init` must be invoked from
# src/lib.rs — by its own path, or through mod.rs's `pub use nwd::init`, which
# lib.rs calls as `android::init`.
for f in src/android/*.rs; do
    m="$(basename "$f" .rs)"
    grep -q "pub unsafe fn init\|pub fn init" "$f" || continue
    if grep -q "android::${m}::init" src/lib.rs; then
        continue
    fi
    if grep -q "pub use ${m}::{init" src/android/mod.rs && grep -q "android::init(" src/lib.rs; then
        continue
    fi
    echo "FAIL: src/android/${m}.rs exports init() and src/lib.rs never calls it —" >&2
    echo "      the class never loads and every probe answers 'not installed'." >&2
    exit 1
done
echo "init wiring: every android module init is called from android_main"
ROOT="$PWD"

# The modules with no JNI in them, or whose shape the harness cannot stub. Named
# rather than pattern-matched, and PRINTED, because a silent skip is how a check
# like this stops covering something without anyone noticing.
# `location.rs` is skipped for the shape its `EnvUnowned` natives take, which
# this harness cannot stub; `nav.rs` takes the SAME shape and is checked anyway,
# because it is the newest JNI in the tree and the one nothing else covers.
SKIP=(mod.rs dex.rs nwd.rs net.rs location.rs)

# The jni version this crate actually resolves to, read from the lockfile rather
# than from Cargo.toml's range — checking against a different patch would be
# checking against a different API.
JNI_VERSION="$(awk '/^name = "jni"$/ { getline; if ($3 ~ /"0\.22/) { gsub(/"/, "", $3); print $3; exit } }' Cargo.lock)"
if [ -z "$JNI_VERSION" ]; then
    echo "could not read the jni 0.22 version out of Cargo.lock" >&2
    exit 1
fi

OUT="${TMPDIR:-/tmp}/carnyx-jnicheck"
rm -rf "$OUT"
mkdir -p "$OUT/src"

cat > "$OUT/Cargo.toml" <<EOF
[package]
name = "carnyx-jnicheck"
version = "0.0.0"
edition = "2021"

[dependencies]
jni = "=$JNI_VERSION"

[workspace]
EOF

python3 - "$ROOT" "$OUT" "${SKIP[@]}" <<'PY'
import os, re, sys

root, out = sys.argv[1], sys.argv[2]
skip = set(sys.argv[3:])
src = os.path.join(root, "src", "android")

head = '''//! GENERATED by tools/check-jni.sh — do not edit, do not commit.
//!
//! Verbatim copies of carnyx's android JNI module bodies, type-checked on the
//! host against the same jni version the real build resolves to.
//!
//! NOTHING IS IMPORTED AT THIS LEVEL. Every jni name a module uses has to come
//! from that module's own `use` lines, copied in with the rest of it — a crate
//! root full of jni types would be handed to each module by `use super::*` and
//! would hide exactly the mistake this script exists to find. The stubs below
//! carry their imports inside their own scope for the same reason.
#![allow(dead_code, unused_variables, clippy::all)]
#![deny(unused_imports)]

/// Stands in for `android::TunerError`. The variants the seam modules construct
/// and the `From` impl they rely on through `?` — see `nwd.rs`.
#[derive(Debug)]
pub enum TunerError { Unavailable(String), NotConnected, NotCalibrated, Java(String) }

impl From<jni::errors::Error> for TunerError {
    fn from(e: jni::errors::Error) -> Self { TunerError::Java(e.to_string()) }
}

/// The real one implements Display (`src/android/mod.rs`), and the seam modules
/// format errors with it. Without this the harness rejects correct code.
impl std::fmt::Display for TunerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TunerError::Unavailable(w) => write!(f, "tuner unavailable: {w}"),
            TunerError::NotConnected => write!(f, "tuner not connected"),
            TunerError::NotCalibrated => write!(f, "tuner frequency scale not learned yet"),
            TunerError::Java(w) => write!(f, "tuner call failed: {w}"),
        }
    }
}

/// Stands in for `android::dex`, which loads the embedded classes.dex. Its
/// SIGNATURES are what the callers type-check against; its behaviour is not
/// under test here and could not be, with no JVM.
///
/// ITS OWN IMPORTS, not the crate root's — see the module note above.
pub mod dex {
    use jni::objects::{JClass, JObject};
    use jni::strings::JNIStr;
    use jni::Env;
    pub fn check(_env: &mut Env) -> Result<(), String> { Ok(()) }
    pub fn load_class<'a>(_env: &mut Env<'a>, _ctx: &JObject<'a>, _name: &JNIStr)
        -> Result<JClass<'a>, jni::errors::Error> { unimplemented!() }
}

/// Stands in for `android::is_foreground`.
pub fn is_foreground() -> bool { true }

/// Stands in for the `ingest_*` edges the seam modules push events through.
/// Their SIGNATURES are what the callers type-check against; where the event
/// goes is not under test here and has no queue to go to.
pub fn ingest_note(_line: String) {}
pub fn ingest_nav(_distance_to: jni::sys::jint, _turn_type: jni::sys::jint, _left_side: jni::sys::jboolean) {}
pub fn ingest_nav_voice(_cmds: Vec<String>, _played: Vec<String>) {}
pub fn ingest_nav_refused(_refused: jni::sys::jboolean) {}
pub fn ingest_nav_info(_route: NavRoute) {}

/// Stands in for `crate::nav::Route`, which `android::mod` re-exports as
/// `NavRoute`. The FIELD TYPES are what the seam type-checks against — the seam
/// builds one of these field by field out of jni scalars, and a field whose type
/// moved is exactly the mistake this harness is for.
#[derive(Default)]
pub struct NavRoute {
    pub arrival_ms: Option<i64>,
    pub left_seconds: Option<i32>,
    pub left_metres: Option<i32>,
    pub map_visible: bool,
    pub street: Option<String>,
    pub turn_xml: Option<String>,
    pub turn_metres: Option<i32>,
    pub imminent: Option<i32>,
    pub after_street: Option<String>,
    pub after_turn_xml: Option<String>,
}
'''

names = sorted(
    f for f in os.listdir(src)
    if f.endswith(".rs") and f not in skip
)
if not names:
    sys.exit("no android modules to check — the skip list covers all of them")

mods = []
for f in names:
    body = open(os.path.join(src, f)).read()
    body = re.sub(r'(?m)^//!.*\n', '', body)          # module docs
    # THE `use` LINES STAY. Stripping them and providing a prelude is what let a
    # module reach a jni type it had never imported — see the header note.
    body = body.replace("super::dex::", "crate::dex::")
    body = body.replace("super::is_foreground()", "crate::is_foreground()")
    # NOT `f`: that is the loop's filename and shadowing it made every generated
    # module `pub mod ingest_`.
    for edge in ("ingest_note", "ingest_nav_voice", "ingest_nav_refused", "ingest_nav_info", "ingest_nav", "NavRoute"):
        body = body.replace("super::" + edge, "crate::" + edge)
    body = body.replace("super::TunerError", "crate::TunerError")
    # NO `use super::*` EITHER, for the same reason: the three stubs are reached
    # by their `crate::` paths, rewritten above, and a glob here would put every
    # name at the crate root back into every module.
    mods.append("pub mod %s {\n%s\n}\n" % (f[:-3], body))

open(os.path.join(out, "src", "lib.rs"), "w").write(head + "\n".join(mods))
print("checking: " + ", ".join(n[:-3] for n in names))
print("skipping: " + ", ".join(sorted(s[:-3] for s in skip)))
PY

echo
cd "$OUT"
if cargo check --quiet 2>&1; then
    echo
    echo "JNI seam type-checks against jni $JNI_VERSION."
else
    echo
    echo "JNI seam does NOT compile. The generated crate is at $OUT" >&2
    exit 1
fi
