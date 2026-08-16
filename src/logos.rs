//! Station logos: the search that finds one, the store that keeps it, and the
//! pixel work that makes it usable on both themes.
//!
//! Ported by hand from CarFM's `src/services/logo*.ts`, `src/services/logoDark/`
//! and `src/components/carfm/LogoSearchOverlay.tsx`. CarFM-original throughout:
//! the whole logo system was written for the car face after the VibeSDR fork
//! (`logoStore.ts` 1ddf063 and `logoPrep.ts` 5a69021, both 2026-07-30; the
//! `logoDark/` tree 264600a / 2c87983, 2026-07-25). VibeSDR's own logo path was
//! `stationLogoCache.ts` + `stationLogo.ts`, which are inherited, are built on the
//! removed Radio-Browser source, and are NOT ported — CarFM's replacement is.
//!
//! ── THE ONE FACT THAT SHAPES EVERYTHING ──────────────────────────────────────
//! There is no automatic logo fetching, and there must not be one. CarFM's
//! `AUTO_LOGO_RESOLUTION` has been `false` since the 2026-07-17 device test, where
//! auto-downloaded logos came back "completely wrong" — text-matching sources
//! happily return unrelated images. The only path that has ever written a logo in
//! a shipping CarFM build is a user tapping a candidate in the logo-search window.
//! The resolver cascade below is ported because the shape is worth keeping, but it
//! ships behind `AUTO_LOGO_RESOLUTION = false` with no background caller, exactly
//! as CarFM has it. Wiring a sweep to it would re-arm a bug the owner already paid
//! for on the road.
//!
//! ── WHAT IS PURE AND WHAT IS NOT ─────────────────────────────────────────────
//! Everything in this file is pure and unit-tested except three named seams:
//! `LogoNet` (six HTTP calls), `ImageCodec` (decode/encode) and `LogoStore`'s
//! filesystem calls. The first two are implemented nowhere in THIS file on
//! purpose — they are traits, so every decision they would otherwise make has
//! been pulled out into a function a test can call, and the host exercises them
//! through `crate::fake::FakeLogoSearch`.
//!
//! The real ones live in `crate::android::net`: `AndroidNet` over
//! `HttpsURLConnection` and `AndroidCodec` over `BitmapFactory`, both compiled
//! only for Android. NEITHER HAS EVER RUN in this container — it has no network,
//! no Android SDK and no head unit — so what is claimed for them is that they
//! compile for `armv7-linux-androideabi`, and nothing more.
//!
//! ── SLINT DECIDES NOTHING ────────────────────────────────────────────────────
//! `ui/logo-search.slint` exposes 21 in-properties and 7 callbacks and computes
//! none of them. `search::Model` below owns the state machine, and `search::View`
//! is the finished set of values it hands over: which body shows, which cell is
//! picked, the three-way Confirm label, the five-way footer hint, both error
//! wordings, the "512×512" caption and the per-candidate background colour.

// ═════════════════════════════════════════════════════════════════════════════
// Rasters
// ═════════════════════════════════════════════════════════════════════════════

/// A decoded, straight-alpha, sRGB image — the currency every stage below deals
/// in. `rgba` is `w * h * 4` bytes, R,G,B,A, un-premultiplied.
///
/// This type is the seam. Nothing above it knows about `slint::Image` and nothing
/// below it knows about an image decoder, which is what lets the whole pipeline
/// run in a unit test with no crates at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    /// A fully transparent raster of the given size.
    pub fn empty(w: u32, h: u32) -> Raster {
        Raster { w, h, rgba: vec![0; (w as usize) * (h as usize) * 4] }
    }

    /// Byte offset of pixel (x, y).
    #[inline]
    pub fn at(&self, x: u32, y: u32) -> usize {
        ((y as usize) * (self.w as usize) + x as usize) * 4
    }

    #[inline]
    pub fn pixels(&self) -> usize {
        (self.w as usize) * (self.h as usize)
    }

    /// Straight-alpha RGBA of one pixel.
    #[inline]
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let j = self.at(x, y);
        [self.rgba[j], self.rgba[j + 1], self.rgba[j + 2], self.rgba[j + 3]]
    }

    /// Paint one pixel. Test helper and stage scratch; not a hot path.
    pub fn set(&mut self, x: u32, y: u32, c: [u8; 4]) {
        let j = self.at(x, y);
        self.rgba[j..j + 4].copy_from_slice(&c);
    }

    /// True when the raster has at least one pixel and the buffer is the right
    /// length — every stage assumes both, and a decoder that lies must not be
    /// allowed to index out of bounds inside a pixel loop.
    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0 && self.rgba.len() == self.pixels() * 4
    }
}

/// `Uint8ClampedArray`'s float→byte conversion: clamp to [0,255], then round HALF
/// TO EVEN.
///
/// This is not pedantry. Every CarFM stage writes its pixels through a
/// `Uint8ClampedArray`, so this rule is baked into every stored logo the head unit
/// already has. Rust's `as u8` truncates and `f64::round` rounds half AWAY from
/// zero; either one drifts by one LSB on exact ties, which looks like a real bug
/// in a golden-image diff and is not one.
#[inline]
pub fn clamped_u8(x: f64) -> u8 {
    if x.is_nan() {
        return 0;
    }
    if x <= 0.0 {
        return 0;
    }
    if x >= 255.0 {
        return 255;
    }
    let f = x.floor();
    if f + 0.5 < x {
        (f as u8).wrapping_add(1)
    } else if x < f + 0.5 {
        f as u8
    } else if (f as u32) % 2 == 1 {
        (f as u8).wrapping_add(1)
    } else {
        f as u8
    }
}

/// JavaScript's `Math.round`, then the clamp — half UP, not half to even.
///
/// The two rules live side by side because CarFM uses both: a stage that writes
/// `out[j] = v` gets `clamped_u8`, and a stage that writes
/// `out[j] = Math.round(v * 255)` gets this one. `linearToSrgb8`, the keying
/// decontamination, the halo composite and every alpha write are the second kind.
#[inline]
pub fn js_round_u8(x: f64) -> u8 {
    if x.is_nan() {
        return 0;
    }
    // Math.round is floor(x + 0.5): ties go toward +infinity, which for the
    // non-negative values here is the same as Rust's ties-away-from-zero.
    let r = (x + 0.5).floor();
    if r <= 0.0 {
        0
    } else if r >= 255.0 {
        255
    } else {
        r as u8
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// A minimal JSON reader
// ═════════════════════════════════════════════════════════════════════════════

/// Just enough JSON to read DuckDuckGo's `i.js` payload and our own `meta.json`.
///
/// Carnyx has no serde and `Cargo.toml` belongs to the build agent, so rather than
/// leave the one piece of parsing that faces a hostile input untested, it is
/// written here and pinned by tests. Swap it for `serde_json` the moment a
/// dependency can be added — this exists to avoid a dependency, not because it is
/// better.
pub mod json {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Json {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Json>),
        Obj(Vec<(String, Json)>),
    }

    impl Json {
        pub fn get(&self, key: &str) -> Option<&Json> {
            match self {
                Json::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Json::Str(s) => Some(s),
                _ => None,
            }
        }
        pub fn as_f64(&self) -> Option<f64> {
            match self {
                Json::Num(n) => Some(*n),
                _ => None,
            }
        }
        /// A JSON number read as a pixel dimension. Non-finite, negative and
        /// fractional values are rejected rather than truncated: DDG reports
        /// integers, and anything else is a payload we do not understand.
        pub fn as_u32(&self) -> Option<u32> {
            let n = self.as_f64()?;
            if n.is_finite() && n >= 0.0 && n <= u32::MAX as f64 && n.fract() == 0.0 {
                Some(n as u32)
            } else {
                None
            }
        }
        pub fn as_bool(&self) -> Option<bool> {
            match self {
                Json::Bool(b) => Some(*b),
                _ => None,
            }
        }
        pub fn as_arr(&self) -> Option<&[Json]> {
            match self {
                Json::Arr(v) => Some(v),
                _ => None,
            }
        }
    }

    struct P {
        s: Vec<char>,
        i: usize,
    }

    impl P {
        fn ws(&mut self) {
            while self.i < self.s.len() && matches!(self.s[self.i], ' ' | '\t' | '\n' | '\r') {
                self.i += 1;
            }
        }
        fn eat(&mut self, c: char) -> bool {
            if self.i < self.s.len() && self.s[self.i] == c {
                self.i += 1;
                true
            } else {
                false
            }
        }
        fn lit(&mut self, word: &str) -> bool {
            let w: Vec<char> = word.chars().collect();
            if self.s.len() >= self.i + w.len() && self.s[self.i..self.i + w.len()] == w[..] {
                self.i += w.len();
                true
            } else {
                false
            }
        }

        fn value(&mut self, depth: u32) -> Option<Json> {
            // A hostile payload must not blow the stack; 64 is far past anything
            // DDG or our own meta file nests to.
            if depth > 64 {
                return None;
            }
            self.ws();
            let c = *self.s.get(self.i)?;
            match c {
                '{' => {
                    self.i += 1;
                    let mut out = Vec::new();
                    self.ws();
                    if self.eat('}') {
                        return Some(Json::Obj(out));
                    }
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        if !self.eat(':') {
                            return None;
                        }
                        let v = self.value(depth + 1)?;
                        out.push((k, v));
                        self.ws();
                        if self.eat(',') {
                            continue;
                        }
                        return if self.eat('}') { Some(Json::Obj(out)) } else { None };
                    }
                }
                '[' => {
                    self.i += 1;
                    let mut out = Vec::new();
                    self.ws();
                    if self.eat(']') {
                        return Some(Json::Arr(out));
                    }
                    loop {
                        out.push(self.value(depth + 1)?);
                        self.ws();
                        if self.eat(',') {
                            continue;
                        }
                        return if self.eat(']') { Some(Json::Arr(out)) } else { None };
                    }
                }
                '"' => self.string().map(Json::Str),
                't' => self.lit("true").then_some(Json::Bool(true)),
                'f' => self.lit("false").then_some(Json::Bool(false)),
                'n' => self.lit("null").then_some(Json::Null),
                _ => self.number(),
            }
        }

        fn string(&mut self) -> Option<String> {
            if !self.eat('"') {
                return None;
            }
            let mut out = String::new();
            loop {
                let c = *self.s.get(self.i)?;
                self.i += 1;
                match c {
                    '"' => return Some(out),
                    '\\' => {
                        let e = *self.s.get(self.i)?;
                        self.i += 1;
                        match e {
                            '"' => out.push('"'),
                            '\\' => out.push('\\'),
                            '/' => out.push('/'),
                            'b' => out.push('\u{08}'),
                            'f' => out.push('\u{0C}'),
                            'n' => out.push('\n'),
                            'r' => out.push('\r'),
                            't' => out.push('\t'),
                            'u' => {
                                let hi = self.hex4()?;
                                // A lone surrogate is not an error here: DDG
                                // titles have carried them, and dropping the
                                // whole result over one bad code unit would lose
                                // a perfectly good image URL.
                                if (0xD800..0xDC00).contains(&hi) {
                                    let save = self.i;
                                    if self.eat('\\') && self.eat('u') {
                                        if let Some(lo) = self.hex4() {
                                            if (0xDC00..0xE000).contains(&lo) {
                                                let cp = 0x10000
                                                    + (((hi - 0xD800) as u32) << 10)
                                                    + (lo - 0xDC00) as u32;
                                                out.push(char::from_u32(cp)?);
                                                continue;
                                            }
                                        }
                                    }
                                    self.i = save;
                                    out.push('\u{FFFD}');
                                } else {
                                    out.push(char::from_u32(hi as u32).unwrap_or('\u{FFFD}'));
                                }
                            }
                            _ => return None,
                        }
                    }
                    _ => out.push(c),
                }
            }
        }

        fn hex4(&mut self) -> Option<u16> {
            let mut v: u16 = 0;
            for _ in 0..4 {
                let c = *self.s.get(self.i)?;
                self.i += 1;
                v = v.checked_mul(16)?.checked_add(c.to_digit(16)? as u16)?;
            }
            Some(v)
        }

        fn number(&mut self) -> Option<Json> {
            let start = self.i;
            if self.i < self.s.len() && self.s[self.i] == '-' {
                self.i += 1;
            }
            while self.i < self.s.len()
                && matches!(self.s[self.i], '0'..='9' | '.' | 'e' | 'E' | '+' | '-')
            {
                self.i += 1;
            }
            if self.i == start {
                return None;
            }
            let text: String = self.s[start..self.i].iter().collect();
            text.parse::<f64>().ok().map(Json::Num)
        }
    }

    /// Parse a complete JSON document. `None` on anything malformed — callers
    /// treat that exactly as they treat an empty result.
    pub fn parse(src: &str) -> Option<Json> {
        let mut p = P { s: src.chars().collect(), i: 0 };
        let v = p.value(0)?;
        p.ws();
        if p.i == p.s.len() {
            Some(v)
        } else {
            None
        }
    }

    /// Escape a string into a JSON literal, quotes included.
    pub fn quote(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The query
// ═════════════════════════════════════════════════════════════════════════════

/// Building the one search string the whole feature rests on.
///
/// From `services/logoDuckDuckGo.ts`. The shape is not a guess: CarFM's header
/// records that `radio <freq> <lowercase-callsign> logo` returned the correct
/// station logo as the #1 hit for 7 of 7 stations in the test market, and that the
/// call sign MUST be lower case. Every other shape they tried failed for much of
/// the US.
pub mod query {
    /// `radio 88.7 wern logo`.
    ///
    /// The call sign is NOT `clean_call`ed here — the caller passes a base that is
    /// already reduced, and a call sign that could not be resolved arrives empty
    /// so the query degrades to `radio 98.1 logo` rather than to junk.
    pub fn station_logo_query(freq_mhz: Option<f32>, callsign: &str) -> String {
        let cs = callsign.trim().to_lowercase();
        // `typeof f === 'number' && isFinite(f)` — an absent dial and a NaN one are
        // the same thing to the query.
        let f = match freq_mhz {
            Some(v) if v.is_finite() => format!("{:.1} ", v),
            _ => String::new(),
        };
        collapse_ws(&format!("radio {f}{cs} logo"))
    }

    /// JavaScript's `.replace(/\s+/g, ' ').trim()`.
    ///
    /// `char::is_whitespace` is the Unicode White_Space property, which differs
    /// from JS `\s` in two code points (JS includes U+FEFF, Unicode does not; JS
    /// excludes U+0085, Unicode includes it). Call signs are ASCII, so neither is
    /// reachable from here — it is written down so the next reader does not have
    /// to re-derive it.
    fn collapse_ws(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut pending = false;
        for c in s.chars() {
            if c.is_whitespace() {
                pending = !out.is_empty();
            } else {
                if pending {
                    out.push(' ');
                    pending = false;
                }
                out.push(c);
            }
        }
        out
    }

    /// JavaScript's `encodeURIComponent`: everything is percent-encoded except
    /// `A-Za-z0-9` and `- _ . ! ~ * ' ( )`.
    ///
    /// Written out rather than reached for, because the two obvious substitutes
    /// are both wrong for this endpoint: a form encoder turns a space into `+`,
    /// and a full RFC 3986 encoder escapes `!*'()`. DDG's `vqd` token is echoed
    /// back verbatim, so an encoder that disagrees by one character invalidates
    /// the token and the search returns nothing at all.
    pub fn encode_component(s: &str) -> String {
        const KEEP: &[u8] = b"-_.!~*'()";
        let mut out = String::with_capacity(s.len());
        for b in s.as_bytes() {
            if b.is_ascii_alphanumeric() || KEEP.contains(b) {
                out.push(*b as char);
            } else {
                out.push_str(&format!("%{:02X}", b));
            }
        }
        out
    }

    /// Registrable-ish host of a URL: `https://en.wikipedia.org/wiki/X` →
    /// `wikipedia.org`. Empty for anything that is not http(s).
    ///
    /// DDG's own `source` field is the provider ("Bing"), not the image origin, so
    /// the caption is derived from the result URL the way the DDG web UI does it.
    ///
    /// THE `.co.uk` WART IS DELIBERATE. Keeping the last two labels unconditionally
    /// turns `a.b.co.uk` into `co.uk`, which is not the registrable domain. It is
    /// what shipped, it is what the reference screenshots show under the candidate
    /// cells, and a public-suffix list to fix it would be a 200 KB dependency to
    /// improve a caption. Ported as-is on purpose.
    pub fn host_of(u: &str) -> String {
        let rest = if let Some(r) = strip_prefix_ci(u, "https://") {
            r
        } else if let Some(r) = strip_prefix_ci(u, "http://") {
            r
        } else {
            return String::new();
        };
        let host: &str = rest.split(['/', '?', '#']).next().unwrap_or("");
        // `.replace(/^www\./i, '')` — one leading `www.`, case-insensitively.
        let host = strip_prefix_ci(host, "www.").unwrap_or(host);
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() > 2 {
            parts[parts.len() - 2..].join(".")
        } else {
            host.to_string()
        }
    }

    /// `str::get` rather than `s[..n]`, and the difference is a crash. A length
    /// check is NOT a char-boundary check: slicing a `&str` at a byte offset that
    /// lands inside a multi-byte character panics. These strings come off the
    /// network, so the input is not ours to trust, and a panic here is a panic on
    /// a dashboard. `get` returns None on a bad boundary instead.
    pub(super) fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
        let head = s.get(..prefix.len())?;
        head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
    }

    /// The two URLs the DuckDuckGo flow needs, built once so the network edge
    /// carries no string arithmetic of its own.
    pub const DDG_UA: &str = "Mozilla/5.0 (Linux; Android) CarFM/1.0";
    /// DDG's `t=` source tag. Polite identification, and CarFM's own tag — the
    /// endpoint is unofficial and scraped, so it says who is asking.
    pub const DDG_APP_TAG: &str = "carnyx";

    /// The search page, whose body carries the one-time `vqd` token.
    pub fn vqd_url(q: &str) -> String {
        format!(
            "https://duckduckgo.com/?q={}&t={}&iar=images&iax=images&ia=images",
            encode_component(q),
            DDG_APP_TAG
        )
    }

    /// The JSON image endpoint the DDG web UI itself drives.
    pub fn results_url(q: &str, vqd: &str) -> String {
        format!(
            "https://duckduckgo.com/i.js?l=us-en&o=json&q={}&vqd={}&f=,,,&p=1&t={}",
            encode_component(q),
            encode_component(vqd),
            DDG_APP_TAG
        )
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The DuckDuckGo protocol, pure half
// ═════════════════════════════════════════════════════════════════════════════

/// Reading DDG's two responses. No sockets here — `parse_vqd` takes the search
/// page's body and `parse_results` takes the `i.js` payload, so the only part of
/// this protocol that can be got wrong silently is the part a test can drive.
pub mod ddg {
    use super::json::{self, Json};
    use super::query::host_of;

    /// One picker-ready image result, already filtered and already attributed.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DdgImage {
        /// Full-size image, tried FIRST on Confirm.
        pub image: String,
        /// DDG's proxied thumbnail — what the grid draws, and the fallback on
        /// Confirm when the full-size URL 403s or busts the size cap.
        pub thumbnail: String,
        pub title: String,
        pub width: Option<u32>,
        pub height: Option<u32>,
        /// Registrable-ish host of the ORIGIN, not DDG's provider label.
        pub source: String,
    }

    /// Scrape the one-time `vqd` token out of the search page.
    ///
    /// Three shapes tried IN ORDER OF SPECIFICITY, because the token's framing has
    /// already shifted across DDG revisions and CarFM carries all three:
    ///   1. `vqd="4-123…"`   2. `vqd=4-123…&`   3. `&vqd=…`
    ///
    /// Each pattern is tried across the WHOLE body before the next is considered,
    /// which is what the JavaScript `||` chain does.
    pub fn parse_vqd(body: &str) -> Option<String> {
        // /vqd=["']([^"'&]+)["']/
        if let Some(v) = scan(body, "vqd=", |rest| {
            let mut it = rest.chars();
            let open = it.next()?;
            if open != '"' && open != '\'' {
                return None;
            }
            let after = &rest[open.len_utf8()..];
            let cap: String = after.chars().take_while(|c| !matches!(c, '"' | '\'' | '&')).collect();
            if cap.is_empty() {
                return None;
            }
            let close = after[cap.len()..].chars().next()?;
            if close == '"' || close == '\'' {
                Some(cap)
            } else {
                None
            }
        }) {
            return Some(v);
        }
        // /vqd=([\d-]+)&/
        if let Some(v) = scan(body, "vqd=", |rest| {
            let cap: String =
                rest.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
            if cap.is_empty() || !rest[cap.len()..].starts_with('&') {
                None
            } else {
                Some(cap)
            }
        }) {
            return Some(v);
        }
        // /&vqd=([^&"']+)/
        scan(body, "&vqd=", |rest| {
            let cap: String = rest.chars().take_while(|c| !matches!(c, '&' | '"' | '\'')).collect();
            if cap.is_empty() {
                None
            } else {
                Some(cap)
            }
        })
    }

    /// Leftmost match of `needle` whose tail satisfies `f`, which is how a regex
    /// engine walks a subject.
    fn scan(body: &str, needle: &str, f: impl Fn(&str) -> Option<String>) -> Option<String> {
        let mut from = 0usize;
        while let Some(rel) = body[from..].find(needle) {
            let at = from + rel + needle.len();
            if let Some(v) = f(&body[at..]) {
                return Some(v);
            }
            from = from + rel + 1;
        }
        None
    }

    /// The first `n` results, IN ARRIVAL ORDER.
    ///
    /// NOTHING IS RANKED. Not sorted, not scored, not deduplicated, not
    /// re-ordered. CarFM says so twice in the source and the Slint doc comment
    /// says it a third time: a wrong #1 hit is still possible, which is exactly
    /// why this only ever runs behind an explicit tap and why the human is the
    /// ranker. Adding a scorer here would be a regression, not an improvement.
    pub fn parse_results(body: &str, n: usize) -> Vec<DdgImage> {
        let Some(root) = json::parse(body) else { return Vec::new() };
        let Some(items) = root.get("results").and_then(Json::as_arr) else { return Vec::new() };
        items
            .iter()
            // Only entries with a usable full-size image survive; everything else
            // is dropped before the take, so a junk row cannot consume a slot.
            .filter_map(|r| {
                let image = r.get("image").and_then(Json::as_str).filter(|s| is_http(s))?;
                let url = r.get("url").and_then(Json::as_str).unwrap_or("");
                let thumb = r.get("thumbnail").and_then(Json::as_str).unwrap_or("");
                Some(DdgImage {
                    image: image.to_string(),
                    thumbnail: if is_http(thumb) { thumb.to_string() } else { image.to_string() },
                    title: r.get("title").and_then(Json::as_str).unwrap_or("").to_string(),
                    width: r.get("width").and_then(Json::as_u32),
                    height: r.get("height").and_then(Json::as_u32),
                    // Origin domain, page URL first, image URL second, DDG's own
                    // provider label only as a last resort.
                    source: {
                        let a = host_of(url);
                        if !a.is_empty() {
                            a
                        } else {
                            let b = host_of(image);
                            if !b.is_empty() {
                                b
                            } else {
                                r.get("source").and_then(Json::as_str).unwrap_or("").to_string()
                            }
                        }
                    },
                })
            })
            .take(n)
            .collect()
    }

    /// Same char-boundary hazard as `strip_prefix_ci`: the old form guarded the
    /// LENGTH and then sliced, which panics when byte 7 or 8 falls inside a
    /// multi-byte character.
    pub(super) fn is_http(s: &str) -> bool {
        s.get(..7).is_some_and(|h| h.eq_ignore_ascii_case("http://"))
            || s.get(..8).is_some_and(|h| h.eq_ignore_ascii_case("https://"))
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Candidate cells
// ═════════════════════════════════════════════════════════════════════════════

/// The finished values one result cell draws. `ui/logo-search.slint`'s
/// `LogoCandidate` declares five fields and computes none of them.
pub mod candidate {
    use super::dark::oklab::{lab_dist, srgb8_to_lab};
    use super::Raster;

    /// `512×512`, U+00D7 MULTIPLICATION SIGN, or EMPTY when either dimension is
    /// missing. DDG omits both often enough that the empty case is normal, and
    /// Slint's float-to-string can neither pick a separator nor drop a decimal.
    pub fn dims_label(w: Option<u32>, h: Option<u32>) -> String {
        match (w, h) {
            (Some(w), Some(h)) => format!("{w}\u{00D7}{h}"),
            _ => String::new(),
        }
    }

    /// `Logo option 3 from wikipedia.org` — the screen-reader label, with the
    /// " from …" half dropped when the origin is unknown. `index` is zero-based
    /// and printed one-based, matching the TSX's `i + 1`.
    pub fn alt_label(index: usize, domain: &str) -> String {
        if domain.is_empty() {
            format!("Logo option {}", index + 1)
        } else {
            format!("Logo option {} from {}", index + 1, domain)
        }
    }

    /// The caption under a cell. CarFM prints `r.source || 'image'`, so a result
    /// whose origin could not be derived still gets a word rather than a gap.
    ///
    /// Note the asymmetry with `alt_label`, which is deliberate in the TSX: the
    /// caption falls back to the literal "image", the accessibility label drops
    /// the clause entirely rather than reading "from image".
    pub fn domain_caption(source: &str) -> String {
        if source.is_empty() {
            "image".to_string()
        } else {
            source.to_string()
        }
    }

    /// §6.4 asks for "the candidate art on its own background", and the reference
    /// gives every cell its own `imgBg`. The shipping TSX hard-codes white, which
    /// is the same thing for the common case of a logo on a white plate and wrong
    /// for a logo on a dark one.
    ///
    /// The rule: if all four corners are opaque and agree, the art carries its own
    /// backing and the cell adopts it; otherwise the art is transparent or
    /// irregular and white is the safe well. Deliberately conservative — a cell
    /// that guesses wrong makes the logo invisible, which is worse than a plain
    /// white well.
    pub fn candidate_background(thumb: &Raster) -> [u8; 3] {
        const WHITE: [u8; 3] = [255, 255, 255];
        if !thumb.is_valid() {
            return WHITE;
        }
        let (w, h) = (thumb.w, thumb.h);
        let corners =
            [thumb.px(0, 0), thumb.px(w - 1, 0), thumb.px(0, h - 1), thumb.px(w - 1, h - 1)];
        if corners.iter().any(|c| c[3] < 250) {
            return WHITE;
        }
        // The same "median of four corners" the dark pipeline's `trim` uses: sort
        // each channel and average the middle two, so one odd corner cannot drag
        // the answer.
        let mut med = [0u8; 3];
        for (c, m) in med.iter_mut().enumerate() {
            let mut v = [corners[0][c], corners[1][c], corners[2][c], corners[3][c]];
            v.sort_unstable();
            *m = ((v[1] as u16 + v[2] as u16) / 2) as u8;
        }
        let med_lab = srgb8_to_lab(med[0], med[1], med[2]);
        for c in corners {
            if lab_dist(srgb8_to_lab(c[0], c[1], c[2]), med_lab) > 0.04 {
                return WHITE;
            }
        }
        med
    }

    /// CarFM's `tokens.ts` `monogram`, for the no-logo header tile. A four-letter
    /// US call sign drops its leading K or W — `WMGN` reads as `MGN` — because
    /// the first letter carries no information east or west of the Mississippi
    /// and three glyphs fit a small tile far better than four.
    ///
    /// Lives here rather than in `station.rs` because the logo window is its only
    /// caller; move it if a second surface ever wants it.
    pub fn monogram(callsign: &str) -> String {
        let base: String =
            callsign.to_uppercase().split('-').next().unwrap_or("").trim().to_string();
        let chars: Vec<char> = base.chars().collect();
        if chars.len() == 4 && (chars[0] == 'K' || chars[0] == 'W') {
            return chars[1..].iter().collect();
        }
        let short: String = chars.iter().take(4).collect();
        if short.is_empty() {
            "?".to_string()
        } else {
            short
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Prep on assign
// ═════════════════════════════════════════════════════════════════════════════

/// Trimming and the size ladder (ANDROID §4.5), from `services/logoPrep.ts`.
///
/// Two jobs, both of which exist because every surface renders a logo with
/// `contain`: the box scales the WHOLE image, so baked-in margin means the visible
/// mark occupies a fraction of a correctly-sized box. Trim first, then pre-render
/// the sizes the surfaces actually ask for so a preset strip decodes small PNGs
/// instead of full-resolution ones.
pub mod prep {
    use super::{clamped_u8, Raster};

    /// Longest-edge sizes pre-rendered for every logo. Covers the preset chip
    /// (~85 dp box) through the hero (256 dp tall) at up to 2× density.
    pub const SIZE_LADDER: [u32; 3] = [128, 256, 512];

    /// Trim ratio below which a separate cropped file is not worth its inode.
    const MIN_GAIN: f64 = 0.02;
    /// Alpha at or below this is empty space, not mark.
    const ALPHA_BG: u8 = 8;
    /// Per-channel distance from the border colour that still counts as margin.
    const BG_TOL: f64 = 0.04;
    /// Smallest crop worth keeping, per edge.
    const MIN_CROP_EDGE: u32 = 8;

    /// Longest edge a master is decoded to before any pixel work.
    ///
    /// CarFM caps at decode, inside Skia, with a single linear tap. Carnyx decodes
    /// full and caps with the area average below, which is a BETTER downscale and
    /// therefore not a bit-identical one: a master over 1024 px will differ from
    /// what the same image produced on the phone. Everything at or under 1024 px —
    /// which is nearly every station logo on the web — is unaffected.
    pub const DECODE_MAX_EDGE: u32 = 1024;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Bounds {
        pub x0: u32,
        pub y0: u32,
        pub x1: u32,
        pub y1: u32,
    }

    /// Bounding box of the VISIBLE MARK: everything that is neither transparent
    /// nor uniform paper margin.
    ///
    /// Deliberately NOT the dark pipeline's `trim`. That one counts a transparent
    /// pixel as CONTENT — it must, because transparency is what its keying stage
    /// operates on — so it will not crop the transparent padding that surrounds
    /// most PNG logos, which is the commonest form of baked-in whitespace.
    pub fn mark_bounds(img: &Raster) -> Option<Bounds> {
        if !img.is_valid() {
            return None;
        }
        let (w, h) = (img.w, img.h);
        let px = &img.rgba;

        // Median colour of the OPAQUE border pixels. Corners are sampled twice,
        // exactly as the JavaScript does — it is a vote, not a set.
        let mut ch: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut border_n = 0usize;
        let mut border_opaque = 0usize;
        let mut add = |x: u32, y: u32| {
            let j = ((y as usize) * (w as usize) + x as usize) * 4;
            border_n += 1;
            if px[j + 3] <= ALPHA_BG {
                return; // a transparent frame casts no colour vote
            }
            border_opaque += 1;
            for (c, v) in ch.iter_mut().enumerate() {
                v.push(px[j + c] as f64 / 255.0);
            }
        };
        for x in 0..w {
            add(x, 0);
            add(x, h - 1);
        }
        for y in 0..h {
            add(0, y);
            add(w - 1, y);
        }
        let med: [f64; 3] = std::array::from_fn(|c| {
            let v = &mut ch[c];
            if v.is_empty() {
                -1.0
            } else {
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                v[v.len() >> 1] // JS `sorted[len >> 1]`: the upper median on even counts
            }
        });

        // THE LOAD-BEARING SUBTLETY. An opaque border only counts as margin when
        // it is PAPER — near-white or near-black. A SATURATED border is the logo
        // itself (a solid badge), and treating that colour as background would
        // classify most of the badge as empty and crop the image down to whatever
        // sits ON it. With a saturated border only transparency marks background,
        // so a fully-opaque badge yields no crop at all, which is correct.
        let mostly_opaque_border = border_opaque as f64 > border_n as f64 * 0.5;
        let paper = med[0] >= 0.0
            && ((med[0] >= 0.9 && med[1] >= 0.9 && med[2] >= 0.9)
                || (med[0] <= 0.1 && med[1] <= 0.1 && med[2] <= 0.1));
        let colour_is_bg = mostly_opaque_border && paper;

        let (mut x0, mut y0) = (w, h);
        let (mut x1, mut y1) = (-1i64, -1i64);
        for y in 0..h {
            for x in 0..w {
                let j = ((y as usize) * (w as usize) + x as usize) * 4;
                if px[j + 3] <= ALPHA_BG {
                    continue; // transparent margin
                }
                if colour_is_bg
                    && (px[j] as f64 / 255.0 - med[0]).abs() <= BG_TOL
                    && (px[j + 1] as f64 / 255.0 - med[1]).abs() <= BG_TOL
                    && (px[j + 2] as f64 / 255.0 - med[2]).abs() <= BG_TOL
                {
                    continue; // paper margin
                }
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x as i64);
                y1 = y1.max(y as i64);
            }
        }
        if x1 < 0 {
            None
        } else {
            Some(Bounds { x0, y0, x1: x1 as u32, y1: y1 as u32 })
        }
    }

    pub fn crop_raster(img: &Raster, b: Bounds) -> Raster {
        let w = b.x1 - b.x0 + 1;
        let h = b.y1 - b.y0 + 1;
        let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
        for y in 0..h as usize {
            let s = ((y + b.y0 as usize) * img.w as usize + b.x0 as usize) * 4;
            let d = y * w as usize * 4;
            out[d..d + w as usize * 4].copy_from_slice(&img.rgba[s..s + w as usize * 4]);
        }
        Raster { w, h, rgba: out }
    }

    /// Area-average downscale to a longest edge of `max_edge`, alpha-weighted.
    ///
    /// A single linear tap — which is what both Skia's decode-time scaler and a
    /// naive `image` crate resize give — aliases badly on the reductions the
    /// ladder needs (512 → 128 destroys thin lettering). Averaging every source
    /// pixel in the footprint keeps it readable. The weighting matters just as
    /// much: without it, transparent padding drags its own black RGB into the
    /// average and the mark grows dark fringes.
    ///
    /// Returns a clone unchanged if the image already fits.
    pub fn resample_raster(img: &Raster, max_edge: u32) -> Raster {
        let longest = img.w.max(img.h) as f64;
        let scale = max_edge as f64 / longest;
        if scale >= 1.0 {
            return img.clone();
        }
        let w = ((img.w as f64 * scale).round() as u32).max(1);
        let h = ((img.h as f64 * scale).round() as u32).max(1);
        let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
        let sx = img.w as f64 / w as f64;
        let sy = img.h as f64 / h as f64;
        for y in 0..h {
            let y0 = (y as f64 * sy).floor() as usize;
            let y1 = (img.h as usize).min((y0 + 1).max(((y as f64 + 1.0) * sy).ceil() as usize));
            for x in 0..w {
                let x0 = (x as f64 * sx).floor() as usize;
                let x1 =
                    (img.w as usize).min((x0 + 1).max(((x as f64 + 1.0) * sx).ceil() as usize));
                let (mut r, mut g, mut b) = (0.0f64, 0.0f64, 0.0f64);
                let (mut a, mut aw, mut n) = (0.0f64, 0.0f64, 0usize);
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        let j = (yy * img.w as usize + xx) * 4;
                        let al = img.rgba[j + 3] as f64;
                        r += img.rgba[j] as f64 * al;
                        g += img.rgba[j + 1] as f64 * al;
                        b += img.rgba[j + 2] as f64 * al;
                        a += al;
                        aw += al;
                        n += 1;
                    }
                }
                let j = ((y as usize) * (w as usize) + x as usize) * 4;
                out[j] = if aw > 0.0 { clamped_u8(r / aw) } else { 0 };
                out[j + 1] = if aw > 0.0 { clamped_u8(g / aw) } else { 0 };
                out[j + 2] = if aw > 0.0 { clamped_u8(b / aw) } else { 0 };
                out[j + 3] = if n > 0 { clamped_u8(a / n as f64) } else { 0 };
            }
        }
        Raster { w, h, rgba: out }
    }

    /// The size ladder for one raster, in WRITE order (largest first).
    ///
    /// PROGRESSIVE ON PURPOSE: each step downscales the previous, already smaller
    /// output rather than re-scanning the master, so 512/256/128 costs roughly one
    /// master pass instead of three — and a two-step reduction looks better than
    /// one big jump. A size the image is already smaller than is skipped, which is
    /// why a 300 px master yields [256, 128] and never a blown-up 512.
    pub fn ladder_rasters(src: &Raster) -> Vec<(u32, Raster)> {
        let mut out = Vec::new();
        let mut cur = src.clone();
        let mut sizes = SIZE_LADDER;
        sizes.sort_unstable_by(|a, b| b.cmp(a)); // largest first
        for size in sizes {
            if cur.w.max(cur.h) <= size {
                continue;
            }
            cur = resample_raster(&cur, size);
            out.push((size, cur.clone()));
        }
        out
    }

    /// The display rendition: flatten a baked-in editor checkerboard, then crop to
    /// the visible mark when the crop is worth taking.
    ///
    /// The flatten runs here as well as in the dark pipeline for the same reason:
    /// a checkerboard reads as content and defeats the crop entirely.
    pub fn display_rendition(master: &Raster) -> Raster {
        let flat = super::dark::stages::flatten_checkerboard(master).0;
        let Some(b) = mark_bounds(&flat) else { return flat };
        let cropped = crop_raster(&flat, b);
        let gain = 1.0
            - (cropped.w as f64 * cropped.h as f64)
                / (flat.w as f64 * flat.h as f64).max(1.0);
        if cropped.w >= MIN_CROP_EDGE && cropped.h >= MIN_CROP_EDGE && gain >= MIN_GAIN {
            cropped
        } else {
            flat
        }
    }

    /// Smallest pre-rendered size that still covers `box_dp` at this screen's
    /// density; `None` means "use the full-size file".
    ///
    /// The density is clamped at 2× the way CarFM clamps `PixelRatio.get()`: a 3×
    /// panel would ask for 384 px of a 128 dp chip and get nothing useful for the
    /// extra decode.
    pub fn ladder_for(box_dp: Option<f32>, scale: f32, available: &[u32]) -> Option<u32> {
        let box_dp = box_dp.filter(|v| *v > 0.0)?;
        if available.is_empty() {
            return None;
        }
        let need = box_dp * scale.min(2.0);
        SIZE_LADDER.iter().copied().find(|s| *s as f32 >= need && available.contains(s))
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Dark adaptation
// ═════════════════════════════════════════════════════════════════════════════

/// Making a logo drawn for white paper readable on the dark face, from
/// `services/logoDark/`.
///
/// The whole tree is framework-free maths and was already unit-tested in Node, so
/// this is a 1:1 port rather than a reinterpretation. Every constant is the
/// HANDOFF DOCUMENT'S, not the old python file's — CarFM's own header says the doc
/// wins where the two disagree, and the one that bites is the neutral chroma
/// threshold: 0.03, not the 0.045 left in a stale signature.
///
/// HUE IS NEVER MODIFIED. All colour work is in OKLab and touches only L or the
/// MAGNITUDE of (a, b); scaling chroma is `a, b *= k`, which preserves the angle.
/// A station's brand hue survives the treatment intact — that was the handoff's
/// one hard rule and it is the reason none of this is done in HSL.
pub mod dark {
    /// The dark surface a treatment is judged against: `Pal.panel` in the dark
    /// theme, `#212B38`, which `ui/tokens.slint:58` carries as the identical
    /// value.
    ///
    /// The STORED png is background-independent — `remap` clears paper to
    /// transparency rather than darkening it — so this colour is only ever the
    /// gate's compositing surface. A future night theme would pass its own and get
    /// its own cached variant; never hardcode it downstream.
    pub const LOGO_DARK_BG: [f64; 3] = [33.0 / 255.0, 43.0 / 255.0, 56.0 / 255.0];

    /// sRGB ⇄ OKLab, Björn Ottosson's matrices.
    pub mod oklab {
        use super::super::js_round_u8;

        /// 8-bit sRGB → linear, as a table. The pipeline converts whole images
        /// twice over, and `powf` per channel per pixel is the single hottest
        /// thing in the file.
        fn srgb_to_linear_u8(v: u8) -> f64 {
            // A `OnceLock` table would save the branch; at 1024×1024 the measured
            // difference was noise next to the two connected-component passes, so
            // this stays a plain function and the file keeps one less static.
            unit_to_linear(v as f64 / 255.0)
        }

        pub fn unit_to_linear(c: f64) -> f64 {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }

        /// Linear (0..1) → 8-bit sRGB. `Math.round` then clamp — half UP, which is
        /// NOT the `Uint8ClampedArray` rule the resampler uses.
        pub fn linear_to_srgb8(x: f64) -> u8 {
            let c = if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
            js_round_u8(c * 255.0)
        }

        fn linear_to_lab(r: f64, g: f64, b: f64) -> [f64; 3] {
            let l_ = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
            let m_ = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
            let s_ = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
            let l = l_.cbrt();
            let m = m_.cbrt();
            let s = s_.cbrt();
            [
                0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
                1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
                0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
            ]
        }

        fn lab_to_linear(l: f64, a: f64, b: f64) -> [f64; 3] {
            let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
            let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
            let s_ = l - 0.0894841775 * a - 1.291485548 * b;
            let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
            [
                4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
                -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
                -0.0041960863 * l3 - 0.7034186147 * m3 + 1.707614701 * s3,
            ]
        }

        pub fn srgb8_to_lab(r: u8, g: u8, b: u8) -> [f64; 3] {
            linear_to_lab(srgb_to_linear_u8(r), srgb_to_linear_u8(g), srgb_to_linear_u8(b))
        }

        /// For colours the pipeline COMPUTES rather than reads — median
        /// backgrounds, the theme colour, a composite over the dark surface.
        pub fn srgb_unit_to_lab(r: f64, g: f64, b: f64) -> [f64; 3] {
            linear_to_lab(unit_to_linear(r), unit_to_linear(g), unit_to_linear(b))
        }

        pub fn lab_to_srgb8(l: f64, a: f64, b: f64) -> [u8; 3] {
            let lin = lab_to_linear(l, a, b);
            [linear_to_srgb8(lin[0]), linear_to_srgb8(lin[1]), linear_to_srgb8(lin[2])]
        }

        /// Euclidean OKLab distance — the "within 0.0x" metric every threshold in
        /// this pipeline is expressed in.
        pub fn lab_dist(a: [f64; 3], b: [f64; 3]) -> f64 {
            let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        }

        /// OKLCh chroma. The pipeline classifies "neutral" by this magnitude and
        /// never needs the hue angle — which is precisely why the angle is safe.
        pub fn chroma(a: f64, b: f64) -> f64 {
            (a * a + b * b).sqrt()
        }
    }

    /// A three-pass box blur, which approximates a Gaussian closely enough at the
    /// 1.0 px and 2.5 px radii used here.
    ///
    /// The handoff recommends this over `ScriptIntrinsicBlur` (deprecated) and
    /// API-31 `RenderEffect` (too new) rather than version-branch for two small
    /// blurs. Separable, running-sum, so cost is independent of radius.
    pub mod blur {
        pub fn box_blur(src: &[f32], w: usize, h: usize, radius: f64, passes: usize) -> Vec<f32> {
            let mut cur = src.to_vec();
            if radius <= 0.0 {
                return cur;
            }
            // `Math.max(1, Math.round(radius))`: 2.5 rounds to 3, half up.
            let r = (radius.round() as isize).max(1) as usize;
            let mut tmp = vec![0f32; w * h];
            for _ in 0..passes {
                blur_h(&cur, &mut tmp, w, h, r);
                blur_v(&tmp, &mut cur, w, h, r);
            }
            cur
        }

        fn clamp(v: isize, lo: isize, hi: isize) -> usize {
            v.max(lo).min(hi) as usize
        }

        fn blur_h(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
            let norm = 1.0 / (2 * r + 1) as f32;
            let (ri, wi) = (r as isize, w as isize - 1);
            for y in 0..h {
                let row = y * w;
                let mut sum = 0f32;
                for k in -ri..=ri {
                    sum += src[row + clamp(k, 0, wi)];
                }
                for x in 0..w {
                    dst[row + x] = sum * norm;
                    sum += src[row + clamp(x as isize + ri + 1, 0, wi)]
                        - src[row + clamp(x as isize - ri, 0, wi)];
                }
            }
        }

        fn blur_v(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
            let norm = 1.0 / (2 * r + 1) as f32;
            let (ri, hi) = (r as isize, h as isize - 1);
            for x in 0..w {
                let mut sum = 0f32;
                for k in -ri..=ri {
                    sum += src[clamp(k, 0, hi) * w + x];
                }
                for y in 0..h {
                    dst[y * w + x] = sum * norm;
                    sum += src[clamp(y as isize + ri + 1, 0, hi) * w + x]
                        - src[clamp(y as isize - ri, 0, hi) * w + x];
                }
            }
        }
    }

    /// The two connected-component primitives the handoff flags as "the one real
    /// gap" — neither is in the Android SDK, and pulling OpenCV in for them would
    /// have cost 10–20 MB per ABI. Both are 4-connectivity, which the doc says is
    /// sufficient.
    pub mod labeling {
        /// Border-connected subset of `mask`: 1 where the pixel is masked AND
        /// reachable from the image border through other masked pixels.
        ///
        /// This is what makes keying safe. A flood from the border cannot reach a
        /// letter's enclosed counter, so the hole in a white "O" on a white plate
        /// stays opaque instead of being punched out.
        ///
        /// Iterative stack, no recursion: a 1-megapixel image would blow the call
        /// stack otherwise, and logos that big are routine.
        pub fn border_connected_mask(mask: &[u8], w: usize, h: usize) -> Vec<u8> {
            let mut out = vec![0u8; w * h];
            let mut stack: Vec<usize> = Vec::new();
            macro_rules! push {
                ($i:expr) => {{
                    let i = $i;
                    if mask[i] != 0 && out[i] == 0 {
                        out[i] = 1;
                        stack.push(i);
                    }
                }};
            }
            for x in 0..w {
                push!(x);
                push!((h - 1) * w + x);
            }
            for y in 0..h {
                push!(y * w);
                push!(y * w + (w - 1));
            }
            while let Some(i) = stack.pop() {
                let x = i % w;
                let y = i / w;
                if x > 0 {
                    push!(i - 1);
                }
                if x < w - 1 {
                    push!(i + 1);
                }
                if y > 0 {
                    push!(i - w);
                }
                if y < h - 1 {
                    push!(i + w);
                }
            }
            out
        }

        pub struct Components {
            /// Per-pixel label; 0 = not in the mask, ≥1 = component id.
            pub labels: Vec<u32>,
            /// `areas[id]` = pixel count. `areas[0]` is unused.
            pub areas: Vec<usize>,
            pub count: usize,
        }

        /// Two-pass union-find labeling, 4-connectivity — so `remap` can protect
        /// a light-neutral blob that is large enough to be a wordmark.
        pub fn connected_components(mask: &[u8], w: usize, h: usize) -> Components {
            let n = w * h;
            let mut parent: Vec<u32> = vec![0; n + 1];
            let mut next: u32 = 0;
            let mut prov: Vec<u32> = vec![0; n];

            fn find(parent: &mut [u32], mut x: u32) -> u32 {
                while parent[x as usize] != x {
                    parent[x as usize] = parent[parent[x as usize] as usize];
                    x = parent[x as usize];
                }
                x
            }
            fn union(parent: &mut [u32], a: u32, b: u32) {
                let ra = find(parent, a);
                let rb = find(parent, b);
                if ra != rb {
                    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                    parent[hi as usize] = lo;
                }
            }

            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if mask[i] == 0 {
                        continue;
                    }
                    let west = if x > 0 && mask[i - 1] != 0 { prov[i - 1] } else { 0 };
                    let north = if y > 0 && mask[i - w] != 0 { prov[i - w] } else { 0 };
                    match (west, north) {
                        (0, 0) => {
                            next += 1;
                            parent[next as usize] = next;
                            prov[i] = next;
                        }
                        (wv, 0) => prov[i] = wv,
                        (0, nv) => prov[i] = nv,
                        (wv, nv) => {
                            prov[i] = wv;
                            if wv != nv {
                                union(&mut parent, wv, nv);
                            }
                        }
                    }
                }
            }

            let mut remap: Vec<u32> = vec![0; next as usize + 1];
            let mut labels: Vec<u32> = vec![0; n];
            let mut areas: Vec<usize> = vec![0];
            let mut count: usize = 0;
            for i in 0..n {
                if prov[i] == 0 {
                    continue;
                }
                let root = find(&mut parent, prov[i]);
                let mut id = remap[root as usize];
                if id == 0 {
                    count += 1;
                    id = count as u32;
                    remap[root as usize] = id;
                    areas.push(0);
                }
                labels[i] = id;
                areas[id as usize] += 1;
            }
            Components { labels, areas, count }
        }
    }

    /// The five stages, in order: flatten a baked-in checkerboard, trim uniform
    /// padding, key the border-connected background to transparency, route on the
    /// remaining coverage, then build and gate the treatments.
    pub mod stages {
        use super::super::{js_round_u8, Raster};
        use super::blur::box_blur;
        use super::labeling::{border_connected_mask, connected_components};
        use super::oklab::{chroma, lab_dist, lab_to_srgb8, srgb8_to_lab, srgb_unit_to_lab};
        use std::collections::HashMap;

        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Treatment {
            /// Lightness remap: clear the paper, lift the ink, keep the hue.
            Remap,
            /// A light glow under untouched ink.
            Halo,
            /// The keyed image, unmodified.
            AsIs,
            /// A grey rounded plate behind the original — the guaranteed floor.
            Plate,
        }

        impl Treatment {
            /// The persisted spelling. `AUTO` and `CUSTOM` exist in the stored
            /// vocabulary but the pipeline never emits them: AUTO means "trust the
            /// pick" and is recorded as `chosen = false` instead, and CUSTOM is
            /// reserved for a hand-edited variant that does not exist yet.
            pub fn as_enum(self) -> &'static str {
                match self {
                    Treatment::Remap => "REMAP",
                    Treatment::Halo => "HALO",
                    Treatment::AsIs => "AS_IS",
                    Treatment::Plate => "PLATE",
                }
            }
            pub fn from_enum(s: &str) -> Option<Treatment> {
                match s {
                    "REMAP" => Some(Treatment::Remap),
                    "HALO" => Some(Treatment::Halo),
                    "AS_IS" => Some(Treatment::AsIs),
                    "PLATE" => Some(Treatment::Plate),
                    _ => None,
                }
            }
        }

        /// Plate geometry. PARAMS, NOT PIXELS: the plate is a rounded rectangle
        /// the UI draws behind the logo, so its corners stay vector-sharp and its
        /// fill can follow the theme. Baking it into the PNG would throw both away.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct PlateParams {
            pub pad_frac: f64,
            pub radius_frac: f64,
            pub fill: [f64; 3],
        }

        pub fn plate_params() -> PlateParams {
            PlateParams { pad_frac: 0.09, radius_frac: 0.14, fill: [0.90, 0.90, 0.90] }
        }

        #[derive(Clone, Debug)]
        pub struct Candidate {
            pub treatment: Treatment,
            pub raster: Raster,
            pub plate: Option<PlateParams>,
        }

        #[derive(Clone, Debug, PartialEq)]
        pub struct CheckerInfo {
            pub detected: bool,
            pub cell: Option<usize>,
            pub note: String,
        }

        /// Detect a baked-in editor checkerboard — two near-neutral colours in a
        /// periodic grid — and flatten B into A so the keying stage sees one
        /// uniform background.
        ///
        /// Without this, a logo exported from an editor with the transparency
        /// checkerboard baked in reads as content: the trim cannot crop it and the
        /// key cannot lift it, and the station ends up with a grey chequered tile.
        pub fn flatten_checkerboard(img: &Raster) -> (Raster, CheckerInfo) {
            let fail = |why: &str| {
                (img.clone(), CheckerInfo { detected: false, cell: None, note: why.to_string() })
            };
            if !img.is_valid() {
                return fail("empty raster");
            }
            let n = img.pixels();
            let px = &img.rgba;

            // Exact opaque colours, counted IN FIRST-APPEARANCE ORDER. The order
            // is load-bearing: the top-two selection below breaks ties with a
            // strict `>`, so whichever colour was seen first wins. A HashMap's
            // iteration order would make the choice — and the flattened output —
            // vary run to run.
            let mut order: Vec<u32> = Vec::new();
            let mut counts: HashMap<u32, usize> = HashMap::new();
            let mut opaque = 0usize;
            for i in 0..n {
                let j = i * 4;
                if px[j + 3] < 128 {
                    continue;
                }
                opaque += 1;
                let key =
                    ((px[j] as u32) << 16) | ((px[j + 1] as u32) << 8) | px[j + 2] as u32;
                match counts.get_mut(&key) {
                    Some(c) => *c += 1,
                    None => {
                        counts.insert(key, 1);
                        order.push(key);
                    }
                }
            }
            if opaque == 0 {
                return fail("fully transparent");
            }
            let (mut a, mut b) = (-1i64, -1i64);
            let (mut ca, mut cb) = (0usize, 0usize);
            for k in &order {
                let c = counts[k];
                if c > ca {
                    b = a;
                    cb = ca;
                    a = *k as i64;
                    ca = c;
                } else if c > cb {
                    b = *k as i64;
                    cb = c;
                }
            }
            if b < 0 {
                return fail("single colour");
            }
            let col_a = [((a >> 16) & 255) as u8, ((a >> 8) & 255) as u8, (a & 255) as u8];
            let col_b = [((b >> 16) & 255) as u8, ((b >> 8) & 255) as u8, (b & 255) as u8];
            let lab_a = srgb8_to_lab(col_a[0], col_a[1], col_a[2]);
            let lab_b = srgb8_to_lab(col_b[0], col_b[1], col_b[2]);

            // Both near-neutral, ≥40% together, ≥10% for B alone, |ΔL| ≥ 0.02.
            // The last test is what separates a checkerboard from a flat
            // background that merely happens to have two near-identical shades.
            if chroma(lab_a[1], lab_a[2]) >= 0.05 || chroma(lab_b[1], lab_b[2]) >= 0.05 {
                return fail("colours not neutral");
            }
            if ((ca + cb) as f64 / opaque as f64) < 0.40 {
                return fail("top-2 cover <40%");
            }
            if (cb as f64 / opaque as f64) < 0.10 {
                return fail("2nd colour <10%");
            }
            if (lab_a[0] - lab_b[0]).abs() < 0.02 {
                return fail("ΔL <0.02 (one flat bg)");
            }

            // Periodicity: the modal run length of the B-mask along rows AND
            // columns must exist and be EQUAL. That pair is the checkerboard
            // signature; a two-tone gradient or a striped background fails it.
            let is_b = |i: usize| {
                px[i * 4] == col_b[0] && px[i * 4 + 1] == col_b[1] && px[i * 4 + 2] == col_b[2]
            };
            let row_mode = modal_run(img.w as usize, img.h as usize, true, &is_b);
            let col_mode = modal_run(img.w as usize, img.h as usize, false, &is_b);
            if row_mode == 0 || col_mode == 0 || row_mode != col_mode {
                return (
                    img.clone(),
                    CheckerInfo {
                        detected: false,
                        cell: None,
                        note: format!("no matching period (row={row_mode} col={col_mode})"),
                    },
                );
            }

            let mut out = img.rgba.clone();
            for i in 0..n {
                let j = i * 4;
                if px[j + 3] < 128 {
                    continue;
                }
                if lab_dist(srgb8_to_lab(px[j], px[j + 1], px[j + 2]), lab_b) <= 0.02 {
                    out[j] = col_a[0];
                    out[j + 1] = col_a[1];
                    out[j + 2] = col_a[2];
                }
            }
            (
                Raster { w: img.w, h: img.h, rgba: out },
                CheckerInfo {
                    detected: true,
                    cell: Some(row_mode),
                    note: format!("{row_mode}px cell"),
                },
            )
        }

        /// Modal run length of a predicate along rows or columns, sampling up to
        /// 64 lines and ignoring runs under 4 px. 0 when there is none.
        fn modal_run(w: usize, h: usize, horizontal: bool, pred: &dyn Fn(usize) -> bool) -> usize {
            let lines = if horizontal { h } else { w };
            let span = if horizontal { w } else { h };
            let step = (lines / 64).max(1);
            // First-seen order again, for the same tie-break reason as above.
            let mut order: Vec<usize> = Vec::new();
            let mut hist: HashMap<usize, usize> = HashMap::new();
            let bump = |run: usize, order: &mut Vec<usize>, hist: &mut HashMap<usize, usize>| {
                if run >= 4 {
                    match hist.get_mut(&run) {
                        Some(c) => *c += 1,
                        None => {
                            hist.insert(run, 1);
                            order.push(run);
                        }
                    }
                }
            };
            let mut l = 0usize;
            while l < lines {
                let mut run = 0usize;
                for s in 0..span {
                    let i = if horizontal { l * w + s } else { s * w + l };
                    if pred(i) {
                        run += 1;
                    } else {
                        bump(run, &mut order, &mut hist);
                        run = 0;
                    }
                }
                bump(run, &mut order, &mut hist);
                l += step;
            }
            let mut best = 0usize;
            let mut best_c = 0usize;
            for len in &order {
                let c = hist[len];
                if c > best_c {
                    best = *len;
                    best_c = c;
                }
            }
            best
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Bbox {
            pub x0: u32,
            pub y0: u32,
            pub x1: u32,
            pub y1: u32,
        }

        /// Crop uniform border padding. Content = differs from the corner median
        /// by more than 0.04 in any channel, OR is already partly transparent.
        ///
        /// The transparency clause is why this is NOT `prep::mark_bounds`: here a
        /// transparent pixel must count as content, because transparency is what
        /// the next stage operates on.
        pub fn trim(img: &Raster) -> (Raster, Bbox) {
            let (w, h) = (img.w, img.h);
            let full = Bbox { x0: 0, y0: 0, x1: w.saturating_sub(1), y1: h.saturating_sub(1) };
            if !img.is_valid() {
                return (img.clone(), full);
            }
            let px = &img.rgba;
            let corners = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)];
            let med: [f64; 3] = std::array::from_fn(|c| {
                let mut v: Vec<f64> = corners
                    .iter()
                    .map(|(x, y)| px[img.at(*x, *y) + c] as f64 / 255.0)
                    .collect();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                (v[1] + v[2]) / 2.0
            });

            let (mut x0, mut y0) = (w, h);
            let (mut x1, mut y1) = (-1i64, -1i64);
            for y in 0..h {
                for x in 0..w {
                    let j = img.at(x, y);
                    let transparent = px[j + 3] < 250;
                    let diff = (0..3)
                        .map(|c| (px[j + c] as f64 / 255.0 - med[c]).abs())
                        .fold(0.0f64, f64::max);
                    if transparent || diff > 0.04 {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x as i64);
                        y1 = y1.max(y as i64);
                    }
                }
            }
            if x1 < 0 {
                return (img.clone(), full); // all uniform
            }
            let bbox = Bbox { x0, y0, x1: x1 as u32, y1: y1 as u32 };
            if bbox == full {
                return (img.clone(), bbox);
            }
            (
                super::super::prep::crop_raster(
                    img,
                    super::super::prep::Bounds {
                        x0: bbox.x0,
                        y0: bbox.y0,
                        x1: bbox.x1,
                        y1: bbox.y1,
                    },
                ),
                bbox,
            )
        }

        #[derive(Clone, Debug, PartialEq)]
        pub struct KeyInfo {
            pub keyed: bool,
            /// Opaque fraction AFTER keying — the routing input.
            pub coverage: f64,
            pub bg: [f64; 3],
            pub note: String,
        }

        /// Flood the border-connected background to alpha 0, feather it, and
        /// decontaminate the partial-alpha edge.
        ///
        /// Two abort paths, and both matter. Fewer than 3 of 4 corners agreeing
        /// means there is no single background to key — a diagonal split, a photo —
        /// and keying anyway would eat half the image. A region outside [2%, 92%]
        /// means the mask found either nothing or everything, and neither is a
        /// background.
        pub fn key_background(img: &Raster) -> (Raster, KeyInfo) {
            let (w, h) = (img.w as usize, img.h as usize);
            let n = w * h;
            let coverage_of = |r: &Raster| -> f64 {
                let o = (0..r.pixels()).filter(|i| r.rgba[i * 4 + 3] >= 128).count();
                o as f64 / r.pixels() as f64
            };
            if !img.is_valid() {
                return (
                    img.clone(),
                    KeyInfo {
                        keyed: false,
                        coverage: 0.0,
                        bg: [0.0; 3],
                        note: "empty raster".into(),
                    },
                );
            }
            let px = &img.rgba;

            // Median border colour over ALL border pixels — transparent ones
            // included, unlike `prep::mark_bounds`, because here a transparent
            // frame is a perfectly good background to key against.
            let mut bs: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            {
                let mut add = |x: usize, y: usize| {
                    let j = (y * w + x) * 4;
                    for (c, v) in bs.iter_mut().enumerate() {
                        v.push(px[j + c] as f64 / 255.0);
                    }
                };
                for x in 0..w {
                    add(x, 0);
                    add(x, h - 1);
                }
                for y in 0..h {
                    add(0, y);
                    add(w - 1, y);
                }
            }
            let bg: [f64; 3] = std::array::from_fn(|c| {
                bs[c].sort_by(|a, b| a.partial_cmp(b).unwrap());
                bs[c][bs[c].len() >> 1]
            });
            let bg_lab = srgb_unit_to_lab(bg[0], bg[1], bg[2]);

            let corner_ok = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)]
                .iter()
                .filter(|(x, y)| {
                    let j = (y * w + x) * 4;
                    lab_dist(srgb8_to_lab(px[j], px[j + 1], px[j + 2]), bg_lab) <= 0.05
                })
                .count();
            if corner_ok < 3 {
                let cov = coverage_of(img);
                return (
                    img.clone(),
                    KeyInfo {
                        keyed: false,
                        coverage: cov,
                        bg,
                        note: "corners disagree — left opaque".into(),
                    },
                );
            }

            let mut near = vec![0u8; n];
            for (i, slot) in near.iter_mut().enumerate() {
                let j = i * 4;
                if px[j + 3] < 128 {
                    *slot = 1; // already transparent counts as background
                    continue;
                }
                if lab_dist(srgb8_to_lab(px[j], px[j + 1], px[j + 2]), bg_lab) <= 0.10 {
                    *slot = 1;
                }
            }
            let bg_mask = border_connected_mask(&near, w, h);
            let removed: usize = bg_mask.iter().map(|v| *v as usize).sum();
            let frac = removed as f64 / n as f64;
            if !(0.02..=0.92).contains(&frac) {
                let cov = coverage_of(img);
                return (
                    img.clone(),
                    KeyInfo {
                        keyed: false,
                        coverage: cov,
                        bg,
                        note: format!(
                            "region {}% out of [2,92]% — left opaque",
                            (frac * 100.0) as i64
                        ),
                    },
                );
            }

            let mut alpha = vec![0f32; n];
            for i in 0..n {
                alpha[i] = if bg_mask[i] != 0 { 0.0 } else { px[i * 4 + 3] as f32 / 255.0 };
            }
            let blurred = box_blur(&alpha, w, h, 1.0, 3);
            let mut out = img.rgba.clone();
            for (i, &soft) in blurred.iter().enumerate() {
                let j = i * 4;
                let a = (((soft as f64) - 0.25) / 0.5).clamp(0.0, 1.0);
                // Decontaminate the feathered edge: a half-transparent pixel still
                // carries half the old background's colour, which on a dark face
                // shows as a pale fringe around the mark.
                if a > 0.001 && a < 0.999 {
                    let d = a.max(0.08);
                    for c in 0..3 {
                        let v = (px[j + c] as f64 / 255.0 - bg[c] * (1.0 - a)) / d;
                        out[j + c] = js_round_u8(v.clamp(0.0, 1.0) * 255.0);
                    }
                }
                out[j + 3] = js_round_u8(a * 255.0);
            }
            let raster = Raster { w: img.w, h: img.h, rgba: out };
            let cov = coverage_of(&raster);
            (
                raster,
                KeyInfo {
                    keyed: true,
                    coverage: cov,
                    bg,
                    note: format!("keyed {}% of pixels", (frac * 100.0) as i64),
                },
            )
        }

        /// Coverage decides the family, and the threshold is the whole point: a
        /// mostly-opaque image is a TILE, which a halo can only sit behind, while
        /// isolated ink can actually be remapped.
        pub fn route(coverage: f64) -> Vec<Treatment> {
            if coverage > 0.80 {
                vec![Treatment::Halo, Treatment::AsIs]
            } else {
                vec![Treatment::Remap, Treatment::Halo]
            }
        }

        /// Lightness remap that keeps the brand hue. Two steps:
        ///
        /// 1. CLEAR THE PAPER to transparency. Light near-neutral ink — letter
        ///    counters, ring gaps, the white field behind a mark — fades to alpha 0
        ///    on a lightness ramp, so the real dark surface shows through. It is
        ///    never DARKENED to black, because that only works on a black theme,
        ///    and it is why the stored PNG is background-independent and the ladder
        ///    can be pre-rendered.
        /// 2. LIFT the remaining dark ink, and never darken anything.
        ///
        /// A large light-neutral component is PROTECTED: that is a white wordmark,
        /// and clearing it would delete the station's name.
        pub fn remap(img: &Raster) -> Raster {
            const NEUTRAL_C: f64 = 0.03; // the document's value, NOT the stale 0.045
            const SOFT_C: f64 = 0.048; // only near-neutral pixels may be cleared
            let n = img.pixels();
            let (w, h) = (img.w as usize, img.h as usize);
            let px = &img.rgba;

            let mut lp = vec![0f64; n];
            let mut ap = vec![0f64; n];
            let mut bp = vec![0f64; n];
            let mut alpha = vec![0f64; n];
            let mut light_neutral = vec![0u8; n];
            for i in 0..n {
                let j = i * 4;
                let lab = srgb8_to_lab(px[j], px[j + 1], px[j + 2]);
                lp[i] = lab[0];
                ap[i] = lab[1];
                bp[i] = lab[2];
                alpha[i] = px[j + 3] as f64 / 255.0;
                if alpha[i] > 0.5 && chroma(lab[1], lab[2]) < NEUTRAL_C && lab[0] > 0.72 {
                    light_neutral[i] = 1;
                }
            }
            let cc = connected_components(&light_neutral, w, h);
            let mut protect = vec![0u8; cc.count + 1];
            // `skip(1)`, because label 0 is "not a component" and is never
            // protected — the same range `1..=cc.count` walked.
            for (id, slot) in protect.iter_mut().enumerate().skip(1) {
                if cc.areas[id] as f64 >= 0.08 * n as f64 {
                    *slot = 1;
                }
            }

            // Paper colour = mean of the UNPROTECTED light-neutral pixels, used to
            // decontaminate partly-cleared edges. White when there are none.
            let (mut pr, mut pg, mut pb, mut pc) = (0.0f64, 0.0f64, 0.0f64, 0usize);
            for i in 0..n {
                if light_neutral[i] != 0 && protect[cc.labels[i] as usize] == 0 {
                    let j = i * 4;
                    pr += px[j] as f64 / 255.0;
                    pg += px[j + 1] as f64 / 255.0;
                    pb += px[j + 2] as f64 / 255.0;
                    pc += 1;
                }
            }
            let paper: [f64; 3] = if pc > 0 {
                [pr / pc as f64, pg / pc as f64, pb / pc as f64]
            } else {
                [1.0, 1.0, 1.0]
            };

            let mut new_a = alpha.clone();
            for i in 0..n {
                if protect[cc.labels[i] as usize] != 0 {
                    continue;
                }
                if alpha[i] > 0.5 && chroma(ap[i], bp[i]) < SOFT_C {
                    // 0 at L ≤ 0.55, 1 at L ≥ 0.85.
                    let cov = ((lp[i] - 0.55) / 0.30).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        new_a[i] = alpha[i] * (1.0 - cov);
                    }
                }
            }

            let mut out = img.rgba.clone();
            for i in 0..n {
                let j = i * 4;
                let a = new_a[i];
                out[j + 3] = js_round_u8(a * 255.0);
                if protect[cc.labels[i] as usize] != 0 {
                    continue; // protected: keep the original lightness and chroma
                }
                if new_a[i] < alpha[i] - 1e-6 {
                    if a > 0.001 {
                        let d = a.max(0.10);
                        for c in 0..3 {
                            let v = (px[j + c] as f64 / 255.0 - paper[c] * (1.0 - a)) / d;
                            out[j + c] = js_round_u8(v.clamp(0.0, 1.0) * 255.0);
                        }
                    }
                    continue;
                }
                if a <= 0.5 {
                    continue; // keyed background — leave it alone
                }
                let old_l = lp[i];
                let c = chroma(ap[i], bp[i]);
                let neutral = c < NEUTRAL_C;
                let mut new_l = if neutral && old_l < 0.5 {
                    1.0 - old_l // dark neutral ink inverts
                } else if !neutral && old_l < 0.45 {
                    old_l.max(0.62) // dark brand colour lifts to a readable floor
                } else {
                    old_l
                };
                new_l = new_l.max(old_l); // never darken
                let (mut av, mut bv) = (ap[i], bp[i]);
                if new_l > old_l {
                    // Lightening a colour without dropping chroma reads as
                    // fluorescent; 0.85 is the doc's damping.
                    av *= 0.85;
                    bv *= 0.85;
                }
                new_l = new_l.min(0.92);
                let rgb = lab_to_srgb8(new_l, av, bv);
                out[j] = rgb[0];
                out[j + 1] = rgb[1];
                out[j + 2] = rgb[2];
            }
            Raster { w: img.w, h: img.h, rgba: out }
        }

        /// A non-destructive light glow under the original ink. Colours untouched —
        /// this is the treatment for art that must not be altered at all.
        pub fn halo(img: &Raster) -> Raster {
            let n = img.pixels();
            let (w, h) = (img.w as usize, img.h as usize);
            let px = &img.rgba;
            let mut alpha = vec![0f32; n];
            for i in 0..n {
                alpha[i] = px[i * 4 + 3] as f32 / 255.0;
            }
            let blurred = box_blur(&alpha, w, h, 2.5, 3);
            const GLOW: [f64; 3] = [0.92, 0.92, 0.92];
            let mut out = img.rgba.clone();
            for i in 0..n {
                let j = i * 4;
                let oa = alpha[i] as f64;
                let glow = ((blurred[i] as f64) * 1.2).clamp(0.0, 1.0) * (1.0 - oa);
                let out_a = oa + glow * (1.0 - oa);
                if out_a <= 0.0001 {
                    out[j + 3] = 0;
                    continue;
                }
                for c in 0..3 {
                    let orig = px[j + c] as f64 / 255.0;
                    let v = (orig * oa + GLOW[c] * glow * (1.0 - oa)) / out_a;
                    out[j + c] = js_round_u8(v.clamp(0.0, 1.0) * 255.0);
                }
                out[j + 3] = js_round_u8(out_a * 255.0);
            }
            Raster { w: img.w, h: img.h, rgba: out }
        }

        /// A CATASTROPHE CHECK, NEVER A RANKER.
        ///
        /// CarFM's note is worth keeping verbatim: the score "was wrong five
        /// separate times in review". So this answers one question — is the
        /// composite still readable at all — and the routing order decides
        /// everything else. `plate` always passes and is the guaranteed floor.
        ///
        /// Ink is eroded by 1 px before measuring so an antialiased edge, which is
        /// half background by construction, cannot fail a perfectly good mark.
        pub fn gate(cand: &Candidate, bg: [f64; 3]) -> (bool, String) {
            if cand.treatment == Treatment::Plate {
                return (true, "plate is the guaranteed floor".into());
            }
            let img = &cand.raster;
            if !img.is_valid() {
                return (false, "no raster".into());
            }
            let (w, h) = (img.w as usize, img.h as usize);
            let n = w * h;
            let px = &img.rgba;
            let bg_lab = srgb_unit_to_lab(bg[0], bg[1], bg[2]);

            let mut ink = vec![0u8; n];
            for i in 0..n {
                ink[i] = if px[i * 4 + 3] > 128 { 1 } else { 0 };
            }
            let mut eroded = vec![0u8; n];
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if ink[i] == 0 {
                        continue;
                    }
                    if x > 0 && ink[i - 1] == 0 {
                        continue;
                    }
                    if x < w - 1 && ink[i + 1] == 0 {
                        continue;
                    }
                    if y > 0 && ink[i - w] == 0 {
                        continue;
                    }
                    if y < h - 1 && ink[i + w] == 0 {
                        continue;
                    }
                    eroded[i] = 1;
                }
            }

            let (mut neutral_tot, mut neutral_pass) = (0usize, 0usize);
            let (mut chrom_tot, mut chrom_pass) = (0usize, 0usize);
            let (mut ink_tot, mut blowout) = (0usize, 0usize);
            for (i, &kept) in eroded.iter().enumerate() {
                if kept == 0 {
                    continue;
                }
                let j = i * 4;
                let a = px[j + 3] as f64 / 255.0;
                let r = px[j] as f64 / 255.0 * a + bg[0] * (1.0 - a);
                let g = px[j + 1] as f64 / 255.0 * a + bg[1] * (1.0 - a);
                let b = px[j + 2] as f64 / 255.0 * a + bg[2] * (1.0 - a);
                let lab = srgb_unit_to_lab(r, g, b);
                ink_tot += 1;
                if lab[0] > 0.95 {
                    blowout += 1;
                }
                let readable = (lab[0] - bg_lab[0]).abs() > 0.32;
                if chroma(lab[1], lab[2]) < 0.04 {
                    neutral_tot += 1;
                    if readable {
                        neutral_pass += 1;
                    }
                } else {
                    chrom_tot += 1;
                    if readable {
                        chrom_pass += 1;
                    }
                }
            }
            if ink_tot == 0 {
                return (false, "no ink".into());
            }
            // Both ink groups are judged separately and the WORSE one decides, so
            // a treatment that saves the wordmark and destroys the roundel fails.
            // A group under 8% of the ink is noise and does not get a vote.
            let mut groups: Vec<f64> = Vec::new();
            if neutral_tot as f64 >= 0.08 * ink_tot as f64 {
                groups.push(neutral_pass as f64 / neutral_tot as f64);
            }
            if chrom_tot as f64 >= 0.08 * ink_tot as f64 {
                groups.push(chrom_pass as f64 / chrom_tot as f64);
            }
            let min_group = groups.iter().copied().fold(f64::INFINITY, f64::min);
            let min_group = if groups.is_empty() { 0.0 } else { min_group };
            let blow = blowout as f64 / ink_tot as f64;
            let pass = min_group >= 0.45 && blow < 0.45;
            (pass, format!("minGroup={min_group:.2} blowout={blow:.2}"))
        }

    }

    /// The orchestrator, and the rule for keeping a human's override.
    pub mod pipeline {
        use super::super::Raster;
        use super::stages::{
            flatten_checkerboard, gate, halo, key_background, plate_params, remap, route, Bbox,
            Candidate, CheckerInfo, KeyInfo, Treatment,
        };

        #[derive(Clone, Debug)]
        pub struct AdaptResult {
            pub bg: [f64; 3],
            pub checkerboard: CheckerInfo,
            pub bbox: Bbox,
            pub key: KeyInfo,
            pub coverage: f64,
            /// The routing order considered, excluding the plate floor.
            pub order: Vec<Treatment>,
            /// Every built candidate: routed ones first, then the plate floor.
            pub candidates: Vec<Candidate>,
            /// First routed candidate that passed the gate, else `Plate`.
            pub pick: Treatment,
            /// Per-treatment gate notes, for logs.
            pub gates: Vec<(Treatment, String)>,
        }

        fn build(t: Treatment, keyed: &Raster) -> Candidate {
            match t {
                Treatment::Remap => {
                    Candidate { treatment: t, raster: remap(keyed), plate: None }
                }
                Treatment::Halo => Candidate { treatment: t, raster: halo(keyed), plate: None },
                Treatment::AsIs => {
                    Candidate { treatment: t, raster: keyed.clone(), plate: None }
                }
                // The plate's PIXELS are the keyed image — the grey rounded
                // rectangle is drawn behind it by the UI, never baked in.
                Treatment::Plate => Candidate {
                    treatment: t,
                    raster: keyed.clone(),
                    plate: Some(plate_params()),
                },
            }
        }

        /// Run the whole pipeline for a given dark background (unit sRGB, read
        /// from the theme token — never hardcoded at the call site).
        pub fn adapt_logo_for_dark(src: &Raster, bg: [f64; 3]) -> AdaptResult {
            let (r1, checkerboard) = flatten_checkerboard(src);
            let (r2, bbox) = super::stages::trim(&r1);
            let (keyed, key) = key_background(&r2);
            let coverage = key.coverage;
            let order = route(coverage);

            let mut candidates: Vec<Candidate> = order.iter().map(|t| build(*t, &keyed)).collect();
            let mut gates: Vec<(Treatment, String)> = Vec::new();
            let mut pick: Option<Treatment> = None;
            for c in &candidates {
                let (ok, note) = gate(c, bg);
                gates.push((c.treatment, format!("{}{note}", if ok { "PASS " } else { "fail " })));
                if ok && pick.is_none() {
                    pick = Some(c.treatment);
                }
            }
            let plate = build(Treatment::Plate, &keyed);
            let (_, plate_note) = gate(&plate, bg);
            candidates.push(plate);
            gates.push((Treatment::Plate, format!("PASS {plate_note}")));

            AdaptResult {
                bg,
                checkerboard,
                bbox,
                key,
                coverage,
                order,
                candidates,
                pick: pick.unwrap_or(Treatment::Plate),
                gates,
            }
        }

        /// Which treatment a re-adaptation should store, and whether it counts as
        /// the human's.
        ///
        /// A prior override survives if the same treatment is still on the table;
        /// otherwise the auto-pick takes over. This is the only place a user's
        /// choice can be lost, and it can only happen when the routing itself
        /// changed — a different image, or a different theme background.
        pub fn choose_treatment(
            prior: Option<(&str, bool)>,
            candidates: &[Candidate],
            auto_pick: Treatment,
        ) -> (Treatment, bool) {
            if let Some((prior_enum, true)) = prior {
                if let Some(c) =
                    candidates.iter().find(|c| c.treatment.as_enum() == prior_enum)
                {
                    return (c.treatment, true);
                }
            }
            (auto_pick, false)
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Per-station hero flags
// ═════════════════════════════════════════════════════════════════════════════

/// The two "Display Call Sign" / "Display Frequency" toggles from the logo
/// window's landing view (§6.4). They affect the HERO only.
pub mod prefs {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct HeroFlags {
        pub show_call: bool,
        pub show_freq: bool,
    }

    /// Both ON — the no-logo default, which is the safe one: a hero with neither
    /// its call sign nor its dial and no logo to show would be blank.
    impl Default for HeroFlags {
        fn default() -> Self {
            HeroFlags { show_call: true, show_freq: true }
        }
    }

    /// Three-valued, and the three values are genuinely different things:
    /// `Unknown` means nobody has looked, `Unset` means we looked and the user has
    /// never chosen, `Set` means they have.
    ///
    /// Collapsing `Unknown` and `Unset` is what produced the "back and forth
    /// sizing" reported from the car on 31 July: these flags pick the hero's logo
    /// SIZE TIER, so answering "unset" before the read came back rendered the logo
    /// at one size and then jumped it to another.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Lookup {
        Unknown,
        Unset,
        Set(HeroFlags),
    }

    /// The flags a station actually gets. An explicit choice always wins;
    /// otherwise the default is LOGO-DEPENDENT — a station with a logo shows the
    /// logo alone, which is §6.4's logo-only hero, and a station without one shows
    /// its call sign and dial because otherwise the hero would be blank.
    pub fn effective(stored: Option<HeroFlags>, has_logo: bool) -> HeroFlags {
        match stored {
            Some(f) => f,
            None if has_logo => HeroFlags { show_call: false, show_freq: false },
            None => HeroFlags { show_call: true, show_freq: true },
        }
    }

    /// What is written when a NEW logo is assigned.
    ///
    /// Always both off, never "whatever the toggles say". The toggles were seeded
    /// from the PRE-EXISTING logo, so a station that had none would arrive at this
    /// point holding (true, true) and would clobber the design default the moment
    /// it got its first logo.
    pub const ON_NEW_LOGO: HeroFlags = HeroFlags { show_call: false, show_freq: false };
}

// ═════════════════════════════════════════════════════════════════════════════
// The store
// ═════════════════════════════════════════════════════════════════════════════

/// The filesystem logo store, from `services/logoStore.ts`.
///
/// WHY FILES. CarFM kept logo images as SQLite blobs, which meant every render
/// handed React Native a base64 `data:` URI — 33% more bytes, the whole image
/// across the bridge as a string, and a re-decode every time because the native
/// image cache cannot key a data URI. Carnyx has no bridge, but the other half of
/// the reason still holds: a pre-rendered ladder means a preset strip decodes
/// small PNGs instead of full-resolution ones, once, off the render path.
///
/// ```text
/// <files>/carnyx-logos/<SAFE>/
///   original.<ext>   master — written once, NEVER altered
///   display.png      trimmed master (§4.5)
///   dark.png         dark-adapted master
///   d-<N>.png        display size ladder (N = longest edge)
///   k-<N>.png        dark size ladder
///   meta.json        mime, source, timestamps, aspect, dark treatment
/// <files>/carnyx-logos/index.json    read cache of every meta.json
/// <files>/carnyx-logos/prefs.json    per-station hero flags
/// ```
///
/// The directory name is `carnyx-logos`, not CarFM's `carfm-logos`: it lives in
/// this package's private storage, nothing else can read it, and naming another
/// app's product in our own tree would only mislead. The LAYOUT is CarFM's,
/// unchanged.
pub mod store {
    use super::dark::stages::Treatment;
    use super::json::{self, Json};
    use super::prefs::HeroFlags;
    use super::prep;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// Where a station's bytes and its metadata come from — injected so the store
    /// can be driven from a test without waiting on a wall clock.
    pub trait Clock: Send + Sync {
        fn now_ms(&self) -> u64;
    }

    pub struct SystemClock;
    impl Clock for SystemClock {
        fn now_ms(&self) -> u64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        }
    }

    /// A clock frozen at a value the test sets. The TTL logic is the only thing in
    /// this file that can be wrong in a way that only shows up a month later.
    pub struct FixedClock(pub std::sync::atomic::AtomicU64);
    impl FixedClock {
        pub fn new(ms: u64) -> FixedClock {
            FixedClock(std::sync::atomic::AtomicU64::new(ms))
        }
        pub fn advance(&self, ms: u64) {
            self.0.fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DarkInfo {
        pub treatment: String,
        /// True only when a HUMAN chose it. A re-adaptation keeps a chosen
        /// treatment and overwrites an automatic one.
        pub chosen: bool,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LogoMeta {
        /// Master's filename, e.g. `original.png`.
        pub file: Option<String>,
        pub mime: String,
        /// `manual` | `ddg` | `wikidata` | `favicon` | `none`.
        pub source: String,
        pub fetched_at: u64,
        /// Trimmed-master size and aspect, recorded at prep time (§4.5).
        pub w: Option<u32>,
        pub h: Option<u32>,
        pub aspect: Option<f64>,
        pub sizes: Vec<u32>,
        pub dark_sizes: Vec<u32>,
        pub dark: Option<DarkInfo>,
    }

    impl LogoMeta {
        /// Written with CarFM's own JSON key names. Nothing reads across the two
        /// apps' sandboxes, so this buys nothing at runtime — it costs nothing
        /// either, and it means a logo tree pulled off a phone for debugging can
        /// be read here without a translation step.
        pub fn to_json(&self) -> String {
            let mut parts: Vec<String> = Vec::new();
            if let Some(f) = &self.file {
                parts.push(format!("\"file\":{}", json::quote(f)));
            }
            parts.push(format!("\"mime\":{}", json::quote(&self.mime)));
            parts.push(format!("\"source\":{}", json::quote(&self.source)));
            parts.push(format!("\"fetchedAt\":{}", self.fetched_at));
            if let Some(w) = self.w {
                parts.push(format!("\"w\":{w}"));
            }
            if let Some(h) = self.h {
                parts.push(format!("\"h\":{h}"));
            }
            if let Some(a) = self.aspect {
                parts.push(format!("\"aspect\":{a}"));
            }
            parts.push(format!(
                "\"sizes\":[{}]",
                self.sizes.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
            ));
            parts.push(format!(
                "\"darkSizes\":[{}]",
                self.dark_sizes.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
            ));
            if let Some(d) = &self.dark {
                parts.push(format!(
                    "\"dark\":{{\"treatment\":{},\"chosen\":{}}}",
                    json::quote(&d.treatment),
                    d.chosen
                ));
            }
            format!("{{{}}}", parts.join(","))
        }

        pub fn from_json(v: &Json) -> Option<LogoMeta> {
            let arr = |k: &str| -> Vec<u32> {
                v.get(k)
                    .and_then(Json::as_arr)
                    .map(|a| a.iter().filter_map(Json::as_u32).collect())
                    .unwrap_or_default()
            };
            Some(LogoMeta {
                file: v.get("file").and_then(Json::as_str).map(str::to_string),
                mime: v.get("mime").and_then(Json::as_str).unwrap_or("image/png").to_string(),
                source: v.get("source").and_then(Json::as_str).unwrap_or("manual").to_string(),
                fetched_at: v.get("fetchedAt").and_then(Json::as_f64).unwrap_or(0.0) as u64,
                w: v.get("w").and_then(Json::as_u32),
                h: v.get("h").and_then(Json::as_u32),
                aspect: v.get("aspect").and_then(Json::as_f64),
                sizes: arr("sizes"),
                dark_sizes: arr("darkSizes"),
                dark: v.get("dark").and_then(|d| {
                    Some(DarkInfo {
                        treatment: d.get("treatment").and_then(Json::as_str)?.to_string(),
                        chosen: d.get("chosen").and_then(Json::as_bool).unwrap_or(false),
                    })
                }),
            })
        }
    }

    #[derive(Debug, PartialEq)]
    pub enum StoreError {
        /// A non-manual write was refused over a manually assigned master.
        ///
        /// CarFM has this hole open: `putOriginal` never checks the existing
        /// `meta.source`, so a forced resolve would overwrite a hand-picked logo
        /// with a DuckDuckGo guess. It is unreachable there only because
        /// AUTO_LOGO_RESOLUTION is false and `enrichNow` has no caller — porting
        /// the cascade without closing it would re-arm it.
        ManualLocked,
        Io(String),
    }

    impl std::fmt::Display for StoreError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                StoreError::ManualLocked => {
                    write!(f, "this station's logo was set by hand")
                }
                StoreError::Io(e) => write!(f, "{e}"),
            }
        }
    }

    /// The mime → extension map. The master keeps its real extension so it is
    /// still a decodable last resort if prep ever fails; a `.bin` makes every
    /// loader guess.
    pub fn ext_for(mime: &str) -> &'static str {
        match mime.to_lowercase().as_str() {
            "image/png" => "png",
            "image/jpeg" | "image/jpg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            "image/svg+xml" => "svg",
            "image/bmp" => "bmp",
            _ => "png",
        }
    }

    /// `base` → directory name: upper-cased, anything outside `[A-Z0-9_-]`
    /// replaced by `_`.
    ///
    /// Done over UTF-16 code units, which is what the JavaScript regex operates
    /// on, so an astral character becomes TWO underscores here exactly as it does
    /// there. Unreachable for FCC call signs; written to match anyway, because a
    /// directory name that disagrees is a logo that silently disappears.
    pub fn safe_base(base: &str) -> String {
        base.to_uppercase()
            .encode_utf16()
            .map(|u| match u {
                0x41..=0x5A | 0x30..=0x39 | 0x5F | 0x2D => u as u8 as char,
                _ => '_',
            })
            .collect()
    }

    #[derive(Default)]
    struct Cache {
        meta: HashMap<String, Option<LogoMeta>>,
        index_loaded: bool,
        prefs: HashMap<String, HeroFlags>,
        prefs_loaded: bool,
    }

    pub struct LogoStore {
        root: PathBuf,
        cache: Mutex<Cache>,
        clock: Box<dyn Clock>,
    }

    impl LogoStore {
        pub fn new(root: impl Into<PathBuf>) -> LogoStore {
            LogoStore::with_clock(root, Box::new(SystemClock))
        }

        pub fn with_clock(root: impl Into<PathBuf>, clock: Box<dyn Clock>) -> LogoStore {
            LogoStore { root: root.into(), cache: Mutex::new(Cache::default()), clock }
        }

        pub fn root(&self) -> &Path {
            &self.root
        }

        pub fn dir(&self, base: &str) -> PathBuf {
            self.root.join(safe_base(base))
        }

        fn file(&self, base: &str, name: &str) -> PathBuf {
            self.dir(base).join(name)
        }

        fn index_path(&self) -> PathBuf {
            self.root.join("index.json")
        }

        fn prefs_path(&self) -> PathBuf {
            self.root.join("prefs.json")
        }

        // ── meta ─────────────────────────────────────────────────────────────

        /// Load `index.json` once. Each station's `meta.json` stays authoritative —
        /// this is a pure READ cache, so a partial write can only ever cost one
        /// station a lookup, never corrupt it.
        fn load_index(&self, c: &mut Cache) {
            if c.index_loaded {
                return;
            }
            c.index_loaded = true;
            let Ok(raw) = fs::read_to_string(self.index_path()) else { return };
            let Some(Json::Obj(all)) = json::parse(&raw) else { return };
            for (k, v) in all {
                c.meta.entry(k).or_insert_with(|| LogoMeta::from_json(&v));
            }
        }

        /// CarFM coalesces this behind a 1500 ms timer because its one-time
        /// DB→filesystem migration wrote meta for every station at once. Carnyx has
        /// no migration and every write here is a single user-initiated assign, so
        /// it is written straight through and there is no timer to leak.
        fn save_index(&self, c: &Cache) {
            let mut keys: Vec<&String> = c.meta.keys().collect();
            keys.sort(); // stable file, so a diff of the tree is readable
            let body: Vec<String> = keys
                .iter()
                .filter_map(|k| {
                    c.meta[*k].as_ref().map(|m| format!("{}:{}", json::quote(k.as_str()), m.to_json()))
                })
                .collect();
            let _ = fs::create_dir_all(&self.root);
            let _ = fs::write(self.index_path(), format!("{{{}}}", body.join(",")));
        }

        pub fn meta(&self, base: &str) -> Option<LogoMeta> {
            let key = safe_base(base);
            let mut c = self.cache.lock().unwrap();
            if !c.meta.contains_key(&key) {
                self.load_index(&mut c);
            }
            if let Some(v) = c.meta.get(&key) {
                return v.clone();
            }
            let m = fs::read_to_string(self.file(base, "meta.json"))
                .ok()
                .and_then(|s| json::parse(&s))
                .and_then(|v| LogoMeta::from_json(&v));
            c.meta.insert(key, m.clone());
            m
        }

        fn write_meta(&self, base: &str, meta: LogoMeta) -> Result<(), StoreError> {
            fs::create_dir_all(self.dir(base)).map_err(|e| StoreError::Io(e.to_string()))?;
            fs::write(self.file(base, "meta.json"), meta.to_json())
                .map_err(|e| StoreError::Io(e.to_string()))?;
            let mut c = self.cache.lock().unwrap();
            self.load_index(&mut c);
            c.meta.insert(safe_base(base), Some(meta));
            self.save_index(&c);
            Ok(())
        }

        /// Patch the fields prep and dark adaptation add, leaving the master's own
        /// record alone. Never used for a new master — see `put_original`.
        fn patch_meta(
            &self,
            base: &str,
            f: impl FnOnce(&mut LogoMeta),
        ) -> Result<(), StoreError> {
            let mut m = self.meta(base).unwrap_or(LogoMeta {
                mime: "image/png".into(),
                source: "manual".into(),
                fetched_at: self.clock.now_ms(),
                ..Default::default()
            });
            f(&mut m);
            self.write_meta(base, m)
        }

        // ── master ───────────────────────────────────────────────────────────

        /// True when this station's logo was set by hand.
        ///
        /// `meta.source` is THE source of truth for stickiness. CarFM has two
        /// mechanisms — this one and a `WHERE logos.source IS NOT 'manual'` guard
        /// on a SQLite upsert — and its manual-assign path writes only the file
        /// store, so a hand-assigned station may have no database row at all.
        /// Carnyx keeps one mechanism and drops the vestigial half rather than
        /// carrying the inconsistency across.
        pub fn is_manual(&self, base: &str) -> bool {
            self.meta(base).map(|m| m.source == "manual").unwrap_or(false)
        }

        /// Store the ORIGINAL bytes. Wipes every derived file first — they belong
        /// to the previous master — and REPLACES meta wholesale.
        ///
        /// The wholesale replace is not laziness. Merging would let a new image
        /// inherit the old one's dark treatment, and a stale `chosen = true` would
        /// then force the OLD treatment onto the NEW art; it would also inherit an
        /// aspect and a ladder that no longer describe anything.
        pub fn put_original(
            &self,
            base: &str,
            bytes: &[u8],
            mime: &str,
            source: &str,
        ) -> Result<(), StoreError> {
            if source != "manual" && self.is_manual(base) {
                return Err(StoreError::ManualLocked);
            }
            self.clear_derived(base);
            self.remove_masters(base);
            fs::create_dir_all(self.dir(base)).map_err(|e| StoreError::Io(e.to_string()))?;
            let file = format!("original.{}", ext_for(mime));
            fs::write(self.file(base, &file), bytes)
                .map_err(|e| StoreError::Io(e.to_string()))?;
            self.write_meta(
                base,
                LogoMeta {
                    file: Some(file),
                    mime: mime.to_string(),
                    source: source.to_string(),
                    fetched_at: self.clock.now_ms(),
                    ..Default::default()
                },
            )
        }

        /// Record that every source was asked and none had this station's logo.
        ///
        /// Meta ONLY, never a file: a zero-byte `original.png` would make
        /// `has_original` answer yes forever. The timestamp is the whole point —
        /// it is what the 30-day TTL reads so a station nobody has art for is not
        /// re-asked on every launch.
        pub fn record_miss(&self, base: &str) -> Result<(), StoreError> {
            if self.is_manual(base) {
                return Err(StoreError::ManualLocked);
            }
            self.write_meta(
                base,
                LogoMeta {
                    file: None,
                    mime: "image/png".into(),
                    source: "none".into(),
                    fetched_at: self.clock.now_ms(),
                    ..Default::default()
                },
            )
        }

        /// Delete every master file, so a mime change cannot leave two behind and
        /// let the probe order pick the stale one.
        fn remove_masters(&self, base: &str) {
            if let Ok(rd) = fs::read_dir(self.dir(base)) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().starts_with("original.") {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
        }

        /// Path of the master, honouring the extension recorded in meta and then
        /// the historical guesses, in CarFM's order.
        pub fn master_path(&self, base: &str) -> Option<PathBuf> {
            let meta = self.meta(base);
            let mut cands: Vec<String> = Vec::new();
            if let Some(f) = meta.as_ref().and_then(|m| m.file.clone()) {
                cands.push(f);
            }
            cands.extend(
                ["original.png", "original.jpg", "original.webp", "original.bin"]
                    .iter()
                    .map(|s| s.to_string()),
            );
            cands.into_iter().map(|f| self.file(base, &f)).find(|p| p.is_file())
        }

        pub fn has_original(&self, base: &str) -> bool {
            self.master_path(base).is_some()
        }

        pub fn read_master(&self, base: &str) -> Option<Vec<u8>> {
            fs::read(self.master_path(base)?).ok()
        }

        // ── derived ──────────────────────────────────────────────────────────

        pub fn put_derived(&self, base: &str, name: &str, bytes: &[u8]) -> Result<(), StoreError> {
            fs::create_dir_all(self.dir(base)).map_err(|e| StoreError::Io(e.to_string()))?;
            fs::write(self.file(base, name), bytes).map_err(|e| StoreError::Io(e.to_string()))
        }

        /// Delete every derived file, keeping the master and its meta.
        pub fn clear_derived(&self, base: &str) {
            if let Ok(rd) = fs::read_dir(self.dir(base)) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if !n.starts_with("original.") && n != "meta.json" {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
        }

        /// Record the trimmed master's geometry and which ladder sizes exist.
        pub fn set_display_meta(
            &self,
            base: &str,
            w: u32,
            h: u32,
            sizes: Vec<u32>,
        ) -> Result<(), StoreError> {
            self.patch_meta(base, |m| {
                m.w = Some(w);
                m.h = Some(h);
                m.aspect = Some(w as f64 / h as f64);
                m.sizes = sizes;
            })
        }

        pub fn put_dark(
            &self,
            base: &str,
            treatment: Treatment,
            png: &[u8],
            chosen: bool,
        ) -> Result<(), StoreError> {
            self.put_derived(base, "dark.png", png)?;
            self.patch_meta(base, |m| {
                m.dark =
                    Some(DarkInfo { treatment: treatment.as_enum().to_string(), chosen });
                m.dark_sizes = Vec::new();
            })
        }

        pub fn set_dark_sizes(&self, base: &str, sizes: Vec<u32>) -> Result<(), StoreError> {
            self.patch_meta(base, |m| m.dark_sizes = sizes)
        }

        pub fn dark_info(&self, base: &str) -> Option<DarkInfo> {
            self.meta(base)?.dark
        }

        pub fn clear_dark(&self, base: &str) {
            if let Ok(rd) = fs::read_dir(self.dir(base)) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n == "dark.png" || n.starts_with("k-") {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
            let _ = self.patch_meta(base, |m| {
                m.dark = None;
                m.dark_sizes = Vec::new();
            });
        }

        // ── read paths (what the face renders) ───────────────────────────────

        /// The image to DISPLAY: the smallest pre-rendered size that covers
        /// `box_dp`, else the trimmed master, else the untouched original.
        ///
        /// No meta means no logo and NO I/O — this is called once per preset tile
        /// per invalidation, and a `stat` per tile is exactly the per-tile cost the
        /// index exists to remove.
        pub fn display_path(
            &self,
            base: &str,
            box_dp: Option<f32>,
            scale: f32,
        ) -> Option<PathBuf> {
            let meta = self.meta(base)?;
            if let Some(n) = prep::ladder_for(box_dp, scale, &meta.sizes) {
                return Some(self.file(base, &format!("d-{n}.png")));
            }
            // `w` is only set once display.png has been written.
            if meta.w.is_some() {
                return Some(self.file(base, "display.png"));
            }
            self.master_path(base)
        }

        /// The dark-adapted image and the treatment it was built with. `None` when
        /// this station has no cached variant yet — the caller falls back to the
        /// light master on a white plate rather than showing nothing.
        pub fn dark_path(
            &self,
            base: &str,
            box_dp: Option<f32>,
            scale: f32,
        ) -> Option<(PathBuf, Treatment)> {
            let meta = self.meta(base)?;
            let d = meta.dark.as_ref()?;
            let t = Treatment::from_enum(&d.treatment)?;
            let name = match prep::ladder_for(box_dp, scale, &meta.dark_sizes) {
                Some(n) => format!("k-{n}.png"),
                None => "dark.png".to_string(),
            };
            Some((self.file(base, &name), t))
        }

        // ── maintenance ──────────────────────────────────────────────────────

        /// Directory names under the root that are stations. The two bookkeeping
        /// files are not.
        pub fn bases(&self) -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            if let Ok(rd) = fs::read_dir(&self.root) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.starts_with('.') || n == "index.json" || n == "prefs.json" {
                        continue;
                    }
                    if e.path().is_dir() {
                        out.push(n);
                    }
                }
            }
            out.sort();
            out
        }

        pub fn remove(&self, base: &str) {
            let _ = fs::remove_dir_all(self.dir(base));
            let mut c = self.cache.lock().unwrap();
            self.load_index(&mut c);
            c.meta.remove(&safe_base(base));
            self.save_index(&c);
        }

        /// Delete every stored logo AND every hero-flag choice, and say how many
        /// stations went. The flags default off a logo's existence, so a logo wipe
        /// that left them behind would leave stations with a blank hero.
        pub fn clear_all(&self) -> usize {
            let n = self.bases().len();
            let _ = fs::remove_dir_all(&self.root);
            let _ = fs::create_dir_all(&self.root);
            let mut c = self.cache.lock().unwrap();
            c.meta.clear();
            c.index_loaded = true; // the tree is gone; there is nothing to re-read
            c.prefs.clear();
            c.prefs_loaded = true;
            n
        }

        // ── hero flags ───────────────────────────────────────────────────────

        fn load_prefs(&self, c: &mut Cache) {
            if c.prefs_loaded {
                return;
            }
            c.prefs_loaded = true;
            let Ok(raw) = fs::read_to_string(self.prefs_path()) else { return };
            let Some(Json::Obj(all)) = json::parse(&raw) else { return };
            for (k, v) in all {
                let call = v.get("showCall").and_then(Json::as_bool);
                let freq = v.get("showFreq").and_then(Json::as_bool);
                if let (Some(show_call), Some(show_freq)) = (call, freq) {
                    c.prefs.insert(k, HeroFlags { show_call, show_freq });
                }
            }
        }

        fn save_prefs(&self, c: &Cache) {
            let mut keys: Vec<&String> = c.prefs.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .iter()
                .map(|k| {
                    let f = c.prefs[*k];
                    format!(
                        "{}:{{\"showCall\":{},\"showFreq\":{}}}",
                        json::quote(k.as_str()),
                        f.show_call,
                        f.show_freq
                    )
                })
                .collect();
            let _ = fs::create_dir_all(&self.root);
            let _ = fs::write(self.prefs_path(), format!("{{{}}}", body.join(",")));
        }

        /// The stored choice, or `None` for "the user has never chosen". Both are
        /// real answers; see `prefs::Lookup` for why the third state matters.
        pub fn prefs(&self, base: &str) -> Option<HeroFlags> {
            let mut c = self.cache.lock().unwrap();
            self.load_prefs(&mut c);
            c.prefs.get(&safe_base(base)).copied()
        }

        pub fn set_prefs(&self, base: &str, flags: HeroFlags) {
            let mut c = self.cache.lock().unwrap();
            self.load_prefs(&mut c);
            c.prefs.insert(safe_base(base), flags);
            self.save_prefs(&c);
        }

        pub fn clear_all_prefs(&self) {
            let mut c = self.cache.lock().unwrap();
            c.prefs.clear();
            c.prefs_loaded = true;
            self.save_prefs(&c);
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The two framework seams
// ═════════════════════════════════════════════════════════════════════════════

/// Turning encoded bytes into pixels and back.
///
/// A SEAM, NOT AN IMPLEMENTATION. `Cargo.toml` carries no image crate today, and
/// it belongs to the build agent, so this is where `image` (decode) and `png`
/// (encode) attach. Two things the real implementation must honour and cannot be
/// tested for here:
///
///  * STRAIGHT ALPHA. Every stage assumes un-premultiplied RGBA. A decoder that
///    hands back premultiplied pixels turns every antialiased edge into a dark
///    fringe, and nothing in the pipeline would notice.
///  * sRGB. CarFM draws through an offscreen Skia surface created with
///    `colorSpace: 'srgb'` specifically so a P3-tagged upload is CONVERTED. The
///    `image` crate does not colour-manage: a P3 logo will be read as sRGB and
///    come out oversaturated. That is a known, accepted regression — record it
///    rather than pretend the guarantee survived.
pub trait ImageCodec: Send + Sync {
    /// Decode to straight-alpha sRGB RGBA8, longest edge capped at
    /// `prep::DECODE_MAX_EDGE`.
    fn decode(&self, bytes: &[u8]) -> Option<Raster>;
    /// Encode straight-alpha sRGB RGBA8 as a PNG.
    fn encode_png(&self, raster: &Raster) -> Option<Vec<u8>>;
}

/// One downloaded image.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedImage {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// Every socket the logo feature touches.
///
/// A SEAM, NOT AN IMPLEMENTATION. Six to eight HTTPS requests per search, and
/// none of them can be made from the host — everything in this file is pure
/// logic, tested against captured bodies.
///
/// THE IMPLEMENTATION IS [`crate::android::net::AndroidNet`], over
/// `HttpsURLConnection`. This doc previously recommended `ureq` + `rustls` +
/// `webpki-roots`, and that recommendation was deliberately NOT taken: the head
/// unit is 32-bit ARM, a TLS stack there is a C dependency to cross-compile and
/// verify, and bundled Mozilla roots go stale on a machine that may never be
/// updated again. The app already dexes Java and binds it over JNI twice, so one
/// more class costs almost nothing and gets the SYSTEM trust store — the one the
/// rest of the device trusts. That module's header carries the full argument.
///
/// GIVE EVERY REQUEST A TIMEOUT. CarFM sets none: its `fetch` can hang until the
/// platform gives up, and on a head unit with a dead SIM that is a spinner the
/// driver cannot cancel. 8 s connect / 15 s total is the pair `CarnyxNet` uses.
pub trait LogoNet: Send + Sync {
    /// Page → `vqd` token → `i.js` JSON → parsed rows. One method because the two
    /// round trips are one operation as far as every caller is concerned, and
    /// because the token is worthless on its own.
    ///
    /// Every failure — transport, non-2xx, bad JSON, missing token — yields an
    /// empty vector rather than an error, exactly as CarFM does. `Err` is reserved
    /// for a failure the UI should word as "the search couldn't finish".
    fn search(&self, query: &str, n: usize) -> Result<Vec<ddg::DdgImage>, String>;

    /// Download one image, size-capped, reporting WHY on failure. The reason is
    /// shown to the driver, so the wording is part of the contract — see
    /// `resolver::fetch_error` for the six strings.
    fn fetch_image(&self, url: &str) -> Result<FetchedImage, String>;

    /// The Wikidata and homepage sources. Both belong to the disabled cascade and
    /// have no caller in a shipping build; a real implementation may return
    /// `Err` unconditionally until the cascade is ever switched on.
    fn fetch_text(&self, url: &str) -> Result<String, String>;
}

// ═════════════════════════════════════════════════════════════════════════════
// Assigning a logo
// ═════════════════════════════════════════════════════════════════════════════

/// What happens between "the user tapped Confirm" and "the tile has new art".
///
/// All of it is CPU-bound pixel work — a 1024 px master through flatten,
/// mark-bounds and three ladder steps is tens of millions of operations, and the
/// dark pipeline runs a second full pass with two box blurs and two
/// connected-component labelings on top. CarFM only survives this on the JS thread
/// by yielding between ladder sizes. On the head unit's 32-bit ARM it is seconds,
/// not milliseconds, so it MUST NOT run on the Slint event loop — see
/// `service::Worker`, which is the only thing that should call in here.
pub mod assign {
    use super::dark::{pipeline, stages::Treatment, LOGO_DARK_BG};
    use super::store::{LogoStore, StoreError};
    use super::{prep, ImageCodec, Raster};

    /// Build `display.png` and its ladder from the stored master.
    ///
    /// Best-effort by design: on any failure the master itself stays the display
    /// image, which is worse-looking but never broken. Returns whether renditions
    /// were written.
    pub fn prepare_renditions(
        store: &LogoStore,
        codec: &dyn ImageCodec,
        base: &str,
    ) -> bool {
        let Some(bytes) = store.read_master(base) else { return false };
        // Decode a COPY of the master — the stored file is only ever read.
        let Some(master) = codec.decode(&bytes) else { return false };
        let display = prep::display_rendition(&master);
        let Some(png) = codec.encode_png(&display) else { return false };
        if store.put_derived(base, "display.png", &png).is_err() {
            return false;
        }
        let mut sizes: Vec<u32> = Vec::new();
        for (size, r) in prep::ladder_rasters(&display) {
            let Some(p) = codec.encode_png(&r) else { continue };
            if store.put_derived(base, &format!("d-{size}.png"), &p).is_ok() {
                sizes.push(size);
            }
        }
        sizes.sort_unstable();
        store.set_display_meta(base, display.w, display.h, sizes).is_ok()
    }

    /// Create or refresh the cached dark variant, keeping a human's override when
    /// the same treatment is still on the table.
    ///
    /// NEVER FAILS LOUDLY. CarFM's note is the rule: "never let logo adaptation
    /// break a save". A station with no dark variant renders its light logo on a
    /// white plate, which is legible; a save that failed because the adapter threw
    /// leaves the driver with no logo at all.
    ///
    /// Derives from the ORIGINAL, never from the trimmed display copy — derived
    /// renditions are never chained off each other.
    pub fn regenerate_dark(
        store: &LogoStore,
        codec: &dyn ImageCodec,
        base: &str,
        gate_bg: [f64; 3],
    ) -> Option<Treatment> {
        let Some(bytes) = store.read_master(base) else {
            store.clear_dark(base);
            return None;
        };
        // A master that will not decode leaves any PRIOR cache intact: re-adapting
        // is a nicety, and wiping a good variant over a transient failure is not.
        let master = codec.decode(&bytes)?;
        let res = pipeline::adapt_logo_for_dark(&master, gate_bg);
        let prior = store.dark_info(base);
        let (treatment, chosen) = pipeline::choose_treatment(
            prior.as_ref().map(|d| (d.treatment.as_str(), d.chosen)),
            &res.candidates,
            res.pick,
        );
        let cand = res
            .candidates
            .iter()
            .find(|c| c.treatment == treatment)
            .or_else(|| res.candidates.first())?;
        let png = codec.encode_png(&cand.raster)?;
        store.put_dark(base, cand.treatment, &png, chosen).ok()?;
        let mut sizes: Vec<u32> = Vec::new();
        for (size, r) in prep::ladder_rasters(&cand.raster) {
            let Some(p) = codec.encode_png(&r) else { continue };
            if store.put_derived(base, &format!("k-{size}.png"), &p).is_ok() {
                sizes.push(size);
            }
        }
        sizes.sort_unstable();
        let _ = store.set_dark_sizes(base, sizes);
        Some(cand.treatment)
    }

    /// Assign a logo from an ordered list of candidate URLs — the full-size image
    /// first, DDG's proxied thumbnail second.
    ///
    /// THE FALLBACK IS WHY THE FEATURE WORKS. The full-size URL frequently 403s or
    /// busts the 1 MiB cap (WERN 88.7 is the case CarFM names) while the proxied
    /// thumbnail downloads every time. The error reported is the LAST candidate's,
    /// because that is the one that ran out of options.
    pub fn assign_from_urls(
        store: &LogoStore,
        codec: &dyn ImageCodec,
        net: &dyn super::LogoNet,
        base: &str,
        urls: &[String],
    ) -> Result<(), String> {
        let cands: Vec<&String> = urls.iter().filter(|u| !u.is_empty()).collect();
        if cands.is_empty() {
            return Err("no image address to download".into());
        }
        let mut last = "the image couldn't be downloaded".to_string();
        for url in cands {
            match net.fetch_image(url) {
                Ok(img) => {
                    store
                        .put_original(base, &img.bytes, &img.mime, "manual")
                        .map_err(|e: StoreError| e.to_string())?;
                    prepare_renditions(store, codec, base);
                    // Auto-pick only: Carnyx has no counterpart to CarFM's
                    // LogoDarkPicker, so the treatment is stored with
                    // chosen = false and a later re-adapt is free to change it.
                    let _ = regenerate_dark(store, codec, base, LOGO_DARK_BG);
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Decode a stored rendition for display. Returns `None` rather than a
    /// placeholder: a surface that cannot get its art must draw the call-sign box,
    /// not an empty rectangle.
    pub fn read_rendition(
        store: &LogoStore,
        codec: &dyn ImageCodec,
        base: &str,
        box_dp: Option<f32>,
        scale: f32,
    ) -> Option<Raster> {
        let path = store.display_path(base, box_dp, scale)?;
        codec.decode(&std::fs::read(path).ok()?)
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The resolver cascade — ported, and switched OFF
// ═════════════════════════════════════════════════════════════════════════════

/// DuckDuckGo → Wikidata → station-homepage favicon, from
/// `services/logoResolver.ts`.
///
/// THIS HAS NO CALLER AND MUST NOT GET ONE. `AUTO_LOGO_RESOLUTION` is `false` and
/// every function below returns `Disabled` without touching the network unless a
/// caller passes `force`, which only a user tap ever should. The 2026-07-17 device
/// test is the reason: auto-resolved logos came back completely wrong, because
/// every text-matching source will happily return an unrelated image rather than
/// nothing. It is ported because the ORDER, the TTL and the recorded-miss
/// behaviour are worth having written down and tested, not because it should run.
pub mod resolver {
    use super::store::{LogoStore, StoreError};
    use super::{assign, ImageCodec, LogoNet};

    /// Flip to true only once background auto-resolution is proven safe, which it
    /// is not.
    pub const AUTO_LOGO_RESOLUTION: bool = false;

    /// Hits AND recorded misses are honoured for 30 days, so the app is not chatty
    /// about a station nobody has art for.
    pub const CACHE_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;

    /// Cap for a stored logo. 200 KB was too tight — a full-resolution PNG station
    /// logo routinely exceeds it, and that was the silent reason manual assigns
    /// failed with a generic error. 1 MiB is generous for a single user-initiated
    /// fetch and still guards against an absurd download.
    pub const MAX_LOGO_BYTES: usize = 1024 * 1024;

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LogoStation {
        pub base: String,
        pub callsign: Option<String>,
        pub homepage: Option<String>,
        pub freq_mhz: Option<f32>,
    }

    /// One place a logo URL might come from. A trait so the cascade's ORDER and
    /// its miss/queue outcomes can be tested against fakes — which is the only way
    /// they will ever be tested, since none of the three real sources is reachable
    /// from here.
    pub trait LogoSource {
        /// The value recorded in `meta.source` when this one wins.
        fn name(&self) -> &'static str;
        /// Whether this source can even be tried for this station.
        fn applies(&self, st: &LogoStation) -> bool;
        fn find(&self, st: &LogoStation) -> Option<String>;
    }

    #[derive(Debug, PartialEq)]
    pub enum Outcome {
        /// The gate is closed and nothing happened.
        Disabled,
        /// Already on disk, and this was not a forced resolve.
        HaveOriginal,
        /// Attempted inside the TTL window.
        WithinTtl,
        Stored {
            source: &'static str,
        },
        /// Every source was asked and none had it. Recorded so the TTL suppresses
        /// a retry — a clean "nobody has this" is an answer, not a failure.
        RecordedMiss,
        /// Something threw. Queued for a later sweep instead of being recorded as
        /// a miss, because a network failure is not evidence about the station.
        Queued,
    }

    /// The six failure strings, verbatim, because they are shown to the driver.
    ///
    /// They live here rather than in the network implementation so the wording can
    /// be pinned by a test, and so the two-candidate fallback in
    /// `assign::assign_from_urls` reports something a person can act on ("image
    /// host returned HTTP 403") instead of "unspecified error".
    pub mod fetch_error {
        use super::MAX_LOGO_BYTES;

        pub fn not_a_web_address() -> String {
            "not a web address".into()
        }
        pub fn unreachable(msg: &str) -> String {
            format!("couldn\u{2019}t reach the image ({msg})")
        }
        pub fn http_status(status: u16) -> String {
            format!("image host returned HTTP {status}")
        }
        pub fn unreadable(msg: &str) -> String {
            format!("couldn\u{2019}t read the image ({msg})")
        }
        pub fn empty() -> String {
            "image was empty".into()
        }
        pub fn too_large(len: usize) -> String {
            format!(
                "image is too large ({} KB, max {} KB)",
                (len as f64 / 1024.0).round() as u64,
                (MAX_LOGO_BYTES as f64 / 1024.0).round() as u64
            )
        }

        /// The mime a downloaded image is stored under: the `Content-Type` up to
        /// the first `;` when it is an image type, else PNG. Servers routinely
        /// send `application/octet-stream` for a perfectly good PNG.
        pub fn mime_from_content_type(ct: Option<&str>) -> String {
            match ct {
                Some(v) if v.starts_with("image/") => {
                    v.split(';').next().unwrap_or("image/png").to_string()
                }
                _ => "image/png".to_string(),
            }
        }

        /// The size and emptiness checks, so the network edge carries no policy.
        pub fn check_bytes(len: usize) -> Option<String> {
            if len == 0 {
                Some(empty())
            } else if len > MAX_LOGO_BYTES {
                Some(too_large(len))
            } else {
                None
            }
        }
    }

    /// Whether a background resolve should even start.
    ///
    /// Split out because it is the whole TTL policy in four lines and it is the
    /// part that would silently make the app chatty if it drifted. A forced (user)
    /// resolve ignores both gates — the user is asking now.
    ///
    /// `auto` is a PARAMETER rather than a direct read of the const for one
    /// reason: with the const false, everything below the first branch is
    /// unreachable and therefore untestable. The only production caller passes
    /// `AUTO_LOGO_RESOLUTION`; the tests pass `true` so the TTL is actually
    /// exercised rather than merely written down.
    pub fn should_resolve(
        auto: bool,
        force: bool,
        has_original: bool,
        fetched_at: Option<u64>,
        now_ms: u64,
    ) -> Result<(), Outcome> {
        if !auto && !force {
            return Err(Outcome::Disabled);
        }
        if !force && has_original {
            return Err(Outcome::HaveOriginal);
        }
        if !force {
            if let Some(at) = fetched_at {
                if now_ms.saturating_sub(at) < CACHE_TTL_MS {
                    return Err(Outcome::WithinTtl);
                }
            }
        }
        Ok(())
    }

    /// Walk the sources in order and store the first hit whose bytes also
    /// download.
    ///
    /// A source that returns a URL that then fails to download does NOT end the
    /// walk — the next source gets its turn. That is deliberate: a Wikidata entry
    /// pointing at a deleted Commons file should not cost the station its favicon.
    pub fn resolve_logo(
        store: &LogoStore,
        codec: &dyn ImageCodec,
        net: &dyn LogoNet,
        sources: &[&dyn LogoSource],
        st: &LogoStation,
        force: bool,
        now_ms: u64,
    ) -> Outcome {
        let base = st.base.to_uppercase();
        let fetched_at = store.meta(&base).map(|m| m.fetched_at);
        if let Err(o) = should_resolve(
            AUTO_LOGO_RESOLUTION,
            force,
            store.has_original(&base),
            fetched_at,
            now_ms,
        ) {
            return o;
        }
        for src in sources.iter().filter(|s| s.applies(st)) {
            let Some(url) = src.find(st) else { continue };
            let Ok(img) = net.fetch_image(&url) else { continue };
            match store.put_original(&base, &img.bytes, &img.mime, src.name()) {
                Ok(()) => {
                    assign::prepare_renditions(store, codec, &base);
                    let _ = assign::regenerate_dark(
                        store,
                        codec,
                        &base,
                        super::dark::LOGO_DARK_BG,
                    );
                    return Outcome::Stored { source: src.name() };
                }
                // The manual guard CarFM lacks. A hand-picked logo outranks every
                // automatic source, and this is where that is enforced rather than
                // assumed.
                Err(StoreError::ManualLocked) => return Outcome::HaveOriginal,
                Err(StoreError::Io(_)) => return Outcome::Queued,
            }
        }
        // A clean "nobody has this" is an answer worth remembering.
        let _ = store.record_miss(&base);
        Outcome::RecordedMiss
    }

    /// The DuckDuckGo source. Applies whenever there is any call sign at all —
    /// `st.callsign` or the base itself.
    pub struct DdgSource<'a>(pub &'a dyn LogoNet);
    impl LogoSource for DdgSource<'_> {
        fn name(&self) -> &'static str {
            "ddg"
        }
        fn applies(&self, st: &LogoStation) -> bool {
            !st.callsign.clone().unwrap_or_else(|| st.base.clone()).is_empty()
        }
        fn find(&self, st: &LogoStation) -> Option<String> {
            let cs = st.callsign.clone().unwrap_or_else(|| st.base.clone());
            let q = super::query::station_logo_query(st.freq_mhz, &cs);
            self.0.search(&q, 1).ok()?.first().map(|r| r.image.clone())
        }
    }

    /// Wikidata: item whose "call sign" (P2317) matches, returning its logo
    /// (P154). Keyless, no auth, and thin on US stations — which is why it sits
    /// second rather than first.
    pub mod wikidata {
        use super::super::json;
        use super::super::query::encode_component;

        pub const UA: &str = "Carnyx/0.1 (https://github.com/ninthfreak/carnyx)";

        pub fn build_sparql(callsign: &str) -> String {
            let cs = callsign.to_uppercase();
            let cs = cs.split('-').next().unwrap_or("").trim();
            let q = format!(
                "SELECT ?logo WHERE {{ ?item wdt:P2317 \"{cs}\" . ?item wdt:P154 ?logo . }} LIMIT 1"
            );
            format!("https://query.wikidata.org/sparql?format=json&query={}", encode_component(&q))
        }

        pub fn parse_sparql_logo(body: &str) -> Option<String> {
            let v = json::parse(body)?;
            let s = v
                .get("results")?
                .get("bindings")?
                .as_arr()?
                .first()?
                .get("logo")?
                .get("value")?
                .as_str()?;
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
    }

    /// The station homepage's own icon — the broad fallback for a station with a
    /// website and no Wikidata entry.
    pub mod favicon {
        /// Highest-value icon in a page's HTML, resolved absolute:
        /// `apple-touch-icon` > `og:image` > `rel=icon` > `/favicon.ico`.
        ///
        /// The order is by RESOLUTION, not by convention: an Apple touch icon is at
        /// least 120 px, an og:image is a real image, and a `favicon.ico` is
        /// usually 16 px and useless in a 128 dp box. A href that will not resolve
        /// is skipped and the search continues.
        pub fn pick_icon_url(html: &str, base_url: &str) -> Option<String> {
            let tags = link_tags(html);
            if let Some(u) = by_rel(&tags, |rel| rel.to_lowercase().contains("apple-touch-icon"))
                .and_then(|h| absolute(&h, base_url))
            {
                return Some(u);
            }
            if let Some(u) = og_image(html).and_then(|h| absolute(&h, base_url)) {
                return Some(u);
            }
            if let Some(u) = by_rel(&tags, |rel| {
                rel.split_whitespace().any(|w| w.eq_ignore_ascii_case("icon"))
            })
            .and_then(|h| absolute(&h, base_url))
            {
                return Some(u);
            }
            absolute("/favicon.ico", base_url)
        }

        // `to_ascii_lowercase` throughout, never `to_lowercase`: the offsets found
        // in the lowered copy index back into the ORIGINAL, and Unicode case
        // folding can change a string's byte length ('İ' lowers to two chars).
        // That mismatch is a panic on a byte index that is not a char boundary,
        // triggered by a page nobody controls.
        fn link_tags(html: &str) -> Vec<String> {
            let lower = html.to_ascii_lowercase();
            let mut out = Vec::new();
            let mut from = 0usize;
            while let Some(rel) = lower[from..].find("<link") {
                let start = from + rel;
                let end = match html[start..].find('>') {
                    Some(e) => start + e + 1,
                    None => break,
                };
                out.push(html[start..end].to_string());
                from = end;
            }
            out
        }

        fn attr(tag: &str, name: &str) -> Option<String> {
            let lower = tag.to_ascii_lowercase();
            let mut from = 0usize;
            while let Some(rel) = lower[from..].find(name) {
                let mut i = from + rel + name.len();
                let b = tag.as_bytes();
                while i < b.len() && (b[i] as char).is_whitespace() {
                    i += 1;
                }
                if i >= b.len() || b[i] != b'=' {
                    from = from + rel + name.len();
                    continue;
                }
                i += 1;
                while i < b.len() && (b[i] as char).is_whitespace() {
                    i += 1;
                }
                if i >= b.len() || (b[i] != b'"' && b[i] != b'\'') {
                    from = i;
                    continue;
                }
                let quote = b[i];
                i += 1;
                let start = i;
                while i < b.len() && b[i] != quote {
                    i += 1;
                }
                return Some(tag[start..i].to_string());
            }
            None
        }

        fn by_rel(tags: &[String], want: impl Fn(&str) -> bool) -> Option<String> {
            for t in tags {
                if let Some(rel) = attr(t, "rel") {
                    if want(&rel) {
                        if let Some(h) = attr(t, "href") {
                            return Some(h);
                        }
                    }
                }
            }
            None
        }

        fn og_image(html: &str) -> Option<String> {
            let lower = html.to_ascii_lowercase();
            let mut from = 0usize;
            while let Some(rel) = lower[from..].find("<meta") {
                let start = from + rel;
                let end = match html[start..].find('>') {
                    Some(e) => start + e + 1,
                    None => break,
                };
                let tag = &html[start..end];
                if attr(tag, "property").map(|p| p.eq_ignore_ascii_case("og:image"))
                    == Some(true)
                {
                    if let Some(c) = attr(tag, "content") {
                        return Some(c);
                    }
                }
                from = end;
            }
            None
        }

        /// Enough URL resolution for the four shapes a page actually uses:
        /// absolute, protocol-relative, root-relative and plain relative. Written
        /// out because Carnyx has no `url` crate, and noted as such: it is not a
        /// general RFC 3986 resolver and should be replaced by one if this cascade
        /// is ever switched on.
        pub fn absolute(href: &str, base_url: &str) -> Option<String> {
            let href = href.trim();
            if href.is_empty() {
                return None;
            }
            if href.starts_with("http://") || href.starts_with("https://") {
                return Some(href.to_string());
            }
            let scheme_end = base_url.find("://")?;
            let scheme = &base_url[..scheme_end];
            if let Some(rest) = href.strip_prefix("//") {
                return Some(format!("{scheme}://{rest}"));
            }
            let after = &base_url[scheme_end + 3..];
            let host_len = after.find(['/', '?', '#']).unwrap_or(after.len());
            let host = &after[..host_len];
            if host.is_empty() {
                return None;
            }
            if let Some(rest) = href.strip_prefix('/') {
                return Some(format!("{scheme}://{host}/{rest}"));
            }
            let path = &after[host_len..];
            let dir = match path.rfind('/') {
                Some(i) => &path[..i + 1],
                None => "/",
            };
            Some(format!("{scheme}://{host}{dir}{href}"))
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The logo-search window
// ═════════════════════════════════════════════════════════════════════════════

/// The state machine behind `ui/logo-search.slint`, from
/// `components/carfm/LogoSearchOverlay.tsx`.
///
/// Every one of the overlay's 21 in-properties comes out of `Model::view()` as a
/// finished value. Slint holds no state of its own here — not the selection, not
/// the toggles, not which of the five bodies is showing — because it cannot see
/// the search, the store or the network, and a second copy of the truth in the
/// `.slint` file would be a second copy to keep in step.
///
/// THE WINDOW OPENS ON A LANDING VIEW, never straight into a search. §6.4 is
/// explicit and the shipping TSX agrees: the current logo plus the two hero
/// toggles, and the search runs only when the Search button is pressed. (The
/// TSX's own header comment says the opposite and is stale; the code below it is
/// the truth.)
pub mod search {
    use super::candidate::{alt_label, candidate_background, dims_label, domain_caption, monogram};
    use super::ddg::DdgImage;
    use super::prefs::{self, HeroFlags};
    use super::query::station_logo_query;
    use super::Raster;

    /// Which of the five mutually exclusive bodies is showing.
    ///
    /// `saving` is deliberately NOT a member: the results grid stays on screen
    /// underneath the spinner, so it is a separate flag rather than a sixth state.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum State {
        Landing,
        Loading,
        Results,
        NoResults,
        Error,
    }

    /// The station the window was opened for.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Target {
        /// Call-sign base — the store key the logo is saved against.
        pub base: String,
        /// What the query is built from. EMPTY is a legitimate value: when neither
        /// the preset name nor the FCC map yields a call sign, the query degrades
        /// to `radio 98.1 logo` rather than searching for junk.
        pub callsign: String,
        pub freq_mhz: f32,
        /// Preset display name — the header title.
        pub name: String,
    }

    /// One result cell, art included.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Cell {
        /// Full-size URL, tried FIRST on Confirm.
        pub image: String,
        /// Proxied thumbnail — what the grid draws, and the Confirm fallback.
        pub thumbnail: String,
        pub domain: String,
        pub dims: String,
        pub alt: String,
        pub background: [u8; 3],
        /// `None` until the thumbnail lands. The cell draws its well and its
        /// caption immediately either way, so the grid never reflows when art
        /// arrives late.
        pub thumb: Option<Raster>,
    }

    /// What Confirm asks the worker to do.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Confirm {
        /// The guard rejected it: no target, already saving, or a non-landing
        /// state with nothing picked.
        Ignore,
        /// Landing view — persist the toggles exactly as the user left them.
        SavePrefs { base: String, flags: HeroFlags },
        /// A cell was picked. Download in URL order, then force the logo-only
        /// hero: the toggles were seeded from the PRE-EXISTING logo, so a station
        /// that had none would otherwise save (true, true) over the design
        /// default the moment it got its first logo.
        AssignLogo { base: String, urls: Vec<String>, flags: HeroFlags },
    }

    /// A search the worker should run. Carries the generation so a late answer to
    /// a superseded question can be dropped rather than painted.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Job {
        pub generation: u64,
        pub query: String,
    }

    #[derive(Debug, Default)]
    pub struct Model {
        target: Option<Target>,
        state: Option<State>,
        cells: Vec<Cell>,
        selected: i32,
        saving: bool,
        /// `Some` = a SAVE failed and this is why; `None` with `state == Error` =
        /// the search itself failed. Two different errors, two different wordings.
        save_error: Option<String>,
        has_logo: bool,
        flags: HeroFlags,
        generation: u64,
    }

    impl Model {
        pub fn new() -> Model {
            Model { state: Some(State::Landing), selected: -1, ..Default::default() }
        }

        pub fn state(&self) -> State {
            self.state.unwrap_or(State::Landing)
        }
        pub fn cells(&self) -> &[Cell] {
            &self.cells
        }
        pub fn generation(&self) -> u64 {
            self.generation
        }
        pub fn target(&self) -> Option<&Target> {
            self.target.as_ref()
        }
        pub fn saving(&self) -> bool {
            self.saving
        }

        /// Open the window. `stored` is the saved hero choice if there is one;
        /// both it and `has_logo` must be resolved BEFORE this is called, because
        /// the toggles' default depends on the logo and a flag that corrects
        /// itself a frame later is visible on screen.
        pub fn open(&mut self, target: Target, has_logo: bool, stored: Option<HeroFlags>) {
            self.target = Some(target);
            self.state = Some(State::Landing);
            self.cells.clear();
            self.selected = -1;
            self.saving = false;
            self.save_error = None;
            self.has_logo = has_logo;
            self.flags = prefs::effective(stored, has_logo);
            // A new window is a new question: anything still in flight for the
            // previous station is now stale.
            self.generation += 1;
        }

        /// The ✕, Cancel and the scrim. All three close and CHANGE NOTHING —
        /// §6.4 is explicit, so this does not persist the toggles the user was
        /// playing with.
        pub fn close(&mut self) {
            self.target = None;
            self.state = Some(State::Landing);
            self.cells.clear();
            self.selected = -1;
            self.saving = false;
            self.save_error = None;
            self.generation += 1;
        }

        /// Start a search. `None` when there is no target to search for.
        pub fn search(&mut self) -> Option<Job> {
            let t = self.target.clone()?;
            self.state = Some(State::Loading);
            self.cells.clear();
            self.selected = -1;
            self.save_error = None;
            self.generation += 1;
            Some(Job {
                generation: self.generation,
                query: station_logo_query(Some(t.freq_mhz), &t.callsign),
            })
        }

        /// "Search again", from no-results or error. Identical work to `search`,
        /// kept separate because the two entry points are different taps and Rust
        /// is the only side that can tell them apart.
        pub fn retry(&mut self) -> Option<Job> {
            self.search()
        }

        /// A search came back. Dropped when it answers a superseded question.
        pub fn results_arrived(&mut self, generation: u64, rows: Vec<DdgImage>) -> bool {
            if generation != self.generation {
                return false;
            }
            self.cells = rows
                .iter()
                .enumerate()
                .map(|(i, r)| Cell {
                    image: r.image.clone(),
                    thumbnail: r.thumbnail.clone(),
                    domain: domain_caption(&r.source),
                    dims: dims_label(r.width, r.height),
                    // The accessibility label uses the RAW origin, so an unknown
                    // one drops the clause instead of reading "from image".
                    alt: alt_label(i, &r.source),
                    background: [255, 255, 255],
                    thumb: None,
                })
                .collect();
            self.selected = -1;
            self.state = Some(if self.cells.is_empty() { State::NoResults } else { State::Results });
            true
        }

        /// One thumbnail decoded. Rows are filled individually as they land so the
        /// grid appears at once rather than after the slowest of four.
        pub fn thumb_arrived(&mut self, generation: u64, index: usize, art: Raster) -> bool {
            if generation != self.generation {
                return false;
            }
            let Some(cell) = self.cells.get_mut(index) else { return false };
            cell.background = candidate_background(&art);
            cell.thumb = Some(art);
            true
        }

        pub fn search_failed(&mut self, generation: u64) -> bool {
            if generation != self.generation {
                return false;
            }
            self.save_error = None;
            self.state = Some(State::Error);
            true
        }

        /// A cell was tapped. Single-select, and Rust moves the selection — the
        /// Slint side toggles nothing.
        pub fn pick(&mut self, index: i32) {
            if self.state() == State::Results
                && index >= 0
                && (index as usize) < self.cells.len()
            {
                self.selected = index;
            }
        }

        pub fn toggle_call(&mut self) {
            self.flags.show_call = !self.flags.show_call;
        }
        pub fn toggle_freq(&mut self) {
            self.flags.show_freq = !self.flags.show_freq;
        }

        /// Whether a cell is picked AND still exists — the `picking` branch that
        /// decides everything Confirm does.
        fn picking(&self) -> Option<&Cell> {
            if self.state() != State::Results || self.selected < 0 {
                return None;
            }
            self.cells.get(self.selected as usize)
        }

        /// Press Confirm. Sets `saving` when it returns work; the caller hands the
        /// result to `saved` or `save_failed`.
        pub fn begin_confirm(&mut self) -> Confirm {
            let Some(t) = self.target.clone() else { return Confirm::Ignore };
            if self.saving {
                return Confirm::Ignore;
            }
            let picked = self.picking().cloned();
            if self.state() != State::Landing && picked.is_none() {
                return Confirm::Ignore;
            }
            self.saving = true;
            match picked {
                Some(c) => Confirm::AssignLogo {
                    base: t.base,
                    // Full size first, proxied thumbnail second.
                    urls: vec![c.image, c.thumbnail],
                    flags: prefs::ON_NEW_LOGO,
                },
                None => Confirm::SavePrefs { base: t.base, flags: self.flags },
            }
        }

        pub fn saved(&mut self) {
            self.saving = false;
        }

        /// The save failed. The reason is the LAST candidate's — the one that ran
        /// out of options — and it is shown to the driver verbatim.
        pub fn save_failed(&mut self, reason: String) {
            self.saving = false;
            self.save_error = Some(reason);
            self.state = Some(State::Error);
        }

        pub fn can_confirm(&self) -> bool {
            !self.saving
                && (self.state() == State::Landing
                    || (self.state() == State::Results && self.selected >= 0))
        }

        /// Everything the overlay draws, finished.
        pub fn view(&self) -> View {
            let t = self.target.clone().unwrap_or_default();
            let state = self.state();
            let can_confirm = self.can_confirm();
            View {
                name: t.name.clone(),
                subtitle: subtitle(&t.name, &t.callsign, t.freq_mhz),
                has_logo: self.has_logo,
                mono: monogram(if t.callsign.is_empty() { &t.name } else { &t.callsign }),
                state,
                query: station_logo_query(Some(t.freq_mhz), &t.callsign),
                selected_index: self.selected,
                saving: self.saving,
                error_title: error_title(state, self.save_error.as_deref()),
                error_body: error_body(state, self.save_error.as_deref()),
                show_call: self.flags.show_call,
                show_freq: self.flags.show_freq,
                search_label: search_label(self.has_logo),
                can_confirm,
                confirm_label: confirm_label(state, self.saving),
                hint: hint(state, self.has_logo, can_confirm),
            }
        }
    }

    /// The finished property set. One struct rather than 17 getters so a caller
    /// cannot fill half the window and leave the other half showing the previous
    /// station's strings.
    #[derive(Clone, Debug, PartialEq)]
    pub struct View {
        pub name: String,
        pub subtitle: String,
        pub has_logo: bool,
        pub mono: String,
        pub state: State,
        pub query: String,
        pub selected_index: i32,
        pub saving: bool,
        pub error_title: String,
        pub error_body: String,
        pub show_call: bool,
        pub show_freq: bool,
        pub search_label: String,
        pub can_confirm: bool,
        pub confirm_label: String,
        pub hint: String,
    }

    /// `WMGN  ·  98.1 MHz`.
    ///
    /// TWO spaces either side of U+00B7 — not the single-spaced interpunct the
    /// Nearby rows use — and the call-sign half is included only when there is one
    /// AND it differs from the name, because `WMGN  ·  WMGN` reads as a bug.
    pub fn subtitle(name: &str, callsign: &str, freq_mhz: f32) -> String {
        let head = if !callsign.is_empty() && callsign != name {
            format!("{callsign}  \u{00B7}  ")
        } else {
            String::new()
        };
        format!("{head}{:.1} MHz", freq_mhz)
    }

    pub fn search_label(has_logo: bool) -> String {
        if has_logo {
            "Search for a different logo".into()
        } else {
            "Search for a logo".into()
        }
    }

    /// "Save" on the landing view, "Confirm" in results, "Saving…" while saving.
    ///
    /// DISAGREEMENT, resolved for the built reference: that three-way is the
    /// dc.html's, while the shipping TSX hard-codes "Confirm" and drops the label
    /// entirely for a bare spinner. §6.4 calls the dc.html "the exact reference"
    /// on appearance, and `ui/logo-search.slint` already elected it, so the label
    /// is always drawn and Rust supplies which of the three it is.
    pub fn confirm_label(state: State, saving: bool) -> String {
        if saving {
            "Saving\u{2026}".into()
        } else if state == State::Landing {
            "Save".into()
        } else {
            "Confirm".into()
        }
    }

    /// The footer hint — five cases over (state, has_logo, can_confirm).
    ///
    /// The first case is the one place the two references disagree on wording:
    /// the dc.html says "Display options apply to the main card", the TSX says
    /// "Choose what shows on the hero". Neither is wrong; the TSX's matches §6.4's
    /// own name for the surface, so it wins here.
    pub fn hint(state: State, has_logo: bool, can_confirm: bool) -> String {
        match state {
            State::Landing if has_logo => "Choose what shows on the hero".into(),
            State::Landing => String::new(),
            _ if can_confirm => "Saved as this station\u{2019}s logo".into(),
            State::Results => "Pick the correct logo".into(),
            _ => String::new(),
        }
    }

    /// Two errors, and which one is showing is not guessable from the state alone —
    /// a failed search and a failed save both land on the error body.
    pub fn error_title(state: State, save_error: Option<&str>) -> String {
        if state != State::Error {
            return String::new();
        }
        match save_error {
            Some(_) => "Couldn\u{2019}t save that logo".into(),
            None => "Search couldn\u{2019}t finish".into(),
        }
    }

    pub fn error_body(state: State, save_error: Option<&str>) -> String {
        if state != State::Error {
            return String::new();
        }
        match save_error {
            // Em dash, and the reason interpolated verbatim: "image host returned
            // HTTP 403" tells the driver to pick a different result, which is
            // exactly what the sentence then advises.
            Some(r) => format!("Couldn\u{2019}t save this logo \u{2014} {r}. Try a different result."),
            None => "Something went wrong reaching the logo search. Check your connection and try again.".into(),
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The worker
// ═════════════════════════════════════════════════════════════════════════════

/// One thread that owns every socket and every pixel pass, so the Slint event
/// loop owns nothing but property writes.
///
/// WHY THIS EXISTS AT ALL. A search is two HTTP round trips plus four thumbnail
/// downloads; a Confirm is a download, a decode, a trim, three ladder resamples
/// and a full dark-adaptation pass. On the head unit's 32-bit ARM that is seconds.
/// Run on the event loop it is a frozen face — and the face is the thing the
/// driver is looking at.
///
/// THE GENERATION COUNTER IS THE OTHER HALF. Every job carries the generation it
/// was issued under, and a result whose generation is stale is dropped by
/// `search::Model`. The worker checks it too, so four thumbnail downloads for a
/// window the user has already closed are abandoned rather than completed. This is
/// the same discipline CarFM's `useStationLogo` uses when it stores every async
/// value WITH the input it answers — without it, the vendor tuner transiting
/// frequencies made a THIRD station's logo appear.
pub mod service {
    use super::ddg::DdgImage;
    use super::prefs::HeroFlags;
    use super::search;
    use super::store::LogoStore;
    use super::{assign, ImageCodec, LogoNet, Raster};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{channel, Sender};
    use std::sync::Arc;
    use std::thread::JoinHandle;

    /// What the worker sends back. Every variant is inert data: the UI thread
    /// hands it straight to `search::Model`, which decides whether it is still
    /// wanted.
    #[derive(Clone, Debug, PartialEq)]
    pub enum Event {
        Results { generation: u64, rows: Vec<DdgImage> },
        Thumb { generation: u64, index: usize, art: Raster },
        SearchFailed { generation: u64 },
        /// A logo landed. The caller invalidates that station's tiles and the
        /// hero's display flags.
        Saved { base: String },
        SaveFailed { reason: String },
    }

    enum Job {
        Search { generation: u64, query: String },
        Assign { base: String, urls: Vec<String>, flags: HeroFlags },
        SavePrefs { base: String, flags: HeroFlags },
        Stop,
    }

    /// How a result gets back onto the UI thread.
    ///
    /// A plain callback rather than anything Slint-shaped on purpose: the caller
    /// wraps `slint::invoke_from_event_loop` around it, and this module stays
    /// testable with a channel on the other end.
    pub type Sink = Box<dyn Fn(Event) + Send + 'static>;

    pub struct Worker {
        tx: Sender<Job>,
        /// The generation the worker should still care about. Shared rather than
        /// passed so a job already in flight can notice it has been superseded.
        current: Arc<AtomicU64>,
        handle: Option<JoinHandle<()>>,
    }

    impl Worker {
        pub fn spawn(
            store: Arc<LogoStore>,
            net: Arc<dyn LogoNet>,
            codec: Arc<dyn ImageCodec>,
            sink: Sink,
        ) -> Worker {
            let (tx, rx) = channel::<Job>();
            let current = Arc::new(AtomicU64::new(0));
            let gen_for_thread = current.clone();
            let handle = std::thread::Builder::new()
                .name("carnyx-logos".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        match job {
                            Job::Stop => break,
                            Job::Search { generation, query } => {
                                run_search(
                                    &*net,
                                    &*codec,
                                    &sink,
                                    &gen_for_thread,
                                    generation,
                                    &query,
                                );
                            }
                            Job::Assign { base, urls, flags } => {
                                match assign::assign_from_urls(
                                    &store, &*codec, &*net, &base, &urls,
                                ) {
                                    Ok(()) => {
                                        store.set_prefs(&base, flags);
                                        sink(Event::Saved { base });
                                    }
                                    Err(reason) => sink(Event::SaveFailed { reason }),
                                }
                            }
                            Job::SavePrefs { base, flags } => {
                                store.set_prefs(&base, flags);
                                sink(Event::Saved { base });
                            }
                        }
                    }
                })
                .ok();
            Worker { tx, current, handle }
        }

        /// Queue a search. Returns immediately — that is the entire point.
        pub fn search(&self, job: &search::Job) {
            self.current.store(job.generation, Ordering::SeqCst);
            let _ = self.tx.send(Job::Search {
                generation: job.generation,
                query: job.query.clone(),
            });
        }

        /// Queue whatever Confirm decided. `Ignore` sends nothing, so the caller
        /// can hand this the model's answer without branching.
        pub fn submit(&self, confirm: search::Confirm) {
            let job = match confirm {
                search::Confirm::Ignore => return,
                search::Confirm::SavePrefs { base, flags } => Job::SavePrefs { base, flags },
                search::Confirm::AssignLogo { base, urls, flags } => {
                    Job::Assign { base, urls, flags }
                }
            };
            let _ = self.tx.send(job);
        }

        /// Abandon anything in flight — the window closed.
        pub fn cancel(&self, generation: u64) {
            self.current.store(generation, Ordering::SeqCst);
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            let _ = self.tx.send(Job::Stop);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    fn run_search(
        net: &dyn LogoNet,
        codec: &dyn ImageCodec,
        sink: &Sink,
        current: &AtomicU64,
        generation: u64,
        query: &str,
    ) {
        let rows = match net.search(query, 4) {
            Ok(r) => r,
            Err(_) => {
                sink(Event::SearchFailed { generation });
                return;
            }
        };
        // The grid is posted BEFORE any thumbnail is fetched, so the cells, their
        // captions and their dimensions appear at once. `ResultCell` states its
        // image geometry explicitly precisely so art arriving later cannot reflow
        // the grid it lands in.
        sink(Event::Results { generation, rows: rows.clone() });
        for (i, r) in rows.into_iter().enumerate() {
            if current.load(Ordering::SeqCst) != generation {
                return; // the question changed; stop paying for the old answer
            }
            let Ok(img) = net.fetch_image(&r.thumbnail) else { continue };
            let Some(art) = codec.decode(&img.bytes) else { continue };
            sink(Event::Thumb { generation, index: i, art });
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// The Slint bridge
// ═════════════════════════════════════════════════════════════════════════════

/// The only place in this file that knows Slint exists.
///
/// Everything above deals in `Raster`, `[u8; 3]` and `String`; these five
/// functions turn that into `Image`, `Color` and `SharedString` and write it to
/// the window. Keeping the conversion in one place is what lets the whole pipeline
/// be unit-tested without a renderer.
pub mod ui {
    use super::search::{Cell, State, View};
    use super::Raster;
    use slint::{Color, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
    use std::rc::Rc;

    /// `Raster` → `slint::Image`.
    ///
    /// `from_rgba8`, NOT `from_rgba8_premultiplied`: every stage above produces
    /// straight alpha, and handing premultiplied pixels to the straight-alpha
    /// constructor darkens every antialiased edge.
    ///
    /// Slint 1.17 has no load-from-encoded-memory API — only `load_from_path`
    /// behind a feature and `load_from_svg_data` — so building the buffer
    /// ourselves is not a preference, it is the only route from decoded bytes to
    /// an `Image`.
    pub fn to_image(r: &Raster) -> Image {
        if !r.is_valid() {
            return Image::default();
        }
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(r.w, r.h);
        buf.make_mut_bytes().copy_from_slice(&r.rgba);
        Image::from_rgba8(buf)
    }

    pub fn to_color(rgb: [u8; 3]) -> Color {
        Color::from_rgb_u8(rgb[0], rgb[1], rgb[2])
    }

    /// The five bodies. Mirrors `LogoSearchState` in `ui/logo-search.slint`; if
    /// that enum ever gains a member this is the one place that stops compiling,
    /// which is the point of writing it out rather than casting.
    pub fn to_ui_state(s: State) -> crate::LogoSearchState {
        match s {
            State::Landing => crate::LogoSearchState::Landing,
            State::Loading => crate::LogoSearchState::Loading,
            State::Results => crate::LogoSearchState::Results,
            State::NoResults => crate::LogoSearchState::NoResults,
            State::Error => crate::LogoSearchState::Error,
        }
    }

    /// One result cell. A cell whose thumbnail has not landed yet gets an empty
    /// image and keeps its caption, so the grid is complete from the first frame.
    pub fn to_candidate(c: &Cell) -> crate::LogoCandidate {
        crate::LogoCandidate {
            thumb: c.thumb.as_ref().map(to_image).unwrap_or_default(),
            domain: SharedString::from(c.domain.as_str()),
            dims: SharedString::from(c.dims.as_str()),
            background: to_color(c.background),
            alt: SharedString::from(c.alt.as_str()),
        }
    }

    /// Write the whole window in one pass.
    ///
    /// MUST RUN ON THE UI THREAD. `slint::Weak::upgrade_in_event_loop` is the only
    /// legal way back from the worker; this function assumes it is already there.
    ///
    /// `logo` is the station's CURRENT art, already at the trimmed rendition —
    /// `hero.slint:138` and `logo-search.slint:866` both derive their plate's
    /// aspect from `logo.width` / `logo.height`, so handing over an untrimmed
    /// master gives both the wrong shape.
    pub fn apply(
        window: &crate::AppWindow,
        view: &View,
        cells: &[Cell],
        logo: Option<&Raster>,
        brand: Color,
    ) {
        window.set_logo_search_name(SharedString::from(view.name.as_str()));
        window.set_logo_search_subtitle(SharedString::from(view.subtitle.as_str()));
        window.set_logo_search_logo(logo.map(to_image).unwrap_or_default());
        window.set_logo_search_has_logo(view.has_logo);
        window.set_logo_search_mono(SharedString::from(view.mono.as_str()));
        window.set_logo_search_brand(brand);
        window.set_logo_search_state(to_ui_state(view.state));
        window.set_logo_search_query(SharedString::from(view.query.as_str()));
        window.set_logo_search_selected_index(view.selected_index);
        window.set_logo_search_saving(view.saving);
        window.set_logo_search_error_title(SharedString::from(view.error_title.as_str()));
        window.set_logo_search_error_body(SharedString::from(view.error_body.as_str()));
        window.set_logo_search_show_call(view.show_call);
        window.set_logo_search_show_freq(view.show_freq);
        window.set_logo_search_search_label(SharedString::from(view.search_label.as_str()));
        window.set_logo_search_can_confirm(view.can_confirm);
        window.set_logo_search_confirm_label(SharedString::from(view.confirm_label.as_str()));
        window.set_logo_search_hint(SharedString::from(view.hint.as_str()));
        set_candidates(window, cells);
    }

    /// The grid alone. Split out because a thumbnail landing changes ONE row, and
    /// rebuilding all 17 strings for it would be waste on the frame that can least
    /// afford it.
    pub fn set_candidates(window: &crate::AppWindow, cells: &[Cell]) {
        let rows: Vec<crate::LogoCandidate> = cells.iter().map(to_candidate).collect();
        window.set_logo_search_candidates(ModelRc::from(Rc::new(VecModel::from(rows))));
    }

    /// Replace one row in place, when the model already has the right length.
    /// Returns false when it does not, so the caller can fall back to
    /// `set_candidates`.
    pub fn update_candidate(window: &crate::AppWindow, index: usize, cell: &Cell) -> bool {
        let model = window.get_logo_search_candidates();
        if index >= model.row_count() {
            return false;
        }
        model.set_row_data(index, to_candidate(cell));
        true
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════════
//
// The pure half is the whole testable surface — there is no network here, no
// image decoder and no head unit — which is exactly why the seams are where they
// are. Every value pinned below is CarFM's, taken from the TypeScript rather than
// from what this port happens to produce.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    // ── helpers ─────────────────────────────────────────────────────────────

    /// A scratch directory, because Carnyx has no `tempfile` dependency and the
    /// store is worth testing against a real filesystem rather than a mock of one.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::SeqCst);
            let p = std::env::temp_dir().join(format!(
                "carnyx-logos-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A codec with a trivial container format, so every stage above it can be
    /// driven end to end without an image crate. The real one decodes PNG/JPEG/
    /// WebP and is the one thing here that has never run.
    struct RawCodec;

    impl ImageCodec for RawCodec {
        fn decode(&self, bytes: &[u8]) -> Option<Raster> {
            if bytes.len() < 12 || &bytes[..4] != b"CRW1" {
                return None;
            }
            let w = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
            let h = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
            let need = (w as usize) * (h as usize) * 4;
            if bytes.len() < 12 + need || w == 0 || h == 0 {
                return None;
            }
            let r = Raster { w, h, rgba: bytes[12..12 + need].to_vec() };
            Some(prep::resample_raster(&r, prep::DECODE_MAX_EDGE))
        }
        fn encode_png(&self, raster: &Raster) -> Option<Vec<u8>> {
            let mut out = b"CRW1".to_vec();
            out.extend_from_slice(&raster.w.to_le_bytes());
            out.extend_from_slice(&raster.h.to_le_bytes());
            out.extend_from_slice(&raster.rgba);
            Some(out)
        }
    }

    fn encoded(r: &Raster) -> Vec<u8> {
        RawCodec.encode_png(r).unwrap()
    }

    #[derive(Default)]
    struct FakeNet {
        results: Vec<ddg::DdgImage>,
        search_fails: bool,
        images: HashMap<String, Result<FetchedImage, String>>,
    }

    impl LogoNet for FakeNet {
        fn search(&self, _q: &str, n: usize) -> Result<Vec<ddg::DdgImage>, String> {
            if self.search_fails {
                Err("no route to host".into())
            } else {
                Ok(self.results.iter().take(n).cloned().collect())
            }
        }
        fn fetch_image(&self, url: &str) -> Result<FetchedImage, String> {
            self.images.get(url).cloned().unwrap_or(Err(resolver::fetch_error::http_status(404)))
        }
        fn fetch_text(&self, _url: &str) -> Result<String, String> {
            Err("not wired".into())
        }
    }

    /// A flat rectangle, and a block painted into one.
    fn field(w: u32, h: u32, c: [u8; 4]) -> Raster {
        let mut r = Raster::empty(w, h);
        for y in 0..h {
            for x in 0..w {
                r.set(x, y, c);
            }
        }
        r
    }

    fn block(r: &mut Raster, x0: u32, y0: u32, w: u32, h: u32, c: [u8; 4]) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                r.set(x, y, c);
            }
        }
    }

    const RED: [u8; 4] = [220, 40, 40, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];

    // ── query ───────────────────────────────────────────────────────────────

    /// The six vectors CarFM's own header records. The lower-casing is the part
    /// that was measured: the proven query is `radio <freq> <lowercase> logo`.
    #[test]
    fn station_logo_query_matches_carfm() {
        use query::station_logo_query as q;
        assert_eq!(q(Some(88.7), "WERN"), "radio 88.7 wern logo");
        assert_eq!(q(Some(98.1), "WMGN"), "radio 98.1 wmgn logo");
        assert_eq!(q(None, "WMGN"), "radio wmgn logo");
        assert_eq!(q(Some(98.1), ""), "radio 98.1 logo");
        assert_eq!(q(None, ""), "radio logo");
        assert_eq!(q(Some(88.7), "  WERN  "), "radio 88.7 wern logo");
        // A non-finite dial is an absent dial, not "radio NaN wern logo".
        assert_eq!(q(Some(f32::NAN), "WERN"), "radio wern logo");
    }

    #[test]
    fn encode_component_is_javascripts() {
        use query::encode_component as e;
        assert_eq!(e("radio 98.1 wmgn logo"), "radio%2098.1%20wmgn%20logo");
        // The nine characters encodeURIComponent leaves alone and RFC 3986 does not.
        assert_eq!(e("-_.!~*'()"), "-_.!~*'()");
        assert_eq!(e("a+b&c=d"), "a%2Bb%26c%3Dd");
        assert_eq!(e("\u{00E9}"), "%C3%A9");
    }

    #[test]
    fn host_of_keeps_the_last_two_labels() {
        use query::host_of as h;
        assert_eq!(h("https://en.wikipedia.org/wiki/X"), "wikipedia.org");
        assert_eq!(h("https://www.wmgn.com/logo.png"), "wmgn.com");
        assert_eq!(h("https://wmgn.com"), "wmgn.com");
        // The wart, ported deliberately: this is not the registrable domain, and
        // it is what the reference screenshots show.
        assert_eq!(h("https://a.b.co.uk/x"), "co.uk");
        assert_eq!(h(""), "");
        assert_eq!(h("ftp://example.com/x"), "");
        assert_eq!(h("HTTPS://WWW.Example.COM/x"), "Example.COM");
    }

    // ── the DDG protocol ────────────────────────────────────────────────────

    #[test]
    fn parse_vqd_tries_three_shapes_in_order() {
        assert_eq!(ddg::parse_vqd(r#"...vqd="4-123456789"..."#).as_deref(), Some("4-123456789"));
        assert_eq!(ddg::parse_vqd("x&vqd=4-987654321&y").as_deref(), Some("4-987654321"));
        assert_eq!(ddg::parse_vqd("&vqd=abc123zzz").as_deref(), Some("abc123zzz"));
        assert_eq!(ddg::parse_vqd("no token here"), None);
        // A `vqd=` that does not match the first shape must not stop the scan —
        // this is the case a naive "find the first vqd=" implementation fails.
        assert_eq!(
            ddg::parse_vqd("vqd=&more &vqd=4-77").as_deref(),
            Some("4-77")
        );
    }

    /// These two used to slice `&str` at a fixed byte offset behind a LENGTH
    /// guard, which is not a char-boundary guard. Every string below puts a
    /// multi-byte character across the offset that was being sliced, so each one
    /// panicked before the fix. They are all reachable: the input is a URL out of
    /// a search response, and nothing upstream promises it is ASCII.
    #[test]
    fn url_prefix_tests_survive_multibyte_input() {
        // 'é' is two bytes, so byte 7 and byte 8 land mid-character.
        assert!(!ddg::is_http("héllo://x"));
        assert!(!ddg::is_http("httpé"));
        assert!(!ddg::is_http("——————"));
        assert!(!ddg::is_http(""));
        // Still recognises the real thing.
        assert!(ddg::is_http("http://a.example"));
        assert!(ddg::is_http("HTTPS://a.example"));
        // And the prefix stripper, whose offset is the caller's prefix length.
        assert_eq!(query::strip_prefix_ci("wwé.example", "www."), None);
        assert_eq!(query::strip_prefix_ci("é", "www."), None);
        assert_eq!(
            query::strip_prefix_ci("WWW.example", "www."),
            Some("example")
        );
    }

    #[test]
    fn parse_results_filters_takes_four_and_falls_back() {
        let body = r#"{"results":[
          {"image":"https://a.example.com/1.png","thumbnail":"https://p.duckduckgo.com/1",
           "url":"https://en.wikipedia.org/wiki/A","width":512,"height":512,"title":"A","source":"Bing"},
          {"image":"ftp://nope/2.png","thumbnail":"https://p/2","url":"https://b.com/x"},
          {"image":"https://c.example.com/3.png","url":"https://www.wmgn.com/logo","source":"Bing"},
          {"image":"https://d.example.com/4.png","thumbnail":"https://p/4","width":300},
          {"image":"https://e.example.com/5.png","thumbnail":"https://p/5","url":""},
          {"image":"https://f.example.com/6.png"}
        ]}"#;
        let r = ddg::parse_results(body, 4);
        assert_eq!(r.len(), 4);
        // The non-http entry is dropped BEFORE the take, so it costs nobody a slot.
        assert_eq!(r[0].source, "wikipedia.org");
        assert_eq!(r[0].width, Some(512));
        // A missing thumbnail falls back to the full-size image.
        assert_eq!(r[1].image, "https://c.example.com/3.png");
        assert_eq!(r[1].thumbnail, "https://c.example.com/3.png");
        assert_eq!(r[1].source, "wmgn.com");
        // One dimension present is the same as none — the caption needs both.
        assert_eq!(r[2].width, Some(300));
        assert_eq!(r[2].height, None);
        // No page URL: the origin falls through to the image URL, never to DDG's
        // "Bing" provider label.
        //
        // AND THE FALLBACK COLLAPSES TOO. `hostOf` is applied to the image URL by
        // the same expression that applies it to the page URL
        // (logoDuckDuckGo.ts:88 — `hostOf(r.url) || hostOf(r.image) || r.source`),
        // so `e.example.com` loses its leading label exactly as `en.wikipedia.org`
        // does above. This line read "e.example.com" when it was written, which
        // asserted a second, uncollapsed rule for the fallback that neither the
        // TypeScript nor `host_of`'s own test has.
        assert_eq!(r[3].source, "example.com");
        assert!(ddg::parse_results("not json", 4).is_empty());
        assert!(ddg::parse_results(r#"{"results":null}"#, 4).is_empty());
    }

    #[test]
    fn json_reads_escapes_and_rejects_trailing_junk() {
        let v = json::parse(r#"{"a":"xé\n","b":[1,2.5,true,null]}"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_str(), Some("x\u{00E9}\n"));
        assert_eq!(v.get("b").unwrap().as_arr().unwrap().len(), 4);
        assert!(json::parse("{} trailing").is_none());
        assert!(json::parse("{\"a\":}").is_none());
        assert_eq!(json::quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    // ── candidate cells ─────────────────────────────────────────────────────

    #[test]
    fn candidate_labels() {
        use candidate::*;
        assert_eq!(dims_label(Some(512), Some(512)), "512\u{00D7}512");
        assert_eq!(dims_label(Some(512), None), "");
        assert_eq!(dims_label(None, None), "");
        assert_eq!(alt_label(2, "wikipedia.org"), "Logo option 3 from wikipedia.org");
        assert_eq!(alt_label(0, ""), "Logo option 1");
        assert_eq!(domain_caption(""), "image");
        assert_eq!(domain_caption("wmgn.com"), "wmgn.com");
    }

    #[test]
    fn monogram_drops_the_leading_k_or_w() {
        use candidate::monogram;
        assert_eq!(monogram("WMGN"), "MGN");
        assert_eq!(monogram("KQRS-FM"), "QRS");
        assert_eq!(monogram("K227EA"), "K227");
        assert_eq!(monogram("BBC"), "BBC");
        assert_eq!(monogram(""), "?");
    }

    #[test]
    fn candidate_background_adopts_only_a_uniform_opaque_corner() {
        // Opaque and uniform: the art carries its own backing.
        let plate = field(16, 16, [240, 240, 240, 255]);
        assert_eq!(candidate::candidate_background(&plate), [240, 240, 240]);
        // Transparent corners: the cell must not tint itself from nothing.
        let mut clear = field(16, 16, CLEAR);
        block(&mut clear, 4, 4, 8, 8, RED);
        assert_eq!(candidate::candidate_background(&clear), [255, 255, 255]);
        // Corners that disagree: white, which is what the shipping build shows.
        let mut split = field(16, 16, WHITE);
        block(&mut split, 0, 0, 8, 16, [10, 10, 10, 255]);
        assert_eq!(candidate::candidate_background(&split), [255, 255, 255]);
    }

    // ── byte rounding ───────────────────────────────────────────────────────

    /// The two rules are not interchangeable, and the difference is exactly one
    /// LSB on a tie — which is invisible until a golden image is diffed.
    #[test]
    fn byte_conversion_follows_the_two_javascript_rules() {
        // Uint8ClampedArray: clamp, then round half to EVEN.
        assert_eq!(clamped_u8(0.5), 0);
        assert_eq!(clamped_u8(1.5), 2);
        assert_eq!(clamped_u8(2.5), 2);
        assert_eq!(clamped_u8(3.5), 4);
        assert_eq!(clamped_u8(191.25), 191);
        assert_eq!(clamped_u8(-4.0), 0);
        assert_eq!(clamped_u8(300.0), 255);
        assert_eq!(clamped_u8(f64::NAN), 0);
        // Math.round: half UP.
        assert_eq!(js_round_u8(0.5), 1);
        assert_eq!(js_round_u8(2.5), 3);
        assert_eq!(js_round_u8(-0.4), 0);
        assert_eq!(js_round_u8(999.0), 255);
    }

    // ── prep ────────────────────────────────────────────────────────────────

    #[test]
    fn mark_bounds_finds_the_mark_on_transparent_and_on_paper() {
        let mut clear = field(256, 256, CLEAR);
        block(&mut clear, 96, 96, 64, 64, RED);
        let b = prep::mark_bounds(&clear).unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (96, 96, 159, 159));

        // The same mark on an opaque WHITE field crops identically: white is paper.
        let mut paper = field(256, 256, WHITE);
        block(&mut paper, 96, 96, 64, 64, RED);
        let b = prep::mark_bounds(&paper).unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (96, 96, 159, 159));
    }

    /// The load-bearing case. A saturated border is the LOGO — a solid badge —
    /// and cropping to what sits on it would destroy the badge.
    #[test]
    fn mark_bounds_never_crops_into_a_saturated_badge() {
        let mut badge = field(256, 256, [176, 42, 110, 255]);
        block(&mut badge, 96, 96, 64, 64, WHITE);
        let b = prep::mark_bounds(&badge).unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0, 0, 255, 255));
        // …and the display rendition is therefore the whole image, uncropped.
        let out = prep::display_rendition(&badge);
        assert_eq!((out.w, out.h), (256, 256));
    }

    #[test]
    fn display_rendition_refuses_a_tiny_or_pointless_crop() {
        // Below the 8×8 floor: a 4×4 mark is not worth its own file.
        let mut tiny = field(64, 64, CLEAR);
        block(&mut tiny, 10, 10, 4, 4, RED);
        assert_eq!(prep::display_rendition(&tiny).w, 64);

        // Below MIN_GAIN: shaving a 1px border off 256² gains 1.6%.
        let mut edge = field(256, 256, WHITE);
        block(&mut edge, 1, 1, 254, 254, RED);
        assert_eq!(prep::display_rendition(&edge).w, 256);

        // Worth taking.
        let mut good = field(256, 256, CLEAR);
        block(&mut good, 96, 96, 64, 64, RED);
        assert_eq!((prep::display_rendition(&good).w, prep::display_rendition(&good).h), (64, 64));
    }

    #[test]
    fn resample_is_an_alpha_weighted_area_average() {
        let mut r = Raster::empty(2, 2);
        r.set(0, 0, [255, 0, 0, 255]);
        r.set(1, 0, [0, 255, 0, 255]);
        r.set(0, 1, [0, 0, 255, 255]);
        // A fully transparent pixel contributes to alpha and NOTHING to colour —
        // this is the weighting that keeps padding from fringing the mark.
        r.set(1, 1, [255, 255, 255, 0]);
        let out = prep::resample_raster(&r, 1);
        assert_eq!((out.w, out.h), (1, 1));
        assert_eq!(out.rgba, vec![85, 85, 85, 191]);
        // Already smaller: returned untouched.
        assert_eq!(prep::resample_raster(&r, 8).rgba, r.rgba);
    }

    #[test]
    fn the_ladder_skips_sizes_the_master_is_already_under() {
        let src = field(300, 100, WHITE);
        let written: Vec<u32> = prep::ladder_rasters(&src).iter().map(|(s, _)| *s).collect();
        // 512 is skipped (never upscale); 256 then 128, largest first.
        assert_eq!(written, vec![256, 128]);
        let sizes = prep::ladder_rasters(&src);
        assert_eq!((sizes[0].1.w, sizes[0].1.h), (256, 85));
        assert_eq!((sizes[1].1.w, sizes[1].1.h), (128, 43));
    }

    #[test]
    fn ladder_for_picks_the_smallest_that_covers_the_box() {
        use prep::ladder_for;
        let all = [128u32, 256, 512];
        assert_eq!(ladder_for(Some(64.0), 2.0, &all), Some(128));
        assert_eq!(ladder_for(Some(129.0), 2.0, &all), Some(512));
        assert_eq!(ladder_for(Some(192.0), 1.0, &all), Some(256));
        // The density clamp: a 3× panel is treated as 2×.
        assert_eq!(ladder_for(Some(64.0), 3.0, &all), Some(128));
        // Nothing written, or nothing big enough → the full-size file.
        assert_eq!(ladder_for(Some(64.0), 2.0, &[]), None);
        assert_eq!(ladder_for(Some(64.0), 2.0, &[256]), Some(256));
        assert_eq!(ladder_for(Some(600.0), 2.0, &all), None);
        // The hero asks for no box at all.
        assert_eq!(ladder_for(None, 2.0, &all), None);
    }

    // ── colour maths ────────────────────────────────────────────────────────

    #[test]
    fn oklab_round_trips_within_one_lsb() {
        use dark::oklab::{lab_to_srgb8, srgb8_to_lab};
        for r in [0u8, 17, 64, 128, 200, 255] {
            for g in [0u8, 33, 96, 160, 255] {
                for b in [0u8, 48, 112, 255] {
                    let lab = srgb8_to_lab(r, g, b);
                    let back = lab_to_srgb8(lab[0], lab[1], lab[2]);
                    for (a, e) in back.iter().zip([r, g, b]) {
                        assert!(
                            (*a as i32 - e as i32).abs() <= 1,
                            "{r},{g},{b} -> {back:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn chroma_scaling_preserves_hue() {
        use dark::oklab::{chroma, srgb8_to_lab};
        let lab = srgb8_to_lab(176, 42, 110);
        let before = lab[2].atan2(lab[1]);
        let (a, b) = (lab[1] * 0.85, lab[2] * 0.85);
        assert!((b.atan2(a) - before).abs() < 1e-9, "hue moved");
        assert!(chroma(a, b) < chroma(lab[1], lab[2]));
    }

    #[test]
    fn box_blur_spreads_a_delta_and_leaves_a_flat_plane_alone() {
        let mut plane = vec![0f32; 49];
        plane[3 * 7 + 3] = 1.0;
        let out = dark::blur::box_blur(&plane, 7, 7, 1.0, 1);
        assert!((out[3 * 7 + 3] - 1.0 / 9.0).abs() < 1e-5);
        assert!((out.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        let flat = vec![0.5f32; 49];
        for v in dark::blur::box_blur(&flat, 7, 7, 2.5, 3) {
            assert!((v - 0.5).abs() < 1e-5);
        }
    }

    #[test]
    fn border_connected_leaves_an_enclosed_hole_alone() {
        // A ring plus one isolated centre pixel. The flood reaches the ring and
        // must not reach the centre — this is what keeps the counter of an "O"
        // opaque when the plate around it is keyed away.
        let (w, h) = (5usize, 5usize);
        let mut mask = vec![0u8; w * h];
        for x in 0..w {
            mask[x] = 1;
            mask[(h - 1) * w + x] = 1;
        }
        for y in 0..h {
            mask[y * w] = 1;
            mask[y * w + w - 1] = 1;
        }
        mask[2 * w + 2] = 1;
        let out = dark::labeling::border_connected_mask(&mask, w, h);
        assert_eq!(out[0], 1);
        assert_eq!(out[2 * w + 2], 0);
        assert_eq!(out.iter().map(|v| *v as usize).sum::<usize>(), 16);
    }

    #[test]
    fn connected_components_counts_two_blobs() {
        let (w, h) = (5usize, 5usize);
        let mut mask = vec![0u8; w * h];
        mask[0] = 1;
        mask[1] = 1;
        mask[w] = 1; // three pixels, one blob
        mask[4 * w + 4] = 1; // one pixel, another
        let cc = dark::labeling::connected_components(&mask, w, h);
        assert_eq!(cc.count, 2);
        assert_eq!(cc.areas[cc.labels[0] as usize], 3);
        assert_eq!(cc.areas[cc.labels[4 * w + 4] as usize], 1);
    }

    // ── the dark stages ─────────────────────────────────────────────────────

    fn checkerboard(cell: u32, a: [u8; 4], b: [u8; 4]) -> Raster {
        let mut r = Raster::empty(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let on = ((x / cell) + (y / cell)).is_multiple_of(2);
                r.set(x, y, if on { a } else { b });
            }
        }
        r
    }

    #[test]
    fn flatten_detects_a_real_checkerboard() {
        let img = checkerboard(8, WHITE, [204, 204, 204, 255]);
        let (out, info) = dark::stages::flatten_checkerboard(&img);
        assert!(info.detected, "{}", info.note);
        assert_eq!(info.cell, Some(8));
        // Every grey cell became the dominant colour, so stage 3 sees one plane.
        assert!(out.rgba.chunks(4).all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255));
    }

    #[test]
    fn flatten_rejects_two_shades_that_are_really_one_background() {
        // |ΔL| < 0.02 — the guard that stops a near-flat background being read as
        // a checkerboard and flattened into a different colour.
        let img = checkerboard(8, WHITE, [254, 254, 254, 255]);
        let (_, info) = dark::stages::flatten_checkerboard(&img);
        assert!(!info.detected);
        assert!(info.note.contains("\u{0394}L"), "{}", info.note);
        // A saturated second colour is art, not a checkerboard.
        let (_, info) = dark::stages::flatten_checkerboard(&checkerboard(8, WHITE, RED));
        assert_eq!(info.note, "colours not neutral");
    }

    #[test]
    fn route_switches_family_at_eighty_percent_coverage() {
        use dark::stages::Treatment::*;
        assert_eq!(dark::stages::route(0.81), vec![Halo, AsIs]);
        assert_eq!(dark::stages::route(0.80), vec![Remap, Halo]);
        assert_eq!(dark::stages::route(0.79), vec![Remap, Halo]);
    }

    #[test]
    fn key_background_aborts_when_the_corners_disagree() {
        let mut split = field(16, 16, WHITE);
        block(&mut split, 0, 0, 8, 16, [10, 10, 10, 255]);
        let (out, info) = dark::stages::key_background(&split);
        assert!(!info.keyed);
        assert!(info.note.starts_with("corners disagree"), "{}", info.note);
        assert_eq!(out.rgba, split.rgba, "an aborted key must not touch the pixels");
    }

    #[test]
    fn remap_lifts_dark_ink_and_never_darkens() {
        use dark::oklab::srgb8_to_lab;
        let mut img = field(32, 32, CLEAR);
        block(&mut img, 8, 8, 16, 16, [0, 0, 0, 255]);
        let out = dark::stages::remap(&img);
        let j = out.at(16, 16);
        let after = srgb8_to_lab(out.rgba[j], out.rgba[j + 1], out.rgba[j + 2]);
        // Black ink inverts and is then capped at L = 0.92.
        assert!(after[0] > 0.85, "L after remap = {}", after[0]);
        assert!(after[0] <= 0.925);
        // No pixel anywhere came out darker than it went in.
        for i in 0..out.pixels() {
            let (a, b) = (i * 4, i * 4);
            if img.rgba[a + 3] > 128 && out.rgba[b + 3] > 128 {
                let before = srgb8_to_lab(img.rgba[a], img.rgba[a + 1], img.rgba[a + 2]);
                let now = srgb8_to_lab(out.rgba[b], out.rgba[b + 1], out.rgba[b + 2]);
                assert!(now[0] >= before[0] - 1e-3, "pixel {i} darkened");
            }
        }
    }

    #[test]
    fn remap_protects_a_white_wordmark() {
        // A white block covering 25% of the image — well over the 8% protect
        // threshold — is a wordmark, and clearing it would delete the station's
        // name from its own logo.
        let mut img = field(20, 20, [0, 0, 0, 255]);
        block(&mut img, 5, 5, 10, 10, WHITE);
        let out = dark::stages::remap(&img);
        let j = out.at(10, 10);
        assert_eq!(&out.rgba[j..j + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn gate_is_a_catastrophe_check() {
        use dark::stages::{gate, Candidate, Treatment};
        let bg = dark::LOGO_DARK_BG;
        let light = Candidate {
            treatment: Treatment::Remap,
            raster: field(8, 8, [200, 200, 200, 255]),
            plate: None,
        };
        assert!(gate(&light, bg).0, "{}", gate(&light, bg).1);
        // A wash barely different from the surface itself: unreadable.
        let wash = Candidate {
            treatment: Treatment::Remap,
            raster: field(8, 8, [42, 52, 66, 255]),
            plate: None,
        };
        assert!(!gate(&wash, bg).0);
        // Blown out to pure white — legible, but not what was drawn.
        let blown = Candidate {
            treatment: Treatment::Remap,
            raster: field(8, 8, [255, 255, 255, 255]),
            plate: None,
        };
        assert!(!gate(&blown, bg).0);
        // The floor always passes, whatever it is carrying.
        let plate = Candidate {
            treatment: Treatment::Plate,
            raster: field(8, 8, [42, 52, 66, 255]),
            plate: Some(dark::stages::plate_params()),
        };
        assert!(gate(&plate, bg).0);
    }

    #[test]
    fn the_pipeline_always_offers_the_plate_last() {
        // Deliberately NOT a centred square: a single centred square has equal
        // modal run lengths along rows and columns, which is the checkerboard
        // signature, and stage 1 would flatten the mark away. That is faithful
        // behaviour, not a bug — but it makes a poor fixture for stages 2 to 5.
        let mut img = field(64, 64, WHITE);
        block(&mut img, 10, 16, 40, 20, [20, 20, 20, 255]);
        let res = dark::pipeline::adapt_logo_for_dark(&img, dark::LOGO_DARK_BG);
        assert_eq!(res.candidates.len(), 3);
        assert_eq!(res.candidates.last().unwrap().treatment, dark::stages::Treatment::Plate);
        assert!(res.candidates[..2].iter().all(|c| c.treatment != dark::stages::Treatment::Plate));
        assert_eq!(res.candidates[0].treatment, res.order[0]);
    }

    #[test]
    fn a_human_override_survives_re_adaptation() {
        use dark::pipeline::choose_treatment;
        use dark::stages::{Candidate, PlateParams, Treatment};
        let mk = |t: Treatment| Candidate {
            treatment: t,
            raster: field(2, 2, WHITE),
            plate: None::<PlateParams>,
        };
        let cands = [mk(Treatment::Remap), mk(Treatment::Halo), mk(Treatment::Plate)];
        // Chosen, still routed: kept.
        assert_eq!(
            choose_treatment(Some(("HALO", true)), &cands, Treatment::Remap),
            (Treatment::Halo, true)
        );
        // Chosen, no longer routed: the auto-pick takes over.
        assert_eq!(
            choose_treatment(Some(("AS_IS", true)), &cands, Treatment::Remap),
            (Treatment::Remap, false)
        );
        // An AUTOMATIC prior is not an override and is simply replaced.
        assert_eq!(
            choose_treatment(Some(("HALO", false)), &cands, Treatment::Remap),
            (Treatment::Remap, false)
        );
        assert_eq!(choose_treatment(None, &cands, Treatment::Plate), (Treatment::Plate, false));
    }

    // ── the store ───────────────────────────────────────────────────────────

    #[test]
    fn safe_base_and_ext_map() {
        use store::{ext_for, safe_base};
        assert_eq!(safe_base("wmgn"), "WMGN");
        assert_eq!(safe_base("K227EA"), "K227EA");
        assert_eq!(safe_base("W249BC-FM"), "W249BC-FM");
        assert_eq!(safe_base("a b/c"), "A_B_C");
        assert_eq!(ext_for("image/png"), "png");
        assert_eq!(ext_for("IMAGE/JPEG"), "jpg");
        assert_eq!(ext_for("image/webp"), "webp");
        assert_eq!(ext_for("application/octet-stream"), "png");
    }

    #[test]
    fn a_replacement_master_wipes_every_derived_file_and_the_dark_treatment() {
        let tmp = TempDir::new("replace");
        let s = store::LogoStore::new(tmp.path());
        s.put_original("WMGN", b"first", "image/png", "manual").unwrap();
        s.put_derived("WMGN", "display.png", b"d").unwrap();
        s.put_derived("WMGN", "d-128.png", b"d").unwrap();
        s.set_display_meta("WMGN", 200, 100, vec![128]).unwrap();
        s.put_dark("WMGN", dark::stages::Treatment::Halo, b"k", true).unwrap();
        assert!(s.dark_info("WMGN").is_some());

        // A DIFFERENT mime, so the old master must go rather than sit alongside.
        s.put_original("WMGN", b"second", "image/jpeg", "manual").unwrap();
        let dir = s.dir("WMGN");
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"original.jpg".to_string()));
        assert!(!names.contains(&"original.png".to_string()));
        assert!(!names.contains(&"display.png".to_string()));
        assert!(!names.contains(&"d-128.png".to_string()));
        // Meta is REPLACED, not merged: a stale chosen=true would otherwise force
        // the old treatment onto the new art.
        let m = s.meta("WMGN").unwrap();
        assert!(m.dark.is_none());
        assert!(m.w.is_none());
        assert!(m.sizes.is_empty());
        assert_eq!(s.read_master("WMGN").unwrap(), b"second");
    }

    /// The hole CarFM leaves open, closed. Its `putOriginal` never checks the
    /// existing source, so a forced resolve would overwrite a hand-picked logo.
    #[test]
    fn an_automatic_source_cannot_overwrite_a_manual_master() {
        let tmp = TempDir::new("sticky");
        let s = store::LogoStore::new(tmp.path());
        s.put_original("WERN", b"mine", "image/png", "manual").unwrap();
        assert_eq!(
            s.put_original("WERN", b"theirs", "image/png", "ddg"),
            Err(store::StoreError::ManualLocked)
        );
        assert_eq!(s.read_master("WERN").unwrap(), b"mine");
        assert!(s.is_manual("WERN"));
        // Another MANUAL write is the one thing that may replace it.
        s.put_original("WERN", b"newer", "image/png", "manual").unwrap();
        assert_eq!(s.read_master("WERN").unwrap(), b"newer");
        // And a recorded miss must not quietly demote it either.
        assert_eq!(s.record_miss("WERN"), Err(store::StoreError::ManualLocked));
    }

    #[test]
    fn display_path_walks_ladder_then_trimmed_then_master() {
        let tmp = TempDir::new("paths");
        let s = store::LogoStore::new(tmp.path());
        s.put_original("WQLF", b"m", "image/png", "manual").unwrap();
        // No renditions yet: the untouched master is still a renderable source.
        assert_eq!(s.display_path("WQLF", Some(128.0), 2.0).unwrap().file_name().unwrap(), "original.png");
        s.set_display_meta("WQLF", 300, 100, vec![]).unwrap();
        assert_eq!(s.display_path("WQLF", Some(128.0), 2.0).unwrap().file_name().unwrap(), "display.png");
        s.set_display_meta("WQLF", 300, 100, vec![128, 256]).unwrap();
        assert_eq!(s.display_path("WQLF", Some(64.0), 2.0).unwrap().file_name().unwrap(), "d-128.png");
        assert_eq!(s.display_path("WQLF", None, 2.0).unwrap().file_name().unwrap(), "display.png");
        // A station with no folder costs no I/O and no guesswork.
        assert!(s.display_path("NOPE", Some(128.0), 2.0).is_none());
    }

    #[test]
    fn a_recorded_miss_has_a_timestamp_but_no_master() {
        let tmp = TempDir::new("miss");
        let s = store::LogoStore::with_clock(
            tmp.path(),
            Box::new(store::FixedClock::new(1_700_000_000_000)),
        );
        s.record_miss("WHHI").unwrap();
        let m = s.meta("WHHI").unwrap();
        assert_eq!(m.source, "none");
        assert_eq!(m.fetched_at, 1_700_000_000_000);
        assert!(!s.has_original("WHHI"));
        assert!(s.display_path("WHHI", Some(128.0), 2.0).is_none());
    }

    #[test]
    fn meta_survives_a_restart_through_the_index() {
        let tmp = TempDir::new("index");
        {
            let s = store::LogoStore::new(tmp.path());
            s.put_original("WMLI", b"m", "image/webp", "manual").unwrap();
            s.set_display_meta("WMLI", 240, 120, vec![128]).unwrap();
            s.put_dark("WMLI", dark::stages::Treatment::Remap, b"k", false).unwrap();
        }
        let s = store::LogoStore::new(tmp.path());
        let m = s.meta("WMLI").unwrap();
        assert_eq!(m.mime, "image/webp");
        assert_eq!(m.file.as_deref(), Some("original.webp"));
        assert_eq!(m.aspect, Some(2.0));
        assert_eq!(m.sizes, vec![128]);
        assert_eq!(m.dark.unwrap().treatment, "REMAP");
        assert_eq!(s.bases(), vec!["WMLI".to_string()]);
    }

    #[test]
    fn clearing_takes_the_hero_flags_with_it() {
        let tmp = TempDir::new("clear");
        let s = store::LogoStore::new(tmp.path());
        s.put_original("WMGN", b"m", "image/png", "manual").unwrap();
        s.put_original("WERN", b"m", "image/png", "manual").unwrap();
        s.set_prefs("WMGN", prefs::HeroFlags { show_call: true, show_freq: false });
        assert_eq!(s.prefs("WMGN"), Some(prefs::HeroFlags { show_call: true, show_freq: false }));
        assert_eq!(s.clear_all(), 2);
        assert!(s.bases().is_empty());
        // The flags default off a logo's existence, so leaving them behind would
        // give a logo-less station a blank hero.
        assert_eq!(s.prefs("WMGN"), None);
        assert!(!s.has_original("WMGN"));
    }

    // ── hero flags ──────────────────────────────────────────────────────────

    #[test]
    fn hero_flag_defaults_depend_on_the_logo() {
        use prefs::{effective, HeroFlags};
        assert_eq!(effective(None, false), HeroFlags { show_call: true, show_freq: true });
        assert_eq!(effective(None, true), HeroFlags { show_call: false, show_freq: false });
        let explicit = HeroFlags { show_call: true, show_freq: false };
        assert_eq!(effective(Some(explicit), false), explicit);
        assert_eq!(effective(Some(explicit), true), explicit);
        assert_eq!(prefs::ON_NEW_LOGO, HeroFlags { show_call: false, show_freq: false });
    }

    // ── the resolver ────────────────────────────────────────────────────────

    struct FakeSource {
        name: &'static str,
        url: Option<String>,
    }

    impl resolver::LogoSource for FakeSource {
        fn name(&self) -> &'static str {
            self.name
        }
        fn applies(&self, _st: &resolver::LogoStation) -> bool {
            true
        }
        fn find(&self, _st: &resolver::LogoStation) -> Option<String> {
            self.url.clone()
        }
    }

    #[test]
    fn the_gate_is_shut_and_the_ttl_only_binds_background_callers() {
        use resolver::{should_resolve, Outcome, CACHE_TTL_MS};
        // The shipping configuration: nothing runs unless a user asked.
        assert_eq!(
            should_resolve(resolver::AUTO_LOGO_RESOLUTION, false, false, None, 0),
            Err(Outcome::Disabled)
        );
        assert!(should_resolve(resolver::AUTO_LOGO_RESOLUTION, true, false, None, 0).is_ok());
        // With auto on, both gates bind — and a forced resolve ignores both.
        assert_eq!(should_resolve(true, false, true, None, 0), Err(Outcome::HaveOriginal));
        assert!(should_resolve(true, true, true, None, 0).is_ok());
        assert_eq!(
            should_resolve(true, false, false, Some(0), CACHE_TTL_MS - 1),
            Err(Outcome::WithinTtl)
        );
        assert!(should_resolve(true, false, false, Some(0), CACHE_TTL_MS).is_ok());
        assert!(should_resolve(true, true, false, Some(0), 1).is_ok());
    }

    #[test]
    fn the_cascade_runs_in_order_and_falls_through_a_dead_url() {
        let tmp = TempDir::new("cascade");
        let s = store::LogoStore::new(tmp.path());
        let mut net = FakeNet::default();
        net.images.insert(
            "https://good/logo.png".into(),
            Ok(FetchedImage { bytes: encoded(&field(4, 4, RED)), mime: "image/png".into() }),
        );
        let first = FakeSource { name: "ddg", url: Some("https://dead/logo.png".into()) };
        let second = FakeSource { name: "wikidata", url: Some("https://good/logo.png".into()) };
        let third = FakeSource { name: "favicon", url: Some("https://good/logo.png".into()) };
        let sources: [&dyn resolver::LogoSource; 3] = [&first, &second, &third];
        let st = resolver::LogoStation { base: "WERN".into(), ..Default::default() };

        // A URL that will not download does not end the walk — the next source
        // gets its turn — and the source that WON is what meta records.
        let out = resolver::resolve_logo(&s, &RawCodec, &net, &sources, &st, true, 0);
        assert_eq!(out, resolver::Outcome::Stored { source: "wikidata" });
        assert_eq!(s.meta("WERN").unwrap().source, "wikidata");
        assert!(s.has_original("WERN"));
    }

    #[test]
    fn exhausting_every_source_records_a_miss() {
        let tmp = TempDir::new("exhaust");
        let s = store::LogoStore::with_clock(
            tmp.path(),
            Box::new(store::FixedClock::new(999)),
        );
        let net = FakeNet::default();
        let a = FakeSource { name: "ddg", url: None };
        let b = FakeSource { name: "wikidata", url: None };
        let sources: [&dyn resolver::LogoSource; 2] = [&a, &b];
        let st = resolver::LogoStation { base: "WHHI".into(), ..Default::default() };
        assert_eq!(
            resolver::resolve_logo(&s, &RawCodec, &net, &sources, &st, true, 0),
            resolver::Outcome::RecordedMiss
        );
        let m = s.meta("WHHI").unwrap();
        assert_eq!(m.source, "none");
        assert_eq!(m.fetched_at, 999);
        assert!(!s.has_original("WHHI"));
    }

    #[test]
    fn the_cascade_will_not_overwrite_a_manual_logo() {
        let tmp = TempDir::new("cascade-manual");
        let s = store::LogoStore::new(tmp.path());
        s.put_original("WMGN", b"mine", "image/png", "manual").unwrap();
        let mut net = FakeNet::default();
        net.images.insert(
            "https://good/logo.png".into(),
            Ok(FetchedImage { bytes: encoded(&field(4, 4, RED)), mime: "image/png".into() }),
        );
        let src = FakeSource { name: "ddg", url: Some("https://good/logo.png".into()) };
        let sources: [&dyn resolver::LogoSource; 1] = [&src];
        let st = resolver::LogoStation { base: "WMGN".into(), ..Default::default() };
        // Forced, so both TTL gates are open — and it still loses to the human.
        assert_eq!(
            resolver::resolve_logo(&s, &RawCodec, &net, &sources, &st, true, 0),
            resolver::Outcome::HaveOriginal
        );
        assert_eq!(s.read_master("WMGN").unwrap(), b"mine");
    }

    #[test]
    fn the_six_failure_strings_are_verbatim() {
        use resolver::fetch_error as e;
        assert_eq!(e::not_a_web_address(), "not a web address");
        assert_eq!(e::unreachable("timeout"), "couldn\u{2019}t reach the image (timeout)");
        assert_eq!(e::http_status(403), "image host returned HTTP 403");
        assert_eq!(e::unreadable("eof"), "couldn\u{2019}t read the image (eof)");
        assert_eq!(e::empty(), "image was empty");
        assert_eq!(
            e::too_large(2_100_000),
            "image is too large (2051 KB, max 1024 KB)"
        );
        assert_eq!(e::check_bytes(0), Some("image was empty".to_string()));
        assert_eq!(e::check_bytes(1024 * 1024), None);
        assert!(e::check_bytes(1024 * 1024 + 1).is_some());
        assert_eq!(e::mime_from_content_type(Some("image/webp; charset=x")), "image/webp");
        assert_eq!(e::mime_from_content_type(Some("application/octet-stream")), "image/png");
        assert_eq!(e::mime_from_content_type(None), "image/png");
    }

    #[test]
    fn wikidata_and_favicon_helpers() {
        use resolver::{favicon::pick_icon_url, wikidata};
        let u = wikidata::build_sparql("wern-fm");
        assert!(u.contains("wdt%3AP2317%20%22WERN%22"), "{u}");
        assert!(u.contains("LIMIT%201"));
        assert_eq!(
            wikidata::parse_sparql_logo(
                r#"{"results":{"bindings":[{"logo":{"value":"https://c/x.svg"}}]}}"#
            )
            .as_deref(),
            Some("https://c/x.svg")
        );
        assert_eq!(wikidata::parse_sparql_logo(r#"{"results":{"bindings":[]}}"#), None);

        // Priority: apple-touch-icon over og:image over rel=icon over the default.
        let html = r#"<link rel="icon" href="/fav.png">
                      <meta property="og:image" content="https://cdn/og.png">
                      <link rel="apple-touch-icon" href="touch.png">"#;
        assert_eq!(
            pick_icon_url(html, "https://wmgn.com/a/b.html").as_deref(),
            Some("https://wmgn.com/a/touch.png")
        );
        let html = r#"<meta property="og:image" content="//cdn/og.png">
                      <link rel="shortcut icon" href="/fav.png">"#;
        assert_eq!(
            pick_icon_url(html, "https://wmgn.com/").as_deref(),
            Some("https://cdn/og.png")
        );
        assert_eq!(
            pick_icon_url(r#"<link rel="ICON" href="/fav.png">"#, "https://wmgn.com/x")
                .as_deref(),
            Some("https://wmgn.com/fav.png")
        );
        assert_eq!(
            pick_icon_url("<html></html>", "https://wmgn.com/x").as_deref(),
            Some("https://wmgn.com/favicon.ico")
        );
    }

    // ── the window ──────────────────────────────────────────────────────────

    fn target() -> search::Target {
        search::Target {
            base: "WMGN".into(),
            callsign: "WMGN".into(),
            freq_mhz: 98.1,
            name: "Magic 98".into(),
        }
    }

    fn rows(n: usize) -> Vec<ddg::DdgImage> {
        (0..n)
            .map(|i| ddg::DdgImage {
                image: format!("https://big/{i}.png"),
                thumbnail: format!("https://thumb/{i}.png"),
                title: String::new(),
                width: Some(512),
                height: Some(512),
                source: "wikipedia.org".into(),
            })
            .collect()
    }

    #[test]
    fn the_window_opens_on_the_landing_view_never_on_a_search() {
        let mut m = search::Model::new();
        m.open(target(), false, None);
        let v = m.view();
        assert_eq!(v.state, search::State::Landing);
        assert_eq!(v.subtitle, "WMGN  \u{00B7}  98.1 MHz");
        assert_eq!(v.query, "radio 98.1 wmgn logo");
        assert_eq!(v.mono, "MGN");
        assert_eq!(v.search_label, "Search for a logo");
        assert_eq!(v.confirm_label, "Save");
        // No logo: both toggles on, and no hint — there is nothing to choose yet.
        assert!(v.show_call && v.show_freq);
        assert_eq!(v.hint, "");
        assert!(v.can_confirm);
    }

    #[test]
    fn a_station_with_a_logo_defaults_to_the_logo_only_hero() {
        let mut m = search::Model::new();
        m.open(target(), true, None);
        let v = m.view();
        assert!(!v.show_call && !v.show_freq);
        assert_eq!(v.search_label, "Search for a different logo");
        assert_eq!(v.hint, "Choose what shows on the hero");
        // An explicit choice outranks the logo-dependent default.
        m.open(target(), true, Some(prefs::HeroFlags { show_call: true, show_freq: false }));
        let v = m.view();
        assert!(v.show_call && !v.show_freq);
    }

    #[test]
    fn the_subtitle_drops_a_callsign_that_repeats_the_name() {
        assert_eq!(search::subtitle("WMGN", "WMGN", 98.1), "98.1 MHz");
        assert_eq!(search::subtitle("Magic 98", "", 98.1), "98.1 MHz");
        assert_eq!(search::subtitle("Magic 98", "WMGN", 98.1), "WMGN  \u{00B7}  98.1 MHz");
    }

    #[test]
    fn a_superseded_search_can_never_paint() {
        let mut m = search::Model::new();
        m.open(target(), false, None);
        let first = m.search().unwrap();
        assert_eq!(m.state(), search::State::Loading);
        // The user hits Search again before the first answer arrives.
        let second = m.retry().unwrap();
        assert_ne!(first.generation, second.generation);
        assert!(!m.results_arrived(first.generation, rows(4)));
        assert_eq!(m.state(), search::State::Loading, "a stale answer must not land");
        assert!(m.results_arrived(second.generation, rows(4)));
        assert_eq!(m.state(), search::State::Results);
        assert_eq!(m.cells().len(), 4);
        // Closing the window supersedes everything still in flight.
        let gen = m.generation();
        m.close();
        assert!(!m.results_arrived(gen, rows(4)));
    }

    #[test]
    fn an_empty_search_is_no_results_and_a_thrown_one_is_an_error() {
        let mut m = search::Model::new();
        m.open(target(), false, None);
        let j = m.search().unwrap();
        m.results_arrived(j.generation, vec![]);
        assert_eq!(m.state(), search::State::NoResults);
        assert!(!m.view().can_confirm);
        // The no-results copy is fixed and lives inline in the .slint, so nothing
        // is supplied for it here.
        assert_eq!(m.view().error_title, "");

        let j = m.retry().unwrap();
        m.search_failed(j.generation);
        let v = m.view();
        assert_eq!(v.state, search::State::Error);
        assert_eq!(v.error_title, "Search couldn\u{2019}t finish");
        assert_eq!(
            v.error_body,
            "Something went wrong reaching the logo search. Check your connection and try again."
        );
    }

    #[test]
    fn the_grid_ranks_nothing_and_the_user_picks() {
        let mut m = search::Model::new();
        m.open(target(), false, None);
        let j = m.search().unwrap();
        m.results_arrived(j.generation, rows(4));
        let v = m.view();
        assert_eq!(v.selected_index, -1, "nothing is pre-selected");
        assert!(!v.can_confirm);
        assert_eq!(v.hint, "Pick the correct logo");
        assert_eq!(v.confirm_label, "Confirm");
        // Arrival order, untouched.
        assert_eq!(m.cells()[0].image, "https://big/0.png");
        assert_eq!(m.cells()[3].image, "https://big/3.png");
        assert_eq!(m.cells()[0].dims, "512\u{00D7}512");
        assert_eq!(m.cells()[0].alt, "Logo option 1 from wikipedia.org");
        assert_eq!(m.cells()[0].domain, "wikipedia.org");

        m.pick(2);
        let v = m.view();
        assert_eq!(v.selected_index, 2);
        assert!(v.can_confirm);
        assert_eq!(v.hint, "Saved as this station\u{2019}s logo");
        // A tap outside the model changes nothing.
        m.pick(9);
        assert_eq!(m.view().selected_index, 2);
    }

    #[test]
    fn thumbnails_land_one_at_a_time_and_set_their_own_well() {
        let mut m = search::Model::new();
        m.open(target(), false, None);
        let j = m.search().unwrap();
        m.results_arrived(j.generation, rows(4));
        assert!(m.cells()[1].thumb.is_none());
        assert!(m.thumb_arrived(j.generation, 1, field(8, 8, [240, 240, 240, 255])));
        assert!(m.cells()[1].thumb.is_some());
        assert_eq!(m.cells()[1].background, [240, 240, 240]);
        // The other three are still empty and still drawing their captions.
        assert!(m.cells()[0].thumb.is_none());
        assert_eq!(m.cells()[0].background, [255, 255, 255]);
        assert!(!m.thumb_arrived(j.generation + 9, 0, field(8, 8, WHITE)));
    }

    #[test]
    fn confirm_on_the_landing_view_saves_only_the_toggles() {
        let mut m = search::Model::new();
        m.open(target(), true, None);
        m.toggle_call();
        assert_eq!(
            m.begin_confirm(),
            search::Confirm::SavePrefs {
                base: "WMGN".into(),
                flags: prefs::HeroFlags { show_call: true, show_freq: false },
            }
        );
        assert!(m.saving());
        assert_eq!(m.view().confirm_label, "Saving\u{2026}");
        assert!(!m.view().can_confirm, "every control is inert while saving");
        // A second press while the first is in flight does nothing.
        assert_eq!(m.begin_confirm(), search::Confirm::Ignore);
        m.saved();
        assert!(!m.saving());
    }

    #[test]
    fn confirm_with_a_pick_forces_the_logo_only_hero() {
        let mut m = search::Model::new();
        // A station with NO logo, so the toggles were seeded (true, true).
        m.open(target(), false, None);
        let j = m.search().unwrap();
        m.results_arrived(j.generation, rows(4));
        m.pick(1);
        match m.begin_confirm() {
            search::Confirm::AssignLogo { base, urls, flags } => {
                assert_eq!(base, "WMGN");
                // Full size first, DDG's proxied thumbnail second — the fallback
                // that makes the feature work when the big URL 403s.
                assert_eq!(urls, vec!["https://big/1.png", "https://thumb/1.png"]);
                // NOT (true, true): a new logo resets to the design default.
                assert_eq!(flags, prefs::HeroFlags { show_call: false, show_freq: false });
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn confirm_is_ignored_when_there_is_nothing_to_confirm() {
        let mut m = search::Model::new();
        assert_eq!(m.begin_confirm(), search::Confirm::Ignore, "no target");
        m.open(target(), false, None);
        let j = m.search().unwrap();
        assert_eq!(m.begin_confirm(), search::Confirm::Ignore, "still loading");
        m.results_arrived(j.generation, rows(4));
        assert_eq!(m.begin_confirm(), search::Confirm::Ignore, "nothing picked");
    }

    #[test]
    fn a_failed_save_reports_the_last_reason_it_saw() {
        let mut m = search::Model::new();
        m.open(target(), false, None);
        let j = m.search().unwrap();
        m.results_arrived(j.generation, rows(2));
        m.pick(0);
        m.begin_confirm();
        m.save_failed(resolver::fetch_error::http_status(403));
        let v = m.view();
        assert_eq!(v.state, search::State::Error);
        assert_eq!(v.error_title, "Couldn\u{2019}t save that logo");
        assert_eq!(
            v.error_body,
            "Couldn\u{2019}t save this logo \u{2014} image host returned HTTP 403. Try a different result."
        );
        assert!(!v.saving);
    }

    // ── assigning ───────────────────────────────────────────────────────────

    #[test]
    fn assign_falls_back_to_the_thumbnail_and_reports_the_last_failure() {
        let tmp = TempDir::new("assign");
        let s = store::LogoStore::new(tmp.path());
        let mut net = FakeNet::default();
        let art = {
            let mut r = field(200, 200, CLEAR);
            block(&mut r, 50, 50, 100, 100, RED);
            r
        };
        net.images.insert("https://big/0.png".into(), Err(resolver::fetch_error::http_status(403)));
        net.images.insert(
            "https://thumb/0.png".into(),
            Ok(FetchedImage { bytes: encoded(&art), mime: "image/png".into() }),
        );
        let urls = vec!["https://big/0.png".to_string(), "https://thumb/0.png".to_string()];
        assert!(assign::assign_from_urls(&s, &RawCodec, &net, "WMGN", &urls).is_ok());

        // The master is the bytes as downloaded, and it is marked manual.
        let m = s.meta("WMGN").unwrap();
        assert_eq!(m.source, "manual");
        assert_eq!(s.read_master("WMGN").unwrap(), encoded(&art));
        // The display rendition is the TRIMMED mark, not the padded original —
        // the hero derives its plate's aspect from these numbers.
        assert_eq!((m.w, m.h), (Some(100), Some(100)));
        assert_eq!(m.sizes, vec![]);
        // A dark variant exists without anyone opening a picker.
        assert!(m.dark.is_some());
        assert!(!m.dark.unwrap().chosen, "auto-pick is not a human's choice");

        // Both URLs dead: the reason reported is the LAST one tried.
        let mut dead = FakeNet::default();
        dead.images.insert("https://big/0.png".into(), Err("first".into()));
        dead.images.insert("https://thumb/0.png".into(), Err("second".into()));
        assert_eq!(
            assign::assign_from_urls(&s, &RawCodec, &dead, "WERN", &urls),
            Err("second".into())
        );
        assert_eq!(
            assign::assign_from_urls(&s, &RawCodec, &dead, "WERN", &[]),
            Err("no image address to download".into())
        );
    }

    #[test]
    fn a_large_master_gets_its_whole_ladder() {
        let tmp = TempDir::new("ladder");
        let s = store::LogoStore::new(tmp.path());
        let mut net = FakeNet::default();
        let mut art = field(600, 600, CLEAR);
        block(&mut art, 20, 20, 560, 560, RED);
        net.images.insert(
            "https://big/x.png".into(),
            Ok(FetchedImage { bytes: encoded(&art), mime: "image/png".into() }),
        );
        assign::assign_from_urls(
            &s,
            &RawCodec,
            &net,
            "WWHG",
            &["https://big/x.png".to_string()],
        )
        .unwrap();
        let m = s.meta("WWHG").unwrap();
        assert_eq!(m.sizes, vec![128, 256, 512]);
        assert!(s.dir("WWHG").join("d-128.png").is_file());
        assert!(s.dir("WWHG").join("k-128.png").is_file());
        // The tile asks for the small file, the hero for the full one.
        assert_eq!(
            s.display_path("WWHG", Some(128.0), 2.0).unwrap().file_name().unwrap(),
            "d-256.png"
        );
        assert_eq!(
            s.display_path("WWHG", None, 2.0).unwrap().file_name().unwrap(),
            "display.png"
        );
    }

    // ── the worker ──────────────────────────────────────────────────────────

    #[test]
    fn the_worker_answers_without_the_caller_ever_waiting() {
        use std::sync::mpsc::channel;
        let tmp = TempDir::new("worker");
        let store = std::sync::Arc::new(store::LogoStore::new(tmp.path()));
        let mut net = FakeNet { results: rows(4), ..Default::default() };
        for i in 0..4 {
            net.images.insert(
                format!("https://thumb/{i}.png"),
                Ok(FetchedImage {
                    bytes: encoded(&field(8, 8, [240, 240, 240, 255])),
                    mime: "image/png".into(),
                }),
            );
        }
        let (tx, rx) = channel::<service::Event>();
        let worker = service::Worker::spawn(
            store,
            std::sync::Arc::new(net),
            std::sync::Arc::new(RawCodec),
            Box::new(move |e| {
                let _ = tx.send(e);
            }),
        );

        let mut m = search::Model::new();
        m.open(target(), false, None);
        let job = m.search().unwrap();
        worker.search(&job);

        // The grid arrives BEFORE any thumbnail, so the cells appear at once.
        let first = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        match first {
            service::Event::Results { generation, rows } => {
                assert_eq!(generation, job.generation);
                assert_eq!(rows.len(), 4);
                assert!(m.results_arrived(generation, rows));
            }
            other => panic!("{other:?}"),
        }
        for _ in 0..4 {
            match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
                service::Event::Thumb { generation, index, art } => {
                    assert!(m.thumb_arrived(generation, index, art));
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(m.cells().iter().all(|c| c.thumb.is_some()));
    }

    #[test]
    fn a_failed_search_reaches_the_ui_as_an_error_not_a_panic() {
        use std::sync::mpsc::channel;
        let tmp = TempDir::new("worker-fail");
        let store = std::sync::Arc::new(store::LogoStore::new(tmp.path()));
        let net = FakeNet { search_fails: true, ..Default::default() };
        let (tx, rx) = channel::<service::Event>();
        let worker = service::Worker::spawn(
            store,
            std::sync::Arc::new(net),
            std::sync::Arc::new(RawCodec),
            Box::new(move |e| {
                let _ = tx.send(e);
            }),
        );
        let mut m = search::Model::new();
        m.open(target(), false, None);
        let job = m.search().unwrap();
        worker.search(&job);
        match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
            service::Event::SearchFailed { generation } => {
                assert!(m.search_failed(generation));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(m.view().error_title, "Search couldn\u{2019}t finish");
    }

    #[test]
    fn the_worker_persists_the_hero_flags_a_confirm_decided() {
        use std::sync::mpsc::channel;
        let tmp = TempDir::new("worker-prefs");
        let store = std::sync::Arc::new(store::LogoStore::new(tmp.path()));
        let (tx, rx) = channel::<service::Event>();
        let worker = service::Worker::spawn(
            store.clone(),
            std::sync::Arc::new(FakeNet::default()),
            std::sync::Arc::new(RawCodec),
            Box::new(move |e| {
                let _ = tx.send(e);
            }),
        );
        let mut m = search::Model::new();
        m.open(target(), true, None);
        m.toggle_freq();
        worker.submit(m.begin_confirm());
        match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
            service::Event::Saved { base } => assert_eq!(base, "WMGN"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            store.prefs("WMGN"),
            Some(prefs::HeroFlags { show_call: false, show_freq: true })
        );
        // `Ignore` is not a job.
        worker.submit(search::Confirm::Ignore);
    }
}
