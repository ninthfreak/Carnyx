//! The status-bar clock (§4.8), decided here rather than at the seam.
//!
//! ## What crosses the wire, and what does not
//!
//! Java answers two FACTS — the hour, the minute, and whether the system is set
//! to 24-hour — and this module turns them into the two strings the face draws.
//! No formatting happens in Java, for the rule this tree states everywhere: a
//! decision made there cannot be tested on a machine with no head unit, and
//! every decision here is tested below.
//!
//! ## 12/24 is not ours
//!
//! §4.8: *"The readout asks Android: `DateFormat.is24HourFormat(context)` …
//! it re-formats on every tick, so flipping the system toggle in Settings ▸
//! System ▸ Date & time changes the radio face with no restart and no app-side
//! preference to keep in sync."* So the flag is an INPUT here, never a stored
//! setting, and the settings row reports it rather than offering it.

/// The blank that holds a digit's column open.
///
/// ── THE SPEC SAYS SPACE, AND THE FONT SAYS OTHERWISE ─────────────────────────
///
/// §4.8 asks for a leading blank rather than a leading zero, and gives the
/// mechanism: *"12-hour single-digit hours pad with U+0020, which DSEG7 sets at
/// digit width — the position simply reads unlit, as on real hardware, and the
/// digit columns do not shift at 1 o'clock."*
///
/// THE FIRST HALF IS THE REQUIREMENT AND THE SECOND HALF IS NOT TRUE OF THE FILE
/// THAT SHIPPED. Measured out of `ui/fonts/DSEG7ClassicMini-Regular.ttf`'s own
/// `hmtx`, at 1000 units per em:
///
/// * `U+0020` — advance **200**, empty outline
/// * `!`      — advance **816**, empty outline
/// * `0`–`9`  — advance **816**
///
/// So padding with a space would pull ` 8:05` 616/1000 em to the left of
/// `12:05` and the columns WOULD shift at one o'clock — the exact failure the
/// paragraph exists to prevent. `!` is DSEG's own blank-digit convention: an
/// empty glyph at digit width, which is what "the position reads unlit" means on
/// real hardware.
///
/// The INTENT is honoured and the mechanism is not, which is the way round that
/// matters. `the_blank_holds_a_digits_column` measures both advances out of the
/// font file, so the day someone reads §4.8 and "corrects" this back to a space,
/// a test says why it is wrong rather than a drive showing a twitching clock.
const BLANK: char = '!';

/// The clock as the face draws it: the time, and a meridiem that is often empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    /// `08:05`, `12:05`, `!8:05` — see [`BLANK`] for the leading character.
    pub time: String,
    /// `A`, `P`, or empty in 24-hour mode. A SINGLE LETTER, never `AM`/`PM`.
    pub meridiem: String,
}

/// Format one reading.
///
/// `hour24` is 0–23 as the platform gave it and `minute` is 0–59; both are
/// clamped rather than trusted, because a clock that panics on a bad reading is
/// worse than one that shows the wrong minute.
///
/// MINUTES ALWAYS PAD WITH A ZERO and hours only in 24-hour mode — §4.8:
/// *"24-hour is zero-padded (`08:05`), 12-hour is not (` 8:05 A`); minutes are
/// always padded."*
pub fn format(hour24: u32, minute: u32, is_24h: bool) -> Clock {
    let h24 = hour24.min(23);
    let m = minute.min(59);
    if is_24h {
        return Clock { time: format!("{h24:02}:{m:02}"), meridiem: String::new() };
    }
    // MIDNIGHT AND NOON ARE BOTH 12, which is the one arithmetic in this file
    // worth stating: `0 % 12` is 0 and a clock never reads `0`.
    let h12 = match h24 % 12 {
        0 => 12,
        other => other,
    };
    let time = if h12 < 10 {
        format!("{BLANK}{h12}:{m:02}")
    } else {
        format!("{h12}:{m:02}")
    };
    // `A` BEFORE NOON AND `P` FROM NOON, which puts 12:00 in `P` and 00:00 in
    // `A` — the ordinary convention, and the one place an off-by-one would be
    // invisible for eleven hours a day.
    let meridiem = if h24 < 12 { "A" } else { "P" };
    Clock { time, meridiem: meridiem.to_string() }
}

/// What the settings row says under "Clock".
///
/// IT REPORTS THE FORMAT, IT DOES NOT OFFER IT. §4.8: *"The settings sub-line
/// reports which format is in force — it does not offer the choice."* The choice
/// lives in Android's own Date & time screen, and a second switch here would be
/// a second source of truth for one fact.
pub fn sub_line(is_24h: bool) -> String {
    let which = if is_24h { "24-hour" } else { "12-hour" };
    format!("Show the time under the icons. Following the system's {which} setting.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_four_hour_zero_pads_and_says_nothing_after_it() {
        assert_eq!(format(8, 5, true), Clock { time: "08:05".into(), meridiem: String::new() });
        assert_eq!(format(0, 0, true).time, "00:00");
        assert_eq!(format(23, 59, true).time, "23:59");
        assert_eq!(format(13, 7, true).time, "13:07");
        for h in 0..24 {
            assert!(format(h, 0, true).meridiem.is_empty(), "no meridiem in 24-hour mode");
        }
    }

    /// TWELVE-HOUR DOES NOT ZERO-PAD, AND THE COLUMN STILL HOLDS.
    #[test]
    fn twelve_hour_blanks_the_leading_digit_rather_than_zeroing_it() {
        assert_eq!(format(8, 5, false), Clock { time: "!8:05".into(), meridiem: "A".into() });
        assert_eq!(format(20, 5, false), Clock { time: "!8:05".into(), meridiem: "P".into() });
        // Two-digit hours take no blank.
        assert_eq!(format(10, 30, false).time, "10:30");
        assert_eq!(format(22, 30, false).time, "10:30");
        // AND THE STRING IS THE SAME LENGTH EITHER WAY, which is the whole point
        // of the blank: five characters at one o'clock and five at twelve.
        for h in 0..24 {
            assert_eq!(format(h, 0, false).time.chars().count(), 5, "hour {h}");
        }
    }

    /// MIDNIGHT AND NOON ARE BOTH 12, AND THEY ARE NOT THE SAME HALF OF THE DAY.
    #[test]
    fn midnight_and_noon_read_twelve_and_differ_only_in_the_letter() {
        assert_eq!(format(0, 0, false), Clock { time: "12:00".into(), meridiem: "A".into() });
        assert_eq!(format(12, 0, false), Clock { time: "12:00".into(), meridiem: "P".into() });
        assert_eq!(format(11, 59, false).meridiem, "A", "the last minute before noon");
        assert_eq!(format(12, 1, false).meridiem, "P", "the first after it");
        assert_eq!(format(23, 59, false), Clock { time: "11:59".into(), meridiem: "P".into() });
    }

    #[test]
    fn a_bad_reading_shows_a_wrong_minute_rather_than_panicking() {
        assert_eq!(format(99, 99, true).time, "23:59");
        assert_eq!(format(24, 60, false).time, "11:59");
    }

    /// THE BLANK HOLDS A DIGIT'S COLUMN, MEASURED OUT OF THE FONT FILE.
    ///
    /// §4.8 states the mechanism as U+0020 "which DSEG7 sets at digit width".
    /// That is not true of the file that shipped with the handoff, and this test
    /// is the evidence — it reads the `hmtx` advances rather than trusting the
    /// sentence. `!` is DSEG's blank-digit convention and is the one that keeps
    /// the columns still.
    ///
    /// A test rather than a comment because the comment is already there and a
    /// future reader with §4.8 open would be right to distrust it.
    #[test]
    fn the_blank_holds_a_digits_column() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui/fonts/DSEG7ClassicMini-Regular.ttf");
        let font = std::fs::read(&path).expect("the clock face is bundled");
        let advance = |ch: char| -> u16 { advance_of(&font, ch).expect("glyph is in the font") };

        let digit = advance('0');
        assert_eq!(digit, advance('8'), "the digits are tabular");
        assert_eq!(
            advance(BLANK),
            digit,
            "the blank must advance exactly one digit, or the columns shift at one o'clock"
        );
        assert_ne!(
            advance(' '),
            digit,
            "if U+0020 ever becomes digit-width, §4.8's own mechanism works and BLANK can go"
        );
    }

    /// One character's advance width, straight out of `cmap` + `hmtx`.
    ///
    /// Hand-rolled because this crate has no font parser and pulling one in to
    /// check two numbers would be the wrong trade — the two tables needed here
    /// are a few dozen lines of offsets.
    fn advance_of(font: &[u8], ch: char) -> Option<u16> {
        let u16_at = |o: usize| -> Option<u16> {
            Some(u16::from_be_bytes(font.get(o..o + 2)?.try_into().ok()?))
        };
        let u32_at = |o: usize| -> Option<u32> {
            Some(u32::from_be_bytes(font.get(o..o + 4)?.try_into().ok()?))
        };
        let mut tables = std::collections::HashMap::new();
        for i in 0..u16_at(4)? as usize {
            let rec = 12 + i * 16;
            let tag = font.get(rec..rec + 4)?.to_vec();
            tables.insert(tag, u32_at(rec + 8)? as usize);
        }
        let cmap = *tables.get(b"cmap".as_slice())?;
        let hhea = *tables.get(b"hhea".as_slice())?;
        let hmtx = *tables.get(b"hmtx".as_slice())?;

        // The format-4 subtable, which is the one a BMP-only face uses.
        let mut sub = None;
        for i in 0..u16_at(cmap + 2)? as usize {
            let off = cmap + u32_at(cmap + 8 + i * 8)? as usize;
            if u16_at(off)? == 4 {
                sub = Some(off);
            }
        }
        let sub = sub?;
        let seg = u16_at(sub + 6)? as usize / 2;
        let code = ch as u32 as u16;
        let ends = sub + 14;
        let starts = ends + seg * 2 + 2;
        let deltas = starts + seg * 2;
        let ranges = deltas + seg * 2;
        let mut glyph = 0u16;
        for i in 0..seg {
            if code <= u16_at(ends + i * 2)? && code >= u16_at(starts + i * 2)? {
                let delta = u16_at(deltas + i * 2)?;
                let range = u16_at(ranges + i * 2)?;
                glyph = if range == 0 {
                    code.wrapping_add(delta)
                } else {
                    let at = ranges + i * 2 + range as usize + (code - u16_at(starts + i * 2)?) as usize * 2;
                    match u16_at(at)? {
                        0 => 0,
                        g => g.wrapping_add(delta),
                    }
                };
                break;
            }
        }
        if glyph == 0 {
            return None;
        }
        // Past the last long metric every glyph repeats the last advance.
        let long = u16_at(hhea + 34)? as usize;
        let i = (glyph as usize).min(long.saturating_sub(1));
        u16_at(hmtx + i * 4)
    }
}
