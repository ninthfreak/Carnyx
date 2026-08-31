//! What is on each frequency around here, learned from the times the GPS worked.
//!
//! Ported from CarFM's `services/stationFinder.ts` (`@carfm/freq_callsign_v1`),
//! and its comment names the exact fault this exists for:
//!
//! > The dataset lookup is GEO-gated (`getNearbyStations` needs a GPS fix to know
//! > which 105.9 you mean). On a head unit that rarely gets a lock — esp. a cold
//! > start in a garage — that returns nothing and every station reads "Tuning…".
//! > So we also LEARN every freq→callsign we resolve into a persistent cache: one
//! > good lock (e.g. while driving) fills it for the whole local band, and later
//! > no-lock starts resolve names straight from it.
//!
//! ## Why the station database alone is not enough
//!
//! 88.7 MHz has 178 full-power licensees across the United States. Without a
//! position there is no way to say which of them is the one being received, and
//! `app::resolve` correctly answers "nothing" rather than naming a stranger. A
//! car parked in its own driveway with no view of the sky therefore has a radio
//! that cannot name a single station — which is not a hypothetical, it is where
//! the owner's head unit lives.
//!
//! This is the missing input: the answer the database gave LAST time there was a
//! fix, kept by frequency. Region-agnostic on purpose, which is fine for a car
//! that lives in one area, and a fresh lock overwrites every entry it can see, so
//! it heals itself if the car moves to another market.
//!
//! ## What it is not
//!
//! It is not authority. A live resolution always wins, because the position is
//! the only thing that can tell 178 stations apart; this only ever answers when
//! there is nothing better. And it is not a position cache — no coordinates are
//! stored, so a stale entry can name the wrong station but can never place the
//! car somewhere it is not.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::logos::json::{self, Json};

/// The file, beside `prefs.json` in the app's own data directory.
pub const FILE: &str = "callsigns.json";

/// A dial as a map key: tenths of a MHz.
///
/// The same key CarFM uses (`Math.round(freqMhz * 10)`), and it has to be an
/// integer — 105.5 does not round-trip through an `f32`, so `==` on two of them
/// is a coin toss and a `HashMap<f32, _>` cannot be built at all.
///
/// `None` for anything outside the FM band, which is not a dial this radio can
/// have been tuned to and so is not something worth remembering. The band edges
/// come from [`crate::app::FM_LO`]/[`crate::app::FM_HI`] rather than being
/// written out again here — see the comment on those constants.
pub fn key(mhz: f32) -> Option<u32> {
    if !mhz.is_finite() || !(crate::app::FM_LO..=crate::app::FM_HI).contains(&mhz) {
        return None;
    }
    Some((mhz * 10.0).round() as u32)
}

/// The learned map.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Callsigns {
    /// Ordered, so the file is stable and a diff of it is readable.
    map: BTreeMap<u32, String>,
}

impl Callsigns {
    /// What was last seen on this dial, if anything ever was.
    pub fn get(&self, mhz: f32) -> Option<&str> {
        self.map.get(&key(mhz)?).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Take everything a fix just revealed, NEAREST FIRST.
    ///
    /// Two rules, both CarFM's:
    ///
    /// * The nearest station on a frequency wins it. Later rows on the same dial
    ///   are farther away and are dropped, which is what makes this the answer to
    ///   "what am I receiving on 88.7" rather than "who is licensed on 88.7".
    /// * A fresh lock OVERWRITES. That is what heals the map after a drive to
    ///   another market; without it, one lock in one town would pin the names for
    ///   good.
    ///
    /// Entries this pass did not see are LEFT ALONE, so a lock that only reaches
    /// half the band does not erase the other half.
    ///
    /// Returns whether anything changed, so the caller can skip a write — this
    /// runs on every fix, and a parked car with a lock produces one every two
    /// seconds.
    pub fn relearn<'a>(&mut self, nearest_first: impl Iterator<Item = (f32, &'a str)>) -> bool {
        let mut taken: BTreeSet<u32> = BTreeSet::new();
        let mut changed = false;
        for (mhz, base) in nearest_first {
            if base.is_empty() {
                continue;
            }
            let Some(k) = key(mhz) else { continue };
            if !taken.insert(k) {
                continue; // a nearer station already claimed this dial
            }
            if self.map.get(&k).map(String::as_str) != Some(base) {
                self.map.insert(k, base.to_string());
                changed = true;
            }
        }
        changed
    }
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Read the file, or return an empty map. Never fails: a radio that cannot read
/// its own cache is a radio that has not learned anything yet, not a broken one.
pub fn load(dir: &Path) -> Callsigns {
    let Ok(text) = fs::read_to_string(path(dir)) else {
        return Callsigns::default();
    };
    from_json(&text).unwrap_or_default()
}

fn from_json(text: &str) -> Option<Callsigns> {
    let Some(Json::Obj(fields)) = json::parse(text) else {
        return None;
    };
    let mut out = Callsigns::default();
    for (k, v) in fields {
        // Keys are the STRING form of the integer key, which is what a JSON
        // object can hold. A key that is not one is a file somebody edited, and
        // the entry is dropped rather than the whole map.
        let Ok(k) = k.parse::<u32>() else { continue };
        let Some(call) = v.as_str().filter(|c| !c.is_empty()) else {
            continue;
        };
        // Round-trip the key through the band check, so a hand-written 9999
        // cannot enter a map that `get` would never consult anyway.
        if (875..=1080).contains(&k) {
            out.map.insert(k, call.to_string());
        }
    }
    Some(out)
}

pub fn to_json(c: &Callsigns) -> String {
    let body: Vec<String> = c
        .map
        .iter()
        .map(|(k, v)| format!("{}:{}", json::quote(&k.to_string()), json::quote(v)))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// Write atomically, and swallow the error.
///
/// Same trade as `prefs::save`: a cache that could not be written is not worth
/// interrupting a drive for, and there is nowhere to report it a driver would
/// see. The temporary-then-rename is what stops a power cut mid-write leaving a
/// half-file that reads as an empty map.
pub fn save(dir: &Path, c: &Callsigns) {
    let _ = try_save(dir, c);
}

fn try_save(dir: &Path, c: &Callsigns) -> std::io::Result<()> {
    use std::io::Write;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(to_json(c).as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("carnyx-callsigns-{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_key_is_tenths_of_a_megahertz_and_only_inside_the_band() {
        assert_eq!(key(88.7), Some(887));
        assert_eq!(key(105.5), Some(1055));
        assert_eq!(key(87.5), Some(875));
        assert_eq!(key(108.0), Some(1080));
        // Outside the band, and the values a corrupt file or a cold tuner
        // produces.
        assert_eq!(key(87.4), None);
        assert_eq!(key(108.1), None);
        assert_eq!(key(0.0), None);
        assert_eq!(key(f32::NAN), None);
        assert_eq!(key(f32::INFINITY), None);
    }

    /// The nearest station on a dial is the one being received. A licensee 90 km
    /// away on the same frequency is not the answer, and taking it would put a
    /// stranger's call sign on the hero — the same failure the position check in
    /// `app::resolve` exists to stop.
    #[test]
    fn the_nearest_station_on_a_dial_wins_it() {
        let mut c = Callsigns::default();
        assert!(c.relearn([(88.7, "WERN"), (88.7, "WFAR"), (105.5, "WMMM")].into_iter()));
        assert_eq!(c.get(88.7), Some("WERN"));
        assert_eq!(c.get(105.5), Some("WMMM"));
        assert_eq!(c.len(), 2);
    }

    /// A fresh lock overwrites, or one lock in one town pins the names for good
    /// and the map never heals after a move.
    #[test]
    fn a_new_lock_replaces_what_the_old_one_learned() {
        let mut c = Callsigns::default();
        c.relearn([(88.7, "WERN")].into_iter());
        assert!(c.relearn([(88.7, "KQED")].into_iter()));
        assert_eq!(c.get(88.7), Some("KQED"));
    }

    /// A lock that only reaches part of the band must not erase the rest.
    #[test]
    fn a_partial_lock_leaves_the_dials_it_did_not_see() {
        let mut c = Callsigns::default();
        c.relearn([(88.7, "WERN"), (105.5, "WMMM")].into_iter());
        c.relearn([(88.7, "WERN")].into_iter());
        assert_eq!(c.get(105.5), Some("WMMM"), "an unseen dial keeps its answer");
    }

    /// Nothing changed means nothing is written. This runs on every fix, and a
    /// car parked with a lock produces one every two seconds.
    #[test]
    fn relearning_the_same_thing_reports_no_change() {
        let mut c = Callsigns::default();
        assert!(c.relearn([(88.7, "WERN")].into_iter()));
        assert!(!c.relearn([(88.7, "WERN")].into_iter()));
        // An empty call sign is not an answer and must not displace one.
        assert!(!c.relearn([(88.7, "")].into_iter()));
        assert_eq!(c.get(88.7), Some("WERN"));
    }

    #[test]
    fn saving_and_loading_round_trips() {
        let d = tmpdir("round-trip");
        let mut c = Callsigns::default();
        c.relearn([(88.7, "WERN"), (105.5, "WMMM"), (94.1, "WJJO")].into_iter());
        save(&d, &c);
        assert_eq!(load(&d), c);
        assert_eq!(load(&d).get(94.1), Some("WJJO"));
    }

    /// A missing file is an empty map, not a failure — it is what every first
    /// run looks like.
    #[test]
    fn a_missing_or_broken_file_is_an_empty_map() {
        let d = tmpdir("missing");
        assert!(load(&d).is_empty());
        fs::write(path(&d), "not json at all").unwrap();
        assert!(load(&d).is_empty());
        fs::write(path(&d), "[1,2,3]").unwrap();
        assert!(load(&d).is_empty());
    }

    /// One unreadable entry costs that entry, not the whole map — the same rule
    /// `prefs::from_json` follows for the same reason.
    #[test]
    fn a_bad_entry_does_not_take_the_good_ones_with_it() {
        let d = tmpdir("partial");
        fs::write(
            path(&d),
            r#"{"887":"WERN","not-a-key":"WHAT","9999":"OUTOFBAND","1055":"","941":"WJJO"}"#,
        )
        .unwrap();
        let c = load(&d);
        assert_eq!(c.get(88.7), Some("WERN"));
        assert_eq!(c.get(94.1), Some("WJJO"));
        assert_eq!(c.get(105.5), None, "an empty call sign is not an answer");
        assert_eq!(c.len(), 2);
    }
}
