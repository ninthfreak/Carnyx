# Carnyx FM Radio — implementation spec

*Bundle v2.1.0 — 2026-08-25 (see `VERSION`).*

The build spec for the **Carnyx** FM radio front-end. Carnyx is a fork of CarFM,
written mostly in **Rust** with **Slint** for the UI, targeting **Android now** and a
possible **Linux** build later. This document describes **intent, structure, tokens, and
behavior** so the UI can be built natively — not by translating web CSS. Every measurement
is a **starting value in logical px** (Slint's `px` = density-independent) to confirm on
device.

**Platform note:** the product does **not** run on DuduOS. DuduOS/Dudu7 is a platform the
owner may switch to some day; the `Dudu7` surface names below are **screen geometries the
design must survive**, not the OS. Do not write DuduOS into the app, its copy, or its docs.

The HTML prototype is the reference for exact values when this document is silent:
`RadioFace` (main face), `CarFmLive` (state host + surface framing),
`NearbyPicker`, `SettingsPanel`. Treat those as visual/behavioral truth; treat
this document as the plan for expressing them in Slint idioms. Slint-specific
architecture, property/callback interfaces and platform setup live in `CARNYX-SLINT.md`.

---

## 0. Non-negotiable: build a real responsive layout, NOT scale-to-fit

**Do not lay the face out at one fixed design-canvas size and scale the whole thing
(a root `Rectangle` with a uniform transform, or a fixed-size window scaled to the
display) to fit the screen. That approach is banned.**
It looks faithful only at the exact reference resolutions and fails everywhere
else, and it breaks three things this product must have:

- **Font-scale** — a uniformly-scaled block can't let text grow with the OS
  setting. Type must size in logical `px` and respond to the platform's text-scale
  factor (on Android, feed `fontScale` into a global scale property and multiply the
  type ramp by it; never freeze it).
- **Touch targets** — scaling the canvas down shrinks buttons below the **48px**
  floor. Targets are specified in real logical px and must stay ≥48px at every surface.
- **Reflow** — the design has **two layout tracks** and sub-modes precisely so it
  *rearranges* per surface. Uniform zoom never reflows; it just shrinks one frozen
  picture.

Build it the Slint way: real logical `px`, `HorizontalLayout`/`VerticalLayout`/
`GridLayout` with `spacing`/`padding` and stretch factors that reflow, `ListView`/
`Flickable` for the scrolling regions, and the track chosen from the root's own
`width`/`height` (§2). The reference
screenshots are **per-surface proportional targets** (match the composition and
relative sizing at each surface's own density) — **not a master canvas to zoom to
fit.** If your output is pixel-identical to a reference only because you scaled a
fixed canvas, you built the wrong thing.

---

## 1. What this is

A head-unit / phone FM radio front-end: one primary screen (the **radio face**) plus two
modal overlays (the **tune overlay** — a tabbed nearby-stations picker + frequency keypad —
and **settings**). Architecture splits cleanly into a **Rust core** (tuner + preset state,
station metadata) feeding a **stateless Slint face component** that renders declared
properties and raises callbacks. See `CARNYX-SLINT.md` for the interface.

### Design language
- **Light-first.** Light ("Simple") is the default theme; dark
  ("Enthusiast") is a first-class alternate, user-selectable, and follows the
  system scheme when set to "system".
- **Interactive blue** is the system accent — selection, active preset, primary
  actions. Fallback value given in §3.
- **Frequency amber is a fixed safety color — never themed, never restyled.** The
  tuned frequency is always amber in both themes so the driver
  finds the readout instantly. Amber is not part of the accent/theming system.

---

## 2. Target surfaces & layout tracks

A head unit can split its screen into **vertical thirds**, and the app also
runs on a phone (Galaxy S21) in portrait and landscape. All five surfaces are
first-class — none is a fallback, none may be dropped:

| Surface | Representative content area (dp) | Track |
|---|---|---|
| Dudu7 full | 1024 × 614 | wide |
| Dudu7 ⅔ slice | 900 × 810 (near-square) | wide, **twoRows** presets |
| Dudu7 ⅓ slice | 470 × 845 (narrow tall) | **tall** |
| Galaxy S21 landscape | 800 × 360 | wide, **landscape** density |
| Galaxy S21 portrait | 360 × 800 | **tall** |

Galaxy S21 panel is 1080 × 2400 px @ ~421 dpi ≈ **xxhdpi (density 3.0)** →
~360 × 800 dp portrait. **Design in dp; never hardcode 1080/2400.**

### Selecting the track
Two layout tracks, chosen from the available window size:
- **tall** — narrow, portrait-like surfaces (S21 portrait, Dudu7 ⅓ slice).
- **wide** — landscape and near-square surfaces (Dudu7 full / ⅔ slice, S21
  landscape).

Derive the track from **`WindowSizeClass`** (`Compact` width → tall;
`Medium`/`Expanded` → wide) or `BoxWithConstraints` on the available width.
Orientation is a fine proxy on the phone (portrait → tall, landscape → wide). Choose the
track from the **root component's actual width/height** each layout pass, so the app
composes correctly inside a head-unit third as well as full-screen. In Slint this is an
ordinary derived property — `property <bool> tall: root.width / root.height < 1.0;` — and
every track-dependent value reads from it. Never latch it once at startup.

Two wide sub-modes are density tweaks, not separate layouts: **twoRows** (Dudu7 ⅔
— presets become a 2-row horizontal grid) and **landscape** (S21 landscape —
slightly smaller type/tiles for the shorter height).

---

## 3. Design tokens

Font: **Atkinson Hyperlegible** (400/700) — an accessibility face for
glance-legibility in a moving car. Bundle both weights; honor system font-scale
(size in `sp`).

### Light ("Simple", default)
| Role | Hex |
|---|---|
| Screen bg | `#EEF1F5` |
| Panel (cards) | `#FFFFFF` |
| Raised (tiles/inputs) | `#F5F7FA` |
| Text | `#1B222C` |
| Dim text | `#67717F` |
| **Amber (frequency — fixed)** | `#C9760A` |
| Blue (accent) | `#2E86FF` |
| Blue fill (accent wash) | `rgba(46,134,255,0.12)` |
| Border | `rgba(20,30,45,0.13)` |
| Meter empty | `rgba(20,30,45,0.10)` |

### Dark ("Enthusiast")
| Role | Hex |
|---|---|
| Screen bg | `#24272C` |
| Panel | `#33373D` |
| Raised | `#43474E` |
| Text | `#E9EEF4` |
| Dim text | `#A3A9B2` |
| **Amber (frequency — fixed)** | `#FFB833` |
| Blue (accent) | `#4A9EFF` |
| Blue fill | `rgba(74,158,255,0.18)` |
| Border | `rgba(255,255,255,0.13)` |
| Meter empty | `rgba(255,255,255,0.10)` |

Amber differs by theme only for contrast against the two backgrounds; it is not a
themeable brand color — keep it out of accent/theming logic.

Card radius ≈ 28dp (hero), 16–18dp (tiles/cards/overlay panels), 12–14dp
(pills/inputs/keys); station-logo tiles ≈ 15–20dp (circle = fully round); the
nearby disc is fully round. Spacing: page padding ≈ 18–24dp, hero gap ≈ 20dp,
general gaps 8–12dp. Shadow: soft and large — hero ≈ `0 20dp 44dp`, modal/overlay
≈ `0 20dp 50dp`, ~16% opacity light / ~50% dark.

---

## 4. Radio face — structure

A vertical stack of three regions. Region behavior differs by track.

### 4.1 Status bar (top, fixed height, does not scroll)
Three zones — **left** (signal), **center** (stereo + tells + genre), **right**
(controls) — but the arrangement is **track-specific** (see "Track layout" below).

- **Signal** — a broadcast-arc glyph (center dot + **four** elliptical arc pairs, lit
  0–5 by strength) with the strength **value laid inside the glyph**, in the clear
  column below the dot. Same arrangement on both tracks — the number never sits beside
  or below the icon. On the built-in NWD tuner the value is a **unitless integer** (no
  suffix); on the RTL-SDR path a much smaller **dB** hangs off the digits’ right, out
  of flow so it never shifts them off the glyph’s axis. Full geometry, colour tokens,
  strength bands and the dotted-arc loss overlay: **`SIGNAL-METER.md`** — build from
  that document, not from this bullet.
- **STEREO / MONO pill** — blue outline + blue-fill when stereo, else dim
  outline. **STEREO** is flanked by two speaker-cone glyphs, drawn in slight
  perspective and mirrored so both flare away from the label: a magnet block at the
  narrow end, two cone walls, and an elliptical rim (the rim reads as an ellipse
  because the cone is turned a few degrees toward the viewer). Stroke 1.8, blue.
  **MONO** keeps BOTH cones in place and changes only their appearance: stroke drops
  from 1.8 to **1.1**, the right cone takes the dim label colour and the left takes the
  pill's own border colour so it reads as a ghost. Blue is reserved for a stereo lock.
  Audio-off and the blank (no station) state show neither cone and no label.
  **The cones must not move between states.** "MONO" is narrower than "STEREO" and the
  pill centres its contents, so a naive implementation pulls both cones inward on mono.
  Pin the label to the width of the widest string — the prototype reserves it with an
  invisible "STEREO" and centres the live label over it; a fixed dp width is equivalent.
  Geometry reference: `RadioFace` (`stereoWaveL` / `stereoWaveR`).
- **Tell strip** — four flags: `RDS`, `HD`(+level, e.g. `HD2`), `TP`/`TA`,
  `AF`. On = full weight + subtle raised shadow; off = ~32% opacity. **TA**
  replaces TP while a traffic announcement is active and **pulses** (amber,
  ~1.1s scale pulse). The tell strip sits directly **below** the STEREO pill.
- **PTY** — program-type / genre text (e.g. "Classic Rock"), dim, **≈26sp wide / 33sp tall**;
  sits **below** the tell strip. Width cap **240dp** on the wide track: it clears
  `Foreign Language`, the widest label the RBDS table can emit (227dp measured at 26sp), by
  13dp — the margin a system font-scale has to eat before anything elides again. **Under the
  cap the line gives up POINTS before it gives up WORDS:** measure the string at full size,
  scale down to fit, floored at **15sp wide / 19sp tall** — the size this line was before it
  grew 175%. Eliding is what happens past the floor, not instead of shrinking. Measure with a
  **hidden gauge, not the live line** (a text element's preferred width depends on its own
  font size, so sizing the visible line from its own measurement is circular), and measure
  **the width the layout actually handed over** rather than predicting it from tokens.
- **OUT OF FM BAND** warning pill (amber) when tuned outside 87.5–108.0 (pill text
  carries no "MHz" label). **nowrap, no shrink, and its height is a MINIMUM** — it sits
  in a nowrap row and its drawn triangle (§7.1) is a few dp wider than the character it
  replaced, so a tightened cluster or a text-scale above 1.0 would otherwise break the
  label onto a second line *below the amber border*. Same rule as the tuner-error pill.
  The hero's out-of-band caption is nowrap for the same reason.

**Track layout:**
- **Wide / landscape:** the signal cluster, the STEREO pill (with its tell strip
  and PTY beneath), and any OUT-OF-BAND pill all sit together in a **left cluster**;
  the controls are the right cluster. This is the default inline arrangement.
- **Tall (portrait / ⅓ slice):** the STEREO pill is **horizontally centered** in the
  status bar, with the **tell strip centered directly beneath it** and the **PTY /
  genre centered directly below the tells**. The signal glyph (value inside it) stays
  in the **left** zone; settings + nearby stay in the **right** zone. Build as three flex
  zones (left `weight(1f)` · center wrap-content · right `weight(1f)`) so the center
  column is truly centered regardless of the side widths.
  - **Tuner-error pill** — when no compatible tuner is connected, one pill
    **replaces the entire OK cluster** (signal / stereo / tells / PTY / out-of-band);
    the two never show together. Amber warning triangle (≈26dp, 2dp stroke, no
    fill) + "Failure to connect to tuner." (≈17sp/700, amber, `letter-spacing 0.3`,
    nowrap) in an amber pill (height ≈44dp, padding 0 16dp, radius 10–12dp, 1.5dp
    amber border, amber-tint fill ≈ `rgba(201,118,10,0.08)` light /
    `rgba(255,184,51,0.10)` dark, gap 11dp). Wire to real tuner/SDR connection
    status: `true` whenever there is no compatible tuner session, clearing once one
    is connected and streaming — while `true`, signal / RDS / stereo / PTY are
    unavailable and must not be shown. The settings gear (right cluster) stays
    visible; its TUNER section (§6.3) is where the driver recovers.
- **Right cluster:**
  - **Driving-status icons** (GPS lock + vehicle-in-motion, plus the OsmAnd tell when that
    integration is on) sit just left of the gear on the **wide / landscape tracks only** — hidden
    entirely on the tall track (portrait / ⅓ slice). Spec in §4.6; the OsmAnd tell in §4.9.
  - **Settings** gear button (44dp square, bordered, dim icon).
  - **Clock** — sits **directly under the icon row**, right-aligned to it, on every track.
    Spec in §4.8 — it is out of flow, so it adds no height to the status bar. The nav **ETA**
    joins it there while OsmAnd is navigating (§4.9).
  - **Nearby-search** button — lives **here on the tall track**; on the wide
    track it sits in the preset band instead (§4.3). Icon spec in §7.

### 4.2 Hero (middle) — the primary readout
A centered column: **station row**, **frequency row** (large amber number, **no "MHz" unit label**), and a **RadioText
strip** beneath the hero card. FM is always MHz, so the unit label is omitted on the
face. The **star save button always sits in the top-right corner of the card** (both tracks),
never inline with the station identity — it is not part of the logo/name centering.

The station row has **two forms**, decided by whether the tuned station has a real logo (§4.5):
- **Real logo present:** the logo image REPLACES the big call-sign text — it sits centered in a
  box sized to a generous share of the card (`ContentScale.Fit`, §4.5). The call sign and frequency
  are **hidden by default** on a logo hero (§6.4), so the logo fills the card; each can be turned
  back on per station, appearing as a small call-sign label beneath the logo and the amber frequency below.
- **No real logo:** **no monogram tile is shown on the hero** — just the big **call sign** (largest
  text; italic-dim "Tuning…" / "Scanning…" when no PS name) and the frequency. The generated cube
  adds no value at hero size, so it is omitted here (it still appears on preset tiles/peek cards).

The call sign and the frequency on the hero are each controlled **per station** by the **Display Call
Sign** / **Display Frequency** options in the logo window (§6.4) — **both default OFF for a station that
has a logo** (logo-only hero), each turnable on individually. These toggles affect the **hero only**,
never the preset tiles, peek cards, or Nearby, and they exist only for stations with a real logo — a
**no-logo** hero always shows its call sign + frequency.
There is **no "MHz" unit label** anywhere on the face — it appears **only on the
overlay's Enter-frequency tab** (§6.1).

- **Frequency** — largest element, amber, tabular figures (~52–60sp). **Display only —
  not tappable**, and it must carry no ripple, pressed state or pointer affordance.
  Frequency entry lives on the tune overlay's Enter-frequency tab (§6.1).
- **Station name** — second largest; italic-dim "Tuning…" / "Scanning…" when no
  program-service name is present. Shown as the **4 core call letters only** (`-FM`/`-AM`
  and hyphens stripped, e.g. `WWHG-FM` → `WWHG`) — this applies everywhere a call sign appears.
- **Logo** — see §4.5 for the real-image model, per-surface fit, and the no-logo behavior.
- **★ save** — toggles the current station as a preset (filled amber when saved); **top-right corner**.
- **RadioText strip** — raised rounded bar. Long text (> ~46 chars)
  **marquee-scrolls** (continuous left ticker, ~16s loop); short text is centered
  static; dim italic placeholder when empty.

**Vertical behavior:**
- **Tall:** the leftover height is **distributed, not pooled**. The hero sits in the
  upper-middle (roughly a third down from the status bar) at a **slightly enlarged**
  size, and the **RadioText strip** is **centered in the gap between the hero and the
  preset shelf** — equal space above and below it. (Three equal flexible gaps: above
  the hero, hero→RadioText, RadioText→presets; in Slint, three `Rectangle`s with equal
  `vertical-stretch: 1` inside the `VerticalLayout`.) The prev/next **peek cards** (§5) still flank the hero
  here, tucked in tighter against it (a smaller negative overlap than on the wide track).
- **Wide:** hero is a fixed-proportion centered card (~62% width, clamped
  470–720dp) flanked by the prev/next **peek cards**.

### 4.3 Preset band (bottom)
**Preset tiles** plus band controls. A tile shows **either** a real logo **or** a call-sign box:
- **Real logo:** the image fills a **borderless, transparent** plate (§4.5, landscape-tolerant so
  wordmarks read); **frequency and call sign are hidden** — the logo carries the identity.
- **No real logo:** a **wide colored box** in the **same landscape aspect as the real-logo plate**,
  with the **4 core call letters centered inside it** (station-color fill), and the **frequency
  beneath** the box — the call sign is inside the box, not repeated below. Active tile shows the amber underline.

- **Wide (default):** a horizontal **scroll rail** of tiles with `‹ ›` page-nav
  buttons on each end, a thin drag scrollbar beneath, and the **nearby-search**
  disc button at the right end. Band height ~140dp (~104dp landscape). The
  **active** tile is emphasized (blue border, enlarged).
- **twoRows (Dudu7 ⅔):** the rail becomes a **2-row** horizontal grid with fixed
  tile width; taller band (~250dp).
- **Tall:** a **3-column vertical grid** (`LazyVerticalGrid(columns = Fixed(3))`)
  that scrolls vertically, pinned as a **bottom shelf**: content-height, **capped
  at ~45% of screen height** (`heightIn(max = screenHeight * 0.45f)`), placed last
  in the column with the hero's `weight(1f)` above providing separation; grid
  aligns to the **top** of the cap. Nearby-search is not in this band on the tall
  track (it's in the status bar).

**Interactions:**
- **Tap** a tile → tune to it.
- **Long-press** (~550ms) any tile → **reorder mode**: tiles wiggle; each shows a blue
  **logo-search badge** (magnifier-over-picture glyph, top-left) and an ✕ remove badge
  (top-right); a **DONE** button appears in the band.
- **Drag to reorder** (one continuous gesture): the long-press flows straight into a
  drag with the same finger — no lift-and-re-press. The picked-up tile lifts (scale
  1.06 + shadow) and tracks the pointer; the wiggle freezes on every tile during the
  drag; the remaining tiles **slide apart to open a real gap** at the insertion point,
  and the gap tracks the pointer. The list is not reordered mid-drag — on release the
  order commits and the dropped tile settles from the finger into the open slot (§8).
  There are no on-screen move arrows.
- **Logo-search badge** → opens the **preset logo-search window** (§6.4) for that
  station. It is the one and only way a logo is assigned (no automatic fetching).
- **Empty state:** dashed placeholder — "No presets yet — tune a station and tap
  the ★".

### 4.4 Prev/Next preset stepper
Step through the preset list and retune by tapping the **peek cards** (§5) that
flank the hero on both tracks.

### 4.5 Station logos — real image assets & per-surface fit

Logos are **real image assets** (transparent or opaque PNG/SVG), not just monogram
tiles. There is **no standard aspect ratio** — expect square badges, wide wordmarks
(up to ~3:1), and tall stacked lockups. Two logo kinds coexist:
- **Real logo** — an image assigned via the logo window (§6.4), with its own intrinsic aspect.
- **Generated monogram** — the colored rounded tile with call letters, used as the fallback
  wherever no real logo exists (preset tiles / peek cards only; never on the hero — see below).

**Containment invariant (every surface, non-negotiable):** a logo renders **fully visible** —
`ContentScale.Fit` (= CSS `object-fit: contain`), **never cropped, never overflowing its box,
centered**. The box has **fixed geometry**; the image scales to the largest size that fits
inside. This single rule handles all aspect ratios automatically: a wide wordmark uses the
box's full width, a square badge or tall lockup uses its full height. **Never size the box from
the logo, and never resize or crop the source to force it into a slot.**

**Separate the plate from the image.** The plate (white tile or transparent container) owns its
own size + padding; the image is width/height 100% with `Fit` **inside** the padded box. That
separation is what mathematically prevents overflow — do **not** hand-tune per-logo pixel heights
(that was the bug we chased before adopting this model).

**Prep on assign.** When a logo is saved, **bounding-box crop** the surrounding transparent/white
margin so the *visible mark* — not baked-in whitespace — fills the box, and **record the intrinsic
aspect ratio** (or bucket it: square / wide wordmark / tall lockup). The hero doesn't need the ratio
to render (Fit handles it), but storing it lets small slots make fallback decisions without re-measuring.

**Dark-mode adaptation is a separate, specified pipeline.** User-supplied logos that read fine in
light mode glare or vanish on the dark theme. Do **not** improvise a filter (`invert`, `hue-rotate`,
blanket white plate). Follow **`DARK-MODE-LOGOS.md`** — a five-stage import-time pipeline
(checkerboard flatten → trim → border-connected keying → coverage routing → OKLCh lightness remap /
halo / plate, with a catastrophe gate), validated against five real station logos. It runs **once at
logo import**, cached per (logoId, dark background colour), never at render time. Read that
document's **"Do not build"** section before writing any of it: several plausible approaches were
implemented and rejected there, including using its own score to *rank* candidates. Its top-priority
items are sRGB normalisation at decode and an import-time **treatment picker** (four candidate
thumbnails on the real dark background, pipeline's pick pre-selected, user can override).

**Per-surface budget (this is how you get max coverage):**

> **Exact dimensions: `LOGO-SIZING.md`.** This section defines the *model*; that document holds the
> per-surface numbers, the hero's three-step height table, and the acceptance test. A logo box is
> never a fixed thumbnail — on every surface it is the largest element in its card and it **grows**
> into space freed by hidden text.

- **Hero — maximum room. The default IS the largest size.** A real logo **replaces the big call
  sign**; `Fit`. Call sign + frequency are **hidden by default** on a logo hero (§6.4), so the
  standard, everyday state of a hero logo is the **top** of the height table — on Dudu7 full that is
  **210dp tall, not 118**. Build that state first and treat it as the norm.
  *The two smaller heights are exceptions, reached only when the user opts back in:* turning on the
  call sign and/or frequency steps the box down to leave room for the label beneath it (one on →
  middle value, both on → smallest). An implementation that renders the smallest value by default is
  wrong by roughly 44%. **No real logo →** show no monogram on the hero at all (call sign +
  frequency only). Star always corner.
- **Preset tiles & prev/next peek cards — aspect-tolerant, and identical to each other.** Real logo →
  **borderless, transparent** plate that the image fills; use a **non-square / landscape-ish** plate so
  wide wordmarks read; freq + call sign are **hidden**. The tile's logo box is a **grow-to-fill child**
  (`weight(1f)` / `flex: 1 1 auto`) that takes all tile height left after padding, gap and the
  call-sign row — **never a fixed dp height**, and it grows with the tuned tile. No logo → a **wide
  colored box in that same landscape aspect with the 4 call letters inside it**, frequency **beneath**
  the box. The prev/next peek cards use the **exact same treatment** as the bottom preset tiles.
- **Nearby search — no logos.** A small fixed square on a text-baseline row cannot render a detailed
  or wide logo legibly, and Nearby is low-traffic, so **there is no logo column** — the row is
  freq · callsign · city/genre · signal · distance.

**General fallback rule (for the real pipeline, beyond the sample data):** hero always uses the
image; tiles/peek use the image whenever one exists. **None of the five shipping surfaces qualifies
for a monogram substitution** — do not swap a real logo for a monogram or call letters on any
surface in this build. The legibility gate below exists only for a *hypothetical future surface* with
a genuinely small fixed box: there, skip the image and show the monogram/callsign when the mark would
render below ~28–32dp tall or its aspect is too extreme, rather than forcing an unreadable shrink.
It is **not** licence to shrink or substitute on the hero, tiles, or peek cards.
Brands commonly ship **two marks** (a horizontal lockup and a compact icon/monogram); when both are
available, prefer the **lockup for the hero** and the **icon for small slots**.

**Test with deliberately extreme aspects** — one square badge, one ~3:1 wordmark, one tall stacked
lockup — on every surface, light and dark. A roughly-square sample (like WERN) looks fine everywhere
and **hides** the wide/tall failure modes, so it is not a sufficient test on its own.

### 4.6 Driving-status icons (GPS lock + vehicle in motion)

Two glance indicators in the status bar's **right cluster**, just left of the settings gear — three
when the OsmAnd integration is on, which inserts its tell between them (§4.9), moving the vehicle
glyph one slot further left. **Wide / landscape tracks only — all are hidden entirely on the tall
track (portrait / ⅓ slice).**

- **GPS lock** — an **angled satellite** glyph (body tilted ~28°, small dish + two downward signal
  arcs toward the ground). **Lit interactive-blue on a GPS fix; with no fix it is not greyed but
  styled like a disabled tell (§4.1) — full text color at ~32% opacity with the same faint 1px
  emboss.** Always present (it communicates lock state); sits a couple of px above the gear's top edge.
- **Vehicle in motion** — a **car with three trailing motion lines**. **Rendered only while the app
  detects motion — absent (not dimmed) when stopped.** It is **amber** (the fixed safety-color family,
  like TA — never the blue accent) and **pulses slowly** (~2.6s opacity + slight-scale, gentler than
  TA's ~1.1s). Centered on the gear's vertical center.

Both are driven by real signals in the build (GPS fix state; motion/speed detection). In the prototype
they are the `gpsLocked` and `inMotion` tweaks on `CarFmLive`.

### 4.7 Audio-priority (on/off) control

The head unit shares one audio bus across sources; the FM app either **holds audio priority** or
**releases it** to another source. A **power-symbol button** in the hero card's **top-left
corner** — mirroring the ★ save button top-right — toggles this. Glyph is the universal power
mark (open ring broken by a top stem). **This is NOT a mute** (audio isn't silenced in place);
it claims or releases the tuner's priority on the shared bus.

- **Active (has priority):** the normal face. Button is a **dim outline**, no fill.
- **Inactive (released):** the button turns **solid amber (`#FFAE1A`) with a white glyph and
  pulses** (an expanding amber ring, ~1.8s) to draw the eye back, and the **whole face goes flat
  and "dead":**
  - **Grayscale** — the entire face desaturates, EXCEPT the power button, which is the one
    element that keeps color.
  - **Depth removed** — the hero card drop shadow, the RDS/HD/TP/AF tell emboss, and all text
    shadows drop to none (flat, lifeless).
  - **Veils** — a **light gray veil** over the hero card and a **darker veil** over the rest of
    the screen; the prev/next peek cards dim further (opacity ~0.28).
  - **Indicators to their off states** — signal icon shows **no lit arcs**, the level reads
    an **em-dash**, the
    **STEREO/MONO pill is empty** (outline with no text), RDS/HD/TP/AF all dim, PTY is hidden,
    and the **RadioText strip is fully invisible**.
- **Callbacks:** `claim` (inactive → take priority) and `release` (active → give it up). In the
  prototype: an `audioActive` boolean on `CarFmLive`; the button fires `onClaimAudio` /
  `onReleaseAudio`.
- **Prototype-only implementation note (does NOT port literally):** the grayscale is applied to a
  **static ancestor** of the hero + peek cards, and the power button is rendered **outside** that
  grayscaled subtree so it stays colored — a workaround for a browser GPU-compositing quirk. On
  Android, just desaturate the face content and draw the button in full color above it.
- Preset-change animation is **disabled while inactive** — see §8.

### 4.8 Clock

A time readout **under the right cluster's icon row**, right-aligned to it, on every track.
Optional: **Settings ▸ APPEARANCE ▸ Clock** (default **on**, persisted).

- **Face: DSEG7 Classic Mini, Regular** (Keshikan, **SIL OFL 1.1** — ships as
  `fonts/DSEG7ClassicMini-Regular.ttf`). A true seven-segment face, so it is the one place in the
  app that is not Atkinson — deliberate: the clock reads as head-unit hardware, not as UI text.
  **Regular only** — never a synthesized bold, which smears the gaps between segments.
- **Vertical placement.** The readout is **out of flow** (absolutely positioned, so it never adds
  height to the status bar) and the **clock line's optical center sits midway between the bottom
  edge of the right cluster and the top line of the hero card**. The cluster's bottom edge is the
  **settings gear** on the wide/landscape tracks and the **nearby button** on the tall track, where
  that sits below the gear. Right edge flush with the gear. **It is the CLOCK LINE that is centered,
  not the block** — so an ETA hanging below it (§4.9) changes neither the clock's size nor its
  position, and moves nothing else on the face. That works because the block sits **right of the
  hero card, above the near peek card**, which starts ~30dp lower: there is room for a second line
  there. The only clamp is a safety one — the block must clear the cluster above it and whatever
  actually sits beneath it at its own x-range (the peek card on the wide tracks, the hero card on
  the tall one).
- **Geometry.** Time at **30sp** (36sp on the tall track) in **`dim` — the same colour as
  the genre line**, `letterSpacing: 1`. DSEG7 sits small on the em, hence ~20% larger than the UI
  face would run. The meridiem is a **single letter, `A` or `P`** (never `AM`/`PM`) in the same face
  at **17sp** (20sp tall), also `dim`, **baseline-aligned** to the time, and absent in 24-hour mode.
- **Leading blank, not a leading zero.** 12-hour single-digit hours pad with **U+0020**, which DSEG7
  sets at digit width — the position simply reads unlit, as on real hardware, and the digit columns
  do not shift at 1 o'clock. Requires `whiteSpace: 'pre'` on the web; on Android just draw the space.
  24-hour **is** zero-padded (`08:05`). Minutes are always padded.
- **12/24-hour is not an app setting.** The readout asks Android:
  `DateFormat.is24HourFormat(context)` (i.e. `Settings.System.TIME_12_24`), and it re-formats on
  every tick, so flipping the system toggle in **Settings ▸ System ▸ Date & time** changes the
  radio face with no restart and no app-side preference to keep in sync. 24-hour is
  **zero-padded** (`08:05`), 12-hour is **not** (` 8:05 A`); minutes are always padded.
- **Tick.** One 1s timer, but state changes **only when the minute changes** — no per-second
  re-render of the face. Formatting is derived at render time, so a format change (system flip,
  or the prototype tweak) shows immediately.
- **Prototype.** `clockOn` on `CarFmLive` (persisted with freq + presets); the settings row fires
  `onSetClock`. Vertical placement is measured (`measureClock()`, folded into the existing
  `updateScroll` pass so it re-runs on mount, update and resize; skipped while the hero
  preset-change animation is in flight). A browser cannot read `TIME_12_24`, so the mock resolves the **host locale**
  (`Intl.DateTimeFormat().resolvedOptions()`) and the `clockFormat` tweak
  (*Follow system · 12-hour · 24-hour*) exists **only** to preview the other case. The settings
  sub-line reports which format is in force — it does not offer the choice.

---

### 4.9 OsmAnd turn-by-turn (maneuver layer)

When OsmAnd is navigating, the radio mirrors the next maneuver. **Three stages, each borrowing an
element that already exists — nothing reflows when navigation starts or ends.**

**What OsmAnd actually hands over** (read from the OsmAnd source, `net.osmand.aidlapi`, Aug 2026):

| Channel | Payload |
| --- | --- |
| `registerForNavigationUpdates` → `ADirectionInfo` (push) | `distanceTo`, `turnType`, `isLeftSide` |
| `getAppInfo` → `AppInfoParams` (poll ~1 Hz) | `arrivalTime`, `leftTime`, `leftDistance`, `mapVisible`, `turnInfo` bundle, `destinationLocation` |
| `turnInfo` keys, prefixed `current_` / `next_` / `after_next` | `turn_distance`, `turn_imminent`, `turn_name` (street, already formatted with ref + destination), `turn_type` (TurnType XML), `turn_lanes`, `turn_angle` |
| `registerForVoiceRouterMessages` → `OnVoiceNavigationParams` | the voice-router command list as it fires |

The push callback carries only the turn and its distance; **the street name, ETA, distance left and
the turn-after-next come from the polled `getAppInfo`**. Two consequences: poll on a timer while
navigating, and treat every field as optional — a missing one collapses, it never leaves a gap.
`turn_lanes` exists but lane guidance is **not** designed yet. OsmAnd does **not** expose its own
unit preference (see Units below).

**Stage 1 — cruising.** A **3dp amber hairline** flush to the face's top inner edge, its width the
fraction of the way to the next turn, plus **ETA directly under the clock** — right-aligned to it,
7dp clear, and in the **UI face, not DSEG7**: an `ETA` label at 11sp/700 with the time at 15sp/700
(17sp on the tall track), all `dim`. Two segmented readouts stacked would read as one four-line
instrument rather than two separate values, which is why only the clock keeps the segment face.
Distance-left joins it on the tall track, where there is room.

**The cruise countdown.** One centered line in the band **between the genre line and the hero
card's top**: `in` + the **distance to the next turn, counting down** (24sp/700 amber, tabular, 26sp
on the tall track) + the maneuver arrow at 26dp + the street in `dim` at 18sp. Out of flow like the
clock, so it costs no height. It is anchored to the **genre line's baseline, not the status bar's
box** — the box runs deeper on the left for the RDS chip row, but the centered column is clear from
the genre down, which is the ~40dp band the eye actually reads. Clamped between that baseline and
the hero card's top line.

**Cruise only.** At stage 2 the strip carries the same maneuver, larger; two copies of one turn on
one screen is noise, so the countdown ends when the strip yields.

*A seconds countdown was considered and rejected as unsourced:* OsmAnd hands over
`next_turn_distance` directly, but no time-to-next-turn — that would have to be derived from
current speed, and a derived clock that disagrees with the voice prompt is worse than no clock.
Distance is the number OsmAnd itself announces.

**The ETA costs nothing.** The clock keeps its size and its position, and no other element moves —
verified on the wide track: hero card 605×248 with its top edge at the same pixel, strip 64dp, clock
30sp centred where it always is, ETA sitting in the empty space right of the hero card and 14dp
clear of the peek card below (§4.8).

**Stage 2 — the RadioText strip yields.** Arrow + distance + street replace RadioText entirely
(RadioText does not alternate back), with `then` + a small dim arrow + the after-next street on the
right, divided by a hairline rule. The strip keeps its own geometry exactly — same 64dp height,
same zone.

**Stage 3 — the hero card takes over.** Big arrow, distance, street. **Only the hero card changes:
the peek cards and the preset shelf stay exactly where they are**, and the strip stops repeating the
imminent turn — it shows only the turn after it. The frequency readout is absent for those seconds,
but the **station logo moves to the card's upper-right corner** (101dp, 115dp on the tall track,
transparent — no plate or border behind it; the call sign in `dim` when a station has no logo), so
the driver still knows what is playing.

**Takeover edge.** Whichever element is carrying the maneuver — strip at stage 2, hero card at stage
3 — wears a **thick amber border that pulses on the vehicle-in-motion tell's exact beat** (6dp on
the hero, 5dp on the strip; 2.6s ease-in-out, amber → amber at 32%). Drawn as an inset ring
(`inset: -1`, radius +1) rather than by thickening the real border, so the card's own geometry and
content never shift; the border colour is animated through CSS variables, so nothing scales.

**Stage triggers are OsmAnd's, not ours.** Escalation follows `next_turn_imminent` (and the voice
router firing), so the radio changes at the same moment OsmAnd speaks — never on distance thresholds
of our own, which would disagree with the voice at highway speeds.

**Colour and theming.** The maneuver is **amber**, the fixed safety family (TA, motion tell) — never
the blue accent, never a band theme's palette. A band theme may **restyle the arrow art only**; the
colour, the sizes and the stage logic are not themeable. A theme that restyles a turn arrow's colour
is a safety bug, not an Easter egg.

**Suppression.** `AppInfoParams.mapVisible` reports whether OsmAnd's own map is on screen; while it
is, the maneuver layer is hidden — the driver is already looking at the turn. Driver-overridable
(**Settings ▸ NAVIGATION ▸ Hide when the map is showing**, default on).

**Arrow system.** `turn_type` arrives as a TurnType XML string; each maps to **one number, degrees
off straight ahead** (`C` 0, `KL/KR` ±20, `TSLL/TSLR` ±45, `TL/TR` ±90, `TSHL/TSHR` ±135, `TU/TRU`
±179) and a single generator draws stem + elbow + barbs from it, so the whole set is one stroke
language. U-turns use an arc instead of an elbow; `RNDB`/`RNLB` get a roundabout glyph with the exit
number set inside the ring. Same 24-unit box and round caps as every other icon (§7.1).

**Units.** Distances follow the locale (feet under 1000 ft, then miles to one decimal; metres under
1 km, then km) because OsmAnd's own unit setting is not exposed over the API. If that ever reads
wrong to a driver it needs an app-side override, not a guess.

**Settings.** `NAVIGATION` section: **OsmAnd integration** (default on; its sub-line reports the
link state and why nothing is showing — OsmAnd idle, off here, or hidden behind the map) and **Hide
when the map is showing**. Off means no mirroring **and** no tell in the status bar.

**The OsmAnd tell.** While the integration is enabled, OsmAnd's own mark sits in the right cluster
**between the vehicle-in-motion glyph and the GPS satellite** (so the car glyph moves one slot
left): **full colour (#FF8800) once the AIDL service is bound, the flat text colour at ~30% when it
is not** — the same "present but inert" language the GPS tell uses with no fix. Wide/landscape
tracks only, like the other two driving tells (§4.6), and nudged down 2dp so **the disc's centre
lines up with the gear's** — the mark's own box is not centred on its disc, the pin tail hangs
below. Small negative margins close the optical gap to the car and the satellite, since each tile
carries its own padding. The art is OsmAnd's **own** monochrome
launcher drawable (`OsmAnd/res/drawable/ic_launcher_osmand_monochrome.xml`, GPL) with its viewBox
cropped to the mark — their geometry and their orange, never a redrawn lookalike. Using it to name
the integration is nominative use; **confirm with OsmAnd before shipping**, and if they decline,
replace it with a neutral route glyph rather than a variation on their mark.

**Prototype.** `navState` (*Not navigating · Cruise · Approach · Turn now*), `osmandLinked` and
`osmandMapVisible` tweaks on `CarFmLive` stand in for the AIDL feed; demo maneuver is `TR` onto
Whitney Way, then `TSLL` onto Odana Rd.

---

## 5. Prev/Next peek cards

Flanking the hero, the previous and next presets show as **smaller cards** that use the **exact same
treatment as the bottom preset tiles** (§4.3): the station's **real logo image** when one exists
(borderless, Fit — §4.5), otherwise a **wide colored call-sign box** (4 letters inside) with the
**frequency beneath**.
(≈ scale 0.88, ≈ 60% opacity, outer edge softened by a fade gradient) that peek in
from the sides and sit slightly behind the hero. Tapping one steps to that preset.
They flank the hero on **both** tracks whenever a previous/next preset exists. Build
them as real sibling composables at the smaller size/alpha, clipped by the screen
edges. Scale/alpha values are starting points.

**The tuck is PROPORTIONAL — 35% of the peek card's own width** (the card is 20% of
the row, so 7% of the row, capped at 72dp) — and not a fixed dp per track. A fixed
46/72dp hid 35% of the plate on a 1024dp head unit and **64% of it on a 360dp
phone**, where the peek came out as a ~26dp shard of artwork that reads as damage
rather than as the next station. At 35% every surface keeps ~65% of the plate
showing (43dp on that phone) and the wide track's number is unchanged.

**Both edges fade, each for its own reason.** The **inner** edge fades where the
hero laps over the card — opaque to 55% of the card, ~10% at the inner edge — and
after the proportional tuck that ramp always begins inside the *visible* part
instead of behind the hero. The **outer** edge fades only where the card is
genuinely **clipped by the surface edge**, which is now the tall track: ~25% at the
very edge, opaque by 14%. On the wide track the peeks sit well inside the surface,
so an outer fade there would read as a soft-edged floating card — do not add one.

---

## 6. Overlays (modal: scrim + centered card)

Each overlay dims the face with a scrim and centers a rounded card (a top-level overlay
`Rectangle` with a `TouchArea` catching background taps, or a `PopupWindow`; or a
scrim `Box` + centered `Surface`).

**Sizing — fit every surface.** Each overlay has a *design* size but must never
exceed the surface it opens on. Cap to the available area with a margin, then
center, and let the body scroll:
`width = min(designW, screenW − 32dp)`, `height = min(designH, screenH − 32dp)`.
The card body (station list / settings groups / keypad) scrolls, so a smaller cap
scrolls internally rather than truncating. Design sizes: tune overlay 900 × 600 (both
tabs), settings 700 × 576. Overlays must
never carry a hard-coded pixel size on a real surface.

### 6.1 Tune overlay — two tabs (`NearbyPicker`)
One overlay, 900 × 600, holding **both** ways to change station: a **Nearby stations**
tab (§6.2) and an **Enter frequency** tab. There is no separate numpad overlay, and the
hero frequency does not open anything (§4.2) — the **nearby button** in the status bar
opens this overlay on Nearby, and Enter frequency is reached by switching tabs.

**Header = the tab bar.** There is **no separate title/subtitle block**. One header row
(1px `border` bottom, padding 12/20dp; narrow 10/14dp) holds two `weight(1)` tab buttons
(gap 10dp, 16dp before the ✕; narrow 12dp) and the close ✕ (52 × 52, radius 12, 1px
`border`, `dim` glyph 22sp). Each tab button is a vertical stack, min-height 58dp
(narrow 50), radius 12, padding 8/16dp (narrow 7/12), gap 2dp:

- **heading** — 20sp bold (narrow 17sp), `text`, or `blue` when active.
- **hint** — 13sp regular (narrow 12sp), letter-spacing 0.3, `dim` **in both states**.
- Both lines single-line and ellipsized.
- Active tab: `1.5dp solid blue` border + `blueFill`. Inactive: 1dp `border`, transparent.

Copy: **Nearby stations** / "Tap to tune · hold to save a preset · best signal first"
(narrow: "Tap to tune · hold to save"), and **Enter frequency** / "Type a frequency, or
seek".

**Contrast requirement:** the active tab's hint stays `dim`. Blue hint text on `blueFill`
measures 2.3:1 and is rejected — the border, fill and blue heading are the whole active
tell.

**Enter-frequency tab.** A scrolling body (padding 8dp / side pad) centring a column
(`max-width` 440dp, gap 9dp, vertically centred), top to bottom:

1. **Readout** — 62dp tall, radius 14, `raised` fill, 1dp `border`; baseline row, gap 10:
   value 42sp bold `amber`, tabular numerals, letter-spacing −1; unit **MHz** 18sp bold `dim`.
2. **Seek row** — two `weight(1)` buttons, 48dp tall, radius 14, `raised`, 1dp `border`;
   `SEEK` 15sp bold letter-spacing 1.5 `text`, with `‹‹` / `››` 22sp bold `blue` on the
   outer side. Scans to the next/previous strong station.
3. **Band error** (conditional) — "⚠ Outside 87.5–108.0 MHz band", 14sp bold `amber`, centred.
4. **Keypad** — wrapping flow, gap 9dp; 12 keys `1–9 . 0 ⌫` in 3 columns
   (`width = (100% − 18dp) / 3`), 48dp tall, radius 12, `raised`, 1dp `border`, 26sp bold `text`.
5. **Actions** — `CANCEL` (1dp `border`, `dim`) and `TUNE` (1.5dp `blue`, `blueFill`, `blue`),
   `weight(1)`, 52dp tall, radius 14, 16sp bold letter-spacing 1.

The column fits 900 × 600 without scrolling; the scroll container exists for shorter
head-unit slices. Every target is ≥ 48dp.

**Behaviour.** Digits append to an entry buffer (max 4 digits, one decimal point); `⌫`
deletes one. The readout shows the buffer while typing, otherwise the live frequency — or
the sweeping value while seeking. **TUNE** validates the **87.5–108.0 MHz** band, commits
and closes; an empty or invalid buffer closes without retuning. **CANCEL**, ✕ or a scrim
tap **from this tab** abandons the entry and restores the frequency the overlay opened on;
✕ or scrim **from the Nearby tab** just closes. Switching tabs clears the buffer and stops
any running seek. Seeking leaves the overlay open when it lands.

State lives in the host, not the overlay: it takes `tab`, `onSetTab`, `freqDisplay`,
`freqError`, `onKey`, `onBack`, `onSeekUp`, `onSeekDown`, `onCommit`, `onCancelTune`
alongside the existing picker props.

### 6.2 Nearby-stations tab (`NearbyPicker`)
The overlay's default tab: a filter area, a scrolling **station list**, and a footer
("FCC data as of <snapshot date>"). Its heading and hint live in the tab button (§6.1).

- **Station row:** frequency (large, tabular, **no "MHz" label**) · call sign · optional service badge (when not "FM"); a second line of
  `city · genre`; a trailing signal icon + distance ("<km> km"); a saved ★ when
  already a preset. **No logo column** — Nearby does not show station logos (§4.5).
  already a preset; a `›` chevron. **Tap** tunes; **long-press** (~550ms) saves it
  as a preset. Rows are sorted best-signal-first. On a **narrow** picker (phone
  portrait / ⅓ vertical slice, i.e. when the picker is clamped below ~620dp wide)
  the row uses **compact metrics** — smaller logo, freq, callsign, gaps and padding,
  with the info column taking the row's slack — so the callsign never wraps and the
  columns don't cram. Wide surfaces keep the full-size metrics.
- **Filter — two levels:**
  - **Bucket row:** `All` · `Music` · `Talk` (Music/Talk shown only when the list
    actually contains such stations), shown **only while All is active**. Selecting
    **Music** or **Talk** hides this row entirely and shows the genre row below (the
    genre row's leading chip is the way back — see below).
  - **Genre row** (shown only inside Music/Talk when >1 genre exists): genre chips
    laid out in **exactly two rows**, flowing column-by-column and scrolling
    horizontally when they overflow. When drilled into Music/Talk there is **no
    separate bucket row** — an **icon-only back-arrow reset** chip (raised fill,
    spanning both rows) followed by a thin vertical **divider** leads the genre
    row and returns to the All/Music/Talk buckets. Selecting a genre filters the
    list; tapping it again clears it.
- **Alternate states:** **no-GPS** (crosshair glyph, "Waiting for GPS…") and
  **empty** ("Station database not installed yet" with install guidance) replace
  the list. Both are placeholder scaffolding for edge cases, not built-out flows.

### 6.3 Settings (`SettingsPanel`)
A header ("Settings" + close ✕) over a scrolling body of grouped sections:

- **TUNER** — a connection status row (wave icon + "Connected …" / amber "Not
  connected"; **RETRY** when errored, **Details** expands a diagnostics panel: device,
  USB ID, sample rate) above a **source picker** (single-select), each row = name · kind ·
  status badge: **RTL-SDR** (USB software-defined radio; Detected / Not detected) ·
  **NWD / NOWADA built-in radio** (integrated head-unit FM tuner; Detected / Not detected) ·
  **FYT / DuduOS built-in radio** (integrated head-unit FM tuner; **Unavailable — greyed**) ·
  **Auto** (probe all sources; no badge) — **`NWD / NOWADA built-in` is the default
  selection.** A **Start radio on boot** toggle.
- **The tuner picker drives the face.** Selecting a source switches the status-bar readout
  (§4.1 / `SIGNAL-METER.md`): RTL-SDR → the dB value with its unit; NWD and Auto → the
  unitless NWD level. The connection status row follows the same selection ("Built-in
  hardware · NWD/NOWADA FM tuner" vs "Local hardware · RTL-SDR (RTL2832U)"), and the
  **Details** diagnostics panel — which names the RTL2832U chip — appears **only** on the
  RTL-SDR selection.
- **APPEARANCE** — **Theme** segmented control: SYSTEM / LIGHT / DARK.
- **SYSTEM** — **Battery optimization** status (amber "Not exempt" with a **FIX**
  action, or blue "EXEMPT"); **Station logos** toggle with a "Clear downloaded
  logos" row when on.
- Footer about line (app name · version · data snapshot).

The built panel (`SettingsPanel.dc.html`, in this bundle) is the exact reference
for these sections, values, and copy.

### 6.4 Preset logo-search window (`LogoSearchOverlay`)
A **modal card over the radio face** (same dim-scrim + rounded-card pattern as the
tune overlay / settings), opened by the per-tile logo-search badge in reorder mode. It
is the **only** way a station logo is assigned — there is no automatic/background logo
fetch.

**It opens on a LANDING view, not a search:**
- **Station has a logo:** shows the **current logo** (large, Fit) plus two option rows,
  **Display Call Sign** and **Display Frequency** — both **unchecked by default** (logo-only
  hero) — and a **"Search for a different logo"** button.
- **No logo:** shows a **"No Logo Installed"** message + a **"Search for a logo"** button.

**Display Call Sign / Display Frequency affect the HERO CARD ONLY**, saved per station, and **default
OFF** — a freshly-assigned logo yields a logo-only hero; check either to bring back the small call-sign
label and/or amber frequency. They do not change the preset tiles, peek cards, or Nearby. They persist
with the station.

**Search runs only when the Search button is pressed** (query built from the station; no query
field, no submit).

- **Trigger glyph:** a **magnifier over a picture** (framed image with a tiny sun +
  hill, lens at lower-right) — deliberately distinct from the Nearby magnifier-over-
  tower (§7). White stroke on the blue badge; ≥48dp touch target (hitSlop is fine).
- **Header:** the station's current logo tile, its **name**, and `callsign · frequency
  MHz`, plus a **query chip** showing the exact string searched (e.g. `radio 98.1 wmgn
  logo`) so the driver can trust the results. A close ✕.
- **States, in order:** **Loading** (spinner + "Searching for logos…"); **Results** —
  the first **four** image candidates in a **2×2 grid** (each cell = the candidate art
  on its own background + a caption of source domain and pixel dimensions); **No
  results** and **Error**, each a short message with a **Search again** action; and a
  **Saving** busy state on Confirm.
- **Selection:** tap a cell to select (single-select) — **blue** 2dp border + blue
  fill tint + a blue check badge (never red/green; §6 colourblind rule). Selecting
  enables **Confirm**.
- **Confirm** (enabled on the landing view, or once a cell is selected in results): saves the
  chosen image as this station's **manual logo** (sticky — never overwritten later) together with
  the **Display Call Sign / Display Frequency** choices, refreshes
  every tile showing that station, and closes. **Cancel** / scrim / ✕ closes and
  changes nothing.
- **Responsive:** the 2×2 grid fits the narrow track (phone portrait / ⅓ slice) with
  no horizontal scroll; light/dark themes as elsewhere.
- Backend wiring (search, save-as-manual, tile refresh) is host-side; the design
  supplies the icon + window. `LogoSearchOverlay.dc.html` is the exact reference; its
  `demoState` prop flips between the states for review (`landing` is the default).

---

## 7. Nearby-search icon (match exactly — do not improvise)

A **magnifier whose lens contains a broadcast tower**: a circular lens (thin
stroke) over a faint glass fill; inside it a narrow **A-frame tower** with a
single low cross-brace and a short antenna mast; a tip dot; and **two broadcast-
wave arcs on each side** of the tip. A subtle barrel / lens-refraction warp bows
the tower slightly. A magnifier **handle** runs off the lower-right at ~45°
(~5 o'clock, stubby butt cap).
Default is themeable, but **as shipped (via `CarFmLive`) it is dark strokes
(`#111111`) on a white disc (`#FFFFFF`)**, with a light-gray border (`#D5DAE1`) and
a faint blue-tinted lens (`#DCE7F5`). In **dark** theme the shipped colors are
light strokes (`#E9EEF4`) on a **panel** disc (`#33373D`), with the lens a step
lighter on `raised` (`#43474E`) and its own rim colour (`#565B63`). **The disc is
not `raised`:** with disc and lens both `#43474E` the glass vanished into the metal
it is set in — this line is corrected from the prose, which said "raised disc". Disc / line / glass / border colors are themeable — a blue disc is
available but is not the default. Render as a vector drawable and reproduce the
tower + waves precisely — `RadioFace` (`lensTower` / `nearbyIcon`) is the exact
reference geometry.

---

## 7.1 Glyphs the type face does not carry — draw them

Atkinson Hyperlegible has **no ✕, ✓, ★, ⚠ or ⌫** in either cut. A text element asking
for one does not fail — it silently falls back to whatever face the platform supplies,
which is why a typed close button reads as borrowed chrome sitting next to icons that
were drawn. **Every one of these is a stroked path, not a character.**

| Glyph | Where | Draw |
|---|---|---|
| **✕ close** | tune overlay, settings, logo window (52dp button) | two crossed strokes on a 24 box — `6.4,6.4 → 17.6,17.6` and `17.6,6.4 → 6.4,17.6`; 2.4dp, round caps; rendered 22dp in `dim` |
| **✓ confirm** | reorder **DONE**, the logo window's check badge, settings checks | polyline `5,12.8 → 9.8,17.6 → 19,6.6` on a 24 box; 2.6dp (3dp on the 17dp badge), round caps and joins |
| **⚠ warning** | tuner-error pill, status-bar out-of-band pill, hero out-of-band caption, keypad band error | ONE triangle, sized to the line it sits on — 26dp/2dp on the tuner-error pill, 19dp/2.2 in the status-bar pill, 16dp/2.3 on the keypad error, 15dp/2.4 on the hero caption; no fill, `currentColor`, so each takes its parent's amber |
| **★ save** | hero star, saved rows in Nearby, **and the empty-band line** | one path — `M12 17.3 L18.2 21 16.5 13.9 22 9.2 14.8 8.6 12 2 9.2 8.6 2 9.2 7.5 13.9 5.8 21 Z` — filled amber when saved, else a 1.7dp outline in `dim`. The empty-band sentence *names* the star, so it ends with the mark at 19dp in the text colour: build that line as **text + icon**, not one string |
| **⌫ backspace** | keypad's twelfth key | outlined key body `M9.6 5.6 H19.6 a1.6 1.6 0 0 1 1.6 1.6 v9.6 a1.6 1.6 0 0 1 -1.6 1.6 H9.6 L2.9 12 Z` with a small ✕ inside (`12.6,9.6 → 17.2,14.4` and back); 1.9dp, round caps, 24dp on the 48dp key. **Key the twelfth key by an id, not by the character** — a glyph carried in a string is a glyph something will eventually print |

Common language: `currentColor`, round caps, no fill, and sized **from the resolved
button** so a 48dp floor carries the glyph up with it. In Slint they are `Path`
elements beside the other icons, tinted by the caller.

---

## 8. Motion

- **Preset-change hero animation (both tracks):** the current hero **shrinks and
  translates into the previous peek slot** while the next card **grows and moves
  into center** — a real position/size morph (FLIP), not a slide or crossfade. The
  dropped far card fades out (0.6 → 0); a new far card fades in (0 → 0.6). Scale
  settles slightly ahead of translation so cards reach the right size before
  landing; ~520ms total, translation on an ease-out cubic, scale on a faster
  ease-out quint. Each moving card ends at its **resting transform** (peek cards
  keep their 0.88 base scale — never identity), so nothing pops. Express as
  animated bounds/scale/alpha transitions between the two card slots.
  **This is the one animation most likely to be skipped — build it from the exact
  FLIP procedure (capture bounds → tune → morph, two easings, resolve to the 0.88
  peek base) in LOSSY-ELEMENTS.md #9. Verify against a screen recording, not a still.**
- **Audio-off:** while audio priority is **released** (§4.7) the preset-change hero animation is
  **disabled** — preset changes swap **instantly** (no morph, no fade). This reinforces the flat
  "dead" feel and avoids animating a desaturated face (it also sidesteps a compositor issue in the
  prototype). Re-enables automatically when priority is reclaimed.
- **Preset reorder (drag):** the picked-up tile lifts (scale 1.06 + shadow,
  `touch-action:none`) and tracks the pointer. The other tiles **slide apart to open a
  gap** at the insertion slot (transform-only, ~160ms ease; the wiggle is frozen for
  the duration). Insertion geometry is computed against the slot rects captured at
  drag start, so the gap does not oscillate as in-flight transforms move. The list is
  NOT reordered mid-drag — on release the order commits and every tile (including the
  dropped one, sliding from the finger) resolves to its new position with a FLIP slide
  (~300ms, decelerate). Committing only on drop avoids the index-churn that live
  reordering causes with an index-keyed list. The long-press→drag is one continuous
  gesture (same pointer); a >12dp move before the long-press fires cancels it, so the
  rail can still be scrolled.
- **Scanning:** frequency ticks through values (~34ms/step) toward the target
  station; small vertical fade on the readout per step.
- **TA flag:** continuous amber scale-pulse (~1.1s) while a traffic announcement
  is active.
- **Vehicle-in-motion icon:** slow amber pulse (~2.6s opacity + slight scale) while the
  vehicle is in motion (§4.6) — gentler and slower than the TA pulse; the icon is absent
  (not dimmed) when stopped.
- **Marquee RadioText:** continuous horizontal ticker (~16s loop) for long text.

Respect reduced-motion / driving-restriction settings if the platform exposes them —
these are glance UIs and none of the motion is essential to function.

---

## 9. Tuner / state model

The ViewModel owns: current `freq`, `presets[]` (name + freq, persisted), and
derived per-station metadata (PS name, logo, PTY, RDS/TP/TA/AF flags, HD level,
signal). Persist `freq` + `presets` (DataStore/prefs).

- **Band:** 87.5–108.0 MHz, 0.1 step, wraps at the ends.
- **Seek/scan:** jump to the next/previous station with signal.
- **Save (★):** toggle the current station in/out of presets.
- **Audio priority:** `audioActive` (has priority / released). `release` hands the shared audio
  bus to another source; `claim` takes it back. The released state is **visual-only in the
  prototype** (not persisted) — full off-state visuals in §4.7.
- **Reorder / remove / change logo:** via long-press reorder mode (drag to reorder,
  ✕ to remove, logo-search badge to open the logo-search window §6.4).
- Prototype station metadata is a fixed demo DB (Madison, WI market); the real
  build pulls PS/RT/PTY/flags from the RDS decoder and signal from the tuner, and
  nearby stations from the FCC dataset.

---

## 10. Safety constraints (must hold)
- Frequency readout is **amber by default** in both themes. It is an ordinary themeable
  value (§12 `freqColor`), not a protected one — no band theme currently changes it.
  (No "MHz" unit label on the face — FM is always MHz; the label appears only on the
  overlay's Enter-frequency tab.)
- **TA** must be visually loud (pulse) — traffic announcements override.
- **Audio-off must be unmistakable:** when audio priority is released the whole face desaturates
  and flattens, and the power button is the **sole colored, pulsing** element (§4.7), so the
  released state is glanceable at speed.
- **No scale-to-fit.** Real responsive logical-px layout that reflows per surface
  (§0) — never a fixed canvas uniformly scaled to the screen.
- Hit targets **≥ 48px** in real logical px (not a scaled-down canvas); honor text-scale
  to ×1.5 without overlap.
- Glance-legible type; nothing critical below ~15sp.

## 12. Band themes (artist Easter eggs)

> **Build this section from `EASTER-EGGS-BUILD.md`, not from the prose here.** That
> document is the authoritative build spec: it ships the real font files (`fonts/`), the
> real vector art (`art/`), exact placement coordinates, per-theme reference captures
> (`screenshots/egg-*.png`) and an acceptance checklist. **Theme art is supplied as SVG —
> never substitute an emoji, a Material icon, or a redrawn approximation, and never
> substitute a system font for a theme face.** The summary below is orientation only.

Cosmetic skins that dress the face for the artist currently playing. Purely
presentational: no layout, control, or behavior changes, and the theme reverts the
instant the track changes.

**Activation.** The incoming RadioText is lower-cased and stripped of punctuation, then
matched against each theme's `names` substrings. First match wins. A theme forced from
the settings secret panel (below) overrides detection and stays until switched off.
Nothing persists across a reboot.

**Registry.** All themes live in one `EGGS` array in `RadioFace` `renderVals()`; each
entry is pure data plus a `motif` string selecting the branch that draws its custom art.
Port it the same way — a data class list, not five hard-coded skins. Fields:

| field | effect |
|---|---|
| `id`, `names` | label + lowercase punctuation-stripped match substrings |
| `font` | display face for station name, call signs, presets, RadioText |
| `heroFont`, `heroScale`, `heroTrack` | hero-only face, size multiplier, letter tracking |
| `freqFont`, `freqScale`, `freqColor` | frequency face, size multiplier, colour |
| `rtFont`, `rtSpacing`, `bold` | RadioText face, tracking, weight |
| `genreText`, `genreColor`, `genreFont` | replace/restyle the genre line |
| `genreCycle` | two genre strings, ~20s each, cross-faded on a 40s loop |
| `genrePulse`, `genrePulseOn` | colour pulse stops for the genre line |
| `genreOutline` | `{color,width}` ring outline behind the genre line |
| `card`, `cardFrame`, `pageBg` | hero card palette, concentric hoop rings, page background |
| `rtPlate` | RadioText plate palette (+ optional `serial` string) |
| `nameBlock`, `nameGhost`, `nameOutline` | call-sign block fill, off-register ghost, outlined lettering |
| `heroGlitch` | banded slice displacement on every call sign |
| `uiAccent`, `uiAccentFill`, `uiAccentOn` | restate the interactive accent (recolours every interactive element) |
| `suppressLogos` | hide station logos so the theme's own type reads |
| `stereoArtL/R`, `stereoArtFilter` | art flanking STEREO, plus a filter to recolour it |
| `modes` | `{light:{…}, dark:{…}` — palette merged over the entry for the active colour scheme |
| `motif` | selects the custom-art branch: `acdc` \| `submarine` \| `xerox` \| `spiral` \| `runes` |
| `genreArt`, `fontScope`, `heroStroke` | art in place of the genre line; limit the theme face to one scope (`hero`); hero stroke weight |
| `tier` | `basic` marks a one-line theme — genre line, optionally its face, nothing else |
| `genreFace` | a face for the genre line that keeps the ordinary egg-genre size and weight, unlike `genreFont`, which changes both |

**Shipping themes.**

1. **AC/DC** (`acdc`) — Squealer display face; red devil horns scrawled either side of the
   hero; a flat-topped lightning bolt splitting every call sign and standing in for the
   settings gear; bolt art flanking STEREO; "High Voltage Rock 'n' Roll" on the genre line
   in pulsing amber→gold. *Light* adds a brown-black ring outline to the genre line only.
   *Dark* is a second egg inside the egg — **Back in Black**, reached simply by driving at
   night once the head unit switches to dark: true-black page, near-black panels separated
   by value rather than outline, a silver hairline on the hero card, hero call sign and
   frequency filled in the panel's own black with a 1.1px silver outline, and the whole
   interactive accent restated silver (including a filter over the fixed-colour bolt art).
2. **The Beatles** (`submarine`) — each slot dressed as a different era: bulbous Yellow
   Submarine type, a Sgt-Pepper drum hoop around the hero (concentric rings), a White-Album
   plate with a serial number for RadioText, Abbey-Road stripes on the ground plane, warm
   cream/gold/brown palette. **Unfinished** — the drum's two lettering treatments are not
   resolved; see the note at the end of this section.
3. **Nirvana** (`xerox`) — the era's print language, **no palette changes at all**:
   Permanent Marker throughout, an off-register second impression behind the call sign
   (neutral 20% black), "Verse Chorus Verse" on the genre line, Onyx at 1.5× on the hero
   card, and a geometric smiley for the gear.
4. **Nine Inch Nails** (`spiral`) — Foundry Gridnik caps tracked 9px with banded glitch
   slicing on every call sign, Singothic for the genre line, RadioText and **all** frequency
   readouts, genre cross-fading "Broken Machines" ⇄ "Things Falling Apart", and a hand-drawn
   downward spiral for the gear. Default palette.
5. **Led Zeppelin** (`runes`) — **no palette of its own**: default theme colours throughout,
   like Nirvana. The theme is type and marks only — Kashmir on the hero card and RadioText
   (`fontScope: 'hero'`, so presets and peek cards keep the default face), the untitled
   fourth record’s **four runes standing in for the genre line** (debossed, not printed),
   and a **rigid airship** replacing the vehicle-in-motion tell. The settings gear is
   **not** replaced on this theme. Secret-panel entry: "Hammer of the Gods".
6. **Tom Petty** (`tophat`) — the smallest theme in the set: **no palette and no font of
   its own**. A single hero prop — the **sketch-outline top hat** modelled on Petty's Baron
   California Hats stage topper (low flared crown, deep low band, moderate brim with lifted
   ends; open strokes, no fills, theme-accent colour), 205 × 165dp in the hero-prop **center**
   slot at `top: -104dp`, `left: 75%`, `rotate(5deg)` — plus the genre line "Heartland Rock".
   Logos stay visible; gear, motion tell and STEREO keep their defaults. Secret-panel entry:
   "Charlie T. Wilbury Jr.".

**Two tiers.** The six above are **advanced** — palette, faces, marks, hero props. A
**basic** theme is one line: it renames the genre line and may lend its face to that line
and the RadioText, and **nothing else moves** — no palette, no hero treatment, no marks, no
gear replacement, and the station logos stay visible. That is what makes a row cheap enough
to add for a band someone likes. **Advanced rows match first**, so a RadioText naming one of
each gets the advanced dress; that is structural (the advanced list is chained ahead of the
basic one), not a convention about where to paste a row.

| id | matches | genre line | face |
|---|---|---|---|
| Eric Clapton | `clapton` | Slowhand | — |
| The Pretty Reckless | `pretty reckless` | Cindy-Lou Who? | — |
| The Who | `the who` | Meaty, Beaty, Big, and Bouncy | — |
| Carry On Wayward Son | `wayward son` | Season Finale | Supernatural Knight, on the genre line + RadioText |

`the who` is two of the commonest words in English adjacent, so it *can* fire on prose;
held anyway, because RadioText is station/artist/track copy and a basic row's worst case is
one wrong genre line rather than a repainted face. It does not collide with "The Guess Who"
— the match wants the two tokens adjacent. **Supernatural Knight has no bold cut**
(`usWeightClass` 400, no bold bit), so the 700 the genre line asks for resolves to the
single cut and **0.02em of tracking rides with it** to stand in for the weight; the family
is "Supernatural Knight" **with the space**, read from the file's own name table, not the
underscored filename. It also carries **no accented Latin** and it reaches the RadioText, so
a Spanish title renders its accents in the fallback face. Both accepted.

**Secret panel.** Six taps on the settings-modal brand line reveals a "band themes" group
listing each theme by a pun name (Powerage · The Walrus was Paul · Smells Like Gen X ·
Now I'm Nothing · Hammer of the Gods · Charlie T. Wilbury Jr.) with a toggle each; picking one forces it on regardless of what is
playing, and the labels are display-only — matching still uses `id`. **Advanced themes only.**
Basic rows are deliberately absent: a one-line theme is not something to go hunting for, it is
something that turns up when the right track plays. They match from live RadioText and nowhere
else.

**Fonts.** All ten theme faces ship as real files in `fonts/` (Squealer, BeatlesYellowSub,
SgtPeppers, MadieRoger, PermanentMarker, Onyx, Gridnik, Singothic, Kashmir, and
SupernaturalKnight — the basic tier's one face). Put them in
`res/font/` and bind by resource id. Nothing is fetched at
runtime, and **no theme falls back to a system face** — a missing font is a build failure, not
a silent substitution. Per-theme assignment table: `EASTER-EGGS-BUILD.md` §1.1.

**Art.** All theme art ships as SVG in `art/` (AC/DC horns L/R + bolt; a gear replacement for
the `acdc`, `submarine`, `xerox` and `spiral` motifs; the four Led Zeppelin runes; and the Tom
Petty top hat sketch) plus
`assets/fan-l2.png`/`fan-r2.png`. Draw those files. Placement
coordinates and tints: `EASTER-EGGS-BUILD.md` §1.2 and §2.

**Scale-up rule.** A theme may change *type, colour and ornament only*. It must never move a
control, change a hit target, alter what a control does, or introduce motion that competes
with driving. Themed elements keep their default sizes unless the theme sets an explicit
scale field.

**Unfinished — The Beatles drum lettering.** The Sgt-Pepper hero needs two lettering
treatments (solid filled letters with a hard offset shadow; thin-outlined letters with an
accent stripe following each stem). The bundled SgtPeppers face is outline-only, so neither
can be produced from it with text effects; the CSS approximations currently in the source
(`callLined` / `freqShadow` / the glyph-flood in `solidType1`) are known-wrong placeholders.
Resolve with the font's solid cut or hand-drawn SVG lettering before building this theme.

## 11. Verify on device (see also `CARNYX-SLINT.md` §6)
- Portrait (360×800): hero upper-middle and slightly enlarged; RadioText centered
  in the gap between hero and presets; 3-col preset shelf pinned bottom, scrolls
  past the ~45% cap; peek cards flank the hero, tucked tight.
- Landscape (800×360): hero + peek cards + preset rail fit with no vertical
  clipping.
- Dudu7 full, ⅔ slice (twoRows) and ⅓ slice (narrow tall) all hold up.
- Font-scale ×1.3 / ×1.5: no overlap; frequency stays readable and amber.
- Theme light↔dark: amber unchanged; accent blue and surfaces swap.
- Overlays (tune overlay / settings) on every surface: card fits with a margin and
  scrolls internally — never clipped on the narrow or short surfaces. Both overlay tabs
  are checked, and the tab headings/hints ellipsize rather than wrap.
- Picker filter: selecting Music/Talk collapses the bucket row to All; genre chips
  sit in two rows and scroll horizontally.
