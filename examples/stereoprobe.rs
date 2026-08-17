//! Drives the STEREO pill through a real `App`.
//!
//! The driver's report was that it "almost never lit up", and the cause was two
//! places that cleared the pilot which CarFM does not have — a frequency
//! notification and the RDS expiry — plus a missing settle window that let a
//! flapping callback strobe the pill. Those are behaviours of `App`, not of any
//! function that can be called on its own, so this exercises `App`.
//!
//! It sleeps: `STEREO_SETTLE` is a real `slint::Timer` and only a real clock
//! moves it. A few seconds is the price of testing the thing that ships.

mod common;

use common::{dir_for, install_platform, launch, pump};
use slint::ComponentHandle;
use std::time::Duration;

/// Comfortably past `app::STEREO_SETTLE` (2000ms), so a case that waits is
/// waiting for the window and not for a scheduling accident.
const PAST_WINDOW: Duration = Duration::from_millis(2400);
/// Comfortably inside it.
const INSIDE_WINDOW: Duration = Duration::from_millis(700);

fn main() {
    let window = install_platform();

    // ── The window holds a report back, then lets it through. ──
    {
        let dir = dir_for("stereo-settles");
        let (ui, app) = launch(&dir, 96.3);
        assert!(!ui.get_stereo_known(), "nothing has reported: the pill starts blank");

        carnyx::android::ingest_stereo(true);
        app.drain_events();
        app.push_all();
        assert!(
            !ui.get_stereo_known(),
            "a pilot must hold still for the settle window before it reaches the pill"
        );

        pump(&window, PAST_WINDOW);
        assert!(ui.get_stereo_known() && ui.get_stereo(), "and then it must light");
        ui.hide().expect("hide");
    }

    // ── A flap inside the window lands once, on the last value. ──
    //
    // The window is TRAILING: every report restarts it, which is what stops a
    // station collapsing its pilot on multipath from strobing the pill for a
    // whole drive.
    {
        let dir = dir_for("stereo-flap");
        let (ui, app) = launch(&dir, 96.3);
        for on in [true, false, true, false] {
            carnyx::android::ingest_stereo(on);
            app.drain_events();
            pump(&window, INSIDE_WINDOW);
            assert!(!ui.get_stereo_known(), "a flap inside the window must not reach the pill");
        }
        carnyx::android::ingest_stereo(true);
        app.drain_events();
        pump(&window, PAST_WINDOW);
        assert!(ui.get_stereo_known() && ui.get_stereo(), "the value that held still wins");
        ui.hide().expect("hide");
    }

    // ── THE REGRESSION. A frequency notification must not blank the pill. ──
    //
    // This is the defect the driver saw. The vendor sends
    // `notifyCurrentFrequency` for its own reasons — its preset walk transits
    // several stations on one wheel press — while `notifyStereo` arrives only
    // when the pilot changes, so a handler that cleared the pilot ran far more
    // often than the one that set it. CarFM's own frequency handler clears
    // name, text, pty, tp, ta and pi, and deliberately not stereo.
    {
        let dir = dir_for("stereo-survives-retune");
        let (ui, app) = launch(&dir, 96.3);
        carnyx::android::ingest_stereo(true);
        app.drain_events();
        pump(&window, PAST_WINDOW);
        assert!(ui.get_stereo_known() && ui.get_stereo(), "lit before the notification");

        // The same dial, reported again — the commonest case by far.
        carnyx::android::ingest_frequency(0, 9630, String::new(), -1);
        app.drain_events();
        app.push_all();
        assert!(
            ui.get_stereo_known() && ui.get_stereo(),
            "a frequency report on the same dial must leave the pill alone"
        );

        // And a real retune. The pilot belongs to the front end, not to the
        // station: it holds until the tuner reports a different one.
        carnyx::android::ingest_frequency(0, 10210, String::new(), -1);
        app.drain_events();
        app.push_all();
        assert_eq!(ui.get_freq_label().as_str(), "102.1", "the dial did move");
        assert!(
            ui.get_stereo_known() && ui.get_stereo(),
            "a retune must leave the pill alone too — only the tuner retracts a pilot"
        );

        // Which the tuner duly does.
        carnyx::android::ingest_stereo(false);
        app.drain_events();
        pump(&window, PAST_WINDOW);
        assert!(ui.get_stereo_known() && !ui.get_stereo(), "MONO, once the tuner says so");
        ui.hide().expect("hide");
    }

    // ── The replayed corpus still leaves the face SETTLED. ──
    //
    // `pump_rds_until_settled` assigns the recorded pilot directly instead of
    // going through the settle window, and this is why: the harness renders the
    // frame it builds, so a two-second wait would mean every reference image
    // shows a blank pill over a station the recording says is in stereo. There
    // is no multipath in a recording, which is the only thing the window is for.
    {
        let dir = dir_for("stereo-replay");
        let (ui, _app) = carnyx::build_with_prefs(&dir).expect("build");
        assert_eq!(ui.get_freq_label().as_str(), "88.7", "the fake opens on the recording");
        assert!(
            ui.get_stereo_known() && ui.get_stereo() == carnyx::fake::WERN_STEREO,
            "the replayed pilot must be on the pill by the first frame"
        );
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    println!("stereo pill: settles, collapses a flap, survives every frequency report, replays warm");
}
