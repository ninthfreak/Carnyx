//! TEMPORARY audit probe: the anchor when the app has asserted nothing.
//!
//! Cases the shipped `wheelprobe` does not cover, because every one of its
//! cases calls `tune_for_test` first (which sets `State::asserted`):
//!
//!   1. cold start, vendor's frequency report lands BEFORE the key in the queue
//!   2. cold start, key first (the order the shipped probe uses)
//!   3. the press after one the app itself made (asserted is Some)
//!   4. after a hardware seek, which clears `asserted`

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
const MULTIPLIER: f32 = 100.0;
const PRESET_NEXT: i32 = 62;
const SEEK_UP: i32 = 5;
const STRIP: [f32; 6] = carnyx::fake::SEED_PRESET_MHZ;

fn vendor_reports(mhz: f32) {
    carnyx::android::ingest_frequency(0, (mhz * MULTIPLIER).round() as i32, String::new(), -1);
}

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");
    window.set_size(PhysicalSize::new(W, H));

    println!("strip: {STRIP:?}\n");

    let mut case = |name: &str, tag: &str, body: &dyn Fn(&Rc<carnyx::app::App>) -> ()| {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("target/carnyx-anchorprobe-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        let (ui, driver) = carnyx::build_with_prefs(&dir).expect("build");
        ui.show().expect("show");
        let mut buf = vec![PremultipliedRgbaColor::default(); (W * H) as usize];
        body(&driver);
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut buf, W as usize);
        });
        let got = ui.get_freq_label().to_string();
        println!("{name}\n   dial after the press: {got}\n");
        ui.hide().expect("hide");
    };

    // 1. COLD START, the vendor's own bank step reported BEFORE the key.
    case(
        "1. cold start (nothing tuned), radio sitting on 102.1 (#0);\n   vendor bank steps to \
         105.5 (our #2) and its report is queued FIRST.\n   want 88.7 (#1)",
        "one",
        &|driver| {
            vendor_reports(102.1);
            driver.drain_events();
            vendor_reports(105.5);
            carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
            driver.drain_events();
        },
    );

    // 2. The same press with the ordinary order: key first.
    case(
        "2. cold start, same press, key queued FIRST (wheelprobe's order).\n   want 88.7 (#1)",
        "two",
        &|driver| {
            vendor_reports(102.1);
            driver.drain_events();
            carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
            vendor_reports(105.5);
            driver.drain_events();
        },
    );

    // 3. The app tuned first, so `asserted` is Some — the shipped fix's case.
    case(
        "3. the app tuned to 102.1 itself, then the same inverted order.\n   want 88.7 (#1)",
        "three",
        &|driver| {
            driver.tune_for_test(102.1);
            driver.drain_events();
            vendor_reports(105.5);
            carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
            driver.drain_events();
        },
    );

    // 4. After a hardware seek, which clears `asserted`.
    case(
        "4. app tuned 102.1, then a PANEL SEEK (clears asserted), the seek lands\n   back on \
         102.1, then the inverted press.  want 88.7 (#1)",
        "four",
        &|driver| {
            driver.tune_for_test(102.1);
            driver.drain_events();
            carnyx::android::ingest_panel_key(SEEK_UP, "down".into());
            driver.drain_events();
            vendor_reports(102.1);
            driver.drain_events();
            vendor_reports(105.5);
            carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
            driver.drain_events();
        },
    );

    // 5. Two presses in a row from cold, inverted order both times: does the
    //    second press recover?
    case(
        "5. cold start, TWO inverted presses in a row from 102.1.\n   want 88.7 then 105.5",
        "five",
        &|driver| {
            vendor_reports(102.1);
            driver.drain_events();
            vendor_reports(105.5);
            carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
            driver.drain_events();
            vendor_reports(96.3);
            carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
            driver.drain_events();
        },
    );
}
