# Mini-handoff — Status-bar clock + OsmAnd turn-by-turn

Scope: **only** the two additions that make bundle **v3.0.0** a major version — the
**status-bar clock** (§4.8) and the **OsmAnd maneuver layer** (§4.9). Everything else in
the face is unchanged from v2.1.0; if your build is behind that, read `README.md` and
`CHANGELOG.md` first — this file assumes the v2.1.0 face is already standing.

`IMPLEMENTATION-SPEC.md` **§4.8 and §4.9 are authoritative**. This file is the porting
order, the Android/Slint-specific parts, and the traps — it does not restate every number.

Reference source: `RadioFace.dc.html` (both features render here), `CarFmLive.dc.html`
(state + the fake AIDL feed), `SettingsPanel.dc.html` (the two settings rows).

---

## 0. Why this is a major bump

The clock is additive. The OsmAnd layer is not: it introduces **a second app's live data
as a driving-safety surface inside the radio face**, adds an AIDL client, a foreground
poll, a new permission-shaped dependency on an app that may not be installed, and a
third-party trademark. It also changes what the hero card and the RadioText strip mean —
for a few seconds at a time they stop being radio.

That is an integration boundary, not a feature. Treat the version as a compatibility
statement: **a build that ships the face without §4.9 is not v3.0.0.**

---

## 1. Build order

Do these in order; each is independently shippable and testable.

1. **Clock** (§4.8) — no external dependency, exercises the out-of-flow measuring that the
   ETA later reuses. Ship it, look at it in the car at night, then continue.
2. **Arrow generator** (§4.9, *Arrow system*) — pure function, no feed. Unit-test it
   against the TurnType table before anything can display a wrong arrow.
3. **AIDL client + tell** — bind, poll, surface the status-bar tell only. No layout changes
   yet; the tell going full colour is your proof the link is alive.
4. **Stage 1** — hairline + ETA + cruise countdown.
5. **Stages 2 and 3** — strip yield, hero takeover, pulsing edge, corner logo.
6. **Settings ▸ NAVIGATION** + suppression.

---

## 2. The clock (§4.8)

**Font.** `fonts/DSEG7ClassicMini-Regular.ttf` (Keshikan, SIL OFL 1.1) ships in this
bundle. Register it as an app asset; **Regular only** — a synthesized bold smears the
segment gaps. This is the one non-Atkinson face in Carnyx, deliberately: it should read as
hardware.

**Placement is measured, not constant.** The readout is out of flow, right edge flush with
the gear, and the **clock line's optical centre** sits midway between the right cluster's
bottom edge and the hero card's top line. In Slint that means a `Rectangle` in the face's
top-level `Rectangle` (not in the status-bar layout) with `y` computed from
`gear.absolute-position` / `hero.absolute-position` — the same pattern the mock's
`measureClock()` uses. Recompute on layout change and on track change.

**It is the LINE that is centred, not the block.** Get this wrong and the ETA (§3.2) will
push the clock up when navigation starts, which is the one thing the whole design avoids.

**12/24 is not ours.** `DateFormat.is24HourFormat(context)`, re-read on every tick. No
app-side preference, no sync problem. The settings sub-line *reports* the format; it does
not offer it.

**Leading blank, not leading zero** in 12-hour: pad the hour with U+0020, which DSEG7 sets
at digit width, so the columns hold still at 1 o'clock. 24-hour zero-pads. Minutes always
pad. Meridiem is a single `A`/`P`, baseline-aligned, absent in 24-hour.

**Tick.** One 1 s timer; only publish a property change when the minute rolls. Do not
re-render the face every second.

**Setting.** Settings ▸ APPEARANCE ▸ **Clock**, default on, persisted.

---

## 3. OsmAnd (§4.9)

### 3.1 The feed

Two channels, and the split matters — see the table in §4.9.

- **Push** (`registerForNavigationUpdates` → `ADirectionInfo`): `distanceTo`, `turnType`,
  `isLeftSide`. That is *all* the push gives you.
- **Poll** (`getAppInfo` → `AppInfoParams`, ~1 Hz while navigating): `arrivalTime`,
  `leftTime`, `leftDistance`, `mapVisible`, and the `turnInfo` bundle — street names
  (`turn_name`, already formatted with ref + destination), `turn_imminent`, `after_next`,
  `turn_type`, `turn_lanes`, `turn_angle`.

So: **ETA, street name, distance-left and the turn-after-next only exist if you poll.**
Poll on a timer while navigating, stop when not. Treat **every field as optional** — a
missing one collapses its element; it never leaves a gap or a placeholder.

`turn_lanes` is present but **lane guidance is not designed** — do not improvise it.

**OsmAnd may not be installed, may not be running, may be idle, may refuse to bind.** Each
of those is a normal state, not an error: the tell goes inert, the layer shows nothing, the
radio is unchanged, and the settings sub-line says which one it is.

### 3.2 Stage 1 — cruising

- **Hairline:** 3dp, amber, flush to the face's top inner edge, width = fraction of the way
  to the next turn, `transition: width .6s linear`.
- **ETA:** directly under the clock, right-aligned to it, 7dp clear, in the **UI face
  (Atkinson), not DSEG7** — `ETA` label 11sp/700, time 15sp/700 (17sp tall), suffix
  11sp/700, all `dim`. Distance-left joins it on the tall track only.
- **Cruise countdown:** one centred line between the genre line and the hero card's top —
  `IN` (13sp/700, letterspacing 1.5, `dim`, uppercase) + distance (24sp/700 amber, tabular;
  26sp tall) + arrow (26dp; 28dp tall) + street (18sp/400 `dim`; 19sp tall). Out of flow,
  anchored to the **genre line's baseline**, clamped above the hero card's top line. Ends
  at stage 2 — two copies of one turn on one screen is noise.

The mock hides the countdown (`opacity: 0`) until it has measured its `top`, so it never
flashes at the wrong y. Do the equivalent.

### 3.3 Stage 2 — the strip yields

Arrow (46dp; 52dp tall) + distance (34sp/700 amber, tabular; 38sp) + street (25sp/700
`text`; 27sp, ellipsized) replace RadioText **entirely** — it does not alternate back. On
the right, past a 1px `border` rule with 16dp of padding (12dp tall): `THEN` (14sp/700,
letterspacing 1.4, `dim`) + a 26dp `dim` arrow + the after-next street (17sp/400 `dim`, max
240dp; 150dp tall).

The strip keeps its **exact** geometry — same 64dp height, same zone.

### 3.4 Stage 3 — the hero card takes over

Centred column, gap 2: arrow (88dp; 104dp tall) over distance (52sp/700 amber, tabular,
line-height 1.05; 60sp tall) over street (25sp/700 `text`, centred, max 92%; 28sp tall).

**Only the hero card changes.** Peek cards and the preset shelf do not move. The strip
stops repeating the imminent turn and shows only the turn after it (its `then` block
promotes to 20sp/700 and loses the divider rule).

**Station identity survives:** the logo moves to the card's **upper-right**, 101dp (115dp
tall), inset 12dp (14dp tall), **transparent — no plate, no border**. No logo, or logos
hidden by a band theme, or mid-scan → the call sign in `dim` at 17sp (19sp tall) in the
same corner.

### 3.5 The takeover edge

Whichever element carries the maneuver wears a pulsing amber ring: **6dp on the hero
(radius 29), 5dp on the strip (radius 17)**, drawn as an **inset ring** (`inset: -1`,
radius +1) rather than by thickening the real border — the card's own geometry and content
must not shift by a pixel. 2.6 s ease-in-out, amber → amber at 32%, **the same beat as the
vehicle-in-motion tell**, and it animates *colour only*; nothing scales.

Slint has no CSS-variable trick here — animate the ring rectangle's `border-color` with an
`animate` block on a 2.6 s cycle, and keep the ring a sibling overlay of the card so the
card's own layout never sees it.

### 3.6 Stage triggers are OsmAnd's

Escalate on `next_turn_imminent` (and the voice router firing), so the radio changes at the
moment OsmAnd *speaks*. **Never on distance thresholds of our own** — at highway speed they
disagree with the voice, and a screen that contradicts the voice is worse than no screen.

### 3.7 Arrows

`turn_type` is a TurnType XML string → **one number, degrees off straight ahead**:
`C` 0, `KL/KR` ±20, `TSLL/TSLR` ±45, `TL/TR` ±90, `TSHL/TSHR` ±135, `TU/TRU` ±179. One
generator draws stem + elbow + barbs from that angle, so all twelve are one stroke
language. U-turns use an arc instead of an elbow; `RNDB`/`RNLB` get a roundabout glyph with
the exit number inside the ring. Same 24-unit box and round caps as every other icon
(§7.1); stroke 2.1 at the large sizes, 2.4 at 26–28dp.

Honour `isLeftSide` for left-hand-traffic roundabouts.

### 3.8 Colour, theming, safety

The maneuver is **amber** — the fixed safety family (TA, motion tell). Never the blue
accent. A band theme may **restyle the arrow art only**; colour, sizes and stage logic are
not themeable. *A theme that recolours a turn arrow is a safety bug, not an Easter egg* —
put that in the theme code as a comment, not just in this doc.

### 3.9 Suppression + settings

`AppInfoParams.mapVisible` true → hide the whole layer; the driver is already looking at
the turn. Driver-overridable.

**Settings ▸ NAVIGATION**: **OsmAnd integration** (default on; sub-line reports the link
state and *why* nothing is showing — OsmAnd idle, off here, or hidden behind the map) and
**Hide when the map is showing** (default on). Integration off = no mirroring **and** no
tell.

### 3.10 The tell

Right cluster, **between the vehicle-in-motion glyph and the GPS satellite** (the car glyph
moves one slot left). Bound → **#FF8800**; enabled but unbound → flat text colour at ~30%,
the same "present but inert" language the GPS tell uses with no fix. Wide/landscape only,
like the other driving tells (§4.6). Nudge down 2dp so the **disc's** centre lines up with
the gear's — the mark's box is not centred on its disc, the pin tail hangs below — and use
small negative margins to close the optical gap to its neighbours.

**Trademark.** The art is OsmAnd's own GPL monochrome launcher drawable
(`OsmAnd/res/drawable/ic_launcher_osmand_monochrome.xml`), viewBox cropped to the mark
(the launcher pads it for the adaptive-icon safe zone; uncropped it renders half-size next
to the other tells). Their geometry, their orange, never a redrawn lookalike. Naming the
integration this way is nominative use, but **confirm with OsmAnd before shipping** — if
they decline, replace it with a neutral route glyph, not a variation on their mark.

### 3.11 Units

Feet under 1000 ft then miles to one decimal; metres under 1 km then km — **by locale**,
because OsmAnd does not expose its own unit preference over the API. If a driver ever sees
the wrong one, that needs an app-side override, not a guess.

---

## 4. Traps

- **Nothing reflows.** Starting or ending navigation must not move one pixel of the resting
  face. Verified on the wide track: hero 605×248 at the same top edge, strip 64dp, clock
  30sp centred where it always is, either way. Screenshot-diff both states and check.
- **The ETA is not a second clock.** UI face, not DSEG7 — two segmented readouts stacked
  read as one four-line instrument instead of two values.
- **Distance, never seconds.** OsmAnd gives `next_turn_distance` and no time-to-turn; a
  derived clock that disagrees with the voice prompt is worse than none.
- **The countdown is cruise-only.** It must end when the strip yields.
- **Poll and push are not interchangeable.** A street name from the push callback does not
  exist; if the poll has not landed yet, the street collapses.
- **Do not thicken the real border** for the takeover edge, and do not scale anything on the
  pulse.

---

## 5. Review states

`CarFmLive.dc.html` stands in for the AIDL feed with four tweaks:
`navState` (*Not navigating · Cruise · Approach · Turn now*), `osmandLinked`,
`osmandMapVisible`, plus `clockFormat` for the clock. Demo maneuver is `TR` onto Whitney
Way, then `TSLL` onto Odana Rd.

Open it, set `navState` to each stage on both tracks and both themes, and diff against the
built app per `CORRECTION-LOOP.md`. There are **no static screenshots for the nav stages** —
render them live.
