//! Band themes — the artist Easter eggs (Design EASTER-EGGS-BUILD §12).
//!
//! A cosmetic skin that dresses the face for the artist currently playing and
//! reverts the instant the track changes. PURELY presentational: §12's
//! "scale-up rule" says a theme may change no layout, no control and no
//! behaviour, and nothing here returns anything but colours, strings and flags.
//!
//! ONE THEME, NOT FIVE. CarFM ships AC/DC, The Beatles, Led Zeppelin, Nirvana
//! and Nine Inch Nails; this is AC/DC alone, because that is what was asked for.
//! The registry is a slice rather than a single entry so adding the next one is
//! a table row and a motif arm, not a refactor — but nothing here pretends the
//! other four exist.
//!
//! PORTED FROM `src/components/carfm/bandThemes.ts`. The matcher is the part
//! that earns its own module: it runs against whatever the broadcaster puts in
//! RadioText, which on this market's stations is rotating advert copy, and it
//! has already been wrong once in the field. See [`match_egg_id`].

/// A resolved band theme.
///
/// Every field is a value the face reads directly — no palette lookups, no
/// framework types — so the whole thing is comparable and testable without a
/// window. CarFM's `Egg` carries fifty-odd optional fields across five themes;
/// this carries what AC/DC actually uses and nothing speculative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Egg {
    /// What the theme is called, and the key the face switches its motif on.
    pub id: &'static str,
    /// The line that replaces the PTY genre text.
    pub genre: &'static str,
    /// Base colour of that line.
    pub genre_ink: u32,
    /// The colour it pulses to. CarFM: `genrePulse` `#E8A400` → `#FFE24A`,
    /// `genrePulseOn: true`.
    pub genre_pulse: u32,
    /// The theme's accent, restated over every interactive element.
    pub accent: u32,
    /// A lightning bolt splits the call sign at its midpoint — WI⚡BA.
    pub call_sign_bolt: bool,
    /// The bolt's ink, which is the genre gold rather than the red accent.
    pub bolt_ink: u32,
    /// Horns overhang the hero card's top corners (EASTER-EGGS §2.1).
    pub horns: bool,
    /// The station's logo is hidden so the lettering can carry the card. CarFM's
    /// `suppressLogos`, and it is what makes the call-sign bolt visible at all —
    /// a hero with art shows no call sign to split.
    pub suppress_logos: bool,
}

/// AC/DC. CarFM's first registry entry, with its own colours verbatim.
///
/// `#E31E24` is the sleeve red and `#E8A400` the gold; both are lifted from
/// `bandThemes.ts` rather than sampled, because a theme that is nearly the right
/// red is just a bug with a costume on.
pub const ACDC: Egg = Egg {
    id: "AC/DC",
    genre: "High Voltage Rock 'n' Roll",
    genre_ink: 0xE8A400,
    genre_pulse: 0xFFE24A,
    accent: 0xE31E24,
    call_sign_bolt: true,
    bolt_ink: 0xE8A400,
    horns: true,
    suppress_logos: true,
};

/// Every theme that exists here, in match order.
const REGISTRY: &[(&Egg, &[&str])] = &[(&ACDC, &["ac dc", "acdc"])];

/// RadioText reduced to lower-case words separated by single spaces.
///
/// Punctuation becomes a SPACE rather than being deleted, and that is the whole
/// reason "AC/DC" can match the stored "ac dc". Deleting it instead would give
/// "acdc" only, and a station that writes the name with a slash — which is how
/// it is written — would never trigger.
pub fn normalize_rt(rt: &str) -> String {
    rt.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect()
}

/// Which theme should be showing, or `None`.
///
/// ON TOKEN BOUNDARIES, NOT RAW SUBSTRINGS, and this is the one piece of this
/// feature with a field failure behind it. Turning punctuation into spaces is
/// what lets "AC/DC" match — and it also turns an advert for "Hometown HVAC DC
/// power" into "hometown hvac dc power", which CONTAINS "ac dc" and repainted
/// CarFM's entire face as AC/DC. That stopped being hypothetical when the local
/// station began rotating advert copy through RadioText; the drive log this
/// port was checked against carries "NicoletLaw.com  Injured? Get Nicolet!"
/// on WIBA, so this matcher runs against arbitrary prose, not a slogan.
///
/// Padding both sides with a space and searching for the padded name buys whole
/// tokens without a regex per entry.
pub fn match_egg_id(rt: &str) -> Option<&'static Egg> {
    let norm = normalize_rt(rt);
    let padded = format!(" {} ", norm.split_whitespace().collect::<Vec<_>>().join(" "));
    if padded.trim().is_empty() {
        return None;
    }
    REGISTRY
        .iter()
        .find(|(_, names)| names.iter().any(|n| padded.contains(&format!(" {n} "))))
        .map(|(egg, _)| *egg)
}

/// The theme the face should wear, given what is playing and whether the face is
/// alive.
///
/// `dead` is audio priority released — the face goes flat and grey — and CarFM
/// suppresses every theme there (`resolveEgg`'s `off`). A grey face wearing a
/// red accent would read as a rendering fault.
pub fn resolve(rt: &str, dead: bool) -> Option<&'static Egg> {
    if dead {
        return None;
    }
    match_egg_id(rt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_matches_however_the_station_writes_it() {
        for rt in [
            "AC/DC - Back in Black",
            "acdc back in black",
            "Now playing: AC-DC, Thunderstruck",
            "ac dc",
            "THUNDERSTRUCK by AC/DC",
        ] {
            assert_eq!(
                match_egg_id(rt).map(|e| e.id),
                Some("AC/DC"),
                "should have matched {rt:?}"
            );
        }
    }

    /// THE ADVERT THAT REPAINTED THE WHOLE FACE.
    ///
    /// CarFM's own note: "Hometown HVAC DC power" normalizes to a string
    /// CONTAINING "ac dc", and before the boundary rule it turned the app red.
    /// The station this app is driven past rotates adverts through RadioText, so
    /// this is the ordinary case rather than a contrived one.
    #[test]
    fn a_substring_inside_other_words_is_not_a_match() {
        for rt in [
            "Hometown HVAC DC power",
            "hvac dc",
            "Tarmacdc",
            "MANIACDCOMICS",
            "reading the tarmac dcim folder",
        ] {
            assert_eq!(match_egg_id(rt), None, "should NOT have matched {rt:?}");
        }
    }

    #[test]
    fn nothing_at_all_matches_nothing() {
        for rt in ["", "   ", "NicoletLaw.com  Injured? Get Nicolet! - Z104"] {
            assert_eq!(match_egg_id(rt), None, "should NOT have matched {rt:?}");
        }
    }

    /// A FLATTENED FACE WEARS NO THEME. CarFM's `resolveEgg({ off })`.
    #[test]
    fn a_dead_face_is_never_themed() {
        assert_eq!(resolve("AC/DC - Back in Black", false).map(|e| e.id), Some("AC/DC"));
        assert_eq!(resolve("AC/DC - Back in Black", true), None);
    }

    /// Punctuation becomes a separator, never nothing — the difference between
    /// "AC/DC" matching and only ever matching the run-together spelling.
    #[test]
    fn punctuation_separates_rather_than_vanishing() {
        assert_eq!(normalize_rt("AC/DC"), "ac dc");
        assert_eq!(normalize_rt("Hi-Fi, 2026!"), "hi fi  2026 ");
    }
}
