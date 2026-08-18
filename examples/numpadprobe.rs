//! Drives the tune overlay's "Enter frequency" tab through a real `App`.
//!
//! `numpad_press` and `numpad_commit` are pure and have their own tests, and
//! those tests prove nothing about the tab: the seek has to empty the buffer
//! before it sweeps, CANCEL has to put back the frequency the tab opened on, and
//! TUNE has to close the overlay whether or not what was typed is a dial. All of
//! that lives in the callbacks, which is the layer this project keeps being
//! bitten one step away from.
//!
//! WHAT CHANGED, and why the assertions below are not the ones that were here.
//! The mini-handoff folds the standalone keypad card into the nearby picker as a
//! second tab and removes the hero-frequency tap, so there is no card to keep
//! standing and no `open-numpad` to open it. §5 makes TUNE a dismissal in every
//! case — "an empty/invalid buffer closes without retuning" — which means the
//! refusal this file used to assert cannot exist: the old card lit an error and
//! stayed up, and nothing stays up now. The out-of-band warning moved to where it
//! can still be read, live while typing; `band_prefix_ok` is what decides when,
//! and the last block here is what holds it to a rule a driver can live with.

mod common;

use carnyx::{NearbyTab, Overlay};
use common::{dir_for, install_platform, launch};
use slint::ComponentHandle;

fn tap(ui: &carnyx::AppWindow, keys: &str) {
    for k in keys.chars() {
        ui.invoke_freq_key(k.to_string().into());
    }
}

/// Open the overlay and switch to the keypad, the only way in that exists.
fn open_freq(ui: &carnyx::AppWindow) {
    ui.invoke_open_nearby();
    ui.set_overlay(Overlay::Nearby);
    ui.invoke_set_nearby_tab(NearbyTab::Freq);
}

fn main() {
    let _window = install_platform();
    let dir = dir_for("numpad");
    let (ui, _app) = launch(&dir, 96.3);

    // ── The overlay opens on the STATION LIST, never on the keypad ──
    //
    // §2: the nearby button is the only entry point and it lands on Nearby
    // stations. The tab is not remembered between visits either — asserted at the
    // end, after this run has left it on the keypad.
    ui.invoke_open_nearby();
    assert_eq!(ui.get_nearby_tab(), NearbyTab::Nearby);

    // ── The keypad opens on the live dial, dimmed ──
    ui.invoke_set_nearby_tab(NearbyTab::Freq);
    assert_eq!(ui.get_freq_display().as_str(), "96.3");
    assert!(ui.get_freq_display_dim(), "an untyped readout is the dial, dimmed");
    assert!(!ui.get_freq_error());

    // ── A sweep is a reason to be legible, even with nothing typed ──
    //
    // The readout follows the sweep, so dimming it through one is dimming the
    // thing being watched.
    ui.set_scanning(true);
    ui.invoke_freq_back(); // any republish
    assert!(!ui.get_freq_display_dim(), "a scan un-dims the readout");
    ui.set_scanning(false);

    // ── THE ENTRY RULES, through the real callback ──
    tap(&ui, "1055");
    assert_eq!(ui.get_freq_display().as_str(), "1055");
    tap(&ui, "9");
    assert_eq!(ui.get_freq_display().as_str(), "1055", "a fifth digit is refused");

    // ── A value that is not a dial CLOSES, and does not retune ──
    //
    // 1055 MHz is not a dial. The old card kept itself up with the buffer intact;
    // §5 replaced that with a dismissal, so what has to be true now is that the
    // radio did not move.
    let before = ui.get_freq_label().to_string();
    ui.invoke_freq_commit();
    assert_eq!(ui.get_overlay(), Overlay::None, "TUNE always closes");
    assert_eq!(ui.get_freq_label().as_str(), before, "and a non-dial does not tune");

    // ── The rounding reaches the radio, not just the pure function ──
    open_freq(&ui);
    tap(&ui, "105.");
    assert_eq!(ui.get_freq_display().as_str(), "105.", "a trailing point stands");
    tap(&ui, "5");
    assert_eq!(ui.get_freq_display().as_str(), "105.5");
    ui.invoke_freq_commit();
    assert_eq!(ui.get_freq_label().as_str(), "105.5");
    assert_eq!(ui.get_overlay(), Overlay::None);

    // ── A seek empties the buffer BEFORE it sweeps ──
    //
    // Without it a half-typed dial sits in the readout through the whole sweep,
    // and the tab's whole point is that the readout follows the sweep to the
    // station it finds.
    open_freq(&ui);
    tap(&ui, "88");
    assert_eq!(ui.get_freq_display().as_str(), "88");
    ui.invoke_freq_seek(1);
    assert!(ui.get_freq_display().as_str() != "88", "the seek must clear the buffer");
    assert!(ui.get_freq_display_dim(), "and hand the readout back to the dial");
    assert!(!ui.get_freq_error());

    // ── CANCEL puts back the frequency the tab opened on ──
    //
    // §5, and it only means anything BECAUSE the seek above leaves the overlay
    // open: a driver can walk the dial several stations away and then change
    // their mind. Typing alone never moves the radio, so a seek is the one path
    // where CANCEL has work to do — and on the fake tuner a sweep may land back
    // where it started, which is why the restore is only asserted when it moved.
    ui.invoke_select_preset(0);
    let opened_on = ui.get_freq_label().to_string();
    open_freq(&ui);
    ui.invoke_freq_seek(1);
    let swept = ui.get_freq_label().to_string();
    ui.invoke_freq_cancel();
    assert_eq!(ui.get_overlay(), Overlay::None, "CANCEL closes");
    if swept != opened_on {
        assert_eq!(
            ui.get_freq_label().as_str(),
            opened_on,
            "a seek moved the dial to {swept}, so CANCEL had to put {opened_on} back"
        );
    }

    // ── Leaving the keypad stops the sweep and clears the buffer ──
    open_freq(&ui);
    tap(&ui, "10");
    ui.invoke_set_nearby_tab(NearbyTab::Nearby);
    assert_eq!(ui.get_nearby_tab(), NearbyTab::Nearby);
    assert!(!ui.get_scanning(), "switching away stops a running seek");
    ui.invoke_set_nearby_tab(NearbyTab::Freq);
    assert!(ui.get_freq_display_dim(), "and the buffer did not survive the trip");

    // ── THE WARNING FIRES ON WHAT CANNOT BE SAVED, AND NOT BEFORE ──
    //
    // The rule that matters, because the naive one — "is this out of band" — lit
    // on the first keystroke of most valid entries. Every prefix of a real dial
    // has to stay quiet; a buffer no further typing can rescue has to speak.
    for (buf, want) in [
        ("1", false),     // on the way to 105.1
        ("10", false),    // and to 100.7
        ("105", false),
        ("105.", false),
        ("8", false),     // on the way to 87.5
        ("87.5", false),  // the bottom of the band is not outside it
        ("108.0", false), // nor is the top
        ("7", true),      // nothing in band starts with 7
        ("109", true),
        ("108.1", true),
    ] {
        ui.invoke_set_nearby_tab(NearbyTab::Freq); // clears the buffer
        tap(&ui, buf);
        assert_eq!(
            ui.get_freq_display().as_str(),
            buf,
            "the readout is the buffer exactly as typed"
        );
        assert_eq!(
            ui.get_freq_error(),
            want,
            "{buf:?} should {} the out-of-band line",
            if want { "show" } else { "not show" }
        );
    }

    // ── And the tab does not persist across a visit ──
    ui.invoke_freq_cancel();
    ui.invoke_open_nearby();
    assert_eq!(ui.get_nearby_tab(), NearbyTab::Nearby, "every visit opens on the list");

    ui.hide().expect("hide");
    let _ = std::fs::remove_dir_all(&dir);
    println!("freq tab: four digits, one decimal, rounded commit, TUNE always closes,");
    println!("          seek clears the buffer, CANCEL restores, warning only when stuck");
}
