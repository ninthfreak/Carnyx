//! Placeholder dial state.
//!
//! The interface comes first and the NWD tuner second (see README), so until the
//! tuner lands, something has to stand in for it. This module is that stand-in
//! and nothing else: it fills the same properties a real tuner will fill, so
//! swapping it out is a matter of deleting this file and writing to the same
//! setters from the tuner thread.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::rc::Rc;

use crate::station::{brand_color, clean_call, format_mhz, plate_label};
use crate::{AppWindow, Preset};

/// A station as the placeholder knows it: name and dial, nothing else.
const STATIONS: &[(&str, f32)] = &[
    ("WQLF-FM", 102.1),
    ("WERN", 88.7),
    ("WWHG-FM", 105.5),
    ("WMGN", 98.1),
    ("WMLI", 96.3),
    ("WJJO", 94.1),
];

fn preset(name: &str, mhz: f32) -> Preset {
    // No station database yet, so there is no resolved call sign to pass — the
    // fallback path takes the preset's own name. Once the database lands this is
    // where the resolved base goes.
    let call = plate_label(None, name);
    Preset {
        name: name.into(),
        call: call.clone().into(),
        // The colour hashes from the CORE letters, so `WWHG` and `WWHG-FM` are
        // the same station and get the same fill.
        brand: brand_color(&call),
        freq_mhz: mhz,
        freq_label: format_mhz(mhz).into(),
        logo: Default::default(),
        has_logo: false,
    }
}

pub fn install(ui: &AppWindow) {
    let presets: Vec<Preset> = STATIONS.iter().map(|(n, f)| preset(n, *f)).collect();
    let model = Rc::new(VecModel::from(presets));
    ui.set_presets(ModelRc::from(model.clone()));

    // Tuned to a station that is NOT in the list, so the face shows the unsaved
    // star and the peek cards fall back to last/first the way CarFM does.
    ui.set_ident(clean_call("WMHX").into());
    ui.set_freq_label(format_mhz(105.1).into());
    ui.set_saved(false);
    ui.set_active_index(-1);
    ui.set_radio_text("Hot 105.1 — Harry Styles — As It Was".into());
    ui.set_pty("Hot AC".into());

    // A mid-strength carrier with a clean RDS lock: two full arc pairs and the
    // half-step ring above them.
    ui.set_full_pairs(2);
    ui.set_half(true);
    ui.set_dot_opacity(1.0);
    ui.set_dotted_arcs(0);
    ui.set_level_text("63".into());
    ui.set_stereo_known(true);
    ui.set_stereo(true);
    ui.set_rds(true);
    ui.set_af(true);
    ui.set_gps_fix(true);

    sync_neighbours(ui, &model);
    install_overlays(ui);

    // ── Interaction, as far as the face alone can carry it ──
    {
        let ui = ui.as_weak();
        let model = model.clone();
        ui.unwrap().on_select_preset(move |i| {
            let ui = ui.unwrap();
            if let Some(p) = model.row_data(i as usize) {
                tune_to(&ui, &model, i, &p);
            }
        });
    }
    {
        let ui = ui.as_weak();
        let model = model.clone();
        ui.unwrap().on_step_preset(move |dir| {
            let ui = ui.unwrap();
            let n = model.row_count() as i32;
            if n == 0 {
                return;
            }
            // With no active preset, stepping starts from the ends — prev is the
            // last entry and next the first, which is what the peek cards show.
            let cur = ui.get_active_index();
            let next = if cur < 0 {
                if dir > 0 { 0 } else { n - 1 }
            } else {
                (cur + dir).rem_euclid(n)
            };
            if let Some(p) = model.row_data(next as usize) {
                tune_to(&ui, &model, next, &p);
            }
        });
    }
    {
        let ui = ui.as_weak();
        ui.unwrap().on_claim_audio(move || ui.unwrap().set_audio_active(true));
    }
    {
        let ui = ui.as_weak();
        ui.unwrap().on_release_audio(move || ui.unwrap().set_audio_active(false));
    }
    {
        let ui = ui.as_weak();
        ui.unwrap().on_toggle_save(move || {
            let ui = ui.unwrap();
            let saved = ui.get_saved();
            ui.set_saved(!saved);
        });
    }
}

fn tune_to(ui: &AppWindow, model: &Rc<VecModel<Preset>>, index: i32, p: &Preset) {
    ui.set_active_index(index);
    ui.set_ident(p.call.clone());
    ui.set_freq_label(p.freq_label.clone());
    ui.set_saved(true);
    sync_neighbours(ui, model);
}

/// The previous and next presets that flank the hero. With no active preset,
/// prev is the last entry and next the first.
fn sync_neighbours(ui: &AppWindow, model: &Rc<VecModel<Preset>>) {
    let n = model.row_count() as i32;
    if n == 0 {
        ui.set_has_prev(false);
        ui.set_has_next(false);
        return;
    }
    let cur = ui.get_active_index();
    let (prev, next) = if cur < 0 {
        (n - 1, 0)
    } else {
        ((cur - 1).rem_euclid(n), (cur + 1).rem_euclid(n))
    };
    if let Some(p) = model.row_data(prev as usize) {
        ui.set_prev_preset(p);
        ui.set_has_prev(true);
    }
    if let Some(p) = model.row_data(next as usize) {
        ui.set_next_preset(p);
        ui.set_has_next(true);
    }
}

// ── Overlay placeholder state (ANDROID §6) ──────────────────────────────────
//
// The same stand-in job the dial state above does, for the four modals. Every
// value here is a FINISHED one — the strings are already formatted, the badges
// already chosen, the signal ranks already normalised over the displayed list —
// because that is the contract the overlays were written to. When the real
// sources land (the FCC database, the tuner probe, the logo search) they write
// to these same setters and this section goes.

use crate::{
    BatteryState, GenreColumn, LogoCandidate, NearbyStation, TunerAction, TunerDetail, TunerGlyph,
    TunerSource,
};
use slint::{Color, Image, Rgba8Pixel, SharedPixelBuffer, SharedString};

fn nearby(
    freq: &str,
    call: &str,
    service: &str,
    meta: &str,
    distance: &str,
    signal_pairs: i32,
    saved: bool,
) -> NearbyStation {
    NearbyStation {
        freq: freq.into(),
        call: call.into(),
        service: service.into(),
        meta: meta.into(),
        distance: distance.into(),
        signal_pairs,
        saved,
    }
}

/// The rows the picker shows, already ranked and already re-normalised: the
/// strongest row carries four lit pairs and the weakest none, which is what
/// `discreteLit(strengthOf(...))` yields over a list of this length.
fn nearby_rows() -> Vec<NearbyStation> {
    vec![
        nearby("98.1", "WMGN", "", "Madison · Adult Contemporary", "4 km", 4, true),
        nearby("94.1", "WJJO", "", "Watertown · Active Rock", "11 km", 4, true),
        nearby("102.1", "WQLF", "FX", "Middleton · Classic Hits", "13 km", 3, false),
        nearby("88.7", "WERN", "", "Madison · Classical", "6 km", 3, false),
        nearby("105.5", "WWHG", "", "Monona · Alternative", "18 km", 2, true),
        nearby("96.3", "WMLI", "FL", "Sun Prairie · Soft AC", "24 km", 1, false),
        nearby("101.5", "WIBA", "", "Madison · News/Talk", "31 km", 1, false),
        nearby("90.9", "WHHI", "FX", "Highland · Public Radio", "46 km", 0, false),
    ]
}

fn genre_col(top: &str, bottom: &str) -> GenreColumn {
    GenreColumn {
        top: top.into(),
        bottom: bottom.into(),
        has_bottom: !bottom.is_empty(),
    }
}

fn source(
    name: &str,
    kind: &str,
    badge: &str,
    badge_lit: bool,
    available: bool,
    selected: bool,
) -> TunerSource {
    TunerSource {
        name: name.into(),
        kind: kind.into(),
        badge: badge.into(),
        badge_lit,
        available,
        selected,
    }
}

fn detail(key: &str, value: &str) -> TunerDetail {
    TunerDetail { key: key.into(), value: value.into() }
}

/// A flat placeholder thumbnail. There is no network here, and a candidate grid
/// with four empty cells says nothing about whether the grid is right — so each
/// cell gets a solid plate with a lighter inset, which is enough to see the art
/// well, the containment and the selection badge behave.
fn swatch(fill: Color, inset: Color, w: u32, h: u32) -> Image {
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    let stride = buf.width() as usize;
    let px = buf.make_mut_slice();
    let bx = (w / 6) as usize;
    let by = (h / 6) as usize;
    for y in 0..h as usize {
        for x in 0..stride {
            let c = if x >= bx && x < stride - bx && y >= by && y < h as usize - by {
                inset
            } else {
                fill
            };
            px[y * stride + x] =
                Rgba8Pixel { r: c.red(), g: c.green(), b: c.blue(), a: 255 };
        }
    }
    Image::from_rgba8(buf)
}

fn candidate(domain: &str, dims: &str, n: usize, fill: Color, inset: Color) -> LogoCandidate {
    LogoCandidate {
        thumb: swatch(fill, inset, 128, 128),
        domain: domain.into(),
        dims: dims.into(),
        // §6.4 wants the art on its own background; white is what the shipping
        // build effectively passes, and it is what these opaque swatches want.
        background: Color::from_rgb_u8(0xFF, 0xFF, 0xFF),
        alt: if domain.is_empty() {
            format!("Logo option {n}").into()
        } else {
            SharedString::from(format!("Logo option {n} from {domain}"))
        },
    }
}

/// Four candidates, in arrival order — the picker never re-sorts them.
pub fn logo_candidates() -> Vec<LogoCandidate> {
    vec![
        candidate("wikipedia.org", "512×512", 1, Color::from_rgb_u8(0x20, 0x65, 0x5B), Color::from_rgb_u8(0xF2, 0xF6, 0xF4)),
        candidate("radio-locator.com", "300×300", 2, Color::from_rgb_u8(0x2E, 0x5E, 0xAA), Color::from_rgb_u8(0xFF, 0xFF, 0xFF)),
        candidate("wmgn.com", "", 3, Color::from_rgb_u8(0xB0, 0x2A, 0x6E), Color::from_rgb_u8(0xFF, 0xF4, 0xF9)),
        candidate("tunein.com", "1024×1024", 4, Color::from_rgb_u8(0x1E, 0x1E, 0x22), Color::from_rgb_u8(0xE8, 0xE8, 0xEC)),
    ]
}

/// Fill every overlay with the state it would have on a working head unit. The
/// modal itself stays closed — `overlay` is left at `Overlay::None`.
pub fn install_overlays(ui: &AppWindow) {
    // §6.1 numpad: nothing typed, so the display is the live dial at 0.45.
    ui.set_numpad_display(ui.get_freq_label());
    ui.set_numpad_display_dim(true);
    ui.set_numpad_can_tune(false);
    ui.set_numpad_error(false);
    ui.set_numpad_error_text(
        format!("Outside {} band", "87.5\u{2013}108.0").into(),
    );

    // §6.2 nearby picker.
    ui.set_nearby_stations(ModelRc::from(Rc::new(VecModel::from(nearby_rows()))));
    ui.set_nearby_snapshot("2026-06-14".into());
    ui.set_nearby_bucket_chips(ModelRc::from(Rc::new(VecModel::from(vec![
        SharedString::from("All"),
        SharedString::from("Music"),
        SharedString::from("Talk"),
    ]))));
    ui.set_nearby_bucket("All".into());
    ui.set_nearby_genre_columns(ModelRc::from(Rc::new(VecModel::from(vec![
        genre_col("Adult Contemporary", "Active Rock"),
        genre_col("Classic Hits", "Classical"),
        genre_col("Alternative", "Soft AC"),
        genre_col("Public Radio", ""),
    ]))));
    ui.set_nearby_genre("".into());
    ui.set_nearby_show_bucket_bar(true);
    ui.set_nearby_show_genre_bar(false);

    // §6.3 settings: the built-in tuner, connected, with no trailing control —
    // the third status state, which only the shipping TSX has.
    ui.set_settings_status_title("Connected".into());
    ui.set_settings_status_sub("Built-in hardware · NWD/NOWADA FM tuner".into());
    ui.set_settings_status_glyph(TunerGlyph::Waves);
    ui.set_settings_status_action(TunerAction::None);
    ui.set_settings_details_label("Details".into());
    ui.set_settings_details_open(false);
    ui.set_settings_tuner_details(ModelRc::from(Rc::new(VecModel::from(vec![
        detail("Device", "Realtek RTL2832U + R820T2"),
        detail("USB ID", "0bda:2838"),
        detail("Sample rate", "2.048 MS/s"),
    ]))));
    ui.set_settings_sources(ModelRc::from(Rc::new(VecModel::from(vec![
        source("RTL-SDR", "USB software-defined radio", "Not detected", false, true, false),
        source("NWD / NOWADA built-in radio", "Integrated head-unit FM tuner", "Detected", true, true, true),
        source("FYT / DuduOS built-in radio", "Integrated head-unit FM tuner", "Unavailable", false, false, false),
        source("Auto", "Probe all sources", "", false, true, false),
    ]))));
    ui.set_settings_autostart(true);
    ui.set_settings_theme_chips(ModelRc::from(Rc::new(VecModel::from(vec![
        SharedString::from("SYSTEM"),
        SharedString::from("LIGHT"),
        SharedString::from("DARK"),
    ]))));
    ui.set_settings_theme("SYSTEM".into());
    ui.set_settings_battery(BatteryState::NotExempt);
    ui.set_settings_battery_sub("Not exempt — Doze may stop the boot-started radio".into());
    ui.set_settings_logos_on(false);
    ui.set_settings_logos_sub(
        "Off — assign logos manually from a station (auto-download in redesign)".into(),
    );
    ui.set_settings_clear_logos_label("Clear all station logos".into());
    ui.set_settings_clearing_logos(false);
    ui.set_settings_show_advanced(true);
    ui.set_settings_about("Carnyx  ·  v0.1.0  ·  FCC station data as of 2026-06-14".into());

    // §6.4 logo search, opened on a station with no logo: both display flags
    // default ON, and the landing view offers the search.
    let call = "WMGN";
    ui.set_logo_search_name(call.into());
    ui.set_logo_search_subtitle("WMGN  ·  98.1 MHz".into());
    ui.set_logo_search_has_logo(false);
    ui.set_logo_search_mono(call.into());
    ui.set_logo_search_brand(brand_color(call));
    ui.set_logo_search_query("radio 98.1 wmgn logo".into());
    ui.set_logo_search_candidates(ModelRc::default());
    ui.set_logo_search_selected_index(-1);
    ui.set_logo_search_show_call(true);
    ui.set_logo_search_show_freq(true);
    ui.set_logo_search_search_label("Search for a logo".into());
    ui.set_logo_search_can_confirm(true);
    ui.set_logo_search_confirm_label("Save".into());
    ui.set_logo_search_hint("Choose what shows on the hero".into());
    ui.set_logo_search_error_title("Search couldn't finish".into());
    ui.set_logo_search_error_body(
        "Something went wrong reaching the logo search. Check your connection and try again."
            .into(),
    );
}
