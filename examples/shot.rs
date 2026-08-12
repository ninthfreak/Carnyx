//! Render the face at every target surface and write PNGs.
//!
//! The head unit is the only real target, but waiting for a car to see whether a
//! layout is right is not a workflow. This drives Slint's software renderer
//! directly — no window system, no GPU — at each of the five surfaces in ANDROID
//! §2, so the face can be diffed against the design references from a terminal.
//!
//!     cargo run --example shot
//!
//! Output lands in `shots/`.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

/// The five first-class surfaces of ANDROID §2, plus the states worth a second
/// look: (name, width, height, dark, state).
const SURFACES: &[(&str, u32, u32, bool, State)] = &[
    ("head-unit-light", 1024, 614, false, State::Normal),
    ("head-unit-dark", 1024, 614, true, State::Normal),
    ("slice-two-thirds", 900, 810, false, State::Normal),
    ("slice-one-third", 470, 845, false, State::Normal),
    ("phone-landscape", 800, 360, false, State::Normal),
    ("phone-portrait", 360, 800, false, State::Normal),
    ("phone-portrait-dark", 360, 800, true, State::Normal),
    ("audio-released", 1024, 614, false, State::AudioReleased),
    ("tuner-error", 1024, 614, false, State::TunerError),
    ("driving", 1024, 614, false, State::Driving),
    ("weak-and-lossy", 1024, 614, false, State::WeakAndLossy),
    ("no-presets", 1024, 614, false, State::NoPresets),
    ("tuned", 1024, 614, false, State::Tuned),
    ("tuned-portrait", 360, 800, false, State::Tuned),
    ("out-of-band", 1024, 614, false, State::OutOfBand),
    ("long-radiotext", 1024, 614, false, State::LongRadioText),
    ("no-callsign", 1024, 614, false, State::NoCallsign),
    ("stereo-unknown", 1024, 614, false, State::StereoUnknown),
    ("long-genre", 1024, 614, false, State::LongGenre),
];

#[derive(Clone, Copy, PartialEq)]
enum State {
    Normal,
    /// §4.7 — priority released to another source; the face goes flat and dead.
    AudioReleased,
    /// §4.1 — no compatible tuner; the fault pill replaces the whole OK cluster.
    TunerError,
    /// §4.6 — moving, with a GPS fix, and a traffic announcement running.
    Driving,
    /// A strong carrier arriving in pieces: dotted outer arcs, mono, no RDS.
    WeakAndLossy,
    NoPresets,
    /// Tuned TO a preset: the enlarged tile, its blue border and underline, and
    /// the neighbours the peek cards then show.
    Tuned,
    /// Dial outside 87.5-108.0.
    OutOfBand,
    /// RadioText past the ~46-character marquee threshold.
    LongRadioText,
    /// The built-in tuner with no GPS lock yet: nothing resolves the call sign, so
    /// the frequency stands as the identity — never an inaccurate "Tuning...".
    NoCallsign,
    /// Nothing has reported yet, so the pill is EMPTY rather than asserting MONO.
    StereoUnknown,
    /// A genre string past the 200dp cap, which must elide rather than push the
    /// controls or collapse the line.
    LongGenre,
}

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
}

fn main() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless { window: window.clone() }))
        .expect("set platform");
    std::fs::create_dir_all("shots").expect("create shots/");

    for (name, w, h, dark, state) in SURFACES {
        let ui = carnyx::build().expect("build window");
        ui.global::<carnyx::Pal>().set_dark(*dark);
        apply(&ui, *state);
        window.set_size(PhysicalSize::new(*w, *h));
        ui.show().expect("show");

        // Settle any `init =>` writes and let the first animation frame land
        // before asking for pixels.
        slint::platform::update_timers_and_animations();
        window.request_redraw();

        let mut buffer =
            vec![PremultipliedRgbaColor::default(); (*w as usize) * (*h as usize)];
        let drawn = window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, *w as usize);
        });
        assert!(drawn, "{name}: nothing was rendered");

        write_png(name, *w, *h, &buffer);
        ui.hide().expect("hide");
        println!("shots/{name}.png  {w}x{h}");
    }
}

/// Push one of the states the face has to be able to draw.
fn apply(ui: &carnyx::AppWindow, state: State) {
    match state {
        State::Normal => {}
        State::AudioReleased => ui.set_audio_active(false),
        State::TunerError => ui.set_tuner_error(true),
        State::Driving => {
            ui.set_in_motion(true);
            ui.set_tp(true);
            ui.set_ta(true);
        }
        State::WeakAndLossy => {
            ui.set_full_pairs(3);
            ui.set_half(false);
            ui.set_dotted_arcs(2);
            ui.set_level_text("71".into());
            ui.set_stereo(false);
            ui.set_rds(false);
            ui.set_af(false);
            ui.set_pty("".into());
            ui.set_radio_text("".into());
        }
        State::NoPresets => {
            ui.set_presets(slint::ModelRc::default());
            ui.set_has_prev(false);
            ui.set_has_next(false);
        }
        State::Tuned => ui.invoke_select_preset(2),
        State::OutOfBand => {
            ui.set_in_band(false);
            ui.set_freq_label("76.5".into());
        }
        State::LongRadioText => ui.set_radio_text(
            "NOW PLAYING ON HOT 105.1 — Harry Styles — As It Was — up next Dua Lipa".into(),
        ),
        State::NoCallsign => ui.set_ident("".into()),
        State::StereoUnknown => ui.set_stereo_known(false),
        State::LongGenre => ui.set_pty("Adult Album Alternative and Classic Rock".into()),
    }
}

fn write_png(name: &str, w: u32, h: u32, buffer: &[PremultipliedRgbaColor]) {
    let path = format!("shots/{name}.png");
    let file = std::fs::File::create(&path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);

    // The renderer works in premultiplied alpha; PNG wants straight alpha.
    let mut bytes = Vec::with_capacity(buffer.len() * 4);
    for px in buffer {
        let a = px.alpha;
        let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255) / a as u32).min(255) as u8 };
        bytes.extend_from_slice(&[un(px.red), un(px.green), un(px.blue), a]);
    }
    enc.write_header().expect("header").write_image_data(&bytes).expect("data");
}
