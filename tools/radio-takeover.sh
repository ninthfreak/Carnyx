#!/usr/bin/env bash
# Install, check or undo Carnyx's radio takeover on the NWD head unit, over
# network ADB — from a phone (Termux) or a PC on the same Wi-Fi or hotspot.
#
#   bash radio-takeover.sh find                      which device is the head unit
#   bash radio-takeover.sh check   [IP[:PORT]]       what is installed; changes nothing
#   bash radio-takeover.sh install [IP[:PORT]] [-y]  make Carnyx the radio app
#   bash radio-takeover.sh undo    [IP[:PORT]] [-y]  put it back as it was
#
# With no IP it looks for the head unit itself (see `find_unit`).
#
# ── WHAT "THE RADIO TAKEOVER" IS ─────────────────────────────────────────────
#
# One line in `<config>/app/replace_source_list.xml` that tells the vendor
# kernel (`com.nwd.kernel`) to launch Carnyx wherever it would launch the stock
# radio app — #133's outcome A. The file, the kernel code that reads it and the
# reasons are in `docs/vendor/replace_source_list.xml` and
# `docs/vendor/README.md`. `<config>` is `ro.nwd.config.path`, `/config` on
# this unit.
#
# ── WHAT IT NEEDS, AND WHY NOTHING SMALLER WILL DO ──────────────────────────
#
# 1. ADB LISTENING ON THE NETWORK on the head unit — a developer-options switch,
#    or `adb tcpip 5555` once over USB. Android 10 has no wireless pairing.
# 2. `adb root`. `/config` is not writable by the factory copier, measured
#    2026-09-25 (README.md, "The route is closed on this unit"), and a plain
#    ADB shell is no more privileged than that. Only a ROM built `userdebug` or
#    `eng`, or with `ro.debuggable=1`, accepts `adb root`; `check` prints the
#    build and says which this is, and `install` stops cleanly if it is refused.
# 3. `adb` on the machine running this. In Termux: `pkg install android-tools`.
#
# ── WHAT IT DOES TO THE HEAD UNIT, IN ORDER ─────────────────────────────────
#
# install: `adb root` -> make `<config>` writable (a remount, only if it is its
# own read-only mount) -> create `<config>/app` -> MERGE the Carnyx entry into
# any existing file, keeping every other entry and backing the original up to
# `replace_source_list.xml.carnyx-backup` (the name `CarnyxRemap` uses) -> tell
# the kernel to reload (`com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE`) -> read the
# file back -> put the mount and adbd back as they were.
#
# undo: restore that backup if there is one; otherwise remove only the Carnyx
# entry, and the file with it if nothing else is left. Then reload.
#
# IT REFUSES rather than guesses: a file it does not recognise, a radio entry
# split over several lines, a `<config>` that is part of the read-only system
# image, Carnyx not installed, or root refused — each stops with a sentence
# saying so, before anything is written.
#
# ── TESTED ON THE HOST, NOT ON THE UNIT ─────────────────────────────────────
#
# `tools/test-radio-takeover.sh` runs every branch against a fake `adb` that
# keeps a fake `/config`. What no host test can settle is the head unit itself:
# whether its ADB listens, whether root is allowed, and how `/config` is
# mounted. The first two are what `check` reports.

set -uo pipefail

PKG=com.ninthfreak.carnyx
CLS=android.app.NativeActivity
NAME=replace_source_list.xml
BACKUP=$NAME.carnyx-backup
RELOAD=com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE
KERNEL=com.nwd.kernel
DEFAULT_PORT=5555
ITEM="<ReplaceSourceItem appid=\"8\" pkgName=\"$PKG\" className=\"$CLS\" />"
ADB=${ADB:-adb}

# Extended regexes. The radio's entry, anyone's: `appid="8"`, either quote, any
# spacing. And Carnyx's entry, whatever its appid or attribute order.
RX_RADIO="<ReplaceSourceItem[^>]*appid[[:space:]]*=[[:space:]]*[\"']8[\"'][^>]*/>"
RX_OURS="<ReplaceSourceItem[^>]*pkgName[[:space:]]*=[[:space:]]*[\"']${PKG//./\\.}[\"'][^>]*/>"

T=""          # the head unit, as host:port
CFG=""        # ro.nwd.config.path
ROOTED=0      # this run asked for adb root
REMOUNTED=0   # this run remounted <config> read-write
ASSUME_YES=0
TMPD=""

say() { printf '%s\n' "$*"; }
die() {
    printf '\nSTOPPED: %s\n' "$*" >&2
    cleanup_device
    exit 1
}

# One command on the head unit, its \r stripped (older adb clients add one).
# Its exit status is the remote command's — adb's shell protocol v2 carries it,
# and both Android 10 and any current adb client speak v2.
on_unit() { "$ADB" -s "$T" shell "$1" | tr -d '\r'; }

# ── Connecting ───────────────────────────────────────────────────────────────

with_port() {
    case "$1" in
        *:*) printf '%s' "$1" ;;
        *) printf '%s:%s' "$1" "$DEFAULT_PORT" ;;
    esac
}

# Connect and wait for the unit to say "device". `unauthorized` is the head
# unit showing its "Allow USB debugging?" prompt, which only a tap can answer.
connect() {
    local target=$1 tries=0 state told=0
    "$ADB" connect "$target" >/dev/null 2>&1
    while :; do
        state=$("$ADB" -s "$target" get-state 2>&1 | tr -d '\r')
        case "$state" in
            device) return 0 ;;
            *unauthorized*)
                if [ "$told" = 0 ]; then
                    say "  $target is asking for permission — tap Allow on the head unit's screen."
                    told=1
                fi
                ;;
        esac
        tries=$((tries + 1))
        [ "$tries" -ge "${CONNECT_TRIES:-30}" ] && return 1
        sleep "${CONNECT_PAUSE:-1}"
        "$ADB" connect "$target" >/dev/null 2>&1
    done
}

# ── Finding the head unit ───────────────────────────────────────────────────

# This machine's Wi-Fi and Ethernet networks, as "a.b.c own-address" lines.
#
# ONLY LOCAL WI-FI AND ETHERNET, and only private addresses. A phone running a
# hotspot also has a cellular interface (`rmnet…`, `ccmni…`), often with a
# private-looking carrier address; scanning that would be knocking on other
# people's devices. So interfaces are kept by name — the hotspot is `ap0`,
# `swlan0`, `softap0` or `wlanN` depending on the phone — and addresses by
# range.
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

# Every address on a.b.c.0/24 that accepts a TCP connection on `port`.
#
# A /24 EVEN WHEN THE NETWORK IS SMALLER — an iPhone hotspot is a /28 — because
# that covers it and a phone does not reliably report its mask to an app.
# Sixty-four at a time, one second each: the whole range in about four seconds.
scan() {
    local prefix=$1 own=$2 port=$3 n
    for n in $(seq 1 254); do
        [ "$prefix.$n" = "$own" ] && continue
        ( timeout 1 bash -c "exec 3<>/dev/tcp/$prefix.$n/$port" 2>/dev/null && echo "$prefix.$n" ) &
        if [ $((n % 64)) = 0 ]; then wait; fi
    done
    wait
}

# The head unit, found: something answering on ADB's port that has the NWD
# kernel installed. Anything else that answers is listed and left alone — this
# script asks for root, and it does not ask a device it cannot identify.
find_unit() {
    local port=${1:-$DEFAULT_PORT} prefix own h
    local -a hits=() units=() others=()
    while read -r prefix own; do
        [ -n "$prefix" ] || continue
        say "Looking for ADB on $prefix.1-254 (this device is $own)…"
        while read -r h; do
            [ -n "$h" ] && hits+=("$h")
        done < <(scan "$prefix" "$own" "$port")
    done < <(local_prefixes)
    if [ "${#hits[@]}" = 0 ]; then
        say "Nothing on this network is listening for ADB on port $port."
        say "Check the head unit is on this Wi-Fi and has network ADB switched on, or pass"
        say "its address: bash radio-takeover.sh check 192.168.x.y"
        return 1
    fi
    for h in "${hits[@]}"; do
        if connect "$h:$port" && "$ADB" -s "$h:$port" shell "pm path $KERNEL" 2>/dev/null | tr -d '\r' | grep -q '^package:'; then
            units+=("$h:$port")
        else
            others+=("$h:$port")
            "$ADB" disconnect "$h:$port" >/dev/null 2>&1
        fi
    done
    for h in "${others[@]}"; do say "  $h answers ADB but is not an NWD head unit (or did not allow this computer)."; done
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
        say "  adb root will be refused on a build of this kind — the takeover cannot be installed this way."
    fi
    say "config: $CFG (ro.nwd.config.path)"
}

carnyx_installed() { on_unit "pm path $PKG" 2>/dev/null | grep -q '^package:'; }

# ── Root, and a writable <config> ───────────────────────────────────────────

become_root() {
    local out
    if [ "$(on_unit 'id -u')" = 0 ]; then return 0; fi
    say "Asking for adb root…"
    out=$("$ADB" -s "$T" root 2>&1 | tr -d '\r')
    case "$out" in
        *"cannot run as root"* | *"not permitted"* | *"unable"*)
            die "the head unit refused adb root: \"$out\". This ROM is a production build, and
without root the takeover cannot be written (README.md: /config is not writable otherwise)." ;;
    esac
    ROOTED=1
    # adbd restarts as root, which drops the network connection.
    sleep "${ROOT_PAUSE:-2}"
    connect "$T" || die "lost the head unit after adb root — run the command again."
    if [ "$(on_unit 'id -u')" != 0 ]; then
        die "adb root answered \"$out\", but the shell is still not root."
    fi
}

writable() { on_unit "touch '$CFG/.carnyx-write-test' && rm -f '$CFG/.carnyx-write-test'" >/dev/null 2>&1; }

# Writable already, or its own read-only mount that can be remounted. NEVER the
# system image: a `<config>` that is a plain directory on a read-only root needs
# dm-verity disabled and a reboot, which is a decision for a person, not a script.
make_writable() {
    local mnt
    writable && return 0
    mnt=$(on_unit 'cat /proc/mounts' | awk -v d="$CFG" '$2 == d')
    if [ -z "$mnt" ]; then
        die "$CFG is not writable, and it is not a mount of its own: it is part of the
read-only system image. Making that writable means 'adb disable-verity', a reboot
and 'adb remount'. This script will not do that on its own; ask first."
    fi
    say "$CFG is mounted read-only — remounting it read-write for this change."
    say "  ($mnt)"
    on_unit "mount -o rw,remount '$CFG'" >/dev/null 2>&1 || die "the remount of $CFG failed."
    REMOUNTED=1
    writable || die "$CFG was remounted and is still not writable."
}

# Put the unit back the way this run found it: the mount read-only again if it
# was, and adbd unprivileged again if this run raised it.
cleanup_device() {
    if [ "$REMOUNTED" = 1 ]; then
        if on_unit "mount -o ro,remount '$CFG'" >/dev/null 2>&1; then
            say "$CFG remounted read-only again."
        else
            say "Could not remount $CFG read-only again; a reboot will."
        fi
        REMOUNTED=0
    fi
    if [ "$ROOTED" = 1 ]; then
        "$ADB" -s "$T" unroot >/dev/null 2>&1 && say "adbd is unprivileged again."
        ROOTED=0
    fi
}

# ── The file ────────────────────────────────────────────────────────────────

reload() {
    if on_unit "am broadcast -a $RELOAD" >/dev/null 2>&1; then
        say "Told the kernel to reload the list."
    else
        say "The reload broadcast failed; a reboot will load the file instead."
    fi
}

# The text with newlines folded to spaces, so an element is one match even when
# its attributes are spread over several lines.
folded() { tr '\n' ' ' < "$1"; }
count() { folded "$2" | grep -oE "$1" | wc -l | tr -d ' '; }
has_ours() { folded "$1" | grep -oE "$RX_RADIO" | grep -qE "pkgName[[:space:]]*=[[:space:]]*[\"']${PKG//./\\.}[\"']"; }

# The existing file with the radio's entry replaced by Carnyx's. Line by line,
# so every other entry, comment and byte of layout survives — and checked
# afterwards, so a shape this cannot edit safely is refused rather than half-done.
merge() {
    local in=$1 out=$2
    if ! grep -q '</ReplaceSourceList>' "$in"; then
        die "the existing $NAME has no </ReplaceSourceList>. It is not a shape this
script knows, so it is left exactly as it is. Its contents:
$(sed 's/^/  /' "$in")"
    fi
    # THROUGH THE ENVIRONMENT, NOT `-v`: awk processes backslash escapes in a
    # `-v` value, which would turn RX_OURS's `\.` into "any character".
    RX="$RX_RADIO" ITEM="$ITEM" awk '
        { line = $0
          if (gsub(ENVIRON["RX"], "", line) > 0 && line ~ /^[ \t]*$/) next
          if (!done && sub(/<\/ReplaceSourceList>/, "    " ENVIRON["ITEM"] "\n</ReplaceSourceList>", line)) done = 1
          print line }' "$in" > "$out"
    if [ "$(count "$RX_RADIO" "$out")" != 1 ] || ! has_ours "$out"; then
        die "the existing $NAME has a radio entry this script cannot edit safely
(probably split across lines). Nothing was changed. Its contents:
$(sed 's/^/  /' "$in")"
    fi
}

fresh() {
    cat > "$1" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<!-- Carnyx's radio takeover: the kernel launches Carnyx for the radio source (appid 8).
     Written by tools/radio-takeover.sh; see docs/vendor/replace_source_list.xml. -->
<ReplaceSourceList>
    $ITEM
</ReplaceSourceList>
EOF
}

confirm() {
    [ "$ASSUME_YES" = 1 ] && return 0
    printf '%s [y/N] ' "$1"
    local answer
    read -r answer || answer=""
    case "$answer" in y | Y | yes | YES) return 0 ;; *) say "Nothing changed."; cleanup_device; exit 0 ;; esac
}

# ── The four commands ───────────────────────────────────────────────────────

do_check() {
    local file=$CFG/app/$NAME
    if carnyx_installed; then say "Carnyx: installed."; else say "Carnyx: NOT installed on the head unit."; fi
    if on_unit "test -f '$file'" >/dev/null 2>&1; then
        say "$file:"
        on_unit "cat '$file'" | sed 's/^/  /'
        "$ADB" -s "$T" pull "$file" "$TMPD/now.xml" >/dev/null 2>&1
        if [ -s "$TMPD/now.xml" ] && has_ours "$TMPD/now.xml"; then
            say "=> The takeover IS installed: the kernel will launch Carnyx for the radio."
        else
            say "=> The file is there, but it does not send the radio to Carnyx."
        fi
    else
        say "$file: absent — the takeover is not installed."
    fi
    if on_unit "test -f '$CFG/app/$BACKUP'" >/dev/null 2>&1; then
        say "A backup of the file as it was before Carnyx is at $CFG/app/$BACKUP."
    fi
}

do_install() {
    local app=$CFG/app file=$CFG/app/$NAME
    carnyx_installed || die "Carnyx is not installed on the head unit. The takeover would point
the radio at an app that is not there; install Carnyx first."
    # ALREADY THERE? Reading needs no root, so a second run asks for nothing.
    if "$ADB" -s "$T" pull "$file" "$TMPD/now.xml" >/dev/null 2>&1 && has_ours "$TMPD/now.xml"; then
        say "The takeover is already installed; nothing to change."
        reload
        return 0
    fi
    confirm "Make Carnyx the head unit's radio app by editing $file?"
    become_root
    make_writable
    on_unit "mkdir -p '$app' && chmod 755 '$app'" >/dev/null 2>&1 || die "could not create $app."
    if on_unit "test -f '$file'" >/dev/null 2>&1; then
        "$ADB" -s "$T" pull "$file" "$TMPD/before.xml" >/dev/null 2>&1 || die "could not read $file."
        if has_ours "$TMPD/before.xml"; then
            say "The takeover is already in the file; nothing to write."
            reload
            cleanup_device
            return 0
        fi
        merge "$TMPD/before.xml" "$TMPD/after.xml"
        if ! on_unit "test -f '$app/$BACKUP'" >/dev/null 2>&1; then
            on_unit "cp '$file' '$app/$BACKUP'" >/dev/null 2>&1 || die "could not back up $file — nothing changed."
            say "Backed the original up to $app/$BACKUP."
        fi
    else
        fresh "$TMPD/after.xml"
    fi
    "$ADB" -s "$T" push "$TMPD/after.xml" "$file" >/dev/null 2>&1 || die "could not write $file."
    on_unit "chmod 644 '$file'" >/dev/null 2>&1
    reload
    "$ADB" -s "$T" pull "$file" "$TMPD/check.xml" >/dev/null 2>&1
    if [ -s "$TMPD/check.xml" ] && has_ours "$TMPD/check.xml"; then
        say "Read back: the takeover is installed."
        sed 's/^/  /' "$TMPD/check.xml"
    else
        die "the file was written but does not read back with the Carnyx entry."
    fi
    cleanup_device
    say ""
    say "Done. Keep Carnyx's 'Stop the radio when Carnyx closes' OFF (the default):"
    say "handing FM back at sleep tells the kernel not to restore the radio at all."
    say "Then switch network ADB off on the head unit again."
}

do_undo() {
    local app=$CFG/app file=$CFG/app/$NAME
    if ! on_unit "test -f '$file' || test -f '$app/$BACKUP'" >/dev/null 2>&1; then
        say "There is no $file and no backup: nothing to undo."
        return 0
    fi
    confirm "Remove Carnyx's radio takeover from $file?"
    become_root
    make_writable
    if on_unit "test -f '$app/$BACKUP'" >/dev/null 2>&1; then
        on_unit "cp '$app/$BACKUP' '$file' && rm -f '$app/$BACKUP'" >/dev/null 2>&1 \
            || die "could not restore $app/$BACKUP."
        say "Restored the original $NAME from the backup."
    else
        "$ADB" -s "$T" pull "$file" "$TMPD/before.xml" >/dev/null 2>&1 || die "could not read $file."
        if ! has_ours "$TMPD/before.xml"; then
            say "$file has no Carnyx entry; left as it is."
        else
            RX="$RX_OURS" awk '
                { line = $0
                  if (gsub(ENVIRON["RX"], "", line) > 0 && line ~ /^[ \t]*$/) next
                  print line }' "$TMPD/before.xml" > "$TMPD/after.xml"
            if [ "$(count '<ReplaceSourceItem' "$TMPD/after.xml")" = 0 ]; then
                on_unit "rm -f '$file'" >/dev/null 2>&1 || die "could not remove $file."
                say "Removed $file — Carnyx's entry was all it held."
            else
                "$ADB" -s "$T" push "$TMPD/after.xml" "$file" >/dev/null 2>&1 || die "could not write $file."
                on_unit "chmod 644 '$file'" >/dev/null 2>&1
                say "Removed Carnyx's entry and kept the rest."
            fi
        fi
    fi
    reload
    cleanup_device
    say "Done. The stock radio app is the radio again from the next wake."
}

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
}

main() {
    local action=${1:-} target=""
    shift || true
    local arg
    for arg in "$@"; do
        case "$arg" in
            -y | --yes) ASSUME_YES=1 ;;
            *) target=$arg ;;
        esac
    done
    case "$action" in
        find | check | install | undo) ;;
        *) usage; exit 2 ;;
    esac
    command -v "$ADB" >/dev/null 2>&1 \
        || { say "No adb here. In Termux: pkg install android-tools"; exit 1; }
    TMPD=$(mktemp -d) || exit 1
    trap 'rm -rf "$TMPD"' EXIT

    if [ -n "$target" ]; then
        T=$(with_port "$target")
        say "Connecting to $T…"
        connect "$T" || { say "Could not connect to $T (is network ADB on, and did the unit allow it?)."; exit 1; }
    else
        find_unit || exit 1
    fi
    [ "$action" = find ] && exit 0

    facts
    case "$action" in
        check) do_check ;;
        install) do_install ;;
        undo) do_undo ;;
    esac
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main "$@"
fi
