# Band themes (artist Easter eggs) — BUILD SPEC

**Bundle v1.14.6 — 2026-08-07.** Companion to `ANDROID-IMPLEMENTATION.md` §12. §12 describes *what each
theme is*. **This document tells you exactly what to build, from which file, at which
coordinates.** Where the two disagree, this document wins.

---

## 0. The contract

Read these five rules before writing any code. Every one of them exists because a build
already broke it.

1. **Draw the supplied vector art. Do not draw your own, and never substitute a glyph.**
   Every piece of theme art in this bundle ships as an SVG file in `art/`. Use those files.
   **An emoji is never an acceptable stand-in for theme art** — not 🤘, not ⚡, not 🙂,
   not any Material icon that looks similar. If a piece of art is missing from `art/`, stop
   and ask; do not improvise one.
2. **Ship the supplied font files. Do not substitute a system face.** Every theme font is a
   real file in `fonts/`. A theme's identity *is* its typeface — Squealer missing from the
   AC/DC theme means the AC/DC theme is not built. Fallback chains in the prototype source
   (`"Squealer", "Atkinson Hyperlegible", sans-serif`) exist only so the HTML degrades in a
   browser; **on Android there is no fallback** — bind the font resource directly.
3. **Every theme is data + one art branch.** Build the registry as a data class list and a
   `when (motif)` branch, exactly as §12 specifies. Do not hard-code five skins.
4. **A theme changes type, colour and ornament only.** No layout, control, hit-target, or
   behaviour change. Ever.
5. **Verify each theme against its reference capture** in `screenshots/egg-*.png` before
   calling it done, using the process in `CORRECTION-LOOP.md`. "It compiles" is not done.

---

## 1. Assets shipped in this bundle

### 1.1 Fonts — `fonts/`

All nine are extracted from the prototype and included as real files. Put them in
`res/font/` and reference by resource id. **None is fetched at runtime.**

| File | Family | Used by | Applied to |
|---|---|---|---|
| `Squealer.otf` | Squealer | AC/DC | station name, call signs, presets, RadioText |
| `BeatlesYellowSub.ttf` | BeatlesYellowSub | The Beatles | everything except the hero |
| `SgtPeppers.ttf` | SgtPeppers | The Beatles | hero call sign + frequency (outline-only cut — see §2.2) |
| `MadieRoger.ttf` | MadieRoger | The Beatles | genre line |
| `PermanentMarker.ttf` | PermanentMarker | Nirvana | everything except the hero |
| `Onyx.ttf` | Onyx | Nirvana | hero call sign + frequency, at 1.5× |
| `Gridnik.woff` | Gridnik | Nine Inch Nails | station name, call signs, presets, hero |
| `Singothic.ttf` | Singothic | Nine Inch Nails | genre line, RadioText, **all** frequency readouts |
| `Kashmir.ttf` | Kashmir | Led Zeppelin | hero card + RadioText **only** (`fontScope: hero`) |

**Verification step, do this first:** render every theme's call sign at 96sp and compare
letterforms against `screenshots/egg-*.png`. If the shape matches Roboto, the font did not
load and the theme is wrong. Fail the build loudly rather than falling back silently.

### 1.2 Vector art — `art/`

| File | Theme | Placement |
|---|---|---|
| `acdc-horn-left.svg` | AC/DC | hero card, **top-left**, overhanging (§2.1) |
| `acdc-horn-right.svg` | AC/DC | hero card, **top-right**, overhanging (§2.1) |
| `acdc-bolt.svg` | AC/DC | settings-gear replacement **and** the inline call-sign splitter |
| `beatles-drum-gear.svg` | The Beatles | settings-gear replacement |
| `nirvana-smiley-gear.svg` | Nirvana | settings-gear replacement |
| `nin-spiral-gear.svg` | Nine Inch Nails | settings-gear replacement |
| `zeppelin-zoso.svg` | Led Zeppelin | genre line, mark 1 of 4 |
| `zeppelin-triquetra.svg` | Led Zeppelin | genre line, mark 2 of 4 |
| `zeppelin-rings.svg` | Led Zeppelin | genre line, mark 3 of 4 |
| `zeppelin-feather.svg` | Led Zeppelin | genre line, mark 4 of 4 |

`zeppelin-runes.json` holds the same four paths with their viewBoxes, for builds that would
rather read one file than four.

The horns carry their own red stroke and glow filter and are used as-is. The four gear
icons are authored on a 24×24 viewBox at a 26dp draw size and use `currentColor` — **tint
them per theme** (values in §2); a raw import renders black. The four Led Zeppelin runes are
traced outlines on their own viewBoxes and also take `currentColor`.

`assets/fan-l2.png` / `fan-r2.png` — AC/DC bolt art flanking the STEREO pill. Raster, fixed
colour, recoloured by filter in dark mode (§2.1).

---

## 2. Per-theme build cards

Each card lists exactly what to draw. Anything not listed keeps its default appearance.

### 2.1 AC/DC — `motif: acdc`

Match `names`: `ac dc`, `acdc`.

**Type.** Squealer on station name, call signs, preset labels, RadioText. `bold: true`,
RadioText tracking `2`.

**Horns — the element most likely to be got wrong. Build it exactly like this:**
- Two SVGs from `art/`, drawn as-is. Do **not** re-derive them, do not replace them with a
  font glyph, an emoji, or a Material icon.
- Each is 70×96 in its own space. Draw at that aspect, **overhanging the hero card's top
  corners**: left horn at `top: -44dp, left: -16dp`; right horn at `top: -44dp,
  right: -16dp`, relative to the hero card's border box.
- Non-interactive (`pointerEvents: none`), z-order above the card fill and below the badge.
- The left file already carries `rotate(-2.5°)` and the right `rotate(+3°)`. Do not add
  rotation on top.
- Stroke `#E31E24`, glow `drop-shadow(0 0 5px rgba(255,59,48,0.7))` — both baked into the
  files. Both are **fixed in light and dark**; the theme's `modes` palette does not touch them.
- They are deliberately crude — uneven weight, kinked joins, overshooting tips. That is the
  design, not an artifact. Reproduce it by shipping the file. §3 documents the generator if
  you must regenerate at another size.

**Reject the horns if:** they are symmetrical, they are a single even-weight stroke, they
sit inside the card, they are an emoji, or the tips are clipped by the card bounds.

**Bolt.** `art/acdc-bolt.svg`, three jobs:
1. Replaces the settings gear in the status bar. 26dp, fill `#E8A400`,
   `drop-shadow(0 1px 1px rgba(0,0,0,0.35))`.
2. Splits **every** call sign at its midpoint — `WIBA` → `WI`⚡`BA` (art, not the emoji).
   Inline size `0.74em × 1.05em`, baseline offset `-0.22em`, `0 0.04em` side margins,
   inherits the text colour.
3. On the dark (Back in Black) palette the hero call sign is filled in the card's own black,
   so the bolt inside it is explicitly filled `#C9C9C9` instead of inheriting.

**STEREO.** `assets/fan-l2.png` left, `fan-r2.png` right, flanking the pill. Slot 20×28dp
(wide) / 28×40dp (tall), art height 24dp / 34dp, `-2dp` outward inset, vertically centred.

**Genre line.** "High Voltage Rock 'n' Roll", pulsing `#E8A400` ⇄ `#FFE24A`.
Light adds a ring outline `#241B0E`, 1px. Logos suppressed.

**Dark = "Back in Black"** (arrives with the head unit's night theme, no separate trigger):
page `#24272C`; hero card bg `#0B0B0B`, border `#A2A2A2`, text `#E8E8E8`, sub `#7E7E7E`;
call sign + frequency filled `#0B0B0B` with a **1.1px `#C9C9C9` outline**; interactive
accent restated `#C9C9C9` / fill `rgba(201,201,201,0.15)` / on-accent `#0B0B0B`; RadioText
plate `#070707` bg, `#171717` border, `#E8E8E8` text; STEREO art filtered
`grayscale(1) brightness(2.3) contrast(0.85)`.

### 2.2 The Beatles — `motif: submarine`

Match: `beatles`. **Hero lettering is unfinished — see §4. Build everything else.**

BeatlesYellowSub throughout, SgtPeppers on the hero (`heroCase: lower`), MadieRoger on the
genre line with a drooping per-character baseline. Accent `#C9A227`, glow `#E8CF7A`.
Card `#F3E8D2` bg / `#A81F28` border / `#241608` text / `#6B4A2A` sub.
Drum-hoop frame: border 8dp plus concentric inset rings
`#F3E8D2 5px`, `#2E4EA0 6.5px`, `#F3E8D2 17px`, `#A81F28 18.5px`.
RadioText plate: white, `#DED6C6` border, `#1A1A1A` text, serial "No. 0101538".
Genre "Rock" in `#4A2C15`. Abbey-Road stripes on the ground plane. Logos suppressed.
Gear → `art/beatles-drum-gear.svg`, tint `#C9A227`.
**Margin-accent slots** are reserved on this motif — lower margins of the hero card, at
`right:-30, bottom:-10` (95% opacity) and `left:-24, bottom:-12` (90%), clear of the tell
strip above and the RadioText plate below. **No art ships in them yet**; leave them empty.

### 2.3 Led Zeppelin — `motif: runes`

Match: `led zeppelin`, `zeppelin`. **No palette changes at all** — default theme colours
throughout, exactly like Nirvana (§2.4). Accent = theme text, glow = theme border, chrome ink
= theme text. Type and marks only.

**Type.** Kashmir, but **scoped to the hero card and RadioText only** (`fontScope: 'hero'`) —
preset tiles and peek cards keep the default face. `heroScale 1.3`, `heroStroke 0.9`,
RadioText tracking 2. Logos suppressed.

**Runes — in place of the genre line.** The four symbols from the untitled fourth record, in
sleeve order: **ZoSo · triquetra · three rings · feather**. Draw `art/zeppelin-*.svg`; they are
specific marks, not lettering, and must not be redrawn or replaced with a font glyph.
- Height `1.45em` each, width from the file’s own aspect, laid in a row with `0.62em` gaps,
  centre-aligned on the line.
- ZoSo alone takes `translateY(0.07em)` and `margin-right: -0.2em` (its glyph sits high and
  wide in its box); the other three take no adjustment.
- Fill `currentColor` at `fill-opacity 0.82`, `fill-rule: evenodd`, so the genre line’s own
  colour still drives them.

**Debossed, not printed.** The marks are cut into the surface, using the same engraved
treatment as the disabled tells — four stacked drop-shadows: two hard 1px edges plus two soft
bloom layers that give the recess a wall rather than just a rim.

| layer | light | dark |
|---|---|---|
| `0 1px 0` hard, below | `rgba(255,255,255,1)` | `rgba(255,255,255,0.28)` |
| `0 -1px 0` hard, above | `rgba(0,0,0,0.45)` | `rgba(0,0,0,0.75)` |
| `0 2.5px 2px` soft, below | `rgba(255,255,255,0.85)` | `rgba(255,255,255,0.14)` |
| `0 -2px 2px` soft, above | `rgba(0,0,0,0.28)` | `rgba(0,0,0,0.55)` |

On Android this is a lossy element — see `LOSSY-ELEMENTS.md` #14.

**Airship — the vehicle-in-motion tell.** This theme replaces the **motion tell**, not the
settings gear; the gear keeps its default icon. A **rigid** airship (not a blimp): long cigar
hull with a blunt nose and tapered tail, three ring frames showing through, small swept tail
fins whose trailing edges run back to the tail point, and a gondola under the bow. Drawn
46×33 on a `0 0 34 24` viewBox, `fill: none`, `stroke: currentColor`, width 1.5 (ring
frames 1.0), round caps and joins. It keeps the motion slot’s **fixed amber safety colour**
and its slow ~2.6s pulse — a theme never recolours that slot. On the wide track it is lifted
`translateY(-6px)` to fly level with the satellite beside it; no lift on the tall track.

**Reject the airship if:** it is a smooth blimp with no ring frames, it is smaller than the
settings gear, it sits below the satellite’s centre line, or it is any colour but the amber
motion tell.

### 2.4 Nirvana — `motif: xerox`

Match: `nirvana`. **No palette changes at all** — default theme colours throughout; accent,
glow and chrome ink all take the current theme's text/border/dim tokens. Print effects only.

Permanent Marker throughout; **Onyx on the hero at 1.5×**. Off-register second impression
behind the call sign: `rgba(0,0,0,0.2)` at `+3dp, +3dp`, printed over the standard hero drop
shadow (never replacing it). Genre "Verse Chorus Verse" in Permanent Marker, `dim`.
RadioText tracking 1. Logos suppressed.
Gear → `art/nirvana-smiley-gear.svg`, tint = theme text colour. It is strict geometry — a
true circle, two symmetrical X eyes on a shared baseline, one quadratic mouth arc, a pill
tongue. **Nothing in this icon is hand-drawn or wobbly**; ship the file as-is.

### 2.5 Nine Inch Nails — `motif: spiral`

Match: `nine inch nails`. Default palette. Gridnik on station name, call signs, presets and
hero (`heroScale 1.3`, tracking 9dp); Singothic on the genre line, RadioText and **every**
frequency readout (`freqScale 0.95`). RadioText tracking 5.

Off-register ghost `rgba(0,0,0,0.16)` at `+2dp, 0`.

**Glitch** (`heroGlitch`) on every call sign: rebuild the text as three horizontal bands over
a hidden layout copy — top band true and solid, middle band shifted `+0.05em` at 42% opacity,
bottom band shifted `-0.035em` at 88%. Clip insets top/mid/bottom:
`inset(-5% -4% 60% -4%)`, `inset(38% -4% 33% -4%)`, `inset(65% -4% -5% -4%)`. Offsets are in
**em** so the same treatment holds from a 99sp hero down to a 13sp tile label.

Genre cross-fades "Broken Machines" ⇄ "Things Falling Apart", ~20s each on a 40s loop.
Logos suppressed. Gear → `art/nin-spiral-gear.svg`, tint = theme text colour.

---

## 3. Regenerating the procedural art (only if you must)

The horns and the NIN spiral are generated, not hand-drawn. **Prefer the shipped SVGs.** If
you need them at a different size, regenerate with this exact algorithm so the output is
identical — it is deterministic, seeded, and any deviation changes the character of the line.

```
nz(s)            = frac(sin(s * 12.9898) * 43758.5453) * 2 - 1
smoothNz(t, sd)  = lerp(nz(i + sd*7.1), nz(i+1 + sd*7.1), smoothstep(frac(f)))
                   where f = t*3.7 + sd, i = floor(f)
```

**Taper.** A single stroke cannot vary in width, so each line is 30 short overlapping
segments (round caps hide the joins). For segment `i`, at `t = (i + 0.5) / 30`:

```
base  = w0 + (w1 - w0) * t^0.78
width = max(0.35, base + bulge * sin(PI * t) + smoothNz(t, seed + 3.3) * 1.15 * (1 - t*0.5))
```

**Horn geometry** — cubic bezier `[p0 c1 c2 p3]`, then `w0, w1, bulge`; seeds 1.7 / 9.3, and
each successive line in a scrawl advances the seed by 5.7:

```
LEFT  (seed 1.7, rotate -2.5°)
  [22.5,90.5, 15,66, 10.5,34, 7.2,7.5]      5.6  1.3  0.40
  [5.6,3.6,   20,26, 46,45,   63.5,52.5]    1.4  5.4  0.35
RIGHT (seed 9.3, rotate +3°)
  [47.5,90.5, 54,68, 60,34, 62.6,7.2]       5.3  1.5  0.35
  [64.6,4.2,  50,26, 24,46, 6.5,52]         1.3  5.5  0.40
```

**Spiral** — 72 segments, total angle `4.7π`, on a 24×24 viewBox:

```
u = i/72;  th = 4.7π * (1 - u)
r = 1.0 + 9.3 * (th / 4.7π) + smoothNz(u * 2.6, 4.2) * 0.6
x = 12 + r*cos(th - 0.9);  y = 12.3 + 0.97 * r * sin(th - 0.9)
width = max(0.55, 1.15 + 0.75*sin((i/72)*PI) + smoothNz(i/72, 8.8) * 0.55)
```

---

## 4. Unfinished — The Beatles drum lettering

The Sgt-Pepper hero needs two lettering treatments: solid filled letters with a hard offset
shadow (call sign), and thin-outlined letters with an accent stripe following each stem
(frequency). The bundled `SgtPeppers.ttf` is an **outline-only cut**, so neither can be
produced from it with text effects. The CSS approximations in the prototype source
(`callLined`, `freqShadow`, the glyph-flood in `solidType1`) are **known-wrong placeholders —
do not port them**. Build the rest of the Beatles theme and leave the hero lettering plain
until the solid cut or hand-drawn SVG lettering is supplied.

---

## 5. Acceptance checklist

Per theme, all must pass:

- [ ] Call-sign letterforms match `screenshots/egg-<theme>.png` — **not** Roboto, not the
      default face.
- [ ] Every font in the theme's row of §1.1 is loaded from `res/font/`, none fetched.
- [ ] Theme art comes from `art/` — no emoji, no Material icon, no hand-redrawn substitute
      anywhere in the theme.
- [ ] Settings gear is replaced by the theme's icon, tinted per §2.
- [ ] Genre line shows the theme's string, in the theme's face and colour.
- [ ] Station logos are hidden (all five themes set `suppressLogos`).
- [ ] Colours match the card's values in §2 exactly, in **both** light and dark.
- [ ] Layout is byte-identical to the un-themed face: no control moved, resized or
      re-targeted; hit targets still ≥48dp.
- [ ] Theme reverts within one RDS update of the track changing.
- [ ] AC/DC only: horns overhang the hero's top corners, uneven and asymmetric, red with a
      glow; bolt splits every call sign; dark mode is Back in Black.
- [ ] Forced from the secret panel (six taps on the settings brand line), the theme renders
      the same as when auto-detected.
