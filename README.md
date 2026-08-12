# Carnyx

Radio for a NOWADA (NWD) Android head unit. Slint interface, Rust logic.

The successor to CarFM, which is React Native and is being retired. Carnyx is
not a port of that codebase — it is a rebuild that salvages the parts worth
keeping: the RDS decoder, the RBDS station identity and geo maths, and the
signal-meter model, all of which already exist as pure Rust.

## Order of work

1. **The interface.** Slint, on the head unit, with placeholder data.
2. **The NWD tuner.** The head unit's built-in FM chip, reached through the
   vendor's `com.nwd.radio.service`.
3. **An SDR tuner** behind the same interface, so the app is not tied to one
   piece of hardware.

Logic comes across from CarFM **when a screen needs it**, never ahead of that.
The first attempt ported module after module without a caller and ended up with
two implementations of everything and nothing retired.

## The face

The face is meant to be visually identical to the one running on the head unit
today, so the reference is **CarFM's shipped code**, not the design bundle in
`docs/design/handoff` — the bundle is v1.10.0 against a v1.14.x face and differs
on real values (`amberFill` is 0.12/0.15 there and 0.08/0.10 in the app).
Sources, in the order they were worked from:

| Carnyx | CarFM |
|---|---|
| `ui/tokens.slint` | `src/components/carfm/tokens.ts` + the `L` ramp in `CarFmFace.tsx` |
| `ui/icons.slint` | `src/components/carfm/icons.tsx` |
| `ui/status-bar.slint` | `CarFmFace.tsx` header |
| `ui/hero.slint` | `CarFmFace.tsx` hero band + `SidePresetCard.tsx` |
| `ui/presets.slint` | `PresetsBand.tsx` + `LogoTile.tsx` |
| `ui/face.slint` | `CarFmFace.tsx` composition |
| `src/station.rs` | `tokens.ts` (`brandColor`, `cleanCall`) |

Sizes are logical pixels, which Slint maps to Android dp, so the spec's dp
numbers transfer 1:1. Layout is real reflow per surface, never a fixed canvas
scaled to fit — see ANDROID §0 in the CarFM design bundle.

Three things JavaScript computed at runtime are baked out by
`tools/gen-icon-paths.py`, because Slint paths have no loops, no trig and no
stroke-dasharray: the signal glyph's dotted lossy arcs, the nearby magnifier's
barrel-warped tower, and the GPS satellite's rotation.

## Looking at it

There is no desktop application, but the face still has to be inspected without
a car:

```sh
cargo run --example shot     # writes shots/*.png for every layout track
```

That drives Slint's software renderer headlessly — no window system, no GPU —
at each of the five surfaces the design has to support, plus the states worth a
second look (audio released, tuner error, driving, lossy reception, no presets).

## Building

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi
cargo install cargo-apk
export ANDROID_HOME=~/Android/Sdk
export ANDROID_NDK_ROOT=~/Android/Sdk/ndk/<version>
cargo apk run --target aarch64-linux-android --lib
```

`cargo build` compiles for the host. That is a compile check only — there is no
desktop application, and the head unit is the only target that matters.

## Licence

GPL-3.0-only.

Atkinson Hyperlegible (`ui/fonts/`) is © Braille Institute of America, under the
SIL Open Font License 1.1.
