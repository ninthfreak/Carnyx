//! How far away a turn is, in the units the driver expects (§4.9).
//!
//! ## Why this is a guess at all
//!
//! §4.9: *"Distances follow the locale (feet under 1000 ft, then miles to one
//! decimal; metres under 1 km, then km) **because OsmAnd's own unit setting is
//! not exposed over the API**. If that ever reads wrong to a driver it needs an
//! app-side override, not a guess."*
//!
//! ## THE SPEC WAS WRONG ABOUT THAT, AND THE GUESS IS NOW THE FALLBACK
//!
//! The premise held for the navigation surface and only for it: `AppInfoParams`
//! hands over `leftDistance`, and the turn bundle `next_turn_distance`, as bare
//! integer metres with no unit beside them and no getter to ask. Every one of
//! those was checked before this was written, and none of them carries a unit.
//!
//! What none of that covers is that OsmAnd exposes its WHOLE settings store
//! through a separate call, `getPreference`, and the units sit in it under
//! `default_metric_system` — an `EnumStringPreference<MetricsConstants>`, marked
//! `makeProfile()`, which is what puts it inside upstream's export gate. So the
//! driver's own answer is readable after all, and [`Units::resolve`] asks for it
//! first. See `CarnyxNav.readMetricSystem` for the call and slot 94 of
//! `IOsmAndAidlInterface.aidl` for where it sits.
//!
//! WHAT WENT WRONG BEFORE IT DID. The failure the spec predicted — "a driver can
//! hear miles and read kilometres" — is exactly what happened, by the route it
//! did not predict. The guess reads the LOCALE's country, this head unit reports
//! no country at all, and an empty code meant [`Units::Metric`]: a US car,
//! OsmAnd speaking miles, the face drawing kilometres. §4.9's own answer to that
//! was "an app-side override, not a guess", which is now [`FALLBACK`].
//!
//! ## What Java is asked, and what it is not
//!
//! Java answers ONE FACT — the ISO 3166 country code — and this file decides
//! what that means, for the reason the whole tree states: a decision made in
//! Java cannot be tested on a machine with no head unit.

/// Which family of units a distance is printed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Units {
    #[default]
    Metric,
    /// Feet and miles.
    Imperial,
}

impl Units {
    /// The countries that post ROAD distances in miles.
    ///
    /// ── THIS IS A ROAD-SIGN LIST, NOT A "METRIC SYSTEM" LIST ────────────────
    ///
    /// The two are not the same and using the wrong one is how the UK gets
    /// kilometres. Britain is metric for almost everything and its road signs
    /// are in miles; several Caribbean states are the same. So the test is
    /// "what does a distance sign say by the roadside", which is the only
    /// question this module is answering.
    ///
    /// ISO 3166-1 alpha-2, upper case:
    ///
    /// * `US` and its territories `PR` `VI` `GU` `AS` `MP`
    /// * `GB` — miles on every road sign, metric elsewhere — and the Crown
    ///   dependencies `IM` `JE` `GG`, which sign the same way and are their own
    ///   ISO codes, not part of `GB`. Left off the first cut of this list,
    ///   which is exactly the mistake a "which countries use miles" list makes
    ///   when it is really a "which locales exist" question.
    /// * `LR` `MM` — the other two countries that never adopted metric
    /// * `AG` `AI` `BS` `BZ` `DM` `FK` `GD` `KN` `KY` `LC` `MS` `SH` `TC` `VC` `VG` `WS`
    ///   — Caribbean and South Atlantic states signing in miles
    ///
    /// EVERYWHERE ELSE IS METRIC, including the whole of Europe bar Britain,
    /// because a wrong guess toward metric is the smaller error: a driver who
    /// reads "400 m" for four hundred metres is right, and one who reads
    /// "1300 ft" for the same distance is being told a number nothing on the
    /// road will confirm.
    const MILES: &'static [&'static str] = &[
        "AG", "AI", "AS", "BS", "BZ", "DM", "FK", "GB", "GD", "GG", "GU", "IM", "JE", "KN", "KY",
        "LC", "LR", "MM", "MP", "MS", "PR", "SH", "TC", "US", "VC", "VG", "VI", "WS",
    ];

    /// What a country signs its roads in.
    ///
    /// An empty or unrecognised code is [`Units::Metric`] — see the list's note
    /// for why that is the safer of the two defaults.
    pub fn for_country(code: &str) -> Units {
        let c = code.trim().to_ascii_uppercase();
        if Self::MILES.contains(&c.as_str()) {
            Units::Imperial
        } else {
            Units::Metric
        }
    }

    /// What OsmAnd's own units preference says, or [`None`] when it said nothing.
    ///
    /// The encoding is `CarnyxNav.metricSystem`'s, which is upstream's
    /// `MetricsConstants` in declaration order with 0 for "not known". Java hands
    /// over WHICH CONSTANT and this decides what it means, which is the split the
    /// module note describes: the six-way choice below has tests and a decision
    /// made in Java would not.
    ///
    /// THREE OF THE SIX DO NOT FIT IN TWO, and rounding them is the honest part
    /// of this function rather than a shortcut. `Units` is metric-or-imperial
    /// because §4.9 specifies exactly two ladders — `m`/`km` and `ft`/`mi` — so
    /// OsmAnd's hybrids have to land on one:
    ///
    /// * `MILES_AND_METERS` and `NAUTICAL_MILES_AND_METERS` keep METRES for the
    ///   short leg, which is where a turn-by-turn display spends nearly all its
    ///   time — "in 240 m" is the line that matters and "in 800 ft" would be the
    ///   wrong one. So they resolve METRIC even though the long leg is miles,
    ///   because the short leg is what the driver reads at a junction.
    /// * `MILES_AND_YARDS` resolves IMPERIAL. Yards are not a ladder this app
    ///   draws, and feet are the near neighbour: both are imperial short units a
    ///   US driver reads without conversion, and the alternative — metres — is
    ///   the one thing a driver who chose yards definitely did not ask for.
    /// * `NAUTICAL_MILES_AND_FEET` resolves IMPERIAL for the same reason in
    ///   reverse: the short leg is feet, and this app has no nautical mile.
    ///
    /// Nobody sets a car head unit to nautical miles, so those two are here to
    /// be TOTAL rather than because they are expected — an unhandled arm would
    /// mean falling through to the guess for a driver who had answered the
    /// question.
    pub fn from_osmand(code: i32) -> Option<Units> {
        match code {
            1 | 3 | 5 => Some(Units::Metric),
            2 | 4 | 6 => Some(Units::Imperial),
            _ => None,
        }
    }

    /// The units to draw in, best source first.
    ///
    /// ── THE ORDER IS THE WHOLE POINT ────────────────────────────────────────
    ///
    /// 1. WHAT OSMAND WAS TOLD. The driver already answered this question, in
    ///    the app that is giving them directions, and any other source is this
    ///    app second-guessing an answer it can now simply read. §4.9 wrote the
    ///    guess below because "OsmAnd's own unit setting is not exposed over the
    ///    API" — true of the navigation surface, false of `getPreference`.
    /// 2. WHAT THE ROAD SIGNS SAY where the unit thinks it is, which is the old
    ///    behaviour and still right for a drive with no navigation running.
    /// 3. [`FALLBACK`], for when the locale is empty or unrecognised.
    ///
    /// THE SPEC ASKED FOR THIS EXACT SHAPE. §4.9's own next sentence is "if that
    /// ever reads wrong to a driver it needs an app-side override, not a guess" —
    /// and it did read wrong: a unit whose locale carries no country signed a US
    /// driver's turns in kilometres while OsmAnd spoke them in miles.
    pub fn resolve(osmand: i32, country: &str) -> Units {
        if let Some(u) = Self::from_osmand(osmand) {
            return u;
        }
        let c = country.trim();
        if c.is_empty() {
            return FALLBACK;
        }
        Self::for_country(c)
    }
}

/// What to draw when nothing can be read: not [`Units::Metric`], which is what
/// an unreadable locale used to mean.
///
/// ── THIS APP IS NORTH AMERICAN BY DESIGN, WHICH SETTLES IT ──────────────────
///
/// [`Units::MILES`]'s note argues metric is the safer default because a wrong
/// guess toward metric is the smaller error. That argument is about an app whose
/// driver could be anywhere. This one's cannot: the station database is the
/// FCC's, and `crate::stations` can only answer questions about the United
/// States, so a Carnyx running anywhere else has already lost a larger feature
/// than this one. A default that is wrong for every user the app can actually
/// serve is not the safer default.
///
/// NOT A STATEMENT ABOUT ONE HEAD UNIT. An earlier draft of this note justified
/// the same constant as "one head unit, in one car, in the United States", which
/// is the wrong reason for a right answer and the kind of reasoning that ends up
/// designing for a single device. The app targets Android head units generally;
/// what narrows it to imperial is the FCC dependency, not whose dashboard it is
/// sitting in.
///
/// It applies ONLY where the locale is silent. A unit that reports `DE` still
/// gets kilometres, because that is a real answer and this is what stands in for
/// no answer at all.
pub const FALLBACK: Units = Units::Imperial;

/// Metres in one foot, and in one mile — exactly, as the international
/// definitions give them.
const FOOT: f64 = 0.3048;
const MILE: f64 = 1609.344;

/// `240 m` / `1.4 km` / `800 ft` / `1.4 mi`, per §4.9's own two breaks.
///
/// THE BREAKS ARE THE SPEC'S: *"feet under 1000 ft, then miles to one decimal;
/// metres under 1 km, then km"*. Past ten of the large unit the tenth is
/// dropped, which is how every turn-by-turn display reads — `9.9 km`, `12 km` —
/// and is the behaviour the metric side already shipped with.
pub fn distance(metres: i32, units: Units) -> String {
    let m = f64::from(metres.max(0));
    match units {
        Units::Metric => {
            if m < 1000.0 {
                return format!("{} m", m as i32);
            }
            large(m / 1000.0, "km")
        }
        Units::Imperial => {
            let feet = m / FOOT;
            if feet < 1000.0 {
                // ROUNDED TO TEN FEET, and the metric side is not rounded at
                // all, which is not an inconsistency. Metres arrive as metres:
                // printing one of them is printing what OsmAnd said. Feet are
                // CONVERTED, and a metre of granularity becomes 3.28 feet — so
                // `787 ft` claims a precision that was never in the number, and
                // a countdown built on it jitters by threes.
                let ten = (feet / 10.0).round() as i32 * 10;
                return format!("{ten} ft");
            }
            large(m / MILE, "mi")
        }
    }
}

/// One decimal under ten, none above.
///
/// THE BRANCH IS ON WHAT WILL BE PRINTED, NOT ON THE RAW VALUE, and that is not
/// pedantry: 9.9997 is under ten, and `{:.1}` prints it as `10.0` — the exact
/// two-character form this branch exists to avoid. Ten miles is 16093.44 m, so
/// a real turn at 16093 m used to read `10.0 mi`. The metric side carried the
/// same wart at 9995 m.
fn large(v: f64, unit: &str) -> String {
    if (v * 10.0).round() < 100.0 {
        format!("{v:.1} {unit}")
    } else {
        format!("{:.0} {unit}", v.round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_reads_the_way_a_turn_by_turn_display_does() {
        let m = Units::Metric;
        assert_eq!(distance(0, m), "0 m");
        assert_eq!(distance(240, m), "240 m");
        assert_eq!(distance(999, m), "999 m");
        assert_eq!(distance(1000, m), "1.0 km");
        assert_eq!(distance(1449, m), "1.4 km");
        assert_eq!(distance(9949, m), "9.9 km");
        assert_eq!(distance(10000, m), "10 km");
        assert_eq!(distance(12400, m), "12 km");
        // THE BRANCH IS ON THE PRINTED VALUE: 9.995 km would format as "10.0 km"
        // under one decimal, which is the form the second branch exists for.
        assert_eq!(distance(9995, m), "10 km");
        // A negative never reaches here through `Nav::state`, which floors it —
        // but this is a `pub fn` and a caller could.
        assert_eq!(distance(-5, m), "0 m");
    }

    /// §4.9'S IMPERIAL BREAK, AT THE FOOT IT NAMES.
    #[test]
    fn imperial_breaks_at_a_thousand_feet() {
        let i = Units::Imperial;
        assert_eq!(distance(0, i), "0 ft");
        // 1000 ft is 304.8 m: one metre either side of the break.
        assert_eq!(distance(304, i), "1000 ft", "just under the break");
        assert_eq!(distance(305, i), "0.2 mi", "and just over it");
        assert_eq!(distance(100, i), "330 ft");
        assert_eq!(distance(1609, i), "1.0 mi");
        assert_eq!(distance(2414, i), "1.5 mi");
        assert_eq!(distance(16093, i), "10 mi");
        assert_eq!(distance(19312, i), "12 mi");
    }

    /// FEET ARE ROUNDED AND METRES ARE NOT, because feet are converted.
    ///
    /// A metre of granularity is 3.28 feet, so an unrounded readout counts down
    /// in threes and claims a precision the source never had.
    #[test]
    fn feet_do_not_claim_a_precision_the_metres_never_had() {
        for m in 90..100 {
            let ft = distance(m, Units::Imperial);
            let n: i32 = ft.trim_end_matches(" ft").parse().expect("a whole number of feet");
            assert_eq!(n % 10, 0, "{m} m gave {ft}, which is not a round ten");
        }
        // And it is still the RIGHT ten: 240 m is 787.4 ft.
        assert_eq!(distance(240, Units::Imperial), "790 ft");
    }

    /// EVERY ONE OF OSMAND'S SIX, and the three awkward ones on purpose.
    ///
    /// The two that keep metres for the short leg have to come out METRIC even
    /// though their long leg is miles, because a turn-by-turn display spends
    /// almost all of its time on the short leg. Getting this backwards would
    /// draw "800 ft" at every junction for a driver who asked for metres, and it
    /// would look like a correct reading of a setting that says "MILES".
    #[test]
    fn osmands_six_unit_choices_each_land_somewhere() {
        assert_eq!(Units::from_osmand(1), Some(Units::Metric), "KILOMETERS_AND_METERS");
        assert_eq!(Units::from_osmand(2), Some(Units::Imperial), "MILES_AND_FEET");
        assert_eq!(Units::from_osmand(3), Some(Units::Metric), "MILES_AND_METERS keeps metres");
        assert_eq!(Units::from_osmand(4), Some(Units::Imperial), "MILES_AND_YARDS");
        assert_eq!(Units::from_osmand(5), Some(Units::Metric), "NAUTICAL_MILES_AND_METERS");
        assert_eq!(Units::from_osmand(6), Some(Units::Imperial), "NAUTICAL_MILES_AND_FEET");
        // 0 IS "OSMAND DID NOT SAY" and must not be a verdict — it is the value
        // the seam returns when OsmAnd is unbound, declined the read, or has no
        // slot 94 at all. Answering `Some(Metric)` here would reinstate the
        // exact bug this whole path exists to fix.
        assert_eq!(Units::from_osmand(0), None, "unknown is not an answer");
        for stray in [-1, 7, 99, i32::MIN, i32::MAX] {
            assert_eq!(Units::from_osmand(stray), None, "{stray} is not a constant");
        }
    }

    /// OSMAND OUTRANKS THE LOCALE, AND THE LOCALE OUTRANKS THE FALLBACK.
    #[test]
    fn the_drivers_own_answer_wins_over_the_guess() {
        // OsmAnd says metric on a US unit: metric. The driver chose it, in the
        // app doing the navigating, and this app does not know better.
        assert_eq!(Units::resolve(1, "US"), Units::Metric);
        // OsmAnd says imperial on a German unit: imperial, same reasoning.
        assert_eq!(Units::resolve(2, "DE"), Units::Imperial);
        // OsmAnd silent: the road signs where the unit thinks it is.
        assert_eq!(Units::resolve(0, "DE"), Units::Metric);
        assert_eq!(Units::resolve(0, "US"), Units::Imperial);
        // OsmAnd silent AND the locale silent, which is THIS head unit: the
        // fallback, not metric. An empty country used to mean kilometres, and
        // that is what drew a US drive in km while OsmAnd called it in miles.
        assert_eq!(Units::resolve(0, ""), FALLBACK);
        assert_eq!(Units::resolve(0, "   "), FALLBACK);
        // A REAL COUNTRY IS STILL A REAL ANSWER. The fallback stands in for
        // silence and must not swallow a locale that spoke.
        assert_eq!(Units::resolve(0, "FR"), Units::Metric);
    }

    /// THE LIST IS ABOUT ROAD SIGNS AND NOT ABOUT THE METRIC SYSTEM.
    ///
    /// Britain is the case that separates the two, and getting it wrong is the
    /// most likely single mistake this table can make.
    #[test]
    fn the_country_table_asks_what_the_road_signs_say() {
        for code in ["US", "GB", "PR", "LR", "MM", "KY", "VG"] {
            assert_eq!(Units::for_country(code), Units::Imperial, "{code} signs in miles");
        }
        // THE CROWN DEPENDENCIES ARE NOT `GB` — each has its own ISO code and
        // the same road signs. A phone bought on the Isle of Man says `IM`.
        for code in ["IM", "JE", "GG"] {
            assert_eq!(Units::for_country(code), Units::Imperial, "{code} signs like Britain");
        }
        for code in ["DE", "FR", "IE", "CA", "AU", "NZ", "ZA", "JP", "IN", "MX"] {
            assert_eq!(Units::for_country(code), Units::Metric, "{code} signs in km");
        }
        // IRELAND AND CANADA ARE THE TRAPS on the other side: both border a
        // miles country, both sign in kilometres.
        assert_eq!(Units::for_country("IE"), Units::Metric);
        assert_eq!(Units::for_country("CA"), Units::Metric);

        // Case and whitespace are the platform's, not ours.
        assert_eq!(Units::for_country("us"), Units::Imperial);
        assert_eq!(Units::for_country(" GB "), Units::Imperial);

        // AN UNKNOWN CODE IS METRIC rather than a panic or a guess — including
        // the empty string, which is what a device with no country set returns.
        for code in ["", "??", "XX", "ZZ"] {
            assert_eq!(Units::for_country(code), Units::Metric, "{code:?}");
        }
        assert_eq!(Units::default(), Units::Metric);
    }
}
