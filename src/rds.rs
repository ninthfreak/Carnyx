//! RDS group decoder for the NWD head-unit tuner, and the two presentation joins
//! that finish its output before Slint sees it.
//!
//! LIFTED, not rewritten. The decoder below comes across from CarFM's
//! `core/rds/src/lib.rs` (e12ca06, 2026-08-08, "Land the Rust RDS core, re-ported
//! against the current decoder"), which is itself the Rust port of
//! `src/services/nwdRds.ts`. Both are CarFM-original: the TypeScript header
//! records the decision NOT to feed the inherited vibedsp C++ `RdsDecoder`, so
//! nothing here descends from VibeSDR.
//!
//! Every threshold, every trust gate and every refusal below was established
//! against real drive logs, and the reasons are kept with them — this is not a
//! clean-room rewrite and must not drift from the behaviour the logs justify. The
//! replay tests at the foot of the file are the fence: they feed real captured
//! hex and pin the values the head unit produced.
//!
//! The vendor framework class `com.nwd.app.NwdFmManager` exposes
//! `getRadioRDSDataArm()`, which returns ONE already-synchronised RDS group as 16
//! hex characters — 4 blocks × 2 bytes, A/B/C/D. All zeros means no group that
//! poll, and so does a Java null.
//!
//! Scope is what the device actually sends: PI, PTY, TP/TA, PS (group 0A/0B),
//! RadioText (2A/2B) and RT+ (3A plus the group the station assigns it to). AF is
//! NOT decoded — every 0A group in the logs carries COUNT=0, which is each
//! station stating outright that it has no alternate frequencies. The face's `af`
//! tell is therefore permanently false on this path.
//!
//! Reference: EN 50067 / IEC 62106.

/// Everything the decoder publishes. Cloned out on every change, so a caller can
/// hold a snapshot without borrowing the decoder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RdsState {
    /// Programme Identification — the broadcast's own station id.
    pub pi: Option<u16>,
    /// Programme Type, 0-31.
    pub pty: Option<u8>,
    /// Traffic Programme flag — "this station carries traffic announcements at
    /// all". Static per station, and consensus-gated on its OWN bit.
    pub tp: bool,
    /// Traffic Announcement in progress — "one is happening RIGHT NOW".
    ///
    /// THREE GUARDS, because an earlier build had none of them and a single
    /// corrupt group could raise this:
    ///   1. read only from group type 0 — bit 4 is TA there, but the RadioText
    ///      A/B flag in a 2A, so reading it everywhere reads noise;
    ///   2. its own consensus tally, not the PTY field's;
    ///   3. gated on a CONFIRMED TP — a station that carries no traffic cannot
    ///      be announcing any.
    ///
    /// Consensus is affordable despite TA needing to react promptly: an
    /// announcement runs tens of seconds, so three groups is well under a second
    /// against the length of the event.
    ///
    /// TA drives a tell and NOTHING else — it must never break the user's mute.
    /// An earlier build read bit 4 from every group under the PTY tally, and a
    /// single corrupt group both raised the pulsing tell and lifted the mute.
    pub ta: bool,
    /// Programme Service name, 8 chars, trimmed. Empty until fully assembled.
    pub ps: String,
    /// RadioText, up to 64 chars, trimmed. Empty until a terminator or full fill.
    pub rt: String,
    /// This station uses PS as a scrolling text field rather than as its name, so
    /// `ps` is not an identity and must not reach the hero.
    pub ps_scrolling: bool,
    /// RT+ artist, sliced out of `rt` by the station's own offsets. Empty unless
    /// the station broadcasts RT+ AND an item is currently running.
    pub rt_artist: String,
    /// RT+ title, same.
    pub rt_title: String,
}

/// RT+ ODA Application Identifier. A 16-bit exact match, which is what makes a
/// single declaration trustworthy: a corrupt block landing on this value by
/// chance is a 1-in-65536 event, on top of the group already having to be a
/// well-formed 3A carrying the confirmed PI.
const RTPLUS_AID: u16 = 0x4bd7;

/// RT+ content-type codes. The full class list runs to 63; these are the two the
/// face has fields for.
const RTPLUS_TITLE: u8 = 1;
const RTPLUS_ARTIST: u8 = 4;

/// The decoded RT+ payload — the running flag and both (content type, start,
/// length) triplets. Compared whole, so "it repeated" means every field did.
type RtPlusPayload = (bool, u8, usize, usize, u8, usize, usize);

/// Whether an ODA may legally ride in this group.
///
/// 0A/0B (PS), 1A/1B (PIN, slow labelling), 2A/2B (RadioText), 3A (the ODA
/// announcement itself), 4A (clock time), 10A (programme type name) and 15B
/// (fast basic tuning) all have uses fixed by the standard. An announcement
/// naming one of them is a corrupted announcement, not a station being unusual —
/// and believing it turns the groups this decoder already parses into RT+
/// payloads.
fn oda_group_is_legal(group: u8, ver_b: bool) -> bool {
    !matches!(
        (group, ver_b),
        (0, _) | (1, _) | (2, _) | (3, false) | (4, false) | (10, false) | (15, true)
    )
}

/// Distinct published PS values, on ONE station, that mean the PS is a scrolling
/// text field rather than a name.
///
/// The two-cycle rule was supposed to make this impossible: a scrolling PS never
/// repeats a complete assembly, so it never publishes. That holds for a FAST
/// scroller and fails for a slow one. WIBA-FM holds each 8-character chunk for
/// about four seconds — several complete assemblies — so consecutive assemblies
/// agree and every chunk publishes.
///
/// Measured on 2026-08-04 the separation is not subtle: WIBA published 16
/// distinct values in half an hour ("Walk", "This Way", "Aerosmit", "erosmith",
/// "NicoletL", "Law.com" …), while WERN published exactly one and WWHG exactly
/// one. Three is comfortably clear of both.
const PS_SCROLL_DISTINCT: usize = 3;

/// Identical PIs required before a value is trusted, or before a disagreement is
/// accepted as a real station change. RDS runs ~11.4 groups/s, so 3 is about a
/// quarter-second — fast enough to feel instant, long enough that scattered
/// block corruption never clears it.
const PI_CONFIRM: u32 = 3;

/// Identical PIs required to DISPLACE a PI already trusted — much higher than the
/// three it takes to acquire one, and deliberately so.
///
/// Acquisition should be fast: there is no incumbent to protect. Displacement
/// should be slow, because what this path catches is a station changing UNDER a
/// stationary dial, which happens over many seconds. The retune case does not
/// come through here at all — the tuner edge calls `reset_for_retune` on every
/// frequency event.
///
/// Three was not enough. On 2026-08-04, WERN's 0xA6FF was misread as 0x57FF three
/// times in a row during a fade, three separate times in one commute — enough to
/// clear the old threshold, wipe the accumulated PS and RadioText, and put the
/// formula's rendering of that number, "WBGX", on the hero in place of the logo.
/// Twelve groups is a little over a second of solid contradicting data, which a
/// fade-induced burst does not sustain and a genuine new station does trivially.
const PI_DISPLACE: u32 = 12;

/// Rolling quality window, in groups. At the ~5 groups/s that actually reach us
/// that is roughly a dozen seconds: long enough to be steady, short enough to
/// follow a drive under a bridge.
const QUALITY_RING: usize = 64;

/// Fewest outcomes before a percentage is worth quoting. Below this the answer is
/// "not enough data", NOT 100% — the failure mode to avoid is a station with
/// almost no groups reading as perfectly intact, which is exactly what the
/// "barely there" samples of 2026-08-05 did: 0% errors on 0.5 groups/s.
const QUALITY_MIN: usize = 16;

/// Reception quality since the last `reset_stats()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Well-formed, non-empty groups.
    pub groups: u32,
    /// Of those, the ones whose block A did not carry the trusted PI.
    pub pi_mismatch: u32,
}

/// Rolling reception quality over the last ~64 groups, for a live display.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quality {
    /// Share of recent groups whose block A carried the trusted PI. NOT a block
    /// error rate: this tuner hands over groups with NO per-block validity, so
    /// block A is the only one whose correctness can be judged at all. Errors in
    /// B, C and D are invisible — which is why RadioText arrives corrupt while
    /// this figure looks healthy, since the text lives in C and D. A proxy for
    /// channel quality, never "% intact", and it must never be labelled as one
    /// in the UI or in a diagnostic.
    pub pi_match_pct: Option<f64>,
    pub samples: usize,
}

/// One-line rendering of everything the decoder publishes.
///
/// Kept in CarFM's exact format on purpose: it is what CarFM's differential
/// harness and its Android adapter both print, so a Carnyx diagnostic line and a
/// CarFM log line from the same drive can be diffed against each other. That is
/// the only cross-check available until this runs in a car.
pub fn format_state(s: &RdsState, t: &Stats, q: &Quality) -> String {
    format!(
        "pi={} pty={} tp={} ta={} ps={:?} rt={:?} scroll={} art={:?} tit={:?} g={} x={} q={} n={}",
        s.pi.map(|v| format!("{v:04x}")).unwrap_or("-".into()),
        s.pty.map(|v| v.to_string()).unwrap_or("-".into()),
        s.tp,
        s.ta,
        s.ps,
        s.rt,
        s.ps_scrolling,
        s.rt_artist,
        s.rt_title,
        t.groups,
        t.pi_mismatch,
        q.pi_match_pct.map(|v| v.to_string()).unwrap_or("-".into()),
        q.samples
    )
}

/// Fold every non-printable byte to a space. Must NOT be applied before the
/// RadioText terminator test — see the note at that call site.
fn chr(byte: u8) -> char {
    if (0x20..=0x7e).contains(&byte) {
        byte as char
    } else {
        ' '
    }
}

pub struct RdsDecoder {
    st: RdsState,
    // Segment buffers. Held separately from `st` so a partially-assembled name is
    // never shown — a half-filled PS reads as a different station.
    ps_buf: [char; 8],
    ps_seen: u8,                 // bitmask of the 4 PS segments seen
    ps_candidate: String,        // previous COMPLETE assembly; must repeat to publish
    ps_seen_values: Vec<String>, // distinct published names — see PS_SCROLL_DISTINCT
    ps_scrolling: bool,          // ...and the verdict once there are too many
    rt_buf: [char; 64],
    rt_seen: u16,              // bitmask of the 16 RT segments seen
    rt_end: Option<usize>,     // index of the 0x0D terminator, once seen
    rt_end_seg: Option<u16>,   // and which segment carried it
    rt_ab: Option<u8>,         // A/B flag; a flip means a new message
    rt_candidate: String,      // previous COMPLETE assembly, corrupt or not
    rt_published: bool,        // has THIS message ever reached the face?
    pi_confirmed: Option<u16>, // the PI we trust; groups not carrying it are dropped
    pi_pending: Option<u16>,
    pi_pending_count: u32,
    stat_groups: u32,
    stat_pi_mismatch: u32,
    q_ring: [u8; QUALITY_RING],
    q_at: usize,
    q_count: usize,
    pty_pending: Option<u8>, // block B is corrupted independently of block A
    pty_count: u32,
    // TP rides in the SAME block as PTY but in its own bit, so the PTY consensus
    // says nothing about it: a corruption leaving bits 9-5 intact while flipping
    // bit 10 satisfied that gate and published a wrong TP off one group. Cheap to
    // fix properly because TP is static per station.
    tp_pending: Option<bool>,
    tp_count: u32,
    // TA is counted only on group 0, where bit 4 actually means TA — so this
    // tally advances far more slowly than TP's.
    ta_pending: Option<bool>,
    ta_count: u32,
    // RT+ (ODA, AID 0x4BD7). The group it rides in is NOT fixed by the standard —
    // the station announces it in a 3A group and may pick any type. WIBA uses 12A;
    // another station may use 11A. So the assignment is learned from the air per
    // station rather than hard-coded.
    rt_plus_group: Option<u8>,
    rt_plus_ver_b: bool,
    // An ODA assignment awaiting its repeat, and the last payload seen — both
    // gates exist because block B and blocks C/D carry no error protection.
    rt_plus_pending: Option<(u8, bool)>,
    rt_plus_last: Option<RtPlusPayload>,
}

impl Default for RdsDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RdsDecoder {
    pub fn new() -> Self {
        Self {
            st: RdsState::default(),
            ps_buf: [' '; 8],
            ps_seen: 0,
            ps_candidate: String::new(),
            ps_seen_values: Vec::new(),
            ps_scrolling: false,
            rt_buf: [' '; 64],
            rt_seen: 0,
            rt_end: None,
            rt_end_seg: None,
            rt_ab: None,
            rt_candidate: String::new(),
            rt_published: false,
            pi_confirmed: None,
            pi_pending: None,
            pi_pending_count: 0,
            stat_groups: 0,
            stat_pi_mismatch: 0,
            q_ring: [0; QUALITY_RING],
            q_at: 0,
            q_count: 0,
            pty_pending: None,
            pty_count: 0,
            tp_pending: None,
            tp_count: 0,
            ta_pending: None,
            ta_count: 0,
            rt_plus_group: None,
            rt_plus_ver_b: false,
            rt_plus_pending: None,
            rt_plus_last: None,
        }
    }

    /// Drop all accumulated text and per-station tallies. NOT the retune call —
    /// that is `reset_for_retune`, and the tuner edge calls it on every frequency
    /// event. This one is the building block: `push` uses it when a persistent new
    /// PI displaces the old one, and `reset_for_retune` builds on it.
    ///
    /// `pi_confirmed` is deliberately NOT cleared, which is exactly why this is
    /// wrong for a retune on its own: the old PI would stand as an incumbent the
    /// new station has to outvote. Clearing it here instead would drop the
    /// decoder into the no-incumbent path where any 3-group run of corruption is
    /// adopted outright, with no trusted PI to outvote it. `st.pi` is re-asserted
    /// from `pi_confirmed` by the next matching group — see the equality branch in
    /// `push`.
    pub fn reset(&mut self) {
        self.st = RdsState::default();
        self.ps_buf = [' '; 8];
        self.ps_seen = 0;
        self.ps_candidate = String::new();
        self.ps_seen_values.clear();
        self.ps_scrolling = false;
        self.rt_buf = [' '; 64];
        self.rt_seen = 0;
        self.rt_end = None;
        self.rt_end_seg = None;
        self.rt_ab = None;
        self.rt_candidate = String::new();
        self.rt_published = false;
        self.pty_pending = None;
        self.pty_count = 0;
        self.tp_pending = None;
        self.tp_count = 0;
        self.ta_pending = None;
        self.ta_count = 0;
        // The ODA assignment belongs to the station, so it goes with the rest of
        // the station's state on a retune.
        self.rt_plus_group = None;
        self.rt_plus_ver_b = false;
        self.rt_plus_pending = None;
        self.rt_plus_last = None;
        self.q_ring = [0; QUALITY_RING];
        self.q_at = 0;
        self.q_count = 0;
    }

    pub fn state(&self) -> RdsState {
        self.st.clone()
    }

    pub fn stats(&self) -> Stats {
        Stats {
            groups: self.stat_groups,
            pi_mismatch: self.stat_pi_mismatch,
        }
    }

    /// Drop everything INCLUDING the trusted PI. For a RETUNE, where the dial
    /// moved and the old PI is known-stale rather than an incumbent worth
    /// defending.
    ///
    /// `reset()` deliberately keeps `pi_confirmed`, which is right for the
    /// station change `push` detects itself (it overwrites it on the next line
    /// anyway). It is wrong for a retune: the new station's PI became DISSENT
    /// needing PI_DISPLACE consecutive matches rather than the three it takes to
    /// acquire from nothing. Measured on the TypeScript original: 12 groups
    /// against 3 on a clean signal, and far worse on real air, where the pending
    /// count resets on any differing block A and the drive logs run 30-35%
    /// block-A errors. Worse than slow — `push` DROPS every group whose PI does
    /// not match the incumbent, so PS, RadioText, PTY and RT+ were all discarded
    /// for the whole window.
    ///
    /// The exposure this leaves — a three-group run of corruption adopted with no
    /// incumbent to outvote it — is exactly what the app already accepts every
    /// time it starts up on a station.
    pub fn reset_for_retune(&mut self) {
        self.reset();
        self.pi_confirmed = None;
        self.pi_pending = None;
        self.pi_pending_count = 0;
    }

    /// Drop TA only, keeping all sticky station state. For an RDS EXPIRY (the
    /// carrier went quiet), not a retune.
    ///
    /// Everything else the decoder holds is station identity worth restoring when
    /// the carrier returns — PS, RadioText, PI, PTY, TP. TA is the exception: it
    /// means "an announcement is happening RIGHT NOW", and after a multi-second
    /// gap that is no longer safe to assume. Dropping the tally too forces it to
    /// re-confirm from fresh group-0s rather than a caller's restore path
    /// resurrecting a `ta` left over from before the gap — and without clearing
    /// the tally, a still-running announcement could not re-publish either.
    pub fn clear_ta(&mut self) {
        self.st.ta = false;
        self.ta_pending = None;
        self.ta_count = 0;
    }

    pub fn reset_stats(&mut self) {
        self.stat_groups = 0;
        self.stat_pi_mismatch = 0;
    }

    /// Separate from `stats` on purpose: those counters are reset every 15s by the
    /// debug sampler, and a display sharing them would blank on every sample.
    pub fn quality(&self) -> Quality {
        if self.q_count < QUALITY_MIN {
            return Quality {
                pi_match_pct: None,
                samples: self.q_count,
            };
        }
        let good: u32 = self.q_ring[..self.q_count].iter().map(|&v| v as u32).sum();
        // f64, and `(100 * good)` FIRST — the TypeScript computes
        // `(100 * good) / qCount` in doubles, and CarFM's differential harness
        // compares the printed value, so the width and the operation order both
        // matter if that comparison is ever run against this port.
        Quality {
            pi_match_pct: Some((100.0 * good as f64) / self.q_count as f64),
            samples: self.q_count,
        }
    }

    /// Feed one group as 16 hex chars. Returns the new state when a user-visible
    /// field changed, or `None` when nothing did — the common case, since the same
    /// group repeats many times per second.
    ///
    /// A VALIDATING WRAPPER, and nothing more. The body lives in
    /// [`push_blocks`](Self::push_blocks); this is the entry for callers that
    /// really do hold the group as text — the host's synthesised corpus in
    /// `crate::fake::FakeRdsStream`, and every test in this file, which is
    /// written against the wire format on purpose.
    ///
    /// A CALLER THAT ALREADY HAS FOUR `u16`s MUST NOT COME THROUGH HERE, and the
    /// device path did: `android::ingest_rds_group` parses the Java bridge's
    /// string into an `RdsGroup` at the edge, and `app.rs` then paid four
    /// `format!`s and a `collect::<String>()` per group to build a hex string
    /// back out of it so that this function could parse it a second time —
    /// about eleven times a second for the length of a drive.
    pub fn push(&mut self, hex: &str) -> Option<RdsState> {
        if hex.len() != 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        // The all-zero group — "no group this poll" — is refused one level down,
        // where both entries meet it. Sixteen '0' characters parse to four zero
        // blocks, so this needs no check of its own and must not grow one: two
        // copies of that rule is two rules to keep in step.
        let word = |i: usize| u16::from_str_radix(&hex[i..i + 4], 16).unwrap_or(0);
        self.push_blocks([word(0), word(4), word(8), word(12)])
    }

    /// The same, from the four blocks themselves.
    ///
    /// THE REAL BODY, and the ONLY place an all-zero group — "no group this
    /// poll" — is refused. [`push`](Self::push) deliberately has no check of its
    /// own: sixteen `'0'` characters parse to four zero blocks and arrive here,
    /// so both entries meet the rule by passing through this one. A second copy
    /// in the hex path would be a second rule to keep in step, and the two could
    /// then disagree about what a group IS.
    ///
    /// There is no length or digit check to make: four `u16`s cannot be
    /// malformed. That is the only validation the string entry adds.
    pub fn push_blocks(&mut self, blocks: [u16; 4]) -> Option<RdsState> {
        let [a, b, c, d] = blocks;
        if a == 0 && b == 0 && c == 0 && d == 0 {
            return None; // "no group this poll"
        }
        self.stat_groups += 1; // well-formed and carrying something

        let before = self.st.clone();

        // ── PI consensus ────────────────────────────────────────────────────
        //
        // These groups arrive with NO error correction applied. On a real drive
        // (2026-08-01, WERN 88.7, true PI a6ff) block A came back as a6ff, a63e,
        // 2631, a699, a6b8, d86f, c6fe, 47ff, 26f5 … — most groups corrupt.
        //
        // PI is repeated in EVERY group precisely so a receiver can use it to
        // reject bad blocks, and treating each new value as a station change was
        // the single cause of four separate faults: accumulated PS/RT wiped on
        // every corrupt group, PTY republished from garbage, and a corrupt PI
        // reaching the callsign lookup (WIBA showing "KDTI-ROCHE…").

        // Ring the block-A outcome for the rolling quality figure. Done here
        // because this is where the decision already exists; no incumbent means
        // there is nothing to judge against yet, so those groups are not counted.
        if let Some(pi) = self.pi_confirmed {
            self.q_ring[self.q_at] = u8::from(a == pi);
            self.q_at = (self.q_at + 1) % QUALITY_RING;
            if self.q_count < QUALITY_RING {
                self.q_count += 1;
            }
        }

        match self.pi_confirmed {
            None => {
                if Some(a) == self.pi_pending {
                    self.pi_pending_count += 1;
                } else {
                    self.pi_pending = Some(a);
                    self.pi_pending_count = 1;
                }
                if self.pi_pending_count < PI_CONFIRM {
                    return None; // not yet trusted
                }
                self.pi_confirmed = Some(a);
                self.st.pi = Some(a);
            }
            Some(pi) if a != pi => {
                self.stat_pi_mismatch += 1;
                if Some(a) == self.pi_pending {
                    self.pi_pending_count += 1;
                } else {
                    self.pi_pending = Some(a);
                    self.pi_pending_count = 1;
                }
                if self.pi_pending_count < PI_DISPLACE {
                    return None; // corrupt block — drop it
                }
                // Persistent disagreement: a real station change.
                self.reset();
                self.pi_confirmed = Some(a);
                self.pi_pending = Some(a);
                self.pi_pending_count = PI_DISPLACE;
                self.st.pi = Some(a);
            }
            Some(pi) => {
                self.pi_pending_count = 0; // a good group resets the dissent counter
                                           // Re-assert after an external reset(), which rebuilds st from
                                           // blank but deliberately keeps pi_confirmed. Without this, every
                                           // later group carrying the same PI lands here and never rewrites
                                           // st.pi, so PI stays null for the rest of the session.
                if self.st.pi.is_none() {
                    self.st.pi = Some(pi);
                }
            }
        }

        // Block B needs the SAME treatment, independently. A correct PI does not
        // mean a correct header: on the same drive, groups carrying the true a6ff
        // reported PTY 22, 15, 6, 8, 17 and 5 — block A survived while block B did
        // not. That is the genre label flickering between random genres, and PI
        // consensus alone does not touch it.
        let group_type = ((b >> 12) & 0xf) as u8;
        let version_b = (b >> 11) & 1 == 1;
        let pty = ((b >> 5) & 0x1f) as u8;
        if Some(pty) == self.pty_pending {
            self.pty_count += 1;
        } else {
            self.pty_pending = Some(pty);
            self.pty_count = 1;
        }
        if self.pty_count >= PI_CONFIRM {
            self.st.pty = Some(pty);
        }

        // Voted separately — see tp_pending. Same threshold, its own tally.
        let tp = (b >> 10) & 1 == 1;
        if Some(tp) == self.tp_pending {
            self.tp_count += 1;
        } else {
            self.tp_pending = Some(tp);
            self.tp_count = 1;
        }
        if self.tp_count >= PI_CONFIRM {
            self.st.tp = tp;
            // TA is conditioned on TP, so losing TP has to drop a latched TA with
            // it — otherwise a retune onto a non-traffic station could inherit the
            // previous one's announcement until the next group 0 arrives.
            if !tp {
                self.st.ta = false;
            }
        }

        if group_type == 0 {
            self.push_ps(b, d);
        } else if group_type == 2 {
            self.push_rt(b, c, d, version_b);
        }

        self.push_rt_plus(b, c, d, group_type, version_b);

        if self.st == before {
            None
        } else {
            Some(self.st.clone())
        }
    }

    /// 0A / 0B — Programme Service name. Two chars per group in block D, and the
    /// segment index is the low 2 bits. TA rides in bit 4 of the same block as
    /// PTY, so it inherits the same trust gate.
    fn push_ps(&mut self, b: u16, d: u16) {
        // Bit 4 carries TA here, and ONLY here: the same bit in a 2A group is the
        // RadioText A/B flag.
        let ta = (b >> 4) & 1 == 1;
        if Some(ta) == self.ta_pending {
            self.ta_count += 1;
        } else {
            self.ta_pending = Some(ta);
            self.ta_count = 1;
        }
        // st.tp, not the raw bit: a station that carries no traffic cannot be
        // announcing any. An unconfirmed TP suppresses TA too, which costs
        // nothing — TP is static and confirms off any three groups.
        if self.ta_count >= PI_CONFIRM {
            self.st.ta = self.st.tp && ta;
        }
        let seg = (b & 0x3) as usize;
        self.ps_buf[seg * 2] = chr((d >> 8) as u8);
        self.ps_buf[seg * 2 + 1] = chr((d & 0xff) as u8);
        self.ps_seen |= 1 << seg;
        if self.ps_seen != 0b1111 {
            return;
        }
        // Publish only when two CONSECUTIVE complete assemblies agree.
        //
        // Without this the display churns: segments keep arriving after the first
        // complete fill, so each one republishes a buffer that is half the old name
        // and half the new. Observed on device 2026-08-01 — "The load", "The cyad",
        // "Aue cy", "Auda94.9", "WOda94.9" before settling on "WOLX94.9".
        let cand: String = self.ps_buf.iter().collect();
        if cand == self.ps_candidate {
            // A repeated complete assembly. That used to be proof of a name; on a
            // slow scroller it only proves the chunk was held for a few seconds.
            // Count the distinct ones and stop trusting PS entirely once there are
            // too many — including retracting what was already shown, because those
            // earlier chunks were never a name either.
            let val = cand.trim().to_string();
            if !self.ps_scrolling {
                if !self.ps_seen_values.contains(&val) {
                    self.ps_seen_values.push(val.clone());
                }
                if self.ps_seen_values.len() >= PS_SCROLL_DISTINCT {
                    self.ps_scrolling = true;
                    self.st.ps_scrolling = true;
                    self.st.ps = String::new();
                } else {
                    self.st.ps = val;
                }
            }
        }
        self.ps_candidate = cand;
        // Start a clean cycle so the next assembly cannot inherit stale chars.
        self.ps_seen = 0;
        self.ps_buf = [' '; 8];
    }

    /// 2A / 2B — RadioText. 2A carries 4 chars (blocks C and D) at segment×4; 2B
    /// carries 2 chars (block D only) at segment×2, with block C repeating PI.
    fn push_rt(&mut self, b: u16, c: u16, d: u16, version_b: bool) {
        let seg = b & 0xf;
        let ab = ((b >> 4) & 1) as u8;
        if self.rt_ab.is_some() && Some(ab) != self.rt_ab {
            // A/B flipped: the broadcaster is starting a new message. Any RT+
            // markers we hold point into the OLD string and are meaningless
            // against the new one, so they go with it.
            self.st.rt_artist = String::new();
            self.st.rt_title = String::new();
            self.rt_buf = [' '; 64];
            self.rt_seen = 0;
            self.rt_end = None;
            self.rt_end_seg = None;
            self.rt_candidate = String::new();
            self.rt_published = false; // a genuinely new message shows at once
        }
        self.rt_ab = Some(ab);

        // 2B is 2A's second half: block C repeats the PI there, so the payload is
        // block D alone. One array and a slice rather than two vectors — the
        // TypeScript builds a fresh array per group, which is not worth copying.
        let raw = [
            (c >> 8) as u8,
            (c & 0xff) as u8,
            (d >> 8) as u8,
            (d & 0xff) as u8,
        ];
        let bytes: &[u8] = if version_b { &raw[2..] } else { &raw[..] };
        let base = if version_b { seg * 2 } else { seg * 4 } as usize;

        for (i, &byte) in bytes.iter().enumerate() {
            let at = base + i;
            if at >= 64 {
                continue;
            }
            // 0x0D ends the message. This MUST be tested on the RAW byte: chr()
            // folds every non-printable to a space, so sanitising first would
            // erase the terminator and the message would never publish.
            if byte == 0x0d {
                if self.rt_end.is_none() || at < self.rt_end.unwrap() {
                    self.rt_end = Some(at);
                    self.rt_end_seg = Some(seg);
                }
                continue;
            }
            self.rt_buf[at] = chr(byte);
        }
        self.rt_seen |= 1 << seg;

        // Same two-cycle rule as PS, and for the same reason. A terminator only
        // means "the message ends here" — it says nothing about whether the
        // segments BEFORE it ever arrived. Acquisition genuinely lands mid-cycle,
        // so publishing on the terminator alone put a fragment on the plate after
        // every retune: segments 2,3 then a CR in segment 4 rendered as "tta
        // Love". Require every segment up to the terminator's own.
        //
        // In u32: a terminator in segment 15 makes the shift amount 16, which
        // overflows a u16 — JavaScript's `(1 << 16) - 1` is what this mirrors, and
        // in Rust the same expression is a debug-build panic.
        let end_mask = match self.rt_end_seg {
            None => 0u16,
            Some(s) => ((1u32 << (s + 1)) - 1) as u16,
        };
        let terminated_and_complete =
            self.rt_end_seg.is_some() && (self.rt_seen & end_mask) == end_mask;
        if !terminated_and_complete && self.rt_seen != 0xffff {
            return;
        }
        let upto = self.rt_end.unwrap_or(64);
        let msg: String = self.rt_buf[..upto]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string();
        // FIRST FILL IS INSTANT, REPLACEMENTS MUST REPEAT.
        //
        // Blocks C and D carry the text itself and nothing protects them: PI
        // consensus guards block A, PTY consensus guards block B, and the
        // characters have no error check at all. On 2026-08-03, 44 of WERN's 83
        // RadioText publishes were corrupt — "Wisconsin PuAoic Radio", "Wisc h yn
        // Public Radio" — every one shown to the driver.
        //
        // Requiring two agreeing cycles for EVERYTHING was the previous rule and
        // it was the "RadioText took forever" complaint, so it only applies to
        // replacements. Corruption is random, so a corrupt assembly essentially
        // never repeats; a real message change costs one extra cycle.
        if !self.rt_published {
            self.st.rt = msg.clone();
            self.rt_published = true;
        } else if msg != self.st.rt && msg == self.rt_candidate {
            self.st.rt = msg.clone();
        }
        // Always the LAST assembly seen, agreeing or not — otherwise a stale
        // candidate could later be confirmed by an unrelated repeat.
        self.rt_candidate = msg;
        self.rt_seen = 0;
        self.rt_end = None;
        self.rt_end_seg = None;
        self.rt_buf = [' '; 64];
    }

    /// RT+ — 3A announces which group carries an ODA and which application it is.
    /// The five low bits of block B are the application group type: four bits of
    /// group number and one of version. Block D is the AID.
    fn push_rt_plus(&mut self, b: u16, c: u16, d: u16, group_type: u8, version_b: bool) {
        if group_type == 3 && !version_b {
            if d == RTPLUS_AID {
                let agt = (b & 0x1f) as u8;
                let group = agt >> 1;
                let ver_b = agt & 1 == 1;
                // THE AID PROVES BLOCK D, NOT BLOCK B. The 1-in-65536 argument
                // for trusting a single announcement covers the application id
                // only; the group NUMBER rides in block B's low bits with no
                // protection at all. A 3A with an intact AID and a corrupted
                // block B pointed RT+ at group 0A, after which ordinary PS
                // groups were parsed as RT+ payloads and set the artist from a
                // slice of the RadioText. Two guards, matching what every other
                // field here already has:
                if !oda_group_is_legal(group, ver_b) {
                    return; // the standard defines that group; it cannot be an ODA
                }
                if self.rt_plus_pending == Some((group, ver_b)) {
                    self.rt_plus_group = Some(group);
                    self.rt_plus_ver_b = ver_b;
                } else {
                    self.rt_plus_pending = Some((group, ver_b));
                }
            }
            return;
        }
        if self.rt_plus_group != Some(group_type) || version_b != self.rt_plus_ver_b {
            return;
        }
        // The payload is 37 bits spread across the five spare bits of B and all of
        // C and D: a toggle, a running flag, then two (content type, start,
        // length) triplets. Length markers are one less than the real length.
        let running = (b >> 3) & 1 == 1;
        let ct1 = ((((b & 0x7) << 3) | (c >> 13)) & 0x3f) as u8;
        let start1 = ((c >> 7) & 0x3f) as usize;
        let len1 = ((c >> 1) & 0x3f) as usize + 1;
        let ct2 = ((((c & 0x1) << 5) | (d >> 11)) & 0x3f) as u8;
        let start2 = ((d >> 5) & 0x3f) as usize;
        let len2 = (d & 0x1f) as usize + 1;
        // THE PAYLOAD NEEDS THE SAME TREATMENT. Blocks C and D carry the offsets
        // and nothing protects them — the very reason RadioText requires two
        // agreeing cycles. Unguarded, one corrupt group rewrote the artist from
        // "LED ZEPPELIN" to "EPPELIN", and a corrupt running bit blanked both
        // tags. Act only on a payload that has repeated, which costs one group
        // (~a second) against an item that runs for minutes.
        let payload = (running, ct1, start1, len1, ct2, start2, len2);
        if self.rt_plus_last != Some(payload) {
            self.rt_plus_last = Some(payload);
            return;
        }
        if !running {
            // The item has ENDED. Keeping the last song on screen over the next
            // one is the same staleness the rest of this decoder works to avoid.
            self.st.rt_artist = String::new();
            self.st.rt_title = String::new();
            return;
        }
        if !self.rt_published || self.st.rt.is_empty() {
            return;
        }
        // Offsets index into the RadioText AS TRANSMITTED. Applying them to a
        // half-assembled or corrupt string yields confident nonsense, so this only
        // ever runs against text that has passed the publish gate — and even then
        // a marker running off the end is rejected rather than clamped, because a
        // truncated artist is still a wrong artist.
        for (ct, start, len) in [(ct1, start1, len1), (ct2, start2, len2)] {
            let Some(v) = self.rt_slice(start, len) else {
                continue;
            };
            if ct == RTPLUS_ARTIST {
                self.st.rt_artist = v;
            } else if ct == RTPLUS_TITLE {
                self.st.rt_title = v;
            }
        }
    }

    /// Byte indices, matching the JavaScript's UTF-16 ones exactly: `chr()` admits
    /// only 0x20..0x7E, so every char in `rt` is one byte and one code unit. A
    /// non-ASCII `rt` cannot arise, and `get` returns None rather than panicking
    /// if that ever stops being true.
    fn rt_slice(&self, start: usize, len: usize) -> Option<String> {
        if start + len > self.st.rt.len() {
            return None;
        }
        let v = self.st.rt.get(start..start + len)?.trim().to_string();
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    }
}

/// The `rds` tell on the status bar: the station is speaking RDS at all.
///
/// A Rust decision, not a Slint one — it mirrors CarFM's
/// `rdsOk={!!liveStation.pi || !!liveStation.name}`. PI alone is enough: a
/// station whose PS never assembles (or whose PS was retracted as a scroller) is
/// still transmitting RDS.
pub fn rds_ok(st: &RdsState) -> bool {
    st.pi.is_some() || !st.ps.is_empty()
}

/// RBDS Programme Type labels, NRSC-4-B Annex F — North America.
///
/// RE-KEYED FROM THE STANDARD rather than lifted from CarFM's `ptyLabels.ts`,
/// which is byte-identical to the squashed import commit and whose only importer
/// at that commit was VibeSDR's `SDRScreen.tsx`. Its provenance could not be
/// settled either way, and this table is a fact list from a published standard,
/// so re-keying it costs ten minutes and removes the question entirely.
///
/// 16 and 17 are the standard's "Rhythm and Blues" / "Soft Rhythm and Blues",
/// abbreviated as CarFM abbreviated them: the genre line is one short row in the
/// status bar's centre column, and this is what the head unit has always shown.
///
/// 0 is "no programme type", 27 and 28 are unassigned in RBDS — all three render
/// as nothing, and `status-bar.slint` already hides the genre text when the
/// string is empty.
const RBDS: [&str; 32] = [
    "",
    "News",
    "Information",
    "Sports",
    "Talk",
    "Rock",
    "Classic Rock",
    "Adult Hits",
    "Soft Rock",
    "Top 40",
    "Country",
    "Oldies",
    "Soft",
    "Nostalgia",
    "Jazz",
    "Classical",
    "R&B",
    "Soft R&B",
    "Foreign Language",
    "Religious Music",
    "Religious Talk",
    "Personality",
    "Public",
    "College",
    "Spanish Talk",
    "Spanish Music",
    "Hip-Hop",
    "",
    "",
    "Weather",
    "Emergency Test",
    "Emergency",
];

/// PTY code → the finished label the genre line prints.
///
/// RBDS ONLY. The same 5-bit code means something different outside the Americas
/// — "5" is Rock in Madison and Education in Munich — and CarFM chose the table
/// by ITU region. That choice is hardcoded to region 2 on the face
/// (`RadioScreen.tsx:1804`), and Carnyx is a North American head unit, so the
/// region-1 table has no reachable caller here. It is deliberately absent rather
/// than carried dead: the IEC 62106 table is where it would come from if a caller
/// ever appears, NOT from `ptyLabels.ts`, whose region-1 half is the part with
/// the unresolved provenance.
pub fn pty_label(pty: Option<u8>) -> &'static str {
    match pty {
        // `pty <= 0 || pty > 31` in the TypeScript; the type makes the upper half
        // of that unreachable here, and 0 means "none" rather than an index.
        Some(p) if (1..=31).contains(&p) => RBDS[p as usize],
        _ => "",
    }
}

// ── The RadioText marquee's own trim ────────────────────────────────────────
//
// Ported from CarFM's `src/services/rtStation.ts` (157d71b, 2026-08-04).

/// Uppercase, alphanumerics only: "101.5 IBA-FM" → "1015IBAFM".
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Is this segment the station naming itself?
///
/// Matched on the dial frequency and the callsign ONLY. Not on the PS, which
/// would be catastrophic here: WIBA scrolls song titles through PS, so a PS of
/// "Walk" would have made this strip "Walk This Way" out of its own RadioText.
fn is_station_segment(seg: &str, freq_token: &str, call_token: &str) -> bool {
    let s = squash(seg);
    if s.is_empty() {
        return false;
    }
    (!freq_token.is_empty() && s.contains(freq_token))
        || (!call_token.is_empty() && s.contains(call_token))
}

/// JavaScript's `split(/\s+-\s+/)`, written out.
///
/// A SPACED hyphen only, which is the whole point: "IBA-FM" must survive as one
/// segment, and it does, because its hyphen has no surrounding whitespace. The
/// match is greedy and left-to-right, so the whitespace either side is consumed
/// and cannot start the next segment.
///
/// `is_ascii_whitespace` rather than a full Unicode `\s`: `chr()` admits only
/// 0x20..0x7E, so a space is the only whitespace RadioText can carry. (It also
/// omits 0x0B, which JavaScript's `\s` matches — the decoder folds that to a
/// space before this ever sees it.)
fn split_spaced_hyphen(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'-' {
            i += 1;
            continue;
        }
        let mut lead = i;
        while lead > start && b[lead - 1].is_ascii_whitespace() {
            lead -= 1;
        }
        let mut trail = i + 1;
        while trail < b.len() && b[trail].is_ascii_whitespace() {
            trail += 1;
        }
        if lead < i && trail > i + 1 {
            parts.push(&text[start..lead]);
            start = trail;
            i = trail;
        } else {
            i += 1;
        }
    }
    parts.push(&text[start..]);
    parts
}

/// Trim a station's own identifier off its RadioText.
///
/// WIBA-FM sends `101.5 IBA-FM - Walk This Way - Aerosmith`. The hero already
/// shows which station is tuned, in large type, so the first eleven characters of
/// the marquee are spent repeating it — on the one line where horizontal space is
/// scarcest and the interesting content sits furthest right.
///
/// WHAT THIS DELIBERATELY DOES NOT DO: split artist from title. The station's own
/// metadata does not distinguish them. From one hour of one drive, 2026-08-04:
///
///   101.5 IBA-FM - Whole Lotta Love - Led Zeppelin     title, then artist
///   101.5 IBA-FM - Walk This Way - Aerosmith           title, then artist
///   101.5 IBA-FM - Led Zeppelin - Kashmir              ARTIST, then title
///
/// Same station, same artist, both orders. Any rule that labelled one side
/// "artist" would be right most of the time and confidently wrong the rest, which
/// is worse than showing the pair unlabelled. `rt_artist` / `rt_title` exist for
/// genuine RT+ and this does not pretend to fill them.
///
/// Only the ENDS are trimmed, and only segments positively identified as the
/// station. Everything else — adverts, slogans, prose — passes through untouched.
pub fn strip_station_from_rt(rt: &str, mhz: Option<f32>, callsign: Option<&str>) -> String {
    let text = rt.trim();
    if text.is_empty() {
        return String::new();
    }
    // "101.5" → "1015". The station writes its own frequency the way the dial
    // displays it, so this matches without needing to know the format — which is
    // why it goes through the same one-decimal rendering the face uses.
    let freq = match mhz {
        Some(v) if v > 0.0 => squash(&crate::station::format_mhz(v)),
        _ => String::new(),
    };
    let call = squash(callsign.unwrap_or(""));
    if freq.is_empty() && call.is_empty() {
        return text.to_string();
    }

    let parts = split_spaced_hyphen(text);
    if parts.len() < 2 {
        return text.to_string(); // nothing to trim from
    }

    // isize, deliberately: both JavaScript walks may run their index off its own
    // end — `lo` past the last segment, `hi` to -1 — and `lo > hi` afterwards is
    // how "every segment was the station" is detected. A usize would have to
    // encode that case some other way, and encoding it differently is how a
    // transcription drifts.
    let mut lo: isize = 0;
    let mut hi: isize = parts.len() as isize - 1;
    while lo <= hi && is_station_segment(parts[lo as usize], &freq, &call) {
        lo += 1;
    }
    while hi >= lo && is_station_segment(parts[hi as usize], &freq, &call) {
        hi -= 1;
    }
    // The whole message was the station's name — a legitimate thing to broadcast,
    // so show it rather than blanking the marquee.
    if lo > hi {
        return text.to_string();
    }
    if lo == 0 && hi == parts.len() as isize - 1 {
        return text.to_string(); // nothing trimmed
    }
    parts[lo as usize..=hi as usize]
        .join(" - ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── The decoder's trust gates, replayed ──────────────────────────────────
    //
    // Ported from CarFM's `core/rds/tests/replay.rs`, which is itself a selection
    // from `tools/tests/nwdRds.test.mjs`. The hex groups are real captures from
    // the drive logs wherever a capture exists. If this decoder disagrees with any
    // assertion here it has diverged from the behaviour the logs justify.

    fn feed(d: &mut RdsDecoder, groups: &[&str], times: usize) {
        for _ in 0..times {
            for g in groups {
                d.push(g);
            }
        }
    }

    /// Groups are dropped until a PI has been seen PI_CONFIRM times, and block B
    /// is gated separately — the pushes that establish the PI return before block
    /// B is ever read, so PTY/TP need their own repeats after that. Five clears
    /// both.
    fn prime(group: &str) -> RdsDecoder {
        let mut d = RdsDecoder::new();
        for _ in 0..5 {
            d.push(group);
        }
        d
    }

    /// Real capture, PI a6ff. Four segments assembling "  WERN  ".
    const PS_WERN: [&str; 4] = [
        "a6ff02c0e0cd2020", // seg 0 → chars 0,1 = "  "
        "a6ff02c5e0cd5745", // seg 1 → chars 2,3 = "WE"
        "a6ff02c2e0cd524e", // seg 2 → chars 4,5 = "RN"
        "a6ff02c7e0cd2020", // seg 3 → chars 6,7 = "  "
    ];

    #[test]
    fn ps_publishes_only_when_two_cycles_agree() {
        let mut d = prime(PS_WERN[0]);
        feed(&mut d, &PS_WERN, 1);
        assert_eq!(d.state().ps, "", "one complete cycle is not enough");
        feed(&mut d, &PS_WERN, 1);
        assert_eq!(d.state().ps, "WERN", "a second agreeing cycle publishes");
        assert_eq!(d.state().pi, Some(0xa6ff), "PI comes from block A");
        assert_eq!(d.state().pty, Some(22), "PTY comes from block B bits 9-5");
        assert!(!d.state().tp, "TP is false for this station");
        // The genre line the head unit actually printed for this station.
        assert_eq!(pty_label(d.state().pty), "Public");
    }

    /// THE PREMISE THE RDS EXPIRY RESTS ON, and getting it wrong left the plate
    /// blank on a station that was playing perfectly well.
    ///
    /// `push` answers `Some` only when something CHANGED. After an expiry the
    /// face has been cleared but the decoder deliberately has not, so the first
    /// group back changes nothing in the decoder and `push` says `None` — which
    /// means a caller that only ever assigns what `push` returns would wait for
    /// a change that has no reason to come. The way out is `state()`, and this
    /// pins both halves: the None, and the fact that `state()` still holds the
    /// station that produced it.
    #[test]
    fn a_settled_station_stops_publishing_but_never_stops_knowing() {
        let mut d = prime(PS_WERN[0]);
        feed(&mut d, &PS_WERN, 1);
        feed(&mut d, &PS_WERN, 1);
        assert_eq!(d.state().ps, "WERN");

        // Another identical cycle. Nothing has changed, so nothing publishes.
        let mut published = None;
        for g in PS_WERN {
            if let Some(p) = d.push(g) {
                published = Some(p);
            }
        }
        assert!(published.is_none(), "an unchanged station publishes nothing");

        // And yet the decoder still knows exactly who it is listening to. This
        // is what the expiry's restore reads back.
        assert_eq!(d.state().ps, "WERN");
        assert_eq!(d.state().pi, Some(0xa6ff));
        assert_eq!(d.state().pty, Some(22));
    }

    #[test]
    fn an_incomplete_assembly_never_publishes() {
        let mut d = prime(PS_WERN[0]);
        feed(&mut d, &PS_WERN[..3], 4);
        assert_eq!(d.state().ps, "");
    }

    /// REGRESSION — a fast scrolling PS. Station 19e2 cycles its name as
    /// advertising copy; the first build showed every frame of it, overwriting the
    /// hero card.
    #[test]
    fn a_ps_that_never_repeats_is_never_published() {
        let mut d = prime("19e202c020204942");
        let scroll: [[&str; 4]; 2] = [
            [
                "19e202c020204942",
                "19e202c1312e3520",
                "19e202c220204942",
                "19e202c3412d3520",
            ],
            [
                "19e202c020204942",
                "19e202c1412d464d",
                "19e202c220203130",
                "19e202c3412d464d",
            ],
        ];
        for i in 0..6 {
            feed(&mut d, &scroll[i % 2], 1);
        }
        assert_eq!(d.state().ps, "");
    }

    /// Build the four 0A groups that spell an 8-char PS for PI 19e2.
    fn ps_groups(text: &str) -> Vec<String> {
        let mut t: Vec<u8> = text.bytes().collect();
        t.resize(8, b' ');
        (0..4u16)
            .map(|seg| {
                let b = 0x04c0u16 | seg;
                let s = seg as usize;
                let d = ((t[s * 2] as u16) << 8) | t[s * 2 + 1] as u16;
                format!("19e2{b:04x}e0cd{d:04x}")
            })
            .collect()
    }

    /// REGRESSION — a SLOW scrolling PS. The two-cycle rule assumed a scroller
    /// never repeats a complete assembly, which holds only if it scrolls fast.
    /// WIBA-FM holds each 8-character chunk for about four seconds — many complete
    /// assemblies — so every chunk cleared the rule and published. On 2026-08-04 it
    /// put "Walk", "This Way", "Aerosmit", "NicoletL", "Law.com" and eleven more
    /// onto the hero in place of the station logo.
    ///
    /// Measured that day: WIBA 16 distinct PS values, WERN 1, WWHG 1.
    #[test]
    fn a_slow_scroller_is_caught_and_retracted() {
        let first = ps_groups("Walk");
        let mut d = prime(&first[0]);

        let hold = |d: &mut RdsDecoder, text: &str| {
            let g = ps_groups(text);
            let refs: Vec<&str> = g.iter().map(String::as_str).collect();
            feed(d, &refs, 3);
        };

        hold(&mut d, "Walk");
        assert_eq!(d.state().ps, "Walk", "the first held chunk still publishes");
        assert!(!d.state().ps_scrolling);

        hold(&mut d, "This Way");
        assert_eq!(d.state().ps, "This Way", "so does the second");

        // The third distinct value is the verdict, and it RETRACTS what was shown —
        // those earlier chunks were never a name either.
        hold(&mut d, "Aerosmit");
        assert!(
            d.state().ps_scrolling,
            "three distinct values means scrolling"
        );
        assert_eq!(d.state().ps, "", "and the earlier chunks are withdrawn");

        hold(&mut d, "NicoletL");
        assert_eq!(d.state().ps, "", "the verdict sticks");
    }

    /// REGRESSION — PI consensus. On 2026-08-01, WERN's true a6ff arrived
    /// alongside a63e, 2631, a699, d86f, c6fe … Treating each as a station change
    /// wiped the accumulated PS and RT on every corrupt group.
    #[test]
    fn scattered_corrupt_pis_do_not_wipe_the_station() {
        let mut d = prime(PS_WERN[0]);
        feed(&mut d, &PS_WERN, 2);
        assert_eq!(d.state().ps, "WERN");

        for bad in ["a63e02c0e0cd2020", "263102c0e0cd2020", "d86f02c0e0cd2020"] {
            d.push(bad);
        }
        assert_eq!(d.state().ps, "WERN", "the name survives the corruption");
        assert_eq!(d.state().pi, Some(0xa6ff), "and so does the trusted PI");
    }

    /// A PERSISTENT disagreement is a real station change, and takes PI_DISPLACE
    /// groups rather than the three it takes to acquire from nothing.
    #[test]
    fn a_persistent_new_pi_displaces_the_old_one() {
        let mut d = prime(PS_WERN[0]);
        feed(&mut d, &PS_WERN, 2);
        assert_eq!(d.state().pi, Some(0xa6ff));

        for _ in 0..11 {
            d.push("19e202c0e0cd2020");
        }
        assert_eq!(d.state().pi, Some(0xa6ff), "eleven is not enough");
        d.push("19e202c0e0cd2020");
        assert_eq!(d.state().pi, Some(0x19e2), "the twelfth displaces it");
        assert_eq!(d.state().ps, "", "and the old station's text goes with it");
    }

    /// Block B is corrupted independently of block A: groups carrying the true
    /// a6ff reported PTY 22, 15, 6, 8, 17 and 5 on one drive.
    #[test]
    fn a_burst_of_one_off_ptys_never_reaches_the_label() {
        let mut d = prime(PS_WERN[0]);
        assert_eq!(d.state().pty, Some(22));
        for bad in ["a6ff01e0e0cd2020", "a6ff00c0e0cd2020", "a6ff0220e0cd2020"] {
            d.push(bad);
        }
        assert_eq!(d.state().pty, Some(22), "a single odd PTY is not adopted");
    }

    #[test]
    fn malformed_input_is_refused() {
        let mut d = RdsDecoder::new();
        assert!(d.push("").is_none());
        assert!(d.push("a6ff02c0e0cd20").is_none(), "too short");
        assert!(d.push("zzzz02c0e0cd2020").is_none(), "not hex");
        assert!(d.push("0000000000000000").is_none(), "no group this poll");
        assert_eq!(d.stats().groups, 0, "none of those count as a group");
    }

    /// THE TWO ENTRIES ARE ONE DECODER.
    ///
    /// `push` is a validating wrapper over `push_blocks`, and the device takes
    /// the second: the vendor hands the app four `u16`s, and building a hex
    /// string for the decoder to parse straight back cost four `format!`s and a
    /// `collect` per group, about eleven times a second. Every test in this file
    /// is written against the string form, so this is the one that says the form
    /// the DEVICE uses reaches the same state — and that the all-zero refusal,
    /// which is the one rule the string entry no longer holds itself, still
    /// holds on both.
    #[test]
    fn the_hex_entry_and_the_block_entry_decode_alike() {
        let blocks: [[u16; 4]; 4] = [
            [0xa6ff, 0x02c0, 0xe0cd, 0x2020],
            [0xa6ff, 0x02c5, 0xe0cd, 0x5745],
            [0xa6ff, 0x02c2, 0xe0cd, 0x524e],
            [0xa6ff, 0x02c7, 0xe0cd, 0x2020],
        ];
        let mut hexed = RdsDecoder::new();
        let mut blocked = RdsDecoder::new();
        for _ in 0..7 {
            for (h, b) in PS_WERN.iter().zip(blocks.iter()) {
                assert_eq!(hexed.push(h), blocked.push_blocks(*b), "group by group");
            }
        }
        assert_eq!(hexed.state().ps, "WERN", "the replay really did decode");
        assert_eq!(hexed.state(), blocked.state());
        assert_eq!(hexed.stats().groups, blocked.stats().groups);

        // The empty poll, on both entries and counted by neither.
        let before = blocked.stats().groups;
        assert!(hexed.push("0000000000000000").is_none());
        assert!(blocked.push_blocks([0, 0, 0, 0]).is_none());
        assert_eq!(blocked.stats().groups, before, "an empty poll is not a group");
    }

    /// The rolling figure stays silent until it has enough outcomes to mean
    /// anything. A station delivering almost nothing must not read as flawless.
    #[test]
    fn quality_is_silent_below_the_floor() {
        let mut d = prime(PS_WERN[0]);
        assert_eq!(
            d.quality().pi_match_pct,
            None,
            "5 groups is under the floor"
        );
        for _ in 0..20 {
            d.push(PS_WERN[0]);
        }
        let q = d.quality();
        assert!(q.samples >= 16);
        assert_eq!(q.pi_match_pct, Some(100.0), "every block A matched");
    }

    /// The loss figure the meter's dotting is fed comes from here: 100 minus the
    /// match percentage. Pinned because the subtraction is the only place the
    /// proxy becomes a number the driver sees.
    #[test]
    fn quality_counts_corrupt_block_as_and_the_loss_is_its_complement() {
        // Exactly PI_CONFIRM priming pushes, not `prime`'s five: the ring only
        // starts once a PI is trusted, and the group that confirms it is checked
        // against the old (absent) incumbent, so three leaves the ring empty.
        let mut d = RdsDecoder::new();
        for _ in 0..3 {
            d.push(PS_WERN[0]);
        }
        assert_eq!(d.quality().samples, 0, "nothing to judge against yet");
        // 16 outcomes, four of them corrupt → 75% match, 25% loss.
        for i in 0..16 {
            if i % 4 == 3 {
                d.push("a63e02c0e0cd2020");
            } else {
                d.push(PS_WERN[0]);
            }
        }
        let q = d.quality();
        assert_eq!(q.samples, 16);
        assert_eq!(q.pi_match_pct, Some(75.0));
        assert_eq!(100.0 - q.pi_match_pct.unwrap(), 25.0);
    }

    // ── RT+ ─────────────────────────────────────────────────────────────────
    // WIBA's 3A is verbatim from the log of 2026-08-01: 19e2 34d8 0000 4bd7. The
    // payload groups are synthesised, because 12A itself was never captured — only
    // the announcement that 12A is where RT+ lives.

    fn hex(a: u16, b: u16, c: u16, d: u16) -> String {
        format!("{a:04x}{b:04x}{c:04x}{d:04x}")
    }

    // ── RadioText, the last segment ─────────────────────────────────────────
    // A 0x0D terminator in SEGMENT 15 makes the completeness mask cover all
    // sixteen segments. JavaScript computes that as (1 << 16) - 1 = 0xFFFF; a u16
    // port overflows the shift — a panic under cargo test's debug profile, a
    // phantom empty publish in release. Both tests are regressions for that.

    #[test]
    fn a_lone_terminator_in_the_last_segment_neither_panics_nor_publishes() {
        const PI: u16 = 0xa6ff;
        let mut d = prime(&hex(PI, 0x0000, 0x0000, 0x2020));
        // Segment 15, CR at position 62, every earlier segment missing.
        d.push(&hex(PI, 0x200f, 0x2020, 0x0d20));
        assert_eq!(
            d.state().rt,
            "",
            "a lone terminated segment is not a message"
        );
    }

    #[test]
    fn a_message_terminating_in_the_last_segment_publishes_whole() {
        const PI: u16 = 0xa6ff;
        const MSG: &[u8] = b"Whole Lotta Love - Led Zeppelin - Royal Albert Hall, early set";
        let mut d = prime(&hex(PI, 0x0000, 0x0000, 0x2020));
        for seg in 0..16u16 {
            let ch = |k: usize| {
                let at = seg as usize * 4 + k;
                if at == 62 {
                    0x0d
                } else {
                    *MSG.get(at).unwrap_or(&b' ') as u16
                }
            };
            d.push(&hex(
                PI,
                0x2000 | seg,
                (ch(0) << 8) | ch(1),
                (ch(2) << 8) | ch(3),
            ));
        }
        assert_eq!(
            d.state().rt,
            std::str::from_utf8(MSG).unwrap().trim_end(),
            "first fill publishes on the terminator's own segment"
        );
    }

    /// REGRESSION — the fragment that used to reach the plate after every retune.
    /// Real capture: WERN's 2A stream joined mid-message, segments 2 and 3 then a
    /// CR in segment 4. Publishing on the terminator alone rendered "tta Love".
    #[test]
    fn a_message_joined_mid_cycle_does_not_publish_its_tail() {
        const PI: u16 = 0xa80b;
        let mut d = prime(&hex(PI, 0x0000, 0x0000, 0x2020));
        d.push(&hex(PI, 0x2002, 0x7474, 0x6120)); // seg 2 = "tta "
        d.push(&hex(PI, 0x2003, 0x4c6f, 0x7665)); // seg 3 = "Love"
        d.push(&hex(PI, 0x2004, 0x0d20, 0x2020)); // seg 4 terminates
        assert_eq!(d.state().rt, "", "segments 0 and 1 never arrived");
    }

    /// Real capture, PI a80b: the first two segments of "Whole Lotta Love by Led",
    /// with the CR inside segment 1. The terminator must be read from the RAW byte
    /// — folding it to a space first would erase it and nothing would ever publish.
    #[test]
    fn a_carriage_return_terminates_the_message_where_it_lands() {
        const PI: u16 = 0xa80b;
        let mut d = prime(&hex(PI, 0x0000, 0x0000, 0x2020));
        d.push("a80b20d057686f6c"); // seg 0 = "Whol"
        d.push("a80b20d165200d20"); // seg 1 = "e " then CR
        assert_eq!(d.state().rt, "Whole");
    }

    /// Lay down a RadioText the offsets can point into, and let it publish.
    fn rt_plus_fixture() -> RdsDecoder {
        const PI: u16 = 0x19e2;
        const RT: &[u8] = b"Led Zeppelin - Kashmir\r";
        let mut d = prime(&hex(PI, 0x0000, 0x0000, 0x2020));
        for _ in 0..2 {
            for seg in 0..6u16 {
                let ch = |k: usize| *RT.get(seg as usize * 4 + k).unwrap_or(&b' ') as u16;
                d.push(&hex(
                    PI,
                    0x2000 | seg,
                    (ch(0) << 8) | ch(1),
                    (ch(2) << 8) | ch(3),
                ));
            }
        }
        d
    }

    #[test]
    fn rt_plus_labels_artist_and_title() {
        const PI: u16 = 0x19e2;
        let mut d = rt_plus_fixture();
        assert_eq!(d.state().rt, "Led Zeppelin - Kashmir");
        assert_eq!(
            d.state().rt_artist,
            "",
            "not claimed before it is announced"
        );

        // WIBA's ACTUAL announcement. 0x34d8 → group 3A, and the low five bits
        // (0x18 = 24) say group 12, version A. Group 12 has no use fixed by the
        // standard, so it is a legal ODA carrier — but one announcement is not
        // enough, because the group number rides in block B unprotected.
        d.push("19e234d800004bd7");

        // Group 12A payload: running=1, artist @0 len 12, title @15 len 7.
        let b = 0xc000u16 | (1 << 3);
        // (0 << 7) is the zero start offset — kept so the field layout stays visible.
        #[allow(clippy::identity_op)]
        let c = (0b100u16 << 13) | (0 << 7) | (11 << 1);
        let dd = (1u16 << 11) | (15 << 5) | 6;
        d.push(&hex(PI, b, c, dd));
        d.push(&hex(PI, b, c, dd));
        assert_eq!(
            d.state().rt_artist,
            "",
            "a single announcement does not assign the group"
        );

        d.push("19e234d800004bd7"); // announced again — now the group is assigned

        d.push(&hex(PI, b, c, dd));
        assert_eq!(
            d.state().rt_artist,
            "",
            "nor does a single payload group publish"
        );
        d.push(&hex(PI, b, c, dd));
        assert_eq!(
            d.state().rt_artist,
            "Led Zeppelin",
            "the repeat publishes it"
        );
        assert_eq!(d.state().rt_title, "Kashmir");

        // running=0 means the item ENDED — gated the same way, so a lone corrupt
        // group cannot blank a running item.
        d.push(&hex(PI, 0xc000, c, dd));
        assert_eq!(
            d.state().rt_artist,
            "Led Zeppelin",
            "one group does not end it"
        );
        d.push(&hex(PI, 0xc000, c, dd));
        assert_eq!(d.state().rt_artist, "", "an ended item clears the artist");
        assert_eq!(d.state().rt_title, "", "and the title");

        // A marker running off the end is REJECTED, not clamped — a truncated
        // artist is still a wrong artist.
        d.push(&hex(PI, b, c, dd));
        d.push(&hex(PI, b, c, dd));
        assert_eq!(d.state().rt_artist, "Led Zeppelin");
        let far_c = (0b100u16 << 13) | (60 << 7) | (11 << 1);
        d.push(&hex(PI, b, far_c, dd));
        d.push(&hex(PI, b, far_c, dd));
        assert_eq!(
            d.state().rt_artist,
            "Led Zeppelin",
            "out of range is refused"
        );
    }

    /// REGRESSION — an ODA announcement naming a group the standard already
    /// defines is a corrupted announcement, not a station being unusual.
    ///
    /// Measured: a 3A carrying an intact AID with a corrupted block B pointed RT+
    /// at group 0A. Ordinary PS groups then satisfied the payload branch — M/S set
    /// is the "running" bit, and a normal AF pair in block C decodes as an ARTIST
    /// marker — so the artist was set from a slice of the RadioText.
    #[test]
    fn an_oda_announced_on_a_defined_group_is_refused() {
        const PI: u16 = 0x19e2;
        let mut d = rt_plus_fixture();
        assert_eq!(d.state().rt, "Led Zeppelin - Kashmir");

        // 3A, correct AID, low five bits 0 → "RT+ lives in group 0A". Twice, so it
        // is the LEGALITY check being tested and not the repeat check.
        d.push(&hex(PI, 0x3000, 0x0000, 0x4bd7));
        d.push(&hex(PI, 0x3000, 0x0000, 0x4bd7));

        // Ordinary 0A groups: M/S set, a real AF pair (100.3 + 89.6 MHz) in block C.
        let ps = hex(PI, 0x0008, 0x8016, 0x2020);
        d.push(&ps);
        d.push(&ps);
        assert_eq!(d.state().rt_artist, "", "a PS group is not an RT+ payload");
        assert_eq!(d.state().rt_title, "");
    }

    #[test]
    fn a_non_rt_plus_oda_declaration_is_ignored() {
        const PI: u16 = 0x19e2;
        let mut d = rt_plus_fixture();
        d.push(&hex(PI, 0x34d8, 0x0000, 0x1234)); // some other AID in the same slot
        let b = 0xc000u16 | (1 << 3);
        // (0 << 7) is the zero start offset — kept so the field layout stays visible.
        #[allow(clippy::identity_op)]
        let c = (0b100u16 << 13) | (0 << 7) | (11 << 1);
        let dd = (1u16 << 11) | (15 << 5) | 6;
        d.push(&hex(PI, b, c, dd));
        assert_eq!(d.state().rt_artist, "");
    }

    /// An A/B flip is the broadcaster saying the message changed, so the new one
    /// publishes at once instead of waiting for a repeat — and the RT+ markers,
    /// which point into the OLD string, go with it.
    #[test]
    fn an_ab_flip_wipes_the_message_and_its_markers() {
        const PI: u16 = 0x19e2;
        let mut d = rt_plus_fixture();
        assert_eq!(d.state().rt, "Led Zeppelin - Kashmir");
        // The same six segments with bit 4 set: a new message, published instantly.
        const RT: &[u8] = b"Whole Lotta Love\r";
        for seg in 0..6u16 {
            let ch = |k: usize| *RT.get(seg as usize * 4 + k).unwrap_or(&b' ') as u16;
            d.push(&hex(
                PI,
                0x2010 | seg,
                (ch(0) << 8) | ch(1),
                (ch(2) << 8) | ch(3),
            ));
        }
        assert_eq!(d.state().rt, "Whole Lotta Love", "one cycle is enough");
        assert_eq!(d.state().rt_artist, "");
        assert_eq!(d.state().rt_title, "");
    }

    /// reset() empties the station's text but deliberately KEEPS the trusted PI —
    /// clearing it would drop the decoder into the no-incumbent path where any
    /// 3-group run of corruption is adopted outright.
    #[test]
    fn reset_keeps_the_trusted_pi_and_re_asserts_it() {
        let mut d = prime(PS_WERN[0]);
        feed(&mut d, &PS_WERN, 2);
        assert_eq!(d.state().ps, "WERN");

        d.reset();
        assert_eq!(d.state().ps, "", "text is gone");
        assert_eq!(d.state().pi, None, "and the published PI with it");

        d.push(PS_WERN[0]);
        assert_eq!(
            d.state().pi,
            Some(0xa6ff),
            "one matching group re-asserts it"
        );
    }

    // ── TP and TA: the guards, one at a time ────────────────────────────────

    const PI_A6FF: u16 = 0xa6ff;

    /// Group 0A. TP is bit 10, TA bit 4, PTY bits 9-5, segment in bits 1-0.
    fn g0(tp: u16, ta: u16, seg: u16) -> String {
        hex(
            PI_A6FF,
            (tp << 10) | (0x16 << 5) | (ta << 4) | seg,
            0x0000,
            0x2020,
        )
    }
    /// Group 2A. Bit 4 here is the RadioText A/B flag, NOT TA.
    fn g2(tp: u16, bit4: u16) -> String {
        hex(
            PI_A6FF,
            (2 << 12) | (tp << 10) | (0x16 << 5) | (bit4 << 4),
            0x2020,
            0x2020,
        )
    }

    #[test]
    fn tp_needs_its_own_consensus_not_the_pty_fields() {
        let mut d = RdsDecoder::new();
        for i in 0..8 {
            d.push(&g0(0, 0, i % 4));
        }
        assert!(!d.state().tp, "TP starts clear");
        // Bit 10 set, PTY bits untouched: the PTY gate is satisfied throughout, so
        // only TP's own tally can hold this back.
        d.push(&g0(1, 0, 0));
        assert!(!d.state().tp, "one group must not move TP");
        d.push(&g0(1, 0, 1));
        assert!(!d.state().tp, "nor two");
        d.push(&g0(1, 0, 2));
        assert!(d.state().tp, "three consecutive do");
        d.push(&g0(0, 0, 3));
        assert!(d.state().tp, "and one contrary group does not clear it");
    }

    #[test]
    fn ta_is_only_read_from_group_zero() {
        let mut d = RdsDecoder::new();
        for i in 0..6 {
            d.push(&g0(1, 0, i % 4));
        }
        assert!(d.state().tp, "TP confirmed");
        assert!(!d.state().ta, "TA starts clear");
        for _ in 0..10 {
            d.push(&g2(1, 1)); // bit 4 set, but in a RadioText group
        }
        assert!(
            !d.state().ta,
            "bit 4 in a 2A is the text A/B flag, never TA"
        );
    }

    #[test]
    fn ta_needs_its_own_consensus() {
        let mut d = RdsDecoder::new();
        for i in 0..6 {
            d.push(&g0(1, 0, i % 4));
        }
        d.push(&g0(1, 1, 0));
        assert!(!d.state().ta, "one group-0 with TA does not raise it");
        d.push(&g0(1, 1, 1));
        assert!(!d.state().ta, "nor two");
        d.push(&g0(1, 1, 2));
        assert!(d.state().ta, "three consecutive do");
        d.push(&g0(1, 0, 3));
        assert!(d.state().ta, "one contrary group does not clear it");
        d.push(&g0(1, 0, 0));
        d.push(&g0(1, 0, 1));
        assert!(!d.state().ta, "three consecutive clear it");
    }

    #[test]
    fn ta_is_suppressed_without_a_confirmed_tp() {
        let mut d = RdsDecoder::new();
        for i in 0..6 {
            d.push(&g0(0, 0, i % 4)); // TP off, confirmed
        }
        assert!(!d.state().tp);
        for i in 0..6 {
            d.push(&g0(0, 1, i % 4)); // TA claimed anyway
        }
        assert!(
            !d.state().ta,
            "a station carrying no traffic cannot be announcing"
        );
    }

    #[test]
    fn losing_tp_drops_a_latched_ta() {
        let mut d = RdsDecoder::new();
        for i in 0..6 {
            d.push(&g0(1, 1, i % 4));
        }
        assert!(d.state().ta, "TA latches on a traffic station");
        for i in 0..4 {
            d.push(&g0(0, 1, i % 4));
        }
        assert!(!d.state().ta, "and drops when TP goes away");
    }

    #[test]
    fn reset_clears_ta_so_a_retune_cannot_inherit_an_announcement() {
        let mut d = RdsDecoder::new();
        for i in 0..6 {
            d.push(&g0(1, 1, i % 4));
        }
        assert!(d.state().ta);
        d.reset();
        assert!(!d.state().ta);
    }

    // ── reset_for_retune: the PI blackout ───────────────────────────────────

    #[test]
    fn reset_for_retune_drops_the_pi_so_the_new_station_acquires_fast() {
        const A: &str = "a6ff02c0e0cd2020"; // PI a6ff
        const B: &str = "57ff02c0e0cd2020"; // a different station
        let acquire = |d: &mut RdsDecoder| {
            let mut n = 0;
            while d.state().pi != Some(0x57ff) && n < 400 {
                d.push(B);
                n += 1;
            }
            n
        };

        let mut fresh = RdsDecoder::new();
        let from_empty = acquire(&mut fresh);
        assert_eq!(
            from_empty, 3,
            "a fresh decoder acquires in PI_CONFIRM groups"
        );

        let mut kept = RdsDecoder::new();
        for _ in 0..8 {
            kept.push(A);
        }
        kept.reset();
        assert_eq!(
            acquire(&mut kept),
            12,
            "reset() keeps the incumbent: displacement"
        );

        let mut cleared = RdsDecoder::new();
        for _ in 0..8 {
            cleared.push(A);
        }
        cleared.reset_for_retune();
        assert_eq!(
            acquire(&mut cleared),
            from_empty,
            "reset_for_retune: fast path"
        );

        let mut wiped = RdsDecoder::new();
        for _ in 0..8 {
            wiped.push(A);
        }
        wiped.reset_for_retune();
        assert_eq!(wiped.state().rt, "", "and it still empties the text");
        assert_eq!(wiped.state().pi, None, "and the PI itself");
    }

    // ── clear_ta: the RDS-expiry path ───────────────────────────────────────

    #[test]
    fn clear_ta_drops_the_announcement_but_keeps_the_station() {
        let mut d = RdsDecoder::new();
        for i in 0..6 {
            d.push(&g0(1, 1, i % 4));
        }
        assert!(d.state().ta, "set before the expiry");
        d.clear_ta();
        assert!(!d.state().ta, "cleared, so a caller's restore reads false");
        assert!(d.state().tp, "TP is sticky station state and stays");

        // Still running: it must re-confirm rather than stay stuck off.
        let mut back = None;
        for i in 0..5 {
            d.push(&g0(1, 1, i % 4));
            if d.state().ta && back.is_none() {
                back = Some(i + 1);
            }
        }
        assert_eq!(back, Some(3), "re-confirms after PI_CONFIRM group-0s");

        // Ended during the gap: TA=0 groups keep it off.
        let mut d2 = RdsDecoder::new();
        for i in 0..6 {
            d2.push(&g0(1, 1, i % 4));
        }
        d2.clear_ta();
        for i in 0..4 {
            d2.push(&g0(1, 0, i % 4));
        }
        assert!(!d2.state().ta, "an ended announcement stays off");
    }

    // ── The joins Slint is not allowed to make ──────────────────────────────

    #[test]
    fn the_rds_tell_follows_pi_or_a_name() {
        let blank = RdsState::default();
        assert!(!rds_ok(&blank));
        let mut d = prime(PS_WERN[0]);
        assert!(rds_ok(&d.state()), "a confirmed PI is enough on its own");
        feed(&mut d, &PS_WERN, 2);
        assert!(rds_ok(&d.state()));
        // A scroller has its PS retracted and still speaks RDS.
        let scrolled = RdsState {
            pi: Some(0x19e2),
            ps_scrolling: true,
            ..RdsState::default()
        };
        assert!(rds_ok(&scrolled));
    }

    #[test]
    fn pty_labels_are_the_rbds_table() {
        // The two the drive logs produce.
        assert_eq!(pty_label(Some(22)), "Public");
        assert_eq!(pty_label(Some(6)), "Classic Rock");
        // 0 is "no programme type"; 27 and 28 are unassigned in RBDS; and the
        // getter this unit exposes returns 0, which is exactly why an empty label
        // has to mean "print nothing" rather than "print the first entry".
        assert_eq!(pty_label(Some(0)), "");
        assert_eq!(pty_label(Some(27)), "");
        assert_eq!(pty_label(Some(28)), "");
        assert_eq!(pty_label(None), "");
        assert_eq!(pty_label(Some(31)), "Emergency");
        // Region 1 would read "Education" here. This is the North American table
        // and nothing selects between them.
        assert_eq!(pty_label(Some(5)), "Rock");
    }

    // ── Trimming the station out of its own RadioText ───────────────────────

    #[test]
    fn the_station_trims_itself_off_both_ends() {
        // Verbatim from the drive of 2026-08-04. The DIAL is what identifies these
        // — the station calls itself "IBA-FM", which contains no callsign token,
        // and that is the reason both matchers exist.
        let strip = |s: &str| strip_station_from_rt(s, Some(101.5), Some("WIBA-FM"));
        assert_eq!(
            strip("101.5 IBA-FM - Walk This Way - Aerosmith"),
            "Walk This Way - Aerosmith"
        );
        assert_eq!(
            strip("101.5 IBA-FM - Led Zeppelin - Kashmir"),
            "Led Zeppelin - Kashmir",
            "artist-then-title survives untouched — the order is not ours to judge"
        );
        // A trailing identifier goes too.
        assert_eq!(strip("Walk This Way - 101.5 IBA-FM"), "Walk This Way");
    }

    #[test]
    fn an_unspaced_hyphen_is_part_of_the_name() {
        // "WERN-FM" must survive as ONE segment: split on every hyphen instead and
        // the callsign match fires on "WERN" alone, leaving a stray "FM" behind.
        assert_eq!(
            strip_station_from_rt("WERN-FM - Wisconsin Public Radio", None, Some("WERN")),
            "Wisconsin Public Radio"
        );
        // Nothing to split on at all.
        assert_eq!(
            strip_station_from_rt("Wisconsin Public Radio", Some(88.7), Some("WERN")),
            "Wisconsin Public Radio"
        );
        // Whitespace on both sides is required, and it is consumed by the split.
        assert_eq!(
            strip_station_from_rt("WERN   -   Kashmir", None, Some("WERN")),
            "Kashmir"
        );
    }

    #[test]
    fn a_message_that_is_only_the_station_is_still_shown() {
        // Blanking the marquee would be worse than repeating the hero.
        assert_eq!(
            strip_station_from_rt("101.5 - 101.5 IBA-FM", Some(101.5), Some("WIBA-FM")),
            "101.5 - 101.5 IBA-FM"
        );
        assert_eq!(
            strip_station_from_rt("101.5 IBA-FM", Some(101.5), Some("WIBA-FM")),
            "101.5 IBA-FM"
        );
        // Without a dial or a callsign there is nothing to identify.
        assert_eq!(
            strip_station_from_rt("101.5 IBA-FM - Kashmir", None, None),
            "101.5 IBA-FM - Kashmir"
        );
        assert_eq!(strip_station_from_rt("   ", Some(101.5), Some("WIBA")), "");
    }

    #[test]
    fn only_the_ends_are_trimmed() {
        // A station naming itself in the MIDDLE is left alone: the segment is part
        // of the message, and adverts and slogans pass through untouched.
        assert_eq!(
            strip_station_from_rt(
                "Kashmir - 101.5 IBA-FM - Led Zeppelin",
                Some(101.5),
                Some("WIBA-FM")
            ),
            "Kashmir - 101.5 IBA-FM - Led Zeppelin"
        );
    }
}
