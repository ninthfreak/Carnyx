#!/usr/bin/env bash
# Run tools/source-remap.sh against a FAKE adb and a fake head unit, on the
# host, through every branch it has.
#
#     bash tools/test-source-remap.sh            VERBOSE=1 prints what each run said
#
# The fake is test-radio-takeover.sh's — same rules: writes into <config> are
# refused unless the fake adbd is root and the mount read-write, and failures
# come with adb's own words — extended with what `source-remap.sh` asks that
# radio-takeover.sh never did: `pm path` and `pm list packages` for any app, and
# `cmd package query-activities` / `resolve-activity`, answered in the shape
# Android 10 prints.
#
# WHAT IT CANNOT SAY is anything about the real unit: what its `cmd` prints,
# whether the kernel honours a remap for any source but the radio, and how
# `<config>` is mounted there.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
SCRIPT=$PWD/tools/source-remap.sh
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad() {
    FAIL=$((FAIL + 1))
    printf '  FAIL  %s\n' "$1"
    [ -f "${S:-}/out" ] && sed 's/^/          | /' "$S/out"
}
check() { if "${@:2}"; then ok "$1"; else bad "$1"; fi; }

# ── The fake adb ─────────────────────────────────────────────────────────────

mkdir -p "$WORK/bin"
cat > "$WORK/bin/adb" <<'MOCK'
#!/usr/bin/env bash
S=${MOCK_STATE:?}
get() { cat "$S/$1" 2>/dev/null; }
put() { printf '%s' "$2" > "$S/$1"; }
serial=""
if [ "${1:-}" = -s ]; then serial=$2; shift 2; fi
cmd=${1:-}
shift || true
cfg=$(get cfg)
under_cfg() { case "$1" in "$cfg" | "$cfg"/*) return 0 ;; esac; return 1; }
# `refuse` names a target (or "all") that nothing answers for.
refused() { [ "$(get refuse)" = all ] || [ "$(get refuse)" = "$1" ]; }
case "$cmd" in
    connect)
        if refused "$1"; then echo "failed to connect to '$1': Connection refused"; exit 1; fi
        echo "connected to $1" ;;
    disconnect) ;;
    get-state)
        if refused "$serial"; then echo "error: device '$serial' not found" >&2; exit 1; fi
        case "$(get auth)" in
            device) echo device ;;
            unauthorized)
                printf '%s\n' "error: device unauthorized." \
                    "This adb server's \$ADB_VENDOR_KEYS is not set" \
                    "Try 'adb kill-server' if that seems wrong." \
                    "Otherwise check for a confirmation dialog on your device." >&2
                exit 1 ;;
            *) echo "error: device $(get auth)" >&2; exit 1 ;;
        esac ;;
    root)
        if [ "$(get rootable)" = 1 ]; then put root 1; echo "restarting adbd as root"
        else echo "adbd cannot run as root in production builds"; fi ;;
    unroot) put root 0; echo "restarting adbd as non root" ;;
    pull)
        if under_cfg "$1" && [ "$(get noread)" = 1 ] && [ "$(get root)" != 1 ]; then
            echo "adb: error: failed to stat remote object '$1': Permission denied" >&2; exit 1
        fi
        cp "$1" "$2" 2>/dev/null || { echo "adb: error: remote object '$1' does not exist" >&2; exit 1; } ;;
    push)
        if under_cfg "$2" && { [ "$(get root)" != 1 ] || [ "$(get ro)" = 1 ]; }; then
            echo "adb: error: failed to copy '$1' to '$2': Read-only file system" >&2; exit 1
        fi
        cp "$1" "$2" ;;
    version) echo "Android Debug Bridge version 1.0.41" ;;
    shell) MOCK_CFG=$cfg MOCK_SERIAL=$serial bash -c "$(cat "$S/preamble")
$1" ;;
    *) echo "fake adb: unhandled '$cmd'" >&2; exit 1 ;;
esac
MOCK
chmod +x "$WORK/bin/adb"

# The device side: what a command run through `adb shell` meets.
preamble() {
    cat <<EOF
S='$1'
EOF
    cat <<'EOF'
get() { command cat "$S/$1" 2>/dev/null; }
put() { printf '%s' "$2" > "$S/$1"; }
guard() {
    local a
    for a in "$@"; do
        case "$a" in "$MOCK_CFG" | "$MOCK_CFG"/*)
            [ "$(get root)" = 1 ] || { echo "$a: Permission denied" >&2; return 1; }
            [ "$(get ro)" != 1 ] || { echo "$a: Read-only file system" >&2; return 1; } ;;
        esac
    done
}
touch() { guard "$@" || return 1; command touch "$@"; }
mkdir() { guard "$@" || return 1; command mkdir "$@"; }
rm() { guard "$@" || return 1; command rm "$@"; }
chmod() { guard "$@" || return 1; command chmod "$@"; }
cp() { guard "${@: -1}" || return 1; command cp "$@"; }
getprop() {
    case "$1" in
        ro.build.type) get type ;;
        ro.build.tags) echo test-keys ;;
        ro.debuggable) get debuggable ;;
        ro.nwd.config.path) get cfg ;;
    esac
}
id() { if [ "$(get root)" = 1 ]; then echo 0; else echo 2000; fi; }
# Packages come from "$S/pkgs": one "package/activity kind where" per line,
# kind app (MAIN/LAUNCHER), home (MAIN/HOME) or none, where user or system.
pm() {
    case "$1" in
        path)
            case "$2" in
                com.nwd.kernel)
                    if [ -z "$(get unit_serial)" ] || [ "$(get unit_serial)" = "$MOCK_SERIAL" ]; then
                        echo "package:/system/priv-app/NwdKernel/NwdKernel.apk"
                    fi ;;
                *) command awk -v p="$2" '{ split($1, a, "/") } a[1] == p { f = 1 } END { exit !f }' "$S/pkgs" \
                       && echo "package:/data/app/$2-1/base.apk" ;;
            esac ;;
        list)
            if [ "${3:-}" = -3 ]; then
                command awk '$3 == "user" { split($1, a, "/"); print "package:" a[1] }' "$S/pkgs" | sort -u
            else
                command awk '{ split($1, a, "/"); print "package:" a[1] }' "$S/pkgs" | sort -u
            fi ;;
    esac
}
# `qa_broken`: an older `cmd` without query-activities.
cmd() {
    [ "$1" = package ] || return 1
    local c k w n=0 found=""
    case "$2" in
        query-activities)
            if [ "$(get qa_broken)" = 1 ]; then echo "Unknown command: query-activities"; return 1; fi
            local kind=app
            [ "${*: -1}" = android.intent.category.HOME ] && kind=home
            while read -r c k w; do
                [ "$k" = "$kind" ] || continue
                echo "  Activity #$n:"
                echo "    priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false"
                echo "    $c"
                n=$((n + 1))
            done < "$S/pkgs"
            [ "$n" = 0 ] && echo "No activities found" ;;
        resolve-activity)
            while read -r c k w; do
                [ "$k" = app ] && [ "${c%%/*}" = "${*: -1}" ] && { found=$c; break; }
            done < "$S/pkgs"
            if [ -n "$found" ]; then
                echo "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=true"
                echo "$found"
            else
                echo "No activity found"
            fi ;;
    esac
}
# `noread`: an unprivileged shell may not look into /config at all.
ls() {
    local a
    for a in "$@"; do
        case "$a" in "$MOCK_CFG"/*)
            if [ "$(get noread)" = 1 ] && [ "$(get root)" != 1 ]; then
                echo "ls: $a: Permission denied" >&2; return 1
            fi ;;
        esac
    done
    command ls "$@"
}
am() {
    if [ "$(get amfail)" = 1 ]; then echo "Error: Activity manager is not running" >&2; return 1; fi
    echo "$*" >> "$S/broadcasts"; echo "Broadcast completed: result=0"
}
# `stuck`: the remount says yes and changes nothing.
mount() {
    if [ "$1" = -o ]; then
        [ "$(get root)" = 1 ] || { echo "mount: Permission denied" >&2; return 1; }
        [ "$(get stuck)" = 1 ] && return 0
        case "$2" in rw,remount) put ro 0 ;; ro,remount) put ro 1 ;; esac
    fi
}
cat() {
    if [ "$1" = /proc/mounts ]; then
        echo "/dev/root / ext4 ro,seclabel,relatime 0 0"
        if [ "$(get ownmount)" = 1 ]; then
            if [ "$(get ro)" = 1 ]; then o=ro; else o=rw; fi
            echo "/dev/block/by-name/config $MOCK_CFG ext4 $o,seclabel,nosuid 0 0"
        fi
    else
        command cat "$@"
    fi
}
EOF
}

# A fresh head unit: `unit NAME key=value…`. Defaults are this unit's shape as
# far as it is known — /config its own read-only mount, no /config/app — on a
# build that allows root.
unit() {
    S=$WORK/$1
    shift
    command rm -rf "$S"
    mkdir -p "$S/unit/config"
    local kv
    for kv in rootable=1 root=0 ro=1 ownmount=1 auth=device carnyx=1 type=userdebug debuggable=1 \
              "cfg=$S/unit/config" "$@"; do
        printf '%s' "${kv#*=}" > "$S/${kv%%=*}"
    done
    preamble "$S" > "$S/preamble"
    : > "$S/broadcasts"
}

# `-y` answers the confirmation; `ask` below is the one run that does not.
# VERBOSE=1 prints what each run said, which is the text a driver will read.
run() {
    MOCK_STATE=$S ADB=$WORK/bin/adb CONNECT_PAUSE=0 ROOT_PAUSE=0 CONNECT_TRIES=3 \
        bash "$SCRIPT" "$@" -y > "$S/out" 2>&1 < /dev/null
    echo $? > "$S/rc"
    if [ "${VERBOSE:-0}" = 1 ]; then
        printf '    $ source-remap.sh %s\n' "$*"
        sed "s#$WORK/[^/]*/unit##g; s/^/    > /" "$S/out"
    fi
}
ask() {
    MOCK_STATE=$S ADB=$WORK/bin/adb CONNECT_PAUSE=0 ROOT_PAUSE=0 CONNECT_TRIES=3 \
        bash "$SCRIPT" "$@" > "$S/out" 2>&1 <<< "$ANSWER"
    echo $? > "$S/rc"
}

rc() { [ "$(cat "$S/rc")" = "$1" ]; }
said() { grep -qF -- "$1" "$S/out"; }
state() { [ "$(cat "$S/$1")" = "$2" ]; }
file() { printf '%s' "$S/unit/config/app/replace_source_list.xml"; }
backup() { printf '%s' "$S/unit/config/app/replace_source_list.xml.carnyx-backup"; }
has() { [ -f "$1" ] && grep -qF -- "$2" "$1"; }
items() { tr '\n' ' ' < "$1" | grep -o '<ReplaceSourceItem' | wc -l | tr -d ' '; }
reloaded() { grep -q 'com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE' "$S/broadcasts"; }
not() { ! "$@"; }

# ── This suite's own helpers ────────────────────────────────────────────────

# A unit with a standard set of apps. In the numbering `apps` gives them —
# user-installed first, then by package — 1 example.music, 2 example.video,
# 3 carnyx, 4 osmand, 5 musicbox, 6 settings, 7 nwd.launcher, 8 nwd.radio.
u() {
    unit "$@"
    cat > "$S/pkgs" <<'P'
com.ninthfreak.carnyx/android.app.NativeActivity app user
com.example.music/.Main app user
org.example.musicbox/org.example.musicbox.Player app user
com.example.video/com.example.video.Player app user
net.osmand.plus/net.osmand.plus.activities.MapActivity app user
com.example.nolauncher/com.example.nolauncher.Svc none user
com.android.settings/.Settings app system
com.nwd.radio/com.nwd.radio.home_horizontalActivity app system
com.nwd.launcher/com.nwd.launcher.Launcher home system
P
}
absent() { printf '%s' "$S/unit/config/app/replace_source_list.xml.carnyx-absent"; }
entry() { printf '<ReplaceSourceItem appid="%s" pkgName="%s" className="%s" />' "$1" "$2" "$3"; }
plain() { MOCK_STATE=$S ADB=$WORK/bin/adb CONNECT_PAUSE=0 ROOT_PAUSE=0 CONNECT_TRIES=3 \
    bash "$SCRIPT" "$@" > "$S/out" 2>&1 < /dev/null; echo $? > "$S/rc"; }
feed() { local input=$1; shift
    MOCK_STATE=$S ADB=$WORK/bin/adb CONNECT_PAUSE=0 ROOT_PAUSE=0 CONNECT_TRIES=3 \
        bash "$SCRIPT" "$@" > "$S/out" 2>&1 <<< "$input"; echo $? > "$S/rc"; }
MUSIC_BOX=$(entry 2 org.example.musicbox org.example.musicbox.Player)
MUSIC_EXAMPLE=$(entry 2 com.example.music com.example.music.Main)
VIDEO_EXAMPLE=$(entry 3 com.example.video com.example.video.Player)

echo "source-remap.sh against a fake head unit"

# ── sources, and arguments ───────────────────────────────────────────────────
S=$WORK/noadb; mkdir -p "$S"
ADB=/nonexistent bash "$SCRIPT" sources > "$S/out" 2>&1; echo $? > "$S/rc"
check "sources: needs no adb and exits 0" rc 0
check "sources: marks the radio as the tested one" said "radio             TESTED"
check "sources: marks the camera as needing --force" said "camera            NEEDS --force"

u args
plain set music musicbox 10.0.0.7 extra
check "an unexpected argument: exits 2" rc 2
plain set 10.0.0.7
check "set with no source: exits 2" rc 2
run set nonsense musicbox 10.0.0.7
check "an unknown source: exits 1" rc 1
check "an unknown source: says so" said "'nonsense' is not a source"
check "an unknown source: asks for no root" state root 0

# ── apps ─────────────────────────────────────────────────────────────────────
u apps
plain apps 10.0.0.7
check "apps: exits 0" rc 0
check "apps: user apps first, numbered" said "    1  com.example.music"
check "apps: musicbox is 5" said "    5  org.example.musicbox"
check "apps: system apps marked" said "    6  com.android.settings  [system]"
check "apps: the home screen marked" said "com.nwd.launcher  [system]  [home screen]"
check "apps: an app with nothing to launch is not offered" not said "nolauncher"
check "apps: asks for no root" state root 0

u apps-old qa_broken=1
plain apps 10.0.0.7
check "apps on an older cmd: says it is asking each package" said "asking each package"
check "apps on an older cmd: still lists them" said "    5  org.example.musicbox"

# ── set, fresh ───────────────────────────────────────────────────────────────
u fresh
run set music 5 10.0.0.7
check "set by number: exits 0" rc 0
check "set by number: writes that app's entry" has "$(file)" "$MUSIC_BOX"
check "set on no file: records that there was none" test -e "$(absent)"
check "set on no file: makes no backup" not test -e "$(backup)"
check "set: reloads" reloaded
check "set: reads back" said "Read back: music now opens org.example.musicbox"
check "set: remounts read-only again" state ro 1
check "set: drops root again" state root 0

run set music org.example.musicbox 10.0.0.7
check "set again: says it is already so" said "already opens org.example.musicbox"
check "set again: asks for no root" not said "Asking for adb root"

run set video video 10.0.0.7
check "set by search word: finds the one match" has "$(file)" "$VIDEO_EXAMPLE"
check "set video: says it does not leave the MODE cycle" said "does not take Video"
check "set video: the music entry stays" has "$(file)" "$MUSIC_BOX"

run set 2 com.example.music 10.0.0.7
check "set by appid and exact package: short activity name expanded" has "$(file)" "$MUSIC_EXAMPLE"
check "set: the old music entry is gone" not has "$(file)" "org.example.musicbox"
check "set: still one music entry" test "$(grep -c 'appid="2"' "$(file)")" = 1

run set gps net.osmand.plus/.activities.MapActivity 10.0.0.7
check "set by package/activity: written as given" has "$(file)" "$(entry 1 net.osmand.plus net.osmand.plus.activities.MapActivity)"

plain list 10.0.0.7
check "list: names the sources" said "music        (appid 2) -> com.example.music"
check "list: and the video one" said "video        (appid 3) -> com.example.video"
check "list: says the rest are stock" said "Every other source opens its stock app"

run set radio com.ninthfreak.carnyx 10.0.0.7
check "set radio to Carnyx: the release-on-sleep reminder" said "Stop the radio when Carnyx closes"
check "set radio to Carnyx: Carnyx's entry" has "$(file)" "$(entry 8 com.ninthfreak.carnyx android.app.NativeActivity)"

# ── choosing ─────────────────────────────────────────────────────────────────
u pick
feed $'5\ny' set music 10.0.0.7
check "picker: exits 0" rc 0
check "picker: shows the list" said "Which app should the music source open?"
check "picker: writes the chosen app" has "$(file)" "$MUSIC_BOX"

u pick-cancel
feed $'' set music 10.0.0.7
check "picker cancelled: says nothing changed" said "Nothing changed."
check "picker cancelled: writes nothing" not test -e "$S/unit/config/app"

u pick-no
feed $'n' set music musicbox 10.0.0.7
check "confirmation declined: writes nothing" not test -e "$S/unit/config/app"
check "confirmation declined: asks for no root" state root 0

u ambiguous
run set music music 10.0.0.7
check "two matches under -y: exits 1" rc 1
check "two matches under -y: lists them" said "org.example.musicbox"
check "two matches under -y: writes nothing" not test -e "$S/unit/config/app"
feed $'2\ny' set music music 10.0.0.7
check "two matches, picked: writes the second" has "$(file)" "$MUSIC_BOX"

u no-app
run set music 10.0.0.7
check "no app under -y: exits 1" rc 1
check "no app under -y: says nobody is here to pick" said "nobody is here to pick"

u missing
run set music com.not.there 10.0.0.7
check "app not installed: exits 1" rc 1
check "app not installed: says so" said "com.not.there is not installed"
check "app not installed: asks for no root" state root 0

u nolauncher
run set music com.example.nolauncher 10.0.0.7
check "app with no activity: exits 1" rc 1
check "app with no activity: says why" said "has no activity to launch"
run set music com.example.nolauncher/.Svc 10.0.0.7
check "app with no activity, activity given: written" has "$(file)" "$(entry 2 com.example.nolauncher com.example.nolauncher.Svc)"

u camera
run set camera musicbox 10.0.0.7
check "camera without --force: exits 1" rc 1
check "camera without --force: names the reverse camera" said "REVERSE camera"
check "camera without --force: writes nothing" not test -e "$S/unit/config/app"
run set reverse musicbox 10.0.0.7 --force
check "camera with --force: written" has "$(file)" "$(entry 13 org.example.musicbox org.example.musicbox.Player)"

# ── merging into a file that was already there ──────────────────────────────
u merge
mkdir -p "$S/unit/config/app"
cat > "$(file)" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<!-- vendor list. The example below is off:
     <ReplaceSourceItem appid="2" pkgName="com.commented.out" className="X" />
     and this comment mentions </ReplaceSourceList> too. -->
<ReplaceSourceList>
    <ReplaceSourceItem appid='2' pkgName='com.vendor.music' className='com.vendor.music.Main'/>
    <ReplaceSourceItem appid="15"
        pkgName="com.vendor.bt" className="B" />
</ReplaceSourceList>
XML
cp "$(file)" "$S/original.xml"
plain list 10.0.0.7
check "list: a commented-out entry is not read as one" not said "com.commented.out"
check "list: a split entry is read" said "bt-music     (appid 15) -> com.vendor.bt"
run set music musicbox 10.0.0.7
check "merge: exits 0" rc 0
check "merge: the vendor music entry is replaced" not grep -q "com.vendor.music" "$(file)"
check "merge: the commented-out example is untouched" has "$(file)" 'pkgName="com.commented.out"'
check "merge: the split Bluetooth entry is untouched" has "$(file)" 'pkgName="com.vendor.bt" className="B" />'
check "merge: inserted after the comment, before the real closing tag" \
    test "$(grep -n "$MUSIC_BOX" "$(file)" | cut -d: -f1)" = 8
check "merge: the original is backed up" cmp -s "$(backup)" "$S/original.xml"
run set video video 10.0.0.7
check "merge: a second change keeps the first backup" cmp -s "$(backup)" "$S/original.xml"

run reset video 10.0.0.7
check "reset one source: exits 0" rc 0
check "reset one source: its entry is gone" not has "$(file)" "com.example.video"
check "reset one source: the other change stays" has "$(file)" "$MUSIC_BOX"
check "reset one source: the backup stays" test -e "$(backup)"

run reset music 10.0.0.7
check "reset the last change: the original returns byte for byte" cmp -s "$(file)" "$S/original.xml"
check "reset the last change: the backup is gone" not test -e "$(backup)"
check "reset: reloads" reloaded
check "reset: read-only again, root dropped" test "$(cat "$S/ro")$(cat "$S/root")" = 10

u merge-all
mkdir -p "$S/unit/config/app"
printf '%s\n' '<ReplaceSourceList>' "    $(entry 8 com.other.radio R)" '</ReplaceSourceList>' > "$(file)"
cp "$(file)" "$S/original.xml"
run set radio carnyx 10.0.0.7
run set music musicbox 10.0.0.7
run reset all 10.0.0.7
check "reset all: the original returns byte for byte" cmp -s "$(file)" "$S/original.xml"
check "reset all: the backup is gone" not test -e "$(backup)"

# ── reset where this script wrote the file ───────────────────────────────────
u fresh-all
run set music musicbox 10.0.0.7
run set video video 10.0.0.7
run reset all 10.0.0.7
check "reset all of our own file: removes it" not test -e "$(file)"
check "reset all of our own file: removes the record" not test -e "$(absent)"

u fresh-one
run set music musicbox 10.0.0.7
run reset music 10.0.0.7
check "reset of the only entry: removes the file" not test -e "$(file)"
check "reset of the only entry: removes the record" not test -e "$(absent)"
run reset music 10.0.0.7
check "reset with no file: says the stock app is already in use" said "already opens its stock app"

u no-record
mkdir -p "$S/unit/config/app"
printf '%s\n' '<ReplaceSourceList>' "    $MUSIC_BOX" "    $VIDEO_EXAMPLE" '</ReplaceSourceList>' > "$(file)"
cp "$(file)" "$S/original.xml"
run reset all 10.0.0.7
check "reset all with no record of the original: exits 1" rc 1
check "reset all with no record: unchanged" cmp -s "$(file)" "$S/original.xml"
run reset video 10.0.0.7
check "reset one with no record: removes only that entry" not has "$(file)" "com.example.video"
check "reset one with no record: the other stays" has "$(file)" "$MUSIC_BOX"
run reset gps 10.0.0.7
check "reset of a source with no entry: says so" said "has no gps entry"

# ── shapes it will not edit ─────────────────────────────────────────────────
u split-target
mkdir -p "$S/unit/config/app"
printf '%s\n' '<ReplaceSourceList>' '  <ReplaceSourceItem appid="2"' '      pkgName="com.vendor.music" className="M" />' '</ReplaceSourceList>' > "$(file)"
cp "$(file)" "$S/original.xml"
run set music musicbox 10.0.0.7
check "the source's own entry split over lines: exits 1" rc 1
check "the source's own entry split over lines: unchanged" cmp -s "$(file)" "$S/original.xml"
check "the source's own entry split over lines: no backup" not test -e "$(backup)"
check "the source's own entry split over lines: read-only again" state ro 1

u duplicate
mkdir -p "$S/unit/config/app"
printf '%s\n' '<ReplaceSourceList>' "    $(entry 2 com.a A)" "    $(entry 2 com.b B)" '</ReplaceSourceList>' > "$(file)"
plain list 10.0.0.7
check "list: warns about two entries for one source" said "some source has two entries"
run set music musicbox 10.0.0.7
check "set over duplicates: one entry left" test "$(grep -c 'appid="2"' "$(file)")" = 1
check "set over duplicates: it is the new one" has "$(file)" "$MUSIC_BOX"

u unfamiliar
mkdir -p "$S/unit/config/app"
printf '%s\n' '<SomethingElse appid="2" />' > "$(file)"
cp "$(file)" "$S/original.xml"
run set music musicbox 10.0.0.7
check "unfamiliar file: exits 1" rc 1
check "unfamiliar file: unchanged" cmp -s "$(file)" "$S/original.xml"

u open-tag
mkdir -p "$S/unit/config/app"
printf '%s\n' '<ReplaceSourceList>' '  <ReplaceSourceItem appid="3" pkgName="v" className="V"></ReplaceSourceItem>' '</ReplaceSourceList>' > "$(file)"
cp "$(file)" "$S/original.xml"
run set music musicbox 10.0.0.7
check "an entry that is not self-closed: refused, unchanged" cmp -s "$(file)" "$S/original.xml"

# ── the unit's own refusals ─────────────────────────────────────────────────
u no-root type=user debuggable=0 rootable=0
run set music musicbox 10.0.0.7
check "root refused: exits 1" rc 1
check "root refused: prints adb's answer" said "| adbd cannot run as root in production builds"
check "root refused: writes nothing" not test -e "$S/unit/config/app"

u system-image ownmount=0
run set music musicbox 10.0.0.7
check "<config> in the system image: exits 1" rc 1
check "<config> in the system image: will not touch verity" said "disable-verity"

# The owner's unit: <config> is /data/nwdappconfig, a plain writable directory.
u data-dir ownmount=0 ro=0
run set music musicbox 10.0.0.7
check "plain writable directory: written" has "$(file)" "$MUSIC_BOX"
check "plain writable directory: no remount either way" not said "remount"
check "plain writable directory: left writable" state ro 0

u unreadable noread=1
plain list 10.0.0.7
check "unreadable <config>: list says it could not read" said "Could not read"
check "unreadable <config>: list does not call it absent" not said "absent"
run set music musicbox 10.0.0.7
check "unreadable <config>: set looks again as root and writes" has "$(file)" "$MUSIC_BOX"

u no-broadcast amfail=1
run set music musicbox 10.0.0.7
check "reload refused: still written" has "$(file)" "$MUSIC_BOX"
check "reload refused: prints the unit's answer" said "| Error: Activity manager is not running"

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
