//! Is the card that LEAVES a real hero card, and does it start where the hero
//! was?
//!
//! The step morph is a FLIP in both directions now. The incoming half landed
//! first (#75): `HeroCard` gained a `scale`, so the arriving station grows out of
//! the peek slot it came from. The OUTGOING half stayed a `PeekCard` standing in
//! for the hero — and a square cannot be a 635x180 card, so what actually flew
//! away from the centre was a 180px square that had never been on screen.
//!
//! CarFM gets away with a peek there because it FLIPs the real node with a
//! NON-UNIFORM transform, stretching the peek's content out to the hero's shape
//! on the way. Nothing in Slint can distort an element. So the departing card is
//! a real `HeroCard` carrying the station that was playing — better than the
//! stretch rather than a substitute for it.
//!
//! THE CLAIM THIS HOLDS, in three parts, each measured where it lives:
//!
//!   1. Rust captures the hero BEFORE the tune. A property check, because that
//!      is a property: `tune` runs synchronously on the next line, so a capture
//!      one line later would snapshot the station that ARRIVED.
//!   2. The first frame of the morph is the face as it stood. Pixels straight out
//!      of the framebuffer, differenced against the frame taken before the step.
//!      That is what a FLIP means, and it is what the square failed: the hero's
//!      rect was a hole for the whole window.
//!   3. The card then shrinks and travels toward the slot it lands in.
//!
//! WHERE IT MEASURES, and why the first attempt was wrong. A single scanline was
//! taken from `edgeprobe` on the assumption that it crosses the hero and nothing
//! else. It does on the wide track. On the TALL track the arriving card — shrunk
//! into the source slot, vertically centred like everything else in the row —
//! crosses the same line, and the two runs merge into one 326px answer for two
//! cards that are 293 and 40. So the band is derived from the resting frame
//! instead: the hero's own rows, and only the top fifth of them, which the
//! shrunken arriving card cannot reach until it is nearly full size. That the
//! frames say one thing and the number says another is the whole reason this file
//! writes PNGs.
//!
//! NOT COVERED, deliberately: the step that hands off to nothing — forward off an
//! unsaved dial, where the destination slot keeps its occupant. No card departs
//! there, by design, because the dial is not stored in any slot and flying it
//! into one would draw the driver a lie.

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

/// The widest run anywhere in a band of rows, and where it sits.
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

/// Mean absolute channel difference over a band of rows, 0..255.
fn band_diff(
    a: &[PremultipliedRgbaColor],
    b: &[PremultipliedRgbaColor],
    w: usize,
    rows: std::ops::Range<usize>,
) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in rows {
        for x in 0..w {
            let (p, q) = (a[y * w + x], b[y * w + x]);
            sum += u64::from(p.red.abs_diff(q.red));
            sum += u64::from(p.green.abs_diff(q.green));
            sum += u64::from(p.blue.abs_diff(q.blue));
            n += 3;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}

/// The resting hero's row span, grown from a scanline that is known to cross it.
///
/// Rows are accepted when their widest run matches the card's — same width to
/// within a tenth, AND start at the same left edge to within 6px. That pair is
/// what makes a generous gap safe: the card's own star, power button, call sign
/// and frequency break the run over more than a hundred consecutive rows, so a
/// tight gap stops at the first of them and reports a card 18 rows tall. Nothing
/// else on the face is both that wide and that far left, so the walk can run to
/// the window edge and still only ever answer with the hero.
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

/// The topmost row, within the hero's own rows, still carrying a run at least
/// half the resting card's width.
///
/// IMMUNE TO THE MERGE that defeats a width measurement. Both flying cards are
/// vertically centred, so a card that shrinks pulls its top edge DOWN whatever it
/// is doing horizontally, and two cards side by side answer with whichever has
/// the higher top. At rest and at frame 0 that is the card's own top; mid-morph
/// both cards are shorter than the resting hero, so the answer descends. A morph
/// that only translated could not move this number at all.
fn top_edge(
    buf: &[PremultipliedRgbaColor],
    w: usize,
    rows: std::ops::Range<usize>,
    min_w: usize,
) -> Option<usize> {
    rows.into_iter().find(|&y| white_run(buf, w, y).2 >= min_w)
}

/// Mean absolute channel difference over a rectangle.
fn rect_diff(
    a: &[PremultipliedRgbaColor],
    b: &[PremultipliedRgbaColor],
    w: usize,
    rows: std::ops::Range<usize>,
    cols: std::ops::Range<usize>,
) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in rows {
        for x in cols.clone() {
            let (p, q) = (a[y * w + x], b[y * w + x]);
            sum += u64::from(p.red.abs_diff(q.red));
            sum += u64::from(p.green.abs_diff(q.green));
            sum += u64::from(p.blue.abs_diff(q.blue));
            n += 3;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}

/// Antialiasing on a rounded corner moves a measured edge by a pixel or two.
const TOL: usize = 4;

/// Frames, when `OUTPROBE_DUMP` is set. A measurement that disagrees with the
/// picture is a measurement pointed at the wrong thing, and this probe has
/// already been caught doing exactly that once.
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
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");

    let dumping = std::env::var("OUTPROBE_DUMP").is_ok();
    let mut failures = 0;

    for &(w, h, scan_y, track) in &[(1024u32, 614u32, 100usize, "wide"), (360, 800, 300, "tall")] {
        for dir in [1i32, -1] {
            let prefs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/outprobe")
                .join(format!("{track}{dir}"));
            let _ = std::fs::remove_dir_all(&prefs_dir);
            let (ui, _driver) = carnyx::build_with_prefs(&prefs_dir).expect("build window");
            // A saved preset with both neighbours present, so a step in either
            // direction has a slot to come from and a slot to land in.
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
            assert!(rest_n > 50, "{track}: no hero card on the scanline at rest");
            let (top, bottom) = hero_rows(&buffer, w as usize, h as usize, scan_y, rest_a, rest_n);
            let band = top + 6..top + 6 + ((bottom - top) / 5).max(10);
            let before = buffer.clone();
            if dumping {
                dump(&format!("out-{track}{dir}-rest"), &buffer, w, h);
            }
            let (base_a, base_b, base_n) = band_run(&buffer, w as usize, band.clone());

            // What the driver is looking at, as the face itself reports it.
            let want = (
                ui.get_ident(),
                ui.get_freq_label(),
                ui.get_saved(),
                ui.get_has_logo(),
                ui.get_show_call(),
                ui.get_show_freq(),
            );

            ui.invoke_step_preset(dir);
            // Past the 16ms arm timer WITHOUT rendering — the condition a head
            // unit is in while it is busy tuning.
            std::thread::sleep(std::time::Duration::from_millis(20));

            // ── 1. The capture happened before the tune ──
            let got = ui.get_outgoing();
            let carried = got.ident == want.0
                && got.freq_label == want.1
                && got.saved == want.2
                && got.has_logo == want.3
                && got.show_call == want.4
                && got.show_freq == want.5;
            // And the hero really did move on, or the comparison above proves
            // nothing at all.
            let moved = ui.get_ident() != want.0 || ui.get_freq_label() != want.1;
            if !carried || !moved {
                println!(
                    "  FAIL {track:<4} dir {dir:>2}: outgoing carries {:?}/{:?}, the hero was \
                     {:?}/{:?} and is now {:?}",
                    got.ident, got.freq_label, want.0, want.1, ui.get_ident()
                );
                failures += 1;
            }

            // ── 2. Frame 0 is the face as it stood ──
            render(&mut buffer);
            if dumping {
                dump(&format!("out-{track}{dir}-t0"), &buffer, w, h);
            }
            let frame0 = buffer.clone();
            let (a0, b0, n0) = band_run(&buffer, w as usize, band.clone());
            let d = band_diff(&before, &buffer, w as usize, band.clone());
            let whole = n0.abs_diff(base_n) <= TOL && a0.abs_diff(base_a) <= TOL;
            if !whole || d >= 0.5 {
                println!(
                    "  FAIL {track:<4} dir {dir:>2}: frame 0 has {n0}px at x{a0}..{b0} where the \
                     resting hero has {base_n}px at x{base_a}..{base_b}, band diff {d:.2}"
                );
                failures += 1;
            }

            // ── 3. Both cards SCALE ──
            //
            // Measured on the card's TOP EDGE rather than its width, because a
            // width cannot be trusted here: on the tall track the arriving card,
            // shrunk into the source slot, sits at the same height as the
            // departing one and their runs merge into a single answer for two
            // cards. Heights do not merge. Everything in this row is vertically
            // centred, so a card that shrinks pulls its top edge down, and the
            // topmost card-width run across the whole row descends and comes back
            // only when a card has grown into the hero's rect again.
            //
            // A morph that merely translated would leave this number fixed at the
            // card's own top for the whole 520ms — which is what the slide did.
            let rows = top.saturating_sub(12)..bottom;
            let half = rest_n / 2;
            let base_top = top_edge(&before, w as usize, rows.clone(), half).unwrap_or(top);
            let mut deepest = base_top;
            let mut series = vec![(0u128, a0, n0, base_top)];
            // ONE FRAME, AT A CHOSEN MOMENT, kept and written AFTER the loop.
            // Encoding a PNG costs longer than the whole 520ms morph, so writing
            // one inside the sampling loop photographs the same late frame over
            // and over — which is how the last attempt at this ended up with four
            // pictures of a resting face.
            let at: Option<u128> = std::env::var("DUMP_AT_MS").ok().and_then(|v| v.parse().ok());
            let mut held: Option<Vec<PremultipliedRgbaColor>> = None;
            let t0 = std::time::Instant::now();
            while t0.elapsed().as_millis() < 900 {
                slint::platform::update_timers_and_animations();
                render(&mut buffer);
                let ms = t0.elapsed().as_millis();
                if let Some(at) = at {
                    if held.is_none() && ms >= at {
                        held = Some(buffer.clone());
                    }
                }
                let (a, _, n) = band_run(&buffer, w as usize, band.clone());
                let te = top_edge(&buffer, w as usize, rows.clone(), half).unwrap_or(bottom);
                deepest = deepest.max(te);
                series.push((ms, a, n, te));
            }
            if let (Some(at), Some(f)) = (at, held) {
                dump(&format!("flip-{track}{dir}-{at:04}"), &f, w, h);
            }
            let settled = *series.last().unwrap();
            // A tenth of the card's height is far more than antialiasing and far
            // less than the shrink actually reaches.
            let margin = ((bottom - top) / 10).max(6);
            let scaled = deepest > base_top + margin;
            let returned = settled.3.abs_diff(base_top) <= TOL
                && settled.2.abs_diff(base_n) <= TOL
                && settled.1.abs_diff(base_a) <= TOL;

            // ── 4. The direction mapping ──
            //
            // At frame 0 the DESTINATION slot is unchanged: the ghost is drawn
            // over it carrying the station that was peeking there, at the weight
            // it already had. The SOURCE slot has changed: its peek is at zero
            // weight and the arriving hero is standing in it instead. So the two
            // slivers either side of the hero say which way the step went, and
            // they say it about the whole morph rather than about one card.
            let dest = if dir > 0 { 0..rest_a } else { rest_a + rest_n..w as usize };
            let src = if dir > 0 { rest_a + rest_n..w as usize } else { 0..rest_a };
            let d_dest = rect_diff(&before, &frame0, w as usize, rows.clone(), dest);
            let d_src = rect_diff(&before, &frame0, w as usize, rows.clone(), src);
            let mapped = d_dest < 1.0 && d_src > 4.0;

            println!(
                "       {track:<4} dir {dir:>2}: {}",
                series
                    .iter()
                    .take(9)
                    .map(|s| format!("{}px/y{}", s.2, s.3))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            if !scaled || !returned || !mapped {
                println!(
                    "  FAIL {track:<4} dir {dir:>2}: scaled {scaled} (top {base_top} to {deepest}, \
                     needs +{margin}), returned {returned} ({}px at x{} y{}), mapped {mapped} \
                     (destination {d_dest:.2}, source {d_src:.2})",
                    settled.2, settled.1, settled.3
                );
                failures += 1;
            } else {
                println!(
                    "  OK   {track:<4} dir {dir:>2}: leaves at {base_n}px on the hero's own rect \
                     (diff {d:.2}); both cards scale, top edge {base_top} to {deepest} and back; \
                     {} slot untouched, {} slot handed over",
                    if dir > 0 { "prev" } else { "next" },
                    if dir > 0 { "next" } else { "prev" }
                );
            }
            ui.hide().expect("hide");
        }
    }

    println!();
    if failures == 0 {
        println!("the card that leaves is the hero, and it starts where the hero was");
    } else {
        println!("{failures} FAILED");
        std::process::exit(1);
    }
}
