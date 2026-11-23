//! Light or dark: the driver's choice, the headlights, the sun, the clock (#142).
//!
//! Pure on purpose — no window, no JNI, no clock of its own — so every rule
//! below is a test case rather than a hope. `App::apply_scheme` gathers the
//! [`Inputs`] and writes the answer to `Pal.dark`.
//!
//! ## The two automatic modes, and what each falls back to
//!
//! **Follow the headlights.** The unit sends `com.nwd.ACTION_ILL_STATE_CHANGE`
//! with `extra_ill_state`, `1` with the lights on — measured on this unit by
//! CarFM on 2026-08-01, while Android's own night flag stayed at DAY (#57). The
//! broadcast reports CHANGES ONLY and no getter for the current state has been
//! found, so a car started after dark with the lights already on hears nothing.
//! Until the first report this drive, the mode follows the sun; with no place
//! known either, the clock.
//!
//! **Follow sunrise and sunset.** [`crate::sun`] from the last place the car
//! had a fix (`prefs::Prefs::place`). With no place yet, the clock.
//!
//! **The clock** is CarFM's own last resort (`RadioScreen.tsx`, "PROVISIONAL
//! NIGHT"): dark from 7 PM to 7 AM. Its reasoning holds here — a wrong guess
//! costs a dim screen at dusk; the other mistake is a white face in the
//! driver's eyes at night.
//!
//! **And nothing**, which only a host build ever reaches: no epoch, no clock,
//! so the answer is `None` and the palette is left where it is. The clock is
//! deliberately consulted ONLY alongside the epoch (see `App::scheme_inputs`),
//! so no screenshot can change with the hour it happened to be taken at.

use crate::settings::Theme;
use crate::sun;

/// Everything the decision can use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inputs {
    pub theme: Theme,
    /// The last headlight report this run: `Some(true)` with them on. Never
    /// persisted — a report from the last drive says nothing about this one.
    pub headlights: Option<bool>,
    /// Where the car last had a fix, to a tenth of a degree.
    pub place: Option<(f64, f64)>,
    /// Now, Unix seconds.
    pub now: Option<i64>,
    /// The local wall clock: hour, minute, 24-hour setting.
    pub local: Option<(u32, u32, bool)>,
}

/// What decided it — for the log line, and so a test can say which rule ran.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Why {
    /// The driver picked Light or Dark.
    Chosen,
    /// The last headlight report.
    Headlights(bool),
    /// The sun, at the stored place.
    Sun(sun::Now),
    /// 7 PM to 7 AM.
    Clock,
    /// Nothing to go on; leave the palette alone.
    Unknown,
}

/// CarFM's provisional night: 19:00 up to but not including 07:00.
fn clock_dark(hour24: u32) -> bool {
    !(7..19).contains(&hour24)
}

fn sun_now(i: &Inputs) -> Option<sun::Now> {
    let (lat, lon) = i.place?;
    Some(sun::now(lat, lon, i.now?))
}

/// The fallback chain both automatic modes end in: the sun, then the clock.
fn by_sun_or_clock(i: &Inputs) -> (Option<bool>, Why) {
    if let Some(n) = sun_now(i) {
        return (Some(!n.up), Why::Sun(n));
    }
    match i.local {
        Some((h, _, _)) => (Some(clock_dark(h)), Why::Clock),
        None => (None, Why::Unknown),
    }
}

/// Dark (`Some(true)`), light (`Some(false)`), or no opinion (`None`).
pub fn decide(i: &Inputs) -> (Option<bool>, Why) {
    match i.theme {
        Theme::Light => (Some(false), Why::Chosen),
        Theme::Dark => (Some(true), Why::Chosen),
        Theme::Headlights => match i.headlights {
            Some(on) => (Some(on), Why::Headlights(on)),
            None => by_sun_or_clock(i),
        },
        Theme::Sun => by_sun_or_clock(i),
    }
}

/// Read `extra_ill_state` out of the receiver's verbatim extras dump.
///
/// THE DUMP, NOT A TYPED EXTRA, because the extra's type was never pinned
/// (`NwdBridge.startIlluminationWatch`): it arrives as `extra_ill_state=1
/// (Byte)`, and a `Boolean` would print `true`. Both are accepted. Anything
/// else is `None`, which leaves the last answer standing rather than guessing.
pub fn parse_headlights(extras: &str) -> Option<bool> {
    const KEY: &str = "extra_ill_state=";
    let at = extras.find(KEY)? + KEY.len();
    let value: String = extras[at..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    match value.as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

/// A Unix time as the driver's wall clock writes it: `4:36 PM` or `16:36`.
///
/// Needs both the epoch and the local reading, because their difference is the
/// device's UTC offset — [`crate::clock::offset_seconds`], the ETA's own seam.
fn wall(at: i64, now: i64, local: (u32, u32, bool)) -> String {
    let (h, m, is24) = local;
    let here = at + crate::clock::offset_seconds(now, h, m);
    let mins = here.rem_euclid(86_400) / 60;
    let (h24, min) = ((mins / 60) as u32, (mins % 60) as u32);
    if is24 {
        format!("{h24:02}:{min:02}")
    } else {
        let h12 = match h24 % 12 {
            0 => 12,
            x => x,
        };
        format!("{h12}:{min:02} {}", if h24 < 12 { "AM" } else { "PM" })
    }
}

const NIGHT_HOURS: &str = "dark from 7 PM to 7 AM";

/// One picker row's description. The two automatic rows say what they are
/// doing right now, so the panel answers "why is it dark?" by itself.
pub fn describe(row: Theme, i: &Inputs) -> String {
    match row {
        Theme::Light => "Always light.".to_string(),
        Theme::Dark => "Always dark.".to_string(),
        Theme::Headlights => match i.headlights {
            Some(true) => "The headlights are on, so it is dark.".to_string(),
            Some(false) => "The headlights are off, so it is light.".to_string(),
            None => {
                let until = if sun_now(i).is_some() {
                    "it follows the sun".to_string()
                } else if i.local.is_some() {
                    format!("it is {NIGHT_HOURS}")
                } else {
                    return "Dark while the headlights are on.".to_string();
                };
                format!("Dark while the headlights are on. The car only reports a change, so until it does, {until}.")
            }
        },
        Theme::Sun => match (sun_now(i), i.now, i.local) {
            (Some(n), Some(now), Some(local)) => match n.next {
                Some(sun::Event::Set(t)) => format!("The sun is up: light until sunset at {}.", wall(t, now, local)),
                Some(sun::Event::Rise(t)) => {
                    format!("The sun is down: dark until sunrise at {}.", wall(t, now, local))
                }
                None if n.up => "The sun stays up all day here, so it is light.".to_string(),
                None => "The sun stays down all day here, so it is dark.".to_string(),
            },
            _ if i.place.is_none() && i.local.is_some() => format!(
                "Dark between sunset and sunrise. It needs one GPS fix to place the sun; until then, {NIGHT_HOURS}."
            ),
            _ => "Dark between sunset and sunrise.".to_string(),
        },
    }
}

/// The diagnostics line for a change, so a drive log says what flipped it.
pub fn log_line(dark: bool, why: &Why) -> String {
    let scheme = if dark { "dark" } else { "light" };
    let because = match why {
        Why::Chosen => "chosen in settings".to_string(),
        Why::Headlights(true) => "headlights on".to_string(),
        Why::Headlights(false) => "headlights off".to_string(),
        Why::Sun(n) => if n.up { "sun up" } else { "sun down" }.to_string(),
        Why::Clock => "7 PM-7 AM, no headlight report or place yet".to_string(),
        Why::Unknown => "nothing to go on".to_string(),
    };
    format!("theme: {scheme} ({because})")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Madison, noon CST on 2026-11-12: the sun is up, setting at 4:36 PM.
    const NOON: i64 = 1_794_506_400;
    /// The same evening at 5 PM CST, after sunset.
    const EVENING: i64 = 1_794_524_400;
    const MADISON: (f64, f64) = (43.1, -89.4);

    fn base(theme: Theme) -> Inputs {
        Inputs { theme, headlights: None, place: None, now: None, local: None }
    }

    #[test]
    fn a_chosen_scheme_ignores_everything_else() {
        let mut i = base(Theme::Light);
        i.headlights = Some(true);
        assert_eq!(decide(&i), (Some(false), Why::Chosen));
        i.theme = Theme::Dark;
        i.headlights = Some(false);
        assert_eq!(decide(&i), (Some(true), Why::Chosen));
    }

    #[test]
    fn the_headlights_decide_once_the_car_reports_them() {
        let mut i = base(Theme::Headlights);
        i.headlights = Some(true);
        assert_eq!(decide(&i).0, Some(true));
        i.headlights = Some(false);
        // Even at night by the clock: the car's report outranks every guess.
        i.local = Some((23, 0, false));
        i.now = Some(EVENING);
        i.place = Some(MADISON);
        assert_eq!(decide(&i), (Some(false), Why::Headlights(false)));
    }

    /// Before the first report the headlights mode follows the sun, then the
    /// clock, then nothing — the chain the module note gives.
    #[test]
    fn with_no_report_yet_the_headlights_mode_falls_back_in_order() {
        let mut i = base(Theme::Headlights);
        assert_eq!(decide(&i), (None, Why::Unknown), "nothing at all");
        i.local = Some((20, 0, false));
        i.now = Some(EVENING);
        assert_eq!(decide(&i), (Some(true), Why::Clock), "no place: the clock");
        i.place = Some(MADISON);
        i.now = Some(NOON);
        i.local = Some((22, 0, false)); // a clock that disagrees: the sun wins
        assert!(matches!(decide(&i), (Some(false), Why::Sun(_))), "the sun beats the clock");
    }

    #[test]
    fn the_sun_mode_is_light_by_day_and_dark_after_sunset() {
        let mut i = base(Theme::Sun);
        i.place = Some(MADISON);
        i.now = Some(NOON);
        assert!(matches!(decide(&i), (Some(false), Why::Sun(_))));
        i.now = Some(EVENING);
        assert!(matches!(decide(&i), (Some(true), Why::Sun(_))));
        // Headlights are not this mode's business.
        i.headlights = Some(false);
        assert_eq!(decide(&i).0, Some(true));
    }

    #[test]
    fn the_clock_guess_is_seven_to_seven() {
        for h in 0..24 {
            assert_eq!(clock_dark(h), !(7..19).contains(&h), "{h}:00");
        }
        assert!(clock_dark(19) && clock_dark(6) && !clock_dark(7) && !clock_dark(18));
    }

    #[test]
    fn the_extra_is_read_from_the_dump_in_either_type() {
        assert_eq!(parse_headlights("extra_ill_state=1 (Byte)"), Some(true));
        assert_eq!(parse_headlights("extra_ill_state=0 (Byte)"), Some(false));
        assert_eq!(parse_headlights("other=3 (Integer), extra_ill_state=true (Boolean)"), Some(true));
        assert_eq!(parse_headlights("extra_ill_state=false (Boolean)"), Some(false));
        assert_eq!(parse_headlights("extra_ill_state=2 (Byte)"), None, "unknown values are not guessed");
        assert_eq!(parse_headlights("(no extras)"), None);
        assert_eq!(parse_headlights(""), None);
    }

    /// THE ROWS SAY WHAT IS HAPPENING, in the driver's own clock format.
    #[test]
    fn the_rows_describe_the_live_state() {
        let mut i = base(Theme::Sun);
        i.place = Some(MADISON);
        i.now = Some(NOON);
        i.local = Some((12, 0, false));
        // The 4:36 PM CST sunset; astral puts it at 22:36:15 UTC.
        assert_eq!(describe(Theme::Sun, &i), "The sun is up: light until sunset at 4:36 PM.");
        i.local = Some((12, 0, true));
        assert_eq!(describe(Theme::Sun, &i), "The sun is up: light until sunset at 16:36.");
        i.now = Some(EVENING);
        i.local = Some((17, 0, false));
        assert_eq!(describe(Theme::Sun, &i), "The sun is down: dark until sunrise at 6:47 AM.");

        assert_eq!(describe(Theme::Headlights, &i), format!(
            "Dark while the headlights are on. The car only reports a change, so until it does, it follows the sun."
        ));
        i.headlights = Some(true);
        assert_eq!(describe(Theme::Headlights, &i), "The headlights are on, so it is dark.");

        let mut no_place = base(Theme::Sun);
        no_place.now = Some(NOON);
        no_place.local = Some((12, 0, false));
        assert_eq!(
            describe(Theme::Sun, &no_place),
            "Dark between sunset and sunrise. It needs one GPS fix to place the sun; until then, dark from 7 PM to 7 AM."
        );
        assert_eq!(describe(Theme::Light, &no_place), "Always light.");
        assert_eq!(describe(Theme::Dark, &no_place), "Always dark.");
    }

    #[test]
    fn the_log_line_names_the_cause() {
        assert_eq!(log_line(true, &Why::Headlights(true)), "theme: dark (headlights on)");
        assert_eq!(log_line(false, &Why::Chosen), "theme: light (chosen in settings)");
        assert_eq!(log_line(true, &Why::Clock), "theme: dark (7 PM-7 AM, no headlight report or place yet)");
    }
}
