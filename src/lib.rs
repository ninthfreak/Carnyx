//! Carnyx — radio for a NOWADA (NWD) Android head unit.
//!
//! The interface is Slint and the logic is Rust. There is no desktop
//! application: `cargo build` on the host is a compile check, and the screenshot
//! example is how the face is inspected without a car.

slint::include_modules!();

pub mod android;
pub mod app;
pub mod fake;
pub mod logos;
pub mod rds;
pub mod settings;
pub mod signal;
pub mod station;
pub mod stations;

/// Build the window and drive it from the real services.
///
/// The window and the `App` are returned together because the `App` owns every
/// callback: drop it and the face goes inert while still drawing. The caller
/// keeps both alive for as long as the window is up.
pub fn build() -> Result<(AppWindow, std::rc::Rc<app::App>), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let driver = app::App::new(&ui, &app::host_db_path());
    Ok((ui, driver))
}

pub fn run() -> Result<(), slint::PlatformError> {
    let (ui, _driver) = build()?;
    ui.run()
}

/// Android entry point. `cargo-apk` calls this; there is no `main`.
///
/// UNTESTED, AND UNTESTABLE HERE. There is no device in this container, so
/// nothing below this line has ever executed. The tuner is deliberately NOT
/// unwrapped: a unit without the vendor service is a normal state and the face
/// reports it, rather than the process dying on the driver's dashboard.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android_app: slint::android::AndroidApp) {
    use std::io::Read;

    // Both of these must be taken BEFORE `init`, which consumes the `AndroidApp`.
    let files_dir = android_app.internal_data_path();
    let assets = android_app.asset_manager();
    slint::android::init(android_app).unwrap();

    let db_path = files_dir
        .map(|dir| {
            // Extracted once rather than read in place: a release APK DEFLATES
            // the asset, and a deflated asset has no file descriptor to hand
            // SQLite. See `stations::install`.
            stations::install(&dir, || {
                let name = std::ffi::CString::new(format!("db/{}", stations::DB_FILE)).unwrap();
                let mut asset = assets
                    .open(&name)
                    .ok_or_else(|| std::io::Error::other("db/stations.sqlite is not in the APK"))?;
                let mut bytes = Vec::new();
                asset.read_to_end(&mut bytes)?;
                Ok(bytes)
            })
            .unwrap_or_else(|_| dir.join(stations::DB_FILE))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(stations::DB_FILE));

    let ui = AppWindow::new().unwrap();
    let _driver = app::App::new(&ui, &db_path);
    // Tuner events arrive on binder and pump threads. The hop back to the UI
    // thread is `invoke_from_event_loop`; the drain itself reads the App out of
    // a thread-local, because `Rc<App>` cannot cross a `Send` boundary.
    app::set_event_wake(|| {
        let _ = slint::invoke_from_event_loop(app::drain_current);
    });
    ui.run().unwrap();
}
