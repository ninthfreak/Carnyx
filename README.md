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
