//! Carnyx — radio for a NOWADA (NWD) Android head unit.
//!
//! The interface is Slint and the logic is Rust. There is no desktop
//! application: `cargo build` on the host is a compile check, and the screenshot
//! example is how the face is inspected without a car.

slint::include_modules!();

pub mod android;
pub mod app;
pub mod callsigns;
pub mod crashlog;
pub mod eggs;
pub mod fake;
pub mod logos;
pub mod nav;
pub mod prefs;
pub mod rds;
pub mod session;
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

    // ── PARTIAL RENDERING, WHICH IS OFF UNLESS THIS LINE RUNS ────────────────
    //
    // Without it every frame repaints all 1024x614 pixels, however little
    // changed. The switch is not exposed as an API: Slint's Android backend
    // builds `SkiaRenderer::default`, whose partial-rendering state comes from
    // `create_partial_renderer_state(None)`, and with no surface to consult that
    // function falls through to exactly this variable
    // (i-slint-renderer-skia-1.17.1/lib.rs:142-147). The OpenGL surface the head
    // unit uses never gets asked; it inherits the trait default, which is false
    // (lib.rs:1102), and only the software surface overrides it to true.
    //
    // THERE IS NO TEARING FAILURE MODE, which is worth stating because the
    // obvious worry is that a swapchain does not preserve the frame behind it.
    // Slint keeps a history of dirty regions indexed by back-buffer age and
    // unions however many the driver says are stale; when the driver cannot
    // report an age at all it returns the WHOLE window instead (lib.rs:716-727).
    // So on hardware without EGL_EXT_buffer_age this is a full repaint — today's
    // behaviour exactly — and on hardware with it, it is correct.
    //
    // BEFORE `init`, because the renderer reads the variable while it is being
    // constructed and never looks again.
    //
    // SAFETY-ADJACENT NOTE for the next edition bump: `set_var` becomes unsafe in
    // Rust 2024 because it races other threads reading the environment. Here it
    // is the first statement of the process's entry point, before Slint, the JVM
    // bridge or any worker exists.
    std::env::set_var("SLINT_SKIA_PARTIAL_RENDERING", "1");

    // All four must be taken BEFORE `init`, which consumes the `AndroidApp`.
    let files_dir = android_app.internal_data_path();
    let assets = android_app.asset_manager();
    let vm = android_app.vm_as_ptr();
    let activity = android_app.activity_as_ptr();
    // THE LIFECYCLE, WATCHED. `init` alone would run the same event loop with
    // nobody looking at it, and there are two things only this listener can do.
    //
    // It WRITES THE PARTING SNAPSHOT. `session` restores the dial and the RDS on
    // the next launch, and the only honest moment to take that picture is the
    // moment the app is being put down. Android blocks in `onPause` until the
    // native thread has taken the command, so this runs before the app is
    // actually backgrounded rather than hopefully afterwards.
    //
    // And it RECORDS WHICH WAY THE RUN ENDED, which is the difference between a
    // fault this repository can fix and one it cannot: a `Destroy` means the
    // Activity was re-created and `config_changes` is the lever, while a `Pause`
    // or `Stop` followed by a cold process means the app was killed outright,
    // and nothing short of a foreground service prevents that.
    //
    // Nothing here can fail loudly: before the App exists, `persist_session_current`
    // finds nothing and `ingest_note` has no sink, and both are no-ops.
    slint::android::init_with_event_listener(android_app, |event| {
        use android_activity::{MainEvent, PollEvent};
        let PollEvent::Main(main) = event else { return };
        // WHICH SIDE OF THE GLASS THE DRIVER IS ON, and this listener is the only
        // code that ever finds out. The station pop-up reads it: a notification
        // announcing a tune the face is already showing would be noise, and one
        // announcing a tune the driver cannot see is the whole feature.
        //
        // BEFORE the `parting` match below and not folded into it, because that
        // one returns early for everything it does not recognise — `Resume`
        // included — and this has to see both edges.
        // FOCUS IS RECORDED AND GATES NOTHING. It is a separate event from
        // Resume/Pause, so recording it is the only way a drive log can say
        // which edges this unit actually raises — but the gate reads the
        // resumed half alone, because on a unit that switches whole screens a
        // lost focus is a shade or a dialog rather than a departure. See
        // `android::FOCUSED`, which records the invented premise that briefly
        // had this gating the pop-up.
        match main {
            MainEvent::GainedFocus => {
                android::set_focused(true);
                android::ingest_note("lifecycle: focus gained".into());
                // The face is answerable again, so the banner has nothing left to
                // say — same reason as `Resume` below.
                android::clear_station_announcement();
            }
            MainEvent::LostFocus => {
                android::set_focused(false);
                android::ingest_note("lifecycle: focus lost".into());
            }
            _ => {}
        }
        match main {
            MainEvent::Resume { .. } => {
                android::set_resumed(true);
                // THE OTHER EDGE, IN THE LOG. `parting` below records Pause,
                // Stop and Destroy and returns early for everything else, so a
                // drive log showed the app going away and never coming back —
                // and the foreground flag is what the station pop-up is gated
                // on, so both edges have to be readable to tell a stuck flag
                // from a press that never arrived.
                android::ingest_note("lifecycle: resume".into());
                // The face IS the answer now, so the banner has nothing left to
                // say. Cleared on the way in rather than left to time out, so a
                // driver who taps the pop-up does not arrive at the face with it
                // still sitting in the shade behind them.
                android::clear_station_announcement();
            }
            MainEvent::Pause | MainEvent::Stop | MainEvent::Destroy => {
                android::set_resumed(false)
            }
            _ => {}
        }
        let parting = match main {
            MainEvent::Pause => session::Parting::Pause,
            MainEvent::Stop => session::Parting::Stop,
            MainEvent::Destroy => session::Parting::Destroy,
            MainEvent::LowMemory => session::Parting::LowMemory,
            // `SaveState` is the Android-blessed moment for this, but
            // `NativeActivity` only raises it when the system intends to restore
            // the instance later — it is not raised on the ordinary path out.
            // Treated as a pause, since that is what it means for our purposes.
            MainEvent::SaveState { .. } => session::Parting::Pause,
            _ => return,
        };
        app::persist_session_current(parting);
        android::ingest_note(format!("lifecycle: {}", parting.name()));
    })
    .unwrap();

    // The app's private data directory, and the prefs file's home. Captured
    // before the map below consumes `files_dir`.
    let files_dir_for_prefs = files_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // THE PANIC HOOK, before anything that can panic. A crash on this unit was
    // undiagnosable — no adb, no console, and the diagnostics log dies with the
    // process — so the next launch reads what this leaves behind. See
    // `crashlog`.
    crashlog::install(&files_dir_for_prefs);

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

    // THE FOREGROUND SERVICE (#67). A process with one is not a candidate for
    // the launcher's cleaner or the low-memory killer, which is the fault
    // `session.rs` currently survives rather than prevents.
    //
    // AFTER `with_tuner`, NOT BEFORE, for the notification's sake: the dial is
    // whatever the restore or the tuner's first report settled on, and the face
    // has it by now. Still start-up, so still in front — which is the constraint
    // that matters, because from Android 12 a background `startForegroundService`
    // throws.
    //
    // Every failure here is ordinary and none is branched on. Under the DEFAULT
    // cargo-apk build the service class is not in the APK at all — cargo-apk
    // packages no Java — so the platform refuses the component and the app
    // behaves exactly as it did before this existed. That is why `session.rs`
    // stays: this prevents the restarts it can, and the restore covers the rest.
    //
    // AND THE OUTCOME GOES IN THE APP'S OWN LOG, not just logcat. This unit has
    // no adb, so a `Log.i` from Java reaches nobody; the settings panel's log is
    // the only channel a driver can read. It already carries the `session:`
    // line, which says `app #N in this process` — so the two lines answer the
    // question together, and neither does alone:
    //
    //   `service: started` then later `app #2 in this process`  the service works
    //   `service: started` but `app #1` on every return          it is not enough
    //   `service: none`                                          it never ran
    //
    // SAFETY: same pointers, same lifetime argument as the tuner above.
    let started = unsafe { android::service::init(vm, activity) }.is_ok()
        && android::service::start(ui.get_freq_label().as_str());
    _driver.log_platform(if started {
        "service: started"
    } else {
        "service: none — this build has no service class, or the platform refused it"
    });

    // THE TWO DIAGNOSTICS PROBES' CLASSES. Loaded here rather than on the tap,
    // because a row that cannot load its class has nothing to say and a settings
    // panel is a poor place to find that out. Loading is ALL this does — neither
    // walks a package list or a settings table until a driver asks it to, which
    // is the whole reason they are rows and not start-up work.
    //
    // The outcome is deliberately not logged: on a host build there is no dex at
    // all, and the rows already answer "unavailable in this build" when tapped,
    // which is a better place to read it than a line at the top of every log.
    //
    // SAFETY: same pointers, same lifetime argument as the two above.
    let _ = unsafe { android::probe::init(vm, activity) };
    let _ = unsafe { android::stock::init(vm, activity) };

    // THE STATION POP-UP'S CLASS, loaded here rather than lazily on the first
    // station change: that change happens while the driver is in another app,
    // which is the worst moment to discover the dex will not load. Loading is
    // all this does — nothing is posted until there is something to say, and on
    // a host build there is no class and `post` answers false forever.
    //
    // SAFETY: same pointers, same lifetime argument as the two above.
    let alerts = unsafe { android::alert::init(vm, activity) }.is_ok();
    _driver.log_platform(if alerts {
        "station pop-up: ready"
    } else {
        "station pop-up: unavailable — the alert class did not load"
    });

    // THE WAKE RECEIVER'S HALF OF THE CONVERSATION (#67's other half).
    //
    // `init` does two things: it loads the class, and it seeds the flag the
    // receiver reads after this process is killed — start-up is unambiguously in
    // front, so `was_foreground` is true from here rather than from whenever
    // `Resume` happens to arrive. Every later edge comes through
    // `android::set_foreground`, which the lifecycle listener already calls.
    //
    // SAFETY: same pointers, same lifetime argument as the three above.
    let _ = unsafe { android::wake::init(vm, activity) };

    // AND WHAT THE RECEIVER DID, if it ran at all. THIS IS THE ONLY EVIDENCE
    // THIS FEATURE CAN PRODUCE: the receiver runs in a process with no face, on
    // a unit with no adb, so "the broadcast never arrived", "the flag said the
    // driver was elsewhere" and "Android refused a background activity start"
    // are three different outcomes that look identical from the driver's seat.
    // Taken and cleared, so the line belongs to the drive it describes.
    //
    // Silence is the ordinary case and is NOT a failure: a launcher tap says
    // nothing, and neither does a cargo-apk build, which packages no Java and
    // whose manifest schema has no `<receiver>` field at all.
    let wake_note = android::take_wake_note();
    if !wake_note.is_empty() {
        _driver.log_platform(&format!("wake: {wake_note}"));
    }

    // AND WHAT THE LAST SLEEP MANAGED, which is a line the diagnostics log has
    // never been able to hold. That log is a ring in memory, so everything
    // written as the MCU cuts power died with the process that wrote it — which
    // is why "Carnyx does not shut off the radio audio when the head unit
    // sleeps" could not be answered from a drive log at all. Both receivers now
    // write it to disk with a blocking `commit` before anything else, and this
    // reads it back.
    //
    // PRINTED EVEN WHEN EMPTY, unlike the wake note above, because here the
    // absence is the finding. A launch that follows an ignition cycle with
    // nothing recorded means the ACC-off broadcast never arrived — a different
    // fault from a release that was attempted and failed, needing a different
    // fix — and a missing line cannot say which, while this one can.
    let sleep_note = android::take_sleep_note();
    _driver.log_platform(&if sleep_note.is_empty() {
        "last sleep: nothing recorded".to_string()
    } else {
        format!("last sleep: {sleep_note}")
    });

    // AND WHETHER PARTIAL RENDERING TOOK, read back rather than assumed. The
    // variable is set at the top of this function; this line is the only evidence
    // a driver can get that the renderer saw it, since the alternative — Slint
    // quietly repainting the whole screen every frame — looks exactly the same on
    // a display. See the note beside the `set_var`.
    _driver.log_platform(match std::env::var("SLINT_SKIA_PARTIAL_RENDERING") {
        Ok(_) => "partial rendering: requested",
        Err(_) => "partial rendering: OFF — every frame repaints the whole screen",
    });

    // WHAT KILLED THE LAST RUN, if anything did and it went through Rust's panic
    // machinery. Read once and deleted, so it is reported for the drive after
    // the crash and not for every drive thereafter. Silence here is not proof of
    // a clean end — an OOM kill, a Java exception or a signal leaves nothing —
    // but it narrows the next question a great deal.
    if let Some(why) = crashlog::take(&files_dir_for_prefs) {
        _driver.log_platform(&format!("crash: the last run panicked — {why}"));
    }

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
