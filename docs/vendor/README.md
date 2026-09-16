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
  appearance of it is an explicit `startActivity` from another process, and the
  process that does it is not in these files.

## What is still unknown

- Whether `KernelService` restricts who may receive `ACTION_MCU_STATE_CHANGE`.
  The `ContentObserver` route exists precisely so this does not have to be known.
- How to bind `IKernelFeature` — transaction 1, `request(byte[])`, a genuinely
  blocking call that would let Carnyx hand the source back synchronously instead
  of by broadcast. The service is not in these APKs, so its binding action and
  AIDL are unestablished. This is the route to a clean fix and it needs a third
  APK off the unit: `com.nwd.kernel`.
