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

**Install the source-build tools first**, or the build gets as far as generating
the ninja files and then dies:

```sh
sudo apt install ninja-build      # Debian/Ubuntu/Pop!_OS; the binary is `ninja`
```

`git` and `python3` are needed too, and are usually already there — skia-bindings
probes for `python`/`python3` and syncs the Skia tree with git before it builds.
`gn` comes with that sync; only `ninja` has to be on PATH yourself.
`SKIA_NINJA_COMMAND` and `SKIA_GN_COMMAND` override both if they live somewhere
unusual.

Skia is resolved **per ABI**, so building both means two answers: armv7 has no
prebuilt and compiles from source, and arm64 may or may not — the curl above,
with the triple swapped, says which. Both are cached in `target/` afterwards.

`--target armv7-linux-androideabi` narrows a build to the one ABI the unit runs.
That is for iterating when a full build is in the way, not for anything that
leaves the machine: what goes on the flash drive should have both.

If the Skia build fails on a very recent NDK, the first thing to try is an older
one — 27.x is what React Native pins for CarFM, and NDKs install side by side.
Point both `ANDROID_NDK_ROOT` and `ANDROID_NDK` at it.

Before a first Android build it is worth checking whether the arm64 prebuilt
exists, because the answer is the difference between a download and an hour:

```sh
curl -sSI -o /dev/null -w '%{http_code}\n' -L \
  https://github.com/rust-skia/skia-binaries/releases/download/0.99.0/skia-binaries-a25a0fdb7d90429aa2d1-aarch64-linux-android-gl-jpegd-jpege-pdf-vulkan.tar.gz
```

`200` means it downloads. `404` means Skia builds from source: budget the time,
and make sure `python3` is installed. NDK 30 is fine for that path —
skia-bindings' version gates are `>= 22` and `>= 23` with no upper bound.

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
