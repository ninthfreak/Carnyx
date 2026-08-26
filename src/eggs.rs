//! Band themes — the artist Easter eggs (Design EASTER-EGGS-BUILD §12).
//!
//! A cosmetic skin that dresses the face for the artist currently playing and
//! reverts the instant the track changes. PURELY presentational: §12's
//! "scale-up rule" says a theme may change no layout, no control and no
//! behaviour, and nothing here returns anything but colours, strings and flags.
//!
//! ── TWO TIERS ────────────────────────────────────────────────────────────────
//!
//! ADVANCED: AC/DC, The Beatles, Led Zeppelin, Nirvana and Nine Inch Nails —
//! every theme the design handoff specifies, each a palette, five faces, marks
//! and ornament. The registry was a slice with one row for exactly this reason,
//! and adding the other four was four rows and a motif arm rather than a
//! refactor. These are the rows the hidden picker in settings would list.
//!
//! BASIC: a genre line, a face, or both, and nothing else — one line of registry
//! through [`basic`], no art, no palette, no shot, and NO LISTING. The tail of
//! bands worth a nod is long and none of them is worth a build. See [`Tier`].
//!
//! The distinction is the owner's and the second tier is new; [`BASIC`] is empty
//! until somebody names a band for it.
//!
//! THREE OF THE FIVE CHANGE NO COLOUR AT ALL. Led Zeppelin, Nirvana and Nine
//! Inch Nails state `accent`/`glow`/`chromeInk` as the LIVE TOKENS, which is the
//! registry's way of writing "leave the palette alone"; what makes them themes is
//! type and marks. So `Skin` is `None` for all three and the only palettes here
//! are AC/DC's "Back in Black" and The Beatles' cream drum card.
//!
//! ── WHAT IS DELIBERATELY NOT CARRIED ─────────────────────────────────────────
//! `stripes` (The Beatles) and `chromeInk` (three themes) are declared in the
//! registry and read by NO component in the reference, alongside `card.sub` and
//! `uiAccentOn`. Carrying a field nothing consumes would be inventing a rule.
//!
//! `genreDroop` (The Beatles) IS consumed there and is not portable here: droop
//! ROTATES the genre line -4°, and Slint 1.17.1 offers no rotation for anything
//! but `Image` and `Path`. CarFM's own droop is already an approximation — §2.2
//! asks for a drooping PER-CHARACTER baseline and RN cannot do that either, so
//! it tilts the whole line. Recorded rather than faked.
//!
//! `heroGlitch` (Nine Inch Nails) WAS in that paragraph and should never have
//! been, and the reason it was is worth keeping: it was judged from CarFM's
//! `GlitchWrap` alone — a `translateX` — without opening the design handoff that
//! defines it. §2.5 states a three-band reconstruction of the text, which needs
//! clipping and not transforms, and RN's twitch is the stand-in for exactly the
//! thing RN cannot do. Now built — see [`Egg::hero_glitch`] and
//! `ui/glitch.slint`.
//!
//! PORTED FROM `src/components/carfm/bandThemes.ts`. The matcher is the part
//! that earns its own module: it runs against whatever the broadcaster puts in
//! RadioText, which on this market's stations is rotating advert copy, and it
//! has already been wrong once in the field. See [`match_egg_id`].

/// How much of the face a theme dresses, and whether it is worth listing.
///
/// TWO TIERS, and the distinction is the owner's: *"All currently defined band
/// Easter Eggs are now considered 'advanced' Easter Eggs."*
///
/// The difference is not a limit the code enforces — an [`Egg`] is one struct and
/// a `Basic` row could in principle set any field on it. It is a statement about
/// what a row IS, and two things follow from it mechanically: [`listed`] returns
/// only the advanced ones, and [`basic`] is the only way to build a basic row, so
/// a basic row cannot reach a field it is not meant to have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The full dress: palette, marks, type, ornament. All five of the original
    /// bands. LISTED in the hidden picker in settings.
    Advanced,
    /// One or two things and nothing else — a genre line, a face, or both. See
    /// [`basic`]. NEVER listed: the picker is a way to look at the five themes
    /// without waiting for the right track, and a hundred rows that each change
    /// a font is not that.
    Basic,
}

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
    /// Advanced or basic. See [`Tier`].
    pub tier: Tier,
    /// The label the hidden picker draws for this row — a PUN, never the id.
    ///
    /// `EGG_MENU` in the reference (`bandThemes.ts:215`) keeps the two apart in
    /// as many words: *"labels are puns — matching still uses the real id"*. So
    /// "Powerage" is what a driver reads and `AC/DC` is what the face switches
    /// on, and re-wording a label can never change what a row does.
    ///
    /// EMPTY ON EVERY BASIC ROW, and that is structural rather than a
    /// convention: [`basic`] builds from [`PLAIN`], which states `""`, so a basic
    /// row cannot acquire one by forgetting. `every_listed_theme_has_a_menu_label`
    /// reads both ends.
    pub menu: &'static str,
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
    /// `stereoArtL` / `stereoArtR` — a fan of bolts stands in for the stereo
    /// pill's speaker cones.
    ///
    /// ONE THEME HAS THIS, and the reference gates on the art's own presence
    /// (`egg?.stereoArtL ? … : null`, `CarFmFace.tsx:1206`) rather than on
    /// "a theme is showing". Only the AC/DC row names the two files, so every
    /// other theme keeps the cones. Carried as a flag because the two PNGs are
    /// bundled by name on this side and there is nothing for a path to select.
    pub stereo_bolts: bool,

    // ── TYPE ─────────────────────────────────────────────────────────────────
    //
    // A face is a FAMILY NAME as the file declares it, and `""` means "the
    // ordinary face". They are separate fields rather than one with fallbacks
    // because the reference's fallbacks are not uniform: `eggRtFont` falls back
    // to the body face, `eggGenreFont` does NOT fall back at all, and
    // `eggHeroFont` falls back but is then vetoed for one motif. Resolving each
    // one here is what keeps that asymmetry out of the face.
    /// The body face — preset tiles and peek cards. Empty when a theme scopes
    /// itself to the hero (`fontScope: 'hero'`, Led Zeppelin).
    pub body_face: &'static str,
    /// The hero lettering. Empty leaves it on the ordinary face, which is what
    /// The Beatles gets: §4 records its SgtPeppers cut as unfinished and the
    /// reference vetoes it outright.
    pub hero_face: &'static str,
    /// `heroScale` — a multiplier on the call sign's size.
    pub hero_scale: f32,
    /// `heroTrack` — letter-spacing in dp. 0 keeps the ordinary -1.
    pub hero_track: f32,
    /// `heroCase: 'lower'` — the call sign is lower-cased.
    pub hero_lower: bool,
    /// The genre line's own face. NO FALLBACK: `eggGenreFont` reads `genreFont`
    /// alone, which is why AC/DC's gold line is Atkinson and not Squealer.
    pub genre_face: &'static str,
    /// SYNTHETIC BOLD FOR THIS THEME'S OWN FACE, as a fraction of the font size,
    /// wherever that face is used — the genre line AND the RadioText. 0 is no
    /// synthesis and is what every row but one states.
    ///
    /// NAMED FOR THE FACE AND NOT THE GENRE LINE, because it reaches both: a
    /// basic theme's font goes to two places by the tier's own rule, and "in
    /// bold" is about the font rather than about one of the lines it sets.
    ///
    /// A DISPLAY FACE USUALLY SHIPS ONE CUT, and asking the engine for 700 then
    /// gets that cut back unchanged — Slint does not thicken a glyph the way a
    /// browser does. `Supernatural Knight` is `usWeightClass` 400 with a
    /// "Regular" subfamily, so "use the attached font in bold" cannot be
    /// honoured by a weight at all.
    ///
    /// BY DILATION, NOT BY A STROKE, for the reason `ui/glitch.slint` records
    /// against the hero lettering: `stroke-width` is inert in Slint 1.17.1's
    /// software renderer, so a stroked bold would be invisible in every shot and
    /// unverifiable off the device. The line is drawn at the four corners of a
    /// square of this radius as well as true, which thickens stems, bars and
    /// diagonals alike by twice the radius.
    pub face_bold: f32,
    /// The RadioText face and its tracking (`rtFont` / `rtSpacing`).
    pub rt_face: &'static str,
    pub rt_track: f32,
    /// The frequency readout's face and scale (`freqFont` / `freqScale`).
    pub freq_face: &'static str,
    pub freq_scale: f32,

    // ── ORNAMENT ─────────────────────────────────────────────────────────────
    /// `genreCycle` — a second genre string, cross-faded with the first on a
    /// long loop. Empty when the line does not cycle.
    pub genre_cycle: &'static str,
    /// `genreArt` — the four marks from Led Zeppelin's fourth record stand IN
    /// PLACE OF the genre line. Debossed, never printed, never a font glyph.
    pub genre_runes: bool,
    /// `nameGhost` — a hard offset drop-shadow under the hero lettering. Alpha
    /// is carried separately because the reference states these as `rgba()`.
    pub ghost_ink: u32,
    pub ghost_alpha: f32,
    pub ghost_dx: f32,
    pub ghost_dy: f32,
    /// `heroGlitch` — every call sign is rebuilt as three horizontal bands.
    ///
    /// NINE INCH NAILS ALONE. EASTER-EGGS-BUILD §2.5 states it as a
    /// construction: "rebuild the text as three horizontal bands over a hidden
    /// layout copy — top band true and solid, middle band shifted +0.05em at 42%
    /// opacity, bottom band shifted -0.035em at 88%", with clip insets naming
    /// where the three cuts fall. It is STATIC, and it is on every call sign
    /// rather than on the hero: the offsets are stated in em "so the same
    /// treatment holds from a 99sp hero down to a 13sp tile label". Drawn by
    /// `ui/glitch.slint`, which carries the numbers.
    ///
    /// CarFM's `GlitchWrap` (`CarFmFace.tsx:407`) is NOT that. It twitches the
    /// identity ±2dp on a 2s loop, which is React Native's stand-in — RN cannot
    /// clip a run of text, so it moves the whole word instead. This port built
    /// the twitch first, from the flag's name and that component alone, and
    /// shipped it as the effect. That was reading the workaround as the
    /// specification.
    pub hero_glitch: bool,
    /// Which mark replaces the settings gear. See [`Gear`].
    pub gear: Gear,
    /// `motif: 'runes'` — the vehicle-in-motion tell flies an airship instead of
    /// a car, and this theme leaves the gear alone.
    pub airship: bool,
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

/// Which band mark stands in for the settings gear.
///
/// PARTIAL ON PURPOSE, exactly as `bandArt.tsx`'s `GEAR_BY_MOTIF` is: Led
/// Zeppelin replaces the vehicle-in-motion tell instead and leaves the gear
/// alone, so it has no entry and the caller draws the ordinary gear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gear {
    /// The ordinary gear. Every unthemed face, and Led Zeppelin.
    Plain,
    /// AC/DC — the bolt.
    Bolt,
    /// The Beatles — a drum hoop.
    Drum,
    /// Nirvana — the smiley.
    Smiley,
    /// Nine Inch Nails — the spiral.
    Spiral,
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
    /// `nameOutline` — the hero lettering FILLED and hairline-outlined.
    ///
    /// ALL THREE FIELDS ARE REAL HERE, and the fill is the one that makes the
    /// treatment what it is: the glyph is painted in the card's own near-black
    /// and separated from it only by the silver hairline, so the call sign reads
    /// as cut out of the panel rather than printed on it. The prototype's own
    /// words: "the glyph is filled in the panel's own black and separated from it
    /// only by a hairline outline".
    ///
    /// CARNYX FILLED IT `#E8E8E8` UNTIL v1.16.1's REFERENCE RENDER SETTLED IT.
    /// CarFM cannot draw a real text stroke — React Native has none — so it fakes
    /// one with a zero-offset shadow and never reads `fill` at all, which read as
    /// the field being decorative. `screenshots/egg-acdc-dark.png` shows hollow
    /// lettering, and Slint has `stroke` / `stroke-style: outside`, so there is
    /// nothing to approximate.
    pub outline_fill: Option<u32>,
    pub outline_ink: Option<u32>,
    pub outline_w: f32,
    /// `rtPlate` — the RadioText strip alone.
    pub rt_bg: Option<u32>,
    pub rt_border: Option<u32>,
    pub rt_text: Option<u32>,
    /// `genreOutline` — an edge on the genre line.
    pub genre_outline_ink: Option<u32>,
    pub genre_outline_w: f32,
    /// `cardFrame` — concentric rules inset into the hero card's edge, each a
    /// colour and an inset in dp. The Beatles' drum hoop. Empty is no frame.
    pub card_rings: &'static [(u32, f32)],
    /// `rtPlate.serial` — a catalogue stamp in the RadioText plate's corner.
    pub rt_serial: &'static str,
}

/// Nothing restated.
pub const NO_SKIN: Skin = Skin {
    page_bg: None,
    accent: None,
    card_bg: None,
    card_border: None,
    card_text: None,
    bolt_ink: None,
    outline_fill: None,
    outline_ink: None,
    outline_w: 0.0,
    rt_bg: None,
    rt_border: None,
    rt_text: None,
    genre_outline_ink: None,
    genre_outline_w: 0.0,
    card_rings: &[],
    rt_serial: "",
};

/// A theme that changes nothing, for the four rows below to state a diff
/// against.
///
/// EVERY ROW IS A DIFF because that is how the registry reads: three of the five
/// themes state NO PALETTE AT ALL — their `accent`/`glow`/`chromeInk` restate the
/// live tokens, which is the registry's way of writing "leave it alone" — and
/// what distinguishes them is type and marks. Spelling out fourteen defaults per
/// row would bury the four or five fields that are the theme.
pub const PLAIN: Egg = Egg {
    id: "",
    // NO LABEL, for the same reason `tier` defaults to Basic: a row that forgets
    // to say is making the smaller claim. An unlisted row with a label is dead
    // string; a listed row without one is a blank line in the picker.
    menu: "",
    // BASIC, so a row that forgets to say is the SMALLER claim. An advanced row
    // that forgot would be missing from the picker, which someone notices the
    // first time they look; a basic row that forgot would appear in it, which
    // nobody notices until the list is a hundred long.
    tier: Tier::Basic,
    genre: "",
    genre_ink: 0,
    genre_pulse: 0,
    accent: 0,
    call_sign_bolt: false,
    horns: false,
    stereo_bolts: false,
    suppress_logos: false,
    body_face: "",
    hero_face: "",
    hero_scale: 1.0,
    hero_track: 0.0,
    hero_lower: false,
    genre_face: "",
    face_bold: 0.0,
    rt_face: "",
    rt_track: 0.0,
    freq_face: "",
    freq_scale: 1.0,
    genre_cycle: "",
    genre_runes: false,
    ghost_ink: 0,
    ghost_alpha: 0.0,
    ghost_dx: 0.0,
    ghost_dy: 0.0,
    hero_glitch: false,
    gear: Gear::Plain,
    airship: false,
    dark: None,
    light: None,
};

/// AC/DC. CarFM's first registry entry, with its own colours verbatim.
///
/// `#E31E24` is the sleeve red and `#E8A400` the gold; both are lifted from
/// `bandThemes.ts` rather than sampled, because a theme that is nearly the right
/// red is just a bug with a costume on.
pub const ACDC: Egg = Egg {
    id: "AC/DC",
    menu: "Powerage",
    tier: Tier::Advanced,
    genre: "High Voltage Rock 'n' Roll",
    genre_ink: 0xE8A400,
    genre_pulse: 0xFFE24A,
    accent: 0xE31E24,
    call_sign_bolt: true,
    horns: true,
    stereo_bolts: true,
    suppress_logos: true,
    // Squealer on the hero and the RadioText. The genre line is NOT Squealer —
    // `eggGenreFont` reads `genreFont` alone and this entry names none — which
    // `screenshots/egg-acdc-dark.png` shows: the gold line is Atkinson.
    body_face: FACE_SQUEALER,
    hero_face: FACE_SQUEALER,
    rt_face: FACE_SQUEALER,
    rt_track: 2.0,
    freq_face: FACE_SQUEALER,
    gear: Gear::Bolt,
    // "BACK IN BLACK": a near-black hero card and RadioText plate, hollow silver
    // lettering, and an interactive accent that stops being blue. Verbatim from
    // `modes.dark` (EASTER-EGGS-BUILD §2.1, handoff v1.16.1).
    //
    // THE PAGE IS NO LONGER RESTATED, and that is the v1.16.0 change. It used to
    // be true black, which put a `#0B0B0B` card on a `#000000` field and left the
    // panel merging into the ground it was meant to sit on. §2.1 now says
    // `page #24272C` — the SAME lifted charcoal every other station gets — so the
    // card reads AGAINST a grey field. Stated here as "restates nothing" rather
    // than as that hex: the changelog's own words are "sits on the same lifted
    // page as every other station", and a copy of the value would silently stop
    // meaning that the next time the ordinary page moves.
    //
    // Two of the block's values are deliberately absent:
    //
    //   `card.sub: '#7E7E7E'`   declared in the registry, read by no component.
    //   `uiAccentOn: '#0B0B0B'` `eggTokens`' own note: "has no home in the app
    //                           palette, so it's carried on the Egg for the art
    //                           pass but not applied here."
    //
    // Carrying either would be inventing a rule the reference does not have.
    dark: Some(Skin {
        accent: Some(0xC9C9C9),
        card_bg: Some(0x0B0B0B),
        card_border: Some(0xA2A2A2),
        card_text: Some(0xE8E8E8),
        bolt_ink: Some(0xC9C9C9),
        outline_fill: Some(0x0B0B0B),
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
    ..PLAIN
};

// The family names the bundled files declare. See `ui/tokens.slint` — these are
// NOT the keys `bandThemes.ts` uses, which are file labels.
const FACE_SQUEALER: &str = "Squealer";
const FACE_BEATLES: &str = "YellowSubmarine";
const FACE_BEATLES_GENRE: &str = "Madie Roger";
const FACE_KASHMIR: &str = "Kashmir";
const FACE_MARKER: &str = "Permanent Marker";
const FACE_ONYX: &str = "Onyx";
const FACE_GRIDNIK: &str = "FoundryGridnik";
const FACE_SINGOTHIC: &str = "Singothic";
/// "Carry On Wayward Son". THE FAMILY, NOT THE FILE: the file arrived as
/// `Supernatural_Knight.ttf` and the family its `name` table declares is
/// "Supernatural Knight", with a space and no underscore. Slint resolves
/// `font-family` against the declared family, so the filename would have
/// silently fallen back to Atkinson —
/// `every_face_a_theme_names_is_bundled_and_imported` is what makes that a red
/// test rather than a disappointment on a dashboard.
const FACE_KNIGHT: &str = "Supernatural Knight";

/// THE BEATLES — `motif: submarine`.
///
/// The one theme that restates a SURFACE rather than an accent: a cream card
/// inside a drum hoop of four concentric rules, and a white RadioText plate with
/// a catalogue serial stamped in its corner. The same in both schemes, because
/// the registry states `card` and `rtPlate` on the ENTRY and not inside `modes`.
///
/// THE HERO STAYS ON THE ORDINARY FACE. The entry names SgtPeppers for it and
/// §4 records that cut as outline-only and unfinished; the reference then vetoes
/// it in code — `eggHeroFont` is gated on `motif !== 'submarine'`. So the body
/// face reaches the tiles and the RadioText and stops there. Not a gap: the
/// reference draws it this way on purpose, and the veto is the port.
pub const BEATLES: Egg = Egg {
    id: "The Beatles",
    menu: "The Walrus was Paul",
    tier: Tier::Advanced,
    genre: "Rock",
    genre_ink: 0x4A2C15,
    hero_lower: true,
    body_face: FACE_BEATLES,
    rt_face: FACE_BEATLES,
    genre_face: FACE_BEATLES_GENRE,
    suppress_logos: true,
    gear: Gear::Drum,
    dark: Some(BEATLES_SKIN),
    light: Some(BEATLES_SKIN),
    ..PLAIN
};

const BEATLES_SKIN: Skin = Skin {
    card_bg: Some(0xF3E8D2),
    card_border: Some(0xA81F28),
    card_text: Some(0x241608),
    rt_bg: Some(0xFFFFFF),
    rt_border: Some(0xDED6C6),
    rt_text: Some(0x1A1A1A),
    rt_serial: "No. 0101538",
    // The drum hoop: cream, blue, cream, red, each inset further from the edge.
    card_rings: &[(0xF3E8D2, 5.0), (0x2E4EA0, 6.5), (0xF3E8D2, 17.0), (0xA81F28, 18.5)],
    ..NO_SKIN
};

/// LED ZEPPELIN — `motif: runes`.
///
/// NO PALETTE CHANGE AT ALL. The entry's `accent`/`glow`/`chromeInk` are the live
/// tokens restated, which is the registry's way of writing "leave it alone", so
/// there is no `Skin` here and nothing on this face changes colour.
///
/// Type and marks only: Kashmir SCOPED TO THE HERO and the RadioText — a display
/// cut is unreadable at tile size and the theme reads from the hero anyway — the
/// four marks from the untitled fourth record standing in for the genre line, and
/// an airship in the vehicle-in-motion slot instead of the car. It is the one
/// theme that leaves the settings gear alone.
pub const ZEPPELIN: Egg = Egg {
    id: "Led Zeppelin",
    menu: "Hammer of the Gods",
    tier: Tier::Advanced,
    hero_face: FACE_KASHMIR,
    hero_scale: 1.3,
    rt_face: FACE_KASHMIR,
    rt_track: 2.0,
    genre_runes: true,
    suppress_logos: true,
    airship: true,
    ..PLAIN
};

/// NIRVANA — `motif: xerox`.
///
/// Default palette throughout, like Led Zeppelin. Permanent Marker on the body
/// and the genre line, Onyx at 1.5x on the hero, and a hard 3/3 drop-shadow under
/// the lettering at 20% black — a photocopied look, which is what `xerox` names.
pub const NIRVANA: Egg = Egg {
    id: "Nirvana",
    menu: "Smells Like Gen X",
    tier: Tier::Advanced,
    genre: "Verse Chorus Verse",
    body_face: FACE_MARKER,
    hero_face: FACE_ONYX,
    hero_scale: 1.5,
    genre_face: FACE_MARKER,
    rt_face: FACE_MARKER,
    rt_track: 1.0,
    ghost_ink: 0x000000,
    ghost_alpha: 0.20,
    ghost_dx: 3.0,
    ghost_dy: 3.0,
    suppress_logos: true,
    gear: Gear::Smiley,
    ..PLAIN
};

/// NINE INCH NAILS — `motif: spiral`.
///
/// Default palette again. Gridnik on the hero at 1.3x tracked out to 9dp,
/// Singothic on the genre line, the RadioText AND every frequency readout, and a
/// genre that cross-fades between two strings on a long loop.
///
/// `heroGlitch` IS NOT CARRIED — see this module's header.
pub const NIN: Egg = Egg {
    id: "Nine Inch Nails",
    menu: "Now I’m Nothing",
    tier: Tier::Advanced,
    genre: "Broken Machines",
    genre_cycle: "Things Falling Apart",
    body_face: FACE_GRIDNIK,
    hero_face: FACE_GRIDNIK,
    hero_scale: 1.3,
    hero_track: 9.0,
    genre_face: FACE_SINGOTHIC,
    rt_face: FACE_SINGOTHIC,
    rt_track: 5.0,
    freq_face: FACE_SINGOTHIC,
    freq_scale: 0.95,
    ghost_ink: 0x000000,
    ghost_alpha: 0.16,
    ghost_dx: 2.0,
    ghost_dy: 0.0,
    // The only theme that moves its own lettering. See `Egg::hero_glitch`.
    hero_glitch: true,
    suppress_logos: true,
    gear: Gear::Spiral,
    ..PLAIN
};

// ── BASIC THEMES ─────────────────────────────────────────────────────────────

/// A basic theme: a genre line, a face, or both, and NOTHING else.
///
/// ## What it is for
///
/// The five advanced themes are each a build — a palette, marks, five faces,
/// ornament, a per-scheme cut. That is the right amount of work for a band worth
/// dressing the whole face for, and far too much for the long tail. A basic row
/// is the long tail: one line of registry, no art, no palette, no shot.
///
/// ## Both arguments are optional, and `""` is how you say so
///
/// `""` for the genre keeps the PTY the broadcaster sent — see `GenreText`,
/// which falls through to it. `""` for the face keeps the ordinary one. A row
/// with neither changes nothing and is caught by
/// `a_basic_theme_has_to_actually_do_something`.
///
/// ## ONE FACE, TWO PLACES
///
/// The face lands on the genre line AND the RadioText, which is the owner's
/// rule: *"Custom fonts, when defined, will be used for both the genre and radio
/// text."* It deliberately does NOT reach the hero, the preset tiles or the
/// frequency readout — those are `hero_face`, `body_face` and `freq_face`, and a
/// row that set them would be an advanced theme with a basic row's paperwork.
///
/// Note this is the one place in the file where a single face feeds two fields.
/// The advanced rows keep them apart because the reference's fallbacks are not
/// uniform (see the TYPE block on [`Egg`]); a basic row has no fallbacks to be
/// asymmetric about, so the rule can be stated once here.
pub const fn basic(id: &'static str, genre: &'static str, face: &'static str) -> Egg {
    Egg {
        id,
        tier: Tier::Basic,
        genre,
        genre_face: face,
        rt_face: face,
        ..PLAIN
    }
}

// ── THE REGISTRY ─────────────────────────────────────────────────────────────

/// The advanced themes, in match order — the reference registry's own order, so
/// a RadioText naming two bands resolves the same way it does there.
const ADVANCED: &[(&Egg, &[&str])] = &[
    (&ACDC, &["ac dc", "acdc"]),
    (&BEATLES, &["beatles"]),
    (&ZEPPELIN, &["led zeppelin", "zeppelin"]),
    (&NIRVANA, &["nirvana"]),
    (&NIN, &["nine inch nails"]),
];

/// The basic themes, in match order among themselves.
///
/// ── BEFORE ADDING ONE, READ [`match_egg_id`] ──
///
/// This list is where the false-match hazard gets dangerous. Five long names are
/// nearly safe; a long tail is not, because the matcher runs against rotating
/// advert copy and a short or ordinary-word band name — "Yes", "Bread", "Free",
/// "Air" — will fire on prose that has nothing to do with music.
/// `no_basic_name_is_short_enough_to_fire_on_prose` holds a floor under it, and
/// a floor is not a substitute for thinking about the name. Two of the three
/// below carry a note about theirs.
const BASIC: &[(&Egg, &[&str])] = &[
    // `clapton` alone, not `eric clapton`. The padded search wants whole tokens
    // either way, and the surname matches BOTH spellings — " eric clapton " has
    // " clapton " inside it — so the longer form would be a second entry that
    // can never match anything the first one misses. Distinctive enough on its
    // own that no advert says it.
    (&CLAPTON, &["clapton"]),
    // NOT `reckless` ALONE, which is the trap this row would otherwise walk
    // into: "reckless" is ordinary English and "reckless driving" is exactly the
    // sort of thing a local station's advert copy says. The band's own name is
    // two words and the pair is safe. `pretty reckless` also matches the full
    // "The Pretty Reckless", for the same substring reason as Clapton above.
    (&PRETTY_RECKLESS, &["pretty reckless"]),
    // "the who" is two of the commonest words in English side by side and the
    // six-character floor passes it at seven, so it CAN fire on prose: "find out
    // the who, what and where" normalises to a string containing " the who ".
    // There is no safer spelling — that is the band's name.
    //
    // RAISED AND SETTLED BY THE OWNER: RadioText is station, artist and track
    // copy rather than arbitrary English, and that construction does not turn up
    // in it. The tier also bounds the damage — a false match swaps one genre
    // line, where the failure that made this hazard famous was an advert
    // repainting CarFM's ENTIRE FACE as AC/DC.
    //
    // AND IT DOES NOT COLLIDE WITH "The Guess Who", which is the near-miss worth
    // naming: the search is for the two tokens ADJACENT, and " the guess who "
    // has a word between them. Pinned by
    // `the_basic_bands_match_the_way_a_station_writes_them`.
    (&THE_WHO, &["the who"]),
    // A SONG, NOT A BAND — the first of those, and the tier did not have to
    // change to take it: `basic` names a row and a genre, and nothing in the
    // matcher ever cared whether the name was an artist's.
    //
    // `wayward son` rather than the full title, on `clapton`'s logic: the search
    // wants whole tokens either way and the short form is inside the long one,
    // so " carry on wayward son " contains " wayward son " and one entry catches
    // both spellings. Distinctive enough on its own — "wayward" is not a word
    // that turns up in advert copy.
    (&WAYWARD_SON, &["wayward son"]),
];

/// Eric Clapton. Genre only; the ordinary faces throughout.
pub const CLAPTON: Egg = basic("Eric Clapton", "Slowhand", "");

/// The Pretty Reckless. The owner named it "Pretty Reckless"; the id is the
/// band's full name and the match covers both.
pub const PRETTY_RECKLESS: Egg = basic("The Pretty Reckless", "Cindy-Lou Who?", "");

/// The Who. See the note on its registry row.
pub const THE_WHO: Egg = basic("The Who", "Meaty, Beaty, Big, and Bouncy", "");

/// "Carry On Wayward Son" — a song rather than an artist, and the first basic
/// row to bring a FACE.
///
/// The face reaches the genre line AND the RadioText, which is the tier's rule
/// and is what `basic` does with its third argument. It does NOT reach the hero,
/// the preset tiles or the dial; those stay ordinary, which is the line between
/// a basic theme and an advanced one.
///
/// THE FONT HAS NO BOLD CUT. `GenreText` asks a basic row's line for
/// `Font.bold`, and `SupernaturalKnight.ttf` is `usWeightClass` 400 with a
/// "Regular" subfamily and no bold bit set — so the engine resolves 700 to the
/// single cut there is. It is a heavy display face already; see
/// `ui/tokens.slint` for the reading and `shots/wayward.png` for what it draws.
pub const WAYWARD_SON: Egg = Egg {
    // "in bold", and the face has no bold cut to give — see `Egg::face_bold`.
    // 0.02em: at the wide track's 26dp genre line that is half a pixel each
    // side, which on a hairline engraved serif is the difference between the
    // instruction being honoured and being quietly dropped.
    face_bold: 0.02,
    ..basic("Carry On Wayward Son", "Season Finale", FACE_KNIGHT)
};

/// Two basic rows that exist ONLY under `cfg(test)`.
///
/// [`BASIC`] is empty and honestly so, which leaves every rule about the tier —
/// the matcher, the picker filter, the two registry checks — asserting over
/// nothing and passing. These give them something to assert over.
///
/// A SUPERSET, NOT A SUBSTITUTE. [`registry`] chains these AFTER the shipped
/// rows rather than in place of them, so every existing matcher test still runs
/// against the real registry and a fixture cannot mask a shipped row. The names
/// are deliberately not bands.
///
/// One of each shape, because the two fail differently: a row with a genre and
/// no font is the ordinary case, and a row with a font and no genre is the one
/// that used to blank the genre line instead of dressing it.
#[cfg(test)]
const BASIC_FIXTURES: &[(&Egg, &[&str])] = &[
    (&FIXTURE_GENRE, &["fixture genre"]),
    (&FIXTURE_FACE, &["fixture face"]),
];

#[cfg(test)]
const FIXTURE_GENRE: Egg = basic("Fixture Genre", "Test Signal", "");
#[cfg(test)]
const FIXTURE_FACE: Egg = basic("Fixture Face", "", FACE_ONYX);

/// Every theme, advanced first.
///
/// THE ORDER IS THE PRECEDENCE and it is structural rather than a convention
/// about where to paste a row: a RadioText that names an advanced band and a
/// basic one gets the advanced dress, because chaining puts those rows first.
/// One flat list would have made that a comment nobody has to obey.
fn registry() -> impl Iterator<Item = &'static (&'static Egg, &'static [&'static str])> {
    let basic = BASIC.iter();
    // Appended, never substituted — see [`BASIC_FIXTURES`].
    #[cfg(test)]
    let basic = basic.chain(BASIC_FIXTURES.iter());
    ADVANCED.iter().chain(basic)
}

/// Every basic row the rules below apply to, fixtures included.
#[cfg(test)]
fn basic_rows() -> impl Iterator<Item = &'static (&'static Egg, &'static [&'static str])> {
    BASIC.iter().chain(BASIC_FIXTURES.iter())
}

/// The themes the hidden picker in settings lists — the advanced ones.
///
/// THE PICKER READS THIS AND DOES NOT FILTER AGAIN. The owner's rule is that
/// basic themes get no listing, and this is the mechanical half of what "basic"
/// MEANS: a rule with no code behind it is a comment.
///
/// It existed before the picker did, for exactly that reason. `ui/settings.slint`
/// carried the note that CarFM wraps the about line in a Pressable counting to
/// six, that a picker was ported here once against themes that did not exist yet
/// — so six taps moved a radio button and changed nothing — and that it was
/// removed rather than left half-built. The themes exist now, and so does the
/// picker.
pub fn listed() -> Vec<&'static Egg> {
    registry().map(|(egg, _)| *egg).filter(|e| e.tier == Tier::Advanced).collect()
}

/// One theme by its id, for the picker's forced choice.
///
/// SEARCHES EVERYTHING, not just [`listed`]. The picker can only offer what
/// `listed` returns, but the id it hands back travels through a preference file
/// and back through this lookup, and a resolver that could only find listed rows
/// would be a SECOND place enforcing the tier rule — which is where the two would
/// eventually disagree. There is one such place and it is `listed`.
pub fn by_id(id: &str) -> Option<&'static Egg> {
    registry().map(|(egg, _)| *egg).find(|e| e.id == id)
}

/// Every theme, listed or not, for tests and for anything that has to check the
/// whole set rather than the picker's slice of it.
pub fn all() -> Vec<&'static Egg> {
    registry().map(|(egg, _)| *egg).collect()
}

/// Every display face this app can actually draw, as `(family, file)`.
///
/// THE FAMILY IS NOT THE FILENAME and that is the whole reason this table exists
/// rather than a list of names. Slint resolves `font-family` against the family
/// the file DECLARES in its `name` table, so `BeatlesYellowSub.ttf` answers to
/// "YellowSubmarine" and `Gridnik.otf` to "FoundryGridnik".
/// `ui/tokens.slint` records that it was checked file by file, and records that
/// copying CarFM's own keys would have missed every one of them.
///
/// ── WHY THIS IS A GUARD AND NOT DOCUMENTATION ────────────────────────────────
///
/// A face nobody bundled does not fail. Slint asks for the family, does not find
/// it, and quietly draws Atkinson — so a theme naming a font that is not in
/// `ui/fonts/` looks like a theme whose font "didn't work", on a unit with no way
/// to ask why. That was survivable while five faces arrived with a design
/// handoff. It stops being survivable the moment a basic theme is one line, and
/// the one line most likely to be wrong is the face.
///
/// `every_face_a_theme_names_is_bundled_and_imported` reads BOTH ends: that every
/// face named by any row is in this table, and that every file in this table is
/// `import`ed by `ui/tokens.slint`. Naming a face and forgetting the import is
/// then a red test rather than a silent Atkinson.
pub const BUNDLED_FACES: &[(&str, &str)] = &[
    (FACE_SQUEALER, "Squealer.otf"),
    (FACE_BEATLES, "BeatlesYellowSub.ttf"),
    (FACE_BEATLES_GENRE, "MadieRoger.ttf"),
    (FACE_KASHMIR, "Kashmir.ttf"),
    (FACE_MARKER, "PermanentMarker.ttf"),
    (FACE_ONYX, "Onyx.ttf"),
    (FACE_GRIDNIK, "Gridnik.otf"),
    (FACE_SINGOTHIC, "Singothic.ttf"),
    (FACE_KNIGHT, "SupernaturalKnight.ttf"),
];

/// Every face `egg` names, in no particular order, skipping the empty ones.
///
/// The five type fields are separate on [`Egg`] because their fallbacks differ;
/// anything asking "what fonts does this row need" wants them together, and
/// wants to be updated when a sixth is added rather than to keep working while
/// missing it.
pub fn faces_used(egg: &Egg) -> Vec<&'static str> {
    [egg.body_face, egg.hero_face, egg.genre_face, egg.rt_face, egg.freq_face]
        .into_iter()
        .filter(|f| !f.is_empty())
        .collect()
}

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
    registry()
        .find(|(_, names)| names.iter().any(|n| padded.contains(&format!(" {n} "))))
        .map(|(egg, _)| *egg)
}

/// The theme the face should wear, given what is playing and whether the face is
/// alive.
///
/// `dead` is audio priority released — the face goes flat and grey — and CarFM
/// suppresses every theme there (`resolveEgg`'s `off`). A grey face wearing a
/// red accent would read as a rendering fault.
/// `forced` is the hidden picker's choice and BEATS THE RADIOTEXT OUTRIGHT —
/// `matchEggId` returns it before it normalises anything (`bandThemes.ts:197`).
/// That is the whole point of the control: a driver looking at the five themes
/// should not have to wait for the right track.
///
/// IT DOES NOT BEAT `dead`, which is checked first here exactly as `resolveEgg`
/// checks `off` before calling the matcher. Silence flattens the face whatever
/// the picker says, because the flat grey face is a state and not a style.
///
/// A forced id no row answers to resolves to nothing rather than to a match on
/// the text — a theme deleted out from under a stored preference must not
/// silently become whichever band happens to be playing.
pub fn resolve(rt: &str, dead: bool, forced: Option<&str>) -> Option<&'static Egg> {
    if dead {
        return None;
    }
    if let Some(id) = forced.filter(|id| !id.is_empty()) {
        return by_id(id);
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
        assert_eq!(resolve("AC/DC - Back in Black", false, None).map(|e| e.id), Some("AC/DC"));
        assert_eq!(resolve("AC/DC - Back in Black", true, None), None);
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
        // THE PAGE IS THE ORDINARY LIFTED ONE. v1.16.0 took the theme off true
        // black so its near-black card reads against a grey field instead of
        // merging into it; restating nothing is how "the same page as every other
        // station" is written down.
        assert_eq!(d.page_bg, None, "pageBg");
        assert_eq!(d.card_bg, Some(0x0B0B0B), "card.bg");
        assert_eq!(d.card_border, Some(0xA2A2A2), "card.border");
        assert_eq!(d.card_text, Some(0xE8E8E8), "card.text");
        assert_eq!(d.outline_ink, Some(0xC9C9C9), "nameOutline.stroke");
        assert_eq!(d.outline_w, 1.1, "nameOutline.width");
        // THE LETTERING IS HOLLOW. Fill and card differ by nothing the eye can
        // measure, which is the whole treatment — the glyph is cut out of the
        // panel and held by the hairline alone.
        assert_eq!(d.outline_fill, Some(0x0B0B0B), "nameOutline.fill");
        assert_eq!(d.outline_fill, d.card_bg, "the fill IS the card's own black");
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

    /// ALL FIVE MATCH, AND ON THE SAME BOUNDARY RULE.
    ///
    /// The names are the registry's own. "zeppelin" without "led" is one of
    /// them, which is why the alias list is per entry rather than derived from
    /// the id — and it is also why the boundary rule has to hold for every row
    /// and not just the one that broke in the field.
    #[test]
    fn every_theme_matches_its_own_names() {
        for (rt, want) in [
            ("AC/DC - Back in Black", "AC/DC"),
            ("The Beatles - Here Comes the Sun", "The Beatles"),
            ("Led Zeppelin - Kashmir", "Led Zeppelin"),
            ("Zeppelin - Immigrant Song", "Led Zeppelin"),
            ("Nirvana - Smells Like Teen Spirit", "Nirvana"),
            ("Nine Inch Nails - The Hand That Feeds", "Nine Inch Nails"),
        ] {
            assert_eq!(match_egg_id(rt).map(|e| e.id), Some(want), "should have matched {rt:?}");
        }
    }

    /// AND NONE OF THEM MATCHES A SUBSTRING INSIDE ANOTHER WORD, which is the
    /// rule the AC/DC advert taught. Every row inherits it, so every row is
    /// asked.
    #[test]
    fn no_theme_matches_inside_another_word() {
        for rt in [
            "the beatlesque new single",
            "zeppelinesque",
            "nirvanas",
            "sunirvana",
            "ninety nine inch nailsmith",
        ] {
            assert_eq!(match_egg_id(rt), None, "should NOT have matched {rt:?}");
        }
    }

    /// THREE OF THE FIVE CHANGE NO COLOUR AT ALL, and that is data rather than
    /// an oversight: their `accent`/`glow`/`chromeInk` restate the live tokens,
    /// which is the registry's way of writing "leave the palette alone". A skin
    /// appearing on one of them would repaint a face the design leaves untouched.
    #[test]
    fn only_two_themes_restate_a_palette() {
        for e in [&ZEPPELIN, &NIRVANA, &NIN] {
            assert_eq!(skin(e, true), NO_SKIN, "{} must not repaint a dark face", e.id);
            assert_eq!(skin(e, false), NO_SKIN, "{} must not repaint a light face", e.id);
        }
        assert_ne!(skin(&ACDC, true), NO_SKIN, "AC/DC restates a dark palette");
        assert_ne!(skin(&BEATLES, true), NO_SKIN, "The Beatles restates a card");
    }

    /// THE BEATLES' CARD IS THE SAME IN BOTH SCHEMES. The registry states `card`
    /// and `rtPlate` on the ENTRY rather than inside `modes`, so a cream drum on
    /// a dark face is the design and not a bug.
    #[test]
    fn the_beatles_card_survives_both_schemes() {
        assert_eq!(skin(&BEATLES, true), skin(&BEATLES, false));
        let k = skin(&BEATLES, true);
        assert_eq!(k.card_bg, Some(0xF3E8D2));
        assert_eq!(k.card_border, Some(0xA81F28));
        assert_eq!(k.rt_serial, "No. 0101538");
        assert_eq!(k.card_rings.len(), 4, "the drum hoop is four rules");
        assert_eq!(k.card_rings[0], (0xF3E8D2, 5.0));
        assert_eq!(k.card_rings[3], (0xA81F28, 18.5));
    }

    /// A FACE IS A FAMILY NAME THE FILE DECLARES, not the key the reference
    /// registers it under. Copying `bandThemes.ts`'s keys would have missed every
    /// one of them silently — the engine falls back rather than failing — so the
    /// names are pinned here against what `ui/tokens.slint` imports.
    #[test]
    fn the_faces_are_the_names_the_files_declare() {
        assert_eq!(BEATLES.body_face, "YellowSubmarine", "not 'BeatlesYellowSub'");
        assert_eq!(BEATLES.genre_face, "Madie Roger", "not 'MadieRoger'");
        assert_eq!(NIN.body_face, "FoundryGridnik", "not 'Gridnik'");
        assert_eq!(NIRVANA.body_face, "Permanent Marker", "not 'PermanentMarker'");
        assert_eq!(NIRVANA.hero_face, "Onyx");
        assert_eq!(ZEPPELIN.hero_face, "Kashmir");
        assert_eq!(NIN.genre_face, "Singothic");
        // AND THE BEATLES' HERO HAS NONE. The entry names SgtPeppers and the
        // reference vetoes it (`motif !== 'submarine'`), so the hero stays on the
        // ordinary face while the tiles carry the theme's.
        assert_eq!(BEATLES.hero_face, "", "§4: the SgtPeppers cut is unfinished");
        // LED ZEPPELIN SCOPES ITSELF TO THE HERO. `fontScope: 'hero'` — a display
        // cut at tile size is unreadable, and the theme reads from the hero.
        assert_eq!(ZEPPELIN.body_face, "", "fontScope: 'hero'");
    }

    /// ONE GEAR EACH, AND LED ZEPPELIN KEEPS THE ORDINARY ONE — it replaces the
    /// vehicle-in-motion tell instead, which is `bandArt.tsx`'s own arrangement
    /// (`GEAR_BY_MOTIF` has no `runes` entry). The two must not both fire.
    #[test]
    fn the_gear_and_the_airship_are_alternatives() {
        for (e, gear, airship) in [
            (&ACDC, Gear::Bolt, false),
            (&BEATLES, Gear::Drum, false),
            (&ZEPPELIN, Gear::Plain, true),
            (&NIRVANA, Gear::Smiley, false),
            (&NIN, Gear::Spiral, false),
        ] {
            assert_eq!(e.gear, gear, "{}", e.id);
            assert_eq!(e.airship, airship, "{}", e.id);
            assert!(
                !(e.gear != Gear::Plain && e.airship),
                "{} must take the gear or the motion slot, not both",
                e.id
            );
        }
    }

    /// ONE THEME BANDS ITS CALL SIGNS, and the same shape of test as the bolts
    /// above, for the same reason: `heroGlitch` reaches every call sign on the
    /// face — hero, peek cards and preset tiles — so a second row picking it up
    /// would rebuild every label on the strip without anyone asking.
    #[test]
    fn only_nine_inch_nails_bands_its_call_signs() {
        assert_eq!(
            all()
                .into_iter()
                .filter(|e| e.hero_glitch)
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec!["Nine Inch Nails"],
        );
        const { assert!(!PLAIN.hero_glitch, "the base states no heroGlitch") };
        // §2.5 states the bands and the off-register ghost as two effects on the
        // same row, one over the other. The handoff pairs them; this holds them
        // together, so neither can be dropped while the other stays.
        const { assert!(NIN.ghost_alpha > 0.0, "heroGlitch needs a ghost to slip") };
        assert_eq!(NIN.ghost_dx, 2.0);
        assert_eq!(NIN.ghost_dy, 0.0);
    }

    /// TWO THEMES PRINT A SECOND IMPRESSION, and since `nameGhost` now reaches
    /// the preset call signs as well as the hero lettering, which rows state it
    /// is no longer a detail of one card. A third row picking it up would put a
    /// drop shadow on every tile on the face without anyone asking.
    #[test]
    fn the_ghost_belongs_to_nirvana_and_nine_inch_nails() {
        assert_eq!(
            all()
                .into_iter()
                .filter(|e| e.ghost_alpha > 0.0)
                .map(|e| (e.id, e.ghost_dx, e.ghost_dy))
                .collect::<Vec<_>>(),
            vec![("Nirvana", 3.0, 3.0), ("Nine Inch Nails", 2.0, 0.0)],
        );
        // A ghost with no offset is the letters printed twice in the same place,
        // which is not an impression off register — it is just darker type.
        for e in all().into_iter().filter(|e| e.ghost_alpha > 0.0) {
            assert!(e.ghost_dx != 0.0 || e.ghost_dy != 0.0, "{}", e.id);
        }
    }

    /// THE BOLTS BELONG TO ONE THEME, and this test exists because they were
    /// gated on "a theme is showing" and appeared on all five. The reference
    /// gates on the ART BEING NAMED — `egg?.stereoArtL ? … : null` — and only the
    /// AC/DC row names `assets/fan-l2.png`, so every other theme keeps the
    /// ordinary speaker cones.
    #[test]
    fn only_acdc_replaces_the_stereo_cones() {
        assert_eq!(
            all()
                .into_iter()
                .filter(|e| e.stereo_bolts)
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec!["AC/DC"],
        );
        // And the base carries none, so an unthemed face cannot reach them
        // through a row that forgot to state it.
        const { assert!(!PLAIN.stereo_bolts, "the base states no stereoArtL") };
    }

    /// A THEMED HERO TAKES NO MORE VERTICAL ROOM THAN AN ORDINARY ONE, which is
    /// what `HeroCard`'s fit enforces at draw time against measured line boxes.
    /// The registry side of it is that `heroScale` is a SCALE-UP and never a
    /// shrink: a value below 1 would ask the fit to grow type, which it cannot
    /// do, and would silently render at the ordinary size instead.
    #[test]
    fn hero_scale_never_asks_for_less_than_the_ordinary_size() {
        for e in [&ACDC, &BEATLES, &ZEPPELIN, &NIRVANA, &NIN] {
            assert!(e.hero_scale >= 1.0, "{} heroScale {}", e.id, e.hero_scale);
        }
        assert_eq!(ZEPPELIN.hero_scale, 1.3);
        assert_eq!(NIRVANA.hero_scale, 1.5);
        assert_eq!(NIN.hero_scale, 1.3);
        // The two that state none sit at the ordinary size, so the fit is a
        // no-op for them — which is what the shots show.
        assert_eq!(ACDC.hero_scale, 1.0);
        assert_eq!(BEATLES.hero_scale, 1.0);
    }

    /// THE THREE BASIC ROWS MATCH THE WAY A STATION WRITES THEM.
    ///
    /// Each is checked against the spellings that actually turn up in RadioText
    /// — surname alone, full name, with and without a leading "The" — and
    /// against the near-misses that must NOT fire. The last two lines are the
    /// hazard the registry's own note describes, held here so nobody has to take
    /// the note on trust.
    #[test]
    fn the_basic_bands_match_the_way_a_station_writes_them() {
        for (rt, want) in [
            ("Eric Clapton - Layla", "Eric Clapton"),
            ("CLAPTON / Cocaine", "Eric Clapton"),
            ("The Pretty Reckless - Death by Rock and Roll", "The Pretty Reckless"),
            ("Pretty Reckless, Heaven Knows", "The Pretty Reckless"),
            ("The Who - Baba O'Riley", "The Who"),
            ("now playing: the who", "The Who"),
        ] {
            assert_eq!(match_egg_id(rt).map(|e| e.id), Some(want), "{rt:?}");
        }

        // WHOLE TOKENS, so a longer word containing the name is not the name —
        // and, the near-miss the owner asked about, ANOTHER BAND WHOSE NAME
        // CONTAINS BOTH OF THE WHO'S WORDS. " the guess who " does not contain
        // " the who ", because the search wants the two tokens adjacent.
        for rt in [
            "Claptone - No Eyes",
            "recklessly cheap tyres",
            "whoever calls first",
            "The Guess Who - American Woman",
            "Guess Who's Back",
        ] {
            assert_eq!(match_egg_id(rt), None, "{rt:?} must not match");
        }

        // "reckless" ALONE IS NOT A MATCH, which is the whole reason the row
        // names two words. Advert copy about reckless driving is the exact
        // string this would otherwise fire on.
        assert_eq!(match_egg_id("cited for reckless driving"), None);

        // AND THE ONE THAT DOES FIRE ON PROSE, asserted rather than hoped about.
        // The owner has settled it — RadioText is station, artist and track copy
        // rather than arbitrary English — so this records the behaviour instead
        // of warning about it, and is the line that fails the day anybody adds a
        // guard.
        assert_eq!(
            match_egg_id("find out the who, what and where").map(|e| e.id),
            Some("The Who"),
            "the prose match is the accepted behaviour, not an oversight"
        );
    }

    /// A SONG THEME, AND THE FIRST BASIC ROW WITH A FACE.
    ///
    /// Two things worth pinning that no other row exercises. The tier took a
    /// SONG without changing — nothing in the matcher ever cared whether a name
    /// was an artist's — and the face lands on the genre line AND the RadioText
    /// from one argument, which is the tier's whole rule about fonts and had
    /// until now been asserted only against a `cfg(test)` fixture.
    #[test]
    fn the_song_theme_dresses_two_lines_from_one_face() {
        assert_eq!(match_egg_id("Kansas - Carry On Wayward Son").map(|e| e.id), Some("Carry On Wayward Son"));
        // The short spelling, which is why the row names the two-word form.
        assert_eq!(match_egg_id("wayward son").map(|e| e.id), Some("Carry On Wayward Son"));
        // Whole tokens still.
        assert_eq!(match_egg_id("waywardson"), None);

        let e = WAYWARD_SON;
        assert_eq!(e.tier, Tier::Basic, "a song theme is basic, and never listed");
        assert_eq!(e.genre, "Season Finale");
        assert_eq!(e.genre_face, FACE_KNIGHT, "the face dresses the genre line");
        assert_eq!(e.rt_face, FACE_KNIGHT, "and the RadioText, from the same argument");
        assert_eq!(e.hero_face, "", "and NOT the hero");
        assert_eq!(e.body_face, "", "nor the preset tiles");
        assert_eq!(e.freq_face, "", "nor the dial");

        // "IN BOLD", AND THE FONT HAS NO BOLD CUT. `SupernaturalKnight.ttf` is
        // usWeightClass 400 with a "Regular" subfamily, so asking the engine for
        // 700 returns that one cut unchanged. The thickening is drawn instead —
        // see `Egg::face_bold` — and it reaches BOTH lines the face sets, which
        // is why the field is not called `genre_bold`.
        assert!(e.face_bold > 0.0, "the row asks for a synthetic bold");
        assert_eq!(e.face_bold, 0.02);

        // Nothing else at all: the whole row is its id, its genre, its face and
        // that bold.
        assert_eq!(
            Egg { id: "", genre: "", genre_face: "", rt_face: "", face_bold: 0.0, ..e },
            PLAIN
        );
        assert!(!listed().iter().any(|l| l.id == e.id));
    }

    /// THE THREE ARE GENRE-ONLY, and none of them dresses anything else.
    ///
    /// The owner named a genre for each and no font, so every face field must
    /// stay empty — a font arriving later is a deliberate edit, not a drift.
    #[test]
    fn the_basic_bands_carry_a_genre_and_no_font() {
        for e in [&CLAPTON, &PRETTY_RECKLESS, &THE_WHO] {
            assert_eq!(e.tier, Tier::Basic, "{}", e.id);
            assert!(!e.genre.is_empty(), "{} has no genre", e.id);
            assert!(faces_used(e).is_empty(), "{} names a face", e.id);
            // And nothing else: the whole row is its id and its genre.
            assert_eq!(Egg { id: "", genre: "", ..*e }, PLAIN, "{}", e.id);
        }
        assert_eq!(CLAPTON.genre, "Slowhand");
        assert_eq!(PRETTY_RECKLESS.genre, "Cindy-Lou Who?");
        assert_eq!(THE_WHO.genre, "Meaty, Beaty, Big, and Bouncy");
    }

    /// Punctuation becomes a separator, never nothing — the difference between
    /// "AC/DC" matching and only ever matching the run-together spelling.
    #[test]
    fn punctuation_separates_rather_than_vanishing() {
        assert_eq!(normalize_rt("AC/DC"), "ac dc");
        assert_eq!(normalize_rt("Hi-Fi, 2026!"), "hi fi  2026 ");
    }

    // ── THE TWO TIERS ────────────────────────────────────────────────────────

    /// THE FIVE ORIGINALS ARE ADVANCED AND NOTHING ELSE IS.
    ///
    /// The owner's words: *"All currently defined band Easter Eggs are now
    /// considered 'advanced' Easter Eggs."* Asserted by NAME rather than by
    /// counting, so adding a sixth advanced theme is a deliberate edit here and
    /// adding a basic one is not an edit at all.
    #[test]
    fn the_five_original_bands_are_the_advanced_tier() {
        let advanced: Vec<&str> = all()
            .into_iter()
            .filter(|e| e.tier == Tier::Advanced)
            .map(|e| e.id)
            .collect();
        assert_eq!(
            advanced,
            ["AC/DC", "The Beatles", "Led Zeppelin", "Nirvana", "Nine Inch Nails"]
        );
    }

    /// EVERY LISTED ROW HAS A LABEL, AND NO UNLISTED ROW HAS ONE.
    ///
    /// The label is a PUN and the id is the key, kept apart because the reference
    /// keeps them apart. A listed row without one is a blank line in the picker
    /// — which is what the driver sees, since the picker draws `menu` and never
    /// the id — and an unlisted row with one is a string nothing can ever read.
    ///
    /// The second half is why this is not just a spelling check: `basic` builds
    /// from `PLAIN`, so a basic row cannot acquire a label by forgetting, and
    /// this fails the day somebody writes one out by hand.
    #[test]
    fn every_listed_theme_has_a_menu_label() {
        for e in listed() {
            assert!(!e.menu.is_empty(), "{} is listed with no label to draw", e.id);
            assert_ne!(e.menu, e.id, "{}'s label is its id — the labels are puns", e.id);
        }
        for e in all() {
            if e.tier == Tier::Basic {
                assert!(e.menu.is_empty(), "{} is unlisted and carries a label", e.id);
            }
        }
    }

    /// THE PICKER'S CHOICE BEATS THE RADIOTEXT, AND SILENCE BEATS THE PICKER.
    ///
    /// `matchEggId` returns a forced id before it normalises anything
    /// (`bandThemes.ts:197`) — a driver looking at the five themes should not have
    /// to wait for the right track. `resolveEgg` checks `off` before calling it at
    /// all, which is the order kept here: the flat grey face is a STATE, and a
    /// forced theme must not dress it.
    #[test]
    fn a_forced_theme_wins_over_the_text_but_not_over_silence() {
        // Nothing playing that matches, forced to AC/DC: the theme shows.
        let forced = resolve("Traffic and weather together", false, Some("AC/DC"));
        assert_eq!(forced.map(|e| e.id), Some("AC/DC"), "the picker overrides the text");

        // Playing Nirvana, forced to AC/DC: the picker still wins.
        let over = resolve("Nirvana - Lithium", false, Some("AC/DC"));
        assert_eq!(over.map(|e| e.id), Some("AC/DC"), "and it overrides a real match");

        // Forced, but the audio priority is gone: flat and grey, no theme.
        assert!(resolve("Nirvana - Lithium", true, Some("AC/DC")).is_none(), "silence wins");

        // Off again, and the text is back in charge.
        assert_eq!(
            resolve("Nirvana - Lithium", false, None).map(|e| e.id),
            Some("Nirvana"),
            "with nothing forced the text decides"
        );
        assert_eq!(
            resolve("Nirvana - Lithium", false, Some("")).map(|e| e.id),
            Some("Nirvana"),
            "and an empty id is 'nothing forced', not 'force nothing'"
        );

        // A stored id no row answers to resolves to NOTHING rather than falling
        // through to the text — a theme deleted under a saved preference must not
        // silently become whichever band is playing.
        assert!(
            resolve("Nirvana - Lithium", false, Some("Some Deleted Band")).is_none(),
            "an unknown forced id shows no theme at all"
        );
    }

    /// THE PICKER LISTS THE ADVANCED ONES AND ONLY THOSE.
    ///
    /// This is the whole mechanical content of "basic themes don't get a
    /// listing". [`listed`] is the one place the rule is enforced — the picker
    /// draws what it returns and does not filter again — so it has to be enforced
    /// there or it is a sentence in a comment.
    #[test]
    fn the_hidden_picker_never_lists_a_basic_theme() {
        let listed = listed();
        assert!(!listed.is_empty(), "the picker would have nothing to show");
        for e in &listed {
            assert_eq!(e.tier, Tier::Advanced, "{} is listed and is not advanced", e.id);
        }
        for e in all() {
            if e.tier == Tier::Basic {
                assert!(
                    !listed.iter().any(|l| l.id == e.id),
                    "{} is basic and reached the picker",
                    e.id
                );
            }
        }
    }

    /// A BASIC THEME TOUCHES EXACTLY TWO THINGS, AND THE FONT TOUCHES TWO PLACES.
    ///
    /// The owner's rule for the tier, stated as a test because `basic` is a
    /// constructor and a constructor is easy to extend by accident: *"Basic
    /// Easter eggs have one or two out of two items: 1- A custom Genre. 2- A
    /// custom font. Custom fonts, when defined, will be used for both the genre
    /// and radio text."*
    ///
    /// The negative half is the important half. `hero_face`, `body_face` and
    /// `freq_face` are the three the font must NOT reach, and a row that set
    /// them would be an advanced theme wearing a basic row's paperwork.
    /// PLAIN STATES NO SYNTHETIC BOLD, and this test exists because it briefly
    /// did. A stray edit put `face_bold: 0.02` on the BASE every theme diffs
    /// against rather than on the one row that asked for it, which would have
    /// thickened the genre line of every theme on the face — and of the
    /// unthemed PTY, since `EggTheme::default()` is built from the same zeroes.
    /// Nothing on screen would have named the cause.
    #[test]
    fn the_base_theme_synthesises_no_bold() {
        const { assert!(PLAIN.face_bold == 0.0, "the base states no synthetic bold") };
        // And exactly one row does.
        let bolded: Vec<&str> =
            all().into_iter().filter(|e| e.face_bold > 0.0).map(|e| e.id).collect();
        assert_eq!(bolded, ["Carry On Wayward Son"]);
    }

    #[test]
    fn a_basic_theme_is_a_genre_a_face_and_nothing_else() {
        let both = basic("Fixture", "Sludge", FACE_ONYX);
        assert_eq!(both.tier, Tier::Basic);
        assert_eq!(both.genre, "Sludge");
        assert_eq!(both.genre_face, FACE_ONYX, "the font dresses the genre line");
        assert_eq!(both.rt_face, FACE_ONYX, "and the RadioText, from the same one");
        assert_eq!(both.hero_face, "", "and NOT the hero");
        assert_eq!(both.body_face, "", "nor the preset tiles");
        assert_eq!(both.freq_face, "", "nor the dial");

        // Everything else is PLAIN: no palette, no marks, no ornament, and the
        // logo stays on the card.
        assert_eq!(Egg { id: "", genre: "", genre_face: "", rt_face: "", ..both }, PLAIN);

        // ONE OF THE TWO IS ENOUGH, both ways round.
        let genre_only = basic("Genre Only", "Skiffle", "");
        assert_eq!(genre_only.genre, "Skiffle");
        assert_eq!(genre_only.rt_face, "", "no font named, no font applied");

        let font_only = basic("Font Only", "", FACE_KASHMIR);
        assert_eq!(font_only.genre, "", "no line of its own — the PTY is dressed instead");
        assert_eq!(font_only.genre_face, FACE_KASHMIR);
        assert_eq!(font_only.rt_face, FACE_KASHMIR);
    }

    /// A BASIC THEME HAS TO ACTUALLY DO SOMETHING.
    ///
    /// `basic(id, "", "")` compiles and is a theme that matches a band and then
    /// changes nothing on the face — indistinguishable from no theme at all
    /// except that it suppresses any later row that would have matched. The
    /// constructor cannot refuse it, so the registry is checked instead.
    #[test]
    fn a_basic_theme_has_to_actually_do_something() {
        let mut seen = 0;
        for (egg, _) in basic_rows() {
            assert!(
                !egg.genre.is_empty() || !egg.genre_face.is_empty(),
                "{} states neither a genre nor a font",
                egg.id
            );
            seen += 1;
        }
        assert!(seen >= 2, "the fixtures are missing, so this asserted nothing");
    }

    /// A BASIC ROW RESOLVES THROUGH THE LIVE MATCHER, AND AN ADVANCED ONE WINS.
    ///
    /// The tier is not a second lookup: basic rows go through [`match_egg_id`]
    /// exactly as the five do, with the same whole-token rule, so the same field
    /// failure is guarded the same way. What the two lists buy is PRECEDENCE —
    /// [`registry`] chains advanced first, so RadioText naming both dresses the
    /// whole face rather than restyling one line.
    #[test]
    fn a_basic_row_resolves_and_yields_to_an_advanced_one() {
        assert_eq!(match_egg_id("Fixture Genre - Some Track").map(|e| e.id), Some("Fixture Genre"));
        assert_eq!(match_egg_id("now playing fixture face").map(|e| e.id), Some("Fixture Face"));
        // Whole tokens, for a basic row too.
        assert_eq!(match_egg_id("prefixture genre"), None);
        // Both named: the advanced dress wins wherever it sits in the string.
        assert_eq!(
            match_egg_id("Fixture Genre and Nirvana").map(|e| e.id),
            Some("Nirvana"),
            "an advanced theme outranks a basic one"
        );
        // And a basic row is reachable but never listed.
        assert!(all().iter().any(|e| e.id == "Fixture Genre"));
        assert!(!listed().iter().any(|e| e.id == "Fixture Genre"));
    }

    /// NO BASIC NAME IS SHORT ENOUGH TO FIRE ON PROSE.
    ///
    /// [`match_egg_id`] carries the field failure this guards against: an advert
    /// for "Hometown HVAC DC power" repainted CarFM's whole face as AC/DC, and
    /// this matcher runs against rotating advert copy rather than a slogan. Five
    /// long names were nearly safe. A long tail is not — "Yes", "Free", "Air",
    /// "Bread", "War", "Cream" are all real bands and all ordinary English.
    ///
    /// SIX CHARACTERS IS A FLOOR, NOT A GUARANTEE, and it is deliberately crude:
    /// the honest check is a person reading the name and asking whether a car
    /// dealership could say it. What this stops is the whole class of two- and
    /// three-letter matches getting in without anyone noticing.
    #[test]
    fn no_basic_name_is_short_enough_to_fire_on_prose() {
        for (egg, names) in basic_rows() {
            for n in *names {
                assert!(
                    n.len() >= 6,
                    "{}: {n:?} is short enough to appear in advert copy",
                    egg.id
                );
                assert_eq!(*n, normalize_rt(n).trim(), "{}: {n:?} is not normalised", egg.id);
            }
        }
    }

    /// EVERY FACE A THEME NAMES IS BUNDLED, AND EVERY BUNDLED FILE IS IMPORTED.
    ///
    /// The failure this exists for is silent at every stage. Slint resolves
    /// `font-family` against the family a file declares; a family nobody
    /// imported is not an error, it is Atkinson. So a theme naming a font that
    /// is not in `ui/fonts/` — or one that is there and was never imported in
    /// `ui/tokens.slint` — renders as "the font didn't work", on a unit with no
    /// way to ask why.
    ///
    /// BOTH ENDS, because either alone leaves a hole: the table could name a
    /// file that does not exist, or `tokens.slint` could import a file the table
    /// does not know about. The file list is read from the source rather than
    /// restated here.
    #[test]
    fn every_face_a_theme_names_is_bundled_and_imported() {
        let tokens = include_str!("../ui/tokens.slint");
        for (family, file) in BUNDLED_FACES {
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("ui/fonts")
                    .join(file)
                    .is_file(),
                "{family} names {file}, which is not in ui/fonts/"
            );
            assert!(
                tokens.contains(&format!("import \"fonts/{file}\";")),
                "{file} is bundled but ui/tokens.slint never imports it, so {family} \
                 silently resolves to Atkinson"
            );
        }
        for egg in all() {
            for face in faces_used(egg) {
                assert!(
                    BUNDLED_FACES.iter().any(|(family, _)| *family == face),
                    "{} names {face:?}, which is not in BUNDLED_FACES",
                    egg.id
                );
            }
        }
    }
}
