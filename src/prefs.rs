//! What survives the ignition.
//!
//! Until this existed, everything the driver chose was gone at the next launch:
//! the preset strip came back as `fake::seed_presets()`, the theme reverted to
//! SYSTEM, and all four diagnostics toggles cleared. On a head unit that power
//! cycles with the key, that is not a rough edge — it is the app forgetting the
//! six stations you drive to, every single morning.
//!
//! ## Two rules, both learned from the rest of this tree
//!
//! **Nothing here may fail.** A missing file is a first run. A corrupt file is a
//! first run. A file written by a newer build with fields this one has never
//! heard of is a first run for those fields and a normal load for the rest. The
//! radio must come up. `load` returns `Prefs`, not `Result<Prefs>`, because there
//! is no failure a driver could act on and no screen on which to tell them.
//!
//! **Writes are atomic.** Same shape as `stations::install`: write a temporary
//! file, `sync_all` it, then rename. A rename is only atomic with respect to what
//! has actually reached the disk, and an ignition cut mid-write is the single
//! most likely moment for this file to be touched — it is written when the driver
//! saves a preset, and they save presets while driving.
//!
//! ## What is NOT here
//!
//! `battery`, `details_open`, `clearing_logos`, `about_taps` and the diagnostics
//! log are all transient or derived, and persisting them would either restore a
//! stale fact about the device or reopen a panel nobody left open. `egg_index` is
//! deliberately excluded too: the band-theme egg is not persisted in CarFM
//! either, and it is revealed by six taps rather than remembered.
//!
//! Per-station hero flags (Display Call Sign / Display Frequency) already persist
//! separately, in `logos::store`, because they belong to a station rather than to
//! the app.

use std::fs;
use std::path::{Path, PathBuf};

use crate::logos::json::{self, Json};
use crate::settings::{Settings, Source, Theme};

/// The file, inside the app's own data directory.
pub const FILE: &str = "prefs.json";

/// Everything worth remembering between launches.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// The preset strip, in the driver's own order. Only the dial is stored —
    /// the FCC row is re-resolved on load, so a database update improves old
    /// presets instead of leaving them stale.
    pub presets: Vec<f32>,
    pub selected: Source,
    pub theme: Theme,
    pub autostart: bool,
    pub logos_on: bool,
    pub diag_on: bool,
    pub diag_overlay_on: bool,
    pub rds_capture_on: bool,
    pub debug_on: bool,
}

impl Default for Prefs {
    /// The defaults are taken FROM `Settings::default` rather than restated, so
    /// the two cannot drift apart. A first run and a cleared file must look
    /// identical.
    fn default() -> Self {
        let s = Settings::default();
        Prefs {
            presets: Vec::new(),
            selected: s.selected,
            theme: s.theme,
            autostart: s.autostart,
            logos_on: s.logos_on,
            diag_on: s.diag_on,
            diag_overlay_on: s.diag_overlay_on,
            rds_capture_on: s.rds_capture_on,
            debug_on: s.debug_on,
        }
    }
}

fn source_name(s: Source) -> &'static str {
    match s {
        Source::Rtl => "rtl",
        Source::Nwd => "nwd",
        Source::Fyt => "fyt",
        Source::Auto => "auto",
    }
}

fn source_from(name: &str) -> Option<Source> {
    match name {
        "rtl" => Some(Source::Rtl),
        "nwd" => Some(Source::Nwd),
        "fyt" => Some(Source::Fyt),
        "auto" => Some(Source::Auto),
        _ => None,
    }
}

fn theme_name(t: Theme) -> &'static str {
    match t {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

fn theme_from(name: &str) -> Option<Theme> {
    match name {
        "system" => Some(Theme::System),
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    }
}

/// Names are STORED, not ordinals.
///
/// An ordinal would silently re-point every saved preference the first time a
/// variant is inserted in the middle of an enum — the driver's theme would flip
/// because someone added a colour scheme. A name costs a few bytes and cannot do
/// that.
pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Read the file, or return defaults. Never fails, by design.
pub fn load(dir: &Path) -> Prefs {
    let Ok(text) = fs::read_to_string(path(dir)) else {
        return Prefs::default();
    };
    from_json(&text).unwrap_or_default()
}

/// Parse, field by field, keeping every field that is readable.
///
/// Deliberately NOT all-or-nothing: a file that has one unreadable field should
/// lose that field, not the driver's six presets. Returns `None` only when the
/// text is not an object at all, which is the one case where there is nothing to
/// salvage.
fn from_json(text: &str) -> Option<Prefs> {
    let v = json::parse(text)?;
    if !matches!(v, Json::Obj(_)) {
        return None;
    }
    let mut p = Prefs::default();

    if let Some(Json::Arr(items)) = v.get("presets") {
        p.presets = items
            .iter()
            .filter_map(|x| x.as_f64())
            // A dial outside the FM band is not a preset, it is corruption, and
            // tuning to it would send the front end somewhere it cannot go.
            .filter(|m| (87.5..=108.0).contains(m))
            .map(|m| m as f32)
            .collect();
    }
    if let Some(s) = v.get("selected").and_then(Json::as_str) {
        if let Some(src) = source_from(s) {
            p.selected = src;
        }
    }
    if let Some(s) = v.get("theme").and_then(Json::as_str) {
        if let Some(t) = theme_from(s) {
            p.theme = t;
        }
    }
    let flag = |key: &str, cur: bool| -> bool {
        match v.get(key) {
            Some(Json::Bool(b)) => *b,
            _ => cur,
        }
    };
    p.autostart = flag("autostart", p.autostart);
    p.logos_on = flag("logosOn", p.logos_on);
    p.diag_on = flag("diagOn", p.diag_on);
    p.diag_overlay_on = flag("diagOverlayOn", p.diag_overlay_on);
    p.rds_capture_on = flag("rdsCaptureOn", p.rds_capture_on);
    p.debug_on = flag("debugOn", p.debug_on);
    Some(p)
}

/// Serialise. One decimal on the dial, which is the precision a dial has.
pub fn to_json(p: &Prefs) -> String {
    let presets: Vec<String> = p.presets.iter().map(|m| format!("{m:.1}")).collect();
    format!(
        concat!(
            "{{\"presets\":[{}],\"selected\":{},\"theme\":{},\"autostart\":{},",
            "\"logosOn\":{},\"diagOn\":{},\"diagOverlayOn\":{},",
            "\"rdsCaptureOn\":{},\"debugOn\":{}}}"
        ),
        presets.join(","),
        json::quote(source_name(p.selected)),
        json::quote(theme_name(p.theme)),
        p.autostart,
        p.logos_on,
        p.diag_on,
        p.diag_overlay_on,
        p.rds_capture_on,
        p.debug_on,
    )
}

/// Write atomically. Errors are swallowed on purpose: a preference that could
/// not be saved is not worth interrupting a drive for, and there is nowhere to
/// report it that a driver would see.
pub fn save(dir: &Path, p: &Prefs) {
    let _ = try_save(dir, p);
}

fn try_save(dir: &Path, p: &Prefs) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let final_path = path(dir);
    let tmp = dir.join(format!("{FILE}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, to_json(p).as_bytes())?;
        // The rename below is only atomic with respect to what has reached the
        // disk. An ignition cut is the likely interruption here.
        f.sync_all()?;
    }
    fs::rename(&tmp, &final_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("carnyx-prefs-{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trips_every_field() {
        let d = tmpdir("round");
        let p = Prefs {
            presets: vec![88.7, 105.5, 98.1],
            selected: Source::Auto,
            theme: Theme::Dark,
            autostart: false,
            logos_on: true,
            diag_on: true,
            diag_overlay_on: true,
            rds_capture_on: true,
            debug_on: true,
        };
        save(&d, &p);
        assert_eq!(load(&d), p);
    }

    #[test]
    fn a_missing_file_is_a_first_run() {
        assert_eq!(load(&tmpdir("missing")), Prefs::default());
    }

    /// The whole point of the module: the radio comes up regardless.
    #[test]
    fn corruption_never_stops_the_radio() {
        for junk in ["", "{", "not json at all", "[1,2,3]", "null", "\u{0}\u{1}"] {
            let d = tmpdir("junk");
            fs::write(path(&d), junk).unwrap();
            assert_eq!(load(&d), Prefs::default(), "input {junk:?}");
        }
    }

    /// One bad field must not cost the driver their presets.
    #[test]
    fn an_unreadable_field_loses_only_itself() {
        let d = tmpdir("partial");
        fs::write(
            path(&d),
            r#"{"presets":[88.7,105.5],"theme":"chartreuse","autostart":"yes","debugOn":true}"#,
        )
        .unwrap();
        let p = load(&d);
        assert_eq!(p.presets, vec![88.7, 105.5], "presets survive");
        assert_eq!(p.theme, Theme::System, "unknown theme falls back");
        assert_eq!(p.autostart, Prefs::default().autostart, "wrong type falls back");
        assert!(p.debug_on, "the readable field is still read");
    }

    /// A dial the front end cannot reach is corruption, not a preset.
    #[test]
    fn out_of_band_presets_are_dropped() {
        let d = tmpdir("band");
        fs::write(path(&d), r#"{"presets":[88.7,0,1e9,-5,107.9,108.1]}"#).unwrap();
        assert_eq!(load(&d).presets, vec![88.7, 107.9]);
    }

    /// Names, not ordinals — inserting an enum variant must not re-point saved
    /// preferences.
    #[test]
    fn enums_are_stored_by_name() {
        let p = Prefs { selected: Source::Fyt, theme: Theme::Light, ..Prefs::default() };
        let text = to_json(&p);
        assert!(text.contains("\"selected\":\"fyt\""), "{text}");
        assert!(text.contains("\"theme\":\"light\""), "{text}");
    }

    /// A half-written file must never be the file that is read. The temporary
    /// is a sibling, so a crash before the rename leaves the previous good copy.
    #[test]
    fn a_save_leaves_no_temporary_behind() {
        let d = tmpdir("atomic");
        save(&d, &Prefs { presets: vec![90.1], ..Prefs::default() });
        let left: Vec<_> = fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec![FILE.to_string()]);
    }

    #[test]
    fn defaults_track_settings_defaults() {
        let s = Settings::default();
        let p = Prefs::default();
        assert_eq!(p.selected, s.selected);
        assert_eq!(p.theme, s.theme);
        assert_eq!(p.autostart, s.autostart);
        assert_eq!(p.logos_on, s.logos_on);
    }
}
