//! Startup and frame diagnostics on stderr (#978).
//!
//! A window that comes up **blank grey** — title bar and menu present, nothing drawn — has
//! several possible causes and no way to tell them apart from the outside: the app might be
//! wedged before its first frame, painting frames that the GPU viewport then fails to draw
//! into, or simply never repainting after a resize. This module makes the difference visible.
//!
//! Two levels, so an ordinary run stays silent:
//!
//! - [`warn`] always prints. Reserved for things that are wrong however the app was started —
//!   the GPU viewport failing to install, or no frame reaching the screen at all.
//! - [`log`] prints only under `BEARCAD_LOG`, and traces the startup sequence frame by frame.
//!
//! ```text
//! BEARCAD_LOG=1 cargo run
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Frames whose UI has been built since launch. `0` after a few seconds means the app never
/// got as far as drawing — a different fault from drawing something that looks empty.
static FRAMES: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_FIRED: AtomicBool = AtomicBool::new(false);

/// How long to wait for the first frame before saying so. Generous: a cold start compiles
/// shaders and probes the GPU.
const FIRST_FRAME_GRACE_SECS: u64 = 8;

/// How many frames to trace in full. The interesting part of a blank-window report is the
/// first handful — after that a working app just repeats itself.
const TRACED_FRAMES: u64 = 5;

/// Whether `BEARCAD_LOG` asks for the trace. Any value but the usual negatives turns it on.
pub fn enabled() -> bool {
    match std::env::var("BEARCAD_LOG") {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "off" | "false" | "no"),
        Err(_) => false,
    }
}

/// Trace a step of the startup sequence. Silent unless `BEARCAD_LOG` is set.
pub fn log(message: impl std::fmt::Display) {
    if enabled() {
        eprintln!("bearcad: {message}");
    }
}

/// Report something wrong. Always printed — a user seeing a broken window shouldn't have to
/// know about an environment variable to find out why.
pub fn warn(message: impl std::fmt::Display) {
    eprintln!("bearcad: warning: {message}");
}

/// Count a frame whose UI was built, and trace the first few with the size they were built at.
///
/// A run that prints frames but shows nothing is a *painting* problem; a run that prints none
/// is a *scheduling* one. That is the distinction the blank-window report needs.
pub fn frame(size: (f32, f32), gpu_viewport: bool) {
    let n = FRAMES.fetch_add(1, Ordering::Relaxed);
    if n < TRACED_FRAMES {
        log(format!(
            "frame {} — {:.0}×{:.0}, 3D viewport {}",
            n + 1,
            size.0,
            size.1,
            if gpu_viewport { "on" } else { "OFF (CPU fallback)" }
        ));
    }
}

/// Whether any frame has been built yet.
pub fn frames_drawn() -> u64 {
    FRAMES.load(Ordering::Relaxed)
}

/// Report something once per process, however many times it happens. For a per-frame fault
/// there is no point in a line per frame — the first one is the whole message.
pub fn warn_once(slot: &'static AtomicBool, message: impl std::fmt::Display) {
    if !slot.swap(true, Ordering::SeqCst) {
        warn(message);
    }
}

/// Watch a launch that comes up blank (#978).
///
/// egui is **reactive**: it draws on input and on request, not continuously. So a window can
/// legitimately sit unpainted — but not at startup, and not for seconds. Two different faults
/// look identical from outside the window, and this is what tells them apart:
///
/// - **No frame at all** — the app never got as far as drawing. It is wedged before its first
///   frame, or nothing ever asked it to paint.
/// - **Frames, but very few** — it drew and then stopped. On macOS the launch sequence resizes
///   the window a beat after opening, and a reactive app that doesn't repaint behind that
///   resize leaves a correctly sized, never-redrawn surface.
///
/// Silence means it kept drawing, which points at presentation rather than scheduling.
#[cfg(not(target_arch = "wasm32"))]
pub fn watch_first_frame() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(FIRST_FRAME_GRACE_SECS));
        if WATCHDOG_FIRED.swap(true, Ordering::SeqCst) {
            return;
        }
        match frames_drawn() {
            0 => warn(format!(
                "no frame drawn {FIRST_FRAME_GRACE_SECS}s after launch — the window will look \
                 blank. The app is running but never asked to paint, or is wedged before its \
                 first frame. Re-run with BEARCAD_LOG=1 for the startup trace."
            )),
            // The launch sequence itself accounts for a handful. Stopping there means the app
            // drew, then stopped being asked to — the window shows whatever the surface held.
            n if n <= LAUNCH_FRAMES_EXPECTED => warn(format!(
                "only {n} frame(s) drawn in {FIRST_FRAME_GRACE_SECS}s — the window may look \
                 blank or stale. Drawing stopped after launch. Re-run with BEARCAD_LOG=1 for \
                 the startup trace."
            )),
            _ => log("watchdog: frames are being drawn"),
        }
    });
}

/// Frames a quiet launch is expected to draw: the opening frames plus the ones the deferred
/// maximize adds. Drawing no more than this in eight seconds means it stopped.
const LAUNCH_FRAMES_EXPECTED: u64 = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_switch_reads_the_usual_negatives_as_off() {
        // The switch has to be unsurprising: `BEARCAD_LOG=0` in a shell profile shouldn't
        // silently turn the trace *on* just by being set.
        for off in ["", "0", "off", "false", "no", "OFF", " 0 "] {
            unsafe { std::env::set_var("BEARCAD_LOG", off) };
            assert!(!enabled(), "{off:?} should read as off");
        }
        for on in ["1", "true", "yes", "trace"] {
            unsafe { std::env::set_var("BEARCAD_LOG", on) };
            assert!(enabled(), "{on:?} should read as on");
        }
        unsafe { std::env::remove_var("BEARCAD_LOG") };
        assert!(!enabled(), "unset is off");
    }

    #[test]
    fn frames_are_counted_so_a_blank_launch_is_distinguishable() {
        let before = frames_drawn();
        frame((800.0, 600.0), true);
        assert_eq!(
            frames_drawn(),
            before + 1,
            "the watchdog's whole question is whether this ever happens"
        );
    }
}
