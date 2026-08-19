//! What one wake from the tuner queue costs.
//!
//! `drain_current` ends in `push_all`, and `push_all` is posted once per event
//! that reaches the queue. The raw RDS pump runs at 90ms (`NwdBridge.RDS_POLL_MS`),
//! so on a station with RDS this is roughly eleven times a second, forever — with
//! whatever overlay the driver has open re-rendering behind it.
//!
//! This measures it rather than reasoning about it, because "that looks
//! expensive" has been wrong in this tree before and a performance claim without
//! a number is an opinion.
//!
//! Run it with the nearby list full and the diagnostics log full, which is the
//! state a drive actually reaches.

mod common;

use common::{dir_for, install_platform};
use slint::ComponentHandle;
use std::time::Instant;

fn bench(name: &str, rounds: u32, mut f: impl FnMut()) -> f64 {
    // One warm pass, so a lazily-built cache is not charged to the measurement.
    f();
    let t = Instant::now();
    for _ in 0..rounds {
        f();
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
    println!("  {name:<22} {per:>8.3} ms");
    per
}

fn main() {
    let _window = install_platform();
    let dir = dir_for("pushbench");
    // A LOCATED fake, so the nearby query returns its full hundred rows and the
    // hero's own resolve has something to find. The no-fix cases elsewhere in the
    // probes measure an empty picker, which is not the state that hurts.
    let ui = carnyx::AppWindow::new().expect("window");
    let app = carnyx::app::App::with_tuner(
        &ui,
        &carnyx::app::host_db_path(),
        &dir,
        Box::new(carnyx::android::FakeTuner::new()),
        false,
        None,
        carnyx::fake::FakeLocation::default(),
    );

    // Fill the diagnostics ring the way a drive fills it. The RDS line alone
    // reaches CarFM's measured 184 lines in forty minutes, and the ring holds 200.
    for i in 0..250 {
        carnyx::android::ingest_note(format!("filler line {i} — a realistic length of diagnostic"));
    }
    app.drain_events();

    println!("one wake from the tuner queue, averaged over 200:");
    let all = bench("push_all", 200, || app.push_all());

    println!("\nand where it goes:");
    let mut parts = 0.0;
    for (name, f) in [
        ("push_hero", 0),
        ("push_presets", 1),
        ("push_meter", 2),
        ("push_nearby", 3),
        ("push_settings", 4),
        ("push_freq", 5),
    ] {
        parts += bench(name, 200, || app.push_one_for_bench(f));
    }
    println!("  {:<22} {parts:>8.3} ms", "(sum)");

    // ── WHAT A LIVE FIX COSTS, which is the cadence nobody chose ────────────
    //
    // `set_position` is the seam every GPS fix lands on, and it re-runs the whole
    // nearby query: bounding box, rank, view, publish. A parked car with a lock
    // produces a fix every two seconds FOREVER — the app's own comment in
    // `refresh_nearby` says so — and every one of them pays this, whether the car
    // moved or not.
    //
    // TWO CASES, because they should not cost the same. A jittered fix is a car
    // standing still: metres of GPS noise, the same stations, the same list. A
    // moved fix is a car that has actually gone somewhere. Only the second has
    // any work to do.
    println!("\nwhat one GPS fix costs (parked cars produce one every 2s):");
    let base = carnyx::fake::FakeLocation::default();
    let mut n = 0u32;
    let jitter = bench("fix, jittered 3m", 50, || {
        n += 1;
        // ~3 metres, alternating, which is ordinary standing-still GPS noise.
        let d = if n % 2 == 0 { 0.000027 } else { -0.000027 };
        app.set_position(carnyx::fake::FakeLocation { lat: base.lat + d, ..base });
    });
    let mut m = 0u32;
    let moved = bench("fix, moved 5km", 50, || {
        m += 1;
        let d = f64::from(m % 10) * 0.0045;
        app.set_position(carnyx::fake::FakeLocation { lat: base.lat + d, ..base });
    });
    println!(
        "  a parked car pays {:.1} ms every 2s to learn it has not moved",
        jitter
    );
    let _ = moved;

    // What the queue actually delivers on a station with RDS: a group every 90ms,
    // each one waking the loop.
    let per_second = 1000.0 / 90.0;
    println!(
        "\nat the RDS pump's 90ms cadence that is {:.1} wakes/s → {:.1}% of one core",
        per_second,
        all * per_second / 10.0
    );

    // ── AND THE SAME WAKE WITH A WINDOW OPEN ──
    //
    // This is what the driver reported. `push_all` REPLACES every model
    // wholesale, so while the nearby list is on screen each wake hands Slint a
    // brand-new hundred-row model and it tears the rows down and builds them
    // again — a hundred repeated elements, every ninety milliseconds.
    let window = _window;
    let mut buf =
        vec![slint::platform::software_renderer::PremultipliedRgbaColor::default(); 1024 * 614];
    window.set_size(slint::PhysicalSize::new(1024, 614));
    ui.show().expect("show");
    let mut draw = || {
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut buf, 1024);
        });
    };

    println!("\none wake AND a frame, averaged over 60:");
    ui.set_overlay(carnyx::Overlay::None);
    draw();
    bench("face only", 60, || {
        app.push_all();
        draw();
    });

    ui.invoke_open_nearby();
    ui.set_overlay(carnyx::Overlay::Nearby);
    draw();
    bench("nearby open", 60, || {
        app.push_all();
        draw();
    });

    ui.set_overlay(carnyx::Overlay::Settings);
    draw();
    bench("settings open", 60, || {
        app.push_all();
        draw();
    });

    // The logo window. Its own model is NOT in `push_all`, so a wake does not
    // rebuild it — but the overlay is still on screen and re-rendered, and it is
    // the window the driver named first.
    ui.invoke_open_logo_search(0);
    ui.invoke_logo_search_search();
    ui.set_overlay(carnyx::Overlay::LogoSearch);
    draw();
    bench("logo search open", 60, || {
        app.push_all();
        draw();
    });
    ui.set_overlay(carnyx::Overlay::None);
    draw();

    println!("\nand what OPENING one costs, averaged over 20:");
    bench("open nearby", 20, || {
        ui.set_overlay(carnyx::Overlay::None);
        draw();
        ui.invoke_open_nearby();
        ui.set_overlay(carnyx::Overlay::Nearby);
        draw();
    });

    bench("open logo search", 20, || {
        ui.set_overlay(carnyx::Overlay::None);
        draw();
        ui.invoke_open_logo_search(0);
        ui.invoke_logo_search_search();
        ui.set_overlay(carnyx::Overlay::LogoSearch);
        draw();
    });

    ui.hide().expect("hide");
    let _ = std::fs::remove_dir_all(&dir);
}
