//! A wheel press WITH the vendor's own preset walk happening alongside it.
//!
//! ## Why this exists, and why `panelprobe` did not catch it
//!
//! > "From certain presets, going 'next' skips presets or actually goes
//! > backwards. It didn't feel like it was a problem wrestling with the head
//! > unit's built-in stored presets, because it was so consistent."
//!
//! `panelprobe` drives the same button through the same queue and reports a
//! clean single step every time, because it presses the button into a SILENT
//! tuner. The head unit is not silent. `PanelKey::PresetNext`'s own doc says so:
//! "Steps the SERVICE's own hardware preset bank, not ours. A normal broadcast
//! cannot be cancelled, so the service acts regardless" — so one physical press
//! produces TWO things, a key broadcast and the vendor retuning its own bank,
//! and the vendor's retune arrives as `notifyCurrentFrequency`.
//!
//! `drain_events` empties the tuner queue BEFORE it applies the deferred panel
//! action (`app.rs`, "the panel key, after the queue is empty"). The frequency
//! report therefore lands on `state.dial` first, and `step_preset` then computes
//! `active()` — its idea of WHERE THE DRIVER IS — from the vendor's dial rather
//! than from ours.
//!
//! This probe injects both, in the order the MCU produces them, and prints where
//! the strip actually lands against where the driver asked to go.

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

/// `FakeTuner`'s scale, and the one the ladder settles on for a 100 kHz unit.
const MULTIPLIER: f32 = 100.0;

const PRESET_NEXT: i32 = 62;

/// The app's own strip, in the order `step_preset` walks it.
const STRIP: [f32; 6] = carnyx::fake::SEED_PRESET_MHZ;

/// Pretend the vendor service retuned its OWN bank and reported it.
///
/// Slot -1 is what the tuner sends for "not a preset in this bank", which is the
/// honest value here: the vendor's bank is not ours and the app never learns its
/// indices.
fn vendor_reports(mhz: f32) {
    carnyx::android::ingest_frequency(0, (mhz * MULTIPLIER).round() as i32, String::new(), -1);
}

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/carnyx-wheelprobe");
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

    println!("strip, in step order: {STRIP:?}\n");

    // ── The control: a press into a silent tuner, which is `panelprobe`'s case.
    println!("A. wheel press, vendor silent (what panelprobe tests)");
    let mut wrong_quiet = 0;
    for (i, from) in STRIP.iter().enumerate() {
        vendor_reports(*from);
        driver.drain_events();
        carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
        driver.drain_events();
        render();
        let want = STRIP[(i + 1) % STRIP.len()];
        let got: f32 = ui.get_freq_label().to_string().parse().unwrap_or(f32::NAN);
        let ok = (got - want).abs() < 0.05;
        if !ok {
            wrong_quiet += 1;
        }
        println!(
            "   from {from:>5} (#{i})  want {want:>5}  got {got:>5}  {}",
            if ok { "ok" } else { "WRONG" }
        );
    }

    // ── The real unit: the vendor steps its own bank on the same press.
    //
    // The bank frequency is deliberately NOT one of ours for the first pass —
    // that is the ordinary case, since the vendor's six were stored by whoever
    // owned the unit before and have nothing to do with the app's six.
    println!("\nB. wheel press, vendor walks its own bank to 99.9 (not in our strip)");
    let mut wrong_loud = 0;
    for (i, from) in STRIP.iter().enumerate() {
        vendor_reports(*from);
        driver.drain_events();
        // ONE PRESS, TWO EFFECTS, in the order the MCU produces them.
        carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
        vendor_reports(99.9);
        driver.drain_events();
        render();
        let want = STRIP[(i + 1) % STRIP.len()];
        let got: f32 = ui.get_freq_label().to_string().parse().unwrap_or(f32::NAN);
        let ok = (got - want).abs() < 0.05;
        if !ok {
            wrong_loud += 1;
        }
        println!(
            "   from {from:>5} (#{i})  want {want:>5}  got {got:>5}  {}",
            if ok { "ok" } else { "WRONG" }
        );
    }

    // ── And the case that makes it look like a SKIP rather than a reset: the
    // vendor's bank entry happens to be a dial the app also has.
    println!("\nC. wheel press, vendor bank lands on 105.5, which IS our #2");
    let mut wrong_collide = 0;
    for (i, from) in STRIP.iter().enumerate() {
        vendor_reports(*from);
        driver.drain_events();
        carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
        vendor_reports(105.5);
        driver.drain_events();
        render();
        let want = STRIP[(i + 1) % STRIP.len()];
        let got: f32 = ui.get_freq_label().to_string().parse().unwrap_or(f32::NAN);
        let ok = (got - want).abs() < 0.05;
        if !ok {
            wrong_collide += 1;
        }
        println!(
            "   from {from:>5} (#{i})  want {want:>5}  got {got:>5}  {}",
            if ok { "ok" } else { "WRONG" }
        );
    }

    // ── THE OTHER ARRIVAL ORDER, because the real one cannot be observed from
    // here. Above, the vendor's report shares a drain with the press. If it
    // instead lands in a LATER drain, our step is correct and the dial is left
    // contaminated afterwards — so the FOLLOWING press is the one that misses.
    // Either way the fault reaches the driver; this pass shows the second shape.
    println!("\nD. vendor reports in a LATER drain — the NEXT press is the one that misses");
    let mut wrong_late = 0;
    for (i, from) in STRIP.iter().enumerate() {
        vendor_reports(*from);
        driver.drain_events();
        // Press one: clean, and it lands where it should.
        carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
        driver.drain_events();
        let first: f32 = ui.get_freq_label().to_string().parse().unwrap_or(f32::NAN);
        // The vendor's own walk catches up now, on its own hop.
        vendor_reports(99.9);
        driver.drain_events();
        // Press two, from a dial the app believes is 99.9.
        carnyx::android::ingest_panel_key(PRESET_NEXT, "down".into());
        driver.drain_events();
        render();
        let want = STRIP[(i + 2) % STRIP.len()];
        let got: f32 = ui.get_freq_label().to_string().parse().unwrap_or(f32::NAN);
        let ok = (got - want).abs() < 0.05;
        if !ok {
            wrong_late += 1;
        }
        println!(
            "   from {from:>5} (#{i})  press1 -> {first:>5}  press2 want {want:>5} got {got:>5}  {}",
            if ok { "ok" } else { "WRONG" }
        );
    }

    println!(
        "\nwrong steps — quiet tuner: {wrong_quiet}/6, vendor off-strip: {wrong_loud}/6, \
         vendor on-strip: {wrong_collide}/6, vendor late: {wrong_late}/6"
    );
    ui.hide().expect("hide");
}
