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
use std::collections::{HashMap, VecDeque};
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
    AppWindow, BatteryState, DiagAction, GenreColumn, HeroSnapshot, NearbyStation, NearbyTab,
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
    /// How many re-commands the LIVE hold has left. Re-armed when a step takes a
    /// hold, so a budget one press spent is not inherited by the next.
    reasserts_left: u8,
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
    /// Decoded art, keyed by call-sign base and the ladder box it was read at.
    ///
    /// NOT an optimisation. `push_presets` runs on every tune, every fix and
    /// every drain, and it renders up to eight tiles; without this, each of
    /// those would be a file read and a PNG decode on the UI thread of a 32-bit
    /// head unit. A `None` value is cached too — "this station has no art" is
    /// the common answer and is worth not re-deriving.
    art: HashMap<(String, u32), Option<slint::Image>>,
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
                    diag_on: saved.diag_on,
                    diag_overlay_on: saved.diag_overlay_on,
                    rds_capture_on: saved.rds_capture_on,
                    debug_on: saved.debug_on,
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
                warm_dial: warm.as_ref().map(|&(dial, _, _)| dial),
                launches,
                freq_seq: 0,
                hold: None,
                reassert: None,
                reasserts_left: 0,
                last_panel: None,
                busy: false,
                panel_action: None,
                callsigns,
                store,
                codec,
                worker,
                art: HashMap::new(),
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
            s.settings.log.push(
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
            service::Event::Saved { base } => {
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
                    s.art.retain(|(b, _), _| *b != key);
                    s.settings.log.push(&stamp(), &format!("logo saved: {key}"));
                }
                // Dismiss and tear down, in that order — `close_logo_search`
                // reads the target, which `close` then clears.
                self.ui().set_overlay(Overlay::None);
                self.close_logo_search();
                self.push_hero();
                self.push_presets();
                self.push_settings();
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
                        s.settings.log.push(
                            &at,
                            if playing {
                                "poll: the MCU handed FM back"
                            } else {
                                "poll: the MCU took FM away"
                            },
                        );
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
                            s.hold = None;
                            // Nothing left to owe: the front end is there.
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
                // Change-gated, and quiet in debug mode, both as CarFM has it:
                // the unchanged case is the common one, since the same group
                // repeats many times a second.
                if changed && !s.settings.debug_on {
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
                    // CarFM logs every accepted reading with what it becomes on
                    // the glyph, and suppresses it in debug mode where the
                    // structured sample carries the same figures. Carnyx has no
                    // structured sample, so debug mode simply goes quiet here.
                    if !s.settings.debug_on {
                        let lit = signal::level_to_lit(Some(f64::from(l.level)));
                        let shown = signal::describe(lit.as_ref());
                        let at = stamp();
                        s.settings.log.push(&at, &format!("level {} @ {} → {shown}", l.level, l.asked));
                    }
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
                let line = match gap {
                    Some(ms) => format!("panel key {code} ({named}) {action} +{ms}ms"),
                    None => format!("panel key {code} ({named}) {action}"),
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
                s.panel_action = panel_action(key, &action, anchor).or(s.panel_action);
            }
            TunerEvent::Illumination { ui_mode, .. } => {
                s.settings.log.push(&stamp(), &format!("illumination {ui_mode}"));
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
        ui.set_ident(ident.clone().into());
        // THE COMMITTED TARGET WHILE ONE IS IN FLIGHT. See `State::hold`: the
        // vendor walks its own bank on the same press, and without this the
        // driver watches the hero flick through stations they never chose.
        ui.set_freq_label(format_mhz(s.shown()).into());
        ui.set_in_band((FM_LO..=FM_HI).contains(&s.shown()));
        let active = s.active();
        ui.set_saved(active >= 0);
        ui.set_active_index(active);

        // The station's own name must not be stripped out of its own RadioText,
        // so the RESOLVED call sign goes in, never the PS: WIBA scrolls song
        // titles through PS, and a PS of "Walk" would strip "Walk This Way".
        let call = row
            .as_ref()
            .map(|r| r.callsign_base.clone())
            .or_else(|| learned.clone());
        ui.set_radio_text(
            rds::strip_station_from_rt(&st.rt, Some(s.shown()), call.as_deref()).into(),
        );
        ui.set_pty(rds::pty_label(st.pty).into());
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
        ui.set_logo(art.unwrap_or_default());
        ui.set_show_call(flags.show_call);
        ui.set_show_freq(flags.show_freq);
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
        let art: Vec<Option<slint::Image>> =
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
        ui.set_settings_tuner_details(ModelRc::from(Rc::new(VecModel::from(
            Vec::<TunerDetail>::new(),
        ))));

        ui.set_settings_sources(ModelRc::from(Rc::new(VecModel::from(
            cfg.sources(available)
                .iter()
                .map(|r| TunerSource {
                    name: r.name.as_str().into(),
                    kind: r.kind.as_str().into(),
                    badge: r.badge.as_str().into(),
                    badge_lit: r.badge_lit,
                    available: r.available,
                    selected: r.selected,
                })
                .collect::<Vec<_>>(),
        ))));

        ui.set_settings_autostart(cfg.autostart);
        ui.set_settings_theme_chips(strings(&settings::theme_chips()));
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

        ui.set_settings_diag_on(cfg.diag_on);
        ui.set_settings_diag_overlay_on(cfg.diag_overlay_on);
        ui.set_settings_rds_capture_on(cfg.rds_capture_on);
        ui.set_settings_debug_on(cfg.debug_on);
        // The log is a 200-line ring and this is a 200-string model. Same rule
        // again, and it matters most here: the diagnostics overlay is the one a
        // driver leaves open while watching the radio misbehave. The cache write
        // waits until the borrow is released, at the foot of this function.
        let lines = cfg.log.lines();
        let diag_changed = s.last_diag.as_ref() != Some(&lines);
        if diag_changed {
            ui.set_settings_diag_lines(strings(&lines));
        }
        ui.set_settings_diag_actions(ModelRc::from(Rc::new(VecModel::from(
            cfg.actions(nwd_active)
                .iter()
                .map(|a| DiagAction {
                    label: a.label.as_str().into(),
                    divider_above: a.divider_above,
                })
                .collect::<Vec<_>>(),
        ))));

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
        drop(s);
        if diag_changed {
            self.state.borrow_mut().last_diag = Some(lines);
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

    fn push_logo_search(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let view = s.logo.view();
        let brand = brand_color(&s.logo.target().map(|t| t.base.clone()).unwrap_or_default());
        crate::logos::ui::apply(&ui, &view, s.logo.cells(), s.logo_art.as_ref(), brand);
    }

    // ── Stored art ───────────────────────────────────────────────────────────

    /// Read one station's stored rendition off disk and decode it.
    ///
    /// `None` covers every ordinary state and they are NOT distinguished here,
    /// because no surface can do anything different with them: no codec (the
    /// host), no logo for this station, an unreadable file, a decode that
    /// failed. Each one means the same thing to the face — draw the call-sign
    /// box.
    fn read_art(&self, base: &str, box_dp: Option<f32>) -> Option<Raster> {
        // Both handles cloned out BEFORE the read: this decodes a PNG through
        // JNI, and holding a `RefCell` borrow across that is a lock held across
        // a call into another runtime.
        let (store, codec) = {
            let s = self.state.borrow();
            (s.store.clone(), s.codec.clone()?)
        };
        let scale = self.ui().window().scale_factor();
        crate::logos::assign::read_rendition(&store, &*codec, base, box_dp, scale)
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
    fn art_for(&self, base: &str, box_dp: Option<f32>) -> Option<slint::Image> {
        if base.is_empty() {
            return None;
        }
        let key = (base.to_uppercase(), box_dp.unwrap_or(0.0).round() as u32);
        // Cloned out in its own statement for the reason spelt out in
        // `drain_events`: an `if let` scrutinee holds its borrow across the body
        // on edition 2021. This one is only a shared borrow and the read below
        // is too, so it never paniced — but it is one edit away from doing so.
        let hit = self.state.borrow().art.get(&key).cloned();
        if let Some(hit) = hit {
            return hit;
        }
        let image = self.read_art(&key.0, box_dp).as_ref().map(crate::logos::ui::to_image);
        self.state.borrow_mut().art.insert(key, image.clone());
        image
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
        on!(on_settings_pick_source, |app, i| {
            if let Some(&src) = settings::Source::ORDER.get(i as usize) {
                app.state.borrow_mut().settings.selected = src;
            }
            app.push_settings();
        });
        on!(on_settings_set_autostart, |app, v| {
            let _ = v;
            // THE FRAMEWORK EDGE, like the battery row above it. "Start radio on
            // boot" needs something to run at boot, and Carnyx has no boot
            // receiver: cargo-apk's manifest struct has one activity and no
            // `receiver` field at all, so `BOOT_COMPLETED` and the NWD unit's
            // own `com.nwd.ACTION_OS_WAKE_UP` cannot be declared. See the
            // Cargo.toml comment beside `config_changes`.
            //
            // CarFM's toggle is not the same control either. Its key is
            // `@vibesdr/car_autostart` (services/carMode.ts:13), it is VibeSDR
            // lineage, and what it starts is a plugged-in RTL-SDR — hardware this
            // app does not have and, by the provenance rule, will not inherit.
            //
            // So the row says so in the log rather than flipping a flag nothing
            // can honour. It was flipping one, and persisting it.
            app.log_unavailable("autostart needs a boot receiver, which cargo-apk cannot declare");
        });
        on!(on_settings_set_theme, |app, t| {
            let t: SharedString = t;
            if let Some(theme) = settings::Theme::parse(t.as_str()) {
                app.state.borrow_mut().settings.theme = theme;
                app.apply_theme(theme);
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
        on!(on_settings_set_diag, |app, v| {
            app.state.borrow_mut().settings.set_diag(v);
            app.push_settings();
        });
        on!(on_settings_set_diag_overlay, |app, v| {
            app.state.borrow_mut().settings.diag_overlay_on = v;
            app.push_settings();
        });
        on!(on_settings_set_rds_capture, |app, v| {
            app.state.borrow_mut().settings.rds_capture_on = v;
            app.push_settings();
        });
        on!(on_settings_set_debug, |app, v| {
            app.state.borrow_mut().settings.set_debug(v);
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
            app.state.borrow_mut().logo.pick(i);
            app.push_logo_search();
        });
        on!(on_logo_search_confirm, |app| {
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
            diag_on: s.settings.diag_on,
            diag_overlay_on: s.settings.diag_overlay_on,
            rds_capture_on: s.settings.rds_capture_on,
            debug_on: s.settings.debug_on,
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
    /// "Clear log" and "Save to file" are implemented. The rest still cross the
    /// framework edge with nothing behind them, and each says so by name in the
    /// log it would have written to — more useful than a silent no-op, and honest
    /// about what has never run. See docs/TASKS.md #87 for what each still needs.
    fn run_diag_action(self: &Rc<App>, index: i32) {
        let (action, label) = {
            let s = self.state.borrow();
            let nwd_active =
                s.tuner.is_available() && s.settings.selected == settings::Source::Nwd;
            let rows = s.settings.actions(nwd_active);
            match rows.get(index as usize) {
                Some(a) => (a.action, a.label.clone()),
                None => return,
            }
        };
        {
            let mut s = self.state.borrow_mut();
            match action {
                settings::Action::ClearLog => s.settings.log.clear(),
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
                _ => {
                    let at = stamp();
                    s.settings
                        .log
                        .push(&at, &format!("{label}: not available without the head unit"));
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
    pub fn log_platform(self: &Rc<App>, line: &str) {
        self.log_unavailable(line);
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
        let existing = if key.is_empty() { None } else { self.read_art(&key, None) };
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
    fn close_logo_search(&self) {
        if self.state.borrow().logo.target().is_none() {
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

fn to_preset(slot: &Slot, logo: Option<slint::Image>) -> Preset {
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
        logo: logo.unwrap_or_default(),
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

    /// Pretend the vendor service retuned its own bank and reported it.
    ///
    /// `FakeTuner`'s scale is 100, and slot -1 is "not a preset in this bank",
    /// which is what the tuner sends for an ordinary retune.
    fn vendor_reports(mhz: f32) {
        crate::android::ingest_frequency(0, (mhz * 100.0).round() as i32, String::new(), -1);
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

        // ── The release edge, with the string the DEVICE really sends. Both
        // edges inside one drain collapse, because `panel_action` is one Option.
        driver.tune_for_test(strip[0]);
        driver.drain_events();
        let real = "com.nwd.action.ACTION_KEY_VALUE";
        crate::android::ingest_panel_key(62, real.into());
        crate::android::ingest_panel_key(62, real.into());
        driver.drain_events();
        assert_eq!(label(&ui), format_mhz(strip[1]).to_string(), "one step, not two");
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

        // Released — so an ordinary retune elsewhere is shown again at once,
        // which is what makes this a bounded exception and not a lie.
        vendor_reports(93.1);
        driver.drain_events();
        driver.push_all();
        assert_eq!(
            ui.get_freq_label().to_string(),
            "93.1",
            "with nothing in flight the face is honest again"
        );
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

        // MORE LINES THAN THE RING HOLDS, so what is asserted is "everything the
        // app still has", not "everything that ever happened" — the ring drops
        // the oldest and the file must contain exactly what survived.
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
        assert_eq!(lines.len(), settings::DiagLog::CAP, "the whole ring, and no more");
        assert_eq!(
            lines.first().map(|l| l.trim()),
            Some(format!("00:00:00  line {}", over - settings::DiagLog::CAP).trim()),
            "starting at the oldest line the ring still holds"
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

    /// Where a labelled diagnostics row currently sits, so a test names the row
    /// rather than an index that a reorder would silently change.
    fn row_index(driver: &Rc<App>, label: &str) -> i32 {
        let s = driver.state.borrow();
        let nwd_active = s.tuner.is_available() && s.settings.selected == settings::Source::Nwd;
        s.settings
            .actions(nwd_active)
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
        let dressed = to_preset(&slot, Some(px));
        assert!(dressed.has_logo);
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
