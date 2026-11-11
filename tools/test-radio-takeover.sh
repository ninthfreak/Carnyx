#!/usr/bin/env bash
# Run tools/radio-takeover.sh against a FAKE adb and a fake head unit, on the
# host, through every branch it has.
#
#     bash tools/test-radio-takeover.sh
#
# ── WHY A FAKE, AND WHAT IT HOLDS TO ────────────────────────────────────────
#
# The script asks a car head unit for root and edits a file its firmware reads
# at every wake. It cannot be tried here, and a mistake in it is found in a
# driveway — so its logic is tried here instead, against a stand-in that keeps
# the unit's rules:
#
# * `/config` is a real directory under a temp dir, and every write into it is
#   REFUSED unless the fake adbd is root AND the fake mount is read-write —
#   the "Permission denied" and "Read-only file system" a real unit gives.
# * `adb root` succeeds or answers "adbd cannot run as root in production
#   builds", as the build would.
# * `getprop`, `id`, `pm`, `am`, `mount` and `/proc/mounts` answer from the
#   scenario's state, so a test can say "a user build", "Carnyx not installed",
#   "/config is part of the system image".
# * FAILURES COME WITH adb's OWN MESSAGES — "Connection refused", "device
#   offline", the unauthorized text, "Permission denied" on a read — because
#   what the tests hold the script to is that those words reach the screen.
#
# WHAT IT CANNOT SAY is anything about the real unit: whether its ADB listens,
# whether root is allowed, how `/config` is really mounted. Those are what
# `radio-takeover.sh check` reports from the car.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1
SCRIPT=$PWD/tools/radio-takeover.sh
WORK=$(mktemp -d)
LISTENER=""
cleanup() {
    [ -n "$LISTENER" ] && kill "$LISTENER" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); printf '  ok    %s\n' "$1"; }
bad() {
    FAIL=$((FAIL + 1))
    printf '  FAIL  %s\n' "$1"
    [ -f "${S:-}/out" ] && sed 's/^/          | /' "$S/out"
}
check() { if "${@:2}"; then ok "$1"; else bad "$1"; fi; }

ITEM='<ReplaceSourceItem appid="8" pkgName="com.ninthfreak.carnyx" className="android.app.NativeActivity" />'

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
    version) echo "Android Debug Bridge version 1.0.41" ;;
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
pm() {
    case "$2" in
        com.nwd.kernel)
            if [ -z "$(get unit_serial)" ] || [ "$(get unit_serial)" = "$MOCK_SERIAL" ]; then
                echo "package:/system/priv-app/NwdKernel/NwdKernel.apk"
            fi ;;
        com.ninthfreak.carnyx) [ "$(get carnyx)" = 1 ] && echo "package:/data/app/com.ninthfreak.carnyx-1/base.apk" ;;
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
        printf '    $ radio-takeover.sh %s\n' "$*"
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

echo "radio-takeover.sh against a fake head unit"

# ── check ────────────────────────────────────────────────────────────────────
unit check-fresh
run check 10.0.0.7
check "check: exits 0" rc 0
check "check: reports the build and that root should work" said "adb root should work"
check "check: says the takeover is absent" said "absent — the takeover is not installed"
check "check: writes nothing" not test -e "$S/unit/config/app"
check "check: asks for no root" state root 0

unit check-user type=user debuggable=0 rootable=0
run check 10.0.0.7
check "check on a user build: says root will be refused" said "adb root will be refused"

# ── the confirmation ─────────────────────────────────────────────────────────
unit declined
ANSWER=n ask install 10.0.0.7
check "declined: exits 0" rc 0
check "declined: says nothing changed" said "Nothing changed."
check "declined: writes nothing" not test -e "$S/unit/config/app"
check "declined: asks for no root" state root 0
ANSWER=y ask install 10.0.0.7
check "accepted: installs" has "$(file)" "$ITEM"

# ── install, fresh ───────────────────────────────────────────────────────────
unit fresh
run install 10.0.0.7
check "install: exits 0" rc 0
check "install: writes the Carnyx entry" has "$(file)" "$ITEM"
check "install: makes no backup when there was no file" not test -e "$(backup)"
check "install: reloads the kernel's list" reloaded
check "install: reads the file back" said "Read back: the takeover is installed"
check "install: remounts /config read-only again" state ro 1
check "install: drops adb root again" state root 0
check "install: reminds about release-on-sleep" said "Stop the radio when Carnyx closes"

run install 10.0.0.7
check "install twice: says it is already there" said "already installed"
check "install twice: still one entry" test "$(items "$(file)")" = 1
check "install twice: needs no root to find that out" not said "Asking for adb root"

run check 10.0.0.7
check "check after install: says it IS installed" said "The takeover IS installed"

run undo 10.0.0.7
check "undo of our own file: exits 0" rc 0
check "undo of our own file: removes it" not test -e "$(file)"
check "undo: reloads" reloaded
check "undo: /config read-only again" state ro 1

run undo 10.0.0.7
check "undo with nothing there: says so" said "nothing to undo"

# ── install over somebody else's file ────────────────────────────────────────
unit merge
mkdir -p "$S/unit/config/app"
cat > "$(file)" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<ReplaceSourceList>
    <ReplaceSourceItem appid="2" pkgName="com.example.music" className="com.example.music.Main" />
</ReplaceSourceList>
XML
cp "$(file)" "$S/original.xml"
run install 10.0.0.7
check "merge: exits 0" rc 0
check "merge: keeps the other entry" has "$(file)" 'pkgName="com.example.music"'
check "merge: adds Carnyx's" has "$(file)" "$ITEM"
check "merge: two entries" test "$(items "$(file)")" = 2
check "merge: backs the original up" cmp -s "$(backup)" "$S/original.xml"
run undo 10.0.0.7
check "undo after merge: the original comes back byte for byte" cmp -s "$(file)" "$S/original.xml"
check "undo after merge: the backup is gone" not test -e "$(backup)"

unit replace
mkdir -p "$S/unit/config/app"
printf '%s\n' "<ReplaceSourceList><ReplaceSourceItem appid='8' pkgName='com.other.radio' className='com.other.radio.Main'/><ReplaceSourceItem appid=\"15\" pkgName=\"com.example.bt\" className=\"X\" /></ReplaceSourceList>" > "$(file)"
cp "$(file)" "$S/original.xml"
run install 10.0.0.7
check "replace: exits 0" rc 0
check "replace: the other radio entry is gone" not has "$(file)" "com.other.radio"
check "replace: the Bluetooth entry on the same line stays" has "$(file)" 'pkgName="com.example.bt"'
check "replace: Carnyx's is in" has "$(file)" "$ITEM"
check "replace: the original is backed up" cmp -s "$(backup)" "$S/original.xml"
run undo 10.0.0.7
check "undo after replace: the original radio entry is back" cmp -s "$(file)" "$S/original.xml"

unit theirs-only
mkdir -p "$S/unit/config/app"
printf '%s\n' '<ReplaceSourceList>' '  <ReplaceSourceItem appid="2" pkgName="com.example.music" className="M" />' '</ReplaceSourceList>' > "$(file)"
cp "$(file)" "$S/original.xml"
run undo 10.0.0.7
check "undo with no Carnyx entry and no backup: leaves the file alone" cmp -s "$(file)" "$S/original.xml"
check "undo with no Carnyx entry: says so" said "has no Carnyx entry"

# ── the refusals: each stops before anything is written ──────────────────────
unit no-root type=user debuggable=0 rootable=0
run install 10.0.0.7
check "root refused: exits 1" rc 1
check "root refused: says so" said "refused adb root"
check "root refused: writes nothing" not test -e "$S/unit/config/app"

unit system-image ownmount=0
run install 10.0.0.7
check "/config in the system image: exits 1" rc 1
check "/config in the system image: will not touch verity" said "part of the"
check "/config in the system image: writes nothing" not test -e "$S/unit/config/app"
check "/config in the system image: drops root again" state root 0

unit no-carnyx carnyx=0
run install 10.0.0.7
check "Carnyx not installed: exits 1" rc 1
check "Carnyx not installed: says so" said "Carnyx is not installed"
check "Carnyx not installed: asks for no root" state root 0

unit unfamiliar
mkdir -p "$S/unit/config/app"
printf '%s\n' '<SomethingElse appid="8" />' > "$(file)"
cp "$(file)" "$S/original.xml"
run install 10.0.0.7
check "unfamiliar file: exits 1" rc 1
check "unfamiliar file: unchanged" cmp -s "$(file)" "$S/original.xml"
check "unfamiliar file: no backup made" not test -e "$(backup)"
check "unfamiliar file: /config read-only again" state ro 1

unit split-entry
mkdir -p "$S/unit/config/app"
printf '%s\n' '<ReplaceSourceList>' '  <ReplaceSourceItem appid="8"' '      pkgName="com.other.radio" className="R" />' '</ReplaceSourceList>' > "$(file)"
cp "$(file)" "$S/original.xml"
run install 10.0.0.7
check "radio entry split over lines: exits 1" rc 1
check "radio entry split over lines: unchanged" cmp -s "$(file)" "$S/original.xml"

unit unauthorized auth=unauthorized
run check 10.0.0.7
check "unanswered prompt: exits 1" rc 1
check "unanswered prompt: says to tap Allow" said "Tap Allow"
check "unanswered prompt: prints adb's own message" said "Otherwise check for a confirmation dialog"

# ── adb's own words, on every failure ────────────────────────────────────────
# An adb that cannot start, as Termux's did on the owner's phone.
unit broken-adb
cat > "$WORK/bin/adb-unlinked" <<'BAD'
#!/bin/sh
echo 'CANNOT LINK EXECUTABLE "adb": cannot locate symbol "_ZNSt6__ndk113__hash_memoryEPKvm" referenced by "/data/data/com.termux/files/usr/bin/adb"...' >&2
exit 1
BAD
chmod +x "$WORK/bin/adb-unlinked"
MOCK_STATE=$S ADB=$WORK/bin/adb-unlinked CONNECT_PAUSE=0 CONNECT_TRIES=3 \
    bash "$SCRIPT" check 10.0.0.7 > "$S/out" 2>&1 < /dev/null
echo $? > "$S/rc"
check "adb that cannot start: exits 1" rc 1
check "adb that cannot start: stops before connecting" not said "Connecting to"
check "adb that cannot start: prints the loader's words" said '| CANNOT LINK EXECUTABLE "adb"'
check "adb that cannot start: says how to fix it in Termux" said "pkg upgrade"

unit offline auth=offline
run check 10.0.0.7
check "offline: exits 1" rc 1
check "offline: prints what get-state said" said "| error: device offline"
check "offline: names the command that said it" said "adb -s 10.0.0.7:5555 get-state:"
check "offline: says it gave up, and after how much" said "Gave up after 3 tries"
check "offline: prints it once, not once a try" test "$(grep -c 'device offline' "$S/out")" = 1

unit refused refuse=all
run check 10.0.0.7
check "connection refused: exits 1" rc 1
check "connection refused: prints what adb connect said" said "| failed to connect to '10.0.0.7:5555': Connection refused"
check "connection refused: prints what get-state said" said "not found"

unit refused-root type=user debuggable=0 rootable=0
run install 10.0.0.7
check "root refused: prints adb's answer" said "| adbd cannot run as root in production builds"

unit system-image-words ownmount=0
run install 10.0.0.7
check "/config in the system image: prints the write test's answer" said "Read-only file system"

unit stuck-remount stuck=1
run install 10.0.0.7
check "remount that changes nothing: exits 1" rc 1
check "remount that changes nothing: says so" said "remounted and is still not writable"
check "remount that changes nothing: prints why" said "| $S/unit/config/.carnyx-write-test: Read-only file system"
check "remount that changes nothing: writes nothing" not test -e "$S/unit/config/app"

unit no-broadcast amfail=1
run install 10.0.0.7
check "reload refused: still installs" has "$(file)" "$ITEM"
check "reload refused: prints the unit's answer" said "| Error: Activity manager is not running"

# A shell that may not look into /config is not told "absent".
unit unreadable noread=1
run check 10.0.0.7
check "unreadable /config: check exits 0" rc 0
check "unreadable /config: check does not call the file absent" not said "absent"
check "unreadable /config: check says it could not tell" said "could not tell whether it is there"
check "unreadable /config: check prints why" said "Permission denied"
run install 10.0.0.7
check "unreadable /config: install says it will look as root" said "will look again as root"
check "unreadable /config: install installs" has "$(file)" "$ITEM"
run install 10.0.0.7
check "unreadable /config: a second install finds it as root" said "already in the file"
check "unreadable /config: still one entry" test "$(items "$(file)")" = 1
run undo 10.0.0.7
check "unreadable /config: undo removes it" not test -e "$(file)"

unit writable ro=0
run install 10.0.0.7
check "already writable: no remount" not said "remounting it read-write"
check "already writable: left writable" state ro 0
check "already writable: installed" has "$(file)" "$ITEM"

unit port
run check 10.0.0.7:5037
check "a given port is used" said "Connecting to 10.0.0.7:5037"
run check 10.0.0.7
check "no port means 5555" said "Connecting to 10.0.0.7:5555"

# ── finding the unit ─────────────────────────────────────────────────────────
echo "finding the head unit"

# THE INTERFACES: Wi-Fi and Ethernet, private ranges only — never cellular.
mkdir -p "$WORK/fakenet"
cat > "$WORK/fakenet/ip" <<'IP'
#!/bin/sh
cat <<'OUT'
1: lo    inet 127.0.0.1/8 scope host lo\       valid_lft forever preferred_lft forever
12: rmnet_data0    inet 10.170.5.2/30 scope global rmnet_data0\       valid_lft forever
25: ap0    inet 192.168.43.1/24 brd 192.168.43.255 scope global ap0\       valid_lft forever
26: wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global wlan0\       valid_lft forever
27: tun0    inet 10.8.0.2/24 scope global tun0\       valid_lft forever
28: wlan1    inet 100.70.1.2/24 scope global wlan1\       valid_lft forever
OUT
IP
chmod +x "$WORK/fakenet/ip"
got=$(PATH="$WORK/fakenet:$PATH" bash -c "source '$SCRIPT'; local_prefixes")
want=$'192.168.1 192.168.1.20\n192.168.43 192.168.43.1'
check "prefixes: the hotspot and the Wi-Fi, not cellular, VPN or loopback" test "$got" = "$want"

mkdir -p "$WORK/fakenet2"
printf '#!/bin/sh\nexit 1\n' > "$WORK/fakenet2/ip"
cat > "$WORK/fakenet2/ifconfig" <<'IFC'
#!/bin/sh
cat <<'OUT'
ccmni0    Link encap:UNSPEC
          inet addr:10.44.3.9  Mask:255.0.0.0
swlan0    Link encap:UNSPEC  Driver wlan
          inet addr:10.67.12.1  Bcast:10.67.12.255  Mask:255.255.255.0
OUT
IFC
chmod +x "$WORK/fakenet2/ip" "$WORK/fakenet2/ifconfig"
got=$(PATH="$WORK/fakenet2:$PATH" bash -c "source '$SCRIPT'; local_prefixes")
check "prefixes: toybox ifconfig when ip is blocked, hotspot only" test "$got" = "10.67.12 10.67.12.1"

# THE SCAN, against a real listener on one loopback address.
PORT=$((40000 + RANDOM % 20000))
python3 -I -c "
import socket, sys, time
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.5', int(sys.argv[1]))); s.listen(64)
sys.stdout.write('up\n'); sys.stdout.flush()
s.settimeout(30)
try:
    while True:
        c, _ = s.accept(); c.close()
except Exception:
    pass
" "$PORT" > "$WORK/listener.out" &
LISTENER=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do grep -q up "$WORK/listener.out" 2>/dev/null && break; sleep 0.2; done
got=$(bash -c "source '$SCRIPT'; scan 127.0.0 127.0.0.9 $PORT")
check "scan: finds the one address that answers, and only it" test "$got" = "127.0.0.5"

# THE CHOICE: two devices answer ADB, one of them is the NWD unit.
unit find unit_serial=192.168.43.20:5555
got=$(MOCK_STATE=$S ADB=$WORK/bin/adb CONNECT_PAUSE=0 CONNECT_TRIES=3 bash -c "
    source '$SCRIPT'
    local_prefixes() { echo '192.168.43 192.168.43.1'; }
    scan() { printf '%s\n' 192.168.43.30 192.168.43.20; }
    find_unit >/dev/null && echo \"\$T\"")
check "find: picks the NWD unit out of two ADB devices" test "$got" = "192.168.43.20:5555"

unit find-none unit_serial=192.168.43.99:5555
got=$(MOCK_STATE=$S ADB=$WORK/bin/adb CONNECT_PAUSE=0 CONNECT_TRIES=3 bash -c "
    source '$SCRIPT'
    local_prefixes() { echo '192.168.43 192.168.43.1'; }
    scan() { echo 192.168.43.30; }
    find_unit; echo \"rc=\$? T=\$T\"")
check "find: an unidentified ADB device is not chosen" grep -q 'rc=1 T=$' <<< "$got"
check "find: says why it was not chosen" grep -qF "192.168.43.30:5555 has no com.nwd.kernel" <<< "$got"

unit find-refused refuse=192.168.43.30:5555 unit_serial=192.168.43.20:5555
got=$(MOCK_STATE=$S ADB=$WORK/bin/adb CONNECT_PAUSE=0 CONNECT_TRIES=3 bash -c "
    source '$SCRIPT'
    local_prefixes() { echo '192.168.43 192.168.43.1'; }
    scan() { printf '%s\n' 192.168.43.30 192.168.43.20; }
    find_unit; echo \"rc=\$? T=\$T\"")
check "find: a device that will not connect is passed over" grep -q 'rc=0 T=192.168.43.20:5555$' <<< "$got"
check "find: and adb's reason is printed" grep -qF "| failed to connect to '192.168.43.30:5555': Connection refused" <<< "$got"

got=$(PATH="$WORK/fakenet2:$PATH" bash -c "
    source '$SCRIPT'
    local_prefixes() { :; }
    find_unit; echo \"rc=\$?\"")
check "find with no Wi-Fi address: exits 1" grep -q 'rc=1$' <<< "$got"
check "find with no Wi-Fi address: does not claim the network is empty" not grep -q "Nothing on this network" <<< "$got"
check "find with no Wi-Fi address: prints what ifconfig reported" grep -qF "| swlan0" <<< "$got"

echo
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
