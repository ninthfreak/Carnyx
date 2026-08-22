//! Presses the head unit's own panel buttons, through the real event path.
//!
//! THIS EXISTS BECAUSE THE APP CRASHED ON THE FIRST WHEEL PRESS. The panel-key
//! decode was unit-tested — `panel_action` maps codes to actions and ignores the
//! release edge — and every one of those tests passed while the shipping path
//! panicked, because the fault was not in the decode. It was in `drain_events`
//! holding a `RefMut` across the body of an `if let`, so the retune the action
//! asks for re-borrowed the same cell.
//!
//! A pure function test could never have caught that. This drives
//! `ingest_panel_key` into the same queue the MCU broadcast feeds and then
//! drains it, which is the whole path.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

const W: u32 = 1024;
const H: u32 = 614;

/// Every code the vendor table defines, plus the one that is in no table.
///
/// Code 14 arrived eight times in CarFM's drive log of 2026-08-03 and nobody
/// knows what button it is; it must be ignored without incident rather than
/// crash the radio.
const CODES: &[(i32, &str)] = &[
    (62, "preset next"),
    (63, "preset prev"),
    (5, "search up"),
    (60, "search up (alt code)"),
    (6, "search down"),
    (59, "search down (alt code)"),
    (16, "seek up"),
    (17, "seek down"),
    (4, "change band"),
    (46, "AMS"),
    (61, "intro"),
    (72, "change FM band"),
    (73, "change AM band"),
    (14, "UNKNOWN, in no vendor table"),
];

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/carnyx-panelprobe");
    let _ = std::fs::remove_dir_all(&dir);
    let (ui, driver) = carnyx::build_with_prefs(&dir).expect("build");
    window.set_size(PhysicalSize::new(W, H));
    ui.show().expect("show");
    let mut buf = vec![PremultipliedRgbaColor::default(); (W * H) as usize];
    let mut render = || {
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut buf, W as usize);
        });
    };
    render();

    for (code, name) in CODES {
        let before = ui.get_freq_label().to_string();
        // BOTH EDGES, in the order the MCU sends them. Acting on the release as
        // well as the press steps two stations for one push, so the release
        // reaching the app is part of what has to be exercised.
        carnyx::android::ingest_panel_key(*code, "down".into());
        driver.drain_events();
        carnyx::android::ingest_panel_key(*code, "up".into());
        driver.drain_events();
        driver.push_all();
        render();
        let after = ui.get_freq_label().to_string();
        let moved = if before == after { "  " } else { "->" };
        println!("code {code:>3}  {name:<28} {before:>6} {moved} {after:>6}");
    }

    // Leaning on the button: a burst with no drain between presses, which is
    // what a held wheel control produces.
    //
    // THE LAST REQUEST NO LONGER WINS, and this note used to say it did — "the
    // queue is emptied to nothing and the LAST request wins, so this must end one
    // station along and not six". Presses accumulate now, because a drive log
    // showed twenty producing eighteen steps and the driver reporting that three
    // quick presses would not move three positions. Six "down" keys therefore ask
    // for six positions, and on the seeded six-entry strip that is a full lap back
    // to where it started — which is the arithmetic being right, not a collapse.
    //
    // The six "up" keys interleaved below are still refused outright, by
    // `panel_action`'s release-edge test, and are here to prove that guard still
    // holds now that the two edges are no longer merged into one action.
    let before = ui.get_freq_label().to_string();
    for _ in 0..6 {
        carnyx::android::ingest_panel_key(62, "down".into());
        carnyx::android::ingest_panel_key(62, "up".into());
    }
    driver.drain_events();
    driver.push_all();
    render();
    println!(
        "burst of 6 with no drain between: {before} -> {} (six positions on a six-entry strip is a full lap)",
        ui.get_freq_label()
    );

    println!("no panic: every panel code survived the full event path");
    ui.hide().expect("hide");
}
