//! Carnyx — radio for a NOWADA Android head unit.
//!
//! The UI is Slint; the logic is Rust. There is no JavaScript layer and no
//! desktop binary: the target is the head unit, and the host build exists only
//! so the code can be compiled and tested off-device.

slint::include_modules!();

/// Build the window and run the event loop. Shared by every entry point, so the
/// Android path and any future one cannot drift apart.
pub fn run() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;

    // Placeholders. The tuner replaces these — the NWD built-in first, then an
    // SDR backend behind the same interface.
    ui.set_frequency("101.5".into());
    ui.set_station("".into());
    ui.set_radio_text("".into());
    ui.set_stereo(false);

    ui.run()
}

/// Android entry point. `android-activity` calls this instead of `main`, so the
/// symbol name and the `no_mangle` are load-bearing — a rename is an app that
/// starts and immediately does nothing.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).expect("Slint Android backend failed to start");
    run().expect("event loop failed");
}
