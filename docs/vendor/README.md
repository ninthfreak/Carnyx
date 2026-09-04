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
