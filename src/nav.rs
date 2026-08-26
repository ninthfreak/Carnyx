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

use crate::units::Units;

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

/// How loudly the face should say the next maneuver (§4.9's three stages).
///
/// ── THE TRIGGER IS OSMAND'S, AND ITS SCALE WAS NOT OBVIOUS ──────────────────
///
/// §4.9: *"Escalation follows `next_turn_imminent` (and the voice router
/// firing), so the radio changes at the same moment OsmAnd speaks — never on
/// distance thresholds of our own, which would disagree with the voice at
/// highway speeds."*
///
/// This shipped once with the integer carried and LOGGED raw and nothing
/// branching on it, because what its values meant could not be read: the class
/// that computes it is not in OsmAnd's `OsmAnd-java` tree. It is in the Android
/// module — `OsmAnd/src/net/osmand/plus/routing/data/AnnounceTimeDistances.java`
/// — reached through `RoutingHelper` → `VoiceRouter.calculateImminent` →
/// `AnnounceTimeDistances.getImminentTurnStatus`, whose whole body is:
///
/// ```java
/// float speed = getSpeed(loc);
/// if (isTurnStateActive(speed, dist, STATE_TURN_NOW)) {
///     return 0;
/// } else if (isTurnStateActive(speed, dist, STATE_PREPARE_TURN)) {
///     // STATE_TURN_IN included
///     return 1;
/// } else {
///     return -1;
/// }
/// ```
///
/// THREE VALUES, AND ZERO IS THE MOST URGENT OF THEM — which is exactly the
/// trap that was worth not guessing at. A reader who assumed a rising scale
/// would have put the hero takeover on the cruise state and left the turn itself
/// unannounced.
///
/// The thresholds behind those two booleans are OsmAnd's, and they are
/// SPEED-SCALED rather than fixed distances: `PREPARE_DISTANCE = DEFAULT_SPEED *
/// 115` and `TURN_IN_DISTANCE = DEFAULT_SPEED * 22`, with a low-speed adjustment
/// on TURN_NOW. That is the whole reason §4.9 forbids thresholds of our own —
/// ours would be in metres and OsmAnd's are in seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Not navigating, or suppressed.
    #[default]
    Idle,
    /// `-1` — a route is running and the turn is far off. §4.9's stage 1: the
    /// hairline, the ETA under the clock, the cruise countdown.
    Cruise,
    /// `1` — `STATE_PREPARE_TURN`, `STATE_TURN_IN` included. §4.9's stage 2:
    /// the RadioText strip yields.
    Approach,
    /// `0` — `STATE_TURN_NOW`. §4.9's stage 3: the hero card takes over.
    TurnNow,
}

impl Stage {
    /// `next_turn_imminent`, read as OsmAnd's own three values.
    ///
    /// ANYTHING ELSE IS `Cruise` — not a panic, and not the loudest state. A
    /// value this table does not know is a newer OsmAnd, and the safe reading of
    /// an unknown urgency is the quiet one: escalating on it would hand the hero
    /// card to a turn that may be ten kilometres away.
    pub fn from_imminent(imminent: Option<i32>) -> Stage {
        match imminent {
            Some(0) => Stage::TurnNow,
            Some(1) => Stage::Approach,
            _ => Stage::Cruise,
        }
    }
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
    /// OsmAnd's `nextInfo.imminent` — `-1` cruise, `1` approach, `0` turn now.
    ///
    /// THE SCALE IS SETTLED AND IT IS NOT A RISING ONE. See [`Stage`], which
    /// holds the source it was read out of and why zero being the loudest value
    /// is the trap. Carried raw here and still LOGGED raw, so a drive can show
    /// what actually arrived rather than what this build made of it.
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
    /// The leg being driven: which turn it is, and how far away it was when it
    /// FIRST appeared. See [`Nav::progress`].
    leg: Option<(String, i32)>,
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
        self.note_leg(&route);
        self.route = Some((route, now));
    }

    /// Remember where this leg started, for [`Nav::progress`].
    ///
    /// A LEG IS A TURN PLUS ITS STREET, and the baseline is the first distance
    /// seen for it. Two rules reset it:
    ///
    /// * THE TURN CHANGED — a new leg, and its own full distance.
    /// * THE DISTANCE WENT UP — which happens without the turn changing when
    ///   OsmAnd recalculates around a missed exit, or when the driver rejoins
    ///   the route further back. Keeping the old baseline there would leave the
    ///   bar past full and then jumping backwards.
    fn note_leg(&mut self, route: &Route) {
        let Some(metres) = route.turn_metres.filter(|m| *m >= 0) else {
            self.leg = None;
            return;
        };
        let id = format!(
            "{}|{}",
            route.turn_xml.as_deref().unwrap_or_default(),
            route.street.as_deref().unwrap_or_default()
        );
        match &self.leg {
            // The same leg, no further away than has been seen: the baseline
            // stands. THIS ARM IS WHAT MAKES ARRIVING WORK — nought metres to
            // the turn is "you are at it", and it must fill the bar rather than
            // read as no leg at all.
            Some((was, baseline)) if *was == id && metres <= *baseline => {}
            // A different turn, or the same one further off than before.
            _ if metres > 0 => self.leg = Some((id, metres)),
            // First sight at nought metres: nothing to divide by, and nothing
            // was watched, so there is no fraction to claim.
            _ => self.leg = None,
        }
    }

    /// How far along this leg the driver is, `0.0` to `1.0` (§4.9's hairline).
    ///
    /// ── THE DENOMINATOR IS OURS BECAUSE OSMAND SENDS NO SUCH THING ──────────
    ///
    /// §4.9 asks for a hairline "its width the fraction of the way to the next
    /// turn". OsmAnd hands over the distance REMAINING and nothing to divide it
    /// by — there is no leg length in the bundle — so the baseline is the first
    /// distance this build saw for this turn, kept by [`Nav::note_leg`].
    ///
    /// WHICH MEANS IT IS HONEST ABOUT ITS OWN LIMIT: joining a route mid-leg
    /// makes the bar start at zero for a turn that is already close, rather than
    /// at four-fifths. That is the right failure — it never claims MORE progress
    /// than has been watched — and it settles within one turn.
    ///
    /// `0.0` with no route, which is a bar of no width rather than a full one.
    pub fn progress(&self, now: u64) -> f32 {
        let Some(route) = self.route(now) else { return 0.0 };
        let (Some((_, baseline)), Some(left)) = (&self.leg, route.turn_metres) else {
            return 0.0;
        };
        if *baseline <= 0 {
            return 0.0;
        }
        let done = (baseline - left.max(0)) as f32 / *baseline as f32;
        done.clamp(0.0, 1.0)
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
        self.leg = None;
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

    /// Which of §4.9's three stages the face should be in, at `now`.
    ///
    /// `showing` IS THE CALLER'S SUPPRESSION — the settings switch and OsmAnd's
    /// own map being in front. Taken as an argument rather than read here
    /// because this module holds no settings, and folding it in would make a
    /// hidden layer indistinguishable from a finished route.
    ///
    /// NO ROUTE MEANS `Idle` WHATEVER THE POLL LAST SAID. A stale `imminent`
    /// outliving its turn is exactly the case `EXPIRY` exists for, and reading
    /// the two independently would leave the hero card taken over by a junction
    /// the driver passed a minute ago.
    pub fn stage(&self, now: u64, showing: bool) -> Stage {
        if !showing || matches!(self.state(now), NavState::Idle) {
            return Stage::Idle;
        }
        match self.route(now) {
            Some(r) => Stage::from_imminent(r.imminent),
            // PUSHING WITHOUT A POLL IS STILL CRUISING. The push callback is the
            // one that keeps arriving on a slow route; a face that fell back to
            // `Idle` because the poll was a second late would blink.
            None => Stage::Cruise,
        }
    }

    /// The last announcement, if it is still fresh.
    pub fn spoken(&self, now: u64) -> Option<&str> {
        self.spoken
            .as_ref()
            .filter(|(_, at)| now.saturating_sub(*at) < Self::SPOKEN_EXPIRY)
            .map(|(text, _)| text.as_str())
    }

    /// `240 m` / `1.4 km` / `790 ft` / `1.4 mi` (§4.9).
    ///
    /// THE UNITS ARE THE DRIVER'S AND NOT THIS MODULE'S. They come from the
    /// device's locale because OsmAnd does not expose its own setting over the
    /// API — [`crate::units`] holds the breaks, the country table and the
    /// reason that is a guess at all.
    ///
    /// This used to be metric-only with the gap written down beside it; the
    /// gap is now closed and the caveat has moved to `units`, which is the file
    /// that can actually do something about it.
    pub fn distance_label(metres: i32, units: Units) -> String {
        crate::units::distance(metres, units)
    }

    /// One line for the diagnostics log, or `None` when there is nothing to say.
    ///
    /// The unit has no adb, so this is the only way a drive can report what the
    /// integration actually received.
    pub fn log_line(&self, now: u64, units: Units) -> Option<String> {
        let head = match self.state(now) {
            NavState::Idle => return None,
            NavState::Waiting => "navigating, no turn yet".to_string(),
            NavState::OffRoute { metres } => {
                format!("OFF ROUTE by {}", Self::distance_label(metres, units))
            }
            NavState::Turn { turn, metres } => {
                format!("{} in {}", turn.name(), Self::distance_label(metres, units))
            }
            NavState::Unknown { code, metres } => {
                format!("turn type {code} (unknown to this build) in {}", Self::distance_label(metres, units))
            }
        };
        let mut out = head;
        if let Some(r) = self.route(now) {
            // THE STREET AND THE RAW `imminent`, still raw now that `Stage`
            // knows what it means: the log's job is to say what ARRIVED, and a
            // line printing "Approach" could not tell a wrong reading of the
            // integer from a wrong integer.
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

/// Everything the settings sub-line needs to answer "why is nothing showing".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link<'a> {
    /// Whatever `CarnyxNav.installedPackage` found, or empty.
    pub package: &'a str,
    /// The switch itself.
    pub on: bool,
    /// The poll is answering. See `App::push_nav`.
    pub linked: bool,
    /// OsmAnd has a route running.
    pub navigating: bool,
    /// OsmAnd's own map is in front.
    pub map_visible: bool,
    /// Settings ▸ NAVIGATION ▸ Hide when the map is showing.
    pub hide_on_map: bool,
}

/// The settings row's sub-line.
///
/// ── IT ANSWERS ONE QUESTION: WHY IS NOTHING SHOWING? ────────────────────────
///
/// §4.9: *"its sub-line reports the link state and why nothing is showing —
/// OsmAnd idle, off here, or hidden behind the map"*. Those three are the
/// reasons a driver actually hits, and each of them looks identical on the face
/// — a blank strip — so the row is the only place they can be told apart.
///
/// THE ROW ALSO HAS TO ANSWER "is OsmAnd even here" WITHOUT BEING TURNED ON,
/// which is why the not-installed case is first and does not depend on `on`. A
/// switch whose only failure report arrives after you flip it makes the driver
/// run an experiment to learn something the package manager already knows.
///
/// NOT ONE OF THESE READS AS A FAULT. A driver without OsmAnd, or with it
/// closed, has not broken anything, and `the_sub_line_never_reads_as_a_fault`
/// holds that line across every branch.
pub fn sub_line(link: &Link) -> String {
    const WHAT: &str = "Show OsmAnd's next turn on the face.";
    if link.package.is_empty() {
        return format!("{WHAT} OsmAnd is not installed.");
    }
    if !link.on {
        // THE SWITCH'S OWN POSITION ALREADY SAYS "off", so this says the thing
        // the switch cannot: which OsmAnd it would talk to if it were on.
        return format!("{WHAT} Off. Found {}.", link.package);
    }
    if !link.linked {
        // ON, INSTALLED, AND NOTHING ANSWERING — OsmAnd is not running. Worded
        // as waiting rather than failing, because it is: the bind stands and
        // the first poll after OsmAnd starts lights it with nothing to press.
        return format!("{WHAT} Waiting for {} to start.", link.package);
    }
    if !link.navigating {
        // "OsmAnd idle" — connected, no route. The single most common reason
        // for a blank strip, and the one most likely to be read as broken.
        return format!("{WHAT} Connected to {}, with no route running.", link.package);
    }
    if link.map_visible && link.hide_on_map {
        // "hidden behind the map" — and it names the switch that changes it,
        // because a driver reading this sentence is asking how to turn it off.
        return format!("{WHAT} Hidden while OsmAnd's map is in front — see below.");
    }
    format!("{WHAT} Showing the next turn from {}.", link.package)
}

/// The sub-line under "Hide when the map is showing".
pub fn hide_sub_line() -> String {
    "While OsmAnd's own map is on screen the radio leaves the turn to it, \
     because you are already looking at one."
        .to_string()
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

    /// A link with OsmAnd found, the switch on, and nothing else claimed.
    fn linked_to(package: &str) -> Link<'_> {
        Link { package, on: true, hide_on_map: true, ..Link::default() }
    }

    /// THE ROW SAYS WHETHER THERE IS ANYTHING TO TALK TO, BEFORE IT IS TAPPED.
    #[test]
    fn the_settings_sub_line_names_the_osmand_it_found() {
        assert!(sub_line(&linked_to("net.osmand.plus")).contains("net.osmand.plus"));
        assert!(sub_line(&Link::default()).contains("not installed"));
        // NOT INSTALLED IS ANSWERED WITH THE SWITCH OFF TOO — the question is
        // about the phone, not about the setting.
        let off_and_absent = Link { on: false, ..Link::default() };
        assert!(sub_line(&off_and_absent).contains("not installed"));
    }

    /// EVERY REASON THE STRIP CAN BE BLANK IS A DIFFERENT SENTENCE.
    ///
    /// §4.9 names three — "OsmAnd idle, off here, or hidden behind the map" —
    /// and they look identical on the face. If two of them ever collapse into
    /// one sentence, the row has stopped doing the only job it has.
    #[test]
    fn each_reason_for_a_blank_strip_says_which_one_it_is() {
        let pkg = "net.osmand.plus";
        let off = Link { on: false, ..linked_to(pkg) };
        let waiting = linked_to(pkg);
        let idle = Link { linked: true, ..linked_to(pkg) };
        let behind_map = Link {
            linked: true,
            navigating: true,
            map_visible: true,
            ..linked_to(pkg)
        };
        let showing = Link { linked: true, navigating: true, ..linked_to(pkg) };

        assert!(sub_line(&off).contains("Off"), "{}", sub_line(&off));
        assert!(sub_line(&waiting).contains("Waiting"), "{}", sub_line(&waiting));
        assert!(sub_line(&idle).contains("no route"), "{}", sub_line(&idle));
        assert!(sub_line(&behind_map).contains("map is in front"), "{}", sub_line(&behind_map));
        assert!(sub_line(&showing).contains("Showing"), "{}", sub_line(&showing));

        // AND NO TWO OF THEM ARE THE SAME STRING.
        let all = [&off, &waiting, &idle, &behind_map, &showing].map(|l| sub_line(l));
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "two states share one sentence");
            }
        }

        // THE SUPPRESSION IS ONLY REPORTED WHEN IT IS SWITCHED ON. With the
        // hide switch off, a visible map changes nothing and the row must not
        // claim the strip is hidden.
        let map_but_not_hiding = Link { hide_on_map: false, ..behind_map.clone() };
        assert_eq!(sub_line(&map_but_not_hiding), sub_line(&showing));
    }

    /// IT NEVER READS AS A FAULT, in any state.
    ///
    /// "Error", "failed" and "unavailable" are what a driver reads as "something
    /// is broken". Not having an app is not broken, and neither is having it
    /// closed — which is the state this row will show most often.
    #[test]
    fn the_sub_line_never_reads_as_a_fault() {
        let pkg = "net.osmand.plus";
        let every_state = [
            Link::default(),
            Link { on: false, ..linked_to(pkg) },
            linked_to(pkg),
            Link { linked: true, ..linked_to(pkg) },
            Link { linked: true, navigating: true, map_visible: true, ..linked_to(pkg) },
            Link { linked: true, navigating: true, ..linked_to(pkg) },
        ];
        for link in &every_state {
            let line = sub_line(link);
            for word in ["rror", "ailed", "navailable", "annot", "roblem"] {
                assert!(!line.contains(word), "{word:?} reads as a fault in {line:?}");
            }
            // And every one of them says what the switch is FOR, so the row
            // still explains itself when it is also explaining a state.
            assert!(line.starts_with("Show OsmAnd's next turn"), "{line}");
        }
        assert!(!hide_sub_line().is_empty());
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
        let line = nav.log_line(100, Units::Metric).unwrap();
        assert!(line.contains("right in 240 m"), "{line}");
        assert!(line.contains("onto Whitney Way"), "{line}");
        assert!(line.contains("[imminent=2]"), "the raw integer, so a drive settles it: {line}");
        assert!(line.contains("map in front"), "{line}");
    }

    /// THE UNITS REACH THE LINE, WHICH IS ALL THIS FILE OWES THEM.
    ///
    /// The breaks, the country table and the reason any of it is a guess are
    /// `crate::units`'s and tested there. What could still go wrong HERE is a
    /// `Units` that is accepted and then dropped — a signature that takes it and
    /// a body that formats metres anyway — so this checks one distance in both
    /// families rather than restating the table.
    #[test]
    fn the_distance_is_the_drivers_own() {
        assert_eq!(Nav::distance_label(240, Units::Metric), "240 m");
        assert_eq!(Nav::distance_label(240, Units::Imperial), "790 ft");
        assert_eq!(Nav::distance_label(4000, Units::Metric), "4.0 km");
        assert_eq!(Nav::distance_label(4000, Units::Imperial), "2.5 mi");
    }

    /// OSMAND'S THREE IMMINENT VALUES, AND ZERO IS THE LOUDEST.
    ///
    /// Transcribed from `AnnounceTimeDistances.getImminentTurnStatus`, whose
    /// body returns 0 for `STATE_TURN_NOW`, 1 for `STATE_PREPARE_TURN` (with
    /// `STATE_TURN_IN` folded in) and -1 otherwise. The ordering is the whole
    /// point: a rising-scale reading would put the hero takeover on the cruise
    /// state and leave the turn itself unannounced.
    #[test]
    fn the_stage_ladder_is_osmands_own_and_zero_is_the_loudest_rung() {
        assert_eq!(Stage::from_imminent(Some(0)), Stage::TurnNow, "0 is STATE_TURN_NOW");
        assert_eq!(Stage::from_imminent(Some(1)), Stage::Approach, "1 is STATE_PREPARE_TURN");
        assert_eq!(Stage::from_imminent(Some(-1)), Stage::Cruise, "-1 is neither");
        // AN UNKNOWN VALUE IS THE QUIET STATE. A newer OsmAnd returning 2 must
        // not be read as more urgent than 1 — the scale does not rise.
        for v in [2, 3, 42, -7] {
            assert_eq!(Stage::from_imminent(Some(v)), Stage::Cruise, "{v}");
        }
        assert_eq!(Stage::from_imminent(None), Stage::Cruise, "a poll with no key");
        // AND THE ORDERING IS BY URGENCY, not by the integer.
        assert!(Stage::Idle < Stage::Cruise);
        assert!(Stage::Cruise < Stage::Approach);
        assert!(Stage::Approach < Stage::TurnNow);
    }

    /// THE STAGE NEEDS A LIVE ROUTE AND PERMISSION TO DRAW.
    #[test]
    fn the_stage_is_idle_without_a_route_or_without_permission() {
        let mut nav = Nav::new();
        assert_eq!(nav.stage(0, true), Stage::Idle, "nothing received");

        nav.update(240, 5, 100);
        assert_eq!(nav.stage(100, true), Stage::Cruise, "pushing, no poll yet");
        // SUPPRESSED IS IDLE, which is what the map-visible switch turns into.
        assert_eq!(nav.stage(100, false), Stage::Idle, "hidden behind OsmAnd's map");

        nav.poll(Route { imminent: Some(1), ..Route::default() }, 100);
        assert_eq!(nav.stage(100, true), Stage::Approach);
        nav.poll(Route { imminent: Some(0), ..Route::default() }, 100);
        assert_eq!(nav.stage(100, true), Stage::TurnNow);

        // AND A STALE `imminent` DOES NOT OUTLIVE ITS TURN. Both halves age on
        // one clock; without that the hero card would stay taken over by a
        // junction the driver passed a minute ago.
        assert_eq!(nav.stage(100 + Nav::EXPIRY, true), Stage::Idle);
    }

    /// THE HAIRLINE'S FRACTION, WHOSE DENOMINATOR THIS BUILD HAS TO INVENT.
    ///
    /// OsmAnd sends the distance REMAINING and nothing to divide it by, so the
    /// baseline is the first distance seen for a turn. What that has to get
    /// right is when to forget it.
    #[test]
    fn the_leg_progress_fills_as_the_turn_comes_up() {
        let mut nav = Nav::new();
        let leg = |xml: &str, street: &str, m: i32| Route {
            turn_xml: Some(xml.into()),
            street: Some(street.into()),
            turn_metres: Some(m),
            ..Route::default()
        };
        assert_eq!(nav.progress(0), 0.0, "no route is an empty bar, not a full one");

        // FIRST SIGHT SETS THE BASELINE, so the bar starts empty.
        nav.poll(leg("TR", "Whitney Way", 800), 100);
        assert_eq!(nav.progress(100), 0.0);
        nav.poll(leg("TR", "Whitney Way", 400), 101);
        assert!((nav.progress(101) - 0.5).abs() < 0.001, "half way");
        nav.poll(leg("TR", "Whitney Way", 0), 102);
        assert_eq!(nav.progress(102), 1.0, "arrived at the turn");

        // A NEW TURN IS A NEW LEG, with its own baseline — the bar empties
        // rather than staying full from the turn just taken.
        nav.poll(leg("TSLL", "Odana Rd", 1200), 103);
        assert_eq!(nav.progress(103), 0.0);
        nav.poll(leg("TSLL", "Odana Rd", 300), 104);
        assert!((nav.progress(104) - 0.75).abs() < 0.001);

        // THE SAME TURN GETTING FURTHER AWAY RE-BASELINES. OsmAnd does this on
        // a recalculation around a missed exit; keeping the old baseline would
        // drive the bar past full and then jump it backwards.
        nav.poll(leg("TSLL", "Odana Rd", 2000), 105);
        assert_eq!(nav.progress(105), 0.0, "re-baselined, not clamped at 1.0");
        nav.poll(leg("TSLL", "Odana Rd", 1000), 106);
        assert!((nav.progress(106) - 0.5).abs() < 0.001);

        // A POLL WITH NO TURN DISTANCE HAS NO LEG.
        nav.poll(Route { street: Some("Odana Rd".into()), ..Route::default() }, 107);
        assert_eq!(nav.progress(107), 0.0);

        // AND IT NEVER LEAVES THE RANGE, whatever arrives.
        nav.poll(leg("TR", "A", 500), 108);
        nav.poll(leg("TR", "A", -50), 109);
        let p = nav.progress(109);
        assert!((0.0..=1.0).contains(&p), "{p}");

        // A STALE ROUTE IS AN EMPTY BAR, on the same clock as everything else.
        assert_eq!(nav.progress(109 + Nav::EXPIRY), 0.0);
        nav.clear();
        assert_eq!(nav.progress(109), 0.0, "clearing takes the leg too");
    }

    /// THE LOG LINE IS THE ONLY EVIDENCE THIS FEATURE CAN PRODUCE ON THE UNIT.
    #[test]
    fn the_log_line_says_what_arrived_or_says_nothing() {
        let mut nav = Nav::new();
        let m = Units::Metric;
        assert_eq!(nav.log_line(0, m), None, "nothing received, nothing to log");

        nav.update(240, 5, 100);
        assert_eq!(nav.log_line(100, m).as_deref(), Some("right in 240 m"));
        // AND THE LOG READS IN THE DRIVER'S UNITS TOO, because the line exists
        // for a driver to quote back: "it said 790 ft" has to be findable.
        assert_eq!(nav.log_line(100, Units::Imperial).as_deref(), Some("right in 790 ft"));

        nav.speak(&[], &["Turn right onto Main Street".to_string()], 100);
        assert_eq!(
            nav.log_line(100, m).as_deref(),
            Some("right in 240 m — \"Turn right onto Main Street\"")
        );

        nav.update(60, 12, 110);
        assert!(nav.log_line(110, m).unwrap().starts_with("OFF ROUTE by 60 m"));

        nav.clear();
        assert_eq!(nav.log_line(110, m), None, "clearing leaves nothing to say");
    }
}
