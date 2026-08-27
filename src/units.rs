//! How far away a turn is, in the units the driver expects (§4.9).
//!
//! ## Why this is a guess at all
//!
//! §4.9: *"Distances follow the locale (feet under 1000 ft, then miles to one
//! decimal; metres under 1 km, then km) **because OsmAnd's own unit setting is
//! not exposed over the API**. If that ever reads wrong to a driver it needs an
//! app-side override, not a guess."*
//!
//! That is the whole problem in one sentence. OsmAnd knows exactly which units
//! this driver has chosen — it is a preference in its own settings — and the
//! AIDL surface hands over `distanceTo` and `leftDistance` as bare integer
//! metres with no unit alongside and no getter to ask. So the radio has to pick,
//! and the only evidence it has is the phone's own locale.
//!
//! WHICH MEANS A DRIVER CAN HEAR MILES AND READ KILOMETRES. OsmAnd's spoken
//! prompt honours ITS setting and this line honours the LOCALE, so a driver with
//! a metric phone who set OsmAnd to miles gets both. The spec's own answer is
//! that this then needs "an app-side override, not a guess" — a switch, once
//! somebody hits it. Recorded here rather than left to be discovered.
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
}

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
