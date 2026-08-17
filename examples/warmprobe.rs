//! Drives the between-launch warm restore through the REAL `App`, not through
//! `session.rs` alone.
//!
//! The unit tests in `src/session.rs` prove the file round-trips and that the
//! freshness window is the right shape. They prove nothing about the thing the
//! driver sees, because the restore is applied in `App::with_tuner` and then
//! confirmed or destroyed by whatever the tuner says next — and this project's
//! standing lesson is that a check one layer away from the code that runs is not
//! a check. Four faults in a row came from exactly that gap.
//!
//! So this builds whole `App`s against a tuner that reports a dial and then goes
//! quiet, and reads the answers off the face's own properties.

mod common;

use common::{dir_for, install_platform, launch};
use carnyx::rds::RdsState;
use carnyx::session::{self, Parting, Session};
use slint::ComponentHandle;

/// The RadioText a case writes and then looks for. Deliberately carries NO call
/// sign: `strip_station_from_rt` removes one from the head of the string, which
/// would make the assertion below depend on that stripping rather than on the
/// restore.
const SONG: &str = "Bitter Sweet Symphony";

fn snapshot(dial: f32, age_secs: u64) -> Session {
    Session {
        dial,
        saved_at: session::now_unix() - age_secs,
        launches: 3,
        parting: Parting::Pause,
        rds: RdsState {
            pi: Some(0x5A2B),
            pty: Some(10),
            tp: true,
            ta: false,
            ps: "WQLF".into(),
            rt: SONG.into(),
            ps_scrolling: false,
            rt_artist: String::new(),
            rt_title: String::new(),
        },
    }
}

fn main() {
    let _window = install_platform();

    // ── The case the driver reported: away and back within seconds. ──
    //
    // Four seconds old, on the dial the radio comes back up on. The face must
    // carry the text on its FIRST frame — a hero that fills in a second later is
    // still a cold start with a delay on it.
    {
        let dir = dir_for("same-dial");
        session::save(&dir, &snapshot(96.3, 4));
        let (ui, app) = launch(&dir, 96.3);
        assert_eq!(
            ui.get_radio_text().as_str(),
            SONG,
            "a four-second-old snapshot on the same dial must survive the connect"
        );
        assert_eq!(ui.get_freq_label().as_str(), "96.3");

        // THE FIRST FREQUENCY REPORT IS NOT A RETUNE. Some units announce the
        // dial they are already on right after binding, and wiping the restore
        // there would undo it one frame after it was drawn.
        carnyx::android::ingest_frequency(0, 9630, String::new(), -1);
        app.drain_events();
        app.push_all();
        assert_eq!(
            ui.get_radio_text().as_str(),
            SONG,
            "a frequency report on the SAME dial must not wipe the restore"
        );

        // A SECOND report on the same dial gets no such protection. The restore
        // is worth exactly one event, because this is the arm that clears the
        // face and an uncalibrated tuner can repeat a frequency it cannot
        // convert — which would otherwise hold stale text on screen for as long
        // as that lasted.
        carnyx::android::ingest_frequency(0, 9630, String::new(), -1);
        app.drain_events();
        app.push_all();
        assert_eq!(
            ui.get_radio_text().as_str(),
            "",
            "the restore protects one frequency report, not every one"
        );

        // Put it back for the retune case below.
        session::save(&dir, &snapshot(96.3, 4));
        ui.hide().expect("hide");
        let (ui, app) = launch(&dir, 96.3);
        assert_eq!(ui.get_radio_text().as_str(), SONG);

        // A real retune must clear it, exactly as it always did.
        carnyx::android::ingest_frequency(0, 10210, String::new(), -1);
        app.drain_events();
        app.push_all();
        assert_eq!(ui.get_freq_label().as_str(), "102.1");
        assert_eq!(
            ui.get_radio_text().as_str(),
            "",
            "a retune to another station must clear the restored text"
        );

        // And the snapshot this run writes on the way out carries the new dial.
        app.persist_session(Parting::Pause);
        let written = session::load(&dir).expect("the parting snapshot");
        assert_eq!(written.dial, 102.1);
        assert_eq!(written.launches, 4, "the launch counter carries across runs");
        assert_eq!(written.parting, Parting::Pause);
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── The radio came back somewhere else. ──
    //
    // A snapshot is a picture of ONE station, and the only way it can lie is by
    // being shown over a different one.
    {
        let dir = dir_for("other-dial");
        session::save(&dir, &snapshot(96.3, 4));
        let (ui, _app) = launch(&dir, 102.1);
        assert_eq!(ui.get_freq_label().as_str(), "102.1");
        assert_eq!(
            ui.get_radio_text().as_str(),
            "",
            "a snapshot from another dial must be thrown away, not shown"
        );
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Away long enough that the running app would have disowned it. ──
    {
        let dir = dir_for("stale");
        session::save(&dir, &snapshot(96.3, session::WARM.as_secs() + 5));
        let (ui, _app) = launch(&dir, 96.3);
        assert_eq!(
            ui.get_radio_text().as_str(),
            "",
            "past the expiry window the snapshot must be refused"
        );
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── A first run, with nothing on disk at all. ──
    {
        let dir = dir_for("first-run");
        let (ui, app) = launch(&dir, 96.3);
        assert_eq!(ui.get_radio_text().as_str(), "");
        app.persist_session(Parting::Destroy);
        let written = session::load(&dir).expect("even a cold run leaves a record");
        assert_eq!(written.launches, 1, "a first run is launch #1");
        assert_eq!(written.parting, Parting::Destroy);
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    println!("warm restore: same dial kept, other dial discarded, stale refused, counter carried");
}
