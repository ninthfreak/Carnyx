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
#
# THAT SERVICE IS NOW REAL, AND ONLY THIS SCRIPT PRODUCES IT. The APK this builds
# declares <service android:name=".CarnyxService"> and carries the class AGP
# compiles from android/app/src/main/java/. An APK from `cargo apk` has neither,
# and the app detects that at run time — it logs "no foreground service on this
# build" and carries on. So the two builds are no longer interchangeable: use
# this one on the unit unless you are specifically testing the other.
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
#
# AND FALL BACK TO THE DEFAULT INSTALL PATH, because the OTHER build already
# does. `cargo apk` reaches the SDK through the `android-build` crate, whose
# order is ANDROID_HOME, then ANDROID_SDK_ROOT, then the platform's default
# location (env_paths/mod.rs:41-56, find_android_sdk.rs) — so a stock Android
# Studio install builds with cargo-apk and no environment variables at all. This
# script used to refuse that machine, which made "it worked last time" true and
# "it works now" false for no reason the user could see. Same order, same paths.
case "$(uname -s)" in
  Darwin) DEFAULT_SDK="$HOME/Library/Android/sdk" ;;
  *)      DEFAULT_SDK="$HOME/Android/Sdk" ;;
esac

# A VARIABLE THAT IS SET AND WRONG IS AN ERROR, NOT A REASON TO LOOK ELSEWHERE.
# Falling through to the default path there would build against an SDK the user
# did not name while telling them nothing — so the fallback applies only when
# nothing is set at all, and a typo stops the build against its own value.
SDK=""
SDK_FROM=""
if [[ -n "${ANDROID_HOME:-}" ]]; then
  [[ -d "$ANDROID_HOME" ]] || die "\$ANDROID_HOME is set to $ANDROID_HOME, which is not a directory.

Fix it or unset it — unset, this script looks in $DEFAULT_SDK."
  SDK="$ANDROID_HOME"; SDK_FROM="\$ANDROID_HOME"
elif [[ -n "${ANDROID_SDK_ROOT:-}" ]]; then
  [[ -d "$ANDROID_SDK_ROOT" ]] || die "\$ANDROID_SDK_ROOT is set to $ANDROID_SDK_ROOT, which is not a directory.

Fix it or unset it — unset, this script looks in $DEFAULT_SDK."
  SDK="$ANDROID_SDK_ROOT"; SDK_FROM="\$ANDROID_SDK_ROOT"
elif [[ -d "$DEFAULT_SDK" ]]; then
  SDK="$DEFAULT_SDK"; SDK_FROM="the default path"
fi

[[ -n "$SDK" ]] || die "No Android SDK.

Looked at, in this order:
  \$ANDROID_HOME       ${ANDROID_HOME:-(not set)}
  \$ANDROID_SDK_ROOT   ${ANDROID_SDK_ROOT:-(not set)}
  the default path    $DEFAULT_SDK (not a directory)

If yours is somewhere else, point at it:
  export ANDROID_HOME=/path/to/Sdk

See the README's Building section."

printf 'SDK:         %s (from %s)\n' "$SDK" "$SDK_FROM"
export ANDROID_HOME="$SDK"
export ANDROID_SDK_ROOT="$SDK"
# ── android.jar, PINNED rather than left to be resolved ─────────────────────
#
# Slint's Android backend compiles one Java helper of its own, and Carnyx's
# build.rs compiles the NWD bridge — both need android.jar, and both locate it
# through android-build, whose order is (env_paths/mod.rs:70-92):
#
#   1. ANDROID_JAR, if set and the file exists — WINS OUTRIGHT.
#   2. else $SDK/platforms/<V>/android.jar, <V> from ANDROID_PLATFORM /
#      ANDROID_API_LEVEL / ANDROID_SDK_VERSION.
#   3. else the highest installed platform.
#
# STEP 2 IS A PIN, NOT A PREFERENCE: when it is set, step 3 never runs, so one
# value naming a platform you do not have makes every platform you DO have
# invisible — and the panic still reads "No Android platforms found", which
# sounds like "install one" to someone with several installed.
#
# AND THE PIN IS NOT ALWAYS YOURS. cargo-ndk passes its own platform level down
# to the cargo it invokes, so unsetting ANDROID_PLATFORM in the shell does not
# clear it: the build script still sees one, set to whatever API level the
# cross-compile is targeting. That level is a NDK/ABI concern and has nothing to
# do with which android.jar you have installed — 26 is right for the .so and
# wrong for the jar unless you happen to have platforms;android-26.
#
# So this does not try to out-argue step 2. It resolves a jar that exists and
# exports ANDROID_JAR, which outranks all of it.
compile_sdk=$(sed -n 's/^ *compileSdk = \([0-9][0-9]*\).*/\1/p' android/app/build.gradle.kts | head -1)
shopt -s nullglob
installed=("$SDK"/platforms/*/android.jar)
shopt -u nullglob
have=""
for j in "${installed[@]}"; do have="$have $(basename "$(dirname "$j")")"; done

if [[ -n "${ANDROID_JAR:-}" && -f "${ANDROID_JAR:-}" ]]; then
  printf 'android.jar:%s (from your ANDROID_JAR)\n' " $ANDROID_JAR"

elif [[ ${#installed[@]} -eq 0 ]]; then
  die "No Android platform in $SDK/platforms.

The NDK and build-tools are not enough: Slint's Android backend and Carnyx's own
build.rs each compile Java against android.jar, and there is none to compile
against.

Install one — $compile_sdk matches compileSdk in android/app/build.gradle.kts:
  sdkmanager 'platforms;android-$compile_sdk'

or in Android Studio: SDK Manager → SDK Platforms.

Already have one elsewhere? Point at it directly:
  export ANDROID_JAR=/path/to/android.jar"

elif [[ -f "$SDK/platforms/android-$compile_sdk/android.jar" ]]; then
  # Preferred, because it is the same level Gradle compiles the Java against.
  export ANDROID_JAR="$SDK/platforms/android-$compile_sdk/android.jar"
  printf 'android.jar: android-%s, matching compileSdk (installed:%s)\n' "$compile_sdk" "$have"

else
  # Highest installed, by VERSION order — a plain lexicographic max would put
  # android-9 above android-34, and "36.1" needs a numeric-aware sort too.
  best=$(for j in "${installed[@]}"; do basename "$(dirname "$j")"; done \
    | sed 's/^android-//' | sort -V | tail -1)
  export ANDROID_JAR="$SDK/platforms/android-$best/android.jar"
  printf 'android.jar: android-%s, highest installed (compileSdk %s is NOT installed; installed:%s)\n' \
    "$best" "$compile_sdk" "$have"
fi

# THE NDK, and the same fallback for the same reason. Android Studio installs it
# under the SDK as ndk/<version> (older setups: ndk-bundle), so a machine with a
# working NDK and no ANDROID_NDK_ROOT is an ordinary machine, not a broken one.
# Highest version wins, by VERSION order — `sort -V`, because a lexicographic max
# puts 26.1.10909125 below 9.0.0.
NDK="${ANDROID_NDK_ROOT:-${ANDROID_NDK_HOME:-${ANDROID_NDK:-}}}"
NDK_FROM="\$ANDROID_NDK_ROOT"
if [[ -z "$NDK" ]]; then
  shopt -s nullglob
  ndks=("$SDK"/ndk/*/)
  shopt -u nullglob
  if [[ ${#ndks[@]} -gt 0 ]]; then
    best_ndk=$(for d in "${ndks[@]}"; do basename "${d%/}"; done | sort -V | tail -1)
    NDK="$SDK/ndk/$best_ndk"
    NDK_FROM="the SDK's ndk/ directory"
  elif [[ -d "$SDK/ndk-bundle" ]]; then
    NDK="$SDK/ndk-bundle"
    NDK_FROM="the SDK's ndk-bundle"
  fi
fi

[[ -n "$NDK" ]] || die "No NDK.

Looked at \$ANDROID_NDK_ROOT, \$ANDROID_NDK_HOME, \$ANDROID_NDK, then
$SDK/ndk/<version> and $SDK/ndk-bundle.

Install one — in Android Studio: SDK Manager -> SDK Tools -> NDK (Side by side),
or:
  sdkmanager --install 'ndk;27.0.12077973'

Then either let this script find it under the SDK, or point at it:
  export ANDROID_NDK_ROOT=/path/to/ndk/<version>"

[[ -d "$NDK" ]] || die "The NDK is set to $NDK, which is not a directory.

Fix it or unset it — unset, this script looks under $SDK/ndk."
printf 'NDK:         %s (from %s)\n' "$NDK" "$NDK_FROM"
export ANDROID_NDK_HOME="$NDK"
export ANDROID_NDK="$NDK"

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

# ── 2. Strip, the way cargo-apk's `strip = "split"` did ─────────────────────
#
# MEASURED, not theorised: the first Gradle APK came out at 172.2 MB against
# cargo-apk's 86.4 MB — exactly double — because nothing was stripping the
# libraries. Skia is compiled -g -gdwarf-2 and statically linked into this one
# library, so unstripped it carries the DWARF for 1158 Skia translation units,
# twice over for two ABIs. That debug info is the entire difference.
#
# `strip = "split"` in Cargo.toml is a [package.metadata.android] key, which is
# cargo-apk's alone; Gradle never reads that table. AGP does strip native
# libraries itself, but only when it can locate an NDK, and it degrades to a
# warning rather than an error when it cannot — so relying on it means the size
# silently doubles again the day the NDK moves. Doing it here is deterministic
# and needs no NDK version pinned in a tracked file.
#
# SPLIT, not discard, matching what cargo-apk did: the symbols are written
# beside the build so a native crash from the unit can still be symbolicated.
#
# --strip-unneeded, and the flag matters more than it looks.
#
# The first cut used --strip-debug, which removes the DWARF and stops there,
# leaving .symtab — the STATIC symbol table — in place. With Skia linked
# statically that table is a large share of the library, and the measured result
# was libraries still at 95.0 MB (arm64) and 78.9 MB (armv7) with every .debug_*
# section already gone. Stripping had run and the file was still fat.
#
# --strip-unneeded removes both, and is what cargo-apk's plain `strip` was doing
# all along. Verified rather than assumed, on a shared library exporting
# android_main: --strip-debug left .symtab present, --strip-unneeded removed it,
# and android_main survived in .dynsym under both.
#
# NEVER --strip-all: this library is reached through System.loadLibrary and JNI,
# so .dynsym has to survive. --strip-unneeded is precisely the flag that means
# "everything not needed for that".
say "2/4  stripping debug symbols out of the libraries"
strip_tool=$(echo "$NDK"/toolchains/llvm/prebuilt/*/bin/llvm-strip | cut -d' ' -f1)
objcopy_tool=$(echo "$NDK"/toolchains/llvm/prebuilt/*/bin/llvm-objcopy | cut -d' ' -f1)
if [[ ! -x "$strip_tool" || ! -x "$objcopy_tool" ]]; then
  die "No llvm-strip/llvm-objcopy under $NDK/toolchains/llvm/prebuilt/*/bin.

Without them the APK ships unstripped and is roughly twice the size it should
be — 172 MB against 86 MB when this was measured."
fi
# NOT under build/outputs/. The extracted debug info is roughly the size of the
# libraries it came out of, and putting it beside the APK made "how big is the
# output" an ambiguous question — the directory total and the APK differ by a
# hundred megabytes and neither is wrong.
SYMBOLS="android/app/build/symbols"
rm -rf "$SYMBOLS"
for abi in "${ABIS[@]}"; do
  so="$JNI_DIR/$abi/libcarnyx.so"
  mkdir -p "$SYMBOLS/$abi"
  before=$(stat -c%s "$so")
  "$objcopy_tool" --only-keep-debug "$so" "$SYMBOLS/$abi/libcarnyx.so.debug"
  "$strip_tool" --strip-unneeded "$so"
  after=$(stat -c%s "$so")
  printf '     %-12s %6.1f MB → %5.1f MB   (symbols kept in %s)\n' \
    "$abi" "$(echo "$before" | awk '{print $1/1048576}')" \
    "$(echo "$after" | awk '{print $1/1048576}')" "$SYMBOLS/$abi"
done

# ── 3. Gradle ───────────────────────────────────────────────────────────────
# The wrapper is checked in, so this needs no Gradle on the PATH — only a JDK 17
# or newer, which Android Studio's bundled jbr satisfies.
say "3/4  gradle — packaging the APK"

# DELETE THE OLD APK FIRST, and this is not tidiness.
#
# AGP packages incrementally: it UPDATES the existing APK in place rather than
# writing a fresh zip. That is normally a speed win, but when an entry shrinks a
# lot the obsolete bytes stay in the file as dead space — the central directory
# points only at the live entries, so the archive reads as correct and every tool
# that lists it agrees, while the file on disk stays the old size.
#
# Measured here: after stripping took the libraries from 95.0/78.9 MB down to
# 46.1/28.9, `unzip -v` totalled about 30 MB of entries and the file was still
# 172.2 MB, with a current timestamp because it really was being rewritten.
# Rebuilding did not shift it. Deleting it and rebuilding did.
#
# Packaging is seconds against a build measured in Skia, so a fresh zip every
# time is free.
APK="android/app/build/outputs/apk/debug/app-debug.apk"
rm -f "$APK"

(cd android && ./gradlew --console=plain assembleDebug)

[[ -f "$APK" ]] || die "Gradle finished but there is no APK at $APK."

# ── 4. The pre-flight check ─────────────────────────────────────────────────
# There is no adb on this unit, so this is the last chance to catch a bad APK on
# a machine that can still tell you why. It checks the ABIs, the manifest, the
# signature and its certificate, and — the one that matters most for a
# hand-written manifest — that android.app.lib_name names a library the APK
# actually contains.
say "4/4  checking it before it goes on the stick"
./tools/check-apk.sh "$APK"
