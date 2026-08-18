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

# Two packagers produce an APK now, and both leave one on disk. With no
# argument, take the one BUILT MOST RECENTLY — that is the one just made and the
# one about to go on the stick.
#
# Preferring a fixed order was the first attempt and it was wrong: cargo-apk's
# APK sits in the tree from earlier builds, so checking "cargo-apk first" quietly
# reported on a stale artifact while the driver was asking about the new one. A
# checker that examines a different file from the one you are holding is worse
# than no checker.
CARGO_APK="target/debug/apk/carnyx.apk"
GRADLE_APK="android/app/build/outputs/apk/debug/app-debug.apk"
if [[ $# -ge 1 ]]; then
  APK="$1"
else
  APK=""
  newest=0
  for cand in "$CARGO_APK" "$GRADLE_APK"; do
    [[ -f "$cand" ]] || continue
    t=$(stat -c%Y "$cand" 2>/dev/null || echo 0)
    if [[ "$t" -gt "$newest" ]]; then newest="$t"; APK="$cand"; fi
  done
  [[ -n "$APK" ]] || APK="$GRADLE_APK"
  # Say which, and name the other, so a stale pick is visible rather than silent.
  for cand in "$CARGO_APK" "$GRADLE_APK"; do
    [[ -f "$cand" ]] || continue
    if [[ "$cand" == "$APK" ]]; then
      printf 'Checking  %s   (built %s — the newer)\n' \
        "$cand" "$(date -d "@$(stat -c%Y "$cand")" '+%Y-%m-%d %H:%M')"
    else
      printf 'Ignoring  %s   (built %s)\n' \
        "$cand" "$(date -d "@$(stat -c%Y "$cand")" '+%Y-%m-%d %H:%M')"
    fi
  done
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

# ── Where the bytes are ────────────────────────────────────────────────────
# Because "the APK is too big" has been answered by reasoning twice now and been
# wrong once. This prints the actual largest entries, stored and compressed, so
# the next question is settled by looking rather than by inference.
echo "Largest entries"
unzip -v "$APK" 2>/dev/null \
  | awk 'NR>3 && NF>=8 && $1 ~ /^[0-9]+$/ {
      name = $8; for (i = 9; i <= NF; i++) name = name " " $i
      printf "  %8.1f MB stored  %8.1f MB in APK   %s\n", $1/1048576, $3/1048576, name
    }' \
  | sort -rn | head -6
echo

# Debug sections are the difference between an 86 MB APK and a 172 MB one, and
# they are invisible from the outside — a stripped and an unstripped library look
# identical in a listing except for the number beside them. So look inside.
readelf=$(command -v llvm-readelf || command -v readelf || true)
if [[ -n "$readelf" ]]; then
  tmp=$(mktemp -d)
  if unzip -qo "$APK" 'lib/*/libcarnyx.so' -d "$tmp" 2>/dev/null; then
    while read -r so; do
      [[ -z "$so" ]] && continue
      abi=$(basename "$(dirname "$so")")
      sections=$("$readelf" -S "$so" 2>/dev/null)
      dwarf=$(grep -c '\.debug_' <<< "$sections")
      symtab=$(grep -c '\.symtab' <<< "$sections")
      dynsym=$(grep -c '\.dynsym' <<< "$sections")
      mb=$(stat -c%s "$so" | awk '{printf "%.1f", $1/1048576}')
      # BOTH tables, because checking only for DWARF was how an unstripped-enough
      # library passed once: every .debug_* gone, .symtab still there, and the
      # file still 95 MB.
      if [[ "$dwarf" -eq 0 && "$symtab" -eq 0 ]]; then
        note "OK" "$abi: fully stripped, ${mb} MB"
      elif [[ "$dwarf" -eq 0 ]]; then
        note "FAIL" "$abi: DWARF gone but .symtab remains — ${mb} MB, needs --strip-unneeded"
        fail=1
      else
        note "FAIL" "$abi: NOT stripped — $dwarf .debug_* sections, ${mb} MB"
        fail=1
      fi
      # The one thing stripping must never take: NativeActivity resolves the
      # library through this.
      if [[ "$dynsym" -eq 0 ]]; then
        note "FAIL" "$abi: .dynsym is GONE — nothing can load this library"
        fail=1
      fi
    done < <(find "$tmp" -name 'libcarnyx.so')
  fi
  rm -rf "$tmp"
else
  note "SKIP" "no readelf — cannot tell whether the library is stripped"
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
  # WHICH KEY, not just whether. Android refuses to install an APK over one
  # signed by a different certificate — INSTALL_FAILED_UPDATE_INCOMPATIBLE,
  # reported on the unit as "App not installed" and nothing else. Two packagers
  # build this app now, and they only interoperate while both sign with the same
  # debug key. Print the digest so two APKs can be compared without adb.
  certs=$("$apksigner" verify --print-certs "$APK" 2>/dev/null)
  sha=$(sed -n 's/.*certificate SHA-256 digest: *//p' <<< "$certs" | head -1)
  dn=$(sed -n 's/.*certificate DN: *//p' <<< "$certs" | head -1)
  [[ -n "$dn" ]] && note "" "signer: $dn"
  [[ -n "$sha" ]] && note "" "SHA-256: $sha"
  # Both cargo-apk and Gradle default to ~/.android/debug.keystore with the alias
  # androiddebugkey, so this is the expected debug identity for both.
  if [[ "$dn" == *"CN=Android Debug"* ]]; then
    note "OK" "the standard debug key — the other packager's APK will match"
  elif [[ -n "$dn" ]]; then
    note "" "(not the standard debug key; compare this digest against the APK"
    note "" " already on the unit before installing over it)"
  fi
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
