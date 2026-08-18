//! Drives the tune overlay's "Enter frequency" tab through a real `App`.
//!
//! `numpad_press`, `numpad_commit` and `entry_can_tune` are pure and have their
//! own tests, and those tests prove nothing about the tab: the seek has to empty
//! the buffer before it sweeps, CANCEL has to put back the frequency the tab
//! opened on, and TUNE has to close the overlay whether or not what was typed is
//! a dial. All of that lives in the callbacks, which is the layer this project
//! keeps being bitten one step away from.
//!
//! TWO TUNERS, BECAUSE ONE LIED. The first cut of this probe ran everything on
//! `SilentTuner`, whose `seek` is a no-op — so "a seek moved the dial" was never
//! true, the CANCEL-restores assertion sat behind a guard that never opened, and
//! "switching away stops a running seek" passed against an app in which nothing
//! was running. The deep review caught all three. So part 1 keeps the silent
//! tuner for the entry rules, where a tuner that says nothing is the right
//! instrument — and part 2 uses `ProbeTuner`, which can do what the NWD front
//! end does in both of its moods: LAND a seek (one frequency report back, like
//! the fakes and a fast search) or HANG mid-sweep (fire-and-forget, no report
//! yet — the state a real head unit is in for the whole search, which is exactly
//! where CANCEL and the tab-switch stop have their work to do).

mod common;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use carnyx::android::{BandPoint, Tuner, TunerError, TunerSnapshot};
use carnyx::{NearbyTab, Overlay};
use common::{dir_for, install_platform, launch, launch_with};
use slint::ComponentHandle;

/// A tuner whose seek either lands somewhere else or stays in flight.
struct ProbeTuner {
    raw: Mutex<i32>,
    /// `Some(raw)` — a seek reports this frequency at once, the way the fakes
    /// land. `None` — a seek is fire-and-forget and nothing reports, the way the
    /// NWD front end behaves for the whole search.
    lands: Mutex<Option<i32>>,
    tunes: AtomicU32,
}

impl ProbeTuner {
    fn at(mhz: f32) -> Arc<ProbeTuner> {
        Arc::new(ProbeTuner {
            raw: Mutex::new((mhz * 100.0).round() as i32),
            lands: Mutex::new(None),
            tunes: AtomicU32::new(0),
        })
    }
}

struct Handle(Arc<ProbeTuner>);

impl Tuner for Handle {
    fn is_available(&self) -> bool {
        true
    }
    fn connect(&self) -> Result<(), TunerError> {
        let raw = *self.0.raw.lock().unwrap();
        carnyx::android::ingest_connected(0, raw, String::new(), String::new(), -1, true);
        Ok(())
    }
    fn disconnect(&self) {}
    fn tune(&self, mhz: f32) -> Result<(), TunerError> {
        let raw = (mhz * 100.0).round() as i32;
        *self.0.raw.lock().unwrap() = raw;
        self.0.tunes.fetch_add(1, Ordering::SeqCst);
        carnyx::android::ingest_frequency(0, raw, String::new(), -1);
        Ok(())
    }
    fn seek(&self, _up: bool) {
        if let Some(raw) = *self.0.lands.lock().unwrap() {
            *self.0.raw.lock().unwrap() = raw;
            carnyx::android::ingest_frequency(0, raw, String::new(), -1);
        }
        // Otherwise: handed to the front end, nothing back yet — mid-sweep.
    }
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

fn tap(ui: &carnyx::AppWindow, keys: &str) {
    for k in keys.chars() {
        ui.invoke_freq_key(k.to_string().into());
    }
}

/// Open the overlay and switch to the keypad, the only way in that exists.
fn open_freq(ui: &carnyx::AppWindow) {
    ui.invoke_open_nearby();
    ui.set_overlay(Overlay::Nearby);
    ui.invoke_set_nearby_tab(NearbyTab::Freq);
}

fn main() {
    let _window = install_platform();

    // ═══ Part 1: the entry rules, on a tuner that says nothing ═══
    let dir = dir_for("numpad");
    let (ui, _app) = launch(&dir, 96.3);

    // ── The overlay opens on the STATION LIST, never on the keypad ──
    //
    // §2: the nearby button is the only entry point and it lands on Nearby
    // stations. The tab is not remembered between visits either — asserted at the
    // end, after this run has left it on the keypad.
    ui.invoke_open_nearby();
    assert_eq!(ui.get_nearby_tab(), NearbyTab::Nearby);

    // ── The keypad opens on the live dial, dimmed ──
    ui.invoke_set_nearby_tab(NearbyTab::Freq);
    assert_eq!(ui.get_freq_display().as_str(), "96.3");
    assert!(ui.get_freq_display_dim(), "an untyped readout is the dial, dimmed");
    assert!(!ui.get_freq_error());

    // ── THE ENTRY RULES, through the real callback ──
    tap(&ui, "1055");
    assert_eq!(ui.get_freq_display().as_str(), "1055");
    tap(&ui, "9");
    assert_eq!(ui.get_freq_display().as_str(), "1055", "a fifth digit is refused");

    // ── A tap on the tab that is already up is NOT a switch ──
    //
    // TabButton fires unconditionally, and before the same-tab gate existed a
    // stray tap on the active tab wiped what was typed and re-based CANCEL's
    // restore point. The buffer surviving the tap is the gate working.
    ui.invoke_set_nearby_tab(NearbyTab::Freq);
    assert_eq!(ui.get_freq_display().as_str(), "1055", "a re-tap keeps the buffer");

    // ── A value that is not a dial CLOSES, and does not retune ──
    //
    // 1055 MHz is not a dial. The old card kept itself up with the buffer intact;
    // §5 replaced that with a dismissal, so what has to be true now is that the
    // radio did not move.
    let before = ui.get_freq_label().to_string();
    ui.invoke_freq_commit();
    assert_eq!(ui.get_overlay(), Overlay::None, "TUNE always closes");
    assert_eq!(ui.get_freq_label().as_str(), before, "and a non-dial does not tune");

    // ── The rounding reaches the radio, not just the pure function ──
    open_freq(&ui);
    tap(&ui, "105.");
    assert_eq!(ui.get_freq_display().as_str(), "105.", "a trailing point stands");
    tap(&ui, "5");
    assert_eq!(ui.get_freq_display().as_str(), "105.5");
    ui.invoke_freq_commit();
    assert_eq!(ui.get_freq_label().as_str(), "105.5");
    assert_eq!(ui.get_overlay(), Overlay::None);

    // ── THE WARNING FIRES ON WHAT CANNOT COMMIT, AND NOT BEFORE ──
    //
    // The rule that matters, because both naive ones are on the record failing:
    // "is this out of band" lit on the first keystroke of most valid entries, and
    // "is this a prefix of a dial's display string" contradicted the commit it
    // warned about — "87.46" commits to 87.5 by rounding and no display string
    // starts with it. The predicate now asks numpad_commit itself, over the
    // keypad's own grammar; `committable_buffers_never_warn` proves the whole
    // space and these are the landmarks.
    for (buf, want) in [
        ("1", false),     // on the way to 105.1
        ("10", false),    // and to 100.7
        ("105", false),
        ("105.", false),
        ("8", false),     // on the way to 87.5
        ("0", false),     // "087.5" commits to 87.5
        ("87.4", false),  // out of band NOW, but "87.46" rounds home
        ("87.46", false), // commits to 87.5 outright
        ("87.5", false),  // the bottom of the band is not outside it
        ("108.0", false), // nor is the top
        ("7", true),      // no 7-prefixed buffer can ever commit
        ("109", true),
        ("108.1", true),  // out of band and un-extendable
        ("1080", true),
    ] {
        // A real reset, not the re-tap the same-tab gate now ignores: away to the
        // list and back, which §5 says clears the buffer on both crossings.
        ui.invoke_set_nearby_tab(NearbyTab::Nearby);
        ui.invoke_set_nearby_tab(NearbyTab::Freq);
        tap(&ui, buf);
        assert_eq!(
            ui.get_freq_display().as_str(),
            buf,
            "the readout is the buffer exactly as typed"
        );
        assert_eq!(
            ui.get_freq_error(),
            want,
            "{buf:?} should {} the out-of-band line",
            if want { "show" } else { "not show" }
        );
    }

    // ── The ✕ and the scrim just close on the NEARBY tab ──
    ui.invoke_set_nearby_tab(NearbyTab::Nearby);
    ui.invoke_nearby_dismiss();
    assert_eq!(ui.get_overlay(), Overlay::None, "nearby-tab dismissal closes");

    // ── And the tab does not persist across a visit ──
    open_freq(&ui);
    ui.invoke_freq_cancel();
    ui.invoke_open_nearby();
    assert_eq!(ui.get_nearby_tab(), NearbyTab::Nearby, "every visit opens on the list");
    ui.hide().expect("hide");
    let _ = std::fs::remove_dir_all(&dir);

    // ═══ Part 2: seeks, on a tuner that can land or hang ═══
    let dir = dir_for("numpad-seek");
    let tuner = ProbeTuner::at(96.3);
    let (ui, _app) = launch_with(&dir, Box::new(Handle(tuner.clone())));

    // ── A landed seek: buffer cleared first, readout follows, then CANCEL ──
    *tuner.lands.lock().unwrap() = Some(10150);
    open_freq(&ui);
    tap(&ui, "88");
    assert_eq!(ui.get_freq_display().as_str(), "88");
    ui.invoke_freq_seek(1);
    assert_eq!(
        ui.get_freq_display().as_str(),
        "101.5",
        "the seek clears the buffer and the readout follows to the station it found"
    );
    assert!(!ui.get_scanning(), "a landed sweep is over");
    assert!(ui.get_freq_display_dim(), "and the readout is the dial again, dimmed");
    // CANCEL after a landing: the dial MOVED, so the restore is unconditional —
    // no guard, which is what the first cut of this probe hid behind.
    ui.invoke_freq_cancel();
    assert_eq!(ui.get_overlay(), Overlay::None, "CANCEL closes");
    assert_eq!(ui.get_freq_label().as_str(), "96.3", "CANCEL put back the opening dial");

    // ── A sweep IN FLIGHT: the state the real front end is in for the search ──
    //
    // `scanning` is the app's own flag now — set when the seek is handed off,
    // cleared by the next frequency report — and mid-sweep is where it earns its
    // keep: the dial has NOT moved yet, so a restore gated on dial movement alone
    // concluded there was nothing to do and let the sweep land after the driver
    // said no. That was the deep review's headline finding.
    *tuner.lands.lock().unwrap() = None;
    open_freq(&ui);
    let tunes_before = tuner.tunes.load(Ordering::SeqCst);
    ui.invoke_freq_seek(1);
    assert!(ui.get_scanning(), "the sweep is in flight");
    assert!(!ui.get_freq_display_dim(), "a sweep is a reason to be legible");
    assert_eq!(tuner.tunes.load(Ordering::SeqCst), tunes_before, "nothing tuned yet — that would cancel it");
    ui.invoke_freq_cancel();
    assert_eq!(ui.get_overlay(), Overlay::None);
    assert_eq!(
        tuner.tunes.load(Ordering::SeqCst),
        tunes_before + 1,
        "CANCEL mid-sweep must TUNE the restore point — the tune is both the stop and the restore"
    );
    assert_eq!(ui.get_freq_label().as_str(), "96.3");
    assert!(!ui.get_scanning(), "the tune ended the sweep");

    // ── Switching to Nearby stops a running sweep (§5) ──
    open_freq(&ui);
    let tunes_before = tuner.tunes.load(Ordering::SeqCst);
    ui.invoke_freq_seek(1);
    assert!(ui.get_scanning());
    ui.invoke_set_nearby_tab(NearbyTab::Nearby);
    assert_eq!(
        tuner.tunes.load(Ordering::SeqCst),
        tunes_before + 1,
        "the switch must re-tune the dial to stop the sweep"
    );
    assert!(!ui.get_scanning(), "and the sweep is over");
    assert_eq!(ui.get_freq_label().as_str(), "96.3", "back where the sweep started");
    ui.invoke_nearby_dismiss();

    // ── The ✕ and the scrim ARE the abandon-entry path on the keypad ──
    //
    // §5 gives the dismissal two meanings and the branch lives in Rust so this
    // exact call can drive it: mid-sweep, a scrim tap must do everything CANCEL
    // does — stop the sweep AND restore.
    open_freq(&ui);
    let tunes_before = tuner.tunes.load(Ordering::SeqCst);
    ui.invoke_freq_seek(1);
    ui.invoke_nearby_dismiss();
    assert_eq!(ui.get_overlay(), Overlay::None, "the scrim closes");
    assert_eq!(
        tuner.tunes.load(Ordering::SeqCst),
        tunes_before + 1,
        "and from the keypad it restores, exactly as CANCEL does"
    );
    assert_eq!(ui.get_freq_label().as_str(), "96.3");

    ui.hide().expect("hide");
    let _ = std::fs::remove_dir_all(&dir);
    println!("freq tab: entry rules, TUNE always closes, warning only when stuck,");
    println!("          seek lands and hangs, CANCEL/scrim restore mid-sweep, tab switch stops the sweep");
}
