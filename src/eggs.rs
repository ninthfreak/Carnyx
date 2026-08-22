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
///
/// NO `Eq`: the skins carry outline widths in logical pixels, which are floats.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// The sleeve accent — CarFM's `accent`, distinct from the `uiAccent` a cut
    /// may restate. NOTHING ON THIS FACE READS IT YET: the reference spends it on
    /// the settings gear's band motif and the theme glow, neither of which is
    /// ported. Kept because it is registry data with a transcribed value, and the
    /// first motif that lands will want the digits already right.
    pub accent: u32,
    /// A lightning bolt splits the call sign at its midpoint — WI⚡BA.
    pub call_sign_bolt: bool,
    /// Horns overhang the hero card's top corners (EASTER-EGGS §2.1).
    pub horns: bool,
    /// The station's logo is hidden so the lettering can carry the card. CarFM's
    /// `suppressLogos`, and it is what makes the call-sign bolt visible at all —
    /// a hero with art shows no call sign to split.
    pub suppress_logos: bool,
    /// What the theme restates for a DARK face, or `None` to keep the ordinary
    /// palette. AC/DC's is "Back in Black".
    pub dark: Option<Skin>,
    /// The same for a LIGHT face.
    pub light: Option<Skin>,
}

/// What a theme restates for one colour scheme.
///
/// `None` MEANS "LEAVE THE ORDINARY TOKEN ALONE", which is what lets one struct
/// describe both a full restatement and a cut that changes one thing.
///
/// AN `Option`, NOT A ZERO SENTINEL, and that cost a render to learn: these are
/// `0xRRGGBB` values, so the "unset" sentinel was the same bit pattern as the one
/// colour "Back in Black" is named after. `pageBg: '#000000'` read as unset, the
/// page stayed navy, and every other field applied — a theme that was 90% right
/// and silently wrong on the one value in its own title.
///
/// TWO REACH `Pal`; THE REST REACH ONE ELEMENT EACH, and that split is the
/// reference's, not a convenience. `CarFmFace.tsx:572-577` folds only the accent
/// pair into the palette — `pal = { ...basePal, blue, blueFill }` — and threads
/// the page background and the card colours down as their own props. So the two
/// that recolour a dozen components at once (`Pal.bg`, `Pal.blue`) go on the
/// global, and the surfaces stay where only their own element can see them. The
/// settings panel, the numpad and the nearby list keep the ordinary dark
/// palette; the hero card and the RadioText plate are the only surfaces that go
/// near-black.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Skin {
    /// `pageBg` — the whole face's ground. Global (`Pal.bg`).
    pub page_bg: Option<u32>,
    /// `uiAccent` — every blue graphic at once. Global (`Pal.blue`, and
    /// `Pal.blue-fill` at the reference's 0.15 alpha).
    pub accent: Option<u32>,
    /// `card.bg` / `card.border` / `card.text` — the HERO CARD alone.
    pub card_bg: Option<u32>,
    pub card_border: Option<u32>,
    pub card_text: Option<u32>,
    /// The call-sign bolt's ink. NOT a registry field: the face states it
    /// outright as `dark ? '#C9C9C9' : heroCardText` (`CarFmFace.tsx:1485`), so
    /// `None` here means "take the card's text colour". The registry's
    /// `settingsBoltColor` is a different bolt — the settings gear's, on a motif
    /// this port does not carry.
    pub bolt_ink: Option<u32>,
    /// `nameOutline` — stroke and width on the hero lettering. Its third field,
    /// `fill`, is declared in the registry and read by nothing.
    pub outline_ink: Option<u32>,
    pub outline_w: f32,
    /// `rtPlate` — the RadioText strip alone.
    pub rt_bg: Option<u32>,
    pub rt_border: Option<u32>,
    pub rt_text: Option<u32>,
    /// `genreOutline` — an edge on the genre line.
    pub genre_outline_ink: Option<u32>,
    pub genre_outline_w: f32,
}

/// Nothing restated.
pub const NO_SKIN: Skin = Skin {
    page_bg: None,
    accent: None,
    card_bg: None,
    card_border: None,
    card_text: None,
    bolt_ink: None,
    outline_ink: None,
    outline_w: 0.0,
    rt_bg: None,
    rt_border: None,
    rt_text: None,
    genre_outline_ink: None,
    genre_outline_w: 0.0,
};

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
    horns: true,
    suppress_logos: true,
    // "BACK IN BLACK": the page goes to true black, the hero card and the
    // RadioText plate sit a few points off it, and the interactive accent stops
    // being blue. Verbatim from `bandThemes.ts`'s `modes.dark` for this entry —
    // `pageBg`, `card`, `nameOutline`, `uiAccent`, `rtPlate` — with two of that
    // block's values deliberately absent:
    //
    //   `card.sub: '#7E7E7E'`   declared in the registry, read by no component.
    //   `uiAccentOn: '#0B0B0B'` `eggTokens`' own note: "has no home in the app
    //                           palette, so it's carried on the Egg for the art
    //                           pass but not applied here."
    //
    // Carrying either would be inventing a rule the reference does not have.
    dark: Some(Skin {
        page_bg: Some(0x000000),
        accent: Some(0xC9C9C9),
        card_bg: Some(0x0B0B0B),
        card_border: Some(0xA2A2A2),
        card_text: Some(0xE8E8E8),
        bolt_ink: Some(0xC9C9C9),
        outline_ink: Some(0xC9C9C9),
        outline_w: 1.1,
        rt_bg: Some(0x070707),
        rt_border: Some(0x171717),
        rt_text: Some(0xE8E8E8),
        ..NO_SKIN
    }),
    // The light cut restates one thing: an outline on the genre line, so the gold
    // has an edge against a pale ground.
    light: Some(Skin {
        genre_outline_ink: Some(0x241B0E),
        genre_outline_w: 1.0,
        ..NO_SKIN
    }),
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

/// The cut of `egg` that applies to the face as it currently stands.
///
/// A theme states its palette PER SCHEME, because "Back in Black" means nothing
/// on a white page. CarFM merges `modes.light`/`modes.dark` over the entry for
/// the active scheme; this is the same choice made explicitly.
pub fn skin(egg: &Egg, dark: bool) -> Skin {
    if dark { egg.dark } else { egg.light }.unwrap_or(NO_SKIN)
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

    /// "BACK IN BLACK", FIELD BY FIELD, against `bandThemes.ts`'s `modes.dark`.
    ///
    /// Written out rather than compared to the constant, because a test that
    /// reads the value it is checking passes whatever the value becomes. These
    /// digits are transcribed from the reference; if one of them moves, the
    /// theme is no longer the one the sleeve is.
    #[test]
    fn the_dark_cut_is_back_in_black() {
        let d = skin(&ACDC, true);
        // TRUE BLACK, AND SPELLED OUT AS PRESENT. `Some(0x000000)` is the exact
        // pair a zero sentinel could not tell apart, and this line is what would
        // have caught it: the page ONLY reads black if the field says so.
        assert_eq!(d.page_bg, Some(0x000000), "pageBg");
        assert_eq!(d.card_bg, Some(0x0B0B0B), "card.bg");
        assert_eq!(d.card_border, Some(0xA2A2A2), "card.border");
        assert_eq!(d.card_text, Some(0xE8E8E8), "card.text");
        assert_eq!(d.outline_ink, Some(0xC9C9C9), "nameOutline.stroke");
        assert_eq!(d.outline_w, 1.1, "nameOutline.width");
        assert_eq!(d.accent, Some(0xC9C9C9), "uiAccent");
        assert_eq!(d.rt_bg, Some(0x070707), "rtPlate.bg");
        assert_eq!(d.rt_border, Some(0x171717), "rtPlate.border");
        assert_eq!(d.rt_text, Some(0xE8E8E8), "rtPlate.text");
        // Stated by the face, not the registry: `dark ? '#C9C9C9' : heroCardText`.
        assert_eq!(d.bolt_ink, Some(0xC9C9C9), "call-sign bolt, dark");
    }

    /// EVERY BLUE GRAPHIC GOES SILVER, and exactly one field is what does it.
    ///
    /// `Pal.blue` is read by the pill, the preset selection, the nav chevrons,
    /// the tells and the scroll thumb; `skin.accent` is what Rust pushes into it.
    /// The light cut leaving it unstated is what keeps a pale face blue.
    #[test]
    fn the_accent_turns_only_on_the_dark_cut() {
        assert_eq!(skin(&ACDC, true).accent, Some(0xC9C9C9));
        assert_eq!(skin(&ACDC, false).accent, None);
    }

    /// The light cut restates ONE thing. Every other field is `None` — "leave
    /// the ordinary token alone" — which is how a themed light face stays
    /// ordinary apart from the gold genre line's edge.
    #[test]
    fn the_light_cut_restates_only_the_genre_outline() {
        let l = skin(&ACDC, false);
        assert_eq!(l.genre_outline_ink, Some(0x241B0E));
        assert_eq!(l.genre_outline_w, 1.0);
        assert_eq!(
            Skin { genre_outline_ink: None, genre_outline_w: 0.0, ..l },
            NO_SKIN,
            "the light cut may not restate anything but the genre outline"
        );
    }

    /// A THEME WITH NO CUT FOR A SCHEME CHANGES NOTHING, rather than falling back
    /// to the other scheme's. Sanity on `skin`'s own `unwrap_or`, since three of
    /// the four other CarFM themes state one mode and not the other.
    #[test]
    fn a_scheme_a_theme_says_nothing_about_is_left_alone() {
        const BARE: Egg = Egg { dark: None, light: None, ..ACDC };
        assert_eq!(skin(&BARE, true), NO_SKIN);
        assert_eq!(skin(&BARE, false), NO_SKIN);
    }

    /// Punctuation becomes a separator, never nothing — the difference between
    /// "AC/DC" matching and only ever matching the run-together spelling.
    #[test]
    fn punctuation_separates_rather_than_vanishing() {
        assert_eq!(normalize_rt("AC/DC"), "ac dc");
        assert_eq!(normalize_rt("Hi-Fi, 2026!"), "hi fi  2026 ");
    }
}
