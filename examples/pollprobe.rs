//! Drives the post-retune level schedule and the vendor-getter poll through a
//! real `App`.
//!
//! Both were found by an audit that asked what CarFM does and what this tree
//! actually does. The level schedule's five constants had been ported into
//! `src/signal.rs` — with the measurements that justify them and a test pinning
//! their ordering — and nothing called any of them; the poll had been built end
//! to end, `pollNumbers` in Java through `NwdTuner::snapshot()` in Rust, and
//! `snapshot()` was called only from tests. Neither gap was visible from the
//! code, because in both cases the parts were all present.
//!
//! So this probe watches the TUNER, not the functions: a scripted tuner counts
//! every command the App sends it and answers the way a head unit does.
//!
//! It sleeps. The schedule is real `slint::Timer`s and only a real clock moves
//! them; a few seconds is the price of testing the thing that ships.

mod common;

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use carnyx::android::{BandPoint, Tuner, TunerError, TunerSnapshot};
use common::{dir_for, install_platform, launch_with, pump};
use slint::ComponentHandle;
use std::time::Duration;

/// Comfortably past `signal::LEVEL_FIRST_READ_MS` (1000ms) and short of the 4s
/// correction.
const PAST_FIRST: Duration = Duration::from_millis(1500);
/// Comfortably past `signal::LEVEL_CORRECTION_MS` (4000ms) counted from the tune.
const PAST_CORRECTION: Duration = Duration::from_millis(3200);
/// Comfortably past one turn of the 1500ms getter poll.
const PAST_POLL: Duration = Duration::from_millis(2000);

/// A tuner that answers like a head unit and counts what it was asked.
pub struct ScriptedTuner {
    raw: Mutex<i32>,
    /// What `snapshot()` reports as the dial, independent of what the callbacks
    /// have said — which is the whole point of a backstop.
    seen_raw: AtomicI32,
    /// The MCU's audio source register: 4 is FM, 0 is somebody else.
    source: AtomicI32,
    /// How many times the App has commanded a reading.
    reads: AtomicI32,
    /// The interval of the last `start_level_watch`, or -1.
    watch_ms: AtomicI32,
    /// What the next commanded read answers with: (level, landed-raw).
    answer: Mutex<Option<(i32, i32)>>,
}

impl ScriptedTuner {
    fn at(mhz: f32) -> ScriptedTuner {
        let raw = (mhz * 100.0).round() as i32;
        ScriptedTuner {
            raw: Mutex::new(raw),
            seen_raw: AtomicI32::new(raw),
            source: AtomicI32::new(4),
            reads: AtomicI32::new(0),
            watch_ms: AtomicI32::new(-1),
            answer: Mutex::new(Some((62, raw))),
        }
    }
}

impl Tuner for ScriptedTuner {
    fn is_available(&self) -> bool {
        true
    }
    fn connect(&self) -> Result<(), TunerError> {
        carnyx::android::ingest_connected(
            0,
            *self.raw.lock().unwrap(),
            String::new(),
            String::new(),
            -1,
            true,
        );
        Ok(())
    }
    fn disconnect(&self) {}
    fn tune(&self, mhz: f32) -> Result<(), TunerError> {
        let raw = (mhz * 100.0).round() as i32;
        *self.raw.lock().unwrap() = raw;
        self.seen_raw.store(raw, Ordering::Relaxed);
        *self.answer.lock().unwrap() = Some((62, raw));
        carnyx::android::ingest_frequency(0, raw, String::new(), -1);
        Ok(())
    }
    fn seek(&self, _up: bool) {}
    fn snapshot(&self) -> Option<TunerSnapshot> {
        let raw = self.seen_raw.load(Ordering::Relaxed);
        let source = self.source.load(Ordering::Relaxed);
        Some(TunerSnapshot {
            raw,
            mhz: Some(raw as f32 / 100.0),
            band: 0,
            // The three getters the poll must ignore, answering the way this
            // firmware answers: empty, empty, and zero.
            ps: String::new(),
            rt: String::new(),
            pty: 0,
            stereo_getter_stuck: true,
            mcu_source: (source >= 0).then_some(source),
        })
    }
    fn set_audio_enabled(&self, _on: bool) {}
    fn set_rds_enabled(&self, _on: bool) {}
    fn send_panel_key(&self, _code: i32) {}
    fn start_level_watch(&self, interval_ms: i64) {
        self.watch_ms.store(interval_ms as i32, Ordering::Relaxed);
    }
    fn stop_level_watch(&self) {}
    fn read_level_now(&self) {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let asked = *self.raw.lock().unwrap();
        match *self.answer.lock().unwrap() {
            Some((level, landed)) => carnyx::android::ingest_level(level, asked, landed, true, None),
            // A rejection: the chip answering that it was not ready, which on the
            // drive logs came back `landed = 0`.
            None => carnyx::android::ingest_level(0, asked, 0, true, None),
        }
    }
    fn band_plan(&self) -> Vec<BandPoint> {
        Vec::new()
    }
    fn start_illumination_watch(&self) {}
}

fn main() {
    let window = install_platform();

    // ── The post-retune schedule ──
    {
        let dir = dir_for("level-schedule");
        let tuner = std::sync::Arc::new(ScriptedTuner::at(96.3));
        let (ui, _app) = launch_with(&dir, Box::new(TunerHandle(tuner.clone())));

        // Connect took a reading and started the watch at CarFM's cadence, not
        // at the Java floor.
        assert_eq!(
            tuner.watch_ms.load(Ordering::Relaxed),
            carnyx::signal::LEVEL_POLL_MS as i32,
            "the periodic watch runs at CarFM's LEVEL_POLL_MS"
        );
        let before = tuner.reads.load(Ordering::Relaxed);

        // THE REGRESSION. A tune must NOT take a reading on the spot — that is
        // the reading `signal`'s measurements say is inflated by +17.7.
        ui.invoke_select_preset(1);
        assert_eq!(
            tuner.reads.load(Ordering::Relaxed),
            before,
            "a tune must not command a reading immediately"
        );
        assert_eq!(ui.get_level_text().as_str(), "—", "and the meter drops the old station's level");

        // 1s: read, and show whatever comes back.
        pump(&window, PAST_FIRST);
        assert_eq!(
            tuner.reads.load(Ordering::Relaxed),
            before + 1,
            "the 1s read must fire exactly once"
        );
        assert_eq!(ui.get_level_text().as_str(), "62", "and reach the meter");

        // 4s: the correction, which is a restart of the periodic watch.
        tuner.watch_ms.store(-1, Ordering::Relaxed);
        pump(&window, PAST_CORRECTION);
        assert_eq!(
            tuner.watch_ms.load(Ordering::Relaxed),
            carnyx::signal::LEVEL_POLL_MS as i32,
            "the 4s correction re-phases the watch"
        );
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── A rejected reading is retried, twice, and no more ──
    {
        let dir = dir_for("level-retry");
        let tuner = std::sync::Arc::new(ScriptedTuner::at(96.3));
        let (ui, _app) = launch_with(&dir, Box::new(TunerHandle(tuner.clone())));
        ui.invoke_select_preset(1);
        // AFTER the tune, not before: `tune` re-arms the scripted answer, the way
        // a real front end starts answering again once it has landed.
        *tuner.answer.lock().unwrap() = None; // every read now lands on 0
        let before = tuner.reads.load(Ordering::Relaxed);

        // The 1s read, then two retries a second apart, then it stops and waits
        // for the periodic watch.
        pump(&window, Duration::from_millis(4800));
        let spent = tuner.reads.load(Ordering::Relaxed) - before;
        assert_eq!(
            spent,
            1 + carnyx::signal::LEVEL_RETRY_MAX as i32,
            "one scheduled read plus LEVEL_RETRY_MAX retries, and no more"
        );
        assert_eq!(ui.get_level_text().as_str(), "—", "a rejection is the absence of a reading");
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── The getter poll backstops the dial ──
    {
        let dir = dir_for("poll-dial");
        let tuner = std::sync::Arc::new(ScriptedTuner::at(96.3));
        let (ui, _app) = launch_with(&dir, Box::new(TunerHandle(tuner.clone())));
        assert_eq!(ui.get_freq_label().as_str(), "96.3");

        // The radio moved and no notification arrived — the case the bridge's own
        // comment says happens to a passive client.
        tuner.seen_raw.store(10210, Ordering::Relaxed);
        pump(&window, PAST_POLL);
        assert_eq!(
            ui.get_freq_label().as_str(),
            "102.1",
            "the poll must notice a dial no callback reported"
        );
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── The getter poll heals the audio state, and an explicit power-off wins ──
    {
        let dir = dir_for("poll-audio");
        let tuner = std::sync::Arc::new(ScriptedTuner::at(96.3));
        let (ui, _app) = launch_with(&dir, Box::new(TunerHandle(tuner.clone())));
        assert!(ui.get_audio_active(), "FM owns the MCU source at launch");

        // Android Auto takes the speakers. Android will never say so — the MCU
        // register is the only thing that does.
        tuner.source.store(0, Ordering::Relaxed);
        pump(&window, PAST_POLL);
        assert!(!ui.get_audio_active(), "the face must follow the MCU away");

        // And back, which is the half Android audio focus can never deliver.
        tuner.source.store(4, Ordering::Relaxed);
        pump(&window, PAST_POLL);
        assert!(ui.get_audio_active(), "the face must follow the MCU back");

        // THE DRIVER'S OWN POWER-OFF OUTRANKS IT. Without the latch the poll
        // would switch the radio back on a second and a half after every press.
        ui.invoke_release_audio();
        assert!(!ui.get_audio_active());
        tuner.source.store(4, Ordering::Relaxed);
        pump(&window, PAST_POLL);
        assert!(
            !ui.get_audio_active(),
            "an explicit power-off must survive the MCU still calling FM the source"
        );

        // And the power button brings it back, clearing the latch.
        ui.invoke_claim_audio();
        pump(&window, PAST_POLL);
        assert!(ui.get_audio_active());
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── The reception-loss band advances once per poll, not once per event ──
    //
    // `settle_dotted_pairs` is a per-sample state machine and `LOSS_BAND_MARGIN`
    // is calibrated against the rate it is stepped at. It used to be stepped
    // inside `push_meter`, which `push_all` calls, which runs on every wake from
    // the tuner queue — and the raw RDS pump is 90ms, so on a station with RDS
    // the band advanced about eleven times a second against CarFM's two thirds
    // of one.
    //
    // 88.7 so the replayed WERN corpus seeds the decoder with a confirmed PI and
    // a quality ring full of good samples: the band starts clean and there is
    // something for the bad groups below to spoil.
    {
        let dir = dir_for("dotted-cadence");
        let tuner = std::sync::Arc::new(ScriptedTuner::at(88.7));
        let (ui, app) = launch_with(&dir, Box::new(TunerHandle(tuner.clone())));
        pump(&window, PAST_POLL);
        assert_eq!(ui.get_dotted_arcs(), 0, "a clean carrier dots nothing");
        assert_eq!(ui.get_full_pairs(), 2, "level 62 draws two pairs, which is the pool");

        // A ruined channel: eighty groups whose block A carries the wrong PI,
        // enough to fill the 64-slot quality ring. The PI VARIES so the decoder
        // cannot reach consensus on a new one and adopt it — that would make the
        // groups match again, which is the opposite of the point.
        //
        // Each one is drained and republished exactly as the wake hop does it on
        // the device, and NOT ONE `update_timers_and_animations` runs in between,
        // so the poll cannot have fired. Anything that moves the band here moved
        // it off an event.
        for i in 0..80u16 {
            carnyx::android::ingest_rds_group(&format!("{:04x}000000000000", 0xF001 + i));
            app.drain_events();
            app.push_all();
        }
        assert_eq!(
            ui.get_dotted_arcs(),
            0,
            "eighty events must not advance the band by themselves"
        );

        // And one poll turn does the whole move at once — the rising side is
        // adopted immediately, so the leading arc dots at the floor rather than
        // at the floor plus the margin.
        pump(&window, PAST_POLL);
        assert_eq!(ui.get_dotted_arcs(), 2, "one poll turn dots the whole pool at total loss");

        // Back the other way: the recorded corpus again, and the same rule holds.
        let corpus = carnyx::fake::FakeRdsStream::new();
        let groups = corpus.all();
        for hex in groups.iter().cycle().take(80) {
            carnyx::android::ingest_rds_group(hex);
            app.drain_events();
            app.push_all();
        }
        assert_eq!(ui.get_dotted_arcs(), 2, "recovery does not advance on events either");
        pump(&window, PAST_POLL);
        assert_eq!(ui.get_dotted_arcs(), 0, "and the poll clears it");
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    println!("level schedule: no immediate read, 1s read, 4s correction, 2 retries");
    println!("getter poll: dial backstopped, audio healed, power-off respected");
    println!("loss band: eighty events move nothing, one poll turn moves it all");
}

/// `Tuner` is implemented for the struct; the App wants a `Box<dyn Tuner>` while
/// the probe keeps a handle to read the counters. One thin forwarder rather than
/// interior-mutability gymnastics in every method.
struct TunerHandle(std::sync::Arc<ScriptedTuner>);

impl Tuner for TunerHandle {
    fn is_available(&self) -> bool {
        self.0.is_available()
    }
    fn connect(&self) -> Result<(), TunerError> {
        self.0.connect()
    }
    fn disconnect(&self) {
        self.0.disconnect()
    }
    fn tune(&self, mhz: f32) -> Result<(), TunerError> {
        self.0.tune(mhz)
    }
    fn seek(&self, up: bool) {
        self.0.seek(up)
    }
    fn snapshot(&self) -> Option<TunerSnapshot> {
        self.0.snapshot()
    }
    fn set_audio_enabled(&self, on: bool) {
        self.0.set_audio_enabled(on)
    }
    fn set_rds_enabled(&self, on: bool) {
        self.0.set_rds_enabled(on)
    }
    fn send_panel_key(&self, code: i32) {
        self.0.send_panel_key(code)
    }
    fn start_level_watch(&self, interval_ms: i64) {
        self.0.start_level_watch(interval_ms)
    }
    fn stop_level_watch(&self) {
        self.0.stop_level_watch()
    }
    fn read_level_now(&self) {
        self.0.read_level_now()
    }
    fn band_plan(&self) -> Vec<BandPoint> {
        self.0.band_plan()
    }
    fn start_illumination_watch(&self) {
        self.0.start_illumination_watch()
    }
}
