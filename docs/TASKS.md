# Task list, with detail

62 items inherited from CarFM (numbers 1–65; 10–12 were never issued), plus
Carnyx's own from 66 on.

## How this is maintained

THIS FILE IS THE LIST. It exists because the list was previously held in a task
store and in conversation, both of which were lost — the CarFM instance that
reconstructed it was supposed to be keeping a document for exactly this reason
and was not. Every session that opens or closes an item edits this file in the
same commit as the work, so the record cannot drift from the tree again.

**Numbers are permanent.** Thirty-four entries below cross-reference each other by
number, and CarFM's tree cites three of them from code: `#55` at
`src/services/tunerCapabilities.ts:134`, `#58` at `NwdRadioModule.kt:753` and
`:977` and four times in `docs/BUILTIN-TUNER-FINDINGS.md`, `#60` at
`src/screens/RadioScreen.tsx:458`. A number is never reused and never renumbered,
so a closed item stays where it is with its status changed rather than being
deleted.

**`**Carnyx:**` lines are this project's status on an inherited item**, and they
are only present where something was actually established — read on both sides,
measured, or shipped. An item with no such line has not been assessed against
Carnyx, which is different from being irrelevant to it. Many are genuinely
CarFM-only: React Native internals, the VibeSDR strip, iOS and watch code.

**CarFM's diagnostics probes are not features** (owner's decision, this session).
The raw-RDS capture and its Downloads export, `probeNwdFmManager`, the full
getter walk, the head-unit settings dump and the vendor-app trampoline probe are
not parity gaps and must not be tracked as such. If Carnyx needs diagnostics they
get built as Carnyx's own.

---

62 items (numbers 1–65; 10–12 were never issued). 31 pending, 31 completed, as
CarFM left them.

**Provenance, stated honestly.** The task store this session can reach is empty,
and no detail document has ever existed in the repo. The subjects below are the
task records; the *detail* is reconstructed, and each item says where its detail
came from:

- **[AUDIT]** — `docs/AUDIT-2026-08-02.md`, 37 confirmed findings with file/line
  evidence. Authoritative and written at the time.
- **[STRIP]** — `docs/CARFM-STRIP-PLAN.md`, the strip-down plan and its
  "Other open items" section. Authoritative.
- **[LICENCE]** — `docs/LICENSING.md`.
- **[SESSION]** — reconstructed from the working session. Accurate as to what was
  discussed and measured, but it is my reconstruction, not a record you wrote.
- **[NO DETAIL FOUND]** — the subject is all that survives. Flagged rather than
  filled in with guesswork.

Numbers are load-bearing: `#55` and `#58` are cited in code comments, so they are
preserved as issued. *(As received this line also named `docs/HANDOFF.md`, which
cites neither — corrected against a grep of the CarFM tree; the real locations
are listed at the top.)*

---

# PENDING (31)

### 15. Wire a genre source for Nearby
**[NO DETAIL FOUND]** — The Nearby picker (`src/components/carfm/NearbyPicker.tsx`)
lists stations by frequency and signal but shows no genre/PTY. The face gets PTY
from the RDS decoder for the *tuned* station only; a nearby station that is not
tuned has no PTY source, so this needs either the station database to carry a
format field or a different source. The audit notes NearbyPicker was "read at a
single call site" and never examined, so nothing more is recorded.

**Carnyx:** carried over, and the data source is the blocker here too. Every one
of the 20,733 rows in the shipped table has `station_class` and the genre column
NULL, so `has_talk` is always false and neither filter bar can appear on real
data (`examples/shot.rs`, the `NearbyGenre` arm). The filter is implemented and
tested in `src/stations.rs`; it has nothing to read.

### 26. Smoke-test on the head unit
**[STRIP §A]** — "After items 8/9, confirm Android Auto + app boot on a real APK
(removed code was iOS/watch no-ops, but SDRScreen wasn't run)." Has since grown
far beyond that scope: as of the last drive log (2026-08-06 20:58) there were 26
unverified commits covering the v1.14 signal meter, the stereo cones, the TA
round-trip, the post-tune read schedule and two bug passes. **This is the
highest-value open item** — it either confirms a large body of work or produces a
short bug list.

**Carnyx:** running on the unit already, and it is the OWNER who builds and
installs it — `cargo apk build --lib` on their own machine, per the README's
Building section. Their field reports are what drive the work: the STEREO pill
"almost never lit up", the app not staying alive in the background, the overlays
opening slowly. Each of those was reproduced, diagnosed and fixed from the
report.

**No session may write that Carnyx has not run on hardware.** This container has
no SDK and no NDK and cannot produce an APK; that is a fact about the container
and says nothing about the app. Confusing the two once already produced a false
claim that contradicted what the owner had said in the same conversation.

What is genuinely unknown is which build carries which fix — that depends on when
the owner last built, and only they know. Ask; do not assume in either
direction.

### 28. Settle four separation decisions
**[STRIP §B, partial]** — The four undecided rebrand/separation questions carried
in the strip plan: (a) rename or keep the internal `Vibe*` functional names — see
#35; (b) `APPSTORE-EXCEPTION.md`, Stuart's GPLv3 §7 store-distribution exception,
moot for an Android-only fork — keep or remove; (c) README positioning, currently
"a fork of VibeSDR, a mobile SDR receiver" rather than a car FM radio; (d)
credits + GPL-3.0 notice, which have nowhere to live since `AboutOverlay.tsx` was
deleted — the settings panel shows only a version line. *Caveat: the mapping of
"four" to exactly these is my inference from §B; the plan does not number them.*

### 29. Retire HF and shortwave code
**[STRIP item 13]** — VTSBar.tsx is deleted, but `stations`, `eibi` (shortwave
schedules) and `userBookmarks` are all still live in the radio screen. **`userBookmarks`
IS the preset store and can never go.** So this is "retire the HF/shortwave parts
of `stations` + `eibi`", not a delete. HF/ham band-plan engine with no FM role.

### 30. Reduce the mode/step machine
**[STRIP item 18]** — ModeSelector and StepPicker are deleted, but the mode/step
state machine in the radio screen and `dataModes` (DAB/ADS-B predicates) are
untouched. CarFM forces `wfm`; stripping the rest is the state-machine surgery
the item warned about. Marked 🔴 high harm — do late, tsc+build gated.

### 31. Remove the native recording pipeline
**[STRIP item 10]** — The UI is gone (RecordingsOverlay, AudioSheet, TunerScreen
deleted), so nothing can start a recording. The Kotlin side is untouched and now
unreachable: `startRecordingNative` / `stopRecordingNative`, the MediaStore
publish, `VibeStreamModule.startRecording`. Keep `AudioPlayer` / `VibePowerModule`.

### 32. Resolve server-sharing orphans
**[STRIP item 11]** — ServerModeScreen and RtlTcpServerScreen are deleted, but
`rtlTcpServer`, `vibeServer`, `vibeAuth` and `mdns` were **kept by your decision**
("Server sharing: keep for now"), leaving them present but unreachable from any
UI. Backend-adjacent; the item is to decide whether they stay orphaned or go.

### 33. Audit Android permissions
**[NO DETAIL FOUND]** — `app.json` requests INTERNET, FOREGROUND_SERVICE,
FOREGROUND_SERVICE_MEDIA_PLAYBACK, RECORD_AUDIO, MODIFY_AUDIO_SETTINGS,
ACCESS_FINE_LOCATION, ACCESS_COARSE_LOCATION, CHANGE_WIFI_MULTICAST_STATE,
FOREGROUND_SERVICE_CONNECTED_DEVICE, WAKE_LOCK and more. Several are inherited
from VibeSDR features that have been removed — RECORD_AUDIO in particular, given
#31. The task is presumably to drop what the FM app no longer needs.

**Carnyx:** effectively settled by starting from nothing. `Cargo.toml` declares
INTERNET, ACCESS_COARSE_LOCATION and ACCESS_FINE_LOCATION and no others, each
with the reason written beside it. The foreground-service and boot permissions
are deliberately absent because nothing can use them — see #67.

### 35. Settle internal Vibe naming
**[STRIP §B]** — `VibeDSP`, `VibeLocalSDR`, `VibeStream*`, `VibeServer`,
`VibePowerModule`, `VibeMDNS`, the `vibedsp/` + `spyserver/` C++ trees, the
`vibesdr.local` mDNS host and the lowercase `vibeserver` detection marker. Left
as-is during the rebrand, and the plan records that **it was never confirmed you
wanted them kept — the assistant assumed it.** LARGE and risky: the Android
package rename showed JNI symbols must move atomically or the app crashes with
`UnsatisfiedLinkError`, and `vibeserver` / `vibesdr.local` are load-bearing for
server detection. Do as a deliberate, tsc+build-gated pass.

### 36. Throttle FFT frames
**[STRIP §E]** — The spectrum WebSocket **cannot be disconnected** — it carries
RDS (`type:"rds"` control messages are multiplexed onto the same socket as the
FFT frames, `UberSDRClient._handleSpectrumMessage`). But the FFT frames
themselves are waste behind the face, and `UberSDRClient.setRateDivisor` can
throttle them toward zero while keeping the socket open. **Unverified
assumption:** that the shim keeps emitting `type:"rds"` when FFT is throttled —
server-side `user_spectrum_websocket.go`, not in this repo. Needs an on-device
check before trusting.

### 37. Resolve dead AUTO logo path
**[AUDIT #15 / #5]** — `resolveStationLogo` returns `null` unconditionally because
`AUTO_LOGO_RESOLUTION` is off. Related: `refreshFreqMap` maps each channel to the
closest FM station within 100 km with **no receivability threshold**, and nothing
consults `score` even though `estimatedSignalDbForFreq` computes it — so tuning an
empty channel occupied by a 90 km class-A licensee paints that station's real logo
and callsign at full size over hiss.

**Carnyx:** the AUTO half is parity, not a gap — `AUTO_LOGO_RESOLUTION` is false
in CarFM and `enrichNow`, its only forced entry point, has no caller there
either, so Carnyx leaving `logos::resolve_logo` unwired matches. **The second
half is carried over and is a live defect.** `app::resolve` takes the nearest
licensee on the dial within `NEARBY_RADIUS_KM` (100 km) with no receivability
threshold, and `src/callsigns.rs` learns nearest-first on the same basis — so an
empty channel still names and paints a distant station over hiss.

### 38. Polish logo fit and memoise leaves
**[STRIP §D + §E]** — Two halves. (a) Logo sizing/fit: better optimise how fetched
logos are sized and laid out in tiles — wide wordmarks vs square marks, padding,
`resizeMode`, upscale of small hits. Deferred by request; the search + assign +
display pipeline works. (b) Child `React.memo` pass ("Fix 2"): redraw only the
changed leaf when the face re-renders (Tell / SignalWaves / StereoWave / LogoTile).
Parked pending an on-device profile — riskier (silent memo failures, stale-UI
bugs) and not verifiable in a still harness.

### 39. Resolve remote-backend access
**[STRIP item 12]** — TunerScreen, FmdxDial and `fmdxDirectory` are deleted, but
`FmdxAdapter` stays by your decision ("Remote SDR backends: keep"). FM-DX is a
reception source whose fate was deferred; the item is to decide the set. Note the
audit's #25 finding interacts: a connected remote OWRX/UberSDR/Kiwi currently
clears `tunerError`, so Settings claims a local RTL-SDR dongle is "Detected".

### 40. Audit background timers
**[STRIP §E]** — Explicitly **do not gate blindly.** Most candidates are already
off or genuinely used. Named: learned-bookmarks poll (30 s, `if (isLocal)` — RUNS
in carFm, but feeds the "what this aerial hears" list and may be used by
NearbyPicker); server-bookmarks (10 min) and EiBi (10 min) are gated off for
local/disabled; decoder spot-flush only starts with a decoder active. Confirm each
is genuinely unused in carFm before touching. Related **[AUDIT #29]**: nothing
pauses on background — the AppState handler returns immediately for tunerless, and
neither the RDS pump nor the 1.5 s poll is gated, so an hour backgrounded keeps
both loops running into a decoder whose output nothing renders.

### 41. Fix VibeSDR naming in briefs
**[STRIP §B + AUDIT #37]** — The remaining root `BRIEF-*.md` files (SpyServer ×2,
FM-DX adapter, the two URI-scheme briefs) still say "VibeSDR". They describe
subsystems that are still here, so they are kept; only the naming is inconsistent.
Separately the SDRScreen→RadioScreen rename left **24 stale references**: 18 in
`CARFM-STRIP-PLAN.md` (plus a line count of 2,747 against the actual 2,728), 2 in
`BUILTIN-TUNER-FINDINGS.md`, 1 in `VibeStreamModule.kt`, 3 in
`web/client/src/main.ts` — where `main.ts:2246` cites `SDRScreen.onShareStation`,
which exists nowhere under `src/`.

### 42. Close two code TODOs
**[NO DETAIL FOUND]** — Two literal TODO comments in the source. The specific
sites were not recorded; a `grep -rn "TODO" src/ android/` would re-identify them.

### 45. Move preset drag off the JS thread
**[NO DETAIL FOUND]** — Preset reorder uses a FLIP animation (300 ms, per
**[STRIP §A]**). The task implies the drag gesture currently runs on the JS thread
rather than the UI thread (react-native-reanimated worklet / Gesture Handler), which
would make it stutter while the RDS decoder and 1.5 s poll are working. No recorded
measurement.

### 46. Decide on a history rewrite
**[NO DETAIL FOUND]** — Whether to rewrite git history for the fork. Context that
bears on it: the repo carries VibeSDR's full history, `main` had 132 Rust build
artifacts committed for days, and `docs/LICENSING.md` records the GPL-3.0 fork
position. Likely motivations are removing inherited noise or large blobs. Your
decision, not a technical blocker.

### 47. Rewrite the GitHub repo metadata
**[NO DETAIL FOUND]** — The repo is still named `VibeSDR-CarFM` with, presumably,
inherited description/topics/homepage. Pairs with the README positioning question
in #28.

### 48. Establish logo dark-mode numeric parity
**[SESSION + design bundle]** — The logo dark-mode adaptation pipeline was built
in TypeScript from Claude Design's `DARK-MODE-LOGOS.md`, whose Python reference
implementation (`pipeline.py`) is **not in the bundle** — the design README says to
request it if you need to diff behaviour. "Numeric parity" means proving the TS
port produces the same output as that Python reference, stage by stage
(checkerboard → trim → key → route → remap/halo/plate/gate), rather than merely
looking right. Cannot be done without requesting `pipeline.py`.

**Carnyx:** carried over unchanged. The pipeline was ported again into
`src/logos.rs` (OKLab, connected components, box blur, the five stages) and is
tested against its own expectations, not against the Python reference. Still
blocked on requesting `pipeline.py`.

### 49. Replace the fabricated tuner diagnostics
**[AUDIT #25]** — `SettingsPanel.tsx:264` maps a **literal array** into the
diagnostics table: `[['Device','Realtek RTL2832U + R820T2'],['USB
ID','0bda:2838'],['Sample rate','2.048 MS/s']]`. None is read from anything — the
session's real rate is `hwSampleRate`, default 2.4 MS/s and user-settable.
Separately `detected: nwdActive ? false : !tunerError`, and `tunerError` means "no
live tuner session", so a **remote** OWRX/UberSDR/Kiwi connection clears it with no
dongle present: Settings then reads "Connected · Local hardware · RTL-SDR
(RTL2832U)", "Detected", "Sample rate 2.048 MS/s". `NwdRadioModule.kt:440-442`
describes its own getters as "the honest replacement for the representative
strings in the settings diagnostics" — the replacement never happened.

**Carnyx:** the fabrication is gone. `push_settings` publishes an EMPTY details
list with the reason in place — the panel describes an RTL-SDR, which is
VibeSDR's hardware path and barred from this tree, so it draws its own emptiness
rather than inventing a device. The `detected` contradiction cannot arise either:
one predicate, `tuner.is_available()`, is asked once and drives both the status
line and the source badge.

### 50. Fix the side-card edge fade
**[NO DETAIL FOUND]** — The prev/next peek cards flanking the hero (~0.88 scale,
~60% opacity) are specified with an edge fade where they tuck behind the hero. The
specific defect was not recorded. **[AUDIT #21]** covers a different peek-card
bug (warm cache warming the wrong sizes), which is not this.

### 52. Decide the FM tune step: 0.1 or 0.2 MHz
**[SESSION]** — **Answered by the hardware and needs applying.** The vendor band
plan reports `step=20`, i.e. **0.2 MHz** — the US/Americas raster. The app's seek
and manual tune should step 0.2 rather than 0.1 in region 2, which also halves the
number of dead channels a manual sweep walks through. The reading came from the
NWD probe's band-plan dump.

**Carnyx:** OPEN AND APPLICABLE, and not done. `numpad_commit` rounds to a tenth
(`(v * 10.0).round() / 10.0`) and the numpad accepts any tenth in band, so manual
tuning still steps 0.1. The vendor band plan's `step=20` is read into `BandPoint`
but nothing consults it.

### 53. Verify the hero-swap arming fix on device
**[SESSION + AUDIT #18]** — The hero prev/next swap is a 520 ms FLIP animation
(LOSSY #9). A fix was made to how the swap is *armed* (so it does not fire on a
retune that is not a preset step). Related and separately open: **[AUDIT #18]** —
the departing clone passes no `freqMhz`, so a logo-bearing card fades out as a
coloured `"100."` box, because `RadioScreen.tsx:2219` names a preset `` `FM ${mhz}` ``
whenever no PS was captured at save time, which is most saves on the NWD tuner.
Needs an on-device screen recording; a still harness cannot check it.

**Carnyx:** the morph itself was rebuilt and fixed twice this session — first for
an invented `opacity: 0` that made the hero vanish instead of move, then for a
`states`/`out` transition so the animation has something to animate from. 520ms
and translate-only, matching `centerTransform`. The AUDIT #18 half is addressed
differently: a preset stores its call sign (`prefs::Preset.call`) and falls back
to the learned map, so it is not named `FM <mhz>`. Still needs the on-device
recording.

### 54. Clear the four licensing items before any distribution
**[LICENCE]** — From `docs/LICENSING.md`. The two hard ones are the prebuilt
binaries under `cpp/sdr-kit/`, which ship per-ABI with **no LICENSE or COPYING file
alongside them**: (a) `librtlsdr.so` (Osmocom) — expected GPL-2.0, but
GPL-2.0-**only** would *conflict* with CarFM's GPL-3.0, so the exact upstream grant
must be confirmed; (b) `libusb1.0.so` — LGPL-2.1-or-later, compatible, but needs
the LGPL relinking offer if distributed as a binary. Resolution for both: identify
the exact upstream commit/version they were built from and vendor the licence texts
into `cpp/sdr-kit/`. The other two are the `APPSTORE-EXCEPTION.md` decision and the
missing credits/GPL-3.0 notice (see #28).

### 55. Migrate source-identity branches onto TunerCapabilities
**[AUDIT #33]** — `src/services/tunerCapabilities.ts` **has no importer anywhere**,
so nothing depends on it. It also contradicts itself: the doc-comment says NWD
RadioText is "unsupported on current evidence" ten lines above a profile marking it
`'live'`; the header's "59/21/10 branch sites" actually measures 12/7/6; and the
MIGRATES pointer names `CarFmFace.tsx:1070-1072` where the branch is at 1076/1078.
The task is to actually route the scattered `nwdActive ? … : …` source checks
through this table. *Note: I set `signal: 'measured'` on the NWD row late in the
session — the row was still claiming `'estimated'`/untested.*

**Carnyx:** no equivalent table exists and none has been missed. The source
picker is real (`settings::Source`), and the one place a source check would
scatter — the meter's scale — deliberately follows the source actually running
rather than the stored selection.

### 56. Remove the dead VTS notification subsystem
**[AUDIT #31]** — `vtsNotif`, `vtsMenuName`, `vtsMenuFreq` are write-only state.
Worse than dead: `liveLogo` is read at exactly one place — inside the effect that
builds the discarded `vtsNotif` — and the logo effect calls `resolveStationLogo`
purely to populate it. The `vtsNotif` dep list includes `liveStation.text`, so
**every decoded RadioText change forces a re-render of the whole screen and the
SVG-heavy face for a value nothing renders.**

### 58. Find a safe signal-level read, after one broke the audio
**[SESSION]** — Historical: an earlier attempt to read signal strength cut the
audio, which is why every read is now treated as dangerous. **Largely resolved
since:** the shipping read is `seek(currentFrequency)`, whose packed return gives
strength in the high 16 bits and the landed frequency in the low 16; it was
verified over twenty presses with no audio interruption, and it now runs unattended
on a 20 s watch. What remains open is whether `getRadioRDSDataArm`'s sibling
`getRadioRDSStrengthArm(int)` offers a **passive** read — one that does not command
the tuner at all — which would remove the last risk and allow a faster cadence.
Never probed.

**Carnyx:** the shipping read is the same `seek(currentFrequency)`, on CarFM's
own 20s cadence (`signal::LEVEL_POLL_MS`) with the post-retune schedule at 1s and
4s. The passive `getRadioRDSStrengthArm` is still unprobed, so the open half of
this item carries over exactly as written.

### 60. Evaluate SDR signal strength numbers
**[SESSION]** — On the RTL-SDR path the face derives its wave count from
`waveStrength(db)`, which maps a dB figure through a 12-segment intermediate. The
question raised was what that dB number actually represents: it is computed from
two of the three factors that determine real signal strength (the third, noise
figure / bandwidth normalisation, is not accounted for), so the scale is not
comparable to the NWD path's unitless level and may not be comparable between
dongles either. Task is to establish what it means before trusting it.

**Carnyx:** not applicable until there is an SDR path. `signal.rs` treats the NWD
level as an ordinal and never prints a unit, precisely so this question stays
answerable later.

### 64. MAYBE: expose local vs DX seek sensitivity
**[SESSION]** — A real car radio's LOCAL/DX switch trades seek sensitivity: LOCAL
stops only on strong stations, DX stops on weak ones too. **Fully unblocked** —
the vendor service exposes both `setLoc(int)` and `setRadioSensitivityArm(int)`,
and the constants `RADIO_LOC_FM_STOP = -65` / `RADIO_DX_FM_STOP = -70` are in the
decompiled `RadioConstant`. Marked MAYBE: you were lukewarm on it.

**Carnyx:** applicable and untouched. `signal::LEVEL_DX_FLOOR` and
`LEVEL_LOC_FLOOR` are already carried as constants; nothing exposes a switch.

### 65. MAYBE: AM band support, only if distributing
**[SESSION]** — You checked the local AM stations and found none you wanted, so
this is only worth doing if the app is distributed to others. Blocked on a design
decision first: you do **not** want the traditional FM1/FM2/FM3/AM band-bank model,
but are considering **preset groups**, and your groups would be **mixed-band** — so
presets need to carry a band before AM can be added coherently.

---

# CARNYX (66–)

Opened by the Slint rebuild. Numbered on from CarFM's 65 so no number is ever
reused across the two trees.

### 66. Tune the RDS decoder against real captures
**PENDING — deferred by the owner until Carnyx is at parity with CarFM.** The
decoder is a faithful lift (constants, consensus gates, quality ring, scrolling-PS
verdict, RT+ clearing all verified against `nwdRds.ts` field for field), so this
is not a port defect — it is the calibration pass CarFM never finished either.
The thresholds worth revisiting are `PI_CONFIRM` 3, `PI_DISPLACE` 12, the
16-sample `QUALITY_MIN` floor and the 25s `RDS_STALE` window, all of which were
set from one drive. Needs real group captures from the unit to tune against; do
not start it before #26.

### 67. Get a foreground service and a boot receiver into the APK
**THE SERVICE IS IN. THE RECEIVER IS NOT.** Half done, and the half that is done
is the one that answers "the app looks like it starts fresh when I switch back".

**What landed.** `android/app/src/main/java/com/ninthfreak/carnyx/CarnyxService.java`
is a real foreground service, declared in the Gradle manifest with
`foregroundServiceType="mediaPlayback"` and the three permissions it needs.
`java/com/ninthfreak/carnyx/CarnyxProcess.java` starts it, `src/android/service.rs`
binds that over JNI, and `src/lib.rs` calls it once at start-up — after
`App::with_tuner`, so the notification can carry the dial, and still while the
activity is in front, because from Android 12 a background
`startForegroundService` throws.

**THE PART THAT IS NOT OBVIOUS: it takes THREE files because Android has two
class loaders here.** Everything in `java/` is dexed by `build.rs`, embedded with
`include_bytes!` and loaded at run time by an `InMemoryDexClassLoader` — that is
how a pure NativeActivity reaches binder at all. A class that exists ONLY there
cannot be a manifest component: Android constructs one through the APPLICATION's
class loader, which has never heard of a loader Rust built after start-up, and
gets a `ClassNotFoundException`. So `CarnyxService` is in the GRADLE source set
(AGP compiles it into the APK's own dex) and only the starter is in the runtime
dex. The starter names the service by STRING through a `ComponentName`, so
neither tree has to resolve into the other at compile time.

**A CORRECTION to the paragraph above, found while checking the Gradle config
rather than by being told.** The first version of it said the service could not
live in `java/` at all. Not true: `android/app/build.gradle.kts` puts `../../java`
in the Gradle JAVA source set as well as the aidl one, so AGP compiles that whole
tree into the APK's dex too, and a service class there WOULD be constructible
under Gradle. What is actually true is narrower — it would be compiled twice, be
dead weight in the embedded dex, and still be absent under cargo-apk. The
placement stands; the reason for it was overstated. (Same check turned up that
`CarnyxProcess` is itself compiled into both dexes under Gradle, and that
parent-first class loading means the APK's copy is the one that runs. Same source
either way, so it changes nothing at run time.)

**And one hazard that came out of it.** `java.srcDirs(...)` in AGP ADDS to the
source set where the Java plugin's same-named property REPLACES it, and this
build relied on the difference without saying so: if it replaced, `src/main/java`
would be dropped and `CarnyxService` would never be compiled — with no build
failure, only a `ClassNotFoundException` on the unit. The AGP API could not be
fetched from here to settle which it is (Google's Maven is unreachable and it is
not mirrored on Central), so the config now names BOTH directories explicitly,
which is correct under either reading.

**ONLY THE GRADLE BUILD HAS IT.** cargo-apk packages no Java and has no `service`
field, so under `cargo apk` the class is genuinely absent and the platform refuses
the component. That path is expected, caught, and logged at info — the app runs
exactly as before. `tools/build-apk-gradle.sh` is now the build to use on the
unit, and its header says so.

**POST_NOTIFICATIONS is declared and deliberately never requested.** A runtime
request needs someone to tap Allow, which on a dashboard at night is nobody — the
manifest's own note about location, applied here. When it is absent the platform
suppresses the service's notification but STILL RUNS THE SERVICE IN THE
FOREGROUND, and the foreground is the entire point. Declaring it lets a driver who
wants the line grant it in Settings; nothing depends on their doing so.

**`session.rs` is NOT retired and its header now says why.** The service prevents
the restarts it can; it makes a process expensive to kill, not unkillable, and it
does not exist at all in the cargo-apk build. The restore covers the rest. That
file previously said Carnyx "CANNOT HAVE" a service — true of cargo-apk, written
before the Gradle spike, and corrected.

**ON THE UNIT: IT BUILDS, IT INSTALLS, IT RUNS.** The owner built the Gradle APK
with the service declared and ran it. So AGP compiles `CarnyxService`, the
manifest merges, the APK installs over the previous one, and the app starts with
the service's start call on its path — none of which had ever happened before.

**WHAT THAT DOES NOT YET SAY.** An app that runs is not a service that runs. Two
things are still unconfirmed, and one line of the log now answers each:

- Did the service enter the foreground? `service: started` in the settings log
  says the platform accepted the start; `service: none` says it refused or the
  class is absent. Added because the Java side reports to logcat and THIS UNIT
  HAS NO adb — the settings panel's log is the only channel a driver can read.
- Did it do its job? The `session:` line already carried the answer and nobody
  had thought to read it that way: it prints `app #N in this process`. Switch to
  another app, come back, open Settings. `app #2 in this process` means the
  process survived and the service earned its keep. `app #1` with a new launch
  number means it was killed and restarted anyway.

**WHAT WAS AND WAS NOT VERIFIED OFF-DEVICE — read this before trusting the rest.**
There is no Android SDK and no NDK on the machine this was written on, so:

- Both Java files compile CLEAN against a real Android API-34 framework jar
  (`org.robolectric:android-all:14-robolectric-10818077`, fetched for the check
  and not vendored), with `-Xlint:all` and no diagnostic in our sources. One
  deprecation was found and removed that way: `Notification.Builder.setPriority`
  has been dead since API 26, where the channel's importance governs, and 26 is
  this app's floor.
- `javap` confirmed the two JNI descriptors the Rust names —
  `(Landroid/content/Context;)V` and `(Ljava/lang/String;)Z` — are the ones the
  compiled class actually carries.
- The manifest parses as XML.
- The RUST IS NOT COMPILED FOR THE TARGET. `cargo check --target
  armv7-linux-androideabi` cannot run here: skia-bindings needs an NDK. Every JNI
  construct in `service.rs` is copied verbatim from `nwd.rs` or `net.rs` rather
  than composed, which is the most it can be held to without a device.
- The Rust and the run-time behaviour were unproven until the unit ran it; the
  build and launch are now confirmed and the two log lines above are what settle
  the rest.

**Still open — the receiver.** So the app comes back when the unit wakes:

    <receiver android:name=".WakeReceiver"
              android:exported="true" android:enabled="true">
        <intent-filter>
            <action android:name="com.nwd.ACTION_OS_WAKE_UP" />
            <action android:name="android.intent.action.BOOT_COMPLETED" />
        </intent-filter>
    </receiver>

`com.nwd.ACTION_OS_WAKE_UP` is the one that matters and BOOT_COMPLETED is the
fallback, not the other way round: CarFM's manifest records that THIS UNIT SLEEPS
on ACC-off rather than shutting down, so BOOT_COMPLETED never fires on an ignition
cycle (CarFM `android/app/src/main/AndroidManifest.xml:91-94`). It has to be a
manifest receiver, because the process is killed while the unit sleeps and only a
manifest-declared receiver gets restarted. It goes in the Gradle source set beside
the service, for the same class-loader reason.

**Also worth doing, now that the service exists:** the notification line is set
once at start-up and never updated, so it shows the dial the app opened on. Wiring
`service::start` to re-post on tune is a few lines (calling it again updates the
notification); it was left out because the process pinning is what #67 is for and
a live line is a separate promise.

**FIVE THINGS BIT DURING THE GRADLE SPIKE**, all handled in the script, and each
cost a round trip because none of them fails where it is caused:

1. `command -v cargo-ndk` asked the wrong question — it is a cargo SUBCOMMAND.
2. `ANDROID_PLATFORM` PINS `android_jar`'s lookup, so a level you do not have
   makes every level you do have invisible; the panic still says "No Android
   platforms found". cargo-ndk sets it too, so unsetting it in the shell does not
   clear it. The script now exports `ANDROID_JAR`, which outranks all of it.
3. cargo-ndk defaults to API 21 while skia-bindings hardcodes 26, and the triple
   wins over the `-D`, so libc++ fails on `strtof_l` after the whole Skia build.
   `--platform` is now read from `min_sdk_version`.
4. `versionCode` — cargo-apk derives 16777472 from the crate semver; a
   hand-written `1` would have been refused as a downgrade. Now derived the same
   way, so the two packagers install over each other in both directions.
5. Nothing stripped the libraries: 172.2 MB against cargo-apk's 86.4 MB.
   `strip = "split"` is a cargo-apk-only key. The script now splits the symbols
   out with the NDK's `llvm-objcopy`/`llvm-strip` itself rather than relying on
   AGP, which only strips when it can find an NDK and merely warns when it
   cannot. **The flag was wrong twice before it was right**: `--strip-debug`
   removes the DWARF but leaves `.symtab`, and with Skia linked statically that
   table is most of what is left — libraries stayed at 95.0/78.9 MB with every
   `.debug_*` section already gone. `--strip-unneeded` takes both and keeps
   `.dynsym`, which `System.loadLibrary` needs; never `--strip-all`. Final:
   46.1 MB (arm64) and 28.9 MB (armv7) on disk, ~30 MB packaged against
   cargo-apk's 86.4, since Gradle deflates entries where cargo-apk stores them.

   Two of those rounds were spent on a file that was current and still wrong.
   **AGP packages incrementally, updating the existing APK in place rather than
   writing a fresh zip**, so when the libraries shrank by ~100 MB the obsolete
   bytes stayed in the file as dead space: the central directory listed only the
   live entries, every tool agreed the archive was ~30 MB, and the file stayed
   172.2 MB with a fresh timestamp. Rebuilding did not shift it; deleting it and
   rebuilding did. `tools/build-apk-gradle.sh` now removes the APK before Gradle
   runs, and `tools/check-apk.sh` takes the most recently built APK rather than a
   fixed path and fails when a file is much larger than the sum of its entries.

Signing needed nothing: cargo-apk and AGP both default to
`~/.android/debug.keystore`, alias `androiddebugkey`, so the certificate matches
and app data survives the swap. The risk that mattered was checked beforehand and
was absent: Slint resolves to plain `NativeActivity`, not GameActivity
(`slint-1.17.1/Cargo.toml:64-68` → `i-slint-backend-android-activity/native-activity`),
and `android.app.NativeActivity` is a framework class with no dependency to add.

### 68. Build a stripped release APK for both ABIs
**PENDING.** armv7-linux-androideabi and aarch64-linux-android. The unit is
32-bit ARM and an arm64-only APK will not install. This is about the RELEASE
artefact — stripped, both ABIs, signed with a keystore held outside the
repository. Debug builds already happen on the owner's machine; see the README's
three cargo-apk traps.

### 69. Add the confirm dialog "clear all logos" needs
**PENDING.** `logos::clear_all` and `clear_all_prefs` exist and have no caller —
the settings row refuses, logging that it "needs the confirm dialog first",
because destroying every stored logo on one tap is not something to do without
one.

### 70. Finish the accessibility annotations
**PENDING, and partly done already.** Three files carry them — `ui/numpad.slint`
(6 `accessible-*` lines), `ui/logo-search.slint` (13) and `ui/nearby.slint` (3,
on #76's TabButton). The other ten have none: `app`, `face`, `hero`, `icons`,
`overlay`, `presets`, `settings`, `status-bar`, `tokens`, `types` — of which
`face`, `hero`, `presets`, `settings` and `status-bar` hold real controls and
are the ones that matter (nearby's station rows and chips are still bare too). CarFM sets `accessibilityRole` and
`accessibilityLabel` on every control.

### 71. Verify the remaining CarFM citations
**PENDING.** Line-numbered citations across `ui/`, concentrated in
`settings.slint` (37) and `logo-search.slint` (17); #76's rewrite cut
`numpad.slint` from 30 to 8, so the census here is smaller than when this item
was filed — recount before verifying. They have never been checked against the lines they name. Ten
behaviour-bearing ones were verified and all held; the rest are mostly chrome,
where a drifted line number is noise rather than a defect — but a drifted CLAIM
is not.

### 72. Decide the fate of the dead diagnostics rows
**PENDING.** Now that CarFM's probes are not features, the settings panel still
draws a "Raw RDS capture" toggle and six action rows — export capture, the four
vendor probes, and "Save to file" — and every one of them logs *"not available
without the head unit"*. That reason is false: they were never ported, so they
will say the same thing on the unit. Either strip them or make each say its real
reason. "Clear log" works; the tuner log itself is Carnyx's own and stays.

### 73. Confirm this session's fixes on the unit
**PENDING — needs a build from the owner, not a blocker in the tree.** Each of
these was written against a defect the owner reported from the unit, and each
needs the same route back: build, drive, report. The hardware
seek no longer cancelling itself, the refused tune no longer spinning the dial,
the vendor-driven retune dropping the old level, the 1s/4s post-retune read
schedule with two retries, the 1.5s getter poll on its own thread with the dial
backstop and the MCU audio self-heal, the STEREO settle window, the
between-launch session restore, and the push-rate work (`push_all` 1.94ms →
0.10ms, a wake plus a frame with the nearby list open 56ms → 10ms, all measured
on x86 by `examples/pushbench.rs`).


### 74. Make the morph shot deterministic
**PENDING.** `examples/shot.rs` renders `hero-step-morph` by stepping a preset,
sleeping past the arm timer WITHOUT drawing, then advancing the clock four times —
so the frame it catches depends on wall-clock timing. Measured: two runs of
identical code differ by about 40,000 pixels, while a static shot like `tuned` is
bit-identical run to run. That makes the one shot covering this animation useless
as a regression baseline; a change can only be judged by looking at it.

Worth trying: a `Platform` with a controllable `duration_since_start`, which would
make animations deterministic and let a shot name an exact point in the 520ms
window instead of racing it. That would also allow a shot per BEAT — one inside
the far card's 120ms delay, where the card must still be invisible, which is the
beat most likely to regress silently.

**A CORRECTION.** This item previously said no probe in this tree had ever driven
a Slint animation and that the morph could only be judged on the unit. That was
wrong: `shot.rs` has photographed this morph mid-travel all along, and it is what
verified the work — the ghost, the travelling card and the faded far peek are all
visible in `shots/hero-step-morph.png`. The probe written alongside that claim,
`examples/morphprobe.rs`, never started the animation because it drove the loop
through `common::pump`, which drains the tuner queue mid-morph and hands
`draw_if_needed` a closure that renders nothing. It has been deleted rather than
left in the tree asserting something untrue.

### 75. Grow the hero card during the step morph
**DONE, both halves.** The step morph is a real FLIP in both directions: the
incoming hero starts at the source peek's footprint and grows into the hero rect,
and the OUTGOING card is now a real `HeroCard` carrying the station that was
playing, starting on the hero's own rect and shrinking into the slot it lands in.

Slint 1.17.1 is the newest release and still exposes no scale transform to user
code (`Transform` carries `//-is_internal`), so both cards are scaled by
ARITHMETIC — every dimension multiplies through one number: padding, corner
radius, border, shadow, layout spacing, the logo plate, the star, the power button
and all three font sizes. The same trick `PeekCard` already used.

The note that used to sit here called this infeasible because a marquee measures
its own text. That was wrong: the marquee is `RadioTextStrip`, a SIBLING of the
card. Nothing inside `HeroCard` measures text.

**The outgoing half.** It was a `PeekCard` standing in for the hero. That is
CarFM's shape — its peek node IS the outgoing card, FLIPped from the hero's rect —
but CarFM uses a NON-UNIFORM transform that stretches the peek's content out to
635x180 on the way. Nothing here can distort an element, so the square was fitted
to `min(card-w, card-h)` and the departing station was represented for the whole
520ms by a 180px square that had never been on screen. Carrying the hero across
instead is better than the stretch, not a substitute for it: the first frame of
the morph is now the face exactly as it stood. `HeroSnapshot` is that capture,
taken in `step_morph` before the tune for the same reason `ghost-preset` is.

**It collapsed four defects.** The travel clamp is gone — a card shrunk to the
peek's footprint centres inside the slot and cannot overflow, so the full 348px
throw is back on the wide track and the tall track is no longer a 21px twitch. The
outgoing beat became visible. And the hero's rect is no longer a HOLE during the
arming window, which is however long a synchronous retune takes on a head unit —
that hole was the "hero card disappears for a moment" reported from the car.

**A bug shipped in the first half, found and fixed here.** On the tall track the
row takes its height from the card, and the card that flew was also the card the
row measured — so its preferred height shrank with its own scale and the height
binding multiplied by the scale AGAIN. The hero collapsed quadratically and its
rect was empty at frame 0, and the row's `min-height` collapsed with it, re-laying
the face out mid-morph. The wide track hid all of it, because there the height is
98% of the row and never asks the card anything. The resting hero is now a
separate, never-scaled card that the row measures; the two that fly are drawn only
while the morph runs.

Verified by rendering: `examples/outprobe.rs` differences the morph's first frame
against the frame taken before the step and requires it back identical, measures
the scaling on the cards' TOP EDGES (widths merge on the tall track, heights do
not), and reads the direction mapping off the two slots either side of the hero.
Four configurations, both tracks, both directions. `examples/edgeprobe.rs`
confirms neither card ever reaches a bezel.

**`examples/scaleprobe.rs` is deleted rather than kept.** Its predicate for "a
card body" was `r < 236 || g < 238 || b < 241 || |r-b| > 6`, and the page
background is `#EEF1F5` — `|238-245| = 7`. It matched the background, so it had
been measuring the GAPS between cards and passing for the wrong reason. `outprobe`
covers what it claimed to.

**Still open, and cosmetic:** the departing card is not pixel-exact at frame 0 on
the tall track. `HeroCard` takes its height from the row, and the row's height is
the INCOMING card's, so a step between a logo hero and a no-logo hero starts the
departing card at the new card's height. Cloning the old node is the only fix and
there is no cloning here.

### 76. Fold the numpad into the tune overlay as a second tab
**DONE.** The nearby picker and the direct-entry keypad are one 900×600 overlay
with two tabs — "Nearby stations" and "Enter frequency" — and the hero's frequency
is display-only text. Built to the mini-handoff "Tune overlay: Nearby / Enter
frequency tabs", which states it wins over ANDROID §6 where they disagree.

**THE REFERENCES IT NAMES ARE NOT IN THIS TREE.** It cites `NearbyPicker.dc.html`
at v1.14.6; `docs/design/handoff/` is v1.10.0 and its NearbyPicker has no tabs at
all — it still draws the title-and-subtitle header this change removes. So every
new metric comes from the mini-handoff's own §3 and §4, which state all of them,
and nothing is inferred from a file that is not present. The unchanged halves —
station list, filter bars, FCC footer — keep the v1.10.0 citations they were built
with. Worth re-checking against v1.14.6 whenever the bundle is updated.

**Removed:** the standalone numpad modal and its card, `Overlay.numpad`, the
`open-numpad` callback through Face and HeroRow, and the hero frequency's
TouchArea. **Restyled, not redrawn:** the keypad, seek row and CANCEL/TUNE pair
are the same components at §4's sizes — readout 78→62dp, keys 64→48dp at radius
12, actions 60→52dp, column gap 14→9dp, capped at 440dp and centred. The old
compact height track is gone; §4 gives one metric set and a scroll container.

**Two host-side decisions the mini-handoff leaves open, both flagged rather than
buried:**

1. WHEN THE OUT-OF-BAND LINE LIGHTS. §6 hands `freqError` to the host without
   saying when, and §5 makes TUNE close the overlay whatever the buffer holds — so
   an error raised BY the commit, as the old card raised it, would never be on
   screen long enough to read. It has to be live. Live the naive way is worse than
   useless — "1" is out of band on the way to "105.1" — which is the exact failure
   already on the record here. So `band_prefix_ok` lights it only when no in-band
   frequency's own display string still starts with what was typed: "7" warns at
   once, "1" and "105." never do. Enumerated over the 206 dials rather than
   reasoned about, so it cannot disagree with the formatter.
2. THE ERROR COPY'S BAND ENDS. §4 writes "Outside 87.5–108.0 MHz band"; the old
   line rendered "108" because `{FM_HI}` drops the trailing zero. Now formatted to
   one decimal on both ends.

**A deviation kept on purpose:** the readout's border turns amber with the
warning. §4 gives it a plain border and puts the error on its own line; the border
costs nothing when there is no error and the line alone is easy to miss at a
glance in a moving car.

Verified by rendering and by driving: `examples/numpadprobe.rs` holds the entry
rules, the always-closing TUNE, the seeks (landed AND still in flight), the
CANCEL/scrim restore, the tab-switch sweep stop, the same-tab re-tap no-op, and
fourteen buffers against the warning rule. Shots: seven `freq-tab-*` states over
three surfaces (1024×614 light+dark, 360×800, 800×360 incl. scrolled — the slice
where the column must scroll to reach CANCEL/TUNE); the Nearby tab's coverage is
the pre-existing `nearby-*` group, whose header the tab bar replaced. An earlier
version of this entry said "seven shots cover both tabs on four surfaces" —
wrong on both counts, corrected here per the completion-claims rule.

**THE DEEP-REVIEW PASS THAT FOLLOWED (51 raw findings, 18 confirmed) fixed:**

1. CANCEL during an in-flight hardware sweep neither stopped it nor restored —
   the restore was gated on the dial having MOVED, and on the NWD front end a
   seek is fire-and-forget: mid-sweep the dial still equals the restore point,
   so the filter dropped it and the sweep landed after the driver said no. The
   restore now also fires while a sweep is in flight; the tune is both the stop
   and the restore.
2. NOTHING EVER SET `scanning`. The UI property existed, the hero's scanning
   face, the readout's un-dim-while-sweeping rule and the stop-sweep-on-
   tab-switch branch all read it — and no production code wrote it, so all three
   were unreachable on the device. It is app state now: raised when a seek is
   handed off, cleared by the next frequency report or any tune. The vendor's
   `notifyRadioScanState(int)` stays unread: its values are undocumented and
   wiring a guess would be task 49's fabricated-diagnostics failure again.
   (Steering-wheel seeks deliberately do NOT raise it — CarFM's face shows no
   scan state for hardware seeks either; only the tab's own SEEK does.)
3. The live warning CONTRADICTED the commit: it tested display-string prefixes,
   `numpad_commit` rounds — so "87.46" warned and then tuned 87.5, and "87.4"
   warned though one more digit rescues it. `entry_can_tune` now asks
   numpad_commit itself over the keypad's own grammar (search via numpad_press,
   ≤4 digits + one point), and `committable_buffers_never_warn` walks every
   typable buffer proving no committable one warns.
4. A tap on the ALREADY-ACTIVE tab wiped the typed buffer and re-based CANCEL's
   restore point to the swept dial. Same-tab taps are a no-op now.
5. TabButton had no 48dp floor — 46px real on 800×360, on the only two controls
   that reach the keypad, in the commit that argued the floor was mandatory for
   the keys beneath them. Floored like CloseButton.
6. The ✕/scrim tab split lived in a private view function no probe could reach —
   against the same commit's own stated principle. It is `on_nearby_dismiss` in
   Rust now, and the probe drives it on both tabs, mid-sweep included.
7. The probe itself certified behaviours it never exercised: its seek tuner was
   a no-op, so "stops a running seek" asserted a flag nothing set and the
   CANCEL-restore assertion hid behind a guard that never opened. It now runs a
   two-mood tuner (lands / hangs mid-sweep) and both fixes were sabotage-tested:
   re-introducing either bug fails the probe.

**Also recorded, deliberate:** the wide tab hint's doubled spaces around the
interpuncts are now VERBATIM from §3 (they had been normalised to single); §4's
tabular numerals are dropped openly (Slint exposes no OpenType features), so the
readout re-centres per keystroke; the freq column's bottom padding is s(24), not
§4's 8 — the corner-clip rule wins where no footer covers the card's corners;
the contract ships one `freq-seek(int)` instead of §6's onSeekUp/onSeekDown plus
two extra finished-value props (`freq-display-dim`, `freq-error-text`), per the
house rule that Slint computes nothing; and CANCEL restores the frequency the
TAB was last switched to (§5's snapshot sentence), which diverges from its
"overlay opened on" sentence only in the seek-then-tab-bounce sequence.

**Not carried over, because there is nothing left to carry:** §7's "delete the
standalone numpad dialog" and "remove the hero frequency's pressed/ripple state" —
the frequency's TouchArea had no pressed state to remove.


### 77. Cover the hero rect when a step hands off to nothing
**PENDING.** When a step discards no card (`hand-off` false — forward off an
unsaved dial), the morph's first frame shows NO card in the hero rect: the
resting card hides the moment `m-out` moves, the outgoing card is gated on
`hand-off`, and the arriving card is still shrunk on the source slot. The
station that was playing vanishes with no exit, and the centre stays
under-covered through the early travel. The gating is deliberate — the unsaved
dial lives in no slot, and flying it into one would draw a lie — but "no
destination" need not mean "no exit": the departing card could fade in place at
the hero rect instead of travelling. That is a design decision (what should the
exit look like?), not a wiring fix, so it is filed rather than improvised.
`outprobe` deliberately excludes this case; whatever is decided needs a fourth
claim there. Pre-dates #75's outgoing work in shape — the incoming-only morph
had the same hole — but #75 made the covered cases good enough that this one now
stands out.


### 78. One station per frequency in the nearby list — a CarFM divergence
**DONE, and it is a DELIBERATE DIVERGENCE from the reference.** `rank_nearby`
now keeps only the highest-scoring row on each dial before it truncates. CarFM
keeps every licence and truncates to 100; this keeps one per frequency.

**IT IS A CORRECTNESS FIX, NOT A TIDY-UP.** A dial holds one station, so a second
row on 88.7 offers the driver something the receiver cannot give them — and
because the 100 cap was spent on those duplicates, real frequencies fell off the
bottom. Measured through the shipped table and the real query:

| | in range | shown before | freqs covered | freqs in range | **never shown** |
|---|---|---|---|---|---|
| Madison | 120 | 100 | 69 | 75 | **6** |
| Chicago | 172 | 100 | 61 | 75 | **14** |
| New York | 236 | 100 | 63 | 90 | **27** |
| Los Angeles | 152 | 100 | 57 | 68 | **11** |
| rural Nevada | 5 | 5 | 4 | 4 | 0 |

After: 75, 75, 90 and 68 rows — fewer rows AND every frequency with a licence in
range. The cap can never bind again: the FM band has 101 channels at 0.2MHz
spacing and the densest metro measured has 90.

**Where it came from.** The owner asked "there could never be 100 different
stations all within tunable range, could there?" There could not, and the list was
never claiming there were — it was showing licence records. The table is 10,646
full-power FM, 8,279 FX translators and 1,808 FL LPFM.

**WHAT IT GIVES UP, all three checked rather than assumed:**

1. A translator or LPFM can be the only row on a frequency a full-power station
   also licenses — 1 to 7 frequencies per metro. Every case examined was right:
   Madison 102.9 keeps a 1km LPFM over a station 76km away, Chicago 97.5 a 1km
   translator over one at 92km. Displacement only happens when the small
   transmitter is very much closer, which is when it is what the radio receives.
2. Those displaced full-power rows no longer reach `Callsigns::relearn`, which
   filters to `service == "FM"`, so a few frequencies lose their learned no-fix
   name. The name lost belonged to a station 80-90km away nobody could hear.
3. The 1-5 signal arcs re-normalise, and the shift says the old range was wrong:
   CarFM's 100 rows gave `[22, 31, 29, 13, 5]` — 22 stations with no arcs at all,
   because unhearable co-channel rows dragged the bottom down. One row per dial
   gives `[6, 13, 32, 19, 5]`. `station_strength` already documents that any
   filter re-normalises.

**Not given up:** the ORDER, and every per-row number. The eight best rows at
Madison are the eight CarFM produced, and `the_madison_query_returns_what_carfm_returned`
still pins each row's distance and score to CarFM's own arithmetic. Only the row
COUNT and the tail moved. The test now also asserts the invariant the divergence
exists for: no two shown rows share a frequency.

**Done in #81:** small transmitters are now dropped past their own reach.

### 79. Stop re-deriving the nearby list for a car that has not moved
**DONE.** Every GPS fix re-ran the whole query — bounding box over 20,733 rows,
rank, view, publish — and the app's own comment records that a parked car with a
lock produces one every two seconds. `hero_row`'s cache made it worse: it compared
the fix with `==` on two f64s, so metres of GPS noise missed it on every fix and
re-ran the licensed-station lookup as well.

Both now ask whether the car has moved 250m, against a 100km radius and distances
shown to the kilometre. Compared against the position the picker was BUILT from,
not the last fix seen, so a car creeping 10m at a time cannot accumulate past the
threshold unnoticed. `examples/pushbench.rs` gained the case and measures it:

    before   fix, jittered 3m   2.148 ms      fix, moved 5km   2.156 ms
    after    fix, jittered 3m   0.001 ms      fix, moved 5km   2.026 ms

Desktop x86; the unit is 32-bit ARM. A real move still costs what it costs.

### 80. Make a crash on the unit say what it was
**DONE.** "The whole app crashes if I stay on the window too long" had nothing
behind it: a Rust panic goes to logcat, the unit has no adb, and the diagnostics
log dies with the process. `src/crashlog.rs` is a panic hook that writes the
message and location to a file beside the session snapshot; the next launch reads
it into the settings log as `crash: the last run panicked — …` and deletes it.

Catches Rust panics — BorrowMutError, unwrap on None, index out of bounds. Cannot
catch a Java exception, an OOM kill or a SIGSEGV in Skia, and that absence is
itself a narrowing: a crash with no `crash:` line is one of those three. Verified
by installing the hook and panicking for real, not only by unit-testing its
halves.

**THE CRASH ITSELF IS NOT DIAGNOSED AND NOT FIXED.** This is instrumentation, not
a repair. Next step is a drive that reproduces it and a look at the log.


### 81. Drop translators and LPFM past their own reach
**DONE. A SECOND DELIBERATE DIVERGENCE from CarFM**, and the one #78 left open:
a 100 W LPFM at 80km was in the list and not on the radio.

**THE DISTANCE IS DERIVED, NOT PICKED.** The FCC's LP100 class — 100 W at 30m
HAAT — is protected to 60 dBuV/m at **5.6 km**, which is the regulator's own
statement of where an LPFM serves. Free-space field goes as `sqrt(ERP)/distance`,
the same law `receivability_score` already uses, so every other small
transmitter's equal-field distance is `5.6km x sqrt(erp / 100W)`. The ERPs are
effectively constants set by regulation — measured over the shipped table,
translators are 0.25 kW at the 50th, 90th AND 99th percentile, LPFM 0.1 kW at all
three — so this resolves to about 17km for an LPFM and 27km for a translator,
and scales correctly for the outliers either way.

**THE ONE JUDGEMENT IS THE 3x MARGIN**, and it is a field strength rather than a
preference: three times the distance is `20*log10(3)` = 9.5 dB down, so the cut
is the ~50 dBuV/m contour — fringe, well below the 70 dBuV/m the FCC calls city
grade, but still usable mono in a moving car. It is deliberately generous, and
the data barely notices: moving it from 3x to 5x changes the result by ONE row in
Madison and by nothing at all in Chicago, New York or Los Angeles, because the
translators being removed sit at a median of 81-87km where no threshold in this
range saves them.

**THE ORDER OF THE TWO RULES IS LOAD-BEARING.** The reach filter runs BEFORE #78's
one-row-per-dial rule. A translator can outscore a full-power station on the same
frequency by being nearer, so filtering afterwards would let it win the dial and
then be dropped, taking the frequency with it and hiding the station the driver
can actually hear. Filtering first hands the dial to the station behind it.
`an_unreachable_translator_yields_the_dial_rather_than_taking_it` holds that.

**Full-power FM is not touched** and keeps the whole 100km radius. A 100 kW class
C genuinely reaches that far, the ranking already orders them, and narrowing them
is a different question.

> **SUPERSEDED BY #82**, which removed that exemption. The reasoning in the
> paragraph above does not survive being checked: `reach_km` already returns
> 531km for 100 kW, so the filter was never going to cut a big station inside a
> 100km search, and the exemption only ever spared weak ones. Kept as written
> because the table below is #81's measurement and is still the record of what
> #81 did; #82 carries the current numbers.

Effect, measured through the real query (rows, and of those, small transmitters):

| | before #78 | after #78 | after #81 |
|---|---|---|---|
| Madison | 100 | 75 (29 small) | **59** (13 small) |
| Chicago | 100 | 75 (21 small) | **63** (9 small) |
| New York | 100 | 90 (29 small) | **65** (2 small) |
| Los Angeles | 100 | 68 (14 small) | **58** (4 small) |
| central Nevada | 5 | 4 | **0** |

**CENTRAL NEVADA GOING TO ZERO IS THE CORRECT ANSWER, not a regression.** Its
entire band within 100km is three translators: one of unknown power at 91km and
two of NINE WATTS at 94km. None is receivable anywhere, in any conditions, so the
picker reports `NoStations` — "No FM stations within range of this location" —
which is true. `a_band_of_nothing_but_distant_translators_is_empty` pins it.

The head of the list is still untouched: Madison's best eight are the eight CarFM
produced, and WMUU-LP — an LPFM 1km away — is still among them, because the rule
is about reach and not about service.

**Still open:** nothing here narrows full-power FM, and a class A at 95km is
listed. That is defensible (it can be receivable on flat ground) and is where the
next question would go if the list still feels long. **Closed by #82.**

### 82. Decide reach on power, not on the licence category
**DONE.** #81's reach filter exempted `service == "FM"` from the distance test.
That exemption is gone; `within_reach` is now one line for every service:

```rust
distance_km <= reach_km(row.erp_kw)
```

**THE EXEMPTION NEVER PROTECTED WHAT IT WAS WRITTEN TO PROTECT.** It was
justified as sparing big stations from a formula derived for small ones, and
`reach_km` scales as `sqrt(ERP)` — it returns 531km for 100 kW and 130km for
6 kW. Above **3.543 kW**, the power whose reach equals the 100km search radius,
the test cannot fire inside the search at all. 7,831 of the 10,646 `FM` rows are
above that line, so for three quarters of them the exemption changed nothing in
either direction. It was load-bearing only for the 2,610 below it and the 205
with no ERP.

**AND THOSE ARE THE ROWS IT SHOULD NOT HAVE SPARED.** "Full power" is a licence
category, not a wattage: 132 of the 183 sub-100 W `FM` rows sit below 92.0 MHz,
in the reserved non-commercial band, where a small educational licence is
ordinary. The old rule therefore dropped an 89 W translator at 90km and kept
WFAR — **fifteen watts** — at the same distance, on the wording of the licence
rather than the signal. Also spared: KGCM at ONE watt and 13km, KBPK at 19 W and
30km, WRRC at 20 W and 78km.

Effect, measured through the real query:

| | after #81 | after #82 | removed |
|---|---|---|---|
| Madison | 59 | **58** | WSUP |
| Chicago | 63 | **61** | WXNU, WZKL |
| New York | 65 | **61** | WFRS, WTSR, WRRC, WFAR |
| Los Angeles | 58 | **54** | KCRU, KMLA, KJAI, KBPK |
| Bozeman MT | 32 | **29** | KYPX, KYPB, KGCM |
| central Nevada | 0 | **0** | — |

**EVERY REMOVED ROW WAS AMONG THE LAST FOUR OF ITS LIST** — Madison lost its
59th of 59, Los Angeles its 55th through 58th of 58 — because the score is the
same physics as the filter, so a row that barely fails reach also barely scores.
In none of the six did the dedupe hand the freed dial to another station: there
was none behind it to hand it to. The head of each list is unchanged; Madison's
best eight are still CarFM's best eight.

The new Madison tail is the cut landing where it should. WGFB is 2.4 kW with a
reach of 82.303km, sitting at 82.218km — on the list by **85 metres**, and last
because it barely fits. `the_madison_query_returns_what_carfm_returned` pins it
to the digit.

**THE HONEST COST IS THE MARGIN CALLS.** WSUP (2.5 kW at 96.1km, reach 84.0) and
WFRS (1.7 kW at 70.8km, reach 69.3 — inside 2%) are now cut, and a 3x fringe
margin is nowhere near accurate enough to adjudicate a 2% difference. They go
anyway: one rule applied to everything is worth more than a second threshold
tuned to rescue two rows, and the table has no HAAT column, so there is no data
with which to model the antenna height a full-power licence usually buys. If the
list later looks short in flat country, the lever is `FRINGE`, which is one
constant and now governs every service at once — and it moves more than it did:
3x to 5x was worth one row in Madison and nothing in Chicago, New York or Los
Angeles while full power was exempt, and is now worth +2, +1, +2, +1, +4 Bozeman,
+0 central Nevada.

**A SIDE EFFECT WORTH NAMING: a NaN ERP is now dropped rather than ranked.**
`clamp_min` passes NaN through, so `reach_km` is NaN, so every comparison in
`within_reach` is false. That is the wanted answer for a corrupt row, but it also
means `rank_nearby` can no longer put a NaN into its own sort — which would have
quietly hollowed out the regression test for the NaN abort. The comparator
is now `score_order`, a named function tested directly on a `Vec<f64>`, so the
guard survives independently of whatever filters run upstream of it.

Pinned by: `a_transmitter_is_dropped_past_its_own_reach` (break-even either side
of 3.543 kW, and all four weak rows the exemption used to spare),
`a_row_whose_erp_is_not_a_number_is_dropped_before_scoring` (the filter half) and
`a_nan_score_sinks_instead_of_aborting` (the sort half).

### 84. Close the four defects the preset sweep confirmed
**DONE**, all four, each with the evidence attached.

**The band guard on save.** `toggle_save` was the one writer that did not check
the band, while `prefs::from_json` drops anything outside 87.5..108.0 on the way
back in — so a tile saved out of band reached disk and was silently deleted at
the next launch. Now refused, with a diagnostics line, as a guarded match arm
rather than an early return so the refusal still publishes. Whether this unit can
report out of band is UNVERIFIED — it has no band button — but the guard costs one
comparison and the failure it prevents is silent data loss.

**The single-entry strip.** `(active + dir).rem_euclid(1)` is always `active`, and
the `moves` test suppressed the morph while the retune went out anyway — dropping
the level and provoking a report that runs `reset_for_retune`, blanking name,
RadioText and PTY. The tune is now gated on the same test. Pinned by
`stepping_a_single_preset_does_not_retune`, which counts `tuned` lines in the
diagnostics log; sabotage-checked by making the tune unconditional again (8 lines
against 7). **A REGRESSION CAME OUT OF THIS AND `wheelprobe` CAUGHT IT**: the
skipped `tune` was also the publish, so the face stayed on the previous station
while the radio sat on the new one. The branch pushes explicitly now.

**The strip never scrolled to the tuned tile.** `viewport-x` had two writers, both
nav buttons, and nothing answered `active-index`. A `changed active-index` handler
now scrolls the minimum needed, on either axis, and leaves the rail alone when the
tile is already in view so it cannot fight the paging buttons. The rule is one
pure function returning the new offset — a Slint function cannot declare a local,
which the first cut did and which does not parse. Before and after are visible in
`shots/many-presets.png`: 24 presets with the dial on the last of them showed six
unlit tiles and a scrollbar at rest; it now shows the lit tile.

**Nothing exercised panel-press → drain → step.** Every call to `drain_events`,
`step_preset`, `step_preset_from` and `apply_panel_action` was above the test
module; the one occurrence inside it was a doc comment. So `cargo test` never
stepped a preset, which is why the vendor-dial bug, the nested-drain overwrite and
the dead release-edge guard all shipped green. Two tests now build a real `App` on
a headless window and drive the whole path. **SLINT'S PLATFORM IS PER-THREAD**, so
the harness installs one per thread rather than once — a `Once` left every test
but the first falling through to winit and dying on a missing `DISPLAY` — and each
call hands out a fresh window adapter, because sharing one fails on the second
`AppWindow`. Both tests sabotage-checked.

**Still open from that sweep:** the release-edge double step, which needs one
observation from the unit — see #83.

### 83. Find out what panel key 14 is
**OPEN.** Code 14 reached the app EIGHT TIMES in CarFM's drive log of
2026-08-03 and is in no row of the vendor's decompiled `handlePanelKey` table, so
nothing knows what button it is. `PanelKey::from_code` returns `None` for it and
the app does nothing, which is the right behaviour for an unknown code and is not
the same as understanding it.

**THE SUSPECT LIST IS NOW SHORT**, because the fascia is known (see
`crate::android::PanelKey`, and #4 below). The owner has, on the wheel: `ch+`,
`ch-`, volume up, volume down, and `mode`, which switches apps. On the head unit:
the Android navigation buttons and a volume control. `ch+`/`ch-` are 62/63 and
are accounted for. So 14 is most likely `mode` or one of the volume keys — the
three controls that are handled by the MCU and the system, and whose broadcast
behaviour has never been checked.

**HOW TO SETTLE IT, and it needs the car rather than this tree.** `apply_event`
logs `panel key {code} ({named}) {action}` for every broadcast that arrives, so
pressing each button once and reading the diagnostics panel names the whole set
in one pass. That same pass answers two other open questions: whether the MCU
re-broadcasts the release edge (count the lines for one push of `ch+` — one or
two), and whether volume is on this transport at all.

**WHY IT IS WORTH KNOWING rather than leaving as noise.** If 14 is `mode`, it is
the app being sent to the background, which is the event `session.rs` and the
foreground service both exist to survive — and an app that could SEE it could
persist its snapshot on the way out instead of hoping. If it is volume, it is
nothing and can be ignored forever, which is also worth writing down once.


---

# COMPLETED (31)

**Carnyx:** applicable and blocked on the same design decision. `prefs::Preset`
carries `mhz` and `call` and no band, so the blocker is unchanged.

### 1. Rip out scale-to-fit; responsive dp/sp CarFmFace (ANDROID §0)
**[Design handoff]** — The face was authored at one canvas size and scaled; §0 of
ANDROID-IMPLEMENTATION required real responsive layout across five surfaces (two
tracks, wide and tall) using dp/sp rather than uniform scaling.

### 2. Build seek-digit slide (LOSSY #6, now required)
**[Design handoff]** — The frequency digits slide (±14, 0.25→1, ~200 ms) when
seeking. Listed in LOSSY-ELEMENTS as lost in the first port, then made required.

### 3. Re-verify all five surfaces at real dp against v1.4.0 references
**[Design handoff]** — Dudu7 full, ⅔ slice, ⅓ slice, S21 landscape, S21 portrait.

### 4. Rebrand app identity (app.json + package.json)
### 5. Rename Android native package com.vibesdr.app → com.ninthfreak.carfm
**[STRIP]** — The rename showed JNI symbols must move atomically or the app
crashes with `UnsatisfiedLinkError` — the precedent cited against #35.
### 6. Rename deep-link scheme vibesdr:// → carfm://
### 7. Swap visible branding VibeSDR → CarFM (keep VibeServer/VibeDSP/VibeLocalSDR/vibesdr.local)
**[STRIP §B]** — The kept names are exactly what #35 revisits; the plan records
that keeping them was an assistant assumption, never your confirmation.
### 8. Faithful README rebrand + drop original screenshots + fork-attribution line
### 9. Strip items 1–9 (dead files, iOS, watch, Siri/CarPlay, browser)
**[STRIP]** — Commits `a2d7a91`, `592f91f`. Android Auto kept when CarPlay was cut.

### 13. App: steering-wheel next/prev preset → hero card swap animation
**[SESSION]** — The wheel arrives as the MCU broadcast `com.nwd.action.ACTION_KEY_VALUE`,
not through Android's input pipeline — which is why MediaSession and activity-key
capture saw nothing. The vendor service also jumps to its own hardware preset
slot; CarFM steps its own preset immediately after so the app's order wins.

### 14. App: CarFM must claim the FM audio source to produce sound
**[SESSION]** — Confirmed trigger is `ACTION_APP_IN_OUT` with `extra_app_id=8`;
verified by `AudioManager.isMusicActive()` rather than by ear.

### 16–20. Logo dark-mode: core / 5 pipeline stages / Skia I/O / cache + hook / picker UI
**[Design DARK-MODE-LOGOS]** — Station logos are arbitrary user uploads that break
on a dark background. Built as: an OKLab + connected-component + blur core
(Skia-free so it is Node-testable), the five pipeline stages (checkerboard → trim →
key → route → remap/halo/plate/gate), sRGB-guaranteed decode/encode via Skia, a
cache + import hook + treatment enum, and the treatment picker UI. Numeric parity
against the Python reference remains open — see #48.

### 21–25. Band themes: registry + matcher / face wiring / 6-tap panel / fonts / per-motif art
**[Design EASTER-EGGS-BUILD §12]** — Five themes resolved from RadioText: AC/DC,
The Beatles, Led Zeppelin, Nirvana, Nine Inch Nails. Real supplied typefaces
bundled as `res/font` (a system font standing in is a failed build), per-motif
vector art, and a hidden force panel revealed by six taps on the about line.

**Carnyx:** NOT PORTED, and the picker that was ported has been removed by the
owner's decision (this session). Six taps revealed a control that moved a radio
button and changed nothing — `settings::egg_id` named five bands and no code read
it. Porting it properly is 434 lines across `bandThemes.ts` and `bandArt.tsx`,
with traced motif art needing to be baked through `tools/gen-icon-paths.py` and
real typefaces bundled. Reopen as a Carnyx item if it is ever wanted; do not
treat its absence as a parity gap.

### 27. Verify animations on device
**[STRIP §A]** — Hero swap FLIP (520 ms), preset-reorder FLIP (300 ms), seek-digit
slide (~200 ms), on a real screen recording.

### 34. Clear 41 dead useState
**[AUDIT #31]** — Seven write-only hooks named: `serverLost`, `reinit`,
`owrxDspDefaults`, `dabSpeed`, `vtsNotif`, `vtsMenuName`, `vtsMenuFreq`. The VTS
three are still open as #56.

### 43. Verify the vendor JSON channel — answered: absent
**[SESSION]** — Checked whether the vendor service exposed a structured/JSON
metadata channel. It does not.

### 44. Settle the stereo pill
**[Design §4.1]** — Superseded twice since: v1.14.3 moved it from three curved
waves to speaker cones, with both cones drawn on mono and the label pinned to the
widest string so the cones cannot shift between STEREO/MONO/blank.

**Carnyx:** the pill was a live defect here and was fixed this session. Two
places cleared the pilot that CarFM does not clear — a frequency notification and
the RDS expiry — and CarFM's 2000ms trailing settle window was missing, so it
"almost never lit up". `examples/stereoprobe.rs` pins all three.

### 51. Locate the real MCU transport for RSSI and RDS
**[SESSION]** — The answer that unlocked most of the app: the AIDL getters are
hollow (`getRtMessage()` hardcoded `""`, `psName` empty for a passive client,
`getPTYType()` returns 0, `isStreroOn()` stuck true), and the real channel is
`NwdFmManager.getRadioRDSDataArm()` returning one synchronised RDS group as 16 hex
chars, plus `seek(freq)`'s packed strength+frequency int.

### 57. Make CarFM follow the head unit's day/night switch
**[AUDIT #2]** — The ROM keeps `androidUiMode` at DAY permanently, so
`useColorScheme()` is useless. The signal is the `NwdIllState` broadcast
(`extra_ill_state=1` = headlights on = night). Known remaining gap: the broadcast
reports **changes only** and no getter for current illumination state has been
found, so starting after dark with the lights already on produces no event.

**Carnyx:** ported. The illumination watch is started in `App::with_tuner`
rather than inside connect, so a session that never binds the tuner still follows
the headlights. The same "changes only, no getter" gap applies.

### 59. Make the tuner-source picker actually select a tuner
**[SESSION]** — The Settings picker (RTL-SDR / built-in / Auto) was presentational
only. Note this has since been narrowed deliberately: the meter's scale now follows
the source actually *running*, not the stored selection, because honouring the
selection blanked the meter whenever the two disagreed.

### 61. Rebuild the signal meter to SIGNAL-METER.md
**[Design SIGNAL-METER]** — Rebuilt again since, to v1.14: thresholds
31/48/60/70/85, a half-step ring at 45% past each band midpoint, a sub-floor dot
ramp below 31, and dotting that spreads inward from the leading drawn arc.

### 62. Swap Talking Heads for Led Zeppelin
**[Design v1.13.0]** — Talking Heads was withdrawn and Led Zeppelin replaced it.
Leftover: the `nameBlock` field survives in the theme registry and in `CarFmFace`
with **no theme defining it** — it belonged to Talking Heads, so that branch is
unreachable. Not in the task list.

### 63. Make the tuner picker drive the face
**[SESSION]** — See #59; the rule has since been inverted deliberately.
