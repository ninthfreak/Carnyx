//! The bundled FCC station table, and everything "stations near me" decides.
//!
//! `assets/db/stations.sqlite` is CarFM's own build of the FCC LMS licence data
//! (`tools/build_station_db/` in that tree, snapshot 2026-07-16) and it comes
//! across byte for byte — 20,733 rows, 2,584,576 bytes, SHA-256
//! `88b6ba2f…6ac`. It is the one part of CarFM the owner named explicitly, and it
//! is public-domain input processed by CarFM's own ETL, so nothing of VibeSDR's
//! travels with it. The `logos` and `logo_wanted` tables the schema still carries
//! are EMPTY in the shipped file (verified, 0 rows) — no third-party artwork
//! rides along, and Carnyx keeps logos outside this database entirely.
//!
//! ## What is in the file, measured rather than assumed
//!
//! Every claim below was read out of the shipped database, because a schema says
//! what a column *may* hold and this code has to be right about what it *does*:
//!
//! * 20,733 rows; `service` is only ever `FM` (10,646), `FX` (8,279) or `FL`
//!   (1,808) — full-power, translator, LPFM.
//! * `station_class` is **NULL in all 20,733 rows**. The LMS field
//!   (`aafq_class_station_code`) is blank in all 339k source rows, so
//!   [`class_bonus`] contributes exactly 0.0 to every score the head unit will
//!   ever compute. It is ported because the column exists and a refresh may fill
//!   it, not because it does anything today; the test below pins that.
//! * `erp_kw` is NULL for 652 rows, otherwise 0.0001 … 1000.0.
//! * `city` and `state` are never NULL in this build, but the schema allows it,
//!   so both are `Option` here — a refresh that drops one must not panic a radio.
//! * `facility_id` is unique across the table: it is the row identity.
//! * `callsign_base == callsign.split('-')[0]` for every row.
//! * `frequency_mhz × 10` and `lat/lon × 1e6` are integral for every row.
//!
//! ## The seam
//!
//! This module does the maths, the SQL and the formatting, and it does no
//! Android. The asset manager is the framework edge and it stays on the far side
//! of [`install`], which takes the asset bytes from a closure the caller
//! supplies — so on the head unit that closure reads through `AAssetManager`, and
//! in `cargo test` it reads the same file off disk and every query below runs
//! against the real 20,733 rows. Nothing here is write-only.
//!
//! ## Provenance
//!
//! The logic is CarFM's, from `src/services/{stationGeo,stationRank,stationDb,
//! stationTypes,stationFinder,stationIdentify}.ts`, `src/services/nwdSignalLevel.ts`
//! and `src/components/carfm/NearbyPicker.tsx`, all of which CarFM wrote — VibeSDR
//! is an SDR client and has no FM station database. CarFM had already ported the
//! geo and ranking halves to Rust itself (`core/stations/`), diffed against the
//! TypeScript by its own differential harness; where that port and this one meet,
//! this one follows it, including the NaN comparator note that is the whole
//! reason `rank_nearby` does not use the obvious sort.
//!
//! ## What Slint gets
//!
//! Finished values. [`NearbyPicker::view`] returns strings that are already
//! formatted, rounded, joined, filtered, sorted and classified, plus the nine
//! properties `ui/nearby.slint` declares — published together, every time,
//! because a stale chip list against a fresh row list is exactly the
//! contradiction `NearbyState` was introduced to prevent.
//!
//! This module does not import the Slint-generated types, so that every test
//! below runs without a window. The bridge is field-for-field and carries no
//! decisions — the types are named to match, so a mismatch is a compile error
//! rather than a wrong list:
//!
//! ```text
//! let v = picker.view(&preset_freqs);
//! ui.set_nearby_state(match v.state {
//!     stations::NearbyState::Loading    => NearbyState::Loading,
//!     stations::NearbyState::List       => NearbyState::List,
//!     stations::NearbyState::NoGps      => NearbyState::NoGps,
//!     stations::NearbyState::NoDatabase => NearbyState::NoDatabase,
//!     stations::NearbyState::NoStations => NearbyState::NoStations,
//! });
//! ui.set_nearby_snapshot(v.snapshot.into());
//! ui.set_nearby_stations(ModelRc::new(VecModel::from(
//!     v.stations.iter().map(|s| NearbyStation {
//!         freq: s.freq.as_str().into(),
//!         call: s.call.as_str().into(),
//!         service: s.service.as_str().into(),
//!         meta: s.meta.as_str().into(),
//!         distance: s.distance.as_str().into(),
//!         signal_pairs: s.signal_pairs,
//!         saved: s.saved,
//!     }).collect::<Vec<_>>())));
//! // …and the remaining six, from the same `v`. All nine or none.
//! ```
//!
//! The `int` a `nearby-tune` / `nearby-save-preset` callback carries indexes the
//! DISPLAYED list; [`NearbyPicker::station_at`] is the only correct way to
//! resolve it, and a filter change between publishing and handling would
//! otherwise silently tune the wrong station.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, Row};

use crate::station::clean_call;

// ── Constants ────────────────────────────────────────────────────────────────

const EARTH_R_KM: f64 = 6371.0088;
const KM_PER_DEG_LAT: f64 = 111.32;
const D2R: f64 = std::f64::consts::PI / 180.0;

/// The picker's radius and cap, from `stationFinder.getNearbyStations`'
/// defaults (`radiusKm = 100`, `limit = 100`). They are the shipping numbers,
/// not a preference.
pub const NEARBY_RADIUS_KM: f64 = 100.0;
pub const NEARBY_LIMIT: usize = 100;

/// Dial-vs-database tolerance, `stationIdentify.ts:29`. The North American FM
/// raster is 0.2 MHz, so half a 0.1 MHz step is loose enough for rounding and far
/// tighter than any channel.
pub const FREQ_EPS: f64 = 0.05;

/// File name of the extracted copy, and of its version stamp's stem.
pub const DB_FILE: &str = "stations.sqlite";

/// Bump when `assets/db/stations.sqlite` is rebuilt, to make every device replace
/// its extracted copy.
///
/// CARNYX'S OWN COUNTER, deliberately starting at 1 rather than inheriting
/// CarFM's `'4'`: different package, different files directory, and a number
/// carried over from another app's migration history would only invite someone to
/// read CarFM's changelog for it. Unlike CarFM a bump carries NOTHING forward —
/// CarFM had to migrate logo rows out of the outgoing copy, and Carnyx keeps
/// logos outside this database, so the replace is a plain one.
pub const DB_ASSET_VERSION: &str = "1";

// ── Geo (CarFM `stationGeo.ts`) ──────────────────────────────────────────────

/// `Math.min`/`Math.max` clamps, NaN included.
///
/// Rust's `f64::min`/`max` DISCARD a NaN operand where JavaScript's PROPAGATE
/// it, so `erp_kw.max(0.0001)` would turn a NaN ERP into a number here and leave
/// it NaN in CarFM. Explicit comparisons reproduce the JavaScript rule: every
/// comparison with NaN is false, so NaN falls through unclamped — which is what
/// makes the NaN case in [`rank_nearby`] reachable rather than theoretical.
fn clamp_min(floor: f64, v: f64) -> f64 {
    if v < floor {
        floor
    } else {
        v
    }
}

fn clamp_max(ceil: f64, v: f64) -> f64 {
    if v > ceil {
        ceil
    } else {
        v
    }
}

/// JavaScript's `Math.round`, which is `floor(x + 0.5)` — round half UP, not
/// half away from zero and not half to even.
///
/// Rust's `f64::round` agrees on every non-negative input, and distances and
/// strengths are non-negative, so this only matters if either ever goes negative.
/// It is spelt out anyway because the two disagree at exactly −0.5 and a silent
/// off-by-one in a signal meter is not a thing anyone would find.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Great-circle distance in km.
///
/// The `min(1, …)` before `asin` is load-bearing: for a near-antipodal pair
/// floating point pushes `sqrt(a)` a hair past 1 and `asin` returns NaN.
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let d_lat = (lat2 - lat1) * D2R;
    let d_lon = (lon2 - lon1) * D2R;
    let a = (d_lat / 2.0).sin().powi(2)
        + (lat1 * D2R).cos() * (lat2 * D2R).cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * EARTH_R_KM * clamp_max(1.0, a.sqrt()).asin()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lon: f64,
    pub max_lon: f64,
}

/// Lat/lon box enclosing a radius around a point, for the indexed SQL prefilter.
///
/// A box is a SUPERSET of the circle; the true-distance trim in [`rank_nearby`]
/// cuts the corners. The 0.01 cosine floor engages above ~89.43° and exists so a
/// polar latitude cannot divide by zero and hand SQLite a box spanning the globe.
///
/// KNOWN HOLE, inherited from CarFM and left deliberately: the box does not wrap
/// the antimeridian, so at Adak (−176.6°) `min_lon` goes below −180 and stations
/// on the far side of the line would be missed. Nothing in the shipped table sits
/// there, so fixing it would be untested code against an absent case.
pub fn bounding_box(lat: f64, lon: f64, radius_km: f64) -> BBox {
    let d_lat = radius_km / KM_PER_DEG_LAT;
    let cos = clamp_min(0.01, (lat * D2R).cos());
    let d_lon = radius_km / (KM_PER_DEG_LAT * cos);
    BBox {
        min_lat: lat - d_lat,
        max_lat: lat + d_lat,
        min_lon: lon - d_lon,
        max_lon: lon + d_lon,
    }
}

/// Small additive bonus by FM station class — a higher class licence reaches
/// further at the same ERP.
///
/// Inert on the shipped data: every row's `station_class` is NULL. See the module
/// header.
pub fn class_bonus(station_class: Option<&str>) -> f64 {
    match station_class.unwrap_or("").to_uppercase().as_str() {
        "C" | "C0" => 6.0,
        "C1" => 4.5,
        "C2" | "B" => 3.0,
        "C3" | "B1" => 1.5,
        // LPFM, translators and unknown carry no bonus — their ERP is already low
        // and says it. "A" is listed in CarFM as an explicit zero, not a default.
        "A" => 0.0,
        _ => 0.0,
    }
}

/// Relative "likely receivability" — HIGHER is better.
///
/// A free-space proxy in dB, `10·log10(ERP) − 20·log10(distance)`, plus the class
/// bonus. It exists to produce ONE ordering: a 100 kW class C at 60 miles beats a
/// 250 W translator at 15 miles. CarFM measured it against 232 settled samples
/// and found it explained 85% of the variance BETWEEN stations and essentially
/// none WITHIN one, which is why CarFM deleted the signal meter that was driven
/// by it and kept the ranking that is not.
///
/// The floors are both about logarithms: an unknown ERP is assumed tiny rather
/// than zero, and standing on top of a transmitter must not evaluate `log10(0)`.
pub fn receivability_score(erp_kw: Option<f64>, station_class: Option<&str>, distance_km: f64) -> f64 {
    let erp = clamp_min(0.0001, erp_kw.unwrap_or(0.05));
    let dist = clamp_min(1.0, distance_km);
    10.0 * erp.log10() - 20.0 * dist.log10() + class_bonus(station_class)
}

// ── Rows (CarFM `stationTypes.ts`) ───────────────────────────────────────────

/// One row of the bundled, read-only station table — the eleven columns and
/// nothing else.
///
/// `genre`, `homepage` and `has_logo` are deliberately ABSENT. CarFM's row type
/// carries them because its query left-joins a `logos` table, but that table
/// ships empty and every one of the three is either permanently NULL or
/// overridden downstream. Carrying a field that is always `None` invites code to
/// be written for a case that cannot occur.
#[derive(Debug, Clone, PartialEq)]
pub struct StationRow {
    /// `WIBA-FM`. Pure `[A-Z0-9-]` across the whole table.
    pub callsign: String,
    /// `WIBA` — `callsign.split('-')[0]`, and the join key a decoded RDS PI
    /// resolves to.
    pub callsign_base: String,
    pub frequency_mhz: f64,
    /// `FM` full-power, `FX` translator, `FL` LPFM.
    ///
    /// A string rather than an enum, matching CarFM's own Rust port: the ETL does
    /// not promise this set, and the one decision that depends on it — whether
    /// the row wears a service badge — is [`StationRow::badge`] rather than a
    /// comparison scattered through the view.
    pub service: String,
    pub station_class: Option<String>,
    pub erp_kw: Option<f64>,
    pub lat: f64,
    pub lon: f64,
    pub city: Option<String>,
    pub state: Option<String>,
    /// Unique. The row identity, and what CarFM keys the list on.
    pub facility_id: i64,
}

impl StationRow {
    /// The badge text a nearby row prints beside the call sign: `FX`, `FL`, or
    /// EMPTY for plain FM.
    ///
    /// Empty is what HIDES the badge — `ui/nearby.slint` tests `service != ""`
    /// and never compares against `"FM"`, so the "is this ordinary" judgement is
    /// made here.
    pub fn badge(&self) -> &str {
        if self.service == "FM" {
            ""
        } else {
            &self.service
        }
    }
}

/// A station that survived the radius trim: the row, its true distance and its
/// receivability rank.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub row: StationRow,
    pub distance_km: f64,
    /// Higher is more likely audible. Not a level.
    pub score: f64,
    /// Programme genre, for the picker's Music/Talk and genre filters.
    ///
    /// ALWAYS `None` on the shipped database, and the filters are therefore
    /// unreachable on a real head unit today. That is not an oversight, it is the
    /// state CarFM is in: `logos.genre` is the only genre store, it is written
    /// from exactly two call sites, and none of the three logo sources feeding
    /// them ever sets a genre — the service that used to (`radioBrowser.ts`) was
    /// deleted. The filter is implemented and tested here because it is written
    /// into `ui/nearby.slint` and is cheap; it is carried on `Ranked` rather than
    /// on `StationRow` because it is not a column of `stations`, and giving a
    /// future `station_genre` table somewhere to land costs nothing.
    pub genre: Option<String>,
}

/// Rank rows already prefiltered to the bounding box, best first.
///
/// The order of the four steps is the behaviour and must not drift:
/// trim by TRUE distance (the box is a superset — this is what actually enforces
/// the radius, not the SQL), score, sort descending, and only THEN cap. Capping
/// before the sort would let the limit hide a better station.
///
/// A NaN SCORE RANKS LAST, EXPLICITLY, and this is not defensive tidiness. The
/// clamps above propagate NaN the way `Math.max` does, and `NaN > radius` is
/// false so a NaN-distance row survives the trim — NaN genuinely reaches this
/// sort. The comparator that looks like JavaScript's rule,
/// `partial_cmp(..).unwrap_or(Equal)`, is not a total order: with NaN, 1, 2 it
/// claims NaN == 1 and NaN == 2 while 1 < 2, and Rust's sort DETECTS that and
/// panics — in release too, because the check lives in `core`. Mapping NaN to
/// `NEG_INFINITY` first gives a real total order and leaves the unrankable rows
/// tied, so the stable sort keeps their input order.
///
/// `sort_by` is stable, as JavaScript's `Array#sort` has been required to be
/// since ES2019, so equal scores keep the order SQLite returned them in and the
/// two implementations agree row for row.
pub fn rank_nearby(
    lat: f64,
    lon: f64,
    radius_km: f64,
    limit: usize,
    rows: Vec<StationRow>,
) -> Vec<Ranked> {
    let mut out: Vec<Ranked> = Vec::new();
    for row in rows {
        let distance_km = haversine_km(lat, lon, row.lat, row.lon);
        if distance_km > radius_km {
            continue;
        }
        let score = receivability_score(row.erp_kw, row.station_class.as_deref(), distance_km);
        out.push(Ranked {
            row,
            distance_km,
            score,
            genre: None,
        });
    }
    let key = |s: f64| if s.is_nan() { f64::NEG_INFINITY } else { s };
    out.sort_by(|a, b| key(b.score).total_cmp(&key(a.score)));
    out.truncate(limit);
    out
}

// ── The database ─────────────────────────────────────────────────────────────

/// Put the bundled database where SQLite can open it, once per version.
///
/// THE ASSET MANAGER STAYS OUT OF THIS FILE. `read_asset` is called only when an
/// extraction is actually needed, so the 2.5 MB read is skipped on every launch
/// after the first; on Android it wraps `AAssetManager`, and in a test it is a
/// `fs::read`. That is the whole seam — everything below this line runs on the
/// host against the real file.
///
/// SQLite is NEVER pointed at the asset itself. cargo-apk ties
/// `disable_aapt_compression` to the build profile (`cargo-apk-0.10.0`,
/// `src/apk.rs:214`), so the asset is stored uncompressed in a debug APK and
/// DEFLATED in a release one. A compressed asset has no file descriptor to map,
/// which is how a design that only ever ran in debug breaks on the first release
/// build. Extracting sidesteps the question entirely.
///
/// The stamp is written LAST, after the rename. A crash in between leaves a
/// complete database with a missing or stale stamp, which the next launch simply
/// redoes; the other order would leave a stamp vouching for a file that is not
/// there.
pub fn install<F>(dir: &Path, read_asset: F) -> io::Result<PathBuf>
where
    F: FnOnce() -> io::Result<Vec<u8>>,
{
    let db_path = dir.join(DB_FILE);
    let stamp_path = dir.join(format!("{DB_FILE}.version"));

    if db_path.exists() {
        if let Ok(stamp) = fs::read_to_string(&stamp_path) {
            if stamp.trim() == DB_ASSET_VERSION {
                return Ok(db_path);
            }
        }
    }

    fs::create_dir_all(dir)?;
    let bytes = read_asset()?;
    let tmp_path = dir.join(format!("{DB_FILE}.tmp"));
    {
        let mut tmp = fs::File::create(&tmp_path)?;
        io::Write::write_all(&mut tmp, &bytes)?;
        // A rename is only atomic with respect to what has reached the disk.
        tmp.sync_all()?;
    }
    fs::rename(&tmp_path, &db_path)?;
    fs::write(&stamp_path, DB_ASSET_VERSION)?;
    Ok(db_path)
}

/// The bundled station table, open for reading.
pub struct StationDb {
    conn: Connection,
}

impl StationDb {
    /// Open the extracted copy READ-ONLY.
    ///
    /// Read-only is not caution, it is what keeps SQLite from wanting to create a
    /// `-journal` beside the file on the first statement; `query_only` then makes
    /// a stray `INSERT` a legible error rather than a corrupted reference table.
    /// These rows are FCC facts and nothing in Carnyx writes them.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "query_only", 1)?;
        Ok(Self { conn })
    }

    /// Stations within `radius_km` of a point, ranked best-receivable first.
    ///
    /// The SQL is an indexed prefilter on `idx_lat`/`idx_lon` and NOTHING ELSE —
    /// the trim, the score, the order and the cap are [`rank_nearby`], where they
    /// can be tested without a database.
    ///
    /// CarFM's version left-joins the `logos` table for `genre`, `homepage` and
    /// `has_logo`. That join is dropped: the table ships empty, `has_logo` is
    /// overridden from the filesystem two layers up in CarFM anyway, and Carnyx's
    /// logos do not live in this file.
    pub fn nearby(
        &self,
        lat: f64,
        lon: f64,
        radius_km: f64,
        limit: usize,
    ) -> rusqlite::Result<Vec<Ranked>> {
        let b = bounding_box(lat, lon, radius_km);
        let mut stmt = self.conn.prepare_cached(
            "SELECT callsign, callsign_base, frequency_mhz, service, station_class, erp_kw,
                    lat, lon, city, state, facility_id
               FROM stations
              WHERE lat BETWEEN ?1 AND ?2 AND lon BETWEEN ?3 AND ?4",
        )?;
        let rows = stmt
            .query_map((b.min_lat, b.max_lat, b.min_lon, b.max_lon), map_row)?
            .collect::<rusqlite::Result<Vec<StationRow>>>()?;
        Ok(rank_nearby(lat, lon, radius_km, limit, rows))
    }

    /// Every row sharing a call-sign base — the lookup a decoded RDS PI lands on.
    pub fn for_callsign_base(&self, base: &str) -> rusqlite::Result<Vec<StationRow>> {
        if base.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT callsign, callsign_base, frequency_mhz, service, station_class, erp_kw,
                    lat, lon, city, state, facility_id
               FROM stations WHERE callsign_base = ?1",
        )?;
        // Uppercased here rather than at the call site: `idx_callsign_base` is a
        // plain index over an all-uppercase column, so a lowercase argument
        // silently matches nothing.
        let rows = stmt
            .query_map([base.to_uppercase()], map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every FULL-POWER station on one dial frequency, nationwide.
    ///
    /// Small (the FM raster puts on the order of a hundred rows on a channel) and
    /// needs no location, which is the point: this is the only station lookup
    /// that still works on a head unit with no GPS fix, and therefore the only
    /// one available exactly when nothing else can contradict a bad PI.
    ///
    /// Translators and LPFM are excluded ON PURPOSE, not for size: the RBDS
    /// call-sign formula does not apply to them (NRSC-G300), so a PI derived from
    /// their call sign is noise, and the only caller matches on a derived PI.
    pub fn at_frequency(&self, mhz: f64) -> rusqlite::Result<Vec<StationRow>> {
        if !mhz.is_finite() || mhz <= 0.0 {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT callsign, callsign_base, frequency_mhz, service, station_class, erp_kw,
                    lat, lon, city, state, facility_id
               FROM stations WHERE service = 'FM' AND ABS(frequency_mhz - ?1) < ?2",
        )?;
        let rows = stmt
            .query_map((mhz, FREQ_EPS), map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The LMS snapshot the table was built from, for the picker's footer.
    ///
    /// The literal `unbuilt` maps to `None`: CarFM's placeholder database ships
    /// that string with zero rows, and "no snapshot" is what tells the picker to
    /// say the dataset was never installed rather than that nothing is in range.
    pub fn snapshot_date(&self) -> rusqlite::Result<Option<String>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'lms_snapshot_date'",
                [],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        Ok(value.filter(|v| v != "unbuilt"))
    }
}

fn map_row(r: &Row<'_>) -> rusqlite::Result<StationRow> {
    Ok(StationRow {
        callsign: r.get(0)?,
        callsign_base: r.get(1)?,
        frequency_mhz: r.get(2)?,
        service: r.get(3)?,
        station_class: r.get(4)?,
        erp_kw: r.get(5)?,
        lat: r.get(6)?,
        lon: r.get(7)?,
        city: r.get(8)?,
        state: r.get(9)?,
        facility_id: r.get(10)?,
    })
}

// ── The picker's view model (CarFM `NearbyPicker.tsx`) ───────────────────────

/// Which of the five mutually exclusive bodies the picker is showing.
///
/// Mirrors the `NearbyState` `ui/nearby.slint` generates, field for field; the
/// bridge in `lib.rs` is a five-arm match. Two types rather than one because this
/// module stays free of the Slint-generated code, which is what lets every test
/// below run without a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearbyState {
    /// Never produced here. The whole query — indexed prefilter, trim, score,
    /// sort, cap and format — measured 0.77 ms at its worst over five widely
    /// separated positions, so there is nothing to wait for. That measurement is
    /// on the HOST; the head unit is a 32-bit ARM and has not been timed. The
    /// variant exists because the Slint enum has it and the screenshot example
    /// renders it.
    Loading,
    List,
    NoGps,
    NoDatabase,
    NoStations,
}

/// The two-level filter's first level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    All,
    Music,
    Talk,
}

impl Bucket {
    pub fn label(self) -> &'static str {
        match self {
            Bucket::All => "All",
            Bucket::Music => "Music",
            Bucket::Talk => "Talk",
        }
    }

    /// Parse the chip label a Slint callback hands back. Anything unrecognised is
    /// `All`, which is the safe reading: an unknown chip must not empty the list.
    pub fn parse(s: &str) -> Bucket {
        match s {
            "Music" => Bucket::Music,
            "Talk" => Bucket::Talk,
            _ => Bucket::All,
        }
    }
}

/// The eleven keywords that make a genre read as spoken-word.
///
/// A plain substring test is byte-identical to CarFM's regex here — it has no
/// anchors and no alternation subtleties, and `"religious talk"` is already
/// subsumed by `"talk"`, so it is carried only to keep the list matching the
/// source it came from.
const TALK_WORDS: [&str; 11] = [
    "news",
    "talk",
    "sports",
    "public",
    "community",
    "college",
    "student",
    "weather",
    "information",
    "personality",
    "religious talk",
];

/// Music unless the genre reads as spoken-word. An UNKNOWN genre is Music, which
/// is why the shipped data — where every genre is unknown — puts every station in
/// the Music bucket and hides the filter bar entirely.
fn is_music(genre: Option<&str>) -> bool {
    // Not trimmed, matching CarFM: a genre of "   " is non-empty, falls through to
    // the keyword test and comes out Music anyway.
    let g = genre.unwrap_or("").to_lowercase();
    if g.is_empty() {
        return true;
    }
    !TALK_WORDS.iter().any(|k| g.contains(k))
}

/// One row of the list, already filtered, sorted, rounded, joined and classified.
/// Field for field the `NearbyStation` struct in `ui/nearby.slint`.
#[derive(Debug, Clone, PartialEq)]
pub struct NearbyRow {
    /// The dial at one decimal, no unit — §6.2 is explicit that this list carries
    /// no "MHz" label.
    pub freq: String,
    /// Call sign with a separated trailing band suffix stripped.
    pub call: String,
    /// `FX` / `FL`, or empty for plain FM.
    pub service: String,
    /// `"<city> · <genre>"`, empty parts dropped. Empty when neither exists,
    /// which drops the row's second line.
    pub meta: String,
    /// `"<km> km"`, already rounded.
    pub distance: String,
    /// Lit arc pairs, 0–4.
    pub signal_pairs: i32,
    pub saved: bool,
}

/// One column of the genre grid, packed column-major.
#[derive(Debug, Clone, PartialEq)]
pub struct GenreColumn {
    pub top: String,
    pub bottom: String,
    /// False for the last column when the genre count is odd.
    pub has_bottom: bool,
}

/// Everything the picker needs, computed together.
///
/// All nine properties, every time. They are derived from one state and are only
/// consistent if they are published as one.
#[derive(Debug, Clone, PartialEq)]
pub struct NearbyView {
    pub state: NearbyState,
    /// The FCC snapshot date, or empty. `ui/nearby.slint` prefixes the fixed
    /// string "FCC data as of " itself and hides the footer when this is empty.
    pub snapshot: String,
    pub stations: Vec<NearbyRow>,
    pub bucket_chips: Vec<String>,
    pub bucket: String,
    pub genre_columns: Vec<GenreColumn>,
    /// The active genre, or empty for none.
    pub genre: String,
    pub show_bucket_bar: bool,
    pub show_genre_bar: bool,
}

/// The picker's whole state: one query's worth of rows, and what the user has
/// filtered it down to.
///
/// RUST OWNS THE SELECTION. `ui/nearby.slint` sends a tapped chip's label and
/// gets a finished list back; the tap-to-clear toggle on the active genre chip is
/// decided here, as is which chip is drawn active.
pub struct NearbyPicker {
    /// False when there was no position fix — the difference between "nothing is
    /// in range" and "we do not know where we are".
    located: bool,
    /// The query result: ranked and ALREADY CAPPED to `NEARBY_LIMIT`.
    ///
    /// The cap is applied before any filter, which is CarFM's behaviour and is
    /// load-bearing: filtering never pulls in a station the cap cut, so drilling
    /// into Music cannot make a row appear that was not in All.
    all: Vec<Ranked>,
    snapshot: Option<String>,
    bucket: Bucket,
    genre: Option<String>,
}

/// The result of applying the current filter once.
struct Filtered<'a> {
    /// The chips the genre band offers, in row order. Empty under `All`.
    genres: Vec<String>,
    /// The selected genre, but only when it is one of `genres`.
    active_genre: Option<String>,
    /// What is on screen, in display order.
    shown: Vec<&'a Ranked>,
}

impl NearbyPicker {
    /// No position fix. The list is empty but the snapshot is still known, so the
    /// footer can still say how old the data is.
    pub fn no_fix(snapshot: Option<String>) -> Self {
        Self {
            located: false,
            all: Vec::new(),
            snapshot,
            bucket: Bucket::All,
            genre: None,
        }
    }

    /// A result for a known position. `rows` is expected to come from
    /// [`StationDb::nearby`] or an equivalent — already ranked, already capped.
    pub fn from_rows(rows: Vec<Ranked>, snapshot: Option<String>) -> Self {
        Self {
            located: true,
            all: rows,
            snapshot,
            bucket: Bucket::All,
            genre: None,
        }
    }

    /// The ranked rows behind the picker, nearest first.
    ///
    /// Exposed for `callsigns::Callsigns::relearn`, which learns what is on each
    /// frequency from the query that has ALREADY run for the picker rather than
    /// running a second one per fix.
    pub fn rows(&self) -> &[Ranked] {
        &self.all
    }

    /// Whether this picker has a position behind it. `false` means no fix, which
    /// is not the same as nothing being in range — and only a located result is
    /// worth learning from.
    pub fn located(&self) -> bool {
        self.located
    }

    /// Run the picker's query at CarFM's shipping radius and cap.
    pub fn query(db: &StationDb, lat: f64, lon: f64) -> rusqlite::Result<Self> {
        let snapshot = db.snapshot_date()?;
        let rows = db.nearby(lat, lon, NEARBY_RADIUS_KM, NEARBY_LIMIT)?;
        Ok(Self::from_rows(rows, snapshot))
    }

    /// Picking a bucket clears the genre — the genre chips belong to the bucket
    /// that was open, and keeping one across the change would filter the new list
    /// by a chip it does not show.
    pub fn pick_bucket(&mut self, label: &str) {
        self.bucket = Bucket::parse(label);
        self.genre = None;
    }

    /// Tapping the ACTIVE genre chip clears it. CarFM toggles on the raw label
    /// without checking membership, and so does this: an unknown label sets a
    /// genre that `active_genre` then declines to honour, which reads as "no
    /// filter" rather than "empty list".
    pub fn pick_genre(&mut self, label: &str) {
        if self.genre.as_deref() == Some(label) {
            self.genre = None;
        } else {
            self.genre = Some(label.to_string());
        }
    }

    /// The icon-only chip in the genre band: back to All, genre cleared.
    pub fn reset_bucket(&mut self) {
        self.pick_bucket(Bucket::All.label());
    }

    /// The rows currently on screen, in display order.
    ///
    /// The index a `tune(i)` or `save-preset(i)` callback carries indexes THIS.
    pub fn shown(&self) -> Vec<&Ranked> {
        self.filter().shown
    }

    /// The station behind a display index, for `tune` and `save-preset`.
    pub fn station_at(&self, index: i32) -> Option<&StationRow> {
        if index < 0 {
            return None;
        }
        self.shown().get(index as usize).map(|r| &r.row)
    }

    /// One pass over the filter state.
    ///
    /// [`shown`](Self::shown) and [`view`](Self::view) both come through here,
    /// deliberately: the list Slint draws and the index space a callback lands in
    /// have to be the same list, and two copies of this arithmetic is exactly how
    /// they would stop being.
    fn filter(&self) -> Filtered<'_> {
        let bucket_rows: Vec<&Ranked> = match self.bucket {
            Bucket::All => self.all.iter().collect(),
            Bucket::Music => self
                .all
                .iter()
                .filter(|r| is_music(r.genre.as_deref()))
                .collect(),
            Bucket::Talk => self
                .all
                .iter()
                .filter(|r| !is_music(r.genre.as_deref()))
                .collect(),
        };

        // The bucket's genres, in ROW ORDER, first-seen wins, de-duplicated —
        // deliberately NOT sorted. CarFM pushes them in list order, so the
        // strongest station's genre leads. `All` offers no genre chips at all.
        let mut genres: Vec<String> = Vec::new();
        if self.bucket != Bucket::All {
            for r in &bucket_rows {
                let g = r.genre.as_deref().unwrap_or("").trim();
                if !g.is_empty() && !genres.iter().any(|s| s == g) {
                    genres.push(g.to_string());
                }
            }
        }

        // A selection is honoured only if it is a chip this bucket offers.
        let active_genre = self
            .genre
            .as_deref()
            .and_then(|g| genres.iter().find(|x| *x == g).cloned());

        let shown: Vec<&Ranked> = match &active_genre {
            Some(g) => bucket_rows
                .iter()
                .copied()
                .filter(|r| r.genre.as_deref().unwrap_or("").trim() == g)
                .collect(),
            None => bucket_rows.clone(),
        };

        Filtered {
            genres,
            active_genre,
            shown,
        }
    }

    /// Every property `ui/nearby.slint` reads, computed from one snapshot of the
    /// state.
    ///
    /// `presets` is the saved preset frequencies in MHz, as `Preset.freq-mhz`
    /// carries them.
    pub fn view(&self, presets: &[f32]) -> NearbyView {
        let Filtered {
            genres,
            active_genre,
            shown,
        } = self.filter();

        // The state is decided by the QUERY, not by the filter: a bucket chip
        // only exists if it has members, so filtering cannot empty the list, and
        // "no stations" must not be reachable by tapping a chip.
        let state = if !self.located {
            NearbyState::NoGps
        } else if self.all.is_empty() {
            if self.snapshot.is_none() {
                NearbyState::NoDatabase
            } else {
                NearbyState::NoStations
            }
        } else {
            NearbyState::List
        };

        // Bucket availability is measured over the WHOLE result, not the current
        // bucket — otherwise drilling into Music would remove the Talk chip that
        // gets you back out.
        let has_music = self.all.iter().any(|r| is_music(r.genre.as_deref()));
        let has_talk = self.all.iter().any(|r| !is_music(r.genre.as_deref()));
        let mut bucket_chips = vec![Bucket::All.label().to_string()];
        if has_music {
            bucket_chips.push(Bucket::Music.label().to_string());
        }
        if has_talk {
            bucket_chips.push(Bucket::Talk.label().to_string());
        }

        let preset_keys: BTreeSet<i32> = presets
            .iter()
            .map(|mhz| preset_key(f64::from(*mhz)))
            .collect();

        let stations = shown
            .iter()
            .map(|r| NearbyRow {
                // f64 formatting rather than `station::format_mhz`, which takes
                // the f32 the face carries: the database value is f64 and there
                // is no reason to round-trip it through a narrower type on the
                // way to a string.
                freq: format!("{:.1}", r.row.frequency_mhz),
                call: clean_call(&r.row.callsign),
                service: r.row.badge().to_string(),
                meta: meta_line(r.row.city.as_deref(), r.genre.as_deref()),
                distance: format!("{} km", js_round(r.distance_km) as i64),
                signal_pairs: discrete_lit_pairs(strength_of(r.score, &shown)),
                saved: preset_keys.contains(&preset_key(r.row.frequency_mhz)),
            })
            .collect();

        NearbyView {
            state,
            snapshot: self.snapshot.clone().unwrap_or_default(),
            stations,
            bucket_chips,
            bucket: self.bucket.label().to_string(),
            genre_columns: genre_columns(&genres),
            genre: active_genre.unwrap_or_default(),
            // Both bars are measured against the QUERY's row count, matching
            // CarFM: with no rows there is nothing to filter.
            show_bucket_bar: !self.all.is_empty()
                && has_music
                && has_talk
                && self.bucket == Bucket::All,
            show_genre_bar: !self.all.is_empty() && self.bucket != Bucket::All,
        }
    }
}

/// `"<city> · <genre>"` with the empty parts dropped, or empty.
///
/// ONE space each side of the interpunct. The design bundle's `.dc.html` uses
/// two; the shipping TSX uses one and behaviour is the TSX's call.
fn meta_line(city: Option<&str>, genre: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = city {
        let c = c.trim();
        if !c.is_empty() {
            parts.push(title_case_city(c));
        }
    }
    if let Some(g) = genre {
        let g = g.trim();
        if !g.is_empty() {
            parts.push(g.to_string());
        }
    }
    parts.join(" · ")
}

/// The FCC ships city names SHOUTING — `MADISON`, `SUN PRAIRIE`, `DE FOREST` —
/// and this is where they stop.
///
/// A DELIBERATE DIVERGENCE FROM CarFM, which prints the column raw. It is here
/// because every Carnyx design render shows "Madison", so shipping
/// the raw column would change the face away from the reference that was signed
/// off; and because casing a string for display is a formatting decision, which
/// belongs on this side of Slint either way. It is one function at one call site
/// and reverting it is deleting the call.
///
/// The pass also NORMALISES: 124 of the 6,582 distinct cities (138 rows) are
/// already title case in the LMS data, so without it the list mixes the two.
///
/// KNOWN MISSES, from reading the actual column: `MC ALLEN` becomes `Mc Allen`
/// (the LMS spells McAllen with a space) and `AFB` becomes `Afb` in four
/// air-base rows. Both are the source's spelling, not a rule this could get
/// right without a name list.
pub fn title_case_city(city: &str) -> String {
    let mut out = String::with_capacity(city.len());
    // Word boundaries are whitespace and the hyphen — NOT the apostrophe, so
    // `LEE'S SUMMIT` reads `Lee's Summit` rather than `Lee'S Summit`.
    let mut at_start = true;
    for ch in city.chars() {
        if at_start {
            out.extend(ch.to_uppercase());
        } else {
            out.extend(ch.to_lowercase());
        }
        at_start = ch.is_whitespace() || ch == '-';
    }
    out
}

/// The integer key a preset frequency hashes to — MHz × 10, rounded.
///
/// f64 ON PURPOSE. In f32 `98.1 * 10.0` is 980.99994 and `105.5 * 10.0` is
/// 1054.9999; the rounding rescues both of those, but the same expression on a
/// Slint `float` is a latent bug waiting for a constant that does not round back.
/// Widening before scaling removes the question.
pub fn preset_key(mhz: f64) -> i32 {
    js_round(mhz * 10.0) as i32
}

/// A station's 1–5 strength: its RANK WITHIN THE DISPLAYED LIST, not a
/// measurement.
///
/// `shown` must be in score order, best first — the ends are read as the range.
/// FILTERING RE-NORMALISES EVERY METER, which is CarFM's behaviour and looks like
/// a bug to anyone who has not been told: the same station shows a different
/// number of arcs under a different filter, because the meter answers "how does
/// this compare with what is on screen".
///
/// The zero-span guard also catches NaN, because CarFM's `((max - min) || 1)`
/// replaces `NaN` and `-0` as well as `0`, and a naive `if span == 0.0` port
/// misses two of the three. A NaN score still reaching the division yields a NaN
/// strength, which Rust's saturating float-to-int cast turns into an unlit meter
/// where JavaScript would render `NaN` arcs — a divergence, and the better of the
/// two behaviours.
fn strength_of(score: f64, shown: &[&Ranked]) -> i32 {
    if shown.len() <= 1 {
        return 5;
    }
    let max = shown[0].score;
    let min = shown[shown.len() - 1].score;
    let mut span = max - min;
    if span == 0.0 || span.is_nan() {
        span = 1.0;
    }
    1 + js_round((score - min) / span * 4.0) as i32
}

/// `nwdSignalLevel.discreteLit`: a 1–5 strength becomes 0–4 lit arc PAIRS.
///
/// The centre dot is the fifth step and is not a pair, which is why this is
/// `n − 1` and not `n`.
fn discrete_lit_pairs(strength: i32) -> i32 {
    let n = strength.clamp(0, 5);
    (n - 1).max(0)
}

/// Pack the genre chips into a two-row grid that flows COLUMN by column: chip 0
/// top-left, chip 1 directly below it, chip 2 at the top of column 2.
///
/// Slint 1.17 has no `grid-auto-flow: column`, and the 2c / 2c+1 arithmetic is a
/// decision, so it happens here.
fn genre_columns(genres: &[String]) -> Vec<GenreColumn> {
    genres
        .chunks(2)
        .map(|pair| GenreColumn {
            top: pair[0].clone(),
            bottom: pair.get(1).cloned().unwrap_or_default(),
            has_bottom: pair.len() > 1,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped asset, read in place. Every database test below runs against
    /// the real 20,733 rows — the file is in the repository, so there is no
    /// reason to test a fixture instead.
    const DB_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/db/stations.sqlite");

    fn db() -> StationDb {
        StationDb::open(Path::new(DB_ASSET)).expect("open the bundled station database")
    }

    // ── Geo ──────────────────────────────────────────────────────────────────

    /// Values produced by running CarFM's own `haversineKm` on the same inputs,
    /// so the head unit measures the distances it always measured.
    #[test]
    fn haversine_matches_the_javascript_to_the_last_bit() {
        assert_eq!(haversine_km(43.07, -89.40, 43.07, -89.40), 0.0);
        assert_eq!(haversine_km(0.0, 0.0, 0.0, 1.0), 111.1950802335329);
        assert_eq!(haversine_km(0.0, 0.0, 1.0, 0.0), 111.1950802335329);
        assert_eq!(haversine_km(43.0731, -89.4012, 41.8781, -87.6298), 196.87329370455393);
    }

    /// Without the `min(1, …)` clamp this returns NaN: floating point pushes
    /// `sqrt(a)` past 1 for an antipodal pair and `asin` gives up.
    #[test]
    fn an_antipodal_pair_does_not_produce_nan() {
        let d = haversine_km(43.07, -89.40, -43.07, 89.40);
        assert!(d.is_finite());
        assert_eq!(d, 19917.639005156874);
    }

    #[test]
    fn the_bounding_box_matches_the_javascript() {
        let b = bounding_box(43.07, -89.40, 100.0);
        assert_eq!(b.min_lat, 42.171688825008985);
        assert_eq!(b.max_lat, 43.968311174991015);
        assert_eq!(b.min_lon, -90.6296874164223);
        assert_eq!(b.max_lon, -88.17031258357771);

        // A degree of latitude is KM_PER_DEG_LAT by definition here, so this one
        // is exact and pins the constant.
        let unit = bounding_box(0.0, 0.0, KM_PER_DEG_LAT);
        assert_eq!((unit.min_lat, unit.max_lat), (-1.0, 1.0));
        assert_eq!((unit.min_lon, unit.max_lon), (-1.0, 1.0));
    }

    /// Without the 0.01 cosine floor this divides by ~0 and hands SQLite a box
    /// spanning the planet.
    #[test]
    fn the_cosine_floor_holds_near_the_pole() {
        let b = bounding_box(89.0, 0.0, 100.0);
        assert_eq!(b.min_lon, -51.47205219057663);
        assert_eq!(b.max_lon, 51.47205219057663);
        let pole = bounding_box(90.0, 0.0, 100.0);
        assert!(pole.max_lon.is_finite() && pole.max_lon < 100.0);
    }

    #[test]
    fn class_bonus_is_case_insensitive_and_defaults_to_zero() {
        assert_eq!(class_bonus(Some("C")), 6.0);
        assert_eq!(class_bonus(Some("c0")), 6.0);
        assert_eq!(class_bonus(Some("C1")), 4.5);
        assert_eq!(class_bonus(Some("B")), 3.0);
        assert_eq!(class_bonus(Some("B1")), 1.5);
        assert_eq!(class_bonus(Some("A")), 0.0);
        assert_eq!(class_bonus(Some("LP")), 0.0);
        assert_eq!(class_bonus(None), 0.0);
    }

    #[test]
    fn receivability_floors_both_logarithms() {
        // 100 kW is +20 dB; the distance floors to 1 km, so it contributes 0.
        assert_eq!(receivability_score(Some(100.0), None, 0.5), 20.0);
        assert_eq!(
            receivability_score(Some(100.0), None, 0.0),
            receivability_score(Some(100.0), None, 1.0)
        );
        // Unknown ERP is assumed to be 0.05 kW, not zero.
        assert_eq!(receivability_score(None, None, 10.0), -33.01029995663981);
        // A zero ERP floors to 0.0001 kW rather than evaluating log10(0).
        assert_eq!(receivability_score(Some(0.0), None, 10.0), -60.0);
    }

    /// The single ordering the score exists to produce.
    #[test]
    fn a_big_distant_station_outranks_a_tiny_near_one() {
        let big = receivability_score(Some(100.0), Some("C"), 96.0);
        let small = receivability_score(Some(0.25), None, 24.0);
        assert!(big > small, "big {big} small {small}");
    }

    // ── Ranking ──────────────────────────────────────────────────────────────

    fn row(call: &str, lat: f64, lon: f64, erp: Option<f64>, class: Option<&str>) -> StationRow {
        StationRow {
            callsign: call.to_string(),
            callsign_base: call.split('-').next().unwrap_or(call).to_string(),
            frequency_mhz: 101.5,
            service: "FM".to_string(),
            station_class: class.map(str::to_string),
            erp_kw: erp,
            lat,
            lon,
            city: Some("MADISON".to_string()),
            state: Some("WI".to_string()),
            facility_id: 0,
        }
    }

    /// The box is a superset of the circle, so this trim is what enforces the
    /// radius — the SQL cannot.
    #[test]
    fn a_row_in_the_box_but_outside_the_circle_is_trimmed() {
        let rows = vec![
            row("WNEA", 43.0731, -89.4012, Some(10.0), None),
            row("WFAR", 43.9, -90.6, Some(10.0), None), // box corner, ~120 km out
        ];
        let out = rank_nearby(43.0731, -89.4012, 100.0, 50, rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].row.callsign, "WNEA");
    }

    #[test]
    fn the_cap_is_applied_after_the_sort_not_before() {
        // Input order puts the weaker station first; a cap applied before the
        // sort would keep it and hide the better one.
        let rows = vec![
            row("WTNY", 43.08, -89.40, Some(0.25), None),
            row("WBIG", 43.20, -89.40, Some(100.0), Some("C")),
        ];
        let out = rank_nearby(43.0731, -89.4012, 200.0, 1, rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].row.callsign, "WBIG");
    }

    #[test]
    fn equal_scores_keep_the_order_sql_returned_them_in() {
        // Two rows at the same distance and ERP tie exactly; a stable sort must
        // leave them as they arrived.
        let rows = vec![
            row("WAAA", 43.10, -89.40, Some(10.0), None),
            row("WBBB", 43.10, -89.40, Some(10.0), None),
        ];
        let out = rank_nearby(43.0731, -89.4012, 100.0, 50, rows);
        assert_eq!(out[0].row.callsign, "WAAA");
        assert_eq!(out[1].row.callsign, "WBBB");
    }

    /// REGRESSION — a NaN score used to abort the process.
    ///
    /// `partial_cmp(..).unwrap_or(Equal)` looks like JavaScript's NaN rule but is
    /// not a total order, and Rust's sort detects that and panics, in release as
    /// well as debug. It does not trip on tidy input, so the NaN rows here are
    /// scattered irregularly across enough of them to reach the detecting path.
    #[test]
    fn a_nan_score_sinks_instead_of_aborting() {
        let nan_at = [1usize, 2, 5, 8, 13, 21, 34, 55, 60, 61, 62, 70, 71, 90];
        let rows: Vec<StationRow> = (0..100)
            .map(|i| {
                let erp = if nan_at.contains(&i) {
                    Some(f64::NAN)
                } else {
                    Some(1.0 + (i * 37 % 97) as f64)
                };
                row(
                    "WNAN",
                    43.0 + (i % 11) as f64 * 0.01,
                    -89.4 - (i % 7) as f64 * 0.01,
                    erp,
                    None,
                )
            })
            .collect();
        let out = rank_nearby(43.0731, -89.4012, 500.0, 1000, rows);
        assert_eq!(out.len(), 100, "every row is inside the radius");
        let first_nan = out.iter().position(|s| s.score.is_nan());
        assert_eq!(first_nan, Some(86), "the 14 NaN rows sink to the tail");
        assert!(out[..86].windows(2).all(|w| w[0].score >= w[1].score));
    }

    // ── The shipped database ─────────────────────────────────────────────────

    /// What the file actually contains, so a rebuilt asset that changes shape
    /// fails here rather than in a car.
    #[test]
    fn the_bundled_table_is_the_2026_07_16_fcc_snapshot() {
        let db = db();
        assert_eq!(db.snapshot_date().unwrap().as_deref(), Some("2026-07-16"));
        let n: i64 = db
            .conn
            .query_row("SELECT count(*) FROM stations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 20_733);
        let fm: i64 = db
            .conn
            .query_row("SELECT count(*) FROM stations WHERE service='FM'", [], |r| r.get(0))
            .unwrap();
        let fx: i64 = db
            .conn
            .query_row("SELECT count(*) FROM stations WHERE service='FX'", [], |r| r.get(0))
            .unwrap();
        let fl: i64 = db
            .conn
            .query_row("SELECT count(*) FROM stations WHERE service='FL'", [], |r| r.get(0))
            .unwrap();
        assert_eq!((fm, fx, fl), (10_646, 8_279, 1_808));
        assert_eq!(fm + fx + fl, n, "no fourth service value");
    }

    /// `class_bonus` is provably inert on this data. A ranking test built only on
    /// class bonuses would prove nothing about the head unit, so this is the test
    /// that says why.
    #[test]
    fn every_shipped_row_has_a_null_station_class() {
        let with_class: i64 = db()
            .conn
            .query_row(
                "SELECT count(*) FROM stations WHERE station_class IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(with_class, 0);
    }

    /// `logos` ships EMPTY, which is what makes the genre filter unreachable on
    /// real data — and is also why no third-party artwork travels in this file.
    #[test]
    fn the_logo_tables_ship_empty() {
        let db = db();
        let logos: i64 = db
            .conn
            .query_row("SELECT count(*) FROM logos", [], |r| r.get(0))
            .unwrap();
        let wanted: i64 = db
            .conn
            .query_row("SELECT count(*) FROM logo_wanted", [], |r| r.get(0))
            .unwrap();
        assert_eq!((logos, wanted), (0, 0));
    }

    /// `callsign_base` is derivable, and the plate label relies on it being the
    /// part before the hyphen.
    #[test]
    fn callsign_base_is_the_callsign_up_to_the_hyphen() {
        let db = db();
        let mismatched: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM stations
                  WHERE callsign_base != substr(callsign, 1,
                        CASE WHEN instr(callsign,'-') > 0
                             THEN instr(callsign,'-') - 1
                             ELSE length(callsign) END)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mismatched, 0);
    }

    /// The whole query, pinned against the real table at a real position. Every
    /// number here was produced by running CarFM's formulae over the same file.
    #[test]
    fn the_madison_query_returns_what_carfm_returned() {
        let db = db();
        let (lat, lon) = (43.07, -89.40);

        // The prefilter is a superset; 156 rows are in the box, 120 are inside
        // the circle, and the cap keeps 100.
        let b = bounding_box(lat, lon, NEARBY_RADIUS_KM);
        let in_box: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM stations
                  WHERE lat BETWEEN ?1 AND ?2 AND lon BETWEEN ?3 AND ?4",
                (b.min_lat, b.max_lat, b.min_lon, b.max_lon),
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(in_box, 156);
        assert_eq!(db.nearby(lat, lon, NEARBY_RADIUS_KM, 1000).unwrap().len(), 120);

        let out = db.nearby(lat, lon, NEARBY_RADIUS_KM, NEARBY_LIMIT).unwrap();
        assert_eq!(out.len(), NEARBY_LIMIT);
        let first: Vec<&str> = out.iter().take(8).map(|r| r.row.callsign.as_str()).collect();
        assert_eq!(
            first,
            [
                "WNWC-FM", "WMGN", "WIBA-FM", "WZEE", "WERN", "WTLX", "WWQM-FM", "WMUU-LP"
            ]
        );

        // Individual rows, distance and score, exactly as the TypeScript produced
        // them.
        let by_call = |c: &str| out.iter().find(|r| r.row.callsign == c).unwrap().clone();
        let wnwc = by_call("WNWC-FM");
        assert_eq!(wnwc.row.facility_id, 49781);
        assert_eq!(wnwc.row.erp_kw, Some(50.0));
        assert_eq!(wnwc.distance_km, 9.51825462079525);
        assert_eq!(wnwc.score, -2.5814463233799536);
        let wmuu = by_call("WMUU-LP");
        assert_eq!(wmuu.row.service, "FL");
        assert_eq!(wmuu.distance_km, 1.1680302299219594);
        assert_eq!(wmuu.score, -11.349081658924897);
        let w300 = by_call("W300BM");
        assert_eq!(w300.row.service, "FX");
        assert_eq!(w300.distance_km, 3.214129385164777);
        assert_eq!(w300.score, -16.16186702039549);

        // The weakest row the cap kept.
        let last = out.last().unwrap();
        assert_eq!(last.row.callsign, "W278DE");
        assert_eq!(last.distance_km, 92.65265843896937);
        assert_eq!(last.score, -45.357757610082494);
    }

    /// The frequency lookup is FULL-POWER ONLY, and that is a rule about RBDS,
    /// not an optimisation.
    #[test]
    fn the_frequency_lookup_excludes_translators_and_lpfm() {
        let db = db();
        let rows = db.at_frequency(101.5).unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.service == "FM"));
        assert!(rows.iter().all(|r| (r.frequency_mhz - 101.5).abs() < FREQ_EPS));
        // 0.05 MHz is half a raster step: the next channel down is 101.3 and must
        // not appear.
        assert!(rows.iter().all(|r| r.frequency_mhz == 101.5));
        // Nonsense in, empty out — never a query.
        assert!(db.at_frequency(0.0).unwrap().is_empty());
        assert!(db.at_frequency(f64::NAN).unwrap().is_empty());
    }

    #[test]
    fn the_callsign_lookup_uppercases_its_argument() {
        let db = db();
        let upper = db.for_callsign_base("WIBA").unwrap();
        assert!(!upper.is_empty());
        assert_eq!(db.for_callsign_base("wiba").unwrap(), upper);
        assert!(db.for_callsign_base("").unwrap().is_empty());
        assert!(db.for_callsign_base("ZZZZ9").unwrap().is_empty());
    }

    // ── Installing the asset ─────────────────────────────────────────────────

    #[test]
    fn install_extracts_once_and_replaces_on_a_version_bump() {
        let dir = std::env::temp_dir().join(format!("carnyx-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        // Counts calls of the ASSET CLOSURE, not of `install` — the point of the
        // closure is that a launch with a current copy never touches the 2.5 MB
        // asset at all.
        let reads = std::cell::Cell::new(0u32);
        let extract = |dir: &Path| {
            install(dir, || {
                reads.set(reads.get() + 1);
                fs::read(DB_ASSET)
            })
            .unwrap()
        };

        let path = extract(&dir);
        assert!(path.exists());
        assert_eq!(fs::metadata(&path).unwrap().len(), 2_584_576);
        assert_eq!(reads.get(), 1);
        // The copy is openable and complete, not merely present.
        assert_eq!(
            StationDb::open(&path).unwrap().snapshot_date().unwrap().as_deref(),
            Some("2026-07-16")
        );

        // Second launch: the stamp matches, so the asset is never read.
        extract(&dir);
        assert_eq!(reads.get(), 1, "a current copy is not re-extracted");
        let stamp = dir.join(format!("{DB_FILE}.version"));
        assert_eq!(fs::read_to_string(&stamp).unwrap(), DB_ASSET_VERSION);

        // A stale stamp forces a fresh extraction, which is what a rebuilt asset
        // relies on.
        fs::write(&stamp, "0").unwrap();
        fs::write(&path, b"not a database").unwrap();
        extract(&dir);
        assert_eq!(fs::metadata(&path).unwrap().len(), 2_584_576);
        assert_eq!(reads.get(), 2);

        // No stamp at all is the same as a stale one — the case a crash between
        // the rename and the stamp leaves behind.
        fs::remove_file(&stamp).unwrap();
        fs::write(&path, b"not a database").unwrap();
        extract(&dir);
        assert_eq!(fs::metadata(&path).unwrap().len(), 2_584_576);
        assert_eq!(reads.get(), 3);
        assert!(!dir.join(format!("{DB_FILE}.tmp")).exists(), "no half-written copy is left");

        fs::remove_dir_all(&dir).unwrap();
    }

    // ── The picker ───────────────────────────────────────────────────────────

    fn ranked(call: &str, mhz: f64, service: &str, city: &str, genre: Option<&str>, km: f64, score: f64) -> Ranked {
        Ranked {
            row: StationRow {
                callsign: call.to_string(),
                callsign_base: call.split('-').next().unwrap_or(call).to_string(),
                frequency_mhz: mhz,
                service: service.to_string(),
                station_class: None,
                erp_kw: Some(10.0),
                lat: 43.07,
                lon: -89.40,
                city: Some(city.to_string()),
                state: Some("WI".to_string()),
                facility_id: 0,
            },
            distance_km: km,
            score,
            genre: genre.map(str::to_string),
        }
    }

    /// A mixed list: three Music, two Talk, one with no genre at all.
    fn mixed() -> Vec<Ranked> {
        vec![
            ranked("WMGN", 98.1, "FM", "MADISON", Some("Adult Contemporary"), 4.0, -2.0),
            ranked("WIBA-FM", 101.5, "FM", "MADISON", Some("News/Talk"), 8.0, -6.0),
            ranked("WQLF-FM", 102.1, "FX", "MIDDLETON", Some("Classic Hits"), 13.4, -10.0),
            ranked("WERN", 88.7, "FM", "MADISON", Some("Public Radio"), 11.0, -14.0),
            ranked("WWHG", 105.5, "FL", "MONONA", Some("Adult Contemporary"), 18.6, -18.0),
            ranked("WHHI", 90.9, "FX", "HIGHLAND", None, 46.4, -22.0),
        ]
    }

    #[test]
    fn a_row_arrives_at_slint_already_finished() {
        let p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        let v = p.view(&[98.1, 105.5]);
        assert_eq!(v.state, NearbyState::List);
        assert_eq!(v.snapshot, "2026-07-16");

        let first = &v.stations[0];
        assert_eq!(first.freq, "98.1");
        assert_eq!(first.call, "WMGN");
        assert_eq!(first.service, "", "plain FM wears no badge");
        assert_eq!(first.meta, "Madison · Adult Contemporary");
        assert_eq!(first.distance, "4 km");
        assert!(first.saved);

        // A translator wears its service; the hyphenated suffix comes off the
        // call sign; the preset it is not.
        let wqlf = &v.stations[2];
        assert_eq!(wqlf.call, "WQLF");
        assert_eq!(wqlf.service, "FX");
        assert_eq!(wqlf.distance, "13 km");
        assert!(!wqlf.saved);
        assert_eq!(v.stations[4].service, "FL");

        // No genre leaves the city alone on the line.
        assert_eq!(v.stations[5].meta, "Highland");
    }

    /// The meter is a rank within what is on screen, so the strongest row always
    /// shows four pairs and the weakest none.
    #[test]
    fn the_signal_meter_spans_the_displayed_list() {
        let p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        let pairs: Vec<i32> = p.view(&[]).stations.iter().map(|s| s.signal_pairs).collect();
        assert_eq!(pairs, [4, 3, 2, 2, 1, 0]);
    }

    /// FILTERING RE-NORMALISES. The same station shows a different meter under a
    /// different filter — CarFM's behaviour, and the reason Rust must recompute
    /// this on every filter change rather than caching it on the row.
    #[test]
    fn filtering_re_normalises_every_meter() {
        let mut p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        assert_eq!(p.view(&[]).stations[3].signal_pairs, 2); // WERN among all six
        p.pick_bucket("Talk");
        let talk = p.view(&[]);
        assert_eq!(talk.stations.len(), 2);
        assert_eq!(talk.stations[0].call, "WIBA"); // News/Talk
        assert_eq!(talk.stations[1].call, "WERN"); // Public Radio
        // Two rows now span the whole scale: best 4 pairs, worst 0.
        assert_eq!(talk.stations[0].signal_pairs, 4);
        assert_eq!(talk.stations[1].signal_pairs, 0);
    }

    /// A single row cannot be ranked against anything, so it reads full.
    #[test]
    fn one_row_alone_shows_a_full_meter() {
        let p = NearbyPicker::from_rows(vec![mixed().remove(0)], Some("2026-07-16".into()));
        assert_eq!(p.view(&[]).stations[0].signal_pairs, 4);
    }

    #[test]
    fn the_bucket_bar_appears_only_when_both_buckets_exist_and_all_is_active() {
        let mut p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        let v = p.view(&[]);
        assert_eq!(v.bucket_chips, ["All", "Music", "Talk"]);
        assert!(v.show_bucket_bar);
        assert!(!v.show_genre_bar, "the genre bar belongs to a drilled-in bucket");

        // Drilling in swaps one bar for the other.
        p.pick_bucket("Music");
        let m = p.view(&[]);
        assert!(!m.show_bucket_bar);
        assert!(m.show_genre_bar);
        // Music is four rows: the no-genre row counts as Music.
        assert_eq!(m.stations.len(), 4);
        assert_eq!(m.bucket, "Music");
        // Both chips stay offered so the way back out is still on screen.
        assert_eq!(m.bucket_chips, ["All", "Music", "Talk"]);
    }

    /// The shipped database has no genres, so every row is Music, `has_talk` is
    /// false and BOTH FILTER BARS ARE UNREACHABLE. The machinery above is built
    /// and tested; on real data it is never seen.
    #[test]
    fn without_genres_the_filter_never_appears() {
        let rows: Vec<Ranked> = mixed()
            .into_iter()
            .map(|mut r| {
                r.genre = None;
                r
            })
            .collect();
        let v = NearbyPicker::from_rows(rows, Some("2026-07-16".into())).view(&[]);
        assert_eq!(v.bucket_chips, ["All", "Music"]);
        assert!(!v.show_bucket_bar);
        assert!(!v.show_genre_bar);
        assert!(v.genre_columns.is_empty());
        assert_eq!(v.stations[0].meta, "Madison", "no genre leaves the city alone");
    }

    #[test]
    fn genres_are_listed_in_row_order_and_packed_column_major() {
        let mut p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        p.pick_bucket("Music");
        let v = p.view(&[]);
        // Row order, first-seen wins, NOT sorted — "Adult Contemporary" leads
        // because the strongest station carries it, and its second appearance is
        // dropped. The no-genre row contributes no chip.
        assert_eq!(v.genre_columns.len(), 1);
        assert_eq!(v.genre_columns[0].top, "Adult Contemporary");
        assert_eq!(v.genre_columns[0].bottom, "Classic Hits");
        assert!(v.genre_columns[0].has_bottom);
    }

    #[test]
    fn an_odd_genre_count_leaves_the_last_column_one_chip_tall() {
        let cols = genre_columns(&["A".into(), "B".into(), "C".into()]);
        assert_eq!(cols.len(), 2);
        assert_eq!((cols[0].top.as_str(), cols[0].bottom.as_str()), ("A", "B"));
        assert!(cols[0].has_bottom);
        assert_eq!(cols[1].top, "C");
        assert_eq!(cols[1].bottom, "");
        assert!(!cols[1].has_bottom);
    }

    #[test]
    fn tapping_the_active_genre_clears_it() {
        let mut p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        p.pick_bucket("Music");
        p.pick_genre("Classic Hits");
        let v = p.view(&[]);
        assert_eq!(v.genre, "Classic Hits");
        assert_eq!(v.stations.len(), 1);
        assert_eq!(v.stations[0].call, "WQLF");

        p.pick_genre("Classic Hits");
        assert_eq!(p.view(&[]).genre, "");
        assert_eq!(p.view(&[]).stations.len(), 4);
    }

    /// A genre the current bucket does not offer is not honoured — it reads as
    /// "no filter", never as "empty list".
    #[test]
    fn a_genre_from_another_bucket_is_ignored_rather_than_emptying_the_list() {
        let mut p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        p.pick_bucket("Music");
        p.pick_genre("News/Talk"); // a Talk genre, while Music is open
        let v = p.view(&[]);
        assert_eq!(v.genre, "");
        assert_eq!(v.stations.len(), 4);
    }

    #[test]
    fn changing_bucket_clears_the_genre() {
        let mut p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        p.pick_bucket("Music");
        p.pick_genre("Classic Hits");
        p.pick_bucket("Talk");
        assert_eq!(p.view(&[]).genre, "");
        p.pick_bucket("Music");
        p.pick_genre("Classic Hits");
        p.reset_bucket();
        let v = p.view(&[]);
        assert_eq!((v.bucket.as_str(), v.genre.as_str()), ("All", ""));
    }

    /// `tune(i)` and `save-preset(i)` index the DISPLAYED list, so the index
    /// space has to follow the filter.
    #[test]
    fn a_callback_index_follows_the_displayed_list() {
        let mut p = NearbyPicker::from_rows(mixed(), Some("2026-07-16".into()));
        assert_eq!(p.station_at(1).unwrap().callsign, "WIBA-FM");
        p.pick_bucket("Talk");
        assert_eq!(p.station_at(1).unwrap().callsign, "WERN");
        assert!(p.station_at(2).is_none());
        assert!(p.station_at(-1).is_none());
        // What the view drew and what an index resolves to are the same list.
        let v = p.view(&[]);
        assert_eq!(v.stations.len(), p.shown().len());
    }

    #[test]
    fn the_three_empty_states_are_told_apart() {
        // No fix at all.
        let v = NearbyPicker::no_fix(Some("2026-07-16".into())).view(&[]);
        assert_eq!(v.state, NearbyState::NoGps);
        assert_eq!(v.snapshot, "2026-07-16", "the footer still knows the date");
        assert!(v.stations.is_empty());

        // A fix, a database, nothing in range.
        let v = NearbyPicker::from_rows(Vec::new(), Some("2026-07-16".into())).view(&[]);
        assert_eq!(v.state, NearbyState::NoStations);

        // A fix, but the dataset was never built: no snapshot date.
        let v = NearbyPicker::from_rows(Vec::new(), None).view(&[]);
        assert_eq!(v.state, NearbyState::NoDatabase);
        assert_eq!(v.snapshot, "");
    }

    /// The picker over the real table at a real position — the path the head unit
    /// takes, end to end, with no fixtures anywhere in it.
    #[test]
    fn the_picker_runs_against_the_shipped_database() {
        let db = db();
        let p = NearbyPicker::query(&db, 43.07, -89.40).unwrap();
        let v = p.view(&[98.1, 101.5]);

        assert_eq!(v.state, NearbyState::List);
        assert_eq!(v.snapshot, "2026-07-16");
        assert_eq!(v.stations.len(), NEARBY_LIMIT);
        assert_eq!(v.stations[0].freq, "102.5");
        assert_eq!(v.stations[0].call, "WNWC");
        assert_eq!(v.stations[0].meta, "Madison");
        assert_eq!(v.stations[0].distance, "10 km");
        assert_eq!(v.stations[0].signal_pairs, 4);
        assert_eq!(v.stations.last().unwrap().call, "W278DE");
        assert_eq!(v.stations.last().unwrap().distance, "93 km");
        assert_eq!(v.stations.last().unwrap().signal_pairs, 0);

        // Both presets are on the list and both are starred, and nothing else is.
        assert_eq!(v.stations.iter().filter(|s| s.saved).count(), 2);
        assert!(v.stations.iter().any(|s| s.freq == "98.1" && s.saved));
        assert!(v.stations.iter().any(|s| s.freq == "101.5" && s.saved));

        // The meter histogram over the hundred kept rows, as CarFM's produced it.
        let mut hist = [0usize; 5];
        for s in &v.stations {
            hist[s.signal_pairs as usize] += 1;
        }
        assert_eq!(hist, [22, 31, 29, 13, 5]);

        // No genres in the shipped data, so the filter is not offered.
        assert!(!v.show_bucket_bar);
        assert!(!v.show_genre_bar);
        assert_eq!(v.bucket_chips, ["All", "Music"]);
    }

    // ── Formatting ───────────────────────────────────────────────────────────

    #[test]
    fn a_preset_key_survives_the_float_that_does_not_round_trip() {
        assert_eq!(preset_key(98.1), 981);
        assert_eq!(preset_key(105.5), 1055);
        assert_eq!(preset_key(87.9), 879);
        assert_eq!(preset_key(107.9), 1079);
        // The two frequencies that made this f64 rather than f32: in f32 the
        // products are 980.99994 and 1054.9999.
        assert_eq!(preset_key(f64::from(98.1f32)), 981);
        assert_eq!(preset_key(f64::from(105.5f32)), 1055);
    }

    #[test]
    fn title_case_city_handles_the_shapes_the_fcc_actually_ships() {
        assert_eq!(title_case_city("MADISON"), "Madison");
        assert_eq!(title_case_city("SUN PRAIRIE"), "Sun Prairie");
        assert_eq!(title_case_city("DE FOREST"), "De Forest");
        assert_eq!(title_case_city("TRUTH OR CONSEQUENCES"), "Truth Or Consequences");
        assert_eq!(title_case_city("MILTON-FREEWATER"), "Milton-Freewater");
        // The apostrophe is not a word boundary.
        assert_eq!(title_case_city("LEE'S SUMMIT"), "Lee's Summit");
        assert_eq!(title_case_city("ST. LOUIS"), "St. Louis");
        assert_eq!(title_case_city("ANCHOR POINT, ETC."), "Anchor Point, Etc.");
        // 138 rows already arrive title-cased and must come out unchanged.
        assert_eq!(title_case_city("Elmira Heights - Horseheads"), "Elmira Heights - Horseheads");
        assert_eq!(title_case_city("Anchorage"), "Anchorage");
        // The known misses, recorded rather than papered over.
        assert_eq!(title_case_city("MC ALLEN"), "Mc Allen");
    }

    #[test]
    fn the_meta_line_drops_what_is_missing() {
        assert_eq!(meta_line(Some("MADISON"), Some("Classic Hits")), "Madison · Classic Hits");
        assert_eq!(meta_line(Some("MADISON"), None), "Madison");
        assert_eq!(meta_line(None, Some("Classic Hits")), "Classic Hits");
        // Empty rather than a space: `ui/nearby.slint` tests `meta != ""` and
        // drops the whole line, where the TSX emitted " " to hold the height.
        assert_eq!(meta_line(None, None), "");
        assert_eq!(meta_line(Some("  "), Some(" ")), "");
    }

    #[test]
    fn talk_is_decided_by_the_eleven_keywords() {
        assert!(!is_music(Some("News/Talk")));
        assert!(!is_music(Some("PUBLIC RADIO")), "the test is case-insensitive");
        assert!(!is_music(Some("Religious Talk")));
        assert!(!is_music(Some("College")));
        assert!(is_music(Some("Classic Hits")));
        assert!(is_music(Some("Adult Contemporary")));
        // Unknown is Music, which is why the shipped data is entirely Music.
        assert!(is_music(None));
        assert!(is_music(Some("")));
    }

    #[test]
    fn a_strength_becomes_lit_pairs_the_way_the_glyph_expects() {
        assert_eq!(discrete_lit_pairs(5), 4);
        assert_eq!(discrete_lit_pairs(1), 0);
        assert_eq!(discrete_lit_pairs(0), 0);
        // Out of range in either direction is clamped, never negative.
        assert_eq!(discrete_lit_pairs(9), 4);
        assert_eq!(discrete_lit_pairs(-3), 0);
    }
}
