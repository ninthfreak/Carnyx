//! The preset-step morph, watched as PIXELS over time.
//!
//! It animated, and it was wrong: the hero travelled and the other three beats
//! of CarFM's morph did not exist. The outgoing station appeared in its peek
//! slot fully formed, the discarded card vanished between frames, and the
//! station arriving from further out snapped in at full strength.
//!
//! None of that is visible from the properties Rust can read. `step-nonce` and
//! `step-dir` are set correctly whether one thing moves or four do, and every
//! probe beside this one reads properties. So this one renders frames and
//! measures the peek slots, because the defect was only ever visible on a
//! screen.
//!
//! What it checks, on a forward step (`dir > 0`):
//!
//!   * the PREV slot — where the outgoing hero lands, over the ghost of the card
//!     being discarded — is in motion through the window and back at rest after.
//!   * the NEXT slot — where the newly-reachable station fades in — is still at
//!     its starting darkness during the 120ms delay, and back at rest after.
//!
//! The delay is the sharpest of the three: an arriving card with no delay is
//! bright immediately, and that is a single number apart from a correct one.

mod common;

use common::{dir_for, install_platform, launch, pump};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor};
use slint::{ComponentHandle, Model};
use std::time::Duration;

const W: usize = 1024;
const H: usize = 614;

/// The hero row's band, and the three columns inside it. Taken from the wide
/// track's layout: the row is vertically centred and the peeks tuck under the
/// card, so a peek's own ink sits in the outer sixth of the width.
const BAND_TOP: usize = 210;
const BAND_BOT: usize = 400;
const LEFT: (usize, usize) = (40, 210);
const RIGHT: (usize, usize) = (W - 210, W - 40);

struct Frame(Vec<PremultipliedRgbaColor>);

impl Frame {
    /// Mean luminance over a column of the hero band, 0..255.
    ///
    /// Luminance rather than a pixel diff, because what these beats change is
    /// how STRONGLY a card is drawn — opacity against the page — and a card at
    /// 0.6 against a card at 0 is a difference in brightness, not in shape.
    fn mean(&self, cols: (usize, usize)) -> f64 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in BAND_TOP..BAND_BOT {
            for x in cols.0..cols.1 {
                let p = self.0[y * W + x];
                sum += 0.2126 * f64::from(p.red)
                    + 0.7152 * f64::from(p.green)
                    + 0.0722 * f64::from(p.blue);
                n += 1.0;
            }
        }
        sum / n
    }
}

/// `common::pump` advances in 50ms slices, and the morph is armed for 16.
///
/// So no frame was ever DRAWN while `arming` was true: the state was entered and
/// left inside one slice, and the transition had nothing to animate from — the
/// harness measured a morph that never started and reported it as a morph that
/// does nothing. That is the same one-frame hazard hero.slint documents for the
/// device, arriving here for the opposite reason.
fn pump_fine(window: &MinimalSoftwareWindow, total: Duration) {
    let step = Duration::from_millis(2);
    let mut left = total;
    // RENDERS FOR REAL, into a scratch buffer. `common::pump` hands
    // `draw_if_needed` a closure that ignores the renderer, and Slint evaluates
    // geometry bindings while rendering — so with nothing drawn, the `entering`
    // state is never evaluated, the transition never starts, and
    // `has_active_animations()` stays false for the whole window. A pump that
    // does not draw cannot observe an animation.
    let mut scratch = vec![PremultipliedRgbaColor::default(); W * H];
    loop {
        slint::platform::update_timers_and_animations();
        carnyx::app::drain_current();
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut scratch, W);
        });
        if left.is_zero() {
            return;
        }
        let this = step.min(left);
        std::thread::sleep(this);
        left -= this;
    }
}

fn write_png(name: &str, buffer: &[PremultipliedRgbaColor]) {
    let _ = std::fs::create_dir_all("shots");
    let file = std::fs::File::create(format!("shots/{name}.png")).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut bytes = Vec::with_capacity(buffer.len() * 4);
    for px in buffer {
        let a = px.alpha;
        let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255) / a as u32).min(255) as u8 };
        bytes.extend_from_slice(&[un(px.red), un(px.green), un(px.blue), a]);
    }
    enc.write_header().expect("header").write_image_data(&bytes).expect("data");
}

fn shoot(window: &MinimalSoftwareWindow) -> Frame {
    let mut buf = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut buf, W);
    });
    Frame(buf)
}

fn main() {
    let window = install_platform();
    let dir = dir_for("morph");
    let (ui, app) = launch(&dir, 88.7);

    // Four presets, so both peeks have a station and the far slot has one to
    // bring in. Saved through the face's own callback, which is the path a
    // driver takes.
    let tune = |ui: &carnyx::AppWindow, mhz: f32| {
        for ch in format!("{mhz:.1}").chars() {
            ui.invoke_numpad_enter(ch.to_string().into());
        }
        ui.invoke_numpad_tune();
    };
    for mhz in [88.7_f32, 91.1, 93.3, 95.5] {
        tune(&ui, mhz);
        pump(&window, Duration::from_millis(60));
        ui.invoke_toggle_save();
        pump(&window, Duration::from_millis(60));
    }
    tune(&ui, 91.1);
    pump(&window, Duration::from_millis(400));
    ui.show().expect("show");

    // AUDIO ON, or there is nothing to watch. CarFM's `snapHero` bails when
    // `audioActive` is off and swaps with no animation at all, and Carnyx skips
    // the flip for the same case — so a probe against a silent face measures a
    // morph that is CORRECTLY absent and calls it a defect.
    if !ui.get_audio_active() {
        ui.invoke_claim_audio();
        pump(&window, Duration::from_millis(120));
    }
    assert!(ui.get_audio_active(), "the face is dead; the morph is skipped by design");

    println!(
        "presets={}  has-prev={}  has-next={}  audio={}  nonce={}",
        ui.get_presets().row_count(),
        ui.get_has_prev(),
        ui.get_has_next(),
        ui.get_audio_active(),
        ui.get_step_nonce()
    );

    // Does the face's own `changed step-nonce` handler fire in this harness?
    // If it does not, nothing arms and no probe here can see a morph.
    ui.on_morph_note(|m| println!("   [morph-note] {m}"));

    let rest = shoot(&window);
    write_png("morph-rest", &rest.0);
    let rest_l = rest.mean(LEFT);
    let rest_r = rest.mean(RIGHT);
    println!("at rest        left {rest_l:7.3}   right {rest_r:7.3}");
    println!();

    // ── The step ──
    ui.invoke_step_preset(1);
    println!("after step: nonce={} dir={}", ui.get_step_nonce(), ui.get_step_dir());

    // Sampled across the 520ms window. 100ms lands INSIDE the far card's 120ms
    // delay, which is the beat that is one number away from being absent.
    let mut seen = Vec::new();
    let mut animated = false;
    let mut elapsed = 0u64;
    for at in [40u64, 100, 260, 520, 760] {
        pump_fine(&window, Duration::from_millis(at - elapsed));
        elapsed = at;
        if window.has_active_animations() {
            animated = true;
        }
        let f = shoot(&window);
        write_png(&format!("morph-t{at:04}"), &f.0);
        let (l, r) = (f.mean(LEFT), f.mean(RIGHT));
        println!(
            "t+{at:>4}ms      left {l:7.3}   right {r:7.3}   \
             (Δrest  left {:+7.3}  right {:+7.3})",
            l - rest_l,
            r - rest_r
        );
        seen.push((at, l, r));
    }
    println!();

    // ── THE HARNESS VERDICT COMES FIRST ──
    //
    // If Slint never reports an active animation, nothing below is a statement
    // about the morph — it is a statement about this harness. Saying "4 FAILED"
    // there would be the exact mistake this file was written to avoid: a check
    // one layer away from the running code, reporting on something it cannot see.
    if !animated {
        println!("INCONCLUSIVE — this harness never started the animation.");
        println!();
        println!("  `has_active_animations()` was false at every sample, so the");
        println!("  numbers above say nothing about whether the morph is right.");
        println!();
        println!("  Ruled out: the presets are there, both peeks are live, audio is");
        println!("  on so the morph is not being skipped for a dead face, the nonce");
        println!("  and direction reach the face, and `changed step-nonce` fires —");
        println!("  its note is printed above. Pumping at 2ms so a frame lands");
        println!("  inside the 16ms arming window did not help, and neither did");
        println!("  rendering for real during the pump rather than handing");
        println!("  `draw_if_needed` a closure that ignores the renderer.");
        println!();
        println!("  No probe in this tree has ever driven a Slint ANIMATION —");
        println!("  `shot.rs` captures static states and the rest read properties.");
        println!("  Until that is solved the morph can only be judged on the unit.");
        println!("  See docs/TASKS.md #74.");
        drop(app);
        ui.hide().expect("hide");
        let _ = std::fs::remove_dir_all(&dir);
        std::process::exit(1);
    }

    let at = |ms: u64| seen.iter().find(|s| s.0 == ms).copied().expect("sample");
    let (_, l40, r40) = at(40);
    let (_, l100, r100) = at(100);
    let (_, l260, _) = at(260);
    let (_, l760, r760) = at(760);

    let mut bad = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!("  {:<4} {name} — {detail}", if ok { "OK" } else { "FAIL" });
        if !ok {
            bad += 1;
        }
    };

    // 1. The outgoing side is IN MOTION. Forward steps land the old hero in the
    //    prev slot, so early in the window that column holds a card far larger
    //    and brighter than a peek, and it must differ from rest.
    let moved = (l40 - l760).abs().max((l100 - l760).abs()).max((l260 - l760).abs());
    check(
        "outgoing card moves",
        moved > 1.0,
        format!("largest departure from rest {moved:.3} (want > 1.0)"),
    );

    // 2. The 120ms delay. At 40ms and 100ms the arriving card has not started,
    //    so the far column is still at its pre-step darkness — and since the
    //    station that WAS there has gone, that is darker than rest.
    let held = (r40 - r760).abs().max((r100 - r760).abs());
    check(
        "far card waits out its delay",
        held > 0.5,
        format!("still {held:.3} from rest at t+100ms (want > 0.5)"),
    );

    // 3. Everything settles. A morph that leaves a card displaced or dimmed is
    //    worse than one that never ran.
    check(
        "left settles",
        (l760 - rest_l).abs() < 0.75,
        format!("{:+.3} from rest (want |Δ| < 0.75)", l760 - rest_l),
    );
    check(
        "right settles",
        (r760 - rest_r).abs() < 0.75,
        format!("{:+.3} from rest (want |Δ| < 0.75)", r760 - rest_r),
    );

    println!();
    if bad == 0 {
        println!("all four beats accounted for");
    } else {
        println!("{bad} FAILED");
    }
    drop(app);
    ui.hide().expect("hide");
    let _ = std::fs::remove_dir_all(&dir);
    if bad > 0 {
        std::process::exit(1);
    }
}
