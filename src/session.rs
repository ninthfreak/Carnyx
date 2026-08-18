//! What the last run left on the dial, so a restart does not look like a first run.
//!
//! ## The fault this exists for
//!
//! > "When switching to a different app and then switching back, it looked like
//! > the app was starting fresh, having to draw elements and wait for things like
//! > radio text to be decoded."
//!
//! CarFM never had to answer that, because CarFM's process stayed resident:
//! `VibeStreamService` runs in the FOREGROUND even when the built-in NWD tuner is
//! the source and the service is not carrying any audio at all
//! (`startNwdControlSession` → `startForegroundMedia` → `startForeground`,
//! VibeStreamService.kt:726-739). A process with a foreground service is not a
//! candidate for the launcher or the low-memory killer, so switching away and
//! back was a resume with every byte of state still in RAM.
//!
//! CARNYX NOW HAS ONE TOO — `src/android/service.rs` and #67 — and this file did
//! not become redundant when it landed. Two things changed and neither retires
//! the restore:
//!
//! 1. The service only exists in the GRADLE build. cargo-apk packages no Java
//!    and its manifest schema has no `service` field (see the `config_changes`
//!    comment in `Cargo.toml`), so an APK built the default way still has no
//!    service and still takes every restart this file was written for.
//! 2. A foreground service makes a process EXPENSIVE TO KILL, not unkillable.
//!    The unit sleeps on ACC-off and the process goes with it; a driver can stop
//!    the app; the low-memory killer will still take it under real pressure. The
//!    service prevents the restarts it can and this file survives the rest.
//!
//! So the two are a pair, not alternatives, and the `session:` line at the top of
//! the log still says which restart happened.
//!
//! ## What is stored, and what is deliberately not
//!
//! The dial and the decoded RDS. Nothing else a restart loses is worth keeping:
//! the signal level is re-read within five seconds by the level watch, the
//! stereo pilot is re-asserted by the next group, the presets and settings
//! already persist in `prefs.json`, and the diagnostics log is a record OF a run
//! rather than something to carry between them.
//!
//! ## Why it may be refused
//!
//! RadioText describes what is playing NOW. CarFM's own rule is that a station's
//! RDS is disowned after twenty-five seconds of silence (`RDS_STALE_MS`), and
//! restoring a snapshot older than that would put text on the face that CarFM
//! itself would have wiped. So the snapshot is only honoured inside that same
//! window — [`WARM`] IS `app::RDS_STALE`, not a second copy of the number — and
//! only on the dial it was taken from.
//!
//! ## The clock
//!
//! Freshness is wall-clock, because the two runs are different processes and a
//! monotonic `Instant` cannot cross that boundary. A head unit is exactly the
//! device where that is risky: it has no battery-backed RTC worth trusting, so
//! the clock can jump forwards by decades when the network or the GPS finally
//! sets it. Both directions FAIL CLOSED — a snapshot from the future and a
//! snapshot from too far in the past are equally refused, and a refused snapshot
//! is just the blank hero this file was written to avoid, which is where the app
//! already was.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::logos::json::{self, Json};
use crate::rds::RdsState;

/// The file, beside `prefs.json` in the app's own data directory.
pub const FILE: &str = "session.json";

/// How stale a snapshot may be and still be worth putting on the face.
///
/// THE SAME NUMBER as the live expiry, referenced rather than restated: text the
/// running app would have disowned must not come back from disk.
pub const WARM: Duration = crate::app::RDS_STALE;

/// How the previous run ended, as far as it got to find out.
///
/// This is the diagnostic that separates the two ways a face can come back
/// blank, which look identical to a driver and need opposite fixes:
///
/// * `Destroy` — the Activity was torn down and re-created, most likely by a
///   configuration change the manifest does not claim to handle. The process may
///   well have survived; `App::apps_built_here` says which.
/// * `Pause` / `Stop` — the app went to the background in good order and was
///   then killed from outside. Nothing in this repository can prevent that; only
///   a foreground service can.
/// * `LowMemory` — the system said so out loud on the way past.
/// * `Unknown` — the file predates this field, or the run ended without ever
///   reaching a lifecycle callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parting {
    #[default]
    Unknown,
    Pause,
    Stop,
    Destroy,
    LowMemory,
}

impl Parting {
    pub fn name(self) -> &'static str {
        match self {
            Parting::Unknown => "unknown",
            Parting::Pause => "pause",
            Parting::Stop => "stop",
            Parting::Destroy => "destroy",
            Parting::LowMemory => "low-memory",
        }
    }

    fn from_name(name: &str) -> Option<Parting> {
        match name {
            "unknown" => Some(Parting::Unknown),
            "pause" => Some(Parting::Pause),
            "stop" => Some(Parting::Stop),
            "destroy" => Some(Parting::Destroy),
            "low-memory" => Some(Parting::LowMemory),
            _ => None,
        }
    }
}

/// One run's parting snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// Where the dial was.
    pub dial: f32,
    /// Unix seconds at the moment of writing.
    pub saved_at: u64,
    /// How many times this app has started since it was installed. Written by
    /// the run that is ending, read by the run that follows it.
    pub launches: u32,
    pub parting: Parting,
    pub rds: RdsState,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            dial: 0.0,
            saved_at: 0,
            launches: 0,
            parting: Parting::Unknown,
            rds: RdsState::default(),
        }
    }
}

impl Session {
    /// How long ago this was written, or `None` if the answer is not usable —
    /// the clock ran backwards between the two runs, or the file has no stamp.
    pub fn age(&self, now: u64) -> Option<Duration> {
        if self.saved_at == 0 || now < self.saved_at {
            return None;
        }
        Some(Duration::from_secs(now - self.saved_at))
    }

    /// The RDS worth putting straight back on the face, with the age that earned
    /// it. `None` when the snapshot is too old, from a clock that moved, or
    /// simply empty — a station that had decoded nothing has nothing to restore.
    pub fn warm_rds(&self, now: u64) -> Option<(RdsState, Duration)> {
        let age = self.age(now)?;
        if age > WARM {
            return None;
        }
        if !(87.5..=108.0).contains(&self.dial) {
            return None;
        }
        if self.rds == RdsState::default() {
            return None;
        }
        Some((self.rds.clone(), age))
    }
}

/// Wall-clock seconds, or 0 if the platform will not say.
///
/// Zero is treated as "no stamp" by [`Session::age`], so a clock this broken
/// refuses the restore rather than inventing an age for it.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Read the file, or return `None`. Never fails: a run that cannot read what the
/// last one left is a run that starts cold, which is what every first launch
/// does anyway.
pub fn load(dir: &Path) -> Option<Session> {
    let text = fs::read_to_string(path(dir)).ok()?;
    from_json(&text)
}

/// Parse field by field, keeping whatever is readable.
///
/// Same rule as `prefs::from_json`: one unreadable field costs that field. The
/// launch counter in particular must survive a snapshot whose RDS half is
/// nonsense, because it is the diagnostic that explains why the file is being
/// read at all.
pub fn from_json(text: &str) -> Option<Session> {
    let v = json::parse(text)?;
    if !matches!(v, Json::Obj(_)) {
        return None;
    }
    let mut s = Session::default();
    if let Some(d) = v.get("dial").and_then(Json::as_f64) {
        if d.is_finite() {
            s.dial = d as f32;
        }
    }
    if let Some(t) = v.get("savedAt").and_then(Json::as_f64) {
        if t.is_finite() && t >= 0.0 && t <= u64::MAX as f64 {
            s.saved_at = t as u64;
        }
    }
    if let Some(n) = v.get("launches").and_then(Json::as_u32) {
        s.launches = n;
    }
    if let Some(p) = v.get("parting").and_then(Json::as_str).and_then(Parting::from_name) {
        s.parting = p;
    }
    if let Some(r) = v.get("rds") {
        s.rds = rds_from_json(r);
    }
    Some(s)
}

fn rds_from_json(v: &Json) -> RdsState {
    let mut st = RdsState::default();
    if !matches!(v, Json::Obj(_)) {
        return st;
    }
    // PI is sixteen bits and PTY is five. A file with a wider number in either
    // was not written by this app, and truncating it would invent a station.
    if let Some(pi) = v.get("pi").and_then(Json::as_u32) {
        if pi <= u16::MAX as u32 {
            st.pi = Some(pi as u16);
        }
    }
    if let Some(pty) = v.get("pty").and_then(Json::as_u32) {
        if pty <= 31 {
            st.pty = Some(pty as u8);
        }
    }
    let flag = |key: &str| matches!(v.get(key), Some(Json::Bool(true)));
    st.tp = flag("tp");
    // TA IS NOT RESTORED, for the same reason `expire_rds` drops it across a
    // silence: TP is "this station carries traffic announcements", which is
    // station identity, while TA is "one is happening RIGHT NOW", which is not
    // safe to assert about a moment the app was not present for.
    st.ta = false;
    st.ps_scrolling = flag("psScrolling");
    let text = |key: &str| {
        v.get(key)
            .and_then(Json::as_str)
            .map(str::to_string)
            .unwrap_or_default()
    };
    st.ps = text("ps");
    st.rt = text("rt");
    st.rt_artist = text("rtArtist");
    st.rt_title = text("rtTitle");
    st
}

pub fn to_json(s: &Session) -> String {
    format!(
        "{{\"dial\":{:.1},\"savedAt\":{},\"launches\":{},\"parting\":{},\"rds\":{}}}",
        s.dial,
        s.saved_at,
        s.launches,
        json::quote(s.parting.name()),
        rds_to_json(&s.rds)
    )
}

fn rds_to_json(st: &RdsState) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pi) = st.pi {
        parts.push(format!("\"pi\":{pi}"));
    }
    if let Some(pty) = st.pty {
        parts.push(format!("\"pty\":{pty}"));
    }
    parts.push(format!("\"tp\":{}", st.tp));
    parts.push(format!("\"psScrolling\":{}", st.ps_scrolling));
    for (key, value) in [
        ("ps", &st.ps),
        ("rt", &st.rt),
        ("rtArtist", &st.rt_artist),
        ("rtTitle", &st.rt_title),
    ] {
        if !value.is_empty() {
            parts.push(format!("{}:{}", json::quote(key), json::quote(value)));
        }
    }
    format!("{{{}}}", parts.join(","))
}

/// Write atomically, and swallow the error.
///
/// Same trade as `prefs::save` and `callsigns::save`. The atomicity matters more
/// here than anywhere else in the tree: this file is written on the way out —
/// on pause, on stop, on destroy — which on a head unit is frequently the same
/// second the ignition is cut.
pub fn save(dir: &Path, s: &Session) {
    let _ = try_save(dir, s);
}

fn try_save(dir: &Path, s: &Session) -> std::io::Result<()> {
    use std::io::Write;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(to_json(s).as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("carnyx-session-{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn decoded() -> RdsState {
        RdsState {
            pi: Some(0x5A2B),
            pty: Some(10),
            tp: true,
            ta: true,
            ps: "WQLF".into(),
            rt: "The Verve - Bitter Sweet Symphony".into(),
            ps_scrolling: false,
            rt_artist: "The Verve".into(),
            rt_title: "Bitter Sweet Symphony".into(),
        }
    }

    #[test]
    fn saving_and_loading_round_trips() {
        let d = tmpdir("round-trip");
        let s = Session {
            dial: 102.1,
            saved_at: 1_700_000_000,
            launches: 12,
            parting: Parting::Pause,
            rds: decoded(),
        };
        save(&d, &s);
        let back = load(&d).expect("the file we just wrote");
        assert_eq!(back.dial, 102.1);
        assert_eq!(back.saved_at, 1_700_000_000);
        assert_eq!(back.launches, 12);
        assert_eq!(back.parting, Parting::Pause);
        assert_eq!(back.rds.ps, "WQLF");
        assert_eq!(back.rds.rt, "The Verve - Bitter Sweet Symphony");
        assert_eq!(back.rds.rt_artist, "The Verve");
        assert_eq!(back.rds.rt_title, "Bitter Sweet Symphony");
        assert_eq!(back.rds.pi, Some(0x5A2B));
        assert_eq!(back.rds.pty, Some(10));
        assert!(back.rds.tp);
    }

    /// "An announcement is happening right now" is not a fact about a moment the
    /// app was not running for. Same rule the live expiry follows.
    #[test]
    fn a_traffic_announcement_is_never_restored() {
        let d = tmpdir("ta");
        save(&d, &Session { dial: 102.1, saved_at: 1, launches: 1, parting: Parting::Stop, rds: decoded() });
        assert!(!load(&d).unwrap().rds.ta, "TA must not come back from disk");
    }

    #[test]
    fn a_missing_or_broken_file_is_no_session() {
        let d = tmpdir("missing");
        assert_eq!(load(&d), None);
        fs::write(path(&d), "not json at all").unwrap();
        assert_eq!(load(&d), None);
        fs::write(path(&d), "[1,2,3]").unwrap();
        assert_eq!(load(&d), None);
    }

    /// A file from a build that never wrote these fields still has to yield its
    /// launch counter, which is the diagnostic the rest of this exists to feed.
    #[test]
    fn unknown_and_absent_fields_cost_only_themselves() {
        let s = from_json(r#"{"launches":7,"colour":"green"}"#).expect("an object is readable");
        assert_eq!(s.launches, 7);
        assert_eq!(s.parting, Parting::Unknown);
        assert_eq!(s.rds, RdsState::default());
        let s = from_json(r#"{"launches":7,"parting":"sideways","rds":42}"#).unwrap();
        assert_eq!(s.launches, 7);
        assert_eq!(s.parting, Parting::Unknown);
        assert_eq!(s.rds, RdsState::default());
    }

    #[test]
    fn the_warm_window_is_the_live_expiry_window() {
        assert_eq!(WARM, crate::app::RDS_STALE);
    }

    #[test]
    fn a_fresh_snapshot_is_restored_and_a_stale_one_is_not() {
        let now = 1_700_000_000;
        let s = Session {
            dial: 102.1,
            saved_at: now - 4,
            launches: 1,
            parting: Parting::Pause,
            rds: decoded(),
        };
        let (rds, age) = s.warm_rds(now).expect("four seconds ago is warm");
        assert_eq!(rds.ps, "WQLF");
        assert_eq!(age, Duration::from_secs(4));

        let stale = Session { saved_at: now - WARM.as_secs() - 1, ..s.clone() };
        assert_eq!(stale.warm_rds(now), None, "past the expiry window it is refused");

        let edge = Session { saved_at: now - WARM.as_secs(), ..s.clone() };
        assert!(edge.warm_rds(now).is_some(), "exactly at the window it still counts");
    }

    /// A head unit sets its clock late. Both directions must refuse rather than
    /// invent an age — a snapshot from the future is not fresh, it is wrong.
    #[test]
    fn a_clock_that_moved_refuses_the_restore() {
        let s = Session {
            dial: 102.1,
            saved_at: 1_700_000_000,
            launches: 1,
            parting: Parting::Pause,
            rds: decoded(),
        };
        assert_eq!(s.age(1_699_999_999), None, "written in the future");
        assert_eq!(s.warm_rds(1_699_999_999), None);
        assert_eq!(s.warm_rds(0), None, "an unset clock is not an age");
        let unstamped = Session { saved_at: 0, ..s };
        assert_eq!(unstamped.age(1_700_000_000), None);
        assert_eq!(unstamped.warm_rds(1_700_000_000), None);
    }

    /// Nothing decoded is nothing to restore, and a dial outside the band is a
    /// file this app did not write.
    #[test]
    fn an_empty_or_impossible_snapshot_is_refused() {
        let now = 1_700_000_000;
        let blank = Session {
            dial: 102.1,
            saved_at: now,
            launches: 1,
            parting: Parting::Pause,
            rds: RdsState::default(),
        };
        assert_eq!(blank.warm_rds(now), None);
        let off_band = Session { dial: 0.0, rds: decoded(), ..blank.clone() };
        assert_eq!(off_band.warm_rds(now), None);
        let in_band = Session { dial: 88.7, rds: decoded(), ..blank };
        assert!(in_band.warm_rds(now).is_some());
    }
}
