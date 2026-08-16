//! THE FAKES. Everything in this module is a stand-in for a subsystem that
//! cannot exist in a container, and every one of them is named so that a reader
//! who lands here by accident knows immediately that nothing behind it is real.
//!
//! There is no head unit, no NWD binder service, no GPS, no network and no image
//! decoder here. The rule this module follows is the one the task set: where a
//! subsystem is framework-bound, wire the UI to a fake so the interface is still
//! exercised, and make the fake obvious in the code rather than plausible.
//!
//! ## What each one stands in for
//!
//! | fake | the real thing | why it is absent |
//! |---|---|---|
//! | [`FakeLocation`] | Android `LocationManager` | no GPS, no framework |
//! | [`FakeRdsStream`] | the vendor RDS pump at 90 ms | no tuner |
//! | [`FakeLogoSearch`] | `logos::LogoNet` + `logos::ImageCodec` | no network, no decoder |
//! | [`seed_presets`] | a preference store of user-saved presets | not written yet |
//!
//! The tuner itself is NOT faked here: `crate::android::FakeTuner` is the
//! builder's own fake and drives the same `ingest_*` path the device drives, so
//! it is used directly.
//!
//! On Android every one of these is replaced or refused — see `crate::app`,
//! which selects them in one place and says so on screen where it can.

use crate::logos::search::Cell;
use crate::logos::{candidate, ddg::DdgImage, Raster};

// ── Position ─────────────────────────────────────────────────────────────────

/// A GPS receiver that is not there.
///
/// The coordinates are Madison, Wisconsin: the same position the station-database
/// tests query from, so the nearby picker's rows on a screenshot are the rows
/// that `the_madison_query_returns_what_carfm_returned` pins.
///
/// `fix` is settable because the satellite icon has two states and a fake that
/// can only produce one of them tests half the icon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FakeLocation {
    pub lat: f64,
    pub lon: f64,
    pub fix: bool,
    /// Drives the car glyph, nothing else. On the device this is a speed
    /// threshold over real fixes.
    pub in_motion: bool,
}

impl Default for FakeLocation {
    fn default() -> Self {
        FakeLocation { lat: 43.07, lon: -89.40, fix: true, in_motion: false }
    }
}

impl FakeLocation {
    /// The no-fix state, which the picker turns into `NearbyState::NoGps`.
    pub fn no_fix() -> FakeLocation {
        FakeLocation { fix: false, ..FakeLocation::default() }
    }

    pub fn position(&self) -> Option<(f64, f64)> {
        self.fix.then_some((self.lat, self.lon))
    }
}

// ── RDS ──────────────────────────────────────────────────────────────────────

/// A pump with no tuner behind it.
///
/// The groups are CarFM's captured hex, replayed in order and then repeated, so
/// the decoder on screen is running the same gates it runs in the car — PI needs
/// three identical groups, PS needs two agreeing complete assemblies — rather
/// than being handed a finished string.
///
/// THE HEX IS A RECORDING, NOT A RADIO. It says nothing about whether this
/// hardware behaves the way the recording did.
///
/// AND IT IS A RECORDING OF ONE STATION. [`FakeRdsStream::carries`] is what
/// keeps that honest: the groups are replayed only on the dial they came from,
/// so tuning away leaves the decoder with nothing to decode — a blank hero with
/// no genre and no RadioText, which is the correct picture of a station this
/// build has never heard. Replaying them everywhere would put WERN's RadioText
/// under someone else's call sign.
#[derive(Debug, Default)]
pub struct FakeRdsStream {
    at: usize,
}

/// The dial the recording was made on.
pub const WERN_MHZ: f32 = 88.7;

/// The stereo pilot the vendor reported alongside this recording.
///
/// Part of the recording rather than a property of the fake tuner, because
/// stereo is a property of the SIGNAL: it belongs to the station that was
/// captured, and a dial with no recording behind it must report nothing at all
/// so the pill stays empty rather than asserting MONO.
pub const WERN_STEREO: bool = true;

/// WERN 88.7 Madison: PI 0x4B2A, PTY 22 (Public), TP set, PS "WERN", and a
/// RadioText of "Wisconsin Public Radio".
///
/// EVERY BLOCK IS COMPUTED, not copied off a wire. Block B is
/// `[type 4][version 1][TP 1][PTY 5][flags 5]`, so a 0A carrying PTY 22 with TP
/// set is `0x06C8 | segment` and a 2A is `0x26C0 | segment`; block D of a 0A is
/// the two PS characters at `2 × segment`, and C and D of a 2A are the four
/// RadioText characters at `4 × segment`. Block C of a 0A is the AF pair, filled
/// with the "no alternative frequencies" code — which is what every 0A in
/// CarFM's drive logs carries.
///
/// THREE FULL PS CYCLES, and the count is arithmetic rather than taste. The
/// decoder publishes a name only after TWO agreeing complete assemblies, and it
/// discards every group that arrives before PI is confirmed — which takes three
/// identical block As. So the first cycle loses its segments 0 and 1 and can
/// never complete, the second cycle is the first complete assembly, and the
/// third is the one that agrees with it. Two cycles publish nothing at all, and
/// a face left blank by a short corpus reads as a wiring fault.
const WERN: &[&str] = &[
    // 0A, PS segments 0-3: "WE", "RN", "  ", "  ". Segments 0 and 1 of this
    // first pass are dropped — PI is not trusted until the third group.
    "4b2a06c8e0e05745",
    "4b2a06c9e0e0524e",
    "4b2a06cae0e02020",
    "4b2a06cbe0e02020",
    // The first assembly that completes.
    "4b2a06c8e0e05745",
    "4b2a06c9e0e0524e",
    "4b2a06cae0e02020",
    "4b2a06cbe0e02020",
    // The second, which agrees with it — this is the cycle that publishes.
    "4b2a06c8e0e05745",
    "4b2a06c9e0e0524e",
    "4b2a06cae0e02020",
    "4b2a06cbe0e02020",
    // 2A RadioText, segments 0-5: "Wisc", "onsi", "n Pu", "blic", " Rad",
    // then "io" and a carriage return, which terminates the message.
    "4b2a26c057697363",
    "4b2a26c16f6e7369",
    "4b2a26c26e205075",
    "4b2a26c3626c6963",
    "4b2a26c420526164",
    "4b2a26c5696f0d20",
];

impl FakeRdsStream {
    pub fn new() -> FakeRdsStream {
        FakeRdsStream::default()
    }

    /// The next group's hex, cycling forever.
    ///
    /// NOT `Iterator::next`: this never ends, and a stream that cannot be
    /// exhausted has no business wearing that trait's name.
    pub fn next_group(&mut self) -> &'static str {
        let hex = WERN[self.at % WERN.len()];
        self.at += 1;
        hex
    }

    /// Every group of one full pass, for a caller that wants the decoder settled
    /// in one go rather than a group at a time.
    pub fn all(&self) -> &'static [&'static str] {
        WERN
    }

    /// Does this recording belong to that dial?
    ///
    /// Compared through the preset key rather than `==` on two f32s, for the
    /// same reason the preset strip does: 88.7 does not round-trip.
    pub fn carries(mhz: f32) -> bool {
        crate::stations::preset_key(f64::from(mhz))
            == crate::stations::preset_key(f64::from(WERN_MHZ))
    }
}

// ── Logo search ──────────────────────────────────────────────────────────────

/// A logo search that never touches a socket.
///
/// `logos::LogoNet` and `logos::ImageCodec` are TRAITS WITH NO IMPLEMENTATIONS
/// in this crate — there is no HTTP client and no PNG/JPEG decoder in
/// `Cargo.toml` — so the search window would otherwise have nothing but its
/// error body to draw. This produces four results synchronously, with flat
/// generated art in place of decoded downloads, so the grid, the selection
/// badge, the captions and Confirm can all be exercised.
///
/// NOTHING HERE IS A LOGO. The domains are the ones the design reference
/// captions show, so the shot matches the reference; the art is four coloured
/// plates and the "download" is a function call.
pub struct FakeLogoSearch;

/// The four captions, in arrival order — a real search is never re-sorted and
/// neither is this.
/// The third row carries no dimensions, because DDG omits them often enough that
/// the empty caption is a normal case rather than an edge one.
///
/// `source` is the ORIGIN, not DDG's provider label: `ddg::parse_results` has
/// already collapsed the page URL to its last two labels by the time a row
/// reaches the model, so a fake that handed over "Bing" here would exercise a
/// path the real one never takes.
const FAKE_RESULTS: [(&str, &str, Option<u32>); 4] = [
    ("https://en.wikipedia.org/logo.png", "wikipedia.org", Some(512)),
    ("https://radio-locator.com/logo.png", "radio-locator.com", Some(300)),
    ("https://www.wmgn.com/logo.png", "wmgn.com", None),
    ("https://tunein.com/logo.png", "tunein.com", Some(1024)),
];

/// The plate colours, matching the four cells of the design reference.
const FAKE_ART: [([u8; 3], [u8; 3]); 4] = [
    ([0x20, 0x65, 0x5B], [0xF2, 0xF6, 0xF4]),
    ([0x2E, 0x5E, 0xAA], [0xFF, 0xFF, 0xFF]),
    ([0xB0, 0x2A, 0x6E], [0xFF, 0xF4, 0xF9]),
    ([0x1E, 0x1E, 0x22], [0xE8, 0xE8, 0xEC]),
];

impl FakeLogoSearch {
    /// What a `LogoNet::search` would have returned.
    pub fn results() -> Vec<DdgImage> {
        FAKE_RESULTS
            .iter()
            .map(|&(url, source, dim)| DdgImage {
                image: url.to_string(),
                thumbnail: url.to_string(),
                title: String::new(),
                width: dim,
                height: dim,
                source: source.to_string(),
            })
            .collect()
    }

    /// What a `LogoNet::fetch_image` plus an `ImageCodec::decode` would have
    /// produced for cell `index`: a plate with a lighter inset, which is enough
    /// to see the art well, the containment and the selection badge behave.
    pub fn art(index: usize, size: u32) -> Raster {
        let (fill, inset) = FAKE_ART[index % FAKE_ART.len()];
        plate(fill, inset, size)
    }
}

/// A solid plate with a lighter inset rectangle. Straight alpha, opaque.
pub fn plate(fill: [u8; 3], inset: [u8; 3], size: u32) -> Raster {
    let mut r = Raster::empty(size, size);
    let border = size / 6;
    for y in 0..size {
        for x in 0..size {
            let inner = x >= border && x < size - border && y >= border && y < size - border;
            let c = if inner { inset } else { fill };
            let at = r.at(x, y);
            r.rgba[at..at + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    r
}

/// The four cells a completed fake search leaves behind, art included.
///
/// Built through the same `candidate::*` functions the real path uses, so the
/// captions and the derived cell background are the shipping code's output even
/// though the pixels are invented.
pub fn logo_cells() -> Vec<Cell> {
    FakeLogoSearch::results()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let art = FakeLogoSearch::art(i, 128);
            Cell {
                image: r.image.clone(),
                thumbnail: r.thumbnail.clone(),
                domain: candidate::domain_caption(&r.source),
                dims: candidate::dims_label(r.width, r.height),
                alt: candidate::alt_label(i, &r.source),
                background: candidate::candidate_background(&art),
                thumb: Some(art),
            }
        })
        .collect()
}

// ── Presets ──────────────────────────────────────────────────────────────────

/// The six dials the face opens on.
///
/// THERE IS NO PRESET STORE. CarFM persists the user's own six; Carnyx has no
/// preference layer at all yet, so these are dial frequencies chosen to exist in
/// the shipped FCC table around the fake position — everything else about them
/// (call sign, city, colour) is looked up for real. Replace this with the store
/// the moment one exists; do not grow it.
pub const SEED_PRESET_MHZ: [f32; 6] = [102.1, 88.7, 105.5, 98.1, 96.3, 94.1];

/// The dial the face is tuned to on launch, deliberately NOT one of the six, so
/// the unsaved star and the peek cards' last/first fallback are both visible.
pub const SEED_DIAL_MHZ: f32 = 105.1;

pub fn seed_presets() -> Vec<f32> {
    SEED_PRESET_MHZ.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rds::RdsDecoder;

    /// The replay is not decoration: it has to actually clear the decoder's
    /// gates, or the hero would be blank on every screenshot and the wiring
    /// would look broken when it is the corpus that is short.
    #[test]
    fn the_replayed_groups_get_all_the_way_through_the_decoder() {
        let mut d = RdsDecoder::new();
        let mut last = None;
        for hex in FakeRdsStream::new().all() {
            if let Some(st) = d.push(hex) {
                last = Some(st);
            }
        }
        let st = last.expect("the replay must publish at least once");
        assert_eq!(st.pi, Some(0x4b2a));
        assert_eq!(st.pty, Some(22));
        // Two agreeing cycles, so the name is published rather than withheld.
        assert_eq!(st.ps, "WERN");
        assert!(st.tp);
        assert!(!st.ps_scrolling);
        assert_eq!(st.rt, "Wisconsin Public Radio");
    }

    #[test]
    fn the_stream_cycles_rather_than_running_out() {
        let mut s = FakeRdsStream::new();
        let first = s.next_group();
        for _ in 1..s.all().len() {
            s.next_group();
        }
        assert_eq!(s.next_group(), first);
    }

    #[test]
    fn the_fake_search_produces_four_cells_with_finished_captions() {
        let cells = logo_cells();
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[0].domain, "wikipedia.org");
        assert_eq!(cells[0].dims, "512\u{00D7}512");
        // No dimensions and no origin: both captions drop rather than printing
        // a placeholder.
        assert_eq!(cells[2].dims, "");
        assert_eq!(cells[2].domain, "wmgn.com");
        assert!(cells.iter().all(|c| c.thumb.is_some()));
    }

    /// The recording is one station's, and playing it on another dial would put
    /// WERN's RadioText under a stranger's call sign.
    #[test]
    fn the_recording_only_plays_on_the_dial_it_was_made_on() {
        assert!(FakeRdsStream::carries(88.7));
        assert!(!FakeRdsStream::carries(105.1));
        assert!(!FakeRdsStream::carries(88.9));
    }

    #[test]
    fn the_position_fake_has_both_states() {
        assert_eq!(FakeLocation::default().position(), Some((43.07, -89.40)));
        assert_eq!(FakeLocation::no_fix().position(), None);
    }
}
