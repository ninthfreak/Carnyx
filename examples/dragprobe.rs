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

        // Now drag right, still without lifting.
        for step in 1..=6 {
            let x = X0 + (480.0 - X0) * (step as f32 / 6.0);
            ui.window()
                .dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(x, Y0) });
            std::thread::sleep(std::time::Duration::from_millis(30));
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
        let slot0 = row(505, 150);
        println!(
            "drift {drift:>4} px -> entered reorder: {entered:<5}  slot0 rgb {:?}",
            slot0
        );
        ui.hide().expect("hide");
    }
}
