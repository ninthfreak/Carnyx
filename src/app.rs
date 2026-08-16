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
//! Faked, because the framework is absent from this container: the tuner
//! (`crate::android::FakeTuner`, the tuner builder's own fake, which drives the
//! same `ingest_*` path the device drives), the position, the RDS pump's source,
//! and the logo search's network and image decoder. Every one of them lives in
//! [`crate::fake`] and is named so.
//!
//! Absent entirely, and reported as such rather than stubbed into looking
//! present: preference persistence, the confirm dialog that must stand in front
//! of "clear all logos", and every DIAGNOSTICS action that crosses the framework
//! edge.
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

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::android::{FakeTuner, Tuner, TunerEvent};
use crate::rds::{self, RdsDecoder, RdsState};
use crate::signal;
use crate::station::{brand_color, clean_call, format_mhz, plate_label};
use crate::stations::{NearbyPicker, NearbyState, StationDb, StationRow};
use crate::{fake, settings};
use crate::{
    AppWindow, BatteryState, DiagAction, GenreColumn, NearbyStation, Overlay, Preset, TunerAction,
    TunerDetail, TunerGlyph, TunerSource,
};

// ── The event queue ──────────────────────────────────────────────────────────

static QUEUE: Mutex<VecDeque<TunerEvent>> = Mutex::new(VecDeque::new());
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

thread_local! {
    /// The App on THIS thread, for the wake hop to find.
    ///
    /// A thread-local rather than a captured handle because
    /// `slint::invoke_from_event_loop` demands a `Send` closure and `Rc<App>` is
    /// not `Send` — which is correct, since the App owns an `AppWindow` and that
    /// must never leave the UI thread. `Weak`, so a closed window drops.
    static CURRENT: RefCell<Option<std::rc::Weak<App>>> = const { RefCell::new(None) };
}

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

/// The sink every tuner event lands in. Deliberately the smallest possible
/// amount of work on a binder thread: push, then signal.
fn enqueue(event: TunerEvent) {
    queue().push_back(event);
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
}

impl Slot {
    /// The label the tile's colour box prints, and the key its colour hashes
    /// from. A resolved call sign wins; the dial stands in when nothing resolved.
    fn call(&self) -> String {
        match &self.row {
            Some(r) => plate_label(Some(&r.callsign), &r.callsign),
            None => plate_label(None, &format_mhz(self.mhz)),
        }
    }

    fn name(&self) -> String {
        match &self.row {
            Some(r) => clean_call(&r.callsign),
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
    tuner: Box<dyn Tuner>,
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
    /// Where `prefs.json` lives — the app's data directory on the device.
    prefs_dir: std::path::PathBuf,
    /// The last thing written, so an unchanged state does not rewrite the file.
    /// `push_settings` runs on far more than settings changes.
    saved: crate::prefs::Prefs,
    dial: f32,
    /// The last trustworthy level reading, or `None` for no reading at all.
    level: Option<i32>,
    /// The hysteresis' one piece of state, settled once per poll and fed back.
    dotted: i32,
    audio: bool,
    stereo: Option<bool>,
    location: fake::FakeLocation,
    picker: NearbyPicker,
    settings: settings::Settings,
    logo: crate::logos::search::Model,
    numpad: String,
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
        active_index(self.dial, &self.presets)
    }
}

/// The whole application. One owner of one `AppWindow`, on one thread.
pub struct App {
    ui: slint::Weak<AppWindow>,
    state: RefCell<State>,
}

/// The FM band, and the only band this app tunes.
const FM_LO: f32 = 87.5;
const FM_HI: f32 = 108.0;

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
        App::with_tuner(ui, db_path, &host_prefs_dir(), Box::new(FakeTuner::new()), false)
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
    pub fn with_tuner(
        ui: &AppWindow,
        db_path: &Path,
        prefs_dir: &Path,
        tuner: Box<dyn Tuner>,
        tuner_is_real: bool,
    ) -> Rc<App> {
        let db = StationDb::open(db_path).ok();
        let snapshot = db.as_ref().and_then(|d| d.snapshot_date().ok()).flatten();

        let location = fake::FakeLocation::default();

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
        let dials: Vec<f32> = if crate::prefs::path(&prefs_dir).exists() {
            saved.presets.clone()
        } else {
            fake::seed_presets()
        };
        // Only the dial is stored, so the FCC row is resolved fresh here. A
        // database update therefore improves old presets rather than leaving
        // them pinned to whatever they resolved to when they were saved.
        let presets: Vec<Slot> = dials
            .into_iter()
            .map(|mhz| Slot { mhz, row: resolve(db.as_ref(), mhz, location.position()) })
            .collect();

        let picker = build_picker(db.as_ref(), location, snapshot.clone());

        let app = Rc::new(App {
            ui: ui.as_weak(),
            state: RefCell::new(State {
                db,
                snapshot,
                tuner,
                tuner_is_real,
                rds: RdsDecoder::new(),
                rds_state: RdsState::default(),
                stream: fake::FakeRdsStream::new(),
                presets,
                dial: fake::SEED_DIAL_MHZ,
                level: None,
                dotted: 0,
                audio: true,
                stereo: None,
                location,
                picker,
                settings: settings::Settings {
                    selected: saved.selected,
                    theme: saved.theme,
                    autostart: saved.autostart,
                    logos_on: saved.logos_on,
                    diag_on: saved.diag_on,
                    diag_overlay_on: saved.diag_overlay_on,
                    rds_capture_on: saved.rds_capture_on,
                    debug_on: saved.debug_on,
                    ..settings::Settings::default()
                },
                logo: crate::logos::search::Model::new(),
                numpad: String::new(),
                prefs_dir,
                saved,
            }),
        });

        CURRENT.with(|c| *c.borrow_mut() = Some(Rc::downgrade(&app)));
        crate::android::set_event_sink(enqueue);
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
    pub fn drain_events(&self) {
        loop {
            let Some(event) = queue().pop_front() else { break };
            self.apply_event(event);
        }
    }

    fn apply_event(&self, event: TunerEvent) {
        let mut s = self.state.borrow_mut();
        match event {
            TunerEvent::Connected(c) => {
                if let Some(mhz) = c.mhz {
                    s.dial = mhz;
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
            }
            TunerEvent::ConnectFailed(why) => {
                s.settings.log.push(&stamp(), &format!("connect failed: {why}"));
            }
            TunerEvent::Disconnected => {
                s.settings.log.push(&stamp(), "disconnected");
            }
            TunerEvent::Frequency(f) => {
                if let Some(mhz) = f.mhz {
                    s.dial = mhz;
                }
                // EVERY frequency event, and `reset_for_retune` rather than
                // `reset`: a full reset leaves the old PI as an incumbent that
                // then needs twelve groups to displace instead of three.
                s.rds.reset_for_retune();
                s.rds_state = RdsState::default();
                // A new station has reported nothing yet, so the pill goes back
                // to EMPTY rather than carrying the last station's pilot — and
                // never to MONO, which would be an assertion nothing made.
                s.stereo = None;
                let line = format!("tuned {:.1}", s.dial);
                s.settings.log.push(&stamp(), &line);
            }
            TunerEvent::RdsGroup(g) => {
                let hex = g
                    .0
                    .iter()
                    .map(|b| format!("{b:04x}"))
                    .collect::<String>();
                if let Some(published) = s.rds.push(&hex) {
                    s.rds_state = published;
                }
            }
            TunerEvent::RadioText(rt) => {
                // The vendor's own getter, which is a different path from the
                // decoded 2A groups and is NOT consensus-gated. It is taken only
                // when the decoder has published nothing.
                if s.rds_state.rt.is_empty() {
                    s.rds_state.rt = rt;
                }
            }
            TunerEvent::Stereo(on) => s.stereo = Some(on),
            TunerEvent::Pty(pty) => {
                if s.rds_state.pty.is_none() && (0..=31).contains(&pty) {
                    s.rds_state.pty = Some(pty as u8);
                }
            }
            TunerEvent::Level(l) => {
                // A reading taken while the tuner was moving is not a reading.
                if l.trustworthy {
                    s.level = Some(l.level);
                }
            }
            TunerEvent::PanelKey { code, .. } => {
                s.settings.log.push(&stamp(), &format!("panel key {code}"));
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
        // The pilot the vendor reported alongside the recording, through the
        // same ingest function the device's callback calls.
        crate::android::ingest_stereo(fake::WERN_STEREO);
    }

    /// One level reading, through the tuner's own path.
    fn read_level(&self) {
        self.state.borrow().tuner.read_level_now();
        self.drain_events();
    }

    // ── Pushing state to the face ────────────────────────────────────────────

    /// Publish everything. Cheap enough to do wholesale, and a partial publish is
    /// how a face ends up showing one station's name over another's dial.
    pub fn push_all(&self) {
        self.push_hero();
        self.push_presets();
        self.push_meter();
        self.push_nearby();
        self.push_settings();
        self.push_numpad();
    }

    fn push_hero(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let row = resolve(s.db.as_ref(), s.dial, s.location.position());
        let st = &s.rds_state;

        // IDENTITY ORDER, and it matters: the station database is authoritative
        // for what is on this dial, the decoded PS is a broadcaster-controlled
        // string that many stations scroll song titles through, and the dial
        // itself is the honest fallback. An empty ident makes the face print the
        // frequency as the identity — never an inaccurate "Tuning…".
        let ident = match (&row, st.ps_scrolling, st.ps.as_str()) {
            (Some(r), _, _) => clean_call(&r.callsign),
            (None, false, ps) if !ps.is_empty() => ps.to_string(),
            _ => String::new(),
        };
        ui.set_ident(ident.clone().into());
        ui.set_freq_label(format_mhz(s.dial).into());
        ui.set_in_band((FM_LO..=FM_HI).contains(&s.dial));
        let active = s.active();
        ui.set_saved(active >= 0);
        ui.set_active_index(active);

        // The station's own name must not be stripped out of its own RadioText,
        // so the RESOLVED call sign goes in, never the PS: WIBA scrolls song
        // titles through PS, and a PS of "Walk" would strip "Walk This Way".
        let call = row.as_ref().map(|r| r.callsign_base.clone());
        ui.set_radio_text(
            rds::strip_station_from_rt(&st.rt, Some(s.dial), call.as_deref()).into(),
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
    }

    fn push_presets(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let rows: Vec<Preset> = s.presets.iter().map(to_preset).collect();
        ui.set_presets(ModelRc::from(Rc::new(VecModel::from(rows))));

        let n = s.presets.len() as i32;
        if n == 0 {
            ui.set_has_prev(false);
            ui.set_has_next(false);
            return;
        }
        // With no active preset, prev is the last entry and next the first —
        // which is what the peek cards show on an unsaved dial.
        let active = s.active();
        let (prev, next) = if active < 0 {
            (n - 1, 0)
        } else {
            ((active - 1).rem_euclid(n), (active + 1).rem_euclid(n))
        };
        ui.set_prev_preset(to_preset(&s.presets[prev as usize]));
        ui.set_next_preset(to_preset(&s.presets[next as usize]));
        ui.set_has_prev(true);
        ui.set_has_next(true);
    }

    fn push_meter(&self) {
        let ui = self.ui();
        let mut s = self.state.borrow_mut();
        // The loss figure is the complement of the decoder's block-A match rate.
        // A PROXY, and never "% intact": this tuner exposes no per-block
        // validity, so errors in C and D — where the text lives — are invisible
        // to it, and RadioText will arrive mangled while this reads healthy.
        let loss = s.rds.quality().pi_match_pct.map(|pct| 100.0 - pct);
        let face = signal::meter_face(s.level, loss, s.dotted, !s.audio);
        // Settled ONCE PER POLL and fed back. Settling at render time would run
        // the hysteresis against itself.
        s.dotted = face.dotted_arcs;

        ui.set_full_pairs(face.full_pairs);
        ui.set_half(face.half);
        ui.set_dot_opacity(face.dot_opacity);
        ui.set_dotted_arcs(face.dotted_arcs);
        ui.set_level_text(face.level_text.into());
    }

    fn push_nearby(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let dials: Vec<f32> = s.presets.iter().map(|p| p.mhz).collect();
        let view = s.picker.view(&dials);

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
        ui.set_settings_diag_lines(strings(&cfg.log.lines()));
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
        ui.set_settings_show_band_themes(cfg.egg_open());
        ui.set_settings_egg_labels(strings(&settings::egg_labels()));
        ui.set_settings_egg_index(cfg.egg_index);
        // The borrow above must be released before `save_prefs` takes its own.
        drop(s);
        self.save_prefs();
    }

    fn push_numpad(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let typed = !s.numpad.is_empty();
        // The buffer is handed over EXACTLY as typed — "104." is a legitimate
        // display string mid-entry and normalising it on the way in would fight
        // the user's fingers.
        ui.set_numpad_display(if typed {
            s.numpad.as_str().into()
        } else {
            SharedString::from(format_mhz(s.dial))
        });
        ui.set_numpad_display_dim(!typed);
        ui.set_numpad_can_tune(typed);
        let value = s.numpad.parse::<f32>().ok();
        let bad = typed && value.is_some_and(|v| !(FM_LO..=FM_HI).contains(&v));
        ui.set_numpad_error(bad);
        ui.set_numpad_error_text(format!("Outside {FM_LO}\u{2013}{FM_HI} band").into());
    }

    fn push_logo_search(&self) {
        let ui = self.ui();
        let s = self.state.borrow();
        let view = s.logo.view();
        let brand = brand_color(&s.logo.target().map(|t| t.base.clone()).unwrap_or_default());
        crate::logos::ui::apply(&ui, &view, s.logo.cells(), None, brand);
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
            }
        }
        self.drain_events();
        {
            let mut s = self.state.borrow_mut();
            s.dial = mhz;
            // A retune is a new station: the level from the old one is not a
            // reading of this one.
            s.level = None;
            s.dotted = 0;
        }
        self.pump_rds_until_settled();
        self.read_level();
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
            let next = {
                let s = app.state.borrow();
                let n = s.presets.len() as i32;
                if n == 0 {
                    return;
                }
                let active = s.active();
                if active < 0 {
                    if dir > 0 { 0 } else { n - 1 }
                } else {
                    (active + dir).rem_euclid(n)
                }
            };
            let mhz = app.state.borrow().presets[next as usize].mhz;
            app.tune(mhz);
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
        on!(on_open_settings, |app| {
            app.push_settings();
        });
        on!(on_open_nearby, |app| {
            app.refresh_nearby();
        });
        on!(on_open_numpad, |app| {
            app.state.borrow_mut().numpad.clear();
            app.push_numpad();
        });
        on!(on_close_overlay, |app| {
            app.state.borrow_mut().numpad.clear();
            app.push_numpad();
        });

        // ── §6.1 numpad ──
        on!(on_numpad_enter, |app, c| {
            {
                let mut s = app.state.borrow_mut();
                // Six characters is "108.0" plus a typed decimal point; past
                // that the buffer can only be junk.
                if s.numpad.len() < 6 {
                    let c: SharedString = c;
                    s.numpad.push_str(c.as_str());
                }
            }
            app.push_numpad();
        });
        on!(on_numpad_backspace, |app| {
            app.state.borrow_mut().numpad.pop();
            app.push_numpad();
        });
        on!(on_numpad_tune, |app| {
            let value = app.state.borrow().numpad.parse::<f32>().ok();
            match value {
                // A rejected value KEEPS THE CARD UP with the buffer intact —
                // the error line is the answer, not a dismissal.
                Some(v) if (FM_LO..=FM_HI).contains(&v) => {
                    app.state.borrow_mut().numpad.clear();
                    app.ui().set_overlay(Overlay::None);
                    app.tune(v);
                }
                _ => app.push_numpad(),
            }
        });
        on!(on_numpad_seek, |app, dir| {
            app.state.borrow().tuner.seek(dir > 0);
            app.drain_events();
            let mhz = app.state.borrow().dial;
            app.tune(mhz);
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
            app.state.borrow_mut().settings.autostart = v;
            app.push_settings();
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
        on!(on_settings_tap_about, |app| {
            app.state.borrow_mut().settings.tap_about();
            app.push_settings();
        });
        on!(on_settings_pick_egg, |app, i| {
            app.state.borrow_mut().settings.egg_index = i;
            app.push_settings();
        });

        // ── §6.4 logo search ──
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

    fn toggle_save(self: &Rc<App>) {
        {
            let mut s = self.state.borrow_mut();
            let dial = s.dial;
            match s.active() {
                // Saved: drop it out of the strip.
                i if i >= 0 => {
                    s.presets.remove(i as usize);
                }
                // Unsaved: take the oldest slot, which is what a six-slot strip
                // with no slot picker can do.
                _ => {
                    let at = s.location.position();
                    let row = resolve(s.db.as_ref(), dial, at);
                    if s.presets.len() >= fake::SEED_PRESET_MHZ.len() {
                        s.presets.remove(0);
                    }
                    s.presets.push(Slot { mhz: dial, row });
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
            presets: s.presets.iter().map(|p| p.mhz).collect(),
            selected: s.settings.selected,
            theme: s.settings.theme,
            autostart: s.settings.autostart,
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

    fn set_audio(self: &Rc<App>, on: bool) {
        {
            let s = self.state.borrow();
            s.tuner.set_audio_enabled(on);
        }
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

    fn refresh_nearby(&self) {
        {
            let mut s = self.state.borrow_mut();
            let (db, loc, snap) = (s.db.as_ref(), s.location, s.snapshot.clone());
            let picker = build_picker(db, loc, snap);
            s.picker = picker;
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
                if s.presets.len() >= fake::SEED_PRESET_MHZ.len() {
                    s.presets.remove(0);
                }
                s.presets.push(Slot { mhz, row: Some(row) });
            }
        }
        self.push_presets();
        self.push_nearby();
        self.save_prefs();
    }

    /// Every DIAGNOSTICS row but "Clear log" crosses the framework edge, and
    /// none of it exists here. Each one says so, by name, in the log it would
    /// have written to — which is more useful than a silent no-op and is
    /// honest about what has never run.
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

    fn log_unavailable(self: &Rc<App>, why: &str) {
        {
            let mut s = self.state.borrow_mut();
            let at = stamp();
            s.settings.log.push(&at, why);
        }
        self.push_settings();
    }

    fn open_logo_search(self: &Rc<App>, index: i32) {
        {
            let mut s = self.state.borrow_mut();
            let Some(slot) = s.presets.get(index as usize).cloned() else { return };
            let target = crate::logos::search::Target {
                base: slot.row.as_ref().map_or_else(String::new, |r| r.callsign_base.clone()),
                callsign: slot.row.as_ref().map_or_else(String::new, |r| r.callsign_base.clone()),
                freq_mhz: slot.mhz,
                name: slot.name(),
            };
            // NO STORE IS CONSULTED. `logos::store::LogoStore` exists and is
            // tested, but nothing has ever written a master to it — there is no
            // decoder to produce one — so every station opens as a station with
            // no logo.
            s.logo.open(target, false, None);
        }
        self.push_logo_search();
    }

    /// A search with no network behind it.
    ///
    /// `logos::LogoNet` and `logos::ImageCodec` have NO IMPLEMENTATIONS in this
    /// crate, so this runs the real state machine against
    /// [`crate::fake::FakeLogoSearch`] instead: the generation counter, the
    /// arrival order, the per-cell thumbnail landing and the selection are all
    /// the shipping code's, and only the bytes are invented.
    fn run_logo_search(self: &Rc<App>) {
        let job = self.state.borrow_mut().logo.search();
        let Some(job) = job else { return };
        self.push_logo_search();

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
        match outcome {
            crate::logos::search::Confirm::Ignore => return,
            // THE SAVE CANNOT HAPPEN. Writing a master needs a decoded image and
            // there is no decoder, so the window reports the failure with the
            // wording it would use for a real one rather than pretending the art
            // landed.
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
}

// ── Helpers ──────────────────────────────────────────────────────────────────

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

fn to_preset(slot: &Slot) -> Preset {
    let call = slot.call();
    Preset {
        name: slot.name().into(),
        call: call.as_str().into(),
        // The colour hashes from the CORE letters, so `WWHG` and `WWHG-FM` are
        // one station and get one fill.
        brand: brand_color(&call),
        freq_mhz: slot.mhz,
        freq_label: format_mhz(slot.mhz).into(),
        logo: Default::default(),
        has_logo: false,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let resolved = Slot { mhz: 88.7, row: resolve(Some(&db), 88.7, here) };
        assert_eq!(resolved.call(), "WERN");
        assert_eq!(resolved.name(), "WERN");
        // Nothing on the dial: the frequency stands as the identity, never an
        // inaccurate "Tuning…".
        let bare = Slot { mhz: 87.9, row: None };
        assert_eq!(bare.name(), "87.9");
        assert_eq!(bare.call(), "87.9");
    }

    #[test]
    fn the_picker_answers_from_the_real_table_and_reports_a_missing_fix() {
        let db = StationDb::open(&host_db_path()).unwrap();
        let located = build_picker(Some(&db), fake::FakeLocation::default(), None);
        let view = located.view(&[]);
        assert_eq!(view.state, NearbyState::List);
        assert_eq!(view.stations.len(), 100);
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
            .map(|&mhz| Slot { mhz, row: None })
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
