//! The Android tuner spine: the seam between the head unit's binder service and
//! everything in Carnyx that is worth testing.
//!
//! ## The shape of it
//!
//! ```text
//!   vendor RadioFeature (AIDL)  ──┐
//!   com.nwd.app.NwdFmManager    ──┤  java/com/ninthfreak/carnyx/NwdBridge.java
//!   MCU broadcasts              ──┘        │  raw ints and strings only
//!                                          ▼
//!                            ingest_* (this file)  ──►  TunerEvent  ──►  the app
//!                                          ▲
//!                                     FakeTuner
//! ```
//!
//! Everything the device can say arrives at the `ingest_*` functions below as
//! raw numbers, and every DECISION about those numbers — which MHz scale the
//! unit is using, whether a level reading can be believed, whether a preset slot
//! is a slot at all, what a panel-key code means — is made here in Rust. That is
//! not tidiness: this container has no head unit, no NWD service and no binder,
//! so the only logic that can be verified before the owner flashes an APK is the
//! logic on this side of the seam. `FakeTuner` drives the same `ingest_*`
//! functions the real device drives, which is what makes the tests below tests
//! of the shipping path rather than of a parallel one.
//!
//! ## What is NOT tested, and cannot be here
//!
//! Everything past `NwdBridge.java`: binding the service, the AIDL transaction
//! codes, the reflection into `com.nwd.app.NwdFmManager`, the broadcasts, the
//! dex loading and the JNI registration. Those have compiled and nothing more.
//!
//! The device-derived comments are carried over from CarFM's
//! `NwdRadioModule.kt`. They are the evidence for the non-obvious rules here and
//! cost far more to re-learn than to keep.

use std::sync::{Arc, Mutex};

#[cfg(target_os = "android")]
pub mod alert;
#[cfg(target_os = "android")]
mod dex;
#[cfg(target_os = "android")]
pub mod location;
#[cfg(target_os = "android")]
pub mod nav;
#[cfg(target_os = "android")]
pub mod net;
#[cfg(target_os = "android")]
pub mod nwd;
#[cfg(target_os = "android")]
pub mod probe;
#[cfg(target_os = "android")]
pub mod service;
#[cfg(target_os = "android")]
pub mod stock;
#[cfg(target_os = "android")]
pub mod wake;

#[cfg(target_os = "android")]
pub use nwd::{init, NwdTuner};

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunerError {
    /// No tuner service on this unit, or `attach` has not run.
    Unavailable(String),
    /// The tuner is not bound, so there is nothing to command.
    NotConnected,
    /// The unit's MHz-to-raw scale has not been observed yet, so a tune request
    /// cannot be converted. See [`Calibration`].
    NotCalibrated,
    /// The Java side refused or threw. The string is for a log, not a user.
    Java(String),
}

impl std::fmt::Display for TunerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(why) => write!(f, "tuner unavailable: {why}"),
            Self::NotConnected => write!(f, "tuner not connected"),
            Self::NotCalibrated => write!(f, "tuner frequency scale not learned yet"),
            Self::Java(why) => write!(f, "tuner call failed: {why}"),
        }
    }
}

impl std::error::Error for TunerError {}

// ── Frequency calibration ────────────────────────────────────────────────────

/// The unit's own frequency scale and current band, learned from what the tuner
/// reports rather than hard-coded.
///
/// The raw integer the vendor service uses for a frequency is not in fixed
/// units: it can be kHz, tens of kHz, or MHz×100, and it differs between head
/// units. CarFM derives the multiplier from whatever the tuner is currently
/// tuned to, and both halves of that rule were learned by breaking:
///
/// * THE BAND MUST BE TRACKED. It was captured once at connect and never
///   updated, while `tune` passed it to `setCurrentFrequency` on every tune — so
///   the moment the head unit was switched FM1 → FM2 the app carried on tuning
///   against the old band. The log of 31 July 2026 caught exactly that: at 07:28
///   the tuner reported band 0 with the user's own six presets, and by 07:55
///   band 1 with the factory list, while the connect-time value stayed 0 for the
///   whole session. Each band has its own preset bank.
///
/// * THE SCALE MUST BE RE-DERIVED, AND ONLY EVER WIDENED. Calibrating once at
///   connect meant calibrating against whatever the unit happened to be tuned to
///   — last on AM, raw 1000, which reads as a ×10 scale. FM then displayed
///   "1059.0 MHz" and a tune to 102.5 sent 1025, outside the 8750..10790 plan.
///   The dial spins and the radio does not move, for the rest of the session.
///   Widening is safe because an AM reading can never exceed an FM one: 8750
///   (×100) and 87500 (×1000) are both far above any AM raw value, so taking the
///   maximum settles on the FM scale and stays there.
///
/// ONE DELIBERATE DIFFERENCE FROM CarFM. There, the multiplier starts at 1000
/// and connect ASSIGNS from the first reading, so a `getCurrentFrequency` that
/// returns 0 — which the AIDL does when the front end is cold — silently sets
/// the scale to ×1 and every tune is wrong until a notification widens it. Here
/// the scale starts UNKNOWN and `raw_for` returns `None` until something real
/// has been seen, so that state is a refused tune with a reason instead of a
/// dial that spins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Calibration {
    mult: Option<i32>,
    band: i8,
}

impl Calibration {
    pub const NEW: Self = Self { mult: None, band: 0 };

    pub const fn new() -> Self {
        Self::NEW
    }

    /// The multiplier a raw reading implies, verbatim from CarFM's ladder.
    ///
    /// The thresholds are wide apart on purpose — they only have to separate
    /// 8750..10790 from 87500..107900 from an AM dial in the hundreds — so a
    /// reading anywhere inside a plausible band lands in the right arm.
    fn scale_for(raw: i32) -> i32 {
        if raw > 50_000 {
            1000
        } else if raw > 5_000 {
            100
        } else if raw > 500 {
            10
        } else {
            1
        }
    }

    /// Fold in a reading from the tuner. Called for the connect-time seed and
    /// for every frequency notification alike.
    pub fn observe(&mut self, band: i8, raw: i32) {
        self.band = band;
        if raw <= 0 {
            // A cold front end reads 0. That is not a scale of ×1, it is no
            // information, and treating it as information is the bug described
            // above.
            return;
        }
        let m = Self::scale_for(raw);
        if self.mult.is_none_or(|cur| m > cur) {
            self.mult = Some(m);
        }
    }

    /// The band byte the tuner last reported. This is what a tune must be sent
    /// against, never a remembered one.
    pub fn band(&self) -> i8 {
        self.band
    }

    /// The learned multiplier, or `None` while the scale is still unknown.
    pub fn multiplier(&self) -> Option<i32> {
        self.mult
    }

    /// A raw reading as MHz, or `None` while the scale is unknown.
    pub fn mhz_for(&self, raw: i32) -> Option<f32> {
        self.mult.map(|m| raw as f32 / m as f32)
    }

    /// MHz as the raw integer the vendor service expects.
    ///
    /// Rounds half away from zero, which for the positive frequencies this ever
    /// sees is the same thing JavaScript's `Math.round` did.
    pub fn raw_for(&self, mhz: f32) -> Option<i32> {
        self.mult.map(|m| (mhz as f64 * m as f64).round() as i32)
    }
}

impl Default for Calibration {
    fn default() -> Self {
        Self::NEW
    }
}

// ── Band plan ────────────────────────────────────────────────────────────────

/// One band from the vendor's `getRadioPoint()`, in the tuner's own units.
///
/// Observed on the device 2026-07-31: FM `8750 / 10790 / 20` (87.5–107.9 MHz in
/// 200 kHz steps), AM `530 / 1710 / 10` (530–1710 kHz in 10 kHz steps), and a
/// third entry of all zeros. The units differ per band — FM in tens of kHz, AM
/// in kHz — so nothing is scaled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BandPoint {
    pub lo: i32,
    pub hi: i32,
    pub step: i32,
}

/// The band plan from the flat `[lo, hi, step, …]` the Java side returns.
///
/// The all-zero third entry is dropped: it is a slot the firmware does not use,
/// and a band with no range would be a trap for anything that iterates bands.
pub fn band_plan_from_raw(flat: &[i32]) -> Vec<BandPoint> {
    flat.chunks_exact(3)
        .map(|c| BandPoint { lo: c[0], hi: c[1], step: c[2] })
        .filter(|b| b.lo != 0 || b.hi != 0 || b.step != 0)
        .collect()
}

// ── Panel keys ───────────────────────────────────────────────────────────────

/// A steering-wheel or front-panel press, decoded from the MCU's own dispatch
/// table (decompiled from the vendor service's `handlePanelKey`).
///
/// The wheel is not a media key on this unit — the MCU broadcasts
/// `com.nwd.action.ACTION_KEY_VALUE` and the event never enters Android's input
/// pipeline at all, which is why capturing it any other way saw nothing.
///
/// ## THIS IS A SOFTWARE TABLE, NOT AN INVENTORY OF BUTTONS
///
/// Every variant below is a row of the vendor's decompiled dispatch table, which
/// covers its whole product line. It says what the SERVICE would do with a code,
/// not that anything on this fascia can send one. Read as a button list it is
/// simply wrong, and it has already been read that way once: an audit reported
/// that a "wheel-driven seek" never sets the scanning flag, which is true of the
/// code and describes an operation this hardware cannot start.
///
/// WHAT THE OWNER ACTUALLY HAS, stated so the mistake is not available again:
///
/// * Steering wheel — `ch+` / `ch-`, volume up/down, and a `mode` button that
///   switches apps.
/// * Head unit — the Android navigation buttons and a volume control.
///
/// So `ch+` and `ch-` are [`Self::PresetNext`] and [`Self::PresetPrev`], and
/// THEY ARE THE ONLY TWO THIS APP CAN EVER RECEIVE AS RADIO COMMANDS. There is
/// no seek button, no search button, no band button, no AMS and no intro. The
/// variants for those are decode-only: they exist so the diagnostics log can
/// name a code rather than print "unknown", and so a different fascia is
/// understood if this ever meets one.
///
/// Volume and `mode` are handled by the MCU and the system; whether they also
/// broadcast is unknown. Code 14 — eight times in CarFM's drive log of
/// 2026-08-03, in no vendor table — is the open question there, and one of those
/// two buttons is the obvious suspect. Not investigated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKey {
    /// Decode-only on this unit — no such button. See the type's own note.
    ChangeBand,
    /// Decode-only on this unit — no such button.
    SearchUp,
    /// Decode-only on this unit — no such button.
    SearchDown,
    /// Decode-only on this unit — no such button.
    SeekUp,
    /// Decode-only on this unit — no such button.
    SeekDown,
    /// Auto memory store. Decode-only on this unit — no such button.
    Ams,
    /// Decode-only on this unit — no such button.
    Intro,
    /// `ch+` on the wheel, and ONE OF THE ONLY TWO CODES THIS UNIT SENDS.
    ///
    /// Steps the SERVICE's own hardware preset bank, not ours. A normal
    /// broadcast cannot be cancelled, so the service acts regardless and the app
    /// reasserts its own preset immediately after.
    PresetNext,
    /// `ch-` on the wheel. The other of the two. See [`Self::PresetNext`].
    PresetPrev,
    /// Decode-only on this unit — no such button.
    ChangeFmBand,
    /// Decode-only on this unit — no such button.
    ChangeAmBand,
}

impl PanelKey {
    /// Decode a raw MCU key code.
    ///
    /// `None` is the interesting answer and is why the raw code travels with the
    /// event: key 14 arrived eight times in CarFM's drive log of 2026-08-03 and
    /// is not in the vendor's table, so nobody knows what button it is.
    ///
    /// Two codes map to each search direction. CarFM's lookup returned whichever
    /// name it hit first, so 60 displayed as "search up alt"; here both collapse
    /// to one variant, because they are one button as far as the app is
    /// concerned and the raw code is still on the event for diagnostics.
    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            4 => Self::ChangeBand,
            5 | 60 => Self::SearchUp,
            6 | 59 => Self::SearchDown,
            16 => Self::SeekUp,
            17 => Self::SeekDown,
            46 => Self::Ams,
            61 => Self::Intro,
            62 => Self::PresetNext,
            63 => Self::PresetPrev,
            72 => Self::ChangeFmBand,
            73 => Self::ChangeAmBand,
            _ => return None,
        })
    }

    /// The label a diagnostics line prints. Finished text: Slint decides nothing.
    pub fn label(self) -> &'static str {
        match self {
            Self::ChangeBand => "change band",
            Self::SearchUp => "search up",
            Self::SearchDown => "search down",
            Self::SeekUp => "seek up",
            Self::SeekDown => "seek down",
            Self::Ams => "auto memory store",
            Self::Intro => "intro scan",
            Self::PresetNext => "preset next",
            Self::PresetPrev => "preset prev",
            Self::ChangeFmBand => "change FM band",
            Self::ChangeAmBand => "change AM band",
        }
    }
}

// ── RDS groups ───────────────────────────────────────────────────────────────

/// One already-synchronised RDS group: four 16-bit blocks.
///
/// The vendor hands these over as 16 hex characters from
/// `NwdFmManager.getRadioRDSDataArm()` — the channel the bound AIDL never
/// exposes, and the reason RadioText is reachable on this unit at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdsGroup(pub [u16; 4]);

/// Parse the vendor's 16-hex-character group.
///
/// Anything that is not exactly four blocks of four hex digits is rejected
/// rather than padded or truncated: a short read is a poll that caught the
/// buffer mid-refresh, and half a group decoded as a whole one would be silent
/// corruption in the decoder's consensus counters.
pub fn parse_group_hex(hex: &str) -> Option<RdsGroup> {
    if hex.len() != 16 {
        return None;
    }
    let mut blocks = [0u16; 4];
    for (i, block) in blocks.iter_mut().enumerate() {
        *block = u16::from_str_radix(hex.get(i * 4..i * 4 + 4)?, 16).ok()?;
    }
    Some(RdsGroup(blocks))
}

// ── Events ───────────────────────────────────────────────────────────────────

/// The tuner is bound and has reported its opening state.
///
/// There is no `stereo` here on purpose. `isStreroOn()` is stuck true on this
/// firmware — it reads true on dead air — so seeding from it put a STEREO on
/// CarFM's face that nothing ever corrected. [`TunerEvent::Stereo`] is the only
/// trustworthy source.
#[derive(Debug, Clone, PartialEq)]
pub struct Connected {
    pub band: i8,
    pub raw: i32,
    pub mhz: Option<f32>,
    pub ps: String,
    /// `getRtMessage()` is hardcoded to `""` on this unit's radio manager, so
    /// this is almost always empty and RadioText comes from the raw groups.
    pub rt: String,
    pub pty: i32,
    /// Whether `registCallback` was accepted. When false, only polling works.
    pub registered: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyChange {
    pub band: i8,
    pub raw: i32,
    pub mhz: Option<f32>,
    pub ps: String,
    /// The tuner's own preset slot for this frequency in the CURRENT bank, 1–6.
    /// `None` when the frequency is not one of the hardware presets. It is not a
    /// signal level, which is what it was first mistaken for.
    pub preset_slot: Option<u8>,
}

/// One signal reading from `NwdFmManager.seek(currentFrequency)`.
///
/// The number is an ORDINAL and never a unit. Printing "55 dB" would be
/// inventing one: the evidence does not distinguish dBµV from an arbitrary chip
/// register, no chip part number appears anywhere in the vendor service, and a
/// different class in that same service carries negative dBm-shaped constants
/// for something else entirely.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelReading {
    pub level: i32,
    pub asked: i32,
    pub landed: i32,
    /// The tuner stayed on the frequency we asked about — the same check
    /// `AWNative.seek` makes before it believes a level. When false the level is
    /// meaningless and must not be shown; on the drive logs the overwhelming
    /// case was `landed = 0`, the tuner saying it was not ready.
    pub trustworthy: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TunerEvent {
    Connected(Connected),
    /// One OsmAnd navigation update, RAW — the three integers exactly as they
    /// crossed the AIDL boundary. `crate::nav` decides what they mean; nothing
    /// on this path does, which is why the sentinels are still in it.
    Nav { distance_to: i32, turn_type: i32, left_side: bool },
    /// One voice-router announcement, as its two unjoined lists. See
    /// `crate::nav::Nav::speak` for which is preferred and why.
    NavVoice { cmds: Vec<String>, played: Vec<String> },
    /// One `getAppInfo` poll answer — the half of the feed that has words in it.
    NavInfo(NavRoute),
    /// A location fix, or the loss of one.
    ///
    /// Not a tuner event, and it travels on the tuner's queue anyway: there is
    /// one device, one drain and one hop back to the UI thread, and a second
    /// queue would be a second thing to get the threading wrong in. `fix: false`
    /// is the honest report of a unit that has an antenna and no sky.
    Position {
        lat: f64,
        lon: f64,
        fix: bool,
        in_motion: bool,
    },
    /// One line for the diagnostics log, from a subsystem with no other way in.
    ///
    /// `android_main` is where the location binding either takes or does not,
    /// and it holds no `App` — the queue is the only route from there to a panel
    /// a person can read. Carries no state and changes nothing on the face: if a
    /// Note is the ONLY evidence of something, that something is under-reported
    /// and belongs in a property instead.
    Note(String),
    /// The bind was accepted but the service never connected, or it was refused.
    ConnectFailed(String),
    Disconnected,
    Frequency(FrequencyChange),
    /// One turn of the vendor-getter poll. See [`start_state_poll`].
    ///
    /// Carries the whole snapshot rather than the two facts the face acts on,
    /// because the other four — PS, RadioText, PTY and the stuck stereo getter —
    /// are what a diagnostic prints, and splitting them out here would put the
    /// decision about which of them is trustworthy in the wrong file. This is the
    /// ingest edge; it reports, and the app decides.
    Snapshot(TunerSnapshot),
    RadioText(String),
    Stereo(bool),
    Pty(i32),
    ScanState(i32),
    RadioState(i32),
    RdsGroup(RdsGroup),
    Level(LevelReading),
    PanelKey {
        code: i32,
        key: Option<PanelKey>,
        action: String,
    },
    /// The vendor's illumination (headlights) broadcast, dumped verbatim
    /// alongside Android's own night flag read at the same instant. The type of
    /// `extra_ill_state` is unknown, so nothing here guesses a getter — the
    /// comparison between `extras` and `ui_mode` is the whole point.
    Illumination {
        action: String,
        extras: String,
        ui_mode: String,
    },
    /// The head unit is going to sleep, and this app has ALREADY handed the FM
    /// source back on the thread that heard the broadcast.
    ///
    /// `action` is the broadcast that said so, carried rather than collapsed to a
    /// flag because the two are not equally trustworthy — see
    /// `NwdBridge.startSleepWatch`. `com.nwd.ACTION_ACCOFF_UPDATE` is the real
    /// signal; `android.intent.action.SCREEN_OFF` is a proxy a screen timeout
    /// also trips. Which one arrived is the first thing a drive log has to answer.
    Sleep {
        action: String,
        /// What `NwdBridge.releaseSource` managed on the receiver's own thread,
        /// or why it did not try. An OUTCOME, not an attempt — the queued
        /// release that follows is a re-send, and only this ran before the SoC
        /// was cut. Empty from a caller that has no Java behind it.
        release: String,
    },
}

// ── Shared state and the event sink ──────────────────────────────────────────

type Sink = Arc<dyn Fn(TunerEvent) + Send + Sync + 'static>;

struct Shared {
    cal: Calibration,
    sink: Option<Sink>,
    /// The motion verdict, kept because the thresholds have hysteresis and
    /// hysteresis is a function of the answer you last gave.
    moving: bool,
}

static SHARED: Mutex<Shared> =
    Mutex::new(Shared { cal: Calibration::NEW, sink: None, moving: false });

/// Lock without ever panicking on poison.
///
/// A panic inside a sink callback poisons this mutex. Propagating that would
/// turn one bad frame into a permanently dead radio, in a car, so the state is
/// taken back instead. Nothing behind this lock has an invariant a half-finished
/// callback could have broken: it is a multiplier, a band byte and a callback
/// pointer.
fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Install the callback every tuner event is delivered to.
///
/// Called on whatever thread the vendor happens to use — a binder thread, the
/// RDS pump, the level watch — so anything that touches the UI must hop to the
/// Slint event loop itself.
pub fn set_event_sink(sink: impl Fn(TunerEvent) + Send + Sync + 'static) {
    shared().sink = Some(Arc::new(sink));
}

pub fn clear_event_sink() {
    shared().sink = None;
}

/// The calibration as it stands. A copy: nothing outside this module mutates it.
pub fn calibration() -> Calibration {
    shared().cal
}

/// Forget everything learned about the device. Only for tests and for a
/// deliberate re-bind.
///
/// The MOTION VERDICT goes too, and it must: it is hysteretic, so it is a
/// function of the last answer, and a test that inherited `moving` from whatever
/// ran before it would pass or fail on test ORDER. Nothing else behind this lock
/// carries state across a re-bind.
pub fn reset_calibration() {
    let mut g = shared();
    g.cal = Calibration::NEW;
    g.moving = false;
}

/// Deliver an event.
///
/// The sink is cloned out and the lock RELEASED before the call. A sink that
/// tunes in response to an event would otherwise re-enter this mutex and
/// deadlock the radio.
fn emit(event: TunerEvent) {
    let sink = shared().sink.clone();
    if let Some(sink) = sink {
        sink(event);
    }
}

// ── The ingest edge ──────────────────────────────────────────────────────────
//
// Everything the device says enters here, as the raw values the vendor produced.
// `nwd.rs` calls these from JNI; `FakeTuner` calls the same ones. Keeping the
// two on one path is what makes the tests below tests of the shipping code.

pub fn ingest_connected(band: i32, raw: i32, ps: String, rt: String, pty: i32, registered: bool) {
    let band = band as i8;
    let mhz = {
        let mut g = shared();
        g.cal.observe(band, raw);
        g.cal.mhz_for(raw)
    };
    emit(TunerEvent::Connected(Connected { band, raw, mhz, ps, rt, pty, registered }));
}

/// A fix from the platform's LocationManager.
///
/// Guards the coordinates rather than trusting them: Android will hand out a
/// (0, 0) fix from a provider that has nothing, and Null Island is 700 km off
/// the Gulf of Guinea, where the nearest FM station is nobody's. An impossible
/// pair is reported as NO fix, which the picker already knows how to draw.
/// Above this the car is moving; below the other, it has stopped.
///
/// CarFM's own pair, converted from mph (`services/motion.ts`, MOVING_ON_MPH 5 /
/// MOVING_OFF_MPH 3). TWO thresholds, not one: a single cut flaps every time GPS
/// speed jitters across it while waiting at lights, and every flap is a driving
/// glyph blinking and a reorder mode being refused and allowed by turns.
pub const MOVING_ON_MPS: f32 = 2.235; // 5 mph
pub const MOVING_OFF_MPS: f32 = 1.341; // 3 mph

/// Whether the car is moving, given the last verdict.
///
/// `has_speed` false is STATIONARY, not unknown: only some providers report
/// speed, and a fix that carries none is no evidence of movement. Guessing from
/// successive positions would be the alternative, and it produces motion for a
/// parked car whose fix wanders.
pub fn settle_motion(was_moving: bool, speed_mps: f32, has_speed: bool) -> bool {
    if !has_speed || !speed_mps.is_finite() || speed_mps < 0.0 {
        return false;
    }
    if was_moving {
        speed_mps >= MOVING_OFF_MPS
    } else {
        speed_mps >= MOVING_ON_MPS
    }
}

pub fn ingest_position(lat: f64, lon: f64, fix: bool, speed_mps: f32, has_speed: bool) {
    let sane = lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0);
    let in_motion = {
        let mut g = shared();
        // A lost fix is not a moving car. Without this the verdict would latch
        // on at the moment the antenna went dark and stay there.
        g.moving = fix && sane && settle_motion(g.moving, speed_mps, has_speed);
        g.moving
    };
    emit(TunerEvent::Position { lat, lon, fix: fix && sane, in_motion });
}

/// Put one line in the diagnostics log from outside the App. See
/// [`TunerEvent::Note`].
pub fn ingest_note(line: String) {
    emit(TunerEvent::Note(line));
}

// ── The vendor-getter poll ───────────────────────────────────────────────────

/// Which poll is the live one. Bumped by every start and every stop, so a thread
/// whose number no longer matches knows it has been superseded and exits.
///
/// The same generation trick `NwdBridge.startRdsPump` and `startLevelWatch` use
/// on the Java side, and for the same reason: there is no way to interrupt a
/// sleeping thread from here, so the thread has to ask.
static POLL_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How finely the poll's sleep is sliced.
///
/// Not a cadence — the cadence is the caller's `interval`. This is how long a
/// stopped poll can still be asleep for, and it matters most off the head unit:
/// the probes build one App after another in one process, and a poll thread from
/// the previous case that woke up and emitted would be a phantom event in the
/// next one.
const POLL_SLICE: std::time::Duration = std::time::Duration::from_millis(100);

/// Read the vendor's own getters, on a thread of their own, for as long as
/// nobody supersedes it.
///
/// OFF THE UI THREAD, and that is the whole point of this function. The three
/// getters are binder calls into the vendor service; CarFM makes them from React
/// Native's native-modules thread and never from the UI thread, and the first
/// version of this in Carnyx used a `slint::Timer`, which is the UI thread. A
/// vendor service that blocks would have hitched the face every 1.5 seconds.
///
/// It sleeps BEFORE its first read, matching `setInterval` rather than
/// `startLevelWatch`: connect has already taken a snapshot of everything this
/// would report.
///
/// The generation is re-checked AFTER the snapshot and before the emit, not just
/// before the read. A thread that was already inside `snapshot()` when it was
/// superseded would otherwise deliver one stale reading into whatever came next.
pub fn start_state_poll(tuner: Arc<dyn Tuner>, interval: std::time::Duration) {
    use std::sync::atomic::Ordering;
    let mine = POLL_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let spawned = std::thread::Builder::new()
        .name("carnyx-state-poll".into())
        .spawn(move || {
            let mut waited = std::time::Duration::ZERO;
            loop {
                if POLL_GEN.load(Ordering::SeqCst) != mine {
                    return;
                }
                if waited < interval {
                    let slice = POLL_SLICE.min(interval - waited);
                    std::thread::sleep(slice);
                    waited += slice;
                    continue;
                }
                waited = std::time::Duration::ZERO;
                let snap = tuner.snapshot();
                if POLL_GEN.load(Ordering::SeqCst) != mine {
                    return;
                }
                if let Some(snap) = snap {
                    emit(TunerEvent::Snapshot(snap));
                }
            }
        });
    if spawned.is_err() {
        // A process that cannot spawn a thread has worse problems, but the face
        // must still come up — and silently having no poll is exactly the class
        // of gap this poll was written to close.
        ingest_note("poll: could not start the state thread".into());
    }
}

/// Retire the current poll. Idempotent; safe to call when none is running.
pub fn stop_state_poll() {
    POLL_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

pub fn ingest_connect_failed(reason: String) {
    emit(TunerEvent::ConnectFailed(reason));
}

pub fn ingest_disconnected() {
    emit(TunerEvent::Disconnected);
}

pub fn ingest_frequency(band: i32, raw: i32, ps: String, slot: i32) {
    let band = band as i8;
    let mhz = {
        let mut g = shared();
        g.cal.observe(band, raw);
        g.cal.mhz_for(raw)
    };
    // 1..=6 are the hardware preset slots; the tuner sends -1 for "not a preset
    // in this bank". Anything else is a value the table has never produced, and
    // presenting it as a slot would be inventing one.
    let preset_slot = (1..=6).contains(&slot).then_some(slot as u8);
    emit(TunerEvent::Frequency(FrequencyChange { band, raw, mhz, ps, preset_slot }));
}

pub fn ingest_radio_text(rt: String) {
    emit(TunerEvent::RadioText(rt));
}

/// One raw group from the pump. A malformed read is dropped here rather than
/// passed on — see [`parse_group_hex`].
pub fn ingest_rds_group(hex: &str) {
    if let Some(group) = parse_group_hex(hex) {
        emit(TunerEvent::RdsGroup(group));
    }
}

pub fn ingest_stereo(on: bool) {
    emit(TunerEvent::Stereo(on));
}

pub fn ingest_pty(pty: i32) {
    emit(TunerEvent::Pty(pty));
}

pub fn ingest_scan_state(state: i32) {
    emit(TunerEvent::ScanState(state));
}

pub fn ingest_radio_state(state: i32) {
    emit(TunerEvent::RadioState(state));
}

pub fn ingest_level(level: i32, asked: i32, landed: i32, ok: bool, error: Option<String>) {
    // `ok` only says the call itself returned an integer. Whether the READING can
    // be believed is the equality check, and there IS one definition of that —
    // `signal::level_is_trustworthy`, which is the one with the tests. This
    // comment used to claim the definition was here, while an identical copy sat
    // in `signal.rs` being tested and never called: two spellings of one rule,
    // and only the untested one shipping.
    let trustworthy = ok && error.is_none() && crate::signal::level_is_trustworthy(asked, landed);
    emit(TunerEvent::Level(LevelReading { level, asked, landed, trustworthy, error }));
}

pub fn ingest_panel_key(code: i32, action: String) {
    emit(TunerEvent::PanelKey { code, key: PanelKey::from_code(code), action });
}

/// Say what is tuned now, to a driver who is looking at another app.
///
/// A SHIM SO THE CALLER NEEDS NO `cfg`. `app.rs` decides WHEN to announce — that
/// is a rule about foreground and about the dial changing, and it is the same
/// rule on every target, so it is tested on the host like everything else. Only
/// the posting is platform work, and off Android there is nothing to post to.
#[cfg(target_os = "android")]
pub fn announce_station(title: &str, text: &str, logo: &str) -> String {
    alert::post(title, text, logo)
}

/// The host has no notification shade. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn announce_station(_title: &str, _text: &str, _logo: &str) -> String {
    "no notification shade in this build".into()
}

/// What could keep this app alive through a sleep. See `probe`.
#[cfg(target_os = "android")]
pub fn keep_alive_report() -> Vec<String> {
    probe::report()
}

/// The host has no vendor power manager to ask about. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn keep_alive_report() -> Vec<String> {
    Vec::new()
}

/// Where the stock radio app could be intercepted without root. See `stock`.
#[cfg(target_os = "android")]
pub fn stock_radio_report() -> Vec<String> {
    stock::report()
}

/// The host has no stock radio app to displace. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn stock_radio_report() -> Vec<String> {
    Vec::new()
}

/// Take the station pop-up down; the driver is back on the face.
#[cfg(target_os = "android")]
pub fn clear_station_announcement() {
    alert::clear();
}

/// The host has no notification shade. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn clear_station_announcement() {}

/// Is the activity RESUMED? THE WHOLE OF [`is_foreground`], and [`FOCUSED`]
/// records the day it was briefly half.
///
/// TRUE UNTIL TOLD OTHERWISE, which is the safe default rather than an
/// assumption. The app is launched into the foreground and `Resume` arrives
/// after the first frames rather than before them, so a `false` start would mean
/// a notification for the station the driver is watching being tuned. Every host
/// build — probes, shots, tests — installs no listener at all and stays true,
/// which is what keeps the pop-up out of them without a `cfg`.
static RESUMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Does the activity's window have INPUT FOCUS? AN OBSERVATION, GATING NOTHING.
///
/// `MainEvent::GainedFocus` and `MainEvent::LostFocus` are SEPARATE events from
/// `Resume` and `Pause` in `android-activity`
/// (android-activity-0.6.1/src/lib.rs:499,503 against :520,531), which is why it
/// is worth recording separately: the drive log shows which edges this unit
/// actually raises, and that is a fact nothing else here can supply.
///
/// IT WAS BRIEFLY HALF OF [`is_foreground`], AND THAT WAS WRONG. The claim was
/// that this is why the station pop-up never fired, on the grounds that the unit
/// resizes apps into vertical thirds and that Android 9's MULTI-RESUME leaves a
/// visible-but-unfocused activity RESUMED with no `Pause`. THE PREMISE WAS
/// INVENTED. The owner does not use that OS and does not use windowing at all —
/// Carnyx runs full screen and the driver switches wholly to another full-screen
/// app, which pauses it. The DUDU OS references in this tree are the DESIGN
/// HANDOFF'S TARGET SURFACES and a tuner-source option, not a description of the
/// unit, and reading them as one is a mistake made three times now.
///
/// AND GATING ON IT COSTS SOMETHING REAL, which is why it was not left in "for
/// breadth". Pulling down the notification shade, raising the volume panel or
/// any system dialog takes focus WITHOUT a pause. Gated, each of those would
/// announce a station over a face the driver is looking at — and worse, would
/// write `was_foreground = false` for [`wake`], so an ignition-off in that
/// window would tell the receiver not to bring the face back.
///
/// TRUE UNTIL TOLD OTHERWISE, for the same reason as [`RESUMED`].
static FOCUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The activity was resumed or paused. See [`RESUMED`].
///
/// These also write the answer OUT OF THE PROCESS, to the shared preferences the
/// wake receiver reads — see [`wake`]. Folded in here rather than added beside
/// the call sites in `lib.rs`: the value is the same value, and a second call
/// site is a place for the two to drift apart on some future lifecycle edge. The
/// atomics are for this run; the file is for the next one, after the unit has
/// slept and killed the process in between.
pub fn set_resumed(on: bool) {
    RESUMED.store(on, std::sync::atomic::Ordering::Relaxed);
    record_foreground(is_foreground());
}

/// The activity's window gained or lost input focus. See [`FOCUSED`].
///
/// STORES AND NOTHING ELSE — no `record_foreground`, deliberately. Focus is not
/// part of the answer the wake receiver is asked, and a shade pull is not the
/// driver leaving.
pub fn set_focused(on: bool) {
    FOCUSED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Does the window have input focus right now? See [`FOCUSED`] for why this is
/// worth reading and why the pop-up gate does not read it.
pub fn is_focused() -> bool {
    FOCUSED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set both at once. FOR TESTS, which have no lifecycle to drive.
pub fn set_foreground(on: bool) {
    RESUMED.store(on, std::sync::atomic::Ordering::Relaxed);
    FOCUSED.store(on, std::sync::atomic::Ordering::Relaxed);
    record_foreground(on);
}

#[cfg(target_os = "android")]
fn record_foreground(on: bool) {
    wake::set_foreground(on);
}

/// The host has no receiver to leave a note for. See the Android arm.
#[cfg(not(target_os = "android"))]
fn record_foreground(_on: bool) {}

/// The clock's two facts. See [`service::clock_now`].
#[cfg(target_os = "android")]
pub fn clock_now() -> Option<(u32, u32, bool)> {
    service::clock_now()
}

/// The host has no platform clock to ask, and drawing one it invented would put
/// a time in every screenshot. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn clock_now() -> Option<(u32, u32, bool)> {
    None
}

/// The device's ISO country, for §4.9's units. See [`service::country_code`].
#[cfg(target_os = "android")]
pub fn country_code() -> String {
    service::country_code()
}

/// The host has no locale worth asking about — every shot must render the same
/// distances wherever it is taken. Empty becomes `Units::Metric`.
#[cfg(not(target_os = "android"))]
pub fn country_code() -> String {
    String::new()
}

/// What the wake receiver did on the way up, and forget it. See [`wake`].
#[cfg(target_os = "android")]
pub fn take_wake_note() -> String {
    wake::take_last_wake()
}

/// The host is never woken by an ignition. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn take_wake_note() -> String {
    String::new()
}

/// What the last sleep managed, and forget it. See [`wake::take_last_sleep`].
#[cfg(target_os = "android")]
pub fn take_sleep_note() -> String {
    wake::take_last_sleep()
}

/// The host never sleeps an ignition. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn take_sleep_note() -> String {
    String::new()
}

/// Whether the face is the thing the driver is looking at.
///
/// THE RESUMED HALF ALONE, and [`FOCUSED`] records why it is not both. The unit
/// runs one app full screen at a time: switching away pauses Carnyx, so `Pause`
/// is a complete signal here, while focus goes away for shades and dialogs the
/// driver raised ON TOP of a face they are still looking at.
pub fn is_foreground() -> bool {
    RESUMED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn ingest_sleep(action: String, release: String) {
    emit(TunerEvent::Sleep { action, release });
}

/// One navigation update from OsmAnd. Called on a BINDER THREAD.
///
/// `jint` is widened here rather than in the seam so the event type stays free
/// of jni types and the host tests can build one.
///
/// `jboolean` IS `bool` IN THIS CRATE'S jni, not the `u8` the C header has and
/// not what older versions of this crate had — `jni-0.22.4` writes
/// `let mut is_copy: jboolean = true;` in its own source. So there is no `!= 0`
/// here; there was, and `tools/check-jni.sh` rejected it.
#[cfg(target_os = "android")]
pub fn ingest_nav(distance_to: jni::sys::jint, turn_type: jni::sys::jint, left_side: jni::sys::jboolean) {
    emit(TunerEvent::Nav { distance_to, turn_type, left_side });
}

/// The same, from a host test. See the Android arm.
#[cfg(not(target_os = "android"))]
pub fn ingest_nav(distance_to: i32, turn_type: i32, left_side: bool) {
    emit(TunerEvent::Nav { distance_to, turn_type, left_side });
}

/// One voice-router announcement from OsmAnd. Called on a BINDER THREAD.
pub fn ingest_nav_voice(cmds: Vec<String>, played: Vec<String>) {
    emit(TunerEvent::NavVoice { cmds, played });
}

/// The poll's answer. Called on `CarnyxNav`'s own poll thread.
///
/// `NavRoute` IS `crate::nav::Route`, re-exported so the seam does not name a
/// module above it — the same shape `TunerSnapshot` takes.
pub use crate::nav::Route as NavRoute;

pub fn ingest_nav_info(route: NavRoute) {
    emit(TunerEvent::NavInfo(route));
}

pub fn ingest_illumination(action: String, extras: String, ui_mode: String) {
    emit(TunerEvent::Illumination { action, extras, ui_mode });
}

// ── The trait at the seam ────────────────────────────────────────────────────

/// What the tuner state looks like right now, read synchronously.
///
/// The push callbacks do not always reach a passive client on this firmware, but
/// these getters do return live values, which is why the face polls at all.
#[derive(Debug, Clone, PartialEq)]
pub struct TunerSnapshot {
    pub raw: i32,
    pub mhz: Option<f32>,
    pub band: i8,
    pub ps: String,
    pub rt: String,
    pub pty: i32,
    /// The vendor's `isStreroOn()`, reported ONLY so a diagnostic can show it.
    /// It is stuck true on this firmware. The stereo pill must follow
    /// [`TunerEvent::Stereo`].
    pub stereo_getter_stuck: bool,
    /// The MCU's own audio source: `Some(4)` is FM, `None` means unreadable.
    /// This, and not Android audio focus, is whether FM is actually playing on a
    /// head unit — focus can go dark after a permanent loss and never come back,
    /// while this register self-heals.
    pub mcu_source: Option<i32>,
}

impl TunerSnapshot {
    /// Is FM what the speakers are actually carrying?
    pub fn fm_is_playing(&self) -> bool {
        self.mcu_source == Some(4)
    }
}

/// The head unit's tuner, or something standing in for it.
///
/// Commands go through here; results come back as [`TunerEvent`]s on the sink,
/// because almost nothing this hardware does is synchronous.
pub trait Tuner: Send + Sync {
    /// TEST HOOK, defaulted to nothing. `FakeTuner` overrides it to stop
    /// echoing a tune back synchronously, which is how the device behaves and
    /// what any test about the commit-to-confirmation window needs. A real
    /// tuner has no such switch and needs none.
    fn set_echo_for_test(&self, _on: bool) {}

    /// TEST HOOK, defaulted to nothing. Where `write_log` puts the file when
    /// there is no Downloads folder to put it in. A real tuner ignores it: on the
    /// device the destination is not ours to choose, which is the point of using
    /// Downloads at all.
    fn set_log_dir_for_test(&self, _dir: std::path::PathBuf) {}

    /// TEST HOOK. The VENDOR SERVICE moving the front end, as distinct from
    /// merely reporting that it did.
    ///
    /// The difference is the whole content of the wrong-landing bug.
    /// `ingest_frequency` — which is what every probe here used — pushes a
    /// report and leaves the simulated hardware exactly where this app put it,
    /// so the radio can never end up anywhere the app did not ask for and the
    /// race cannot be reproduced. Keys 62/63 make the vendor's RadioService
    /// walk its OWN preset bank, and that walk COMMANDS the tuner. Whichever
    /// command lands last is where the driver ends up.
    fn vendor_tune_for_test(&self, _mhz: f32) {}

    /// TEST HOOK. The IGNITION going off, delivered the way the device delivers
    /// it — through a receiver that has to have been registered first.
    ///
    /// Defaulted to nothing, and `FakeTuner` answers it only once
    /// `start_sleep_watch` has run. Calling `ingest_sleep` instead would push the
    /// event straight past the registration and prove nothing about whether
    /// anything is listening, which is the entire content of the bug this hook
    /// exists to pin.
    fn push_sleep_for_test(&self, _action: &str) {}

    /// TEST HOOK. Where the front end actually is, which on the device only the
    /// hardware knows. `None` from a real tuner: this exists so a test can
    /// assert about the RADIO rather than about the label, and those are the
    /// two different things `State::hold` deliberately lets diverge.
    fn tuned_mhz_for_test(&self) -> Option<f32> {
        None
    }

    /// Does this unit have the vendor radio service at all?
    fn is_available(&self) -> bool;

    /// Ask for the bind. The OUTCOME arrives as `Connected` or `ConnectFailed`,
    /// never as a return value — `bindService` succeeding only means the request
    /// was accepted.
    fn connect(&self) -> Result<(), TunerError>;

    fn disconnect(&self);

    /// Tune, in MHz. Fails with [`TunerError::NotCalibrated`] until the unit has
    /// told us what scale it uses.
    fn tune(&self, mhz: f32) -> Result<(), TunerError>;

    /// Hardware seek to the next receivable station.
    fn seek(&self, up: bool);

    fn snapshot(&self) -> Option<TunerSnapshot>;

    fn set_audio_enabled(&self, on: bool);

    /// Mirror the driver's "Release FM on sleep" switch to wherever the sleep
    /// receiver can read it.
    ///
    /// THE RECEIVER CANNOT REACH `Settings`. It runs on a binder thread and the
    /// switch lives behind a `RefCell` on the UI thread, so the value has to be
    /// pushed rather than pulled — the same shape the foreground flag takes for
    /// the wake receiver. Defaulted to nothing: a fake has no receiver to tell.
    fn set_release_on_sleep(&self, _on: bool) {}

    fn set_rds_enabled(&self, on: bool);

    fn send_panel_key(&self, code: i32);

    /// Start the periodic level read. Every tick COMMANDS the tuner, so the
    /// implementation floors the interval and skips ticks where FM does not own
    /// the speakers.
    fn start_level_watch(&self, interval_ms: i64);

    fn stop_level_watch(&self);

    /// One reading now — call after a retune so the meter does not sit on the
    /// previous station's level.
    fn read_level_now(&self);

    fn band_plan(&self) -> Vec<BandPoint>;

    /// Start listening for the head unit's illumination (headlights) broadcast.
    ///
    /// Deliberately separate from `connect`, and callable without it: day/night
    /// belongs to the VEHICLE, not to whichever tuner is selected. CarFM had
    /// this inside connect alone, so a session that never bound the built-in
    /// tuner never registered the receiver and stayed light all night. Idempotent.
    fn start_illumination_watch(&self);

    /// Start listening for the head unit going to sleep, so the FM source can be
    /// handed back before this process stops running.
    ///
    /// SEPARATE FROM `connect` FOR THE REASON DIRECTLY ABOVE, and it was not.
    /// `NwdBridge.startSleepWatch` was called from inside `connect()`, after
    /// `bindService` returned true — so a unit whose vendor service refused the
    /// bind registered no receiver, and the `sleep:` line that is the ONLY
    /// evidence of which broadcast fires never appeared on the session most worth
    /// reading. That is the illumination bug again, one line away from the note
    /// describing it. The ignition going off belongs to the vehicle, not to
    /// whichever tuner is selected.
    ///
    /// Defaulted to nothing rather than required: four example probes implement
    /// this trait and none of them has an ignition. Idempotent.
    ///
    /// RETURNS A LINE FOR THE DIAGNOSTICS LOG, empty for "nothing to say". It
    /// returned nothing at all, and the Java said what it had done to logcat —
    /// which on a unit with no adb reaches nobody, so "the receiver never
    /// registered" and "the broadcast never arrived" were indistinguishable from
    /// the driver's seat and need different fixes.
    fn start_sleep_watch(&self) -> String {
        String::new()
    }

    /// Write the diagnostics log to the head unit's public Downloads folder and
    /// return the human-readable path.
    ///
    /// NOT A TUNER CONCERN, and it is here anyway for the reason
    /// `start_illumination_watch` above is: this trait is the app's only seam
    /// onto the Java bridge, and the bridge is where the Context lives. CarFM
    /// kept its `writeLog` in `VibeStreamModule` rather than the radio module,
    /// which is the tidier split — worth one more trait member here, not worth a
    /// second JNI class and a second dex load.
    ///
    /// CALLABLE WITHOUT A CONNECTED RADIO. The seam only needs `android::init`
    /// to have run, which is what makes the log exportable on a unit where the
    /// vendor service never bound — the session most worth reading.
    ///
    /// The default is the honest answer for a build with no Java behind it.
    fn write_log(&self, _text: &str) -> Result<String, TunerError> {
        Err(TunerError::Unavailable("no Android bridge in this build".into()))
    }
}

// ── The fake ─────────────────────────────────────────────────────────────────

/// A tuner with no head unit behind it.
///
/// This exists so the rest of Carnyx can be exercised in a container: it drives
/// the same [`ingest_connected`] / [`ingest_frequency`] path a real device
/// drives, so anything downstream sees events that are indistinguishable from
/// the article. It is deterministic — no threads, no timers — because a fake
/// that needed waiting on would be a second thing to debug.
///
/// The defaults describe the unit Carnyx targets: raw frequencies in units of
/// 10 kHz, the FM band plan the device reported on 2026-07-31, and a handful of
/// stations to seek between.
pub struct FakeTuner {
    inner: Mutex<FakeState>,
    /// What `is_available` answers, and it is not decoration.
    ///
    /// The settings panel derives its whole status line from ONE predicate —
    /// `Tuner::is_available` — so a fake that always says yes makes the panel
    /// claim "Connected · Built-in hardware · NWD/NOWADA FM tuner" over a
    /// simulation. That is a positive lie on a driver's dashboard, and it is
    /// exactly what happened when `android::init` failed and the Android path
    /// substituted a default `FakeTuner`.
    ///
    /// `new` stays available because the host and the screenshots are a
    /// deliberate simulation of a WORKING unit; `unavailable` is for the
    /// fallback, where the honest answer is no.
    available: bool,
}

struct FakeState {
    multiplier: i32,
    band: i8,
    raw: i32,
    ps: String,
    connected: bool,
    /// Raw frequencies a seek will stop on, ascending.
    stations: Vec<i32>,
    level: i32,
    audio: bool,
    ill_watching: bool,
    /// Has the sleep watch been armed? Same rule as `ill_watching` and for the
    /// same reason: an event the app never asked to hear must not arrive, or the
    /// "registered inside connect" bug is unreproducible here.
    sleep_watching: bool,
    /// Does `tune` report the new frequency back straight away?
    ///
    /// TRUE BY DEFAULT AND FALSE ON THE DEVICE. `NwdBridge.tune` calls
    /// `setCurrentFrequency` and returns; the frequency comes back later as
    /// `notifyCurrentFrequency`. The fake collapsing that into one synchronous
    /// call is convenient for most tests and WRONG for any test about what
    /// happens between commanding a tune and hearing about it — which is exactly
    /// the window `State::hold` exists for.
    echo: bool,
    /// Where `write_log` writes when there is no Downloads folder to write to.
    /// `None` means a directory under the system temp dir.
    log_dir: Option<std::path::PathBuf>,
    /// How many logs this fake has written, which is what names the file. See
    /// `FakeTuner::write_log` for why it counts rather than stamps.
    logs_written: u32,
}

/// The last value the app pushed through [`Tuner::set_release_on_sleep`].
///
/// RECORDED BECAUSE THE PUSH IS THE FEATURE, AND THE TRAIT DEFAULT HIDES IT. On
/// the unit that value is mirrored into shared preferences for `SleepReceiver`,
/// which runs in a COLD PROCESS with no settings file read and no Rust — a
/// switch the app never pushed is a switch that receiver cannot honour. The
/// trait's default implementation is an empty body, so a missing push looks
/// exactly like a working one from every test that does not check.
///
/// A STATIC RATHER THAN A FIELD ON THE FAKE, because the app owns its tuner as a
/// `Box<dyn Tuner>` and no test can reach back through it. The tests that read
/// this hold `harness::ui_lock`, which serialises them, the same way [`RESUMED`]
/// and [`FOCUSED`] are read.
static RELEASE_MIRROR: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

/// Forget what was pushed, so one test's push is not another's evidence.
pub fn clear_release_mirror() {
    *RELEASE_MIRROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// What the app last pushed. See [`RELEASE_MIRROR`].
pub fn last_release_mirror() -> Option<bool> {
    *RELEASE_MIRROR.lock().unwrap_or_else(|e| e.into_inner())
}

impl FakeTuner {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(FakeState {
                multiplier: 100,
                band: 0,
                raw: 8870,
                ps: "WERN".into(),
                connected: false,
                stations: vec![8870, 9110, 10150, 10250, 10590],
                level: 62,
                audio: false,
                ill_watching: false,
                sleep_watching: false,
                echo: true,
                log_dir: None,
                logs_written: 0,
            }),
            available: true,
        }
    }

    /// The same simulation, reporting itself ABSENT.
    ///
    /// For the one caller that needs it: `android_main`, when `android::init`
    /// could not give it a real tuner. The face still has something to draw
    /// against, and the settings panel tells the truth about the radio.
    pub fn unavailable() -> Self {
        Self { available: false, ..Self::new() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Replace the stations a seek walks. Ascending raw frequencies.
    pub fn set_stations(&self, raw: Vec<i32>) {
        self.lock().stations = raw;
    }

    /// Set the level the next reading reports.
    pub fn set_level(&self, level: i32) {
        self.lock().level = level;
    }

    /// Pretend the MCU pushed a group. The string is the vendor's 16 hex chars.
    pub fn push_rds_hex(&self, hex: &str) {
        ingest_rds_group(hex);
    }

    /// Model the device's ASYNCHRONOUS frequency reporting: `tune` commands and
    /// says nothing, and the caller injects the report when it chooses. See
    /// `FakeState::echo`.
    pub fn set_echo(&self, on: bool) {
        self.lock().echo = on;
    }

    /// The vendor service walking its OWN hardware preset bank: it moves the
    /// front end and then says so.
    ///
    /// NOT `ingest_frequency`, and that distinction is the point. A bare report
    /// leaves `raw` where this app last put it, so the simulated radio is always
    /// obediently on the app's station no matter what the report claims — which
    /// is why six wheel-probe cases full of vendor traffic never once caught a
    /// wrong LANDING. This is the vendor issuing a command, so the last command
    /// wins here exactly as it does on the unit.
    ///
    /// Reports unconditionally, whatever `echo` says: `echo` models whether OUR
    /// tune answers synchronously, and the vendor's notification is not ours.
    pub fn vendor_tunes(&self, mhz: f32) {
        let (band, raw) = {
            let mut s = self.lock();
            let raw = (f64::from(mhz) * f64::from(s.multiplier)).round() as i32;
            s.raw = raw;
            s.ps = String::new();
            (s.band, raw)
        };
        ingest_frequency(band as i32, raw, String::new(), -1);
    }

    /// Pretend the wheel was pressed.
    pub fn push_panel_key(&self, code: i32) {
        ingest_panel_key(code, "com.nwd.action.ACTION_KEY_VALUE".into());
    }

    /// Is FM claimed? Only the fake exposes this; on the device the MCU decides.
    pub fn audio_enabled(&self) -> bool {
        self.lock().audio
    }

    /// Pretend the ignition went off. A no-op unless the watch was started, so
    /// the "registered inside connect, so a refused bind hears nothing" bug is
    /// reproducible here rather than only on a dashboard.
    pub fn push_sleep(&self, action: &str) {
        if self.lock().sleep_watching {
            // The fake has no Java receiver, so nothing was released before the
            // hop and the outcome is empty. The queued release in
            // `drain_events` is what the host tests exercise.
            ingest_sleep(action.into(), String::new());
        }
    }

    /// Pretend the headlights changed. A no-op unless the watch was started, so
    /// the "an RTL-SDR session stayed light all night" bug is reproducible.
    pub fn push_illumination(&self, extras: &str, ui_mode: &str) {
        if self.lock().ill_watching {
            ingest_illumination(
                "com.nwd.ACTION_ILL_STATE_CHANGE".into(),
                extras.into(),
                ui_mode.into(),
            );
        }
    }
}

impl Default for FakeTuner {
    fn default() -> Self {
        Self::new()
    }
}

impl Tuner for FakeTuner {
    fn is_available(&self) -> bool {
        self.available
    }

    /// Record it rather than ignore it. See [`RELEASE_MIRROR`].
    fn set_release_on_sleep(&self, on: bool) {
        *RELEASE_MIRROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(on);
    }

    fn connect(&self) -> Result<(), TunerError> {
        let (band, raw, ps) = {
            let mut s = self.lock();
            s.connected = true;
            (s.band, s.raw, s.ps.clone())
        };
        ingest_connected(band as i32, raw, ps, String::new(), -1, true);
        Ok(())
    }

    fn disconnect(&self) {
        self.lock().connected = false;
        ingest_disconnected();
    }

    fn tune(&self, mhz: f32) -> Result<(), TunerError> {
        let raw = {
            let s = self.lock();
            if !s.connected {
                return Err(TunerError::NotConnected);
            }
            (mhz as f64 * s.multiplier as f64).round() as i32
        };
        let band = {
            let mut s = self.lock();
            s.raw = raw;
            s.ps = String::new();
            s.band
        };
        if self.lock().echo {
            ingest_frequency(band as i32, raw, String::new(), -1);
        }
        Ok(())
    }

    fn set_echo_for_test(&self, on: bool) {
        self.set_echo(on);
    }

    fn vendor_tune_for_test(&self, mhz: f32) {
        self.vendor_tunes(mhz);
    }

    fn push_sleep_for_test(&self, action: &str) {
        self.push_sleep(action);
    }

    fn tuned_mhz_for_test(&self) -> Option<f32> {
        let s = self.lock();
        Some(s.raw as f32 / s.multiplier as f32)
    }

    fn seek(&self, up: bool) {
        let (band, raw) = {
            let mut s = self.lock();
            if !s.connected || s.stations.is_empty() {
                return;
            }
            let here = s.raw;
            let next = if up {
                s.stations.iter().copied().find(|&f| f > here).or_else(|| s.stations.first().copied())
            } else {
                s.stations.iter().copied().rev().find(|&f| f < here).or_else(|| s.stations.last().copied())
            };
            s.raw = next.unwrap_or(here);
            (s.band, s.raw)
        };
        ingest_frequency(band as i32, raw, String::new(), -1);
    }

    fn snapshot(&self) -> Option<TunerSnapshot> {
        let s = self.lock();
        if !s.connected {
            return None;
        }
        Some(TunerSnapshot {
            raw: s.raw,
            mhz: Some(s.raw as f32 / s.multiplier as f32),
            band: s.band,
            ps: s.ps.clone(),
            rt: String::new(),
            pty: -1,
            stereo_getter_stuck: true,
            mcu_source: Some(if s.audio { 4 } else { 0 }),
        })
    }

    fn set_audio_enabled(&self, on: bool) {
        self.lock().audio = on;
    }

    fn set_rds_enabled(&self, _on: bool) {}

    fn send_panel_key(&self, code: i32) {
        self.push_panel_key(code);
    }

    fn start_level_watch(&self, _interval_ms: i64) {
        self.read_level_now();
    }

    fn stop_level_watch(&self) {}

    fn read_level_now(&self) {
        let (level, here) = {
            let s = self.lock();
            (s.level, s.raw)
        };
        ingest_level(level, here, here, true, None);
    }

    fn band_plan(&self) -> Vec<BandPoint> {
        // The plan the device actually reported, including the unused third
        // entry, so the filter in `band_plan_from_raw` is exercised.
        band_plan_from_raw(&[8750, 10790, 20, 530, 1710, 10, 0, 0, 0])
    }

    fn start_illumination_watch(&self) {
        // There are no headlights in a container. `push_illumination` is how a
        // test or a mock-up drives the day/night path.
        self.lock().ill_watching = true;
    }

    fn start_sleep_watch(&self) -> String {
        // And no ignition. `push_sleep` is how a test drives that path.
        self.lock().sleep_watching = true;
        // Deliberately silent: a line here would appear in every screenshot of
        // the settings panel, and there is nothing on a host worth saying.
        String::new()
    }

    /// The same file, somewhere a host can reach.
    ///
    /// A REAL WRITE, not a stub that returns a plausible path. There is no
    /// Downloads folder off the device, but there is a filesystem, and the whole
    /// value of this seam is that "Save to file" can be exercised — the version
    /// this replaces was a menu row that wrote "not available without the head
    /// unit" into the very log it was asked to export.
    fn write_log(&self, text: &str) -> Result<String, TunerError> {
        let dir = self
            .lock()
            .log_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("carnyx-logs"));
        std::fs::create_dir_all(&dir).map_err(|e| TunerError::Java(e.to_string()))?;
        // COUNTED, NOT CLOCK-STAMPED, and that is the difference from the device
        // path. `NwdBridge.writeLog` names the file for the second it was written
        // because a driver reads them in order; a test that did the same could
        // collide with itself twice in one second and could not name the file it
        // expects. The device's own naming is not what is under test here.
        let n = {
            let mut s = self.lock();
            s.logs_written += 1;
            s.logs_written
        };
        let path = dir.join(format!("carnyx-tuner-log-{n}.txt"));
        std::fs::write(&path, text).map_err(|e| TunerError::Java(e.to_string()))?;
        Ok(path.display().to_string())
    }

    fn set_log_dir_for_test(&self, dir: std::path::PathBuf) {
        self.lock().log_dir = Some(dir);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The settings panel derives its entire status line from this one
    /// predicate, so a fake that always answered yes made the panel print
    /// "Connected · Built-in hardware · NWD/NOWADA FM tuner" over a simulation
    /// whenever `android::init` failed on a real head unit. The comment in
    /// `android_main` asserted the opposite was happening, which is precisely
    /// why a comment is not a test.
    #[test]
    fn the_fallback_fake_does_not_claim_a_tuner() {
        assert!(
            !FakeTuner::unavailable().is_available(),
            "the fallback fake must report ABSENT — the status line believes it"
        );
        // The host and the 58 screenshots deliberately simulate a working unit.
        assert!(FakeTuner::new().is_available());
        // Everything else about the two is identical; only the answer changes.
        assert_eq!(
            FakeTuner::unavailable().snapshot(),
            FakeTuner::new().snapshot()
        );
    }

    /// Android hands out a (0, 0) fix from a provider that has nothing, and
    /// Null Island is 700 km off the Gulf of Guinea. Trusting it would put the
    /// picker's "best signal first" list in the Atlantic and re-resolve every
    /// preset's call sign against stations that do not exist — silently, because
    /// a fix is a fix as far as the face is concerned.
    #[test]
    fn an_impossible_position_is_reported_as_no_fix() {
        let h = Harness::new();
        let bad = [
            (0.0, 0.0),
            (91.0, 10.0),
            (-91.0, 10.0),
            (10.0, 181.0),
            (10.0, -181.0),
            (f64::NAN, 10.0),
            (10.0, f64::INFINITY),
        ];
        for (lat, lon) in bad {
            ingest_position(lat, lon, true, 0.0, false);
            match h.drain().as_slice() {
                [TunerEvent::Position { fix, .. }] => {
                    assert!(!fix, "({lat}, {lon}) must not be reported as a fix")
                }
                other => panic!("expected one Position, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_real_position_survives_the_guard() {
        let h = Harness::new();
        // Madison, which is where the shipped fake sits.
        ingest_position(43.07, -89.40, true, 20.0, true);
        match h.drain().as_slice() {
            [TunerEvent::Position { lat, lon, fix, in_motion }] => {
                assert!(*fix);
                assert!(*in_motion);
                assert_eq!((*lat, *lon), (43.07, -89.40));
            }
            other => panic!("expected one Position, got {other:?}"),
        }
    }

    /// A PARKED CAR MUST STOP BEING A MOVING ONE, and getting there needs two
    /// thresholds rather than one.
    ///
    /// The device fault this pins is upstream of the maths: the location
    /// registration carried a 50-metre distance filter, so a car that stopped
    /// earned no further callback at all and the last fix — taken while still
    /// rolling — stood as the final word. The glyph stayed lit and reorder mode
    /// stayed refused until the car moved another fifty metres. The filter is
    /// zero now; this is the half that can be tested.
    #[test]
    fn the_motion_verdict_has_hysteresis_and_clears_when_the_car_stops() {
        // Rising: 3 mph is not yet moving, 5 mph is.
        assert!(!settle_motion(false, 1.5, true));
        assert!(settle_motion(false, MOVING_ON_MPS, true));
        // Falling: still moving at 3 mph, stopped below it. A single threshold
        // would flap here, and every flap blinks the driving glyph.
        assert!(settle_motion(true, 1.5, true));
        assert!(!settle_motion(true, 1.0, true));
        // No speed reported is STATIONARY, not "carry on as before" — otherwise
        // a provider that never reports speed latches whatever it started with.
        assert!(!settle_motion(true, 0.0, false));
        assert!(!settle_motion(true, f32::NAN, true));
        assert!(!settle_motion(true, -1.0, true));
    }

    /// The verdict is STATE, so a stop has to survive the round trip through
    /// `ingest_position` and not just through the pure helper.
    #[test]
    fn a_stop_reaches_the_face_through_the_shipping_path() {
        let h = Harness::new();
        ingest_position(43.07, -89.40, true, 20.0, true);
        match h.drain().as_slice() {
            [TunerEvent::Position { in_motion, .. }] => assert!(*in_motion),
            other => panic!("expected one Position, got {other:?}"),
        }
        // Parked, and the provider still reporting because the distance filter
        // is gone.
        ingest_position(43.07, -89.40, true, 0.0, true);
        match h.drain().as_slice() {
            [TunerEvent::Position { in_motion, fix, .. }] => {
                assert!(!*in_motion, "a stopped car must stop reading as moving");
                assert!(*fix);
            }
            other => panic!("expected one Position, got {other:?}"),
        }
    }

    /// Losing the antenna is not a moving car. Without the `fix &&` guard the
    /// verdict latches at whatever it was when the sky went dark.
    #[test]
    fn a_lost_fix_clears_the_motion_verdict() {
        let h = Harness::new();
        ingest_position(43.07, -89.40, true, 20.0, true);
        let _ = h.drain();
        ingest_position(0.0, 0.0, false, 20.0, true);
        match h.drain().as_slice() {
            [TunerEvent::Position { in_motion, fix, .. }] => {
                assert!(!*in_motion);
                assert!(!*fix);
            }
            other => panic!("expected one Position, got {other:?}"),
        }
    }

    /// A genuine loss of fix must stay a loss, not be rescued by sane numbers.
    #[test]
    fn losing_the_fix_is_reported_even_with_valid_coordinates() {
        let h = Harness::new();
        ingest_position(43.07, -89.40, false, 0.0, false);
        match h.drain().as_slice() {
            [TunerEvent::Position { fix, .. }] => assert!(!fix),
            other => panic!("expected one Position, got {other:?}"),
        }
    }

    /// A connect must CLAIM the audio source, not assume it.
    ///
    /// The face opens with the power button lit, so until the claim was sent the
    /// UI and the MCU disagreed: the radio tuned and RDS arrived, and there was
    /// silence until the driver toggled power off and on, because the ON half is
    /// the only thing that had ever broadcast app-IN. This pins that a Connected
    /// event reaches `set_audio_enabled`.
    #[test]
    fn connecting_emits_the_event_the_audio_claim_hangs_off() {
        let h = Harness::new();
        let t = FakeTuner::new();
        assert!(!t.audio_enabled(), "audio starts unclaimed");
        t.connect().unwrap();
        assert!(
            h.drain().iter().any(|e| matches!(e, TunerEvent::Connected(_))),
            "connect must emit Connected — App::apply_event claims the audio \
             source there, and without the event there is no claim and no sound"
        );
        t.set_audio_enabled(true);
        assert!(t.audio_enabled(), "the claim must reach the tuner");
    }

    /// Collect events for the duration of a test.
    ///
    /// The sink and the calibration are process-global — there is one tuner in
    /// one process — so the tests take a lock rather than run in parallel
    /// against each other's state.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    struct Harness {
        events: Arc<Mutex<Vec<TunerEvent>>>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Harness {
        fn new() -> Self {
            let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            reset_calibration();
            let events: Arc<Mutex<Vec<TunerEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = events.clone();
            set_event_sink(move |e| sink.lock().unwrap().push(e));
            Self { events, _guard: guard }
        }

        fn drain(&self) -> Vec<TunerEvent> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            clear_event_sink();
            reset_calibration();
        }
    }

    #[test]
    fn scale_ladder_matches_the_values_the_device_produces() {
        // FM as this unit reports it, x100.
        assert_eq!(Calibration::scale_for(8750), 100);
        assert_eq!(Calibration::scale_for(10790), 100);
        // FM on a unit that reports kHz.
        assert_eq!(Calibration::scale_for(87500), 1000);
        assert_eq!(Calibration::scale_for(107900), 1000);
        // AM, 530..1710 kHz.
        assert_eq!(Calibration::scale_for(530), 10);
        assert_eq!(Calibration::scale_for(1710), 10);
        // The boundaries themselves belong to the arm BELOW, because the ladder
        // tests with `>` and not `>=`.
        assert_eq!(Calibration::scale_for(500), 1);
        assert_eq!(Calibration::scale_for(501), 10);
        assert_eq!(Calibration::scale_for(5_000), 10);
        assert_eq!(Calibration::scale_for(5_001), 100);
        assert_eq!(Calibration::scale_for(50_000), 100);
        assert_eq!(Calibration::scale_for(50_001), 1000);
    }

    #[test]
    fn an_am_reading_cannot_undo_an_fm_one() {
        // The exact failure from CarFM: connect while the head unit is on AM,
        // raw 1000, which reads as a x10 scale. FM then displayed "1059.0 MHz"
        // and a tune to 102.5 sent 1025 — outside the 8750..10790 plan, so the
        // dial spun and the radio did not move for the rest of the session.
        let mut cal = Calibration::new();
        cal.observe(1, 1000);
        assert_eq!(cal.multiplier(), Some(10));
        cal.observe(0, 10250);
        assert_eq!(cal.multiplier(), Some(100), "an FM reading must widen the scale");
        cal.observe(1, 1000);
        assert_eq!(cal.multiplier(), Some(100), "and an AM reading must never narrow it back");
        assert_eq!(cal.raw_for(102.5), Some(10250));
        assert_eq!(cal.mhz_for(10250), Some(102.5));
    }

    #[test]
    fn a_cold_front_end_reading_zero_teaches_nothing() {
        // CarFM assigned from this reading and landed on a x1 scale. Here it is
        // no information, and a tune is refused with a reason instead.
        let mut cal = Calibration::new();
        cal.observe(0, 0);
        assert_eq!(cal.multiplier(), None);
        assert_eq!(cal.raw_for(102.5), None);
        cal.observe(0, 8870);
        assert_eq!(cal.raw_for(102.5), Some(10250));
    }

    #[test]
    fn the_band_is_tracked_on_every_reading() {
        // FM1 -> FM2 mid-session, the 31 July log. The band must follow, because
        // a tune is sent against it and each band has its own preset bank.
        let mut cal = Calibration::new();
        cal.observe(0, 8870);
        assert_eq!(cal.band(), 0);
        cal.observe(1, 10250);
        assert_eq!(cal.band(), 1);
        // Even a reading that teaches nothing about the scale still moves the band.
        cal.observe(0, 0);
        assert_eq!(cal.band(), 0);
    }

    #[test]
    fn rounding_matches_the_javascript_it_replaces() {
        let mut cal = Calibration::new();
        cal.observe(0, 8870);
        // 88.7 x 100 is 8869.999... in binary floating point; Math.round and
        // f64::round both land on 8870.
        assert_eq!(cal.raw_for(88.7), Some(8870));
        assert_eq!(cal.raw_for(107.9), Some(10790));
        assert_eq!(cal.raw_for(87.5), Some(8750));
        assert_eq!(cal.raw_for(101.55), Some(10155));
    }

    #[test]
    fn a_group_is_four_blocks_or_nothing() {
        assert_eq!(parse_group_hex("F2010541E0CDA1B2"), Some(RdsGroup([0xF201, 0x0541, 0xE0CD, 0xA1B2])));
        assert_eq!(parse_group_hex("0000000000000000"), Some(RdsGroup([0, 0, 0, 0])));
        // Short, long, and non-hex are all a poll that caught the buffer
        // mid-refresh, not a group.
        assert_eq!(parse_group_hex("F2010541E0CDA1B"), None);
        assert_eq!(parse_group_hex("F2010541E0CDA1B22"), None);
        assert_eq!(parse_group_hex("F2010541E0CDA1BZ"), None);
        assert_eq!(parse_group_hex(""), None);
        // Lower case is what the device sends on some reads.
        assert_eq!(parse_group_hex("f2010541e0cda1b2"), Some(RdsGroup([0xF201, 0x0541, 0xE0CD, 0xA1B2])));
    }

    #[test]
    fn panel_keys_decode_both_search_codes_and_admit_the_unknown_one() {
        assert_eq!(PanelKey::from_code(5), Some(PanelKey::SearchUp));
        assert_eq!(PanelKey::from_code(60), Some(PanelKey::SearchUp));
        assert_eq!(PanelKey::from_code(6), Some(PanelKey::SearchDown));
        assert_eq!(PanelKey::from_code(59), Some(PanelKey::SearchDown));
        assert_eq!(PanelKey::from_code(62), Some(PanelKey::PresetNext));
        assert_eq!(PanelKey::from_code(63), Some(PanelKey::PresetPrev));
        // Key 14 arrived eight times in the drive log of 2026-08-03 and is in no
        // table anywhere. Reporting it as unknown is the correct answer.
        assert_eq!(PanelKey::from_code(14), None);
        assert_eq!(PanelKey::from_code(-1), None);
        assert_eq!(PanelKey::SearchUp.label(), "search up");
    }

    #[test]
    fn the_band_plan_drops_the_unused_entry() {
        // Exactly what getRadioPoint() returned on 2026-07-31.
        let plan = band_plan_from_raw(&[8750, 10790, 20, 530, 1710, 10, 0, 0, 0]);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0], BandPoint { lo: 8750, hi: 10790, step: 20 });
        assert_eq!(plan[1], BandPoint { lo: 530, hi: 1710, step: 10 });
        // A trailing partial triple is not a band.
        assert_eq!(band_plan_from_raw(&[8750, 10790, 20, 530]).len(), 1);
        assert!(band_plan_from_raw(&[]).is_empty());
    }

    #[test]
    fn a_level_is_only_trustworthy_when_the_tuner_stayed_put() {
        let h = Harness::new();
        ingest_level(62, 10250, 10250, true, None);
        // The overwhelming rejection on the drive logs: landed = 0, the tuner
        // saying it was not ready.
        ingest_level(0, 10250, 0, true, None);
        ingest_level(70, 10250, 8870, true, None);
        ingest_level(0, 0, 0, false, Some("not connected".into()));
        let got = h.drain();
        let trust: Vec<bool> = got
            .iter()
            .map(|e| match e {
                TunerEvent::Level(l) => l.trustworthy,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(trust, vec![true, false, false, false]);
    }

    #[test]
    fn a_preset_slot_is_one_to_six_and_nothing_else() {
        let h = Harness::new();
        ingest_frequency(0, 8870, "WERN".into(), 3);
        ingest_frequency(0, 8870, "WERN".into(), -1);
        ingest_frequency(0, 8870, "WERN".into(), 0);
        ingest_frequency(0, 8870, "WERN".into(), 7);
        let slots: Vec<Option<u8>> = h
            .drain()
            .iter()
            .map(|e| match e {
                TunerEvent::Frequency(f) => f.preset_slot,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(slots, vec![Some(3), None, None, None]);
    }

    #[test]
    fn a_malformed_group_never_reaches_the_decoder() {
        let h = Harness::new();
        ingest_rds_group("F2010541E0CDA1B2");
        ingest_rds_group("nonsense");
        ingest_rds_group("");
        assert_eq!(h.drain().len(), 1);
    }

    #[test]
    fn the_fake_drives_the_same_path_the_device_drives() {
        let h = Harness::new();
        let t = FakeTuner::new();

        // Before a connect there is no state to read and no tune to send.
        assert!(t.snapshot().is_none());
        assert_eq!(t.tune(102.5), Err(TunerError::NotConnected));

        t.connect().unwrap();
        match &h.drain()[..] {
            [TunerEvent::Connected(c)] => {
                assert_eq!(c.raw, 8870);
                assert_eq!(c.mhz, Some(88.7));
                assert_eq!(c.ps, "WERN");
            }
            other => panic!("unexpected {other:?}"),
        }
        // The connect taught the scale, so a tune now converts.
        assert_eq!(calibration().multiplier(), Some(100));

        t.tune(102.5).unwrap();
        match &h.drain()[..] {
            [TunerEvent::Frequency(f)] => {
                assert_eq!(f.raw, 10250);
                assert_eq!(f.mhz, Some(102.5));
            }
            other => panic!("unexpected {other:?}"),
        }

        // Seek walks the station list and wraps at each end.
        t.seek(true);
        t.seek(true);
        t.seek(true);
        let seen: Vec<i32> = h
            .drain()
            .iter()
            .map(|e| match e {
                TunerEvent::Frequency(f) => f.raw,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(seen, vec![10590, 8870, 9110]);

        // A wheel press arrives decoded, with the raw code still attached.
        t.send_panel_key(62);
        match &h.drain()[..] {
            [TunerEvent::PanelKey { code, key, .. }] => {
                assert_eq!(*code, 62);
                assert_eq!(*key, Some(PanelKey::PresetNext));
            }
            other => panic!("unexpected {other:?}"),
        }

        // Audio is not playing until it is claimed.
        assert!(!t.snapshot().unwrap().fm_is_playing());
        t.set_audio_enabled(true);
        assert!(t.snapshot().unwrap().fm_is_playing());

        assert_eq!(t.band_plan().len(), 2);
    }

    #[test]
    fn day_night_needs_its_own_watch_and_not_a_tuner() {
        let h = Harness::new();
        let t = FakeTuner::new();
        // The bug: the watch used to be started only by connect, so a session
        // that never bound the built-in tuner never heard the broadcast.
        t.push_illumination("extra_ill_state=1 (Byte)", "DAY");
        assert!(h.drain().is_empty(), "nothing arrives before the watch starts");

        // And it must not need a connect.
        t.start_illumination_watch();
        t.push_illumination("extra_ill_state=1 (Byte)", "DAY");
        match &h.drain()[..] {
            [TunerEvent::Illumination { extras, ui_mode, .. }] => {
                // The extra's TYPE is unknown, so the dump is verbatim and
                // Android's own night flag travels beside it — that comparison
                // is the whole reason this event exists.
                assert_eq!(extras, "extra_ill_state=1 (Byte)");
                assert_eq!(ui_mode, "DAY");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_sink_may_command_the_tuner_from_inside_a_callback() {
        // The lock must not be held across the sink call. A sink that reacts to
        // an event by reading the calibration is the obvious shape, and holding
        // the lock would deadlock the radio the first time it happened.
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_calibration();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        set_event_sink(move |_| sink.lock().unwrap().push(calibration().multiplier()));
        ingest_frequency(0, 10250, String::new(), -1);
        assert_eq!(*seen.lock().unwrap(), vec![Some(100)]);
        clear_event_sink();
        reset_calibration();
    }
}
