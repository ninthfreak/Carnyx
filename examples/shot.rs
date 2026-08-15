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
use carnyx::{GenreColumn, LogoSearchState, NearbyState, Overlay};

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
    ("weak-and-lossy", 1024, 614, false, State::WeakAndLossy),
    ("no-presets", 1024, 614, false, State::NoPresets),
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
    // §6.1 numpad. `compact` is a HEIGHT track (Metrics.h < 560), so 800×360 is
    // the branch that matters and 360×800 is NOT compact despite being narrow.
    ("numpad-head-unit", 1024, 614, false, State::Numpad, 0.0),
    ("numpad-head-unit-dark", 1024, 614, true, State::Numpad, 0.0),
    ("numpad-typing", 1024, 614, false, State::NumpadTyping, 0.0),
    ("numpad-out-of-band", 1024, 614, false, State::NumpadError, 0.0),
    ("numpad-phone-portrait", 360, 800, false, State::NumpadTyping, 0.0),
    ("numpad-phone-landscape-compact", 800, 360, false, State::NumpadTyping, 0.0),
    ("numpad-phone-landscape-scrolled", 800, 360, false, State::NumpadTyping, -6.0),
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
    ("settings-band-themes", 1024, 614, false, State::SettingsEgg, -8.0),
    ("settings-diagnostics-portrait", 360, 800, false, State::SettingsDiag, -11.0),
    ("settings-phone-portrait", 360, 800, false, State::Settings, 0.0),
    ("settings-phone-portrait-scrolled", 360, 800, false, State::Settings, -8.0),
    // §6.4 logo search: both landing views, the grid, and the two dead ends.
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
    /// A genre string past the 200dp cap, which must elide rather than push the
    /// controls or collapse the line.
    LongGenre,
    /// Reorder mode, where every tile carries the logo-search badge (§6.4).
    Reordering,
    /// No position: the satellite icon's other state.
    NoGpsFix,
    /// A dial that is not one of the six saved slots.
    UnsavedDial,

    // ── §6 overlays ──
    /// §6.1 as it opens: nothing typed, so the display is the live dial, dimmed.
    Numpad,
    /// Mid-entry. The buffer is raw — "104." is a legitimate display string and
    /// must NOT be normalised on the way in.
    NumpadTyping,
    /// TUNE rejected by the band check: amber display border, the error line.
    NumpadError,
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
    /// The hidden BAND THEMES group, opened the way the driver opens it: six
    /// taps on the about line, through the real counter.
    SettingsEgg,
    /// §6.4 landing on a station with no logo.
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

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() }))
        .expect("set platform");
    std::fs::create_dir_all("shots").expect("create shots/");

    let face = SURFACES.iter().map(|&(n, w, h, d, s)| (n, w, h, d, s, 0.0));
    for (name, w, h, dark, state, scroll) in face.chain(OVERLAYS.iter().copied()) {
        // A whole application per shot, driven by the real services: the station
        // database is opened and queried, the RDS decoder is fed until its
        // consensus gates clear, the signal maths runs, and the settings panel is
        // derived. The arms below push only what makes THIS shot different.
        //
        // `driver` must outlive the render — it owns every callback, and a face
        // whose callbacks have been dropped still draws but answers nothing.
        let (ui, driver) = carnyx::build().expect("build window");
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

        write_png(name, w, h, &buffer);
        ui.hide().expect("hide");
        println!("shots/{name}.png  {w}x{h}");
    }
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
        State::Tuned => ui.invoke_select_preset(2),
        State::OutOfBand => {
            ui.set_in_band(false);
            ui.set_freq_label("76.5".into());
        }
        State::LongRadioText => ui.set_radio_text(
            "NOW PLAYING ON HOT 105.1 — Harry Styles — As It Was — up next Dua Lipa".into(),
        ),
        State::NoCallsign => ui.set_ident("".into()),
        State::StereoUnknown => ui.set_stereo_known(false),
        State::LongGenre => ui.set_pty("Adult Album Alternative and Classic Rock".into()),
        State::Reordering => ui.set_reordering(true),
        State::NoGpsFix => driver.set_position(FakeLocation::no_fix()),
        State::UnsavedDial => {
            // Through the numpad's own callbacks, so the band check, the buffer
            // rules and the tune all run rather than a property being set.
            for c in ["1", "0", "5", ".", "1"] {
                ui.invoke_numpad_enter(c.into());
            }
            ui.invoke_numpad_tune();
        }

        // ── §6 overlays ────────────────────────────────────────────────────
        //
        // The App has already filled every one of these from the real services,
        // so each arm below only pushes what makes ITS shot different and then
        // raises the modal.
        State::Numpad => ui.set_overlay(Overlay::Numpad),
        State::NumpadTyping => {
            // A raw mid-entry buffer. Rust owns the input rules and hands the
            // string over exactly as typed — "104." is legitimate here.
            ui.set_numpad_display("104.".into());
            ui.set_numpad_display_dim(false);
            ui.set_numpad_can_tune(true);
            ui.set_overlay(Overlay::Numpad);
        }
        State::NumpadError => {
            ui.set_numpad_display("76.5".into());
            ui.set_numpad_display_dim(false);
            ui.set_numpad_can_tune(true);
            ui.set_numpad_error(true);
            ui.set_overlay(Overlay::Numpad);
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
            // Raw capture on is what adds the export row — the only second
            // action row reachable without a head unit, since the four vendor
            // probes need a live NWD service.
            ui.invoke_settings_set_rds_capture(true);
            ui.invoke_settings_set_diag_overlay(true);
            // Two real tunes, so the log has more in it than the connect line.
            ui.invoke_select_preset(1);
            ui.invoke_select_preset(3);
            ui.set_overlay(Overlay::Settings);
        }
        State::SettingsEgg => {
            // Six taps, counted in Rust exactly as a driver's six taps are.
            for _ in 0..6 {
                ui.invoke_settings_tap_about();
            }
            ui.invoke_settings_pick_egg(2);
            ui.set_overlay(Overlay::Settings);
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
