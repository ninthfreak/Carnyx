//! Turn-by-turn navigation, decided here rather than at the seam.
//!
//! ## What arrives, and how little of it there is
//!
//! OsmAnd's AIDL API hands an outside app THREE INTEGERS per update —
//! `distanceTo`, `turnType`, `isLeftSide` — and separately, on a different
//! callback, the voice router's announcement as a list of strings. There is no
//! street name in that structured half, no exit number, no ETA, no distance to
//! the destination: all of those live in the POLLED `getAppInfo` and only if
//! something asks. Everything this module does is make those streams into one
//! thing a face can draw.
//!
//! ## Why all of it is on this side of the wire
//!
//! `CarnyxNav.java` reads a bundle and calls a native method. It decides
//! nothing: not what turn type 13 means, not whether `-1` is a turn, not which
//! of the voice router's two lists is the instruction, not when a route has gone
//! stale. That is the same rule `CarnyxLocation` states for the motion verdict
//! and for the same reason — a decision made in Java cannot be tested on a
//! machine with no head unit, and every decision here is tested below.
//!
//! ## What is NOT here
//!
//! Presentation. This module publishes a [`Nav`] and stops: a turn, a distance
//! in metres, and the words. The SHAPE of a maneuver arrow is
//! [`crate::arrow`]'s, and where any of it sits on the face is §4.9's.

/// Which way the next turn goes.
///
/// THE INTEGERS ARE OSMAND'S, read out of `OsmAnd-java/.../router/TurnType.java`
/// where each is a `public static final int`.
///
/// THE TWO CHANNELS CARRY DIFFERENT AMOUNTS OF IT, which is worth stating here
/// because the difference is a roundabout's exit number:
///
/// * The PUSH callback crosses the boundary bare — `OsmandAidlApi` sets
///   `directionInfo.setTurnType(ndi.directionInfo.getTurnType().getValue())`
///   with nothing packed alongside, so a roundabout says 13 and no more.
/// * The POLL carries `TurnType.toXmlString()`, and that method ends
///   `case RNDB: return "RNDB" + exitOut;` — so the exit number DOES travel,
///   inside the string. [`Turn::from_xml`] is where it is read out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// `C = 1` — carry on.
    Straight,
    /// `TL = 2`, `TSLL = 3`, `TSHL = 4`.
    Left,
    SlightLeft,
    SharpLeft,
    /// `TR = 5`, `TSLR = 6`, `TSHR = 7`.
    Right,
    SlightRight,
    SharpRight,
    /// `KL = 8`, `KR = 9` — a fork rather than a turn.
    KeepLeft,
    KeepRight,
    /// `TU = 10`, `TRU = 11`.
    UTurn,
    RightUTurn,
    /// `RNDB = 13`, `RNLB = 14`. Which exit rides in the POLL's string only —
    /// see the type note and [`Turn::from_xml`].
    Roundabout,
    RoundaboutLeft,
}

impl Turn {
    /// `OFFR = 12` is deliberately absent from [`Turn`] — see [`Nav::state`].
    const OFF_ROUTE: i32 = 12;

    /// The OsmAnd constant, or `None` for anything this build does not know.
    ///
    /// UNKNOWN IS A REAL ANSWER AND NOT A DEFAULT. An integer this table does
    /// not have is a turn type added to OsmAnd after this was written, and
    /// drawing it as "straight ahead" would be an instruction — the wrong one,
    /// confidently. It becomes [`NavState::Unknown`] instead, which the face can
    /// show as a distance with no glyph.
    pub fn from_osmand(value: i32) -> Option<Turn> {
        Some(match value {
            1 => Turn::Straight,
            2 => Turn::Left,
            3 => Turn::SlightLeft,
            4 => Turn::SharpLeft,
            5 => Turn::Right,
            6 => Turn::SlightRight,
            7 => Turn::SharpRight,
            8 => Turn::KeepLeft,
            9 => Turn::KeepRight,
            10 => Turn::UTurn,
            11 => Turn::RightUTurn,
            13 => Turn::Roundabout,
            14 => Turn::RoundaboutLeft,
            _ => return None,
        })
    }

    /// The turn, and a roundabout's exit number, out of a TurnType XML string.
    ///
    /// THIS IS THE POLL'S ENCODING AND NOT THE PUSH'S — see the type note. The
    /// strings are `TurnType.toXmlString()`'s own output, and the roundabouts
    /// are the only ones with anything appended.
    ///
    /// UNRECOGNISED IS `None`, WHICH IS WHERE THIS DELIBERATELY DIFFERS FROM
    /// OSMAND. Its own `TurnType.fromString` ends `if (t == null) { t =
    /// TurnType.straight(); }` — a sensible default for a router that is about
    /// to recompute, and a wrong instruction given confidently for a face that
    /// is about to draw an arrow. Same rule as [`Turn::from_osmand`].
    pub fn from_xml(s: &str) -> Option<(Turn, Option<u32>)> {
        let plain = |t: Turn| Some((t, None));
        match s {
            "C" => plain(Turn::Straight),
            "TL" => plain(Turn::Left),
            "TSLL" => plain(Turn::SlightLeft),
            "TSHL" => plain(Turn::SharpLeft),
            "TR" => plain(Turn::Right),
            "TSLR" => plain(Turn::SlightRight),
            "TSHR" => plain(Turn::SharpRight),
            "KL" => plain(Turn::KeepLeft),
            "KR" => plain(Turn::KeepRight),
            "TU" => plain(Turn::UTurn),
            "TRU" => plain(Turn::RightUTurn),
            // `OFFR` IS NOT A TURN, exactly as `from_osmand(12)` is not. Off-route
            // arrives on the push channel where there is a state for it; nothing
            // on the face draws an arrow for it.
            _ => {
                // `RNDB3`, `RNLB1`. OsmAnd's own parser also accepts `EXIT3`
                // here — `s.startsWith("EXIT") || s.startsWith("RNDB") ||
                // s.startsWith("RNLB")`, all four characters long — even though
                // `toXmlString` never emits it. Accepted for the same reason:
                // the wire is whatever OsmAnd will parse back.
                let turn = match s.get(..4)? {
                    "RNDB" | "EXIT" => Turn::Roundabout,
                    "RNLB" => Turn::RoundaboutLeft,
                    _ => return None,
                };
                let rest = &s[4..];
                if rest.is_empty() {
                    return Some((turn, None));
                }
                // THERE IS NO EXIT ZERO. `exitOut` is a plain `int` field that
                // stays 0 unless `getExitTurn` set it, and `toXmlString`
                // concatenates it either way — so `RNDB0` is a roundabout whose
                // exit OsmAnd did not name, not the zeroth exit.
                let exit = rest.parse::<u32>().ok()?;
                Some((turn, (exit > 0).then_some(exit)))
            }
        }
    }

    /// A short name, for the diagnostics log and for a face with no glyph yet.
    ///
    /// NOT USER COPY. The design handoff will bring the words and the marks it
    /// wants; these exist so a drive log reads `nav: Right in 240 m` rather than
    /// `nav: 5 in 240`, which is the difference between a line that settles a
    /// question and one that needs this file open beside it.
    pub fn name(self) -> &'static str {
        match self {
            Turn::Straight => "straight on",
            Turn::Left => "left",
            Turn::SlightLeft => "slight left",
            Turn::SharpLeft => "sharp left",
            Turn::Right => "right",
            Turn::SlightRight => "slight right",
            Turn::SharpRight => "sharp right",
            Turn::KeepLeft => "keep left",
            Turn::KeepRight => "keep right",
            Turn::UTurn => "U-turn",
            Turn::RightUTurn => "U-turn right",
            Turn::Roundabout => "roundabout",
            Turn::RoundaboutLeft => "roundabout",
        }
    }
}

/// What the face should be told, resolved from one update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavState {
    /// OsmAnd is not sending, or what it sent has gone stale. See [`Nav::EXPIRY`].
    Idle,
    /// Navigating, with nothing to say yet — OsmAnd's own `(-1, -1)`, which its
    /// sender builds the update with and only overwrites when there IS a next
    /// direction. Not an error and not the end of a route.
    Waiting,
    /// The driver has left the route. `metres` is the DEVIATION from it, which is
    /// a different quantity from every other distance here — OsmAnd overloads the
    /// one field — so it is a separate variant rather than a flag beside a turn.
    OffRoute { metres: i32 },
    /// A turn, and how far to it.
    Turn { turn: Turn, metres: i32 },
    /// A turn type this build does not know. The distance is still good.
    Unknown { code: i32, metres: i32 },
}

/// What the POLL adds — everything with words in it.
///
/// THE PUSH CALLBACK HAS NO TEXT AT ALL. `ADirectionInfo` is three integers, so
/// the street name, the turn after next, the ETA and the distance left exist
/// only in `getAppInfo`'s answer and only if something asks for it. The design
/// handoff states the split in as many words: *"Poll and push are not
/// interchangeable. A street name from the push callback does not exist; if the
/// poll has not landed yet, the street collapses."*
///
/// EVERY FIELD IS OPTIONAL AND A MISSING ONE COLLAPSES ITS ELEMENT. That is the
/// handoff's rule and it is why these are `Option` rather than empty strings
/// with a convention: "no street on this route" and "the poll has not answered
/// yet" are the same to a face and neither is a gap to fill with a placeholder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    /// Unix millis at the destination. `None` when not navigating.
    pub arrival_ms: Option<i64>,
    /// Seconds to the destination.
    pub left_seconds: Option<i32>,
    /// Metres to the destination — NOT to the next turn.
    pub left_metres: Option<i32>,
    /// OsmAnd's own map is in front. The handoff hides the whole layer here:
    /// the driver is already looking at the turn.
    pub map_visible: bool,
    /// The next turn's street, already formatted by OsmAnd with its ref and
    /// destination — `RoutingHelperUtils.formatStreetName(street, ref, dest, "")`.
    pub street: Option<String>,
    /// The next turn as a TurnType XML STRING — "TR", "TSLL", "C", "RNDB"…
    ///
    /// NOT THE INTEGER THE PUSH SENDS. The two channels describe the same turn in
    /// two encodings, which is a trap worth naming: `ADirectionInfo.turnType` is
    /// `TurnType.getValue()` and this is `TurnType.toXmlString()`.
    pub turn_xml: Option<String>,
    /// Metres to the next turn, from the poll's own reading.
    pub turn_metres: Option<i32>,
    /// OsmAnd's `nextInfo.imminent`.
    ///
    /// AN INTEGER WHOSE SCALE IS NOT ESTABLISHED. The handoff escalates the
    /// display on it — *"Escalate on `next_turn_imminent` … never on distance
    /// thresholds of our own"* — and the class that computes it,
    /// `AnnounceTimeDistances.getImminentTurnStatus`, is not in OsmAnd's Java
    /// sources any more, so what its values mean could not be read. It is carried
    /// raw and LOGGED raw; one drive with a route running settles it, and until
    /// then nothing branches on it.
    pub imminent: Option<i32>,
    /// The turn after the next one, for the handoff's `THEN` block.
    pub after_street: Option<String>,
    pub after_turn_xml: Option<String>,
}

impl Route {
    /// Is there a route at all?
    ///
    /// THE DESTINATION IS THE TEST, not the turn: OsmAnd answers `getAppInfo`
    /// whether or not it is navigating and zeroes the route fields when it is
    /// not, so a zero arrival AND a zero distance is "no route" rather than a
    /// route that has arrived.
    pub fn navigating(&self) -> bool {
        self.arrival_ms.is_some() || self.left_metres.is_some() || self.turn_xml.is_some()
    }
}

/// The navigation state the face draws, and the clock that ages it.
#[derive(Clone, Debug, Default)]
pub struct Nav {
    /// The last update's three integers, and when they landed.
    last: Option<(i32, i32, u64)>,
    /// The last spoken instruction, and when it was said.
    spoken: Option<(String, u64)>,
    /// The last poll's answer, and when it landed. Aged on the same clock as the
    /// push: a poll that has stopped answering is a route that has ended.
    route: Option<(Route, u64)>,
}

impl Nav {
    /// How long an update stays good, in seconds.
    ///
    /// AN EXPIRY IS NOT OPTIONAL HERE, and this is the one thing about the API
    /// that a caller must not get wrong. OsmAnd sends while it is navigating and
    /// simply STOPS when the route ends, is cancelled, or the app is closed —
    /// there is no "navigation over" message on this callback. Without a clock,
    /// the last turn before the driver arrived would sit on the face for the rest
    /// of the drive, pointing at a junction miles behind them.
    ///
    /// TWELVE SECONDS, from the sender's own cadence: the updates come off
    /// `IRoutingDataUpdateListener`, which fires on routing recalculation — about
    /// one per location fix. Twelve is long enough to ride out a tunnel or a
    /// dropped fix and short enough that a finished route clears before the
    /// driver has parked.
    pub const EXPIRY: u64 = 12;

    /// How long a spoken instruction stays on screen, in seconds.
    ///
    /// SHORTER THAN THE TURN, on purpose. The turn and its distance stay true
    /// until the turn is taken; the words are an ANNOUNCEMENT and go stale as
    /// soon as the next one is due. Keeping them as long would leave "in four
    /// hundred metres, turn right" under a distance reading 30 m.
    pub const SPOKEN_EXPIRY: u64 = 20;

    pub fn new() -> Nav {
        Nav::default()
    }

    /// One update from OsmAnd. `now` is a monotonic second count, passed in
    /// rather than read, so the tests below can drive the clock.
    pub fn update(&mut self, distance_to: i32, turn_type: i32, now: u64) {
        self.last = Some((distance_to, turn_type, now));
    }

    /// One voice-router announcement.
    ///
    /// `played` FIRST, `cmds` SECOND, and the order is the decision. `cmds` is
    /// what the router queued; `played` is what the engine actually said, and
    /// when they differ it is because the engine dropped or merged something —
    /// so `played` is the truer record of what the driver just heard. Falling
    /// back to `cmds` covers the case where the driver has muted the voice, when
    /// `played` arrives empty and the instruction is still worth showing.
    pub fn speak(&mut self, cmds: &[String], played: &[String], now: u64) {
        let pick = |parts: &[String]| -> Option<String> {
            let joined = parts
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            (!joined.is_empty()).then_some(joined)
        };
        if let Some(text) = pick(played).or_else(|| pick(cmds)) {
            self.spoken = Some((text, now));
        }
    }

    /// One poll answer from `getAppInfo`.
    pub fn poll(&mut self, route: Route, now: u64) {
        self.route = Some((route, now));
    }

    /// The last poll, if it is still fresh. See [`Nav::EXPIRY`].
    pub fn route(&self, now: u64) -> Option<&Route> {
        self.route
            .as_ref()
            .filter(|(_, at)| now.saturating_sub(*at) < Self::EXPIRY)
            .map(|(r, _)| r)
    }

    /// Forget everything. The switch went off, or OsmAnd went away.
    pub fn clear(&mut self) {
        self.last = None;
        self.spoken = None;
        self.route = None;
    }

    /// What to draw, at `now`.
    pub fn state(&self, now: u64) -> NavState {
        let Some((metres, code, at)) = self.last else {
            return NavState::Idle;
        };
        if now.saturating_sub(at) >= Self::EXPIRY {
            return NavState::Idle;
        }
        // OFF-ROUTE IS CHECKED BEFORE THE `-1` PAIR, because a deviation is a
        // real reading and the sender writes it into the same two fields.
        if code == Turn::OFF_ROUTE {
            return NavState::OffRoute { metres: metres.max(0) };
        }
        // THE SENTINEL IS EITHER FIELD, not both. OsmAnd builds the update as
        // `(-1, -1)` and overwrites the pair together, so in practice they move
        // as one — but a distance of -1 with a real turn type is not a distance,
        // and testing both is what makes that impossible to draw.
        if metres < 0 || code < 0 {
            return NavState::Waiting;
        }
        match Turn::from_osmand(code) {
            Some(turn) => NavState::Turn { turn, metres },
            None => NavState::Unknown { code, metres },
        }
    }

    /// The last announcement, if it is still fresh.
    pub fn spoken(&self, now: u64) -> Option<&str> {
        self.spoken
            .as_ref()
            .filter(|(_, at)| now.saturating_sub(*at) < Self::SPOKEN_EXPIRY)
            .map(|(text, _)| text.as_str())
    }

    /// `240 m` / `1.4 km`, the reference's own break.
    ///
    /// METRIC ONLY, and that is a gap rather than a choice: OsmAnd sends metres
    /// and honours its own units setting for the SPOKEN text, so a driver with
    /// OsmAnd set to miles will hear miles and read metres here until the design
    /// handoff says what this should do. Recorded so it is a known difference and
    /// not a surprise.
    ///
    /// The break is at a kilometre and the tenth is dropped past ten, which is
    /// how every turn-by-turn display reads: `950 m`, `1.4 km`, `12 km`.
    pub fn distance_label(metres: i32) -> String {
        let m = metres.max(0);
        if m < 1000 {
            return format!("{m} m");
        }
        let km = f64::from(m) / 1000.0;
        if km < 10.0 {
            format!("{km:.1} km")
        } else {
            format!("{:.0} km", km.round())
        }
    }

    /// One line for the diagnostics log, or `None` when there is nothing to say.
    ///
    /// The unit has no adb, so this is the only way a drive can report what the
    /// integration actually received.
    pub fn log_line(&self, now: u64) -> Option<String> {
        let head = match self.state(now) {
            NavState::Idle => return None,
            NavState::Waiting => "navigating, no turn yet".to_string(),
            NavState::OffRoute { metres } => {
                format!("OFF ROUTE by {}", Self::distance_label(metres))
            }
            NavState::Turn { turn, metres } => {
                format!("{} in {}", turn.name(), Self::distance_label(metres))
            }
            NavState::Unknown { code, metres } => {
                format!("turn type {code} (unknown to this build) in {}", Self::distance_label(metres))
            }
        };
        let mut out = head;
        if let Some(r) = self.route(now) {
            // THE STREET AND THE RAW `imminent`, which is the whole reason this
            // line exists in this shape: the handoff escalates on that integer
            // and nothing in this tree could establish its scale, so one drive
            // with a route running is what settles it.
            if let Some(street) = &r.street {
                out.push_str(&format!(" onto {street}"));
            }
            if let Some(i) = r.imminent {
                out.push_str(&format!(" [imminent={i}]"));
            }
            if r.map_visible {
                out.push_str(" [OsmAnd map in front]");
            }
        }
        if let Some(words) = self.spoken(now) {
            out.push_str(&format!(" — \"{words}\""));
        }
        Some(out)
    }
}

/// The settings row's sub-line.
///
/// THE ROW HAS TO ANSWER "is OsmAnd even here" WITHOUT BEING TURNED ON. A switch
/// whose only failure report arrives after you flip it makes the driver run an
/// experiment to learn something the package manager already knows — and on a
/// dashboard that is the difference between a feature and a puzzle.
///
/// `package` is whatever `CarnyxNav.installedPackage` found, or empty.
pub fn sub_line(package: &str) -> String {
    if package.is_empty() {
        // NOT WORDED AS A FAULT. A driver without OsmAnd has not broken
        // anything; they have a switch for an app they do not have.
        return "Show OsmAnd's next turn on the face. OsmAnd is not installed.".to_string();
    }
    format!("Show OsmAnd's next turn on the face. Found {package}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EVERY CONSTANT OSMAND DEFINES, against the file it defines them in.
    ///
    /// Transcribed from `OsmAnd-java/src/main/java/net/osmand/router/TurnType.java`,
    /// which declares them as `public static final int C = 1` and so on through
    /// `RNLB = 14`. A table is exactly the kind of thing that is copied with one
    /// value off by one, and the failure would be a driver told to turn left at a
    /// right turn.
    #[test]
    fn the_turn_table_is_osmands_own() {
        let expected = [
            (1, Turn::Straight),
            (2, Turn::Left),
            (3, Turn::SlightLeft),
            (4, Turn::SharpLeft),
            (5, Turn::Right),
            (6, Turn::SlightRight),
            (7, Turn::SharpRight),
            (8, Turn::KeepLeft),
            (9, Turn::KeepRight),
            (10, Turn::UTurn),
            (11, Turn::RightUTurn),
            (13, Turn::Roundabout),
            (14, Turn::RoundaboutLeft),
        ];
        for (code, turn) in expected {
            assert_eq!(Turn::from_osmand(code), Some(turn), "code {code}");
        }
        // 12 IS OFF-ROUTE AND IS NOT A TURN. It shares the field with the turn
        // types and means the driver has left the route, so a `Turn` for it would
        // put a glyph on the face for a manoeuvre nobody has to make.
        assert_eq!(Turn::from_osmand(12), None, "OFFR must not be a Turn");
        // And anything past the table is unknown rather than a guess.
        for code in [0, 15, 99, -3] {
            assert_eq!(Turn::from_osmand(code), None, "code {code}");
        }
    }

    /// THE POLL'S ENCODING IS A STRING, AND THE ROUNDABOUTS CARRY A NUMBER IN IT.
    ///
    /// Transcribed against `TurnType.toXmlString()`, which is a switch returning
    /// the bare name for every type but the two roundabouts, where it returns
    /// `"RNDB" + exitOut`. Every name here appears in that switch.
    #[test]
    fn the_xml_names_are_osmands_own_and_the_exit_rides_in_them() {
        let expected = [
            ("C", Turn::Straight),
            ("TL", Turn::Left),
            ("TSLL", Turn::SlightLeft),
            ("TSHL", Turn::SharpLeft),
            ("TR", Turn::Right),
            ("TSLR", Turn::SlightRight),
            ("TSHR", Turn::SharpRight),
            ("KL", Turn::KeepLeft),
            ("KR", Turn::KeepRight),
            ("TU", Turn::UTurn),
            ("TRU", Turn::RightUTurn),
        ];
        for (xml, turn) in expected {
            assert_eq!(Turn::from_xml(xml), Some((turn, None)), "{xml}");
        }
        // THE EXIT NUMBER, which is the whole reason this parser exists rather
        // than a name match.
        assert_eq!(Turn::from_xml("RNDB3"), Some((Turn::Roundabout, Some(3))));
        assert_eq!(Turn::from_xml("RNLB1"), Some((Turn::RoundaboutLeft, Some(1))));
        assert_eq!(Turn::from_xml("RNDB12"), Some((Turn::Roundabout, Some(12))));
        // `EXIT3` is accepted because OsmAnd's own `fromString` accepts it.
        assert_eq!(Turn::from_xml("EXIT3"), Some((Turn::Roundabout, Some(3))));

        // AND THERE IS NO EXIT ZERO: `exitOut` defaults to 0 and is concatenated
        // unconditionally, so `RNDB0` is a roundabout with no exit named.
        assert_eq!(Turn::from_xml("RNDB0"), Some((Turn::Roundabout, None)));
        assert_eq!(Turn::from_xml("RNDB"), Some((Turn::Roundabout, None)));

        // OFF-ROUTE IS NOT A TURN, the same as `from_osmand(12)`.
        assert_eq!(Turn::from_xml("OFFR"), None);

        // AND AN UNKNOWN NAME IS `None` RATHER THAN "STRAIGHT ON", which is
        // where this differs from OsmAnd's own parser on purpose.
        for junk in ["", "X", "TLL", "RNDBx", "RNDB-1", "rndb3", "STRAIGHT"] {
            assert_eq!(Turn::from_xml(junk), None, "{junk:?} must not become a turn");
        }
    }

    /// A ROUTE THAT ENDS JUST STOPS SENDING, WHICH IS WHY THERE IS A CLOCK.
    ///
    /// The API has no "navigation finished" message on this callback. Without an
    /// expiry the last turn before the driver arrived would stay on the face for
    /// the rest of the drive, pointing at a junction behind them.
    #[test]
    fn an_update_ages_out_and_takes_the_turn_with_it() {
        let mut nav = Nav::new();
        nav.update(240, 5, 100);
        assert_eq!(nav.state(100), NavState::Turn { turn: Turn::Right, metres: 240 });
        // Still good a second before the expiry.
        assert_eq!(
            nav.state(100 + Nav::EXPIRY - 1),
            NavState::Turn { turn: Turn::Right, metres: 240 }
        );
        // And gone on it.
        assert_eq!(nav.state(100 + Nav::EXPIRY), NavState::Idle);
        // A fresh update revives it — this is a clock, not a latch.
        nav.update(90, 2, 200);
        assert_eq!(nav.state(200), NavState::Turn { turn: Turn::Left, metres: 90 });
    }

    /// THE THREE THINGS THAT ARE NOT A TURN, and they are three different states.
    #[test]
    fn the_sentinel_the_deviation_and_the_unknown_are_told_apart() {
        let mut nav = Nav::new();

        // OsmAnd's own `(-1, -1)`: navigating, nothing to say. NOT an error, and
        // not something to draw a turn for.
        nav.update(-1, -1, 10);
        assert_eq!(nav.state(10), NavState::Waiting);

        // Off route: the distance is a DEVIATION, which is why it is not a turn
        // with a distance beside it.
        nav.update(75, 12, 20);
        assert_eq!(nav.state(20), NavState::OffRoute { metres: 75 });

        // A turn type from a newer OsmAnd: the distance is still good and the
        // glyph is not. Drawing this as "straight on" would be a wrong
        // instruction given confidently.
        nav.update(300, 42, 30);
        assert_eq!(nav.state(30), NavState::Unknown { code: 42, metres: 300 });

        // A real turn type with a negative distance is still not a distance.
        nav.update(-1, 5, 40);
        assert_eq!(nav.state(40), NavState::Waiting);
    }

    /// THE WORDS ARE THE ONLY PLACE A STREET NAME CAN COME FROM, so which list
    /// is read matters.
    #[test]
    fn the_spoken_instruction_prefers_what_was_actually_said() {
        let mut nav = Nav::new();
        let cmds = ["Turn right".to_string(), "onto Main Street".to_string()];
        let played = ["Turn right onto Main Street".to_string()];

        nav.speak(&cmds, &played, 100);
        assert_eq!(nav.spoken(100), Some("Turn right onto Main Street"));

        // A MUTED DRIVER STILL GETS THE TEXT. `played` comes back empty when the
        // voice is off, and the queued command is then the whole record.
        nav.clear();
        nav.speak(&cmds, &[], 100);
        assert_eq!(nav.spoken(100), Some("Turn right onto Main Street"));

        // Empty on both sides changes nothing rather than blanking what stands.
        nav.speak(&[], &[], 101);
        assert_eq!(nav.spoken(101), Some("Turn right onto Main Street"));
        nav.speak(&["   ".to_string()], &[], 102);
        assert_eq!(nav.spoken(102), Some("Turn right onto Main Street"));

        // And the words age out FASTER than the turn, because they are an
        // announcement rather than a state.
        assert_eq!(nav.spoken(100 + Nav::SPOKEN_EXPIRY), None);
        const { assert!(Nav::SPOKEN_EXPIRY > Nav::EXPIRY) };
    }

    /// THE ROW SAYS WHETHER THERE IS ANYTHING TO TALK TO, BEFORE IT IS TAPPED.
    #[test]
    fn the_settings_sub_line_names_the_osmand_it_found() {
        assert!(sub_line("net.osmand.plus").contains("net.osmand.plus"));
        assert!(sub_line("").contains("not installed"));
        // AND IT NEVER READS AS A FAULT. "Error", "failed" and "unavailable" are
        // what a driver reads as "something is broken"; not having an app is not.
        for word in ["rror", "ailed", "navailable"] {
            assert!(!sub_line("").contains(word), "{word} reads as a fault");
        }
    }

    /// THE POLL CARRIES EVERY WORD, AND IT AGES ON THE SAME CLOCK.
    ///
    /// The push callback has no text at all, so a face that shows a street name
    /// is showing the poll's answer — and a poll that has stopped answering is a
    /// route that has ended, which must clear rather than freeze.
    #[test]
    fn the_poll_carries_the_words_and_expires_with_the_route() {
        let mut nav = Nav::new();
        assert!(nav.route(0).is_none(), "nothing polled yet");

        let route = Route {
            arrival_ms: Some(1_700_000_000_000),
            left_seconds: Some(840),
            left_metres: Some(9_400),
            map_visible: false,
            street: Some("Whitney Way".into()),
            turn_xml: Some("TR".into()),
            turn_metres: Some(240),
            imminent: Some(1),
            after_street: Some("Odana Rd".into()),
            after_turn_xml: Some("TSLL".into()),
        };
        nav.poll(route.clone(), 100);
        assert_eq!(nav.route(100).unwrap().street.as_deref(), Some("Whitney Way"));
        assert!(nav.route(100).unwrap().navigating());

        // THE SAME CLOCK AS THE PUSH. A poll that stops is a route that ended.
        assert!(nav.route(100 + Nav::EXPIRY - 1).is_some());
        assert!(nav.route(100 + Nav::EXPIRY).is_none(), "a stale poll is no route");

        // AN EMPTY ANSWER IS NOT A ROUTE. OsmAnd answers `getAppInfo` whether or
        // not it is navigating and zeroes the route fields when it is not, which
        // the seam turns into `None` — so this is what "idle" looks like here.
        nav.poll(Route::default(), 200);
        assert!(!nav.route(200).unwrap().navigating(), "zeros are not a route");

        // AND CLEARING TAKES IT, with everything else.
        nav.poll(route, 300);
        nav.clear();
        assert!(nav.route(300).is_none());
    }

    /// THE LOG CARRIES THE RAW `imminent`, WHICH IS THE POINT OF THE LINE.
    ///
    /// The handoff escalates the display on that integer and forbids distance
    /// thresholds of our own — and its scale could not be read from OsmAnd's
    /// sources, because the class that computes it has moved out of the Java
    /// tree. So nothing branches on it yet and one drive with a route running is
    /// what settles it. This test exists so that line cannot quietly be dropped.
    #[test]
    fn the_log_line_prints_the_unexplained_imminent_integer() {
        let mut nav = Nav::new();
        nav.update(240, 5, 100);
        nav.poll(
            Route {
                street: Some("Whitney Way".into()),
                turn_xml: Some("TR".into()),
                imminent: Some(2),
                map_visible: true,
                ..Route::default()
            },
            100,
        );
        let line = nav.log_line(100).unwrap();
        assert!(line.contains("right in 240 m"), "{line}");
        assert!(line.contains("onto Whitney Way"), "{line}");
        assert!(line.contains("[imminent=2]"), "the raw integer, so a drive settles it: {line}");
        assert!(line.contains("map in front"), "{line}");
    }

    #[test]
    fn the_distance_reads_the_way_a_turn_by_turn_display_does() {
        assert_eq!(Nav::distance_label(0), "0 m");
        assert_eq!(Nav::distance_label(240), "240 m");
        assert_eq!(Nav::distance_label(999), "999 m");
        assert_eq!(Nav::distance_label(1000), "1.0 km");
        assert_eq!(Nav::distance_label(1449), "1.4 km");
        assert_eq!(Nav::distance_label(9949), "9.9 km");
        assert_eq!(Nav::distance_label(10000), "10 km");
        assert_eq!(Nav::distance_label(12400), "12 km");
        // A negative never reaches here through `state`, which floors it — but
        // this is a `pub fn` and a caller could.
        assert_eq!(Nav::distance_label(-5), "0 m");
    }

    /// THE LOG LINE IS THE ONLY EVIDENCE THIS FEATURE CAN PRODUCE ON THE UNIT.
    #[test]
    fn the_log_line_says_what_arrived_or_says_nothing() {
        let mut nav = Nav::new();
        assert_eq!(nav.log_line(0), None, "nothing received, nothing to log");

        nav.update(240, 5, 100);
        assert_eq!(nav.log_line(100).as_deref(), Some("right in 240 m"));

        nav.speak(&[], &["Turn right onto Main Street".to_string()], 100);
        assert_eq!(
            nav.log_line(100).as_deref(),
            Some("right in 240 m — \"Turn right onto Main Street\"")
        );

        nav.update(60, 12, 110);
        assert!(nav.log_line(110).unwrap().starts_with("OFF ROUTE by 60 m"));

        nav.clear();
        assert_eq!(nav.log_line(110), None, "clearing leaves nothing to say");
    }
}
