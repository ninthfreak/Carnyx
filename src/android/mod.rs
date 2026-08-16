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
mod dex;
#[cfg(target_os = "android")]
pub mod location;
#[cfg(target_os = "android")]
pub mod net;
#[cfg(target_os = "android")]
pub mod nwd;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKey {
    ChangeBand,
    SearchUp,
    SearchDown,
    SeekUp,
    SeekDown,
    /// Auto memory store.
    Ams,
    Intro,
    /// Steps the SERVICE's own hardware preset bank, not ours. A normal
    /// broadcast cannot be cancelled, so the service acts regardless and the app
    /// reasserts its own preset immediately after.
    PresetNext,
    PresetPrev,
    ChangeFmBand,
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
    /// The bind was accepted but the service never connected, or it was refused.
    ConnectFailed(String),
    Disconnected,
    Frequency(FrequencyChange),
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
}

// ── Shared state and the event sink ──────────────────────────────────────────

type Sink = Arc<dyn Fn(TunerEvent) + Send + Sync + 'static>;

struct Shared {
    cal: Calibration,
    sink: Option<Sink>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { cal: Calibration::NEW, sink: None });

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

/// Forget everything learned. Only for tests and for a deliberate re-bind.
pub fn reset_calibration() {
    shared().cal = Calibration::NEW;
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
pub fn ingest_position(lat: f64, lon: f64, fix: bool, in_motion: bool) {
    let sane = lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0);
    emit(TunerEvent::Position { lat, lon, fix: fix && sane, in_motion });
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
    // `ok` only says the call itself returned an integer. Whether the READING
    // can be believed is the equality check, and it is made here so there is one
    // definition of a trustworthy level.
    let trustworthy = ok && error.is_none() && asked > 0 && asked == landed;
    emit(TunerEvent::Level(LevelReading { level, asked, landed, trustworthy, error }));
}

pub fn ingest_panel_key(code: i32, action: String) {
    emit(TunerEvent::PanelKey { code, key: PanelKey::from_code(code), action });
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

    /// Pretend the wheel was pressed.
    pub fn push_panel_key(&self, code: i32) {
        ingest_panel_key(code, "com.nwd.action.ACTION_KEY_VALUE".into());
    }

    /// Is FM claimed? Only the fake exposes this; on the device the MCU decides.
    pub fn audio_enabled(&self) -> bool {
        self.lock().audio
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
        ingest_frequency(band as i32, raw, String::new(), -1);
        Ok(())
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
            ingest_position(lat, lon, true, false);
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
        ingest_position(43.07, -89.40, true, true);
        match h.drain().as_slice() {
            [TunerEvent::Position { lat, lon, fix, in_motion }] => {
                assert!(*fix);
                assert!(*in_motion);
                assert_eq!((*lat, *lon), (43.07, -89.40));
            }
            other => panic!("expected one Position, got {other:?}"),
        }
    }

    /// A genuine loss of fix must stay a loss, not be rescued by sane numbers.
    #[test]
    fn losing_the_fix_is_reported_even_with_valid_coordinates() {
        let h = Harness::new();
        ingest_position(43.07, -89.40, false, false);
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
