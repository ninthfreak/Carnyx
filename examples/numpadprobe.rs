//! Drives the direct-entry TUNE card through a real `App`.
//!
//! `numpad_press` and `numpad_commit` are pure and have their own tests, and
//! those tests prove nothing about the card: the refusal is state that a keypress
//! clears, the seek has to empty the buffer before it sweeps, and a rejected
//! value has to leave the card standing with what was typed still in it. All
//! three live in the callbacks, which is the layer this project keeps being
//! bitten one step away from.

mod common;

use carnyx::Overlay;
use common::{dir_for, install_platform, launch};
use slint::ComponentHandle;

fn tap(ui: &carnyx::AppWindow, keys: &str) {
    for k in keys.chars() {
        ui.invoke_numpad_enter(k.to_string().into());
    }
}

fn main() {
    let _window = install_platform();
    let dir = dir_for("numpad");
    let (ui, _app) = launch(&dir, 96.3);

    // ── The card opens on the live dial, dimmed, with nothing to tune ──
    ui.invoke_open_numpad();
    assert_eq!(ui.get_numpad_display().as_str(), "96.3");
    assert!(ui.get_numpad_display_dim(), "an untyped display is the dial, dimmed");
    assert!(!ui.get_numpad_can_tune());
    assert!(!ui.get_numpad_error());

    // ── A sweep is a reason to be legible, even with nothing typed ──
    //
    // CarFM's `opacity: buf || scanning ? 1 : 0.45`. The display follows the
    // sweep, so dimming it through one is dimming the thing being watched.
    ui.set_scanning(true);
    ui.invoke_numpad_backspace(); // any republish
    assert!(!ui.get_numpad_display_dim(), "a scan un-dims the display");
    ui.set_scanning(false);

    // ── THE ENTRY RULES, through the real callback ──
    tap(&ui, "1055");
    assert_eq!(ui.get_numpad_display().as_str(), "1055");
    tap(&ui, "9");
    assert_eq!(ui.get_numpad_display().as_str(), "1055", "a fifth digit is refused");
    assert!(ui.get_numpad_can_tune(), "there is something typed, so TUNE is live");

    // ── A refused value KEEPS THE CARD UP and says why ──
    //
    // 1055 MHz is not a dial. It used to leave `numpad_error` false — the flag
    // was derived from the buffer and `1055` parses — so TUNE did nothing at all
    // and said nothing about it.
    ui.invoke_numpad_tune();
    assert!(ui.get_numpad_error(), "a refused commit shows the error");
    assert_eq!(ui.get_numpad_display().as_str(), "1055", "and keeps what was typed");

    // A keypress clears the refusal, as CarFM's `press` does.
    ui.invoke_numpad_backspace();
    assert!(!ui.get_numpad_error(), "a keypress clears the error");
    assert_eq!(ui.get_numpad_display().as_str(), "105");

    // ── Junk that does not parse is refused the same way ──
    //
    // It cannot be typed any more, so it is built the only way left: the rules
    // let a trailing point stand, and "105." commits as 105.0 — while a bare
    // point never gets in at all.
    tap(&ui, ".");
    assert_eq!(ui.get_numpad_display().as_str(), "105.");
    ui.invoke_numpad_tune();
    assert_eq!(ui.get_overlay(), Overlay::None, "105. is a dial and tunes");
    assert_eq!(ui.get_freq_label().as_str(), "105.0");

    // ── The rounding reaches the radio, not just the pure function ──
    ui.invoke_open_numpad();
    tap(&ui, "105.");
    tap(&ui, "5");
    assert_eq!(ui.get_numpad_display().as_str(), "105.5");
    ui.invoke_numpad_tune();
    assert_eq!(ui.get_freq_label().as_str(), "105.5");

    // ── A seek empties the buffer BEFORE it sweeps ──
    //
    // CarFM's `seek` calls `reset()` first (Numpad.tsx:43). Without it a
    // half-typed dial sits in the display through the whole sweep, and the card's
    // whole point is that the display follows the sweep to the station it finds.
    ui.invoke_open_numpad();
    tap(&ui, "88");
    assert_eq!(ui.get_numpad_display().as_str(), "88");
    ui.invoke_numpad_seek(1);
    assert!(ui.get_numpad_display().as_str() != "88", "the seek must clear the buffer");
    assert!(ui.get_numpad_display_dim(), "and hand the display back to the dial");
    assert!(!ui.get_numpad_can_tune());
    assert!(!ui.get_numpad_error());

    ui.hide().expect("hide");
    let _ = std::fs::remove_dir_all(&dir);
    println!("numpad: four digits, one decimal, rounded commit, refusal shown, seek clears");
}
