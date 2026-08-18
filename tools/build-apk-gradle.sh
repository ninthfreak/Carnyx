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

# ── The environment, and why it takes four names for two directories ────────
# cargo-ndk reads ANDROID_NDK_HOME. skia-bindings reads ANDROID_NDK and nothing
# else (build_support/platform/android.rs:69) and panics without it. The SDK is
# ANDROID_HOME to some tools and ANDROID_SDK_ROOT to others. So: accept whatever
# is set, export all of them, and let no tool downstream go looking. This is the
# same trap the README documents for the cargo-apk build; changing packagers does
# not fix it, because half of it is skia's.
#
# ANDROID_SDK_ROOT is the newer name and plenty of setups export only that one.
# Accept either, and export both, rather than refusing a machine that is set up
# correctly under the other spelling.
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[[ -n "$SDK" ]] || die "Neither ANDROID_HOME nor ANDROID_SDK_ROOT is set — see the README's Building section."
[[ -d "$SDK" ]] || die "The SDK path points at $SDK, which does not exist."
export ANDROID_HOME="$SDK"
export ANDROID_SDK_ROOT="$SDK"
# ── android.jar, resolved THE SAME WAY the build resolves it ────────────────
#
# Checked here because otherwise it surfaces 150 crates and several minutes into
# the build, as a panic from a dependency's build script that names none of this:
#
#   i-slint-backend-android-activity build.rs:34: No Android platforms found
#
# Slint's Android backend compiles one Java helper of its own, and Carnyx's
# build.rs compiles the NWD bridge — both need android.jar.
#
# THIS MIRRORS android-build-0.1.4 src/env_paths/mod.rs:70-92 STEP FOR STEP, and
# it has to: the first version of this check globbed for any installed platform,
# passed, and the build died anyway. A preflight that resolves differently from
# the build is worse than no preflight at all.
#
#   1. ANDROID_JAR, if set and the file exists — wins outright, nothing else is
#      consulted.
#   2. Otherwise $SDK/platforms/<V>/android.jar, where <V> is the FIRST SET of
#      ANDROID_PLATFORM, ANDROID_API_LEVEL, ANDROID_SDK_VERSION — prefixed
#      "android-" if it is a bare number, and suffixed "-ext<N>" from
#      ANDROID_SDK_EXTENSION.
#   3. ONLY IF none of those three is set: the highest installed platform.
#
# STEP 2 IS THE TRAP. Setting ANDROID_PLATFORM PINS the lookup — step 3 never
# runs — so one stale value makes every platform you have invisible and the error
# still says "No Android platforms found", which reads as "install one" when you
# already have several.
platform_pin="${ANDROID_PLATFORM:-${ANDROID_API_LEVEL:-${ANDROID_SDK_VERSION:-}}}"
shopt -s nullglob
installed=("$SDK"/platforms/*/android.jar)
shopt -u nullglob
have=""
for j in "${installed[@]}"; do have="$have $(basename "$(dirname "$j")")"; done
[[ -n "$have" ]] || have=" (none)"

if [[ -n "${ANDROID_JAR:-}" && -f "${ANDROID_JAR:-}" ]]; then
  printf 'android.jar: %s (pinned by ANDROID_JAR)\n' "$ANDROID_JAR"

elif [[ -n "$platform_pin" ]]; then
  want="$platform_pin"
  [[ "$want" == android-* ]] || want="android-$want"
  if [[ "$want" != *-ext* && -n "${ANDROID_SDK_EXTENSION:-}" ]]; then
    ext="${ANDROID_SDK_EXTENSION#-}"; ext="${ext#ext}"
    [[ -n "$ext" ]] && want="$want-ext$ext"
  fi
  if [[ -f "$SDK/platforms/$want/android.jar" ]]; then
    printf 'android.jar: %s (pinned to %s)\n' "$SDK/platforms/$want/android.jar" "$want"
  else
    die "ANDROID_PLATFORM pins the build to '$want', which is not installed.

  looking for: $SDK/platforms/$want/android.jar
  installed:  $have

This is a PIN, not a preference — with it set, nothing falls back to the highest
installed platform, so the platforms you do have are ignored and the build fails
with 'No Android platforms found' as though you had none.

Pick one:
  unset ANDROID_PLATFORM                    # use the highest installed
  export ANDROID_PLATFORM=android-34        # or name one from 'installed' above
  sdkmanager 'platforms;$want'              # or install the one it wants

ANDROID_SDK_EXTENSION is part of the name too, and is currently '${ANDROID_SDK_EXTENSION:-unset}'."
  fi

elif [[ ${#installed[@]} -eq 0 ]]; then
  die "No Android platform in $SDK/platforms.

The NDK and build-tools are not enough: Slint's Android backend and Carnyx's own
build.rs each compile Java against android.jar, and there is none to compile
against.

Install one — 34 matches compileSdk in android/app/build.gradle.kts:
  sdkmanager 'platforms;android-34'

or in Android Studio: SDK Manager → SDK Platforms → Android 14 (API 34).

Already have one elsewhere? Point at it directly:
  export ANDROID_JAR=/path/to/android.jar"

else
  printf 'android.jar: highest installed, from:%s\n' "$have"
fi

NDK="${ANDROID_NDK_ROOT:-${ANDROID_NDK_HOME:-${ANDROID_NDK:-}}}"
[[ -n "$NDK" ]] || die "No NDK: set ANDROID_NDK_ROOT (and ANDROID_NDK, which skia-bindings reads)."
export ANDROID_NDK_HOME="$NDK"
export ANDROID_NDK="$NDK"
[[ -d "$NDK" ]] || die "ANDROID_NDK_ROOT points at $NDK, which does not exist."

# ASK CARGO, NOT THE PATH.
#
# `cargo ndk` is a cargo SUBCOMMAND. What matters is whether cargo can dispatch
# to it, which is not the same question as whether a binary called `cargo-ndk` is
# visible to `command -v` in a non-interactive shell — cargo resolves subcommands
# through its own directory as well as PATH, and a script does not inherit an
# interactive shell's full setup. The first version of this check asked the wrong
# one and refused a machine that had cargo-ndk installed and working.
if ! ndk_version=$(cargo ndk --version 2>&1); then
  die "cargo ndk will not run here.

Install it with:
  cargo install cargo-ndk

It sets the cross-compiler, linker and sysroot for each ABI — the job cargo-apk
was doing before, and the reason this is not just 'cargo build'.

If it IS installed, this is a path problem rather than a missing tool:
  cargo             $(command -v cargo || echo '(not found)')
  cargo-ndk binary  $(command -v cargo-ndk || echo '(not on PATH)')
  cargo ndk said    ${ndk_version:-(no output)}"
fi

# ── THE API LEVEL, AND WHY IT IS NOT cargo-ndk's DEFAULT ────────────────────
#
# cargo-ndk defaults to API 21 and puts it in the target triple it hands cc-rs:
# --target=armv7a-linux-androideabi21, LAST on the compiler line, so it wins.
# skia-bindings meanwhile hardcodes 26 and passes -D__ANDROID_API__=26. The two
# halves then disagree and the build dies deep in libc++:
#
#   locale:776: error: 'strtof_l' is unavailable: introduced in Android 26
#
# because bionic marks strtof_l/strtod_l __INTRODUCED_IN(26) and the availability
# check reads the level in the TRIPLE, not the -D. Skia's own 258 GN targets
# build fine at 26 and link libskia.a; only the four binding .cpp files compiled
# by cc-rs get 21, so the failure arrives at the very end of a long build.
#
# This is the same disagreement Cargo.toml documents at min_sdk_version — it was
# hit once before at 23, under cargo-apk, and cargo-apk fixed it by deriving its
# CXXFLAGS from that key. cargo-ndk has no Cargo.toml metadata to read, so the
# level has to be passed explicitly.
#
# Read from Cargo.toml rather than written twice, and cross-checked against the
# Gradle minSdk, so the three cannot drift apart silently.
API=$(sed -n 's/^min_sdk_version = \([0-9][0-9]*\).*/\1/p' Cargo.toml | head -1)
[[ -n "$API" ]] || die "Could not read min_sdk_version from Cargo.toml."
gradle_min=$(sed -n 's/^ *minSdk = \([0-9][0-9]*\).*/\1/p' android/app/build.gradle.kts | head -1)
[[ "$gradle_min" == "$API" ]] || die "API level mismatch, and these must agree:
  Cargo.toml min_sdk_version   $API
  build.gradle.kts minSdk      ${gradle_min:-(not found)}"

# Verified against the installed cargo-ndk rather than assumed, because getting
# the flag name wrong costs a full Skia rebuild to discover.
if ! cargo ndk --help 2>&1 | grep -q -- '--platform'; then
  die "This cargo-ndk has no --platform flag, so the API level cannot be set:

  $ndk_version

Without it the bindings compile at cargo-ndk's default (21) while skia-bindings
compiles at 26, and the build fails in libc++ on strtof_l. Check 'cargo ndk
--help' for the equivalent flag on this version."
fi

# ── 1. Rust ─────────────────────────────────────────────────────────────────
# -o writes straight into the jniLibs layout Gradle expects:
# armeabi-v7a/libcarnyx.so, arm64-v8a/libcarnyx.so.
#
# The directory is wiped first. cargo-ndk copies in but never removes, so an ABI
# dropped from the command line would otherwise linger from a previous run and
# ship in an APK that no longer builds it.
say "1/3  cargo ndk — compiling libcarnyx.so for: ${ABIS[*]}"
printf '     %s\n     NDK %s\n     API %s (from Cargo.toml min_sdk_version)\n' \
  "$ndk_version" "$NDK" "$API"
rm -rf "$JNI_DIR"
mkdir -p "$JNI_DIR"
targets=()
for abi in "${ABIS[@]}"; do targets+=(-t "$abi"); done
cargo ndk "${targets[@]}" --platform "$API" -o "$JNI_DIR" build --lib

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
