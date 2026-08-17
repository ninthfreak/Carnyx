#!/usr/bin/env bash
# Check an APK before it goes on the flash drive.
#
# The head unit is loaded from a USB stick, not adb, so there is no `adb install`
# to print a real failure code and no logcat to read — the unit says "App not
# installed" and nothing else. This checks, on the machine that built it, the
# things that cause exactly that message.
#
#   ./tools/check-apk.sh                  # whichever of the two builds is there
#   ./tools/check-apk.sh path/to/some.apk
set -uo pipefail

# Two packagers produce an APK now. With no argument, take whichever exists —
# cargo-apk's first, since it is still the default build.
CARGO_APK="target/debug/apk/carnyx.apk"
GRADLE_APK="android/app/build/outputs/apk/debug/app-debug.apk"
if [[ $# -ge 1 ]]; then
  APK="$1"
elif [[ -f "$CARGO_APK" ]]; then
  APK="$CARGO_APK"
else
  APK="$GRADLE_APK"
fi
# Both, always. The unit is 32-bit ARM, so armeabi-v7a is what actually installs
# today — but the project ships both, and an APK quietly missing one is the kind
# of thing that is only noticed on different hardware, months later.
NEED_ABIS=(armeabi-v7a arm64-v8a)

if [[ ! -f "$APK" ]]; then
  echo "No APK at $APK — build one first:" >&2
  echo "  cargo apk build --lib          # the cargo-apk build" >&2
  echo "  ./tools/build-apk-gradle.sh    # the Gradle build" >&2
  exit 1
fi

echo "APK:  $APK"
echo "Size: $(du -h "$APK" | cut -f1)"
echo

fail=0
note() { printf '  %-4s %s\n' "$1" "$2"; }

# ── Native libraries ───────────────────────────────────────────────────────
# INSTALL_FAILED_NO_MATCHING_ABIS is the failure this catches, and it is the one
# a build for the wrong --target produces.
echo "Native libraries"
abis=$(unzip -l "$APK" 2>/dev/null | sed -n 's#.*\blib/\([^/]*\)/.*#\1#p' | sort -u)
if [[ -z "$abis" ]]; then
  note "FAIL" "no lib/ entries at all — the .so never made it in"
  fail=1
else
  while read -r abi; do note "" "$abi"; done <<< "$abis"
  for need in "${NEED_ABIS[@]}"; do
    if grep -qx "$need" <<< "$abis"; then
      note "OK" "$need present"
    elif [[ "$need" == "armeabi-v7a" ]]; then
      note "FAIL" "$need MISSING — this will not install on the unit"
      fail=1
    else
      note "FAIL" "$need MISSING — installs on the unit, but the build is short an ABI"
      fail=1
    fi
  done
fi
echo

# ── Manifest ───────────────────────────────────────────────────────────────
# android:exported must be declared on the launcher activity or Android 12+
# refuses the package outright (targetSdk >= 31).
echo "Manifest"
aapt=$(ls "${ANDROID_HOME:-$HOME/Android/Sdk}"/build-tools/*/aapt 2>/dev/null | sort -V | tail -1)
if [[ -z "$aapt" ]]; then
  note "SKIP" "no aapt found — cannot read the manifest back"
  note "" "(the same aapt cargo-apk needs; see the README)"
else
  badging=$("$aapt" dump badging "$APK" 2>/dev/null)
  if [[ -z "$badging" ]]; then
    note "FAIL" "aapt could not parse the APK"
    fail=1
  else
    grep -E "^(package|sdkVersion|targetSdkVersion|launchable-activity|native-code)" \
      <<< "$badging" | sed 's/^/       /'
    grep -q "^launchable-activity" <<< "$badging" \
      || { note "FAIL" "no launchable activity — nothing to tap"; fail=1; }
  fi
  tree=$("$aapt" dump xmltree "$APK" AndroidManifest.xml 2>/dev/null)
  if grep -q "android:exported" <<< "$tree"; then
    note "OK" "android:exported is declared"
  else
    note "FAIL" "android:exported MISSING — Android 12+ refuses to install"
    fail=1
  fi

  # ── android.app.lib_name ─────────────────────────────────────────────────
  # NativeActivity loads `lib<value>.so` and nothing tells you when the name is
  # wrong: the activity starts, finds no library, and the screen stays black.
  # There is no logcat here to ask why, so the name is cross-checked against the
  # libraries actually in the APK.
  #
  # cargo-apk derived this value from the crate; a Gradle manifest states it by
  # hand, which is the one thing the move to Gradle stopped generating for us.
  libname=$(awk '
    /"android\.app\.lib_name"/ { seen = 1; next }
    seen && /android:value/ {
      if (match($0, /="[^"]*"/)) {
        print substr($0, RSTART + 2, RLENGTH - 3)
      }
      exit
    }
  ' <<< "$tree")
  if [[ -z "$libname" ]]; then
    note "FAIL" "no android.app.lib_name meta-data — NativeActivity has no library to load"
    fail=1
  else
    note "" "android.app.lib_name = $libname"
    missing=""
    while read -r abi; do
      [[ -z "$abi" ]] && continue
      unzip -l "$APK" "lib/$abi/lib$libname.so" >/dev/null 2>&1 || missing="$missing $abi"
    done <<< "$abis"
    if [[ -z "$missing" ]]; then
      note "OK" "lib$libname.so present for every packaged ABI"
    else
      note "FAIL" "lib$libname.so MISSING for:$missing — black screen, no error on the unit"
      fail=1
    fi
  fi
fi
echo

# ── Signature ──────────────────────────────────────────────────────────────
echo "Signature"
apksigner=$(ls "${ANDROID_HOME:-$HOME/Android/Sdk}"/build-tools/*/apksigner 2>/dev/null | sort -V | tail -1)
if [[ -z "$apksigner" ]]; then
  note "SKIP" "no apksigner found"
elif "$apksigner" verify "$APK" >/dev/null 2>&1; then
  note "OK" "signed"
else
  note "FAIL" "not signed, or the signature does not verify"
  fail=1
fi
echo

# ── What to copy ───────────────────────────────────────────────────────────
if [[ $fail -eq 0 ]]; then
  echo "Looks installable. Copy it across, and check it arrived whole:"
  echo "  md5sum $APK"
  echo "  # after copying, on the stick:"
  echo "  md5sum /media/\$USER/<stick>/$(basename "$APK")"
else
  echo "Do NOT carry this one out to the car — fix the FAILs above first."
fi
exit $fail
