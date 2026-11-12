#!/usr/bin/env bash
# Point any of the NWD head unit's factory sources (radio, music, video, GPS,
# launcher, …) at an app of your choice, over network ADB — from a phone
# (Termux) or a PC on the same Wi-Fi or hotspot.
#
#   bash source-remap.sh sources                          the source table; needs no unit
#   bash source-remap.sh find                             which device is the head unit
#   bash source-remap.sh list    [IP[:PORT]]              what each source points at now
#   bash source-remap.sh apps    [IP[:PORT]]              the unit's launchable apps, numbered
#   bash source-remap.sh set     SOURCE [APP] [IP[:PORT]] [-y] [--force]
#   bash source-remap.sh reset   SOURCE|all   [IP[:PORT]] [-y]
#
#   SOURCE  a name from `sources` (music, video, gps, radio, launcher…) or its appid number.
#   APP     a number from `apps`, a package name, package/activity, or a word to
#           search for (`set music player`). Left out, you pick from a numbered list.
#   reset   puts ONE source back the way the original file had it; `reset all`
#           puts the whole original file back.
#
# With no IP it looks for the head unit itself. Lines that start with "|" are
# adb's or the head unit's own words, unedited.
#
# ── WHAT IT EDITS ───────────────────────────────────────────────────────────
#
# `<config>/app/replace_source_list.xml`, the list `com.nwd.kernel` consults
# in `SourceMgr.startApp` before launching a source's app: for every appid in
# the list it launches the package and class given there instead of the stock
# one. `<config>` is `ro.nwd.config.path` (`/data/nwdappconfig` on the owner's
# unit, `/config` by default). The full account is `docs/vendor/README.md` and
# the handoff `FACTORY-XML-HANDOFF.md`.
#
# This REPLACES WHICH APP a source opens. It cannot remove a source from the
# MODE cycle: the list has no field for that, and an entry with an empty or bad
# package does not skip the source — the kernel still stops on it and fails to
# launch anything.
#
# ── THE RULES IT KEEPS ──────────────────────────────────────────────────────
#
# * MERGE, NEVER OVERWRITE. Only the named source's entry changes; every other
#   entry, comment and byte of layout survives, and the result is parsed and
#   compared with what was there before it is written. Text inside <!-- -->
#   comments is never read as an entry and never edited.
# * THE ORIGINAL IS KEPT ONCE. Before the first change the file is copied to
#   `replace_source_list.xml.carnyx-backup` — the same name radio-takeover.sh
#   uses, so both tools share one record of the original. When there was no
#   file at all, an empty `replace_source_list.xml.carnyx-absent` records that
#   instead. Neither is ever overwritten.
# * RESET IS PER SOURCE. `reset music` restores the original's music entry (or
#   removes the entry if the original had none) and leaves every other source
#   alone. When nothing differs from the original any more, the original file
#   goes back byte for byte and the record is removed.
# * RELOAD, THEN READ BACK. `com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE` after
#   every write, then the file is pulled and checked.
# * THE PACKAGE MUST BE INSTALLED, and must have an activity to launch. The
#   kernel's className fallback (`getLaunchIntentForPackage`) only helps an app
#   that HAS a launcher activity.
# * THE CAMERA (appid 13) needs --force. It is the reverse camera too.
#
# radio-takeover.sh's `undo` restores the WHOLE backup: run after this script
# has set other sources, it puts them all back. Use `reset radio` here instead.
#
# ── TESTED ON THE HOST, NOT ON THE UNIT ─────────────────────────────────────
#
# `tools/test-source-remap.sh` runs every branch against a fake `adb`. Only the
# radio remap has been run on a real unit; every other source is untried.

set -uo pipefail

PROG=source-remap.sh
NAME=replace_source_list.xml
BACKUP=$NAME.carnyx-backup
ABSENT=$NAME.carnyx-absent
RELOAD=com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE
KERNEL=com.nwd.kernel
CARNYX=com.ninthfreak.carnyx
DEFAULT_PORT=5555
ADB=${ADB:-adb}

RX_PKG='^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z0-9_]+)+$'
RX_CLS='^[A-Za-z_$][A-Za-z0-9_.$]*$'
RX_IP='^[0-9]{1,3}(\.[0-9]{1,3}){3}(:[0-9]+)?$'
RX_ANY_ITEM='<ReplaceSourceItem[^>]*/>'

T=""          # the head unit, as host:port
CFG=""        # ro.nwd.config.path
ROOTED=0      # this run asked for adb root
REMOUNTED=0   # this run remounted <config> read-write
ASSUME_YES=0
FORCE=0
TMPD=""

SAID=""       # why the last `exists` or `has_package` could not tell
WRITE_SAID="" # what the last `writable` test answered

# ── The sources ─────────────────────────────────────────────────────────────
# From com.nwd.kernel.source.SourceConstant. "appid name risk description".
# risk: tested | plausible | caution | force
SOURCE_TABLE='1 gps plausible GPS / navigation
2 music plausible Music
3 video plausible Video
4 launcher plausible Launcher: what the kernel opens on wake when it restores no source
5 dvd caution DVD
6 ipod caution iPod
7 cdc caution CD changer
8 radio tested Radio (the one remap run on a real unit)
9 aux caution AUX in
10 bt caution Bluetooth
11 tv caution TV
12 tpms caution Tire-pressure monitor
13 camera force Camera, INCLUDING THE REVERSE CAMERA
14 setting caution Settings
15 bt-music caution Bluetooth music
16 dvd-sd caution DVD (SD)
17 dvd-usb caution DVD (USB)
18 dvr caution Dash cam / DVR
19 phone caution Phone
26 car-setting caution Car settings
27 screen-protect caution Screen protect
28 tscalibration caution Touch calibration
29 nwd-phone caution NWD phone
31 aux-rear caution Rear AUX
227 usblink caution USB link / phone mirror
228 mhl caution MHL
229 can-aux caution CAN AUX
230 can-cdc caution CAN CD changer
231 can-cd caution CAN CD
232 mp5 caution MP5
233 can-bt-audio caution CAN Bluetooth audio
234 can-onstar-phone caution CAN OnStar phone
235 can-radio caution CAN radio
236 can-setting caution CAN settings
237 can-sync caution CAN sync
238 can-mediabox caution CAN media box
239 can-usb caution CAN USB
252 audio-setting caution Audio settings
253 datetime-setting caution Date/time settings
255 other caution Other'

# Other words people use for a source.
alias_of() {
    case "$1" in
        nav | navigation | maps) echo gps ;;
        fm | tuner) echo radio ;;
        home) echo launcher ;;
        bluetooth) echo bt ;;
        settings) echo setting ;;
        btmusic | bluetooth-music | a2dp) echo bt-music ;;
        reverse | backup-camera | rear-camera) echo camera ;;
        dashcam) echo dvr ;;
        *) echo "$1" ;;
    esac
}

# SOURCE (a number or a name) -> "appid name risk description", or nothing.
source_row() {
    local want
    want=$(printf '%s' "$1" | tr 'A-Z_' 'a-z-')
    want=$(alias_of "$want")
    printf '%s\n' "$SOURCE_TABLE" | awk -v w="$want" '$1 == w || $2 == w { print; exit }'
}
source_name() { local r; r=$(source_row "$1"); if [ -n "$r" ]; then printf '%s' "$r" | awk '{ print $2 }'; else printf 'appid %s' "$1"; fi; }

say() { printf '%s\n' "$*"; }

# adb's or the head unit's own words, unedited, under the line they explain.
quote() {
    if [ -n "$1" ]; then printf '%s\n' "$1" | sed 's/^/  | /'; else say "  | (said nothing)"; fi
}

die() {
    printf '\nSTOPPED: %s\n' "$1" >&2
    if [ "$#" -gt 1 ]; then quote "$2" >&2; fi
    cleanup_device
    exit 1
}

must() {
    local what=$1 out rc
    shift
    out=$("$@" 2>&1 | tr -d '\r')
    rc=$?
    [ "$rc" = 0 ] && return 0
    die "$what" "${out:-(no message; exit status $rc)}"
}

on_unit() { "$ADB" -s "$T" shell "$1" | tr -d '\r'; }

# ── Connecting (as radio-takeover.sh) ───────────────────────────────────────

with_port() {
    case "$1" in
        *:*) printf '%s' "$1" ;;
        *) printf '%s:%s' "$1" "$DEFAULT_PORT" ;;
    esac
}

connect() {
    local target=$1 tries=0 said state last_said="" last_state="" told=0 waited=0
    local limit=${CONNECT_TRIES:-30}
    while :; do
        said=$("$ADB" connect "$target" 2>&1 | tr -d '\r')
        state=$("$ADB" -s "$target" get-state 2>&1 | tr -d '\r')
        if [ "$state" = device ]; then
            [ "$waited" = 1 ] && say "  It says \"device\" now."
            return 0
        fi
        if [ "${said#already }" != "$last_said" ]; then
            say "  adb connect $target:"
            quote "$said"
            last_said=${said#already }
        fi
        if [ "$state" != "$last_state" ]; then
            say "  adb -s $target get-state:"
            quote "$state"
            last_state=$state
        fi
        case "$state" in
            *unauthorized*)
                [ "$told" = 1 ] || say "  -> Tap Allow on the head unit's screen (\"Allow USB debugging?\")."
                told=1 ;;
        esac
        tries=$((tries + 1))
        if [ "$tries" -ge "$limit" ]; then
            say "  Gave up after $tries tries."
            return 1
        fi
        [ "$waited" = 1 ] || say "  Waiting for it to say \"device\" (up to $limit tries)…"
        waited=1
        sleep "${CONNECT_PAUSE:-1}"
    done
}

let_go() { "$ADB" disconnect "$1" >/dev/null 2>&1; }

# ── Finding the head unit (as radio-takeover.sh) ────────────────────────────

local_prefixes() {
    {
        ip -4 -o addr show 2>/dev/null | awk '{ print $2, $4 }'
        ifconfig 2>/dev/null | awk '
            /^[^ \t]/ { i = $1; sub(":", "", i) }
            /inet / { a = $2; sub("addr:", "", a); print i, a }'
    } | awk '
        $1 ~ /^(ap|swlan|softap|wlan|wl|eth|en)/ {
            split($2, p, "/"); n = split(p[1], o, ".")
            if (n != 4) next
            if (o[1] == 10 || (o[1] == 172 && o[2] >= 16 && o[2] <= 31) || (o[1] == 192 && o[2] == 168))
                print o[1] "." o[2] "." o[3], p[1]
        }' | sort -u
}

scan() {
    local prefix=$1 own=$2 port=$3 n
    for n in $(seq 1 254); do
        [ "$prefix.$n" = "$own" ] && continue
        ( timeout 1 bash -c "exec 3<>/dev/tcp/$prefix.$n/$port" 2>/dev/null && echo "$prefix.$n" ) &
        if [ $((n % 64)) = 0 ]; then wait; fi
    done
    wait
}

find_unit() {
    local port=${1:-$DEFAULT_PORT} prefix own h nets=0
    local -a hits=() units=()
    while read -r prefix own; do
        [ -n "$prefix" ] || continue
        nets=$((nets + 1))
        say "Looking for ADB on $prefix.1-254 (this device is $own)…"
        while read -r h; do
            [ -n "$h" ] && hits+=("$h")
        done < <(scan "$prefix" "$own" "$port")
    done < <(local_prefixes)
    if [ "$nets" = 0 ]; then
        say "Found no Wi-Fi or Ethernet address on this device to search from."
        say "ip -4 -o addr show:"
        quote "$(ip -4 -o addr show 2>&1)"
        say "ifconfig:"
        quote "$(ifconfig 2>&1)"
        say "Pass the head unit's address instead: bash $PROG list 192.168.x.y"
        return 1
    fi
    if [ "${#hits[@]}" = 0 ]; then
        say "Nothing on this network is listening for ADB on port $port."
        say "Check the head unit is on this Wi-Fi and has network ADB switched on, or pass"
        say "its address: bash $PROG list 192.168.x.y"
        return 1
    fi
    for h in "${hits[@]}"; do
        h=$h:$port
        say "$h answers on ADB's port. Asking what it is…"
        if ! connect "$h"; then
            say "  Could not connect to $h; adb's answers are above. Left alone."
            let_go "$h"
            continue
        fi
        has_package "$KERNEL" "$h"
        case $? in
            0) units+=("$h") ;;
            1) say "  $h has no $KERNEL: it is not an NWD head unit. Left alone."; let_go "$h" ;;
            2) say "  Could not ask $h what is installed:"; quote "$SAID"; say "  Left alone."; let_go "$h" ;;
        esac
    done
    case "${#units[@]}" in
        0) say "No NWD head unit among them."; return 1 ;;
        1) T=${units[0]}; say "Head unit: $T"; return 0 ;;
        *) say "More than one NWD head unit: ${units[*]} — pass the one you mean."; return 1 ;;
    esac
}

# ── What the unit is ────────────────────────────────────────────────────────

facts() {
    local type tags dbg
    type=$(on_unit 'getprop ro.build.type')
    tags=$(on_unit 'getprop ro.build.tags')
    dbg=$(on_unit 'getprop ro.debuggable')
    CFG=$(on_unit 'getprop ro.nwd.config.path')
    [ -n "$CFG" ] || CFG=/config
    say "build: $type/$tags, ro.debuggable=${dbg:-?}"
    if [ "$type" = userdebug ] || [ "$type" = eng ] || [ "$dbg" = 1 ]; then
        say "  adb root should work on a build of this kind."
    else
        say "  adb root will be refused on a build of this kind — changes cannot be written this way."
    fi
    say "config: $CFG (ro.nwd.config.path)"
}

has_package() {
    local raw
    raw=$("$ADB" -s "${2:-$T}" shell "pm path $1; echo ':asked'" 2>&1 | tr -d '\r')
    SAID=$(printf '%s\n' "$raw" | grep -vx ':asked')
    if printf '%s\n' "$raw" | grep -q '^package:'; then return 0; fi
    [ "$raw" = ":asked" ] && return 1
    return 2
}

exists() {
    local rc
    SAID=$(on_unit "ls -d '$1'" 2>&1)
    rc=$?
    case "$SAID" in *"No such file or directory"*) return 1 ;; esac
    [ "$rc" = 0 ] && return 0
    return 2
}

# ── Root, and a writable <config> (as radio-takeover.sh) ────────────────────

become_root() {
    local out rc uid
    if [ "$(on_unit 'id -u')" = 0 ]; then return 0; fi
    say "Asking for adb root…"
    out=$("$ADB" -s "$T" root 2>&1 | tr -d '\r')
    rc=$?
    quote "$out"
    case "$out" in
        *"cannot run as root"* | *"not permitted"* | *"unable"*)
            die "the head unit refused adb root (its answer is above). This ROM is a production
build, and without root $NAME cannot be written." ;;
    esac
    [ "$rc" = 0 ] || die "adb root failed with exit status $rc; its answer is above."
    ROOTED=1
    sleep "${ROOT_PAUSE:-2}"
    connect "$T" || die "lost the head unit after adb root (adb's answers are above) — run the command again."
    uid=$(on_unit 'id -u' 2>&1)
    if [ "$uid" != 0 ]; then
        die "adb root answered as above, but the shell is still not root. 'id -u' said:" "$uid"
    fi
}

writable() { WRITE_SAID=$(on_unit "touch '$CFG/.carnyx-write-test' && rm -f '$CFG/.carnyx-write-test'" 2>&1); }

make_writable() {
    local mnt
    writable && return 0
    mnt=$(on_unit 'cat /proc/mounts' | awk -v d="$CFG" '$2 == d')
    if [ -z "$mnt" ]; then
        die "$CFG is not writable, and it is not a mount of its own: it is part of the
read-only system image. Making that writable means 'adb disable-verity', a reboot
and 'adb remount'. This script will not do that on its own; ask first.
A test write in $CFG answered:" "$WRITE_SAID"
    fi
    say "$CFG is not writable. A test write answered:"
    quote "$WRITE_SAID"
    say "It is its own mount — remounting it read-write for this change."
    say "  ($mnt)"
    must "the remount of $CFG failed." on_unit "mount -o rw,remount '$CFG'"
    REMOUNTED=1
    writable || die "$CFG was remounted and is still not writable. A test write answered:" "$WRITE_SAID"
}

# Only undoes what THIS run did: the read-only remount only if this run made it
# read-write, so a <config> that is a plain directory on /data is never touched.
cleanup_device() {
    local out
    if [ "$REMOUNTED" = 1 ]; then
        REMOUNTED=0
        if out=$(on_unit "mount -o ro,remount '$CFG'" 2>&1); then
            say "$CFG remounted read-only again."
        else
            say "Could not remount $CFG read-only again; a reboot will. The unit answered:"
            quote "$out"
        fi
    fi
    if [ "$ROOTED" = 1 ]; then
        ROOTED=0
        if out=$("$ADB" -s "$T" unroot 2>&1 | tr -d '\r'); then
            say "adbd is unprivileged again."
        else
            say "adb unroot failed; a reboot will drop root. adb answered:"
            quote "$out"
        fi
    fi
}

reload() {
    local out
    if out=$(on_unit "am broadcast -a $RELOAD" 2>&1); then
        say "Told the kernel to reload the list."
    else
        say "The reload broadcast failed; a reboot will load the file instead. The unit answered:"
        quote "$out"
    fi
}

confirm() {
    [ "$ASSUME_YES" = 1 ] && return 0
    printf '%s [y/N] ' "$1"
    local answer
    read -r answer || answer=""
    case "$answer" in y | Y | yes | YES) return 0 ;; *) say "Nothing changed."; cleanup_device; exit 0 ;; esac
}

# ── The file: reading and editing, comment-aware ────────────────────────────
#
# One awk pass, two modes. `strip` prints every line with the text inside
# <!-- --> removed, so a commented-out example is never read as an entry.
# `edit` prints the file as it is, except that OUTSIDE comments it deletes every
# match of RX (a line left blank by that goes too) and, given ITEM, inserts it
# before the first </ReplaceSourceList>. RX and ITEM come in through the
# environment, not `-v`, because awk processes backslash escapes in `-v`.
XML_AWK='
BEGIN { rx = ENVIRON["RX"]; item = ENVIRON["ITEM"]; mode = ENVIRON["MODE"] }
{
    line = $0; out = ""; removed = 0
    while (line != "") {
        if (inc) {
            q = index(line, "-->")
            if (q == 0) { if (mode == "edit") out = out line; line = "" }
            else { if (mode == "edit") out = out substr(line, 1, q + 2); line = substr(line, q + 3); inc = 0 }
        } else {
            p = index(line, "<!--")
            if (p == 0) { seg = line; line = "" } else { seg = substr(line, 1, p - 1); line = substr(line, p + 4) }
            if (mode == "edit") {
                if (rx != "") removed += gsub(rx, "", seg)
                if (item != "" && !done && sub(/<\/ReplaceSourceList>/, "    " item "\n</ReplaceSourceList>", seg)) done = 1
            }
            out = out seg
            if (p > 0) { if (mode == "edit") out = out "<!--"; inc = 1 }
        }
    }
    if (mode == "edit" && removed > 0 && out ~ /^[ \t]*$/) next
    print out
}'

visible() { MODE="strip" awk "$XML_AWK" "$1"; }
folded_visible() { visible "$1" | tr '\n' ' '; }

# Entries an appid regex matches: any quotes, any spacing.
rx_appid() { printf '%s' "<ReplaceSourceItem[^>]*appid[[:space:]]*=[[:space:]]*[\"']$1[\"'][^>]*/>"; }

# Every entry as "appid<TAB>pkgName<TAB>className", in file order.
items_of() {
    [ -f "$1" ] || return 0
    folded_visible "$1" | grep -oE "$RX_ANY_ITEM" | awk '
        function attr(s, n,   m) {
            if (match(s, n "[[:space:]]*=[[:space:]]*\"[^\"]*\"") || match(s, n "[[:space:]]*=[[:space:]]*\047[^\047]*\047")) {
                m = substr(s, RSTART, RLENGTH); sub(/^[^=]*=[[:space:]]*./, "", m); sub(/.$/, "", m); return m
            }
            return ""
        }
        { print attr($0, "appid") "\t" attr($0, "pkgName") "\t" attr($0, "className") }'
}
others_than() { items_of "$2" | awk -F'\t' -v a="$1" '$1 != a'; }
entries_for() { items_of "$2" | awk -F'\t' -v a="$1" '$1 == a'; }
# The entry's own text, folded onto one line, as it stands in the file.
raw_entry() { folded_visible "$2" | grep -oE "$(rx_appid "$1")" | head -n 1 | tr -s ' \t' ' '; }
opening_tags() { folded_visible "$1" | grep -o '<ReplaceSourceItem' | wc -l | tr -d ' '; }

# A file this script can edit: a </ReplaceSourceList> outside comments, and
# every <ReplaceSourceItem it holds parsed as a whole, self-closed entry.
familiar() {
    local f=$1
    folded_visible "$f" | grep -q '</ReplaceSourceList>' || return 1
    [ "$(opening_tags "$f")" = "$(items_of "$f" | wc -l | tr -d ' ')" ]
}

refuse_unfamiliar() {
    die "$NAME is not a shape this script can edit safely (no </ReplaceSourceList>,
or an entry it cannot parse whole). Nothing was changed. Its contents:
$(sed 's/^/  /' "$1")"
}

# Replace appid A's entries in IN with ITEM (or with nothing, if ITEM is
# empty), writing OUT. Then PROVE it: every other entry exactly as before, and
# appid A's entries exactly ITEM's. A file where that does not hold is refused,
# not half-edited.
rewrite() {
    local a=$1 item=$2 in=$3 out=$4 want got
    RX="$(rx_appid "$a")" ITEM="$item" MODE=edit awk "$XML_AWK" "$in" > "$out"
    if [ "$(others_than "$a" "$in")" != "$(others_than "$a" "$out")" ]; then
        die "editing $NAME would have changed entries other than $(source_name "$a")'s.
Nothing was changed. Its contents:
$(sed 's/^/  /' "$in")"
    fi
    want=""
    if [ -n "$item" ]; then
        printf '<ReplaceSourceList>\n%s\n</ReplaceSourceList>\n' "$item" > "$TMPD/one.xml"
        want=$(entries_for "$a" "$TMPD/one.xml")
    fi
    got=$(entries_for "$a" "$out")
    if [ "$got" != "$want" ] || ! familiar "$out"; then
        die "the edit of $(source_name "$a")'s entry did not come out as intended (probably
an entry split across lines). Nothing was changed. Its contents:
$(sed 's/^/  /' "$in")"
    fi
}

fresh() {
    cat > "$2" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<!-- Source remaps for com.nwd.kernel: each entry makes the kernel launch the
     given app for that source. Written by tools/source-remap.sh. -->
<ReplaceSourceList>
    $1
</ReplaceSourceList>
EOF
}

# ── The apps on the unit ────────────────────────────────────────────────────
#
# Every launchable activity, as "package<TAB>activity<TAB>kind<TAB>where" with
# kind "app" (MAIN/LAUNCHER) or "home" (MAIN/HOME), where "user" or "system".
# Sorted: user-installed first, then by package — the order `apps` numbers them
# in, so a number means the same thing to `apps` and to `set` while nothing is
# installed or removed in between.
#
# `cmd package query-activities` answers for all packages in one call. Its
# component lines are the only lines that look like `pkg/activity`; short
# forms (`pkg/.Main`) are expanded. If it gives nothing (an older `cmd`), the
# fallback asks `resolve-activity` once per package — in ONE adb shell, so it
# is one round trip, but slower on the unit.
APPS_DONE=0
load_apps() {
    [ "$APPS_DONE" = 1 ] && return 0
    local launch home users
    launch=$(on_unit 'cmd package query-activities --brief -a android.intent.action.MAIN -c android.intent.category.LAUNCHER' 2>&1)
    home=$(on_unit 'cmd package query-activities --brief -a android.intent.action.MAIN -c android.intent.category.HOME' 2>&1)
    if ! printf '%s\n' "$launch" | grep -qE '^[[:space:]]*[A-Za-z][A-Za-z0-9_.]*/[A-Za-z0-9_.$]+[[:space:]]*$'; then
        say "query-activities gave no list (it answered below); asking each package instead…"
        quote "$(printf '%s\n' "$launch" | head -n 5)"
        launch=$(on_unit 'for p in $(pm list packages | sed "s/^package://"); do cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.LAUNCHER "$p" | tail -n 1; done' 2>&1)
    fi
    users=$(on_unit 'pm list packages -3' 2>&1 | sed -n 's/^package://p')
    {
        printf '%s\n' "$launch" | sed 's/^/app /'
        printf '%s\n' "$home" | sed 's/^/home /'
    } | awk -v users="$users" '
        BEGIN { n = split(users, u, "\n"); for (i = 1; i <= n; i++) if (u[i] != "") U[u[i]] = 1 }
        {
            kind = $1; c = $2
            if (NF != 2 || c !~ /^[A-Za-z][A-Za-z0-9_.]*\/[A-Za-z0-9_.$]+$/) next
            split(c, pc, "/"); p = pc[1]; a = pc[2]
            if (substr(a, 1, 1) == ".") a = p a
            if (seen[p "/" a]++) { if (kind == "home") K[p "/" a] = "home"; next }
            K[p "/" a] = kind; order[++m] = p "/" a
        }
        END {
            for (i = 1; i <= m; i++) {
                split(order[i], pc, "/")
                print pc[1] "\t" pc[2] "\t" K[order[i]] "\t" ((pc[1] in U) ? "user" : "system")
            }
        }' | sort -t "$(printf '\t')" -k4,4r -k1,1 -k2,2 > "$TMPD/apps.tsv"
    APPS_DONE=1
}

print_apps() {
    awk -F'\t' '{
        tag = ($4 == "user") ? "" : "  [system]"
        if ($3 == "home") tag = tag "  [home screen]"
        printf "  %3d  %s%s\n", NR, $1, tag
    }' "$1"
}

# ── Commands ────────────────────────────────────────────────────────────────

do_sources() {
    say "Sources the kernel knows (com.nwd.kernel SourceConstant). Use the name or the number."
    say ""
    printf '%s\n' "$SOURCE_TABLE" | awk '{
        risk = $3; d = ""; for (i = 4; i <= NF; i++) d = d (i > 4 ? " " : "") $i
        mark = (risk == "tested") ? "TESTED" : (risk == "plausible") ? "untried" : (risk == "force") ? "NEEDS --force" : "untried, hardware"
        printf "  %3s  %-17s %-18s %s\n", $1, $2, mark, d
    }'
    say ""
    say "Other names: nav, fm, home, bluetooth, settings."
}

# The file as it is on the unit, pulled to $TMPD/now.xml: 0 there, 1 absent,
# 2 could not tell (SAID or the pull's answer printed).
pull_current() {
    local file=$CFG/app/$NAME out
    exists "$file"
    case $? in
        1) rm -f "$TMPD/now.xml"; return 1 ;;
        2) return 2 ;;
    esac
    if out=$("$ADB" -s "$T" pull "$file" "$TMPD/now.xml" 2>&1 | tr -d '\r'); then return 0; fi
    SAID=$out
    return 2
}

describe_items() {
    local a p c note
    while IFS=$'\t' read -r a p c; do
        [ -n "$a" ] || continue
        note=""
        has_package "$p"
        case $? in
            1) note="  <- NOT INSTALLED: this source will launch nothing" ;;
            2) note="  (could not tell whether it is installed)" ;;
        esac
        printf '  %-12s (appid %s) -> %s%s\n' "$(source_name "$a")" "$a" "$p" "$note"
        [ "$p" = "$CARNYX" ] || [ -z "$c" ] || printf '  %16s activity %s\n' "" "$c"
    done
}

do_list() {
    local seen n
    pull_current
    seen=$?
    case $seen in
        0)
            n=$(items_of "$TMPD/now.xml" | wc -l | tr -d ' ')
            if [ "$n" = 0 ]; then
                say "$CFG/app/$NAME is there but remaps nothing: every source opens its stock app."
            else
                say "Remapped sources ($CFG/app/$NAME):"
                items_of "$TMPD/now.xml" | describe_items
                say "Every other source opens its stock app."
            fi
            if [ "$(items_of "$TMPD/now.xml" | cut -f1 | sort | uniq -d)" != "" ]; then
                say "WARNING: some source has two entries. Which one the kernel uses is not known;"
                say "  'set' or 'reset' on that source leaves exactly one."
            fi
            familiar "$TMPD/now.xml" || say "WARNING: the file has entries this script cannot parse whole; 'set' will refuse to edit it." ;;
        1) say "$CFG/app/$NAME: absent — every source opens its stock app." ;;
        2) say "Could not read $CFG/app/$NAME without root. The unit answered:"; quote "$SAID"
           say "('set' and 'reset' will look again as root.)" ;;
    esac
    if [ "$seen" != 2 ]; then
        if exists "$CFG/app/$BACKUP"; then say "The original file is kept at $CFG/app/$BACKUP."
        elif exists "$CFG/app/$ABSENT"; then say "Recorded: there was no file before this script wrote one."
        fi
    fi
}

do_apps() {
    load_apps
    if [ ! -s "$TMPD/apps.tsv" ]; then
        die "the head unit listed no launchable apps. What it answered is above."
    fi
    say "Launchable apps on the head unit (user-installed first). Use the number with 'set':"
    print_apps "$TMPD/apps.tsv"
    say ""
    say "Package names only: Android's shell has no command that prints app labels."
}

# APP as given -> "package<TAB>activity", or a numbered choice. Sets PICK.
PICK=""
choose_app() {
    local want=$1 src=$2 hits n pick
    load_apps
    if [ -z "$want" ]; then
        [ "$ASSUME_YES" = 1 ] && die "no app named, and -y means nobody is here to pick one. Give the app: set $src APP"
        [ -s "$TMPD/apps.tsv" ] || die "the head unit listed no launchable apps."
        say "Which app should the $src source open?"
        print_apps "$TMPD/apps.tsv"
        printf 'Number (Enter to cancel): '
        read -r pick || pick=""
        [ -n "$pick" ] || { say "Nothing changed."; exit 0; }
        want=$pick
        cp "$TMPD/apps.tsv" "$TMPD/menu.tsv"
    else
        cp "$TMPD/apps.tsv" "$TMPD/menu.tsv"
    fi
    if [[ "$want" =~ ^[0-9]+$ ]]; then
        PICK=$(awk -F'\t' -v n="$want" 'NR == n { print $1 "\t" $2 }' "$TMPD/menu.tsv")
        [ -n "$PICK" ] || die "there is no app number $want in the list. Run 'apps' to see it."
        return 0
    fi
    if [[ "$want" == */* ]]; then
        local p=${want%%/*} c=${want#*/}
        [ "${c:0:1}" = . ] && c=$p$c
        [[ "$p" =~ $RX_PKG ]] && [[ "$c" =~ $RX_CLS ]] || die "'$want' is not a package/activity name."
        PICK=$(printf '%s\t%s' "$p" "$c")
        return 0
    fi
    # An exact package: its launcher activity, else its home activity.
    PICK=$(awk -F'\t' -v p="$want" '$1 == p { print $1 "\t" $2 "\t" $3 }' "$TMPD/apps.tsv" \
        | sort -t "$(printf '\t')" -k3,3 | head -n 1 | cut -f1,2)
    [ -n "$PICK" ] && return 0
    if [[ "$want" =~ $RX_PKG ]]; then
        has_package "$want"
        case $? in
            0) die "$want is installed but has no activity to launch, so the kernel would open
nothing for this source. If you know its activity, give it as $want/ActivityName." ;;
            1) die "$want is not installed on the head unit." ;;
            2) die "could not tell whether $want is installed. 'pm path' answered:" "$SAID" ;;
        esac
    fi
    # A word to search for.
    awk -F'\t' -v w="$want" 'BEGIN { w = tolower(w) } index(tolower($1), w)' "$TMPD/apps.tsv" > "$TMPD/menu.tsv"
    hits=$(wc -l < "$TMPD/menu.tsv" | tr -d ' ')
    case "$hits" in
        0) die "no launchable app on the head unit has '$want' in its package name. Run 'apps' for the list." ;;
        1) PICK=$(cut -f1,2 "$TMPD/menu.tsv"); return 0 ;;
    esac
    [ "$ASSUME_YES" = 1 ] && die "'$want' matches $hits apps and -y means nobody is here to pick:
$(print_apps "$TMPD/menu.tsv")"
    say "'$want' matches $hits apps:"
    print_apps "$TMPD/menu.tsv"
    printf 'Number (Enter to cancel): '
    read -r n || n=""
    [ -n "$n" ] || { say "Nothing changed."; exit 0; }
    [[ "$n" =~ ^[0-9]+$ ]] || die "'$n' is not a number."
    PICK=$(awk -F'\t' -v n="$n" 'NR == n { print $1 "\t" $2 }' "$TMPD/menu.tsv")
    [ -n "$PICK" ] || die "there is no number $n in that list."
}

# Back the original up once, or record that there was none.
keep_original() {
    local app=$CFG/app file=$CFG/app/$NAME had=$1
    exists "$app/$BACKUP"; local b=$?
    exists "$app/$ABSENT"; local m=$?
    [ "$b" = 2 ] || [ "$m" = 2 ] && die "could not tell whether a record of the original is there, even as root. The unit answered:" "$SAID"
    [ "$b" = 0 ] || [ "$m" = 0 ] && return 0
    if [ "$had" = 1 ]; then
        must "could not back up $file — nothing changed." on_unit "cp '$file' '$app/$BACKUP'"
        say "Backed the original up to $app/$BACKUP."
    else
        must "could not record that there was no $NAME — nothing changed." on_unit "touch '$app/$ABSENT'"
    fi
}

write_file() {
    local file=$CFG/app/$NAME
    must "could not write $file." "$ADB" -s "$T" push "$1" "$file"
    must "wrote $file, but could not make it readable (chmod 644), so the kernel may not read it." \
        on_unit "chmod 644 '$file'"
}

read_back() {
    local file=$CFG/app/$NAME
    must "wrote $file, but could not read it back." "$ADB" -s "$T" pull "$file" "$TMPD/check.xml"
    cmp -s "$TMPD/check.xml" "$1" || die "$file reads back differently from what was written. It reads:" "$(cat "$TMPD/check.xml")"
}

do_set() {
    local src=$1 want=$2 row a name risk desc p c item file=$CFG/app/$NAME app=$CFG/app had cur
    row=$(source_row "$src")
    [ -n "$row" ] || die "'$src' is not a source this script knows. Run 'sources' for the list."
    read -r a name risk desc <<< "$row"
    if [ "$risk" = force ] && [ "$FORCE" != 1 ]; then
        die "$name (appid $a) is the camera source, which includes the REVERSE camera. If the
app you point it at fails to open, reversing shows nothing. This needs --force."
    fi
    choose_app "$want" "$name"
    p=$(printf '%s' "$PICK" | cut -f1)
    c=$(printf '%s' "$PICK" | cut -f2)
    [[ "$p" =~ $RX_PKG ]] && [[ "$c" =~ $RX_CLS ]] || die "'$p/$c' is not a valid package/activity name."
    has_package "$p"
    case $? in
        1) die "$p is not installed on the head unit." ;;
        2) die "could not tell whether $p is installed. 'pm path' answered:" "$SAID" ;;
    esac
    item="<ReplaceSourceItem appid=\"$a\" pkgName=\"$p\" className=\"$c\" />"

    # Already so? Looked at without root first, so a repeat asks for nothing.
    pull_current
    case $? in
        0) cur=$(entries_for "$a" "$TMPD/now.xml")
           if [ "$cur" = "$(printf '%s\t%s\t%s' "$a" "$p" "$c")" ]; then
               say "$name already opens $p; nothing to change."
               return 0
           fi ;;
        1) cur="" ;;
        2) say "Could not read $file without root; will look again as root. The unit answered:"
           quote "$SAID"; cur="?" ;;
    esac

    say ""
    say "Source:  $name (appid $a) — $desc"
    case "$cur" in
        "") say "Now:     the stock app" ;;
        "?") say "Now:     (not readable yet)" ;;
        *) say "Now:     $(printf '%s\n' "$cur" | cut -f2 | paste -sd, -)" ;;
    esac
    say "New:     $p"
    say "         activity $c"
    case "$risk" in
        tested) ;;
        plausible) say "Note:    untried on a real unit for this source. 'reset $name' undoes it." ;;
        caution) say "Note:    untried, and this source is tied to vendor hardware an ordinary app
         may not drive. 'reset $name' undoes it." ;;
        force) say "Note:    THE REVERSE CAMERA. Test it parked before driving." ;;
    esac
    [ "$a" = 3 ] && say "Note:    this changes which app the Video source OPENS; it does not take Video
         out of the MODE cycle."
    confirm "Make the $name source open $p?"

    become_root
    make_writable
    must "could not create $app." on_unit "mkdir -p '$app' && chmod 755 '$app'"
    exists "$file"
    case $? in
        0)
            had=1
            must "could not read $file." "$ADB" -s "$T" pull "$file" "$TMPD/before.xml"
            familiar "$TMPD/before.xml" || refuse_unfamiliar "$TMPD/before.xml"
            if [ "$(entries_for "$a" "$TMPD/before.xml")" = "$(printf '%s\t%s\t%s' "$a" "$p" "$c")" ]; then
                say "$name already opens $p; nothing to write."
                reload; cleanup_device; return 0
            fi
            [ "$(entries_for "$a" "$TMPD/before.xml" | wc -l | tr -d ' ')" -gt 1 ] \
                && say "There were several $name entries; they become this one."
            rewrite "$a" "$item" "$TMPD/before.xml" "$TMPD/after.xml" ;;
        1) had=0; fresh "$item" "$TMPD/after.xml" ;;
        2) die "could not tell whether $file is there, even as root — nothing changed. The unit answered:" "$SAID" ;;
    esac
    keep_original "$had"
    write_file "$TMPD/after.xml"
    reload
    read_back "$TMPD/after.xml"
    say "Read back: $name now opens $p."
    items_of "$TMPD/check.xml" | describe_items
    cleanup_device
    say ""
    say "Done. The kernel uses it the next time the $name source is selected, and on wake"
    say "when $name was the source at power-off."
    if [ "$a" = 8 ] && [ "$p" = "$CARNYX" ]; then
        say "Keep Carnyx's 'Stop the radio when Carnyx closes' OFF: handing FM back at sleep"
        say "tells the kernel not to restore the radio at all."
    fi
}

# Restore the original file verbatim and drop the record.
restore_original() {
    local app=$CFG/app file=$CFG/app/$NAME
    must "could not restore $app/$BACKUP." on_unit "cp '$app/$BACKUP' '$file' && chmod 644 '$file' && rm -f '$app/$BACKUP'"
    say "Restored the original $NAME, byte for byte."
}
remove_ours() {
    local app=$CFG/app file=$CFG/app/$NAME marked=$1
    must "could not remove $file." on_unit "rm -f '$file' '$app/$ABSENT'"
    if [ "$marked" = 0 ]; then say "Removed $file — there was none before this script."
    else say "Removed $file — the entry just reset was the last one in it."; fi
}

do_reset() {
    local src=$1 app=$CFG/app file=$CFG/app/$NAME row a name b m f orig want
    if [ "$src" = all ]; then
        a=all; name="every source"
    else
        row=$(source_row "$src")
        [ -n "$row" ] || die "'$src' is not a source this script knows. Run 'sources' for the list."
        read -r a name _ <<< "$row"
    fi
    exists "$file"; f=$?
    if [ "$f" = 1 ] && [ "$a" != all ]; then
        say "There is no $file: $name already opens its stock app."
        return 0
    fi
    if [ "$f" = 0 ] && [ "$a" != all ] && pull_current && [ -z "$(entries_for "$a" "$TMPD/now.xml")" ] \
        && ! exists "$app/$BACKUP"; then
        say "$file has no $name entry, and there is no backup: $name already opens its stock app."
        return 0
    fi
    confirm "Put $name back the way the original file had it?"
    become_root
    exists "$file"; f=$?
    exists "$app/$BACKUP"; b=$?
    exists "$app/$ABSENT"; m=$?
    [ "$f" = 2 ] || [ "$b" = 2 ] || [ "$m" = 2 ] \
        && die "could not tell what is in $app, even as root. The unit answered:" "$SAID"
    make_writable

    if [ "$a" = all ]; then
        if [ "$b" = 0 ]; then restore_original
        elif [ "$m" = 0 ]; then remove_ours 0
        elif [ "$f" = 1 ]; then say "There is no $file and no record: nothing to reset."
        else die "there is no record of what $NAME held originally (no $BACKUP, no
$ABSENT), so 'reset all' cannot know what to put back. Reset sources one at a
time instead; each one's entry is then removed."
        fi
        reload; cleanup_device; return 0
    fi

    must "could not read $file." "$ADB" -s "$T" pull "$file" "$TMPD/before.xml"
    familiar "$TMPD/before.xml" || refuse_unfamiliar "$TMPD/before.xml"
    orig=""
    if [ "$b" = 0 ]; then
        must "could not read $app/$BACKUP." "$ADB" -s "$T" pull "$app/$BACKUP" "$TMPD/orig.xml"
        orig=$(raw_entry "$a" "$TMPD/orig.xml")
    fi
    rewrite "$a" "$orig" "$TMPD/before.xml" "$TMPD/after.xml"

    # Nothing differs from the original any more: put it back verbatim.
    if [ "$b" = 0 ] && [ "$(items_of "$TMPD/after.xml" | sort)" = "$(items_of "$TMPD/orig.xml" | sort)" ]; then
        restore_original
    elif [ "$(items_of "$TMPD/after.xml" | wc -l | tr -d ' ')" = 0 ] && [ "$b" != 0 ]; then
        remove_ours "$m"
    else
        write_file "$TMPD/after.xml"
        read_back "$TMPD/after.xml"
        if [ -n "$orig" ]; then say "$name is back to the original's entry."
        else say "Removed the $name entry; it opens its stock app."; fi
    fi
    reload
    cleanup_device
    say "Done."
}

usage() {
    sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
}

main() {
    local action=${1:-} src="" app="" target="" arg
    shift || true
    for arg in "$@"; do
        case "$arg" in
            -y | --yes) ASSUME_YES=1 ;;
            --force) FORCE=1 ;;
            -h | --help) usage; exit 0 ;;
            -*) say "Unknown option: $arg"; usage; exit 2 ;;
            *)
                if [[ "$arg" =~ $RX_IP ]]; then target=$arg
                elif [ -z "$src" ] && { [ "$action" = set ] || [ "$action" = reset ]; }; then src=$arg
                elif [ -z "$app" ] && [ "$action" = set ]; then app=$arg
                else say "Did not expect '$arg' here."; usage; exit 2
                fi ;;
        esac
    done
    case "$action" in
        sources) do_sources; exit 0 ;;
        find | list | apps) ;;
        set | reset) [ -n "$src" ] || { say "Which source? Run 'sources' for the list."; exit 2; } ;;
        *) usage; exit 2 ;;
    esac
    command -v "$ADB" >/dev/null 2>&1 \
        || { say "No adb here. In Termux: pkg install android-tools"; exit 1; }
    local v
    if ! v=$("$ADB" version 2>&1); then
        say "adb is installed but will not run. It answered:"
        quote "$v"
        say "In Termux, a 'CANNOT LINK EXECUTABLE' answer is a half-finished upgrade: run"
        say "'pkg upgrade', then 'adb version' again."
        exit 1
    fi
    TMPD=$(mktemp -d) || exit 1
    trap 'rm -rf "$TMPD"' EXIT

    if [ -n "$target" ]; then
        T=$(with_port "$target")
        say "Connecting to $T…"
        connect "$T" || { say "Could not connect to $T. adb's answers are above."; exit 1; }
    else
        find_unit || exit 1
    fi
    [ "$action" = find ] && exit 0
    [ "$action" = apps ] && { do_apps; exit 0; }

    facts
    case "$action" in
        list) do_list ;;
        set) do_set "$src" "$app" ;;
        reset) do_reset "$src" ;;
    esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main "$@"
fi
