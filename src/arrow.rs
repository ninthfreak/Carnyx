//! The maneuver arrows (§4.9), generated rather than drawn.
//!
//! ## One number, one stroke language
//!
//! §4.9: *"`turn_type` arrives as a TurnType XML string; each maps to one
//! number, degrees off straight ahead (`C` 0, `KL/KR` ±20, `TSLL/TSLR` ±45,
//! `TL/TR` ±90, `TSHL/TSHR` ±135, `TU/TRU` ±179) and a single generator draws
//! stem + elbow + barbs from it, so the whole set is one stroke language.
//! U-turns use an arc instead of an elbow; `RNDB`/`RNLB` get a roundabout glyph
//! with the exit number set inside the ring. Same 24-unit box and round caps as
//! every other icon (§7.1)."*
//!
//! So there are exactly three constructions here and not thirteen hand-drawn
//! paths: the ELBOW takes an angle, the ARC is the U-turn, and the RING is the
//! roundabout.
//!
//! ## What makes them a set: one height, one centre
//!
//! ── THE FIRST ATTEMPT AT THIS FILE GOT IT WRONG, AND IT IS WORTH SAYING HOW ──
//!
//! Fixing the tail, the elbow and the head length and varying only the angle
//! LOOKS like the way to make one language, and it is not. It leaves `C` 17
//! units long and `TL` a 10 × 10 mark adrift in the bottom-left corner of its
//! box: the same construction, but half the size and nowhere near the middle. In
//! a slot that swaps arrows as the route runs, that reads as the icon jumping.
//!
//! The rule that actually holds the set together is this one: **every glyph is
//! [`HEIGHT`] tall and centred in its box**, and the STEM absorbs the
//! difference. A right turn gets a long stem with a short head off the top; a
//! straight-on gets a short stem and all head. Both are 19.6 units tall, both
//! sit on the box's centre, and nothing moves when one replaces the other.
//! [`every_glyph_is_the_same_height_and_centred`](tests) is that rule as a test.
//!
//! ## Sign
//!
//! POSITIVE IS RIGHT, NEGATIVE IS LEFT, which is not an arbitrary pick: it is
//! OsmAnd's own. `RouteResultPreparation` sets `t.setTurnAngle((float) -mpi)`
//! where a POSITIVE `mpi` selects `TL`/`TSHL`/`TU` — so negating it puts right
//! on the positive side. Matching the source means the day `turn_angle` is
//! plumbed through (it is a populated `float` in the bundle; see [`ring`]) it
//! drops straight into [`degrees`]'s output with no sign fix.
//!
//! ## What is NOT here
//!
//! Colour, size and stroke width. §4.9 fixes the colour (amber, never a band
//! theme's palette) and the mini-handoff fixes the weights — *"stroke 2.1 at the
//! large sizes, 2.4 at 26–28dp"* — but both are the CALLER's, because one glyph
//! is drawn at three sizes in three places. This module answers only "what
//! shape", in a 24-unit box, and [`fits_the_box`](tests) proves every one of
//! them stays inside it at the heavier of the two strokes.

use crate::nav::Turn;

/// The icon box every glyph in this tree is drawn on (§7.1).
pub const BOX: f32 = 24.0;

/// How tall every glyph is, before the stroke.
///
/// `19.6 + 2.4` — the heavier stroke's full width — is 22, which leaves a unit
/// clear top and bottom inside the 24 box. See the module note for why this is
/// the constant that makes the set a set.
pub const HEIGHT: f32 = 19.6;

// ── The elbow arrow ──────────────────────────────────────────────────────────

/// Corner to tip. THE HEAD IS THE ONE FIXED LENGTH: it is the instruction, and
/// an instruction that changes size with the angle is two marks, not one.
///
/// MEASURED FROM WHERE THE CORNER ENDS, NOT FROM THE ELBOW, which is a
/// correction rather than a preference. Measuring from the elbow leaves
/// `HEAD_LEN - ELBOW_R` of head actually drawn, and at ±135° — where the head
/// doubles back alongside its own stem — 6.9 units of head against 4.8-unit
/// barbs is not an arrow, it is a pennant. `shots/arrows.png` is what said so.
const HEAD_LEN: f32 = 9.5;
/// The corner's radius, drawn as a quadratic through the elbow.
///
/// A ROUND JOIN IS NOT ENOUGH AT 90°. `stroke-line-join: round` rounds the
/// corner by half the stroke — 1.2 units at the heavy weight — which on a right
/// turn reads as a mitre with the point knocked off. The quadratic gives a real
/// radius, and at `C` its control point is collinear with both ends, so the
/// straight-ahead arrow comes out of the same expression as a straight line with
/// no special case.
const ELBOW_R: f32 = 2.6;
/// Each barb of the arrowhead.
const BARB_LEN: f32 = 4.8;
/// Half the arrowhead's opening.
const BARB_DEG: f32 = 32.0;

// ── The U-turn ───────────────────────────────────────────────────────────────

/// Half the loop's width. The approach and the exit straddle the centre line.
const U_R: f32 = 4.0;
/// How far the exit runs below the loop's centre line, before the head.
///
/// SHORTER THAN THE APPROACH ON PURPOSE. A U-turn drawn with two equal legs
/// reads as a staple; the driver has to see which end is the arrow, and the
/// short leg is the one that is leaving.
const U_EXIT: f32 = 8.6;

// ── The roundabout ───────────────────────────────────────────────────────────

const RING_R: f32 = 6.6;
/// How much of the ring is left open, in degrees, for the arrowhead to point
/// into. Measured back from 12 o'clock along the direction of travel, so the
/// gap sits in the upper quadrant and never near the entry stem at the bottom.
const GAP_DEG: f32 = 40.0;
/// The circulation arrowhead is smaller than a maneuver arrowhead, because it is
/// a direction-of-travel cue and not the instruction. The instruction is the
/// number set inside the ring.
const RING_BARB_LEN: f32 = 3.4;

/// Degrees off straight ahead, from §4.9's table.
///
/// THE TABLE IS THE SPEC'S AND NOT DERIVED FROM ANYTHING. OsmAnd's `turn_angle`
/// carries the route's real angle, but §4.9 fixes these thirteen values so the
/// glyph for a left turn is the same glyph every time — a 78° left and a 96°
/// left drawn faithfully would be two different icons for one instruction.
///
/// The roundabouts have no entry: their glyph is a ring, not a bent arrow, so
/// there is no angle to give. That is why this returns an `Option` rather than
/// answering `0.0` for them, which would be indistinguishable from `C`.
pub fn degrees(turn: Turn) -> Option<f32> {
    Some(match turn {
        Turn::Straight => 0.0,
        Turn::KeepLeft => -20.0,
        Turn::KeepRight => 20.0,
        Turn::SlightLeft => -45.0,
        Turn::SlightRight => 45.0,
        Turn::Left => -90.0,
        Turn::Right => 90.0,
        Turn::SharpLeft => -135.0,
        Turn::SharpRight => 135.0,
        Turn::UTurn => -179.0,
        Turn::RightUTurn => 179.0,
        Turn::Roundabout | Turn::RoundaboutLeft => return None,
    })
}

/// The SVG path for one maneuver, on a 24-unit box.
///
/// The exit NUMBER is not in here: it is text, and it is set inside the ring by
/// the caller drawing a `Text` over this `Path`. See [`Turn::from_xml`] for
/// where the number comes from.
pub fn path(turn: Turn) -> String {
    let cmds = match turn {
        // A U-TURN IS AN ARC, NOT A 179° ELBOW. The elbow construction would put
        // the tip 0.17 units to one side of the stem and the two would overlap
        // into an unreadable blot; the spec says arc for exactly that reason.
        Turn::UTurn => u_turn(-1.0),
        Turn::RightUTurn => u_turn(1.0),
        Turn::Roundabout => ring(-1.0),
        Turn::RoundaboutLeft => ring(1.0),
        other => elbow(degrees(other).expect("every non-roundabout turn has an angle")),
    };
    // CENTRED HERE AND NOWHERE ELSE, so the three constructions do not each have
    // to know where the middle of the box is.
    let (lo, hi) = bbox(&cmds);
    let dx = BOX / 2.0 - (lo.0 + hi.0) / 2.0;
    let dy = BOX / 2.0 - (lo.1 + hi.1) / 2.0;
    emit(&cmds, dx, dy)
}

/// Stem, radiused elbow, head, two barbs — the whole set bar three.
///
/// Built about the elbow at the local origin; [`path`] moves it into the box.
fn elbow(deg: f32) -> Vec<Cmd> {
    let (dx, dy) = heading(deg);
    let corner_in = (0.0, ELBOW_R);
    let corner_out = (dx * ELBOW_R, dy * ELBOW_R);
    let tip = (dx * (ELBOW_R + HEAD_LEN), dy * (ELBOW_R + HEAD_LEN));
    let (b1, b2) = barb_ends(tip, deg, BARB_LEN);

    // THE STEM IS WHAT MAKES EVERY GLYPH THE SAME HEIGHT. Everything above is
    // fixed by the angle; the tail is then placed so that tail − top = HEIGHT.
    // The corner's own apex counts: past 90° it is the highest ink there is.
    let mut top = tip.1.min(b1.1).min(b2.1).min(corner_in.1).min(corner_out.1);
    if let Some(apex) = quad_extreme(corner_in.1, 0.0, corner_out.1) {
        top = top.min(apex);
    }
    let tail = HEIGHT + top;

    vec![
        Cmd::Move(0.0, tail),
        Cmd::Line(corner_in.0, corner_in.1),
        Cmd::Quad(0.0, 0.0, corner_out.0, corner_out.1),
        Cmd::Line(tip.0, tip.1),
        Cmd::Move(b1.0, b1.1),
        Cmd::Line(tip.0, tip.1),
        Cmd::Line(b2.0, b2.1),
    ]
}

/// Approach, half-loop, exit, barbs. `side` is +1 for a right U-turn.
///
/// Built about the loop's centre line at the local origin.
fn u_turn(side: f32) -> Vec<Cmd> {
    // THE APPROACH IS ON THE OPPOSITE SIDE FROM THE EXIT, which is the one thing
    // to get right here: a driver making a LEFT U-turn comes up on the right of
    // the loop and leaves on the left.
    let approach_x = -side * U_R;
    let exit_x = side * U_R;
    // Sweep 1 is clockwise ON SCREEN, because y runs down. A right U-turn goes
    // up the left side and over the top to the right, which is clockwise.
    let sweep = i32::from(side > 0.0);
    let tip = (exit_x, U_EXIT);
    let (b1, b2) = barb_ends(tip, 180.0, BARB_LEN);
    // The loop's top is the highest ink; the tail then sets the height.
    let tail = HEIGHT - U_R;

    vec![
        Cmd::Move(approach_x, tail),
        Cmd::Line(approach_x, 0.0),
        // TWO QUARTERS RATHER THAN ONE HALF, so the loop's top is an ENDPOINT.
        // Every arc in this file turns a quarter between two axis extremes,
        // which is what lets `bbox` bound them from their endpoints alone.
        Cmd::Arc(U_R, sweep, 0.0, -U_R),
        Cmd::Arc(U_R, sweep, exit_x, 0.0),
        Cmd::Line(tip.0, tip.1),
        Cmd::Move(b1.0, b1.1),
        Cmd::Line(tip.0, tip.1),
        Cmd::Line(b2.0, b2.1),
    ]
}

/// Entry stem, ring, circulation arrowhead. `side` is +1 for left-hand traffic.
///
/// ── THE RING CLAIMS NO EXIT, AND THAT IS DELIBERATE ──────────────────────────
///
/// §4.9 asks for *"a roundabout glyph with the exit number set inside the ring"*
/// — the NUMBER is the instruction, and this draws no exit arm. `turn_angle` is
/// in the bundle (`bundle.putFloat(prefix + "turn_angle", tt.getTurnAngle())` in
/// `ExternalApiHelper.updateRouteDirectionInfo`), so the true exit bearing could
/// be plumbed and drawn one day. Until it is, an exit arm would have to point
/// somewhere chosen rather than known — and an arrow pointing at a road the
/// driver must not take is the same class of failure as drawing an unknown turn
/// type as "straight on".
///
/// What DOES differ between the two is the circulation direction, and it is the
/// only difference OsmAnd encodes: `RNDB` is right-hand traffic (counter-
/// clockwise on screen — you enter and bear right) and `RNLB` is left-hand
/// traffic, its mirror. That is what the arrowhead on the ring says.
///
/// ── AN OPEN RING, BECAUSE A CLOSED ONE HAS NOWHERE TO PUT THE ARROW ─────────
///
/// The first version drew a closed ring with the circulation arrowhead laid on
/// top of it at 3 o'clock. The head's barbs and the ring's stroke doubled up in
/// the same few units and the glyph read as a circle with a lump on its side —
/// `shots/arrows.png` again. So the ring STOPS where the head is, leaving a 40°
/// gap it points into: the same mark as a rotate icon, which is already the
/// universal "goes around this way".
///
/// Built about the ring's centre at the local origin.
fn ring(side: f32) -> Vec<Cmd> {
    // `m` mirrors the whole ring: right-hand traffic circulates counter-
    // clockwise on screen (sweep 0) and left-hand is its reflection.
    let m = -side;
    let sweep = i32::from(side > 0.0);
    let x = |v: f32| m * v;

    // ── THE HEAD SITS AT TWELVE O'CLOCK ─────────────────────────────────────
    //
    // Not at 3 o'clock, which is where it went first: there it shared its few
    // units with the ring's own stroke AND sat beside the exit number, and the
    // glyph read as a circle with a growth on its side. The top of the ring is
    // the one stretch with nothing else in it — the entry stem is at 6 o'clock
    // and the number is in the middle.
    let head = (0.0, -RING_R);
    // Counter-clockwise means travelling LEFT across the top of the ring.
    let travel = -90.0 * m;
    let (b1, b2) = barb_ends(head, travel, RING_BARB_LEN);

    // The gap's far lip, `GAP_DEG` short of the head — so the arrow points into
    // the opening rather than at its own tail. Not an axis extreme, but the
    // whole segment from there to the top stays inside one quadrant, so its
    // ends still bound it, which is what `bbox` needs.
    let lip_at = (270.0 - GAP_DEG).to_radians();
    let lip = (x(RING_R * lip_at.cos()), RING_R * lip_at.sin());

    // The barbs reach past the ring, so the top of the ink is one of them.
    let top = (-RING_R).min(b1.1).min(b2.1);
    let tail = HEIGHT + top;

    vec![
        Cmd::Move(0.0, tail),
        Cmd::Line(0.0, RING_R),
        // Split at every axis extreme it crosses, so no arc turns past one of
        // its own endpoints and `bbox` can bound them all from the ends.
        Cmd::Move(lip.0, lip.1),
        Cmd::Arc(RING_R, sweep, x(-RING_R), 0.0),
        Cmd::Arc(RING_R, sweep, 0.0, RING_R),
        Cmd::Arc(RING_R, sweep, x(RING_R), 0.0),
        Cmd::Arc(RING_R, sweep, head.0, head.1),
        Cmd::Move(b1.0, b1.1),
        Cmd::Line(head.0, head.1),
        Cmd::Line(b2.0, b2.1),
    ]
}

/// The two barb ends behind `tip`, opening `2 × BARB_DEG` about the direction of
/// TRAVEL. `deg` is where the arrow is going, not where the barbs point.
fn barb_ends(tip: (f32, f32), deg: f32, len: f32) -> ((f32, f32), (f32, f32)) {
    let back = deg + 180.0;
    let (lx, ly) = heading(back - BARB_DEG);
    let (rx, ry) = heading(back + BARB_DEG);
    ((tip.0 + lx * len, tip.1 + ly * len), (tip.0 + rx * len, tip.1 + ry * len))
}

/// A unit vector `deg` off straight ahead, in SCREEN coordinates.
///
/// Straight ahead is UP, which is `-y` — so this is not the textbook
/// `(cos, sin)`. Getting it wrong draws every left turn as a right one.
fn heading(deg: f32) -> (f32, f32) {
    let r = deg.to_radians();
    (r.sin(), -r.cos())
}

/// Where a quadratic turns back on itself, on one axis, if it does inside the
/// segment.
///
/// `B(t) = (1-t)²a + 2t(1-t)b + t²c` and `B'(t) = 0` at `t = (a-b)/(a-2b+c)`. A
/// zero denominator is a monotonic curve, whose ends already bound it.
///
/// THIS IS NOT DECORATION. The elbow's corner bulges past both of its endpoints
/// past 90°, and it is the highest ink in a sharp turn — bounding the curve by
/// its endpoints alone would make `TSHL` and `TSHR` half a unit taller than the
/// rest of the set.
fn quad_extreme(a: f32, b: f32, c: f32) -> Option<f32> {
    let denom = a - 2.0 * b + c;
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (a - b) / denom;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    let u = 1.0 - t;
    Some(u * u * a + 2.0 * t * u * b + t * t * c)
}

/// One command in a glyph, in local coordinates.
enum Cmd {
    Move(f32, f32),
    Line(f32, f32),
    /// Control point, then end point.
    Quad(f32, f32, f32, f32),
    /// Radius, sweep flag, end point. Always a quarter turn — see [`u_turn`].
    Arc(f32, i32, f32, f32),
}

/// The ink's bounding box, exactly.
///
/// Arcs are bounded by their endpoints because every one of them is a quarter
/// turn between two axis extremes; quadratics get [`quad_extreme`] on both axes.
fn bbox(cmds: &[Cmd]) -> ((f32, f32), (f32, f32)) {
    let mut lo = (f32::MAX, f32::MAX);
    let mut hi = (f32::MIN, f32::MIN);
    let mut at = (0.0, 0.0);
    let see = |p: (f32, f32), lo: &mut (f32, f32), hi: &mut (f32, f32)| {
        lo.0 = lo.0.min(p.0);
        lo.1 = lo.1.min(p.1);
        hi.0 = hi.0.max(p.0);
        hi.1 = hi.1.max(p.1);
    };
    for c in cmds {
        match *c {
            Cmd::Move(x, y) | Cmd::Line(x, y) | Cmd::Arc(_, _, x, y) => {
                see((x, y), &mut lo, &mut hi);
                at = (x, y);
            }
            Cmd::Quad(cx, cy, x, y) => {
                see((x, y), &mut lo, &mut hi);
                if let Some(v) = quad_extreme(at.0, cx, x) {
                    see((v, at.1), &mut lo, &mut hi);
                }
                if let Some(v) = quad_extreme(at.1, cy, y) {
                    see((at.0, v), &mut lo, &mut hi);
                }
                at = (x, y);
            }
        }
    }
    (lo, hi)
}

/// The SVG path data, translated into the box.
fn emit(cmds: &[Cmd], dx: f32, dy: f32) -> String {
    let mut out = String::new();
    for c in cmds {
        if !out.is_empty() {
            out.push(' ');
        }
        match *c {
            Cmd::Move(x, y) => out.push_str(&format!("M {} {}", n(x + dx), n(y + dy))),
            Cmd::Line(x, y) => out.push_str(&format!("L {} {}", n(x + dx), n(y + dy))),
            Cmd::Quad(cx, cy, x, y) => out.push_str(&format!(
                "Q {} {} {} {}",
                n(cx + dx),
                n(cy + dy),
                n(x + dx),
                n(y + dy)
            )),
            Cmd::Arc(r, sweep, x, y) => out.push_str(&format!(
                "A {} {} 0 0 {} {} {}",
                n(r),
                n(r),
                sweep,
                n(x + dx),
                n(y + dy)
            )),
        }
    }
    out
}

/// Two decimals, with the noise trimmed off.
///
/// The path string is compared in tests and read in diffs, so `21.5` beats
/// `21.50` and `12` beats `12.00`. `-0` is folded to `0`, which `f32` will
/// otherwise produce for a left-hand angle's cosine.
fn n(v: f32) -> String {
    let mut s = format!("{v:.2}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EVERY VALUE IN §4.9'S TABLE, AGAINST THE SENTENCE THAT STATES IT.
    ///
    /// A table is the kind of thing that is transcribed with one sign flipped,
    /// and the failure here would be a driver sent left at a right turn.
    #[test]
    fn the_angles_are_the_specs_own() {
        let expected = [
            (Turn::Straight, 0.0),
            (Turn::KeepLeft, -20.0),
            (Turn::KeepRight, 20.0),
            (Turn::SlightLeft, -45.0),
            (Turn::SlightRight, 45.0),
            (Turn::Left, -90.0),
            (Turn::Right, 90.0),
            (Turn::SharpLeft, -135.0),
            (Turn::SharpRight, 135.0),
            (Turn::UTurn, -179.0),
            (Turn::RightUTurn, 179.0),
        ];
        for (turn, deg) in expected {
            assert_eq!(degrees(turn), Some(deg), "{turn:?}");
        }
        // The roundabouts have no angle, and `0.0` would read as "straight on".
        assert_eq!(degrees(Turn::Roundabout), None);
        assert_eq!(degrees(Turn::RoundaboutLeft), None);
    }

    /// LEFT IS LEFT. The one test that would catch a sign flip in `heading`.
    #[test]
    fn left_turns_go_left_and_right_turns_go_right() {
        // Straight ahead is UP the page, which is decreasing y.
        let (dx, dy) = heading(0.0);
        assert!(dy < -0.99 && dx.abs() < 0.01, "straight ahead is up: {dx},{dy}");
        assert!(heading(90.0).0 > 0.99, "90 degrees is to the right");
        assert!(heading(-90.0).0 < -0.99, "-90 degrees is to the left");

        // And it survives the whole way to the path string: a left turn's tip is
        // left of its own stem, and a right turn's is right of it.
        for turn in [Turn::Left, Turn::SlightLeft, Turn::KeepLeft, Turn::SharpLeft] {
            let (tip, stem) = (tip_of(turn), stem_of(turn));
            assert!(tip.0 < stem, "{turn:?} tip {tip:?} must be left of its stem at {stem}");
        }
        for turn in [Turn::Right, Turn::SlightRight, Turn::KeepRight, Turn::SharpRight] {
            let (tip, stem) = (tip_of(turn), stem_of(turn));
            assert!(tip.0 > stem, "{turn:?} tip {tip:?} must be right of its stem at {stem}");
        }
        // Straight on does not lean at all.
        assert!((tip_of(Turn::Straight).0 - stem_of(Turn::Straight)).abs() < 0.01);

        // A keep-left leans less than a slight left, which leans less than a
        // left — the set has to be ORDERED or it is thirteen icons, not one
        // language.
        let lean = |t: Turn| stem_of(t) - tip_of(t).0;
        assert!(lean(Turn::KeepLeft) < lean(Turn::SlightLeft), "KL must lean less than TSLL");
        assert!(lean(Turn::SlightLeft) < lean(Turn::Left), "TSLL must lean less than TL");
        // And a sharp left is further round than a left, which past 90 degrees
        // means further DOWN the page rather than further across it.
        assert!(tip_of(Turn::SharpLeft).1 > tip_of(Turn::Left).1, "TSHL must come back down");
        assert!(tip_of(Turn::Left).1 > tip_of(Turn::SlightLeft).1, "TL comes down past TSLL");
    }

    /// STRAIGHT AHEAD IS ONE LINE, with no kink at the elbow.
    ///
    /// `C` runs through the same quadratic as every other angle. If the control
    /// point ever stops being collinear with its ends, the "straight on" arrow
    /// grows a bend — so this checks that the whole shaft shares one x, rather
    /// than pinning a literal string that every tuned constant would invalidate.
    #[test]
    fn the_straight_arrow_has_no_elbow_in_it() {
        let d = path(Turn::Straight);
        // Everything up to the arrowhead — the tail, the corner and the tip —
        // stands on the box's centre line.
        let shaft = d.split(" M ").next().expect("a shaft before the barbs");
        for (x, _) in points(shaft) {
            assert!((x - 12.0).abs() < 0.01, "the straight arrow bends at x={x}: {d}");
        }
        // And it is a shaft, not a stub: tail to tip is the full height.
        let (lo, hi) = extent(Turn::Straight);
        assert!((hi.1 - lo.1 - HEIGHT).abs() < 0.01, "{d}");
    }

    /// A U-TURN IS AN ARC AND A ROUNDABOUT IS A RING — the two constructions the
    /// spec calls out as NOT elbows.
    #[test]
    fn the_two_special_constructions_are_not_elbows() {
        for turn in [Turn::UTurn, Turn::RightUTurn, Turn::Roundabout, Turn::RoundaboutLeft] {
            let d = path(turn);
            assert!(d.contains(" A "), "{turn:?} must be drawn with arcs: {d}");
            assert!(!d.contains(" Q "), "{turn:?} must not use the elbow: {d}");
        }
        for turn in [Turn::Straight, Turn::KeepRight, Turn::Left, Turn::SharpRight] {
            assert!(path(turn).contains(" Q "), "{turn:?} must use the elbow");
        }
    }

    /// THE TWO U-TURNS ARE MIRRORS, AND THEY TURN OPPOSITE WAYS.
    ///
    /// The arc's sweep flag is one character and flipping it draws a left U-turn
    /// that loops to the right — a mistake no test on the tip position would
    /// catch, because both tips end up where they should be either way.
    #[test]
    fn the_u_turns_loop_opposite_ways() {
        let left = path(Turn::UTurn);
        let right = path(Turn::RightUTurn);
        assert!(left.contains("A 4 4 0 0 0 "), "a left U-turn sweeps counter-clockwise: {left}");
        assert!(right.contains("A 4 4 0 0 1 "), "a right U-turn sweeps clockwise: {right}");
        assert!(!left.contains("A 4 4 0 0 1 "), "and not the other way too: {left}");
        assert!(!right.contains("A 4 4 0 0 0 "), "and not the other way too: {right}");

        // A left U-turn comes up on the RIGHT of the loop and leaves on the left.
        assert!(stem_of(Turn::UTurn) > 12.0, "a left U-turn approaches on the right");
        assert!(stem_of(Turn::RightUTurn) < 12.0, "a right U-turn approaches on the left");
        assert!(tip_of(Turn::UTurn).0 < stem_of(Turn::UTurn), "and leaves on the other side");
        assert!(tip_of(Turn::RightUTurn).0 > stem_of(Turn::RightUTurn));
        // Both end pointing back DOWN the page — that is what a U-turn is. The
        // tip must be below the loop's top by most of the glyph's height.
        for turn in [Turn::UTurn, Turn::RightUTurn] {
            let pts = points(&path(turn));
            let top = pts.iter().fold(f32::MAX, |a, p| a.min(p.1));
            assert!(tip_of(turn).1 - top > HEIGHT / 2.0, "{turn:?} must double back");
        }
        // The two are exact mirrors of each other about the box's centre line.
        assert_eq!(mirrored(Turn::UTurn), sorted(points(&path(Turn::RightUTurn))));
    }

    /// THE ROUNDABOUTS ARE MIRRORS AND NEITHER OF THEM POINTS AT AN EXIT.
    #[test]
    fn the_roundabouts_mirror_and_claim_no_exit() {
        let rndb = path(Turn::Roundabout);
        let rnlb = path(Turn::RoundaboutLeft);
        assert_ne!(rndb, rnlb, "the two traffic sides must not draw the same glyph");
        // RIGHT-HAND TRAFFIC CIRCULATES COUNTER-CLOCKWISE, which across the top
        // of the ring — where the head is — means travelling LEFT. Its barbs
        // therefore trail to the RIGHT of the head, and left-hand traffic is the
        // mirror. This is the assertion that would catch a flipped sweep flag:
        // the head's POSITION is the same either way, only its aim differs.
        assert!(head_trails_right(Turn::Roundabout), "RNDB must circulate counter-clockwise");
        assert!(!head_trails_right(Turn::RoundaboutLeft), "RNLB must circulate clockwise");
        // And the arc is drawn the way the head points, not against it.
        assert!(rndb.contains(" 0 0 0 "), "RNDB sweeps counter-clockwise: {rndb}");
        assert!(!rndb.contains(" 0 0 1 "), "and never the other way: {rndb}");
        assert!(rnlb.contains(" 0 0 1 "), "RNLB sweeps clockwise: {rnlb}");
        assert!(!rnlb.contains(" 0 0 0 "), "and never the other way: {rnlb}");
        assert_eq!(mirrored(Turn::Roundabout), sorted(points(&path(Turn::RoundaboutLeft))));

        // FOUR ARCS: the ring is split at every axis extreme it crosses, which
        // is what lets the bounds be read off the endpoints.
        for d in [&rndb, &rnlb] {
            assert_eq!(d.matches(" A ").count(), 4, "the ring is four arcs: {d}");
        }
        // THE RING IS OPEN, or the arrowhead has nothing to point into: the arc
        // starts a gap short of where it ends.
        for turn in [Turn::Roundabout, Turn::RoundaboutLeft] {
            let pts = points(&path(turn));
            // The arc's start (the third point: tail, ring join, then the lip)
            // must not coincide with the head it ends at.
            let (lip, head) = (pts[2], tip_of(turn));
            let apart = ((lip.0 - head.0).powi(2) + (lip.1 - head.1).powi(2)).sqrt();
            assert!(apart > 3.0, "{turn:?} closes the ring: lip {lip:?} head {head:?}");
        }
    }

    /// EVERY GLYPH IS THE SAME HEIGHT AND SITS ON THE BOX'S CENTRE.
    ///
    /// THIS IS THE RULE THE FIRST VERSION OF THIS FILE BROKE — see the module
    /// note. Without it the construction still "works": every arrow points the
    /// right way, and `C` is 17 units long while `TL` is a 10 × 10 mark in the
    /// bottom-left corner. In a slot that swaps arrows as the route runs, that
    /// is the icon jumping every time the turn changes.
    #[test]
    fn every_glyph_is_the_same_height_and_centred() {
        for turn in every_turn() {
            let (lo, hi) = extent(turn);
            let h = hi.1 - lo.1;
            assert!((h - HEIGHT).abs() < 0.01, "{turn:?} is {h} tall, not {HEIGHT}");
            let (cx, cy) = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
            assert!((cx - 12.0).abs() < 0.01, "{turn:?} sits off-centre at x={cx}");
            assert!((cy - 12.0).abs() < 0.01, "{turn:?} sits off-centre at y={cy}");
            // And none of them is a sliver: an arrow read at speed has to have
            // width as well as height.
            assert!(hi.0 - lo.0 >= 4.5, "{turn:?} is too narrow: {}", hi.0 - lo.0);
        }
    }

    /// EVERY GLYPH STAYS INSIDE THE 24-UNIT BOX, AT THE HEAVIER STROKE.
    ///
    /// §7.1 fixes the box and the mini-handoff fixes the weights (*"stroke 2.1
    /// at the large sizes, 2.4 at 26–28dp"*). A round cap adds half the stroke
    /// beyond every endpoint in every direction, so the check is the ink grown
    /// by 1.2.
    #[test]
    fn fits_the_box() {
        const CAP: f32 = 2.4 / 2.0;
        for turn in every_turn() {
            let (lo, hi) = extent(turn);
            assert!(lo.0 - CAP >= 0.0, "{turn:?} clips the left edge at x={}", lo.0);
            assert!(hi.0 + CAP <= BOX, "{turn:?} clips the right edge at x={}", hi.0);
            assert!(lo.1 - CAP >= 0.0, "{turn:?} clips the top at y={}", lo.1);
            assert!(hi.1 + CAP <= BOX, "{turn:?} clips the bottom at y={}", hi.1);
        }
    }

    /// THE OUTPUT IS A PATH SLINT CAN PARSE, not just a string that looks like
    /// one. `Path.commands` takes SVG path data; a stray token draws NOTHING and
    /// fails silently, which on this face means a maneuver with no arrow rather
    /// than a crash.
    #[test]
    fn the_commands_are_well_formed_svg() {
        for turn in every_turn() {
            let d = path(turn);
            assert!(d.starts_with("M "), "{turn:?}: a path must open with a moveto: {d}");
            let mut expect: usize = 0;
            for tok in d.split_whitespace() {
                match tok {
                    "M" | "L" => expect = 2,
                    "Q" => expect = 4,
                    "A" => expect = 7,
                    num => {
                        assert!(expect > 0, "{turn:?}: unexpected token {num} in {d}");
                        assert!(num.parse::<f32>().is_ok(), "{turn:?}: {num} is not a number");
                        expect -= 1;
                    }
                }
            }
            assert_eq!(expect, 0, "{turn:?}: a command is short of arguments: {d}");
        }
    }

    /// The ink's bounds, read back out of the EMITTED STRING rather than from
    /// the generator's own `bbox` — so a bug in `emit` cannot hide behind a
    /// correct `bbox`.
    fn extent(turn: Turn) -> ((f32, f32), (f32, f32)) {
        let pts = points(&path(turn));
        let mut lo = (f32::MAX, f32::MAX);
        let mut hi = (f32::MIN, f32::MIN);
        for (x, y) in pts {
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        (lo, hi)
    }

    /// Every point the ink reaches, parsed out of a path string.
    ///
    /// Quadratics contribute their apex as well as their end, by the same
    /// formula the generator uses — a curve that bulges past its ends is ink
    /// too, and past 90° the elbow's corner is the topmost ink there is.
    fn points(d: &str) -> Vec<(f32, f32)> {
        let toks: Vec<&str> = d.split_whitespace().collect();
        let num = |i: usize| toks[i].parse::<f32>().expect("a number");
        let mut out: Vec<(f32, f32)> = Vec::new();
        let mut at = (0.0, 0.0);
        let mut i = 0;
        while i < toks.len() {
            match toks[i] {
                "M" | "L" => {
                    at = (num(i + 1), num(i + 2));
                    out.push(at);
                    i += 3;
                }
                "Q" => {
                    let (cx, cy) = (num(i + 1), num(i + 2));
                    let end = (num(i + 3), num(i + 4));
                    out.push(end);
                    if let Some(v) = quad_extreme(at.0, cx, end.0) {
                        out.push((v, at.1));
                    }
                    if let Some(v) = quad_extreme(at.1, cy, end.1) {
                        out.push((at.0, v));
                    }
                    at = end;
                    i += 5;
                }
                "A" => {
                    at = (num(i + 6), num(i + 7));
                    out.push(at);
                    i += 8;
                }
                _ => i += 1,
            }
        }
        out
    }

    /// The arrow's tip — the middle of the three-point arrowhead, which every
    /// construction emits last.
    fn tip_of(turn: Turn) -> (f32, f32) {
        let pts = points(&path(turn));
        pts[pts.len() - 2]
    }

    /// Where the glyph's stem stands, which is its FIRST point: every
    /// construction starts at the tail.
    fn stem_of(turn: Turn) -> f32 {
        points(&path(turn))[0].0
    }

    /// Do the arrowhead's barbs trail to the right of its tip?
    ///
    /// Barbs trail BEHIND the direction of travel, so this reads which way a
    /// head is aimed without depending on where it happens to sit. The
    /// arrowhead is the last three points every construction emits.
    fn head_trails_right(turn: Turn) -> bool {
        let pts = points(&path(turn));
        let (b1, tip, b2) = (pts[pts.len() - 3], tip_of(turn), pts[pts.len() - 1]);
        assert!(
            (b1.0 - tip.0).signum() == (b2.0 - tip.0).signum(),
            "{turn:?}: the barbs straddle the tip, so which way it aims is not a side"
        );
        b1.0 > tip.0
    }

    /// One glyph's points reflected about the box's centre line, for comparing
    /// a left-hand construction against its right-hand twin.
    ///
    /// SORTED, BECAUSE REFLECTION REORDERS. The arrowhead is drawn
    /// `barb → tip → barb`, so mirroring swaps which barb comes first — the two
    /// glyphs are the same SHAPE without being the same SEQUENCE, and shape is
    /// what "mirror" means here.
    fn mirrored(turn: Turn) -> Vec<(f32, f32)> {
        sorted(points(&path(turn)).into_iter().map(|(x, y)| (round2(BOX - x), y)).collect())
    }

    /// Sorted AND de-duplicated, because a point can be visited twice: the ring
    /// closes on the extreme it opened at, and the circulation head lands on
    /// another. Which of those coincides is not itself mirrored, so counting
    /// visits would call two identical shapes different.
    fn sorted(mut pts: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
        pts.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a path"));
        pts.dedup();
        pts
    }

    /// Two decimals, to compare a reflected coordinate against one that was
    /// printed at two decimals and parsed back.
    fn round2(v: f32) -> f32 {
        (v * 100.0).round() / 100.0
    }

    fn every_turn() -> [Turn; 13] {
        [
            Turn::Straight,
            Turn::KeepLeft,
            Turn::KeepRight,
            Turn::SlightLeft,
            Turn::SlightRight,
            Turn::Left,
            Turn::Right,
            Turn::SharpLeft,
            Turn::SharpRight,
            Turn::UTurn,
            Turn::RightUTurn,
            Turn::Roundabout,
            Turn::RoundaboutLeft,
        ]
    }

    /// The helpers above really do find what they claim, checked against a shape
    /// whose answer is known by hand — otherwise every test here is measuring
    /// the helper rather than the generator.
    #[test]
    fn the_helpers_find_what_they_claim() {
        // A right turn's head runs from its stem across by the corner's radius
        // and then a full head length — see `HEAD_LEN` for why the head is
        // measured from where the corner ends rather than from the elbow.
        let (tip, stem) = (tip_of(Turn::Right), stem_of(Turn::Right));
        assert!((tip.0 - stem - ELBOW_R - HEAD_LEN).abs() < 0.01, "tip {tip:?} stem {stem}");
        // Straight on: the tip is a head length above the elbow, and the whole
        // glyph is one column.
        assert!((tip_of(Turn::Straight).0 - 12.0).abs() < 0.01);
        // And a quadratic's apex is found where one exists and not where it does
        // not: a symmetric arch peaks at its middle, a monotone run has none.
        assert_eq!(quad_extreme(0.0, 2.0, 0.0), Some(1.0));
        assert_eq!(quad_extreme(0.0, 1.0, 2.0), None, "a straight run has no apex");
    }
}
