#!/usr/bin/env bash
# Build the APK with Gradle instead of cargo-apk.
#
# Rust still compiles the library; Gradle only packages it. Two steps, kept
# explicit rather than hidden behind a Gradle task that shells out to cargo,
# because when this fails you want to know which half failed without reading a
# Gradle stack trace in a car park.
#
#   ./tools/build-apk-gradle.sh              # debug APK, both ABIs
#   ./tools/build-apk-gradle.sh armeabi-v7a  # just the ABI the unit needs
#
# WHY THIS EXISTS: cargo-apk cannot declare a <service> or a <receiver>, so it
# cannot produce the foreground service that stops the app being killed when you
# switch away from it. See android/settings.gradle.kts and task #67.
set -euo pipefail

cd "$(dirname "$0")/.."

read -r -a ABIS <<< "${*:-armeabi-v7a arm64-v8a}"

JNI_DIR="android/app/src/main/jniLibs"

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
die() { printf '\n%s\n' "$*" >&2; exit 1; }

# ── The three environment variables, and why there are three ────────────────
# cargo-ndk reads ANDROID_NDK_HOME. skia-bindings reads ANDROID_NDK and nothing
# else (build_support/platform/android.rs:69) and panics without it. The SDK
# tools want ANDROID_HOME. This is the same trap the README documents for the
# cargo-apk build; moving packagers does not fix it, because it is skia's.
[[ -n "${ANDROID_HOME:-}" ]] || die "ANDROID_HOME is not set — see the README's Building section."
NDK="${ANDROID_NDK_ROOT:-${ANDROID_NDK_HOME:-${ANDROID_NDK:-}}}"
[[ -n "$NDK" ]] || die "No NDK: set ANDROID_NDK_ROOT (and ANDROID_NDK, which skia-bindings reads)."
export ANDROID_NDK_HOME="$NDK"
export ANDROID_NDK="$NDK"
[[ -d "$NDK" ]] || die "ANDROID_NDK_ROOT points at $NDK, which does not exist."

command -v cargo-ndk >/dev/null 2>&1 || die "cargo-ndk is not installed:
  cargo install cargo-ndk

It is what sets the cross-compiler, linker and sysroot for each ABI — the job
cargo-apk was doing before, and the reason this is not just 'cargo build'."

# ── 1. Rust ─────────────────────────────────────────────────────────────────
# -o writes straight into the jniLibs layout Gradle expects:
# armeabi-v7a/libcarnyx.so, arm64-v8a/libcarnyx.so.
#
# The directory is wiped first. cargo-ndk copies in but never removes, so an ABI
# dropped from the command line would otherwise linger from a previous run and
# ship in an APK that no longer builds it.
say "1/3  cargo ndk — compiling libcarnyx.so for: ${ABIS[*]}"
rm -rf "$JNI_DIR"
mkdir -p "$JNI_DIR"
targets=()
for abi in "${ABIS[@]}"; do targets+=(-t "$abi"); done
cargo ndk "${targets[@]}" -o "$JNI_DIR" build --lib

for abi in "${ABIS[@]}"; do
  [[ -f "$JNI_DIR/$abi/libcarnyx.so" ]] \
    || die "cargo-ndk reported success but $JNI_DIR/$abi/libcarnyx.so is not there."
done

# ── 2. Gradle ───────────────────────────────────────────────────────────────
# The wrapper is checked in, so this needs no Gradle on the PATH — only a JDK 17
# or newer, which Android Studio's bundled jbr satisfies.
say "2/3  gradle — packaging the APK"
(cd android && ./gradlew --console=plain assembleDebug)

APK="android/app/build/outputs/apk/debug/app-debug.apk"
[[ -f "$APK" ]] || die "Gradle finished but there is no APK at $APK."

# ── 3. The pre-flight check ─────────────────────────────────────────────────
# There is no adb on this unit, so this is the last chance to catch a bad APK on
# a machine that can still tell you why. It checks the ABIs, the manifest, the
# signature, and — the one that matters most for a hand-written manifest — that
# android.app.lib_name names a library the APK actually contains.
say "3/3  checking it before it goes on the stick"
./tools/check-apk.sh "$APK"
