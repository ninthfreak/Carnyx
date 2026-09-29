# What the vendor's own firmware says

Read out of two APKs the owner copied off the head unit on 2026-09-04:

| file | package | version | what it is |
|---|---|---|---|
| `com.nwd.radio_v1103.apk` | `com.nwd.radio` | 1.1.0.3 (1103) | the stock FM app |
| `com.nwd.radio.service_v214.apk` | `com.nwd.radio.service` | 2.1.4 (214) | the bound tuner service |

Tooling: `androguard` 4.x for the manifests, the string cross-references and the
decompiles; `strings` for the sweep that produced `nwd-actions.txt`. The APKs
themselves are NOT in this repository — they are vendor firmware, they are 11 MB
and 0.5 MB, and everything this tree needs from them is written down here.

`/data/smallota/app/com.nwd.radio/com.nwd.radio.apk` is where the first one lives
on the unit, world-readable, if it ever needs re-reading.

## `nwd-actions.txt`

Every `com.nwd.*` string containing `ACTION` in either dex — 194 of them. This is
the vendor's broadcast vocabulary as the firmware itself spells it, and it
replaces a list this tree had been assembling by hand from what CarFM happened to
have seen.

**A NAME IN THIS FILE IS NOT A LIVE BROADCAST.** Both APKs bundle the same vendor
SDK, so its constants are in both string pools whether or not either app uses
them. `com.nwd.ACTION_OS_SLEEP`, `com.nwd.ACTION_OS_WAKE_UP` and
`com.nwd.ACTION_ACCOFF_UPDATE` are all in there and NOTHING in the stock radio app
references any of them — no sender, no receiver. Cross-reference before believing.

## What was established, with the evidence

**The stock app has no implicit door.** Its whole manifest is one activity,
`com.nwd.radio.home_horizontalActivity`, `launchMode="2"`, with one intent-filter:
`MAIN` + `LAUNCHER`. No receivers, no services, no providers. Whatever launches it
on ACC-on must be naming the component explicitly, so there is no filter for
Carnyx to declare and no chooser to become the default of. That closes route 2 of
#96. (`targetSdk="25"`, which is also why the app itself escapes several of the
restrictions Carnyx lives under. It holds `WRITE_SECURE_SETTINGS`,
`MANAGE_ACTIVITY_STACKS`, `MODIFY_AUDIO_ROUTING` and `ACCESS_FM_RADIO`.)

The tuner service is the same shape: one service, one action
(`com.nwd.radio.service.ACTION_RADIO_SERVICE`), no receivers.

**FM is source 4, and that is now measured rather than assumed.**

```java
// com.nwd.radio.arm.allwinner.AWFMFeature.isRadioSource()
return SettingTableKey.getIntValue(cr, "mcu_current_source") == 4;
```

**The source switch carries `extra_source_id`, a BYTE — not `extra_media_source`.**
CarFM's `BUILTIN-TUNER-FINDINGS.md` left "the exact `EXTRA_MEDIA_SOURCE` value"
open and pointed at the wrong extra: `extra_media_source` belongs to
`OuterBroadcastSender.sendMediaPlayInfo`, which talks to the CAN bus. The one that
moves the radio is:

```java
// com.nwd.radio.arm.allwinner.AWRadioManager$1.onReceive
byte newSource = intent.getByteExtra("extra_source_id", 0);
if (newSource == 4) { InitFM(); }
else if (!NewRdsManager.getInstance().isRdsEnable()) { ExitFm(); }
```

**And the app is told by name that it is about to be killed.**

```java
if ("com.nwd.ACTION_KILL_OTHER_APP".equals(action)) {
    String pkg = intent.getStringExtra("extra_package_name");
    if (pkg.equals("com.nwd.radio")) { ExitFm(); }
}
```

**All of it through a RUNTIME receiver**, which is why none of these actions
appears in any manifest on the unit and why a sweep of declared receivers finds
nobody:

```java
// AWRadioManager.registReceiver()
filter.addAction("com.nwd.action.ACTION_APP_IN_OUT");
filter.addAction("com.nwd.android.ACTION_EXIT_ARM_FM_RAIDO");
filter.addAction("com.nwd.ACTION_MEDIA_PLAY");
filter.addAction("com.nwd.action.ACTION_MCU_STATE_CHANGE");
filter.addAction("com.nwd.action.ACTION_CHANGE_SOURCE");
filter.addAction("com.nwd.ACTION_KILL_OTHER_APP");
mContext.registerReceiver(mReceiver, filter);
```

See `docs/TASKS.md` #133 for what follows from this.

---

# The ACC-off recipe, read off this unit's own code path

Added 2026-09-16, after the owner reported that the FM handback works when Carnyx
is closed by hand and does NOT work when the car is switched off. Everything
below was read directly out of the two APKs above and quoted; the reading was
done twice, once by a fan-out of agents over all 645 `com.nwd` classes and once
by hand against the bytecode for the claims acted on.

## Which radio manager this unit runs, and why it decides everything

`RadioService.onCreate()` picks ONE of four implementations and the choice
changes what every vendor call does:

| picked when | class | `getRadioType()` |
|---|---|---|
| `isNewArmRadio()` | `ArmRadioManager` | 1 |
| `getIsSprdRadio()` | `SprdRadioManager` | 3 |
| `getIsAllWinnerRaido()` | `AWRadioManager` | 2 |
| `getIsMtkRadio()` | `ArmRadioManager` | 3 |
| otherwise | `RadioManager` (MCU) | 0 |

The development unit is Allwinner, so `AWRadioManager` is the path that matters,
and **Carnyx should log `getRadioType()` (transaction 29) once at launch** rather
than inferring it. Everything below is the Allwinner path unless it says
otherwise.

## `setRadioBackServiceOn(false)` does nothing here

    ArmRadioManager:  public void setRadioBackServiceOn(boolean p1) { return; }
    AWRadioManager:   { LOG.print("setRadioBackServiceOn  "); return; }
    SprdRadioManager: { LOG.print("setRadioBackServiceOn  "); return; }

Only `RadioManager` — the MCU path, type 0 — emits anything. Half of Carnyx's
teardown has been a logged no-op on this hardware.

## Why the handback broadcast dies at ACC-off

`com.nwd.action.ACTION_REQUEST_CHANGE_SOURCE` has **no receiver in either APK**.
It is consumed by a THIRD process, `com.nwd.kernel.service.KernelService`, which
is installed on the unit but is not one of the two files here. So Carnyx's
release has to be queued by ActivityManager, dispatched, and delivered across a
process boundary — and at ACC-off the SoC suspends first. Closing the app by hand
leaves the system awake, the delivery happens, and the source changes. Same code,
opposite outcome, and that is the whole of the bug.

## `com.nwd.ACTION_ACCOFF_UPDATE` IS DEAD CODE

A string constant in both APKs. Never sent, never registered, never compared
against, in 645 classes. Three builds of this app listened for it. Every
`last sleep: nothing recorded` was accurate.

## What the vendor actually listens for

`AWRadioManager.registReceiver()` registers a RUNTIME receiver for six actions,
one of which is the ACC signal:

    com.nwd.action.ACTION_APP_IN_OUT
    com.nwd.android.ACTION_EXIT_ARM_FM_RAIDO
    com.nwd.ACTION_MEDIA_PLAY
    com.nwd.action.ACTION_MCU_STATE_CHANGE      <- this one
    com.nwd.action.ACTION_CHANGE_SOURCE
    com.nwd.ACTION_KILL_OTHER_APP

and its handler for that action is, in full:

    int v6 = SettingTableKey.getIntValue(resolver, "mcu_state", 1);
    if (v6 != 3 && v6 != 2) {
        if (v6 != 0) {
            if (v6 == 1) { if (rds.isRdsEnable()) InitFM(); ... }
        } else { ... AWNative.PowerState = 0; }
    } else {
        ExitFm();
    }
    NewRdsManager.getInstance().notifyPowerState(v6);

**`mcu_state`: 1 awake, 2 and 3 going to sleep, 0 powered down.** The vendor tears
the tuner down on 2 and 3.

TWO THINGS FOLLOW, AND BOTH ARE LOAD-BEARING:

1. The handler re-reads the SETTING and ignores any extra on the intent. The
   broadcast is a nudge; `Settings.System "mcu_state"` is the fact. It is a plain
   integer readable with no permission, so a `ContentObserver` on
   `Settings.System.getUriFor("mcu_state")` is a second, independent route that
   does not depend on whether `KernelService` restricts its broadcast — which
   cannot be determined from these two files.
2. The action fires on EVERY transition including the wake. Anything that treats
   it as a sleep unconditionally will hand the source back at ACC-ON.

## `ACTION_KILL_OTHER_APP` releases FM, it does not kill an app

In `AWRadioManager$1`:

    String v9 = intent.getStringExtra("extra_package_name");
    if (v9.equals("com.nwd.radio")) { access$4(this$0).ExitFm(); }

Only that one package name is honoured and the effect is an FM audio teardown —
not a force-stop, and nothing that touches `mcu_current_source` or the UI. The
receiver is registered dynamically with no permission and no caller check, so a
broadcast from Carnyx WOULD be honoured. It is a way to silence the tuner; it is
not a way to stop the stock app resuming.

## `mcu_current_source` is watched on this unit

    AWRadioManager:91  registerContentObserver(getUriFor("mcu_current_source"), 1, mSourceObserver)
    AWRadioManager$6   onChange -> if (mSourceId != v0) handleSourceChange((byte) v0)
    handleSourceChange(byte p5) -> p5 != 4 tears playback down, == 4 restarts it

A fan-out reader claimed the Allwinner manager does NOT watch this key. It does;
the claim was wrong and was caught by reading the file. Recorded because the same
reader was right about many other things and a single wrong claim in a long list
is the hard kind to notice.

## Dead ends, so nobody spends a day on them twice

- **`sendRadioCommand(byte, byte)`**, transaction 27, the raw MCU escape hatch:
  it is genuinely a blocking binder call, but it emits a protocol-type-3 (radio
  family) frame while a source change is protocol-type-1, and the caller cannot
  reach the header. No byte pair through it stops audio or releases the source.
- On the MCU path only, the two bytes become `F0 05 03 01 00 <a> <b> <ck>`.
  Irrelevant on Allwinner, recorded because it cost a day to establish.
- **The stock app cannot start itself.** `com.nwd.radio` declares exactly one
  component, a MAIN/LAUNCHER activity — no receiver, no service, no provider. Every
  appearance of it is an explicit `startActivity` from another process. That
  process is `com.nwd.kernel` — see "What launches the stock radio app, and how
  to replace it" at the end of this file. It was not in the first two APKs but is
  in the kernel APK.

## What is still unknown

- Whether `KernelService` restricts who may receive `ACTION_MCU_STATE_CHANGE`.
  The `ContentObserver` route exists precisely so this does not have to be known.
- ~~How to bind `IKernelFeature`.~~ RESOLVED with the `com.nwd.kernel` APK — the
  bind action, descriptor, transaction and frame are all established and built as
  `CarnyxKernel`. See "The synchronous handback, from `com.nwd.kernel`" above.
  The 09-17 drive confirmed the blocking call reaches the wire.

---

# The synchronous handback, from `com.nwd.kernel`

Added 2026-09-16 from three more APKs the owner pulled off the unit:

| file | package | what it is |
|---|---|---|
| `com.nwd.kernel_v210.apk` | `com.nwd.kernel` | the MCU/UART service — **the one that matters** |
| `com.nwd.setting.service_v392.apk` | `com.nwd.setting.service` | the settings service |
| `com.nwd.backcar_v176.apk` | `com.nwd.backcar` | reversing camera |
| `com.nwd.factory.setting_v340.apk` | `com.nwd.factory.setting` | factory menu |
| `com.android.launcher.nwd.res.k24_v1.apk` | launcher resources | no code of interest |

`com.nwd.kernel` is the third process that the `ACTION_REQUEST_CHANGE_SOURCE`
broadcast was going to all along. Having it closes the loop.

## It is bindable by a third-party app

    <service android:name=".service.KernelService">
        <intent-filter>
            <action android:name="com.nwd.kernel.service.KernelService" />
    manifest: no targetSdkVersion declared, minSdk 19, no android:permission

No `targetSdkVersion` means it defaults to `minSdkVersion` — 19, far below the 31
where `exported` stopped defaulting to true for a component with an intent
filter. **So it is exported, it is unguarded, and the binding action is its own
class name.**

`onBind` returns `mBinder`, a `com.nwd.kernel.aidl.IKernelFeature$Stub`.

## The call, and it is blocking the whole way to the wire

    IKernelFeature      descriptor "com.nwd.kernel.aidl.IKernelFeature"
                        TRANSACTION_request = 1          void request(byte[])

    Stub dispatch:      case 1: enforceInterface(...); this.request(p7.createByteArray());
                                p8.writeNoException(); break;

    KernelService$2:    public void request(byte[] p2) {
                            if (access$1(this$0) != null)
                                ProtocalUtil.writeDataToMCU(access$1(this$0), p2);
                        }

    ProtocalUtil:       if (isCanWriteData2Uart()) {
                            KernelProtocal.calCheckSumAndWriteEndOfData(p4);
                            p3.writeData(p4);          // ICommunicator — the UART
                        }

No handler, no queue, no worker thread, no `FLAG_ONEWAY`. The bytes reach the
serial port before `request` returns. **This is the synchronous release the
broadcast could never be.**

THE CHECKSUM IS COMPUTED FOR US. `calCheckSumAndWriteEndOfData` runs inside
`writeDataToMCU`, on our buffer, before the write — so the caller supplies the
frame with a spare final byte and does not have to get the arithmetic right.

## The frame

`ActionProtocalUtil.requestChangeSource(uart, sourceId, sourceType, ack)`:

    byte[] v0 = KernelProtocal.generateNullProtocal(5, 1, 3);
    int v1 = getProtocalDataStartOffset(v0);      // 5
    v0[v1]     = sourceType;
    v0[v1 + 1] = sourceId;
    writeDataToMCU(uart, v0);

`generateNullProtocal(len, type, dataType)` builds `byte[len + 3]` with
`[0] = 0xF0`, `[1] = len`, `[2] = type`, `[3] = dataType`. So:

    F0 05 01 03 00 <sourceType> <sourceId> <checksum>
       │  │  │  │                                └─ written by the service
       │  │  │  └─ dataType 3 = CHANGE_SOURCE
       │  │  └──── type 1 = ACTION family
       │  └─────── length 5
       └────────── header

`sourceType` 0 is front, `sourceId` 0 is `SOURCE_ANDROID` (4 is `SOURCE_RADIO`).
Handing the source back to Android is therefore
**`F0 05 01 03 00 00 00 00`**, last byte ignored.

## The write window is open at 2 and 3

`ProtocalUtil.setCanWriteData2Uart(0)` — the gate that would make the write a
silent no-op — is reached from `MCUDeviceManager.onGetOsSleepState` and only
inside `if (this.mDeviceState.getMcuState() == 0)`. `mcu_state` 0 is ALREADY
POWERED DOWN. At 2 and 3, the states that mean "going to sleep", the UART is
still writable. That is the window, and it is the same window the
`mcu_state` watch fires in.

## And the state broadcast is unrestricted

`SourceMgr$1` and `SourceMgr$2`:

    Intent v3_1 = new Intent("com.nwd.action.ACTION_MCU_STATE_CHANGE");
    v3_1.putExtra("extra_mcu_state", (byte) v2);
    ...sendBroadcast(v3_1);

The plain one-argument `sendBroadcast`, with no receiver permission. So the
broadcast half of the `mcu_state` watch IS deliverable to a third-party app —
the question the `ContentObserver` was added to avoid having to answer. Both
routes are real; keep both, because they fail independently.

## What building this needs

BIND EARLY AND HOLD THE BINDER. `bindService` is asynchronous — the callback
arrives on the main looper — and at `mcu_state` 2 there is no time to start
binding. Carnyx has to be bound before the driver switches off, the way it
already holds the radio service.

The one thing NOT established here: the vendor wraps its own source change with
an `AckHelper` that expects an MCU acknowledgement and retries for three seconds.
Sending the raw frame skips that bookkeeping. Whether the MCU is content with an
unacknowledged frame from a stranger is a question for a drive, not a decompile.

## The other three APKs: nothing that helps, two things worth knowing

Surveyed 2026-09-16 for the ACC-off problem. None of them moves it forward, which
is itself worth writing down so nobody reads them again hoping.

**`com.nwd.setting.service`** shares a user id with `com.nwd.kernel`
(`com.nwd.kernel.setting`) and exports `SettingService` on
`com.nwd.setting.service.ACTION_SETTING_SERVICE` with no permission and no
declared `targetSdkVersion` — so, like the kernel service, bindable. Its
`SettingFeature` has 39 transactions and **none of them changes the audio
source**. The closest are `setMute` (10) and `shortMute` (39), which would
silence the output without stopping the stock app resuming, and
`setAutoWakeup` (38), which is about the unit waking itself rather than about FM.
A dead end for this problem.

**`com.nwd.factory.setting`** exports five services with no permission at all,
including `FactorySettingService` and an `AutoUpdateService`. Nothing in them
touches the audio source — but one of them, `CopyFileService`, turned out to be
the write route for the kernel remap: an unguarded, config-driven file copier
that writes a caller-named destination. See "The factory copier writes arbitrary
paths" near the end of this file. That an unguarded factory service on a shipping
head unit can write `/config` is exactly why it was worth writing down.

**`com.nwd.backcar`** runs as `android.uid.system` and exports `BackcarService`.
Reversing camera; irrelevant here.

**`com.android.launcher.nwd.res.k24`** is a resource package with no code of
interest.

---

# What launches the stock radio app, and how to replace it

Read off `com.nwd.kernel_v210` on 2026-09-17, after the 09-17 drive proved the
handback works and the stock app still launches anyway. This is the answer to the
question the dead-end note above left open: *"the process that does it is not in
these files."* It is `com.nwd.kernel`, and it now is.

## The kernel restores a source on power-up and launches its app

`com.nwd.kernel.source.SourceMgr` owns which app serves each audio "source". On
power-up it restores the source that was active at the last power-off and calls
`startApp(SourceItem)`, which ends in `startActivity` on that source's app. The
radio source is **appid 8** (`SourceConstant.APPID_RADIO`) — a different number
from the MCU's own source id 4; do not confuse them.

    SourceMgr.startApp(SourceItem item):
      ...
      if (mReplaceSourceList.isContain(item.getAppid())) {
          item.setPackageName(mReplaceSourceList.findPkgNameByAppid(appid));
          item.setClassName(mReplaceSourceList.findClassNameByAppid(appid));
      }
      item.setComponent(new ComponentName(pkg, cls));
      item.addFlags(FLAG_ACTIVITY_NEW_TASK);
      startActivity(item);            // catch -> getLaunchIntentForPackage(pkg)

This closes the loop with the 09-17 log. Carnyx handed the MCU audio source back
(`mcu_current_source` went 4 -> 0), and the radio app launched regardless,
because the kernel restores from its OWN record, not from `mcu_current_source` at
launch time. That record is `SourceKeeper`, a `source_keeper` SharedPreferences
file (`key_packname` / `key_classname` / appid) inside the kernel's data.

## The substitution hook: `replace_source_list.xml`

`com.nwd.kernel.source.ReplaceSourceList` reads a config file and, for any appid
it contains, overrides the package and class the kernel is about to launch:

    ReplaceSourceList.CONFIG = getConfigPath() + "/app/replace_source_list.xml"
    getConfigPath() = SystemProperties("ro.nwd.config.path", "/config")

so on this unit: **`/config/app/replace_source_list.xml`**. Format, one entry per
remapped source:

    <ReplaceSourceList>
      <ReplaceSourceItem appid="8" pkgName="com.ninthfreak.carnyx"
                         className="android.app.NativeActivity" />
    </ReplaceSourceList>

The file lives in this tree at `docs/vendor/replace_source_list.xml`, with its
full header. An `appid="8"` entry pointing at Carnyx makes the kernel launch
Carnyx wherever it would have launched the stock radio app. The stock app is a
passive MAIN/LAUNCHER-only target, so once the kernel stops pointing at it, it is
never seen — this is #133 outcome A's substitution half, guaranteed by the code
rather than inferred.

`className` is not load-bearing: if it fails to resolve, `startApp` falls back to
`getLaunchIntentForPackage(pkgName)`. The **package name** is the field that has
to be right. Carnyx's is `com.ninthfreak.carnyx`, launcher activity
`android.app.NativeActivity` (confirmed in `android/app/src/main/AndroidManifest.xml`).

The kernel reloads the file live on the broadcast
`com.nwd.ACTION_REPLACE_SOURCE_LIST_CHANGE` (registered in `ReplaceSourceList`'s
constructor, no permission to send), so a reboot is not strictly required after
writing it.

## Which app, not whether — and the release-on-sleep interaction

The remap decides WHICH app the kernel launches for the radio source. It does not
decide WHETHER the radio source is restored at all. That is
`SourceMgr.keepCurrentSource`, which runs at power-off and reads
`mcu_current_source`:

    keepCurrentSource (mIsKeepLauncherSource=1, mNewResumeSourceMode=-1 on this ROM):
      v1 = Settings.System getInt("mcu_current_source")
      if (currentApp.getSourceProperty() != 0 || v1 != 0 || isNwdMedia())
          keepSource(getTopSource())              // radio stays -> Carnyx on wake
      else
          keepSource(getInitSourceByAppid(4))     // launcher -> nothing on wake

So `mcu_current_source` at power-off is the byte that decides whether the radio
source (hence Carnyx) is restored. **Carnyx's release-on-sleep handback sets that
byte to 0.** With the remap in place, "restore the radio source" *is* "launch
Carnyx" — the goal — so the handback is now counterproductive: it can suppress
the launch on exactly the cycles where the radio was playing. `release_on_sleep`
therefore ships OFF as of #133 (`src/settings.rs`). It stays meaningful only on a
unit WITHOUT the remap, where the handback is the sole lever on the wake
relaunch.

Config-flag defaults were read from `NwdConfig` (`mIsKeepLauncherSource = 1`,
`mNewResumeSourceMode = -1`); they are overridable from the same config
properties file, so a unit with a different config could route the power-off save
differently.

## Where the file goes, and how to write it

`/config` is a system partition, and the kernel only ever reads
`replace_source_list.xml`. But `com.nwd.factory.setting` exposes a general
file-copier that can WRITE it — see the next section. The blunt routes remain:

- root: remount `/config` rw, drop the file, `chmod 644`;
- a recovery / ADB shell with system access.

## The factory copier writes arbitrary paths, `CopyFileService` (2026-09-17)

`com.nwd.factory.setting` (targetSdk **19**, no `sharedUserId`) exports
`com.nwd.factory.copy.CopyFileService` **exported, no permission, no caller
check** — any app or `adb shell am startservice` can drive it. It is a
config-driven copier, and its config names the destination:

    CopyFileService.onStartCommand:
      GlobalData.setPath(intent.getStringExtra("COPY_PATH"))   // source dir
      GlobalData.setCopyApk(intent.getBooleanExtra("COPY_APK", false))
      // needs <COPY_PATH>/CopyFileConfig.xml (or ApkUpdateConfig.xml if COPY_APK)
      // dialog UNLESS COPY_PATH == <config>/app  OR  <COPY_PATH>/autocopy exists
      -> CopyFileThread

    CopyFileThread.run():
      count = getConfigCount(CopyFileConfig.xml)   // number of <PathItem>
      if (count <= 1) ParserXMLFile(...) else ParserXMLFileEx(...)

    ParserXMLFile, per <PathItem PathSrc="..." PathDes="...">:
      src = (COPY_PATH == <config>/app) ? PathSrc : COPY_PATH + PathSrc
      if (File(src).isDirectory()) {
          if (IsDesDirExist(PathDes))              // true if PathDes already exists
              for entry in src.list():
                  CopyFile(src/entry, PathDes/entry)   // chmod 777 each
      } else if (src is a file) {
          if (name != "update.zip") -> "srcPath isn't dir", SKIPPED
          else -> OTA path; with an `autocopy` marker present, MASTER_CLEAR
      }

**THE PAYLOAD MUST BE A DIRECTORY COPY, and this is not a style choice.** With a
single `<PathItem>` the copier takes `ParserXMLFile`, which only copies the
CONTENTS of a source DIRECTORY into the destination directory. A lone FILE source
is logged and skipped unless it is named `update.zip` — and that branch, with an
`autocopy` marker present, fires a `MASTER_CLEAR` **factory reset**. So the file
approach does not work and its neighbour is dangerous.

### `/config/app` DOES NOT EXIST, and the destination gate is why three attempts failed

`IsDesDirExist(PathDes)` returns true only when `PathDes` **already exists** (or
is a mounted external/internal root). Measured on the unit, 2026-09-24:

    /config: dir, readable, not writable, 2 entries
    /config/app: ABSENT

So naming `/config/app` as `PathDes` fails the gate and the copy is skipped
outright — no error about `/config`, no write attempted. An earlier note here
asserted `/config/app` exists "because the kernel reads its files". That was an
assumption and it was wrong: the kernel only ever READS that path, and
`ReplaceSourceList.loadConfig` finds `!file.exists()` and returns in silence, so
an absent directory looks identical to an empty one from the firmware side.

**The way through is the copier's own directory-creating branch.** In
`ParserXMLFile`, each entry of the source directory that is NOT a file goes to
`CopyFolder`, and `CopyFolder` opens with `new File(dst).mkdirs()`. So:

    PathDes = /config                      # exists, so the gate passes
    PathSrc = /payload
    <COPY_PATH>/payload/app/replace_source_list.xml

gives `CopyFolder(<COPY_PATH>/payload/app, /config/app)`, which **creates**
`/config/app` and copies the file in. The destination directory is carried as
part of the payload rather than assumed to be there.

### THE ROUTE IS CLOSED ON THIS UNIT: `/config` is not writable by the copier (2026-09-25)

The layout above was built, staged correctly and confirmed at the dialog — and
`/config/app` was still ABSENT afterward:

    12:26:07 (before the fixed build)  payload/app: dir, 1 entries  /config/app: ABSENT
    12:59:33 (after, dialog confirmed) payload/app: dir, 1 entries  /config/app: ABSENT

`CopyFolder` runs `new File("/config/app").mkdirs()` and **ignores the result**;
on failure it proceeds, the `FileOutputStream` throws, the throw is caught, and
nothing is written. An absent `/config/app` after a confirmed copy therefore
means the `mkdirs` failed — i.e. `/config` is not writable by the factory
process. This is the one question the firmware could not answer, and the unit
has now answered it: **the unprivileged file-drop route cannot install the
remap.** Getting the file into `/config/app` needs root, or a recovery / ADB
shell with system access.

The `com.nwd.backcar` lead — a system-uid app that COULD write `/config` — was
examined 2026-09-25 and is a dead end. It runs as `android.uid.system` and
exports four components, none drivable by a caller: `BackcarService.onBind`
returns null and its `onStartCommand` reads nothing from the intent;
`BootReceiver` only fires on boot and starts that service; `TestActivity` and
`BlackActivity` are camera-test UI. No component takes a path or writes a
caller-named file, and there is no provider. The system uid is real but there is
no operation to hand it, so it cannot be reached from an unprivileged app.

That exhausts the unprivileged surface across all six APKs. Installing the remap
into `/config/app` needs root, or a recovery / ADB shell with system access.
There is no application-code route on this unit.

The recipe, no root:

1. A dir the factory app can read. **NOT an app-specific directory.** Measured
   2026-09-18: staging in `getExternalFilesDir(null)` —
   `/storage/emulated/0/Android/data/<pkg>/` — made the copier report a missing
   path and write nothing, because Android 10 walls that subtree off from other
   apps regardless of their legacy-storage status. Use a shared location;
   `Download/` is proven on this unit, since `NwdBridge.writeLog` puts the
   diagnostics log there and it comes off the device as an ordinary file. Call it
   `<SRC>`.
2. Lay it out as a directory copy, with the destination directory carried
   INSIDE the payload (see the gate above — `/config/app` does not exist):

       <SRC>/
         CopyFileConfig.xml
         payload/
           app/
             replace_source_list.xml      # this repo's copy

   with `CopyFileConfig.xml`:

       <?xml version="1.0" encoding="utf-8"?>
       <CopyConfig>
         <PathItem PathSrc="/payload" PathDes="/config" />
       </CopyConfig>

   The parser only keys on `PathItem` elements and their `PathSrc` / `PathDes`
   attributes; the root element name is not checked. `PathSrc` is relative to
   `COPY_PATH`, `PathDes` is absolute.
3. **No `autocopy` marker.** Omitting it keeps the payload away from the
   `update.zip`/`MASTER_CLEAR` branch entirely AND means the copy only proceeds
   when a human taps OK on the copier's confirmation dialog — the right gate for
   a system-partition write.
4. Start it:

       am startservice -n com.nwd.factory.setting/com.nwd.factory.copy.CopyFileService \
         --es COPY_PATH <SRC>

   Because it is exported and permissionless, Carnyx issues this `startService`
   itself — see "Carnyx installs it itself" below.

## Carnyx installs it itself: the `InstallRadioRemap` action

`CarnyxRemap.java` (dexed by `build.rs`) does exactly the recipe above:
`Download/carnyx-remap/` as `<SRC>`, `payload/` holding the remap, one
`CopyFileConfig.xml`, no `autocopy`, then `startService` on `CopyFileService`.
The staged files go through MediaStore, because a targetSdk 34 app cannot open a
`FileOutputStream` anywhere in shared storage — and the app-specific directory it
CAN write is the one the factory app cannot read (see the measurement above).
Each file is deleted before it is re-inserted, or a second install would leave
`name (1).xml` beside it and the copier takes everything in the directory. `com.nwd.factory.setting` is in `<queries>` so the package is
visible on targetSdk 30+. Two settings rows drive it — "Install radio takeover
(writes to /config)" and "Check radio takeover". The seam is
`src/android/remap.rs`.

**IT MERGES, IT DOES NOT REPLACE, AND THAT IS NOT A NICETY.**
`ReplaceSourceList` is a LIST — the kernel remaps every `ReplaceSourceItem` in
the file — and `CopyFile` runs `if (dst.exists()) dst.delete()` before writing.
So staging a one-entry file and firing the copier blind would delete any other
source remap the unit already had, silently and unrecoverably. The file exists so
an integrator can repoint sources at their own apps, so it is not ours to
overwrite. `install()` therefore reads `/config/app/replace_source_list.xml`
first and:

- absent → writes ours alone;
- already maps appid 8 to Carnyx → does nothing at all and says so, so a second
  tap costs no system write;
- present with other entries → keeps every one, replaces only the appid 8 entry,
  and stages a byte-exact backup as `replace_source_list.xml.carnyx-backup` in
  `/config/app` (written once, so a later install cannot back up its own output
  over the true original);
- present but unreadable or malformed → **refuses**. Overwriting a file it cannot
  read would discard contents nobody has seen.

The rebuild is lossless for everything the kernel reads: `loadConfig` takes
exactly `appid`, `pkgName` and `className` per item. Comments and formatting are
not preserved, which is why the original is backed up rather than trusted to the
rebuild. `verify()` parses the file rather than substring-matching it, and
reports which package owns the radio entry plus how many other entries survived.

**IT CHECKS BEFORE AND AFTER, NOT JUST BEFORE.** Asking the copier is not the
copier succeeding, and the copy lands whenever a human confirms the vendor
dialog — so there is no moment `install()` could usefully check for itself. It
keeps the before-state it already read, then a daemon watcher polls `DEST` every
500 ms for up to 120 s and posts the verdict into the diagnostics log the instant
it knows, through a hand-registered `nativeRemapNote` (the pattern
`CarnyxLocation.nativeNote` uses). That comparison is what separates the three
answers that matter:

- the file changed and the radio entry is now Carnyx → the copier wrote it, so
  `/config` **is** writable by the factory process;
- the file changed to something else → somebody else's write won;
- **nothing changed in 120 s** → either the dialog was never confirmed or the
  factory app cannot write `/config` — which is the one question the firmware
  could not answer.

Without the before-state, "the file names Carnyx" is ambiguous: it could have
said that before this app ever ran. `verify()` reports the same comparison when
an install ran in the same process, so a manual Check distinguishes "we just did
this" from "it was already like this".

## THE ONE UNKNOWN THIS CANNOT SETTLE: is `/config` writable by that process?

`com.nwd.factory.setting` has NO `sharedUserId=android.uid.system`. Its
Android permissions (`WRITE_SECURE_SETTINGS`, `MOUNT_UNMOUNT_FILESYSTEMS`, …)
are not filesystem ownership of `/config`. Whether its `FileOutputStream` and
`chmod 777` on `/config/app` succeed depends on that partition's unix
permissions and SELinux policy on the running unit, which a decompile cannot
show. The evidence FOR is that the vendor's own copier treats `<config>/app` as
a first-class destination and this app is the provisioning tool; the evidence
AGAINST certainty is the missing system uid.

Settle it cheaply on the device before trusting the route:

    ls -laZ /config /config/app          # ownership, mode, SELinux context

or just run the copy once and check whether
`/config/app/replace_source_list.xml` appears. If the factory process cannot
write there, the route degrades to root / recovery, and Carnyx cannot
self-install the remap.

This is why the remap is outcome A's clean answer, and why whether Carnyx can
install it ITSELF is a single yes/no about one partition's permissions.

## Provenance

`com.nwd.kernel_v210` (`com.nwd.kernel`):
`SourceMgr` (`startApp`, `keepCurrentSource`, `SourceKeeper`),
`ReplaceSourceList`, `SourceConstant.APPID_RADIO = 8`,
`NwdConfig` (`mIsKeepLauncherSource`, `mNewResumeSourceMode`),
`NwdConfigUtils.getConfigPath` -> `ro.nwd.config.path` default `/config`.
