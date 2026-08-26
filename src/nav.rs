//! Turn-by-turn navigation, decided here rather than at the seam.
//!
//! ## What arrives, and how little of it there is
//!
//! OsmAnd's AIDL API hands an outside app THREE INTEGERS per update —
//! `distanceTo`, `turnType`, `isLeftSide` — and separately, on a different
//! callback, the voice router's announcement as a list of strings. There is no
//! street name in the structured half, no exit number, no ETA, no distance to
//! the destination. Everything this module does is make those two thin streams
//! into one thing a face can draw.
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
//! Presentation. The design handoff for the navigation strip is still in
//! progress, so this module publishes a [`Nav`] and stops: a turn, a distance in
//! metres, and the words. How that is drawn — glyph, colour, size, where it sits
//! on the face — is the handoff's, and guessing at it now would be building
//! something to throw away.

/// Which way the next turn goes.
///
/// THE INTEGERS ARE OSMAND'S, read out of `OsmAnd-java/.../router/TurnType.java`
/// where each is a `public static final int`. They cross the AIDL boundary bare:
/// `OsmandAidlApi` sets `directionInfo.setTurnType(ndi.directionInfo.getTurnType()
/// .getValue())`, with nothing packed alongside — so a roundabout says RNDB and
/// the EXIT NUMBER, which `TurnType.getExitOut()` has on the other side of the
/// wire, does not travel.
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
    /// `RNDB = 13`, `RNLB = 14`. Which exit is NOT on the wire; see the type note.
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

/// The navigation state the face draws, and the clock that ages it.
#[derive(Clone, Debug, Default)]
pub struct Nav {
    /// The last update's three integers, and when they landed.
    last: Option<(i32, i32, u64)>,
    /// The last spoken instruction, and when it was said.
    spoken: Option<(String, u64)>,
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

    /// Forget everything. The switch went off, or OsmAnd went away.
    pub fn clear(&mut self) {
        self.last = None;
        self.spoken = None;
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
        Some(match self.spoken(now) {
            Some(words) => format!("{head} — \"{words}\""),
            None => head,
        })
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
        assert!(Nav::SPOKEN_EXPIRY > Nav::EXPIRY, "the words must not outlive the turn's clock by accident");
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
