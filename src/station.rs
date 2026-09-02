//! Station identity: the small pieces of logic the face needs in order to draw a
//! station, ported from CarFM's `src/components/carfm/tokens.ts`.
//!
//! These live here rather than in the `.slint` files on purpose. Slint decides
//! nothing: it is handed a finished call sign, a finished colour and a finished
//! frequency string, and it draws them.

use slint::Color;

/// Stable brand-ish fills for a call-sign box, in the order CarFM lists them —
/// the hash below indexes this array, so the order is part of the behaviour.
const BRAND_BGS: [u32; 10] = [
    0x20655B, 0x2E5EAA, 0x1E1E22, 0xB02A6E, 0x3A7D44,
    0x6B4FA1, 0xB4541B, 0x265D73, 0x7D2E2E, 0x4A4E69,
];

/// Stable colour for a station's call-sign box, hashed from the call sign.
///
/// This is JavaScript's `((h << 5) - h + c) | 0` over UTF-16 code units, kept
/// bit-for-bit so a station keeps the colour it has always had. The hash is taken
/// as `i64` before `abs` because `i32::MIN.abs()` panics in Rust, while JS's
/// `Math.abs` widens to a float and yields 2147483648 — the two disagree on
/// exactly one input, and it is reachable.
pub fn brand_color(key: &str) -> Color {
    let mut h: i32 = 0;
    for unit in key.encode_utf16() {
        h = h.wrapping_shl(5).wrapping_sub(h).wrapping_add(unit as i32);
    }
    let idx = ((h as i64).abs() % BRAND_BGS.len() as i64) as usize;
    let rgb = BRAND_BGS[idx];
    Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Drop a trailing band suffix from a call sign: `WWHG-FM` → `WWHG`,
/// `WJJO FM` → `WJJO`. Applied everywhere a call sign is displayed.
///
/// The suffix must be SEPARATED and TRAILING. CarFM's earlier version matched
/// `(fm|am)` anywhere and amputated real call signs — `WHAM` → `WH`, `KJFM` →
/// `KJ` — which is why the separator test below is not optional.
pub fn clean_call(s: &str) -> String {
    let t = s.trim_end();
    let lower = t.to_ascii_lowercase();
    if lower.ends_with("fm") || lower.ends_with("am") {
        let stem = &t[..t.len() - 2];
        let trimmed = stem.trim_end_matches([' ', '\t', '\n', '\r', '-']);
        // Only a separated suffix counts: something must have been trimmed.
        if trimmed.len() < stem.len() {
            return trimmed.trim().to_string();
        }
    }
    t.trim().to_string()
}

// ── Plate ink (handoff v3.3.0 §5) ───────────────────────────────────────────

/// The two candidates a brand fill is scored against.
///
/// v3.3.0 §11 names them: near-black `#141821` and white. NOT the theme's `text`
/// and `bg` — a station's fill is its own and does not change with the theme, so
/// the ink that sits on it must not either. §11a's third screenshot is the check:
/// "plate inks unchanged by theme — they follow the station's fill".
const INK_DARK: u32 = 0x141821;
const INK_LIGHT: u32 = 0xFFFFFF;

/// WCAG relative luminance, with the sRGB linearisation §5 spells out.
///
/// The `0.03928` knee and the `2.4` exponent are the standard's, and the weights
/// are Rec.709's. NOT the Rec.709 luma used by `Pal.flat` next door: that one
/// desaturates a colour for the dead face and operates on the raw channels, this
/// one linearises first because contrast is a physical-light ratio. The two look
/// similar and answer different questions.
fn rel_lum(c: Color) -> f64 {
    let ch = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.red()) + 0.7152 * ch(c.green()) + 0.0722 * ch(c.blue())
}

/// The WCAG contrast ratio between two colours, 1.0 to 21.0.
///
/// Alpha is ignored: everything this scores is opaque ink on an opaque plate.
pub fn contrast(a: Color, b: Color) -> f64 {
    let (x, y) = (rel_lum(a), rel_lum(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// Whichever of the two candidates actually scores higher on `bg`.
///
/// ── SCORED, NEVER THRESHOLDED, AND §5 IS EMPHATIC ABOUT WHY ─────────────────
///
/// The obvious implementation is "light fill gets dark ink" against a luminance
/// cutoff, and a cutoff always lands arbitrarily close to some real fill: §5
/// gives WZEE's cyan at luminance 0.3095, which a 0.32 threshold misses by 0.01
/// and inks white at 2.92:1. Comparing the two ratios has no such edge — it is
/// exactly as right at 0.3095 as anywhere else.
pub fn ink_on(bg: Color) -> Color {
    let dark = Color::from_rgb_u8((INK_DARK >> 16) as u8, (INK_DARK >> 8) as u8, INK_DARK as u8);
    let light = Color::from_rgb_u8((INK_LIGHT >> 16) as u8, (INK_LIGHT >> 8) as u8, INK_LIGHT as u8);
    if contrast(bg, dark) >= contrast(bg, light) {
        dark
    } else {
        light
    }
}

/// The ink for a station plate: `bg` is the station's fill, or [`None`] when it
/// has none and the plate is the theme's `raised`.
///
/// ── THE `None` ARM IS NOT REACHABLE FROM TODAY'S PRESET PATH ────────────────
///
/// §5 describes it as reachable through the app's own save flow, and in the
/// prototype it is: there, a station saved from an unknown frequency has no
/// `logoBg` and lands on `raised` with white ink at 1.08:1. HERE IT CANNOT
/// HAPPEN, and that was checked rather than assumed — `app::to_preset` fills
/// every preset with `brand_color(&call)`, and that function is total: it hashes
/// into ten fixed values and answers for the empty string as readily as for
/// `WMGN`. There is no station in this tree without a fill.
///
/// The arm stays because it is §5's rule and because the function should be
/// total over what it is handed, not because a caller needs it today. What would
/// make it live is a plate drawn on `Pal.raised` — an empty slot growing a label,
/// or a fill made optional so an authored one can be absent.
///
/// NO AUTHORED-INK BRANCH, and that is a difference from §5 rather than an
/// omission of it. The spec honours a station's `logoFg` while it clears 4.5:1
/// and overrides it otherwise — but nothing in this tree has a `logoFg` to
/// honour. Fills come from [`brand_color`], a hash into ten fixed values, and no
/// station record carries an authored ink. If one ever does, the branch goes
/// here: `if let Some(fg) = fg { if contrast(bg, fg) >= 4.5 { return fg } }`
/// before the call below, and nowhere else.
pub fn plate_ink(bg: Option<Color>, theme_text: Color) -> Color {
    match bg {
        None => theme_text,
        Some(bg) => ink_on(bg),
    }
}

/// The label a preset tile or peek card prints inside its colour box.
///
/// `base` is the call sign resolved from the station database, when one resolved.
/// It is printed IN FULL — four letters is the common case but not the rule: of
/// the 20,733 stations in CarFM's shipped database, 8,277 are FM translators with
/// six-character call signs (`K227EA`, `W249BC`), plus a handful of seven. The
/// box's font size divides by the label's length, so a longer call sign shrinks
/// to fit rather than overflowing.
///
/// Truncation to four applies ONLY to the fallback, where there is no resolved
/// call sign and the preset's own name has to stand in — a name like "FM 88.7"
/// is not a call sign and there is nothing to preserve.
pub fn plate_label(base: Option<&str>, name: &str) -> String {
    if let Some(b) = base {
        if !b.is_empty() {
            return clean_call(b);
        }
    }
    let short: String = clean_call(name).chars().take(4).collect();
    if !short.is_empty() {
        return short;
    }
    name.trim().chars().take(4).collect::<String>().to_uppercase()
}

/// The dial, to one decimal. FM is always MHz, so no unit is ever appended on
/// the face — the label appears only in the tune overlay's frequency tab.
pub fn format_mhz(mhz: f32) -> String {
    format!("{:.1}", mhz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::from_rgb_u8(r, g, b)
    }

    /// THE RATIOS §5 MEASURED, TO TWO DECIMALS.
    ///
    /// Every one of these is a real station from the handoff's own table, with
    /// the fill and the ratio it published. They pin the linearisation as much as
    /// the choice: a `rel_lum` that skipped the sRGB knee, or used Rec.601
    /// weights, still picks the right ink on most of these and gets every ratio
    /// wrong — so the numbers are asserted, not just the winners.
    #[test]
    fn the_ink_guard_reproduces_the_handoffs_measured_ratios() {
        let dark = rgb(0x14, 0x18, 0x21);
        let white = rgb(0xFF, 0xFF, 0xFF);
        let cases: [(&str, Color, Color, f64); 5] = [
            ("WZEE cyan", rgb(14, 165, 196), dark, 6.09),
            ("WMHX orange", rgb(232, 89, 12), dark, 4.96),
            ("WMGN magenta", rgb(194, 30, 122), white, 5.58),
            ("WORT purple", rgb(107, 63, 160), white, 7.38),
            ("WJJO near-black", rgb(28, 31, 36), white, 16.52),
        ];
        for (what, fill, want_ink, want_ratio) in cases {
            assert_eq!(ink_on(fill), want_ink, "{what}: wrong ink");
            let got = contrast(fill, want_ink);
            assert!(
                (got - want_ratio).abs() < 0.01,
                "{what}: ratio {got:.2}, handoff measured {want_ratio:.2}"
            );
        }
    }

    /// THE TWO THAT A LUMINANCE THRESHOLD GETS WRONG.
    ///
    /// §5: "WZEE's cyan sits at luminance 0.3095, which a 0.32 threshold misses
    /// by 0.01." This is that sentence as a test — the cyan and the orange both
    /// take DARK ink despite reading as mid-to-bright colours, which is the exact
    /// pair a "light fill gets dark ink" cutoff inverts.
    #[test]
    fn a_mid_bright_fill_still_takes_dark_ink() {
        for (what, fill) in [("WZEE cyan", rgb(14, 165, 196)), ("WMHX orange", rgb(232, 89, 12))] {
            let ink = ink_on(fill);
            assert_eq!(ink, rgb(0x14, 0x18, 0x21), "{what} must not take white");
            assert!(
                contrast(fill, ink) >= 4.5,
                "{what} clears AA at {:.2}",
                contrast(fill, ink)
            );
        }
    }

    /// EVERY FILL THIS APP CAN ACTUALLY PRODUCE CLEARS AA.
    ///
    /// `brand_color` hashes into ten fixed values, so unlike the handoff's
    /// arbitrary station data this set is CLOSED and can be checked exhaustively.
    /// A future edit to `BRAND_BGS` that added an unreadable fill would fail here
    /// rather than on a dashboard.
    #[test]
    fn every_brand_fill_clears_aa_with_the_ink_the_guard_picks() {
        for rgb_u32 in BRAND_BGS {
            let fill = rgb((rgb_u32 >> 16) as u8, (rgb_u32 >> 8) as u8, rgb_u32 as u8);
            let ratio = contrast(fill, ink_on(fill));
            assert!(ratio >= 4.5, "fill #{rgb_u32:06X} only reaches {ratio:.2}:1");
        }
    }

    /// AN UNBRANDED PLATE TAKES THE THEME'S INK, NOT WHITE.
    ///
    /// The case §5 calls out as reachable through the app's own save flow: a
    /// station saved from a dial with no call sign has no fill, and white on the
    /// theme's `raised` measures 1.08:1.
    #[test]
    fn a_plate_with_no_fill_takes_the_themes_own_ink() {
        let theme_text = rgb(0x1B, 0x22, 0x2C);
        assert_eq!(plate_ink(None, theme_text), theme_text);
        // And a fill still overrides it, in both directions.
        assert_eq!(plate_ink(Some(rgb(28, 31, 36)), theme_text), rgb(0xFF, 0xFF, 0xFF));
        assert_eq!(plate_ink(Some(rgb(14, 165, 196)), theme_text), rgb(0x14, 0x18, 0x21));
    }

    #[test]
    fn clean_call_only_strips_a_separated_trailing_suffix() {
        assert_eq!(clean_call("WWHG-FM"), "WWHG");
        assert_eq!(clean_call("WJJO FM"), "WJJO");
        assert_eq!(clean_call("WQLF-fm"), "WQLF");
        // The call signs the old any-position match used to amputate.
        assert_eq!(clean_call("WHAM"), "WHAM");
        assert_eq!(clean_call("KJFM"), "KJFM");
        assert_eq!(clean_call("KAFM"), "KAFM");
        // Trailing whitespace after the suffix is part of the match.
        assert_eq!(clean_call("WERN-AM  "), "WERN");
        assert_eq!(clean_call("WERN"), "WERN");
    }

    #[test]
    fn plate_label_prints_a_resolved_call_sign_in_full() {
        // FM translators are 40% of the shipped station database and their call
        // signs are six characters. Truncating them to four was the bug this
        // signature exists to prevent.
        assert_eq!(plate_label(Some("K227EA"), "K227EA"), "K227EA");
        assert_eq!(plate_label(Some("W249BC"), "some preset name"), "W249BC");
        assert_eq!(plate_label(Some("DK203DR"), ""), "DK203DR");
        assert_eq!(plate_label(Some("WWHG-FM"), ""), "WWHG");
    }

    #[test]
    fn plate_label_falls_back_to_four_characters() {
        assert_eq!(plate_label(None, "WWHG-FM"), "WWHG");
        assert_eq!(plate_label(None, "magic 98"), "magi");
        assert_eq!(plate_label(None, ""), "");
        // An empty base is no base.
        assert_eq!(plate_label(Some(""), "WQLF-FM"), "WQLF");
    }

    /// Values taken from CarFM's `brandColor` for the same keys, so a station
    /// keeps the colour it already had on the head unit.
    #[test]
    fn brand_color_matches_the_javascript_hash() {
        let expect = |key: &str, rgb: u32| {
            let c = brand_color(key);
            assert_eq!(
                (c.red() as u32) << 16 | (c.green() as u32) << 8 | c.blue() as u32,
                rgb,
                "brand_color({key:?})"
            );
        };
        // ((h << 5) - h + c) | 0 then abs % 10, evaluated against the JS:
        //   WQLF 2672084 -> 4   WERN 2660746 -> 6   WWHG 2677727 -> 7
        //   WMGN 2668093 -> 3   WMLI 2668243 -> 3   WJJO 2665304 -> 4
        expect("WQLF", BRAND_BGS[4]);
        expect("WERN", BRAND_BGS[6]);
        expect("WWHG", BRAND_BGS[7]);
        expect("WMGN", BRAND_BGS[3]);
        expect("WMLI", BRAND_BGS[3]);
        expect("WJJO", BRAND_BGS[4]);
    }
}
