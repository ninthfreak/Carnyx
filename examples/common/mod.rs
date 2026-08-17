//! The rig the behaviour probes share: a headless platform, a tuner that
//! reports a dial and then says nothing, and a way to build a whole `App`
//! against them.
//!
//! It lives here because the probes exist for one reason — this project's
//! standing lesson is that a check one layer away from the code that runs is not
//! a check — and a rig copied into each of them is a second thing to keep true.
//!
//! Each example compiles its OWN copy of this module, so a helper only one probe
//! needs is dead code in the others. That is the shape of a shared rig, not a
//! defect in it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use carnyx::android::{BandPoint, Tuner, TunerError, TunerSnapshot};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{PhysicalSize, PlatformError};

pub struct Headless {
    pub window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

/// Install the headless platform. Once per process, which is once per probe.
pub fn install_platform() -> Rc<MinimalSoftwareWindow> {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");
    window.set_size(PhysicalSize::new(1024, 614));
    window
}

/// A tuner that reports one dial and then says nothing.
///
/// `FakeTuner` cannot be used for these: it always comes up on 88.7, which is
/// the one dial its synthesised RDS corpus covers, so `pump_rds_until_settled`
/// fills the decoder AND the pill the instant it connects and there is nothing
/// left to observe. This reports whatever dial the case needs and never emits a
/// group or a pilot, which is what a real head unit looks like in the second
/// after it binds.
pub struct SilentTuner {
    raw: Mutex<i32>,
}

impl SilentTuner {
    pub fn at(mhz: f32) -> SilentTuner {
        SilentTuner { raw: Mutex::new((mhz * 100.0).round() as i32) }
    }
}

impl Tuner for SilentTuner {
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
        carnyx::android::ingest_frequency(0, raw, String::new(), -1);
        Ok(())
    }
    fn seek(&self, _up: bool) {}
    fn snapshot(&self) -> Option<TunerSnapshot> {
        None
    }
    fn set_audio_enabled(&self, _on: bool) {}
    fn set_rds_enabled(&self, _on: bool) {}
    fn send_panel_key(&self, _code: i32) {}
    fn start_level_watch(&self, _interval_ms: i64) {}
    fn stop_level_watch(&self) {}
    fn read_level_now(&self) {}
    fn band_plan(&self) -> Vec<BandPoint> {
        Vec::new()
    }
    fn start_illumination_watch(&self) {}
}

/// A clean preference directory of its own, so one case cannot inherit what
/// another wrote.
pub fn dir_for(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("carnyx-probe-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Build a whole `App` in `dir` against a tuner sitting on `tuner_dial`, and
/// give back the face plus the driver that owns its callbacks.
pub fn launch(dir: &Path, tuner_dial: f32) -> (carnyx::AppWindow, Rc<carnyx::app::App>) {
    launch_with(dir, Box::new(SilentTuner::at(tuner_dial)))
}

/// The same, against a tuner the caller scripts.
pub fn launch_with(dir: &Path, tuner: Box<dyn Tuner>) -> (carnyx::AppWindow, Rc<carnyx::app::App>) {
    // The calibration is process-global and is seeded by whichever tuner
    // connected last. Cleared between cases so a dial from an earlier one cannot
    // decide what a raw reading means in this one.
    carnyx::android::reset_calibration();
    let ui = carnyx::AppWindow::new().expect("window");
    let driver = carnyx::app::App::with_tuner(
        &ui,
        &carnyx::app::host_db_path(),
        dir,
        tuner,
        false,
        None,
        carnyx::fake::FakeLocation::no_fix(),
    );
    (ui, driver)
}

/// Let real time pass while Slint's timers run and the tuner queue drains.
///
/// Two things need it. The settle windows are real `slint::Timer`s and only a
/// real clock moves them. And the vendor-getter poll runs on its OWN THREAD,
/// pushing events into the process-global queue — on the head unit
/// `slint::invoke_from_event_loop` hops those to the UI thread and drains them,
/// and there is no running event loop here, so this stands in for that hop. It
/// calls the very function the device installs as its wake, which is the point:
/// a probe that drained some other way would be testing some other path.
pub fn pump(window: &MinimalSoftwareWindow, total: std::time::Duration) {
    let step = std::time::Duration::from_millis(50);
    let mut left = total;
    loop {
        slint::platform::update_timers_and_animations();
        carnyx::app::drain_current();
        window.request_redraw();
        window.draw_if_needed(|_| {});
        if left.is_zero() {
            return;
        }
        let this = step.min(left);
        std::thread::sleep(this);
        left -= this;
    }
}
