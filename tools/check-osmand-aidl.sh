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

# ── LOCAL FIRST: EVERY .aidl MUST RESOLVE WITHOUT THE SDK'S HELP ─────────────
#
# This section needs no network and ALWAYS runs. It re-implements the one rule
# our `aidl` invocation lives by — `-I java` and nothing else — and holds every
# .aidl in the tree to it: each import must resolve to a file on the include
# path, and every type a signature uses must be a builtin, an import, or a
# same-package neighbour.
#
# IT EXISTS BECAUSE THE CALLBACK SHIPPED UNRESOLVABLE. `onKeyEvent(in KeyEvent)`
# is upstream's line, and upstream compiles it with no import because Gradle
# passes the SDK's preprocessed framework.aidl (`-p`), where every framework
# parcelable is pre-declared. Our build.rs passes no `-p`, no container check
# ran aidl (there is no SDK here), and the first device build died in the aidl
# step. Framework types need a declaration under java/ (see
# java/android/view/KeyEvent.aidl) — and this is what notices the next one
# BEFORE a build on the unit's own machine does.
python3 - <<'LOCALPY'
import os, re, sys

ROOT = "java"
BUILTIN = {
    "void", "int", "long", "boolean", "float", "double", "byte", "char", "short",
    "String", "CharSequence", "List", "Map", "IBinder", "FileDescriptor",
    "in", "out", "inout", "oneway", "interface", "parcelable", "import", "package",
}
fail = []

aidls = []
for dirpath, _, names in os.walk(ROOT):
    for n in names:
        if n.endswith(".aidl"):
            aidls.append(os.path.join(dirpath, n))

for path in sorted(aidls):
    text = open(path, encoding="utf-8").read()
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"//.*", "", text)

    pkg = re.search(r"\bpackage\s+([\w.]+)\s*;", text)
    pkg = pkg.group(1) if pkg else ""
    if pkg and not path.startswith(os.path.join(ROOT, *pkg.split("."))):
        fail.append(f"{path}: declares package {pkg}, which is not its directory.")

    imports = re.findall(r"\bimport\s+([\w.]+)\s*;", text)
    by_simple = {}
    for imp in imports:
        f = os.path.join(ROOT, *imp.split(".")) + ".aidl"
        if not os.path.exists(f):
            fail.append(f"{path}: import {imp} has no {f} — aidl searches -I{ROOT} and nowhere else.")
        by_simple[imp.rsplit(".", 1)[-1]] = imp

    # Parcelable declarations have no signatures to scan.
    if re.search(r"\bparcelable\s+\w+\s*;", text):
        continue

    # TYPE POSITIONS ONLY — return types and parameter types. A first cut
    # scanned every capitalised identifier and flagged the NWD tree's
    # ALL-CAPS method names (`void AMS();`), which have built on the unit
    # for weeks. Method names are not resolved by aidl; types are.
    body = text[text.index("{"):] if "{" in text else ""
    used = set()
    for ret, _name, params in re.findall(r"([\w.<>\[\], ]+?)\s+(\w+)\s*\(([^)]*)\)\s*;", body):
        used.update(re.findall(r"[\w.]+", ret))
        for param in params.split(","):
            toks = param.split()
            while toks and toks[0] in ("in", "out", "inout"):
                toks.pop(0)
            if len(toks) >= 2:
                for t in toks[:-1]:
                    used.update(re.findall(r"[\w.]+", t))
    for t in sorted(used):
        simple = t.rsplit(".", 1)[-1]
        if simple in BUILTIN or simple in by_simple or not simple[:1].isupper():
            continue
        qualified = os.path.join(ROOT, *t.split(".")) + ".aidl" if "." in t else ""
        here = os.path.join(os.path.dirname(path), simple + ".aidl")
        if (qualified and os.path.exists(qualified)) or os.path.exists(here):
            continue
        fail.append(f"{path}: uses type {t}, which is not imported, not beside it, and not a builtin.")

if fail:
    for f in sorted(set(fail)):
        print("FAIL: " + f)
    sys.exit(1)
print(f"  local: {len(aidls)} .aidl files resolve with -I{ROOT} alone (framework types declared)")
LOCALPY
if [ $? -ne 0 ]; then
    echo
    echo "AIDL will not resolve on the machine that builds the APK." >&2
    exit 1
fi

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
fetch "$BASE/customization/PreferenceParams.java" "$TMP/prefparams.java" || true

# THE UNITS READ, which is the one preference Carnyx asks OsmAnd for and the
# reason §4.9's locale guess is now only a fallback. Three separate things have
# to stay true for it and each fails silently on its own: the preference's ID,
# the fact that it is a PROFILE preference (a global one is refused by
# `isExportAvailableForPref` and `getPreference` answers a bare `false`), and
# the six enum CONSTANT NAMES, which are what `EnumStringPreference` puts on the
# wire because its `toString` is `Enum.name()`.
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd/src/net/osmand/plus/settings/backend/OsmandSettings.java" \
    -o "$TMP/settings.java" 2>/dev/null || true
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd-shared/src/commonMain/kotlin/net/osmand/shared/settings/enums/MetricsConstants.kt" \
    -o "$TMP/metrics.kt" 2>/dev/null || true
# The turnInfo bundle's keys are built by string concatenation in OsmAnd's own
# writer, not declared in the api module — so they are checked against that file.
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd/src/net/osmand/plus/helpers/ExternalApiHelper.java" \
    -o "$TMP/ext.java" 2>/dev/null || true

# `next_turn_imminent` is the only trigger 4.9 permits for escalating the
# maneuver display, and its three values are NOT a rising scale: zero is the
# most urgent. `crate::nav::Stage` reads them; this is where that reading is
# held against the function that produces them.
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd/src/net/osmand/plus/routing/data/AnnounceTimeDistances.java" \
    -o "$TMP/atd.java" 2>/dev/null || true

# The V2 service's PERMISSION GATE, learned from a drive rather than a reading:
# every call goes through `getApi`, which checks the connected-apps list, and
# an unknown caller is added to it DISABLED and refused with -1s and nulls.
# CarnyxNav detects that state and the settings row tells the driver where the
# toggle is — copy that depends on all three of these files staying true.
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd/src/net/osmand/aidl/OsmandAidlServiceV2.java" \
    -o "$TMP/svc2.java" 2>/dev/null || true
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd/src/net/osmand/aidl/OsmandAidlApi.java" \
    -o "$TMP/api.java" 2>/dev/null || true
curl -sS --fail --max-time 45 \
    "https://raw.githubusercontent.com/osmandapp/OsmAnd/master/OsmAnd/src/net/osmand/plus/plugins/PluginsFragment.java" \
    -o "$TMP/plugins.java" 2>/dev/null || true

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

for name in ("registerForNavigationUpdates", "registerForVoiceRouterMessages", "getAppInfo",
             "getPreference"):
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
    # `preferenceId` IS NOT A TYPO FOR `prefId`. Upstream's field is `prefId`
    # and its bundle key is `preferenceId`; spelling the key after the field
    # hands OsmAnd a null id, which `getPreference` answers with a plain
    # `false` — a call that works and never finds anything.
    "prefparams.java": ("PreferenceParams", ["preferenceId", "appModeKey", "value"]),
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

# ── the units preference, which is the only one Carnyx reads ─────────────────
#
# THREE THINGS, EACH OF WHICH FAILS SILENTLY ALONE. A renamed id, a preference
# demoted from profile to global, or a renamed enum constant all end the same
# way: `metricSystem` stays 0, `Units::resolve` falls through to the locale, and
# the driver is back to reading kilometres while OsmAnd speaks miles — with
# nothing thrown and nothing logged, because "OsmAnd did not say" is a state
# this app is built to tolerate.
sett = os.path.join(tmp, "settings.java")
if os.path.exists(sett) and os.path.getsize(sett):
    text = open(sett, encoding="utf-8", errors="replace").read()
    m = re.search(r'METRIC_SYSTEM\s*=.*?"(\w+)".*?\.(makeProfile|makeGlobal)\(\)', text, re.S)
    if not m:
        fail.append("OsmandSettings.METRIC_SYSTEM is gone or reshaped — "
                    "CarnyxNav.readMetricSystem reads it by id.")
    elif m.group(1) != "default_metric_system":
        fail.append(f"the units preference is now id '{m.group(1)}', not "
                    f"'default_metric_system' — CarnyxNav asks for the old name.")
    elif m.group(2) != "makeProfile":
        fail.append("METRIC_SYSTEM is no longer a PROFILE preference. "
                    "isExportAvailableForPref refuses a global one, so "
                    "getPreference would answer false and units fall back.")
    else:
        print("  units: still id 'default_metric_system', still makeProfile "
              "(so getPreference's export gate lets it through)")

met = os.path.join(tmp, "metrics.kt")
if os.path.exists(met) and os.path.getsize(met):
    text = open(met, encoding="utf-8", errors="replace").read()
    # Declaration order IS the encoding CarnyxNav.encodeMetrics assigns 1..6,
    # but the wire value is the NAME, so what has to hold is the set of names
    # and the order our table was written from.
    want = ["KILOMETERS_AND_METERS", "MILES_AND_FEET", "MILES_AND_METERS",
            "MILES_AND_YARDS", "NAUTICAL_MILES_AND_METERS", "NAUTICAL_MILES_AND_FEET"]
    got = re.findall(r"^\t(\w+)\(", text, re.M)
    if got != want:
        fail.append(f"MetricsConstants changed.\n    upstream: {got}\n    ours:     {want}\n"
                    f"    CarnyxNav.encodeMetrics matches these names and "
                    f"units.rs::from_osmand maps the numbers.")
    else:
        print(f"  units: MetricsConstants still the same {len(want)} names in the same order")

# ── every parcelable in the poll's bundle must have a vendored class ─────────
#
# NOT because anything reads them — nothing does — but because Android 10's
# Bundle.unparcel() is ALL-OR-NOTHING: the first getter deserializes every
# value, through OUR classloader. AppInfoParams carried three ALatLon values,
# this tree had no ALatLon class, and every poll of a drive threw
# BadParcelableException into a logcat nobody can read, with the permission
# gate open and both subscriptions working. If upstream adds a parcelable to
# this bundle, the next build must vendor its class BEFORE a drive finds it.
appinfo = os.path.join(tmp, "appinfo.java")
if os.path.exists(appinfo) and os.path.getsize(appinfo):
    text = open(appinfo, encoding="utf-8", errors="replace").read()
    body = re.search(r"public void writeToBundle\s*\(.*?\n\t\}", text, re.S)
    if not body:
        fail.append("AppInfoParams.writeToBundle is gone or reshaped — the poll reads its bundle.")
    else:
        fields = re.findall(r"putParcelable(?:ArrayList)?\(\"\w+\",\s*(\w+)\)", body.group(0))
        types = set()
        for f in fields:
            m = re.search(r"private\s+(?:ArrayList<)?(\w+)>?\s+" + f + r"\s*;", text)
            types.add(m.group(1) if m else f"<type of {f} not found>")
        unvendored = []
        for t in sorted(types):
            hits = []
            for dirpath, _, names in os.walk(ours):
                if t + ".java" in names:
                    hits.append(dirpath)
            if not hits:
                unvendored.append(t)
        if unvendored:
            fail.append(f"AppInfoParams's bundle carries parcelable type(s) {unvendored} with no "
                        "vendored class — Android 10 unparcels EVERY value, and a missing class "
                        "is a BadParcelableException on every poll. Vendor the class (read "
                        "nothing from it); see map/ALatLon.java.")
        else:
            print(f"  poll bundle: parcelable types {sorted(types)} all have vendored classes")

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

# ── `imminent`: three values, and zero is the loudest ────────────────────────
#
# `crate::nav::Stage::from_imminent` maps 0 -> TurnNow, 1 -> Approach, anything
# else -> Cruise. That ordering is the trap: a reader who assumed the integer
# rose with urgency would put the hero takeover on the cruise state. The body of
# `getImminentTurnStatus` is four lines and this checks all of them.
atd = os.path.join(tmp, "atd.java")
if os.path.exists(atd):
    text = open(atd, encoding="utf-8", errors="replace").read()
    body = re.search(r"getImminentTurnStatus\s*\([^)]*\)\s*\{(.*?)\n\t\}", text, re.S)
    if not body:
        fail.append("AnnounceTimeDistances.getImminentTurnStatus is gone or reshaped — "
                    "nav::Stage reads its return values.")
    else:
        b = re.sub(r"//.*", "", body.group(1))
        want = [
            (r"isTurnStateActive\([^)]*STATE_TURN_NOW\)[^{]*\{\s*return\s+0\s*;",
             "STATE_TURN_NOW no longer returns 0 — Stage::TurnNow reads 0."),
            (r"isTurnStateActive\([^)]*STATE_PREPARE_TURN\)[^{]*\{\s*return\s+1\s*;",
             "STATE_PREPARE_TURN no longer returns 1 — Stage::Approach reads 1."),
            (r"else\s*\{\s*return\s+-1\s*;",
             "the cruising fall-through no longer returns -1 — Stage::Cruise reads it."),
        ]
        bad = [msg for pat, msg in want if not re.search(pat, b, re.S)]
        fail.extend(bad)
        # And nothing else may return from it: a fourth value would be a rung
        # this ladder does not have, and `from_imminent` would read it as Cruise.
        returns = sorted(set(re.findall(r"return\s+(-?\d+)\s*;", b)))
        if returns and returns != ["-1", "0", "1"]:
            fail.append(f"getImminentTurnStatus now returns {returns} — nav::Stage "
                        "knows only -1, 0 and 1.")
        if not bad:
            print("  imminent: still -1 cruise / 1 prepare / 0 turn-now "
                  "(zero is the most urgent)")

# ── the permission gate: refused means "toggle me in Plugins" ────────────────
#
# Three claims Carnyx makes to the DRIVER, each pinned to the line that makes
# it true. If any moves, the refused sub-line's instructions need re-checking
# before a driver follows them to a screen that no longer has the switch.
svc2 = os.path.join(tmp, "svc2.java")
if os.path.exists(svc2):
    text = open(svc2, encoding="utf-8", errors="replace").read()
    gate = re.search(r"private OsmandAidlApi getApi\(.*?\n\t\}", text, re.S)
    if not gate or "isAppEnabled" not in gate.group(0):
        fail.append("OsmandAidlServiceV2.getApi no longer gates on isAppEnabled — "
                    "the refused detection and its sub-line copy are built on that gate.")
    else:
        print("  gate: getApi still checks isAppEnabled(callingPackage) on every call")
api = os.path.join(tmp, "api.java")
if os.path.exists(api):
    text = open(api, encoding="utf-8", errors="replace").read()
    if not re.search(r"new ConnectedApp\(app, pack, false\)", text):
        fail.append("OsmandAidlApi.isAppEnabled no longer adds unknown callers DISABLED — "
                    "the 'Carnyx appears in the Plugins list switched off' claim depends on it.")
    else:
        print("  gate: unknown callers are still added to connected-apps DISABLED")
plugins = os.path.join(tmp, "plugins.java")
if os.path.exists(plugins):
    text = open(plugins, encoding="utf-8", errors="replace").read()
    if "ConnectedApp" not in text:
        fail.append("PluginsFragment no longer lists connected apps — the sub-line "
                    "sends the driver to the Plugins screen for the toggle.")
    else:
        print("  gate: the enable toggle still lives on the Plugins screen")

# ── arrivalTime IS SECONDS, NOT MILLIS — a drive reported a static ETA wrong
# by hours before this was caught. `AppInfoParams.arrivalTime` is named and
# javadoc'd like a millis field but upstream builds it from `getLeftTime()`
# (seconds) plus `currentTimeMillis() / 1000` — i.e. seconds — and
# `CarnyxNav.pollOnce` multiplies by 1000L to correct it at the JNI seam. If
# upstream ever starts sending real millis, that multiply doubles the error
# instead of fixing it, silently, with no compile-time signal — the same
# class of failure this whole script exists to catch positionally.
if os.path.exists(api):
    text = open(api, encoding="utf-8", errors="replace").read()
    # THERE ARE TWO `arrivalTime = ...;` ASSIGNMENTS — a `= 0` initializer and
    # the real formula further down — so every match is checked rather than
    # just the first; `re.search` alone found the initializer and passed.
    assigns = re.findall(r"arrivalTime\s*=\s*[^;]*;", text)
    formula = next((a for a in assigns if "leftTime" in a), None)
    # `/\s*1000` AND NOT JUST "1000" — a formula that MULTIPLIES by 1000
    # (`leftTime * 1000L + currentTimeMillis()`) contains the substring "1000"
    # too, but means the opposite thing: leftTime already converted to millis,
    # i.e. the field is already millis and CarnyxNav's `*1000L` would double
    # it. Caught in the standalone sabotage pass before this shipped.
    divides_by_1000 = bool(formula and re.search(r"/\s*1000\b", formula))
    if not assigns:
        fail.append("OsmandAidlApi no longer sets arrivalTime the way expected — "
                    "confirm its unit before trusting CarnyxNav's *1000L conversion.")
    elif not formula or not divides_by_1000:
        fail.append(
            "OsmandAidlApi.arrivalTime's formula changed shape "
            f"(found {assigns!r}) — it no longer visibly divides "
            "currentTimeMillis() by 1000, so it may already be millis. "
            "CarnyxNav.pollOnce's `* 1000L` would then double the error."
        )
    else:
        print("  eta: arrivalTime is still leftTime + currentTimeMillis()/1000 — seconds, "
              "which is why CarnyxNav.pollOnce multiplies by 1000L")

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
