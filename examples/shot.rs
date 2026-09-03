//! Render the face at every target surface and write PNGs.
//!
//! The head unit is the only real target, but waiting for a car to see whether a
//! layout is right is not a workflow. This drives Slint's software renderer
//! directly — no window system, no GPU — at each of the five surfaces in ANDROID
//! §2, so the face can be diffed against the design references from a terminal.
//!
//!     cargo run --example shot
//!
//! Output lands in `shots/`.

//! ## COMPARING SHOTS: NOT WITH GIT, AND NOT ALL OF THEM
//!
//! `/shots` is in `.gitignore` — these are output, not source — so
//! `git status shots/` reports nothing whatever changes, and a check built on it
//! is vacuous. It has been used as one; it proves nothing.
//!
//! Compare a copy instead, and compare PIXELS rather than checksums: identical
//! images have come out with different PNG bytes here. `tools/cmp-shots.py` is
//! that comparison — it reads RAW RGBA BYTES, because PIL's `getbbox()` on an
//! RGBA difference looks at the ALPHA CHANNEL ALONE and once called 63 of 63
//! shots identical across a change that repainted a whole card.
//!
//!     cp -r shots /tmp/base   # before
//!     tools/cmp-shots.py /tmp/base shots hero-step-morph.png …
//!
//! A HANDFUL OF SHOTS ARE NOT DETERMINISTIC and will differ between two runs of
//! an unchanged tree — they capture a moment in something that moves:
//! `hero-step-morph` (a frame mid-travel), `logo-search-loading` (the spinner),
//! `audio-released` (the power button's ring, by a single LSB), `nearby-loading`,
//! `nav-cruise`, `nav-approach`, `nav-turn-now` and their portraits, `driving` (the
//! vehicle-in-motion tell pulses on a 2.6s beat — added to this list after a
//! clock change appeared to move it and two renders from ONE build showed it
//! moving on its own), and
//! `settings-diagnostics-open`/`-full` (the log carries wall-clock stamps).
//! `nearby-loading` is a spinner like `logo-search-loading` and belongs on that
//! list too; it was found by a full comparison rather than added when it was
//! written. The three `nav-*` shots put the vehicle in motion, so they inherit
//! `driving`'s 2.6s pulse and move by the same handful of levels.
//!
//! Their ETA does NOT drift, and that took arranging: it is derived from the
//! real wall clock against a forced local reading, so a FIXED arrival epoch
//! would have rendered a different time every minute. The arrival is set
//! relative to `now`, which makes the difference — and therefore the printed
//! ETA — the same on every run.
//! `long-radiotext` drifts too when the marquee is mid-scroll, and so do the
//! THEMED shots whose genre line pulses — `acdc`, `acdc-portrait`, `beatles-dark`
//! have all been seen to move by a dozen levels between two runs of one build,
//! which is the 900ms `GenreText` animation caught at a different phase and not a
//! change. Everything else
//! reproduces exactly, which is what makes the set usable as a regression check
//! at all — and the way to tell the two apart is to render twice from ONE build
//! and compare those, which is how `audio-released` was added to this list.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, Model, ModelRc, PhysicalSize, PlatformError, VecModel,
};

use carnyx::app::App;
use carnyx::fake::FakeLocation;
use carnyx::logos::dark::stages::Treatment;
use carnyx::{GenreColumn, LogoPlate, LogoSearchState, NearbyState, NearbyTab, Overlay};

/// The five first-class surfaces of ANDROID §2, plus the states worth a second
/// look: (name, width, height, dark, state).
const SURFACES: &[(&str, u32, u32, bool, State)] = &[
    ("head-unit-light", 1024, 614, false, State::Normal),
    ("head-unit-dark", 1024, 614, true, State::Normal),
    ("slice-two-thirds", 900, 810, false, State::Normal),
    ("slice-one-third", 470, 845, false, State::Normal),
    ("phone-landscape", 800, 360, false, State::Normal),
    ("phone-portrait", 360, 800, false, State::Normal),
    ("phone-portrait-dark", 360, 800, true, State::Normal),
    ("audio-released", 1024, 614, false, State::AudioReleased),
    ("tuner-error", 1024, 614, false, State::TunerError),
    ("driving", 1024, 614, false, State::Driving),
    // §4.9's maneuver layer at stage 1, on both tracks. Driven through the real
    // nav seam so the arrow, the distance and the ETA are the ones the device
    // would compute, not properties pushed past the logic.
    ("nav-cruise", 1024, 614, false, State::NavCruise),
    ("nav-cruise-portrait", 360, 800, false, State::NavCruise),
    ("nav-approach", 1024, 614, false, State::NavApproach),
    ("nav-approach-portrait", 360, 800, false, State::NavApproach),
    ("nav-turn-now", 1024, 614, false, State::NavTurnNow),
    ("nav-turn-now-portrait", 360, 800, false, State::NavTurnNow),
    ("nav-poll-only", 1024, 614, false, State::NavPollOnly),
    ("weak-and-lossy", 1024, 614, false, State::WeakAndLossy),
    ("no-presets", 1024, 614, false, State::NoPresets),
    // A LONG STRIP. Both save paths used to cap the list at six and delete the
    // oldest preset to make room, so nothing had ever rendered more than six.
    ("many-presets", 1024, 614, false, State::ManyPresets),
    ("many-presets-portrait", 360, 800, false, State::ManyPresets),
    // §8's looping rail, and the pair is the point: the same switch, one strip
    // long enough for §8.1 to honour it and one that is not.
    ("preset-loop", 1024, 614, false, State::PresetLoop),
    // DARK IS THE CASE §8.3 WAS DESIGNED FOR: "an inset-shadow-only cue
    // disappears at low ambient brightness", which is why both of the seam's
    // rules stay solid. This is the shot that says whether they do.
    ("preset-loop-dark", 1024, 614, true, State::PresetLoop),
    ("preset-loop-declined", 1024, 614, false, State::PresetLoopDeclined),
    ("tuned", 1024, 614, false, State::Tuned),
    ("tuned-portrait", 360, 800, false, State::Tuned),
    ("out-of-band", 1024, 614, false, State::OutOfBand),
    ("long-radiotext", 1024, 614, false, State::LongRadioText),
    ("no-callsign", 1024, 614, false, State::NoCallsign),
    ("stereo-unknown", 1024, 614, false, State::StereoUnknown),
    ("long-genre", 1024, 614, false, State::LongGenre),
    // The satellite icon has two states and only one of them was ever shot. Both
    // go through `App::set_position`, which is the seam a real LocationManager
    // callback lands on — not a property override.
    // Tuned OFF the strip: the unsaved star, and the peek cards falling back to
    // last/first because no tile is active.
    ("unsaved-dial", 1024, 614, false, State::UnsavedDial),
    ("gps-no-fix", 1024, 614, false, State::NoGpsFix),
    ("gps-no-fix-portrait", 360, 800, false, State::NoGpsFix),
];

/// The four overlays of ANDROID §6, and the states each one reports.
///
/// Every one is shot on the head unit AND on a narrow surface, because §6's
/// sizing rule (`min(designW, screenW − 32dp)`, then centre, then let the body
/// scroll) and §6.2's 620dp width track are the two places this breaks — a card
/// that fits 1024×614 says nothing about a 360dp phone.
///
/// The last field is wheel notches sent at the centre of the surface before the
/// pixels are read, which is how the scrolled shots are taken. The overlays draw
/// no scrollbar — every reference hides its own — so a body that has silently
/// put its last row below the fold looks identical to one that fits until it is
/// actually scrolled.
const OVERLAYS: &[(&str, u32, u32, bool, State, f32)] = &[
    // §6.2's frequency tab — the old §6.1 numpad, folded into the picker by the
    // mini-handoff. The compact HEIGHT track is gone with the standalone card
    // (§4 gives one metric set and a scroll container), so 800×360 is no longer a
    // second branch to photograph — it is the surface where the column has to
    // scroll, which is a different claim and still worth a shot.
    ("freq-tab-head-unit", 1024, 614, false, State::FreqTab, 0.0),
    ("freq-tab-head-unit-dark", 1024, 614, true, State::FreqTab, 0.0),
    ("freq-tab-typing", 1024, 614, false, State::FreqTyping, 0.0),
    ("freq-tab-out-of-band", 1024, 614, false, State::FreqError, 0.0),
    ("freq-tab-phone-portrait", 360, 800, false, State::FreqTyping, 0.0),
    ("freq-tab-phone-landscape", 800, 360, false, State::FreqTyping, 0.0),
    ("freq-tab-phone-landscape-scrolled", 800, 360, false, State::FreqTyping, -6.0),
    // §6.2 nearby picker: the list with the bucket row, the drilled-in genre
    // row, and the bodies that replace the list entirely.
    ("nearby-head-unit", 1024, 614, false, State::Nearby, 0.0),
    ("nearby-head-unit-dark", 1024, 614, true, State::Nearby, 0.0),
    ("nearby-genre-filter", 1024, 614, false, State::NearbyGenre, 0.0),
    ("nearby-scrolled", 1024, 614, false, State::Nearby, -6.0),
    ("nearby-no-gps", 1024, 614, false, State::NearbyNoGps, 0.0),
    ("nearby-loading", 1024, 614, false, State::NearbyLoading, 0.0),
    ("nearby-slice-one-third", 470, 845, false, State::NearbyGenre, 0.0),
    ("nearby-phone-portrait", 360, 800, false, State::Nearby, 0.0),
    // §6.3 settings. The panel is taller than its 576dp card on every surface,
    // so the scrolled shots are the only way to see ADVANCED and the about line.
    ("settings-head-unit", 1024, 614, false, State::Settings, 0.0),
    ("settings-head-unit-dark", 1024, 614, true, State::Settings, 0.0),
    ("settings-scrolled-mid", 1024, 614, false, State::Settings, -2.0),
    ("settings-scrolled-end", 1024, 614, false, State::Settings, -8.0),
    ("settings-diagnostics-open", 1024, 614, false, State::SettingsDiag, -7.0),
    ("settings-diagnostics-full", 1024, 614, false, State::SettingsDiagFull, -9.0),
    // THE ROWS UNDER THE WELL, which the shot above stops short of. This is the
    // one that shows a probe row answering under itself (#126): the same state,
    // scrolled far enough that the well has gone by.
    ("settings-diagnostics-rows", 1024, 614, false, State::SettingsDiagFull, -14.0),
    ("settings-diagnostics-portrait", 360, 800, false, State::SettingsDiag, -11.0),
    ("settings-phone-portrait", 360, 800, false, State::Settings, 0.0),
    ("settings-phone-portrait-scrolled", 360, 800, false, State::Settings, -8.0),
    // The hidden fifth section, which no other shot can reach: it sits BELOW the
    // about line, so it needs the end scroll and then some.
    // §4.8's clock, which no ordinary shot carries: the host has no platform
    // clock, so `android::clock_now` answers None and the readout draws nothing.
    // Three renders — the two formats, and the tall track where it is 36sp.
    ("clock-head-unit", 1024, 614, false, State::Clock12, 0.0),
    ("clock-head-unit-dark", 1024, 614, true, State::Clock12, 0.0),
    ("clock-24-hour", 1024, 614, false, State::Clock24, 0.0),
    ("clock-portrait", 360, 800, false, State::Clock12, 0.0),
    ("settings-band-themes", 1024, 614, false, State::SettingsEggs, -11.0),
    ("settings-band-themes-dark", 1024, 614, true, State::SettingsEggs, -11.0),
    ("settings-band-themes-portrait", 360, 800, false, State::SettingsEggs, -14.0),
    // §6.4 logo search: both landing views, the grid, and the two dead ends.
    // The dark picker, which no other shot can reach: it opens only on a save,
    // and its swatches are drawn on the real dark ground whatever the face is —
    // which is the whole point of the control and the one thing only a render
    // checks. Both schemes, because the CARD follows the face and the SWATCHES
    // do not.
    ("logo-dark-picker", 1024, 614, false, State::LogoDarkPick, 0.0),
    ("logo-dark-picker-dark", 1024, 614, true, State::LogoDarkPick, 0.0),
    ("logo-dark-picker-portrait", 360, 800, false, State::LogoDarkPick, 0.0),
    ("logo-search-landing", 1024, 614, false, State::LogoLanding, 0.0),
    ("logo-search-landing-with-logo", 1024, 614, false, State::LogoLandingHasLogo, 0.0),
    ("logo-search-results", 1024, 614, false, State::LogoResults, 0.0),
    ("logo-search-results-dark", 1024, 614, true, State::LogoResults, 0.0),
    ("logo-search-loading", 1024, 614, false, State::LogoLoading, 0.0),
    ("logo-search-no-results", 1024, 614, false, State::LogoNoResults, 0.0),
    ("logo-search-error", 1024, 614, false, State::LogoError, 0.0),
    ("logo-search-landing-phone", 360, 800, false, State::LogoLanding, 0.0),
    ("logo-search-results-phone", 360, 800, false, State::LogoResults, 0.0),
    // The entry point the logo window opens from: the per-tile badge, which
    // exists only in reorder mode (§6.4 block 0).
    ("reorder-logo-badges", 1024, 614, false, State::Reordering, 0.0),
    ("reorder-logo-badges-portrait", 360, 800, false, State::Reordering, 0.0),
    // A drag in flight: tile 0 held, the finger between slots 2 and 3, the gap
    // open behind it. Driven by real pointer events — see the DRAG block below.
    ("reorder-drag", 1024, 614, false, State::Dragging, 0.0),
    // The §8 step morph, caught mid-travel: the hero has left the right-hand
    // peek slot and is most of the way to the centre. Driven by the real
    // `step-preset` callback — see the STEP block below.
    ("hero-step-morph", 1024, 614, false, State::Stepping, 0.0),
    // THE ONE BAND THEME THAT EXISTS (Design EASTER-EGGS §12). Covered by shots
    // because everything it changes is visual — the horns, the bolt splitting the
    // call sign, the bolts in the pill, the gold genre line, the Squealer
    // lettering on the card and the RadioText — and none of it is reachable from
    // a property assertion. It is also the only place a bundled typeface other
    // than Atkinson is exercised, so a face that stopped loading would show here
    // as boxes rather than as silence.
    //
    // LAST IN THE RUN, ON PURPOSE. Slint's animations run on a wall clock shared
    // by the whole process, so a shot's phase depends on how much work preceded
    // it: adding these in the middle shifted `nearby-loading`'s pulsing dots by
    // enough to fail a pixel comparison, with nothing about that shot changed.
    // New shots go on the end, where they cannot move the ones already here.
    ("acdc", 1024, 614, false, State::Acdc, 0.0),
    ("acdc-dark", 1024, 614, true, State::Acdc, 0.0),
    // THE FOUR BACKINGS A DARK-MODE LOGO CAN NEED, side by side. Nothing here is
    // assertable: what a `plate` treatment needs is a grey slab UNDER it, and the
    // failure it guards against — a keyed dark mark drawn on a transparent plate
    // over a near-black card — is an invisible tile, which no property can
    // report.
    //
    // THE LIGHT SHOT IS THE GEOMETRY CONTROL, not a second colour test. Three of
    // the four are the same picture on a pale ground, and `fallback`'s white
    // plate is invisible on it by definition; what the pair proves together is
    // that the BOX does not move — the tiles keep one size across all four, and
    // the hero card is the same card whether or not its logo carries a slab.
    // That is the deviation in `HeroLogo` under test: the reference's plate
    // shrink-wraps and would stand 48dp taller than the card holding it.
    ("logo-plates", 1024, 614, false, State::LogoPlates, 0.0),
    ("logo-plates-dark", 1024, 614, true, State::LogoPlates, 0.0),
    // THE GENRE SHRINK, ON REAL INPUT. A band theme's genre line is 47dp on the
    // tall track, and "High Voltage Rock 'n' Roll" at 47dp is more than twice a
    // 360dp phone. Nothing invented reaches this shot: it is the one theme this
    // app carries, its own string, on a surface the app supports. The line must
    // come down in size until it fits rather than elide to "High Voltage Ro…".
    //
    // It also covers a gap the earlier AC/DC shots left — both were 1024×614, so
    // no shot had ever put a themed genre on the centred track at all.
    ("acdc-portrait", 360, 800, false, State::Acdc, 0.0),
    // THE OTHER FOUR BAND THEMES. Each is a display face, a genre line and a
    // mark or two, and none of it is reachable from a property assertion — the
    // whole point of a theme is what it looks like. Dark as well as light for
    // The Beatles, whose cream card and drum hoop are stated on the ENTRY rather
    // than inside `modes`, so they have to survive both schemes unchanged.
    ("beatles", 1024, 614, false, State::Beatles, 0.0),
    ("beatles-dark", 1024, 614, true, State::Beatles, 0.0),
    ("zeppelin", 1024, 614, false, State::Zeppelin, 0.0),
    ("nirvana", 1024, 614, false, State::Nirvana, 0.0),
    ("nin", 1024, 614, false, State::Nin, 0.0),
    // THE HERO FIT, ON THE TRACK THAT DOES NOT NEED IT. Nirvana carries the
    // largest `heroScale` there is (1.5) and its Onyx is the face that overran
    // the wide card hardest, so it is the one that shows whether the fit stayed
    // a CAP rather than becoming a shrink: the tall card sizes to its own
    // content and has nothing to overflow, and the lettering must still read as
    // a display cut rather than as body type.
    ("nirvana-portrait", 360, 800, false, State::Nirvana, 0.0),
    // A BASIC THEME, which is a different thing to look at and worth one shot.
    // The five above are each a whole dress; this one replaces the genre line
    // and NOTHING ELSE — the hero keeps its ordinary face and its logo, the
    // preset tiles are untouched, the palette is the plain one. What this shot
    // is really for is proving that a row stating a genre and no colour renders
    // in the ordinary dim token, because that is the ordinary case for the tier
    // and it used to render transparent.
    ("clapton", 1024, 614, false, State::Clapton, 0.0),
    // THE SAME TIER WITH A LONG LINE. "Meaty, Beaty, Big, and Bouncy" is 29
    // characters where "Slowhand" is 8, and a themed genre is set at 33/47 where
    // the PTY it replaces is 26/33 — so this is the surface that says whether
    // the top bar's shrink absorbs a basic theme's line or elides it. Two shots
    // for one tier, because they answer different questions.
    ("the-who", 1024, 614, false, State::TheWho, 0.0),
    ("the-who-portrait", 360, 800, false, State::TheWho, 0.0),
    // THE FIRST BASIC THEME WITH A FACE, and the first for a song. What this
    // shot is for is the tier's rule about fonts: one face on the genre line AND
    // the RadioText, and on nothing else — the hero, the tiles and the dial must
    // still be Atkinson beside it.
    ("wayward", 1024, 614, false, State::Wayward, 0.0),
];

#[derive(Clone, Copy, PartialEq)]
enum State {
    Normal,
    /// §4.7 — priority released to another source; the face goes flat and dead.
    AudioReleased,
    /// §4.1 — no compatible tuner; the fault pill replaces the whole OK cluster.
    TunerError,
    /// §4.6 — moving, with a GPS fix, and a traffic announcement running.
    Driving,
    /// §4.9's stage 1 — the cruise hairline, the ETA under the clock and the
    /// countdown between the genre line and the hero card.
    NavCruise,
    /// §4.9's stage 2 — the RadioText strip yields to the maneuver.
    NavApproach,
    /// §4.9's stage 3 — the hero card takes over and the logo moves to its
    /// upper-right corner.
    NavTurnNow,
    /// THE DRIVE THIS BUILD ACTUALLY GETS: a healthy 1 Hz poll and NO push.
    ///
    /// Every other nav state above feeds both channels in the same breath, so
    /// the poll-only frame was never rendered once — and that is exactly how
    /// the maneuver layer shipped gated on the push, which fires only when the
    /// vehicle crosses a route geometry node. This shot must look like
    /// `nav-approach`: same strip yield, same countdown, same ETA.
    NavPollOnly,
    /// A strong carrier arriving in pieces: dotted outer arcs, mono, no RDS.
    WeakAndLossy,
    NoPresets,
    /// Tuned TO a preset: the enlarged tile, its blue border and underline, and
    /// the neighbours the peek cards then show.
    Tuned,
    /// Dial outside 87.5-108.0.
    OutOfBand,
    /// RadioText past the ~46-character marquee threshold.
    LongRadioText,
    /// The built-in tuner with no GPS lock yet: nothing resolves the call sign, so
    /// the frequency stands as the identity — never an inaccurate "Tuning...".
    NoCallsign,
    /// Nothing has reported yet, so the pill is EMPTY rather than asserting MONO.
    StereoUnknown,
    /// THE WIDEST GENRE THE RBDS TABLE CAN ACTUALLY EMIT.
    ///
    /// It used to be "Adult Album Alternative and Classic Rock", which no build
    /// of this app can produce: `rds::pty_label` indexes a fixed table of 31
    /// strings and the longest of them is two words. A shot testing a string the
    /// face cannot receive proves nothing about the face — it only proves the
    /// cap elides SOMETHING, which was never in doubt.
    ///
    /// "Foreign Language", PTY 18. What is under test is whether the real worst
    /// case still fits the 200dp cap now that the type is 175% of what it was.
    LongGenre,
    Acdc,
    /// The four logo backings on one band, plus one on the hero.
    LogoPlates,
    Beatles,
    Zeppelin,
    Nirvana,
    /// A BASIC theme: a genre line and nothing else. See the surface list.
    Clapton,
    /// The same tier, with a genre line long enough to test the shrink.
    TheWho,
    /// A basic theme with a FACE — and for a song rather than a band.
    Wayward,
    Nin,
    /// Reorder mode, where every tile carries the logo-search badge (§6.4).
    Reordering,
    /// EIGHTEEN PRESETS. The strip is not limited, and this is the shot that
    /// says so: the band has to scroll rather than overflow, and the tall
    /// track's three-column grid has to wrap and stay inside its own height cap.
    ManyPresets,
    /// EIGHTEEN PRESETS AND THE RAIL SET TO WRAP (§8), nudged sideways so a seam
    /// is on screen.
    ///
    /// THE NUDGE IS A REAL GESTURE, for the reason `Dragging` gives: the loop
    /// lives inside the Flickable and is reachable only by scrolling it. A shot
    /// that set `viewport-x` would prove the copies can be drawn, not that a
    /// finger can reach them.
    PresetLoop,
    /// The same switch on a strip that does not overflow. §8.1 refuses, so this
    /// is an ordinary bounded rail with its position bar — the shot that says
    /// turning the setting on does not by itself change the face.
    PresetLoopDeclined,
    /// Reorder mode with a DRAG IN FLIGHT.
    ///
    /// The only state produced by synthetic input rather than by setting a
    /// property, and deliberately so: the drag lives inside `PresetsBand` and is
    /// reachable only through the gesture. A shot that set a flag would prove
    /// the tiles can be drawn displaced, not that a finger can displace them.
    Dragging,
    /// A preset step caught PART WAY THROUGH its morph.
    ///
    /// Rendered by pumping the animation clock and stopping short, which is the
    /// only way to photograph an animation: at rest it is indistinguishable from
    /// a face that never moved.
    Stepping,
    /// No position: the satellite icon's other state.
    NoGpsFix,
    /// A dial that is not one of the six saved slots.
    UnsavedDial,

    // ── §6 overlays ──
    /// The frequency tab as it opens: nothing typed, so the readout is the live
    /// dial, dimmed.
    FreqTab,
    /// Mid-entry. The buffer is raw — "104." is a legitimate readout string and
    /// must NOT be normalised on the way in.
    FreqTyping,
    /// A buffer no further typing can bring into band: the warning line, live.
    FreqError,
    /// §6.2 with rows and the bucket row.
    Nearby,
    /// Drilled into Music, so the genre row replaces the bucket row and one
    /// genre is active.
    NearbyGenre,
    /// A result with no location at all.
    NearbyNoGps,
    /// The fetch in flight with nothing to fall back on.
    NearbyLoading,
    /// §6.3 as it opens.
    Settings,
    /// DIAGNOSTICS on: the four nested switches, the log well with real lines in
    /// it, and the action rows the log's own state decides.
    SettingsDiag,
    /// The same with raw capture on too, which is what adds the export row and
    /// is the only way to see a second action row at all without a head unit.
    SettingsDiagFull,
    /// §6.4 landing on a station with no logo.
    /// The settings panel with BAND THEMES revealed — the section six taps on
    /// the about line unlocks, with one theme forced so the tick has somewhere
    /// to be. The only render of a group that breaks the panel's two-tone rule.
    /// §4.8's clock at 8:05 in the morning — the SINGLE-DIGIT hour, which is the
    /// case the blank-digit pad exists for.
    Clock12,
    /// The same instant in 24-hour, where it zero-pads and drops the meridiem.
    Clock24,
    SettingsEggs,
    /// §6.4's dark-mode logo picker, the step after a logo is assigned.
    LogoDarkPick,
    LogoLanding,
    /// §6.4 landing on a station that already has one.
    LogoLandingHasLogo,
    /// The 2×2 candidate grid with the second cell picked.
    LogoResults,
    LogoLoading,
    LogoNoResults,
    /// A save failure — the second of the two error wordings.
    LogoError,
}

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

/// Where the synthetic drag starts and ends, on the 1024x614 wide track.
///
/// The end lands BETWEEN slot 2 and slot 3 on purpose. Ending on a slot centre
/// parks the lifted tile exactly over the gap it opened, which looks identical
/// to a strip that never moved — and that is precisely the reading that made a
/// working drag look broken once already.
const DRAG_Y: f32 = 505.0;
const DRAG_FROM_X: f32 = 150.0;
const DRAG_TO_X: f32 = 480.0;

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() }))
        .expect("set platform");
    std::fs::create_dir_all("shots").expect("create shots/");

    // OPTIONAL FILTERS: `cargo run --example shot -- clapton the-who` renders
    // only the surfaces whose name contains one of the arguments. Rendering all
    // eighty to look at one of them is most of a minute, which is enough
    // friction to stop anyone looking — and looking is the whole point of this
    // harness.
    //
    // A FILTER THAT MATCHES NOTHING IS AN ERROR rather than an empty run: a typo
    // would otherwise finish silently and read as success.
    let filters: Vec<String> = std::env::args().skip(1).collect();
    let wanted =
        |name: &str| filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()));
    let mut drawn = 0usize;

    let face = SURFACES.iter().map(|&(n, w, h, d, s)| (n, w, h, d, s, 0.0));
    for (name, w, h, dark, state, scroll) in face.chain(OVERLAYS.iter().copied()) {
        if !wanted(name) {
            continue;
        }
        drawn += 1;
        // A whole application per shot, driven by the real services: the station
        // database is opened and queried, the RDS decoder is fed until its
        // consensus gates clear, the signal maths runs, and the settings panel is
        // derived. The arms below push only what makes THIS shot different.
        //
        // `driver` must outlive the render — it owns every callback, and a face
        // whose callbacks have been dropped still draws but answers nothing.
        // A PREFERENCE DIRECTORY PER SHOT. An App reads and writes `prefs.json`,
        // so one shared directory let a shot that saves a preset change what
        // every later shot started from — see `carnyx::build_with_prefs`.
        let prefs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/carnyx-shots")
            .join(name);
        let _ = std::fs::remove_dir_all(&prefs_dir);
        let (ui, driver) = carnyx::build_with_prefs(&prefs_dir).expect("build window");
        ui.global::<carnyx::Pal>().set_dark(dark);
        apply(&ui, &driver, state);
        window.set_size(PhysicalSize::new(w, h));
        ui.show().expect("show");

        // Settle any `init =>` writes and let the first animation frame land
        // before asking for pixels.
        slint::platform::update_timers_and_animations();
        window.request_redraw();

        let mut buffer = vec![PremultipliedRgbaColor::default(); (w as usize) * (h as usize)];
        let render = |buffer: &mut Vec<PremultipliedRgbaColor>| {
            window.request_redraw();
            let drawn = window.draw_if_needed(|renderer| {
                renderer.render(buffer, w as usize);
            });
            assert!(drawn, "{name}: nothing was rendered");
        };
        render(&mut buffer);

        if scroll != 0.0 {
            // A wheel event over the centre of the card, which is where every
            // overlay puts its scrolling body. One notch is 120 logical px, the
            // step Slint's Flickable takes from a scroll wheel.
            ui.window().dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(w as f32 / 2.0, h as f32 / 2.0),
                delta_x: 0.0,
                delta_y: scroll * 120.0,
            });
            slint::platform::update_timers_and_animations();
            render(&mut buffer);
        }

        if state == State::Dragging {
            // A REAL GESTURE. Press on the first tile, then drag right across
            // two slots. Everything downstream is the shipping path: the
            // TouchArea's grab, the band's 4dp slop, `slot-at`, the inline slot
            // arithmetic and the lifted tile.
            let press = LogicalPosition::new(DRAG_FROM_X, DRAG_Y);
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: press,
                button: slint::platform::PointerEventButton::Left,
            });
            // TWO moves. The first crosses the slop and turns the press into a
            // drag; a single jump would too, but two is what a finger does and
            // it proves the drag survives more than one event.
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new((DRAG_FROM_X + DRAG_TO_X) / 2.0, DRAG_Y),
            });
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(DRAG_TO_X, DRAG_Y) });
            // Slint evaluates a dirty binding lazily, AT RENDER, so the 160ms
            // gap animation does not start until something asks for pixels — one
            // render after a sleep only ever catches its first frame. Pump the
            // clock instead, which is what a running event loop would do.
            for _ in 0..8 {
                slint::platform::update_timers_and_animations();
                render(&mut buffer);
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            slint::platform::update_timers_and_animations();
            render(&mut buffer);
            // NO RELEASE, deliberately. The shot is of a drag in flight, and a
            // release would commit the reorder into `prefs.json` — which would
            // make the next run of this harness start from a different preset
            // order and quietly invalidate every comparison against it.
        }

        if state == State::PresetLoop {
            // NUDGE THE RAIL SIDEWAYS so a seam is on screen.
            //
            // The loop parks the window at the first tile of a turn, which puts
            // the seam that precedes that turn 9px off the left edge — the copies
            // are seamless in the middle and the mark is only ever at a boundary.
            // Dragging the content 60px to the right brings it inside.
            //
            // ONE MOVE OF 60px, which clears the Flickable's 8px capture
            // threshold in a single event, so the gesture is a scroll and never a
            // press on the tile underneath.
            let press = LogicalPosition::new(DRAG_FROM_X, DRAG_Y);
            ui.window().dispatch_event(WindowEvent::PointerPressed {
                position: press,
                button: slint::platform::PointerEventButton::Left,
            });
            ui.window().dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(DRAG_FROM_X + 60.0, DRAG_Y),
            });
            slint::platform::update_timers_and_animations();
            render(&mut buffer);
            // NO RELEASE, for `Dragging`'s reason and one of its own: a release
            // hands the viewport a momentum animation, and where that animation
            // has reached at the moment the shot is taken is a race.
        }

        if state == State::Stepping {
            // The REAL callback, so the tune, the republish and the arming all
            // run in the shipping order.
            ui.invoke_step_preset(1);
            // NO RENDER HERE, deliberately, and it is the condition the device
            // is in. Slint evaluates bindings lazily, at render, so a frame
            // drawn while the morph is armed is what USED to make this work in
            // the harness and never on a head unit busy tuning. Sleep past the
            // 16ms arm timer without drawing anything, so the travel has to
            // survive never having been rendered at its starting point.
            std::thread::sleep(std::time::Duration::from_millis(40));
            // Stop at roughly a third of the 520ms travel. Pumping to the end
            // would photograph the resting face and prove nothing.
            for _ in 0..4 {
                slint::platform::update_timers_and_animations();
                render(&mut buffer);
                std::thread::sleep(std::time::Duration::from_millis(45));
            }
            slint::platform::update_timers_and_animations();
            render(&mut buffer);
        }

        write_png(name, w, h, &buffer);
        ui.hide().expect("hide");
        println!("shots/{name}.png  {w}x{h}");
    }
    assert!(drawn > 0, "no surface matched {filters:?}");
}

/// Push one of the states the face has to be able to draw.
///
/// Every arm that CAN reach the real services does — a preset tap goes through
/// the real tune path, a position change goes through `App::set_position`, the
/// diagnostics switches go through their own callbacks. The handful that cannot
/// say why in place.
fn apply(ui: &carnyx::AppWindow, driver: &Rc<App>, state: State) {
    match state {
        State::Normal => {}
        State::AudioReleased => ui.set_audio_active(false),
        State::TunerError => ui.set_tuner_error(true),
        State::Driving => {
            // The fix and the motion go through the real position seam; TP/TA
            // are pushed, because a traffic announcement needs a broadcaster
            // sending one and the replayed corpus has none.
            driver.set_position(FakeLocation { in_motion: true, ..FakeLocation::default() });
            ui.set_tp(true);
            ui.set_ta(true);
            // THE OSMAND TELL IN ITS LIT STATE (§4.9). Pushed rather than driven
            // through the nav seam, because the host has no OsmAnd to bind to —
            // and this is the state worth holding a picture of: the mark in
            // their own orange between the car and the satellite, with the car
            // moved one slot left to make room for it.
            ui.set_settings_nav_on(true);
            ui.set_nav_linked(true);
        }
        // The three stages share one setup and differ only in the rung OsmAnd
        // reports — which is the claim worth photographing: the same route data
        // moves through three layouts on `next_turn_imminent` alone.
        State::NavCruise | State::NavApproach | State::NavTurnNow | State::NavPollOnly => {
            driver.set_position(FakeLocation { in_motion: true, ..FakeLocation::default() });
            ui.set_settings_nav_on(true);
            // THROUGH THE SEAM `CarnyxNav` CALLS, not by setting properties. A
            // shot that pushed `nav-arrow` directly would render happily with
            // the XML parser, the arrow generator and the stage ladder all
            // disconnected — which is most of what there is to get wrong.
            // §4.8'S CLOCK, WHICH THE ETA HANGS UNDER — and which the ETA
            // cannot be computed without: the timezone offset comes out of the
            // difference between this reading and the epoch second.
            driver.set_clock_for_test(8, 5, false);
            // AN ARRIVAL RELATIVE TO NOW, NOT AN ABSOLUTE INSTANT, and that is
            // what makes this shot reproducible. The offset is derived from the
            // real wall clock against a fixed local reading, so a FIXED arrival
            // epoch would render a different time every minute. Twenty-one
            // minutes from now against a clock reading 8:05 is 8:26, always.
            let arrival_ms = (carnyx::session::now_unix() as i64 + 21 * 60) * 1000;
            let leg = |metres: i32| carnyx::nav::Route {
                arrival_ms: Some(arrival_ms),
                left_seconds: Some(1_260),
                left_metres: Some(9_400),
                map_visible: false,
                street: Some("Whitney Way".into()),
                turn_xml: Some("TR".into()),
                turn_metres: Some(metres),
                // OsmAnd's own rung: -1 cruise, 1 approach, 0 turn now. Zero is
                // the LOUDEST, which is the trap `crate::nav::Stage` documents.
                imminent: Some(match state {
                    // Poll-only stands at stage 2, so this shot is directly
                    // comparable with `nav-approach` — the two must match.
                    State::NavApproach | State::NavPollOnly => 1,
                    State::NavTurnNow => 0,
                    _ => -1,
                }),
                after_street: Some("Odana Rd".into()),
                after_turn_xml: Some("TSLL".into()),
            };
            // TWO POLLS, AND THE FIRST ONE IS THE POINT: the hairline's
            // denominator is the distance FIRST seen for a leg, so a single poll
            // leaves the bar empty however close the turn is. Eight hundred
            // metres back, then two hundred and forty — seven tenths of the way.
            // NO PUSH AT ALL for the poll-only shot — `ingest_nav` is the push
            // seam and this state deliberately never calls it. That is the whole
            // claim: the poll carries the layer by itself.
            let push = state != State::NavPollOnly;
            if push {
                carnyx::android::ingest_nav(800, 5, false);
            }
            carnyx::android::ingest_nav_info(leg(800));
            driver.drain_events();
            if push {
                carnyx::android::ingest_nav(240, 5, false);
            }
            carnyx::android::ingest_nav_info(leg(240));
            driver.drain_events();
            ui.set_nav_linked(true);
        }
        State::WeakAndLossy => {
            ui.set_full_pairs(3);
            ui.set_half(false);
            ui.set_dotted_arcs(2);
            ui.set_level_text("71".into());
            ui.set_stereo(false);
            ui.set_rds(false);
            ui.set_af(false);
            ui.set_pty("".into());
            ui.set_radio_text("".into());
        }
        State::NoPresets => {
            ui.set_presets(slint::ModelRc::default());
            ui.set_has_prev(false);
            ui.set_has_next(false);
        }
        State::ManyPresets => {
            // Through the REAL save path, one dial at a time, so the cap would
            // still be enforced if it were there — setting the model directly
            // would prove only that Slint can draw a long list.
            for step in 0..18 {
                let mhz = 88.1 + (step as f32) * 1.1;
                driver.save_dial_for_test(mhz);
            }
            driver.settle_meter_for_test();
        }
        State::PresetLoop => {
            // The same eighteen dials as `ManyPresets`, through the same save
            // path, on top of the strip the app seeds — enough to overflow the
            // rail by a wide margin, which is what §8.1 needs before it looks at
            // the switch at all.
            for step in 0..18 {
                let mhz = 88.1 + (step as f32) * 1.1;
                driver.save_dial_for_test(mhz);
            }
            driver.settle_meter_for_test();
            // THROUGH THE SETTINGS CALLBACK, not the window property: the
            // property is republished from Rust's own state on every
            // `push_settings`, so a shot that set it directly would have it
            // overwritten by the next push and render a rail that does not loop.
            ui.invoke_settings_set_preset_loop(true);
        }
        State::PresetLoopDeclined => ui.invoke_settings_set_preset_loop(true),
        State::Tuned => {
            ui.invoke_select_preset(2);
            driver.settle_meter_for_test();
        }
        State::OutOfBand => {
            ui.set_in_band(false);
            ui.set_freq_label("76.5".into());
        }
        State::LongRadioText => ui.set_radio_text(
            "NOW PLAYING ON HOT 105.1 — Harry Styles — As It Was — up next Dua Lipa".into(),
        ),
        State::NoCallsign => ui.set_ident("".into()),
        State::StereoUnknown => ui.set_stereo_known(false),
        State::LongGenre => ui.set_pty(carnyx::rds::pty_label(Some(18)).into()),
        // Straight into the decoded RadioText, which is what the theme resolves
        // off, AND THEN A REPUBLISH — this arm runs before the window's first
        // publish, so unlike the arms above it that set a face property directly,
        // a value written into `State` here would never reach the window at all.
        // The first cut of this shot rendered an untouched face for exactly that
        // reason. The station keeps its own logo in the seed, so this also proves
        // `suppress-logo`: without it the card shows art and there is no call
        // sign for the bolt to split.
        State::Acdc => {
            driver.set_radio_text_for_test("AC/DC - Back in Black");
            driver.push_all();
        }
        // The same seam as AC/DC: straight into the decoded RadioText, which is
        // what a theme resolves off, and then a republish — this arm runs before
        // the window's first publish, so a value written into `State` alone would
        // never reach the face.
        State::Beatles => {
            driver.set_radio_text_for_test("The Beatles - Here Comes the Sun");
            driver.push_all();
        }
        State::Zeppelin => {
            driver.set_radio_text_for_test("Led Zeppelin - Kashmir");
            driver.push_all();
        }
        State::Nirvana => {
            driver.set_radio_text_for_test("Nirvana - Smells Like Teen Spirit");
            driver.push_all();
        }
        State::Nin => {
            driver.set_radio_text_for_test("Nine Inch Nails - The Hand That Feeds");
            driver.push_all();
        }
        State::Clapton => {
            driver.set_radio_text_for_test("Eric Clapton - Layla");
            driver.push_all();
        }
        State::TheWho => {
            driver.set_radio_text_for_test("The Who - Baba O'Riley");
            driver.push_all();
        }
        State::Wayward => {
            driver.set_radio_text_for_test("Kansas - Carry On Wayward Son");
            driver.push_all();
        }
        // PROPERTIES, NOT THE STORE, and that is a limit of the host rather than
        // a shortcut. `art_for` needs an `ImageCodec` to turn a stored PNG into
        // pixels, and there is none off the device — `Net` is `None` here, which
        // is why no other shot has ever shown a real logo. So this drives the
        // face the way the READ would have: one synthetic picture, four backings,
        // exactly the four values `logos::assign::read_for_theme` can return.
        //
        // What it proves is the half no test can see — that the four are drawn
        // differently, and that `plate` gets a slab under it. What it does NOT
        // prove is which of the four a given station resolves to; that is
        // `the_dark_face_reads_the_adapted_file_and_the_light_face_does_not`.
        State::LogoPlates => {
            let art = logo_art(2);
            let rows: Vec<carnyx::Preset> = [
                ("LIGHT", 88.7, LogoPlate::Light),
                ("FALLBACK", 90.5, LogoPlate::Fallback),
                ("BARE", 96.3, LogoPlate::Bare),
                ("PLATE", 105.5, LogoPlate::Plate),
            ]
            .iter()
            .map(|&(call, mhz, plate)| carnyx::Preset {
                name: call.into(),
                call: call.into(),
                brand: slint::Color::from_rgb_u8(0x3B, 0x6E, 0x4A),
                // THE GUARD'S ANSWER, not a constant. These tiles all carry a
                // logo, so the ink is never printed here — but hard-coding white
                // would make this shot the one place a fill and its ink could
                // disagree, which is exactly what §5 exists to prevent.
                ink: carnyx::station::ink_on(slint::Color::from_rgb_u8(0x3B, 0x6E, 0x4A)),
                // §13.1's two precomputed numbers, from the real functions rather
                // than literals — a shot that fitted its type by a different rule
                // than the face would be a shot of something that does not ship.
                call_ramp: carnyx::station::call_ramp(call.chars().count()),
                call_cap_div: carnyx::station::call_cap_div(call.chars().count()),
                freq_mhz: mhz,
                freq_label: format!("{mhz:.1}").into(),
                has_logo: true,
                logo: art.clone(),
                plate,
            })
            .collect();
            ui.set_presets(ModelRc::from(Rc::new(VecModel::from(rows))));
            // The hero takes the one backing with geometry of its own — the grey
            // slab is padded by 0.11 of the logo height and rounded by 0.14,
            // neither of which any other surface uses.
            ui.set_has_logo(true);
            ui.set_logo(art);
            ui.set_logo_plate(LogoPlate::Plate);
            ui.set_show_call(false);
            ui.set_show_freq(false);
        }
        // The press below arms the drag with no hold, which is the rule once the
        // mode is already open.
        State::Reordering | State::Dragging => ui.set_reordering(true),
        // Nothing to set up: the step is invoked after the first render, so the
        // morph has a resting frame to travel from.
        State::Stepping => {}
        State::NoGpsFix => driver.set_position(FakeLocation::no_fix()),
        State::UnsavedDial => {
            // Through the tab's own callbacks, so the band check, the buffer
            // rules and the tune all run rather than a property being set.
            for c in ["1", "0", "5", ".", "1"] {
                ui.invoke_freq_key(c.into());
            }
            ui.invoke_freq_commit();
            driver.settle_meter_for_test();
        }

        // ── §6 overlays ────────────────────────────────────────────────────
        //
        // The App has already filled every one of these from the real services,
        // so each arm below only pushes what makes ITS shot different and then
        // raises the modal.
        // THROUGH THE REAL CALLBACKS in all three. The tab is reached the only way
        // the app offers — open the picker, tap the second tab — and the buffer is
        // typed rather than pushed, so the input rules and the live band check are
        // what put the shot on screen.
        State::FreqTab => {
            ui.invoke_open_nearby();
            ui.invoke_set_nearby_tab(NearbyTab::Freq);
            ui.set_overlay(Overlay::Nearby);
        }
        State::FreqTyping => {
            ui.invoke_open_nearby();
            ui.invoke_set_nearby_tab(NearbyTab::Freq);
            // A raw mid-entry buffer: "104." is legitimate and is not normalised.
            for c in ["1", "0", "4", "."] {
                ui.invoke_freq_key(c.into());
            }
            ui.set_overlay(Overlay::Nearby);
        }
        State::FreqError => {
            // 76.5 is below the band and no further typing can rescue it, which
            // is exactly when the line is allowed to appear. TUNE is not pressed —
            // it would close the overlay now (§5), and the warning is live.
            ui.invoke_open_nearby();
            ui.invoke_set_nearby_tab(NearbyTab::Freq);
            for c in ["7", "6", ".", "5"] {
                ui.invoke_freq_key(c.into());
            }
            ui.set_overlay(Overlay::Nearby);
        }
        State::Nearby => ui.set_overlay(Overlay::Nearby),
        State::NearbyGenre => {
            // THE ONLY NEARBY SHOT THAT IS NOT THE REAL QUERY, and it cannot be:
            // `station_class` and the genre column are NULL in all 20,733 shipped
            // rows, so every station classifies as Music, `has_talk` is false and
            // NEITHER filter bar can ever appear on real data. The filter is
            // implemented and tested in src/stations.rs; it has no data source.
            // Pushed here so the drilled-in layout is still inspected.
            ui.set_nearby_bucket("Music".into());
            ui.set_nearby_genre("Classic Hits".into());
            ui.set_nearby_show_bucket_bar(false);
            ui.set_nearby_show_genre_bar(true);
            ui.set_nearby_genre_columns(ModelRc::from(Rc::new(VecModel::from(vec![
                genre_col("Adult Contemporary", "Active Rock"),
                genre_col("Classic Hits", "Classical"),
                genre_col("Alternative", "Soft AC"),
                genre_col("Public Radio", ""),
            ]))));
            // The list arrives already filtered and its meters already
            // re-normalised over what is left, which is what Rust owes it.
            retain_nearby(ui, &["102.1", "88.7", "105.5"]);
            ui.set_overlay(Overlay::Nearby);
        }
        State::NearbyNoGps => {
            // Through the real seam: no fix means the picker itself reports
            // NoGps, rather than the property being overwritten to say so.
            driver.set_position(FakeLocation::no_fix());
            ui.set_overlay(Overlay::Nearby);
        }
        State::NearbyLoading => {
            // ALSO UNREACHABLE IN PRACTICE, and pushed for that reason: the
            // query measures 0.77 ms against the real file, so the picker never
            // produces Loading. The body exists for a slower device.
            ui.set_nearby_state(NearbyState::Loading);
            ui.set_nearby_show_bucket_bar(false);
            ui.set_overlay(Overlay::Nearby);
        }
        State::Settings => ui.set_overlay(Overlay::Settings),
        State::SettingsDiag => {
            // Through the real callback, so the nested switches, the action rows
            // and the log all come from `settings::Settings` deciding.
            ui.invoke_settings_set_diag(true);
            ui.set_overlay(Overlay::Settings);
        }
        State::SettingsDiagFull => {
            ui.invoke_settings_set_diag(true);
            // The two switches that used to be set here — raw capture, mirror to
            // the face — were CarFM's diagnostics and are gone, along with the
            // export row that capture used to add. What is left is the log
            // itself, which is what this shot is of.
            // Two real tunes, so the log has more in it than the connect line.
            ui.invoke_select_preset(1);
            ui.invoke_select_preset(3);
            driver.settle_meter_for_test();
            // AND ONE PROBE ROW, TAPPED, so the shot shows what a row says under
            // itself afterwards (#126). Through the real callback and the real
            // deferred half; on this host the class does not exist, so the row
            // reads "Nothing to report — see the log above", which is the
            // honest answer and the one the shot is of.
            {
                use slint::Model;
                let rows = ui.get_settings_diag_actions();
                let keep = (0..rows.row_count())
                    .find(|&i| {
                        rows.row_data(i)
                            .map(|a| a.label.starts_with("What could keep"))
                            .unwrap_or(false)
                    })
                    .expect("the keep-alive row") as i32;
                ui.invoke_settings_pick_diag_action(keep);
                driver.run_pending_probe();
            }
            ui.set_overlay(Overlay::Settings);
        }
        // 8:05 rather than a round number: a single-digit hour is what the
        // blank-digit pad is for, and 05 is what "minutes always pad" is for.
        State::Clock12 => driver.set_clock_for_test(8, 5, false),
        State::Clock24 => driver.set_clock_for_test(8, 5, true),
        State::SettingsEggs => {
            // THROUGH THE REAL CALLBACK, so the list, the label and the lit row
            // all come from `eggs::listed()` deciding rather than from a literal
            // written here. The taps are set rather than clicked: there is no
            // pointer in a screenshot.
            ui.set_settings_egg_taps(6);
            ui.invoke_settings_force_egg(2);
            ui.set_overlay(Overlay::Settings);
        }
        State::LogoDarkPick => {
            // THROUGH THE REAL DOOR, so the header carries a real station: the
            // window is opened for a preset the way the reorder badge opens it,
            // and only then is the save it is answering pushed in.
            ui.invoke_open_logo_search(0);
            // THROUGH THE REAL EVENT SEAM, which is how the worker reaches this
            // screen on the unit: a save that assigned a master, then the four
            // treatments. The rasters here are the fake search's art rather than
            // pipeline output — the host has no codec to run the pipeline with —
            // so what this shot proves is the LAYOUT, the labels, the badge, the
            // slab and the selection, and not the adaptation.
            driver.push_logo_event_for_test(carnyx::logos::service::Event::Saved {
                base: "WMGN".into(),
                assigned: true,
            });
            driver.push_logo_event_for_test(carnyx::logos::service::Event::DarkChoices {
                base: "WMGN".into(),
                items: vec![
                    (Treatment::Remap, carnyx::fake::FakeLogoSearch::art(0, 192)),
                    (Treatment::Halo, carnyx::fake::FakeLogoSearch::art(1, 192)),
                    (Treatment::AsIs, carnyx::fake::FakeLogoSearch::art(2, 192)),
                    (Treatment::Plate, carnyx::fake::FakeLogoSearch::art(3, 192)),
                ],
                pick: Treatment::Halo,
                open_on: Treatment::Halo,
            });
            ui.set_overlay(Overlay::LogoSearch);
        }
        State::LogoLanding => ui.set_overlay(Overlay::LogoSearch),
        State::LogoLandingHasLogo => {
            ui.set_logo_search_has_logo(true);
            ui.set_logo_search_logo(logo_art(0));
            // §6.4's logo-only hero: a station WITH a logo defaults both off.
            ui.set_logo_search_show_call(false);
            ui.set_logo_search_show_freq(false);
            ui.set_logo_search_search_label("Search for a different logo".into());
            ui.set_overlay(Overlay::LogoSearch);
        }
        State::LogoResults => {
            // The real state machine, run against `fake::FakeLogoSearch` because
            // `logos::LogoNet` and `logos::ImageCodec` have no implementations in
            // this crate. The generation counter, the arrival order, the captions
            // and the selection are all shipping code; only the pixels are made
            // up. Every string below therefore comes from `logos::search::View`.
            ui.invoke_open_logo_search(0);
            ui.invoke_logo_search_search();
            ui.invoke_logo_search_pick(1);
            ui.set_overlay(Overlay::LogoSearch);
        }
        State::LogoLoading => {
            ui.set_logo_search_state(LogoSearchState::Loading);
            ui.set_logo_search_can_confirm(false);
            ui.set_logo_search_confirm_label("Confirm".into());
            ui.set_logo_search_hint("".into());
            ui.set_overlay(Overlay::LogoSearch);
        }
        State::LogoNoResults => {
            ui.set_logo_search_state(LogoSearchState::NoResults);
            ui.set_logo_search_can_confirm(false);
            ui.set_logo_search_confirm_label("Confirm".into());
            ui.set_logo_search_hint("".into());
            ui.set_overlay(Overlay::LogoSearch);
        }
        State::LogoError => {
            // The SAVE-failure wording, reached the way a driver reaches it:
            // pick a result and confirm it, with no image decoder behind the
            // Confirm. The two error wordings are different strings and this is
            // the second of them.
            ui.invoke_open_logo_search(0);
            ui.invoke_logo_search_search();
            ui.invoke_logo_search_pick(1);
            ui.invoke_logo_search_confirm();
            ui.set_overlay(Overlay::LogoSearch);
        }
    }
}

/// One column of the pushed genre grid. See the `NearbyGenre` arm for why this
/// grid cannot come from the shipped database.
fn genre_col(top: &str, bottom: &str) -> GenreColumn {
    GenreColumn {
        top: top.into(),
        bottom: bottom.into(),
        has_bottom: !bottom.is_empty(),
    }
}

/// One piece of the fake search's generated art, as a `slint::Image`.
///
/// NOT A LOGO. There is no image decoder in this crate, so the "already has a
/// logo" landing view is shown with the same flat plate the fake search
/// produces, through the same `logos::ui::to_image` the real path uses.
fn logo_art(index: usize) -> slint::Image {
    carnyx::logos::ui::to_image(&carnyx::fake::FakeLogoSearch::art(index, 192))
}

/// Keep only the rows on these dial frequencies, and re-rank what is left.
///
/// The picker draws `signal-pairs` exactly as given and never sorts, so the
/// filtered shot has to arrive with its meters already re-normalised over the
/// shorter list — which is the thing Rust owes it, and the thing a shot taken
/// with the unfiltered ranks would hide.
fn retain_nearby(ui: &carnyx::AppWindow, keep: &[&str]) {
    let rows: Vec<_> = ui
        .get_nearby_stations()
        .iter()
        .filter(|s| keep.contains(&s.freq.as_str()))
        .collect();
    let n = rows.len();
    let rows: Vec<_> = rows
        .into_iter()
        .enumerate()
        .map(|(i, mut s)| {
            // strengthOf's 1-5 rank over the DISPLAYED list, less one; a
            // one-row list is a full five.
            s.signal_pairs = if n <= 1 { 4 } else { (4 * (n - 1 - i) / (n - 1)) as i32 };
            s
        })
        .collect();
    ui.set_nearby_stations(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn write_png(name: &str, w: u32, h: u32, buffer: &[PremultipliedRgbaColor]) {
    let path = format!("shots/{name}.png");
    let file = std::fs::File::create(&path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);

    // The renderer works in premultiplied alpha; PNG wants straight alpha.
    let mut bytes = Vec::with_capacity(buffer.len() * 4);
    for px in buffer {
        let a = px.alpha;
        let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255) / a as u32).min(255) as u8 };
        bytes.extend_from_slice(&[un(px.red), un(px.green), un(px.blue), a]);
    }
    enc.write_header().expect("header").write_image_data(&bytes).expect("data");
}
