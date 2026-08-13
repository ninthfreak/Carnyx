#!/usr/bin/env bash
# Check an APK before it goes on the flash drive.
#
# The head unit is loaded from a USB stick, not adb, so there is no `adb install`
# to print a real failure code and no logcat to read — the unit says "App not
# installed" and nothing else. This checks, on the machine that built it, the
# things that cause exactly that message.
#
#   ./tools/check-apk.sh                              # target/debug/apk/carnyx.apk
#   ./tools/check-apk.sh path/to/some.apk
set -uo pipefail

APK="${1:-target/debug/apk/carnyx.apk}"
# The unit is 32-bit ARM. An APK without this ABI cannot install on it.
NEED_ABI="armeabi-v7a"

if [[ ! -f "$APK" ]]; then
  echo "No APK at $APK — build one first:" >&2
  echo "  cargo apk build --lib --target armv7-linux-androideabi" >&2
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
  if grep -qx "$NEED_ABI" <<< "$abis"; then
    note "OK" "$NEED_ABI present"
  else
    note "FAIL" "$NEED_ABI MISSING — this will not install on the unit"
    fail=1
  fi
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
