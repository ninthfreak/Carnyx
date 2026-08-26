//! The wiring: the one place where every real subsystem meets the face.
//!
//! Everything Slint draws is a finished value, so this module is where the
//! finishing happens — the station database resolves a dial into a call sign,
//! the RDS decoder's published state becomes the hero's text, the signal maths
//! becomes five meter properties, the picker's nine properties are published
//! together, and the settings panel's whole surface is derived from
//! [`crate::settings`]. Nothing below decides anything itself; it joins the
//! modules that do and pushes the result.
//!
//! ## What is real here and what is not
//!
//! Real, and running: the FCC station table (`src/stations.rs`, 20,733 rows off
//! the shipped SQLite file), the RDS decoder and its consensus gates
//! (`src/rds.rs`), the signal maths and its hysteresis (`src/signal.rs`), the
//! nearby picker's ranking and filters, the logo search's state machine, and the
//! settings panel's derived layer.
//!
//! Faked ON THE HOST ONLY, because the framework is absent from this container:
//! the tuner (`crate::android::FakeTuner`, the tuner builder's own fake, which
//! drives the same `ingest_*` path the device drives), the position, the RDS
//! pump's source, and the logo search's network and image decoder. Every one of
//! them lives in [`crate::fake`] and is named so. On the device all four are
//! real — [`App::with_tuner`] takes the tuner and the [`Net`] pair from the
//! caller, and `android_main` is the only caller that can build them.
//!
//! Absent entirely, and reported as such rather than stubbed into looking
//! present: the confirm dialog that must stand in front of "clear all logos",
//! and every DIAGNOSTICS action that crosses the framework edge.
//!
//! ## Threading
//!
//! Tuner events arrive on whatever thread the vendor uses. The sink therefore
//! does nothing but push onto a lock-guarded queue, and [`App::drain_events`]
//! applies them on the UI thread. On the host the fake emits synchronously and
//! the queue is drained immediately after each command; on the device the drain
//! needs a wake, which is `slint::invoke_from_event_loop` — see
//! [`set_event_wake`]. The queue exists precisely so that the UI thread is the
//! only thread that ever touches an `AppWindow`.
//!
//! The logo worker is the SECOND producer on that path, and it gets its own
//! queue for one reason: its events are not tuner events and must not be
//! reachable from `apply_event`, where a stray match arm could route a
//! thumbnail into the RDS decoder. Same wake, same drain, same UI thread.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::android::{FakeTuner, Tuner, TunerEvent};
use crate::logos::prefs::HeroFlags;
use crate::logos::store::LogoStore;
use crate::logos::{service, ImageCodec, LogoNet, Raster};
use crate::rds::{self, RdsDecoder, RdsState};
use crate::signal;
use crate::station::{brand_color, clean_call, format_mhz, plate_label};
use crate::stations::{NearbyPicker, NearbyState, StationDb, StationRow};
use crate::{fake, settings};
use crate::{
    AppWindow, BandGear, BatteryState, DiagAction, EggTheme, GenreColumn, HeroSnapshot,
    LogoPlate,
    NearbyStation, NearbyTab,
    Overlay, Preset, TunerAction, TunerDetail, TunerGlyph, TunerSource,
};

// ── The event queue ──────────────────────────────────────────────────────────

static QUEUE: Mutex<VecDeque<TunerEvent>> = Mutex::new(VecDeque::new());
/// The logo worker's own queue. Separate from the tuner's so a thumbnail can
/// never reach `apply_event`, and so the two producers cannot deadlock behind
/// one lock — the worker holds its sink across a decode.
static LOGO_QUEUE: Mutex<VecDeque<service::Event>> = Mutex::new(VecDeque::new());
type Wake = Box<dyn Fn() + Send + Sync>;
static WAKE: OnceLock<Wake> = OnceLock::new();

/// Install the "an event is waiting" signal.
///
/// On Android this is `slint::invoke_from_event_loop(|| …drain…)`. On the host
/// there is no running event loop in the screenshot example, and calling
/// `invoke_from_event_loop` there fails — so the host installs nothing and the
/// caller drains after each command instead.
pub fn set_event_wake(wake: impl Fn() + Send + Sync + 'static) {
    let _ = WAKE.set(Box::new(wake));
}

fn queue() -> std::sync::MutexGuard<'static, VecDeque<TunerEvent>> {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner())
}

fn logo_queue() -> std::sync::MutexGuard<'static, VecDeque<service::Event>> {
    LOGO_QUEUE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The logo worker's outside world, handed in rather than built here.
///
/// Both halves are `Arc<dyn …>` because the worker thread owns a clone of each
/// while the UI thread keeps the codec for decoding STORED renditions — a
/// station's art has to be turned into pixels on the thread that draws it, and
/// that read never touches the network.
///
/// `None` is the host: there is no `HttpsURLConnection` and no `BitmapFactory`
/// off the device, so the screenshot path keeps [`crate::fake::FakeLogoSearch`]
/// and says so on the face rather than pretending a search ran.
pub struct Net {
    pub http: Arc<dyn LogoNet>,
    pub codec: Arc<dyn ImageCodec>,
}

thread_local! {
    /// The App on THIS thread, for the wake hop to find.
    ///
    /// A thread-local rather than a captured handle because
    /// `slint::invoke_from_event_loop` demands a `Send` closure and `Rc<App>` is
    /// not `Send` — which is correct, since the App owns an `AppWindow` and that
    /// must never leave the UI thread. `Weak`, so a closed window drops.
    static CURRENT: RefCell<Option<std::rc::Weak<App>>> = const { RefCell::new(None) };
}

/// How many Apps this PROCESS has built.
///
/// The whole diagnostic for "why did the radio start fresh", and it works
/// because a static is per-process and `android_main` runs per-ACTIVITY. Come
/// back from another app and read the log:
///
/// * `app #2 in this process` — the Activity was destroyed and re-created while
///   the process lived. That is a configuration change the manifest did not
///   claim, and `config_changes` in `Cargo.toml` is where it is fixed.
/// * `app #1 in this process`, every time — the process itself was killed. No
///   manifest flag prevents that; only a foreground service does, and cargo-apk
///   cannot declare one.
static APPS_BUILT_HERE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Drain and republish on the current thread's App. This is what the wake hop
/// runs; it does nothing if there is no App here, which is the correct answer
/// for a late event after the window has gone.
pub fn drain_current() {
    let app = CURRENT.with(|c| c.borrow().as_ref().and_then(std::rc::Weak::upgrade));
    if let Some(app) = app {
        app.drain_events();
        app.push_all();
    }
}

/// The App on this thread, if there still is one.
///
/// Every timer callback below goes through this rather than capturing a handle:
/// `CURRENT` is a `Weak`, so a window that has gone finds nothing and the late
/// tick does nothing, which is the right answer for all of them.
fn current() -> Option<Rc<App>> {
    CURRENT.with(|c| c.borrow().as_ref().and_then(std::rc::Weak::upgrade))
}

/// One level reading now. The 1s post-retune read and the retry both land here.
fn read_level_current() {
    let Some(app) = current() else { return };
    app.read_level();
    app.push_meter();
}

/// How long a probe waits before it runs.
///
/// ONE FRAME, not a delay. The only thing that has to happen in between is a
/// repaint carrying the "reading…" line, and the event loop renders after every
/// poll — so this is the smallest interval that reliably lands on the far side
/// of one render rather than a guess at how long the probe takes.
const PROBE_DEFER_MS: u64 = 16;

/// How long after a sleep release the MCU's source register is still evidence
/// ABOUT that release rather than about ordinary source arbitration.
///
/// Ten seconds because the vendor is slow: `NwdBridge.setAudioEnabled`'s own
/// note records the MCU acting "a second later", and the poll's cadence is
/// 1.5s — so a window of one or two turns would miss the reading it exists to
/// catch. Nothing depends on the exact value; it only decides which of two
/// sentences a log line uses.
const SLEEP_RELEASE_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

/// The hidden band-theme picker's first row — SettingsPanel.tsx:628's
/// `{ id: null, label: 'Off (auto-detect)' }`.
///
/// A ROW LIKE THE OTHERS, not a "clear" affordance beside them. It is what the
/// reference draws, and it is also the only way out of a forced theme that reads
/// as part of the same list: a driver who forced AC/DC and wants their radio back
/// taps the row above it rather than hunting for a different control.
const EGG_MENU_OFF: &str = "Off (auto-detect)";

/// The dark-logo picker's state, from the assign that opens it to the answer.
///
/// WHY IT EXISTS AT ALL, in the reference's own words: *"A human glance caught
/// five errors the metric scored as successes across five logos, so the pick is
/// a default, never a verdict."* The pipeline routes, gates and chooses, and it
/// is right most of the time; this is the screen where a person disagrees.
///
/// `items` EMPTY MEANS THE ADAPTATION IS STILL RUNNING, which is a real state
/// and not a placeholder — building four treatments is seconds of pixel work on
/// this unit, taken on the worker while the driver watches. The reference spins
/// an `ActivityIndicator` over exactly this wait.
struct DarkPick {
    /// The store key. Held here rather than read back from the search model,
    /// which `close_logo_search` clears out from under this.
    base: String,
    /// Every treatment the master can build, in the pipeline's order.
    items: Vec<(crate::logos::dark::stages::Treatment, crate::logos::Raster)>,
    /// The one the pipeline chose on its own, drawn AUTO.
    pick: crate::logos::dark::stages::Treatment,
    /// Which row is selected, as an index into `items`.
    selected: usize,
}

/// What a probe calls itself in the log. One place, so the "reading…" line, the
/// footer and the unavailable line cannot drift apart.
fn probe_name(action: settings::Action) -> &'static str {
    match action {
        settings::Action::ProbeStockRadio => "stock radio probe",
        _ => "keep-alive probe",
    }
}

/// Run whichever probe the last tap asked for. See `State::pending_probe`.
fn run_pending_probe_current() {
    let Some(app) = current() else { return };
    app.run_pending_probe();
}

/// The 4s correction — and the re-phasing of the periodic watch, which is the
/// same call.
///
/// CarFM restarts the watch here rather than taking a second one-off reading, and
/// says why: "startLevelWatch reads once immediately and then sleeps the cadence,
/// so restarting it here is both the 4s correction and a clean '…and every 20s
/// from now' — without it the watch is a free-running native thread a retune
/// never touches, and the next update after this could land anywhere in its
/// cycle" (RadioScreen.tsx:2839-2846). `NwdBridge.startLevelWatch` reads before
/// its first sleep too, so the assumption holds on this side.
fn correct_level_current() {
    let Some(app) = current() else { return };
    app.state.borrow().tuner.start_level_watch(LEVEL_WATCH_MS);
    app.drain_events();
    app.push_meter();
}

/// How long after a step the frame tally is written.
///
/// The morph is 520ms; the slack lets the last frames land before the count is
/// read, and a press that arrives inside the window simply restarts it.
const MORPH_REPORT_AFTER: std::time::Duration = std::time::Duration::from_millis(640);

/// Write what the last morph actually managed, in the driver's own log.
///
/// THE ONE MEASUREMENT THIS PROJECT CANNOT TAKE ANY OTHER WAY. Every figure in
/// the performance review came from a desktop running the software renderer, and
/// the APK renders with Skia on a 32-bit ARM GPU. `morphbench`'s own table is the
/// scale to read this against: at 30ms a frame the morph gets 17 frames and reads
/// as motion, at 90ms it gets 6 and is visibly stepped, at 170ms it gets 3 and is
/// a cut with a tail. Now it says which.
///
/// STRICTLY THIS COUNTS ANIMATION ADVANCES, one per turn of the event loop, and
/// a turn is a frame only on a loop that draws every turn. The device's does.
/// A harness that pumps timers without drawing counts turns and not frames, which
/// is exactly what `a_step_reports_the_frames_it_got` demonstrates, and
/// `morphbench` is where the two are shown to agree when drawing really happens.
fn report_morph_frames() {
    let Some(app) = current() else { return };
    let (frames, elapsed) = {
        let mut s = app.state.borrow_mut();
        let Some(since) = s.morph_since.take() else { return };
        (std::mem::take(&mut s.morph_frames), since.elapsed())
    };
    let ms = elapsed.as_secs_f64() * 1000.0;
    // A morph that drew NOTHING is the most important case this can report, and
    // dividing by it would panic, so it is spelt out rather than computed.
    let per = if frames == 0 {
        "no frames at all".to_string()
    } else {
        format!("{:.1} ms/frame", ms / f64::from(frames))
    };
    let line = format!("frames: {frames} in {ms:.0} ms — {per}");
    app.state.borrow_mut().settings.log.push(&stamp(), &line);
    app.push_settings();
}

/// Put the settled pilot on the pill. What [`STEREO_SETTLE`] waits for.
fn settle_stereo_current() {
    let Some(app) = current() else { return };
    let changed = {
        let mut s = app.state.borrow_mut();
        let pending = s.stereo_pending.take();
        match pending {
            Some(on) if s.stereo != Some(on) => {
                s.stereo = Some(on);
                true
            }
            _ => false,
        }
    };
    if changed {
        app.push_hero();
    }
}

/// Write the parting snapshot for the current thread's App.
///
/// Called from the Android lifecycle listener in `lib.rs`, which runs on this
/// same thread inside `poll_events` — so the borrow below is safe in exactly the
/// way a borrow from a callback is. Does nothing when there is no App, which is
/// what a lifecycle event before construction or after teardown deserves.
pub fn persist_session_current(parting: crate::session::Parting) {
    if let Some(app) = current() {
        app.persist_session(parting);
    }
}

/// Start the post-retune read schedule over.
///
/// CarFM's `NwdRadioFrequency` handler, verbatim in shape (RadioScreen.tsx:2836-2846):
/// clear whatever is pending, re-arm the retry budget, read at 1s and correct at
/// 4s. The evidence for those two numbers is in [`crate::signal`] — the first
/// reading after a tune is inflated by +17.7 on average, with cases of +45, +48
/// and +57, and the excess is almost entirely inside the first second. So the
/// meter shows something fast and then tells the truth, which beats a long dash.
/// Advance the reception-loss band by one step. THE ONLY CALLER IS THE POLL.
///
/// `settle_dotted_pairs` is a per-sample state machine — previous count in, next
/// count out, fed back — and `LOSS_BAND_MARGIN` is calibrated against the rate it
/// is stepped at. CarFM steps it in exactly one place, inside its 1.5s poll
/// (RadioScreen.tsx:3060-3069), and says why in a comment worth keeping: "the
/// ring's percentage drifts by fractions of a point every poll, and both the
/// rounding and the hysteresis have to happen once, against the band actually on
/// screen."
///
/// Returns whether the band moved, so the caller can skip a republish.
///
/// THE LOSS FIGURE is the complement of the decoder's block-A match rate. A
/// PROXY, and never "% intact": this tuner exposes no per-block validity, so
/// errors in C and D — where the text lives — are invisible to it, and RadioText
/// will arrive mangled while this reads healthy.
///
/// GATED ON STALENESS, because the expiry deliberately does NOT reset the decoder
/// — so its ring survives the carrier going quiet, and a figure held over from
/// before that would be describing air this radio is no longer receiving.
fn settle_dotted(s: &mut State) -> bool {
    let loss = if s.rds_stale {
        None
    } else {
        s.rds.quality().pi_match_pct.map(|pct| 100.0 - pct)
    };
    let next = signal::settle_dotted_pairs(s.dotted, loss, signal::dottable(s.level));
    let moved = next != s.dotted;
    s.dotted = next;
    moved
}

fn arm_level_schedule(s: &mut State) {
    s.level_first.stop();
    s.level_correction.stop();
    s.level_retry.stop();
    s.level_retries = signal::LEVEL_RETRY_MAX;
    s.level_first.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::from_millis(signal::LEVEL_FIRST_READ_MS),
        read_level_current,
    );
    s.level_correction.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::from_millis(signal::LEVEL_CORRECTION_MS),
        correct_level_current,
    );
}

/// The sink every tuner event lands in. Deliberately the smallest possible
/// amount of work on a binder thread: push, then signal.
fn enqueue(event: TunerEvent) {
    queue().push_back(event);
    if let Some(wake) = WAKE.get() {
        wake();
    }
}

/// The same, for the logo worker. Runs on `carnyx-logos`, never the UI thread.
fn enqueue_logo(event: service::Event) {
    logo_queue().push_back(event);
    if let Some(wake) = WAKE.get() {
        wake();
    }
}

// ── The face's own state ─────────────────────────────────────────────────────

/// One preset slot. The dial is the identity; everything else is resolved.
#[derive(Clone, Debug)]
struct Slot {
    mhz: f32,
    /// The FCC row this dial resolved to, if one did.
    row: Option<StationRow>,
    /// The call sign this dial LAST resolved to, kept across launches.
    ///
    /// Resolution needs a position and a head unit does not always have one, so
    /// without this a cold start with no fix showed six bare frequencies and the
    /// logo window had no call sign to search on. See [`crate::prefs::Preset`].
    saved_call: Option<String>,
}

impl Slot {
    /// The best call sign known for this dial: what it resolves to now, else
    /// what it last resolved to, else nothing.
    fn identity(&self) -> Option<&str> {
        match &self.row {
            Some(r) => Some(r.callsign.as_str()),
            None => self.saved_call.as_deref(),
        }
    }

    /// The base the logo search keys on. `callsign_base` is
    /// `callsign.split('-')[0]` for every row in the table, so a stored call sign
    /// is reduced the same way rather than guessed at.
    fn base(&self) -> String {
        match &self.row {
            Some(r) => r.callsign_base.clone(),
            None => self
                .saved_call
                .as_deref()
                .and_then(|c| c.split('-').next())
                .unwrap_or_default()
                .to_string(),
        }
    }

    /// The label the tile's colour box prints, and the key its colour hashes
    /// from. A call sign wins; the dial stands in when there is none at all.
    fn call(&self) -> String {
        match self.identity() {
            Some(c) => plate_label(Some(c), c),
            None => plate_label(None, &format_mhz(self.mhz)),
        }
    }

    fn name(&self) -> String {
        match self.identity() {
            Some(c) => clean_call(c),
            None => format_mhz(self.mhz),
        }
    }
}

struct State {
    /// `None` when the database could not be opened. Everything that reads it
    /// degrades rather than failing: the picker reports no database, and the
    /// hero falls back to the dial.
    db: Option<StationDb>,
    snapshot: Option<String>,
    /// The head unit, or the fake standing in for it.
    ///
    /// `Arc` rather than `Box` because the vendor-getter poll runs on its OWN
    /// THREAD and needs a handle of its own. `Tuner` is already `Send + Sync`;
    /// this is what lets a second owner exist.
    tuner: Arc<dyn Tuner>,
    /// Whether the tuner behind the trait is the real vendor service.
    ///
    /// False in this container. It is NOT a second availability predicate — the
    /// panel asks `tuner.is_available()` for that, once, so the status line and
    /// the source badge can never contradict each other. This exists only to put
    /// the simulation on the record, which it does in the diagnostics log at
    /// start-up, where a fact about the tuner belongs.
    tuner_is_real: bool,
    rds: RdsDecoder,
    /// The last state the decoder PUBLISHED. A group that changes nothing
    /// publishes nothing, and the face must keep showing what it had.
    rds_state: RdsState,
    stream: fake::FakeRdsStream,
    presets: Vec<Slot>,
    /// The dial THIS APP last put on the air deliberately, as opposed to one the
    /// radio reported having moved to on its own. `None` until the app tunes
    /// anything, which is the ordinary state for the first press of a run.
    ///
    /// Set by [`App::tune`], so every deliberate path — the keypad, the nearby
    /// list, a preset tap, the peek cards, the wheel — records itself with no
    /// call site having to remember to. NOT set by the `Frequency` arm, and that
    /// asymmetry is the point: the vendor walking its own hardware preset bank
    /// arrives there and must not be mistaken for the driver going somewhere.
    ///
    /// Cleared by a hardware seek, which IS the driver deliberately leaving the
    /// strip, so stepping afterwards behaves exactly as it did before any of
    /// this: off-strip means the next press lands on entry 0.
    asserted: Option<f32>,
    /// Where `prefs.json` lives — the app's data directory on the device.
    prefs_dir: std::path::PathBuf,
    /// The last thing written, so an unchanged state does not rewrite the file.
    /// `push_settings` runs on far more than settings changes.
    saved: crate::prefs::Prefs,
    dial: f32,
    /// How many frequency reports the tuner has made. The HOLD's release rule
    /// needs "a report arrived AFTER the commit", and a frequency alone cannot
    /// say that — see [`Hold`].
    freq_seq: u64,
    /// THE FACE IS COMMITTED TO A STATION THE TUNER HAS NOT REACHED YET.
    ///
    /// One wheel press makes the vendor service walk its OWN hardware preset
    /// bank, so the dial visits frequencies this app never asked for and every
    /// one of them is reported. `asserted` already stops those reports moving
    /// where the next STEP starts from; this stops them being DISPLAYED, which
    /// is the half the driver sees: "you can see it jump to the wrong station
    /// for a moment before going to the correct one."
    ///
    /// CarFM's, and its own comment states the shape exactly: "from the moment a
    /// preset change is committed until the dial settles on it, the FACE renders
    /// the TARGET and ignores the live tuner. Purely cosmetic and time-boxed"
    /// (`CarFmFace.tsx:756-760`).
    hold: Option<Hold>,
    /// A target the FRONT END still owes, because a report has just said it is
    /// somewhere else while a hold was live.
    ///
    /// THE HOLD ALONE ONLY FIXES THE FACE. It renders the target and rides out
    /// the vendor's transit frequencies, and for two seconds that is the whole
    /// story — but the hold has no opinion about where the RADIO is. One press
    /// makes the vendor's RadioService walk its own hardware bank, and that walk
    /// COMMANDS the tuner; whichever command lands last is the station coming
    /// out of the speakers. When the vendor's lands after ours, nothing put the
    /// driver back, so the face showed the right station for two seconds and
    /// then quietly admitted the radio was elsewhere: "a few times when using
    /// the steering wheel controls, it did end up jumping to a wrong station."
    ///
    /// `NwdBridge.java:511-514` already asserted this was handled — "the app
    /// reasserts its own preset immediately after, which is what makes the app's
    /// order win". It was a description of an intention, not of the code. This
    /// is the code.
    ///
    /// Taken and re-commanded by `drain_events`, never from inside `apply_event`
    /// — that runs with `State` borrowed and `tune` re-enters the same cell.
    reassert: Option<f32>,
    /// The dial the station pop-up last spoke for, in MHz.
    ///
    /// A `Cell` because `push_hero` is the one place that has both the resolved
    /// identity and the landed dial, and it runs with `State` borrowed SHARED.
    ///
    /// KEPT IN STEP WHETHER OR NOT ANYTHING IS POSTED, which is the part that is
    /// easy to get wrong: if it only moved when a notification went out, then
    /// tuning on the face and THEN switching away would leave it stale, and the
    /// next ordinary push — an RDS group, a level read — would announce a
    /// station the driver had chosen by hand a minute earlier. It tracks the
    /// dial; the foreground flag decides whether the change is worth saying.
    ///
    /// `f32::NAN` so the first push can never match it. No dial compares equal
    /// to NaN, including NaN, which is exactly the "nothing has been announced"
    /// this needs and is why it is not `0.0` — a zero would be a real value the
    /// out-of-band path could in principle land on.
    announced: std::cell::Cell<f32>,
    /// Turn-by-turn from OsmAnd, if the driver has switched it on.
    ///
    /// HOLDS ITS OWN CLOCK, which is what the tick below feeds: OsmAnd stops
    /// sending when a route ends and never says it has, so the state has to age
    /// out or the last turn sits on the face for the rest of the drive. See
    /// `crate::nav::Nav::EXPIRY`.
    nav: crate::nav::Nav,
    /// The last navigation line written to the diagnostics log.
    ///
    /// UPDATES ARRIVE ABOUT ONCE A SECOND while a route is running, so logging
    /// each one would evict a three-minute drive from a six-hundred-line ring.
    /// The line is written when it CHANGES, which is the only time it carries
    /// anything a reader did not already have.
    nav_said: String,
    /// Feet and miles, or metres and kilometres (§4.9).
    ///
    /// READ ONCE AT START-UP from the device's locale, because OsmAnd does not
    /// expose its own unit setting over the API. A driver does not cross a
    /// border mid-drive often enough to poll for it, and units changing under a
    /// live countdown would be worse than being one launch behind.
    units: crate::units::Units,
    /// Which OsmAnd `CarnyxNav` found, or empty. Read once at start-up: an app
    /// is not installed and uninstalled while a driver is looking at a switch,
    /// and asking the package manager on every republish would put a binder call
    /// in the path of ordinary radio traffic.
    nav_package: String,
    /// How many re-commands the LIVE hold has left. Re-armed when a step takes a
    /// hold, so a budget one press spent is not inherited by the next.
    reasserts_left: u8,
    /// The band theme the hidden picker is forcing, or `None` for auto-detect.
    ///
    /// NOT PERSISTED, WHICH IS THE REFERENCE'S CHOICE AND ALSO THE SAFE ONE.
    /// CarFM holds it in a `useState` beside `eggTaps` (`SettingsPanel.tsx:88`),
    /// so it dies with the screen. This is a control for LOOKING at the five
    /// themes without waiting for the right track; a forced theme surviving a
    /// restart would be a face wearing someone else's colours with no visible
    /// reason and six taps between the driver and the way back.
    ///
    /// A `String` rather than `&'static Egg` because the picker names an ID —
    /// the same value that crosses to Slint and comes back as an index — and
    /// `eggs::by_id` is where a name becomes a row. Storing the row here would
    /// put a second resolver in the state.
    forced_egg: Option<String>,
    /// The last panel key and when it arrived, for the diagnostics line only.
    ///
    /// INSTRUMENTATION, NOT A GUARD, and the distinction is the point.
    /// `panel_action` refuses a release edge by testing `action` for "up", but
    /// `NwdBridge.java:540` passes the INTENT ACTION there —
    /// "com.nwd.action.ACTION_KEY_VALUE" — so on the unit that test compares
    /// against a string it never receives. Whether that costs anything depends
    /// on a fact this side cannot see: does one physical press produce ONE
    /// broadcast or two? The intent carries a key byte and nothing else, with no
    /// press/release flag, so the answer exists only in the timing and the
    /// timing can only be observed on the unit.
    ///
    /// `examples/wheelprobe.rs` case E shows the cost if the answer is two: a
    /// step of two stations when the edges land in separate drains. That is a
    /// wrong landing, and it is NOT the one [`State::reassert`] fixes.
    ///
    /// So this logs the gap rather than acting on it. Debouncing on a guessed
    /// window would swallow a genuine quick double-press to defend against a
    /// second broadcast that may not exist; one drive with this line settles it.
    last_panel: Option<(i32, std::time::Instant)>,
    /// The last trustworthy level reading, or `None` for no reading at all.
    level: Option<i32>,
    /// The hysteresis' one piece of state, settled once per poll and fed back.
    dotted: i32,
    audio: bool,
    /// The driver pressed the power button OFF, as opposed to the MCU having
    /// taken the source away.
    ///
    /// Two different facts that `audio` alone cannot tell apart, and the getter
    /// poll needs them apart: it heals `audio` from the MCU's own source
    /// register, and an explicit power-off must survive that. CarFM's
    /// `userPoweredOffRef` (RadioScreen.tsx:3010).
    user_powered_off: bool,
    /// The unit told us it is going to sleep and the FM source has not been
    /// handed back yet.
    ///
    /// A FLAG RATHER THAN THE CALL, because the arm that sets it holds a borrow
    /// of this struct and the call crosses into the vendor's binder. DISTINCT
    /// from `user_powered_off`: that is the driver's own choice at the power
    /// button and is meant to survive, this is the ignition going off, which is
    /// nobody's choice and must not come back looking like one.
    sleep_release: bool,
    /// When the FM source was last handed back for a sleep.
    ///
    /// THE POLL NEEDS IT TO TELL TWO OPPOSITE THINGS APART. The self-heal below
    /// prints "the MCU handed FM back" whenever the source register reads 4
    /// again, and those words were written for the Android Auto case, where the
    /// MCU taking FM away and giving it back is good news. Seconds after a sleep
    /// release the identical reading means the OPPOSITE — the release did not
    /// take — and it was being reported as a recovery.
    sleep_released_at: Option<std::time::Instant>,
    /// The post-retune level schedule: read at 1s, correct at 4s, retry a
    /// rejection twice. See [`crate::signal`] for the measurements and
    /// `arm_level_schedule` for the wiring.
    level_first: slint::Timer,
    level_correction: slint::Timer,
    level_retry: slint::Timer,
    /// Rejected reads left before the schedule gives up and waits for the
    /// periodic watch. Re-armed by any good read.
    level_retries: u32,
    /// The pilot on the pill: `Some(true)` STEREO, `Some(false)` MONO, `None` a
    /// blank pill because nothing trustworthy has reported yet.
    ///
    /// ONLY EVER WRITTEN AFTER THE SETTLE WINDOW — see [`STEREO_SETTLE`]. Two
    /// other places used to clear it, a retune and the RDS expiry, and both were
    /// this app's own invention rather than CarFM's: CarFM writes its `fmStereo`
    /// from the vendor callback and from nowhere else, so a reported pilot holds
    /// until the tuner reports a different one. Clearing it here meant the pill
    /// was blanked by every frequency notification the vendor sent, and the
    /// driver saw it "almost never lit up".
    stereo: Option<bool>,
    /// The last pilot the tuner reported, still holding still. Applied to
    /// `stereo` by `stereo_settle` and dropped if a newer one arrives first.
    stereo_pending: Option<bool>,
    /// The settle window's timer, restarted by every report. See
    /// [`STEREO_SETTLE`].
    stereo_settle: slint::Timer,
    location: fake::FakeLocation,
    /// When the last well-formed RDS group arrived, for the expiry below.
    ///
    /// `Instant`, not a wall clock: this measures a gap, and a wall clock can be
    /// stepped by the head unit picking up time from GPS mid-drive.
    last_rds_at: Option<std::time::Instant>,
    /// The carrier has gone quiet. Set by the expiry, cleared by the next group.
    rds_stale: bool,
    /// The station on the current dial, resolved once per (dial, position).
    ///
    /// `resolve` is a SQLite query and `push_hero` runs on EVERY wake from the
    /// tuner queue — a group every 90ms on a station with RDS — so this was
    /// eleven index walks a second to answer a question whose two inputs change
    /// on a retune and on a GPS fix. Measured at 1.70ms of `push_all`'s 1.94ms
    /// (`examples/pushbench.rs`).
    resolved: Option<(i32, fake::FakeLocation, Option<StationRow>)>,
    /// The last view handed to each surface, so a republish that would change
    /// nothing can stop before it replaces a model.
    ///
    /// REPLACING A `ModelRc` IS NOT FREE: Slint tears the repeater's items down
    /// and builds them again. With the nearby list open that is a hundred rows
    /// rebuilt and re-rendered per wake, and the measurement is brutal — a wake
    /// plus a frame costs 7ms with the face alone and 56ms with that list on
    /// screen, against groups arriving every 90ms.
    last_nearby: Option<crate::stations::NearbyView>,
    last_presets: Option<Vec<Preset>>,
    last_diag: Option<Vec<String>>,
    /// The settings panel's own two models, under the same rule. They were the
    /// only lists in `push_settings` without one — the diagnostics lines beside
    /// them had been guarded and these had not — so every wake from the tuner
    /// queue rebuilt the source list and the diagnostics buttons whether or not a
    /// character of them differed, roughly eleven times a second on a station
    /// with RDS.
    last_sources: Option<Vec<TunerSource>>,
    last_diag_actions: Option<Vec<DiagAction>>,
    /// The two models in `push_settings` that CANNOT change: the theme chips are
    /// a fixed list and the tuner-details list is always empty (the panel
    /// describes an RTL-SDR, which the provenance rule bars from this tree). One
    /// publish is the whole truth about both, so they are published once and then
    /// never touched again rather than re-derived per wake.
    statics_published: bool,
    /// FRAMES THE DRIVER ACTUALLY GOT during the morph now running, and when it
    /// started. See `ui/hero.slint`'s `morph-frame`.
    ///
    /// The whole performance review was measured on a desktop running the
    /// SOFTWARE renderer; the APK renders with Skia on a 32-bit ARM GPU. So the
    /// number that decides whether the card animation reads as motion or as a cut
    /// — how many frames fit in 520ms — has never been observed where it matters,
    /// and cannot be from here. This counts them on the unit and puts the answer
    /// in the log, which now saves to Downloads.
    morph_frames: u32,
    morph_since: Option<std::time::Instant>,
    /// Fires once after a morph should have finished, to write the tally. A timer
    /// rather than a check on the next step, so one press on its own still
    /// reports.
    morph_report: slint::Timer,
    /// THE PROBE A TAP HAS ASKED FOR BUT NOT YET RUN, and the timer that runs it.
    ///
    /// A probe is hundreds of milliseconds of binder work on the UI THREAD — the
    /// stock-radio one walks every installed package, asks sixteen intent
    /// queries and hashes two signing certificates. Run inline from the tap it
    /// froze the face for that whole time with nothing to show for it, which
    /// from the driver's side is a row that does nothing. Deferring by one frame
    /// lets the well repaint with "reading…" first, so the tap is visibly
    /// answered before the freeze rather than after it.
    ///
    /// A second tap during the first simply replaces this; the first tap's line
    /// stays in the log, which is the honest record of what was asked.
    /// The line the settings panel shows above the log well. See
    /// `SettingsOverlay::diag-status` for why it is not in the well.
    diag_status: String,
    pending_probe: Option<settings::Action>,
    probe_run: slint::Timer,
    /// The dial the RDS on screen was RESTORED from, when it came off disk
    /// rather than off the air. See [`crate::session`].
    ///
    /// It exists because the restore is optimistic: it is applied before the
    /// tuner has said a word, so the face is warm on the first frame. This is
    /// what lets the first `Connected`/`Frequency` event confirm it — or throw
    /// it away, if the radio turns out to be somewhere else. Cleared for good by
    /// the first real group, and by the expiry.
    warm_dial: Option<f32>,
    /// How many times this app has started since it was installed, counting the
    /// run that is happening now. Read from the last run's snapshot; written
    /// back into the next one.
    launches: u32,
    /// A retune or a panel action is in flight, so a drain re-entered from
    /// inside one must not start another. See `drain_events`.
    ///
    /// NAMED FOR THE RETUNE, NOT THE PANEL KEY, and the first cut got that
    /// wrong: it was set only around `apply_panel_action` inside `drain_events`,
    /// which left every UI-initiated `tune` — a preset tap, the keypad, the
    /// nearby list — draining with the flag CLEAR. That drain would take a
    /// queued wheel key, run a whole second step, and then the outer `tune`
    /// would write its own older `dial` and `asserted` on top: the front end
    /// left on one station with the face and the anchor naming another.
    busy: bool,
    /// What a panel key asked for, applied after the drain.
    ///
    /// DEFERRED, because `apply_event` holds a mutable borrow of this struct and
    /// every one of these actions retunes — which borrows it again and panics.
    /// The queue is drained to nothing first, then the last request is honoured:
    /// a driver leaning on the wheel button wants to end up one station along
    /// from where they started, not to replay every intermediate step.
    panel_action: Option<PanelAction>,
    /// Set by a Position event, cleared by the drain. The picker rebuild and the
    /// hero republish are done ONCE per drain rather than once per fix: a moving
    /// car produces a fix a second, and each one would otherwise re-run a
    /// 20,733-row query and repaint the hero.
    location_dirty: bool,
    picker: NearbyPicker,
    settings: settings::Settings,
    logo: crate::logos::search::Model,
    /// What was on each frequency the last time a fix let the database answer.
    ///
    /// The only thing that can name a station on a unit that cannot see the sky
    /// — see [`crate::callsigns`].
    callsigns: crate::callsigns::Callsigns,
    /// Where every station's art lives, under the app's own data directory.
    /// Always present — a store with nothing in it is the ordinary first-run
    /// state and answers every read with `None`.
    store: Arc<LogoStore>,
    /// The decoder, for turning STORED renditions into pixels on the UI thread.
    /// `None` on the host, where there is no `BitmapFactory`.
    codec: Option<Arc<dyn ImageCodec>>,
    /// The thread that owns every socket and every pixel pass. `None` on the
    /// host, which is what makes `run_logo_search` fall back to the fake.
    worker: Option<service::Worker>,
    /// Decoded art, keyed by call-sign base, the ladder box it was read at, and
    /// WHICH THEME it was read for.
    ///
    /// NOT an optimisation. `push_presets` runs on every tune, every fix and
    /// every drain, and it renders up to eight tiles; without this, each of
    /// those would be a file read and a PNG decode on the UI thread of a 32-bit
    /// head unit. A `None` value is cached too — "this station has no art" is
    /// the common answer and is worth not re-deriving.
    ///
    /// THE THEME IS PART OF THE KEY, not a reason to clear the map. Light and
    /// dark are different FILES for the same station — `d-128.png` against
    /// `k-128.png` — so a driver switching theme at dusk would otherwise pay for
    /// eight decodes, and switching back would pay again. Keyed, both sets stay
    /// warm and a switch costs the republish alone. It is also what makes the
    /// backing safe to cache: a `LogoPlate` belongs to one theme's answer.
    art: HashMap<(String, u32, bool), Option<(slint::Image, LogoPlate)>>,
    /// Stations already queued for a dark adaptation this run. See
    /// `request_dark_adaptation` — without it a master that will not decode
    /// re-queues on every republish.
    adapt_tried: HashSet<String>,
    /// The dark-logo picker, from the moment a logo is assigned until the driver
    /// answers it. `None` at every other time, which is what keeps the logo
    /// window in its ordinary states.
    dark_pick: Option<DarkPick>,
    /// The open logo window's own art, at full size.
    ///
    /// A `Raster` rather than an `Image` because `logos::ui::apply` wants one,
    /// and held rather than re-read because `push_logo_search` runs on every
    /// keystroke of state the window has — one decode per window, not per push.
    logo_art: Option<Raster>,
    /// The position `picker` was built from, so a fix that has not moved can be
    /// dropped without re-deriving anything. `None` until the first build.
    picker_at: Option<(f64, f64)>,
    /// The frequency tab's entry buffer, exactly as typed.
    numpad: String,
    /// A hardware sweep is in flight: WE handed a seek to the tuner and no
    /// frequency report has come back since.
    ///
    /// OWNED HERE, NOT READ FROM THE VENDOR. `notifyRadioScanState(int)` exists
    /// on the callback interface, but its integer values are undocumented
    /// (NWD-RADIO-INTEGRATION §2) and nothing has decoded them on the unit, so
    /// wiring the flag to a guess would be exactly the fabricated-diagnostics
    /// failure of task 49. Set when a seek is handed off, cleared by the next
    /// `Frequency` event — a landing and a retune both end a sweep — which is
    /// honest on every tuner this app has: the fakes land synchronously and the
    /// NWD front end answers a seek with one `notifyCurrentFrequency`.
    ///
    /// Until this existed the UI's `scanning` property was never set at all:
    /// the hero's scanning face, the readout's un-dim-while-sweeping rule and
    /// the stop-sweep-on-tab-switch branch were all unreachable in production.
    scanning: bool,
    /// What the dial read when the frequency tab was opened, and what CANCEL puts
    /// back (mini-handoff §5).
    ///
    /// Needed because SEEK runs with the overlay open and leaves the overlay open
    /// when it lands, so a driver can walk the dial several stations away and then
    /// change their mind — and "restores the frequency the overlay opened on" has
    /// to mean something at that point. `None` while the nearby tab is up.
    freq_restore: Option<f32>,
}

impl State {
    /// Which preset the dial is currently sitting on, or -1.
    ///
    /// DERIVED, NEVER STORED. It was stored once, and the tuner reporting its
    /// own frequency on connect then left a face showing the unsaved star over a
    /// dial that was plainly in the strip. There is one source of truth for
    /// where the radio is — `dial` — and this is read off it.
    ///
    /// Matched through `preset_key`, not by comparing floats: 105.5 does not
    /// round-trip through an f32 and `==` on two of them is a coin toss.
    fn active(&self) -> i32 {
        // OFF THE SHOWN DIAL, so the star and the lit tile agree with the
        // frequency beside them. During a hold the face is committed to the
        // target, and a star that flickered against the vendor's transit
        // frequencies would contradict the very number it sits next to. CarFM:
        // "activeIndex already reflects the committed target (it derives from
        // the effective dial)" (`CarFmFace.tsx:1087-1089`).
        active_index(self.shown(), &self.presets)
    }

    /// Where the STRIP is, which is not always where the radio is.
    ///
    /// `active` answers "is the thing playing one of the driver's presets" and
    /// must read `dial`, because that is what the star and the lit tile are
    /// about. THIS answers a different question — "which entry does the next
    /// step move on from" — and reading `dial` for it is the defect this
    /// exists to close.
    ///
    /// THE VENDOR MOVES `dial` WITHOUT BEING ASKED. Its service steps its own
    /// hardware preset bank on the same wheel press the app is reacting to, and
    /// the frequency it lands on arrives as an ordinary `Frequency` event that
    /// sets `dial` like any other. A step computed from that walks the strip
    /// from wherever the VENDOR'S bank happens to be: off-strip it resolves to
    /// -1 and every press lands on entry 0, which is the reported "going next
    /// goes backwards"; on-strip it resolves to some unrelated index, which is
    /// the reported "skips presets".
    ///
    /// A FREQUENCY AND NOT AN INDEX, deliberately. `reorder_preset` argues
    /// against storing an index — "`active()` re-derives from the same dial and
    /// simply lands on a different index, which is why it was never stored" —
    /// and that argument is right and applies here too. Storing the dial the app
    /// last asserted keeps the property: a drag re-resolves it to the entry's
    /// new position, and deleting that entry drops cleanly to -1.
    /// What the FACE should show: the committed target while a change is in
    /// flight, the real dial otherwise.
    ///
    /// CarFM's `const mhz = pending ? pending.mhz : rawMhz` (`CarFmFace.tsx:824`)
    /// — "everything below derives from the EFFECTIVE dial". Every user-visible
    /// reader goes through this; the tuner-facing ones keep reading `dial`,
    /// because a hold is a rendering decision and never a claim about where the
    /// radio is.
    fn shown(&self) -> f32 {
        self.hold.map_or(self.dial, |h| h.mhz)
    }

    /// Drop a hold the tuner never honoured, so a refused tune cannot freeze the
    /// face. Cheap enough to call on every publish.
    fn expire_hold(&mut self) {
        if self.hold.is_some_and(|h| h.at.elapsed() >= HOLD_CAP) {
            self.hold = None;
            // AND THE RE-COMMAND GOES WITH IT. Past the cap this app has stopped
            // claiming the front end is on the target, so it must stop trying to
            // put it there too — a re-assert fired after the face has told the
            // truth would move the radio out from under a station the driver can
            // now see is playing.
            self.reassert = None;
        }
    }

    fn anchor(&self) -> i32 {
        step_anchor(self.asserted, self.dial, &self.presets)
    }
}

/// A station the face is showing before the tuner has got there.
///
/// See [`State::hold`]. Three fields and every one of them is load-bearing.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Hold {
    /// What to render instead of the live dial.
    mhz: f32,
    /// The report counter AS OF THE COMMIT. Only a LATER report can release the
    /// hold, and that is not a detail — it is the bug CarFM shipped and then
    /// fixed. `tune` writes `s.dial` synchronously, so a release rule that
    /// compares the target against `dial` is true the instant the hold is taken
    /// and releases it in the same breath. CarFM's note: "the old comparison
    /// released the hold in the same commit that opened it, always. The 30 July
    /// drive log shows it exactly: six steps, six instant 'settled' lines, not
    /// one 'holding' line, and the vendor's transit frequencies painting the
    /// hero for ~1 s after each" (`CarFmFace.tsx:770-775`).
    since_seq: u64,
    /// When it was taken, for the cap. A tune the front end ignores must not
    /// freeze the face forever.
    at: std::time::Instant,
}

/// How long the face will render a station the tuner has not confirmed.
///
/// CarFM's two seconds (`CarFmFace.tsx:1809`), and timed from the COMMIT rather
/// than from the last report — keying it on the dial would restart the clock on
/// every transit frequency the vendor emits, which is the churn the cap exists
/// to bound.
const HOLD_CAP: std::time::Duration = std::time::Duration::from_millis(2000);

/// How many times one step will re-command its target when the front end is
/// reported somewhere else. See [`State::reassert`].
///
/// SMALL ON PURPOSE, and the reason is that this is a race with another process
/// rather than a retry against a flaky call. Two commands cover the case the
/// hardware actually produces — the vendor's bank walk is one retune per press,
/// so one re-assert after it is enough and the second is slack for a report that
/// arrives out of order. An unbounded version would be an app and a vendor
/// service taking turns retuning the radio for as long as the driver held still,
/// which is worse than either landing.
///
/// When the budget is spent the hold simply expires and the face shows where the
/// radio really is. Losing honestly beats fighting forever.
const REASSERT_TRIES: u8 = 2;

/// What a press on the head unit's own panel means to this app.
///
/// The vendor MCU broadcasts `com.nwd.action.ACTION_KEY_VALUE` and the press
/// never enters Android's input pipeline, so nothing reaches the window: these
/// arrive as tuner events and have to be turned into commands here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelAction {
    /// Step OUR preset strip. CarFM runs the same animated step the on-screen
    /// arrows do rather than a silent jump, so the wheel and the face behave
    /// identically — and the hardware bank the service just stepped is not the
    /// strip the driver can see.
    ///
    /// `from` IS THE STRIP POSITION AS IT WAS WHEN THE KEY ARRIVED, and carrying
    /// it is the fix for the wheel stepping from the wrong place. See
    /// [`App::step_preset_from`].
    Step { dir: i32, from: i32 },
    /// Hand the seek to the tuner and take whatever dial it settles on.
    Seek(bool),
}

/// The whole application. One owner of one `AppWindow`, on one thread.
pub struct App {
    ui: slint::Weak<AppWindow>,
    state: RefCell<State>,
}

/// Retire the poll thread with the App that started it.
///
/// The thread holds an `Arc` to the tuner, so nothing stops it on its own, and it
/// emits into a queue that is global to the process. On the head unit a leftover
/// poll would outlive a destroyed Activity; off it, the probes build one App
/// after another in one process and a leftover poll would deliver a phantom
/// reading into the next one. `Disconnected` stops it too — this is the case
/// where nothing said goodbye.
impl Drop for App {
    fn drop(&mut self) {
        crate::android::stop_state_poll();
    }
}

/// The FM band, and the only band this app tunes.
const FM_LO: f32 = 87.5;
const FM_HI: f32 = 108.0;

/// How often to re-read the signal level while connected.
///
/// The Java bridge floors this at 5s (`NwdBridge.LEVEL_MIN_INTERVAL_MS`) because
/// every tick commands the tuner, and the vendor rate-limits its own comparable
/// read to 900ms. Asking for less than the floor just gets clamped, so the floor
/// is what is asked for.
/// How often to re-read the signal level while parked on a station.
///
/// CarFM's `LEVEL_POLL_MS`, referenced rather than restated. This used to be
/// 5_000, justified by the Java side's own floor (`NwdBridge.LEVEL_MIN_INTERVAL_MS`)
/// — but that floor is a MINIMUM the bridge clamps to, not a cadence anyone
/// chose, and reading four times as often as CarFM commands the tuner four times
/// as often for no gain. The floor still applies underneath; it simply is not
/// reached.
const LEVEL_WATCH_MS: i64 = signal::LEVEL_POLL_MS as i64;

/// How often to read the vendor's own getters.
///
/// CarFM's poll interval (`RadioScreen.tsx:3100`, `}, 1500)`). It exists because
/// the push callbacks are not reliable on this unit — `NwdBridge`'s own comment
/// on `pollNumbers` says so: "the push notify* callbacks do not always reach a
/// passive client, but these synchronous getters do return live values".
///
/// OFF THE UI THREAD, on a thread of its own — see
/// [`crate::android::start_state_poll`]. The three getters are binder calls into
/// the vendor service, and CarFM makes them from React Native's native-modules
/// thread, never from the UI thread. The first version of this in Carnyx used a
/// `slint::Timer`, which IS the UI thread, and a vendor service that blocked
/// would have hitched the face every 1.5 seconds.
const POLL_MS: std::time::Duration = std::time::Duration::from_millis(1500);

/// How long a silence has to last before the RDS on screen is disowned.
///
/// CarFM's `RDS_STALE_MS` (RadioScreen.tsx:453). Long enough that a tunnel, an
/// overpass or a burst of multipath does not blank a good station; short enough
/// that a driver is not reading a song title from a transmitter they left behind
/// twenty miles ago.
///
/// Public because `session::WARM` IS this number: a snapshot older than the
/// window the running app would have disowned must not come back from disk, and
/// the way to guarantee that is one constant rather than two that agree today.
pub const RDS_STALE: std::time::Duration = std::time::Duration::from_secs(25);

/// How long a reported pilot has to hold still before it reaches the STEREO pill.
///
/// CarFM's `setStereoDebounced` (RadioScreen.tsx:2700-2703), the same 2000ms, and
/// it is a TRAILING window: every report restarts it, so a callback that keeps
/// flapping never lands at all and the pill holds whatever it last settled on.
///
/// The flapping is measured, not feared. CarFM counts `stereoFlips` on the raw
/// event precisely because multipath collapses the pilot without touching the
/// signal level — "a station can read 55 and still flap. WERN did exactly that
/// all through one commute" (RadioScreen.tsx:494-497). Applied straight through,
/// that is a pill strobing between STEREO and MONO for a whole drive.
///
/// The two getters that would answer instantly are both useless here and were
/// checked on the device: `isStreroOn()` and `getStationStereoState()` each read
/// true on dead air (NwdRadioModule.kt:706-710). The push callback is the only
/// honest source, which is why this waits for it rather than polling.
const STEREO_SETTLE: std::time::Duration = std::time::Duration::from_millis(2000);

impl App {
    /// Build the application against a station database at `db_path`.
    ///
    /// A missing or unreadable database is a NORMAL state, not a failure: the
    /// face still draws, the hero falls back to the dial, and the picker says
    /// there is no database. Refusing to start because a reference table is
    /// missing would be the wrong trade in a car.
    pub fn new(ui: &AppWindow, db_path: &Path) -> Rc<App> {
        // The fake, and it is announced as one: `tuner_is_real` is what the
        // settings panel reads, so nothing on screen claims a tuner that is not
        // there. This is the host and screenshot path; `android_main` calls
        // `with_tuner` instead and hands over the vendor service.
        App::with_tuner(
            ui,
            db_path,
            &host_prefs_dir(),
            Box::new(FakeTuner::new()),
            false,
            None,
            // MADISON, and only here. This is the position the station-database
            // tests query from, so the picker's rows on a screenshot are the
            // rows `the_madison_query_returns_what_carfm_returned` pins. On the
            // device the seed is `no_fix` — see `android_main`.
            fake::FakeLocation::default(),
        )
    }

    /// The same face, driven by whichever tuner the caller could actually get.
    ///
    /// Split out from [`App::new`] because the real one cannot be built here:
    /// `android::init` needs a `JavaVM` and a live activity, so only
    /// `android_main` can supply it. Everything downstream of this line is
    /// identical either way, which is the point of the [`Tuner`] trait — the
    /// face has no idea which one it has.
    ///
    /// `tuner_is_real` is not derived from the tuner, deliberately. The settings
    /// panel states, on screen, whether the head unit's radio is present, and
    /// that claim should come from the caller that knows how the tuner was
    /// obtained rather than from a value the tuner reports about itself.
    ///
    /// `location` is the STARTING position, and it is a parameter for the same
    /// reason. It used to be `FakeLocation::default()` — Madison, with `fix`
    /// TRUE — built right here, which meant the device booted claiming a
    /// satellite lock it did not have and a nearby list for a city 800 miles
    /// away. The device passes `no_fix`; the host passes Madison, because that
    /// is the position the screenshots and the database tests are pinned to.
    pub fn with_tuner(
        ui: &AppWindow,
        db_path: &Path,
        prefs_dir: &Path,
        tuner: Box<dyn Tuner>,
        tuner_is_real: bool,
        net: Option<Net>,
        location: fake::FakeLocation,
    ) -> Rc<App> {
        let db = StationDb::open(db_path).ok();
        let snapshot = db.as_ref().and_then(|d| d.snapshot_date().ok()).flatten();

        // WHAT THE DRIVER CHOSE LAST TIME.
        //
        // Passed in rather than derived from `db_path`, and that is not
        // fastidiousness: deriving it put `prefs.json` next to the shipped
        // database in `assets/`, and `assets` is what cargo-apk packages, so a
        // host run would have posted the developer's own preferences into the
        // APK. On the device this is the app's private data directory; on the
        // host it is under `target/`.
        let prefs_dir = prefs_dir.to_path_buf();
        let saved = crate::prefs::load(&prefs_dir);

        // The seed list is a FIRST-RUN default, not a fallback: once the driver
        // has presets, an empty strip is a real state they chose and must not be
        // silently repopulated with six stations from Madison.
        // Loaded before the strip is built, because it is what names a preset
        // when there is no fix to resolve one.
        let callsigns = crate::callsigns::load(&prefs_dir);

        let stored: Vec<crate::prefs::Preset> = if crate::prefs::path(&prefs_dir).exists() {
            saved.presets.clone()
        } else {
            fake::seed_presets()
                .into_iter()
                .map(|mhz| crate::prefs::Preset { mhz, call: None })
                .collect()
        };
        // The row is resolved fresh, so a database update improves old presets
        // rather than leaving them pinned to whatever they resolved to when they
        // were saved. It resolves to NOTHING without a position, which is why the
        // stored call sign comes across as the fallback — see `Slot::saved_call`.
        let presets: Vec<Slot> = stored
            .into_iter()
            .map(|e| Slot {
                row: resolve(db.as_ref(), e.mhz, location.position()),
                mhz: e.mhz,
                // The preset's own stored call sign first, then whatever the
                // learned map knows about that dial. A preset saved before this
                // file existed has no call sign of its own, and the map is what
                // gives it one back.
                saved_call: e
                    .call
                    .or_else(|| callsigns.get(e.mhz).map(str::to_string)),
            })
            .collect();

        let picker = build_picker(db.as_ref(), location, snapshot.clone());

        // WHAT THE LAST RUN LEFT ON THE DIAL.
        //
        // Read here and applied below rather than after the tuner answers,
        // because the whole point is the FIRST frame: the driver's complaint is
        // that coming back from another app looks like a cold start, and a hero
        // that fills in a second later is still a cold start with a delay on it.
        // `session::warm_rds` refuses anything the live expiry would already
        // have wiped, so an optimistic restore can be wrong about which station
        // is tuned but can never be wrong about how old the text is.
        let previous = crate::session::load(&prefs_dir);
        let launches = previous.as_ref().map(|p| p.launches).unwrap_or(0).saturating_add(1);
        let warm = previous
            .as_ref()
            .and_then(|p| p.warm_rds(crate::session::now_unix()).map(|(rds, age)| (p.dial, rds, age)));

        // The store sits BESIDE `prefs.json`, under the same private directory,
        // for the same reason it is passed in rather than derived: `assets/` is
        // what cargo-apk packages, and a store rooted there would have shipped
        // the developer's own logos inside the APK.
        let store = Arc::new(LogoStore::new(logo_dir(&prefs_dir)));
        let codec = net.as_ref().map(|n| n.codec.clone());
        // `enqueue_logo` rather than anything Slint-shaped: the worker must not
        // know a window exists, and the wake hop is already installed for the
        // tuner. On the host WAKE is empty and the events sit in the queue until
        // the caller drains — which is exactly what the screenshot path wants.
        let worker = net.map(|n| {
            service::Worker::spawn(
                store.clone(),
                n.http,
                n.codec,
                Box::new(enqueue_logo) as service::Sink,
            )
        });

        let app = Rc::new(App {
            ui: ui.as_weak(),
            state: RefCell::new(State {
                db,
                snapshot,
                // The poll thread gets a clone of this on connect.
                tuner: Arc::from(tuner),
                tuner_is_real,
                rds: RdsDecoder::new(),
                announced: std::cell::Cell::new(f32::NAN),
                nav: crate::nav::Nav::new(),
                units: crate::units::Units::default(),
                nav_said: String::new(),
                nav_package: String::new(),
                // The DECODER is not seeded, only the published state. A decoder
                // restored from disk would have consensus tallies it never
                // earned, and the first corrupt group off the air would then
                // publish against them. What comes back is a picture of a
                // station, not a claim to have received it.
                rds_state: warm.as_ref().map(|(_, rds, _)| rds.clone()).unwrap_or_default(),
                stream: fake::FakeRdsStream::new(),
                presets,
                // NOT seeded from the restored dial. A warm restore is the app
                // saying where the radio WAS, not the app putting it there, and
                // the first `Frequency` event may well contradict it.
                asserted: None,
                // The last dial beats the seed, which is a Madison frequency
                // from the host fake. The tuner overwrites it the moment it
                // connects; until then this is the better guess by far.
                dial: warm.as_ref().map(|&(dial, _, _)| dial).unwrap_or(fake::SEED_DIAL_MHZ),
                level: None,
                dotted: 0,
                audio: true,
                user_powered_off: false,
                sleep_release: false,
                sleep_released_at: None,
                level_first: slint::Timer::default(),
                level_correction: slint::Timer::default(),
                level_retry: slint::Timer::default(),
                level_retries: signal::LEVEL_RETRY_MAX,
                stereo: None,
                stereo_pending: None,
                stereo_settle: slint::Timer::default(),
                location,
                location_dirty: false,
                picker,
                settings: settings::Settings {
                    selected: saved.selected,
                    theme: saved.theme,
                    logos_on: saved.logos_on,
                    release_on_sleep: saved.release_on_sleep,
                    diag_on: saved.diag_on,
                    ..settings::Settings::default()
                },
                logo: crate::logos::search::Model::new(),
                // THE RESTORED TEXT KEEPS THE OLD STATION'S CLOCK. Stamping it
                // `now` would hand a snapshot taken twenty-four seconds ago a
                // fresh twenty-five second lease, so a station that had already
                // gone quiet would sit on the face for another half minute. The
                // fallback is only reached when the device has been up for less
                // time than the snapshot's age, which means a reboot happened in
                // between and the lease is the least of it.
                last_rds_at: warm.as_ref().and_then(|&(_, _, age)| {
                    std::time::Instant::now()
                        .checked_sub(age)
                        .or_else(|| Some(std::time::Instant::now()))
                }),
                rds_stale: false,
                resolved: None,
                last_nearby: None,
                last_presets: None,
                last_diag: None,
                last_sources: None,
                last_diag_actions: None,
                statics_published: false,
                morph_frames: 0,
                morph_since: None,
                morph_report: slint::Timer::default(),
                diag_status: String::new(),
                pending_probe: None,
                probe_run: slint::Timer::default(),
                warm_dial: warm.as_ref().map(|&(dial, _, _)| dial),
                launches,
                freq_seq: 0,
                hold: None,
                reassert: None,
                reasserts_left: 0,
                forced_egg: None,
                last_panel: None,
                busy: false,
                panel_action: None,
                callsigns,
                store,
                codec,
                worker,
                art: HashMap::new(),
                adapt_tried: HashSet::new(),
                dark_pick: None,
                logo_art: None,
                picker_at: None,
                numpad: String::new(),
                scanning: false,
                freq_restore: None,
                prefs_dir,
                saved,
            }),
        });

        CURRENT.with(|c| *c.borrow_mut() = Some(Rc::downgrade(&app)));
        // THE LAUNCH RECORD, first line of every run's log.
        //
        // This is the answer to "why did it start fresh", written where the
        // driver can read it without a cable. It says how the last run ended,
        // how long ago, and — the part that matters — whether this process has
        // built an App before. See `APPS_BUILT_HERE`.
        {
            let here = APPS_BUILT_HERE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let mut s = app.state.borrow_mut();
            let last = match &previous {
                None => "no previous run on record".to_string(),
                Some(p) => {
                    let ago = match p.age(crate::session::now_unix()) {
                        Some(d) => format!("{}s ago", d.as_secs()),
                        None => "at an unusable time".to_string(),
                    };
                    format!("last run ended in {} {}", p.parting.name(), ago)
                }
            };
            let restored = match &warm {
                Some((dial, rds, _)) if !rds.ps.is_empty() => {
                    format!("RDS restored on {dial:.1} ({})", rds.ps)
                }
                Some((dial, _, _)) => format!("RDS restored on {dial:.1}"),
                None => "cold RDS".to_string(),
            };
            let at = stamp();
            // INTO THE HEAD: this is the line that says which run the rest of the
            // log belongs to, and in a plain ring it is the first one evicted.
            s.settings.log.push_head(
                &at,
                &format!("session: launch #{launches}, app #{here} in this process, {last}, {restored}"),
            );
        }
        crate::android::set_event_sink(enqueue);
        // DAY AND NIGHT BELONG TO THE VEHICLE, not to whichever tuner is
        // selected, so this is started here and not inside `connect_tuner`.
        // CarFM registered the receiver inside connect alone, and a session that
        // never bound the built-in tuner stayed light all night.
        app.state.borrow().tuner.start_illumination_watch();
        // AND THE IGNITION, on exactly the same argument. This was registered
        // inside `NwdBridge.connect()` instead, after `bindService` returned
        // true, so a unit whose vendor service refused the bind never heard the
        // unit go to sleep — and the `sleep:` line is the only evidence of which
        // broadcast fires at all. Same bug as the headlights, one line away from
        // the comment describing it.
        // AND WHAT IT MANAGED, into the log. The receiver either registered or it
        // did not, and until now it said so only to logcat — so on a unit with
        // no adb, "the watch never registered" and "the broadcast never arrived"
        // looked the same and need different fixes. Empty on the host, so no
        // screenshot moves.
        // THE SWITCH THE RECEIVER READS. It runs on a binder thread and cannot
        // reach `Settings`, so the restored value is pushed down before the
        // watch is armed — otherwise the first ignition cycle after a launch
        // would use Java's default rather than the driver's choice.
        {
            let s = app.state.borrow();
            s.tuner.set_release_on_sleep(s.settings.release_on_sleep);
        }
        // ── OSMAND, IF THE DRIVER HAS ASKED FOR IT ───────────────────────
        //
        // The package is read either way, because the settings row says which
        // OsmAnd it found whether or not the switch is on — a driver deciding
        // whether to turn it on is exactly who needs that sentence.
        //
        // THE BIND IS GATED ON THE SWITCH, and that is not a saving. It uses
        // `BIND_AUTO_CREATE`, so binding STARTS OsmAnd; a radio that launched a
        // maps app at boot because a preference file said so would be doing
        // something nobody asked for on a screen nobody is looking at.
        {
            let found = app.nav_installed_package();
            // AND WHICH UNITS THE ROADS AROUND THIS DRIVER ARE SIGNED IN
            // (§4.9). Same one-shot read, and for a stronger reason: this one
            // must not change while a countdown is running.
            let units = crate::units::Units::for_country(&crate::android::country_code());
            let mut s = app.state.borrow_mut();
            s.nav_package = found;
            s.units = units;
        }
        if app.state.borrow().settings.nav_on {
            let outcome = app.set_nav_running(true);
            if !outcome.is_empty() {
                let at = stamp();
                app.state.borrow_mut().settings.log.push(&at, &format!("nav: {outcome}"));
            }
        }
        let sleep_watch = app.state.borrow().tuner.start_sleep_watch();
        if !sleep_watch.is_empty() {
            let at = stamp();
            // INTO THE HEAD, with the rest of the launch block: whether the watch
            // registered at all is half of what a sleep report needs, and it is
            // written in the first second of a run.
            app.state.borrow_mut().settings.log.push_head(&at, &format!("sleep watch: {sleep_watch}"));
        }
        // The restored theme has to reach the palette; `Settings` alone only
        // records the choice.
        let theme = app.state.borrow().settings.theme;
        app.apply_theme(theme);
        app.connect_tuner();
        app.install_callbacks(ui);
        app.push_all();
        app
    }

    fn ui(&self) -> AppWindow {
        self.ui.unwrap()
    }

    /// Ask the tuner to bind, then apply whatever it said.
    ///
    /// The fake answers synchronously, so the drain immediately after is enough.
    /// A real bind answers on a binder thread minutes later or never, which is
    /// why the outcome is an event rather than a return value.
    fn connect_tuner(self: &Rc<App>) {
        {
            // On the record, once, in the place a fact about the tuner belongs.
            let mut s = self.state.borrow_mut();
            if !s.tuner_is_real {
                let at = stamp();
                s.settings
                    .log
                    .push(&at, "tuner: SIMULATED — no NWD service in this build");
            }
        }
        let outcome = self.state.borrow().tuner.connect();
        match outcome {
            Ok(()) => {
                self.drain_events();
                // Settle the decoder against the replayed corpus so the face
                // opens with a station rather than with eight seconds of blank
                // hero.
                self.pump_rds_until_settled();
                self.read_level();
            }
            // A SYNCHRONOUS refusal — bindService itself said no, rather than
            // the bind being accepted and failing later, which arrives as a
            // ConnectFailed event instead. This branch was previously an absent
            // `else`, so a unit that refused the bind looked identical to one
            // that was still connecting: nothing on screen, nothing in the log.
            //
            // Nothing swaps the tuner here. `is_available` is the one predicate
            // the status line reads and it answers for itself, so recording the
            // refusal is this function's whole job.
            Err(e) => {
                let at = stamp();
                let mut s = self.state.borrow_mut();
                s.settings.log.push(&at, &format!("tuner: connect refused — {e}"));
            }
        }
        self.push_settings();
    }

    /// Apply everything the tuner has said since the last drain.
    ///
    /// MUST RUN ON THE UI THREAD.
    ///
    /// Takes the `Rc` rather than a bare reference because a panel key applied
    /// at the end of the batch retunes, and every command path needs the handle
    /// its callbacks are registered against.
    pub fn drain_events(self: &Rc<App>) {
        loop {
            let Some(event) = queue().pop_front() else { break };
            self.apply_event(event);
        }
        // The logo worker's queue, on the same hop. Drained AFTER the tuner's so
        // a `Saved` event's invalidation lands on top of whatever a retune in
        // the same batch already rebuilt.
        loop {
            let Some(event) = logo_queue().pop_front() else { break };
            self.apply_logo_event(event);
        }
        // THE SLEEP RELEASE, before the panel key and deferred for the same
        // reason: no borrow may be held across the vendor call. Taken in its own
        // statement so the `RefMut` is dropped before the call runs.
        let releasing = std::mem::replace(&mut self.state.borrow_mut().sleep_release, false);
        if releasing {
            let (tuner, at) = {
                let s = self.state.borrow();
                (s.tuner.clone(), stamp())
            };
            tuner.set_audio_enabled(false);
            let mut s = self.state.borrow_mut();
            s.audio = false;
            s.sleep_released_at = Some(std::time::Instant::now());
            // A RE-SEND, AND THE LINE SAYS SO. The release that matters ran on
            // the receiver's thread before this event was queued; this block
            // repeats it, which is harmless — the OFF path is two idempotent
            // calls — and is what the host tests drive. The old wording,
            // "FM source released for sleep", asserted an outcome this block
            // never observed.
            s.settings.log.push(&at, "sleep: FM release re-sent from the drain");
        }
        // The panel key, after the queue is empty and every borrow is gone. Both
        // arms retune, and a retune drains again — which is why this is taken
        // out of the state FIRST, so a re-entrant drain finds nothing to do and
        // the recursion is one level deep rather than unbounded.
        // TAKEN IN ITS OWN STATEMENT, and that is not style. On edition 2021 the
        // temporary in an `if let` scrutinee lives to the END OF THE IF-LET, so
        // writing this as `if let Some(a) = self.state.borrow_mut()...take()`
        // kept the `RefMut` alive across the body — and the body retunes, which
        // borrows the same cell. That is a `BorrowMutError` panic on the first
        // press of a steering-wheel button, which is exactly what it did.
        //
        // AND THE RECURSION IS NOT ACTUALLY ONE LEVEL DEEP, which the paragraph
        // above got wrong. Taking the action first stops the NESTED drain from
        // re-applying THIS action — but nothing stopped it applying a DIFFERENT
        // one, because the nested drain runs its own queue loop and a key that
        // arrives while `tune` is working installs a fresh `panel_action`
        // (`apply_event`'s `PanelKey` arm). The inner step then tunes to its own
        // target, and the outer `tune` — which drains BEFORE it writes `dial`
        // and `asserted` — overwrites both with its older one. The radio is left
        // on the inner station with `asserted` naming the outer, and nothing
        // heals `asserted`, so every wheel press after that anchors on a preset
        // the driver is not listening to.
        //
        // The guard is a flag rather than a reorder because the drain-then-write
        // order in `tune` is itself deliberate: it flushes stale frequency
        // reports so they cannot clobber the dial being asserted. So the nested
        // drain leaves the new action pending, and the loop here picks it up
        // once the outer step has finished and its writes have landed. A burst
        // still collapses to one step — the queue loop above overwrites the same
        // `Option` — and this loop terminates because each pass needs a key that
        // really arrived.
        //
        // NOT REPRODUCED IN A PROBE, and the reason is structural rather than an
        // omission: it needs a key to land between the outer queue loop emptying
        // and the nested drain, which on the device is another thread delivering
        // a broadcast and here is a window no single-threaded harness can reach
        // into. Held by reading, not by measurement.
        loop {
            let pending = {
                let mut s = self.state.borrow_mut();
                if s.busy {
                    None
                } else {
                    s.panel_action.take()
                }
            };
            let Some(action) = pending else { break };
            self.state.borrow_mut().busy = true;
            self.apply_panel_action(action);
            self.state.borrow_mut().busy = false;
        }
        // AND THE TARGET RE-COMMANDED, if a report said the front end left it.
        // See [`State::reassert`]: the hold fixes what the driver SEES, and this
        // is what puts the radio back under it.
        //
        // AFTER THE PANEL LOOP, because a key that has already arrived outranks
        // a target the driver has stopped asking for — and the guard below is
        // what stops the older one landing on top of the newer step anyway.
        //
        // NOT WRAPPED IN `busy`: `tune` sets and clears it itself across its own
        // nested drain, which is the same protection and already correct. The
        // check here is for a re-assert reached from INSIDE that nested drain.
        //
        // Terminates on the budget — `REASSERT_TRIES` re-commands per hold, and
        // only a fresh step re-arms it.
        loop {
            let owed = {
                let mut s = self.state.borrow_mut();
                if s.busy {
                    None
                } else {
                    // THE HOLD THAT ASKED FOR THIS MUST STILL BE THE LIVE ONE.
                    // A newer press installs a newer target and a newer hold, and
                    // re-commanding the older one on top of it would step the
                    // driver backwards — the exact complaint this whole path
                    // exists to answer. Dropped either way: a target whose hold
                    // has gone is a target nothing is claiming any more.
                    let owed = s.reassert.take();
                    owed.filter(|mhz| s.hold.is_some_and(|h| h.mhz == *mhz))
                }
            };
            let Some(mhz) = owed else { break };
            self.state.borrow_mut().settings.log.push(&stamp(), &format!("reasserting {mhz:.1}"));
            self.tune(mhz);
        }
        // Once, after the whole batch: a position change re-runs the nearby
        // query and re-resolves the hero and the strip, and none of that is
        // cheap enough to do per event.
        if std::mem::take(&mut self.state.borrow_mut().location_dirty) {
            // NEARBY FIRST, because it is what LEARNS the band from this fix and
            // the strip below reads what it learned.
            self.refresh_nearby();
            self.resolve_presets();
            self.push_hero();
            self.push_presets();
        }
    }

    /// Apply one thing the logo worker said.
    ///
    /// MUST RUN ON THE UI THREAD. Every variant is inert data; the decisions
    /// were all made in `logos::search::Model`, which answers `false` for a
    /// result whose generation the driver has already moved past.
    fn apply_logo_event(&self, event: service::Event) {
        match event {
            service::Event::Results { generation, rows } => {
                let changed = self.state.borrow_mut().logo.results_arrived(generation, rows);
                if changed {
                    self.push_logo_search();
                }
            }
            service::Event::Thumb { generation, index, art } => {
                let landed = self.state.borrow_mut().logo.thumb_arrived(generation, index, art);
                if !landed {
                    return;
                }
                // ONE ROW, not all seventeen properties. A thumbnail lands on
                // the frame that can least afford a full republish — three more
                // are still downloading behind it.
                let cell = self.state.borrow().logo.cells().get(index).cloned();
                let replaced = match &cell {
                    Some(c) => crate::logos::ui::update_candidate(&self.ui(), index, c),
                    None => false,
                };
                if !replaced {
                    self.push_logo_search();
                }
            }
            service::Event::SearchFailed { generation } => {
                let changed = self.state.borrow_mut().logo.search_failed(generation);
                if changed {
                    self.push_logo_search();
                }
            }
            service::Event::Saved { base, assigned } => {
                {
                    let mut s = self.state.borrow_mut();
                    s.logo.saved();
                    // UPPERCASED to match the key `art_for` writes. The store is
                    // case-insensitive (`store::safe_base` uppercases) but this
                    // cache is a plain `HashMap`, so a lower-case base here
                    // would leave the OLD art on the face until the next launch.
                    //
                    // Every rung goes, not just one: the hero's entry and the
                    // tile's are different keys holding the same stale station.
                    let key = base.to_uppercase();
                    s.art.retain(|(b, _, _), _| *b != key);
                    // AND THE ONE-SHOT GUARD GOES WITH IT. A new master is a new
                    // picture: `put_original` wipes the old dark treatment, and
                    // `assign_from_urls` adapts the replacement at import — but
                    // if THAT pass failed, this station is owed another attempt
                    // and `adapt_tried` would otherwise refuse it for the rest of
                    // the run.
                    s.adapt_tried.remove(&key);
                    s.settings.log.push(&stamp(), &format!("logo saved: {key}"));
                }
                // A NEW MASTER OPENS THE DARK PICKER RATHER THAN CLOSING THE
                // WINDOW. The reference does the same and at the same moment —
                // `setDarkPick` on the save's answer, with the picker's own close
                // then closing the search behind it — because this is the one
                // instant the driver is already looking at that logo and thinking
                // about it. Asking later would be asking about a picture they
                // have stopped caring about.
                //
                // ONLY FOR AN ASSIGN. `SavePrefs` answers the same event and
                // moves no picture; there is nothing new to adapt when the driver
                // has toggled "Display Call Sign".
                if assigned {
                    self.state.borrow_mut().dark_pick =
                        Some(DarkPick {
                            base,
                            items: Vec::new(),
                            // The plate FLOOR until the pipeline answers. Every
                            // master can build it, so the AUTO badge lands on a
                            // real row even in the instant before the choices
                            // arrive — and no row is drawn in that instant anyway.
                            pick: crate::logos::dark::stages::Treatment::Plate,
                            selected: 0,
                        });
                    // The base is read back out rather than kept from above: it
                    // was moved into `DarkPick`, and one owner of that string is
                    // one place it can go stale.
                    let want = self.state.borrow().dark_pick.as_ref().map(|d| d.base.clone());
                    if let (Some(w), Some(base)) = (self.state.borrow().worker.as_ref(), want) {
                        w.offer_dark(&base);
                    }
                    // The window stays up, in its waiting state, and the face
                    // behind it is already repainted — `art` was invalidated
                    // above, so the new logo is on the hero before the picker
                    // asks about its dark cut.
                    self.push_logo_search();
                    self.push_hero();
                    self.push_presets();
                    return;
                }
                // Dismiss and tear down, in that order — `close_logo_search`
                // reads the target, which `close` then clears.
                self.ui().set_overlay(Overlay::None);
                self.close_logo_search();
                self.push_hero();
                self.push_presets();
                self.push_settings();
            }
            // THE PICKER'S FOUR TREATMENTS ARRIVED.
            //
            // Dropped when the driver has already moved on — closing the window
            // clears `dark_pick`, and a base that no longer matches is an answer
            // to a question about a different station.
            service::Event::DarkChoices { base, items, pick, open_on } => {
                {
                    let mut s = self.state.borrow_mut();
                    let Some(d) = s.dark_pick.as_mut().filter(|d| d.base == base) else {
                        return;
                    };
                    d.selected = items.iter().position(|(t, _)| *t == open_on).unwrap_or(0);
                    d.pick = pick;
                    d.items = items;
                }
                self.push_logo_search();
            }
            // NOTHING TO OFFER — no master, or one that will not decode. The
            // window closes as an ordinary save would have, because there is no
            // question to put and a picker with no answers is worse than none.
            service::Event::NoDarkChoices { base } => {
                if self.state.borrow().dark_pick.as_ref().is_none_or(|d| d.base != base) {
                    return;
                }
                self.close_dark_pick();
            }
            // A BACKGROUND PASS FINISHED. It repaints one station's art and
            // touches nothing else — no overlay is dismissed, no search state is
            // cleared, nothing is announced. The driver did not ask for this and
            // should notice only that a logo stopped sitting on a white plate.
            service::Event::Adapted { base } => {
                {
                    let mut s = self.state.borrow_mut();
                    let key = base.to_uppercase();
                    // The DARK rungs only. The light entries are still correct —
                    // an adaptation writes `dark.png` and the `k-*` ladder and
                    // leaves `display.png` alone — and dropping them would buy a
                    // decode per tile for a picture that has not changed.
                    s.art.retain(|(b, _, dark), _| !(*dark && *b == key));
                    s.settings.log.push(&stamp(), &format!("adapted {key} for dark"));
                }
                self.push_hero();
                self.push_presets();
            }
            service::Event::SaveFailed { reason } => {
                {
                    let mut s = self.state.borrow_mut();
                    s.settings.log.push(&stamp(), &format!("logo save failed: {reason}"));
                    // The BARE reason: `error_body` wraps it as "Couldn't save
                    // this logo — {reason}. Try a different result.", so a
                    // reason that repeats the prefix reads twice on the face.
                    s.logo.save_failed(reason);
                }
                self.push_logo_search();
                self.push_settings();
            }
        }
    }

    /// Write the parting snapshot: the dial, the RDS on it, and how this run
    /// ended.
    ///
    /// Cheap enough to run from a lifecycle callback that Android is waiting on
    /// — one small file, written temporary-then-renamed — and rare enough that
    /// the flash does not care: pause, stop, destroy and the low-memory warning,
    /// which together happen a handful of times a drive.
    ///
    /// STALE RDS IS NOT WRITTEN. If the carrier has already gone quiet the face
    /// is showing nothing, and preserving nothing is the honest snapshot; the
    /// dial and the launch counter still go down, because those are facts.
    pub fn persist_session(&self, parting: crate::session::Parting) {
        let s = self.state.borrow();
        let snapshot = crate::session::Session {
            dial: s.dial,
            saved_at: crate::session::now_unix(),
            launches: s.launches,
            parting,
            rds: if s.rds_stale { RdsState::default() } else { s.rds_state.clone() },
        };
        crate::session::save(&s.prefs_dir, &snapshot);
    }

    fn apply_event(&self, event: TunerEvent) {
        let mut s = self.state.borrow_mut();
        match event {
            // ── OSMAND, AND EVERY DECISION ABOUT IT IS ONE CALL AWAY ─────────
            //
            // The three integers go into `crate::nav` exactly as OsmAnd sent
            // them, sentinels included, and what they mean is decided there
            // where it is tested. Nothing is logged per update: these arrive at
            // about one a second while a route is running and would fill the
            // ring in three minutes. The log line is written on a CHANGE of
            // state instead — see `push_nav`.
            TunerEvent::Nav { distance_to, turn_type, .. } => {
                let now = crate::session::now_unix();
                s.nav.update(distance_to, turn_type, now);
                // THE BORROW GOES BEFORE THE PUBLISH, which is the rule this
                // file states everywhere: `push_nav` takes its own, and holding
                // one across it is the `BorrowMutError` this codebase keeps
                // warning about.
                drop(s);
                self.push_nav();
            }
            TunerEvent::NavVoice { cmds, played } => {
                let now = crate::session::now_unix();
                s.nav.speak(&cmds, &played, now);
                drop(s);
                self.push_nav();
            }
            TunerEvent::NavInfo(route) => {
                let now = crate::session::now_unix();
                s.nav.poll(route, now);
                drop(s);
                self.push_nav();
            }
            TunerEvent::Connected(c) => {
                if let Some(mhz) = c.mhz {
                    s.dial = mhz;
                    // The first word from the radio about where it actually is.
                    // A warm restore that guessed wrong is thrown away here,
                    // before anybody reads it off the hero.
                    settle_warm(&mut s, mhz);
                }
                s.settings.log.push(&stamp(), "connect ok");
                // CLAIM THE AUDIO SOURCE. The face opens with the power button
                // lit, and until this ran, that was a claim nobody had made: the
                // tuner tuned, RDS and signal arrived, and the MCU was still
                // routing some other source, so there was no SOUND. Toggling the
                // power button off and on fixed it, because the ON half is the
                // only thing that had ever sent the app-IN broadcast.
                //
                // Here rather than beside `connect()`, because `connect` returning
                // Ok only means bindService was accepted. `setAudioEnabled`
                // needs a live binder for its `setRadioBackServiceOn` half, and
                // this event is the first moment there is one.
                if s.audio {
                    s.tuner.set_audio_enabled(true);
                }
                // THE METER'S ONLY HEARTBEAT. Without this the level is read
                // once here and once per retune, and the bars then sit frozen on
                // a reading minutes old while the car drives out of range —
                // which is worse than no meter, because it looks live.
                //
                // Each tick COMMANDS the tuner, so the bridge skips ticks where
                // FM does not own the MCU source: it can never retune a front end
                // that Bluetooth or Android Auto is using.
                //
                // Here rather than beside `connect`, for the same reason the
                // audio claim is: `connect` returning Ok only means bindService
                // was accepted, and this needs a live binder.
                s.tuner.start_level_watch(LEVEL_WATCH_MS);
                // THE GETTER POLL, on its own thread. Started here for the same
                // reason, and stopped on `Disconnected`: a poll against a dead
                // binder is a thread waking every 1.5s to be told nothing.
                crate::android::start_state_poll(s.tuner.clone(), POLL_MS);
            }
            TunerEvent::Position { lat, lon, fix, in_motion } => {
                let next = fake::FakeLocation { lat, lon, fix, in_motion };
                if next != s.location {
                    s.location = next;
                    s.location_dirty = true;
                }
                // THE CAR PULLED AWAY. CarFM closes reorder mode on motion
                // (`closeOnMotion`, CarFmFace.tsx:641) and so does this: a mode
                // that is refused while moving must not survive because the
                // driver happened to be stopped when they opened it.
                //
                // The overlay above it goes too. Leaving the logo window up over
                // a face that has left reorder mode strands the driver in a
                // window whose only door has closed behind them.
                if in_motion && self.ui().get_reordering() {
                    self.ui().set_reordering(false);
                    if self.ui().get_overlay() == Overlay::LogoSearch {
                        self.ui().set_overlay(Overlay::None);
                    }
                    s.settings.log.push(&stamp(), "reorder: closed, the car is moving");
                }
            }
            TunerEvent::Note(line) => {
                s.settings.log.push(&stamp(), &line);
            }
            TunerEvent::ConnectFailed(why) => {
                s.settings.log.push(&stamp(), &format!("connect failed: {why}"));
            }
            TunerEvent::Disconnected => {
                s.settings.log.push(&stamp(), "disconnected");
                // Nothing to command, and a watch left running against a dead
                // binder is a thread waking every five seconds to fail.
                s.tuner.stop_level_watch();
                crate::android::stop_state_poll();
                // Nor is a post-retune schedule worth finishing against a binder
                // that has gone: both timers would fire into a tuner that cannot
                // answer, and the retry budget belongs to the next connection.
                s.level_first.stop();
                s.level_correction.stop();
                s.level_retry.stop();
                s.level_retries = signal::LEVEL_RETRY_MAX;
                // The reading it last produced is not a reading of anything now.
                s.level = None;
                s.dotted = 0;
            }
            TunerEvent::Snapshot(snap) => {
                // ONE TURN OF THE VENDOR-GETTER POLL — the dial backstop, the
                // audio self-heal and the reception-loss band.
                //
                // It arrives as an EVENT because the poll runs on its own thread
                // (`android::start_state_poll`): the three getters are binder
                // calls, CarFM makes them off the UI thread, and the first
                // version of this used a `slint::Timer`, which is the UI thread.
                // So it takes the same road every binder callback takes — push,
                // wake, drain here — and the work below happens where all the
                // other state changes happen.
                //
                // WHAT THIS DELIBERATELY DOES NOT TOUCH: PS, RadioText and PTY.
                // CarFM's poll does not drive them either, and spells out why —
                // "on this unit those getters are worthless: getRtMessage() is a
                // hardcoded \"\" on one manager and region-gated on the other,
                // psName stays empty for a passive bound client, and getPTYType()
                // returns 0". Nor the stereo pilot, whose getter is stuck true
                // (see `STEREO_SETTLE`). The snapshot carries all four so a
                // diagnostic can show them; none of them reaches the face.
                //
                // The RDS expiry lives on the 1s heartbeat rather than here,
                // which is where CarFM keeps it. Same rule, a slightly faster
                // clock.

                // THE DIAL BACKSTOP. A `notifyCurrentFrequency` that never
                // arrived leaves the face on a station the radio has left; this
                // is what notices. Deliberately NOT a retune: the RDS reset
                // belongs to the callback, and CarFM's poll does not reset it
                // either — it sets the frequency and nothing else.
                if let Some(mhz) = snap.mhz {
                    let moved = crate::stations::preset_key(f64::from(mhz))
                        != crate::stations::preset_key(f64::from(s.dial));
                    if (FM_LO..=FM_HI).contains(&mhz) && moved {
                        s.dial = mhz;
                        let at = stamp();
                        s.settings.log.push(&at, &format!("poll: dial is {mhz:.1}"));
                    }
                }

                // THE AUDIO SELF-HEAL, and it is the reason this poll exists at
                // all.
                //
                // On a head unit the audio is analog and MCU-routed, so Android's
                // audio focus cannot answer "is FM actually playing" — and after
                // a permanent AUDIOFOCUS_LOSS the OS never sends a GAIN, so a
                // focus-driven state goes dark when Android Auto takes over and
                // never comes back when the MCU hands FM back. The MCU's own
                // source register is the truth and it heals itself. CarFM:
                // `if (typeof p.source === 'number' && p.source >= 0 &&
                //     !userPoweredOffRef.current) setFmAudioActive(p.source === 4);`
                //
                // An explicit power-off still wins, which is what
                // `user_powered_off` is for: without it the poll would switch the
                // radio back on 1.5s after the driver switched it off.
                if snap.mcu_source.is_some() && !s.user_powered_off {
                    let playing = snap.fm_is_playing();
                    if s.audio != playing {
                        s.audio = playing;
                        let at = stamp();
                        // THE SAME READING MEANS TWO OPPOSITE THINGS, and which
                        // one depends on whether a sleep release just ran. See
                        // `State::sleep_released_at`: within the window, FM
                        // coming back is the release FAILING, not the MCU
                        // relenting, and it needs a different fix.
                        let since = s.sleep_released_at.map(|at| at.elapsed());
                        let line = match (playing, since) {
                            (true, Some(d)) if d < SLEEP_RELEASE_WINDOW => format!(
                                "poll: mcu source still FM {:.1}s after the sleep release — it did not take",
                                d.as_secs_f32()
                            ),
                            (true, _) => "poll: the MCU handed FM back".to_string(),
                            (false, _) => "poll: the MCU took FM away".to_string(),
                        };
                        s.settings.log.push(&at, &line);
                    }
                }

                // THE RECEPTION-LOSS BAND, once per turn. This is the whole
                // reason the hysteresis has a cadence at all — see
                // `settle_dotted`.
                settle_dotted(&mut s);
            }
            TunerEvent::Frequency(f) => {
                // EVERY report counts, including the ones this app did not ask
                // for — the counter's whole job is to distinguish "a report has
                // arrived since the commit" from "the dial happens to equal the
                // target", and `tune` makes the second true immediately.
                s.freq_seq = s.freq_seq.wrapping_add(1);
                // However this report came about — a seek landing, a retune, the
                // vendor walking its own presets — the front end is now ON a
                // frequency, so no sweep is in flight any more.
                s.scanning = false;
                if let Some(mhz) = f.mhz {
                    s.dial = mhz;
                    // THE HOLD RELEASES ON THE TUNER'S WORD, or not at all. A
                    // report that arrived after the commit and names the target
                    // means the front end really is there, so the face can go
                    // back to telling the truth. Anything else is the vendor
                    // walking its own bank, and is ridden out.
                    if let Some(h) = s.hold {
                        if s.freq_seq > h.since_seq
                            && (mhz - h.mhz).abs() < crate::stations::FREQ_EPS as f32
                        {
                            // ARRIVING IS NOT THE END OF IT, and releasing here was
                            // a real defect the unit showed. The vendor's bank walk
                            // is a retune of its own and it can land AFTER the
                            // report that confirms ours — a drive log caught
                            // exactly that: "settled on 94.9" and then, with the
                            // hold already gone, "tuned 104.1". Nothing corrected
                            // it, because releasing had disarmed the defence at the
                            // moment the attack arrived.
                            //
                            // So the hold now runs its full `HOLD_CAP` whatever
                            // happens, and only `expire_hold` ends it. Nothing is
                            // hidden by that: the face shows the station the driver
                            // asked for, we are actively keeping the radio there,
                            // and any tune the DRIVER makes meanwhile drops the
                            // hold in `tune` before it can lie about one.
                            //
                            // Only the pending re-command is dropped — at this
                            // instant the front end really is where it belongs and
                            // nothing is owed. The budget is deliberately NOT
                            // re-armed; a vendor that keeps moving still has to
                            // spend from the same two.
                            s.reassert = None;
                            let line = format!("settled on {:.1}", h.mhz);
                            s.settings.log.push(&stamp(), &line);
                        } else if s.freq_seq > h.since_seq {
                            let line =
                                format!("holding {:.1}, dial went to {mhz:.1}", h.mhz);
                            s.settings.log.push(&stamp(), &line);
                            // AND THE RADIO IS TAKEN BACK, not just the face.
                            // This branch is the vendor's own bank walk having
                            // commanded the front end AFTER we did, so the
                            // station playing is not the one the driver asked
                            // for and no later event will heal it. See
                            // [`State::reassert`] for why the hold alone was not
                            // enough and why this is budgeted.
                            //
                            // RECORDED, NOT DONE, because `apply_event` holds
                            // `State` borrowed for the whole match and `tune`
                            // borrows the same cell. `drain_events` takes it.
                            if s.reasserts_left > 0 {
                                s.reasserts_left -= 1;
                                s.reassert = Some(h.mhz);
                            }
                        }
                    }
                }
                // A WARM RESTORE THIS EVENT CONFIRMS IS NOT A RETUNE. On the
                // first frequency report of a run the radio is telling us where
                // it already was, not moving — and if that is the dial the
                // snapshot came from, wiping it here would undo the restore one
                // frame after it was drawn, which is the blank hero all of this
                // exists to prevent.
                //
                // EXACTLY ONE EVENT'S WORTH OF PROTECTION, retired on the way
                // through rather than left to the first group. This is the arm
                // that clears the face, and an uncalibrated tuner can report a
                // frequency it cannot convert — `f.mhz` is None, the dial does
                // not move, and the check would go on answering yes for as long
                // as that lasted. From the second event on, a retune resets as
                // it always did.
                let dial = s.dial;
                let confirmed = settle_warm(&mut s, dial);
                s.warm_dial = None;
                if !confirmed {
                    // EVERY frequency event, and `reset_for_retune` rather than
                    // `reset`: a full reset leaves the old PI as an incumbent
                    // that then needs twelve groups to displace instead of three.
                    s.rds.reset_for_retune();
                    s.rds_state = RdsState::default();
                    // The expiry clock restarts with the dial. Carrying the old
                    // station's last-heard stamp across a retune would expire the
                    // new one the moment it was tuned, if the old one had been
                    // quiet for twenty-four seconds already.
                    s.last_rds_at = None;
                    s.rds_stale = false;
                    // THE PILOT IS NOT CLEARED HERE, and it used to be. The
                    // argument for clearing it was that a new station has
                    // reported nothing yet, so the pill should go blank rather
                    // than carry the last station's lock — which sounds right and
                    // was never checked against CarFM, where this same handler
                    // clears `name`, `text`, `pty`, `tp`, `ta` and `pi` and
                    // deliberately not stereo (RadioScreen.tsx:2794-2802).
                    //
                    // The cost on the device is the whole defect: the vendor
                    // sends `notifyCurrentFrequency` for its own reasons — its
                    // preset walk transits several stations on one wheel press —
                    // while `notifyStereo` arrives only when the pilot actually
                    // changes. So the blanking ran far more often than the
                    // refilling, and the pill was almost never lit.
                }
                // THE POST-RETUNE READ SCHEDULE, on every frequency report, which
                // is where CarFM arms it. Unconditional — including on the warm
                // restore above, where the RDS is kept but the level still has to
                // be re-measured, because a level is a reading of the air right
                // now and nothing on disk can stand in for it.
                // THE LEVEL BELONGS TO THE STATION WE JUST LEFT. `tune` drops it
                // on the way out, but a retune the VENDOR drove — a landed seek,
                // the hardware buttons, the service's own preset walk — never
                // passes through `tune`, so the meter sat on the old station's
                // reading until the 1s read replaced it. CarFM clears it right
                // here, in the same handler (`setFmLevel(null)`,
                // RadioScreen.tsx:2829).
                s.level = None;
                arm_level_schedule(&mut s);
                let line = format!("tuned {:.1}", s.dial);
                s.settings.log.push(&stamp(), &line);
            }
            TunerEvent::RdsGroup(g) => {
                // A LIVE GROUP RETIRES THE RESTORE. From here on the decoder is
                // the authority on this dial and the snapshot has done its job,
                // so a later retune must clear the face in the ordinary way.
                s.warm_dial = None;
                // The carrier is alive. Stamped for EVERY group that arrives,
                // published or not — a group the consensus gates reject is still
                // proof that there is a transmitter out there.
                s.last_rds_at = Some(std::time::Instant::now());
                let hex = g
                    .0
                    .iter()
                    .map(|b| format!("{b:04x}"))
                    .collect::<String>();
                let published = s.rds.push(&hex);
                let mut changed = false;
                if std::mem::take(&mut s.rds_stale) {
                    // THE CARRIER IS BACK. The expiry cleared the face but
                    // deliberately did NOT reset the decoder, so the decoder
                    // still holds this station — and `push` returns None when
                    // nothing CHANGED, which after an expiry is the normal case.
                    // Without this the plate would stay blank indefinitely while
                    // a perfectly good station played, waiting for a change that
                    // has no reason to come. CarFM restores `decoder.state()`
                    // wholesale here for exactly this reason.
                    s.rds_state = s.rds.state();
                    changed = true;
                } else if let Some(published) = published {
                    s.rds_state = published;
                    changed = true;
                }
                // ON THE RECORD, when something actually changed.
                //
                // CarFM logs a line here on every published change
                // (RadioScreen.tsx:2955) and Carnyx logged nothing at all, so the
                // one subsystem whose behaviour is hardest to reason about on a
                // dashboard left no trace. `format_state` is the format CarFM's
                // own differential harness prints, which is what makes a Carnyx
                // line and a CarFM line from the same drive diffable — it was
                // written for exactly this and had no caller.
                //
                // Change-gated: the unchanged case is the common one, since the
                // same group repeats many times a second. It used to be quiet in
                // reception-testing mode as well; that mode was CarFM's and is
                // gone.
                if changed {
                    let line = rds::format_state(&s.rds_state, &s.rds.stats(), &s.rds.quality());
                    let at = stamp();
                    s.settings.log.push(&at, &format!("RDS {line}"));
                }
            }
            TunerEvent::RadioText(rt) => {
                // The vendor's own getter, which is a different path from the
                // decoded 2A groups and is NOT consensus-gated. It is taken only
                // when the decoder has published nothing.
                //
                // AND NOT WHILE STALE. This getter answers from the vendor's own
                // cache, so after an expiry it hands back the very string the
                // expiry just removed — the carrier is gone and the text would
                // come straight back on the next poll. Only a decoded group is
                // evidence that there is a transmitter out there, and only a
                // group clears the flag.
                if s.rds_state.rt.is_empty() && !s.rds_stale {
                    s.rds_state.rt = rt;
                }
            }
            TunerEvent::Stereo(on) => {
                // LOGGED RAW, before the settle window, because the flapping the
                // window hides is the measurement worth having — it is what
                // CarFM's `stereoFlips` counter exists to record, and it is the
                // only way to tell "the pilot is genuinely marginal" from "this
                // callback never arrives on this unit" without a cable.
                s.settings.log.push(&stamp(), &format!("stereo {on}"));
                s.stereo_pending = Some(on);
                s.stereo_settle.start(
                    slint::TimerMode::SingleShot,
                    STEREO_SETTLE,
                    settle_stereo_current,
                );
            }
            TunerEvent::Pty(pty) => {
                // Same cache, same rule as the vendor RadioText above.
                if s.rds_state.pty.is_none() && !s.rds_stale && (0..=31).contains(&pty) {
                    s.rds_state.pty = Some(pty as u8);
                }
            }
            TunerEvent::Level(l) => {
                // A reading taken while the tuner was moving is not a reading.
                if l.trustworthy {
                    // A good read RE-ARMS the budget, so a rejection later in the
                    // drive gets its own two attempts rather than inheriting a
                    // budget some earlier retune spent.
                    s.level_retries = signal::LEVEL_RETRY_MAX;
                    s.level = Some(l.level);
                    // Every accepted reading, with what it becomes on the glyph.
                    // This used to be suppressed in reception-testing mode, where
                    // CarFM's structured sample carried the same figures; that
                    // mode was CarFM's and is gone, so the line is unconditional.
                    let lit = signal::level_to_lit(Some(f64::from(l.level)));
                    let shown = signal::describe(lit.as_ref());
                    let at = stamp();
                    s.settings.log.push(&at, &format!("level {} @ {} → {shown}", l.level, l.asked));
                } else {
                    // A REJECTION IS RETRIED, not left to the periodic watch.
                    //
                    // On CarFM's drive logs a rejection was almost always
                    // `landed=0` — the chip saying it was not ready — which the
                    // next attempt a second later usually clears. Without this, a
                    // rejected read right after a retune leaves the meter blank
                    // until the next tick, because the retune already dropped the
                    // previous reading and there is nothing to fall back on.
                    let at = stamp();
                    let why = l.error.as_deref().unwrap_or("");
                    s.settings.log.push(
                        &at,
                        &format!("level: REJECTED asked={} landed={} {why}", l.asked, l.landed),
                    );
                    if s.level_retries > 0 {
                        s.level_retries -= 1;
                        s.level_retry.start(
                            slint::TimerMode::SingleShot,
                            std::time::Duration::from_millis(signal::LEVEL_RETRY_MS),
                            read_level_current,
                        );
                    }
                }
            }
            TunerEvent::PanelKey { code, key, action } => {
                let named = key.map_or("unknown", crate::android::PanelKey::label);
                // THE GAP SINCE THE LAST IDENTICAL KEY, which is the one fact
                // that decides whether this unit sends a release edge at all.
                // See [`State::last_panel`]: two lines a few milliseconds apart
                // is a press and a release, and means every press is stepping
                // twice; one line per press means the guard that cannot fire has
                // nothing to guard against.
                let gap = s
                    .last_panel
                    .and_then(|(c, at)| (c == code).then(|| at.elapsed().as_millis()));
                s.last_panel = Some((code, std::time::Instant::now()));
                // WHICH SIDE OF THE GLASS THE DRIVER WAS ON, appended to a line
                // that already exists rather than written as a second one. This
                // is what separates the two ways the station pop-up can fail to
                // appear, and it costs no ring space:
                //
                //   `panel key … [background]` and a `station pop-up:` line
                //       — the whole path ran; read that line for the outcome
                //   `panel key … [background]` and NO pop-up line
                //       — the dial did not move, so there was nothing to say
                //   `panel key … [face]`
                //       — THE FLAG NEVER CLEARED, and that is the bug: the
                //         announce gate is `!is_foreground()`, so a stuck flag
                //         suppresses every pop-up while the driver is elsewhere
                //   no `panel key` line at all
                //       — the press never reached this process
                //
                // FOCUS RIDES ALONG WHEN IT DISAGREES, and gates nothing — see
                // `android::FOCUSED`. `[face, unfocused]` says a `LostFocus`
                // arrived with no `Pause`, which on a unit that switches whole
                // screens should not happen; if it does, that is the fact this
                // line exists to catch.
                let where_ = match (
                    crate::android::is_foreground(),
                    crate::android::is_focused(),
                ) {
                    (true, true) => "face",
                    (true, false) => "face, unfocused",
                    (false, _) => "background",
                };
                let line = match gap {
                    Some(ms) => format!("panel key {code} ({named}) {action} +{ms}ms [{where_}]"),
                    None => format!("panel key {code} ({named}) {action} [{where_}]"),
                };
                s.settings.log.push(&stamp(), &line);
                // THE ANCHOR IS READ HERE, NOT WHERE THE ACTION RUNS, and that
                // is the whole fix. `drain_events` applies the action only after
                // this queue is EMPTY — and one wheel press puts two things in
                // it, because the vendor service steps its own hardware bank on
                // the same press and reports the frequency it landed on. By the
                // time the action ran, `dial` was the vendor's dial and the step
                // was computed from a position the driver was never in.
                let anchor = s.anchor();
                // PRESSES ADD UP; THEY DO NOT OVERWRITE EACH OTHER.
                //
                // This was `.or(s.panel_action)`, which keeps whichever action is
                // newest and silently loses the ones before it. A step takes about
                // 680ms on the unit — the retune, the drain, the RDS pump and the
                // republish are all synchronous, and `busy` holds any queued key
                // until they finish — so anything pressed inside that window was
                // simply dropped: "it's impossible to quickly press prev or next 3
                // or more times and have it move that many positions". A drive log
                // measured 20 presses producing 18 steps, with gaps as short as
                // 203ms.
                //
                // Summing the direction instead makes a burst ONE step of several
                // positions, which is both correct and far quicker than three
                // steps would have been: one tune, one morph, one settle.
                // `step_preset_from` already handles any magnitude —
                // `(active + dir).rem_euclid(n)` never cared — `step_morph` takes
                // `dir.signum()` for the direction the cards fly, and
                // `step_discards` only asks the sign.
                //
                // THE ANCHOR IS THE FIRST PRESS'S, not the last. Every key in a
                // burst reads the same `anchor()` anyway, because nothing has
                // moved `asserted` yet, but keeping the earliest is what stays
                // right if that ever stops being true.
                let fresh = panel_action(key, &action, anchor);
                s.panel_action = match (fresh, s.panel_action) {
                    (
                        Some(PanelAction::Step { dir, .. }),
                        Some(PanelAction::Step { dir: queued, from }),
                    ) => Some(PanelAction::Step { dir: queued + dir, from }),
                    (Some(now), _) => Some(now),
                    (None, queued) => queued,
                };
            }
            TunerEvent::Illumination { ui_mode, .. } => {
                s.settings.log.push(&stamp(), &format!("illumination {ui_mode}"));
            }
            // ── THE UNIT IS GOING TO SLEEP ────────────────────────────────────
            //
            // Hand the FM source back before this process stops running. The MCU
            // remembers the current source across a sleep and restores it on
            // ACC-on, so a unit left on FM comes back into FM and the stock radio
            // app launches itself. Released, there is nothing to restore.
            //
            // FLAGGED HERE, ACTED ON AFTER THE DRAIN — the rule the panel key
            // follows, for the same reason: `set_audio_enabled` crosses into the
            // vendor's binder, and a `RefCell` borrow held across a call into
            // another runtime is a lock held over code that can call back.
            //
            // The action is logged verbatim because the two triggers are not
            // equally trustworthy, and which one arrives is what a drive settles.
            TunerEvent::Sleep { action, release } => {
                // LOGGED EITHER WAY. Which broadcast arrives, and whether one
                // arrives at all, is the open question this path exists to
                // settle — and it is worth answering on a unit where the driver
                // has turned the release off.
                //
                // `release` IS AN OUTCOME, NOT A PLAN. `NwdBridge.releaseSource`
                // already ran, on the thread that heard the broadcast, before
                // this event was queued — because the queued path is a thread
                // hop and a drain taken while the MCU is cutting power to the
                // SoC, with no wake lock behind it. Empty means no Java did it,
                // which on the host is every time.
                let on = s.settings.release_on_sleep;
                let tail = if release.is_empty() {
                    if on { String::new() } else { " (release is off)".to_string() }
                } else {
                    format!(" — {release}")
                };
                s.settings.log.push(&stamp(), &format!("sleep: {action}{tail}"));
                s.sleep_release = on;
            }
            TunerEvent::ScanState(_) | TunerEvent::RadioState(_) => {}
        }
    }

    /// Feed the replayed corpus in until the decoder has published a name.
    ///
    /// THE SOURCE IS A RECORDING. `fake::FakeRdsStream` is CarFM's captured
    /// group shapes replayed; on the device this loop is the vendor's 90 ms pump
    /// and every group goes to `push()` undeduplicated, because the consensus
    /// gates count the repeats.
    /// The running morph's frame tally, for `examples/morphbench.rs`.
    ///
    /// Exposed so the probe can hold the tally against its OWN count of frames.
    /// The device has no second opinion available — that is the whole reason this
    /// counter exists — so the mechanism has to be shown to track real frames
    /// somewhere, and here is the only place it can be.
    pub fn morph_frames_for_bench(&self) -> u32 {
        self.state.borrow().morph_frames
    }

    /// The RDS pump, for `examples/morphbench.rs`.
    ///
    /// It runs inside `tune`, which runs after the morph is armed and before any
    /// frame can be drawn — so its cost is time the animation spends advancing
    /// off screen. Exposed so that can be measured rather than argued about.
    pub fn pump_rds_until_settled_for_bench(&self) {
        self.pump_rds_until_settled();
    }

    fn pump_rds_until_settled(&self) {
        // NEVER against a real radio. The corpus below is SYNTHESISED — the
        // block bit-layouts for WERN were computed, not captured — so replaying
        // it into the live decoder would put an invented call sign, genre and
        // song title on the face of a car that is tuned to a real transmitter.
        // Wrong information stated confidently is worse than none, and the dial
        // guard underneath is not enough: 88.7 is a real frequency and a real
        // unit can be sitting on it.
        if self.state.borrow().tuner_is_real {
            return;
        }
        // The recording belongs to one dial. Off it, there is nothing to feed
        // the decoder and the hero correctly shows a station with no RDS.
        if !fake::FakeRdsStream::carries(self.state.borrow().dial) {
            return;
        }
        let count = self.state.borrow().stream.all().len();
        for _ in 0..count {
            let hex = self.state.borrow_mut().stream.next_group();
            let published = self.state.borrow_mut().rds.push(hex);
            if let Some(st) = published {
                self.state.borrow_mut().rds_state = st;
            }
        }
        // The pilot the vendor reported alongside the recording, assigned
        // DIRECTLY rather than through `ingest_stereo`.
        //
        // Not an inconsistency — the loop above assigns `rds_state` directly too,
        // for the same reason. This function's whole job is to leave the face
        // SETTLED against a replay, and `STEREO_SETTLE` is a two-second window
        // that exists to absorb a live callback flapping on real multipath. There
        // is no multipath in a recording, and a settled face that has to wait two
        // seconds for its pill is not settled.
        self.state.borrow_mut().stereo = Some(fake::WERN_STEREO);
    }

    /// One level reading, through the tuner's own path.
    fn read_level(self: &Rc<App>) {
        self.state.borrow().tuner.read_level_now();
        self.drain_events();
    }

    // ── Pushing state to the face ────────────────────────────────────────────

    /// Publish everything. Cheap enough to do wholesale, and a partial publish is
    /// how a face ends up showing one station's name over another's dial.
    /// One of `push_all`'s six, by index — for `examples/pushbench.rs` only, so a
    /// measurement can say WHICH of them costs what without every push having to
    /// be public.
    pub fn push_one_for_bench(&self, which: u8) {
        match which {
            0 => self.push_hero(),
            1 => self.push_presets(),
            2 => self.push_meter(),
            3 => self.push_nearby(),
            4 => self.push_settings(),
            _ => self.push_freq(),
        }
    }

    pub fn push_all(&self) {
        // THE CAP, CHECKED WHERE EVERYTHING IS REPUBLISHED ANYWAY. A tune the
        // front end never honours would otherwise leave the face committed to a
        // station it never reached. See `HOLD_CAP`.
        self.state.borrow_mut().expire_hold();
        self.push_hero();
        self.push_presets();
        self.push_meter();
        self.push_nearby();
        self.push_settings();
        self.push_freq();
    }

    /// The station licensed on the dial, from the cache when the dial and the fix
    /// have not moved.
    ///
    /// Both inputs are already tracked — a retune sets `dial`, a fix sets
    /// `location` — so a miss is exactly as rare as a real change. Cloned out of
    /// the cell in its own statement, because an `if let` scrutinee holds its
    /// borrow across the body on edition 2021 and the store below takes a
    /// mutable one.
    ///
    /// "HAVE NOT MOVED" IS A DISTANCE, NOT `==`. It was `==` on the whole
    /// `FakeLocation`, which compares two f64s exactly — so the metres of noise a
    /// stationary GPS produces missed the cache on every fix and re-ran the
    /// lookup, forever, at whatever cadence the unit reports. The station
    /// licensed on a frequency does not change because the car moved a street's
    /// width; `NEARBY_REFRESH_M` is the same threshold the nearby query uses and
    /// for the same reason.
    fn hero_row(&self) -> Option<StationRow> {
        let (key, loc) = {
            let s = self.state.borrow();
            (crate::stations::preset_key(f64::from(s.shown())), s.location)
        };
        let hit = self.state.borrow().resolved.clone();
        if let Some((k, l, row)) = hit {
            // The fix flag is compared exactly — losing or gaining a lock is a
            // real change and must miss — and only the POSITION is given slack.
            let same_place = match (l.position(), loc.position()) {
                (Some(a), Some(b)) => metres_between(a, b) < Self::NEARBY_REFRESH_M,
                (None, None) => true,
                _ => false,
            };
            if k == key && same_place {
                return row;
            }
        }
        let row = {
            let s = self.state.borrow();
            resolve(s.db.as_ref(), s.shown(), s.location.position())
        };
        self.state.borrow_mut().resolved = Some((key, loc, row.clone()));
        row
    }

    fn push_hero(&self) {
        let ui = self.ui();
        let row = self.hero_row();
        let s = self.state.borrow();
        let st = &s.rds_state;

        // The sweep flag rides the hero push because the hero is its biggest
        // consumer; the readout's dim rule reads the state directly.
        ui.set_scanning(s.scanning);

        // IDENTITY ORDER, and it matters: the station database is authoritative
        // for what is on this dial, the LEARNED map is that same database's
        // answer from the last time a fix let it speak, the decoded PS is a
        // broadcaster-controlled string that many stations scroll song titles
        // through, and the dial itself is the honest fallback. An empty ident
        // makes the face print the frequency as the identity — never an
        // inaccurate "Tuning…".
        //
        // THE LEARNED ANSWER OUTRANKS PS on purpose. It comes from the licence
        // table; PS is whatever the broadcaster is putting out this second. The
        // cost is that a car driven to another market before its next fix can
        // show a stale call sign, which CarFM accepts for the same reason — a
        // fresh lock overwrites, so it heals itself.
        let learned = s.callsigns.get(s.shown()).map(str::to_string);
        let ident = match (&row, &learned, st.ps_scrolling, st.ps.as_str()) {
            (Some(r), _, _, _) => clean_call(&r.callsign),
            (None, Some(c), _, _) => clean_call(c),
            (None, None, false, ps) if !ps.is_empty() => ps.to_string(),
            _ => String::new(),
        };
        // The station's own name must not be stripped out of its own RadioText,
        // so the RESOLVED call sign goes in, never the PS: WIBA scrolls song
        // titles through PS, and a PS of "Walk" would strip "Walk This Way".
        let call = row
            .as_ref()
            .map(|r| r.callsign_base.clone())
            .or_else(|| learned.clone());
        let shown_rt = rds::strip_station_from_rt(&st.rt, Some(s.shown()), call.as_deref());

        // ── THE BAND THEME (Design EASTER-EGGS §12) ──
        //
        // Resolved from the RadioText THE DRIVER IS SHOWN, after
        // `strip_station_from_rt` has taken the station's own name out of it. A
        // station whose name contained a band's would otherwise wear that theme
        // for as long as it was tuned, which is a skin stuck on rather than an
        // Easter egg.
        //
        // `!s.audio` is the flattened face — audio priority released — and it
        // suppresses every theme, as CarFM's `resolveEgg({ off })` does. A red
        // accent on a grey face reads as a rendering fault, not as a joke.
        //
        // RESOLVED BEFORE THE IDENTITY IS PUBLISHED, and that ordering is load
        // bearing for exactly one theme: The Beatles states `heroCase: 'lower'`,
        // which changes the STRING rather than how it is drawn, and a case rule
        // has to be applied where the string is. Everything else a theme does is
        // a colour, a face or a mark, and none of those needs to happen here.
        let themed = crate::eggs::resolve(&shown_rt, !s.audio, s.forced_egg.as_deref());
        let ident = if themed.is_some_and(|e| e.hero_lower) {
            ident.to_lowercase()
        } else {
            ident
        };

        ui.set_ident(ident.clone().into());
        // THE COMMITTED TARGET WHILE ONE IS IN FLIGHT. See `State::hold`: the
        // vendor walks its own bank on the same press, and without this the
        // driver watches the hero flick through stations they never chose.
        ui.set_freq_label(format_mhz(s.shown()).into());
        ui.set_in_band((FM_LO..=FM_HI).contains(&s.shown()));
        let active = s.active();
        ui.set_saved(active >= 0);
        ui.set_active_index(active);

        ui.set_radio_text(shown_rt.as_str().into());
        ui.set_pty(rds::pty_label(st.pty).into());

        // ── THE STATION POP-UP, HALF ONE: HAS THE DIAL MOVED? ─────────────────
        //
        // The wheel changes station whether or not the face is on screen: the
        // MCU broadcasts, `NwdBridge` hears it, and `State::reassert` makes this
        // app's choice the one that plays. A driver in another app therefore
        // gets a station change with NOTHING TO SEE.
        //
        // THE QUESTION IS ANSWERED HERE AND ACTED ON BELOW, because the second
        // half needs the station's LOGO, and the logo is resolved after this
        // borrow is dropped — `art_for` writes its decode back into the same
        // cell. The marker has to move inside the borrow; the announcement has
        // to wait for the picture.
        //
        // THE MARKER MOVES WHETHER OR NOT ANYTHING IS ANNOUNCED. Tuning on the
        // face and then switching away must stay silent, and it does, because
        // the marker moved while the face was in front. Had it only moved on an
        // announcement, the next ordinary push — an RDS group, a level read —
        // would announce a station the driver had chosen by hand a minute ago.
        //
        // The comparison is `!=` rather than an epsilon on purpose: `shown()` is
        // a dial the app itself commanded in 0.1 MHz steps, not a measurement, so
        // two presses never land a float's-breadth apart — and `announced` starts
        // as NaN, which compares unequal to everything including itself.
        // READ ONCE, USED TWICE. The gate below is a hundred and fifty lines
        // away and used to call `is_foreground()` again; between the two reads
        // the flag can flip, and the marker had already moved.
        let front = crate::android::is_foreground();
        let landed = s.shown();
        #[allow(clippy::float_cmp)]
        let moved = s.announced.get() != landed;
        if moved {
            s.announced.set(landed);
        }

        // ── AN ORDERING RACE THAT WAS LOOKED AT AND LEFT ALONE ────────────────
        //
        // Slint's `poll_events` runs `process_event` — which drains
        // `invoke_from_event_loop`, and so `drain_current` and this function —
        // BEFORE it calls the lifecycle listener
        // (i-slint-backend-android-activity-1.17.1/lib.rs:114-123). So on the
        // poll iteration carrying `Pause`, a queued drain runs here with the
        // foreground flag still TRUE, and if the dial changed in that same
        // iteration the edge is consumed with nothing announced.
        //
        // MAKING THE TRANSITION ITSELF AN EDGE WAS TRIED AND REVERTED. It
        // announces every hand-tune the moment the driver switches away, which
        // is what `the_station_pop_up_speaks_only_for_a_change_the_driver_cannot_see`
        // forbids in as many words: "an ordinary push must not announce a
        // station the driver chose themselves a minute ago". The marker cannot
        // tell "tuned, then switched away" from "tuned WHILE switching away",
        // and only the second is the race.
        //
        // It needs the dial to move in the one poll iteration that carries
        // Pause — a wheel press landing in the same handful of milliseconds as
        // the app going to the background — so the cost of the cure is certain
        // and the disease is not. The `hero:` line below is what would show it
        // happening at all.

        // THE PER-SCHEME CUT. "Back in Black" means nothing on a white page, so a
        // theme states its palette for light and dark separately and the face's
        // own scheme picks. `Pal.dark` is the authority on that — it is what
        // every other token here already reads.
        let pal = ui.global::<crate::Pal>();
        let skin = themed.map_or(crate::eggs::NO_SKIN, |e| crate::eggs::skin(e, pal.get_dark()));
        ui.set_egg(match themed {
            Some(e) => EggTheme {
                on: true,
                advanced: e.tier == crate::eggs::Tier::Advanced,
                genre: e.genre.into(),
                // ZERO IS "THE ORDINARY TOKEN", not black. Three of the five
                // themes state their genre colour as `pal.dim` — the live token
                // restated — so a zero here has to fall through to `Pal.dim` and
                // not paint the line black. `opt_rgb` is the same rule the skins
                // use, and `GenreText` tests the alpha.
                genre_ink: opt_rgb(nonzero(e.genre_ink)),
                genre_pulse: opt_rgb(nonzero(e.genre_pulse)),
                call_bolt: e.call_sign_bolt,
                horns: e.horns,
                stereo_bolts: e.stereo_bolts,
                suppress_logo: e.suppress_logos,
                body_face: e.body_face.into(),
                hero_face: e.hero_face.into(),
                genre_face: e.genre_face.into(),
                face_bold: e.face_bold,
                rt_face: e.rt_face.into(),
                freq_face: e.freq_face.into(),
                hero_scale: e.hero_scale,
                hero_track: e.hero_track,
                freq_scale: e.freq_scale,
                rt_track: e.rt_track,
                hero_lower: e.hero_lower,
                genre_cycle: e.genre_cycle.into(),
                genre_runes: e.genre_runes,
                ghost_ink: ghost(e),
                ghost_dx: e.ghost_dx,
                ghost_dy: e.ghost_dy,
                hero_glitch: e.hero_glitch,
                gear: match e.gear {
                    crate::eggs::Gear::Plain => BandGear::Plain,
                    crate::eggs::Gear::Bolt => BandGear::Bolt,
                    crate::eggs::Gear::Drum => BandGear::Drum,
                    crate::eggs::Gear::Smiley => BandGear::Smiley,
                    crate::eggs::Gear::Spiral => BandGear::Spiral,
                },
                airship: e.airship,
                ring_1: opt_rgb(ring(skin, 0).map(|r| r.0)),
                ring_1_inset: ring(skin, 0).map_or(0.0, |r| r.1),
                ring_2: opt_rgb(ring(skin, 1).map(|r| r.0)),
                ring_2_inset: ring(skin, 1).map_or(0.0, |r| r.1),
                ring_3: opt_rgb(ring(skin, 2).map(|r| r.0)),
                ring_3_inset: ring(skin, 2).map_or(0.0, |r| r.1),
                ring_4: opt_rgb(ring(skin, 3).map(|r| r.0)),
                ring_4_inset: ring(skin, 3).map_or(0.0, |r| r.1),
                rt_serial: skin.rt_serial.into(),
                card_bg: opt_rgb(skin.card_bg),
                card_border: opt_rgb(skin.card_border),
                card_text: opt_rgb(skin.card_text),
                bolt_ink: opt_rgb(skin.bolt_ink),
                outline_fill: opt_rgb(skin.outline_fill),
                outline_ink: opt_rgb(skin.outline_ink),
                outline_w: skin.outline_w,
                rt_bg: opt_rgb(skin.rt_bg),
                rt_border: opt_rgb(skin.rt_border),
                rt_text: opt_rgb(skin.rt_text),
                genre_outline_ink: opt_rgb(skin.genre_outline_ink),
                genre_outline_w: skin.genre_outline_w,
            },
            None => EggTheme::default(),
        });
        // AND THE TWO THE WHOLE FACE READS. Every blue graphic takes `Pal.blue` —
        // the pill, the preset selection, the nav chevrons, the tells, the scroll
        // thumb — so the accent turns silver on this one line rather than in a
        // dozen components, and the ground turns with it. The card and plate
        // colours above stay off the palette on purpose: the reference restates
        // only these two globally, leaving the settings panel, the numpad and the
        // nearby list on the ordinary dark tokens.
        pal.set_egg_page_bg(opt_rgb(skin.page_bg));
        pal.set_egg_accent(opt_rgb(skin.accent));
        // THE CALL SIGN CUT IN TWO, for the bolt to stand between. Slint has no
        // substring and this is a decision anyway. The midpoint rounds UP so an
        // odd count leaves the longer half first; `char_indices` rather than a
        // byte split, because a call sign is only ASCII until the day it is not.
        let (ident_a, ident_b) = match themed {
            Some(e) if e.call_sign_bolt => {
                let mid = ident.chars().count().div_ceil(2);
                let cut = ident.char_indices().nth(mid).map_or(ident.len(), |(i, _)| i);
                (ident[..cut].to_string(), ident[cut..].to_string())
            }
            _ => (String::new(), String::new()),
        };
        ui.set_ident_a(ident_a.as_str().into());
        ui.set_ident_b(ident_b.as_str().into());
        ui.set_rds(rds::rds_ok(st));
        ui.set_tp(st.tp);
        ui.set_ta(st.ta);
        // AF IS NEVER DECODED ON THIS PATH. Every 0A group in the drive logs
        // carries COUNT=0, and CarFM only ever filled this from the SDR backend,
        // which Carnyx does not have. Asserting it would be a false statement on
        // the face.
        ui.set_af(false);

        ui.set_stereo_known(s.stereo.is_some());
        ui.set_stereo(s.stereo.unwrap_or(false));
        ui.set_gps_fix(s.location.fix);
        ui.set_in_motion(s.location.in_motion);
        ui.set_tuner_error(!s.tuner.is_available());
        ui.set_audio_active(s.audio);

        // The hero's own art. Dropped LAST and outside the borrow above, because
        // `art_for` writes the decode back into the cache and would otherwise be
        // a second mutable borrow of `state`.
        let base = row
            .as_ref()
            .map(|r| r.callsign_base.clone())
            .or(learned)
            .unwrap_or_default();
        drop(s);
        // No box: the hero takes the full-size rendition, as CarFM's does
        // (`LogoTile.tsx:60` — "`undefined` is the HERO's size").
        let art = self.art_for(&base, None);
        let flags = self.hero_flags(&base, art.is_some());
        ui.set_has_logo(art.is_some());
        // The backing travels WITH the art, so a station with none leaves the
        // property at `light` — the hero draws no plate without a logo anyway,
        // and a stale `plate` left behind would size the next one wrongly for a
        // frame.
        ui.set_logo_plate(art.as_ref().map_or(LogoPlate::Light, |(_, p)| *p));
        ui.set_logo(art.map(|(i, _)| i).unwrap_or_default());
        ui.set_show_call(flags.show_call);
        ui.set_show_freq(flags.show_freq);

        // ── THE STATION POP-UP, HALF TWO: SAY IT ──────────────────────────────
        //
        // Only a change the driver cannot already see is worth a banner, so this
        // is the one place the foreground flag is read.
        //
        // THE CALL SIGN AND THE DIAL ALWAYS, AND THE MARK AS WELL WHEN THERE IS
        // ONE. Sending the logo alone was tried and does not survive contact with
        // the platform: `setLargeIcon` draws into a small square at the banner's
        // right edge, and station logos are landscape wordmarks, so the name ends
        // up a few pixels tall with an empty card beside it. The words are what a
        // driver can read at a glance; the logo identifies the station next to
        // them, and its absence costs nothing.
        // NO LINE FOR THE IN-FRONT CASE, and one was tried. It fires on every
        // ordinary tune the driver watches — twice in a short host session — and
        // it can NEVER appear in the background case, which is the reported
        // fault, so it spent ring space the probe reports need on a sentence
        // about nothing having gone wrong. `panel key … [face]` already carries
        // the same fact on a line that exists anyway.
        if moved && !front {
            let dial = format_mhz(landed);
            // The call sign leads when there is one; the dial is the honest
            // fallback and never an inaccurate "Tuning…", which is the rule the
            // hero lettering follows above.
            let title = if ident.is_empty() { dial.clone() } else { ident.clone() };
            let logo = self.notification_logo(&base);
            let outcome = crate::android::announce_station(&title, &format!("{dial} FM"), &logo);
            // THE LINE THAT SETTLES THE OPEN QUESTION. Whether a backgrounded
            // wheel press retunes at all depends on the Slint event loop pumping
            // while the activity is stopped, which cannot be tested off the unit
            // — so every announcement records that it happened, what it showed,
            // and whether the platform took it. One drive reads it back out of
            // the settings log.
            crate::android::ingest_note(format!(
                "station pop-up: {title} at {dial} ({}) — {outcome}",
                if logo.is_empty() { "no logo" } else { "logo" },
            ));
        }
    }

    /// The file the station pop-up should show for `base`, or empty for none.
    ///
    /// THE SAME PICTURE THE FACE WOULD DRAW, chosen by `path_for_theme` — which
    /// mirrors `read_for_theme`'s pick and stops short of its decode, because
    /// Android decodes the file itself and at a size it chooses.
    ///
    /// ASKED AT THE TILE RUNG rather than the hero's full size. A large icon is
    /// drawn at about 64dp; handing the platform a master a thousand pixels on a
    /// side to resample is work for nothing, and 128dp is the smallest rendition
    /// this store keeps.
    fn notification_logo(&self, base: &str) -> String {
        if base.is_empty() {
            return String::new();
        }
        let store = self.state.borrow().store.clone();
        let dark = self.ui().global::<crate::Pal>().get_dark();
        let scale = self.ui().window().scale_factor();
        crate::logos::assign::path_for_theme(&store, base, Some(TILE_BOX_DP), scale, dark)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn push_presets(&self) {
        let ui = self.ui();
        // Art first, with NO borrow held: `art_for` decodes on a miss and writes
        // the result back, so resolving it inside the map below would be a
        // mutable borrow inside a shared one.
        let bases: Vec<String> = self
            .state
            .borrow()
            .presets
            .iter()
            .map(|p| p.row.as_ref().map_or_else(String::new, |r| r.callsign_base.clone()))
            .collect();
        let art: Vec<Option<(slint::Image, LogoPlate)>> =
            bases.iter().map(|b| self.art_for(b, Some(TILE_BOX_DP))).collect();

        let s = self.state.borrow();
        let rows: Vec<Preset> = s
            .presets
            .iter()
            .zip(art.iter())
            .map(|(p, a)| to_preset(p, a.clone()))
            .collect();
        // Same rule as `push_nearby`: an identical model is still a repeater
        // rebuilt, and the preset band is on screen the whole time.
        let same = s.last_presets.as_ref() == Some(&rows);
        drop(s);
        if !same {
            self.state.borrow_mut().last_presets = Some(rows.clone());
            ui.set_presets(ModelRc::from(Rc::new(VecModel::from(rows))));
        }
        let s = self.state.borrow();

        let n = s.presets.len() as i32;
        if n == 0 {
            ui.set_has_prev(false);
            ui.set_has_next(false);
            return;
        }
        // With no active preset, prev is the last entry and next the first —
        // which is what the peek cards show on an unsaved dial.
        //
        // THE ANCHOR, NOT THE DIAL, so the cards name the stations a press will
        // actually reach. They differ only once the vendor has moved the radio
        // out from under the strip, and in that case a peek drawn off `dial`
        // would promise one station and the wheel would deliver another.
        let active = s.anchor();
        let (prev, next) = if active < 0 {
            (n - 1, 0)
        } else {
            ((active - 1).rem_euclid(n), (active + 1).rem_euclid(n))
        };
        // The peek cards reuse the tiles' art — same plate, same ladder rung, and
        // it is already decoded.
        ui.set_prev_preset(to_preset(&s.presets[prev as usize], art[prev as usize].clone()));
        ui.set_next_preset(to_preset(&s.presets[next as usize], art[next as usize].clone()));
        ui.set_has_prev(true);
        ui.set_has_next(true);
    }

    /// PUBLISHES the meter. It does not advance anything.
    ///
    /// The reception-loss band is settled by `settle_dotted`, once per poll turn,
    /// and this only reads what that left. It used to settle here — and this
    /// function is reached from `push_all`, which runs on every wake from the
    /// tuner queue, so a per-sample hysteresis was being stepped at whatever rate
    /// the radio happened to be talking. See `signal::meter_face`.
    fn push_meter(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let face = signal::meter_face(s.level, s.dotted, !s.audio);

        ui.set_full_pairs(face.full_pairs);
        ui.set_half(face.half);
        ui.set_dot_opacity(face.dot_opacity);
        ui.set_dotted_arcs(face.dotted_arcs);
        ui.set_level_text(face.level_text.into());
    }

    fn push_nearby(&self) {
        let ui = self.ui();
        let view = {
            let s = self.state.borrow();
            let dials: Vec<f32> = s.presets.iter().map(|p| p.mhz).collect();
            s.picker.view(&dials)
        };
        // NOTHING CHANGED, SO NOTHING IS REPLACED. Every `set_*` below hands
        // Slint a new model, and a new model is a repeater rebuilt — a hundred
        // rows torn down and constructed again, per wake, whether or not one
        // character of them differs. The published state is identical either way;
        // this only stops the churn. See `State::last_nearby`.
        //
        // THE VIEW IS STILL BUILT BEFORE THE GUARD, and that is a deliberate
        // decision rather than an oversight. Building it is what makes the
        // comparison possible — 58 rows formatted into strings, the bucket chips
        // and the genre columns — and it is thrown away on most wakes. Skipping
        // that would mean versioning the picker: a counter bumped by a rebuild, a
        // filter tap and a preset edit, compared instead of the view. `pushbench`
        // prices the waste at 0.051ms of `push_all`'s 0.079ms, which is 0.5% of a
        // core at the RDS pump's cadence, and the failure mode of a missed bump is
        // a nearby list that silently stops updating. Not worth that trade.
        if self.state.borrow().last_nearby.as_ref() == Some(&view) {
            return;
        }
        self.state.borrow_mut().last_nearby = Some(view.clone());

        // ALL NINE, OR NONE. They are derived from one state and are only
        // consistent published together — a stale chip list against a fresh row
        // list is exactly what `NearbyState` exists to prevent.
        ui.set_nearby_state(match view.state {
            NearbyState::List => crate::NearbyState::List,
            NearbyState::Loading => crate::NearbyState::Loading,
            NearbyState::NoGps => crate::NearbyState::NoGps,
            NearbyState::NoStations => crate::NearbyState::NoStations,
            NearbyState::NoDatabase => crate::NearbyState::NoDatabase,
        });
        ui.set_nearby_snapshot(view.snapshot.into());
        ui.set_nearby_stations(ModelRc::from(Rc::new(VecModel::from(
            view.stations
                .iter()
                .map(|r| NearbyStation {
                    freq: r.freq.as_str().into(),
                    call: r.call.as_str().into(),
                    service: r.service.as_str().into(),
                    meta: r.meta.as_str().into(),
                    distance: r.distance.as_str().into(),
                    signal_pairs: r.signal_pairs,
                    saved: r.saved,
                })
                .collect::<Vec<_>>(),
        ))));
        ui.set_nearby_bucket_chips(strings(&view.bucket_chips));
        ui.set_nearby_bucket(view.bucket.into());
        ui.set_nearby_genre_columns(ModelRc::from(Rc::new(VecModel::from(
            view.genre_columns
                .iter()
                .map(|c| GenreColumn {
                    top: c.top.as_str().into(),
                    bottom: c.bottom.as_str().into(),
                    has_bottom: c.has_bottom,
                })
                .collect::<Vec<_>>(),
        ))));
        ui.set_nearby_genre(view.genre.into());
        ui.set_nearby_show_bucket_bar(view.show_bucket_bar);
        ui.set_nearby_show_genre_bar(view.show_genre_bar);
    }

    fn push_settings(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let cfg = &s.settings;
        // ONE predicate, asked once. Two of them is how a panel ends up saying
        // "Connected" over a row that says "Not detected".
        let available = s.tuner.is_available();
        let error = !available;
        let nwd_active = available && cfg.selected == settings::Source::Nwd;

        let status = settings::status(error, cfg.selected);
        ui.set_settings_status_title(status.title.into());
        ui.set_settings_status_sub(status.sub.into());
        ui.set_settings_status_glyph(match status.glyph {
            "warn" => TunerGlyph::Warning,
            _ => TunerGlyph::Waves,
        });
        ui.set_settings_status_action(match status.action {
            "retry" => TunerAction::Retry,
            "details" => TunerAction::Details,
            _ => TunerAction::None,
        });

        let open = settings::details_open(cfg.details_open, error, nwd_active);
        ui.set_settings_details_open(open);
        ui.set_settings_details_label(if open { "Hide details" } else { "Details" }.into());
        // THE DETAILS PANEL HAS NOTHING REAL TO SHOW. It describes an RTL-SDR,
        // which is VibeSDR's hardware path and is barred from this tree by the
        // provenance rule, so the list is empty and the panel draws its own
        // emptiness rather than inventing a device.
        //
        // PUBLISHED ONCE, with the theme chips below it. Neither list has an
        // input: this one is always empty and `theme_chips()` takes no argument
        // and returns the same three labels forever. Handing Slint a fresh model
        // of unchanging content on every wake from the tuner queue is a repeater
        // rebuilt for nothing. See `State::statics_published`.
        if !s.statics_published {
            ui.set_settings_tuner_details(ModelRc::from(Rc::new(VecModel::from(
                Vec::<TunerDetail>::new(),
            ))));
            ui.set_settings_theme_chips(strings(&settings::theme_chips()));
        }

        // THE SOURCE LIST, under `last_presets`' rule. Its inputs are whether a
        // tuner is available and which one is selected, so it changes on a bind,
        // a loss and a tap in this panel — never on a frequency report, which is
        // what almost every call here is.
        let sources: Vec<TunerSource> = cfg
            .sources(available)
            .iter()
            .map(|r| TunerSource {
                name: r.name.as_str().into(),
                kind: r.kind.as_str().into(),
                badge: r.badge.as_str().into(),
                badge_lit: r.badge_lit,
                available: r.available,
                selected: r.selected,
            })
            .collect();
        let sources_changed = s.last_sources.as_ref() != Some(&sources);
        if sources_changed {
            ui.set_settings_sources(ModelRc::from(Rc::new(VecModel::from(sources.clone()))));
        }

        ui.set_settings_theme(cfg.theme.label().into());
        ui.set_settings_battery(match cfg.battery {
            settings::Battery::Checking => BatteryState::Checking,
            settings::Battery::Exempt => BatteryState::Exempt,
            settings::Battery::NotExempt => BatteryState::NotExempt,
        });
        ui.set_settings_battery_sub(cfg.battery.sub().into());
        ui.set_settings_logos_on(cfg.logos_on);
        ui.set_settings_logos_sub(settings::logos_sub(cfg.logos_on).into());
        ui.set_settings_clear_logos_label(
            settings::clear_logos_label(cfg.clearing_logos).into(),
        );
        ui.set_settings_clearing_logos(cfg.clearing_logos);

        ui.set_settings_release_on_sleep(cfg.release_on_sleep);
        ui.set_settings_clock_on(cfg.clock_on);
        ui.set_settings_nav_on(cfg.nav_on);
        ui.set_settings_nav_hide_on_map(cfg.nav_hide_on_map);
        // ── THE SUB-LINE SAYS WHY NOTHING IS SHOWING (§4.9) ──────────────────
        //
        // Every reason the maneuver layer can be blank — no OsmAnd, the switch
        // off, OsmAnd closed, OsmAnd open with no route, or the layer hidden
        // behind OsmAnd's own map — looks the same on the face, so this row is
        // the only place they can be told apart. `crate::nav::sub_line` holds
        // the wording and the ordering; this only gathers the facts.
        let route = s.nav.route(crate::session::now_unix());
        ui.set_settings_nav_sub(
            crate::nav::sub_line(&crate::nav::Link {
                package: &s.nav_package,
                on: cfg.nav_on,
                linked: route.is_some(),
                navigating: route.is_some_and(crate::nav::Route::navigating),
                map_visible: route.is_some_and(|r| r.map_visible),
                hide_on_map: cfg.nav_hide_on_map,
            })
            .into(),
        );
        ui.set_settings_nav_hide_sub(crate::nav::hide_sub_line().into());
        ui.set_settings_diag_on(cfg.diag_on);
        // The log is a 200-line ring and this is a 200-string model. Same rule
        // again, and it matters most here: the diagnostics overlay is the one a
        // driver leaves open while watching the radio misbehave. The cache write
        // waits until the borrow is released, at the foot of this function.
        let lines = cfg.log.lines();
        let diag_changed = s.last_diag.as_ref() != Some(&lines);
        if diag_changed {
            ui.set_settings_diag_lines(strings(&lines));
        }
        // THE DIAGNOSTICS BUTTONS, same rule. The list is now fixed — the five
        // rows that came and went with the raw-capture switch and the live source
        // were CarFM's probes and are gone — so this could in principle be built
        // once, but it is cheap and the shape is shared with every other list
        // here.
        let diag_actions: Vec<DiagAction> = cfg
            .actions()
            .iter()
            .map(|a| DiagAction {
                label: a.label.as_str().into(),
                divider_above: a.divider_above,
            })
            .collect();
        let actions_changed = s.last_diag_actions.as_ref() != Some(&diag_actions);
        if actions_changed {
            ui.set_settings_diag_actions(ModelRc::from(Rc::new(VecModel::from(
                diag_actions.clone(),
            ))));
        }

        ui.set_settings_diag_status(s.diag_status.as_str().into());

        // ── THE HIDDEN BAND-THEME PICKER ─────────────────────────────────────
        //
        // The whole list, finished, with the off row already in it — the panel's
        // index is an index into THIS list, and a list the two sides build
        // differently is an off-by-one waiting to happen.
        //
        // LABELS, NEVER IDS. `Egg::menu` is a pun and `Egg::id` is the key the
        // matcher and the face switch on, and the reference keeps them apart in
        // as many words. Nothing that crosses this seam is an id.
        //
        // `listed()` AND NO SECOND FILTER. The owner's rule is that basic themes
        // get no listing, and `eggs::listed` is the one place it is enforced.
        let listed = crate::eggs::listed();
        let mut menu: Vec<slint::SharedString> = Vec::with_capacity(listed.len() + 1);
        menu.push(EGG_MENU_OFF.into());
        menu.extend(listed.iter().map(|e| slint::SharedString::from(e.menu)));
        ui.set_settings_egg_menu(slint::ModelRc::from(std::rc::Rc::new(
            slint::VecModel::from(menu),
        )));
        // ZERO WHEN NOTHING IS FORCED, and zero when the stored id names a row
        // that is no longer listed — the picker cannot show a choice it cannot
        // offer, and lighting nothing while a theme is forced would be worse than
        // lighting the off row, which is at least a way back.
        let choice = s
            .forced_egg
            .as_deref()
            .and_then(|id| listed.iter().position(|e| e.id == id))
            .map(|i| i as i32 + 1)
            .unwrap_or(0);
        ui.set_settings_egg_choice(choice);

        ui.set_settings_about(
            settings::about_line(
                "Carnyx",
                env!("CARGO_PKG_VERSION"),
                s.snapshot.as_deref(),
            )
            .into(),
        );
        // The borrow above must be released before `save_prefs` takes its own —
        // and before the diagnostics cache is written back.
        //
        // AND THE `save_prefs` AT THE END OF THIS FUNCTION IS WHERE EVERY PANEL
        // SETTING PERSISTS. Five fields of `prefs::Prefs` — the source, the
        // theme, the logo switch, the sleep release and the diagnostics switch —
        // are moved by callbacks that write nothing themselves, and they reach
        // the disk because every one of those callbacks ends in `push_settings`.
        // Worth saying out loud: from the callback's side that write is
        // invisible, and the obvious "fix" is to add one there — a second write
        // of the same struct, which the equality check below would discard.
        drop(s);
        {
            let mut s = self.state.borrow_mut();
            if diag_changed {
                s.last_diag = Some(lines);
            }
            if sources_changed {
                s.last_sources = Some(sources);
            }
            if actions_changed {
                s.last_diag_actions = Some(diag_actions);
            }
            s.statics_published = true;
        }
        self.save_prefs();
    }

    /// §5's abandon-entry: CANCEL, and the ✕ or scrim while the frequency tab
    /// is up. Closes the overlay and puts back the frequency the tab opened on.
    ///
    /// THE RESTORE FIRES ON TWO CONDITIONS, not one. A moved dial is the obvious
    /// case — a seek landed somewhere else. The second is a sweep still in
    /// FLIGHT: on the NWD front end a seek is fire-and-forget and `s.dial` does
    /// not move until `notifyCurrentFrequency` lands, so mid-sweep the dial
    /// still EQUALS the restore point — and a dial-equality test alone concluded
    /// there was nothing to do, dropped the restore, and let the sweep land on a
    /// new station after the driver had said no to it. Tuning the restore point
    /// is both halves at once: it is the documented sweep-cancel and it is the
    /// restore.
    fn freq_cancel(self: &Rc<App>) {
        let restore = {
            let mut s = self.state.borrow_mut();
            s.numpad.clear();
            let dial = s.dial;
            let sweeping = s.scanning;
            s.freq_restore.take().filter(|v| *v != dial || sweeping)
        };
        self.ui().set_overlay(Overlay::None);
        match restore {
            Some(v) => self.tune(v),
            None => self.push_freq(),
        }
    }

    fn push_freq(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let typed = !s.numpad.is_empty();
        // The buffer is handed over EXACTLY as typed — "104." is a legitimate
        // display string mid-entry and normalising it on the way in would fight
        // the user's fingers.
        ui.set_freq_display(if typed {
            s.numpad.as_str().into()
        } else {
            SharedString::from(format_mhz(s.shown()))
        });
        // DIM ONLY WHEN THERE IS NOTHING TO SAY. A sweep runs behind this tab and
        // the readout follows it, so a scan is as much a reason to be legible as
        // a typed dial.
        ui.set_freq_display_dim(!typed && !s.scanning);
        ui.set_freq_error(typed && !entry_can_tune(&s.numpad));
        // ONE DECIMAL ON BOTH ENDS, explicitly. `{FM_HI}` renders 108.0 as "108",
        // which is how the old card's line read; §4 writes the copy out as
        // "Outside 87.5–108.0 MHz band" and the band's ends are quoted to a tenth
        // everywhere else on the face. U+2013 EN DASH between them.
        ui.set_freq_error_text(format!("Outside {FM_LO:.1}\u{2013}{FM_HI:.1} MHz band").into());
    }

    /// Publish the clock (§4.8).
    ///
    /// FORMATTED EVERY TIME AND PUBLISHED ONLY ON A CHANGE. §4.8 asks for both:
    /// *"One 1s timer, but state changes only when the minute changes — no
    /// per-second re-render of the face"*, and *"Formatting is derived at render
    /// time, so a format change … shows immediately"*. Deriving costs two
    /// integers and a `format!`; the property write is what would cost a frame,
    /// and that is what the comparison guards.
    ///
    /// EMPTY WHEN THERE IS NO READING, which on every host build is always — the
    /// class that answers lives in the embedded dex. Drawing `00:00` there would
    /// be a real time and a lie; the face draws nothing instead, so no shot
    /// carries a clock that was never read.
    fn push_clock(&self) {
        let on = self.state.borrow().settings.clock_on;
        let now = on.then(crate::android::clock_now).flatten();
        let clock = match now {
            Some((h, m, is24)) => crate::clock::format(h, m, is24),
            None => crate::clock::Clock::default(),
        };
        let ui = self.ui();
        if ui.get_clock_time() != clock.time.as_str() {
            ui.set_clock_time(clock.time.as_str().into());
        }
        if ui.get_clock_meridiem() != clock.meridiem.as_str() {
            ui.set_clock_meridiem(clock.meridiem.as_str().into());
        }
        // THE SUB-LINE FOLLOWS THE SYSTEM, so it has to be republished when the
        // system flips — which this is the only thing that notices.
        let sub = crate::clock::sub_line(now.map(|(_, _, is24)| is24).unwrap_or(false));
        if ui.get_settings_clock_sub() != sub.as_str() {
            ui.set_settings_clock_sub(sub.as_str().into());
        }
    }

    /// Publish the navigation state, and log it when it changes.
    ///
    /// FINISHED STRINGS, as every other surface gets: the Slint side is handed
    /// a distance already formatted and a turn already named, because the panel
    /// decides nothing. The design handoff for the strip is still in progress,
    /// so what is published is deliberately the DATA and not a layout — a glyph
    /// id, a label and the words, which is what any drawing of this will bind
    /// to whatever it ends up looking like.
    /// Bind or unbind OsmAnd. Returns a line for the diagnostics log.
    ///
    /// THE HOST HAS NO OSMAND AND SAYS SO BY SAYING NOTHING. An empty answer is
    /// "there was nothing to do", which is the honest one for every build that
    /// is not the device — and it keeps the log line out of every screenshot.
    /// Which OsmAnd is installed, or `""`. See `CarnyxNav.installedPackage`.
    #[cfg(target_os = "android")]
    fn nav_installed_package(&self) -> String {
        crate::android::nav::installed_package()
    }

    /// The host has no OsmAnd. See the Android arm.
    #[cfg(not(target_os = "android"))]
    fn nav_installed_package(&self) -> String {
        String::new()
    }

    #[cfg(target_os = "android")]
    fn set_nav_running(&self, on: bool) -> String {
        if on {
            crate::android::nav::start()
        } else {
            crate::android::nav::stop()
        }
    }

    /// See the Android arm.
    #[cfg(not(target_os = "android"))]
    fn set_nav_running(&self, _on: bool) -> String {
        String::new()
    }

    fn push_nav(&self) {
        let now = crate::session::now_unix();
        let (state, spoken, line) = {
            let s = self.state.borrow();
            (
                s.nav.state(now),
                s.nav.spoken(now).unwrap_or_default().to_string(),
                s.nav.log_line(now, s.units),
            )
        };
        let ui = self.ui();
        let units = self.state.borrow().units;
        use crate::nav::NavState;
        let (active, turn, distance) = match state {
            NavState::Idle => (false, String::new(), String::new()),
            NavState::Waiting => (true, String::new(), String::new()),
            NavState::OffRoute { metres } => (
                true,
                "off route".to_string(),
                crate::nav::Nav::distance_label(metres, units),
            ),
            NavState::Turn { turn, metres } => (
                true,
                turn.name().to_string(),
                crate::nav::Nav::distance_label(metres, units),
            ),
            NavState::Unknown { metres, .. } => {
                (true, String::new(), crate::nav::Nav::distance_label(metres, units))
            }
        };
        ui.set_nav_active(active);
        ui.set_nav_turn(turn.into());
        ui.set_nav_distance(distance.into());
        ui.set_nav_instruction(spoken.into());

        // ── THE POLL'S HALF, which is everything with words in it ────────────
        //
        // EMPTY MEANS COLLAPSE, which is the handoff's own rule: "Treat every
        // field as optional — a missing one collapses its element; it never
        // leaves a gap or a placeholder." So a missing street is `""` and the
        // strip draws nothing there, rather than a dash or a spinner.
        let route = self.state.borrow().nav.route(now).cloned();

        // ── IS THE LINK UP? (§4.9's status-bar tell) ─────────────────────────
        //
        // A FRESH POLL IS THE ANSWER, AND IT IS A BETTER ONE THAN THE BIND.
        // `bindService` returning true means the request was ACCEPTED, not that
        // anything is on the other end: `onServiceConnected` may not have run
        // yet, and a binder that has since died still leaves the bind looking
        // good until Android gets round to saying otherwise.
        //
        // `CarnyxNav.pollOnce` calls `nativeNavInfo` on every answered
        // `getAppInfo`, and OsmAnd answers that whether or not it is navigating
        // — so a poll inside `Nav::EXPIRY` means the service is bound AND
        // talking, which is what a lit tell should claim. An idle OsmAnd still
        // lights it; one that has gone away goes dim within twelve seconds, on
        // the same clock that ages the turn.
        ui.set_nav_linked(route.is_some());

        let r = route.unwrap_or_default();
        ui.set_nav_street(r.street.clone().unwrap_or_default().into());
        ui.set_nav_after_street(r.after_street.clone().unwrap_or_default().into());
        // THE POLL'S OWN TURN, as a TurnType XML string — "TR", "TSLL", "RNDB".
        // NOT the integer the push sends: the arrow generator the handoff
        // specifies reads the XML string, and the two encodings of the same turn
        // are exactly the kind of thing that gets crossed.
        ui.set_nav_turn_xml(r.turn_xml.clone().unwrap_or_default().into());
        ui.set_nav_after_turn_xml(r.after_turn_xml.clone().unwrap_or_default().into());
        // OSMAND'S MAP IS IN FRONT — the driver is already looking at the turn.
        ui.set_nav_map_visible(r.map_visible);

        // ── THE ONE GATE THE MANEUVER LAYER BINDS TO (§4.9) ──────────────────
        //
        // "`AppInfoParams.mapVisible` reports whether OsmAnd's own map is on
        // screen; while it is, the maneuver layer is hidden — the driver is
        // already looking at the turn. Driver-overridable (Settings ▸
        // NAVIGATION ▸ Hide when the map is showing, default on)."
        //
        // SEPARATE FROM `nav-active`, WHICH STAYS RAW. Three surfaces ask
        // different questions of the same state: the maneuver layer asks "may I
        // draw", the settings sub-line asks "why am I not drawing", and the
        // diagnostics log asks "what arrived". Folding the suppression into
        // `nav-active` would answer the first by lying to the other two.
        let suppressed = r.map_visible && self.state.borrow().settings.nav_hide_on_map;
        ui.set_nav_showing(active && !suppressed);

        // ON A CHANGE ONLY. See `State::nav_said`.
        let line = line.unwrap_or_default();
        let changed = self.state.borrow().nav_said != line;
        if changed {
            let at = stamp();
            let mut s = self.state.borrow_mut();
            s.nav_said = line.clone();
            if !line.is_empty() {
                s.settings.log.push(&at, &format!("nav: {line}"));
            }
        }
    }

    fn push_logo_search(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let view = s.logo.view();
        let brand = brand_color(&s.logo.target().map(|t| t.base.clone()).unwrap_or_default());
        crate::logos::ui::apply(&ui, &view, s.logo.cells(), s.logo_art.as_ref(), brand);

        // ── THE DARK PICKER, WHICH OVERRIDES THE VIEW ABOVE ──────────────────
        //
        // Written after `apply` and not folded into it, because `search::View`
        // is the SEARCH's model and this is a step that happens after a search
        // has already finished. Folding it in would put a second question's
        // state inside the answer to the first, and `close_logo_search` would
        // have to know about both.
        //
        // The five properties it takes over are the ones a state owns: the body,
        // the selection, the two button labels and whether Confirm is live.
        let Some(d) = s.dark_pick.as_ref() else {
            // EVERY PROPERTY THE PICKER TOOK OVER GOES BACK, and the left button
            // is the one that would otherwise stay wrong: `apply` above writes
            // the state, the selection, the hint and the confirm label on every
            // push, but nothing else writes `cancel-label` — so a window reopened
            // after a picker would have said "Skip" over the results grid.
            ui.set_logo_search_dark_choices(Default::default());
            ui.set_logo_search_cancel_label("Cancel".into());
            return;
        };
        let rows: Vec<crate::DarkChoice> = d
            .items
            .iter()
            .map(|(t, art)| crate::logos::ui::to_dark_choice(*t, art, *t == d.pick))
            .collect();
        crate::logos::ui::set_dark_choices(&ui, &rows);
        ui.set_logo_search_state(crate::LogoSearchState::DarkPick);
        ui.set_logo_search_selected_index(d.selected as i32);
        // SKIP, NOT CANCEL. The logo is already saved by the time this opens and
        // the auto-pick already stands; what the left button declines is the
        // question, not the save. `LogoDarkPicker.tsx` words it the same way.
        ui.set_logo_search_cancel_label("Skip".into());
        ui.set_logo_search_confirm_label("Use this".into());
        // NOTHING TO CONFIRM WHILE THE ADAPTATION RUNS, which is also the frame
        // where `selected` points at a row that does not exist yet.
        ui.set_logo_search_can_confirm(!d.items.is_empty());
        ui.set_logo_search_hint(if d.items.is_empty() {
            Default::default()
        } else {
            "Shown on the real dark background".into()
        });
    }

    // ── Stored art ───────────────────────────────────────────────────────────

    /// Read one station's stored rendition off disk and decode it.
    ///
    /// `None` covers every ordinary state and they are NOT distinguished here,
    /// because no surface can do anything different with them: no codec (the
    /// host), no logo for this station, an unreadable file, a decode that
    /// failed. Each one means the same thing to the face — draw the call-sign
    /// box.
    fn read_art(
        &self,
        base: &str,
        box_dp: Option<f32>,
        dark: bool,
    ) -> Option<(Raster, crate::logos::assign::Backing)> {
        // Both handles cloned out BEFORE the read: this decodes a PNG through
        // JNI, and holding a `RefCell` borrow across that is a lock held across
        // a call into another runtime.
        let (store, codec) = {
            let s = self.state.borrow();
            (s.store.clone(), s.codec.clone()?)
        };
        let scale = self.ui().window().scale_factor();
        crate::logos::assign::read_for_theme(&store, &*codec, base, box_dp, scale, dark)
    }

    /// The image a surface should draw for `base`, or `None` for the call-sign
    /// box.
    ///
    /// `box_dp` picks the ladder rung — 128 for a preset chip, `None` for the
    /// hero, which is CarFM's own split (`LogoTile.tsx:337`, `boxDp = fill ? 128
    /// : …`, and the hero calls `useStationLogo` with no box at all).
    ///
    /// `settings.logos_on` IS DELIBERATELY NOT CONSULTED, and gating on it was a
    /// bug that shipped: a driver could search a logo, save it, and never see it
    /// on the face. That switch is CarFM's `@carfm/logos_enabled_v1` and it
    /// governs AUTO-DOWNLOAD, which is what its own subtitle says — "Auto-download
    /// station artwork over Wi-Fi" against "Off — assign logos manually from a
    /// station". CarFM's `useStationLogo` never reads it. A hand-assigned logo is
    /// something the driver chose on purpose; a preference about background
    /// fetching has no business hiding it.
    fn art_for(&self, base: &str, box_dp: Option<f32>) -> Option<(slint::Image, LogoPlate)> {
        if base.is_empty() {
            return None;
        }
        // THE THEME IS READ ONCE PER LOOKUP, from the palette rather than from
        // `settings.theme`: `Theme::System` sets neither, so the setting can say
        // one thing while the face shows another, and what a logo has to match is
        // the face.
        let dark = self.ui().global::<crate::Pal>().get_dark();
        let key = (base.to_uppercase(), box_dp.unwrap_or(0.0).round() as u32, dark);
        // Cloned out in its own statement for the reason spelt out in
        // `drain_events`: an `if let` scrutinee holds its borrow across the body
        // on edition 2021. This one is only a shared borrow and the read below
        // is too, so it never paniced — but it is one edit away from doing so.
        let hit = self.state.borrow().art.get(&key).cloned();
        if let Some(hit) = hit {
            return hit;
        }
        let read = self.read_art(&key.0, box_dp, dark);
        let art = read.map(|(r, backing)| (crate::logos::ui::to_image(&r), plate_of(backing)));
        self.state.borrow_mut().art.insert(key.clone(), art.clone());
        // ASKED ON EVERY DARK READ THAT FOUND ART, not only on the white
        // fallback. The on-demand half of CarFM's `useDarkLogo` covers a station
        // with no variant at all — every logo saved before the dark read existed,
        // since `assign_from_urls` adapts at import and nothing has ever gone
        // back for the rest. But a variant can also be PRESENT AND STALE: handoff
        // v1.16.0 moved the dark surface the gate judges against, and a pick made
        // against the old one draws as `Bare` or `Plate` rather than as a
        // fallback, so gating on the fallback would never notice.
        //
        // `request_dark_adaptation` is the one that decides; this only asks.
        if dark && art.is_some() {
            self.request_dark_adaptation(&key.0);
        }
        art
    }

    /// Queue one station's dark adaptation IF IT NEEDS ONE, at most once per run.
    ///
    /// GUARDED BECAUSE THE TRIGGER REPEATS. `art_for` asks on every dark
    /// republish, and a station whose master will not decode never stops
    /// answering "no variant" — so without the guard a dark face would queue
    /// seconds of pixel work per tile per tune, forever. CarFM's `regenTried`
    /// set, for the same reason.
    ///
    /// TWO THINGS COUNT AS NEEDING ONE, and `wants_dark_adaptation` knows both:
    /// no cached variant, and a variant whose gate background is not the one the
    /// face composites onto now.
    ///
    /// The set is never pruned: it is bounded by the number of stations that have
    /// logos, and a driver who assigns a new one gets a fresh adaptation from
    /// `assign_from_urls` at import rather than from here.
    fn request_dark_adaptation(&self, base: &str) {
        {
            let mut s = self.state.borrow_mut();
            if s.worker.is_none() || !s.adapt_tried.insert(base.to_string()) {
                return;
            }
            // The store is asked LAST, so a station with no logo at all cannot
            // reach the worker — `wants_dark_adaptation` is a meta lookup, but
            // queueing on it without the guard above would still repeat.
            if !crate::logos::assign::wants_dark_adaptation(
                &s.store,
                base,
                crate::logos::dark::LOGO_DARK_BG,
            ) {
                return;
            }
            s.settings.log.push(&stamp(), &format!("adapting {base} for dark"));
        }
        let s = self.state.borrow();
        if let Some(w) = s.worker.as_ref() {
            w.adapt(base);
        }
    }

    /// The hero flags a station actually gets, once it is known whether it has
    /// art. `logos::prefs::effective` is the whole rule; this only supplies it
    /// with the stored answer.
    fn hero_flags(&self, base: &str, has_logo: bool) -> HeroFlags {
        let stored = if base.is_empty() {
            None
        } else {
            self.state.borrow().store.prefs(&base.to_uppercase())
        };
        crate::logos::prefs::effective(stored, has_logo)
    }

    // ── Commands ─────────────────────────────────────────────────────────────

    /// Tune, and re-resolve everything that hangs off the dial.
    fn tune(self: &Rc<App>, mhz: f32) {
        {
            let s = self.state.borrow();
            // The refusal is REPORTED, not swallowed: an uncalibrated unit
            // cannot convert MHz to its own raw scale, and a silent no-op there
            // is CarFM's "the dial spins and the radio does not move".
            if let Err(e) = s.tuner.tune(mhz) {
                drop(s);
                let mut s = self.state.borrow_mut();
                s.settings.log.push(&stamp(), &format!("tune refused: {e}"));
                // AND NOTHING ELSE MOVES. The lines below set the dial, drop the
                // level and republish, and they used to run whether or not the
                // command was accepted — so an uncalibrated unit put a frequency
                // on the hero that the front end was not on. That is exactly the
                // failure the comment above names: the dial spins and the radio
                // does not move.
                return;
            }
        }
        // MARKED BUSY ACROSS THE DRAIN, so the nested drain below cannot take a
        // queued wheel key and run a second step whose target this function then
        // overwrites. The key stays pending and `drain_events`' loop applies it
        // once this tune's writes have landed.
        //
        // AND THE ANCHOR IS WRITTEN BEFORE THE DRAIN, unlike the dial. A key
        // arriving mid-tune has its strip position read the moment it lands, and
        // reading it from a stale `asserted` anchored the step on where the
        // driver was BEFORE this tune — so two presses advanced one station and
        // the second was a no-op under the `moves` gate. `dial` still comes
        // after, because the drain exists to flush stale frequency reports
        // before this function asserts the dial it just commanded.
        self.state.borrow_mut().busy = true;
        {
            let mut s = self.state.borrow_mut();
            s.asserted = Some(mhz);
            // A HOLD BELONGS TO THE STEP THAT TOOK IT, and this is where one
            // that has been overtaken is dropped. `step_preset_from` commits the
            // face and then calls this with the SAME frequency, so its own hold
            // survives; every other tune — a preset tile, the nearby list, the
            // numpad, a re-assert for a target that has since changed — is the
            // driver going somewhere else, and leaving the old hold standing
            // would render the abandoned station over the new one and then
            // re-command the radio back to it.
            if s.hold.is_some_and(|h| {
                f64::from((h.mhz - mhz).abs()) >= crate::stations::FREQ_EPS
            }) {
                s.hold = None;
                s.reassert = None;
            }
        }
        self.drain_events();
        {
            let mut s = self.state.borrow_mut();
            s.busy = false;
            s.dial = mhz;
            // `asserted` — where the app put the strip — was written ABOVE, before
            // the drain, for the reason given there. It is the half of the wheel
            // fix that survives BETWEEN drains, where capturing the anchor as the
            // key arrives cannot help, because by the next press the vendor has
            // already moved `dial` on its own hop.
            //
            // A retune is the documented way to cancel a hardware sweep (see
            // `apply_panel_action`), so whatever sweep was in flight is not any
            // more — and on a front end that answers with no further frequency
            // report, nothing else would ever clear the flag.
            s.scanning = false;
            // A retune is a new station: the level from the old one is not a
            // reading of this one.
            s.level = None;
            s.dotted = 0;
        }
        self.pump_rds_until_settled();
        // NO IMMEDIATE READ HERE, and there used to be one.
        //
        // It is the reading `signal`'s own measurements say to throw away: +17.7
        // on average against the same station twenty seconds later, with cases of
        // +45, +48 and +57. The schedule armed by the frequency report that
        // follows this tune reads at 1s and corrects at 4s instead, which is
        // CarFM's answer and the reason those constants were ported.
        self.push_all();
    }

    fn install_callbacks(self: &Rc<App>, ui: &AppWindow) {
        macro_rules! on {
            ($setter:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = Rc::downgrade(self);
                ui.$setter(move |$($arg),*| {
                    if let Some($app) = weak.upgrade() {
                        $body
                    }
                });
            }};
        }

        // ── The face ──
        on!(on_select_preset, |app, i| {
            let mhz = app.state.borrow().presets.get(i as usize).map(|p| p.mhz);
            if let Some(mhz) = mhz {
                app.tune(mhz);
            }
        });
        on!(on_step_preset, |app, dir| {
            app.step_preset(dir);
        });
        on!(on_toggle_save, |app| {
            app.toggle_save();
        });
        on!(on_claim_audio, |app| {
            app.set_audio(true);
        });
        on!(on_release_audio, |app| {
            app.set_audio(false);
        });
        on!(on_done_reordering, |app| {
            app.ui().set_reordering(false);
        });
        on!(on_tick, |app| {
            app.tick();
        });
        on!(on_enter_reordering, |app| {
            app.enter_reordering();
        });
        on!(on_reorder_preset, |app, from, to| {
            app.reorder_preset(from, to);
        });
        on!(on_morph_note, |app, m| {
            app.log_unavailable(&format!("step morph: {m}"));
        });
        // COUNTING ONLY, and nothing else is safe here: this runs while Slint is
        // evaluating the animation, so touching a UI property or republishing
        // from inside it would re-enter the very evaluation that called it.
        on!(on_morph_frame, |app| {
            app.state.borrow_mut().morph_frames += 1;
        });
        on!(on_drag_note, |app, m| {
            // Straight to the log the driver can read, because this gesture
            // cannot be observed any other way on the unit.
            app.log_unavailable(&format!("reorder: {m}"));
        });
        on!(on_open_settings, |app| {
            app.push_settings();
        });
        on!(on_open_nearby, |app| {
            // §2: the nearby button opens the overlay on NEARBY STATIONS, always.
            // The tab is not remembered between visits — a driver who left on the
            // keypad last time is not asking for it this time, and the button they
            // pressed says which half they want.
            {
                let mut s = app.state.borrow_mut();
                s.numpad.clear();
                s.freq_restore = None;
            }
            app.ui().set_nearby_tab(NearbyTab::Nearby);
            app.push_freq();
            app.refresh_nearby();
        });
        on!(on_close_overlay, |app| {
            {
                let mut s = app.state.borrow_mut();
                s.numpad.clear();
                s.freq_restore = None;
            }
            app.push_freq();
            app.close_logo_search();
        });

        // ── §6.2's frequency tab ──
        on!(on_set_nearby_tab, |app, t| {
            let t: NearbyTab = t;
            // A TAP ON THE TAB THAT IS ALREADY UP IS NOT A SWITCH. TabButton
            // fires unconditionally — `active` is display-only — so without this
            // gate a stray tap on the active "Enter frequency" tab wiped a
            // half-typed dial and, worse, re-based CANCEL's restore point to
            // wherever a seek had just swept to.
            if t == app.ui().get_nearby_tab() {
                return;
            }
            let sweeping = {
                let mut s = app.state.borrow_mut();
                // Either way the buffer goes (§5). Switching away abandons a
                // half-typed dial; switching in starts clean.
                s.numpad.clear();
                // §5: switching TO the frequency tab snapshots the restore point.
                // Switching away drops it, so a later CANCEL cannot resurrect a
                // frequency from a visit two taps ago.
                s.freq_restore = (t == NearbyTab::Freq).then_some(s.dial);
                s.scanning
            };
            // §5: switching to Nearby stops any running seek. There is no cancel
            // on the tuner trait — `Tuner::seek` is handed over and let go — so
            // the only thing that stops a sweep is a tune. Mid-sweep `s.dial` is
            // still where the sweep STARTED (the landing has not reported), so
            // this puts the driver back exactly there.
            if t == NearbyTab::Nearby && sweeping {
                let dial = app.state.borrow().dial;
                app.tune(dial);
            }
            app.ui().set_nearby_tab(t);
            app.push_freq();
        });
        on!(on_freq_key, |app, c| {
            {
                let mut s = app.state.borrow_mut();
                let c: SharedString = c;
                s.numpad = numpad_press(&s.numpad, c.as_str());
            }
            app.push_freq();
        });
        on!(on_freq_back, |app| {
            app.state.borrow_mut().numpad.pop();
            app.push_freq();
        });
        on!(on_freq_commit, |app| {
            // §5: "TUNE commits the buffer and closes the overlay. An empty/invalid
            // buffer closes without retuning." Both halves are here, so the whole
            // rule is in one place and a probe can drive it.
            //
            // A REFUSAL IS NO LONGER A STATE. The old card kept itself up with the
            // buffer intact and lit an error line; there is no card to keep up now,
            // and §5 replaced that with the live warning `entry_can_tune` drives.
            let dial = numpad_commit(&app.state.borrow().numpad);
            {
                let mut s = app.state.borrow_mut();
                s.numpad.clear();
                s.freq_restore = None;
            }
            app.ui().set_overlay(Overlay::None);
            match dial {
                Some(v) => app.tune(v),
                None => app.push_freq(),
            }
        });
        on!(on_freq_cancel, |app| {
            app.freq_cancel();
        });
        on!(on_nearby_dismiss, |app| {
            // THE ✕ AND THE SCRIM, from either tab. §5 splits the two meanings —
            // "just closes" on the station list, "abandons entry and restores"
            // on the keypad — and the branch lives HERE rather than in the view
            // for the same reason TUNE's close does: a rule written into the
            // overlay instance only runs when the tap goes through that
            // instance, which is unreachable from every probe.
            if app.ui().get_nearby_tab() == NearbyTab::Freq {
                app.freq_cancel();
            } else {
                {
                    let mut s = app.state.borrow_mut();
                    s.numpad.clear();
                    s.freq_restore = None;
                }
                app.ui().set_overlay(Overlay::None);
                app.push_freq();
            }
        });
        on!(on_freq_seek, |app, dir| {
            {
                let mut s = app.state.borrow_mut();
                // THE BUFFER GOES FIRST. The readout shows the buffer whenever
                // there is one, so a half-typed dial left standing would sit
                // there through the whole sweep instead of following it to the
                // station it finds.
                s.numpad.clear();
                // The sweep is in flight from here until a frequency report
                // lands. On the fakes that is the very next drain; on the NWD
                // front end it is however long the search takes, and this flag
                // is what CANCEL and the tab switch read to know a tune is still
                // owed.
                s.scanning = true;
                // AND THE STRIP IS LEFT DELIBERATELY, exactly as `PanelAction::Seek`
                // does. That arm had this line and this one did not, which put the
                // rule on the DEAD path only: `PanelKey` records that this fascia
                // has no seek button, so `on_freq_seek` is the only seek there is.
                // Without it a stale `asserted` survived the sweep and the next
                // ch+ stepped from where the driver had been BEFORE seeking —
                // where reading `dial` used to give -1 and land on entry 0. A
                // regression introduced with the anchor itself.
                s.asserted = None;
                // And the committed face, for the reason `PanelAction::Seek`
                // gives: a live hold's re-assert would retune mid-sweep and
                // cancel it.
                s.hold = None;
                s.reassert = None;
            }
            // Handed over and let go — see `PanelAction::Seek`. Re-tuning after a
            // hardware seek cancels it.
            app.state.borrow().tuner.seek(dir > 0);
            app.drain_events();
            app.pump_rds_until_settled();
            app.push_all();
        });

        // ── §6.2 nearby ──
        on!(on_nearby_tune, |app, i| {
            // Through `station_at`, never by indexing a cached list: the int
            // indexes the DISPLAYED list, which the filter changes.
            let mhz = app
                .state
                .borrow()
                .picker
                .station_at(i)
                .map(|r| r.frequency_mhz as f32);
            if let Some(mhz) = mhz {
                app.ui().set_overlay(Overlay::None);
                app.tune(mhz);
            }
        });
        on!(on_nearby_save_preset, |app, i| {
            app.save_nearby_preset(i);
        });
        on!(on_nearby_pick_bucket, |app, b| {
            let b: SharedString = b;
            app.state.borrow_mut().picker.pick_bucket(b.as_str());
            app.push_nearby();
        });
        on!(on_nearby_pick_genre, |app, g| {
            let g: SharedString = g;
            app.state.borrow_mut().picker.pick_genre(g.as_str());
            app.push_nearby();
        });
        on!(on_nearby_reset_bucket, |app| {
            app.state.borrow_mut().picker.reset_bucket();
            app.push_nearby();
        });

        // ── §6.3 settings ──
        on!(on_settings_retry_tuner, |app| {
            app.connect_tuner();
            app.push_all();
        });
        on!(on_settings_toggle_details, |app| {
            let open = app.state.borrow().settings.details_open;
            app.state.borrow_mut().settings.details_open = !open;
            app.push_settings();
        });
        // NONE OF THE HANDLERS BELOW CALLS `save_prefs`, AND THAT IS NOT AN
        // OMISSION — `push_settings` ends with it, and every one of them ends
        // with `push_settings`. Stated here because the absence reads as a bug
        // from this side: five of these switches are fields of `prefs::Prefs`
        // and are read back at launch, so a reader looking for where they are
        // written finds nothing in the handler that moves them. An explicit call
        // here would be a second write of the same struct, which `save_prefs`
        // would discard anyway on its equality check.
        on!(on_settings_pick_source, |app, i| {
            if let Some(&src) = settings::Source::ORDER.get(i as usize) {
                app.state.borrow_mut().settings.selected = src;
            }
            app.push_settings();
        });
        on!(on_settings_set_theme, |app, t| {
            let t: SharedString = t;
            if let Some(theme) = settings::Theme::parse(t.as_str()) {
                app.state.borrow_mut().settings.theme = theme;
                app.apply_theme(theme);
                // TWO THINGS DO NOT DERIVE THEMSELVES FROM `Pal`, and both are
                // republished here. A band theme states its palette per scheme,
                // so the cut has to be re-resolved when the scheme moves under
                // it — switch to dark with AC/DC playing and "Back in Black"
                // only arrives on this line. And a logo is a different FILE per
                // theme: the dark face reads `k-*`/`dark.png` where the light
                // one reads `d-*`/`display.png`, and no property binding can
                // reach across that. Every other token turns on its own.
                app.push_hero();
                app.push_presets();
            }
            app.push_settings();
        });
        on!(on_settings_fix_battery, |app| {
            // THE FRAMEWORK EDGE. The real row opens
            // ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, which needs an
            // Activity. There is none here, so it says so in the log rather than
            // flipping a flag it cannot honour.
            app.log_unavailable("battery exemption needs the Android settings activity");
        });
        on!(on_settings_force_egg, |app, i| {
            // THE INDEX COMES BACK, AND THE ID IS LOOKED UP HERE. The panel was
            // handed labels; turning one back into a theme is this side's job,
            // and `listed()` is asked again rather than cached so the mapping
            // cannot drift from the one that built the list.
            //
            // ROW 0 IS OFF. Anything past the end is also off — a stale index
            // from a list that has since shrunk should stop forcing rather than
            // force whatever now sits at that position.
            let listed = crate::eggs::listed();
            let forced = usize::try_from(i)
                .ok()
                .filter(|i| *i > 0)
                .and_then(|i| listed.get(i - 1))
                .map(|e| e.id.to_string());
            app.state.borrow_mut().forced_egg = forced;
            // THE FACE, THEN THE PANEL. `push_hero` is what resolves the theme
            // and republishes the palette, the faces and the marks; without it
            // the tick moves and nothing else does, which is precisely the
            // half-built control this picker was removed for being once already.
            app.push_hero();
            app.push_settings();
        });
        on!(on_settings_set_logos, |app, v| {
            // AUTO-DOWNLOAD, not "show logos" — see `art_for`. Nothing on the
            // face changes when this moves, and nothing downloads either:
            // `AUTO_LOGO_RESOLUTION` is false and has no caller.
            app.state.borrow_mut().settings.logos_on = v;
            app.push_settings();
        });
        on!(on_settings_clear_logos, |app| {
            // DESTRUCTIVE, AND THE CONFIRM DOES NOT EXIST. SettingsPanel.tsx
            // puts a two-button alert in front of this; wiping every assigned
            // logo, every dark variant and every per-station hero choice on ONE
            // tap with no undo is not the shipping behaviour, so until
            // ui/confirm.slint exists this must not do the work.
            app.log_unavailable("clear all logos needs the confirm dialog first");
        });
        on!(on_settings_set_clock_on, |app, v| {
            app.state.borrow_mut().settings.clock_on = v;
            // THE READOUT GOES AT ONCE, not on the next tick: a switch whose
            // effect waits up to a second reads as a switch that did not work.
            app.push_clock();
            app.push_settings();
        });
        on!(on_settings_set_nav_on, |app, v| {
            app.state.borrow_mut().settings.nav_on = v;
            // THE BIND FOLLOWS THE SWITCH IMMEDIATELY, both ways. Off has to
            // unsubscribe rather than just stop reading: OsmAnd holds our
            // callback and would go on paying for a transaction per location fix
            // for a face that is no longer drawing it.
            let outcome = app.set_nav_running(v);
            {
                let at = stamp();
                let mut s = app.state.borrow_mut();
                if !v {
                    // The state goes with the switch. Without this the last turn
                    // would sit on the face until `Nav::EXPIRY` cleared it,
                    // twelve seconds after the driver switched it off.
                    s.nav.clear();
                    s.nav_said.clear();
                }
                if !outcome.is_empty() {
                    s.settings.log.push(&at, &format!("nav: {outcome}"));
                }
            }
            app.push_nav();
            app.push_settings();
        });
        on!(on_settings_set_nav_hide_on_map, |app, v| {
            app.state.borrow_mut().settings.nav_hide_on_map = v;
            // NOTHING TO BIND OR UNBIND. This is a DISPLAY rule and not a link
            // rule: the poll keeps answering either way, and the face decides
            // whether to draw what it says. `push_nav` republishes so the
            // maneuver layer appears or goes the moment the switch moves rather
            // than at the next OsmAnd update.
            app.push_nav();
            app.push_settings();
        });
        on!(on_settings_set_release_on_sleep, |app, v| {
            app.state.borrow_mut().settings.release_on_sleep = v;
            // DOWN TO THE RECEIVER TOO. It cannot read `Settings` from a binder
            // thread, so a switch that only moved here would be a switch the
            // sleep path never honoured. Taken outside the borrow above.
            app.state.borrow().tuner.set_release_on_sleep(v);
            app.push_settings();
        });
        on!(on_settings_set_diag, |app, v| {
            app.state.borrow_mut().settings.set_diag(v);
            app.push_settings();
        });
        on!(on_settings_pick_diag_action, |app, i| {
            app.run_diag_action(i);
        });
        on!(on_open_logo_search, |app, i| {
            app.open_logo_search(i);
        });
        on!(on_logo_search_toggle_call, |app| {
            app.state.borrow_mut().logo.toggle_call();
            app.push_logo_search();
        });
        on!(on_logo_search_toggle_freq, |app| {
            app.state.borrow_mut().logo.toggle_freq();
            app.push_logo_search();
        });
        on!(on_logo_search_search, |app| {
            app.run_logo_search();
        });
        on!(on_logo_search_retry, |app| {
            app.run_logo_search();
        });
        on!(on_logo_search_pick, |app, i| {
            // THE PICKER OWNS THE SELECTION WHILE IT IS UP. Both bodies report a
            // tap through this one callback — the results grid and the dark
            // swatches — and the index means a different list in each, so the
            // branch is here rather than in two callbacks the panel would have
            // to choose between.
            {
                let mut s = app.state.borrow_mut();
                if let Some(d) = s.dark_pick.as_mut() {
                    if let Ok(i) = usize::try_from(i) {
                        if i < d.items.len() {
                            d.selected = i;
                        }
                    }
                    drop(s);
                    app.push_logo_search();
                    return;
                }
                s.logo.pick(i);
            }
            app.push_logo_search();
        });
        on!(on_logo_search_confirm, |app| {
            // "USE THIS" WHILE THE PICKER IS UP, and it is a different verb from
            // Confirm: the logo is already saved, so this stores WHICH RENDITION
            // of it the dark face draws, with `chosen = true` so every later
            // regeneration honours it.
            let chosen = {
                let s = app.state.borrow();
                s.dark_pick.as_ref().and_then(|d| {
                    d.items.get(d.selected).map(|(t, _)| (d.base.clone(), *t))
                })
            };
            if let Some((base, treatment)) = chosen {
                if let Some(w) = app.state.borrow().worker.as_ref() {
                    w.set_dark(&base, treatment);
                }
                app.state.borrow_mut().settings.log.push(
                    &stamp(),
                    &format!(
                        "dark logo: {} set to {}",
                        base.to_uppercase(),
                        crate::logos::ui::treatment_label(treatment)
                    ),
                );
                app.close_dark_pick();
                return;
            }
            app.confirm_logo();
        });
    }

    // ── The command bodies ───────────────────────────────────────────────────

    /// Tune to `mhz` and save it, for the screenshot harness only.
    ///
    /// Goes through `tune` and `toggle_save`, so a shot built with it exercises
    /// the shipping save path rather than writing the model directly — which is
    /// the difference between proving the strip has no limit and proving Slint
    /// can draw a long list.
    pub fn save_dial_for_test(self: &Rc<App>, mhz: f32) {
        self.tune(mhz);
        self.toggle_save();
    }

    /// Tune the way the DRIVER does, for probes that need a starting position.
    ///
    /// `examples/wheelprobe.rs` needs to put the strip somewhere before it
    /// presses a button, and it must not do that by faking a frequency report:
    /// a report is the RADIO saying where it went, and since the wheel fix that
    /// deliberately no longer moves the app's idea of where the strip is. Using
    /// one to set up a test would prove nothing about a driver who tuned.
    pub fn tune_for_test(self: &Rc<App>, mhz: f32) {
        self.tune(mhz);
    }

    /// Hand the app one logo-worker event, for the screenshot harness.
    ///
    /// THE SEAM THE WORKER SPEAKS THROUGH, and the only way a shot can reach the
    /// dark picker: that screen is opened by a save the host cannot perform —
    /// there is no image codec here to decode a master with, so the pipeline it
    /// would run has nothing to run on. Driving the seam renders the real screen
    /// from real state; faking the properties would render a picture of one.
    pub fn push_logo_event_for_test(self: &Rc<App>, event: service::Event) {
        self.apply_logo_event(event);
    }

    /// Set the clock to a fixed reading, for tests and screenshots.
    ///
    /// THROUGH `crate::clock::format`, not by writing the properties: the shot
    /// is evidence about what the face draws for 8:05, and one that set the
    /// strings itself would be evidence about the shot. The host has no platform
    /// clock — `android::clock_now` answers `None` there, which is why no
    /// ordinary render carries a time.
    pub fn set_clock_for_test(self: &Rc<App>, hour24: u32, minute: u32, is_24h: bool) {
        let c = crate::clock::format(hour24, minute, is_24h);
        let ui = self.ui();
        ui.set_clock_time(c.time.as_str().into());
        ui.set_clock_meridiem(c.meridiem.as_str().into());
        ui.set_settings_clock_sub(crate::clock::sub_line(is_24h).as_str().into());
    }

    /// Put a track in the RadioText, for tests and shots of the band themes.
    ///
    /// Straight into the decoded state rather than through the RDS pump: a theme
    /// resolves off the string the face is SHOWN, and synthesising group 2A
    /// blocks to spell out a band name would be testing the decoder rather than
    /// the matcher. `examples/shot.rs` and the egg tests both want the string.
    pub fn set_radio_text_for_test(self: &Rc<App>, rt: &str) {
        self.state.borrow_mut().rds_state.rt = rt.to_string();
    }

    /// Switch the fake tuner's synchronous echo off, for tests about the window
    /// between commanding a tune and hearing about it. No-op on a real tuner.
    pub fn set_echo_for_test(self: &Rc<App>, on: bool) {
        self.state.borrow().tuner.set_echo_for_test(on);
    }

    /// Point the fake tuner's log export at a directory a test can read. No-op on
    /// a real tuner, where the destination is Downloads and not ours to choose.
    pub fn set_log_dir_for_test(self: &Rc<App>, dir: std::path::PathBuf) {
        self.state.borrow().tuner.set_log_dir_for_test(dir);
    }

    /// The VENDOR SERVICE retuning the front end on its own, for tests about who
    /// gets the last word. See `Tuner::vendor_tune_for_test` — this moves the
    /// simulated hardware, which a bare frequency report does not.
    pub fn vendor_tunes_for_test(self: &Rc<App>, mhz: f32) {
        self.state.borrow().tuner.vendor_tune_for_test(mhz);
    }

    /// Where the FRONT END is, not where the face says it is. `None` on a real
    /// tuner. See `Tuner::tuned_mhz_for_test`.
    pub fn tuned_mhz_for_test(self: &Rc<App>) -> Option<f32> {
        self.state.borrow().tuner.tuned_mhz_for_test()
    }

    /// Take the meter to where a settled poll would leave it, for the screenshot
    /// harness only.
    ///
    /// The level schedule reads at 1s and corrects at 4s, and the loss band is
    /// settled by a 1.5s poll — all real `slint::Timer`s, against a harness that
    /// renders the frame it builds. Without this, every shot that tunes would
    /// show an empty meter over a station: a picture of a transient rather than
    /// of the face. Waiting four seconds sixty-two times is not a workflow.
    ///
    /// It is those timers' OUTCOME, not a bypass of them — the same
    /// `tuner.read_level_now()` and the same `settle_dotted`, taken early.
    ///
    /// PUSHES ONLY THE METER. It used to end in `push_all`, and that was wrong in
    /// a way that cost a reference image: `apply` in the harness has arms that
    /// set face properties DIRECTLY, because no fake can produce a lossy carrier
    /// or a traffic announcement, and a full republish overwrote every one of
    /// them. `weak-and-lossy.png` came back as an ordinary strong-signal shot.
    pub fn settle_meter_for_test(self: &Rc<App>) {
        self.state.borrow().level_first.stop();
        self.state.borrow().level_correction.stop();
        self.read_level();
        settle_dotted(&mut self.state.borrow_mut());
        self.push_meter();
    }

    /// Save the current dial, or drop it if it is already saved.
    ///
    /// THE STRIP HAS NO LIMIT, and the one that used to be here was never a
    /// decision. Both save paths capped the list at `fake::SEED_PRESET_MHZ.len()`
    /// — the length of the six-element demo array that seeds the SCREENSHOT
    /// HARNESS — and then silently deleted the driver's oldest preset to make
    /// room. A constant from `crate::fake` was acting as shipping policy, and the
    /// comment justifying it as "what a six-slot strip with no slot picker can
    /// do" was written to explain the accident rather than from any intent.
    ///
    /// Nothing needed the cap. The band is a `Flickable` whose content width and
    /// grid height are computed from the list length, so it already scrolls; the
    /// tall track already wraps to three columns and caps its own height at 42%
    /// of the screen. Removing it changes no geometry.
    fn toggle_save(self: &Rc<App>) {
        {
            let mut s = self.state.borrow_mut();
            // WHAT THE DRIVER IS LOOKING AT. Mid-hold the face shows the target,
            // so a star pressed then means that station and not whichever one
            // the vendor's bank walk is passing through.
            let dial = s.shown();
            match s.active() {
                // Saved: drop it out of the strip.
                i if i >= 0 => {
                    s.presets.remove(i as usize);
                }
                // OUT OF BAND IS NOT SAVEABLE, and until now this was the one
                // writer that did not check. `prefs::from_json` drops anything
                // outside the band on the way back in — "a dial outside the FM
                // band is not a preset, it is corruption" — so a tile saved out
                // of band was written to disk and then silently deleted at the
                // next launch, taking the driver's last save with it.
                //
                // `s.dial` really can leave the band: the `Frequency` arm takes
                // whatever the tuner reports, unguarded, where the 1.5s poll is
                // guarded. The face already knows — `set_in_band` publishes it —
                // so refusing agrees with what the driver is being shown.
                //
                // WHETHER THIS UNIT CAN REPORT OUT OF BAND IS UNVERIFIED. It has
                // no band button, so the likely route is a vendor report during a
                // band change the app never asked for. The guard costs one
                // comparison and the failure it prevents is silent data loss.
                //
                // AN ARM RATHER THAN AN EARLY RETURN, so the refusal still
                // reaches the diagnostics panel: `push_all` below is what
                // publishes the log, and `save_prefs` finds the strip unchanged
                // and writes nothing.
                _ if !(FM_LO..=FM_HI).contains(&dial) => {
                    let line = format!("save refused: {dial:.1} is outside the FM band");
                    s.settings.log.push(&stamp(), &line);
                }
                // Unsaved: append it. THE STRIP HAS NO LIMIT — see `toggle_save`'s
                // own note below.
                _ => {
                    let at = s.location.position();
                    let row = resolve(s.db.as_ref(), dial, at);
                    // THE LEARNED MAP IS THE FALLBACK HERE TOO. Saving a preset
                    // in a driveway with no fix resolves no row, and taking only
                    // the row left the new tile showing a bare frequency while
                    // the map on disk knew perfectly well what was on that dial.
                    let saved_call = row
                        .as_ref()
                        .map(|r| r.callsign.clone())
                        .or_else(|| s.callsigns.get(dial).map(str::to_string));
                    s.presets.push(Slot { mhz: dial, row, saved_call });
                }
            }
        }
        self.push_all();
        // The strip is a preference too, and the one the driver would miss most.
        self.save_prefs();
    }

    /// Write the driver's choices, if any of them have actually changed.
    ///
    /// Called from `push_settings` and after every preset edit, which between
    /// them cover every mutation worth keeping. The change check is not an
    /// optimisation for its own sake: `push_settings` also runs on connect, on
    /// every tuner event that touches the panel, and on each diagnostics line,
    /// and rewriting the file on all of those would put a flash write in the
    /// path of ordinary radio traffic.
    fn save_prefs(&self) {
        let mut s = self.state.borrow_mut();
        let now = crate::prefs::Prefs {
            presets: s
                .presets
                .iter()
                .map(|p| crate::prefs::Preset {
                    mhz: p.mhz,
                    call: p.identity().map(str::to_string),
                })
                .collect(),
            selected: s.settings.selected,
            theme: s.settings.theme,
            logos_on: s.settings.logos_on,
            release_on_sleep: s.settings.release_on_sleep,
            clock_on: s.settings.clock_on,
            nav_on: s.settings.nav_on,
            nav_hide_on_map: s.settings.nav_hide_on_map,
            diag_on: s.settings.diag_on,
        };
        if now == s.saved {
            return;
        }
        crate::prefs::save(&s.prefs_dir, &now);
        s.saved = now;
    }

    /// The power button, and only the power button.
    ///
    /// `user_powered_off` is latched here rather than derived from `audio`,
    /// because the getter poll writes `audio` from the MCU's own source register
    /// and the two facts have to stay apart: "the driver switched the radio off"
    /// must outlive "the MCU says FM is not the current source", or the poll
    /// would switch the radio back on 1.5s after every press.
    fn set_audio(self: &Rc<App>, on: bool) {
        {
            let s = self.state.borrow();
            s.tuner.set_audio_enabled(on);
        }
        self.state.borrow_mut().user_powered_off = !on;
        self.state.borrow_mut().audio = on;
        self.drain_events();
        self.push_all();
    }

    fn apply_theme(&self, theme: settings::Theme) {
        // SYSTEM has nothing to read here: there is no Android UiModeManager and
        // no desktop preference, so it holds whatever is already set rather than
        // guessing light.
        let ui = self.ui();
        match theme {
            settings::Theme::Light => ui.global::<crate::Pal>().set_dark(false),
            settings::Theme::Dark => ui.global::<crate::Pal>().set_dark(true),
            settings::Theme::System => {}
        }
    }

    /// A new position.
    ///
    /// This is the seam a real `LocationManager` callback lands on: it takes the
    /// fix, re-runs the nearby query against it, and republishes. Nothing else
    /// in this crate knows where the position came from, which is why the fake
    /// and the framework can share one path.
    pub fn set_position(&self, location: fake::FakeLocation) {
        self.state.borrow_mut().location = location;
        self.refresh_nearby();
        self.push_hero();
    }

    /// One second has passed.
    ///
    /// The only heartbeat in the process. Everything here has to be cheap enough
    /// to run forever on a 32-bit head unit, and cheap here means "a comparison
    /// unless something is actually wrong".
    pub fn tick(self: &Rc<App>) {
        if self.expire_rds() {
            self.push_hero();
            self.push_meter();
            self.push_settings();
        }
        // THE CLOCK, WHICH IS WHY THIS TIMER IS 1s AND NOT SLOWER. §4.8 asks
        // for a 1s tick that only changes state on the minute roll, which is
        // what `push_clock` does with its own comparison.
        self.push_clock();
        // THE ROUTE AGES OUT ON THIS CLOCK AND ON NOTHING ELSE. OsmAnd stops
        // sending when a route ends, is cancelled, or the app is closed, and
        // never says which — so without a tick the last turn before the driver
        // arrived would stay on the face for the rest of the drive. Republished
        // unconditionally rather than on a computed edge: `push_nav` is four
        // property writes and a string compare, which is cheaper than the state
        // needed to decide whether to skip it.
        self.push_nav();
    }

    /// Disown the RDS on screen when the carrier has gone quiet.
    ///
    /// LOSING A STATION IS SILENCE, NOT AN EVENT. Drive out of range without
    /// touching the dial and the groups simply stop; every merge in this file is
    /// "keep what we had" and only a retune ever cleared anything. So the strip
    /// went on scrolling the last song, the genre kept the old PTY, TP and TA
    /// stayed latched and the RDS tell stayed lit — all of it over hiss, for as
    /// long as the drive lasted. This is CarFM's expiry, and the two rules that
    /// look wrong are the ones it paid for:
    ///
    /// THE DECODER IS NOT RESET. Expiry means "the carrier went quiet", not "we
    /// are on a different station" — a retune is what means that, and the
    /// frequency handler already resets there. Resetting here made the
    /// corruption WORSE: it clears `rt_published`, which re-opens the
    /// instant-publish path for the first complete assembly after the signal
    /// returns, and that assembly is received in exactly the marginal conditions
    /// that caused the gap. Eight of the eleven corrupt RadioText changes on
    /// WERN on 2026-08-04 were first fills after an expiry. Keeping the decoder
    /// means the confirmed text comes straight back instead of being re-acquired
    /// from the worst air of the drive.
    ///
    /// TA IS DROPPED IN THE DECODER, not just on the face. TP and the text are
    /// station identity worth restoring; TA means "an announcement is happening
    /// RIGHT NOW", which is not safe to assume across a multi-second silence.
    /// Clearing the tally with it forces a re-confirm — without that, a
    /// still-running announcement could not re-publish, because the tally would
    /// already be satisfied.
    ///
    /// The hero keeps its name through the station database and the learned
    /// call-sign map, which is by design: the dial has not moved, so what is
    /// licensed on it has not changed.
    fn expire_rds(self: &Rc<App>) -> bool {
        let mut s = self.state.borrow_mut();
        let Some(at) = s.last_rds_at else { return false };
        if at.elapsed() < RDS_STALE {
            return false;
        }
        s.last_rds_at = None;
        s.rds_stale = true;
        // Whatever came off disk has now outlived its own window, so the next
        // retune must clear the face rather than protect a restore.
        s.warm_dial = None;
        s.rds.clear_ta();
        s.rds_state = RdsState::default();
        // THE PILOT IS NOT CLEARED HERE EITHER. The argument was that it came
        // from the same carrier that has gone quiet — but RDS and the stereo
        // pilot are different subcarriers and they fail independently: 57 kHz
        // can be lost to multipath while 19 kHz still locks, which is why
        // CarFM's expiry clears `name`, `text`, `pty`, `tp`, `ta` and `pi` and
        // leaves stereo alone (RadioScreen.tsx:3046-3049). A station whose RDS
        // is marginal is exactly the one a driver is watching this pill on.
        let at = stamp();
        s.settings
            .log
            .push(&at, &format!("RDS expired — no group for {}s", RDS_STALE.as_secs()));
        true
    }

    /// Carry out what a panel key asked for.
    ///
    /// MUST NOT run inside `apply_event`: everything here retunes, and a retune
    /// borrows the state `apply_event` is already holding.
    fn apply_panel_action(self: &Rc<App>, action: PanelAction) {
        match action {
            PanelAction::Step { dir, from } => self.step_preset_from(dir, from),
            // UNREACHABLE ON THIS UNIT, and kept anyway. Its four codes — search
            // and seek, up and down — are rows of the vendor's decompiled
            // dispatch table, and the fascia has no button for any of them: the
            // wheel is `ch+`/`ch-`, volume and `mode`, and the head unit has the
            // Android navigation buttons and a volume control.
            // `crate::android::PanelKey` carries the full inventory.
            //
            // SAID HERE BECAUSE AN AUDIT MISSED IT. This arm does not set
            // `s.scanning = true` where the on-screen seek does, which was
            // reported as "CANCEL cannot abort a wheel-started sweep" — true of
            // the code, and describing a sweep this hardware cannot start.
            // `on_freq_seek` is the only seek there is, and it sets the flag.
            // Making this arm match would be tidying an unreachable branch;
            // recording that it IS unreachable is worth more.
            PanelAction::Seek(up) => {
                {
                    let mut s = self.state.borrow_mut();
                    // THE DRIVER IS LEAVING THE STRIP ON PURPOSE, which is the
                    // one case where the app should forget where it had put
                    // them. Stepping after a seek then behaves exactly as it
                    // always did: the landed dial is off-strip, so the next
                    // press lands on entry 0.
                    s.asserted = None;
                    // AND ANY COMMITTED FACE GOES WITH IT. A seek within two
                    // seconds of a step would otherwise be fought by that step's
                    // re-assert, which would cancel the sweep — the one thing
                    // `NwdBridge.seek`'s note says never to do.
                    s.hold = None;
                    s.reassert = None;
                    s.tuner.seek(up);
                }
                // NO RE-TUNE AFTER, and there used to be one.
                //
                // `NwdBridge.seek` calls `RadioFeature.search` and returns at
                // once (NwdBridge.java:337-342); the frequency it lands on
                // arrives later as `notifyCurrentFrequency`. So the drain below
                // finds nothing, `s.dial` is still where the seek STARTED, and
                // tuning to it commanded the front end straight back — cancelling
                // the seek on the one device this app is for.
                //
                // CarFM hands a hardware seek over and stops: "the landed
                // frequency arrives via the tuner callback, so there's no
                // client-side sweep to animate here" (CarFmFace.tsx:918-921).
                // Everything that hangs off the new dial is already the
                // `Frequency` arm's work.
                self.drain_events();
                self.pump_rds_until_settled();
                self.push_all();
            }
        }
    }

    /// One step through the preset strip, in DISPLAYED order, wrapping.
    ///
    /// Shared by the on-screen arrows, the peek cards and the wheel, so all
    /// three animate identically — CarFM makes the same point about its
    /// hardware step running "the SAME animated stepPreset ... not a silent
    /// frequency jump".
    fn step_preset(self: &Rc<App>, dir: i32) {
        let from = self.state.borrow().anchor();
        self.step_preset_from(dir, from);
    }

    /// The step itself, from a strip position the CALLER decided.
    ///
    /// SPLIT OUT SO THE WHEEL CAN NAME ITS OWN ORIGIN. The on-screen arrows and
    /// the peek cards act the instant they are touched, so reading the anchor
    /// inside is the same as reading it outside. A wheel press is DEFERRED —
    /// `drain_events` runs it only once the tuner queue is empty — and in the
    /// meantime the vendor service, which stepped its own hardware preset bank
    /// on the very same press, reports the frequency it landed on and moves
    /// `dial`. Reading the anchor inside therefore read the VENDOR'S position;
    /// `from` carries the driver's.
    fn step_preset_from(self: &Rc<App>, dir: i32, from: i32) {
        let next = {
            let s = self.state.borrow();
            let n = s.presets.len() as i32;
            if n == 0 {
                return;
            }
            // A strip that shrank since the key arrived cannot honour the old
            // index, and stepping off the end of it would panic below.
            let active = if from >= n { -1 } else { from };
            if active < 0 {
                if dir > 0 {
                    0
                } else {
                    n - 1
                }
            } else {
                (active + dir).rem_euclid(n)
            }
        };
        let (mhz, moves, discards) = {
            let s = self.state.borrow();
            let n = s.presets.len() as i32;
            let mhz = s.presets[next as usize].mhz;
            let active = if from >= n { -1 } else { from };

            // §8: THE MORPH RUNS ONLY WHEN THE FREQUENCY ACTUALLY CHANGES.
            // CarFM's is in `componentDidUpdate` behind exactly that test, and
            // without it a step that lands where the dial already is still plays
            // the whole 520ms — cards flying for a swap that never happened. The
            // reachable case is a single preset, where `(active + dir) % 1` is
            // `active`, so every press re-tunes the station already playing.
            let moves = (mhz - s.dial).abs() as f64 > crate::stations::FREQ_EPS;

            // AND WHETHER ANYTHING IS ACTUALLY DISCARDED. The ghost stands for a
            // card that is about to stop existing, so it is only honest when the
            // slot's occupant changes. Stepping forward off an UNSAVED dial is
            // the case that breaks it: with no active preset the peeks show the
            // last and first entries, and after landing on entry 0 the prev slot
            // still shows the last entry — the same card. Fading a card out on
            // top of an identical one is a ghost of nothing.
            let discards = step_discards(n, active, next, dir);
            (mhz, moves, discards)
        };
        // ARM THE MORPH BEFORE THE TUNE. This is a FLIP: the hero is put back
        // where the incoming station came from and travels to where it belongs,
        // so the cards must already hold the new stations when it starts.
        if moves {
            // COMMIT THE FACE TO THE TARGET FIRST. CarFM's `commitTo`, called on
            // the same line as the tune (`CarFmFace.tsx:1098-1099`), and taken
            // BEFORE `tune` so the counter it records is the one from before any
            // report this tune provokes. See `State::hold`.
            {
                let mut s = self.state.borrow_mut();
                let since_seq = s.freq_seq;
                s.hold = Some(Hold { mhz, since_seq, at: std::time::Instant::now() });
                // A FRESH BUDGET PER PRESS. See [`State::reassert`]. Re-armed
                // here rather than topped up so a press that spent both attempts
                // fighting the vendor cannot leave the next one with none.
                s.reasserts_left = REASSERT_TRIES;
                s.reassert = None;
            }
            // START THE TALLY BEFORE THE MORPH IS ARMED, so the clock covers the
            // whole window including whatever `tune` below spends before the
            // first frame can be drawn. See `State::morph_frames`.
            {
                let mut s = self.state.borrow_mut();
                s.morph_frames = 0;
                s.morph_since = Some(std::time::Instant::now());
                s.morph_report.start(
                    slint::TimerMode::SingleShot,
                    MORPH_REPORT_AFTER,
                    report_morph_frames,
                );
            }
            self.step_morph(dir, discards);
            self.tune(mhz);
            return;
        }
        // AND THE TUNE IS GATED ON THE SAME TEST, which it was not. `moves`
        // suppressed the morph and the retune went out anyway, so on a
        // ONE-ENTRY STRIP — where `(active + dir).rem_euclid(1)` is always
        // `active` — every press re-commanded the front end to the station
        // already playing. That is not a no-op: `tune` drops `level` and
        // `dotted`, and the frequency report it provokes runs
        // `rds.reset_for_retune`, so the name, the RadioText and the PTY all
        // blank and have to be decoded again. Pressing next on a single preset
        // wiped the face.
        //
        // Reachable from the peek cards, which the hero row draws at n == 1.
        // From the wheel it was usually masked, because the vendor has moved the
        // dial by then and `moves` is true — in which case the tune above is the
        // intended reassertion and still happens.
        //
        // THE ANCHOR STILL MOVES. The driver pressed the button and the strip's
        // position is now this entry, whatever the radio was already doing; only
        // the command to the tuner is skipped. `tune` would have set this.
        {
            let mut s = self.state.borrow_mut();
            s.asserted = Some(mhz);
            // AND IT CLEARS A LATCHED SWEEP, which `tune` was also doing here.
            // `tune`'s own note: on a front end that answers a seek with no
            // further frequency report, nothing else would ever clear this. Skip
            // the tune and the flag stays set, so the hero keeps the sweeping
            // face for the rest of the session.
            s.scanning = false;
            // AND IT RETIRES THE PREVIOUS STEP'S HOLD, which nothing else on this
            // branch would. `tune` is what normally drops a hold that has been
            // overtaken, and this branch deliberately does not call it — so a hold
            // from the press before, which since `settled` no longer ends on
            // arrival, stayed live here with a re-command still owed against it.
            // The driver pressed a key and landed where they already were; the
            // older press is finished either way, and letting its re-assert fire
            // afterwards drags the radio back to a station they have left.
            //
            // `wheelprobe` case C caught it the moment the hold stopped being
            // released on settle: one row of six went to 88.7 where 105.5 was
            // asked for, because the row before it had left a hold behind.
            s.hold = None;
            s.reassert = None;
        }
        // AND IT STILL PUBLISHES, which the first cut of this forgot. `tune` was
        // doing that too, so skipping it left the face on the PREVIOUS station
        // while the radio sat on the new one — a display lie, and worse than the
        // redundant retune this branch exists to avoid. `wheelprobe` caught it
        // at once: stepping from 88.7 to 105.5 while the vendor had already put
        // the dial on 105.5 left 88.7 on the hero.
        self.push_all();
    }

    /// Re-resolve every preset's FCC row against the position as it now stands.
    ///
    /// THIS WAS MISSING, and its absence is what turned a first fix into nothing
    /// at all. The rows were resolved once, in the constructor, against whatever
    /// position existed then — which on the device is no fix, because the GPS has
    /// not answered yet when the window is built. When the fix finally arrived
    /// the hero picked it up, because the hero re-resolves on every push, and the
    /// strip did not, because it renders a row that was decided at start-up. Six
    /// presets stayed bare frequencies for the whole session and the logo window
    /// had no call sign to search on.
    ///
    /// The stored fallback is refreshed alongside, so a dial that has resolved
    /// once keeps its identity through the next cold start with no fix.
    fn resolve_presets(&self) {
        let rows: Vec<Option<StationRow>> = {
            let s = self.state.borrow();
            let here = s.location.position();
            s.presets.iter().map(|p| resolve(s.db.as_ref(), p.mhz, here)).collect()
        };
        let mut changed = false;
        {
            let mut s = self.state.borrow_mut();
            // Cloned out first: the loop borrows `s.presets` mutably, and the map
            // is read inside it.
            let learned = s.callsigns.clone();
            for (slot, row) in s.presets.iter_mut().zip(rows) {
                // A preset that has never resolved takes the learned answer, so
                // a strip saved before this map existed gets its names back on
                // the first launch after one.
                if slot.saved_call.is_none() {
                    if let Some(c) = learned.get(slot.mhz) {
                        slot.saved_call = Some(c.to_string());
                        changed = true;
                    }
                }
                if slot.row == row {
                    continue;
                }
                changed = true;
                if let Some(r) = &row {
                    slot.saved_call = Some(r.callsign.clone());
                }
                slot.row = row;
            }
        }
        // Only when something actually moved: this runs on every fix, and a car
        // parked with a lock produces one every two seconds.
        if changed {
            self.save_prefs();
        }
    }

    /// How far the car has to move before the nearby list is worth re-deriving.
    ///
    /// THE COST THIS AVOIDS IS PAID EVERY TWO SECONDS, FOREVER. A parked car with
    /// a lock produces a fix on that cadence and every one of them re-ran the
    /// whole query — bounding box, rank, view, publish — to conclude that nothing
    /// had changed. `examples/pushbench.rs` measures it: 2.1ms per fix on a
    /// desktop, and this unit is a 32-bit ARM head unit.
    ///
    /// 250 METRES, against a 100km radius and a list whose distances are shown to
    /// the kilometre. Nothing on screen can change over less than that: the
    /// nearest station's distance is rounded past it, the ranking is by a score
    /// dominated by ERP and log-distance, and no station enters or leaves a 100km
    /// circle because the car shifted a street's width. It is two orders of
    /// magnitude below the radius and one above ordinary standing-still GPS
    /// noise, which is metres.
    const NEARBY_REFRESH_M: f64 = 250.0;

    fn refresh_nearby(&self) {
        // A FIX THAT HAS NOT MOVED IS NOT NEWS. Cheap and exact — the comparison
        // is against the position the current picker was BUILT from, not the last
        // fix seen, so a car creeping 10m at a time can never accumulate its way
        // past the threshold without the list being rebuilt.
        {
            let s = self.state.borrow();
            if let (Some(built), Some(now)) = (s.picker_at, s.location.position()) {
                if s.picker.located() && metres_between(built, now) < Self::NEARBY_REFRESH_M {
                    return;
                }
            }
        }
        let learned = {
            let mut s = self.state.borrow_mut();
            let (db, loc, snap) = (s.db.as_ref(), s.location, s.snapshot.clone());
            let picker = build_picker(db, loc, snap);
            s.picker = picker;
            // Recorded even when there is no fix (None), so the guard above
            // cannot skip the first located refresh after one arrives.
            s.picker_at = loc.position();

            // LEARN WHAT THIS FIX REVEALED. One good lock fills the local band,
            // and every no-fix start afterwards can name a station from it —
            // which is the whole reason a car that cannot see the sky from its
            // own driveway is not left with six bare frequencies.
            //
            // From the rows the picker just queried rather than a second query:
            // this runs on every position change, and a parked car with a lock
            // produces one every two seconds.
            //
            // FULL-POWER FM ONLY, matching CarFM's `s.service === 'FM'` filter.
            // A translator can sit on a frequency it does not define.
            if s.picker.located() {
                let rows: Vec<(f32, String)> = s
                    .picker
                    .rows()
                    .iter()
                    .filter(|r| r.row.service == "FM")
                    .map(|r| (r.row.frequency_mhz as f32, r.row.callsign_base.clone()))
                    .collect();
                s.callsigns.relearn(rows.iter().map(|(m, b)| (*m, b.as_str())))
            } else {
                false
            }
        };
        if learned {
            let s = self.state.borrow();
            crate::callsigns::save(&s.prefs_dir, &s.callsigns);
        }
        self.push_nearby();
    }

    fn save_nearby_preset(self: &Rc<App>, i: i32) {
        let row = self.state.borrow().picker.station_at(i).cloned();
        let Some(row) = row else { return };
        {
            let mut s = self.state.borrow_mut();
            let mhz = row.frequency_mhz as f32;
            let key = crate::stations::preset_key(f64::from(mhz));
            if let Some(at) = s
                .presets
                .iter()
                .position(|p| crate::stations::preset_key(f64::from(p.mhz)) == key)
            {
                s.presets.remove(at);
            } else {
                let saved_call = Some(row.callsign.clone());
                s.presets.push(Slot { mhz, row: Some(row), saved_call });
            }
        }
        self.push_presets();
        self.push_nearby();
        self.save_prefs();
    }

    /// The DIAGNOSTICS rows.
    ///
    /// BOTH ROWS DO SOMETHING NOW, which they did not before: five of the seven
    /// fell through to a line saying they were unavailable, because they were
    /// CarFM's vendor probes and had never been written. Those rows are gone
    /// rather than stubbed, so there is no catch-all arm left to write.
    /// Put a probe's report in the log, or say that it had nothing to say.
    ///
    /// SHARED BY BOTH PROBE ROWS so the two cannot drift. The rule is the part
    /// worth holding in one place: an empty report is a STATE, not a failure —
    /// on a host build the class does not exist, and on the unit it means the
    /// class never loaded — and both deserve a line rather than a tap that does
    /// nothing visible, which is the exact fault the five removed rows had.
    ///
    /// One stamp for the whole report, not one per line: it is a single reading,
    /// and stamping each line would read as a burst of separate events.
    fn log_report(log: &mut settings::DiagLog, lines: Vec<String>, unavailable: &str) {
        let at = stamp();
        if lines.is_empty() {
            log.push(&at, unavailable);
            return;
        }
        for line in lines {
            log.push(&at, &line);
        }
    }

    /// The deferred half of a probe row: do the binder work, write the report.
    ///
    /// PUBLIC FOR THE TESTS, which drive it directly rather than waiting on a
    /// timer — a test that slept for a frame would be a test of the clock. The
    /// device path is `run_pending_probe_current`, one tick after the tap.
    ///
    /// THE FOOTER IS NOT DECORATION. A probe that hangs or dies inside the
    /// vendor's binder leaves only the "reading…" line, and a probe that
    /// finished leaves a count — so the two are distinguishable in a log read
    /// hours later, which is the only way this unit reports anything.
    pub fn run_pending_probe(self: &Rc<App>) {
        let Some(action) = self.state.borrow_mut().pending_probe.take() else { return };
        let name = probe_name(action);
        // OUTSIDE THE BORROW. This crosses into Java and walks the package
        // manager; a `RefCell` held across it would be a lock held over code
        // that can call back, which is the rule the panel key and the sleep
        // release already follow.
        let lines = match action {
            settings::Action::ProbeStockRadio => crate::android::stock_radio_report(),
            _ => crate::android::keep_alive_report(),
        };
        let count = lines.len();
        {
            let mut s = self.state.borrow_mut();
            Self::log_report(
                &mut s.settings.log,
                lines,
                &format!("{name}: unavailable in this build"),
            );
            let at = stamp();
            s.diag_status = if count > 0 {
                s.settings.log.push(&at, &format!("{name}: done, {count} lines"));
                format!("{name}: {count} lines at {at}")
            } else {
                format!("{name}: nothing to report — see the log")
            };
        }
        self.push_settings();
    }

    fn run_diag_action(self: &Rc<App>, index: i32) {
        let action = {
            let s = self.state.borrow();
            match s.settings.actions().get(index as usize) {
                Some(a) => a.action,
                None => return,
            }
        };
        {
            let mut s = self.state.borrow_mut();
            match action {
                settings::Action::ClearLog => s.settings.log.clear(),
                // ── WHAT COULD KEEP US ALIVE THROUGH A SLEEP ──────────────────
                //
                // The report is a handful of lines and they go into the log
                // rather than into a dialog, because the log is the only thing
                // on this unit that can be carried off it — the panel has no
                // alert, and "Save to file" is right there under this row.
                //
                // AN EMPTY REPORT IS A STATE, NOT A FAILURE: on a host build the
                // class does not exist, and on the unit it means the class never
                // loaded. Both deserve a line rather than a tap that does
                // nothing visible.
                // ── BOTH PROBES ARE DEFERRED BY A FRAME ───────────────────────
                //
                // The tap writes "reading…" and returns; `probe_run` does the
                // binder work on the next tick. See `State::pending_probe` for
                // why, and `run_pending_probe` for what happens then.
                //
                // The stock-radio report is the LONGER of the two — a package,
                // its components and an intent sweep — so the Java side caps
                // every list it prints; the ring holds 600 lines and this is one
                // of several writers.
                settings::Action::ProbeKeepAlive | settings::Action::ProbeStockRadio => {
                    s.diag_status = format!("{}: reading…", probe_name(action));
                    s.settings.log.push(&stamp(), &format!("{}: reading…", probe_name(action)));
                    s.pending_probe = Some(action);
                    s.probe_run.start(
                        slint::TimerMode::SingleShot,
                        std::time::Duration::from_millis(PROBE_DEFER_MS),
                        run_pending_probe_current,
                    );
                }
                settings::Action::SaveLog => {
                    let at = stamp();
                    if s.settings.log.is_empty() {
                        // CarFM's "Nothing to save" alert (`SettingsPanel.tsx:125`).
                        // There is no alert here; the log IS the channel, and a
                        // line saying why is what a tap has to leave behind.
                        s.settings.log.push(&at, "save to file: the log is empty");
                    } else {
                        // `lines().join("\n")` is CarFM's `diagText()` exactly —
                        // "lines.join('\\n')" (`services/diag.ts:105`), same order,
                        // oldest first. THE WHOLE RING, not the handful on screen:
                        // that is the entire reason this exists.
                        let text = s.settings.log.lines().join("\n");
                        // ON THE UI THREAD, which CarFM's was not — a `@ReactMethod`
                        // runs on the native-modules thread. A MediaStore insert
                        // plus a write of at most 200 short lines is a few
                        // milliseconds of binder, taken on an explicit tap in a
                        // settings panel, so the hitch lands where nobody is
                        // driving by it. Worth knowing if this ever grows.
                        let outcome = s.tuner.write_log(&text);
                        let line = match outcome {
                            Ok(path) => format!("log saved to {path}"),
                            Err(e) => format!("save to file failed: {e}"),
                        };
                        s.settings.log.push(&at, &line);
                    }
                }
            }
        }
        self.push_settings();
    }

    /// One line into the diagnostics log the driver can read.
    ///
    /// Named for its first use — reporting a thing this build cannot do — and it
    /// still does that, but the reorder gesture writes its own trace through it
    /// too. Anything that has to say something to a person and has nowhere else
    /// to say it comes here.
    /// Same channel, for the platform wiring that happens in `android_main`
    /// BEFORE any callback can fire.
    ///
    /// PUBLIC BECAUSE THE UNIT HAS NO adb. The foreground service reports itself
    /// to logcat, which on a head unit reaches nobody — the settings panel's log
    /// is the one channel a driver can actually read, and whether the service
    /// started is exactly the kind of thing that has to be readable there when
    /// the answer turns out to be "it didn't".
    /// INTO THE HEAD, WHICH NEVER SCROLLS AWAY. Every caller is in
    /// `android_main`'s start-up block and every line it writes is a fact the run
    /// establishes once and cannot establish again — how the last run ended, what
    /// the wake receiver did, what the last sleep managed. In a plain ring those
    /// are the first lines evicted, and they are the ones a drive log is read
    /// for; see `DiagLog::push_head` for the drive that proved it.
    pub fn log_platform(self: &Rc<App>, line: &str) {
        {
            let mut s = self.state.borrow_mut();
            let at = stamp();
            s.settings.log.push_head(&at, line);
        }
        self.push_settings();
    }

    fn log_unavailable(self: &Rc<App>, why: &str) {
        {
            let mut s = self.state.borrow_mut();
            let at = stamp();
            s.settings.log.push(&at, why);
        }
        self.push_settings();
    }

    fn open_logo_search(self: &Rc<App>, index: i32) {
        let Some(slot) = self.state.borrow().presets.get(index as usize).cloned() else {
            return;
        };
        // Through `Slot::base`, so a preset whose row has not resolved still
        // searches on the call sign it was saved with. Reading `slot.row`
        // directly is what left the logo window with an empty query on a unit
        // with no fix.
        let base = slot.base();
        let target = crate::logos::search::Target {
            base: base.clone(),
            callsign: base.clone(),
            freq_mhz: slot.mhz,
            name: slot.name(),
        };

        // What this station ALREADY has, read before the window opens. The three
        // answers are genuinely different and the window renders each one
        // differently: art plus explicit flags, art with the flags never chosen,
        // and no art at all.
        //
        // Read OUTSIDE any borrow — `read_art` decodes through JNI.
        let key = base.to_uppercase();
        // FALSE, NOT THE LIVE THEME. This is the picture the window shows beside
        // "Current logo", and what it is showing is the MASTER the driver
        // assigned — the thing a new pick would replace. A dark-adapted variant
        // there would offer to replace something that is not a file.
        let existing = if key.is_empty() {
            None
        } else {
            self.read_art(&key, None, false).map(|(r, _)| r)
        };
        let stored = if key.is_empty() {
            None
        } else {
            self.state.borrow().store.prefs(&key)
        };

        {
            let mut s = self.state.borrow_mut();
            s.logo.open(target, existing.is_some(), stored);
            s.logo_art = existing;
        }
        self.push_logo_search();
    }

    /// Queue the search on the worker, or run the fake when there is no worker.
    ///
    /// The split is the whole difference between the device and the host, and it
    /// is ONE branch on purpose: the generation counter, the arrival order, the
    /// per-cell thumbnail landing and the selection are `search::Model`'s either
    /// way. Only where the bytes come from changes.
    fn run_logo_search(self: &Rc<App>) {
        let job = self.state.borrow_mut().logo.search();
        let Some(job) = job else { return };
        self.push_logo_search();

        // The real path returns immediately; the results arrive as events on the
        // logo queue and land through `apply_logo_event`.
        {
            let s = self.state.borrow();
            if let Some(w) = s.worker.as_ref() {
                w.search(&job);
                return;
            }
        }

        // No worker — the host. `crate::fake::FakeLogoSearch` stands in for the
        // network, synchronously, and the face says so.
        let mut s = self.state.borrow_mut();
        if s.logo.results_arrived(job.generation, fake::FakeLogoSearch::results()) {
            for (i, cell) in fake::logo_cells().into_iter().enumerate() {
                if let Some(art) = cell.thumb {
                    s.logo.thumb_arrived(job.generation, i, art);
                }
            }
        }
        drop(s);
        self.push_logo_search();
    }

    fn confirm_logo(self: &Rc<App>) {
        let outcome = self.state.borrow_mut().logo.begin_confirm();
        if matches!(outcome, crate::logos::search::Confirm::Ignore) {
            return;
        }

        // The worker owns the download, the decode, the trim, the ladder and the
        // dark pass — seconds of work on a 32-bit head unit, and the face is the
        // thing the driver is looking at. The window stays up in its `saving`
        // state until a `Saved` or `SaveFailed` event comes back.
        {
            let s = self.state.borrow();
            if let Some(w) = s.worker.as_ref() {
                w.submit(outcome);
                drop(s);
                self.push_logo_search();
                return;
            }
        }

        match outcome {
            crate::logos::search::Confirm::Ignore => {}
            // NO WORKER, so no decoder: writing a master needs decoded pixels.
            // The window reports the failure with the wording it would use for a
            // real one rather than pretending the art landed.
            crate::logos::search::Confirm::AssignLogo { .. } => {
                // The BARE reason. `error_body` wraps it as "Couldn\u{2019}t save this
                // logo \u{2014} {reason}. Try a different result.", so a reason that
                // repeats the prefix reads twice on the face — which is exactly
                // what the first render of this shot showed.
                self.state
                    .borrow_mut()
                    .logo
                    .save_failed("no image decoder is built in".into());
            }
            // The toggles alone: nothing to download, and nothing to store them
            // in either, so they live only as long as the window does.
            crate::logos::search::Confirm::SavePrefs { .. } => {
                self.state.borrow_mut().logo.saved();
                self.ui().set_overlay(Overlay::None);
            }
        }
        self.push_logo_search();
    }

    /// Tear the logo window down: forget the target, drop the art, and tell the
    /// worker to stop paying for an answer nobody is waiting for.
    ///
    /// The cancel is the point. A search is two round trips plus four thumbnail
    /// downloads, and `run_search` checks the shared generation between each
    /// one — so a window closed after the grid appears abandons the remaining
    /// thumbnails instead of finishing them. Without this call the worker would
    /// run every job to completion on a head unit's radio link.
    ///
    /// Called on EVERY overlay close, so it starts by checking there was a logo
    /// window at all: bumping the generation for a numpad dismissal would be
    /// harmless but untrue.
    /// Put the dark picker away and close the window behind it.
    ///
    /// BOTH BUTTONS END HERE and so does the ✕ — Skip, Use this, and a dismissal
    /// all leave a logo that is saved and a dark variant that exists, differing
    /// only in whether `chosen` is set. The reference's picker does the same:
    /// its `onClose` closes the search overlay behind it, whichever way it was
    /// answered.
    fn close_dark_pick(&self) {
        // THE CLEAR IS `close_logo_search`'s, NOT THIS FUNCTION'S. It republishes
        // only when it finds a picker to put away, so clearing the field here
        // first would make it think there was nothing to undo — and the left
        // button would stay saying "Skip" over the next window's results grid.
        self.ui().set_overlay(Overlay::None);
        self.close_logo_search();
        self.push_settings();
    }

    fn close_logo_search(&self) {
        // THE PICKER GOES FIRST, WHATEVER CLOSED THE WINDOW. The ✕ and the scrim
        // reach `close_overlay`, which calls this directly and knows nothing
        // about the picker; a `dark_pick` left standing would put the window
        // straight back into the picker's body the next time it opened.
        let had_pick = self.state.borrow_mut().dark_pick.take().is_some();
        if self.state.borrow().logo.target().is_none() {
            // A REPUBLISH IS STILL OWED IF A PICKER WAS UP. The early return is
            // for a window that was never open; a picker that WAS open has left
            // five properties on the window pointing at itself — the state, the
            // rows, the selection and both button labels — and nothing else
            // writes the left one.
            if had_pick {
                self.push_logo_search();
            }
            return;
        }
        let mut s = self.state.borrow_mut();
        s.logo.close();
        s.logo_art = None;
        let generation = s.logo.generation();
        if let Some(w) = s.worker.as_ref() {
            w.cancel(generation);
        }
        drop(s);
        self.push_logo_search();
    }

    /// Tell the hero a step is coming, and which way.
    ///
    /// A COUNTER, not a flag. Two steps in the same direction must both animate,
    /// and a flag that is already true changes nothing — so `HeroRow` watches the
    /// nonce, and the direction is only meaningful when it moves.
    /// Arm the hero morph, and hand the face the card it is about to lose.
    ///
    /// CALLED BEFORE THE TUNE, while the peeks still hold the pre-step stations.
    /// One of them is about to be discarded outright — stepping forward, the old
    /// PREV card is replaced by the outgoing hero and never appears again — and
    /// CarFM keeps it on screen by stamping an absolutely-positioned clone of the
    /// node before React replaces its content.
    ///
    /// Nothing here can clone an element, so the STATION is carried across
    /// instead: whichever peek is about to be overwritten is copied into
    /// `ghost-preset`, and the face draws it as a card that fades 0.6 → 0 over
    /// the morph. Read from the UI rather than recomputed from the preset list,
    /// because what has to fade out is exactly what the driver was looking at.
    ///
    /// The HERO is carried across the same way, and for the same reason — see
    /// `outgoing` below. Two snapshots, two different cards: the ghost is the
    /// peek that stops existing, `outgoing` is the station that was playing.
    fn step_morph(self: &Rc<App>, dir: i32, discards: bool) {
        let ui = self.ui();
        let dir = dir.signum();
        // Forward discards the PREV card, back discards the NEXT one — the slot
        // the outgoing hero is about to land in.
        ui.set_ghost_preset(if dir > 0 {
            ui.get_prev_preset()
        } else {
            ui.get_next_preset()
        });
        // And the HERO ITSELF, because the card that flies out is a real hero
        // card now, not a peek standing in for one. Same reason as the ghost, one
        // step further: `tune` runs synchronously on the caller's next line, so by
        // the time a frame is drawn the hero already holds the station that
        // ARRIVED. Read off the UI rather than rebuilt from the preset list — a
        // station tuned by hand is not in that list at all, and it is still the
        // card the driver was looking at.
        ui.set_outgoing(HeroSnapshot {
            ident: ui.get_ident(),
            freq_label: ui.get_freq_label(),
            logo: ui.get_logo(),
            has_logo: ui.get_has_logo(),
            plate: ui.get_logo_plate(),
            show_call: ui.get_show_call(),
            show_freq: ui.get_show_freq(),
            saved: ui.get_saved(),
        });
        ui.set_hand_off(discards);
        ui.set_step_dir(dir);
        ui.set_step_nonce(ui.get_step_nonce().wrapping_add(1));
    }

    /// Move one preset, after a drag in reorder mode has let go.
    ///
    /// The band never touches the list — it only reports where the finger
    /// landed, and every guard is here. An out-of-range index is DROPPED rather
    /// than clamped: a clamp would silently move the wrong preset, and a gesture
    /// that produced a nonsensical index is one that should do nothing.
    ///
    /// THE DIAL IS NOT TOUCHED. Reordering changes which tile a station sits on,
    /// never what is playing, so `active()` re-derives from the same dial and
    /// simply lands on a different index — which is why it was never stored.
    fn reorder_preset(self: &Rc<App>, from: i32, to: i32) {
        {
            let mut s = self.state.borrow_mut();
            let n = s.presets.len();
            let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
                return;
            };
            if from >= n || to >= n || from == to {
                return;
            }
            let slot = s.presets.remove(from);
            s.presets.insert(to, slot);
        }
        self.push_presets();
        self.push_hero();
        // The ORDER is what changed, and the order is what `prefs.json` stores,
        // so this has to reach disk or the drag is undone by the next launch.
        self.save_prefs();
    }

    /// Long-press on a preset tile — the door to reorder mode.
    ///
    /// REFUSED WHILE THE CAR IS MOVING, which is CarFM's rule
    /// (`CarFmFace.tsx:1654`, `if (blockedWhileDriving()) return;`): reordering
    /// is a two-handed, look-at-it task and the logo window behind it is worse.
    ///
    /// WHAT IS NOT PORTED, and it is visible: CarFM answers a refusal by
    /// swelling the §4.6 motion-car glyph, so the driver sees WHY nothing
    /// happened. That is `services/driveLock.ts`'s event bus plus an animation
    /// on one leaf, and neither exists here yet — so this refusal is silent on
    /// the face and only shows up in the diagnostics log.
    fn enter_reordering(self: &Rc<App>) {
        if self.state.borrow().location.in_motion {
            self.log_unavailable("reorder: refused, the car is moving");
            return;
        }
        self.ui().set_reordering(true);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// What a panel press means, or `None` for a press this app does nothing with.
///
/// ONLY THE DOWN EDGE ACTS. The MCU sends the press and the release as separate
/// broadcasts, and acting on both steps two stations for one push of the wheel.
///
/// A key this app has no answer for returns `None` rather than a no-op action,
/// so a pending request from an earlier press is not silently cleared by a
/// button that does nothing.
/// One key into the numpad's buffer, and the rules are CarFM's
/// (`Numpad.tsx:45-53`) rather than a size limit standing in for them.
///
/// AT MOST FOUR DIGITS, at most one decimal point, and never a LEADING one. The
/// buffer here used to be capped at six characters and nothing else, which let
/// `..`, `1.2.3`, `.1055` and `100000` be typed — none of which parses, so the
/// TUNE button stayed lit over a value that could not be tuned and pressing it
/// did nothing at all.
///
/// The key is checked rather than trusted. `ui/numpad.slint` only ever sends the
/// twelve keys, so this cannot change what a driver sees; it stops the buffer
/// from being a place where anything at all can be appended.
fn numpad_press(buf: &str, key: &str) -> String {
    if key == "." {
        // A leading decimal is refused outright — CarFM's `b.length === 0` arm.
        // "0.5" is not an FM dial, so there is nothing to lose.
        if buf.contains('.') || buf.is_empty() {
            return buf.to_string();
        }
        return format!("{buf}.");
    }
    if key.len() != 1 || !key.as_bytes()[0].is_ascii_digit() {
        return buf.to_string();
    }
    // DIGITS, not characters: "105." is four characters and three digits, and it
    // must still accept its fourth.
    if buf.chars().filter(|c| *c != '.').count() >= 4 {
        return buf.to_string();
    }
    format!("{buf}{key}")
}

/// A packed `0xRRGGBB` as a Slint colour.
///
/// The band-theme palette is written as hex in `eggs.rs` because that is how the
/// design states it and how CarFM stores it; a theme that is nearly the right red
/// is a bug wearing a costume, so the digits are carried across unchanged and
/// converted in exactly one place.
fn rgb(packed: u32) -> slint::Color {
    slint::Color::from_rgb_u8(
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        (packed & 0xFF) as u8,
    )
}

/// The same for a colour a theme may or may not have stated.
///
/// A `Skin` describes both a full restatement and a cut that changes one thing,
/// so most of its fields are `None` most of the time. `None` becomes a FULLY
/// TRANSPARENT colour, and every element that reads one tests its alpha to
/// decide whether the theme said anything at all — which is why this cannot go
/// back to a zero sentinel: `#000000` is a colour "Back in Black" actually
/// states, and as a `u32` it is indistinguishable from "nothing stated".
fn opt_rgb(packed: Option<u32>) -> slint::Color {
    match packed {
        Some(c) => rgb(c),
        None => slint::Color::from_argb_u8(0, 0, 0, 0),
    }
}

/// Does a step actually DISCARD the card in the slot the outgoing hero lands in?
///
/// The ghost stands for a card that is about to stop existing, so drawing one
/// when the slot's occupant does not change is a ghost of nothing — two
/// identical cards, one fading out on top of the other.
///
/// The peeks are derived from the active index the same way `push_hero` derives
/// them, including its rule that an UNSAVED dial (`active < 0`) shows the last
/// entry on the left and the first on the right. That rule is what makes the
/// answer interesting: stepping forward off an unsaved dial lands on entry 0,
/// whose prev is the last entry — exactly what was already there.
fn step_discards(n: i32, active: i32, next: i32, dir: i32) -> bool {
    if n <= 0 {
        return false;
    }
    let (old_prev, old_next) = if active < 0 {
        (n - 1, 0)
    } else {
        ((active - 1).rem_euclid(n), (active + 1).rem_euclid(n))
    };
    let (new_prev, new_next) = ((next - 1).rem_euclid(n), (next + 1).rem_euclid(n));
    if dir > 0 {
        old_prev != new_prev
    } else {
        old_next != new_next
    }
}

/// Could this buffer, or anything the keypad still lets the driver type after
/// it, commit to a dial?
///
/// WHEN THE OUT-OF-BAND LINE LIGHTS, and the one judgement call in this file that
/// the mini-handoff leaves open. §6 hands `freqError` to the host without saying
/// when to set it, and §5 makes TUNE close the overlay whatever the buffer holds —
/// so an error raised BY the commit, the way the old standalone card raised it,
/// would never be on screen long enough to read. It has to be live.
///
/// Live the naive way is worse than useless: "1" is out of band on the way to
/// "105.1", and a warning that fires on the first keystroke of most valid entries
/// is a warning drivers learn to ignore. That exact failure is on the record here —
/// it is why the old card made the refusal state rather than a predicate.
///
/// THE FIRST CUT OF THIS RULE WAS WRONG TOO, in the other direction. It asked
/// whether some in-band dial's DISPLAY STRING starts with the buffer — which
/// contradicts the commit it warns about, because `numpad_commit` parses and
/// ROUNDS: "87.46" commits to 87.5 and no display string starts with "87.46", so
/// the line said "Outside the band" over a buffer TUNE then tuned. And "87.4"
/// warned even though appending a digit rescues it. So the rule now asks the only
/// two functions whose answer matters: does `numpad_commit` take this buffer, or
/// any extension of it that `numpad_press` would actually let the driver type?
/// The search IS the keypad's grammar — at most four digits and one point, a few
/// thousand nodes at worst — so the three functions cannot disagree by
/// construction. `committable_buffers_never_warn` proves the whole space.
fn entry_can_tune(buf: &str) -> bool {
    if numpad_commit(buf).is_some() {
        return true;
    }
    ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "."].iter().any(|k| {
        let next = numpad_press(buf, k);
        next != buf && entry_can_tune(&next)
    })
}

/// What the buffer tunes to, or `None` if it tunes to nothing.
///
/// CarFM's `commit` (`Numpad.tsx:56-60`): parse, ROUND TO A TENTH, then check the
/// band. The rounding is the part that was missing — "105.55" tuned 105.55 here
/// and 105.6 in CarFM, and a dial is a tenth or it is not a dial.
///
/// `Math.round` breaks ties toward +∞ and Rust's `round` away from zero; every
/// value that reaches this is positive, so the two agree. It is written down
/// because that is only true while the band floor stays above zero.
fn numpad_commit(buf: &str) -> Option<f32> {
    let parsed = buf.parse::<f32>().ok().filter(|v| v.is_finite())?;
    let dial = (parsed * 10.0).round() / 10.0;
    (FM_LO..=FM_HI).contains(&dial).then_some(dial)
}

/// `from` is the strip position the caller read BEFORE any of the vendor's own
/// traffic could move it — see the `PanelKey` arm of `apply_event`, which is the
/// only caller that matters and reads it out of the same borrow that receives
/// the key.
fn panel_action(
    key: Option<crate::android::PanelKey>,
    action: &str,
    from: i32,
) -> Option<PanelAction> {
    use crate::android::PanelKey as K;
    if action.eq_ignore_ascii_case("up") {
        return None;
    }
    match key? {
        // The service has ALREADY stepped its own hardware preset bank and a
        // broadcast cannot be cancelled, so this reasserts the strip the driver
        // can actually see.
        K::PresetNext => Some(PanelAction::Step { dir: 1, from }),
        K::PresetPrev => Some(PanelAction::Step { dir: -1, from }),
        K::SearchUp | K::SeekUp => Some(PanelAction::Seek(true)),
        K::SearchDown | K::SeekDown => Some(PanelAction::Seek(false)),
        // BAND, AMS and INTRO are not refused so much as absent: this app is
        // FM-only and has no auto-store or scan-preview. The honest answer is
        // the diagnostics line and nothing else — inventing a behaviour for a
        // button whose meaning the driver already knows would be worse.
        K::ChangeBand | K::ChangeFmBand | K::ChangeAmBand | K::Ams | K::Intro => None,
    }
}

/// The FCC row on a dial: the NEAREST full-power station on that frequency, if
/// one is close enough to be the station actually being received.
///
/// POSITION IS NOT OPTIONAL HERE, and getting that wrong is the whole point of
/// this function. 88.7 MHz has 178 full-power licensees across the United
/// States; taking the first row the table returns puts a Kentucky call sign on a
/// Wisconsin hero, which is precisely the class of error CarFM's
/// `stationIdentify` exists to prevent. With no fix there is nothing to
/// disambiguate against, so NOTHING resolves and the face prints the frequency
/// as the identity — never an inaccurate name and never "Tuning…".
///
/// The cap is the picker's own radius. Beyond it a co-channel licensee is not
/// what the speakers are carrying, and naming it would be a guess wearing a call
/// sign.
///
/// `at_frequency` excludes translators and LPFM, so a dial carrying only a
/// translator resolves to nothing — a translator's call sign is not the station
/// anyone is listening to.
fn resolve(db: Option<&StationDb>, mhz: f32, at: Option<(f64, f64)>) -> Option<StationRow> {
    let (lat, lon) = at?;
    let rows = db?.at_frequency(f64::from(mhz)).ok()?;
    rows.into_iter()
        .map(|r| {
            let km = crate::stations::haversine_km(lat, lon, r.lat, r.lon);
            (r, km)
        })
        .filter(|(_, km)| *km <= crate::stations::NEARBY_RADIUS_KM)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(r, _)| r)
}

/// Metres between two fixes, near enough for a threshold.
///
/// EQUIRECTANGULAR, NOT HAVERSINE, and that is not a corner cut: over the few
/// hundred metres this is asked about, the two agree to far better than the
/// threshold's own precision, and this is on the path every GPS fix takes.
fn metres_between(a: (f64, f64), b: (f64, f64)) -> f64 {
    const M_PER_DEG_LAT: f64 = 111_320.0;
    let dlat = (a.0 - b.0) * M_PER_DEG_LAT;
    let dlon = (a.1 - b.1) * M_PER_DEG_LAT * a.0.to_radians().cos();
    (dlat * dlat + dlon * dlon).sqrt()
}

fn build_picker(
    db: Option<&StationDb>,
    loc: fake::FakeLocation,
    snapshot: Option<String>,
) -> NearbyPicker {
    match (db, loc.position()) {
        (Some(db), Some((lat, lon))) => {
            NearbyPicker::query(db, lat, lon).unwrap_or_else(|_| NearbyPicker::no_fix(snapshot))
        }
        _ => NearbyPicker::no_fix(snapshot),
    }
}

/// Reconcile a restored RDS snapshot against the dial the radio actually
/// reports, and say whether the snapshot belongs here.
///
/// The restore in `with_tuner` is optimistic — it is applied before the tuner
/// has said a word, so the face is warm on the first frame rather than a second
/// later. This is the other half of that bargain: the FIRST thing the radio says
/// about its own dial either confirms the snapshot or destroys it. A snapshot is
/// a picture of one station, and the one way it can lie is by being shown over a
/// different one.
///
/// Through `preset_key` for the same reason `active_index` uses it: 105.5 does
/// not round-trip through an f32.
fn settle_warm(s: &mut State, mhz: f32) -> bool {
    let Some(warm) = s.warm_dial else { return false };
    if crate::stations::preset_key(f64::from(warm)) == crate::stations::preset_key(f64::from(mhz)) {
        return true;
    }
    s.warm_dial = None;
    s.rds_state = RdsState::default();
    s.last_rds_at = None;
    s.rds_stale = false;
    false
}

/// Which slot a dial sits on, or -1.
///
/// Through `preset_key` rather than `==` on two f32s: 105.5 does not round-trip
/// through an f32, so a float comparison here is a coin toss on exactly the
/// values a preset strip is made of.
fn active_index(dial: f32, presets: &[Slot]) -> i32 {
    let key = crate::stations::preset_key(f64::from(dial));
    presets
        .iter()
        .position(|p| crate::stations::preset_key(f64::from(p.mhz)) == key)
        .map_or(-1, |i| i as i32)
}

/// [`State::anchor`]'s rule, as a free function so it can be tested without a
/// window: a whole `State` needs a Slint `AppWindow`, a database and a tuner to
/// build, and this is the one line that decides where every wheel press starts
/// from.
fn step_anchor(asserted: Option<f32>, dial: f32, presets: &[Slot]) -> i32 {
    active_index(asserted.unwrap_or(dial), presets)
}

/// The preset chip's ladder box, in dp.
///
/// CarFM's own number for a fill-mode plate (`LogoTile.tsx:337`, `boxDp = fill ?
/// 128 : …`), and Carnyx's tiles are all fill-mode — `PresetPlate` is given
/// `fill: true` on every track. 128 is also the bottom rung of `SIZE_LADDER`, so
/// a chip decodes a 128 px PNG rather than a 512 px one.
const TILE_BOX_DP: f32 = 128.0;

fn to_preset(slot: &Slot, logo: Option<(slint::Image, LogoPlate)>) -> Preset {
    let call = slot.call();
    Preset {
        name: slot.name().into(),
        call: call.as_str().into(),
        // The colour hashes from the CORE letters, so `WWHG` and `WWHG-FM` are
        // one station and get one fill.
        brand: brand_color(&call),
        freq_mhz: slot.mhz,
        freq_label: format_mhz(slot.mhz).into(),
        has_logo: logo.is_some(),
        plate: logo.as_ref().map_or(LogoPlate::Light, |(_, p)| *p),
        logo: logo.map(|(i, _)| i).unwrap_or_default(),
    }
}

/// A registry colour that may be "unstated".
///
/// Three of the five themes give their genre line the live `pal.dim` rather than
/// a colour of their own, and the registry writes that as the token itself. Here
/// it is a zero, and a zero has to reach the face as TRANSPARENT so `GenreText`
/// falls through — painting it black would be reading "leave it alone" as an
/// instruction.
fn nonzero(packed: u32) -> Option<u32> {
    (packed != 0).then_some(packed)
}

/// `nameGhost` as one colour: the registry states these as `rgba()`, so the ink
/// and the alpha arrive separately and are put back together here.
fn ghost(e: &crate::eggs::Egg) -> slint::Color {
    if e.ghost_alpha <= 0.0 {
        return slint::Color::from_argb_u8(0, 0, 0, 0);
    }
    let c = rgb(e.ghost_ink);
    slint::Color::from_argb_u8(
        (e.ghost_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
        c.red(),
        c.green(),
        c.blue(),
    )
}

/// One ring of a `cardFrame`, or `None` for an unused slot.
///
/// FOUR FIXED SLOTS rather than a model, because a Slint struct cannot carry a
/// list and the frame is an ornament on one card — the reference's own frame has
/// exactly four rules and no theme has ever had a different count. A model would
/// be a repeater and a second component for four numbers.
fn ring(skin: crate::eggs::Skin, i: usize) -> Option<(u32, f32)> {
    skin.card_rings.get(i).copied()
}

/// A read's answer as the face's own vocabulary.
///
/// TWO ENUMS FOR ONE FACT, and deliberately: `logos::assign::Backing` belongs to
/// a module that has never heard of Slint — that separation is what lets the
/// whole pipeline run in a unit test with no crates at all — and `LogoPlate` is
/// generated from `types.slint`. This is the one place they meet.
fn plate_of(b: crate::logos::assign::Backing) -> LogoPlate {
    use crate::logos::assign::Backing;
    match b {
        Backing::Light => LogoPlate::Light,
        Backing::Fallback => LogoPlate::Fallback,
        Backing::Bare => LogoPlate::Bare,
        Backing::Plate => LogoPlate::Plate,
    }
}

fn strings(v: &[String]) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(
        v.iter().map(SharedString::from).collect::<Vec<_>>(),
    )))
}

/// A wall-clock stamp for the diagnostics log.
///
/// `HH:MM:SS` off the system clock, in UTC — there is no timezone database in
/// this crate and a wrong local time would be worse than an honest one.
fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}

/// Where the station database lives on the host: straight out of the source
/// tree, because there is no APK and therefore no asset to extract.
///
/// On Android this is `stations::install(files_dir, || <AAssetManager read>)` —
/// see `crate::stations::install`, which exists because a release APK deflates
/// the asset and a deflated asset has no file descriptor to hand SQLite.
pub fn host_db_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/db/stations.sqlite")
}

/// Where the HOST keeps preferences.
///
/// Under `target/`, deliberately: `assets/` is shipped verbatim into the APK by
/// cargo-apk, so anything written beside the station database would be packaged
/// and handed to every driver. `target/` is already ignored and already
/// disposable.
pub fn host_prefs_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/carnyx-host")
}

/// Where a station's art lives: one directory per call-sign base, under the same
/// private directory as `prefs.json`.
///
/// A SUBDIRECTORY, and it is not tidiness. `LogoStore` keeps its own hero flags
/// in a file called `prefs.json` at its root (`store::prefs_path`) — the same
/// name [`crate::prefs::FILE`] uses. Rooted at the same directory the two would
/// be ONE FILE, and whichever wrote last would silently destroy the other's
/// contents.
pub fn logo_dir(prefs_dir: &Path) -> PathBuf {
    prefs_dir.join("logos")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `App` on a real window, for the tests that need the whole path.
    ///
    /// WHY THIS DID NOT EXIST, and what it costs. Everything from the panel key
    /// to the step — `drain_events`' ordering, `apply_panel_action`,
    /// `step_preset_from`, `tune` — was reachable only through an `AppWindow`,
    /// and nothing in this module built one. So `cargo test` never once stepped a
    /// preset or drained the queue, and every defect found in that region this
    /// week shipped green: the step reading its origin off the vendor's dial, the
    /// nested-drain overwrite of `asserted`, and a release-edge guard asserted
    /// against a string the device cannot send.
    ///
    /// ONE PLATFORM PER PROCESS, hence the `Once`. Slint's platform is global and
    /// `set_platform` fails on a second call, while the test harness runs tests on
    /// several threads. The tests below therefore take a MUTEX and share one
    /// platform: they are the only tests in this crate that build a window, so
    /// serialising them costs nothing and keeps every window on one thread.
    #[cfg(test)]
    mod harness {
        use std::rc::Rc;
        use std::sync::{Mutex, MutexGuard, OnceLock};

        use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
        use slint::platform::{Platform, WindowAdapter};
        use slint::PlatformError;

        struct Headless;

        impl Platform for Headless {
            /// A FRESH ADAPTER EVERY TIME, unlike the probes in `examples/`,
            /// which keep one because they render from it. Handing the same
            /// `MinimalSoftwareWindow` to a second `AppWindow` in one process
            /// fails: the tests below passed one at a time and the second one to
            /// run failed in `AppWindow::new` when both ran together.
            fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
                Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
            }
        }

        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        /// Hold this for the length of a UI test.
        ///
        /// SET EVERY TIME, NOT ONCE. Slint's platform is PER-THREAD, and the test
        /// harness runs tests on several threads — so a `Once` set it for
        /// whichever thread got there first and every other test fell through to
        /// the real winit backend and died with "neither WAYLAND_DISPLAY nor
        /// WAYLAND_SOCKET nor DISPLAY is set". Each thread installs its own; the
        /// repeat call on a thread that already has one returns an error and is
        /// ignored.
        ///
        /// The mutex is still worth having: it keeps these tests from building
        /// windows concurrently, which is not a shape this tree has ever run.
        pub fn ui_lock() -> MutexGuard<'static, ()> {
            let guard = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let _ = slint::platform::set_platform(Box::new(Headless));
            guard
        }
    }

    /// Build an app on a scratch prefs directory, named per test so two cannot
    /// collide on disk.
    fn app_for(tag: &str) -> (AppWindow, Rc<App>) {
        let dir = std::env::temp_dir().join(format!("carnyx-apptest-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ui = AppWindow::new().expect("window");
        let driver = App::with_tuner(
            &ui,
            &host_db_path(),
            &dir,
            Box::new(crate::android::FakeTuner::new()),
            false,
            None,
            fake::FakeLocation::default(),
        );
        (ui, driver)
    }

    /// EVERY SWITCH IN THE PANEL REACHES THE DISK.
    ///
    /// `Prefs` carries five settings and `App::with_tuner` reads all five back at
    /// launch, so every one of them is meant to survive — and NOT ONE of the five
    /// callbacks that move them writes anything. They persist because each ends
    /// in `push_settings`, whose last statement is `save_prefs`.
    ///
    /// WRITTEN BECAUSE THAT ARRANGEMENT LOOKS BROKEN FROM THE CALLBACK'S SIDE and
    /// was read that way once, during a bug sweep, on the way to adding four
    /// redundant saves. The file is the only place the answer is unambiguous, so
    /// this asserts against the file rather than the model — which also makes it
    /// the test that fails if anyone ever takes `save_prefs` out of
    /// `push_settings` and fixes up only the callers they can see.
    ///
    /// The chip label matters: `Theme::parse` matches the panel's own upper-case
    /// label and leaves the choice alone on anything else, so a lower-case string
    /// here would change nothing and pass against a genuinely broken save.
    #[test]
    fn every_remembered_setting_is_written_when_it_changes() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("persist");
        let dir = std::env::temp_dir().join("carnyx-apptest-persist");
        driver.drain_events();

        // A fresh directory has no file at all until something writes one. The
        // chip label is what the panel passes and `Theme::parse` matches, and it
        // is upper case — a lower-case string here parses to nothing, changes
        // nothing, and would make this test pass against a broken save.
        ui.invoke_settings_set_theme(settings::Theme::Dark.label().into());
        let after_theme = crate::prefs::load(&dir);
        assert_eq!(after_theme.theme, settings::Theme::Dark, "the theme reaches the disk");

        ui.invoke_settings_set_logos(false);
        assert!(!crate::prefs::load(&dir).logos_on, "and so does the logo switch");

        ui.invoke_settings_set_diag(true);
        assert!(crate::prefs::load(&dir).diag_on, "and the diagnostics switch");

        let i = settings::Source::ORDER.iter().position(|s| *s == settings::Source::Rtl).unwrap();
        ui.invoke_settings_pick_source(i as i32);
        assert_eq!(
            crate::prefs::load(&dir).selected,
            settings::Source::Rtl,
            "and the source picker"
        );

        // The one that was already right, kept here so a future edit cannot
        // quietly take it back with the others.
        ui.invoke_settings_set_release_on_sleep(false);
        assert!(!crate::prefs::load(&dir).release_on_sleep, "and the sleep release");
    }

    /// THE RELEASE SWITCH IS PUSHED DOWN, AT START-UP AND ON EVERY MOVE.
    ///
    /// `SleepReceiver` is a MANIFEST receiver, and the reason it exists is that
    /// the process may already be dead when ACC-off is broadcast — so when it
    /// runs there is no settings file read, no Rust and no `Settings` to consult.
    /// The only thing it can read is the copy `CarnyxWake.setReleaseOnSleep`
    /// leaves in shared preferences, and the only thing that writes that copy is
    /// this push. A push that never happens leaves the receiver releasing the
    /// source on a unit whose driver turned the feature off.
    ///
    /// NOTHING ELSE IN THIS FILE WOULD CATCH IT. `Tuner::set_release_on_sleep`
    /// has an empty default body, so a missing call and a working one are
    /// indistinguishable from every other test — which is why the fake records
    /// into a static and this reads it back.
    #[test]
    fn the_release_switch_reaches_the_receiver_that_cannot_ask() {
        let _ui_lock = harness::ui_lock();
        crate::android::clear_release_mirror();

        // START-UP pushes the restored value, before the watch is armed.
        let (ui, driver) = app_for("release-mirror");
        assert_eq!(
            crate::android::last_release_mirror(),
            Some(true),
            "start-up pushes the restored switch, which defaults on"
        );

        // AND EVERY MOVE pushes the new one.
        ui.invoke_settings_set_release_on_sleep(false);
        assert_eq!(
            crate::android::last_release_mirror(),
            Some(false),
            "turning it off reaches the receiver too"
        );
        ui.invoke_settings_set_release_on_sleep(true);
        assert_eq!(crate::android::last_release_mirror(), Some(true), "and back on");
        drop(driver);
    }

    /// THE SLEEP WATCH IS ARMED AT START-UP, NOT INSIDE `connect`.
    ///
    /// `NwdBridge.startSleepWatch` was called from `connect()`, after
    /// `bindService` returned true. A unit whose vendor service refuses the bind
    /// therefore registered no receiver at all — and the `sleep:` line is the
    /// only evidence of WHICH broadcast fires, on the session most worth reading.
    /// It is the illumination bug, which this file already carries a comment
    /// about, repeated one line away from that comment.
    ///
    /// `FakeTuner::push_sleep` is a no-op until the watch is armed, exactly as
    /// `push_illumination` is, so this fails if the arming is ever moved back.
    /// Going through the fake rather than `ingest_sleep` is the whole point: the
    /// ingest edge would deliver the event either way and prove nothing.
    #[test]
    fn the_sleep_watch_does_not_depend_on_the_tuner_binding() {
        let _ui_lock = harness::ui_lock();
        let (_ui, driver) = app_for("sleepwatch");
        driver.drain_events();

        // Through the TUNER, which stays silent unless someone armed the watch.
        let tuner = driver.state.borrow().tuner.clone();
        tuner.push_sleep_for_test("com.nwd.ACTION_ACCOFF_UPDATE");
        driver.drain_events();

        let s = driver.state.borrow();
        assert!(!s.audio, "the source went back, so the watch was armed");
        assert!(
            s.settings.log.lines().iter().any(|l| l.contains("sleep: com.nwd.ACTION_ACCOFF_UPDATE")),
            "and the broadcast is named in the log"
        );
    }

    /// THE IGNITION GOING OFF HANDS THE FM SOURCE BACK, AND IS NOT A POWER-OFF.
    ///
    /// The MCU sleeps the SoC on ACC-off and restores its own radio app on
    /// ACC-on, so a unit left on FM comes back into FM and the stock radio app
    /// launches itself. Handing the source back is what stops that.
    ///
    /// ASSERTED THROUGH THE TUNER'S OWN SNAPSHOT rather than through a flag on
    /// this side: `FakeTuner` reports `mcu_source` as 4 while it holds the source
    /// and 0 once it does not, so this proves `set_audio_enabled(false)` actually
    /// crossed the seam rather than that Carnyx merely decided it had.
    ///
    /// AND `user_powered_off` MUST NOT MOVE. That flag is the driver's own choice
    /// at the power button; the ignition going off is nobody's choice, and a
    /// sleep that set it would be a face that came back dead for a reason the
    /// driver never gave.
    #[test]
    fn sleep_hands_the_source_back_and_is_not_a_power_off() {
        let _ui_lock = harness::ui_lock();
        let (_ui, driver) = app_for("sleep");
        driver.drain_events();
        assert_eq!(
            driver.state.borrow().tuner.snapshot().unwrap().mcu_source,
            Some(4),
            "the app holds the source before the unit sleeps"
        );
        assert!(driver.state.borrow().audio);

        crate::android::ingest_sleep("com.nwd.ACTION_ACCOFF_UPDATE".into(), String::new());
        driver.drain_events();

        let s = driver.state.borrow();
        assert_eq!(
            s.tuner.snapshot().unwrap().mcu_source,
            Some(0),
            "and hands it back when the unit says it is going down"
        );
        assert!(!s.audio, "the face knows it no longer holds it");
        assert!(!s.user_powered_off, "an ignition cycle is not the driver powering off");

        // BOTH LINES, because which trigger fired is the open question this path
        // exists to settle on the unit — `ACTION_ACCOFF_UPDATE` or `SCREEN_OFF`.
        let lines = s.settings.log.lines();
        assert!(
            lines.iter().any(|l| l.contains("sleep: com.nwd.ACTION_ACCOFF_UPDATE")),
            "the log names the broadcast verbatim, got {lines:?}"
        );
        // The QUEUED release, which on the host is the only one there is — the
        // release that matters on the unit runs in `NwdBridge.releaseSource` on
        // the receiver's own thread and reaches this side as the event's
        // `release` field, which a fake leaves empty.
        assert!(
            lines.iter().any(|l| l.contains("sleep: FM release re-sent from the drain")),
            "and records that the release ran, got {lines:?}"
        );
        drop(s);

        // ── AND THE SWITCH REALLY SWITCHES IT OFF ─────────────────────────────
        //
        // Default ON, so the half above is the shipped behaviour. Off must leave
        // the source alone — and must STILL LOG the broadcast, because which one
        // arrives is the open question and is worth answering on a unit where the
        // driver does not want the release.
        driver.set_audio(true);
        assert_eq!(driver.state.borrow().tuner.snapshot().unwrap().mcu_source, Some(4));
        driver.state.borrow_mut().settings.release_on_sleep = false;

        crate::android::ingest_sleep("android.intent.action.SCREEN_OFF".into(), String::new());
        driver.drain_events();

        let s = driver.state.borrow();
        assert_eq!(
            s.tuner.snapshot().unwrap().mcu_source,
            Some(4),
            "with the switch off the source is kept"
        );
        assert!(s.audio, "and the face still holds it");
        let lines = s.settings.log.lines();
        assert!(
            lines.iter().any(|l| l.contains("SCREEN_OFF") && l.contains("release is off")),
            "the broadcast is still recorded, and says why nothing happened: {lines:?}"
        );
        assert!(
            !lines.iter().skip_while(|l| !l.contains("SCREEN_OFF")).any(|l| l.contains("released for sleep")),
            "and nothing was released after it: {lines:?}"
        );
    }


    /// Pretend the vendor service retuned its own bank and reported it.
    ///
    /// `FakeTuner`'s scale is 100, and slot -1 is "not a preset in this bank",
    /// which is what the tuner sends for an ordinary retune.
    fn vendor_reports(mhz: f32) {
        crate::android::ingest_frequency(0, (mhz * 100.0).round() as i32, String::new(), -1);
    }

    /// WHAT THE POP-UP SAYS, AND WHEN IT KEEPS QUIET.
    ///
    /// The posting itself is Android's and cannot run here — `announce_station`
    /// is a no-op off the unit — but the RULE is not platform work and this is
    /// where it is settled: which dial changes are worth telling a driver about,
    /// and which the driver can already see.
    #[test]
    fn the_station_pop_up_speaks_only_for_a_change_the_driver_cannot_see() {
        let _ui_lock = harness::ui_lock();
        let (_ui, driver) = app_for("popup");
        let strip = fake::SEED_PRESET_MHZ;
        let said = |driver: &Rc<App>| -> Vec<String> {
            driver
                .state
                .borrow()
                .settings
                .log
                .lines()
                .into_iter()
                .filter(|l| l.contains("station pop-up:"))
                .collect()
        };

        // ── IN FRONT: the face is the answer, so nothing is announced.
        crate::android::set_foreground(true);
        driver.tune_for_test(strip[1]);
        driver.drain_events();
        assert!(said(&driver).is_empty(), "a tune the driver is watching says nothing");

        // ── THE STALE-MARKER BUG, and the reason the marker moves even when
        // nothing is posted. Switching away after tuning by hand must stay
        // silent: the dial has not moved since, and an ordinary push — an RDS
        // group, a level read — must not announce a station the driver chose
        // themselves a minute ago.
        crate::android::set_foreground(false);
        driver.push_all();
        assert!(said(&driver).is_empty(), "switching away is not a station change");

        // ── AWAY: now the dial moves, and this is the whole feature.
        driver.tune_for_test(strip[2]);
        driver.drain_events();
        let lines = said(&driver);
        assert_eq!(lines.len(), 1, "one change, one line: {lines:?}");
        assert!(
            lines[0].contains(&format_mhz(strip[2]).to_string()),
            "the line names the dial it landed on: {lines:?}"
        );

        // ── AND ONCE PER CHANGE. Every RDS group and every level read pushes the
        // hero again; none of them is a station change.
        driver.push_all();
        driver.push_all();
        assert_eq!(said(&driver).len(), 1, "a redraw is not a retune");

        // ── AND THE LOGO BRANCH. A station with a saved mark sends the mark
        // and no words; the line records which was sent, so the wiring from
        // `notification_logo` through to the announcement is covered here and
        // not only in `path_for_theme`'s own test.
        assert!(lines[0].contains("(no logo)"), "nothing is saved for this one yet: {lines:?}");

        let base = driver.hero_row().map(|r| r.callsign_base).unwrap_or_default();
        assert!(!base.is_empty(), "the seeded strip resolves to a real station");
        driver
            .state
            .borrow()
            .store
            .put_original(&base, b"a picture", "image/png", "manual")
            .unwrap();
        driver.tune_for_test(strip[3]);
        driver.drain_events();
        driver.tune_for_test(strip[2]);
        driver.drain_events();
        let lines = said(&driver);
        assert!(
            lines.last().unwrap().contains("(logo)"),
            "a station with a master sends the picture: {lines:?}"
        );

        // Leave the flag as every other test expects to find it.
        crate::android::set_foreground(true);
    }

    /// THE WHOLE WHEEL PATH, from the broadcast to the dial it lands on.
    ///
    /// This is the test that did not exist. `examples/wheelprobe.rs` covers the
    /// same ground in more detail and with more cases, but it is an example: it
    /// compiles under `cargo test` and never runs. This runs.
    #[test]
    fn a_wheel_press_steps_from_where_the_app_put_the_strip() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("wheel");
        let strip = fake::SEED_PRESET_MHZ;
        let label = |ui: &AppWindow| ui.get_freq_label().to_string();

        // ── The ordinary case: a silent tuner, one press, one step.
        driver.tune_for_test(strip[1]);
        driver.drain_events();
        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();
        assert_eq!(label(&ui), format_mhz(strip[2]).to_string(), "next from entry 1");

        // ── THE DEFECT THE DRIVER REPORTED. One wheel press makes the vendor
        // service step its OWN hardware preset bank and report the frequency it
        // landed on, and `drain_events` applies the deferred key only once that
        // queue is empty. Reading the strip position at that point read the
        // VENDOR'S dial: off-strip it resolved to -1 and every press went to
        // entry 0, which is "next goes backwards".
        driver.tune_for_test(strip[1]);
        driver.drain_events();
        crate::android::ingest_panel_key(62, "down".into());
        vendor_reports(99.9);
        driver.drain_events();
        assert_eq!(
            label(&ui),
            format_mhz(strip[2]).to_string(),
            "the vendor's own bank walk must not move the strip"
        );

        // ── And the same fault a drain later, which the key-time anchor cannot
        // reach and `State::asserted` is what covers.
        driver.tune_for_test(strip[3]);
        driver.drain_events();
        vendor_reports(99.9);
        driver.drain_events();
        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();
        assert_eq!(
            label(&ui),
            format_mhz(strip[4]).to_string(),
            "a vendor report between presses must not move the strip either"
        );

        // ── TWO KEYS IN ONE DRAIN ARE TWO STEPS, and this assertion used to say
        // the opposite: "one step, not two", because `panel_action` was one
        // Option and the newer key overwrote the older.
        //
        // That was written against an open question — whether the MCU sends a
        // release broadcast as well as a press, in which case collapsing them was
        // the only thing standing between one push and a double step. A drive log
        // has now answered it. The panel line records the gap since the last
        // identical key, and across twenty presses the SHORTEST was 203ms, with
        // the rest spread from 417ms to 2969ms. A press and its own release are
        // tens of milliseconds apart; 203ms is a person pressing twice. Twenty
        // presses also produced eighteen steps, not forty. This fascia sends ONE
        // broadcast per press, so there is no release edge to swallow — and
        // swallowing was costing the driver every press made inside the ~680ms a
        // step takes on the unit.
        driver.tune_for_test(strip[0]);
        driver.drain_events();
        let real = "com.nwd.action.ACTION_KEY_VALUE";
        crate::android::ingest_panel_key(62, real.into());
        crate::android::ingest_panel_key(62, real.into());
        driver.drain_events();
        assert_eq!(
            label(&ui),
            format_mhz(strip[2]).to_string(),
            "two presses in one drain move two positions"
        );

        // AND A LONGER BURST, because the arithmetic is what changed and one
        // extra press only proves addition once. Five keys from entry 0 land on
        // entry 5 of a six-entry strip, in a single step rather than five.
        driver.tune_for_test(strip[0]);
        driver.drain_events();
        for _ in 0..5 {
            crate::android::ingest_panel_key(62, real.into());
        }
        driver.drain_events();
        assert_eq!(
            label(&ui),
            format_mhz(strip[5]).to_string(),
            "five presses move five positions"
        );

        // AND THEY CANCEL. A next and a prev in the same drain is a driver who
        // changed their mind; summing the directions is what makes that land
        // where they started instead of somewhere neither key asked for.
        driver.tune_for_test(strip[2]);
        driver.drain_events();
        crate::android::ingest_panel_key(62, real.into());
        crate::android::ingest_panel_key(63, real.into());
        driver.drain_events();
        assert_eq!(
            label(&ui),
            format_mhz(strip[2]).to_string(),
            "a next and a prev together go nowhere"
        );
    }

    /// THE FACE DOES NOT FLICK THROUGH THE VENDOR'S OWN BANK.
    ///
    /// > "initial selection is a bit erratic, you can see it jump to the wrong
    /// > station for a moment before going to the correct one"
    ///
    /// One wheel press makes the vendor service walk its OWN hardware preset
    /// bank, and every frequency it passes through is reported. `asserted`
    /// stopped those reports moving where the next STEP starts from; this is the
    /// other half — they must not be DISPLAYED either.
    #[test]
    fn a_vendor_bank_walk_is_not_shown_on_the_hero() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("hold");
        let strip = fake::SEED_PRESET_MHZ;

        driver.tune_for_test(strip[1]);
        driver.drain_events();
        assert_eq!(ui.get_freq_label().to_string(), format_mhz(strip[1]).to_string());

        // THE DEVICE REPORTS ASYNCHRONOUSLY, and the fake must too for this
        // test to mean anything. `NwdBridge.tune` calls `setCurrentFrequency`
        // and returns; the frequency comes back later as
        // `notifyCurrentFrequency`. With the fake's default synchronous echo the
        // hold is released by our OWN tune before the vendor has said a word,
        // which is the fake collapsing the very window under test.
        driver.set_echo_for_test(false);

        // The press, and the vendor's transit frequency arriving with it.
        crate::android::ingest_panel_key(62, "down".into());
        vendor_reports(99.9);
        driver.drain_events();
        driver.push_all();
        assert_eq!(
            ui.get_freq_label().to_string(),
            format_mhz(strip[2]).to_string(),
            "the hero shows the target, never the frequency the vendor passed through"
        );

        // A SECOND transit report, still not the target: the face holds.
        vendor_reports(93.1);
        driver.drain_events();
        driver.push_all();
        assert_eq!(
            ui.get_freq_label().to_string(),
            format_mhz(strip[2]).to_string(),
            "and it keeps holding while the vendor walks"
        );

        // The tuner confirms the target, and the hold lifts.
        vendor_reports(strip[2]);
        driver.drain_events();
        driver.push_all();
        assert_eq!(ui.get_freq_label().to_string(), format_mhz(strip[2]).to_string());

        // ── AND ARRIVING DOES NOT END THE HOLD, which this used to assert the
        // opposite of: it expected a vendor report after the settle to be shown
        // at once, "with nothing in flight the face is honest again".
        //
        // A drive log showed what that cost. The vendor's bank walk is a retune
        // of its own and it can land AFTER the report confirming ours — the log
        // has "settled on 94.9" and then, with the hold already released, "tuned
        // 104.1", uncorrected, because releasing had disarmed the defence at the
        // moment the attack arrived. That is the "strange move" the driver saw.
        //
        // So a settled hold keeps standing until `HOLD_CAP` and keeps defending.
        // Nothing is concealed by that: the face shows the station the driver
        // asked for, the radio is being held there, and a tune the DRIVER makes
        // drops the hold in `tune` before it could misrepresent one.
        driver.set_echo_for_test(true);
        vendor_reports(93.1);
        driver.drain_events();
        driver.push_all();
        assert_eq!(
            ui.get_freq_label().to_string(),
            format_mhz(strip[2]).to_string(),
            "a late vendor move is ridden out, not displayed"
        );
        assert_eq!(
            driver.tuned_mhz_for_test().map(format_mhz),
            Some(format_mhz(strip[2])),
            "and the radio is taken back to the target"
        );
    }

    /// THE AC/DC THEME REACHES THE FACE, and leaves again.
    ///
    /// `eggs::tests` covers the matcher as a leaf — which strings match and which
    /// adverts must not. This is the other half: that a match actually dresses the
    /// window, that the call sign is cut where the bolt goes, and that a track
    /// change takes all of it away again. A theme that arrives and will not leave
    /// is worse than one that never arrives.
    #[test]
    fn the_acdc_theme_dresses_the_face_and_reverts() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("egg-acdc");
        driver.push_all();
        assert!(!ui.get_egg().on, "no theme with nothing playing");

        driver.set_radio_text_for_test("AC/DC - Back in Black");
        driver.push_all();
        let egg = ui.get_egg();
        assert!(egg.on, "AC/DC in the RadioText dresses the face");
        assert_eq!(egg.genre.to_string(), "High Voltage Rock 'n' Roll");
        assert!(egg.horns && egg.call_bolt && egg.suppress_logo);

        // THE CALL SIGN IS CUT WHERE THE BOLT STANDS. The halves must rejoin to
        // exactly what the hero shows — a split that loses or duplicates a letter
        // is a misspelt station, which is worse than no Easter egg.
        let ident = ui.get_ident().to_string();
        assert!(!ident.is_empty(), "the seeded dial resolves to a call sign");
        let (a, b) = (ui.get_ident_a().to_string(), ui.get_ident_b().to_string());
        assert_eq!(format!("{a}{b}"), ident, "the halves rejoin to the whole");
        assert!(!a.is_empty() && !b.is_empty(), "the bolt stands between two halves");
        assert!(a.chars().count() >= b.chars().count(), "an odd count leaves the longer half first");

        // AND IT LEAVES THE INSTANT THE TRACK DOES. §12: a skin that outlives its
        // track is a skin stuck on.
        driver.set_radio_text_for_test("Nicolet Law - Injured? Get Nicolet!");
        driver.push_all();
        assert!(!ui.get_egg().on, "a different track takes the theme away");
        assert_eq!(ui.get_ident_a().to_string(), "", "and the split is put back");
    }

    /// THE TIER REACHES THE FACE, which is the only thing that reads it.
    ///
    /// `EggTheme.advanced` is what `GenreText` sizes the line from: an advanced
    /// theme may state its own sizes, a basic one takes the ordinary ones. The
    /// two tiers are NOT distinguishable by looking at the other fields — a
    /// basic row stating a genre is, field for field, an advanced row that
    /// happens to state only a genre — so if the flag does not travel, nothing
    /// downstream can work it out.
    ///
    /// AND IT IS FALSE WITH NO THEME SHOWING, so the unthemed PTY keeps the
    /// ordinary metrics through the same branch rather than through a second one.
    #[test]
    fn the_tier_travels_to_the_face() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("egg-tier");
        driver.push_all();
        assert!(!ui.get_egg().advanced, "no theme, no claim to the themed metrics");

        driver.set_radio_text_for_test("Nirvana - Smells Like Teen Spirit");
        driver.push_all();
        assert!(ui.get_egg().on);
        assert!(ui.get_egg().advanced, "Nirvana is one of the five");

        driver.set_radio_text_for_test("Eric Clapton - Layla");
        driver.push_all();
        let egg = ui.get_egg();
        assert!(egg.on, "a basic theme is still a theme");
        assert_eq!(egg.genre.to_string(), "Slowhand");
        assert!(!egg.advanced, "and it takes the ordinary genre metrics");
    }

    /// THE DARK-LOGO PICKER, FROM THE ASSIGN THAT OPENS IT TO THE ANSWER.
    ///
    /// The engine has shipped since #76 — `offer_dark`, `set_dark_treatment` and
    /// `choose_treatment` are all built and tested — and until now nothing could
    /// reach any of it. This asserts the SCREEN: that a new master opens the
    /// picker instead of closing the window, that the rows and the two labels
    /// arrive finished, that a tap moves the selection, and that every way out
    /// closes the window.
    ///
    /// DRIVEN THROUGH `apply_logo_event`, which is the seam the worker speaks
    /// through. The worker itself cannot run here — the host has no image codec,
    /// so `offer_dark` would have nothing to decode — and pretending otherwise
    /// would be testing a fake pipeline rather than the wiring that was missing.
    #[test]
    fn the_dark_picker_opens_on_a_new_logo_and_every_button_closes_it() {
        use crate::logos::dark::stages::Treatment;
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("dark-picker");
        let art = || crate::logos::Raster::empty(4, 4);

        // A PREFS SAVE CLOSES THE WINDOW, as it always has. Nothing was
        // downloaded, so there is nothing to ask about.
        ui.set_overlay(Overlay::LogoSearch);
        driver.apply_logo_event(service::Event::Saved {
            base: "WMGN".into(),
            assigned: false,
        });
        assert_eq!(ui.get_overlay(), Overlay::None, "a flag toggle just saves");
        assert!(driver.state.borrow().dark_pick.is_none());

        // A NEW MASTER KEEPS IT OPEN, waiting on the adaptation.
        ui.set_overlay(Overlay::LogoSearch);
        driver.apply_logo_event(service::Event::Saved {
            base: "WMGN".into(),
            assigned: true,
        });
        assert_eq!(ui.get_overlay(), Overlay::LogoSearch, "the window stays up");
        assert_eq!(ui.get_logo_search_state(), crate::LogoSearchState::DarkPick);
        assert!(!ui.get_logo_search_can_confirm(), "nothing to confirm while it adapts");

        // THE TREATMENTS ARRIVE. The badge follows `pick`, the selection follows
        // `open_on`, and they are deliberately different rows here.
        driver.apply_logo_event(service::Event::DarkChoices {
            base: "WMGN".into(),
            items: vec![(Treatment::Remap, art()), (Treatment::Halo, art()), (Treatment::Plate, art())],
            pick: Treatment::Remap,
            open_on: Treatment::Halo,
        });
        use slint::Model;
        let rows = ui.get_logo_search_dark_choices();
        assert_eq!(rows.row_count(), 3);
        assert_eq!(rows.row_data(0).unwrap().label.to_string(), "Recolor", "words, not stage names");
        assert!(rows.row_data(0).unwrap().auto, "the pipeline's pick is badged");
        assert!(!rows.row_data(1).unwrap().auto);
        assert!(rows.row_data(2).unwrap().plated, "only the plate asks for its slab");
        assert!(!rows.row_data(0).unwrap().plated);
        assert_eq!(ui.get_logo_search_selected_index(), 1, "it opens on the stored row");
        assert_eq!(ui.get_logo_search_cancel_label().to_string(), "Skip");
        assert_eq!(ui.get_logo_search_confirm_label().to_string(), "Use this");
        assert!(ui.get_logo_search_can_confirm());

        // A TAP MOVES THE SELECTION, through the same callback the results grid
        // uses — the index means a different list in each.
        ui.invoke_logo_search_pick(2);
        assert_eq!(ui.get_logo_search_selected_index(), 2);
        assert_eq!(driver.state.borrow().dark_pick.as_ref().unwrap().selected, 2);

        // "USE THIS" CLOSES, and says which treatment in the log.
        ui.invoke_logo_search_confirm();
        assert_eq!(ui.get_overlay(), Overlay::None);
        assert!(driver.state.borrow().dark_pick.is_none());
        assert!(
            driver
                .state
                .borrow()
                .settings
                .log
                .lines()
                .iter()
                .any(|l| l.contains("dark logo: WMGN set to Plate")),
            "the choice is recorded"
        );

        // THE LEFT BUTTON GOES BACK TO "Cancel" when the picker does. Nothing
        // else writes that label, so a window reopened after a picker would
        // otherwise say "Skip" over the results grid.
        ui.invoke_close_overlay();
        ui.set_overlay(Overlay::LogoSearch);
        driver.apply_logo_event(service::Event::Saved {
            base: "WMGN".into(),
            assigned: false,
        });
        assert_eq!(ui.get_logo_search_cancel_label().to_string(), "Cancel");

        // AND SO DOES THE ✕. Skip is not cancel — the logo is already saved and
        // the auto-pick already stands — but both leave by the same door.
        ui.set_overlay(Overlay::LogoSearch);
        driver.apply_logo_event(service::Event::Saved {
            base: "WMGN".into(),
            assigned: true,
        });
        assert_eq!(ui.get_logo_search_state(), crate::LogoSearchState::DarkPick);
        ui.invoke_close_overlay();
        assert!(driver.state.borrow().dark_pick.is_none(), "the ✕ puts the picker away too");

        // AN OFFER WITH NOTHING IN IT CLOSES THE WINDOW rather than showing an
        // empty picker: no master, or one that will not decode.
        ui.set_overlay(Overlay::LogoSearch);
        driver.apply_logo_event(service::Event::Saved {
            base: "WMGN".into(),
            assigned: true,
        });
        driver.apply_logo_event(service::Event::NoDarkChoices { base: "WMGN".into() });
        assert_eq!(ui.get_overlay(), Overlay::None);
        assert!(driver.state.borrow().dark_pick.is_none());
    }

    /// TURN-BY-TURN, FROM THE EVENT SEAM TO THE FACE, AND THE SWITCH THAT GATES IT.
    ///
    /// The Java half cannot run here — there is no OsmAnd in this container and
    /// no binder to bind — so what is asserted is everything from `ingest_nav`
    /// inward: the sentinels, the naming, the formatting, the log line, the
    /// expiry, and that the switch both persists and clears what is on screen.
    ///
    /// DRIVEN THROUGH `ingest_nav`, which is the function `CarnyxNav`'s native
    /// method calls. A test that wrote `State::nav` directly would pass with the
    /// event plumbing disconnected, which is the half most likely to be wrong.
    #[test]
    fn a_navigation_update_reaches_the_face_and_the_switch_gates_it() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("nav");

        assert!(!ui.get_nav_active(), "nothing received, nothing shown");
        // BOTH SWITCHES DEFAULT ON, which §4.9 states outright: "OsmAnd
        // integration (default on …)" and "Hide when the map is showing,
        // default on". The integration was off here until the handoff said
        // otherwise — see `settings::Settings::nav_on` for what turning it on
        // by default actually costs and why it is defensible.
        assert!(ui.get_settings_nav_on(), "§4.9: the integration defaults on");
        assert!(ui.get_settings_nav_hide_on_map(), "§4.9: the map suppression defaults on");
        // THE ROW SAYS WHETHER THERE IS ANYTHING TO TALK TO before it is tapped.
        assert!(
            ui.get_settings_nav_sub().to_string().contains("not installed"),
            "the host has no OsmAnd and the row says so: {}",
            ui.get_settings_nav_sub()
        );

        // A REAL TURN.
        crate::android::ingest_nav(240, 5, false);
        driver.drain_events();
        assert!(ui.get_nav_active());
        assert_eq!(ui.get_nav_turn().to_string(), "right");
        assert_eq!(ui.get_nav_distance().to_string(), "240 m");

        // THE WORDS, which are the only place a street name can come from.
        crate::android::ingest_nav_voice(
            vec!["Turn right".into()],
            vec!["Turn right onto Main Street".into()],
        );
        driver.drain_events();
        assert_eq!(ui.get_nav_instruction().to_string(), "Turn right onto Main Street");

        // AND IT IS IN THE LOG, once — the updates arrive about once a second
        // and logging each would evict a three-minute drive from the ring.
        let said = |d: &Rc<App>| -> usize {
            d.state.borrow().settings.log.lines().iter().filter(|l| l.contains("nav:")).count()
        };
        let before = said(&driver);
        crate::android::ingest_nav(240, 5, false);
        crate::android::ingest_nav(240, 5, false);
        driver.drain_events();
        assert_eq!(said(&driver), before, "an unchanged update says nothing new");

        // OSMAND'S OWN `-1, -1`: navigating, nothing to say. Active, and blank —
        // which is a state and not a gap.
        crate::android::ingest_nav(-1, -1, false);
        driver.drain_events();
        assert!(ui.get_nav_active(), "still navigating");
        assert_eq!(ui.get_nav_turn().to_string(), "");
        assert_eq!(ui.get_nav_distance().to_string(), "");

        // OFF ROUTE is its own thing: that distance is a deviation.
        crate::android::ingest_nav(75, 12, false);
        driver.drain_events();
        assert_eq!(ui.get_nav_turn().to_string(), "off route");
        assert_eq!(ui.get_nav_distance().to_string(), "75 m");

        // THE SWITCH CLEARS THE FACE IMMEDIATELY. Without this the last turn
        // would sit there until `Nav::EXPIRY`, twelve seconds after the driver
        // said stop.
        ui.invoke_settings_set_nav_on(true);
        assert!(crate::prefs::load(&driver.state.borrow().prefs_dir).nav_on, "and it persists");
        ui.invoke_settings_set_nav_on(false);
        assert!(!ui.get_nav_active(), "off means off, now");
        assert_eq!(ui.get_nav_turn().to_string(), "");
        assert!(!crate::prefs::load(&driver.state.borrow().prefs_dir).nav_on);
    }

    /// OSMAND'S MAP IN FRONT HIDES THE LAYER, AND THE DRIVER CAN SAY OTHERWISE.
    ///
    /// §4.9: "`AppInfoParams.mapVisible` reports whether OsmAnd's own map is on
    /// screen; while it is, the maneuver layer is hidden — the driver is already
    /// looking at the turn. Driver-overridable (Settings ▸ NAVIGATION ▸ Hide
    /// when the map is showing, default on)."
    ///
    /// THROUGH `nav-showing` AND NOT `nav-active`, which is the distinction the
    /// property exists for: three surfaces ask different questions of one state,
    /// and a suppression folded into `nav-active` would have the settings row
    /// reporting "not navigating" while OsmAnd is mid-route.
    #[test]
    fn osmands_own_map_hides_the_maneuver_layer_unless_the_driver_says_not_to() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("nav-map");

        // A route, with OsmAnd's map NOT in front.
        crate::android::ingest_nav(240, 5, false);
        crate::android::ingest_nav_info(crate::nav::Route {
            street: Some("Whitney Way".into()),
            turn_xml: Some("TR".into()),
            turn_metres: Some(240),
            map_visible: false,
            ..crate::nav::Route::default()
        });
        driver.drain_events();
        assert!(ui.get_nav_active(), "navigating");
        assert!(ui.get_nav_showing(), "and nothing is hiding it");
        assert!(!ui.get_nav_map_visible());

        // THE MAP COMES TO THE FRONT. The route is unchanged — only the layer goes.
        crate::android::ingest_nav_info(crate::nav::Route {
            street: Some("Whitney Way".into()),
            turn_xml: Some("TR".into()),
            turn_metres: Some(240),
            map_visible: true,
            ..crate::nav::Route::default()
        });
        driver.drain_events();
        assert!(ui.get_nav_map_visible(), "OsmAnd says its map is up");
        assert!(ui.get_nav_active(), "STILL navigating — the route did not end");
        assert!(!ui.get_nav_showing(), "and the layer is hidden");

        // AND THE ROW SAYS WHICH OF THE FIVE REASONS THIS IS. A blank strip with
        // no explanation is the failure the sub-line exists to prevent.
        //
        // THE PACKAGE IS PLANTED, because the host has no OsmAnd and "not
        // installed" is answered FIRST — correctly, since it is the truest thing
        // to say about this machine. The wording of all five branches is
        // `nav::each_reason_for_a_blank_strip_says_which_one_it_is`'s; what this
        // is checking is that `push_settings` gathers the right FACTS, which is
        // the half a unit test on `sub_line` cannot see.
        driver.state.borrow_mut().nav_package = "net.osmand.plus".to_string();
        driver.push_settings();
        assert!(
            ui.get_settings_nav_sub().to_string().contains("map is in front"),
            "got {}",
            ui.get_settings_nav_sub()
        );

        // THE DRIVER OVERRIDES IT, and the layer comes back with no new update
        // from OsmAnd — the switch republishes rather than waiting for one.
        ui.invoke_settings_set_nav_hide_on_map(false);
        assert!(ui.get_nav_showing(), "the override puts the layer back");
        assert!(ui.get_nav_map_visible(), "the fact is unchanged; only the rule moved");
        assert!(
            !crate::prefs::load(&driver.state.borrow().prefs_dir).nav_hide_on_map,
            "and it persists"
        );

        // Back on, and it hides again.
        ui.invoke_settings_set_nav_hide_on_map(true);
        assert!(!ui.get_nav_showing());
    }

    /// THE HIDDEN PICKER DRESSES THE FACE, WHICH IS THE ONLY REASON IT IS BACK.
    ///
    /// It was ported once and removed, and the note in `ui/settings.slint` says
    /// exactly why: the themes did not exist yet, so six taps "moved a radio
    /// button and changed nothing on the face". So this asserts through the FACE
    /// and not through the tick — the tick moving is not the feature.
    ///
    /// DRIVEN THROUGH THE UI CALLBACK, so the index the panel would send is the
    /// index this test sends. A test that set `forced_egg` directly would pass
    /// with the list and the lookup disagreeing, which is the one mistake the
    /// two-list design makes possible.
    #[test]
    fn the_hidden_picker_dresses_the_face_and_undresses_it() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("egg-picker");
        driver.set_radio_text_for_test("Traffic and weather together on the eights");
        driver.push_all();
        assert!(!ui.get_egg().on, "nothing playing that matches, so no theme");

        // The list the panel is handed: the off row, then the advanced ones.
        use slint::Model;
        let menu = ui.get_settings_egg_menu();
        let listed = crate::eggs::listed();
        assert_eq!(menu.row_count(), listed.len() + 1, "every listed theme, plus off");
        assert_eq!(menu.row_data(0).unwrap().to_string(), EGG_MENU_OFF);
        assert_eq!(
            menu.row_data(1).unwrap().to_string(),
            listed[0].menu,
            "the rows are labels, never ids"
        );
        assert_eq!(ui.get_settings_egg_choice(), 0, "auto-detect until something is picked");

        // FORCE THE FIRST THEME. The RadioText still names nothing.
        ui.invoke_settings_force_egg(1);
        let egg = ui.get_egg();
        assert!(egg.on, "the forced theme is showing");
        assert!(egg.advanced, "and it is one of the five");
        assert_eq!(egg.genre.to_string(), crate::eggs::ACDC.genre);
        assert_eq!(ui.get_settings_egg_choice(), 1, "and the row is lit");

        // A TUNE DOES NOT TAKE IT BACK. The picker outranks the text until the
        // driver says otherwise, which is what makes it usable for looking at a
        // theme while an advert is playing.
        driver.set_radio_text_for_test("Nirvana - Lithium");
        driver.push_all();
        assert_eq!(
            ui.get_egg().genre.to_string(),
            crate::eggs::ACDC.genre,
            "the forced theme outlasts a matching track"
        );

        // OFF, and the text is back in charge.
        ui.invoke_settings_force_egg(0);
        assert_eq!(ui.get_settings_egg_choice(), 0);
        assert_eq!(
            ui.get_egg().genre.to_string(),
            crate::eggs::NIRVANA.genre,
            "auto-detect resumes on the track that is actually playing"
        );

        // AN INDEX PAST THE END IS OFF, not the last row. A stale index from a
        // list that has shrunk must stop forcing rather than force something else.
        ui.invoke_settings_force_egg(1);
        assert!(ui.get_egg().advanced);
        ui.invoke_settings_force_egg(listed.len() as i32 + 5);
        assert_eq!(ui.get_settings_egg_choice(), 0, "out of range is off");
    }

    /// "BACK IN BLACK" — the dark cut, and the fact that it FOLLOWS THE SCHEME.
    ///
    /// A theme states its palette per colour scheme, so which cut applies is
    /// decided at publish time from `Pal.dark`. Nothing else republishes on a
    /// theme change — every ordinary token is derived inside `Pal` and turns on
    /// its own — so switching to dark with AC/DC already playing would have left
    /// the light cut standing until the next track. The scheme flip in the middle
    /// of this is the whole point of it.
    #[test]
    fn back_in_black_arrives_with_the_dark_scheme_and_leaves_with_it() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("egg-back-in-black");
        let pal = ui.global::<crate::Pal>();

        // The ordinary ground and accent, remembered before anything is themed.
        ui.invoke_settings_set_theme("LIGHT".into());
        let plain_page = pal.get_page();
        let plain_blue = pal.get_blue();

        // ── The LIGHT cut restates neither. ──
        driver.set_radio_text_for_test("AC/DC - Back in Black");
        driver.push_all();
        assert!(ui.get_egg().on, "the theme is showing");
        assert_eq!(pal.get_page(), plain_page, "a pale face keeps its ground");
        assert_eq!(pal.get_blue(), plain_blue, "and its blue");
        assert_eq!(ui.get_egg().card_bg.alpha(), 0, "the light cut states no card colour");

        // ── THE SCHEME MOVES UNDER A THEME THAT IS ALREADY SHOWING. ──
        ui.invoke_settings_set_theme("DARK".into());
        let egg = ui.get_egg();
        let dark_page = pal.get_page();
        // THE PAGE IS THE ORDINARY LIFTED CHARCOAL, and this assertion inverted
        // with handoff v1.16.0: it used to demand TRUE BLACK, which put a
        // `#0B0B0B` card on a `#000000` field and let the panel merge into the
        // ground it was meant to sit on. §2.1 now puts the theme on the same page
        // as every other station, so the card reads AGAINST a grey field.
        assert_eq!(
            dark_page,
            slint::Color::from_rgb_u8(0x24, 0x27, 0x2C),
            "the theme sits on the lifted page, not on black"
        );
        assert_eq!(
            pal.get_blue(),
            slint::Color::from_rgb_u8(0xC9, 0xC9, 0xC9),
            "every blue graphic goes silver: they all read this one token"
        );
        assert_eq!(egg.card_bg, slint::Color::from_rgb_u8(0x0B, 0x0B, 0x0B), "the hero card");
        assert_eq!(egg.rt_bg, slint::Color::from_rgb_u8(0x07, 0x07, 0x07), "the RadioText plate");
        assert_eq!(egg.bolt_ink, slint::Color::from_rgb_u8(0xC9, 0xC9, 0xC9), "the call-sign bolt");
        // THE LETTERING IS HOLLOW: filled in the card's own black, held by the
        // silver hairline alone. The fill and the card must be the same colour —
        // that identity IS the treatment.
        assert_eq!(egg.outline_fill, egg.card_bg, "the call sign is cut out of the card");
        assert_eq!(egg.outline_ink, slint::Color::from_rgb_u8(0xC9, 0xC9, 0xC9), "the hairline");

        // ── AND BACK, so the cut is chosen every time rather than latched. ──
        ui.invoke_settings_set_theme("LIGHT".into());
        assert_eq!(pal.get_page(), plain_page, "the ground comes back");
        assert_eq!(pal.get_blue(), plain_blue, "and the blue with it");

        // ── A TRACK CHANGE TAKES THE PALETTE BACK TOO, not just the ornament. ──
        //
        // The PAGE is no longer the thing to watch for that — the theme stopped
        // restating it — so the accent is, and it is the better witness anyway:
        // it is the token a dozen components read.
        ui.invoke_settings_set_theme("DARK".into());
        let silver = slint::Color::from_rgb_u8(0xC9, 0xC9, 0xC9);
        assert_eq!(pal.get_blue(), silver, "still silver while the track plays");
        driver.set_radio_text_for_test("Nicolet Law - Injured? Get Nicolet!");
        driver.push_all();
        assert_ne!(pal.get_blue(), silver, "the accent reverts with the theme");
        assert_eq!(pal.get_page(), dark_page, "and the page never moved for it at all");
        assert_eq!(ui.get_egg().card_bg.alpha(), 0, "the card reverts too");
    }

    /// A STEP REPORTS WHAT IT MANAGED, in the log the driver can export.
    ///
    /// This is the only way the head unit's real frame rate can ever be known:
    /// every figure this repository has came from a desktop running the software
    /// renderer, and the APK renders with Skia on a 32-bit ARM GPU.
    /// `examples/morphbench.rs` holds the counter against an independent frame
    /// count and reports the drift; this holds the other half — that a step arms
    /// the report, that the timer fires it, and that the line reaches the log.
    ///
    /// WHAT IT COUNTS IS ANIMATION ADVANCES, one per turn of the event loop, and
    /// this harness is what proves the distinction: it pumps timers and never
    /// draws, and still tallies — so the count is turns, and a turn is a frame
    /// only where the loop draws every turn, as the device's does. `morphbench`
    /// is where the two are checked against each other with drawing switched on.
    ///
    /// The zero case is exercised separately below, because it is the reading the
    /// driver most needs to be able to get — a morph that drew nothing at all —
    /// and it is an average over zero, which must report rather than panic.
    #[test]
    fn a_step_reports_the_frames_it_got() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("frames");
        driver.push_all();

        let log_has = |driver: &Rc<App>, needle: &str| -> Option<String> {
            driver.state.borrow().settings.log.lines().into_iter().find(|l| l.contains(needle))
        };
        assert!(log_has(&driver, "frames:").is_none(), "nothing reported before a step");

        ui.invoke_step_preset(1);

        // Drive the timer the way the event loop would. The report is armed for
        // `MORPH_REPORT_AFTER`, so this waits a little past it and no longer.
        let deadline = std::time::Instant::now() + MORPH_REPORT_AFTER * 3;
        while std::time::Instant::now() < deadline && log_has(&driver, "frames:").is_none() {
            slint::platform::update_timers_and_animations();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let line = log_has(&driver, "frames:").expect("the step reported its frame count");
        assert!(
            line.contains(" ms/frame"),
            "the tally is reported as a per-frame cost, got {line:?}"
        );
        let counted: u32 = line
            .split_whitespace()
            .nth(2)
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("a number where the count belongs, got {line:?}"));
        assert!(counted > 0, "the loop above turned the clock, so something advanced");

        // AND THE TALLY IS CONSUMED, not left to be added to the next step's.
        assert_eq!(driver.morph_frames_for_bench(), 0);
        assert!(driver.state.borrow().morph_since.is_none(), "the window is closed");

        // THE ZERO CASE, which is an average over nothing. A morph that drew no
        // frames at all is the single most useful thing this line can say, so it
        // must not be the one that divides by zero — and no step can be made to
        // produce it on demand, hence driving the reporter directly.
        {
            let mut s = driver.state.borrow_mut();
            s.morph_frames = 0;
            s.morph_since = Some(std::time::Instant::now());
        }
        report_morph_frames();
        let line = driver
            .state
            .borrow()
            .settings
            .log
            .lines()
            .into_iter()
            .rev()
            .find(|l| l.contains("frames:"))
            .expect("the zero report reached the log");
        assert!(line.contains("no frames at all"), "got {line:?}");
    }

    /// "SAVE TO FILE" WRITES THE WHOLE RING, NOT THE HANDFUL ON SCREEN.
    ///
    /// The row existed and did nothing: every action but "Clear log" fell into a
    /// `_` arm that wrote "not available without the head unit" into the very log
    /// it had been asked to export. This unit has no adb, so the only way a log
    /// left it was a screenshot of the last few lines while the ring held 200 —
    /// which is why the panel-key gap line that #86 turns on could not have been
    /// read.
    ///
    /// DRIVEN THROUGH THE UI CALLBACK, not by calling `run_diag_action` directly:
    /// the row's index comes from `settings::diag_actions`, and a test that
    /// picked the index itself would still pass if the row moved.
    #[test]
    fn saving_the_log_writes_every_line_the_ring_holds() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("savelog");

        let out = std::env::temp_dir().join("carnyx-savelog-out");
        let _ = std::fs::remove_dir_all(&out);
        driver.set_log_dir_for_test(out.clone());

        // THE LAUNCH BLOCK IS ALREADY THERE, in the head, and it does not scroll.
        // Measured rather than assumed: `App::with_tuner` writes the `session:`
        // line and this test is about the RING, so the head's size is subtracted
        // from what the file should hold rather than guessed at.
        let head = driver.state.borrow().settings.log.head_len();
        assert!(head > 0, "the launch block put something in the head");

        // MORE LINES THAN THE RING HOLDS, so what is asserted is "everything the
        // app still has", not "everything that ever happened" — the ring drops
        // the oldest and the file must contain exactly what survived, with the
        // head in front of it.
        let over = settings::DiagLog::CAP + 10;
        {
            let mut s = driver.state.borrow_mut();
            for i in 0..over {
                s.settings.log.push("00:00:00", &format!("line {i}"));
            }
        }

        let index = row_index(&driver, "Save to file");
        ui.invoke_settings_pick_diag_action(index);

        let written = std::fs::read_to_string(out.join("carnyx-tuner-log-1.txt"))
            .expect("the save wrote a file");
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(
            lines.len(),
            head + settings::DiagLog::CAP,
            "the head and the whole ring, and no more"
        );
        assert_eq!(
            lines.get(head).map(|l| l.trim()),
            Some(format!("00:00:00  line {}", over - settings::DiagLog::CAP).trim()),
            "the ring starts at the oldest line it still holds"
        );
        assert_eq!(
            lines.last().map(|l| l.trim()),
            Some(format!("00:00:00  line {}", over - 1).trim()),
            "and ending at the newest"
        );

        // The confirmation goes where the driver is already looking, and names
        // the path — there is no alert dialog here, so the log is the channel.
        let last = driver.state.borrow().settings.log.lines().pop().unwrap_or_default();
        assert!(
            last.contains("log saved to") && last.contains("carnyx-tuner-log-1.txt"),
            "the panel says where it went, got {last:?}"
        );
    }

    /// AND AN EMPTY LOG WRITES NOTHING.
    ///
    /// CarFM's "Nothing to save" alert (`SettingsPanel.tsx:125`). A file of zero
    /// bytes in Downloads is worse than a refusal: it reads as a log that was
    /// captured and found nothing.
    #[test]
    fn saving_an_empty_log_writes_no_file() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("savelog-empty");

        let out = std::env::temp_dir().join("carnyx-savelog-empty-out");
        let _ = std::fs::remove_dir_all(&out);
        driver.set_log_dir_for_test(out.clone());
        driver.state.borrow_mut().settings.log.clear();

        let index = row_index(&driver, "Save to file");
        ui.invoke_settings_pick_diag_action(index);

        assert!(!out.join("carnyx-tuner-log-1.txt").exists(), "no file was written");
        let last = driver.state.borrow().settings.log.lines().pop().unwrap_or_default();
        assert!(last.contains("the log is empty"), "and it says why, got {last:?}");
    }

    /// THE NEW PROBE'S ROW RUNS AND SAYS SOMETHING, on a build where it cannot
    /// possibly succeed.
    ///
    /// The host has no vendor power manager and no class to load, so the report
    /// is empty — and an empty report must still leave a line. A tap that writes
    /// nothing reads as a broken row, which is exactly the failure the five
    /// removed rows had: they wrote "not available without the head unit" into
    /// the log and looked like a build limitation rather than an unwritten
    /// function. This one is honest about which it is.
    #[test]
    fn the_keep_alive_probe_leaves_a_line_even_where_it_cannot_run() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("keepalive");
        driver.drain_events();
        let before = driver.state.borrow().settings.log.lines().len();

        ui.invoke_settings_pick_diag_action(row_index(&driver, "What could keep Carnyx alive through sleep"));

        // THE TAP ANSWERS BEFORE THE WORK. The row defers the binder walk by a
        // frame so the well can repaint; what the driver sees FIRST is this.
        let after_tap = driver.state.borrow().settings.log.lines();
        assert!(after_tap.len() > before, "the tap wrote something immediately");
        assert!(
            after_tap.last().unwrap().contains("keep-alive probe: reading"),
            "and it says the probe has started, got {:?}",
            after_tap.last()
        );

        // Then the deferred half, driven directly — waiting on a timer here
        // would be a test of the clock.
        driver.run_pending_probe();
        let lines = driver.state.borrow().settings.log.lines();
        assert!(lines.len() > after_tap.len(), "the deferred half wrote too");
        assert!(
            lines.last().unwrap().contains("keep-alive probe"),
            "and it names the probe, got {:?}",
            lines.last()
        );

        // AND IT ONLY RUNS ONCE. `pending_probe` is taken, so a second tick with
        // no tap behind it must write nothing — otherwise every frame after a
        // tap would re-walk the package manager.
        let settled = driver.state.borrow().settings.log.lines().len();
        driver.run_pending_probe();
        assert_eq!(
            driver.state.borrow().settings.log.lines().len(),
            settled,
            "a tick with nothing pending does nothing"
        );
    }

    /// LOSING FOCUS IS NOT LEAVING, AND A PAUSE IS.
    ///
    /// This test asserted the OPPOSITE for one commit. The reasoning was that the
    /// unit composes apps side by side, so another app takes focus and never
    /// produces a `Pause`, and that this was why no pop-up appeared with another
    /// app in front. THE OWNER SAYS OTHERWISE IN AS MANY WORDS: no windowing, no
    /// side-by-side — Carnyx runs full screen and the driver switches the whole
    /// screen to another app. That pauses it.
    ///
    /// So focus is recorded and gates nothing, and this pins both halves of that:
    /// a shade or a dialog over the face must stay SILENT — the driver is looking
    /// at the face and would also be told the station over the top of it — and a
    /// pause must still speak.
    #[test]
    fn a_shade_over_the_face_stays_silent_and_a_pause_speaks() {
        let _ui_lock = harness::ui_lock();
        let (_ui, driver) = app_for("popup-focus");
        let strip = fake::SEED_PRESET_MHZ;
        let said = |d: &Rc<App>| -> usize {
            d.state.borrow().settings.log.lines().iter().filter(|l| l.contains("station pop-up:")).count()
        };

        // In front and focused: a tune says nothing.
        crate::android::set_resumed(true);
        crate::android::set_focused(true);
        driver.tune_for_test(strip[1]);
        driver.drain_events();
        assert_eq!(said(&driver), 0, "a tune the driver is watching says nothing");

        // THE SHADE COMES DOWN: focus goes, the activity is still resumed. The
        // face is still what is behind it and the driver is still in front of it.
        crate::android::set_focused(false);
        assert!(crate::android::is_foreground(), "a lost focus is not a departure");
        driver.tune_for_test(strip[2]);
        driver.drain_events();
        assert_eq!(said(&driver), 0, "and a tune under a shade still says nothing");

        // THE DRIVER LEAVES: the whole screen goes to another app, which pauses.
        crate::android::set_resumed(false);
        assert!(!crate::android::is_foreground(), "a pause is a departure");
        driver.tune_for_test(strip[3]);
        driver.drain_events();
        assert_eq!(said(&driver), 1, "and now the wheel's change is announced");

        crate::android::set_foreground(true);
    }

    /// THE SAME BAR FOR THE SECOND PROBE ROW, and it earns its own test rather
    /// than riding on the shared helper: the two rows reach different classes
    /// through different seams, and a row wired to the wrong one would pass any
    /// test that only exercised the helper.
    #[test]
    fn the_stock_radio_probe_leaves_a_line_even_where_it_cannot_run() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("stockradio");
        driver.drain_events();
        let before = driver.state.borrow().settings.log.lines().len();

        ui.invoke_settings_pick_diag_action(row_index(
            &driver,
            "Where the stock radio app can be intercepted",
        ));

        let after_tap = driver.state.borrow().settings.log.lines();
        assert!(after_tap.len() > before, "the tap wrote something immediately");
        assert!(
            after_tap.last().unwrap().contains("stock radio probe: reading"),
            "and it says the probe has started, got {:?}",
            after_tap.last()
        );

        driver.run_pending_probe();
        let lines = driver.state.borrow().settings.log.lines();
        assert!(
            lines.last().unwrap().contains("stock radio probe"),
            "and it names the probe, got {:?}",
            lines.last()
        );
    }

    /// EVERY ROW RUNS ITS OWN ACTION, AND EVERY ACTION HAS A ROW.
    ///
    /// The failure this catches is the one a copied `DiagAction` block produces
    /// silently: `row_index` finds the FIRST row with a label, so two rows
    /// carrying the same `Action` would send both taps to one probe and both
    /// tests above would still pass. The other direction is a variant with no
    /// row — a probe nobody can reach.
    #[test]
    fn each_diagnostics_row_runs_its_own_action() {
        let rows = settings::diag_actions();
        let mut seen: Vec<settings::Action> = Vec::new();
        for row in &rows {
            assert!(
                !seen.contains(&row.action),
                "{:?} is on two rows, so one of them is dead",
                row.action
            );
            seen.push(row.action);
        }
        for action in [
            settings::Action::SaveLog,
            settings::Action::ClearLog,
            settings::Action::ProbeKeepAlive,
            settings::Action::ProbeStockRadio,
        ] {
            assert!(seen.contains(&action), "{action:?} has no row");
        }
    }

    /// Where a labelled diagnostics row currently sits, so a test names the row
    /// rather than an index that a reorder would silently change.
    fn row_index(driver: &Rc<App>, label: &str) -> i32 {
        let s = driver.state.borrow();
        s.settings
            .actions()
            .iter()
            .position(|a| a.label == label)
            .unwrap_or_else(|| panic!("no {label:?} row")) as i32
    }

    /// THE VENDOR'S BANK WALK DOES NOT GET THE LAST WORD ON THE RADIO.
    ///
    /// > "A few times when using the steering wheel controls, it did end up
    /// > jumping to a wrong station."
    ///
    /// The hold fixed what the driver SEES while the vendor walks its own bank.
    /// It says nothing about where the radio ends up, and the walk is a real
    /// retune: when the vendor's command lands after ours, the front end is on
    /// the vendor's station, the face renders the target for two seconds, and
    /// then the hold expires and admits it. See [`State::reassert`].
    ///
    /// ASSERTS ABOUT THE TUNER, NOT THE LABEL, and that is the whole reason this
    /// test needs `vendor_tunes_for_test` rather than the `vendor_reports` every
    /// other case here uses. A bare report leaves the fake's own frequency where
    /// this app put it, so the simulated radio is obediently correct however
    /// loudly the report disagrees — which is why six wheel-probe cases full of
    /// vendor traffic never caught this.
    #[test]
    fn a_vendor_retune_after_ours_does_not_keep_the_radio() {
        let _ui_lock = harness::ui_lock();
        let (_ui, driver) = app_for("reassert");
        let strip = fake::SEED_PRESET_MHZ;

        driver.tune_for_test(strip[1]);
        driver.drain_events();
        // Asynchronous reporting, as on the device — see the hold test above.
        driver.set_echo_for_test(false);

        // The press. Our step commands strip[2] and the front end takes it.
        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();
        assert_eq!(
            driver.tuned_mhz_for_test().map(format_mhz),
            Some(format_mhz(strip[2])),
            "our own step reached the front end"
        );

        // AND THEN the vendor's bank walk retunes on top of it. This is the
        // ordering the fix is about: before it, the run ended here.
        driver.vendor_tunes_for_test(99.9);
        driver.drain_events();

        assert_eq!(
            driver.tuned_mhz_for_test().map(format_mhz),
            Some(format_mhz(strip[2])),
            "the app re-commands its own target, so the driver is left on it"
        );
    }

    /// AND IT GIVES UP RATHER THAN FIGHTING FOREVER.
    ///
    /// A budget that never ran out would be an app and a vendor service taking
    /// turns retuning the radio for as long as the driver held still. Past
    /// [`REASSERT_TRIES`] the app stops, the hold expires on its own clock, and
    /// the face shows where the radio really is — see [`State::reassert`].
    #[test]
    fn the_reassert_budget_runs_out() {
        let _ui_lock = harness::ui_lock();
        let (_ui, driver) = app_for("reassert-cap");
        let strip = fake::SEED_PRESET_MHZ;

        driver.tune_for_test(strip[1]);
        driver.drain_events();
        driver.set_echo_for_test(false);

        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();

        // A vendor that retunes after every one of ours, one more time than the
        // budget allows.
        //
        // EVERY ATTEMPT INSIDE THE BUDGET IS CHECKED, and that is not padding.
        // Asserting only the final 99.9 is satisfied by the re-assert never
        // happening at all — the test passed with the whole feature switched
        // off, which is a test of nothing. The budget is only a budget if the
        // spends before it are visible.
        for attempt in 1..=REASSERT_TRIES {
            driver.vendor_tunes_for_test(99.9);
            driver.drain_events();
            assert_eq!(
                driver.tuned_mhz_for_test().map(format_mhz),
                Some(format_mhz(strip[2])),
                "attempt {attempt} of {REASSERT_TRIES} is inside the budget and takes the radio back"
            );
        }

        driver.vendor_tunes_for_test(99.9);
        driver.drain_events();
        assert_eq!(
            driver.tuned_mhz_for_test().map(format_mhz),
            Some(format_mhz(99.9)),
            "and the one past the budget is not answered"
        );
    }

    /// A NEWER PRESS OUTRANKS AN OLDER TARGET.
    ///
    /// The re-assert is recorded when a report contradicts a hold and acted on
    /// later, so a second press can land in between. Re-commanding the first
    /// press's target on top of the second one would step the driver BACKWARDS —
    /// which is the complaint the whole wheel path exists to answer, and would
    /// be a fine way to reintroduce it. `drain_events` drops a target whose hold
    /// is no longer the live one.
    #[test]
    fn a_stale_reassert_cannot_undo_a_newer_step() {
        let _ui_lock = harness::ui_lock();
        let (_ui, driver) = app_for("reassert-stale");
        let strip = fake::SEED_PRESET_MHZ;

        driver.tune_for_test(strip[1]);
        driver.drain_events();
        driver.set_echo_for_test(false);

        // First press: target strip[2], which the front end takes.
        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();

        // THE VENDOR'S RETUNE AND THE SECOND PRESS IN ONE BATCH, which is the
        // only interleaving where a stale target is still pending when a newer
        // step runs. `drain_events` empties the tuner queue first — recording
        // the re-assert — then applies the key, and only then re-commands.
        // Separate drains would have spent the re-assert before the second press
        // arrived and the test would pass without exercising anything.
        driver.vendor_tunes_for_test(99.9);
        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();

        assert_eq!(
            driver.tuned_mhz_for_test().map(format_mhz),
            Some(format_mhz(strip[3])),
            "the newer step stands; the older target is dropped, not replayed"
        );
    }

    /// THE SEEK THIS UNIT ACTUALLY HAS LEAVES THE STRIP.
    ///
    /// `PanelAction::Seek` cleared `asserted` and `on_freq_seek` did not, which
    /// put the rule on the DEAD path only — `PanelKey` records that this fascia
    /// has no seek button, so the on-screen one is the only seek there is. A
    /// stale anchor then survived the sweep and the next step went from where the
    /// driver had been BEFORE seeking. Found by review, after the anchor itself
    /// had shipped.
    #[test]
    fn an_on_screen_seek_leaves_the_strip_behind() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("seek");
        let strip = fake::SEED_PRESET_MHZ;

        // On entry 0, then seek away to whatever the fake lands on.
        driver.tune_for_test(strip[0]);
        driver.drain_events();
        ui.invoke_freq_seek(1);
        driver.drain_events();
        let landed = ui.get_freq_label().to_string();
        assert_ne!(landed, format_mhz(strip[0]).to_string(), "the seek moved the dial");

        // Now step. Off the strip, the rule is entry 0 — NOT entry 1, which is
        // what stepping from the pre-seek anchor would give.
        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();
        assert_eq!(
            ui.get_freq_label().to_string(),
            format_mhz(strip[0]).to_string(),
            "a step after a seek starts from off-strip, not from the old anchor"
        );
    }

    /// A UI TUNE DOES NOT LET A QUEUED WHEEL KEY RUN INSIDE IT.
    ///
    /// `tune` drains before it writes `dial`, so a key sitting in the queue was
    /// applied by that nested drain and its step's target was then overwritten by
    /// the outer tune — the front end left on one station with the face naming
    /// another. The `busy` flag closes it; this holds that the queued key still
    /// gets its turn afterwards rather than being dropped.
    #[test]
    fn a_queued_wheel_key_waits_for_a_tune_in_flight() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("busy");
        let strip = fake::SEED_PRESET_MHZ;

        driver.tune_for_test(strip[0]);
        driver.drain_events();

        // A wheel key lands, and before any drain the driver taps a tile.
        crate::android::ingest_panel_key(62, "down".into());
        ui.invoke_select_preset(3);
        driver.drain_events();

        // The tap and then the key, in that order, each landing once: entry 3,
        // then one step on from it. The failure this guards against ended on
        // entry 1 with the face showing entry 3.
        assert_eq!(
            ui.get_freq_label().to_string(),
            format_mhz(strip[4]).to_string(),
            "the tap lands, then the queued key steps once from where it left off"
        );
    }

    /// A ONE-ENTRY STRIP DOES NOT RE-TUNE THE STATION ALREADY PLAYING.
    ///
    /// `(active + dir).rem_euclid(1)` is always `active`, so every press landed
    /// on the same entry and `tune` went out anyway — dropping the level and
    /// provoking a frequency report that runs `reset_for_retune`, which blanks
    /// the name, the RadioText and the PTY. Pressing next wiped the face.
    #[test]
    fn stepping_a_single_preset_does_not_retune() {
        let _ui_lock = harness::ui_lock();
        let (ui, driver) = app_for("single");

        // ONE PRESET, built through the shipping path. `save_dial_for_test` is
        // tune-then-toggle, and `toggle_save` REMOVES a dial that is already
        // saved — so walking the seeded six empties the strip, and the seventh
        // call puts a single entry back.
        for mhz in fake::SEED_PRESET_MHZ {
            driver.save_dial_for_test(mhz);
        }
        driver.save_dial_for_test(98.1);
        driver.drain_events();
        assert!(ui.get_saved(), "the dial is saved");
        assert_eq!(
            slint::Model::row_count(&ui.get_presets()),
            1,
            "and it is the only entry"
        );

        // COUNTED OFF THE DIAGNOSTICS LOG, which records one `tuned N` line per
        // frequency report — so a retune that went out is visible and one that
        // did not is visible too. The alternative was a counter on the fake, and
        // the log is already the thing a driver reads when the radio misbehaves.
        let tuned_lines = |ui: &AppWindow| {
            slint::Model::iter(&ui.get_settings_diag_lines())
                .filter(|l: &slint::SharedString| l.contains("tuned "))
                .count()
        };
        let before = tuned_lines(&ui);
        crate::android::ingest_panel_key(62, "down".into());
        driver.drain_events();
        driver.push_all();
        assert_eq!(ui.get_freq_label().to_string(), "98.1", "still on the one entry");
        assert_eq!(
            tuned_lines(&ui),
            before,
            "and the front end was not re-commanded, so the RDS was not blanked"
        );
    }

    /// `assets/` is packaged into the APK verbatim by cargo-apk, so ANYTHING
    /// written there by a host run ships to every driver. Deriving the prefs
    /// directory from `db_path` did exactly that, putting `prefs.json` beside the
    /// station database. This pins the separation rather than the current paths.
    #[test]
    fn host_prefs_never_live_in_the_shipped_assets_tree() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        assert!(
            !host_prefs_dir().starts_with(&assets),
            "host prefs at {:?} would be packaged into the APK",
            host_prefs_dir()
        );
        // And the database really is in there, so the check above is meaningful.
        assert!(host_db_path().starts_with(&assets));
    }

    /// `LogoStore` writes its own hero flags to a file called `prefs.json` at
    /// its root — the SAME NAME `crate::prefs` uses. Rooted at the prefs
    /// directory the two would be one file and each would destroy the other, so
    /// the separation is pinned here rather than left to the comment.
    #[test]
    fn the_logo_store_cannot_collide_with_the_preference_file() {
        let dir = Path::new("/tmp/carnyx-test");
        let logos = logo_dir(dir);
        assert_ne!(logos, dir, "the store must not be rooted at the prefs directory");
        assert!(logos.starts_with(dir), "the store belongs under the prefs directory");
        // The name that would collide, spelt out so a rename of either side
        // fails here rather than on a driver's unit.
        assert_ne!(logos.join("prefs.json"), crate::prefs::path(dir));
        // And the store is not packaged into the APK either.
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        assert!(!logo_dir(&host_prefs_dir()).starts_with(&assets));
    }

    /// `has_logo` is not a decoration flag — a tile with it set draws a
    /// borderless transparent plate and prints the CALL SIGN beneath, and a tile
    /// without it draws the coloured box and prints the FREQUENCY. Setting it
    /// without art is a blank plate over the wrong caption.
    #[test]
    fn a_tile_claims_a_logo_only_when_it_was_handed_one() {
        let slot = Slot { mhz: 88.7, row: None, saved_call: None };

        let bare = to_preset(&slot, None);
        assert!(!bare.has_logo);
        assert_eq!(bare.freq_label, "88.7");

        // A 1×1 image is enough: what is under test is that the flag follows the
        // Option, not what the pixels are.
        let px = crate::logos::ui::to_image(&Raster { w: 1, h: 1, rgba: vec![0, 0, 0, 255] });
        let dressed = to_preset(&slot, Some((px.clone(), LogoPlate::Plate)));
        assert!(dressed.has_logo);
        // AND THE BACKING RIDES WITH THE ART. A tile that took the picture and
        // dropped the answer to "what goes behind it" would put a keyed `plate`
        // mark on a transparent plate — a dark logo on a dark card.
        assert_eq!(dressed.plate, LogoPlate::Plate);
        assert_eq!(
            to_preset(&slot, Some((px, LogoPlate::Bare))).plate,
            LogoPlate::Bare,
            "the tile reports what it was handed, not a constant"
        );
        // No art, no plate: a stale one would size the next logo wrongly.
        assert_eq!(bare.plate, LogoPlate::Light);
    }

    /// The resolution is by POSITION, and the assertion that matters is the
    /// negative one: 88.7 MHz has full-power licensees all over the country and
    /// taking the table's first row puts a stranger's call sign on the hero.
    #[test]
    fn the_shipped_database_resolves_the_dials_the_face_opens_on() {
        let db = StationDb::open(&host_db_path()).expect("the shipped database opens");
        let here = fake::FakeLocation::default().position();
        // Every seeded preset must resolve, or the strip opens showing bare
        // frequencies and the wiring looks broken.
        for mhz in fake::SEED_PRESET_MHZ {
            assert!(resolve(Some(&db), mhz, here).is_some(), "{mhz} resolved to nothing");
        }
        assert_eq!(resolve(Some(&db), 88.7, here).unwrap().callsign_base, "WERN");

        // The row the table happens to return FIRST on this dial is a different
        // station in a different state — this is the bug the position check
        // exists to stop, so it is pinned rather than described.
        let first_in_table = db.at_frequency(88.7).unwrap().into_iter().next().unwrap();
        assert_ne!(first_in_table.callsign_base, "WERN");

        // NO FIX, NO NAME. With nothing to disambiguate 178 co-channel
        // licensees against, the honest answer is the frequency.
        assert!(resolve(Some(&db), 88.7, None).is_none());
        // A dial with nothing full-power within range resolves to nothing rather
        // than to whatever is least far away.
        assert!(resolve(Some(&db), 87.9, here).is_none());
        // No database at all is a normal state, not a panic.
        assert!(resolve(None, 88.7, here).is_none());
    }

    #[test]
    fn a_slot_prints_a_resolved_call_sign_and_falls_back_to_the_dial() {
        let db = StationDb::open(&host_db_path()).unwrap();
        let here = fake::FakeLocation::default().position();
        let resolved =
            Slot { mhz: 88.7, row: resolve(Some(&db), 88.7, here), saved_call: None };
        assert_eq!(resolved.call(), "WERN");
        assert_eq!(resolved.name(), "WERN");
        // Nothing on the dial: the frequency stands as the identity, never an
        // inaccurate "Tuning…".
        let bare = Slot { mhz: 87.9, row: None, saved_call: None };
        assert_eq!(bare.name(), "87.9");
        assert_eq!(bare.call(), "87.9");
        assert_eq!(bare.base(), "");
    }

    /// WHERE A WHEEL PRESS STEPS FROM, and why it is not simply the dial.
    ///
    /// THE BUG THIS CLOSES, in the driver's words: "from certain presets, going
    /// 'next' skips presets or actually goes backwards", consistently. One wheel
    /// press makes the vendor service step its OWN hardware preset bank — that
    /// is `PanelKey::PresetNext`'s documented behaviour and cannot be cancelled
    /// — and the frequency it lands on arrives as an ordinary `Frequency` event
    /// that moves `dial`. `drain_events` applies the deferred press only once
    /// that queue is EMPTY, so the step used to be computed from the vendor's
    /// dial: off-strip it resolved to -1 and every press went to entry 0, which
    /// is the "backwards"; on-strip it resolved to an unrelated index, which is
    /// the "skips".
    ///
    /// It read as consistent because the vendor's bank is a fixed list walked
    /// deterministically, so the same starting preset missed the same way every
    /// time.
    #[test]
    fn the_step_anchor_ignores_a_dial_the_app_did_not_choose() {
        let strip: Vec<Slot> = [102.1_f32, 88.7, 105.5, 98.1, 96.3, 94.1]
            .iter()
            .map(|&mhz| Slot { mhz, row: None, saved_call: None })
            .collect();

        // Nothing asserted yet — the first press of a run — falls back to the
        // dial, which is the behaviour that was always right.
        assert_eq!(step_anchor(None, 105.5, &strip), 2);
        assert_eq!(step_anchor(None, 99.9, &strip), -1);

        // THE FIX. The app put the driver on 105.5; the vendor then moved the
        // radio to its own bank entry. The strip is still on 105.5.
        assert_eq!(step_anchor(Some(105.5), 99.9, &strip), 2, "vendor off-strip");
        // And the case that read as a SKIP rather than a reset: the vendor's
        // bank entry is a dial the driver also has, so the old rule resolved it
        // to a real, wrong index instead of to -1.
        assert_eq!(step_anchor(Some(102.1), 105.5, &strip), 0, "vendor on-strip");

        // A DELIBERATE TUNE ELSEWHERE IS NOT IGNORED. `tune` sets `asserted`, so
        // the keypad, the nearby list and a preset tap all move the anchor —
        // only the radio reporting its own movement does not.
        assert_eq!(step_anchor(Some(99.9), 105.5, &strip), -1, "the driver left the strip");

        // A FREQUENCY AND NOT AN INDEX, which is what survives a reorder: the
        // same anchor resolves to wherever its entry was dragged to.
        let dragged: Vec<Slot> = [88.7_f32, 105.5, 102.1, 98.1, 96.3, 94.1]
            .iter()
            .map(|&mhz| Slot { mhz, row: None, saved_call: None })
            .collect();
        assert_eq!(step_anchor(Some(102.1), 99.9, &dragged), 2, "follows the drag");
        // And deleting the anchored entry drops cleanly rather than pointing at
        // whatever slid into its index.
        let deleted: Vec<Slot> = [88.7_f32, 105.5, 98.1]
            .iter()
            .map(|&mhz| Slot { mhz, row: None, saved_call: None })
            .collect();
        assert_eq!(step_anchor(Some(102.1), 99.9, &deleted), -1, "the entry is gone");

        // An empty strip has no anchor and must not panic.
        assert_eq!(step_anchor(Some(102.1), 102.1, &[]), -1);
    }

    /// THE GHOST ONLY EXISTS WHEN A CARD REALLY GOES.
    ///
    /// Written because the first cut stamped one unconditionally, and the case
    /// it gets wrong is not exotic: an unsaved dial is what the face shows every
    /// time the driver tunes somewhere that is not a preset.
    #[test]
    fn a_step_discards_a_card_only_when_the_slot_changes_hands() {
        // The ordinary case, six presets, sitting on entry 2. Forward, the prev
        // slot goes from entry 1 to entry 2; back, the next slot goes from 3 to 2.
        assert!(step_discards(6, 2, 3, 1), "forward off a preset discards prev");
        assert!(step_discards(6, 2, 1, -1), "back off a preset discards next");

        // AN UNSAVED DIAL. `push_hero` shows the last entry on the left and the
        // first on the right, so stepping forward lands on entry 0 — whose prev
        // is the last entry, which is already the card in that slot.
        assert!(!step_discards(6, -1, 0, 1), "forward off an unsaved dial discards nothing");
        // Backwards off the same dial lands on the last entry, whose next is
        // entry 0 — again already there.
        assert!(!step_discards(6, -1, 5, -1), "back off an unsaved dial discards nothing");

        // ONE PRESET. Every slot is the same entry, so nothing can be discarded
        // in either direction. (The step is refused before this by the
        // frequency guard, but the answer must still be honest.)
        assert!(!step_discards(1, 0, 0, 1));
        assert!(!step_discards(1, 0, 0, -1));

        // TWO PRESETS, where prev and next are the same card. Stepping from 0 to
        // 1 moves the prev slot from entry 1 to entry 0, so a card does go.
        assert!(step_discards(2, 0, 1, 1));

        // Wrapping at the ends behaves like anywhere else.
        assert!(step_discards(6, 5, 0, 1), "wrapping forward still discards");
        assert!(step_discards(6, 0, 5, -1), "wrapping back still discards");

        // An empty strip cannot be stepped, and must not answer true.
        assert!(!step_discards(0, -1, 0, 1));
    }

    /// THE NUMPAD'S ENTRY RULES, which used to be a six-character cap.
    ///
    /// CarFM's `press` (Numpad.tsx:45-53). The cap let `..`, `1.2.3`, `.1055` and
    /// `100000` be typed, and none of them parses — so TUNE stayed lit over a
    /// value that could not be tuned and pressing it did nothing at all.
    #[test]
    fn the_numpad_takes_four_digits_and_one_decimal() {
        // The ordinary dial, one key at a time.
        let typed = ["1", "0", "5", ".", "1"]
            .iter()
            .fold(String::new(), |b, k| numpad_press(&b, k));
        assert_eq!(typed, "105.1");

        // FOUR DIGITS, and the decimal point is not one of them: "105." is four
        // characters and three digits, so its fourth digit is still welcome.
        assert_eq!(numpad_press("105.1", "9"), "105.1", "a fifth digit is refused");
        assert_eq!(numpad_press("1055", "9"), "1055");
        assert_eq!(numpad_press("105.", "1"), "105.1");

        // ONE decimal point, and never a leading one.
        assert_eq!(numpad_press("105.1", "."), "105.1", "a second point is refused");
        assert_eq!(numpad_press("", "."), "", "a leading point is refused");
        assert_eq!(numpad_press("1", "."), "1.");

        // The keypad only ever sends the twelve keys; anything else is not a key.
        assert_eq!(numpad_press("10", "x"), "10");
        assert_eq!(numpad_press("10", "12"), "10");
        assert_eq!(numpad_press("10", ""), "10");
    }

    /// THE COMMIT, and the rounding that was missing.
    ///
    /// CarFM's `commit` (Numpad.tsx:56-60) rounds to a tenth before it checks the
    /// band. Carnyx tuned the raw parse, so "105.55" tuned 105.55 where CarFM
    /// tunes 105.6 — and a dial is a tenth or it is not a dial.
    #[test]
    fn the_numpad_rounds_to_a_tenth_and_then_checks_the_band() {
        assert_eq!(numpad_commit("105.1"), Some(105.1));
        // A trailing point is a legitimate mid-entry string and parses.
        assert_eq!(numpad_commit("105."), Some(105.0));
        assert_eq!(numpad_commit("88"), Some(88.0));

        // ROUNDED, not truncated, and rounded BEFORE the band check — 108.04 is
        // out of band as typed and in band once it is a dial.
        assert_eq!(numpad_commit("105.55"), Some(105.6));
        assert_eq!(numpad_commit("108.04"), Some(108.0));

        // Outside the band, and unparseable, are both "nothing to tune".
        assert_eq!(numpad_commit("87.4"), None);
        assert_eq!(numpad_commit("108.1"), None);
        assert_eq!(numpad_commit("1"), None);
        assert_eq!(numpad_commit(""), None);
        assert_eq!(numpad_commit("."), None);
        assert_eq!(numpad_commit("1.2.3"), None);
    }

    /// The live warning and the commit must agree: a buffer that commits, or that
    /// the keypad can still carry to a commit, never warns; a buffer no typing
    /// can rescue always does.
    #[test]
    fn entry_warning_only_when_stuck() {
        for ok in ["1", "10", "105", "105.", "8", "9", "0", "87.5", "108.0", "87.4", "87.46", "087."] {
            assert!(entry_can_tune(ok), "{ok:?} can still reach a dial");
        }
        for stuck in ["7", "6", "12", "120", "109", "108.1", "1080"] {
            assert!(!entry_can_tune(stuck), "{stuck:?} is beyond rescue");
        }
    }

    /// Exhaustive, not sampled: every buffer the keypad can physically produce is
    /// walked, and any that `numpad_commit` accepts must not be warning — the
    /// exact contradiction the display-string-prefix rule shipped with.
    #[test]
    fn committable_buffers_never_warn() {
        fn walk(buf: String, count: &mut u32) {
            for k in ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "."] {
                let next = numpad_press(&buf, k);
                if next != buf {
                    if numpad_commit(&next).is_some() {
                        assert!(entry_can_tune(&next), "{next:?} commits but would warn");
                    }
                    *count += 1;
                    walk(next, count);
                }
            }
        }
        let mut count = 0;
        walk(String::new(), &mut count);
        // The grammar is 4 digits + one point; if this collapses, the walk is
        // not covering the keypad any more.
        assert!(count > 10_000, "only {count} buffers walked");
    }

    /// The wheel and the panel buttons, which decoded to a log line and nothing
    /// else until now: the head unit's own controls did not work.
    #[test]
    fn a_panel_press_becomes_an_action_and_a_release_does_not() {
        use crate::android::PanelKey as K;

        // The two that matter most on the wheel. The anchor rides along on both,
        // UNCHANGED — this function must not have an opinion about it, because
        // its whole job is to carry the caller's reading of the strip through to
        // a step that happens later.
        assert_eq!(
            panel_action(Some(K::PresetNext), "down", 3),
            Some(PanelAction::Step { dir: 1, from: 3 })
        );
        assert_eq!(
            panel_action(Some(K::PresetPrev), "down", 3),
            Some(PanelAction::Step { dir: -1, from: 3 })
        );
        // Including -1, which is a real anchor and means "off the strip".
        assert_eq!(
            panel_action(Some(K::PresetNext), "down", -1),
            Some(PanelAction::Step { dir: 1, from: -1 })
        );
        // Both search codes and both seek codes reach the same two answers, and
        // a seek carries no anchor because it does not step the strip.
        assert_eq!(panel_action(Some(K::SearchUp), "down", 0), Some(PanelAction::Seek(true)));
        assert_eq!(panel_action(Some(K::SeekUp), "down", 0), Some(PanelAction::Seek(true)));
        assert_eq!(panel_action(Some(K::SearchDown), "down", 0), Some(PanelAction::Seek(false)));
        assert_eq!(panel_action(Some(K::SeekDown), "down", 0), Some(PanelAction::Seek(false)));

        // THE RELEASE DOES NOTHING. The MCU sends both edges, and acting on both
        // steps two stations for one push.
        assert_eq!(panel_action(Some(K::PresetNext), "up", 0), None);
        assert_eq!(panel_action(Some(K::PresetNext), "UP", 0), None);

        // Buttons this app has no answer for, and the unknown code 14 that
        // arrived eight times in CarFM's drive log and is in no vendor table.
        assert_eq!(panel_action(Some(K::Ams), "down", 0), None);
        assert_eq!(panel_action(Some(K::ChangeBand), "down", 0), None);
        assert_eq!(panel_action(None, "down", 0), None);
        assert_eq!(K::from_code(14), None);
    }

    /// THE DRIVEWAY, end to end.
    ///
    /// The owner cannot get a fix from their own driveway. Everything that names
    /// a station is position-gated, because 88.7 has 178 full-power licensees and
    /// picking one without a position puts a stranger on the hero — so a cold
    /// start there could name nothing at all.
    ///
    /// This is the way out, and it is CarFM's: one good lock while driving learns
    /// what is on each frequency, and every no-fix start afterwards reads the
    /// names straight out of it. Driven through the real query the picker runs,
    /// against the real 20,733-row database, then reloaded from disk exactly as a
    /// cold start would.
    #[test]
    fn one_lock_while_driving_names_the_band_from_the_driveway_forever_after() {
        let db = StationDb::open(&host_db_path()).expect("the shipped database opens");
        let dir = std::env::temp_dir().join("carnyx-driveway");
        let _ = std::fs::remove_dir_all(&dir);

        // ── The drive: a fix, so the picker's query answers. ──
        let (lat, lon) = fake::FakeLocation::default().position().unwrap();
        let picker = NearbyPicker::query(&db, lat, lon).expect("the nearby query runs");
        assert!(picker.located());
        let rows: Vec<(f32, String)> = picker
            .rows()
            .iter()
            .filter(|r| r.row.service == "FM")
            .map(|r| (r.row.frequency_mhz as f32, r.row.callsign_base.clone()))
            .collect();
        assert!(!rows.is_empty(), "the query has to return something to learn from");

        let mut learned = crate::callsigns::Callsigns::default();
        assert!(learned.relearn(rows.iter().map(|(m, b)| (*m, b.as_str()))));
        crate::callsigns::save(&dir, &learned);

        // The band it filled covers the dials the face opens on.
        assert_eq!(learned.get(88.7), Some("WERN"));
        assert!(learned.len() >= 6, "one lock should fill more than a preset or two");

        // ── The driveway: a cold start, no fix, nothing but the file. ──
        let cold = crate::callsigns::load(&dir);
        assert_eq!(cold, learned, "the map survives the launch");

        // Every seeded preset can be named, with no position at all.
        for mhz in fake::SEED_PRESET_MHZ {
            assert!(resolve(Some(&db), mhz, None).is_none(), "no fix resolves nothing");
            let named = cold.get(mhz).expect("the learned map names it anyway");
            let slot = Slot { mhz, row: None, saved_call: Some(named.to_string()) };
            assert_ne!(slot.name(), format_mhz(mhz), "{mhz} must not read as a bare dial");
            assert!(!slot.base().is_empty(), "{mhz} must give the logo search a key");
        }
        assert_eq!(cold.get(88.7), Some("WERN"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// THE REGRESSION, pinned.
    ///
    /// A preset whose row has not resolved — which is every preset on a head
    /// unit that has not got a GPS fix yet, because resolution needs a position
    /// — must still carry the call sign it was saved with. It did not: the tiles
    /// came back as bare frequencies and `open_logo_search` built its query from
    /// an empty base, so the logo window had nothing to search for.
    #[test]
    fn a_preset_keeps_its_call_sign_with_no_fix_to_resolve_against() {
        let remembered = Slot { mhz: 88.7, row: None, saved_call: Some("WERN-FM".into()) };

        // What the tile prints, and what its colour hashes from.
        assert_eq!(remembered.name(), "WERN");
        assert_eq!(remembered.call(), "WERN");
        // What the logo search keys on. `callsign_base` is
        // `callsign.split('-')[0]` for every row in the table, and a stored call
        // sign is reduced the same way.
        assert_eq!(remembered.base(), "WERN");
        // And it is what gets written back, so the next cold start has it too.
        assert_eq!(remembered.identity(), Some("WERN-FM"));

        // A LIVE ROW STILL WINS. The fallback is for when nothing resolves, not
        // a cache that could outrank the station actually on this dial here.
        let db = StationDb::open(&host_db_path()).unwrap();
        let here = fake::FakeLocation::default().position();
        let resolved = Slot {
            mhz: 88.7,
            row: resolve(Some(&db), 88.7, here),
            saved_call: Some("KSTALE".into()),
        };
        assert_eq!(resolved.base(), "WERN");
        assert_eq!(resolved.name(), "WERN");
    }

    #[test]
    fn the_picker_answers_from_the_real_table_and_reports_a_missing_fix() {
        let db = StationDb::open(&host_db_path()).unwrap();
        let located = build_picker(Some(&db), fake::FakeLocation::default(), None);
        let view = located.view(&[]);
        assert_eq!(view.state, NearbyState::List);
        // One row per dial, and no transmitter past its own reach
        // (`rank_nearby`) — at Madison that is 58 of the 120 in range, under the
        // 100 cap, so nothing is truncated any more.
        assert_eq!(view.stations.len(), 58);
        assert_eq!(view.stations[0].call, "WNWC");
        // NoGps is the difference between "nothing is in range" and "we do not
        // know where we are", and it must survive a working database.
        let lost = build_picker(Some(&db), fake::FakeLocation::no_fix(), None);
        assert_eq!(lost.view(&[]).state, NearbyState::NoGps);
    }

    /// A saved dial must light the row it saved, through `preset_key` rather
    /// than a float comparison that cannot round-trip.
    #[test]
    fn a_seeded_preset_marks_its_row_in_the_picker() {
        let db = StationDb::open(&host_db_path()).unwrap();
        let picker = build_picker(Some(&db), fake::FakeLocation::default(), None);
        let view = picker.view(&fake::SEED_PRESET_MHZ);
        let saved: Vec<&str> = view
            .stations
            .iter()
            .filter(|r| r.saved)
            .map(|r| r.freq.as_str())
            .collect();
        assert!(saved.contains(&"88.7"), "88.7 is a seeded preset: {saved:?}");
        assert!(saved.contains(&"98.1"));
        // And the ones that are not seeded stay unmarked.
        assert!(!saved.contains(&"90.9"));
    }

    /// The regression this replaced a stored field to prevent: the tuner reports
    /// its own frequency on connect, and a face that had cached "nothing is
    /// active" then drew the unsaved star over a dial plainly in the strip.
    #[test]
    fn the_active_slot_is_read_off_the_dial_not_remembered() {
        let slots: Vec<Slot> = fake::SEED_PRESET_MHZ
            .iter()
            .map(|&mhz| Slot { mhz, row: None, saved_call: None })
            .collect();
        assert_eq!(active_index(102.1, &slots), 0);
        assert_eq!(active_index(94.1, &slots), 5);
        // 105.5 is the value that does not round-trip through an f32, which is
        // why the match goes through `preset_key` and not `==`.
        assert_eq!(active_index(105.5, &slots), 2);
        // Off the strip, and an empty strip, are both -1 rather than a panic.
        assert_eq!(active_index(105.1, &slots), -1);
        assert_eq!(active_index(102.1, &[]), -1);
    }

    #[test]
    fn the_stamp_is_eight_characters_of_clock() {
        let s = stamp();
        assert_eq!(s.len(), 8);
        assert_eq!(s.as_bytes()[2], b':');
        assert_eq!(s.as_bytes()[5], b':');
        assert!(s.chars().filter(char::is_ascii_digit).count() == 6);
    }
}
