# Carnyx

Radio for a NOWADA (NWD) Android head unit. Slint interface, Rust logic.

**The unit is 32-bit ARM** (`armeabi-v7a`). Every build has to produce that ABI;
an arm64-only APK will not install on it.

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

Everything below runs on your development machine. Nothing is ever compiled on
the head unit — the Android build is a cross-compile, and what you carry over is
an APK file.

`cargo build` compiles for the host. That is a compile check only: there is no
desktop application, and the head unit is the only target that matters.

### An APK to sideload

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi
cargo install cargo-apk

export ANDROID_HOME=~/Android/Sdk
export ANDROID_NDK_ROOT=~/Android/Sdk/ndk/<version>
export ANDROID_NDK="$ANDROID_NDK_ROOT"       # yes, a third one — see below
export JAVA_HOME=<android-studio>/jbr        # only if javac is not on PATH

cargo apk build --lib
```

`ANDROID_NDK` is not a typo for `ANDROID_NDK_ROOT`. cargo-apk reads the latter;
`skia-bindings` reads `ANDROID_NDK` and nothing else
(`build_support/platform/android.rs:69`), and panics with `ANDROID_NDK variable
not set` without it. Both are needed.

The APK lands at **`target/debug/apk/carnyx.apk`**, and goes to the unit on a USB
stick. (`cargo apk run` and `adb install` exist, but this unit is not on adb.)

**Check it before you carry it out:**

```sh
./tools/check-apk.sh
```

Without adb there is no install error to read — the unit says "App not
installed" and stops. That script checks, on the machine that built it, the
things which produce exactly that message: the `armeabi-v7a` library is present,
the manifest declares `android:exported` and a launchable activity, and the APK
is signed.

`--lib` because the crate is a `cdylib` with no `main`: the entry point is
`android_main`. No `--target` flag, on purpose — `build_targets` in `Cargo.toml`
lists both ABIs and **both are built every time**. The unit is 32-bit today, but
an APK is a poor place to discover months later that one architecture was never
compiled. `tools/check-apk.sh` fails if either is missing. No `cmdline-tools` are
needed.

### Skia comes along whether you want it or not

Slint's Android backend depends on `i-slint-renderer-skia` **non-optionally** —
`i-slint-backend-android-activity`'s manifest declares it under
`[target.'cfg(target_os = "android")'.dependencies]` with no feature guarding it.
So an Android build always pulls `skia-bindings`, which first tries to download a
prebuilt keyed by `<rust-skia-hash>-<target-triple>-<features>` and, failing
that, compiles Skia from source.

There is no armv7 prebuilt for skia-bindings 0.99 — the download 404s. Since the
head unit is 32-bit, that is not avoidable by dropping the ABI: **every build for
this device compiles Skia from source.** The published `skia-bindings` crate is
2.3 MB and contains no Skia tree, so the build fetches one.

**Install the source-build tools first.** Both of these fail LATE — `ninja` after
GN has generated, `libclang` after the entire Skia tree has compiled — so a
missing one costs the whole build before it says anything:

```sh
sudo apt install ninja-build libclang-dev   # Debian/Ubuntu/Pop!_OS
```

`ninja` builds Skia. `libclang` is what bindgen loads to read Skia's headers, and
without it the build script panics at the very last step:

```
Unable to find libclang: "couldn't find any valid shared libraries matching:
['libclang.so', ...], set the `LIBCLANG_PATH` environment variable"
```

The NDK ships its own copy if you would rather not install the distro package:

```sh
export LIBCLANG_PATH=$(dirname "$(find "$ANDROID_NDK/toolchains/llvm/prebuilt/linux-x86_64" \
  -name 'libclang.so*' | head -1)")
```

`git` and `python3` are needed too, and are usually already there — skia-bindings
probes for `python`/`python3` and syncs the Skia tree with git before it builds.
`gn` comes with that sync; only `ninja` has to be on PATH yourself.
`SKIA_NINJA_COMMAND` and `SKIA_GN_COMMAND` override both if they live somewhere
unusual.

**Changing `min_sdk_version` throws the Skia build away.** cargo-apk turns it into
`RUSTFLAGS=-Clink-arg=--target=armv7a-linux-androideabi<n>`, and RUSTFLAGS is part
of the fingerprint cargo hashes into the build directory name — so the whole
`skia-bindings-<hash>/out/skia` tree is a different directory and all 1158 targets
compile again. Environment variables that are not RUSTFLAGS (`LIBCLANG_PATH`,
`ANDROID_NDK`) do not have this effect; ninja finds its output up to date and only
the last steps re-run.

Skia is resolved **per ABI**, and neither of ours downloads: both the armv7 and
the aarch64 prebuilt URLs 404 at 0.99.0 (checked). **A full both-ABI build is two
Skia builds from source**, roughly an hour each, and both are cached in `target/`
afterwards.

`--target armv7-linux-androideabi` narrows a build to the one ABI the unit runs.
That is for iterating when a full build is in the way, not for anything that
leaves the machine: what goes on the flash drive should have both.

### Debug APKs are not representative

A debug APK is around **225 MB per ABI**, and the reason is not just symbols:
cargo passes `OPT_LEVEL=0` through to Skia's `extra_cflags`, so the entire Skia
tree is compiled `-O0`. It installs and it draws, which makes it a fine smoke
test, but **anything it suggests about frame rate is meaningless**. Judge
performance on a release build or not at all.

Release needs the keystore, and fails loudly without it rather than falling back
to the debug key — `cargo-apk` returns `MissingReleaseKey` unless both
`CARGO_APK_RELEASE_KEYSTORE` and `CARGO_APK_RELEASE_KEYSTORE_PASSWORD` are set
(`cargo-apk/src/apk.rs:272-300`). The debug-key fallback exists only for the
`dev` profile.

```sh
export CARGO_APK_RELEASE_KEYSTORE=~/keys/carnyx-release.keystore
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=...
cargo apk build --lib --release        # -> target/release/apk/carnyx.apk
```

**skia-bindings 0.99 does not build against a recent NDK.** Skia's own toolchain
picks a VERSIONED compiler wrapper — `armv7a-linux-androideabi26-clang++`, which
already carries its target — and then skia-bindings appends an UNVERSIONED
`--target=armv7-linux-androideabi` on top of it. A modern sysroot refuses that:

```
sysroot/usr/include/sys/cdefs.h:365:2: error: Unversioned target triples are not supported!
```

The flag is emitted in TWO places, and both are unconditional:

- `build_support/platform.rs:156-159`, in `GnArgsBuilder::into_gn_args()`, which
  pushes `--target=<triple>` into both `extra_cflags` and `extra_asmflags`. This
  is the one that reaches the C++ compile and produces the error above.
- `build_support/platform/android.rs:131`, in `additional_clang_args()`, which
  feeds bindgen only.

Neither can be configured away. `SKIA_GN_ARGS` is appended after the generated
args (`build_support/skia/config.rs:326`) so it could re-assign the C++ flags,
but bindgen has no such escape: `BindgenArgsBuilder::set_target_override`
(`platform.rs:252`) exists and is never called by anything in the crate.

The `#error` tests `__ANDROID_MIN_SDK_VERSION__`, which Clang defines only from
a versioned triple suffix — `-D__ANDROID_API__=26` cannot substitute for it,
because that macro is an alias of the one being tested.

**Use NDK r27 LTS.** The `#error` was added to bionic in November 2025 and first
ships in r30 (30.0.15729638 is r30 Beta 2, which is what produced the failure
above), but unversioned triples were already silently mis-compiling in r28 and
r29 — see android/ndk#2206. r27 is the last release where this is sound.

Any r27 patch works; the whole line predates the bionic change, and r27d
(27.3.13750724) shipped 15 July 2025, four months before it landed. Install
**27.3.13750724** unless CarFM has already put one on the machine — React Native
0.86 pins 27.1.12297006 (`node_modules/react-native/gradle/libs.versions.toml`),
which is equally sound and saves a 660 MB download. NDKs sit side by side; both `ANDROID_NDK_ROOT` and `ANDROID_NDK` have
to point at the one you want.

**Confirmed on r27.3.13750724: Skia itself builds for armv7.** All 1156 ninja
targets compile and `libskia.a` links, from source, with no unversioned-triple
error anywhere. Despite the paragraph below, 32-bit ARM Android is not broken in
skia-bindings 0.99 — it is only unshipped.

**Then set `min_sdk_version = 26`.** After Skia links, skia-bindings compiles
four binding files of its own (`bindings.cpp`, `gl.cpp`, `vulkan.cpp`,
`gpu.cpp`) through cc-rs rather than GN, and those failed:

```
sysroot/usr/include/c++/v1/locale:776:10: error: 'strtof_l' is unavailable: introduced in Android 26
sysroot/usr/include/c++/v1/locale:781:10: error: 'strtod_l' is unavailable: introduced in Android 26
```

Three `--target` flags reach that compiler line and the LAST one wins:
cc-rs contributes `armv7-none-linux-android`, skia-bindings adds its unversioned
`armv7-linux-androideabi`, and cargo-apk appends
`CXXFLAGS=--target=armv7a-linux-androideabi23` — built from `min_sdk_version` in
`Cargo.toml`. So Skia compiled at API 26 (skia-bindings hardcodes `ndk_api=26`)
while its own bindings compiled at API 23, and libc++'s `<locale>` needs
`strtof_l`/`strtod_l`, which bionic marks `__INTRODUCED_IN(26)`.

`-D__ANDROID_API__=26` is on that line too and does not help — it draws only a
macro-redefinition warning, because the availability attributes read
`__ANDROID_MIN_SDK_VERSION__` from the target triple, not `__ANDROID_API__`.

The floor is skia-bindings', not ours: API 26 is hardcoded at
`build_support/platform/android.rs:10`. Matching it means the APK will not
install below Android 8.0. **If the head unit is older than Android 8.0, Skia is
not available to this project at all** and the way forward is FemtoVG, below,
which carries no such floor.

The paragraph below is why armv7 was expected to fail, and it is still true of
what upstream ships — it was simply not what stopped the build.
**rust-skia has never supported 32-bit ARM Android** — not dropped, never added.
Its README lists `aarch64-linux-android` and `x86_64-linux-android` only, there
is no armv7 prebuilt at any version, and rust-skia#850 has been open since
October 2023 asking for it.

**It works anyway.** The APK installed on the head unit and the face rendered
correctly — so 32-bit ARM Android is unsupported by rust-skia in the sense that
nobody publishes a binary or promises it keeps working, not in the sense that it
does not work. Slint on Skia on armv7 Android is a real, running configuration on
this hardware. The install also settles the API 26 floor from the other
direction: the unit accepted the package, so it is Android 8.0 or newer.

What "unsupported" costs us is a version pin, not a rewrite. Nothing in CI
upstream tests this target, so a future skia-bindings could break it without
anyone noticing; that is an argument for changing the Skia version deliberately
rather than for avoiding it.

The two Skia-free routes below are therefore contingency, not plan. Keep them
because the reasoning is expensive to rebuild, not because anything currently
needs them.

Before a first Android build it is worth checking whether the arm64 prebuilt
exists, because the answer is the difference between a download and an hour:

```sh
curl -sSI -o /dev/null -w '%{http_code}\n' -L \
  https://github.com/rust-skia/skia-binaries/releases/download/0.99.0/skia-binaries-a25a0fdb7d90429aa2d1-aarch64-linux-android-gl-jpegd-jpege-pdf-vulkan.tar.gz
```

`200` means it downloads. `404` means Skia builds from source: budget the time,
and make sure `python3` is installed. skia-bindings' own version gates are
`>= 22` and `>= 23` with no upper bound, so it will happily *start* on any recent
NDK — the failure above comes from the sysroot, not from a version check.

### No Skia at all (contingency, not the current plan)

Skia now builds and runs on the unit, so nothing below is needed today. It is
kept because the research behind it was expensive and the situation that would
call for it — a skia-bindings release that drops armv7, or an NDK requirement we
cannot meet — is one bad upgrade away.

Every problem in the section above comes from one place: Slint's `android-activity`
backend hard-depends on the Skia renderer. Nothing here needs Skia. The face is
text, rectangles and a few paths, on a 32-bit head unit.

Dropping Skia does NOT mean dropping the GPU. Slint's third renderer, FemtoVG, is
pure Rust over OpenGL ES — `femtovg` and `glow`, no C++ anywhere — and unlike the
software renderer it implements `draw_box_shadow` and radius-aware `combine_clip`
properly, so nothing on this face renders differently.

Upstream excludes it from Android, but as wiring, not as a limitation:
`i-slint-renderer-femtovg` is declared under
`[target.'cfg(not(target_os = "android"))'.dependencies]` in both `slint`
(Cargo.toml:353) and `i-slint-backend-selector` (Cargo.toml:154), so the feature
simply is not offered there. Depending on the renderer crate directly sidesteps
that, and it works — checked, not assumed:

```sh
rustup target add armv7-linux-androideabi
# with i-slint-renderer-femtovg = { version = "=1.17.1", features = ["opengl"] }
# under [target.'cfg(target_os = "android")'.dependencies]:
cargo check --target armv7-linux-androideabi
```

`i-slint-renderer-femtovg` 1.17.1, `femtovg` 0.25.1 and `glow` 0.17.0 all compile
clean for `armv7-linux-androideabi`, with no `skia-bindings` anywhere in the
dependency graph. A custom `WindowAdapter` returning `&FemtoVGOpenGLRenderer` as
its `&dyn Renderer` type-checks: version pinning puts `slint` and the renderer on
the same `i-slint-core`, so the sealed `Renderer` trait unifies.

What is NOT yet proven is the EGL side. `FemtoVGRenderer::new` takes an
`impl OpenGLInterface` — the caller supplies the context — so this route needs an
EGL context and surface built against the `ANativeWindow` that `android-activity`
hands over, plus correct teardown and recreation across suspend/resume. That is
ordinary Android GL plumbing, but it is the part that has to be written and it
has not been.

Slint's software renderer is already driven directly by `examples/shot.rs`, which
renders every layout track headlessly through a custom `Platform`. The same shape
on Android — `android-activity` for the window and events, `MinimalSoftwareWindow`
for the drawing — drops `i-slint-backend-android-activity`, and with it Skia, the
C++ toolchain, the NDK version constraint and most of the APK's size.
`i-slint-backend-android-activity` is an OPTIONAL dependency of `slint`, pulled
in only by the `backend-android-activity-06` feature, so not enabling that
feature is all it takes to keep skia-bindings out of the build entirely.
Upstream's own adapter is 1019 lines and touches the renderer in exactly two
places; the 747-line Java/IME helper beside it is for text input, which this face
does not have.

**Every screenshot in `shots/` is already software-rendered.** `examples/shot.rs`
is the only way this face has ever been looked at, so the face that has been
reviewed and signed off IS the software-rendered face — the software renderer is
the known quantity here and Skia is the one that has never run.

What the software renderer actually costs, measured against
`i-slint-renderer-software` 1.17.1 rather than against the documentation:

- **Drop shadows are not drawn.** `draw_box_shadow` (lib.rs:3088) has an empty
  body and a `// TODO`. This is real and visible: `ui/hero.slint:198-200` puts a
  22px/14px `#0000002E` shadow under the hero card, and it is absent from every
  shot. It is also fixable without Skia — a pre-blurred PNG behind the card via
  `@image-url("...", nine-slice(...))`, which the software renderer does support
  (lib.rs:2697).
- **Clipping ignores the corner radius.** `combine_clip` (lib.rs:3097)
  intersects rectangles and carries a `// TODO: handle radius and border`. Five
  sites set `clip: true` with a radius — `ui/presets.slint:22,45,411` and
  `ui/hero.slint:44,145` — but in all five the rounded *background* is drawn by
  the Rectangle itself, which does honour the radius, and the clipped children
  are inset from the corners. No visible difference in any current shot.
- **Text stroking works.** This was previously listed here as a loss and that was
  wrong. `platform_text_stroke_brush` is implemented (lib.rs:3306) and the shared
  parley text path calls it (`i-slint-core/textlayout/sharedparley.rs:1229`); the
  halo on the signal readout (`ui/status-bar.slint:98-100`) is visible in
  `shots/head-unit-light.png` under magnification.
- **Path fill and stroke work**, behind the `path` feature, which `std` enables
  by default. Nothing in `ui/icons.slint` is affected.

Not done, and not a small change: roughly 200-300 lines of window and event
plumbing, unproven on the device until it is flashed.

### "App not installed"

That is the on-device installer refusing the package, and its dialog never says
why. With the unit on adb, `adb install -r <apk>` prints the real code; loading
from a stick, there is no such luxury — so run `./tools/check-apk.sh` first and
carry out only an APK that passes.

Causes, in the order worth checking:

- **The file manager lacks install permission.** Android requires "Install
  unknown apps" per app, granted to whatever browses the stick. Nothing about the
  APK will fix this one.
- **A truncated copy.** `md5sum` on both sides before unmounting.

And the two this project can produce, both of which `check-apk.sh` catches:

- **`INSTALL_FAILED_NO_MATCHING_ABIS`** — the APK has no native library for the
  unit's CPU. The unit is 32-bit, so `armeabi-v7a` is the one that decides
  whether it installs at all:

  ```sh
  unzip -l target/debug/apk/carnyx.apk | grep 'lib/'
  ```

  A build narrowed with `--target aarch64-linux-android` produces exactly this,
  which is why narrowing is for iterating only.

- **`INSTALL_PARSE_FAILED_MANIFEST_MALFORMED`** — almost certainly a missing
  `android:exported`. Android 12 refuses any package whose targetSdk is 31+ when
  a component with an intent-filter does not declare it, and cargo-apk defaults
  `Activity::exported` to `None` while always serialising a MAIN/LAUNCHER filter.
  `Cargo.toml` now sets `exported = true`; if you are building an older checkout,
  that is the fix.

To read back what the APK actually declares, rather than what `Cargo.toml` asked
for: `aapt dump badging target/debug/apk/carnyx.apk`.

### Release builds need a keystore

`cargo apk build --lib --release` **fails** with `MissingReleaseKey` unless a key
is configured. The auto-generated `~/.android/debug.keystore` (password
`android`) is dev-profile only.

**Keep the keystore outside this repository.** Not in it and gitignored —
outside it. A key inside the working tree survives only as long as the ignore
rules stay right, and there is no undo for a signing key that reaches a remote.

Generate one, once, wherever you keep secrets:

```sh
mkdir -p ~/.keys
keytool -genkeypair -v \
  -keystore ~/.keys/carnyx-release.jks \
  -storetype PKCS12 -alias carnyx \
  -keyalg RSA -keysize 4096 -validity 10000 \
  -storepass "$PASS" -keypass "$PASS"
```

**One key, one password**, and that is not a style preference: cargo-apk shells
out to `apksigner sign --ks <path> --ks-pass pass:<password>` and passes neither
`--ks-key-alias` nor `--key-pass` (`ndk-build::apk::UnsignedApk::sign`). A
keystore holding several aliases, or one whose key password differs from its
store password, cannot be used this way. PKCS12 with both passwords set the same
is the shape that fits.

Then build with the path and password in the environment:

```sh
CARGO_APK_RELEASE_KEYSTORE=~/.keys/carnyx-release.jks \
CARGO_APK_RELEASE_KEYSTORE_PASSWORD="$PASS" \
cargo apk build --lib --release
# -> target/release/apk/carnyx.apk
```

cargo-apk also accepts a `Cargo.toml` stanza:

```toml
[package.metadata.android.signing.release]
path = "release.keystore"        # relative to the crate root, or absolute
keystore_password = "..."
```

**Do not use it.** `keystore_password` is not optional in that struct, so the
stanza puts a plaintext password in a tracked file. The environment variables
keep both the path and the password out of the repository entirely.

`*.keystore` and `*.jks` are gitignored as a backstop, not as the plan.

### Two cargo-apk traps

Both read out of its source rather than guessed:

- **It shells out to `aapt`, not `aapt2`** (`ndk-build::apk`). Recent build-tools
  packages ship only `aapt2`. Check with `ls $ANDROID_HOME/build-tools/*/aapt`;
  if nothing comes back, install an older build-tools alongside the current one.
- **It picks the HIGHEST build-tools version it finds**, by `max()` over the
  directory names (`ndk-build::ndk::Ndk::from_env`) — so installing an older one
  alongside is not enough on its own if the newest lacks `aapt`.

`xbuild` (`cargo install xbuild`) is the alternative that uses `aapt2`. It has
not been tried here.

**Untested path.** No APK has ever been built: this repository has been developed
in a container with no SDK and no NDK. The `[package.metadata.android]` block is
schema-checked against cargo-apk and accepted, and the paths and failure modes
above are read from its source, but nothing past manifest parsing has been run.
Expect to iterate on the first attempt.

## Licence

GPL-3.0-only.

Atkinson Hyperlegible (`ui/fonts/`) is © Braille Institute of America, under the
SIL Open Font License 1.1.
