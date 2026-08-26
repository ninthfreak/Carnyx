# Carnyx — Rust + Slint architecture notes

*Bundle v2.0.0 — 2026-08-25.*

Carnyx is a fork of CarFM: **mostly Rust**, **Slint** for the UI, **Android now**, a possible
**Linux** build later. This file covers how the design maps onto Slint and Rust.
`IMPLEMENTATION-SPEC.md` remains authoritative for *what* to build (surfaces, tracks,
tokens, structure); `LOSSY-ELEMENTS.md` covers the elements that need a specific Slint
technique.

**Carnyx does not run on DuduOS.** The `Dudu7` surface names are screen geometries the
design must survive, not the OS. Don't put DuduOS in the code, the UI copy, or the docs.

## 1. Split

- **Rust core** owns tuner state (current frequency, presets, RDS/RadioText, signal, stereo,
  HD, TA/TP/AF flags), station metadata, GPS + FCC nearby data, persistence, and audio
  priority. It is the only place with I/O.
- **Slint UI** is stateless with respect to the radio: it renders declared properties and
  raises callbacks. No tuner logic in `.slint`.
- The prototype mirrors this split already — `CarFmLive` is the core, `RadioFace` is the
  view. Read them that way.

## 2. Suggested file layout

```
ui/
  main.slint            root window; owns theme + track, hosts face and overlays
  theme.slint           global singleton: colour tokens + type ramp (§3 of the spec)
  radio_face.slint      the face: status bar, hero, RadioText, preset band
  hero_card.slint       hero + peek cards (one component, sized by slot — see LOSSY #9)
  preset_tile.slint     preset tile: logo plate or call-sign box
  tune_overlay.slint    two tabs: nearby list + frequency keypad
  settings.slint        settings overlay
  band_theme.slint      per-theme colours/fonts/props (the EGGS registry)
src/
  main.rs               window setup, platform init, wiring
  tuner/                SDR + RDS
  stations/             FCC data, nearby search, logos
  model.rs              structs shared with the UI
build.rs                slint-build compiles ui/main.slint
```

## 3. Theming

Put the light and dark token sets in a Slint **global singleton** and switch one property:

```slint
export global Theme {
    in property <bool> dark;
    out property <color> page:  root.dark ? #24272C : #DDE3EC;
    out property <color> panel: root.dark ? #33373D : #FFFFFF;
    // …one line per token in IMPLEMENTATION-SPEC.md §3
}
```

Every component reads `Theme.page` etc. Do not copy hexes into components — the dark ramp has
already been re-tuned once across every surface, and it will move again.

**Band themes** are a second, independent layer: a struct per theme (accent, glow, fonts,
motif flag, genre text, hero-prop selection) chosen by matching the RDS artist string. Keep it
a data registry exactly as `RadioFace`'s `EGGS` is, not a branch per theme in the markup.
`EASTER-EGGS-BUILD.md` is the authoritative per-theme spec.

## 4. Interface sketch

The face component's properties/callbacks, mirroring `RadioFace`'s props:

```slint
export component RadioFace {
    in property <string> freq;            in property <string> callsign;
    in property <string> radio-text;      in property <string> genre;
    in property <int>    signal-level;    in property <bool>   stereo;
    in property <bool>   gps-locked;      in property <bool>   in-motion;
    in property <bool>   audio-priority;  in property <bool>   tuner-error;
    in property <[PresetTile]> presets;   in property <int>    active-preset;
    in property <BandTheme>    theme-egg;
    callback tune(string);        callback step-preset(int);
    callback save-preset();       callback open-nearby();
    callback open-settings();     callback toggle-audio-priority();
}
```

Derive the track inside the component — `property <bool> tall: root.width / root.height < 1.0;`
— and never latch it at startup (§2 of the spec).

## 5. Platform

- **Android:** Slint's Android backend (`android-activity`). Feed the system **text-scale
  factor** into a global and multiply the type ramp by it — the spec bans freezing type size
  (§0). Keep touch targets ≥48 logical px. Test inside a vertical-third window, not just
  full-screen.
- **Linux (later):** the same `.slint` runs on the desktop backend, and on a kiosk unit via
  `linuxkms`. Nothing in this design needs a platform-specific layout — if you find yourself
  writing one, the track logic is wrong.
- **Fonts:** register the band-theme faces from `fonts/` at startup (or embed them at build
  time). A system font standing in for a theme face is a failed build.
- **Assets:** the SVGs in `art/` render directly via `Image { source: @image-url(…) }`.

## 6. Verifying

Run the loop in `CORRECTION-LOOP.md` — render, screenshot, diff against
`screenshots/`, fix to the picture. A desktop `cargo run` window resized to each reference
aspect is a legitimate way to shoot most surfaces; use a device for text-scale, touch targets,
and the vertical-third window.
