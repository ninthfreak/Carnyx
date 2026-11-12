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
//! `battery`, `details_open`, `clearing_logos` and the diagnostics log are all
//! transient or derived, and persisting them would either restore a stale fact
//! about the device or reopen a panel nobody left open.
//!
//! `autostart` USED TO BE HERE and is not any more. Its row cannot work — a boot
//! receiver is undeclarable under cargo-apk — and CarFM's toggle is a different
//! control anyway, keyed `@vibesdr/car_autostart` and starting a plugged-in
//! RTL-SDR. There is nothing left to remember; see `settings::Settings`.
//!
//! Per-station hero flags (Display Call Sign / Display Frequency) already persist
//! separately, in `logos::store`, because they belong to a station rather than to
//! the app.

use std::fs;
use std::path::{Path, PathBuf};

use crate::logos::json::{self, Json};
use crate::settings::{NavApp, Settings, Source, Theme};

/// The file, inside the app's own data directory.
pub const FILE: &str = "prefs.json";

/// One saved preset.
///
/// THE CALL SIGN IS STORED, and it did not used to be. Only the dial was, on the
/// reasoning that the FCC row is re-resolved on load so a database update
/// improves old presets rather than leaving them stale. That reasoning holds
/// only while there is something to resolve AGAINST: resolution needs a
/// position, because 88.7 has 178 full-power licensees across the country, and
/// on a head unit that has not got a GPS fix yet there is none. The driver's
/// six presets came back as bare frequencies with no call signs, and the logo
/// window — which searches on the call sign — had nothing to search for.
///
/// So the dial is still authoritative and the row is still re-resolved, and this
/// is the FALLBACK that keeps a preset's identity when nothing resolves. It
/// tracks the most recent successful resolution for that dial, so it improves
/// with the database exactly as the original design wanted.
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub mhz: f32,
    /// The call sign this dial last resolved to, or `None` if it never has.
    pub call: Option<String>,
}

/// Everything worth remembering between launches.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// The preset strip, in the driver's own order.
    pub presets: Vec<Preset>,
    pub selected: Source,
    pub theme: Theme,
    pub logos_on: bool,
    /// See `settings::Settings::permissions_asked`. Defaults OFF, so an install
    /// that predates the key gets the one offer and then never again.
    pub permissions_asked: bool,
    pub release_on_sleep: bool,
    /// See `settings::Settings::clock_on`. Defaults ON.
    pub clock_on: bool,
    /// See `settings::Settings::nav_on`. Defaults ON.
    pub nav_on: bool,
    /// See `settings::Settings::nav_app`. Defaults to OsmAnd, so a file from
    /// before CoMaps was an option reads exactly as it did.
    pub nav_app: NavApp,
    /// See `settings::Settings::nav_hide_on_map`. Defaults ON.
    pub nav_hide_on_map: bool,
    /// See `settings::Settings::preset_loop`. Defaults OFF.
    pub preset_loop: bool,
    /// See `settings::Settings::come_forward`. Defaults OFF.
    pub come_forward: bool,
    pub diag_on: bool,
    /// Where the car last had a GPS fix, to a tenth of a degree, for the theme's
    /// sunrise/sunset mode (#142). `None` until the first fix.
    ///
    /// A POSITION, KEPT ON PURPOSE AND KEPT COARSE. `callsigns.rs` makes a point
    /// of storing no coordinates; this has to, because sunrise is a function of
    /// where you are and a head unit in a garage has no fix to ask. A tenth of a
    /// degree (about 11km) moves sunrise by under a minute, so nothing finer is
    /// written.
    pub place: Option<(f64, f64)>,
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
            logos_on: s.logos_on,
            permissions_asked: s.permissions_asked,
            release_on_sleep: s.release_on_sleep,
            clock_on: s.clock_on,
            nav_on: s.nav_on,
            nav_app: s.nav_app,
            nav_hide_on_map: s.nav_hide_on_map,
            preset_loop: s.preset_loop,
            come_forward: s.come_forward,
            diag_on: s.diag_on,
            place: None,
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

fn nav_app_name(a: NavApp) -> &'static str {
    match a {
        NavApp::OsmAnd => "osmand",
        NavApp::CoMaps => "comaps",
    }
}

fn nav_app_from(name: &str) -> Option<NavApp> {
    match name {
        "osmand" => Some(NavApp::OsmAnd),
        "comaps" => Some(NavApp::CoMaps),
        _ => None,
    }
}

fn theme_name(t: Theme) -> &'static str {
    match t {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::Headlights => "headlights",
        Theme::Sun => "sun",
    }
}

fn theme_from(name: &str) -> Option<Theme> {
    match name {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        "headlights" => Some(Theme::Headlights),
        "sun" => Some(Theme::Sun),
        // WHAT EVERY INSTALL BEFORE #142 SAVED unless the driver picked light or
        // dark by hand. "System" meant "follow the car", which on this ROM never
        // worked — Android's night flag is pinned at DAY — so it comes back as
        // the mode that does what it was for.
        "system" => Some(Theme::Headlights),
        _ => None,
    }
}

/// One decimal of a degree: about 11km north-south, which moves sunrise and
/// sunset by well under a minute and is all the sun mode needs.
fn coarse(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
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
            .filter_map(|x| {
                // TWO SHAPES, and the bare number is not legacy cruft to be
                // tidied away later — it is what is on the driver's unit right
                // now, and dropping it would wipe their strip on the update that
                // introduced the object form.
                let (mhz, call) = match x {
                    Json::Obj(_) => (
                        x.get("mhz")?.as_f64()?,
                        x.get("call")
                            .and_then(Json::as_str)
                            .filter(|c| !c.is_empty())
                            .map(str::to_string),
                    ),
                    _ => (x.as_f64()?, None),
                };
                // A dial outside the FM band is not a preset, it is corruption,
                // and tuning to it would send the front end somewhere it cannot
                // go. Filtered HERE rather than after the fact, so the call sign
                // cannot come adrift from the dial it belongs to.
                //
                // THE BAND COMES FROM `app`, not from a fourth copy of the two
                // numbers written out here. `f64` because the JSON reader answers
                // in doubles and the cast to `f32` belongs on the value that is
                // kept, not on the one that is tested.
                let band = f64::from(crate::app::FM_LO)..=f64::from(crate::app::FM_HI);
                band.contains(&mhz).then_some(Preset { mhz: mhz as f32, call })
            })
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
    // AN UNKNOWN APP KEEPS THE DEFAULT, the theme's rule: a file from a newer
    // build naming a third app must not land this one on nothing.
    if let Some(s) = v.get("navApp").and_then(Json::as_str) {
        if let Some(a) = nav_app_from(s) {
            p.nav_app = a;
        }
    }
    let flag = |key: &str, cur: bool| -> bool {
        match v.get(key) {
            Some(Json::Bool(b)) => *b,
            _ => cur,
        }
    };
    p.logos_on = flag("logosOn", p.logos_on);
    // FALLS BACK THROUGH `p`, which already holds `Settings::default`'s true —
    // so a file written before this key existed comes back with the feature ON
    // rather than silently off.
    p.permissions_asked = flag("permissionsAsked", p.permissions_asked);
    p.release_on_sleep = flag("releaseOnSleep", p.release_on_sleep);
    p.clock_on = flag("clockOn", p.clock_on);
    p.nav_on = flag("navOn", p.nav_on);
    p.nav_hide_on_map = flag("navHideOnMap", p.nav_hide_on_map);
    p.preset_loop = flag("presetLoop", p.preset_loop);
    p.come_forward = flag("comeForward", p.come_forward);
    p.diag_on = flag("diagOn", p.diag_on);
    // A PLACE THAT IS NOT ON THE EARTH IS DROPPED, not clamped: a corrupt pair
    // would put sunrise somewhere else entirely, and no place at all falls back
    // to the clock, which is the honest answer.
    if let Some(Json::Arr(xs)) = v.get("place") {
        if let [lat, lon] = xs.as_slice() {
            if let (Some(lat), Some(lon)) = (lat.as_f64(), lon.as_f64()) {
                if lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0 {
                    p.place = Some((coarse(lat), coarse(lon)));
                }
            }
        }
    }
    // `diagOverlayOn`, `rdsCaptureOn` and `debugOn` are DELIBERATELY not read.
    // They were CarFM's diagnostics and are gone; a file written by an older
    // build still carries them, and ignoring an unknown key is what this reader
    // has always done with every other one.
    Some(p)
}

/// Serialise. One decimal on the dial, which is the precision a dial has.
pub fn to_json(p: &Prefs) -> String {
    // An OBJECT per preset, so the dial and its call sign cannot come apart. A
    // preset that has never resolved writes no `call` at all rather than an
    // empty string — absent and unknown are the same thing here, and one of them
    // is shorter.
    let presets: Vec<String> = p
        .presets
        .iter()
        .map(|e| match &e.call {
            Some(c) => format!("{{\"mhz\":{:.1},\"call\":{}}}", e.mhz, json::quote(c)),
            None => format!("{{\"mhz\":{:.1}}}", e.mhz),
        })
        .collect();
    // Written only once there is one. Rounded again here so a caller that forgot
    // cannot put more precision on disk than the field's note promises.
    let place = match p.place {
        Some((lat, lon)) => format!(",\"place\":[{:.1},{:.1}]", coarse(lat), coarse(lon)),
        None => String::new(),
    };
    format!(
        concat!(
            "{{\"presets\":[{}],\"selected\":{},\"theme\":{},",
            "\"logosOn\":{},\"permissionsAsked\":{},\"releaseOnSleep\":{},",
            "\"clockOn\":{},\"navOn\":{},\"navApp\":{},",
            "\"navHideOnMap\":{},\"presetLoop\":{},\"comeForward\":{},\"diagOn\":{}{}}}"
        ),
        presets.join(","),
        json::quote(source_name(p.selected)),
        json::quote(theme_name(p.theme)),
        p.logos_on,
        p.permissions_asked,
        p.release_on_sleep,
        p.clock_on,
        p.nav_on,
        json::quote(nav_app_name(p.nav_app)),
        p.nav_hide_on_map,
        p.preset_loop,
        p.come_forward,
        p.diag_on,
        place,
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

    fn at(mhz: f32, call: Option<&str>) -> Preset {
        Preset { mhz, call: call.map(str::to_string) }
    }

    /// THE BARE-NUMBER FORM IS NOT LEGACY CRUFT. It is what is on the driver's
    /// unit at the moment the object form ships, and a reader that rejected it
    /// would wipe their strip on the update meant to fix it.
    #[test]
    fn a_file_written_before_call_signs_still_loads_its_presets() {
        let d = tmpdir("bare-numbers");
        fs::write(path(&d), r#"{"presets":[102.1,88.7],"theme":"dark"}"#).unwrap();
        let p = load(&d);
        assert_eq!(p.presets, vec![at(102.1, None), at(88.7, None)]);
        assert_eq!(p.theme, Theme::Dark);
    }

    /// The two forms mix, because a strip can be half-resolved: some dials have
    /// found their station and some never have.
    #[test]
    fn the_two_preset_shapes_read_side_by_side() {
        let d = tmpdir("mixed-shapes");
        fs::write(
            path(&d),
            r#"{"presets":[{"mhz":88.7,"call":"WERN"},105.5,{"mhz":98.1}]}"#,
        )
        .unwrap();
        assert_eq!(
            load(&d).presets,
            vec![at(88.7, Some("WERN")), at(105.5, None), at(98.1, None)]
        );
    }

    /// An out-of-band dial is dropped WITH its call sign. Filtering the dials
    /// separately would slide every later call sign onto the wrong station.
    #[test]
    fn dropping_a_corrupt_dial_takes_its_call_sign_with_it() {
        let d = tmpdir("drop-keeps-pairing");
        fs::write(
            path(&d),
            r#"{"presets":[{"mhz":8.7,"call":"GHOST"},{"mhz":88.7,"call":"WERN"}]}"#,
        )
        .unwrap();
        assert_eq!(load(&d).presets, vec![at(88.7, Some("WERN"))]);
    }

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
            presets: vec![at(88.7, Some("WERN")), at(105.5, None), at(98.1, Some("WMGN-FM"))],
            selected: Source::Auto,
            theme: Theme::Dark,
            logos_on: true,
            permissions_asked: false,
            release_on_sleep: false,
            clock_on: false,
            nav_on: true,
            // NOT THE DEFAULT, so the round trip proves the key is written and
            // read rather than falling back to OsmAnd both ways.
            nav_app: NavApp::CoMaps,
            nav_hide_on_map: false,
            preset_loop: true,
            come_forward: true,
            diag_on: true,
            place: Some((43.1, -89.4)),
        };
        save(&d, &p);
        assert_eq!(load(&d), p);
    }

    /// THE THEME, #142: four names, and every install before it saved
    /// "system" — which comes back as following the headlights, the mode that
    /// does what "system" was for. An unknown name keeps the default.
    #[test]
    fn theme_names_round_trip_and_system_becomes_headlights() {
        for t in [Theme::Light, Theme::Dark, Theme::Headlights, Theme::Sun] {
            let d = tmpdir("theme-names");
            save(&d, &Prefs { theme: t, ..Prefs::default() });
            assert_eq!(load(&d).theme, t, "{t:?} survives");
        }
        let d = tmpdir("theme-legacy");
        fs::write(path(&d), r#"{"theme":"system"}"#).unwrap();
        assert_eq!(load(&d).theme, Theme::Headlights);
        let d = tmpdir("theme-unknown");
        fs::write(path(&d), r#"{"theme":"sepia"}"#).unwrap();
        assert_eq!(load(&d).theme, Theme::Headlights, "the default");
    }

    /// THE PLACE is written coarse and read back coarse; a pair that is not on
    /// the Earth, or not a pair, is dropped rather than trusted.
    #[test]
    fn the_place_is_kept_to_a_tenth_of_a_degree_and_nonsense_is_dropped() {
        let d = tmpdir("place-coarse");
        save(&d, &Prefs { place: Some((43.07314, -89.40123)), ..Prefs::default() });
        let text = fs::read_to_string(path(&d)).unwrap();
        assert!(text.contains(r#""place":[43.1,-89.4]"#), "no more precision on disk: {text}");
        assert_eq!(load(&d).place, Some((43.1, -89.4)));

        let d = tmpdir("place-none");
        save(&d, &Prefs::default());
        assert!(!fs::read_to_string(path(&d)).unwrap().contains("place"), "absent until a fix");
        assert_eq!(load(&d).place, None);

        for junk in [r#"{"place":[91,0]}"#, r#"{"place":[0,181]}"#, r#"{"place":[1]}"#, r#"{"place":"here"}"#] {
            let d = tmpdir("place-junk");
            fs::write(path(&d), junk).unwrap();
            assert_eq!(load(&d).place, None, "{junk}");
        }
    }

    /// A FILE FROM BEFORE COMAPS reads OsmAnd, and one naming an app this build
    /// does not know keeps the default rather than landing on nothing.
    #[test]
    fn the_nav_app_defaults_to_osmand_for_old_and_unknown_files() {
        let d = tmpdir("nav-app-old");
        fs::write(path(&d), r#"{"navOn":true}"#).unwrap();
        assert_eq!(load(&d).nav_app, NavApp::OsmAnd);
        let d = tmpdir("nav-app-unknown");
        fs::write(path(&d), r#"{"navApp":"waze"}"#).unwrap();
        assert_eq!(load(&d).nav_app, NavApp::OsmAnd);
        let d = tmpdir("nav-app-comaps");
        fs::write(path(&d), r#"{"navApp":"comaps"}"#).unwrap();
        assert_eq!(load(&d).nav_app, NavApp::CoMaps);
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
            r#"{"presets":[88.7,105.5],"theme":"chartreuse","logosOn":"yes","diagOn":true}"#,
        )
        .unwrap();
        let p = load(&d);
        assert_eq!(p.presets, vec![at(88.7, None), at(105.5, None)], "presets survive");
        assert_eq!(p.theme, Theme::Headlights, "unknown theme falls back");
        assert_eq!(p.logos_on, Prefs::default().logos_on, "wrong type falls back");
        assert!(p.diag_on, "the readable field is still read");
    }

    /// A FILE FROM A BUILD BEFORE FIELDS WERE DROPPED still reads.
    ///
    /// Now covers two removals, not one: `autostart`, and the three diagnostics
    /// flags that went with CarFM's tools — `diagOverlayOn`, `rdsCaptureOn` and
    /// `debugOn`. Every unit in the field has all four in its file.
    ///
    /// This is the one way removing a persisted field can hurt a driver, and it
    /// is worth a test rather than an assurance: `from_json` reads named fields
    /// and ignores everything else, so a key nobody asks for any more costs
    /// nothing. A serde struct with `deny_unknown_fields` would have failed the
    /// whole file here and reset every preset on the unit.
    #[test]
    fn a_file_written_before_autostart_was_dropped_still_loads() {
        let d = tmpdir("stale-key");
        fs::write(
            path(&d),
            concat!(
                r#"{"presets":[{"mhz":102.1,"call":"WQLF"},88.7],"selected":"nwd","#,
                r#""theme":"dark","autostart":true,"logosOn":true,"diagOn":true,"#,
                r#""diagOverlayOn":false,"rdsCaptureOn":true,"debugOn":false}"#
            ),
        )
        .unwrap();
        let p = load(&d);
        assert_eq!(p.presets, vec![at(102.1, Some("WQLF")), at(88.7, None)]);
        assert_eq!(p.selected, Source::Nwd);
        assert_eq!(p.theme, Theme::Dark);
        assert!(p.logos_on && p.diag_on);
        // A file written before `presetLoop` existed comes back with looping
        // OFF, which is the handoff's default (§8: "opt-in via the `presetLoop`
        // prop (default off)"). An absent key must never turn a rail that a
        // driver has learnt the shape of into one with no ends.
        assert!(!p.preset_loop, "a file predating the key leaves looping off");
        assert!(!p.come_forward, "and predating comeForward leaves that off too");
        let _ = fs::remove_dir_all(&d);
    }

    /// A dial the front end cannot reach is corruption, not a preset.
    #[test]
    fn out_of_band_presets_are_dropped() {
        let d = tmpdir("band");
        fs::write(path(&d), r#"{"presets":[88.7,0,1e9,-5,107.9,108.1]}"#).unwrap();
        assert_eq!(load(&d).presets, vec![at(88.7, None), at(107.9, None)]);
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
        save(&d, &Prefs { presets: vec![at(90.1, None)], ..Prefs::default() });
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
        assert_eq!(p.logos_on, s.logos_on);
    }
}
