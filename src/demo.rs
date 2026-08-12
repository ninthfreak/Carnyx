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
