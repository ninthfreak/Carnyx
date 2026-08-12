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
/// the face — the label appears only in the tune numpad.
pub fn format_mhz(mhz: f32) -> String {
    format!("{:.1}", mhz)
}

#[cfg(test)]
mod tests {
    use super::*;

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
