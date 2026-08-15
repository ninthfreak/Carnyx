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

use carnyx::{LogoSearchState, NearbyState, Overlay, TunerAction};

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
    ("settings-diagnostics-open", 1024, 614, false, State::SettingsDiag, 0.0),
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
    /// The RTL path selected, with its diagnostics panel open.
    SettingsDiag,
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
        let ui = carnyx::build().expect("build window");
        ui.global::<carnyx::Pal>().set_dark(dark);
        apply(&ui, state);
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
fn apply(ui: &carnyx::AppWindow, state: State) {
    match state {
        State::Normal => {}
        State::AudioReleased => ui.set_audio_active(false),
        State::TunerError => ui.set_tuner_error(true),
        State::Driving => {
            ui.set_in_motion(true);
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

        // ── §6 overlays ────────────────────────────────────────────────────
        //
        // `demo::install_overlays` has already filled every one of these with
        // the state it would have on a working head unit, so each arm below
        // only pushes what makes ITS shot different and then raises the modal.
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
            // Drilled into Music: §6.2 hides the bucket row and shows the genre
            // row with the back-arrow chip that is the only way out of it.
            ui.set_nearby_bucket("Music".into());
            ui.set_nearby_genre("Classic Hits".into());
            ui.set_nearby_show_bucket_bar(false);
            ui.set_nearby_show_genre_bar(true);
            // The list is already filtered when it arrives, and the meters are
            // re-normalised over what is left.
            retain_nearby(ui, &["102.1", "88.7", "105.5"]);
            ui.set_overlay(Overlay::Nearby);
        }
        State::NearbyNoGps => {
            ui.set_nearby_state(NearbyState::NoGps);
            ui.set_nearby_show_bucket_bar(false);
            ui.set_overlay(Overlay::Nearby);
        }
        State::NearbyLoading => {
            ui.set_nearby_state(NearbyState::Loading);
            ui.set_nearby_show_bucket_bar(false);
            ui.set_overlay(Overlay::Nearby);
        }
        State::Settings => ui.set_overlay(Overlay::Settings),
        State::SettingsDiag => {
            // The RTL path selected instead of the built-in tuner, which is the
            // only combination that offers Details at all.
            let sources: Vec<_> = ui
                .get_settings_sources()
                .iter()
                .enumerate()
                .map(|(i, mut s)| {
                    s.selected = i == 0;
                    s
                })
                .collect();
            ui.set_settings_sources(ModelRc::from(Rc::new(VecModel::from(sources))));
            ui.set_settings_status_sub("Local hardware · RTL-SDR (RTL2832U)".into());
            ui.set_settings_status_action(TunerAction::Details);
            ui.set_settings_details_label("Hide details".into());
            ui.set_settings_details_open(true);
            ui.set_overlay(Overlay::Settings);
        }
        State::LogoLanding => ui.set_overlay(Overlay::LogoSearch),
        State::LogoLandingHasLogo => {
            ui.set_logo_search_has_logo(true);
            ui.set_logo_search_logo(carnyx::demo::logo_candidates()[0].thumb.clone());
            // §6.4's logo-only hero: a station WITH a logo defaults both off.
            ui.set_logo_search_show_call(false);
            ui.set_logo_search_show_freq(false);
            ui.set_logo_search_search_label("Search for a different logo".into());
            ui.set_overlay(Overlay::LogoSearch);
        }
        State::LogoResults => {
            ui.set_logo_search_state(LogoSearchState::Results);
            ui.set_logo_search_candidates(ModelRc::from(Rc::new(VecModel::from(
                carnyx::demo::logo_candidates(),
            ))));
            ui.set_logo_search_selected_index(1);
            ui.set_logo_search_can_confirm(true);
            ui.set_logo_search_confirm_label("Confirm".into());
            ui.set_logo_search_hint("Saved as this station's logo".into());
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
            ui.set_logo_search_state(LogoSearchState::Error);
            ui.set_logo_search_error_title("Couldn't save that logo".into());
            ui.set_logo_search_error_body(
                "Couldn't save this logo — the image was too large. Try a different result."
                    .into(),
            );
            ui.set_logo_search_can_confirm(false);
            ui.set_logo_search_confirm_label("Confirm".into());
            ui.set_logo_search_hint("".into());
            ui.set_overlay(Overlay::LogoSearch);
        }
    }
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
