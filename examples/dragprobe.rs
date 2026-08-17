//! Drives the FULL reorder gesture the way a driver does: long-press to enter
//! the mode, then drag with the same finger.
//!
//! This exists because `examples/shot.rs` set `reordering` to true BEFORE
//! pressing, so it only ever exercised the second half — a drag begun when the
//! mode was already open. The entry path, which is the half a driver actually
//! starts from, had never run outside the car.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, PlatformError};

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

const W: u32 = 1024;
const H: u32 = 614;
/// Tile 0's centre on the wide track.
const X0: f32 = 150.0;
const Y0: f32 = 505.0;

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).expect("platform");

    // `drift` is the finger wandering during the hold, in logical px. A finger on
    // a dashboard is never still, and a Flickable watching for a scroll may claim
    // the gesture the moment it moves.
    for drift in [0.0_f32, 3.0, 12.0] {
        let (ui, _driver) = carnyx::build().expect("build");
        window.set_size(PhysicalSize::new(W, H));
        ui.show().expect("show");
        let mut buf = vec![PremultipliedRgbaColor::default(); (W * H) as usize];
        let mut render = || {
            window.request_redraw();
            window.draw_if_needed(|r| {
                r.render(&mut buf, W as usize);
            });
        };
        slint::platform::update_timers_and_animations();
        render();

        assert!(!ui.get_reordering(), "must start outside reorder mode");

        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position: LogicalPosition::new(X0, Y0),
            button: slint::platform::PointerEventButton::Left,
        });

        // Hold past the 550ms timer, pumping the clock like a running event loop.
        for i in 0..14 {
            if drift > 0.0 {
                let dx = drift * (i as f32 / 13.0);
                ui.window().dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(X0 + dx, Y0),
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(60));
            slint::platform::update_timers_and_animations();
            render();
        }

        let entered = ui.get_reordering();

        // Now drag right, still without lifting, in MANY SMALL STEPS.
        //
        // Small steps on purpose. The device report is that a tile "would start
        // to move, just a tiny bit, but then freeze", which is a gesture that
        // ARMS and then stops receiving moves — and a probe that jumps in six
        // big strides cannot tell that apart from one that works, because the
        // gap it opens looks the same after the first stride. Forty 8px steps
        // across the whole strip means the final position is only reached if
        // every move landed.
        for step in 1..=40 {
            let x = X0 + (760.0 - X0) * (step as f32 / 40.0);
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(x, Y0) });
            std::thread::sleep(std::time::Duration::from_millis(8));
            slint::platform::update_timers_and_animations();
            render();
        }
        for _ in 0..6 {
            slint::platform::update_timers_and_animations();
            render();
            std::thread::sleep(std::time::Duration::from_millis(40));
        }

        // Did the strip actually move? Sample the plate colours across the band.
        // Tile 0 is WQLF (green); if the drag took, slot 0 now shows WERN
        // (orange-brown) because the tiles behind the held one slid left.
        let row = |y: u32, x: u32| {
            let p = buf[(y * W + x) as usize];
            (p.red, p.green, p.blue)
        };
        // WHERE THE HELD TILE ENDED UP. It was tile 0 (WQLF, green) and the
        // finger finished over the far right of the strip, so a drag that ran
        // to completion leaves it near x=760 and a drag that froze early leaves
        // it near where it started. Sampled at the tile's own centre height.
        let green = |x: u32| {
            let (r, g, b) = row(505, x);
            g > 90 && g < 160 && r < 90 && b < 100
        };
        let mut held_at = None;
        for x in (100..900).step_by(4) {
            if green(x) {
                held_at = Some(x);
            }
        }
        println!(
            "drift {drift:>4} px -> entered: {entered:<5} slot0 {:?} held tile last seen at x={:?}",
            row(505, 150),
            held_at
        );
        assert!(entered, "the long press must open reorder mode");
        // THE ASSERTION THAT MATTERS. The finger finished at x=760, so a drag
        // that ran to completion leaves the held tile straddling it. A drag that
        // armed and then froze leaves it back near x=150 — which is what the
        // driver saw, and what this probe missed for two rounds by only ever
        // checking that the gap opened at all.
        let held = held_at.expect("the lifted tile must be somewhere on the strip");
        assert!(
            held > 700,
            "the held tile froze at x={held}; it should have followed the finger to ~760"
        );
        ui.hide().expect("hide");
    }
}
