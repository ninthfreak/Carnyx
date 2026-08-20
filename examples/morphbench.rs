//! How much of the 520ms morph is over before a frame can be drawn?
//!
//! ## The question
//!
//! > "it's an actual animation unlike what I've seen in Carnyx"
//!
//! Every other probe here checks the morph's SHAPE — where the cards are, how
//! big, how bright. None of them checks whether the driver ever sees it move,
//! and that is a different question with a different answer.
//!
//! `step_preset_from` arms the morph and then calls `tune`, which is
//! SYNCHRONOUS: it commands the tuner, drains the whole event queue, pumps the
//! RDS decoder until it settles, and republishes every model. Only when all of
//! that returns can Slint draw. Slint's animations run on WALL CLOCK, so
//! whatever time passes in there is time the morph spends advancing with nothing
//! on screen — and the first frame the driver sees is the morph already that far
//! through.
//!
//! At 520ms, a 300ms block means the first visible frame is 58% done and the
//! travel is 93% done (the position curve is a cubic ease-out). That is not a
//! slow animation; it is a hard cut with a short tail, which is exactly what
//! "not an actual animation" describes.
//!
//! ## What this measures, and what it cannot
//!
//! The host path, with `FakeTuner`. That is a LOWER BOUND and not the number
//! from the car: the fake's `tune` is a function call where the device's is a
//! binder round trip into the vendor service, and the fake's RDS corpus is
//! replayed from memory where the device's is a 90ms hardware pump. If the host
//! already loses a meaningful slice of the window, the unit loses more.

mod common;

use common::{dir_for, install_platform};
use slint::ComponentHandle;
use std::time::Instant;

/// The position curve, CarFM's `easeMove` — how far the travel has run at `t`.
fn travelled(ms: f64) -> f64 {
    let p = (ms / 520.0).clamp(0.0, 1.0);
    1.0 - (1.0 - p).powi(3)
}

fn main() {
    let window = install_platform();
    let dir = dir_for("morphbench");
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
    app.drain_events();

    window.set_size(slint::PhysicalSize::new(1024, 614));
    ui.show().expect("show");
    let mut buf =
        vec![slint::platform::software_renderer::PremultipliedRgbaColor::default(); 1024 * 614];
    let mut draw = || {
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut buf, 1024);
        });
    };
    draw();

    // Warm every lazy cache first, so the measurement is the steady-state cost a
    // driver pays on the tenth press rather than the one-off cost of the first.
    for _ in 0..3 {
        ui.invoke_step_preset(1);
        draw();
    }

    println!("what happens between the wheel press and the first frame:\n");

    let mut worst: f64 = 0.0;
    let mut total = 0.0;
    const ROUNDS: u32 = 12;
    for i in 0..ROUNDS {
        let t = Instant::now();
        // THE SHIPPING CALLBACK. `on_step_preset` is what both the wheel and the
        // peek cards reach, and it returns only when the tune, the drain, the RDS
        // pump and the republish are all done.
        ui.invoke_step_preset(1);
        let blocked = t.elapsed().as_secs_f64() * 1000.0;
        // The first frame the driver could possibly see.
        draw();
        total += blocked;
        worst = worst.max(blocked);
        if i < 3 {
            println!(
                "  press {}: blocked {:>7.1} ms  → first visible frame is {:>4.0}% \
                 into the morph, travel {:>3.0}% done",
                i + 1,
                blocked,
                (blocked / 520.0 * 100.0).min(100.0),
                travelled(blocked) * 100.0
            );
        }
    }
    let mean = total / f64::from(ROUNDS);

    println!(
        "\n  mean {:.1} ms, worst {:.1} ms over {ROUNDS} presses",
        mean, worst
    );
    println!(
        "  mean: {:.0}% of the window gone, {:.0}% of the travel already done",
        (mean / 520.0 * 100.0).min(100.0),
        travelled(mean) * 100.0
    );
    println!(
        "  worst: {:.0}% of the window gone, {:.0}% of the travel already done",
        (worst / 520.0 * 100.0).min(100.0),
        travelled(worst) * 100.0
    );

    // ── HOW MANY FRAMES THE MORPH ACTUALLY GETS ─────────────────────────────
    //
    // The block above turned out to be small, so the question becomes the other
    // one: an animation is only an animation if enough frames are drawn during
    // it. Draw as fast as the machine allows for the whole 520ms and count.
    println!("\nframes drawn across one 520ms morph, drawing flat out:");
    ui.invoke_step_preset(1);
    let start = Instant::now();
    let mut frames = 0u32;
    let mut worst_gap: f64 = 0.0;
    let mut last = start;
    while start.elapsed().as_millis() < 520 {
        slint::platform::update_timers_and_animations();
        draw();
        frames += 1;
        let now = Instant::now();
        worst_gap = worst_gap.max((now - last).as_secs_f64() * 1000.0);
        last = now;
    }
    let per = 520.0 / f64::from(frames);
    println!(
        "  {frames} frames, {per:.1} ms apart on average, worst gap {worst_gap:.1} ms"
    );
    println!(
        "  → on a unit where one frame costs N ms, the morph gets 520/N frames:\n             at  30 ms/frame that is 17 frames (smooth)\n             at  90 ms/frame that is  6 frames (visibly stepped)\n             at 170 ms/frame that is  3 frames (reads as a cut, not motion)"
    );

    // Where it goes. Each of these is inside `tune`, in this order.
    println!("\nand where that time goes (same work, timed separately):");
    let t = Instant::now();
    for _ in 0..ROUNDS {
        app.pump_rds_until_settled_for_bench();
    }
    println!(
        "  pump_rds_until_settled {:>7.1} ms",
        t.elapsed().as_secs_f64() * 1000.0 / f64::from(ROUNDS)
    );
    let t = Instant::now();
    for _ in 0..ROUNDS {
        app.push_all();
    }
    println!(
        "  push_all               {:>7.1} ms",
        t.elapsed().as_secs_f64() * 1000.0 / f64::from(ROUNDS)
    );

    println!(
        "\nHOST NUMBERS ARE A LOWER BOUND. The fake tuner's `tune` is a function\n\
         call where the device's is a binder round trip, and its RDS is replayed\n\
         from memory where the device's is a 90ms hardware pump."
    );

    ui.hide().expect("hide");
    let _ = std::fs::remove_dir_all(&dir);
}
