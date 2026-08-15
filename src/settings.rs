//! The settings panel's derived layer.
//!
//! `ui/settings.slint` decides nothing: it is handed finished titles, finished
//! sub-lines, a finished list of action rows with their rules already applied,
//! and an index saying which egg is ticked. Everything that decides any of that
//! is here, and it is all pure — no Slint types, no framework calls — so it can
//! be tested on a machine with no head unit.
//!
//! The content is the SHIPPING `SettingsPanel.tsx`, not the design bundle. The
//! bundle documents an ADVANCED section CarFM removed and misses DIAGNOSTICS
//! entirely; every string below was read off the TSX.
//!
//! ## What is NOT here
//!
//! The framework edge. Every DIAGNOSTICS action row crosses it — a native log
//! write, a vendor-service probe, a `su -c id` recon — and none of that exists
//! in this container or in this crate. `DiagAction` names a row and says where
//! the rules fall; performing one is [`Action`], and the handler for it is
//! deliberately a refusal (see `crate::app`).

use std::collections::VecDeque;

// ── Which tuner is driving ───────────────────────────────────────────────────

/// The four rows of the source picker, in the reference's order.
///
/// Only [`Source::Nwd`] has an implementation. RTL-SDR is VibeSDR's, which the
/// provenance rule bars from this tree, and the FYT/DuduOS path never existed in
/// CarFM either — both rows are drawn because the picker is four rows in every
/// reference, and both report themselves unavailable rather than pretending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Rtl,
    Nwd,
    Fyt,
    Auto,
}

impl Source {
    pub const ORDER: [Source; 4] = [Source::Rtl, Source::Nwd, Source::Fyt, Source::Auto];

    pub fn name(self) -> &'static str {
        match self {
            Source::Rtl => "RTL-SDR",
            Source::Nwd => "NWD / NOWADA built-in radio",
            Source::Fyt => "FYT / DuduOS built-in radio",
            Source::Auto => "Auto",
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Source::Rtl => "USB software-defined radio",
            Source::Nwd | Source::Fyt => "Integrated head-unit FM tuner",
            Source::Auto => "Probe all sources",
        }
    }
}

/// One finished row of the source picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRow {
    pub name: String,
    pub kind: String,
    /// "Detected" / "Not detected" / "Unavailable", or empty for Auto — which
    /// carries no badge because it is not a device.
    pub badge: String,
    pub badge_lit: bool,
    pub available: bool,
    pub selected: bool,
}

// ── Theme ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ORDER: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

    /// The chip label. UPPER CASE is the chip's own styling in the reference and
    /// it is baked into the string because the chip is matched by label.
    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "SYSTEM",
            Theme::Light => "LIGHT",
            Theme::Dark => "DARK",
        }
    }

    /// Match a tapped chip back to the enum. An unknown label leaves the current
    /// choice alone rather than silently resetting to SYSTEM.
    pub fn parse(label: &str) -> Option<Theme> {
        Theme::ORDER.into_iter().find(|t| t.label() == label)
    }
}

pub fn theme_chips() -> Vec<String> {
    Theme::ORDER.iter().map(|t| t.label().to_string()).collect()
}

// ── Battery ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Battery {
    Checking,
    Exempt,
    NotExempt,
}

impl Battery {
    pub fn sub(self) -> &'static str {
        match self {
            Battery::Checking => "Checking…",
            Battery::Exempt => "Exempt — the boot-started radio will keep running",
            Battery::NotExempt => "Not exempt — Doze may stop the boot-started radio",
        }
    }
}

// ── DIAGNOSTICS ──────────────────────────────────────────────────────────────

/// One action row of the DIAGNOSTICS section: a label, and whether a rule is
/// drawn above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagAction {
    pub label: String,
    pub divider_above: bool,
    /// What a tap actually asks for. `label` is what is drawn; this is what is
    /// dispatched, so re-wording a row cannot change what it does.
    pub action: Action,
}

/// What a DIAGNOSTICS row does. Every variant but [`Action::ClearLog`] crosses
/// the framework edge and has no implementation on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SaveLog,
    ExportRdsCapture,
    DumpSettings,
    ProbeTrampoline,
    RunRdsProbe,
    ProbeFmManager,
    ClearLog,
}

/// The rows that exist right now, in order, with their dividers.
///
/// Read off `SettingsPanel.tsx:565-609` row by row rather than re-derived: "Save
/// to file" is always present and carries NO rule (the log well runs straight
/// into it), the raw-capture export appears only while capture is on, the four
/// vendor probes appear only while the built-in tuner is the live source, and
/// "Clear log" is always last. Every row after the first carries a rule.
pub fn diag_actions(rds_capture_on: bool, nwd_active: bool) -> Vec<DiagAction> {
    let mut rows = vec![DiagAction {
        label: "Save to file".into(),
        divider_above: false,
        action: Action::SaveLog,
    }];
    let mut push = |label: &str, action: Action| {
        rows.push(DiagAction { label: label.into(), divider_above: true, action });
    };
    if rds_capture_on {
        push("Export raw RDS capture", Action::ExportRdsCapture);
    }
    if nwd_active {
        push(
            "Dump head unit settings (boot / power-on behaviour)",
            Action::DumpSettings,
        );
        push(
            "Probe vendor-app replacement (read-only; may prompt for root)",
            Action::ProbeTrampoline,
        );
        push("Run RDS probe (dumps every tuner getter)", Action::RunRdsProbe);
        push(
            "Probe NwdFmManager (signal \u{00B7} raw RDS \u{00B7} stereo)",
            Action::ProbeFmManager,
        );
    }
    push("Clear log", Action::ClearLog);
    rows
}

/// The tuner log: a bounded ring of already-stamped lines, oldest first.
///
/// The stamp is passed in rather than read from a clock, because a log whose
/// content depends on when the test ran is a log that cannot be pinned.
#[derive(Debug, Default)]
pub struct DiagLog {
    lines: VecDeque<String>,
}

impl DiagLog {
    /// CarFM keeps 200. The face shows a handful and the file export takes the
    /// lot, so the only thing the bound protects is memory on a unit that runs
    /// for days.
    pub const CAP: usize = 200;

    pub fn new() -> DiagLog {
        DiagLog::default()
    }

    /// `HH:MM:SS␣␣text` — two spaces, which is what makes the stamps read as a
    /// column in a proportional face.
    pub fn push(&mut self, stamp: &str, text: &str) {
        if self.lines.len() == Self::CAP {
            self.lines.pop_front();
        }
        self.lines.push_back(format!("{stamp}  {text}"));
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

// ── The BAND THEMES egg ──────────────────────────────────────────────────────

/// Six taps on the about line opens it. The counter never resets and is never
/// persisted, so the section stays for the life of the process and is gone on the
/// next launch — `SettingsPanel.tsx:88-89`.
pub const EGG_TAPS: u32 = 6;

/// The puns, in registry order, with the "off" head in front.
///
/// Transcribed from `bandThemes.ts:215-221`, including U+2019 in "Now I’m
/// Nothing". The ids they map to (`AC/DC`, `The Beatles`, …) never reach Slint:
/// an index is what crosses, because the labels are puns and two could one day
/// rhyme.
pub fn egg_labels() -> Vec<String> {
    [
        "Off (auto-detect)",
        "Powerage",
        "The Walrus was Paul",
        "Hammer of the Gods",
        "Smells Like Gen X",
        "Now I\u{2019}m Nothing",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The theme id an egg index forces, or `None` for "off (auto-detect)".
pub fn egg_id(index: i32) -> Option<&'static str> {
    match index {
        1 => Some("AC/DC"),
        2 => Some("The Beatles"),
        3 => Some("Led Zeppelin"),
        4 => Some("Nirvana"),
        5 => Some("Nine Inch Nails"),
        _ => None,
    }
}

// ── The about line ───────────────────────────────────────────────────────────

/// `Carnyx  ·  v0.1.0  ·  FCC station data as of 2026-07-16`.
///
/// TWO spaces either side of each U+00B7, and an em dash when there is no
/// snapshot date (`SettingsPanel.tsx:314`). The shape is the load-bearing part
/// and it is pinned by a test below.
pub fn about_line(product: &str, version: &str, snapshot: Option<&str>) -> String {
    let date = snapshot.unwrap_or("\u{2014}");
    format!("{product}  \u{00B7}  v{version}  \u{00B7}  FCC station data as of {date}")
}

// ── Tuner status ─────────────────────────────────────────────────────────────

/// The status block's three finished strings and its two enums' worth of choice,
/// as plain data. `crate::app` maps the two `&'static str` discriminants onto the
/// generated Slint enums.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub title: String,
    pub sub: String,
    /// "waves" when a tuner is live, "warn" when it is not.
    pub glyph: &'static str,
    /// "none", "retry" or "details".
    pub action: &'static str,
}

/// The three-way sub-line of `SettingsPanel.tsx:344-348`, and the action that
/// goes with it.
///
/// THE BUTTON AND THE PANEL DISAGREE, and that is CarFM's defect reproduced
/// rather than quietly corrected. The Details BUTTON below appears on
/// `!error && rtl` (`SettingsPanel.tsx:358`), while the panel it opens is gated
/// on `!error && !nwd_active` (`:369`) — see [`details_open`]. Select RTL-SDR
/// while the built-in tuner is the live source and the button is there,
/// tappable, and opens nothing. It is documented at `details-open` in
/// `ui/settings.slint` too, and it wants raising with the owner rather than
/// fixing inside a port.
pub fn status(error: bool, selected: Source) -> Status {
    if error {
        return Status {
            title: "Not connected".into(),
            sub: "No USB tuner found".into(),
            glyph: "warn",
            action: "retry",
        };
    }
    let rtl = selected == Source::Rtl;
    Status {
        title: "Connected".into(),
        sub: if rtl {
            "Local hardware \u{00B7} RTL-SDR (RTL2832U)".into()
        } else {
            "Built-in hardware \u{00B7} NWD/NOWADA FM tuner".into()
        },
        glyph: "waves",
        action: if rtl { "details" } else { "none" },
    }
}

/// Whether the details PANEL may open — the other half of the mismatch above,
/// and deliberately a different expression from the button's.
pub fn details_open(requested: bool, error: bool, nwd_active: bool) -> bool {
    requested && !error && !nwd_active
}

// ── The SYSTEM sub-lines ─────────────────────────────────────────────────────

pub fn logos_sub(on: bool) -> &'static str {
    if on {
        "Auto-download station artwork over Wi-Fi"
    } else {
        "Off \u{2014} assign logos manually from a station (auto-download in redesign)"
    }
}

pub fn clear_logos_label(clearing: bool) -> &'static str {
    if clearing {
        "Clearing\u{2026}"
    } else {
        "Clear all station logos"
    }
}

// ── The whole panel's mutable state ──────────────────────────────────────────

/// Everything the panel remembers. No persistence: CarFM stores these in
/// AsyncStorage and Carnyx has no preference store yet, so every one of them is
/// back at its default on the next launch. That is a missing subsystem, not a
/// design choice, and it is stated here so nobody reads the defaults as settled.
#[derive(Debug)]
pub struct Settings {
    pub selected: Source,
    pub theme: Theme,
    pub autostart: bool,
    pub battery: Battery,
    pub logos_on: bool,
    pub clearing_logos: bool,
    pub details_open: bool,
    pub diag_on: bool,
    pub diag_overlay_on: bool,
    pub rds_capture_on: bool,
    pub debug_on: bool,
    pub egg_index: i32,
    pub about_taps: u32,
    pub log: DiagLog,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            selected: Source::Nwd,
            theme: Theme::System,
            autostart: true,
            battery: Battery::NotExempt,
            logos_on: false,
            clearing_logos: false,
            details_open: false,
            diag_on: false,
            diag_overlay_on: false,
            rds_capture_on: false,
            debug_on: false,
            egg_index: 0,
            about_taps: 0,
            log: DiagLog::new(),
        }
    }
}

impl Settings {
    /// One tap on the about line. Returns whether the egg is now open.
    pub fn tap_about(&mut self) -> bool {
        self.about_taps = self.about_taps.saturating_add(1);
        self.egg_open()
    }

    pub fn egg_open(&self) -> bool {
        self.about_taps >= EGG_TAPS
    }

    /// Reception testing implies capture (`SettingsPanel.tsx:107-114`): the
    /// sampler's whole output is the captured stream, so a debug session with
    /// capture off records nothing.
    pub fn set_debug(&mut self, on: bool) {
        self.debug_on = on;
        if on {
            self.diag_on = true;
        }
    }

    /// Turning the log off takes its dependants with it. The TSX leaves them set
    /// and merely stops drawing them, which is how it can show a stored `true`
    /// against a section that is not running.
    pub fn set_diag(&mut self, on: bool) {
        self.diag_on = on;
        if !on {
            self.diag_overlay_on = false;
            self.rds_capture_on = false;
            self.debug_on = false;
        }
    }

    /// The rows the DIAGNOSTICS action list currently has.
    pub fn actions(&self, nwd_active: bool) -> Vec<DiagAction> {
        diag_actions(self.rds_capture_on, nwd_active)
    }

    /// The four rows of the source picker, with availability filled in.
    ///
    /// `nwd_available` is the only one that is ever probed. The other two report
    /// what is true: there is no RTL-SDR support in this tree and never will be
    /// under the provenance rule, and no FYT path was ever written.
    pub fn sources(&self, nwd_available: bool) -> Vec<SourceRow> {
        Source::ORDER
            .iter()
            .map(|&s| {
                let (badge, badge_lit, available) = match s {
                    Source::Nwd if nwd_available => ("Detected", true, true),
                    Source::Nwd => ("Not detected", false, false),
                    Source::Rtl => ("Not detected", false, true),
                    Source::Fyt => ("Unavailable", false, false),
                    Source::Auto => ("", false, true),
                };
                SourceRow {
                    name: s.name().to_string(),
                    kind: s.kind().to_string(),
                    badge: badge.to_string(),
                    badge_lit,
                    available,
                    selected: s == self.selected,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The order and the dividers, read off `SettingsPanel.tsx:565-609`. The
    /// first row's missing rule is the whole point: the log well runs into it.
    #[test]
    fn the_action_rows_appear_by_their_own_rules() {
        let base = diag_actions(false, false);
        assert_eq!(
            base.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
            ["Save to file", "Clear log"]
        );
        assert!(!base[0].divider_above);
        assert!(base[1].divider_above);

        let capture = diag_actions(true, false);
        assert_eq!(capture[1].label, "Export raw RDS capture");
        assert_eq!(capture.len(), 3);

        let full = diag_actions(true, true);
        assert_eq!(
            full.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
            [
                "Save to file",
                "Export raw RDS capture",
                "Dump head unit settings (boot / power-on behaviour)",
                "Probe vendor-app replacement (read-only; may prompt for root)",
                "Run RDS probe (dumps every tuner getter)",
                "Probe NwdFmManager (signal \u{00B7} raw RDS \u{00B7} stereo)",
                "Clear log",
            ]
        );
        // Every row after the first carries a rule, and only the first does not.
        assert!(!full[0].divider_above);
        assert!(full[1..].iter().all(|a| a.divider_above));
        // The label is drawn, the action is dispatched, and they are separate —
        // so re-wording a row cannot change what it does.
        assert_eq!(full[5].action, Action::ProbeFmManager);
    }

    #[test]
    fn the_egg_menu_is_the_five_puns_behind_an_off_head() {
        let labels = egg_labels();
        assert_eq!(labels.len(), 6);
        assert_eq!(labels[0], "Off (auto-detect)");
        // U+2019 RIGHT SINGLE QUOTATION MARK, not an ASCII apostrophe — the
        // difference is invisible in a diff and visible on the face.
        assert_eq!(labels[5], "Now I\u{2019}m Nothing");
        assert!(labels[5].contains('\u{2019}'));
        assert_eq!(egg_id(0), None);
        assert_eq!(egg_id(1), Some("AC/DC"));
        assert_eq!(egg_id(5), Some("Nine Inch Nails"));
        // Out of range is "off", never a panic and never a neighbour.
        assert_eq!(egg_id(6), None);
        assert_eq!(egg_id(-1), None);
    }

    #[test]
    fn six_taps_open_the_egg_and_it_never_closes() {
        let mut s = Settings::default();
        for _ in 0..5 {
            assert!(!s.tap_about());
        }
        assert!(s.tap_about());
        // It never resets: a seventh tap is not a toggle.
        assert!(s.tap_about());
        assert!(s.egg_open());
    }

    /// The shape, byte for byte. Two spaces either side of each U+00B7 is what
    /// makes the line read as three fields rather than one sentence.
    #[test]
    fn the_about_line_keeps_its_double_spaced_interpuncts() {
        assert_eq!(
            about_line("Carnyx", "0.1.0", Some("2026-07-16")),
            "Carnyx  \u{00B7}  v0.1.0  \u{00B7}  FCC station data as of 2026-07-16"
        );
        // No database, no date: an em dash, not "null" and not an empty tail.
        assert_eq!(
            about_line("Carnyx", "0.1.0", None),
            "Carnyx  \u{00B7}  v0.1.0  \u{00B7}  FCC station data as of \u{2014}"
        );
        let line = about_line("Carnyx", "0.1.0", Some("x"));
        assert_eq!(line.matches("  \u{00B7}  ").count(), 2);
    }

    #[test]
    fn the_status_block_is_three_way_and_reproduces_carfms_details_defect() {
        let nwd = status(false, Source::Nwd);
        assert_eq!(nwd.title, "Connected");
        assert_eq!(nwd.sub, "Built-in hardware \u{00B7} NWD/NOWADA FM tuner");
        assert_eq!(nwd.action, "none");

        let err = status(true, Source::Nwd);
        assert_eq!(err.sub, "No USB tuner found");
        assert_eq!(err.glyph, "warn");
        assert_eq!(err.action, "retry");

        // The defect, pinned so nobody "fixes" it by accident: RTL selected while
        // the built-in tuner is live still offers Details, and the panel behind
        // it stays shut. Two expressions, one button, on purpose.
        assert_eq!(status(false, Source::Rtl).action, "details");
        assert!(!details_open(true, false, true));
        // With no built-in tuner live, the same request does open it.
        assert!(details_open(true, false, false));
    }

    #[test]
    fn the_log_is_a_bounded_ring_of_stamped_lines() {
        let mut log = DiagLog::new();
        assert!(log.is_empty());
        log.push("12:00:01", "connect ok");
        assert_eq!(log.lines(), ["12:00:01  connect ok"]);
        for i in 0..DiagLog::CAP {
            log.push("12:00:02", &format!("line {i}"));
        }
        assert_eq!(log.lines().len(), DiagLog::CAP);
        // The oldest went, not the newest.
        assert_eq!(log.lines()[DiagLog::CAP - 1], "12:00:02  line 199");
        assert!(!log.lines().iter().any(|l| l.ends_with("connect ok")));
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn turning_the_log_off_takes_its_dependants_with_it() {
        let mut s = Settings::default();
        s.set_diag(true);
        s.diag_overlay_on = true;
        s.rds_capture_on = true;
        s.set_debug(true);
        assert!(s.diag_on && s.debug_on);
        s.set_diag(false);
        assert!(!s.diag_overlay_on && !s.rds_capture_on && !s.debug_on);
        // And reception testing turns the log back on, because its output IS the
        // log — SettingsPanel.tsx:107-114.
        s.set_debug(true);
        assert!(s.diag_on);
    }

    #[test]
    fn the_source_picker_is_four_rows_and_only_one_can_be_probed() {
        let s = Settings::default();
        let rows = s.sources(true);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].name, "NWD / NOWADA built-in radio");
        assert!(rows[1].selected && rows[1].badge_lit && rows[1].available);
        // Auto carries no badge; it is not a device.
        assert_eq!(rows[3].badge, "");
        // No head unit: the row that matters goes dark, and the two that were
        // never implemented say so either way.
        let absent = s.sources(false);
        assert_eq!(absent[1].badge, "Not detected");
        assert!(!absent[1].available);
        assert_eq!(absent[2].badge, "Unavailable");
    }

    #[test]
    fn a_theme_chip_round_trips_and_an_unknown_label_is_refused() {
        assert_eq!(theme_chips(), ["SYSTEM", "LIGHT", "DARK"]);
        assert_eq!(Theme::parse("DARK"), Some(Theme::Dark));
        assert_eq!(Theme::parse("Dark"), None);
        assert_eq!(Theme::parse(""), None);
    }
}
