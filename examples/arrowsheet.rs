//! Draw all thirteen maneuver arrows on one sheet, so they can be LOOKED at.
//!
//! `src/arrow.rs` proves its invariants — every glyph the same height, centred,
//! inside the box, left going left. None of that is the same as the set reading
//! like turn arrows, and the first version of the generator passed every test it
//! had while drawing a left turn as a small mark in the corner of its box.
//!
//! Writes `shots/arrows.svg` (gitignored, like every other shot). Render it to
//! a PNG with the Chromium that ships in this container:
//!
//! ```text
//! cargo run --example arrowsheet
//! /opt/pw-browsers/chromium --headless --no-sandbox --disable-gpu \
//!     --hide-scrollbars --force-device-scale-factor=3 \
//!     --screenshot=shots/arrows.png --window-size=1040,215 \
//!     "file://$PWD/shots/arrows.svg"
//! ```

use carnyx::arrow;
use carnyx::nav::Turn;

/// The turns in the order §4.9's table lists them, so the sheet reads as the
/// spec does: straight, then out to each side by degree.
const SHEET: &[(&str, Turn)] = &[
    ("TU -179", Turn::UTurn),
    ("TSHL -135", Turn::SharpLeft),
    ("TL -90", Turn::Left),
    ("TSLL -45", Turn::SlightLeft),
    ("KL -20", Turn::KeepLeft),
    ("C 0", Turn::Straight),
    ("KR 20", Turn::KeepRight),
    ("TSLR 45", Turn::SlightRight),
    ("TR 90", Turn::Right),
    ("TSHR 135", Turn::SharpRight),
    ("TRU 179", Turn::RightUTurn),
    ("RNDB", Turn::Roundabout),
    ("RNLB", Turn::RoundaboutLeft),
];

/// The two weights the mini-handoff names: *"stroke 2.1 at the large sizes, 2.4
/// at 26–28dp"*. Both rows are drawn, because a glyph that reads at one weight
/// and blots at the other is not finished.
const WEIGHTS: [f32; 2] = [2.1, 2.4];

/// One cell, in SVG units. The glyph is a 24 box inside it.
const CELL: f32 = 80.0;
const PAD: f32 = 14.0;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shots");
        std::fs::create_dir_all(&dir).expect("the shots directory is writable");
        dir.join("arrows.svg").display().to_string()
    });
    let cols = SHEET.len() as f32;
    let w = cols * CELL;
    let h = WEIGHTS.len() as f32 * (CELL + 22.0) + 10.0;

    let mut svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' \
         viewBox='0 0 {w} {h}'><rect width='100%' height='100%' fill='#0E0E10'/>"
    );
    let scale = (CELL - 2.0 * PAD) / arrow::BOX;

    for (row, weight) in WEIGHTS.iter().enumerate() {
        let top = row as f32 * (CELL + 22.0);
        for (col, (label, turn)) in SHEET.iter().enumerate() {
            let x = col as f32 * CELL + PAD;
            let y = top + PAD;
            // The box itself, so a glyph that hugs an edge is visible as such.
            svg.push_str(&format!(
                "<rect x='{x}' y='{y}' width='{}' height='{}' fill='none' \
                 stroke='#2A2A2E' stroke-width='1'/>",
                arrow::BOX * scale,
                arrow::BOX * scale,
            ));
            svg.push_str(&format!(
                "<g transform='translate({x},{y}) scale({scale})'>\
                 <path d='{}' fill='none' stroke='#FFA000' stroke-width='{weight}' \
                 stroke-linecap='round' stroke-linejoin='round'/></g>",
                arrow::path(*turn),
            ));
            // A roundabout carries its exit number inside the ring; the sheet
            // shows one so the space it has to fit in is visible.
            if matches!(turn, Turn::Roundabout | Turn::RoundaboutLeft) {
                let (cx, cy) = ring_centre(*turn);
                svg.push_str(&format!(
                    "<text x='{}' y='{}' fill='#FFA000' font-family='sans-serif' \
                     font-size='{}' font-weight='700' text-anchor='middle' \
                     dominant-baseline='central'>3</text>",
                    x + cx * scale,
                    y + cy * scale,
                    9.0 * scale,
                ));
            }
            svg.push_str(&format!(
                "<text x='{}' y='{}' fill='#8A8F98' font-family='monospace' \
                 font-size='11' text-anchor='middle'>{label}</text>",
                col as f32 * CELL + CELL / 2.0,
                top + CELL + 4.0,
            ));
        }
    }
    svg.push_str("</svg>");
    std::fs::write(&out, svg).expect("the sheet is writable");
    println!("{out}");

    // And the paths themselves, so a diff of this run reads without a browser.
    for (label, turn) in SHEET {
        println!("{label:>10}  {}", arrow::path(*turn));
    }
}

/// Where a roundabout's ring sits, for placing the exit number over it.
///
/// Read back out of the emitted path rather than assumed: the ring's centre is
/// not the box's centre, because the circulation arrowhead pushes the ink to one
/// side and the whole glyph is then centred on that.
fn ring_centre(turn: Turn) -> (f32, f32) {
    let d = arrow::path(turn);
    let toks: Vec<&str> = d.split_whitespace().collect();
    // The four ring quarters end at the ring's four extremes.
    let mut xs: Vec<f32> = Vec::new();
    let mut ys: Vec<f32> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if *t == "A" {
            xs.push(toks[i + 6].parse().expect("a number"));
            ys.push(toks[i + 7].parse().expect("a number"));
        }
    }
    let mid = |v: &[f32]| {
        let lo = v.iter().copied().fold(f32::MAX, f32::min);
        let hi = v.iter().copied().fold(f32::MIN, f32::max);
        (lo + hi) / 2.0
    };
    (mid(&xs), mid(&ys))
}
