# Factory XML handoff: the kernel source remap

A self-contained brief for CREATING OR ALTERING the NWD head unit's source-remap
file — `replace_source_list.xml` — to make the kernel launch a chosen app in
place of a stock one. Written so a fresh session, or a person at a laptop, can
act from this page alone. Everything here is drawn from the firmware decompile
and one real run on the unit (2026-11-12); where something is NOT yet measured it
says so.

This is operator tooling and research. It is NOT part of the Carnyx APK — the
in-app installer was removed (#137). The route is an external ADB step.

## 0. What this lets you do, and what it does not

The head unit's kernel (`com.nwd.kernel`) decides which Android app owns each
audio/video "source" (radio, music, video, …). A single XML file can override
that: for any source you list, the kernel launches YOUR package instead of the
stock one when it starts that source's app. That is proven for the power-up
restore; whether the mode button or the launcher's tiles go through the same
`startApp` path has NOT been checked.

- IT CHANGES WHICH APP a source launches. The radio source pointed at Carnyx is
  proven on this unit — the radio comes up as Carnyx on wake.
- IT DOES NOT change Android's own defaults (home screen, browser, dialer). Those
  are set in Android Settings ▸ Apps ▸ Default apps, not here.
- IT DOES NOT decide WHETHER a source is restored at all on wake — see §8, the
  release-on-sleep interaction.

Only the radio source (appid 8) has actually been exercised. Every other entry in
§4 is read from the kernel's constant table and is UNTRIED. Treat a first remap
of any other source as an experiment, and keep the backup (§7).

## 1. What the kernel does with the file

Read off `com.nwd.kernel_v210`:

    SourceMgr.startApp(SourceItem item):
      if (mReplaceSourceList.isContain(item.getAppid())) {
          item.setPackageName(findPkgNameByAppid(appid));   // from the XML
          item.setClassName(findClassNameByAppid(appid));   // from the XML
      }
      item.setComponent(new ComponentName(pkg, cls));
      item.addFlags(FLAG_ACTIVITY_NEW_TASK);
      startActivity(item);            // on failure: getLaunchIntentForPackage(pkg)

`ReplaceSourceList` loads the file (`loadConfig`) and reloads it live on a
broadcast (§8). For any appid present in the file, it swaps in the package and class you
gave before the kernel launches. A source NOT in the file is launched as the
factory configured it. The stock radio app is a passive MAIN/LAUNCHER-only target
with no services or receivers, so once the kernel stops pointing at it, it is
never seen.

## 2. Where the file lives

    ReplaceSourceList.CONFIG = getConfigPath() + "/app/replace_source_list.xml"
    getConfigPath()          = SystemProperties("ro.nwd.config.path", "/config")

On THIS unit `ro.nwd.config.path` is **`/data/nwdappconfig`** (measured
2026-11-12), so the file is:

    /data/nwdappconfig/app/replace_source_list.xml

`/config` is only the firmware default. ALWAYS read the property on the unit you
are working on rather than assuming — `tools/radio-takeover.sh` does this on every
run, and manually it is:

    adb shell getprop ro.nwd.config.path

## 3. The file format

One `<ReplaceSourceItem>` per remapped source, inside one `<ReplaceSourceList>`:

    <?xml version="1.0" encoding="utf-8"?>
    <ReplaceSourceList>
        <ReplaceSourceItem appid="8"  pkgName="com.ninthfreak.carnyx" className="android.app.NativeActivity" />
        <ReplaceSourceItem appid="2"  pkgName="com.example.player"    className="com.example.player.MainActivity" />
    </ReplaceSourceList>

Three attributes per item:

- **`appid`** — which source to override. The full table is §4. Radio is 8.
- **`pkgName`** — the replacement app's package name. THIS IS THE LOAD-BEARING
  FIELD; it has to be exactly right.
- **`className`** — the app's launcher activity. NOT load-bearing: if it fails to
  resolve, `startApp` falls back to `getLaunchIntentForPackage(pkgName)`, i.e. it
  launches the package's declared launcher activity anyway. Get it close; the
  package is what matters.

`loadConfig` reads only those three attributes per item; comments and layout in
the file mean nothing to the kernel. The script edits line by line and keeps
them, and a hand edit should too — then the backup (§7) is a byte-exact undo.

## 4. The appid table

Read from `com.nwd.kernel.source.SourceConstant` (`SourceConstant.smali`):

| appid | Constant | Source |
|------:|----------|--------|
| 1   | APPID_GPS              | GPS / navigation |
| 2   | APPID_MUSIC            | Music |
| 3   | APPID_VIDEO            | Video |
| 4   | APPID_LAUNCHER         | Launcher (what the kernel opens on wake when it restores no source) |
| 5   | APPID_DVD              | DVD |
| 6   | APPID_IPOD             | iPod |
| 7   | APPID_CDC              | CD changer |
| 8   | APPID_RADIO            | **Radio (tested — Carnyx)** |
| 9   | APPID_AUX              | AUX in |
| 10  | APPID_BT               | Bluetooth |
| 11  | APPID_TV               | TV |
| 12  | APPID_TPMS             | Tire-pressure monitor |
| 13  | APPID_CAMERA           | Camera (includes reverse camera) |
| 14  | APPID_SETTING          | Settings |
| 15  | APPID_BT_MUSIC         | Bluetooth music |
| 16  | APPID_DVD_SD           | DVD (SD) |
| 17  | APPID_DVD_USB          | DVD (USB) |
| 18  | APPID_DVR              | Dash cam / DVR |
| 19  | APPID_PHONE            | Phone |
| 26  | APPID_CAR_SETTING      | Car settings |
| 27  | APPID_SCREEN_PROTECT   | Screen protect |
| 28  | APPID_TSCALIBRATION    | Touch calibration |
| 29  | APPID_NWD_PHONE        | NWD phone |
| 31  | APPID_AUX_REAR         | Rear AUX |
| 227 | APPID_USBLINK          | USB link / phone mirror |
| 228 | APPID_MHL              | MHL |
| 229 | APPID_CAN_AUX          | CAN AUX |
| 230 | APPID_CAN_CDC          | CAN CD changer |
| 231 | APPID_CAN_CD           | CAN CD |
| 232 | APPID_MP5              | MP5 |
| 233 | APPID_CAN_BT_AUDIO     | CAN Bluetooth audio |
| 234 | APPID_CAN_ONSTAR_PHONE | CAN OnStar phone |
| 235 | APPID_CAN_RADIO        | CAN radio |
| 236 | APPID_CAN_SETTING      | CAN settings |
| 237 | APPID_CAN_SYNC         | CAN sync |
| 238 | APPID_CAN_MEDIABOX     | CAN media box |
| 239 | APPID_CAN_USB          | CAN USB |
| 252 | APPID_AUDIO_SETTING    | Audio settings |
| 253 | APPID_DATATIME_SETTING | Date/time settings |
| 255 | APPID_OTHER            | Other |

Note: appid is the KERNEL's source id. It is NOT the MCU's own source id (the MCU
calls the radio source 4). Do not confuse the two.

## 5. Which sources are reasonable to remap, which to leave alone

Reasoned from the table, NOT tested beyond radio:

- **Reasonable** — sources backed by an ordinary Android app: `1` GPS,
  `2` Music, `3` Video, and `4` Launcher (points the on-wake "no source" launch
  at an app of your choice). The radio worked because Carnyx drives the tuner
  itself; a music/video replacement would need an app that actually plays that
  source.
- **Leave alone** — sources tied to vendor hardware a plain app cannot drive, or
  where a wrong launch is a safety problem: `13` Camera (this includes the
  REVERSE camera — a broken remap here is dangerous), `10`/`15` Bluetooth,
  `19`/`29` Phone, `12` TPMS, `9`/`31` AUX, and the DVD, TV and `229–239` CAN
  entries.

Whatever you remap, keep release-on-sleep in mind (§8) and keep the backup (§7).

## 6. Finding an app's package name and launcher activity

On the unit, over ADB. These are standard Android 10 commands; none of them has
been run on this unit yet.

    adb shell pm list packages -3                 # third-party packages (user-installed)
    adb shell pm list packages                    # everything, if you need a system app
    adb shell cmd package resolve-activity --brief <pkgName> | tail -n 1
        # prints  <pkgName>/<launcher-activity>  — the className to use

For a Slint/`winit`/NativeActivity app there is no named Activity; use
`android.app.NativeActivity` and lean on the className fallback (§3).

## 7. Doing it

### 7a. The radio, with the script (today, no hand-editing)

`tools/radio-takeover.sh` does the whole radio-source job and refuses rather than
guess. It only handles appid 8 → Carnyx.

    bash radio-takeover.sh check    [IP[:PORT]]    # what is there; changes nothing
    bash radio-takeover.sh install  [IP[:PORT]]    # make Carnyx the radio app
    bash radio-takeover.sh undo     [IP[:PORT]]    # restore the backup / remove our entry

It reads `ro.nwd.config.path`, asks for `adb root` only if the shell is not
already root, MERGES into any existing file, backs the original up to
`replace_source_list.xml.carnyx-backup`, broadcasts the reload, and reads the
file back. See its header and `docs/vendor/README.md` for the full account, and
`tools/test-radio-takeover.sh` for the host tests.

### 7b. Any other source, by hand

**Superseded by `tools/source-remap.sh` (#139),** which sets or resets any
source and lists the unit's installed apps to pick from; per #139 it is built and
host-tested, NOT yet run against the unit. The by-hand steps stay as a reference.

`radio-takeover.sh` is radio-only, so without #139's tool other sources are
edited by hand. The steps below
mirror the script's, one decision at a time. They have NOT been run as written;
the script's version of each step has. Read every error adb prints — none is
hidden here, and each one is the reason a step failed.

    CFG=$(adb shell getprop ro.nwd.config.path | tr -d '\r'); CFG=${CFG:-/config}
    FILE=$CFG/app/replace_source_list.xml
    BAK=$FILE.mybackup

    # 1. ROOT, only if needed. If this prints 0, skip `adb root` (on this unit it
    #    printed 0 already).
    adb shell id -u
    adb root                    # only if the line above was not 0

    # 2. READ WHAT IS THERE FIRST — never blind-overwrite (§8, the merge rule).
    #    "does not exist" means there is no file yet; any other error, stop.
    adb pull "$FILE" ./before.xml

    # 3. IS <config> WRITABLE? On this unit it was, with no remount.
    adb shell "touch '$CFG/.w' && rm '$CFG/.w'"
    #    Only if that failed: is $CFG its own mount? A line printed = yes.
    adb shell "grep ' $CFG ' /proc/mounts"
    #    Only if it printed a line, remount it (and remember you did):
    adb shell "mount -o rw,remount '$CFG'"
    #    No line = part of the system image. Stop; see §9.

    # 4. BACK THE ORIGINAL UP ONCE — only if step 2 found a file:
    adb shell "test -f '$BAK' || cp '$FILE' '$BAK'"

    # 5. WRITE: before.xml plus your one <ReplaceSourceItem>, every other item kept.
    adb shell "mkdir -p '$CFG/app' && chmod 755 '$CFG/app'"
    adb push ./after.xml "$FILE"
    adb shell "chmod 644 '$FILE'"

    # 6. RELOAD AND READ BACK.
    adb shell "am broadcast -a com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE"
    adb pull "$FILE" ./check.xml && cat ./check.xml

    # 7. PUT THINGS BACK AS YOU FOUND THEM, and only what you changed:
    #    `adb shell "mount -o ro,remount '$CFG'"` ONLY if you remounted in step 3,
    #    `adb unroot` ONLY if you ran `adb root` in step 1. Never remount a
    #    directory that is not its own mount: on this unit $CFG sits on /data.

To undo: copy `$BAK` back over the file (or delete your one line and, if no
items are left, remove the file), then broadcast the reload again.

### 7c. Extending the script to any appid

DONE AS A SEPARATE TOOL: `tools/source-remap.sh` (#139). What follows is the
analysis that preceded it. What it would take: make `appid`, `pkgName` and `className` parameters instead of
the hardcoded radio constants (`PKG`, `CLS`, `appid="8"`), and give `undo` a mode
that removes only the entry you name rather than restoring the whole backup — so
undoing one source does not wipe entries added for another. The merge, backup,
reload and read-back machinery already generalises; the regexes key on appid and
pkgName. The host test harness (`tools/test-radio-takeover.sh`) would take new
scenarios for the extra appids.

## 8. The hard rules

1. **MERGE, NEVER OVERWRITE.** The file is a LIST — the kernel remaps every item
   in it — and it may already carry entries someone else set up. Read it first,
   add or replace only your one item, keep the rest. The script enforces this; by
   hand it is on you.
2. **BACK UP THE ORIGINAL, ONCE.** Before the first change, copy the existing
   file aside (the script uses `replace_source_list.xml.carnyx-backup`). Write the
   backup only if one does not already exist, or a second run backs up its own
   output over the true original.
3. **RELOAD AFTER WRITING.** Broadcast `com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE`
   (no permission needed to send it) and the kernel re-reads the file live; a
   reboot does the same.
4. **PACKAGE OVER CLASS.** `pkgName` must be exact; `className` falls back to the
   package's launcher activity if it does not resolve.
5. **STOP-ON-CLOSE vs. A RADIO REMAP — NOT EXPECTED TO CONFLICT.** Carnyx's
   "Stop the radio when Carnyx closes" switch (default ON) hands the FM source
   back when Carnyx is torn down, which zeroes `mcu_current_source`, and
   `SourceMgr.keepCurrentSource` reads that byte at power-off — so on paper it
   could stop the remap's wake relaunch. In practice the 2026-09-17 drive saw the
   handback take (4 → 0) and the radio app still launch afterwards,
   because the kernel restores from its own `SourceKeeper` record
   (`docs/vendor/README.md`). So leave the switch ON. NOT CONFIRMED for the
   remap itself: whether the switch was on during the 2026-11-12 wake test was
   not recorded.

## 9. Prerequisites and gotchas

- **Network ADB must be listening on the unit.** This unit listened on port 5555
  with no switch the owner could find; otherwise it is a developer-options toggle
  or `adb tcpip 5555` once over USB. Android 10 has no wireless pairing.
- **`adb root` must be allowed to write `<config>`.** It works only on a
  `userdebug`/`eng` build, or a `user` build left with `ro.debuggable=1`. On THIS
  unit the shell was already root (inferred from the script's output, #135).
  Check with
  `adb shell getprop ro.build.type` and `ro.debuggable`.
- **adb itself must run.** On the owner's phone, Termux's `adb` would not start —
  `CANNOT LINK EXECUTABLE "adb": cannot locate symbol "_ZNSt6__ndk1..."`, a libc++
  mismatch from a partial upgrade. Fix: `pkg upgrade` (everything together), then
  `adb version`. The script now checks `adb version` first and says this.
- **`<config>` may be read-only or part of the system image.** If it is its own
  read-only mount, a `mount -o rw,remount` is enough (what the script does). If it
  is part of the read-only system image, writing it needs `adb disable-verity`, a
  reboot and `adb remount` — a deliberate step, not something to do blind.
- **The config path is per-unit.** `/data/nwdappconfig` here; read
  `ro.nwd.config.path` on any other unit.

## 10. Confidence: measured vs. inferred

- **Seen on the unit (2026-11-12):** the config path is `/data/nwdappconfig`
  (printed by `check`); the radio remap installs and reads back (printed by
  `install` and `check`); the radio comes up as Carnyx on wake (the owner's test).
- **Inferred from what the script did NOT print (#135):** the ADB shell was
  already root, and `<config>` was writable without a remount.
- **Read from the firmware (decompile of `com.nwd.kernel_v210`):** the launch
  mechanism (§1), the file format and load path (§2–3), the appid table (§4), the
  reload broadcast, the className fallback, and the release-on-sleep interaction.
- **Inferred, NOT tested:** everything about remapping any source other than the
  radio (§5 especially). Treat the first remap of a new source as an experiment
  and keep the backup.

## 11. Source-of-truth files

- `docs/vendor/README.md` — the full firmware account (kernel launch, the copier,
  what each APK exposes, the dead ends). Start here for depth.
- `docs/vendor/replace_source_list.xml` — the radio remap artifact with its full
  header.
- `tools/radio-takeover.sh` — the working installer/checker for the radio source;
  its header is a condensed version of §7–9.
- `tools/test-radio-takeover.sh` — host tests, and the shape any extension should
  keep.
- `docs/TASKS.md` #133 (why the remap is the lever), #135 (the script and the
  real run), #136 (the config-path correction), #137 (removal from the app).
