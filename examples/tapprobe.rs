//! What does a tap on the hero card's own edge actually do?
//!
//! ## The claim under test
//!
//! The peek cards are declared BEFORE the resting hero (`ui/hero.slint`, the two
//! `PeekCard` instances then `card := HeroCard`) and are tucked under it by
//! `Metrics.peek-overlap`. `HeroCard` carries no card-wide `TouchArea` — only the
//! save star and the power button — and a Slint `Rectangle` does not consume
//! pointer events. So the hero may occlude the peeks VISUALLY while the peek's
//! own full-card `TouchArea` still wins the hit test, which would make a strip of
//! the hero's left and right edge a live "step preset" button.
//!
//! `ui/hero.slint` documents exactly this mechanism for the card IN FLIGHT and
//! says the flying card is `inert` because of it. The question here is whether
//! the RESTING peeks underlap the resting hero the same way.
//!
//! ## How this answers it
//!
//! Not by reading the tree — by dispatching real pointer events into the window
//! and watching the dial. Every sample retunes to a known preset first, taps
//! once, and reads `freq-label` back, so each x is independent of the last.
//!
//! An accidental tap near the card's edge silently changing station is a defect;
//! a tap there doing nothing is the design working. This prints which.
//!
//! ## The answer, measured
//!
//! CONFIRMED on the wide track, at 1024x614. The hero card's body spans
//! x 200..824. Tapping x 74..254 tunes the PREVIOUS preset and x 770..950 the
//! NEXT one, so 56px of the hero's own body on each side steps the strip. The
//! live band is y 128..304 — 180px, which is exactly `peek-w * 0.88`, the
//! PeekCard's authored square. It is the resting peek underlapping the hero,
//! not anything to do with the morph.
//!
//! TWO WRONG ANSWERS ON THE WAY, both from the measurement rather than the
//! code, and both worth leaving recorded because the same traps are still here:
//!
//!  1. Sampling the vertical extent at the midpoint of the whole hit band put
//!     the sample OUTSIDE the card, where the peek is simply the peek.
//!  2. The preset TILE for the previous station sits lower down and tunes the
//!     identical frequency, so "which rows tune 88.7" reported one span of
//!     432px across a gap. It is two regions: the peek at y 128..304 and the
//!     tile at y 460..556. Runs, not first-to-last.
//!
//! WHAT THIS DOES NOT ESTABLISH. On the tall track no x on the centre row moves
//! the dial at all — not over the card and not over the peek slots either — so
//! this probe has not shown the tall track to be free of the defect, only that
//! it did not find it there. The tall track's peeks are clipped off both bezels
//! by design, and why they take no tap on that row is unexamined.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, PlatformError};

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

/// The longest run of near-white pixels on row `y` — a card body. Same rule as
/// `edgeprobe`, where the reasoning is written out.
fn white_run(buf: &[PremultipliedRgbaColor], w: usize, y: usize) -> (usize, usize, usize) {
    let mut best = (0usize, 0usize, 0usize);
    let mut run_start = None;
    for x in 0..w {
        let p = buf[y * w + x];
        let white = p.red >= 250 && p.green >= 250 && p.blue >= 250;
        match (white, run_start) {
            (true, None) => run_start = Some(x),
            (false, Some(s)) => {
                if x - s > best.2 {
                    best = (s, x - 1, x - s);
                }
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = run_start {
        if w - s > best.2 {
            best = (s, w - 1, w - s);
        }
    }
    best
}

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");

    let mut failures = 0;

    for &(w, h, scan_y, track) in &[(1024u32, 614u32, 100usize, "wide"), (360, 800, 300, "tall")] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/tapprobe")
            .join(track);
        let _ = std::fs::remove_dir_all(&dir);
        let (ui, driver) = carnyx::build_with_prefs(&dir).expect("build");
        window.set_size(PhysicalSize::new(w, h));
        ui.show().expect("show");

        let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        let mut render = || {
            window.request_redraw();
            window.draw_if_needed(|r| {
                r.render(&mut buf, w as usize);
            });
        };

        // Entry 2 of the seeded six, so both neighbours exist and a step in
        // either direction has somewhere to go.
        ui.invoke_select_preset(2);
        driver.drain_events();
        render();
        let home = ui.get_freq_label().to_string();
        let (card_a, card_b, card_n) = white_run(&buf, w as usize, scan_y);
        assert!(card_n > 40, "{track}: no hero card found on the scanline");

        let tap = |x: f32, y: f32| {
            let at = LogicalPosition::new(x, y);
            let win = ui.window();
            win.dispatch_event(WindowEvent::PointerMoved { position: at });
            win.dispatch_event(WindowEvent::PointerPressed { position: at, button: PointerEventButton::Left });
            win.dispatch_event(WindowEvent::PointerReleased { position: at, button: PointerEventButton::Left });
        };

        // One sample per x: go home, tap, read back. `select_preset(2)` is the
        // ordinary tile path, so the reset itself is never the thing measured.
        let probe = |x: f32, y: f32| -> String {
            ui.invoke_select_preset(2);
            driver.drain_events();
            tap(x, y);
            driver.drain_events();
            ui.get_freq_label().to_string()
        };

        let mid_y = (h as f32) / 2.0;
        println!("\n{track} {w}x{h}: hero card body spans x {card_a}..{card_b} ({card_n}px), home {home}");

        // Sweep the whole row width and record every x whose tap moves the dial.
        let mut hits: Vec<(u32, String)> = Vec::new();
        let mut x = 0u32;
        while x < w {
            let got = probe(x as f32, mid_y);
            if got != home {
                hits.push((x, got));
            }
            x += 2;
        }

        // Collapse to contiguous bands so the output is readable.
        let mut bands: Vec<(u32, u32, String)> = Vec::new();
        for (x, got) in hits {
            match bands.last_mut() {
                Some((_, end, g)) if *g == got && x - *end <= 2 => *end = x,
                _ => bands.push((x, x, got)),
            }
        }

        for (a, b, got) in &bands {
            let inside = *b >= card_a as u32 && *a <= card_b as u32;
            let where_ = if inside { "OVERLAPS THE HERO CARD" } else { "peek slot" };
            println!("   x {a:>4}..{b:<4} -> {got:<6}  {where_}");
            if inside {
                // Only the part actually over the card is the defect.
                let lo = (*a).max(card_a as u32);
                let hi = (*b).min(card_b as u32);
                println!(
                    "        {}px of the hero's own body tunes a different station",
                    hi.saturating_sub(lo) + 2
                );
                failures += 1;
            }
        }
        if bands.is_empty() {
            println!("   nothing on this row changes the dial");
        }

        // And the vertical extent, at an x proven live above (if any).
        if let Some((a, b, got_for)) = bands.iter().find(|(a, b, _)| {
            *b >= card_a as u32 && *a <= card_b as u32
        }) {
            // AN X OVER THE CARD, not the midpoint of the whole band — the
            // band reaches well outside the hero and its own peek slot is
            // taller than the overlap, so sampling there measures the peek
            // rather than the defect.
            let lo = (*a).max(card_a as u32);
            let hi = (*b).min(card_b as u32);
            let x = ((lo + hi) / 2) as f32;
            // MEASURED DIFFERENTIALLY, against a column over the middle of the
            // hero card where no peek can reach. Answering "which rows tune the
            // prev station" is not enough: the PRESET TILE for that same station
            // sits in the strip below and gives the identical answer, which is
            // what made a first cut of this probe report a 432px band. A row is
            // the peek's only if the overlap column tunes there and the control
            // column does not.
            let want = got_for.clone();
            let control = ((card_a + card_b) / 2) as f32;
            let mut live: Vec<u32> = Vec::new();
            let mut other = 0u32;
            let mut y = 0u32;
            while y < h {
                let r = probe(x, y as f32);
                if r == want && probe(control, y as f32) != want {
                    live.push(y);
                } else if r != home {
                    other += 1;
                }
                y += 4;
            }
            // CONTIGUOUS RUNS, not first..last. The preset TILE for this same
            // station sits lower down and answers identically, so a single
            // span across both reported a band 2.4x the peek's own height.
            let mut runs: Vec<(u32, u32)> = Vec::new();
            for y in &live {
                match runs.last_mut() {
                    Some((_, end)) if *y - *end <= 4 => *end = *y,
                    _ => runs.push((*y, *y)),
                }
            }
            match runs.first() {
                Some(_) => println!(
                    "   at x={x}, taps tuning {want}: {}  ({other} row(s) via something else)",
                    runs.iter()
                        .map(|(a, b)| format!("y {a}..{b} ({}px)", b - a + 4))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None => println!("   at x={x}, no row is live"),
            }
        }
    }

    println!();
    if failures == 0 {
        println!("no tap inside the hero card's body changes station");
    } else {
        println!("{failures} band(s) of the hero card's own body step the preset strip");
        std::process::exit(1);
    }
}
