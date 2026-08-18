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
**PENDING.** Carnyx does not survive being switched away from: return and the
face redraws and the RadioText re-decodes. CarFM survives because
`VibeStreamService` runs in the foreground even on the built-in tuner
(`startNwdControlSession` → `startForegroundMedia` → `startForeground`), which
pins the process above the launcher's cleaner and the low-memory killer.
**cargo-apk cannot declare either component** — `ndk_build::manifest::Application`
has one activity and no `service`, `receiver` or `provider` field, and there is no
custom-manifest escape hatch. The same absence blocks CarFM's `BootReceiver`,
which is what brings the app back when the unit wakes on ACC. Needs the APK built
by something that can write its own manifest: Gradle, or a forked cargo-apk. Until
then `src/session.rs` survives the restart instead of preventing it, and the
`session:` line at the top of the log says which of the two restarts happened.

**GRADLE SPIKE CONFIRMED ON THE UNIT.** `android/` holds a complete Gradle build
of the same APK — wrapper checked in, `tools/build-apk-gradle.sh` runs
`cargo ndk`, strips, then `gradlew assembleDebug`. It installed over the
cargo-apk build and ran; the owner's words were "the most complete Carnyx yet".
So the packager question is settled and the service and receiver are now
buildable. They are written into `AndroidManifest.xml` as comments, with the
permissions and API-34 typing each needs, and go in next.

The risk that mattered was checked beforehand and was absent: Slint resolves to
plain `NativeActivity`, not GameActivity (`slint-1.17.1/Cargo.toml:64-68` →
`i-slint-backend-android-activity/native-activity`), and `android.app.NativeActivity`
is a framework class with no dependency to add. The whole contract between
packager and Rust turned out to be two lines cargo-apk emits: the activity class
and the `android.app.lib_name` meta-data.

**Five things bit on the way, all now handled in the script**, and each cost a
round trip because none of them fails where it is caused:

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
and app data survives the swap.

**cargo-apk remains the default build until the service lands**, and the two
manifests are kept in parity by hand.

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
**PENDING, and partly done already.** Two files carry them — `ui/numpad.slint`
(three controls) and `ui/logo-search.slint` (four) — 19 `accessible-*` lines
between them, including `accessible-checkable`, `accessible-item-selected` and
`accessible-enabled`. The other eleven have none: `app`, `face`, `hero`, `icons`,
`nearby`, `overlay`, `presets`, `settings`, `status-bar`, `tokens`, `types` — of
which `face`, `hero`, `presets`, `nearby`, `settings` and `status-bar` hold real
controls and are the ones that matter. CarFM sets `accessibilityRole` and
`accessibilityLabel` on every control.

### 71. Verify the remaining CarFM citations
**PENDING.** 112 line-numbered citations across `ui/`, concentrated in
`settings.slint` (37), `numpad.slint` (30) and `logo-search.slint` (17), with the
remaining 28 spread over `overlay`, `nearby`, `presets`, `app`, `hero` and
`status-bar`. They have never been checked against the lines they name. Ten
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
**PENDING.** CarFM's morph FLIPs the incoming hero from the source peek's rect to
the hero rect, interpolating position on a cubic ease-out and SIZE on a quintic
one so the scale settles slightly ahead of the travel. Carnyx travels the card and
does not resize it.

`PeekCard` can be scaled because every dimension in it multiplies through one
`scale` property — that is what makes the outgoing card's shrink possible.
`HeroCard` cannot: 159 lines of independently derived metrics, paddings, corner
radii, font sizes and a marquee whose width is MEASURED from its own text, so a
uniform scale there is a rebuild of the component rather than a property. Slint
1.17 still exposes no scale transform to user code — `Transform` carries
`//-is_internal` (i-slint-compiler-1.17.1 builtins.slint:500-507) — and animating
`width`/`height` re-lays the card out every frame, which reads as jitter.

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
