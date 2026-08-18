//! Does the card actually SCALE during the step morph, or only move?
//!
//! It only moved, and the driver's words were exact: "stuttery movement, no
//! scaling". Measured here rather than reasoned about — position interpolated
//! smoothly while every frame held the same size.
//!
//! The cause was arithmetic, not Slint. The hero card is 635 wide by 180 TALL
//! and a peek is 180 by 180 — identical heights — so fitting a growing square to
//! `min(card-w, resolved-card-h)` fitted it to a size the peek already was: a
//! 0.2% change. Slint animates a hand-rolled scale perfectly well; there was
//! simply nothing to animate.
//!
//! #75 fixed it at the root by giving `HeroCard` its own `scale`, so the morph is
//! a real FLIP: the incoming hero starts at the source peek's footprint and grows
//! into the hero rect. This holds that line — it fails if the card ever stops
//! taking intermediate sizes across the window.

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

/// The widest horizontal run of the OUTGOING card's brand plate on row `y`.
///
/// Aiming at a region was the first attempt and it measured the wrong thing: the
/// outgoing card starts near the middle of the screen and only enters the prev
/// slot late, so a window over the left edge saw mostly the stationary ghost.
///
/// The plate is the right target because it is part of the card and scales with
/// it — and its colour identifies WHICH card. The outgoing station here is WERN,
/// whose plate is a warm orange; the ghost's is green and the far card's is
/// magenta, so nothing else answers this predicate.
fn plate_run(buf: &[PremultipliedRgbaColor], w: usize, y: usize) -> (usize, usize, usize) {
    let orange = |p: &PremultipliedRgbaColor| {
        let (r, g, b) = (i32::from(p.red), i32::from(p.green), i32::from(p.blue));
        // Any card body: distinctly not the page background (241,243,246-ish).
        r < 236 || g < 238 || b < 241 || (r as i32 - b as i32).abs() > 6
    };
    let (mut best, mut start) = ((0usize, 0usize, 0usize), None);
    for x in 0..w {
        match (orange(&buf[y * w + x]), start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                if x - s > best.2 {
                    best = (s, x - 1, x - s);
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        if w - s > best.2 {
            best = (s, w - 1, w - s);
        }
    }
    best
}

fn dump(name: &str, buf: &[PremultipliedRgbaColor], w: u32, h: u32) {
    let _ = std::fs::create_dir_all("shots");
    let mut rgba = Vec::with_capacity(buf.len() * 4);
    for p in buf {
        let a = u32::from(p.alpha);
        let un = |c: u8| if a == 0 { 0u8 } else { ((u32::from(c)) * 255 / a).min(255) as u8 };
        rgba.extend_from_slice(&[un(p.red), un(p.green), un(p.blue), p.alpha]);
    }
    let f = std::fs::File::create(format!("shots/{name}.png")).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&rgba).unwrap();
}

fn main() {
    let (w, h) = (1024u32, 614u32);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");

    let prefs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/scaleprobe");
    let _ = std::fs::remove_dir_all(&prefs_dir);
    let (ui, _driver) = carnyx::build_with_prefs(&prefs_dir).expect("build window");
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

    // A scanline through the peek cards' vertical middle, where the plate is.
    let scan_y = 290usize; // below the hero card (87..267), across the peeks (127..308)

    slint::platform::update_timers_and_animations();
    render(&mut buffer);
    let (a, b, rest_w) = plate_run(&buffer, w as usize, scan_y);
    println!("rest      : outgoing-card plate x {a}..{b}  (w {rest_w})");
    println!();

    ui.invoke_step_preset(1);
    std::thread::sleep(std::time::Duration::from_millis(20));

    // ONE FRAME, AT A CHOSEN MOMENT. Writing a PNG costs about 650ms, which is
    // longer than the whole 520ms morph — so dumping inside the sampling loop
    // photographs the same late frame over and over. When DUMP_AT_MS is set this
    // advances the clock to that instant, writes one frame and stops.
    if let Ok(at) = std::env::var("DUMP_AT_MS") {
        let at: u64 = at.parse().expect("DUMP_AT_MS");
        // REAL elapsed time, not accumulated sleeps: a render costs more than
        // the sleep between them, so counting sleeps overshot the whole 520ms
        // window and photographed the resting face at every requested instant.
        let t0 = std::time::Instant::now();
        while (t0.elapsed().as_millis() as u64) < at {
            std::thread::sleep(std::time::Duration::from_millis(4));
            slint::platform::update_timers_and_animations();
            render(&mut buffer);
        }
        let (a, b, n) = plate_run(&buffer, w as usize, scan_y);
        println!("t+{at}ms : widest card body x {a}..{b} (w {n})");
        dump(&format!("flip-{at:04}"), &buffer, w, h);
        return;
    }

    let t0 = std::time::Instant::now();
    let mut widths = Vec::new();
    for i in 0..16 {
        slint::platform::update_timers_and_animations();
        render(&mut buffer);
        let ms = t0.elapsed().as_millis();
        let (a, b, n) = plate_run(&buffer, w as usize, scan_y);
        println!("t={ms:>4}ms : plate x {a}..{b}  (w {n})");
        widths.push(n);
        if i < 15 {
            std::thread::sleep(std::time::Duration::from_millis(28));
        }
    }

    println!();
    let distinct: std::collections::BTreeSet<_> = widths.iter().copied().collect();
    println!("distinct widths across the morph: {distinct:?}");
    if distinct.len() <= 2 {
        println!();
        println!("NO SCALING — #75. The card moves but never changes size, because the");
        println!("hero is 635x180 and a peek is 180x180, so fitting the growing square");
        println!("to the hero's smaller dimension fits it to a size the peek already is.");
        println!("Position animates fine; this is arithmetic, not a Slint limit.");
        std::process::exit(1);
    }
    println!("the card takes {} intermediate sizes — the scale is animating", distinct.len());
}
