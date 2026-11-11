//! Turn-by-turn from CoMaps, read into the same [`Route`] OsmAnd's poll fills.
//!
//! ## What CoMaps offers, and since when
//!
//! CoMaps (`app.comaps.*`, a fork of Organic Maps) has published a read-only
//! content provider for live navigation since its v2026.07.16 release —
//! `NavigationContentProvider`, authority `<package>.provider.navigation`, one
//! row at `/live`, guarded by `<package>.permission.READ_NAVIGATION_DATA`
//! (protection level `dangerous`, so the driver grants it in a dialog). Organic
//! Maps has no such provider; this is CoMaps' own.
//!
//! `CarnyxComaps.java` is the wire: it watches that URI, queries it when CoMaps
//! says it changed, and hands every column across as a NAME and a STRING. This
//! module decides what any of it means, for the reason `crate::nav`'s header
//! gives — a decision made in Java cannot be tested on a machine with no head
//! unit, and every decision below is tested.
//!
//! ## Three facts about the feed that shape everything here
//!
//! All three were read in CoMaps' source rather than assumed.
//!
//! 1. **THE PUSH IS THE LIVENESS.** CoMaps' `NavigationService.onLocationUpdated`
//!    refreshes the cached `RoutingInfo` and calls `notifyChange` on every GPS
//!    fix while it is navigating, and on nothing else but a route being built or
//!    cleared. So a row arriving IS "a route is running and CoMaps just moved"
//!    — the role OsmAnd's 1 Hz poll plays — and [`crate::nav::Nav::EXPIRY`]
//!    ages it out when the fixes stop exactly as it ages out a silent poll.
//! 2. **THE CACHE IS NEVER CLEARED.** `RoutingController.mCachedRoutingInfo` is
//!    written in three places and reset in none: when a route ends, the provider
//!    goes on answering with the last row, `session_state` still `OnRoute`, for
//!    as long as CoMaps runs. Carnyx therefore never POLLS this provider — a poll
//!    would draw a finished route forever — and treats `route_point_count = 0`
//!    (the geometry IS cleared on cancel, on builds from v2026.08.31) as "no
//!    route" whatever the state column says.
//! 3. **THE DISTANCES ARE DISPLAY STRINGS.** `dist_to_turn` and `dist_to_target`
//!    are `Distance.toString(context)` — "350 ft", "0.2 mi", "1,2 km" — rounded
//!    for a screen and localised. [`distance`] reads them back to metres, and the
//!    unit word is also how this module learns which units the driver chose in
//!    CoMaps (see [`Reading::units`]).
//!
//! ## What CoMaps does not send, and what that costs the face
//!
//! The provider carries no turn-after-next (no `THEN` block), no spoken
//! instruction text, and no "map in front" flag, so those three collapse on
//! this provider. It also carries no `imminent` ladder, so [`imminent`] derives
//! one from CoMaps' OWN voice timing — the nearest thing to §4.9's rule that
//! the face escalate "at the same moment the voice speaks".

use crate::nav::{Route, Turn};
use crate::units::Units;

/// One `/live` row, column by column, exactly as the cursor gave it.
///
/// NAMES AND STRINGS, NOT A STRUCT ON THE JAVA SIDE, because the column set is
/// CoMaps' and it grows: v2026.07.16 has twelve columns, v2026.08.31 seventeen.
/// A name lookup reads whichever build is installed; a positional one would
/// read the wrong column on the other.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    cells: Vec<(String, Option<String>)>,
}

impl Row {
    /// Pair the two arrays the seam delivers. A length mismatch keeps the
    /// shorter — it cannot happen from `CarnyxComaps.queryOnce`, which builds
    /// both from one cursor, and dropping a column is safer than misaligning
    /// every one after it.
    pub fn new(names: Vec<String>, values: Vec<Option<String>>) -> Row {
        Row { cells: names.into_iter().zip(values).collect() }
    }

    /// An EMPTY cursor — CoMaps has never cached a route this run.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// One column, trimmed, with empty and null both meaning "absent".
    pub fn get(&self, column: &str) -> Option<&str> {
        self.cells
            .iter()
            .find(|(name, _)| name == column)
            .and_then(|(_, v)| v.as_deref())
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    fn int(&self, column: &str) -> Option<i64> {
        // `MatrixCursor.getString` on an Integer column is `String.valueOf`, so
        // an int arrives as its decimal digits. A double column ("0.0") would
        // not parse as an integer and is not one of the columns read this way.
        self.get(column)?.parse::<i64>().ok()
    }
}

/// `RoutingInfo.RoutingSessionState`, by the enum NAME the provider writes
/// (`info.routingSessionState.name()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    NoValidRoute,
    RouteBuilding,
    RouteNotStarted,
    OnRoute,
    RouteNeedsRebuild,
    RouteFinished,
    RouteNoFollowing,
    RouteRebuilding,
    OffRoute,
}

impl Session {
    /// The name, or `None` for one this build does not know — which is read as
    /// "no route", the quiet answer, never as a route.
    pub fn parse(name: &str) -> Option<Session> {
        Some(match name {
            "NoValidRoute" => Session::NoValidRoute,
            "RouteBuilding" => Session::RouteBuilding,
            "RouteNotStarted" => Session::RouteNotStarted,
            "OnRoute" => Session::OnRoute,
            "RouteNeedsRebuild" => Session::RouteNeedsRebuild,
            "RouteFinished" => Session::RouteFinished,
            "RouteNoFollowing" => Session::RouteNoFollowing,
            "RouteRebuilding" => Session::RouteRebuilding,
            "OffRoute" => Session::OffRoute,
            _ => return None,
        })
    }
}

/// CoMaps' `CarDirection`, by enum NAME, as a [`Turn`].
///
/// `None` IS A REAL ANSWER, the same rule [`Turn::from_osmand`] states: a name
/// this table does not have is a direction added after it was written, and
/// drawing it as anything would be an instruction given confidently. The two
/// that are not turns at all — `REACHED_YOUR_DESTINATION` and
/// `START_AT_THE_END_OF_STREET` — are `None` too; the route is still running,
/// so the face shows the distance and no arrow.
///
/// THREE READINGS ARE CHOICES, and each is CoMaps' own where it has one:
///
/// * `NO_TURN` is drawn STRAIGHT because CoMaps draws it straight — its own
///   `CarDirection.NO_TURN` carries `R.drawable.ic_turn_straight`.
/// * A HIGHWAY EXIT is a KEEP: `EXIT_HIGHWAY_TO_RIGHT` is a fork to the right,
///   which is what `KR` draws. There is no exit-ramp glyph in [`Turn`].
/// * EVERY ROUNDABOUT IS THE RIGHT-HAND-TRAFFIC ONE (`RNDB`). The provider does
///   not export `RoutingInfo.isLeftHandTraffic`, and this app is North American
///   by design (see `crate::units`); a UK drive would draw the ring the wrong
///   way round with the right exit number in it.
pub fn turn(direction: &str) -> Option<Turn> {
    Some(match direction {
        "NO_TURN" | "GO_STRAIGHT" => Turn::Straight,
        "TURN_RIGHT" => Turn::Right,
        "TURN_SHARP_RIGHT" => Turn::SharpRight,
        "TURN_SLIGHT_RIGHT" => Turn::SlightRight,
        "TURN_LEFT" => Turn::Left,
        "TURN_SHARP_LEFT" => Turn::SharpLeft,
        "TURN_SLIGHT_LEFT" => Turn::SlightLeft,
        "U_TURN_LEFT" => Turn::UTurn,
        "U_TURN_RIGHT" => Turn::RightUTurn,
        "ENTER_ROUND_ABOUT" | "LEAVE_ROUND_ABOUT" | "STAY_ON_ROUND_ABOUT" => Turn::Roundabout,
        "EXIT_HIGHWAY_TO_LEFT" => Turn::KeepLeft,
        "EXIT_HIGHWAY_TO_RIGHT" => Turn::KeepRight,
        _ => return None,
    })
}

const FOOT: f64 = 0.3048;
const MILE: f64 = 1609.344;

/// Which of CoMaps' four distance units a word names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Metre,
    Kilometre,
    Foot,
    Mile,
}

impl Word {
    /// Metres per one of this unit, and which system it belongs to.
    fn scale(self) -> (f64, Units) {
        match self {
            Word::Metre => (1.0, Units::Metric),
            Word::Kilometre => (1000.0, Units::Metric),
            Word::Foot => (FOOT, Units::Imperial),
            Word::Mile => (MILE, Units::Imperial),
        }
    }
}

/// EVERY UNIT WORD COMAPS WRITES, IN EVERY TRANSLATION IT SHIPS.
///
/// GENERATED, NOT REMEMBERED. Read out of `android/sdk/src/main/res/values*/
/// strings.xml` — the `m`, `km`, `ft` and `mi` strings, 59 files — at CoMaps
/// `bf5945db` (2026-11-09): 75 distinct words, and NO word names two different
/// units in two languages, which is what makes one flat table safe.
///
/// THIS USED TO BE FOUR LATIN WORDS AND FOUR CYRILLIC ONES, under a comment
/// claiming every translation but one used them. About forty do not — Spanish
/// writes feet as `pie`, Danish `fod`, Greek metres as `μ`, Chinese `米`,
/// Portuguese `Km` — and an unread unit is not a missing distance but a missing
/// TURN: `Nav::polled_state` needs a distance to draw one, so in those languages
/// the whole countdown, arrow and street went blank (#134's bug check).
///
/// A NEW TRANSLATION CAN STILL ADD A WORD, and that is reported rather than
/// guessed: `read` names the string in `Reading::unread`, and the app logs it
/// once as "CoMaps distance not understood".
const WORDS: &[(&str, Word)] = &[
    ("m", Word::Metre),
    ("μ", Word::Metre),
    ("м", Word::Metre),
    ("מ׳", Word::Metre),
    ("متر", Word::Metre),
    ("मी", Word::Metre),
    ("মি", Word::Metre),
    ("மீ", Word::Metre),
    ("ม.", Word::Metre),
    ("ມ", Word::Metre),
    ("公尺", Word::Metre),
    ("米", Word::Metre),
    ("Km", Word::Kilometre),
    ("km", Word::Kilometre),
    ("χλμ", Word::Kilometre),
    ("км", Word::Kilometre),
    ("ק״מ", Word::Kilometre),
    ("كم", Word::Kilometre),
    ("کیلومتر", Word::Kilometre),
    ("किमी", Word::Kilometre),
    ("কিমি", Word::Kilometre),
    ("கி.மீ", Word::Kilometre),
    ("กม.", Word::Kilometre),
    ("ກມ", Word::Kilometre),
    ("公里", Word::Kilometre),
    ("Addım", Word::Foot),
    ("fit", Word::Foot),
    ("fod", Word::Foot),
    ("fot", Word::Foot),
    ("ft", Word::Foot),
    ("jalga", Word::Foot),
    ("kaki", Word::Foot),
    ("láb", Word::Foot),
    ("oin", Word::Foot),
    ("pie", Word::Foot),
    ("pé", Word::Foot),
    ("pėd.", Word::Foot),
    ("stopa", Word::Foot),
    ("vt.", Word::Foot),
    ("čevlji", Word::Foot),
    ("πόδια", Word::Foot),
    ("стопа", Word::Foot),
    ("фут", Word::Foot),
    ("רגל", Word::Foot),
    ("قدم", Word::Foot),
    ("پای", Word::Foot),
    ("फूट", Word::Foot),
    ("ফুট", Word::Foot),
    ("அடி", Word::Foot),
    ("ฟุต", Word::Foot),
    ("ຟຸດ", Word::Foot),
    ("英尺", Word::Foot),
    ("풋", Word::Foot),
    ("dặm", Word::Mile),
    ("mf", Word::Mile),
    ("mi", Word::Mile),
    ("mil", Word::Mile),
    ("milj", Word::Mile),
    ("myl", Word::Mile),
    ("myl.", Word::Mile),
    ("míl", Word::Mile),
    ("o meu", Word::Mile),
    ("μιλ", Word::Mile),
    ("ми", Word::Mile),
    ("міля", Word::Mile),
    ("מייל", Word::Mile),
    ("مایل", Word::Mile),
    ("ميل", Word::Mile),
    ("میل", Word::Mile),
    ("মাইল", Word::Mile),
    ("மை", Word::Mile),
    ("ไมล์", Word::Mile),
    ("ໄມລ໌", Word::Mile),
    ("英哩", Word::Mile),
    ("英里", Word::Mile),
];

/// A gap between number and unit: Java's U+00A0, the native side's U+202F, or
/// an ordinary space.
fn is_gap(c: char) -> bool {
    c.is_whitespace() || c == '\u{00A0}' || c == '\u{202F}'
}

/// Split a distance string into its number and its unit word.
///
/// BY SUFFIX, NOT AT THE LAST SPACE, because one unit is two words: Galician
/// writes miles as `o meu`. A table word counts only when a gap stands in front
/// of it, so `m` never matches the tail of `km`, nor `ม.` the tail of `กม.` —
/// and the longest match wins, for the same reason.
///
/// CASE IS FORGIVEN AS A FALLBACK, on the last word only: the table is exact,
/// and a translation that later capitalises `Mi` should not cost the driver
/// their turn arrow.
fn split_unit(text: &str) -> Option<(&str, Word)> {
    let exact = WORDS
        .iter()
        .filter_map(|&(w, word)| {
            let rest = text.strip_suffix(w)?;
            rest.ends_with(is_gap).then_some((rest, word, w.len()))
        })
        .max_by_key(|&(_, _, len)| len)
        .map(|(rest, word, _)| (rest, word));
    if exact.is_some() {
        return exact;
    }
    let cut = text.rfind(is_gap)?;
    let (rest, last) = text.split_at(cut);
    let last = last.trim_start_matches(is_gap).to_lowercase();
    WORDS
        .iter()
        .find(|(w, _)| w.to_lowercase() == last)
        .map(|&(_, word)| (rest, word))
}

/// One of CoMaps' distance strings, back in metres, and the units it was in.
///
/// ── THE FORMAT, FROM COMAPS' OWN SOURCE ─────────────────────────────────────
///
/// `Distance.toString(context)` is `mDistanceStr + '\u{00A0}' + unit`, where
/// the unit is the app's localised `R.string.m` / `km` / `ft` / `mi` (every one
/// of them is in [`WORDS`]) and the number comes from
/// `measurement_utils::ToStringPrecisionLocale`:
///
/// * ONE DECIMAL, and only on km or mi below 10 — "0.2 mi", "9.5 km" — with
///   the LOCALE's decimal mark ("1,2 km" in German).
/// * NO DECIMALS otherwise, and a GROUPING mark only from 10 000 up.
///
/// So a separator with exactly one digit after it is a decimal, and any other
/// separator is grouping. That rule needs no locale table, and it is exact for
/// every string that formatter can produce.
///
/// A unit word not in [`WORDS`] is `None`, and the turn waits for a distance
/// rather than being drawn against a guess.
pub fn distance(text: &str) -> Option<(i32, Units)> {
    // EVERY KIND OF GAP BECOMES ONE SPACE FIRST, so the two-word unit matches
    // whether its inner gap is the translation's plain space or something that
    // turned it non-breaking on the way.
    let text: String = text.trim().chars().map(|c| if is_gap(c) { ' ' } else { c }).collect();
    let (number, word) = split_unit(&text)?;
    let (per_unit, units) = word.scale();
    let value = read_number(number.trim_matches(is_gap))?;
    let metres = (value * per_unit).round();
    if !metres.is_finite() || metres < 0.0 || metres > f64::from(i32::MAX) {
        return None;
    }
    Some((metres as i32, units))
}

/// The number half of a distance string. See [`distance`] for the rule.
fn read_number(s: &str) -> Option<f64> {
    if s.is_empty() || !s.chars().next()?.is_ascii_digit() || !s.chars().last()?.is_ascii_digit() {
        return None;
    }
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    // THE LAST SEPARATOR, and how many digits follow it.
    let tail = s.chars().rev().take_while(char::is_ascii_digit).count();
    let has_separator = s.chars().any(|c| !c.is_ascii_digit());
    if has_separator && tail == 1 {
        let whole = &digits[..digits.len() - 1];
        let tenth = &digits[digits.len() - 1..];
        return format!("{whole}.{tenth}").parse().ok();
    }
    digits.parse().ok()
}

/// §4.9's three rungs, from CoMaps' own voice timing.
///
/// ── WHY THESE NUMBERS, AND NOT ONES OF OURS ─────────────────────────────────
///
/// §4.9 escalates "at the same moment the voice speaks — never on distance
/// thresholds of our own". OsmAnd sends its ladder; CoMaps does not, so the
/// next best thing is CoMaps' OWN rule for when it speaks, which is in
/// `libs/routing/turns_notification_manager.cpp` and `turns_sound_settings.cpp`:
///
/// * TURN NOW is spoken inside `ComputeDistToPronounceDistM` — speed × 8 s,
///   clamped to 35–150 m for a vehicle (the manager's constructor:
///   `m_settings(8, 35, 150, …)`).
/// * The FIRST warning starts inside that PLUS `ComputeTurnDistanceM` — speed ×
///   30 s, clamped to 200–2000 m in metric and 500–5000 ft in imperial
///   (`SetLengthUnits`).
///
/// Returned in OsmAnd's encoding — `0` turn now, `1` approach, `-1` cruise —
/// because [`crate::nav::Stage::from_imminent`] is where the ladder is read,
/// and one reader is better than two.
///
/// THE SPEED IS CARNYX'S OWN GPS, not CoMaps'. With no reading the clamps'
/// floors apply, which is the quiet end: a stationary car escalates at 35 m
/// and 235 m, exactly where CoMaps itself would at zero speed.
pub fn imminent(turn_metres: i32, speed_mps: Option<f32>, units: Units) -> i32 {
    let v = speed_mps
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map_or(0.0, f64::from);
    let say_now = (v * 8.0).clamp(35.0, 150.0);
    let (lo, hi) = match units {
        Units::Metric => (200.0, 2000.0),
        Units::Imperial => (500.0 * FOOT, 5000.0 * FOOT),
    };
    let first = (v * 30.0).clamp(lo, hi) + say_now;
    let d = f64::from(turn_metres);
    if d <= say_now {
        0
    } else if d <= first {
        1
    } else {
        -1
    }
}

/// What one row says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// Ready for [`crate::nav::Nav::poll`]. `Route::default()` — not navigating
    /// — for every state that is not a route being driven.
    pub route: Route,
    /// The units CoMaps wrote its distances in, when a distance was readable.
    ///
    /// THE DRIVER'S OWN CHOICE, read off the unit word. CoMaps' setting is not
    /// a column, but every distance it writes is formatted in it, so "350 ft"
    /// IS the answer — the CoMaps counterpart of OsmAnd's
    /// `default_metric_system` read (#113).
    pub units: Option<Units>,
    /// A distance string this module could not read, for the diagnostics log.
    /// A turn without a distance collapses to "navigating, no turn yet", and a
    /// drive log that does not say why would send the next reader to the wrong
    /// file.
    pub unread: Option<String>,
}

/// One row, read.
///
/// `now_ms` is Unix millis, passed in so the tests can pin it: the provider
/// gives seconds LEFT, and [`Route::arrival_ms`] is a wall-clock instant.
pub fn read(row: &Row, now_ms: i64, speed_mps: Option<f32>) -> Reading {
    // THE CLEARED GEOMETRY OUTRANKS THE STALE STATE. See fact 2 in the header:
    // after a route ends the state column keeps saying `OnRoute`, and this
    // column (from v2026.08.31) is the one that tells the truth. On an older
    // build it is absent, which is `None` here and changes nothing.
    if row.int("route_point_count") == Some(0) {
        return Reading::default();
    }
    let on_route = match row.get("session_state").and_then(Session::parse) {
        Some(Session::OnRoute) => true,
        // OFF THE ROUTE, OR RECALCULATING BECAUSE OF IT. All three are one
        // state to the face: the last turn is the one just missed, and the
        // layer goes quiet until CoMaps has a new one (`NavState::OffRoute`).
        // No distance rides with it — CoMaps does not export the deviation.
        Some(Session::OffRoute | Session::RouteNeedsRebuild | Session::RouteRebuilding) => false,
        // PLANNING, PREVIEWING, FINISHED, OR A STATE THIS BUILD DOES NOT KNOW:
        // not a route being driven.
        _ => return Reading::default(),
    };

    let mut units = None;
    let mut unread = None;
    let mut read_distance = |column: &str| -> Option<i32> {
        let text = row.get(column)?;
        match distance(text) {
            Some((m, u)) => {
                units = Some(u);
                Some(m)
            }
            None => {
                unread.get_or_insert_with(|| format!("{column} = {text:?}"));
                None
            }
        }
    };

    let mut route = Route::default();
    if on_route {
        route.turn_metres = read_distance("dist_to_turn");
        let exit = row.int("exit_num").filter(|n| *n > 0).and_then(|n| u32::try_from(n).ok());
        route.turn_xml = row.get("car_direction").and_then(turn).map(|t| t.xml(exit));
    } else {
        route.turn_xml = Some("OFFR".to_string());
    }
    route.left_metres = read_distance("dist_to_target").filter(|m| *m > 0);
    route.left_seconds = row
        .int("total_time_seconds")
        .filter(|s| *s > 0)
        .and_then(|s| i32::try_from(s).ok());
    route.arrival_ms = route.left_seconds.map(|s| now_ms + i64::from(s) * 1000);
    route.street = row.get("next_street").map(str::to_string);
    // THE LADDER ONLY WHERE THERE IS A TURN TO CLIMB TOWARDS. Off-route and a
    // turn this table cannot draw carry none, and `Stage` reads `None` as cruise.
    route.imminent = match (on_route && route.turn_xml.is_some(), route.turn_metres) {
        (true, Some(m)) => Some(imminent(m, speed_mps, units.unwrap_or(Units::Metric))),
        _ => None,
    };
    Reading { route, units, unread }
}

/// Where the CoMaps link stands, as `CarnyxComaps` reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    /// Not started, or stopped.
    #[default]
    Off,
    /// Installed, and the driver has not granted the permission yet.
    NeedsPermission,
    /// The observer is registered: every route CoMaps drives will arrive.
    Watching,
    /// The registration failed for a reason that is not the permission. The
    /// log line beside it says which.
    Failed,
}

impl Link {
    /// `CarnyxComaps.LINK_*`. Anything else is `Off`, the quiet reading.
    pub fn from_code(code: i32) -> Link {
        match code {
            1 => Link::NeedsPermission,
            2 => Link::Watching,
            3 => Link::Failed,
            _ => Link::Off,
        }
    }
}

/// Everything the settings sub-line needs when CoMaps is the selected app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status<'a> {
    /// The CoMaps that has the navigation provider, or empty.
    pub package: &'a str,
    /// A CoMaps that is installed WITHOUT the provider — a build from before
    /// v2026.07.16 — or empty.
    pub outdated: &'a str,
    /// Settings ▸ NAVIGATION ▸ Nav integration.
    pub on: bool,
    pub link: Link,
    /// A route is running and a row arrived inside `Nav::EXPIRY`.
    pub navigating: bool,
}

/// The NAVIGATION row's sub-line while CoMaps is selected.
///
/// THE SAME QUESTION `crate::nav::sub_line` ANSWERS — why is nothing showing?
/// — with CoMaps' reasons. Two of them OsmAnd does not have: a CoMaps too old
/// to have the provider, and the permission dialog. Neither reads as a fault,
/// on the same rule `the_sub_line_never_reads_as_a_fault` holds for OsmAnd.
pub fn sub_line(s: &Status) -> String {
    const WHAT: &str = "Show CoMaps' next turn on the face.";
    // THE LIVE LINK OUTRANKS THE INSTALL RECORD. A watcher that is watching, or
    // waiting on the permission, has FOUND a CoMaps with the provider — the Java
    // side looked it up itself — so "not installed" from a record read earlier
    // would contradict a mark that is lit and a turn on the face, which is what
    // #134's bug check caught. `App::refresh_nav_installs` keeps the record
    // fresh on taps; this keeps the sentence true between them.
    let live = s.on && matches!(s.link, Link::Watching | Link::NeedsPermission);
    if s.package.is_empty() && !live {
        // WHEN THE SWITCH IS ON, THE PICKER IS SHOWING, AND THE FIX IS A TAP:
        // install or update CoMaps, then tap its row, which looks again.
        let then = if s.on { " Once it is, tap CoMaps below." } else { "" };
        if !s.outdated.is_empty() {
            let then = if s.on { " After updating, tap CoMaps below." } else { "" };
            return format!(
                "{WHAT} The installed CoMaps ({}) is too old to share its navigation; \
                 version 2026.07.16 or newer can.{then}",
                s.outdated
            );
        }
        return format!("{WHAT} CoMaps is not installed.{then}");
    }
    let name = if s.package.is_empty() { "CoMaps" } else { s.package };
    if !s.on {
        return format!("{WHAT} Off. Found {name}.");
    }
    match s.link {
        Link::NeedsPermission => format!(
            "{WHAT} Waiting for permission: tap CoMaps below, then Allow."
        ),
        Link::Failed => format!(
            "{WHAT} Could not connect to {name}'s navigation feed. The log says why."
        ),
        Link::Off => format!("{WHAT} Connecting to {name}."),
        Link::Watching if !s.navigating => {
            format!("{WHAT} Connected to {name}, with no route running.")
        }
        Link::Watching => format!("{WHAT} Showing the next turn from {name}."),
    }
}

/// The sub-line under "Hide when the map is showing", while CoMaps is selected.
///
/// SAYS THE SWITCH DOES NOTHING HERE, because it does nothing here: CoMaps
/// does not tell other apps whether its map is on screen, so the face cannot
/// know to step aside. A switch that silently stops working when another row
/// changes is the kind of row this panel has twice been cut down for.
pub fn hide_sub_line() -> String {
    "CoMaps doesn't tell other apps when its map is on screen, so this has no \
     effect while CoMaps is selected."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[(&str, &str)]) -> Row {
        Row::new(
            cells.iter().map(|(k, _)| k.to_string()).collect(),
            cells.iter().map(|(_, v)| Some(v.to_string())).collect(),
        )
    }

    /// A July build's row, every column it has, mid-route. Distances carry the
    /// U+00A0 `Distance.toString` puts between number and unit.
    fn on_route() -> Vec<(&'static str, &'static str)> {
        vec![
            ("session_state", "OnRoute"),
            ("car_direction", "TURN_RIGHT"),
            ("pedestrian_direction", "NONE"),
            ("dist_to_turn", "350\u{00A0}ft"),
            ("dist_to_target", "4.2\u{00A0}mi"),
            ("dist_to_next_stop", ""),
            ("total_time_seconds", "540"),
            ("time_to_next_stop", "0"),
            ("current_street", "University Ave"),
            ("next_street", "Whitney Way"),
            ("completion_percent", "41.5"),
            ("exit_num", "0"),
        ]
    }

    /// EVERY `CarDirection` COMAPS DECLARES, against its own file.
    ///
    /// Transcribed from `android/sdk/.../routing/CarDirection.java`, SEVENTEEN
    /// constants. Fifteen draw; the two that are not turns do not. (This said
    /// eighteen and sixteen until #134's bug check counted the file.)
    #[test]
    fn every_comaps_direction_is_read_or_refused_on_purpose() {
        let drawn = [
            ("NO_TURN", Turn::Straight),
            ("GO_STRAIGHT", Turn::Straight),
            ("TURN_RIGHT", Turn::Right),
            ("TURN_SHARP_RIGHT", Turn::SharpRight),
            ("TURN_SLIGHT_RIGHT", Turn::SlightRight),
            ("TURN_LEFT", Turn::Left),
            ("TURN_SHARP_LEFT", Turn::SharpLeft),
            ("TURN_SLIGHT_LEFT", Turn::SlightLeft),
            ("U_TURN_LEFT", Turn::UTurn),
            ("U_TURN_RIGHT", Turn::RightUTurn),
            ("ENTER_ROUND_ABOUT", Turn::Roundabout),
            ("LEAVE_ROUND_ABOUT", Turn::Roundabout),
            ("STAY_ON_ROUND_ABOUT", Turn::Roundabout),
            ("EXIT_HIGHWAY_TO_LEFT", Turn::KeepLeft),
            ("EXIT_HIGHWAY_TO_RIGHT", Turn::KeepRight),
        ];
        assert_eq!(drawn.len(), 15, "fifteen of CoMaps' seventeen draw");
        for (name, t) in drawn {
            assert_eq!(turn(name), Some(t), "{name}");
        }
        for name in ["START_AT_THE_END_OF_STREET", "REACHED_YOUR_DESTINATION", "", "TURN_UP"] {
            assert_eq!(turn(name), None, "{name} is not a turn to draw");
        }
    }

    /// THE STRINGS COMAPS' FORMATTER CAN PRODUCE, and the ones it cannot.
    #[test]
    fn the_distance_strings_read_back_to_metres_and_name_their_units() {
        let nb = |s: &str| s.replace(' ', "\u{00A0}");
        assert_eq!(distance(&nb("350 ft")), Some((107, Units::Imperial)));
        assert_eq!(distance(&nb("0.2 mi")), Some((322, Units::Imperial)));
        assert_eq!(distance(&nb("120 m")), Some((120, Units::Metric)));
        assert_eq!(distance(&nb("1.3 km")), Some((1300, Units::Metric)));
        // THE LOCALE'S DECIMAL MARK — one digit after it is a decimal.
        assert_eq!(distance(&nb("1,3 km")), Some((1300, Units::Metric)));
        // GROUPING, from 10 000 up — three digits after it is not.
        assert_eq!(distance(&nb("12,500 mi")), Some((20_116_800, Units::Imperial)));
        assert_eq!(distance(&nb("12.500 km")), Some((12_500_000, Units::Metric)));
        // THE NATIVE SIDE'S NARROW SPACE, and a plain one.
        assert_eq!(distance("40\u{202F}m"), Some((40, Units::Metric)));
        assert_eq!(distance("40 m"), Some((40, Units::Metric)));
        // TRANSLATIONS WITH THEIR OWN WORDS — the ones the bug check found
        // blanking the turn, and the Cyrillic pair that worked before.
        assert_eq!(distance(&nb("500 фут")), Some((152, Units::Imperial)));
        assert_eq!(distance(&nb("2,5 км")), Some((2500, Units::Metric)));
        assert_eq!(distance(&nb("350 pie")), Some((107, Units::Imperial)), "Spanish feet");
        assert_eq!(distance(&nb("350 fod")), Some((107, Units::Imperial)), "Danish feet");
        assert_eq!(distance(&nb("1,2 Km")), Some((1200, Units::Metric)), "Portuguese km");
        assert_eq!(distance(&nb("120 μ")), Some((120, Units::Metric)), "Greek metres");
        assert_eq!(distance(&nb("1,2 χλμ")), Some((1200, Units::Metric)), "Greek km");
        assert_eq!(distance(&nb("120 米")), Some((120, Units::Metric)), "Chinese metres");
        assert_eq!(distance(&nb("0.3 英里")), Some((483, Units::Imperial)), "Chinese miles");
        // A SUFFIX THAT IS ANOTHER WORD'S TAIL: Thai metres `ม.` end Thai km
        // `กม.`, Greek `μ` ends `χλμ`, and `m` ends `km`. Each must read as
        // its own unit, not the shorter one.
        assert_eq!(distance(&nb("120 ม.")), Some((120, Units::Metric)));
        assert_eq!(distance(&nb("1.2 กม.")), Some((1200, Units::Metric)));
        // THE TWO-WORD UNIT, Galician miles — the reason the split is by suffix.
        // As CoMaps writes it (U+00A0 before the word, a plain space inside
        // it), and with every gap non-breaking.
        assert_eq!(distance("0,3\u{00A0}o meu"), Some((483, Units::Imperial)));
        assert_eq!(distance(&nb("0,3 o meu")), Some((483, Units::Imperial)));
        // CASE FORGIVEN ON THE LAST WORD, for a translation that changes it later.
        assert_eq!(distance(&nb("4.2 MI")), Some((6759, Units::Imperial)));
        // NOT A DISTANCE: refused rather than guessed.
        for bad in ["", "ft", "350", "350 yd", "-5 m", "about 3 km", "3. km", "350km"] {
            assert_eq!(distance(&nb(bad)), None, "{bad:?}");
        }
    }

    /// EVERY WORD IN THE TABLE READS AS ITS OWN UNIT, and none names two.
    ///
    /// The table is generated from CoMaps' translations (see [`WORDS`]); this is
    /// what holds a hand edit to it honest.
    #[test]
    fn every_comaps_unit_word_reads_and_no_word_names_two_units() {
        assert_eq!(WORDS.len(), 75, "the table as generated from CoMaps bf5945db");
        for &(w, word) in WORDS {
            let (per, units) = word.scale();
            let expect = (7.0 * per).round() as i32;
            assert_eq!(distance(&format!("7\u{00A0}{w}")), Some((expect, units)), "{w:?}");
        }
        let mut seen = std::collections::BTreeMap::new();
        for &(w, word) in WORDS {
            if let Some(was) = seen.insert(w, word) {
                panic!("{w:?} is listed twice, as {was:?} and {word:?}");
            }
        }
    }

    /// COMAPS' OWN THRESHOLDS, at the speeds that pin each clamp.
    #[test]
    fn the_stage_ladder_follows_when_comaps_itself_speaks() {
        let m = Units::Metric;
        // STOPPED, or no speed at all: the floors, 35 m and 200 + 35 m.
        assert_eq!(imminent(30, None, m), 0);
        assert_eq!(imminent(35, Some(0.0), m), 0);
        assert_eq!(imminent(36, None, m), 1);
        assert_eq!(imminent(235, None, m), 1);
        assert_eq!(imminent(236, None, m), -1);
        // 15 m/s (54 km/h): say-now 120 m, first warning 450 + 120 = 570 m.
        assert_eq!(imminent(120, Some(15.0), m), 0);
        assert_eq!(imminent(121, Some(15.0), m), 1);
        assert_eq!(imminent(570, Some(15.0), m), 1);
        assert_eq!(imminent(571, Some(15.0), m), -1);
        // 35 m/s (126 km/h): say-now at its 150 m ceiling, first warning
        // 1050 + 150 = 1200 m.
        assert_eq!(imminent(150, Some(35.0), m), 0);
        assert_eq!(imminent(151, Some(35.0), m), 1);
        assert_eq!(imminent(1200, Some(35.0), m), 1);
        assert_eq!(imminent(1201, Some(35.0), m), -1);
        // 70 m/s: the first warning's 2000 m ceiling too, so 2150 m.
        assert_eq!(imminent(2150, Some(70.0), m), 1);
        assert_eq!(imminent(2151, Some(70.0), m), -1);
        // IMPERIAL MOVES THE FIRST WARNING'S FLOOR to 500 ft (152 m) + 35 m.
        assert_eq!(imminent(187, None, Units::Imperial), 1);
        assert_eq!(imminent(188, None, Units::Imperial), -1);
        // A NONSENSE SPEED IS NO SPEED.
        assert_eq!(imminent(236, Some(f32::NAN), m), -1);
        assert_eq!(imminent(236, Some(-3.0), m), -1);
    }

    /// A ROW MID-ROUTE FILLS EVERY FIELD THE PROVIDER CAN FILL.
    #[test]
    fn a_row_on_route_reads_into_the_route_the_face_already_draws() {
        let r = read(&row(&on_route()), 1_700_000_000_000, Some(15.0));
        assert_eq!(r.units, Some(Units::Imperial), "the unit word is the driver's choice");
        assert_eq!(r.unread, None);
        let route = r.route;
        assert!(route.navigating());
        assert_eq!(route.turn_xml.as_deref(), Some("TR"));
        assert_eq!(route.turn_metres, Some(107));
        assert_eq!(route.street.as_deref(), Some("Whitney Way"), "the street you turn ONTO");
        assert_eq!(route.left_metres, Some(6759));
        assert_eq!(route.left_seconds, Some(540));
        assert_eq!(route.arrival_ms, Some(1_700_000_540_000));
        // 107 m at 15 m/s is inside the 120 m say-now distance.
        assert_eq!(route.imminent, Some(0));
        // WHAT COMAPS DOES NOT SEND, collapsed rather than filled in.
        assert!(!route.map_visible);
        assert_eq!(route.after_street, None);
        assert_eq!(route.after_turn_xml, None);
    }

    /// THE EXIT NUMBER RIDES INTO THE RING, through the same string OsmAnd's does.
    #[test]
    fn a_roundabout_carries_its_exit_in_the_turn_string() {
        let mut cells = on_route();
        cells[1] = ("car_direction", "ENTER_ROUND_ABOUT");
        cells[11] = ("exit_num", "3");
        let route = read(&row(&cells), 0, None).route;
        assert_eq!(route.turn_xml.as_deref(), Some("RNDB3"));
        assert_eq!(Turn::from_xml("RNDB3"), Some((Turn::Roundabout, Some(3))));
    }

    /// OFF THE ROUTE IS QUIET, AND RECALCULATING IS THE SAME QUIET.
    #[test]
    fn off_route_and_rebuilding_read_as_off_route_with_no_turn_left_over() {
        for state in ["OffRoute", "RouteNeedsRebuild", "RouteRebuilding"] {
            let mut cells = on_route();
            cells[0] = ("session_state", state);
            let route = read(&row(&cells), 0, None).route;
            assert_eq!(route.turn_xml.as_deref(), Some("OFFR"), "{state}");
            assert_eq!(route.turn_metres, None, "{state}: no deviation is given");
            assert_eq!(route.imminent, None, "{state}: nothing to escalate towards");
            let mut nav = crate::nav::Nav::new();
            nav.poll(route, 100);
            assert_eq!(nav.state(100), crate::nav::NavState::OffRoute { metres: -1 });
            assert_eq!(nav.stage(100, true), crate::nav::Stage::Idle);
        }
    }

    /// EVERY STATE THAT IS NOT A ROUTE BEING DRIVEN IS NO ROUTE.
    #[test]
    fn planning_finished_and_unknown_states_are_not_a_route() {
        for state in [
            "NoValidRoute",
            "RouteBuilding",
            "RouteNotStarted",
            "RouteFinished",
            "RouteNoFollowing",
            "SomethingNewer",
        ] {
            let mut cells = on_route();
            cells[0] = ("session_state", state);
            let r = read(&row(&cells), 0, None);
            assert_eq!(r.route, Route::default(), "{state}");
            assert!(!r.route.navigating(), "{state}");
        }
        assert!(!read(&Row::default(), 0, None).route.navigating(), "an empty cursor");
    }

    /// THE STALE CACHE: a cleared geometry outranks a state that still says
    /// `OnRoute`. See fact 2 in the module header.
    #[test]
    fn a_cleared_route_geometry_beats_the_stale_on_route_state() {
        let mut cells = on_route();
        cells.push(("route_point_count", "0"));
        assert!(!read(&row(&cells), 0, None).route.navigating());
        let mut live = on_route();
        live.push(("route_point_count", "812"));
        assert!(read(&row(&live), 0, None).route.navigating());
    }

    /// THE COLUMNS ARE READ BY NAME, so a newer build's extra columns, in any
    /// order, change nothing.
    #[test]
    fn a_newer_builds_extra_columns_in_another_order_read_the_same() {
        let mut cells = on_route();
        cells.reverse();
        cells.insert(3, ("dest_title", "Home"));
        cells.insert(0, ("route_revision", "7"));
        let a = read(&row(&on_route()), 5_000, None);
        let b = read(&row(&cells), 5_000, None);
        assert_eq!(a, b);
    }

    /// A DISTANCE THAT DOES NOT READ IS REPORTED, and the turn waits for one.
    #[test]
    fn an_unreadable_distance_is_named_for_the_log() {
        let mut cells = on_route();
        cells[3] = ("dist_to_turn", "350 yd");
        let r = read(&row(&cells), 0, None);
        assert_eq!(r.unread.as_deref(), Some("dist_to_turn = \"350 yd\""));
        assert_eq!(r.route.turn_metres, None);
        assert_eq!(r.route.imminent, None);
        // The destination's distance still read, so the units did too.
        assert_eq!(r.units, Some(Units::Imperial));
    }

    /// A NULL CELL IS AN ABSENT ONE, the way an empty string is.
    #[test]
    fn a_null_street_collapses_rather_than_reading_as_text() {
        let names = vec!["session_state".to_string(), "next_street".to_string()];
        let values = vec![Some("OnRoute".to_string()), None];
        let r = read(&Row::new(names, values), 0, None);
        assert_eq!(r.route.street, None);
    }

    fn status(link: Link) -> Status<'static> {
        Status { package: "app.comaps.fdroid", on: true, link, ..Status::default() }
    }

    /// EACH REASON FOR A BLANK STRIP SAYS WHICH ONE IT IS.
    #[test]
    fn each_comaps_reason_for_a_blank_strip_is_its_own_sentence() {
        let lines = [
            sub_line(&Status::default()),
            sub_line(&Status { outdated: "app.comaps.google", ..Status::default() }),
            sub_line(&Status { on: false, ..status(Link::Off) }),
            sub_line(&status(Link::NeedsPermission)),
            sub_line(&status(Link::Failed)),
            sub_line(&status(Link::Off)),
            sub_line(&status(Link::Watching)),
            sub_line(&Status { navigating: true, ..status(Link::Watching) }),
        ];
        let distinct: std::collections::BTreeSet<_> = lines.iter().collect();
        assert_eq!(distinct.len(), lines.len(), "{lines:#?}");
        assert!(lines[0].ends_with("CoMaps is not installed."));
        assert!(lines[1].contains("app.comaps.google") && lines[1].contains("2026.07.16"));
        assert!(lines[2].ends_with("Off. Found app.comaps.fdroid."));
        assert!(lines[3].contains("tap CoMaps below"));
        assert!(lines[7].ends_with("Showing the next turn from app.comaps.fdroid."));
        // NONE OF THEM READS AS A FAULT — the OsmAnd row's rule.
        for line in &lines {
            for word in ["error", "Error", "failed", "Failed", "fault"] {
                assert!(!line.contains(word), "{line:?} reads as a fault");
            }
        }
    }

    /// A LIT LINK IS NEVER CALLED "NOT INSTALLED" — #134's bug check, where the
    /// record read at launch said one thing and the watcher, which looks the
    /// package up itself, said the other.
    #[test]
    fn a_live_link_outranks_a_stale_not_installed_record() {
        let stale = |link| Status { on: true, link, ..Status::default() };
        let showing = sub_line(&Status { navigating: true, ..stale(Link::Watching) });
        assert_eq!(showing, "Show CoMaps' next turn on the face. Showing the next turn from CoMaps.");
        let waiting = sub_line(&stale(Link::NeedsPermission));
        assert!(waiting.contains("Waiting for permission"), "{waiting}");
        // A link that is NOT up still believes the record, and with the switch
        // on the sentence now says how to recover.
        assert_eq!(
            sub_line(&stale(Link::Off)),
            "Show CoMaps' next turn on the face. CoMaps is not installed. Once it is, tap CoMaps below."
        );
        let old = sub_line(&Status { outdated: "app.comaps.fdroid", ..stale(Link::Off) });
        assert!(old.ends_with("After updating, tap CoMaps below."), "{old}");
        // SWITCHED OFF, a stale lit link counts for nothing: the record speaks.
        assert!(sub_line(&Status { on: false, ..stale(Link::Watching) }).ends_with("CoMaps is not installed."));
    }

    #[test]
    fn the_link_codes_are_carnyx_comaps_own() {
        assert_eq!(Link::from_code(0), Link::Off);
        assert_eq!(Link::from_code(1), Link::NeedsPermission);
        assert_eq!(Link::from_code(2), Link::Watching);
        assert_eq!(Link::from_code(3), Link::Failed);
        assert_eq!(Link::from_code(99), Link::Off, "unknown is the quiet reading");
    }
}
