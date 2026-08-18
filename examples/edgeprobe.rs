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
//! So the throw is now clamped to the room the card actually has, and this is
//! what holds that line: it renders both layout tracks in both directions and
//! fails if the card is ever narrower than it is at rest, or touching an edge.

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

/// The longest run of near-white pixels on row `y` — the hero card's plate.
///
/// A truncated card is a SHORTER run, which is what makes this measurable at
/// all: the card's own width never changes, so any frame whose run is narrower
/// than the resting one has had part of it cut away by the window edge.
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

/// Antialiasing on a rounded corner moves the measured edge by a pixel or two.
const TOL: usize = 4;

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
            let (_, _, rest_n) = white_run(&buffer, w as usize, scan_y);
            assert!(rest_n > 50, "{track}: no card found on the scanline at rest");

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
                let (a, b, n) = white_run(&buffer, w as usize, scan_y);
                let cut = n + TOL < rest_n || a == 0 || b >= w as usize - 1;
                if cut && worst.map_or(true, |(_, _, _, wn)| n < wn) {
                    worst = Some((ms, a, b, n));
                }
                if i < 13 {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
            }

            match worst {
                None => println!("  OK   {track:<4} dir {dir:>2}: card stays whole, {rest_n}px at rest"),
                Some((ms, a, b, n)) => {
                    println!(
                        "  FAIL {track:<4} dir {dir:>2}: at t+{ms}ms the card is x {a}..{b} \
                         ({n}px of {rest_n}) — it is off the screen edge"
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
