//! Signal level for the NWD head-unit tuner — the scale, the map onto the face's
//! five-state bar glyph, and the reception-loss overlay that dots it.
//!
//! Ported by hand from CarFM's `src/services/nwdSignalLevel.ts` (886fc7e,
//! 2026-08-04, "Put a measured signal level on the face — under development") and
//! its test table in `tools/tests/signalLevel.test.mjs`. CarFM-original: the whole
//! head-unit level path is NWD tuner work and VibeSDR had none of it. There was no
//! Rust version of this in the CarFM tree, so unlike `rds.rs` this is a
//! transcription rather than a lift, and the tests below are the transcription's
//! only proof — they are the JavaScript table, row for row.
//!
//! ── Where the number comes from ──────────────────────────────────────────────
//! `NwdFmManager.seek(rawCurrentFrequency)` returns a packed int: strength in the
//! high 16 bits, the frequency it landed on in the low 16. Verified on device
//! 2026-08-04 over twenty presses — landed-ok every time, no audio interruption,
//! and the value tracks both station and position (105.9 read 54 in a Madison
//! driveway and 73 approaching Evansville; 101.5 fell 62 → ~50 leaving Madison).
//!
//! ── What the number MEANS is unresolved ──────────────────────────────────────
//! It is treated here as an ORDINAL, never as a unit. Printing "55 dB" would be
//! inventing one. The evidence does not distinguish dBµV (10 very weak, 31 a fair
//! local threshold, 73 a strong local) from an arbitrary chip register (0..100 or
//! 0..127). A different class in the same vendor service, `RadioConstant`, carries
//! RADIO_LOC_FM_STOP = -65 and RADIO_DX_FM_STOP = -70 — negative, dBm-shaped —
//! which proves the vendor codebase carries two unrelated scales. No chip part
//! number appears anywhere in the service APK, so there is no datasheet to reach.
//!
//! ── What a level CANNOT tell you ─────────────────────────────────────────────
//! WERN 88.7 read a healthy 53-55 while losing RDS fifteen times and flapping
//! stereo across one commute. That is not an error in the reading: multipath
//! leaves the carrier strong and destroys the 57 kHz subcarrier and the stereo
//! pilot. This meter shows three solid bars straight through it, which is exactly
//! what the dotted overlay exists for — and why the loss figure must actually be
//! wired rather than left at zero.

/// Weakest level a DX seek will stop on — `NewRdsManager.RADIO_FM_DX_STOP`. Below
/// this the tuner itself does not consider the frequency a station.
///
/// Nothing in the glyph reads it today; it is the other half of the vendor's own
/// pair and the reason 31 is known to be a threshold rather than a guess.
pub const LEVEL_DX_FLOOR: i32 = 10;

/// Weakest level a LOCAL seek will stop on — `NewRdsManager.RADIO_FM_LOC_STOP`.
/// The vendor's own weak/solid boundary, and the anchor that matters most.
pub const LEVEL_LOC_FLOOR: i32 = 31;

/// Top of the useful span.
///
/// Was 75, set from twenty manual presses whose highest was 73. The drive of
/// 2026-08-05 sampled 156 readings and reached 103, so 75 was badly short and
/// nearly a third of the drive would have pinned to four bars.
///
/// 105 comes from that distribution: min 30, p50 63, p75 79, p90 92, p99 101, max
/// 103. Still empirical rather than specified — no chip datasheet is reachable —
/// but it rests on a commute rather than on two car parks.
pub const LEVEL_TOP: i32 = 105;

/// The bands, from SIGNAL-METER.md v1.14. Deliberately NOT evenly spread: two of
/// the five steps fall in 48-70, where stereo lock and multipath actually change.
/// An even spread spends steps on high levels that all sound the same.
///
/// Replaces 31/45/60/74/89, which were an even division of the observed range.
/// These came from a mock-up instead — chosen for how the meter FEELS in use.
///
/// 31 stays the bottom either way: it is the vendor's own LEVEL_LOC_FLOOR, which
/// is what makes "nothing lit" mean "the tuner would not call this a station".
///
/// ONLY THE BOTTOM BAND IS EVIDENCE-BACKED. The other four are a design judgement
/// and are expected to move again.
const BANDS: [i32; 5] = [31, 48, 60, 70, 85];

/// Opacity of the half-step ring — the next pair up, lit once the level passes the
/// midpoint of the band it is in. Five states become nine without a sixth wave
/// being added.
///
/// Design measured this at 1.69:1 (light) / 2.83:1 (dark) against the 3:1 floor
/// for non-text graphics, and flagged that dotting starts on this ring — so the
/// first element carrying the loss signal is the least visible thing on the glyph.
/// Kept at 0.45 by decision: this display is a rough impression, not a
/// measurement, and 0.45 is what reads as "half".
///
/// DUPLICATED IN SLINT. `ui/icons.slint`'s `fade()` hard-codes 0.45 because a
/// Slint function cannot read a Rust constant. The test at the foot of this file
/// pins the number so a change here fails loudly rather than silently
/// desynchronising the glyph from its own model.
pub const HALF_STEP_OPACITY: f32 = 0.45;

/// Below this the reading counts as clean and nothing dots. Anchored on
/// measurement: audio rated clean ran ~16% loss and audio rated crackle ~44.5%, at
/// matched signal levels, and 30 is the midpoint between those populations.
pub const LOSS_DOT_FLOOR: f64 = 30.0;

/// Hysteresis for the loss figure, so a value sitting near a rounding boundary
/// cannot flip an arc between solid and dotted every poll.
///
/// ONE-DIRECTIONAL, and deliberately: it applies only on the way DOWN. Dotting
/// starts at the floor itself — 30% dots the leading arc, as specified — and only
/// clears once the figure has fallen this far below the boundary it crossed.
///
/// That asymmetry is the right one for a warning. Flicker is what the margin
/// exists to stop, and the falling side alone stops it: a figure oscillating
/// around a boundary latches on and holds. Applying it on the way up as well
/// bought no extra stability and cost 5 points of sensitivity — it silently moved
/// the documented 30% floor to 35%.
pub const LOSS_BAND_MARGIN: f64 = 5.0;

/// What the glyph draws for a level. Every field is a finished Slint property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalLit {
    /// Fully-lit arc pairs, 0-4, innermost first.
    pub full_pairs: i32,
    /// Is the NEXT pair up drawn at [`HALF_STEP_OPACITY`]? Never true at the top
    /// band — there is no fifth pair to light.
    pub half: bool,
    /// Centre dot opacity, 0-1. Zero only at level 0. `f32` because that is what a
    /// Slint `float` property takes; the ramp itself is computed in `f64`, as the
    /// JavaScript did.
    pub dot_opacity: f32,
}

/// JavaScript's `Math.round`, which breaks ties toward +∞ — Rust's `f64::round`
/// breaks them away from zero, so the two disagree on exactly the negative halves.
///
/// No negative value can reach either call site here (both are gated on a loss at
/// or above the floor), so this is a guard against a future one rather than a
/// live difference. It is written out because a silent disagreement in rounding is
/// the kind of thing that turns up as one arc, once, on a drive.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Level → what lights (SIGNAL-METER.md v1.14 "Strength → elements lit").
///
///   0        nothing at all
///   1-30     dot only, ramped ~36% → 85%
///   31-39    dot
///   40-47    dot + pair 1 @ 45%
///   48-53    + pair 1
///   54-59    + pair 2 @ 45%
///   60-64    + pair 2
///   65-69    + pair 3 @ 45%
///   70-77    + pair 3
///   78-84    + pair 4 @ 45%
///   85+      + pair 4
///
/// SUB-FLOOR DOT RAMP. Below 31 the dot is not dark; it eases up by
/// 0.25 + 0.6·√f where f is the fraction of the way from 0 to 31. Levels the glyph
/// used to render identically — 12 against 30 — are now told apart.
///
/// On THIS hardware it will rarely show: across the 156 samples of 2026-08-05 the
/// minimum was 30 and exactly one sample fell below 31. Built anyway, because it
/// costs nothing.
///
/// `None` means "no reading", which is not the same as a reading of zero: the
/// caller draws the empty glyph either way but prints an em-dash rather than a 0.
pub fn level_to_lit(level: Option<f64>) -> Option<SignalLit> {
    // `Number.isFinite` in the original — NaN and both infinities are refused, and
    // a NaN reaching the band walk would otherwise fall out as band 0.
    let level = level.filter(|v| v.is_finite())?;
    if level <= 0.0 {
        return Some(SignalLit {
            full_pairs: 0,
            half: false,
            dot_opacity: 0.0,
        });
    }
    if level < LEVEL_LOC_FLOOR as f64 {
        let f = level / LEVEL_LOC_FLOOR as f64;
        return Some(SignalLit {
            full_pairs: 0,
            half: false,
            dot_opacity: (0.25 + 0.6 * f.sqrt()) as f32,
        });
    }
    // Which band, and therefore how many pairs are FULLY lit.
    let mut band = 0usize;
    while band + 1 < BANDS.len() && level >= BANDS[band + 1] as f64 {
        band += 1;
    }
    // Past the midpoint of the band, the next ring up half-lights. The top band
    // has no next ring, so it never half-lights.
    let lo = BANDS[band];
    let hi = BANDS.get(band + 1).copied();
    let half = match hi {
        // Integer division, both operands positive — the JavaScript's
        // `Math.floor((lo + hi - 1) / 2)` exactly.
        Some(h) => level > ((lo + h - 1) / 2) as f64,
        None => false,
    };
    Some(SignalLit {
        full_pairs: band as i32,
        half,
        dot_opacity: 1.0,
    })
}

/// The five discrete steps, for paths with no unitless level: the Nearby list's
/// per-station strength and the settings preview.
///
/// SIGNAL-METER is explicit that these take none of the richer model — no
/// half-steps and no sub-floor ramp — because they have no level to place inside a
/// band. `steps` is 0-5: 0 lights nothing, 1 the dot, 2-5 the dot plus that many
/// pairs outward.
pub fn discrete_lit(steps: f64) -> SignalLit {
    let n = js_round(steps).clamp(0.0, 5.0) as i32;
    SignalLit {
        full_pairs: (n - 1).max(0),
        half: false,
        dot_opacity: if n >= 1 { 1.0 } else { 0.0 },
    }
}

/// How many arcs the glyph is drawing at all — the pool dotting works over.
///
/// DUPLICATED IN SLINT: `ui/icons.slint` re-derives this as
/// `full-pairs + (half && full-pairs < 4 ? 1 : 0)`. The two agree only because
/// `level_to_lit` never sets `half` in the top band and `discrete_lit` never sets
/// it at all — pinned by a test below, since a change to either side would
/// silently desynchronise the glyph from the number under it.
pub fn drawn_arcs(lit: Option<&SignalLit>) -> i32 {
    match lit {
        None => 0,
        Some(l) => l.full_pairs + i32::from(l.half),
    }
}

/// Reception loss → how many of the DRAWN arcs are dotted, spreading inward from
/// the leading arc (SIGNAL-METER.md v1.14 "Dotting").
///
/// The leading arc is the half-step ring when one is lit, otherwise the outermost
/// fully-lit pair — completely unlit pairs are not in the pool. Nothing dots below
/// 30%: a hard floor, not a rounding artefact. From 30 to 100 the dots spread
/// inward by round((loss-30)/70 × dottable), anchored at 1 so the control is never
/// dead at exactly 30.
///
/// NOTHING IS EXEMPT but the centre dot, which is a filled circle with no dotted
/// form. An earlier build clamped this so the innermost pair always stayed solid,
/// on the reasoning that "strong but lossy" should never collapse into reading as
/// weak; that clamp is gone by decision. At 100% every drawn arc dots.
///
/// Anchoring at 1 gives the control PRESENCE, not resolution: with a single arc
/// drawn, every loss from 30 to 100 renders identically.
pub fn loss_to_dotted_arcs(loss_pct: Option<f64>, dottable: i32) -> i32 {
    let Some(loss) = loss_pct.filter(|v| v.is_finite()) else {
        return 0;
    };
    if dottable <= 0 || loss < LOSS_DOT_FLOOR {
        return 0;
    }
    let spread = js_round(((loss - LOSS_DOT_FLOOR) / (100.0 - LOSS_DOT_FLOOR)) * dottable as f64);
    (spread as i32).clamp(1, dottable)
}

/// Apply the margin. `prev` is the count currently drawn, `loss_pct` the current
/// figure, `dottable` the number of arcs the glyph is drawing right now.
///
/// The margin decides HOW FAR to move, never WHETHER to move. An earlier version
/// tested whether the nudged figure landed on the SAME count, which quietly meant
/// "adjacent counts only": a figure jumping two counts in one poll matched nothing
/// and the glyph refused to move at all, so a WORSE figure drew FEWER dots. That
/// failed from every starting count, in both directions.
///
/// `dottable` is passed rather than captured because the pool changes with the
/// LEVEL, not with the loss — a level crossing a band boundary adds or removes an
/// arc under a perfectly steady loss figure. The result is clamped to the current
/// pool so a stale higher count cannot outlive the arcs it referred to.
///
/// Pure, and stateful only through `prev`: the caller settles this ONCE per poll
/// and feeds the answer back next time. Settling at render time instead would
/// re-run the hysteresis against itself.
pub fn settle_dotted_pairs(prev: i32, loss_pct: Option<f64>, dottable: i32) -> i32 {
    let next = loss_to_dotted_arcs(loss_pct, dottable);
    let held = prev.min(dottable); // the pool may have shrunk
    if next == held {
        return held;
    }
    let Some(loss) = loss_pct.filter(|v| v.is_finite()) else {
        return next; // no figure → solid, at once
    };
    if next > held {
        // Rising: adopt at once, so the leading arc dots at 30% and not at 30+margin.
        return next;
    }
    // Falling: the figure has to drop the full margin below the boundary before an
    // arc goes solid again. Clamped, so a multi-count fall still moves.
    let nudged = loss_to_dotted_arcs(Some(loss + LOSS_BAND_MARGIN), dottable);
    held.min(next.max(nudged))
}

/// A reading is only trustworthy when the tuner stayed on the frequency we asked
/// about — the same check the vendor's own seek path makes before it believes a
/// level. Units are the vendor's raw frequency integer: 10150 is 101.5 MHz.
///
/// A rejected read is the ABSENCE of a reading, not a zero. Eight of the ten
/// rejections on the drive logs came back `landed = 0` — the chip saying it was
/// not ready — and the other two landed on a genuinely different frequency
/// mid-tune.
pub fn level_is_trustworthy(asked: i32, landed: i32) -> bool {
    asked > 0 && asked == landed
}

/// The em-dash the meter prints when there is no reading. Its own constant because
/// `status-bar.slint` defaults to the same glyph and the two must not drift.
pub const NO_READING: &str = "—";

/// The level, printed inside the glyph.
///
/// NO UNIT IS EVER APPENDED. The scale is the vendor's and unidentified, so "dB"
/// would be a fabricated diagnostic. (CarFM appends a unit on one path only — its
/// RTL-SDR readout, which reports genuine dB. Carnyx has no SDR path: the whole
/// SDR engine is VibeSDR's and stays out of this tree.)
///
/// `audio_off` is audio priority released — the flat, dead state — where the face
/// prints the em-dash regardless of what the last reading was. The tuner edge also
/// passes `None` during a scan, since the level keeps arriving throughout a sweep
/// and belongs to whatever the dial swept past.
pub fn level_text(level: Option<i32>, audio_off: bool) -> String {
    match level {
        Some(v) if !audio_off => v.to_string(),
        _ => NO_READING.to_string(),
    }
}

/// Everything the signal cluster needs, finished. Slint decides none of it.
#[derive(Clone, Debug, PartialEq)]
pub struct MeterFace {
    pub full_pairs: i32,
    pub half: bool,
    pub dot_opacity: f32,
    pub dotted_arcs: i32,
    pub level_text: String,
}

/// The poll-time join: a reading and a loss figure in, the five face properties
/// out.
///
/// `prev_dotted` is the count currently on screen and `dotted_arcs` is what must
/// be fed back as the next call's `prev_dotted` — the hysteresis is the one piece
/// of state here, and it belongs to the caller so that a redraw cannot advance it.
///
/// `loss` is `100 - quality().pi_match_pct`, or `None` while the RDS carrier is
/// stale. It is a PROXY: block A is the only block this tuner lets anyone judge,
/// so errors in C and D — where the text lives — are invisible to it.
pub fn meter_face(
    level: Option<i32>,
    loss: Option<f64>,
    prev_dotted: i32,
    audio_off: bool,
) -> MeterFace {
    // Audio released: the empty glyph and no dots at all, matching CarFM's
    // `off ? discreteLit(0)` and `dottedArcs={off ? 0 : …}`.
    if audio_off {
        return MeterFace {
            full_pairs: 0,
            half: false,
            dot_opacity: 0.0,
            dotted_arcs: 0,
            level_text: NO_READING.to_string(),
        };
    }
    let lit = level_to_lit(level.map(f64::from)).unwrap_or_else(|| discrete_lit(0.0));
    let dottable = drawn_arcs(Some(&lit));
    MeterFace {
        full_pairs: lit.full_pairs,
        half: lit.half,
        dot_opacity: lit.dot_opacity,
        dotted_arcs: settle_dotted_pairs(prev_dotted, loss, dottable),
        level_text: level_text(level, audio_off),
    }
}

// ── The post-retune read schedule ───────────────────────────────────────────
//
// Timings only: the thread that obeys them lives at the framework edge and cannot
// be tested in this container. They are here because the ORDER between them is a
// property worth pinning, and the test at the foot of the file pins it.

/// Post-retune reads: show something fast, then correct it.
///
/// A reading taken immediately after tuning is systematically inflated. Across 24
/// paired comparisons on 2026-08-05 — first reading after a tune against the same
/// station 20 seconds later — the mean excess was +17.7, with cases of +45, +48
/// and +57, and the excess was positive on all five frequencies independently
/// (+7.9 to +25.3). The value is the return of `seek()`, the chip's own scan
/// primitive, so a read taken while the tune is still completing plausibly reports
/// that operation rather than the settled channel.
///
/// Normalising each reading against its OWN station's settled level puts the
/// excess almost entirely in the 0-1s bucket (+20.0 median, n=29); from 2s on
/// every band scatters around zero. So: read at 1s and show whatever comes back,
/// then read again at 4s and correct it. A brief wrong number beats a long dash.
pub const LEVEL_FIRST_READ_MS: u64 = 1_000;

/// The correction, and the point the periodic cadence is re-phased from.
pub const LEVEL_CORRECTION_MS: u64 = 4_000;

/// A rejected read is not a reading — it is the absence of one, and immediately
/// after a retune there is no previous reading to fall back on.
pub const LEVEL_RETRY_MS: u64 = 1_000;
pub const LEVEL_RETRY_MAX: u32 = 2;

/// How often to re-read while parked on a station. Each read COMMANDS the tuner,
/// so this stays well clear of the vendor's own rate limit on the comparable read
/// (900 ms, `ArmRadioManager.clearVaildFlag`), and the native watch floors its own
/// interval at 5s regardless.
pub const LEVEL_POLL_MS: u64 = 20_000;

#[cfg(test)]
mod tests {
    use super::*;

    // This is `tools/tests/signalLevel.test.mjs`, transcribed. Where a value below
    // is a log value it is verbatim from 2026-08-04 / 08-05; the band table is
    // transcribed from SIGNAL-METER.md v1.14, not derived from the logs.

    fn lit(n: f64) -> SignalLit {
        level_to_lit(Some(n)).expect("a finite level always lights something")
    }

    /// "0" or "1+half" — the shape the JavaScript table is written in.
    fn shown(n: f64) -> String {
        let l = lit(n);
        format!("{}{}", l.full_pairs, if l.half { "+half" } else { "" })
    }

    #[test]
    fn the_vendors_own_anchor_is_where_pairs_become_possible() {
        // Below the local-seek floor the tuner itself refuses to call the
        // frequency a station. The glyph does not go dark there — the dot RAMPS —
        // but the floor is still where the first arc pair becomes possible.
        let below = lit(LEVEL_LOC_FLOOR as f64 - 1.0);
        assert_eq!(below.full_pairs, 0);
        assert!(below.dot_opacity > 0.8, "...but the dot is not dark");
        assert_eq!(lit(LEVEL_LOC_FLOOR as f64).dot_opacity, 1.0);
    }

    /// SIGNAL-METER v1.14 "Strength → elements lit", row by row. THIS TABLE IS THE
    /// SPEC TABLE; if it disagrees, the code is wrong.
    #[test]
    fn the_band_table_is_the_spec_table() {
        const TABLE: [(f64, &str); 23] = [
            (0.0, "0"),
            (1.0, "0"),
            (30.0, "0"), // sub-floor: dot only, ramped
            (31.0, "0"),
            (39.0, "0"), // dot
            (40.0, "0+half"),
            (47.0, "0+half"), // dot + pair 1 @ 45%
            (48.0, "1"),
            (53.0, "1"), // + pair 1
            (54.0, "1+half"),
            (59.0, "1+half"), // + pair 2 @ 45%
            (60.0, "2"),
            (64.0, "2"), // + pair 2
            (65.0, "2+half"),
            (69.0, "2+half"), // + pair 3 @ 45%
            (70.0, "3"),
            (77.0, "3"), // + pair 3
            (78.0, "3+half"),
            (84.0, "3+half"), // + pair 4 @ 45%
            (85.0, "4"),
            (103.0, "4"),
            (999.0, "4"), // + pair 4
            (105.0, "4"),
        ];
        for (level, want) in TABLE {
            assert_eq!(shown(level), want, "level {level}");
        }
        assert!(
            !lit(999.0).half,
            "the top band never half-lights a fifth pair"
        );
    }

    /// 0.25 + 0.6·√f, f = level/31. Levels the glyph used to render identically —
    /// 12 against 30 — are now told apart.
    #[test]
    fn the_sub_floor_dot_ramps() {
        let pct = |n: f64| (lit(n).dot_opacity * 100.0).round() as i32;
        assert_eq!(lit(0.0).dot_opacity, 0.0, "level 0 draws nothing at all");
        assert_eq!(pct(1.0), 36, "level 1 is about 36%");
        assert_eq!(pct(30.0), 84, "level 30 is about 84%");
        assert_ne!(lit(12.0).dot_opacity, lit(30.0).dot_opacity);
        let ramp: Vec<f32> = [1.0, 5.0, 12.0, 20.0, 30.0]
            .iter()
            .map(|&v| lit(v).dot_opacity)
            .collect();
        assert!(
            ramp.windows(2).all(|w| w[1] > w[0]),
            "the ramp only rises: {ramp:?}"
        );
        assert!(
            lit(30.0).dot_opacity < 1.0,
            "and never reaches a full dot before the floor"
        );
    }

    #[test]
    fn a_level_that_is_not_a_number_lights_nothing_knowable() {
        assert_eq!(level_to_lit(None), None);
        assert_eq!(level_to_lit(Some(f64::NAN)), None);
        assert_eq!(level_to_lit(Some(f64::INFINITY)), None);
        assert_eq!(level_to_lit(Some(f64::NEG_INFINITY)), None);
        // A reading of zero is NOT the absence of one: it lights the empty glyph.
        assert_eq!(level_to_lit(Some(0.0)).map(|l| l.dot_opacity), Some(0.0));
    }

    /// The discrete path has no unitless level, so it takes no half-steps and no
    /// ramp — five states only.
    #[test]
    fn the_discrete_path_is_five_states_and_nothing_more() {
        assert_eq!(discrete_lit(0.0).full_pairs, 0);
        assert_eq!(discrete_lit(0.0).dot_opacity, 0.0);
        assert_eq!(discrete_lit(1.0).full_pairs, 0, "the dot alone");
        assert_eq!(discrete_lit(1.0).dot_opacity, 1.0);
        assert_eq!(discrete_lit(5.0).full_pairs, 4, "every pair");
        assert_eq!(discrete_lit(99.0).full_pairs, 4, "clamped above 5");
        assert_eq!(discrete_lit(-3.0).full_pairs, 0, "and below 0");
        assert_eq!(discrete_lit(-3.0).dot_opacity, 0.0);
        for n in 0..=5 {
            assert!(!discrete_lit(n as f64).half, "discrete never half-lights");
        }
        // Half-integers exist on this path — the Nearby list divides a range down
        // to five steps — so the tie-breaking direction is behaviour, not detail.
        assert_eq!(discrete_lit(2.5).full_pairs, 2, "ties round up, as JS does");
    }

    /// Dotting spreads inward from the LEADING drawn arc. The pool is what is
    /// DRAWN, half-step ring included. Nothing below 30%.
    #[test]
    fn dotting_spreads_inward_from_the_leading_arc() {
        assert_eq!(loss_to_dotted_arcs(Some(29.9), 4), 0, "below the floor");
        assert_eq!(loss_to_dotted_arcs(Some(30.0), 4), 1, "the floor itself");
        assert_eq!(loss_to_dotted_arcs(Some(100.0), 4), 4, "total loss");
        assert_eq!(loss_to_dotted_arcs(Some(100.0), 3), 3);
        assert_eq!(loss_to_dotted_arcs(Some(45.0), 4), 1, "the demo case");
        assert_eq!(loss_to_dotted_arcs(Some(80.0), 4), 3);
        assert_eq!(loss_to_dotted_arcs(Some(100.0), 0), 0, "nothing drawn");
        assert_eq!(loss_to_dotted_arcs(None, 4), 0, "no figure");
        assert_eq!(loss_to_dotted_arcs(Some(f64::NAN), 4), 0);
        // Presence, not resolution: one drawn arc renders every loss identically.
        let one: Vec<i32> = [30.0, 50.0, 70.0, 100.0]
            .iter()
            .map(|&v| loss_to_dotted_arcs(Some(v), 1))
            .collect();
        assert_eq!(one, vec![1, 1, 1, 1]);
    }

    /// One-directional: dotting starts at the floor itself and only clears once the
    /// figure has fallen a margin below the boundary it crossed.
    #[test]
    fn the_hysteresis_holds_only_on_the_way_down() {
        assert_eq!(settle_dotted_pairs(0, Some(30.0), 4), 1, "no margin up");
        assert_eq!(settle_dotted_pairs(0, Some(29.9), 4), 0);
        assert_eq!(settle_dotted_pairs(1, Some(80.0), 4), 3, "a clear rise");
        assert_eq!(settle_dotted_pairs(3, Some(0.0), 4), 0, "a clear fall");
        assert_eq!(settle_dotted_pairs(2, Some(60.0), 4), 2, "staying put");
        assert_eq!(
            settle_dotted_pairs(3, None, 4),
            0,
            "losing the figure goes solid immediately"
        );
        assert_eq!(
            settle_dotted_pairs(0, Some(100.0), 4),
            4,
            "a multi-count jump"
        );
        // The pool shrinks when the LEVEL drops a band. A count held over from a
        // bigger pool must not outlive the arcs it referred to.
        assert_eq!(settle_dotted_pairs(4, Some(100.0), 2), 2);
        assert_eq!(settle_dotted_pairs(3, Some(100.0), 0), 0);
    }

    /// More loss must NEVER draw fewer dots, from any starting count, at any pool
    /// size. This is the property the pre-7cf6ece shape violated in six places.
    #[test]
    fn settling_is_monotone_in_loss_at_every_pool_and_starting_count() {
        let mut reversals: Vec<String> = Vec::new();
        for dottable in 1..=4 {
            for prev in 0..=dottable {
                for x in 0..100 {
                    let a = settle_dotted_pairs(prev, Some(x as f64), dottable);
                    let b = settle_dotted_pairs(prev, Some(x as f64 + 1.0), dottable);
                    if b < a {
                        reversals.push(format!(
                            "pool {dottable}, showing {prev}: {x}%→{a} but {}%→{b}",
                            x + 1
                        ));
                    }
                }
            }
        }
        assert!(reversals.is_empty(), "{reversals:?}");
    }

    #[test]
    fn drawn_arcs_is_the_pool_the_dotting_works_over() {
        assert_eq!(
            drawn_arcs(level_to_lit(Some(31.0)).as_ref()),
            0,
            "a bare dot"
        );
        assert_eq!(
            drawn_arcs(level_to_lit(Some(40.0)).as_ref()),
            1,
            "a half ring"
        );
        assert_eq!(drawn_arcs(level_to_lit(Some(103.0)).as_ref()), 4);
        assert_eq!(drawn_arcs(None), 0);
        // The design bundle ships every reference screenshot at level 79 / loss 45.
        assert_eq!(drawn_arcs(level_to_lit(Some(79.0)).as_ref()), 4);
        assert!(lit(79.0).half, "...with the leading one a half-step");
        assert_eq!(
            loss_to_dotted_arcs(Some(45.0), drawn_arcs(level_to_lit(Some(79.0)).as_ref())),
            1,
            "...and it dots exactly that leading arc"
        );
    }

    /// THE GLYPH RE-DERIVES TWO OF THESE NUMBERS IN SLINT, and they agree only by
    /// construction. `ui/icons.slint` computes
    /// `drawn = full-pairs + (half && full-pairs < 4 ? 1 : 0)` and hard-codes 0.45
    /// as the half-step opacity. If a band ever lets `half` ride on four full
    /// pairs, the glyph would draw four arcs while this module dotted five.
    #[test]
    fn nothing_ever_half_lights_a_fifth_pair() {
        assert_eq!(
            HALF_STEP_OPACITY, 0.45,
            "icons.slint fade() hard-codes this"
        );
        for level in 0..=200 {
            let l = lit(level as f64);
            assert!(
                l.full_pairs < 4 || !l.half,
                "level {level} would desynchronise the Slint pool"
            );
            let slint_drawn = l.full_pairs + i32::from(l.half && l.full_pairs < 4);
            assert_eq!(slint_drawn, drawn_arcs(Some(&l)), "level {level}");
        }
        for steps in 0..=5 {
            let l = discrete_lit(steps as f64);
            assert!(!l.half);
            assert_eq!(l.full_pairs + i32::from(l.half), drawn_arcs(Some(&l)));
        }
    }

    /// Order is the invariant worth protecting: the first read has to land before
    /// the correction, and a retry has to fit between them or it is not a retry —
    /// it just races the correction it was meant to stand in for.
    ///
    /// `const` assertions, so a reordered schedule fails to COMPILE rather than
    /// failing a test run. The JavaScript could only check this at runtime.
    #[test]
    fn the_post_tune_schedule_stays_in_order() {
        const { assert!(LEVEL_FIRST_READ_MS < LEVEL_CORRECTION_MS) };
        const { assert!(LEVEL_FIRST_READ_MS + LEVEL_RETRY_MS < LEVEL_CORRECTION_MS) };
        const {
            assert!(
                LEVEL_FIRST_READ_MS + LEVEL_RETRY_MAX as u64 * LEVEL_RETRY_MS
                    <= LEVEL_CORRECTION_MS,
                "every retry fits, not just the first"
            )
        };
        // The native watch floors its cadence at 5s to protect the chip; a constant
        // under that would claim a rate it will not get.
        const { assert!(LEVEL_POLL_MS >= 5_000) };
        const { assert!(LEVEL_CORRECTION_MS < LEVEL_POLL_MS) };
    }

    #[test]
    fn a_reading_that_moved_is_not_a_reading() {
        assert!(level_is_trustworthy(10150, 10150), "101.5 stayed put");
        assert!(!level_is_trustworthy(10150, 10590));
        assert!(
            !level_is_trustworthy(0, 0),
            "a zero request is never trusted"
        );
        // The chip's own "not ready" answer.
        assert!(!level_is_trustworthy(10150, 0));
    }

    #[test]
    fn the_level_prints_bare_or_not_at_all() {
        assert_eq!(level_text(Some(63), false), "63");
        assert_eq!(level_text(Some(0), false), "0", "zero is a reading");
        assert_eq!(level_text(None, false), NO_READING);
        assert_eq!(level_text(Some(63), true), NO_READING, "audio released");
        // No unit is ever appended on this path — the scale is unidentified.
        assert!(!level_text(Some(63), false).contains("dB"));
    }

    /// The whole join, at the frame the design bundle ships every reference
    /// screenshot at: level 79, loss 45 — four arcs drawn, the leading one a
    /// half-step, exactly one of them dotted.
    #[test]
    fn the_reference_frame_lands_on_the_bundles_numbers() {
        let f = meter_face(Some(79), Some(45.0), 0, false);
        assert_eq!(f.full_pairs, 3);
        assert!(f.half);
        assert_eq!(f.dot_opacity, 1.0);
        assert_eq!(f.dotted_arcs, 1);
        assert_eq!(f.level_text, "79");
    }

    #[test]
    fn the_meter_goes_dead_when_audio_is_released() {
        let f = meter_face(Some(79), Some(45.0), 3, true);
        assert_eq!(f.full_pairs, 0);
        assert!(!f.half);
        assert_eq!(f.dot_opacity, 0.0);
        assert_eq!(f.dotted_arcs, 0, "and the loss overlay goes with it");
        assert_eq!(f.level_text, NO_READING);
    }

    #[test]
    fn no_reading_draws_the_empty_glyph_and_an_em_dash() {
        let f = meter_face(None, None, 2, false);
        assert_eq!(f.full_pairs, 0);
        assert_eq!(f.dot_opacity, 0.0);
        assert_eq!(f.dotted_arcs, 0);
        assert_eq!(f.level_text, NO_READING);
    }

    /// WERN 88.7 sat at 53-55 while losing RDS fifteen times across one commute.
    /// The meter shows the same solid bars either way — the dots are the ONLY
    /// difference between a clean weak signal and a strong shredded one.
    #[test]
    fn a_strong_carrier_arriving_in_pieces_is_still_dotted() {
        let clean = meter_face(Some(54), Some(16.0), 0, false);
        let shredded = meter_face(Some(54), Some(44.5), 0, false);
        assert_eq!(clean.full_pairs, shredded.full_pairs);
        assert_eq!(clean.half, shredded.half);
        assert_eq!(clean.dotted_arcs, 0, "audio rated clean ran ~16% loss");
        assert_eq!(shredded.dotted_arcs, 1, "audio rated crackle ~44.5%");
    }

    /// The settled count is the caller's state: feeding it back is what makes the
    /// margin one-directional rather than a fresh decision every poll.
    #[test]
    fn the_settled_count_is_carried_between_polls() {
        // Up to three dots at 80% loss…
        let a = meter_face(Some(79), Some(80.0), 0, false);
        assert_eq!(a.dotted_arcs, 3);
        // …and a figure that has fallen just under the 3-arc boundary holds there
        // rather than flickering, because the margin has not been cleared yet.
        let b = meter_face(Some(79), Some(72.0), a.dotted_arcs, false);
        assert_eq!(b.dotted_arcs, 3);
        // A real fall does move it.
        let c = meter_face(Some(79), Some(40.0), b.dotted_arcs, false);
        assert_eq!(c.dotted_arcs, 1);
    }
}
