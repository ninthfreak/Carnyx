# Changelog — Carnyx design handoff

## v3.0.0 — 2026-08-26

**Major.** The OsmAnd layer puts a second app's live data on the radio face as a
driving-safety surface: an AIDL client, a foreground poll, a dependency on an app that may
not be installed, a third-party mark — and, for a few seconds at a time, a hero card and a
RadioText strip that stop being radio. That is an integration boundary, not a feature. A
build that ships this face without §4.9 is not v3.0.0. Porting order and the Android/Slint
specifics: **`NAVIGATION-HANDOFF.md`**.

Rolls up the clock work (previously drafted as 2.2.0) — neither shipped as a bundle.

### Added — OsmAnd turn-by-turn
- **OsmAnd turn-by-turn mirroring** (spec §4.9). Three stages driven by OsmAnd's own announcement
  state, not our thresholds: amber hairline + ETA while cruising, the RadioText strip yielding to
  arrow + distance + street (with `then` + after-next) on the prepare announcement, the hero card
  taking over on "turn now" — peek cards and preset shelf untouched, and nothing reflows at any
  stage. Maneuver arrows are generated from the TurnType XML string via a degrees-off-straight
  table, so the whole set is one stroke language; amber is fixed (safety family) and band themes
  may restyle the arrow art only. Hidden while `AppInfoParams.mapVisible` is true, overridable in
  the new **NAVIGATION** settings section.
- **Cruise countdown** (spec §4.9). A centered line between the genre and the hero card — `in` +
  the distance to the next turn counting down in amber + arrow + street. Out of flow, anchored to
  the genre line's baseline (not the status-bar box, which runs deeper on the left), so it adds no
  height and moves nothing. Ends when the strip yields at stage 2. Distance, not seconds: OsmAnd
  supplies `next_turn_distance` but no time-to-turn, and a derived clock could disagree with the
  voice prompt.
- **Takeover edge + station plate.** The element carrying the maneuver (strip at stage 2, hero card
  at stage 3) wears a thick amber border (6dp hero / 5dp strip) pulsing on the motion tell's exact
  beat (2.6s ease-in-out), drawn as an inset ring so no geometry shifts. During the hero takeover the
  station logo moves to the card's upper-right corner at 101dp, transparent — no plate behind it.
- **OsmAnd tell in the status bar.** With the integration on, OsmAnd's own mark sits between the
  vehicle-in-motion glyph and the GPS satellite (car moves one slot left): full colour #FF8800 when
  the AIDL link is bound, flat text colour at 30% when it is not. Art is OsmAnd's own GPL
  monochrome launcher drawable, viewBox cropped to the mark — confirm use with OsmAnd before
  shipping (spec §4.9).
- **ETA sits directly under the clock** in the **UI face** (label 11sp + time 15sp, `dim`) rather
  than DSEG7 — two stacked segmented readouts read as one instrument. **It costs nothing:** the
  clock keeps its 30sp size and its exact position (the clock *line* is what the placement centres,
  not the block), and no other element moves — the ETA lands in the empty space right of the hero
  card, 14dp above the peek card. Verified: hero 605×248 at the same top edge, strip 64dp, either
  way (spec §4.8/§4.9).
- **Grounded the integration in the OsmAnd source** rather than assumption: the push callback gives
  only `distanceTo`/`turnType`/`isLeftSide`, while `getAppInfo` supplies `arrivalTime`, `leftTime`,
  `leftDistance`, `mapVisible` and a `turnInfo` bundle carrying street names and an `after_next`
  turn. ETA, distance left and the turn-after-next are therefore all real; lane guidance
  (`turn_lanes`) exists but is not designed yet; OsmAnd's unit preference is not exposed.

### Added — status-bar clock
- **Clock in the status bar** (spec §4.8, §4.1 right cluster). Set in **DSEG7 Classic Mini Regular**
  (Keshikan, SIL OFL 1.1 — `fonts/DSEG7ClassicMini-Regular.ttf`), the one non-Atkinson face in the
  app, at 30sp (36sp tall) in `dim`, matching the genre line. Meridiem is a single `A`/`P` in the
  same face at 17sp (20sp tall); 12-hour hours pad with a blank segment position rather than a zero.
  **Out of flow**, right edge flush with the gear, its optical center midway between the right
  cluster's bottom edge and the hero card's top line. **On/off in Settings ▸ APPEARANCE ▸
  Clock**, default on, persisted. **12/24-hour is not an app setting:** the readout asks
  `DateFormat.is24HourFormat` (`Settings.System.TIME_12_24`) and re-formats each tick, so the
  Android system toggle drives it live; the settings sub-line reports the format rather than
  duplicating the choice. Ticks once a second but re-renders only on the minute.

## v2.1.0 — 2026-08-25

Re-sync with the shipped Slint app, plus two fixes the owner asked for. Every item
below moved in the mock-up **and** in the docs.

### Folded in from the app (the app was ahead; the mock-up followed)
- **Nearby magnifier, dark theme.** The disc is `panel` (`#33373D`), not `raised` — with
  disc and lens both `#43474E` the glass was invisible. Lens stays `raised`, rim keeps its
  own `#565B63`. Spec §7 prose corrected too; it had said "raised disc" since v1.10.
- **Genre line.** Width cap **200 → 240dp** (clears `Foreign Language`, 227dp at 26sp) and
  the line now **shrinks before it elides**, floored at 15sp wide / 19sp tall. Spec §4.1.
- **The basic theme tier** (4 rows): Eric Clapton · The Pretty Reckless · The Who · Carry On
  Wayward Son. One genre line each, no palette, no marks, logos stay visible; advanced rows
  match first. Wayward Son brings the tier's one face, **Supernatural Knight**, on the genre
  line and RadioText only. New registry fields `tier` and `genreFace`; the font ships in
  `fonts/`. Spec §12 + `EASTER-EGGS-BUILD.md` §6.

### Fixed
- **✕ and ✓ are drawn, not typed** — the "ugly X". Atkinson Hyperlegible carries no
  ✕ ✓ ★ ⚠ ⌫, so a text glyph silently falls back to a face nobody chose. The three overlay
  close buttons and the reorder DONE check are now stroked paths in the house icon language.
  Then the rest of the set followed: **⚠** in the status-bar out-of-band pill, on the hero's
  out-of-band caption and on the keypad's band error, and **★** at the end of the empty-band
  line — which became text + icon rather than one string, since the sentence names the mark.
  The keypad's **⌫** went the same way — the Slint side already drew it, the mock-up was still
  typing it, and the key is now keyed by an id so the character never travels in a string at
  all. Geometry for all five: spec **§7.1** (new). No typed symbol is left in the mock-up.
- **Peek-card tuck and fade (task #50).** The overlap was a fixed 46/72dp against a card
  that is 20% of the row, so it hid 35% of the plate on the head unit and **64% on a 360dp
  phone** — a 26dp shard of artwork. It is now **35% of the card** (7% of the row, capped at
  72), which reproduces the wide track exactly and takes that phone's visible plate to 43dp.
  The inner edge still fades under the hero; the **outer** edge now fades too on the tall
  track, where the wider peeks reach the surface edge — which is what §5's "outer edge
  softened" was always asking for. Spec §5.

### Settled with the owner
- **The remaining typed glyphs are drawn too** (⚠ ×3, ★) — see above.
- **Basic themes stay out of the secret panel.** That list is the advanced set; a one-line
  theme turns up when the right track plays rather than being picked off a menu. Basic rows
  match from live RadioText only — the prototype tweak can still force one for review.
