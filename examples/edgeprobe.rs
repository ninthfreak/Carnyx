//! Does the hero card ever leave the screen during the step morph?
//!
//! It did, on every preset step, and nothing else in this tree could see it.
//! The properties were all correct — `step-nonce`, `step-dir` and the travel
//! were exactly what they should be — and `shots/hero-step-morph.png` catches a
//! frame late enough that the card is already back inside the row. The defect
//! only existed in the FIRST hundred milliseconds, and only as pixels.
//!
//! The cause is worth stating because it is structural rather than a typo.
//! CarFM's incoming hero is SCALED DOWN to the peek's footprint at the start of
//! its FLIP, so centring it on the source peek slot puts a small card in a small
//! slot. Slint exposes no scale transform to user code, so Carnyx's hero travels
//! at FULL SIZE (#75) — and centring a 635px card on a slot 348px off the row's
//! centre put a quarter of it past the bezel, with the save star gone on a
//! forward step and the power button gone on a back one. On the tall track it
//! was about 40%.
//!
//! #75 fixed it properly: `HeroCard` scales, so the card starts AT the peek's
//! footprint and grows, which cannot overflow and needs no clamp. This holds
//! that line — both layout tracks, both directions, failing if the card ever
//! reaches a screen edge.
//!
//! WHAT IT COVERS, checked by sabotage rather than asserted. Deleting the scale
//! from the outgoing card — restoring the exact defect above, a full-size card
//! travelling to a peek slot — fails all four configurations here. Pushing the
//! travel 400px past the slot fails the wide track only.
//!
//! WHAT IT DOES NOT COVER, for the same reason the second sabotage survives on
//! one track: this measures the card while it is HERO-SIZED, in a band above the
//! peek slots, and a card shrunk to a peek's footprint has left that band. That
//! is the right scope rather than a gap in it. The tall track's peek slots run
//! off both bezels at rest by design — they tuck under the hero and are clipped —
//! so a card that has shrunk into one of them is SUPPOSED to be clipped exactly
//! as the peek it lands on is. Leaving the screen is a defect at full size and
//! the layout at a peek's size.

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

/// The longest run of near-white pixels on row `y` — a card body.
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

/// The widest run anywhere in a band of rows.
fn band_run(
    buf: &[PremultipliedRgbaColor],
    w: usize,
    rows: std::ops::Range<usize>,
) -> (usize, usize, usize) {
    let mut best = (0usize, 0usize, 0usize);
    for y in rows {
        let r = white_run(buf, w, y);
        if r.2 > best.2 {
            best = r;
        }
    }
    best
}

/// The resting hero's row span, grown from a scanline known to cross it.
///
/// Duplicated from `outprobe`, which is where the reasoning is written out. The
/// short version: a row is the card's when its widest run matches the card's
/// width to within a tenth AND starts at the same left edge, and that pair is
/// what makes a generous gap safe — the card's own star, power button, call sign
/// and frequency break the run over more than a hundred consecutive rows.
fn hero_rows(
    buf: &[PremultipliedRgbaColor],
    w: usize,
    h: usize,
    scan_y: usize,
    a: usize,
    n: usize,
) -> (usize, usize) {
    let ok = |y: usize| {
        let r = white_run(buf, w, y);
        r.2 * 10 >= n * 9 && r.0.abs_diff(a) <= 6
    };
    let (mut top, mut gap) = (scan_y, 0);
    for y in (0..scan_y).rev() {
        if ok(y) {
            top = y;
            gap = 0;
        } else {
            gap += 1;
            if gap > 250 {
                break;
            }
        }
    }
    let (mut bottom, mut gap) = (scan_y, 0);
    for y in scan_y + 1..h {
        if ok(y) {
            bottom = y;
            gap = 0;
        } else {
            gap += 1;
            if gap > 250 {
                break;
            }
        }
    }
    (top, bottom)
}

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");

    let mut failures = 0;

    // Both layout tracks, because the tall one was the worse of the two and the
    // wide one is the only one usually looked at. The scanline is chosen per
    // track to cross the card's plate and no text.
    for &(w, h, scan_y, track) in &[(1024u32, 614u32, 100usize, "wide"), (360, 800, 300, "tall")] {
        for dir in [1i32, -1] {
            let prefs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/edgeprobe")
                .join(format!("{track}{dir}"));
            let _ = std::fs::remove_dir_all(&prefs_dir);
            let (ui, _driver) = carnyx::build_with_prefs(&prefs_dir).expect("build window");
            // Land on a saved preset so both peeks exist and a step in either
            // direction has somewhere to come from.
            ui.invoke_select_preset(2);
            window.set_size(PhysicalSize::new(w, h));
            ui.show().expect("show");

            let mut buffer = vec![PremultipliedRgbaColor::default(); (w as usize) * (h as usize)];
            let render = |buffer: &mut Vec<PremultipliedRgbaColor>| {
                window.request_redraw();
                window.draw_if_needed(|r| {
                    r.render(buffer, w as usize);
                });
            };

            slint::platform::update_timers_and_animations();
            render(&mut buffer);
            let (rest_a, _, rest_n) = white_run(&buffer, w as usize, scan_y);
            assert!(rest_n > 50, "{track}: no card found on the scanline at rest");
            // A BAND, NOT THE SCANLINE, and the tall track is why. The peek slots
            // there run off both bezels at rest by design — they tuck under the
            // hero and are clipped — and the arriving card, shrunk into one of
            // them, sits at the same height as the departing hero. On a scanline
            // through the middle of the row their two runs merge into a single
            // answer that reaches x=359, and this probe read that as the HERO at
            // the bezel: a card 293px wide and one 40px wide reported as one card
            // 326px wide, touching an edge neither of them touches. Measured
            // instead in the top fifth of the card's own rows, which is above the
            // peeks on both tracks and which the arriving card cannot reach until
            // it has grown most of the way home. `outprobe` derives the same band
            // and says more about why.
            let (top, bottom) = hero_rows(&buffer, w as usize, h as usize, scan_y, rest_a, rest_n);
            let band = top + 6..top + 6 + ((bottom - top) / 5).max(10);
            let (_, _, base_n) = band_run(&buffer, w as usize, band.clone());

            ui.invoke_step_preset(dir);
            // Past the 16ms arm timer WITHOUT rendering — the condition the head
            // unit is in, busy tuning, and the one `shot.rs` reproduces too.
            std::thread::sleep(std::time::Duration::from_millis(20));

            let mut worst: Option<(u128, usize, usize, usize)> = None;
            let t0 = std::time::Instant::now();
            for i in 0..14 {
                slint::platform::update_timers_and_animations();
                render(&mut buffer);
                let ms = t0.elapsed().as_millis();
                let (a, b, n) = band_run(&buffer, w as usize, band.clone());
                // NOT a width comparison any more. Since #75 the hero legitimately
                // SHRINKS during the flip — it starts at the peek's footprint —
                // so "narrower than at rest" is now the correct state for most of
                // the window. What must never happen is the card reaching a
                // bezel, which is what the unclamped slide did.
                // `n == 0` is "no card crosses this scanline", which is now an
                // ordinary state: shrunk to the peek's footprint the hero is far
                // shorter than at rest and simply is not on this row. Reading
                // that as `a == 0` reported a card at the left bezel that was not
                // there at all.
                let cut = n > 20 && (a == 0 || b >= w as usize - 1);
                if cut && worst.map_or(true, |(_, _, _, wn)| n < wn) {
                    worst = Some((ms, a, b, n));
                }
                if i < 13 {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
            }

            match worst {
                None => println!("  OK   {track:<4} dir {dir:>2}: card stays whole, {base_n}px at rest"),
                Some((ms, a, b, n)) => {
                    println!(
                        "  FAIL {track:<4} dir {dir:>2}: at t+{ms}ms the card is x {a}..{b} \
                         ({n}px) — it has reached a screen edge"
                    );
                    failures += 1;
                }
            }
            ui.hide().expect("hide");
        }
    }

    println!();
    if failures == 0 {
        println!("the hero card never leaves the screen");
    } else {
        println!("{failures} FAILED");
        std::process::exit(1);
    }
}
