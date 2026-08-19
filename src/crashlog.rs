//! What killed the last run, kept where the next one can read it.
//!
//! ## Why this exists
//!
//! > "if I stay on the window too long the whole app crashes"
//!
//! And there was no way to answer that. A Rust panic in `android_main` unwinds
//! into a process that has no console attached, `Log` output goes to logcat, and
//! THIS UNIT HAS NO adb — so the entire diagnosis available from the car was
//! "it disappeared". `session.rs` records how a run ENDED CLEANLY (pause, stop,
//! destroy, low memory); nothing recorded how one ended badly, which is the case
//! that needs recording most.
//!
//! So: a panic hook writes the message, the file and the line to a small file
//! beside the session snapshot, and the next launch reads it, puts it in the
//! diagnostics log the settings panel shows, and deletes it. One drive, one
//! answer, no cable.
//!
//! ## What it can and cannot catch
//!
//! A Rust panic, wherever it happens on the Rust side — a `BorrowMutError`, an
//! `unwrap` on a None, a slice index, an arithmetic overflow in debug. That
//! covers the crashes this codebase can cause by itself, and the hook runs
//! BEFORE the unwind, so the file is on disk even when the unwind then aborts
//! the process.
//!
//! It cannot catch what does not go through Rust's panic machinery: a Java
//! exception, an OOM kill by the low-memory killer, a SIGSEGV in Skia, or the
//! platform killing the process outright. Those leave nothing here — and that
//! absence is itself information, because "the app vanished and `crash:` says
//! nothing" narrows it to exactly that list.
//!
//! ## Why it writes rather than logs
//!
//! The diagnostics log lives in memory and dies with the process, which is the
//! one moment it would be needed. A file survives; `session.rs` already
//! established that a small file beside the prefs is how this app remembers
//! anything across a restart.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FILE: &str = "panic.txt";

/// Where to write, captured at start-up because a panic hook takes no arguments
/// and the panicking thread is in no state to go looking.
static DIR: OnceLock<PathBuf> = OnceLock::new();

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Install the hook. Call once, as early as the prefs directory is known.
///
/// CHAINED, NOT REPLACED. The default hook is what prints to stderr and to
/// logcat, and a machine that DOES have a cable should keep it — this only adds
/// a copy that survives the process.
pub fn install(dir: &Path) {
    if DIR.set(dir.to_path_buf()).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = DIR.get() {
            // Deliberately not `to_string()` on the whole `PanicHookInfo`: the
            // payload is what a human reads and the location is what a developer
            // needs, and the default rendering buries both in a sentence.
            let what = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic with a payload of no known type".to_string());
            let at = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "an unknown location".to_string());
            // A best-effort write with no unwrap anywhere: this runs inside a
            // panic, and a second panic here would replace a diagnosable crash
            // with an undiagnosable one.
            let _ = std::fs::write(path(dir), format!("{what}\nat {at}\n"));
        }
        previous(info);
    }));
}

/// What the last run died of, if it died of a panic. Consumed: reading it
/// deletes it, so one crash is reported once rather than at every launch until
/// the next one overwrites it.
pub fn take(dir: &Path) -> Option<String> {
    let p = path(dir);
    let text = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    let mut lines = text.lines();
    let what = lines.next()?.trim().to_string();
    if what.is_empty() {
        return None;
    }
    // The second line is written as `at <file>:<line>`; the "at " is prose for
    // the file and reads as noise inside the parentheses below.
    match lines.next().map(|l| l.trim().trim_start_matches("at ").trim()) {
        Some(at) if !at.is_empty() => Some(format!("{what} ({at})")),
        _ => Some(what),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("carnyx-crashlog-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn nothing_to_report_when_the_last_run_ended_cleanly() {
        let d = dir("clean");
        assert_eq!(take(&d), None);
    }

    #[test]
    fn a_written_panic_is_reported_once_and_then_gone() {
        let d = dir("once");
        std::fs::write(path(&d), "called `Option::unwrap()` on a `None` value\nat src/app.rs:1234\n")
            .unwrap();
        assert_eq!(
            take(&d).as_deref(),
            Some("called `Option::unwrap()` on a `None` value (src/app.rs:1234)")
        );
        // Consumed, so a crash from three drives ago cannot keep reappearing.
        assert_eq!(take(&d), None);
        assert!(!path(&d).exists());
    }

    #[test]
    fn a_truncated_file_still_says_what_it_can() {
        let d = dir("partial");
        std::fs::write(path(&d), "attempt to subtract with overflow\n").unwrap();
        assert_eq!(take(&d).as_deref(), Some("attempt to subtract with overflow"));
    }

    #[test]
    fn an_empty_file_is_not_a_crash_report() {
        let d = dir("empty");
        std::fs::write(path(&d), "\n\n").unwrap();
        assert_eq!(take(&d), None);
    }
}
