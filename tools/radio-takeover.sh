#!/usr/bin/env bash
# Install, check or undo Carnyx's radio takeover on the NWD head unit, over
# network ADB — from a phone (Termux) or a PC on the same Wi-Fi or hotspot.
#
#   bash radio-takeover.sh find                      which device is the head unit
#   bash radio-takeover.sh check   [IP[:PORT]]       what is installed; changes nothing
#   bash radio-takeover.sh install [IP[:PORT]] [-y]  make Carnyx the radio app
#   bash radio-takeover.sh undo    [IP[:PORT]] [-y]  put it back as it was
#
# With no IP it looks for the head unit itself (see `find_unit`). Lines that
# start with "|" are adb's or the head unit's own words, unedited.
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
# AND IT SAYS WHY, in adb's words: every failure prints what adb or the unit
# answered, unedited, under the sentence it explains. Output is thrown away in
# three places only, each because failing there is the expected answer: the
# port scan (a closed port on 253 addresses), the interface lookup (`ip` and
# `ifconfig` stand in for each other, and both are shown when neither finds an
# address) and letting go of a device that is not the unit.
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

SAID=""       # why the last `exists` or `has_package` could not tell
WRITE_SAID="" # what the last `writable` test answered

say() { printf '%s\n' "$*"; }

# adb's or the head unit's own words, unedited, under the line they explain.
quote() {
    if [ -n "$1" ]; then printf '%s\n' "$1" | sed 's/^/  | /'; else say "  | (said nothing)"; fi
}

# Stop, saying why — and, given them, the words adb or the unit answered.
die() {
    printf '\nSTOPPED: %s\n' "$1" >&2
    if [ "$#" -gt 1 ]; then quote "$2" >&2; fi
    cleanup_device
    exit 1
}

# An adb command that has to work. When it does not, the run stops with `what`
# and every word adb or the unit said: the reason is in those, not in `what`.
must() {
    local what=$1 out rc
    shift
    out=$("$@" 2>&1 | tr -d '\r')
    rc=$?
    [ "$rc" = 0 ] && return 0
    die "$what" "${out:-(no message; exit status $rc)}"
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
#
# ADB'S ANSWERS ARE PRINTED AS THEY CHANGE. What `adb connect` and
# `adb get-state` say is the only account of why a wait is a wait, and all there
# is to go on when it ends without a connection — so none of it is kept back.
# ("already connected to X" counts as no change from "connected to X".)
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

# Done with a device that is not the unit. Its answer is not wanted: it may
# already be gone, and either way there is nothing to do about it.
let_go() { "$ADB" disconnect "$1" >/dev/null 2>&1; }

# ── Finding the head unit ───────────────────────────────────────────────────

# This machine's Wi-Fi and Ethernet networks, as "a.b.c own-address" lines.
#
# ONLY LOCAL WI-FI AND ETHERNET, and only private addresses. A phone running a
# hotspot also has a cellular interface (`rmnet…`, `ccmni…`), often with a
# private-looking carrier address; scanning that would be knocking on other
# people's devices. So interfaces are kept by name — the hotspot is `ap0`,
# `swlan0`, `softap0` or `wlanN` depending on the phone — and addresses by
# range.
#
# EITHER TOOL MAY BE MISSING OR BLOCKED, which is why both are asked and their
# complaints dropped here; `find_unit` prints both in full when neither yields
# an address.
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
# A refused or silent port is the expected answer from nearly every address, so
# those are not reported.
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
# kernel installed. Anything else that answers is left alone, with the reason —
# this script asks for root, and it does not ask a device it cannot identify.
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
        say "Pass the head unit's address instead: bash radio-takeover.sh check 192.168.x.y"
        return 1
    fi
    if [ "${#hits[@]}" = 0 ]; then
        say "Nothing on this network is listening for ADB on port $port."
        say "Check the head unit is on this Wi-Fi and has network ADB switched on, or pass"
        say "its address: bash radio-takeover.sh check 192.168.x.y"
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
        say "  adb root will be refused on a build of this kind — the takeover cannot be installed this way."
    fi
    say "config: $CFG (ro.nwd.config.path)"
}

# Whether a package is installed on the unit (`$T`, or the serial given):
# 0 yes, 1 no, 2 could not tell — and then SAID holds why. "No" only when the
# unit's own shell ran `pm path` and it printed nothing, which is how Android 10
# answers for a package it does not have; a dropped connection, or `pm`
# complaining, is not an answer.
has_package() {
    local raw
    raw=$("$ADB" -s "${2:-$T}" shell "pm path $1; echo ':asked'" 2>&1 | tr -d '\r')
    SAID=$(printf '%s\n' "$raw" | grep -vx ':asked')
    if printf '%s\n' "$raw" | grep -q '^package:'; then return 0; fi
    [ "$raw" = ":asked" ] && return 1
    return 2
}

# Whether a path is on the unit: 0 yes, 1 no, 2 could not tell — and then SAID
# holds why. "No" only when the unit itself said "No such file or directory": a
# directory the shell may not look into, or a dropped connection, is not an
# answer, and reading either as "absent" is how a wrong report gets made.
exists() {
    local rc
    SAID=$(on_unit "ls -d '$1'" 2>&1)
    rc=$?
    case "$SAID" in *"No such file or directory"*) return 1 ;; esac
    [ "$rc" = 0 ] && return 0
    return 2
}

# ── Root, and a writable <config> ───────────────────────────────────────────

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
build, and without root the takeover cannot be written (README.md: /config is
not writable otherwise)." ;;
    esac
    [ "$rc" = 0 ] || die "adb root failed with exit status $rc; its answer is above."
    ROOTED=1
    # adbd restarts as root, which drops the network connection.
    sleep "${ROOT_PAUSE:-2}"
    connect "$T" || die "lost the head unit after adb root (adb's answers are above) — run the command again."
    uid=$(on_unit 'id -u' 2>&1)
    if [ "$uid" != 0 ]; then
        die "adb root answered as above, but the shell is still not root. 'id -u' said:" "$uid"
    fi
}

# WRITE_SAID keeps the answer, for the sentence that explains a refusal.
writable() { WRITE_SAID=$(on_unit "touch '$CFG/.carnyx-write-test' && rm -f '$CFG/.carnyx-write-test'" 2>&1); }

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

# Put the unit back the way this run found it: the mount read-only again if it
# was, and adbd unprivileged again if this run raised it.
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

# ── The file ────────────────────────────────────────────────────────────────

reload() {
    local out
    if out=$(on_unit "am broadcast -a $RELOAD" 2>&1); then
        say "Told the kernel to reload the list."
    else
        say "The reload broadcast failed; a reboot will load the file instead. The unit answered:"
        quote "$out"
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
    local file=$CFG/app/$NAME out seen
    has_package "$PKG"
    case $? in
        0) say "Carnyx: installed." ;;
        1) say "Carnyx: NOT installed on the head unit." ;;
        2) say "Carnyx: could not tell whether it is installed. 'pm path $PKG' answered:"; quote "$SAID" ;;
    esac
    exists "$file"
    seen=$?
    case $seen in
        0)
            if out=$("$ADB" -s "$T" pull "$file" "$TMPD/now.xml" 2>&1 | tr -d '\r'); then
                say "$file:"
                sed 's/^/  /' "$TMPD/now.xml"
                if has_ours "$TMPD/now.xml"; then
                    say "=> The takeover IS installed: the kernel will launch Carnyx for the radio."
                else
                    say "=> The file is there, but it does not send the radio to Carnyx."
                fi
            else
                say "$file is there, but reading it failed. adb answered:"
                quote "$out"
            fi ;;
        1) say "$file: absent — the takeover is not installed." ;;
        2) say "$file: could not tell whether it is there (check asks for no root). The unit answered:"
           quote "$SAID" ;;
    esac
    # The backup sits beside the file: when the file could not be looked for,
    # neither can it, and that answer was just printed.
    if [ "$seen" != 2 ]; then
        exists "$CFG/app/$BACKUP"
        case $? in
            0) say "A backup of the file as it was before Carnyx is at $CFG/app/$BACKUP." ;;
            2) say "Could not tell whether $CFG/app/$BACKUP is there. The unit answered:"; quote "$SAID" ;;
        esac
    fi
}

do_install() {
    local app=$CFG/app file=$CFG/app/$NAME out
    has_package "$PKG"
    case $? in
        1) die "Carnyx is not installed on the head unit. The takeover would point
the radio at an app that is not there; install Carnyx first." ;;
        2) die "could not tell whether Carnyx is installed on the head unit. 'pm path $PKG' answered:" "$SAID" ;;
    esac
    # ALREADY THERE? Looked for without root first, so a second run asks for
    # nothing — where an unprivileged shell may read <config>. Where it may not,
    # that is said, and the look is made again as root below.
    exists "$file"
    case $? in
        0)
            if out=$("$ADB" -s "$T" pull "$file" "$TMPD/now.xml" 2>&1 | tr -d '\r'); then
                if has_ours "$TMPD/now.xml"; then
                    say "The takeover is already installed; nothing to change."
                    reload
                    return 0
                fi
            else
                say "Could not read $file without root; will look again as root. adb answered:"
                quote "$out"
            fi ;;
        2) say "Could not see whether $file is there without root; will look again as root. The unit answered:"
           quote "$SAID" ;;
    esac
    confirm "Make Carnyx the head unit's radio app by editing $file?"
    become_root
    make_writable
    must "could not create $app." on_unit "mkdir -p '$app' && chmod 755 '$app'"
    exists "$file"
    case $? in
        0)
            must "could not read $file." "$ADB" -s "$T" pull "$file" "$TMPD/before.xml"
            if has_ours "$TMPD/before.xml"; then
                say "The takeover is already in the file; nothing to write."
                reload
                cleanup_device
                return 0
            fi
            merge "$TMPD/before.xml" "$TMPD/after.xml"
            exists "$app/$BACKUP"
            case $? in
                1)
                    must "could not back up $file — nothing changed." on_unit "cp '$file' '$app/$BACKUP'"
                    say "Backed the original up to $app/$BACKUP." ;;
                2) die "could not tell whether $app/$BACKUP is there, even as root — nothing changed. The unit answered:" "$SAID" ;;
            esac ;;
        1) fresh "$TMPD/after.xml" ;;
        2) die "could not tell whether $file is there, even as root — nothing changed. The unit answered:" "$SAID" ;;
    esac
    must "could not write $file." "$ADB" -s "$T" push "$TMPD/after.xml" "$file"
    must "wrote $file, but could not make it readable (chmod 644), so the kernel may not read it." \
        on_unit "chmod 644 '$file'"
    reload
    must "wrote $file, but could not read it back." "$ADB" -s "$T" pull "$file" "$TMPD/check.xml"
    if has_ours "$TMPD/check.xml"; then
        say "Read back: the takeover is installed."
        sed 's/^/  /' "$TMPD/check.xml"
    else
        die "the file was written but does not read back with the Carnyx entry. It reads:" \
            "$(cat "$TMPD/check.xml")"
    fi
    cleanup_device
    say ""
    say "Done. Keep Carnyx's 'Stop the radio when Carnyx closes' OFF (the default):"
    say "handing FM back at sleep tells the kernel not to restore the radio at all."
    say "Then switch network ADB off on the head unit again."
}

do_undo() {
    local app=$CFG/app file=$CFG/app/$NAME f b
    # Looked for without root first, so "nothing to undo" asks for nothing.
    exists "$file"
    f=$?
    [ "$f" = 2 ] && { say "Could not see whether $file is there without root. The unit answered:"; quote "$SAID"; }
    exists "$app/$BACKUP"
    b=$?
    [ "$b" = 2 ] && { say "Could not see whether $app/$BACKUP is there without root. The unit answered:"; quote "$SAID"; }
    if [ "$f" = 1 ] && [ "$b" = 1 ]; then
        say "There is no $file and no backup: nothing to undo."
        return 0
    fi
    confirm "Remove Carnyx's radio takeover from $file?"
    become_root
    exists "$app/$BACKUP"
    b=$?
    [ "$b" = 2 ] && die "could not tell whether $app/$BACKUP is there, even as root. The unit answered:" "$SAID"
    exists "$file"
    f=$?
    [ "$f" = 2 ] && die "could not tell whether $file is there, even as root. The unit answered:" "$SAID"
    if [ "$f" = 1 ] && [ "$b" = 1 ]; then
        say "There is no $file and no backup: nothing to undo."
        cleanup_device
        return 0
    fi
    make_writable
    if [ "$b" = 0 ]; then
        must "could not restore $app/$BACKUP." on_unit "cp '$app/$BACKUP' '$file' && rm -f '$app/$BACKUP'"
        say "Restored the original $NAME from the backup."
    else
        must "could not read $file." "$ADB" -s "$T" pull "$file" "$TMPD/before.xml"
        if ! has_ours "$TMPD/before.xml"; then
            say "$file has no Carnyx entry; left as it is."
        else
            RX="$RX_OURS" awk '
                { line = $0
                  if (gsub(ENVIRON["RX"], "", line) > 0 && line ~ /^[ \t]*$/) next
                  print line }' "$TMPD/before.xml" > "$TMPD/after.xml"
            if [ "$(count '<ReplaceSourceItem' "$TMPD/after.xml")" = 0 ]; then
                must "could not remove $file." on_unit "rm -f '$file'"
                say "Removed $file — Carnyx's entry was all it held."
            else
                must "could not write $file." "$ADB" -s "$T" push "$TMPD/after.xml" "$file"
                must "wrote $file, but could not make it readable (chmod 644), so the kernel may not read it." \
                    on_unit "chmod 644 '$file'"
                say "Removed Carnyx's entry and kept the rest."
            fi
        fi
    fi
    reload
    cleanup_device
    say "Done. The stock radio app is the radio again from the next wake."
}

usage() {
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
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
        connect "$T" || { say "Could not connect to $T. adb's answers are above."; exit 1; }
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
