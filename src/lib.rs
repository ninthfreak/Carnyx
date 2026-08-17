//! Carnyx — radio for a NOWADA (NWD) Android head unit.
//!
//! The interface is Slint and the logic is Rust. There is no desktop
//! application: `cargo build` on the host is a compile check, and the screenshot
//! example is how the face is inspected without a car.

slint::include_modules!();

pub mod android;
pub mod app;
pub mod callsigns;
pub mod fake;
pub mod logos;
pub mod prefs;
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
    build_with_prefs(&app::host_prefs_dir())
}

/// The same, against a preference directory of the caller's choosing.
///
/// THE SCREENSHOT HARNESS NEEDS THIS. Every shot builds a whole App, and an App
/// both READS and WRITES `prefs.json` — so with one shared directory a shot that
/// saves a preset changes what every later shot starts from. That is not a
/// theory: `many-presets` saved eighteen dials, and the portrait render of the
/// same state then loaded all eighteen and TOGGLED THEM BACK OFF, because
/// `toggle_save` is a toggle. The second shot rendered six presets and an unsaved
/// hero while claiming to show a long strip.
///
/// A per-shot directory makes each render independent of the ones before it,
/// which is what a screenshot is supposed to be.
pub fn build_with_prefs(
    prefs_dir: &std::path::Path,
) -> Result<(AppWindow, std::rc::Rc<app::App>), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let driver = app::App::with_tuner(
        &ui,
        &app::host_db_path(),
        prefs_dir,
        Box::new(android::FakeTuner::new()),
        false,
        None,
        fake::FakeLocation::default(),
    );
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

    // All four must be taken BEFORE `init`, which consumes the `AndroidApp`.
    let files_dir = android_app.internal_data_path();
    let assets = android_app.asset_manager();
    let vm = android_app.vm_as_ptr();
    let activity = android_app.activity_as_ptr();
    slint::android::init(android_app).unwrap();

    // The app's private data directory, and the prefs file's home. Captured
    // before the map below consumes `files_dir`.
    let files_dir_for_prefs = files_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("."));

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

    // The vendor radio, or an honest stand-in.
    //
    // What can fail HERE is `init` — the dex load and the JNI wiring — not the
    // bind. A unit that simply has no NWD service (the FYT/DuduOS ones) takes
    // the Ok arm: `init` succeeds, and it is `connect` that fails later, which
    // `NwdTuner::is_available` then reports honestly on its own.
    //
    // The fallback uses `unavailable()` rather than `new()` for one reason: the
    // settings panel derives its status from `Tuner::is_available`, and the
    // default fake answers yes. Handing it a yes-saying fake made the panel
    // print "Connected · Built-in hardware · NWD/NOWADA FM tuner" over a
    // simulation — a positive lie, and worse than saying nothing.
    //
    // Falling back at all rather than panicking is deliberate: a panic here is a
    // dead screen on a dashboard.
    //
    // SAFETY: `vm` and `activity` came from `AndroidApp::vm_as_ptr` and
    // `activity_as_ptr` a few lines above, which is exactly what `init`
    // documents. The activity outlives this call — `ui.run()` below is what
    // ends the process.
    let (tuner, tuner_is_real): (Box<dyn android::Tuner>, bool) =
        match unsafe { android::init(vm, activity) } {
            Ok(nwd) => (Box::new(nwd), true),
            Err(_) => (Box::new(android::FakeTuner::unavailable()), false),
        };

    // HTTPS and image decoding, both the platform's. `init` loads the class out
    // of the embedded dex; there are no natives to register, because every call
    // is Rust asking Java a question and waiting for the answer.
    //
    // `None` on failure, and that is a real state rather than a stumble: the app
    // then behaves exactly as the host does — the logo window opens, the search
    // reports that nothing was found, and no station gets art. A logo is not
    // worth a dead dashboard.
    //
    // SAFETY: same pointers, same lifetime argument as the tuner above.
    let net = match unsafe { android::net::init(vm, activity) } {
        Ok(()) => Some(app::Net {
            http: std::sync::Arc::new(android::net::AndroidNet),
            codec: std::sync::Arc::new(android::net::AndroidCodec),
        }),
        Err(_) => None,
    };

    let _driver = app::App::with_tuner(
        &ui,
        &db_path,
        &files_dir_for_prefs,
        tuner,
        tuner_is_real,
        net,
        // NO FIX, until a real one arrives. The default is Madison with `fix`
        // true — a host fake — and building it inside `with_tuner` put it on the
        // device: the satellite glyph was lit from the first frame and the
        // nearby list was for Wisconsin, wherever the car actually was.
        fake::FakeLocation::no_fix(),
    );

    // REAL POSITION, if this unit will give one.
    //
    // `start` returning false is NOT the end of it, and treating it as such is
    // why GPS never worked: on a first launch the grant does not exist yet, so
    // `start` asks for it and answers false, and the grant lands seconds later
    // when the driver taps Allow. `CarnyxLocation` watches for that itself and
    // registers the providers when it appears — see `requestGrant`.
    //
    // Both outcomes reach the diagnostics log through the tuner's own event
    // path, so a unit that never gets a fix says so somewhere a person can read
    // rather than just showing an unlit satellite.
    //
    // SAFETY: same pointers, same lifetime argument as the tuner above.
    match unsafe { android::location::init(vm, activity) } {
        Ok(()) => {
            let listening = android::location::start();
            android::ingest_note(if listening {
                "location: listening".into()
            } else {
                "location: waiting for the permission grant".into()
            });
        }
        Err(e) => android::ingest_note(format!("location: unavailable — {e}")),
    }
    // Tuner events arrive on binder and pump threads. The hop back to the UI
    // thread is `invoke_from_event_loop`; the drain itself reads the App out of
    // a thread-local, because `Rc<App>` cannot cross a `Send` boundary.
    app::set_event_wake(|| {
        let _ = slint::invoke_from_event_loop(app::drain_current);
    });
    ui.run().unwrap();
}
