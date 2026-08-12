//! Carnyx — radio for a NOWADA (NWD) Android head unit.
//!
//! The interface is Slint and the logic is Rust. There is no desktop
//! application: `cargo build` on the host is a compile check, and the screenshot
//! example is how the face is inspected without a car.

slint::include_modules!();

pub mod demo;
pub mod station;

/// Build the window and fill it with placeholder dial state.
pub fn build() -> Result<AppWindow, slint::PlatformError> {
    let ui = AppWindow::new()?;
    demo::install(&ui);
    Ok(ui)
}

pub fn run() -> Result<(), slint::PlatformError> {
    build()?.run()
}

/// Android entry point. `cargo-apk` calls this; there is no `main`.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).unwrap();
    run().unwrap();
}
