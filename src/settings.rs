//! The settings panel's derived layer.
//!
//! `ui/settings.slint` decides nothing: it is handed finished titles, finished
//! sub-lines, and a finished list of action rows with their rules already
//! applied. Everything that decides any of that
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

/// What a DIAGNOSTICS row does.
///
/// TWO ROWS, AND THEY ARE THE MECHANISM RATHER THAN AN INVESTIGATION. There were
/// seven: five more carried CarFM's own probes across — export the raw RDS
/// capture, dump the head unit's boot settings, probe the vendor-app trampoline,
/// dump every tuner getter, probe `NwdFmManager`. Those were built to answer
/// CarFM's questions, and porting them was reading someone else's notebook as a
/// specification. They are gone. What is left is somewhere to write a line and a
/// way to read it back on a unit with no adb, which is what any NEW diagnostic
/// this project needs will be built on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SaveLog,
    ClearLog,
    /// Ask what could keep this app alive through a sleep. See
    /// `CarnyxKeepAlive.java`.
    ///
    /// A NEW ROW, NOT A RETURNING ONE. The five that went were CarFM's probes,
    /// carried across without a question of our own behind them. This one has
    /// one: the MCU kills apps on ACC-off, CarFM recorded that as settled and
    /// built its wake design around it, and nobody has ever checked whether this
    /// ROM has the keep-alive list vendor Androids usually do. The answer
    /// decides whether #67's wake receiver is the whole story.
    ProbeKeepAlive,
}

/// The rows that exist right now, in order, with their dividers.
///
/// No longer conditional on anything. The old list grew and shrank with the
/// capture flag and the live source, because four of its rows only meant
/// anything against the vendor tuner; with those gone both rows always apply.
pub fn diag_actions() -> Vec<DiagAction> {
    vec![
        DiagAction { label: "Save to file".into(), divider_above: false, action: Action::SaveLog },
        DiagAction {
            label: "What could keep Carnyx alive through sleep".into(),
            divider_above: true,
            action: Action::ProbeKeepAlive,
        },
        DiagAction { label: "Clear log".into(), divider_above: true, action: Action::ClearLog },
    ]
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

/// Everything the panel remembers.
///
/// SEVEN OF THESE SURVIVE A LAUNCH and the rest do not, which is deliberate
/// rather than unfinished. [`crate::prefs`] stores the preset strip, the selected
/// source, the theme, the logo switch and the four diagnostics switches.
///
/// THAT IS NOT CARFM'S SET "KEY FOR KEY", and this doc used to claim it was.
/// CarFM keeps `@carfm/{preset_order_v1, tuner_backend_v1, theme_v1,
/// logos_enabled_v1, diag_enabled, diag_overlay, rds_capture, debug_mode_v1,
/// battery_prompted_v1, freq_callsign_v1}`. The learned call-sign map is here
/// too, in its own file; `battery_prompted` is the one genuinely absent.
///
/// `autostart` was in this list and is NOT persisted any more. It has no key
/// under `@carfm/` at all — CarFM's is `@vibesdr/car_autostart`
/// (services/carMode.ts:13), it is VibeSDR lineage, and what it starts is a
/// plugged-in RTL-SDR. Carnyx has no SDR and can declare no boot receiver, so
/// the row reports itself unavailable and there is nothing left to remember.
///
/// The rest are SESSION state and belong nowhere else: `battery` is read from
/// the OS every launch, `clearing_logos` is a spinner, `details_open` is whether
/// a disclosure is open right now, and `log` is a ring buffer of this session.
#[derive(Debug)]
pub struct Settings {
    pub selected: Source,
    pub theme: Theme,
    pub battery: Battery,
    pub logos_on: bool,
    pub clearing_logos: bool,
    pub details_open: bool,
    /// Hand the FM source back when the head unit says it is going to sleep
    /// (#92).
    ///
    /// DEFAULT ON, which is the only default that matches what it is for. The
    /// MCU remembers the current source across a sleep and restores it on ACC-on,
    /// so a unit left on FM comes back into FM and the stock radio app launches
    /// itself. Handing the source back means there is nothing for it to restore.
    /// Off is for a driver who would rather Carnyx kept it, and for finding out
    /// whether this path is the cause of something else.
    pub release_on_sleep: bool,
    /// The master switch for the log itself. The three flags that used to sit
    /// beside it — mirror the log onto the face, capture raw RDS, reception
    /// testing mode — were CarFM's investigation tools and are gone.
    pub diag_on: bool,
    pub log: DiagLog,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            selected: Source::Nwd,
            theme: Theme::System,
            battery: Battery::NotExempt,
            logos_on: false,
            clearing_logos: false,
            details_open: false,
            release_on_sleep: true,
            diag_on: false,
            log: DiagLog::new(),
        }
    }
}

impl Settings {
    /// Turn the log on or off. Nothing hangs off it any more — the three flags
    /// that did were CarFM's and have been removed — so this is now the plain
    /// setter it looks like.
    pub fn set_diag(&mut self, on: bool) {
        self.diag_on = on;
    }

    /// The rows the DIAGNOSTICS action list currently has.
    pub fn actions(&self) -> Vec<DiagAction> {
        diag_actions()
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

    /// TWO ROWS, ALWAYS, AND ONLY THE FIRST HAS NO RULE ABOVE IT.
    ///
    /// This test used to enumerate seven and assert which of them appeared under
    /// which conditions. Five were CarFM's vendor probes — export the raw
    /// capture, dump the boot settings, probe the trampoline, dump every getter,
    /// probe `NwdFmManager` — and none of them had ever been written. They are
    /// gone, and with them the whole idea of a list that changes shape.
    #[test]
    fn the_action_rows_are_the_mechanism_and_nothing_else() {
        let rows = diag_actions();
        assert_eq!(
            rows.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(),
            ["Save to file", "What could keep Carnyx alive through sleep", "Clear log"]
        );
        assert!(!rows[0].divider_above, "the log well runs straight into the first");
        assert!(rows[1..].iter().all(|a| a.divider_above));
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

    /// THE LOG SWITCH HAS NO DEPENDANTS ANY MORE, which is the point of the
    /// test rather than a reason to delete it. It used to drag three flags down
    /// with it — mirror to the face, raw capture, reception testing — and one of
    /// those turned it back ON when set. All three were CarFM's and are gone, so
    /// this is now a plain switch and must stay one.
    #[test]
    fn the_log_switch_is_a_plain_switch() {
        let mut s = Settings::default();
        s.set_diag(true);
        assert!(s.diag_on);
        s.set_diag(false);
        assert!(!s.diag_on);
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
