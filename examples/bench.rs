//! How long a full software-rendered frame takes, at head-unit size.
//!
//! This exists because the renderer choice for Android turns on one number. The
//! Skia path does not build for this device's 32-bit ARM, and the alternative is
//! Slint's own software renderer on the CPU — which is only a real option if a
//! frame fits comfortably in a frame budget on a head unit's processor.
//!
//!     cargo run --release --example bench
//!
//! `--release` is not optional: a debug build measures the wrong thing by an
//! order of magnitude. `RepaintBufferType::NewBuffer` forces a FULL repaint every
//! frame, so this is the pessimistic number — on the device, a swapchain allows
//! partial repaint and most frames here only touch the marquee and the meter.
use std::rc::Rc;
use std::time::Instant;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

fn main() {
    let (w, h) = (1024u32, 614u32);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() })).unwrap();

    let ui = carnyx::build().unwrap();
    window.set_size(PhysicalSize::new(w, h));
    ui.show().unwrap();

    let mut buffer = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    let mut times = Vec::new();
    for _ in 0..120 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let t = Instant::now();
        window.draw_if_needed(|r| {
            r.render(&mut buffer, w as usize);
        });
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{}x{} full frame: min {:.2} ms  median {:.2} ms  max {:.2} ms",
        w,
        h,
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
}
