//! Diagnostics: what the app is doing, on stderr and on disk (#978/#1023).
//!
//! A window that comes up **blank grey** — title bar and menu present, nothing drawn — has
//! several possible causes and no way to tell them apart from the outside: the app might be
//! wedged before its first frame, painting frames that the GPU viewport then fails to draw
//! into, or simply never repainting after a resize. This module makes the difference visible,
//! and does the same for everything else that goes wrong later.
//!
//! Three levels, so a terminal shows what happened without showing everything:
//!
//! - [`warn`] — something is wrong. Always on stderr.
//! - [`info`] — something notable happened: a document opened, an import landed, an action
//!   refused. Always on stderr, so `cargo run` narrates the session.
//! - [`log`] — the fine-grained trace. On stderr only under `BEARCAD_LOG`, because it is far
//!   too much to read past; **always** in the file.
//!
//! Every level is written to a **log file** as well ([`init`]), so a problem can be debugged
//! after the fact rather than only while watching. The file is not gated on anything: by the
//! time you know you wanted logging, the run that broke is over.
//!
//! ```text
//! cargo run                 # warnings + notable events on stderr, everything in the file
//! BEARCAD_LOG=1 cargo run   # the full trace on stderr too
//! ```

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

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

/// Whether `BEARCAD_LOG` asks for the full trace **on stderr**. Any value but the usual
/// negatives turns it on. The file gets the trace either way.
pub fn enabled() -> bool {
    match std::env::var("BEARCAD_LOG") {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "off" | "false" | "no"),
        Err(_) => false,
    }
}

/// The open log file, once [`init`] has opened one. Absent in tests and in the catalog
/// subprocess, which is what keeps a `cargo test` run from writing thousands of lines to
/// disk — nothing calls `init`, so nothing has a file.
static FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static FILE_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Seconds since [`init`], for the timestamp on each line.
static STARTED: OnceLock<std::time::Instant> = OnceLock::new();

/// Where the log is written: `$BEARCAD_LOG_FILE` if set, else `bearcad.log` in the system
/// temp directory. Somewhere predictable and always writable beats somewhere tidy — this file
/// exists to be found in a hurry.
pub fn default_log_path() -> PathBuf {
    match std::env::var_os("BEARCAD_LOG_FILE") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::temp_dir().join("bearcad").join("bearcad.log"),
    }
}

/// The log file in use, once opened.
pub fn log_path() -> Option<&'static Path> {
    FILE_PATH.get().map(PathBuf::as_path)
}

/// Start logging to disk, keeping the previous run's log beside it (#1023).
///
/// **Two files, not a rotation series.** The log you want is almost always from the run that
/// just misbehaved, and the one before it when the app died on startup and you restarted it to
/// look. A third is archaeology.
///
/// Failing to open the file is not worth interrupting a launch over: stderr still works, and
/// the reason is reported there.
pub fn init(path: PathBuf, header: impl std::fmt::Display) {
    if let Some(dir) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(dir) {
            warn(format!("cannot create the log directory {}: {err}", dir.display()));
            return;
        }
    }
    // Keep the previous run's log as `.prev` — an app that dies at startup is one you restart
    // immediately, which would otherwise overwrite the evidence.
    if path.exists() {
        let _ = std::fs::rename(&path, path.with_extension("prev.log"));
    }
    match std::fs::File::create(&path) {
        Ok(file) => {
            let _ = STARTED.set(std::time::Instant::now());
            let _ = FILE.set(Mutex::new(file));
            let _ = FILE_PATH.set(path.clone());
            write_line("---", header);
            // On stderr too, so the terminal says where to look before anything goes wrong.
            eprintln!("bearcad: logging to {}", path.display());
        }
        Err(err) => warn(format!("cannot write the log file {}: {err}", path.display())),
    }
}

/// Append one line to the log file, if there is one.
fn write_line(level: &str, message: impl std::fmt::Display) {
    let Some(file) = FILE.get() else { return };
    let elapsed = STARTED
        .get()
        .map(|t| t.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "[{elapsed:8.3}] {level:<5} {message}");
        let _ = file.flush();
    }
}

/// Trace a step in detail. On stderr only under `BEARCAD_LOG`; always in the file.
pub fn log(message: impl std::fmt::Display) {
    if enabled() {
        eprintln!("bearcad: {message}");
    }
    write_line("trace", message);
}

/// Report something notable — a document opened, an import landed, an action refused (#1023).
/// Always printed, so a terminal running the app narrates the session.
pub fn info(message: impl std::fmt::Display) {
    eprintln!("bearcad: {message}");
    write_line("info", message);
}

/// Report something wrong. Always printed — a user seeing a broken window shouldn't have to
/// know about an environment variable to find out why.
pub fn warn(message: impl std::fmt::Display) {
    eprintln!("bearcad: warning: {message}");
    write_line("WARN", message);
}

/// A short name for an action, for the log (#1023): its variant, without the payload.
///
/// `{:?}` on the whole action would be unreadable — some carry entire meshes — and the
/// variant is what a trace is actually read for: the sequence of what was done.
pub fn action_label(action: &impl std::fmt::Debug) -> String {
    let text = format!("{action:?}");
    let end = text
        .find(|c: char| c == '(' || c == '{' || c == ' ')
        .unwrap_or(text.len());
    text[..end].to_string()
}

/// Send panics to the log as well as to stderr (#1023). A panic is exactly the failure you
/// cannot reproduce on demand, so it is the one most worth having on disk — along with
/// everything the run did beforehand, which is already there above it.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        write_line("PANIC", format!("{payload} — at {location}"));
        if let Some(path) = log_path() {
            eprintln!("bearcad: the log for this run is at {}", path.display());
        }
        previous(info);
    }));
}

/// Count a frame whose UI was built, and trace the first few with the size they were built at.
///
/// A run that prints frames but shows nothing is a *painting* problem; a run that prints none
/// is a *scheduling* one. That is the distinction the blank-window report needs.
/// The last painted size, quantized to whole logical pixels and packed as `(w << 32) | h`,
/// so a change is a single atomic compare. `u64::MAX` until the first frame.
static LAST_FRAME_SIZE: AtomicU64 = AtomicU64::new(u64::MAX);
/// Size changes reported so far, capped by [`TRACED_RESIZES`] — a live drag-resize would
/// otherwise write a line per frame.
static RESIZES_LOGGED: AtomicU64 = AtomicU64::new(0);
const TRACED_RESIZES: u64 = 24;

/// Frames whose GPU viewport blit did not happen. A window that looks grey while frames are
/// being built is asking exactly this question, and one warning at the first failure cannot
/// say whether it then recovered.
static FRAMES_GPU_MISSED: AtomicU64 = AtomicU64::new(0);

/// What the window believed about itself on the most recent frame, for [`watch_first_frame`]
/// to report. The watchdog runs on its own thread and cannot touch egui, so the UI thread
/// leaves this behind for it (#1032).
static WINDOW_STATE: Mutex<Option<String>> = Mutex::new(None);

/// Record the GPU viewport failing to paint a frame it built.
pub fn gpu_blit_missed() {
    FRAMES_GPU_MISSED.fetch_add(1, Ordering::Relaxed);
}

/// Publish what the window says about itself this frame — size, scale, and whatever state
/// flags the platform reports. Cheap enough to call every frame; only read on failure.
pub fn note_window_state(state: impl std::fmt::Display) {
    if let Ok(mut slot) = WINDOW_STATE.lock() {
        *slot = Some(state.to_string());
    }
}

/// What the **window server** says, as opposed to what egui believes (#1032). Separate
/// because it answers a different question — whether the window is on screen at all — and
/// because it is sampled less often.
static PLATFORM_WINDOW_STATE: Mutex<Option<String>> = Mutex::new(None);

pub fn note_platform_window_state(state: impl std::fmt::Display) {
    if let Ok(mut slot) = PLATFORM_WINDOW_STATE.lock() {
        *slot = Some(state.to_string());
    }
}

pub fn frame(size: (f32, f32), gpu_viewport: bool) {
    // A window that maximizes after launch, or one whose surface never follows a resize,
    // both look like "frame 1 — 960×640" and then silence. Report the size *changing*, not
    // just the first few frames (#1032).
    let packed = ((size.0.round().max(0.0) as u64) << 32) | (size.1.round().max(0.0) as u64);
    let previous = LAST_FRAME_SIZE.swap(packed, Ordering::Relaxed);
    if previous != packed && previous != u64::MAX {
        let n = RESIZES_LOGGED.fetch_add(1, Ordering::Relaxed);
        if n < TRACED_RESIZES {
            info(format!(
                "frame size {}×{} → {:.0}×{:.0}",
                previous >> 32,
                previous & 0xffff_ffff,
                size.0,
                size.1
            ));
        } else if n == TRACED_RESIZES {
            log("frame size changes: further ones not traced");
        }
    }
    let n = FRAMES.fetch_add(1, Ordering::Relaxed);
    if n < TRACED_FRAMES {
        let line = format!(
            "frame {} — {:.0}×{:.0}, 3D viewport {}",
            n + 1,
            size.0,
            size.1,
            if gpu_viewport { "on" } else { "OFF (CPU fallback)" }
        );
        // The **first** frame is a milestone worth seeing without asking: a terminal that
        // stops before it says the window never painted, which is the whole question a grey
        // window asks (#1023). The rest are trace.
        if n == 0 { info(line) } else { log(line) }
    }
}

/// Whether any frame has been built yet.
pub fn frames_drawn() -> u64 {
    FRAMES.load(Ordering::Relaxed)
}

/// Forward `log` crate records — wgpu's, winit's, eframe's — into this module (#1032).
///
/// Nothing installed a logger before, so everything those crates had to say was dropped on
/// the floor. That is precisely the wrong thing to discard when a window comes up grey: a
/// failed surface acquisition, a lost or outdated swapchain, a surface reconfigured to a
/// size the window no longer has, are all reported there and nowhere else.
///
/// `Warn` and above by default, since wgpu at `Info` narrates every resource it makes.
/// `BEARCAD_GPU_LOG=1` opens it up to `Debug` for a run that needs the whole conversation.
struct LogBridge;

impl ::log::Log for LogBridge {
    fn enabled(&self, metadata: &::log::Metadata<'_>) -> bool {
        metadata.level() <= ::log::max_level()
    }

    fn log(&self, record: &::log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("{}: {}", record.target(), record.args());
        match record.level() {
            ::log::Level::Error | ::log::Level::Warn => warn(line),
            // Info from these crates is trace as far as BearCAD is concerned — it is about
            // adapters and buffers, not about what the user did.
            _ => log(line),
        }
    }

    fn flush(&self) {
        // Nothing to do: `write_line` flushes the file on every record, precisely so a crash
        // cannot swallow the line that explains it.
    }
}

/// Route wgpu/winit/eframe logging into the diagnostics file. Call once, early.
pub fn install_log_bridge() {
    let verbose = matches!(
        std::env::var("BEARCAD_GPU_LOG").as_deref().map(str::trim),
        Ok("1") | Ok("on") | Ok("true") | Ok("yes")
    );
    let level = if verbose {
        ::log::LevelFilter::Debug
    } else {
        ::log::LevelFilter::Warn
    };
    if ::log::set_logger(&LogBridge).is_ok() {
        ::log::set_max_level(level);
    }
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
                 first frame.{}",
                where_to_look()
            )),
            // The launch sequence itself accounts for a handful. Stopping there means the app
            // drew, then stopped being asked to — the window shows whatever the surface held.
            n if n <= LAUNCH_FRAMES_EXPECTED => warn(format!(
                "only {n} frame(s) drawn in {FIRST_FRAME_GRACE_SECS}s — the window may look \
                 blank or stale. Drawing stopped after launch.{}",
                where_to_look()
            )),
            // A verdict either way, on stderr: at this point the interesting question is
            // whether the app is drawing at all, and only one of the two answers being
            // visible leaves the other looking like silence (#1023).
            // A verdict, plus everything the UI thread knows that the outside of the window
            // does not (#1032). "It is painting" alone leaves the next question — *what* is
            // it painting, and at what size — with nowhere to look.
            n => info(format!(
                "watchdog: {n} frames drawn — the app is painting. A window that still looks \
                 blank is a presentation fault, not a scheduling one.{}",
                presentation_snapshot()
            )),
        }
    });
}

/// Everything the UI thread knows about how the last frame reached the screen: the size it
/// painted at, whether the GPU viewport blit landed, and what the window says about itself.
/// Appended to the watchdog's verdict, where "it is painting" is otherwise a dead end.
fn presentation_snapshot() -> String {
    let mut out = String::new();
    let packed = LAST_FRAME_SIZE.load(Ordering::Relaxed);
    if packed != u64::MAX {
        out.push_str(&format!(
            " Last frame {}×{}.",
            packed >> 32,
            packed & 0xffff_ffff
        ));
    }
    let missed = FRAMES_GPU_MISSED.load(Ordering::Relaxed);
    let drawn = FRAMES.load(Ordering::Relaxed);
    if missed == 0 {
        out.push_str(" The 3D viewport blit landed on every frame.");
    } else if missed >= drawn {
        out.push_str(&format!(
            " The 3D viewport blit failed on all {missed} frame(s) — the viewport is \
             building scenes it never draws."
        ));
    } else {
        out.push_str(&format!(
            " The 3D viewport blit failed on {missed} of {drawn} frame(s)."
        ));
    }
    if let Some(state) = WINDOW_STATE.lock().ok().and_then(|s| s.clone()) {
        out.push_str(&format!(" Window: {state}."));
    }
    if let Some(state) = PLATFORM_WINDOW_STATE.lock().ok().and_then(|s| s.clone()) {
        out.push_str(&format!(" {state}."));
    }
    out.push_str(&where_to_look());
    out
}

/// Where the whole story is, for a message that has only told part of it.
fn where_to_look() -> String {
    match log_path() {
        Some(path) => format!(" The full trace for this run is in {}.", path.display()),
        None => " Re-run with BEARCAD_LOG=1 for the startup trace.".to_string(),
    }
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

    /// #1023: a log line names the action, not its payload — some actions carry whole
    /// meshes, and what a trace is read for is the *sequence* of what was done.
    #[test]
    fn an_action_logs_its_name_without_its_payload() {
        #[derive(Debug)]
        #[allow(dead_code)]
        enum Sample {
            Plain,
            Tuple(u32, Vec<u8>),
            Struct { path: String },
        }
        assert_eq!(action_label(&Sample::Plain), "Plain");
        assert_eq!(action_label(&Sample::Tuple(7, vec![1, 2, 3])), "Tuple");
        assert_eq!(
            action_label(&Sample::Struct { path: "/tmp/x".into() }),
            "Struct"
        );
    }

    /// #1023: the log goes somewhere predictable, and `BEARCAD_LOG_FILE` moves it — a run
    /// whose log you can't find is a run you can't debug.
    #[test]
    fn the_log_file_has_a_findable_default_and_an_override() {
        unsafe { std::env::remove_var("BEARCAD_LOG_FILE") };
        let default = default_log_path();
        assert!(default.starts_with(std::env::temp_dir()), "under temp: {default:?}");
        assert_eq!(default.file_name().unwrap(), "bearcad.log");

        unsafe { std::env::set_var("BEARCAD_LOG_FILE", "/tmp/somewhere-else.log") };
        assert_eq!(default_log_path(), std::path::PathBuf::from("/tmp/somewhere-else.log"));
        // An empty value is as good as unset, rather than a log written to "".
        unsafe { std::env::set_var("BEARCAD_LOG_FILE", "") };
        assert_eq!(default_log_path().file_name().unwrap(), "bearcad.log");
        unsafe { std::env::remove_var("BEARCAD_LOG_FILE") };
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
