#!/usr/bin/env bash
# Check our trimmed OsmAnd AIDL against the real one.
#
# ── THE HOLE THIS FILLS ───────────────────────────────────────────────────────
#
# A Binder transaction id is POSITIONAL. `aidl` numbers the methods it sees in
# declaration order from `FIRST_CALL_TRANSACTION`, the number is all that crosses
# the wire, and the method NAME travels nowhere. So
# `java/net/osmand/aidlapi/IOsmAndAidlInterface.aidl` is upstream's method list
# with ninety-seven of its ninety-nine payloads thrown away and the SLOTS kept —
# see that file's header for why the whole thing was not vendored.
#
# What that buys is nine files instead of about a hundred. What it costs is this
# script: if OsmAnd inserts a method ABOVE slot 65, every id below it shifts, our
# `registerForNavigationUpdates` starts dialling whatever now sits at 65, and
# NOTHING FAILS TO COMPILE. It would be a wrong call on a head unit with no adb —
# the same class of silent failure `tools/check-jni.sh` exists for, which is why
# this is a script and not a comment.
#
# Vendoring the real file would NOT avoid this: it has identical positional ids
# and would break the same way. The difference is that this is checked.
#
# ── WHAT IT CHECKS ────────────────────────────────────────────────────────────
#
#   1. upstream's interface still has the same METHOD COUNT as ours
#   2. `registerForNavigationUpdates` is still at the index our file gives it
#   3. `registerForVoiceRouterMessages` likewise
#   4. the CALLBACK's nine methods are still in the order we implement
#   5. the two payloads we actually read still carry the bundle keys we read
#
# It does NOT check that OsmAnd behaves, that the service binds, or that the
# version on the owner's unit matches master. Those need a device.
#
# ── NETWORK ───────────────────────────────────────────────────────────────────
#
# It fetches from raw.githubusercontent.com. With no network it says so and exits
# 0: a check that cannot run is not a failure, and turning an aeroplane into a
# red build would teach everyone to ignore it.
set -uo pipefail

cd "$(dirname "$0")/.."

BASE="https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd-api/src/net/osmand/aidlapi"
OURS="java/net/osmand/aidlapi"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fetch() {
    curl -sS --fail --max-time 45 "$1" -o "$2" 2>/dev/null
}

if ! fetch "$BASE/IOsmAndAidlInterface.aidl" "$TMP/iface.aidl"; then
    echo "cannot reach raw.githubusercontent.com — OsmAnd AIDL not checked."
    exit 0
fi
fetch "$BASE/IOsmAndAidlCallback.aidl" "$TMP/cb.aidl" || true
fetch "$BASE/navigation/ADirectionInfo.java" "$TMP/dir.java" || true
fetch "$BASE/navigation/OnVoiceNavigationParams.java" "$TMP/voice.java" || true
fetch "$BASE/info/AppInfoParams.java" "$TMP/appinfo.java" || true
# The turnInfo bundle's keys are built by string concatenation in OsmAnd's own
# writer, not declared in the api module — so they are checked against that file.
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd/src/net/osmand/plus/helpers/ExternalApiHelper.java" \
    -o "$TMP/ext.java" 2>/dev/null || true

python3 - "$TMP" "$OURS" <<'PY'
import os, re, sys

tmp, ours = sys.argv[1], sys.argv[2]
fail = []

def methods(text, iface):
    """Every method of `iface`, in declaration order — which is id order."""
    text = re.sub(r'/\*.*?\*/', '', text, flags=re.S)
    text = re.sub(r'//.*', '', text)
    i = text.index('interface ' + iface)
    body = text[i:]
    body = body[body.index('{') + 1:]
    return re.findall(
        r'(?:oneway\s+)?(?:[A-Za-z_][\w.<>\[\], ]*?)\s+(\w+)\s*\([^;]*?\)\s*;',
        body, flags=re.S)

# ── 1-3: the interface's shape ───────────────────────────────────────────────
up = methods(open(os.path.join(tmp, "iface.aidl")).read(), "IOsmAndAidlInterface")
mine = methods(open(os.path.join(ours, "IOsmAndAidlInterface.aidl")).read(), "IOsmAndAidlInterface")

if len(up) != len(mine):
    fail.append(f"method COUNT moved: upstream has {len(up)}, ours has {len(mine)}. "
                f"Every id past the insertion point has shifted.")
else:
    print(f"  interface: {len(mine)} slots, same as upstream")

for name in ("registerForNavigationUpdates", "registerForVoiceRouterMessages", "getAppInfo"):
    try:
        u = up.index(name)
    except ValueError:
        fail.append(f"upstream no longer declares {name} at all")
        continue
    try:
        m = mine.index(name)
    except ValueError:
        fail.append(f"our file no longer declares {name}")
        continue
    if u != m:
        fail.append(f"{name} MOVED: upstream slot {u}, ours {m}. "
                    f"Renumber our file or every call lands on the wrong method.")
    else:
        print(f"  {name}: slot {m}")

# ── 4: the callback's order, which OsmAnd dials ──────────────────────────────
cb = os.path.join(tmp, "cb.aidl")
if os.path.exists(cb) and os.path.getsize(cb):
    up_cb = methods(open(cb).read(), "IOsmAndAidlCallback")
    mine_cb = methods(open(os.path.join(ours, "IOsmAndAidlCallback.aidl")).read(), "IOsmAndAidlCallback")
    if up_cb != mine_cb:
        fail.append(f"callback order changed.\n    upstream: {up_cb}\n    ours:     {mine_cb}")
    else:
        print(f"  callback: {len(mine_cb)} methods, same order "
              f"(updateNavigationInfo at {mine_cb.index('updateNavigationInfo')})")

# ── 5: the bundle keys we read ───────────────────────────────────────────────
# The payloads cross as ONE Bundle — `AidlParams.writeToParcel` is
# `dest.writeBundle(...)` — so the contract is the KEY NAMES, not a field order.
KEYS = {
    "dir.java": ("ADirectionInfo", ["distanceTo", "turnType", "isLeftSide"]),
    "voice.java": ("OnVoiceNavigationParams", ["cmds", "played"]),
    "appinfo.java": ("AppInfoParams", ["arrivalTime", "leftTime", "leftDistance",
                                       "mapVisible", "turnInfo"]),
}
for fname, (cls, keys) in KEYS.items():
    path = os.path.join(tmp, fname)
    if not (os.path.exists(path) and os.path.getsize(path)):
        continue
    text = open(path).read()
    missing = [k for k in keys if f'"{k}"' not in text]
    if missing:
        fail.append(f"{cls} no longer carries bundle key(s) {missing} — we read them.")
    else:
        print(f"  {cls}: bundle keys {keys} all present")

# ── 6: the turnInfo keys, and the prefix that is not what it looks like ──────
ext = os.path.join(tmp, "ext.java")
if os.path.exists(ext) and os.path.getsize(ext):
    text = open(ext).read()
    for const, value in [("PARAM_NT_DISTANCE", "turn_distance"),
                         ("PARAM_NT_IMMINENT", "turn_imminent"),
                         ("PARAM_NT_DIRECTION_NAME", "turn_name"),
                         ("PARAM_NT_DIRECTION_TURN", "turn_type")]:
        if f'{const} = "{value}"' not in text:
            fail.append(f"turnInfo key {const} is no longer \"{value}\" — CarnyxNav reads it by name.")
    # THE AFTER-NEXT PREFIX HAS NO TRAILING UNDERSCORE and the next one does, so
    # the real keys are `next_turn_name` and `after_nextturn_name`. If upstream
    # ever tidies that, our reads go null and the THEN block silently empties.
    if 'updateTurnInfo("next_"' not in text:
        fail.append('the next-turn prefix is no longer "next_".')
    if 'updateTurnInfo("after_next"' not in text:
        fail.append('the after-next prefix is no longer "after_next" (no trailing '
                    'underscore) — CarnyxNav reads `after_nextturn_name`.')
    if not [f for f in fail if "prefix" in f or "turnInfo key" in f]:
        print("  turnInfo: keys and both prefixes unchanged "
              "(after-next still has no trailing underscore)")

if fail:
    print()
    for f in fail:
        print("FAIL: " + f)
    sys.exit(1)
PY
status=$?

echo
if [ $status -eq 0 ]; then
    echo "OsmAnd AIDL matches upstream where it has to."
else
    echo "OsmAnd AIDL has DRIFTED. See java/net/osmand/aidlapi/IOsmAndAidlInterface.aidl." >&2
fi
exit $status
