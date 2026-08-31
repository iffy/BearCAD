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

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Frames whose UI has been built since launch. `0` after a few seconds means the app never
/// got as far as drawing — a different fault from drawing something that looks empty.
static FRAMES: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_FIRED: AtomicBool = AtomicBool::new(false);
/// egui multipass id-instability warnings this process has logged (#1614 / #1211).
static WIDGET_ID_CHANGE_WARNINGS: AtomicU64 = AtomicU64::new(0);

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
static STARTED: OnceLock<crate::time::Instant> = OnceLock::new();

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
            let _ = STARTED.set(crate::time::Instant::now());
            let _ = FILE.set(Mutex::new(file));
            let _ = FILE_PATH.set(path.clone());
            write_line("---", header);
            // On stderr too, so the terminal says where to look before anything goes wrong.
            eprintln!("bearcad: logging to {}", path.display());
        }
        Err(err) => warn(format!("cannot write the log file {}: {err}", path.display())),
    }
}

/// How much of the run to keep in memory (#1654). Enough that a long working session still
/// reaches back past whatever went wrong, and small enough to carry without thinking about it.
const RECENT_LINES: usize = 5000;

/// The run's own log, in memory: what a bug report attaches, and what
/// `bearcad.session_log()` reads back. Kept separately from the file because the file is
/// optional — a run that couldn't open one still has a session worth describing.
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// Lines pushed out of [`RECENT`] by newer ones, so the read-back can say so.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Append one line to the run's log — in memory always, and to the file if there is one.
fn write_line(level: &str, message: impl std::fmt::Display) {
    let elapsed = STARTED
        .get()
        .map(|t| t.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    let line = format!("[{elapsed:8.3}] {level:<5} {message}");
    if let Ok(mut recent) = RECENT.lock() {
        recent.push_back(line.clone());
        while recent.len() > RECENT_LINES {
            recent.pop_front();
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
    let Some(file) = FILE.get() else { return };
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

/// What this run has done so far, newest last (#1654) — the log without going to disk.
///
/// This is what a DEV bug report attaches: a report that says "it went wrong" is worth far
/// more with the sequence that led there underneath it.
pub fn session_log() -> String {
    let mut out = String::new();
    let dropped = DROPPED.load(Ordering::Relaxed);
    if dropped > 0 {
        out.push_str(&format!("… earlier lines dropped ({dropped})\n"));
    }
    if let Ok(recent) = RECENT.lock() {
        for line in recent.iter() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
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
    let message = message.to_string();
    if message.contains("changed id between passes") {
        WIDGET_ID_CHANGE_WARNINGS.fetch_add(1, Ordering::Relaxed);
    }
    // An egui id clash reaches us as a rect and two hashes; on its own nobody can act on it,
    // so the line carries what the app was doing when it happened (#1823).
    let message = annotate_warning(&message).unwrap_or(message);
    eprintln!("bearcad: warning: {message}");
    write_line("WARN", &message);
}

/// What the app was doing when a log line was written (#1823).
///
/// An egui id-clash warning on its own is a rect and two hashes — nothing anyone can act on.
/// The app republishes this every frame so a warning can say *when* it happened, what the app
/// was in the middle of, and which part of the window the rect it names actually lies in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiContext {
    pub frame: u64,
    /// Workbench script name ("model" | "sketch" | "drawing" | "view").
    pub workbench: &'static str,
    /// Active tool's script name.
    pub tool: &'static str,
    /// Named regions of the window as `(name, [min_x, min_y, max_x, max_y])`, so a warning
    /// carrying a rect can name the pane it landed in.
    pub regions: Vec<(String, [f32; 4])>,
    /// Windows and dialogs open at the time.
    pub windows: Vec<String>,
    /// What was selected, in one line (#1828). Half of what a pane draws depends on it, so
    /// a warning that doesn't say it can't be reproduced.
    pub selection: String,
}

/// Named rows a pane drew this frame, as `(label, [min_x, min_y, max_x, max_y])` (#1828).
///
/// "In the Context pane" is fifty widgets; the row is one. Rows publish themselves as they
/// are built, and the id-clash warning names the innermost one its rect lies inside.
static UI_ROWS: Mutex<Vec<(String, [f32; 4])>> = Mutex::new(Vec::new());

/// Record a labelled row at `rect`, for a warning later this frame to name.
pub fn note_ui_row(label: impl Into<String>, rect: [f32; 4]) {
    let label = label.into();
    if label.is_empty() {
        return;
    }
    if let Ok(mut rows) = UI_ROWS.lock() {
        // A pane can rebuild several times a frame (egui multipass, detached windows); a
        // cap keeps a runaway from growing without bound.
        if rows.len() < 512 {
            rows.push((label, rect));
        }
    }
}

static UI_CONTEXT: Mutex<Option<UiContext>> = Mutex::new(None);

/// Publish what the app is doing, for any warning logged from here on (#1823). Called once
/// per frame, which is also where the rows collected for the last one are dropped.
pub fn set_ui_context(context: UiContext) {
    if let Ok(mut rows) = UI_ROWS.lock() {
        rows.clear();
    }
    if let Ok(mut slot) = UI_CONTEXT.lock() {
        *slot = Some(context);
    }
}

/// Does `rect` sit inside `region` (with half a pixel of slack)?
fn inside(rect: [f32; 4], region: [f32; 4]) -> bool {
    rect[0] >= region[0] - 0.5
        && rect[1] >= region[1] - 0.5
        && rect[2] <= region[2] + 0.5
        && rect[3] <= region[3] + 0.5
}

fn area(r: [f32; 4]) -> f32 {
    (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0)
}

/// The `[[x y] - [x y]]` rect an egui widget warning names.
fn warned_rect(message: &str) -> Option<[f32; 4]> {
    let open = message.find("[[")?;
    let close = message[open..].find("]]")? + open;
    let numbers: Vec<f32> = message[open..close + 2]
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    (numbers.len() == 4).then(|| [numbers[0], numbers[1], numbers[2], numbers[3]])
}

/// Turn an egui id-clash warning into a line someone can file (#1823): the bare rect and
/// hashes, plus the frame, the workbench and tool the app was in, which pane the rect lies in,
/// and what else was open. `None` for every other warning — they are logged as they came.
pub fn annotate_warning(message: &str) -> Option<String> {
    if !message.contains("changed id between passes") {
        return None;
    }
    let context = UI_CONTEXT.lock().ok()?.clone()?;
    let mut parts = vec![format!("frame {}", context.frame)];
    if !context.workbench.is_empty() {
        parts.push(format!("{} workbench", context.workbench));
    }
    if !context.tool.is_empty() {
        parts.push(format!("{} tool", context.tool));
    }
    if let Some(rect) = warned_rect(message) {
        // The *innermost* region: a combo popup or a tooltip drawn over a pane is the thing
        // to name, not the pane it happens to cover (#1828).
        let region = context
            .regions
            .iter()
            .filter(|(_, r)| inside(rect, *r))
            .min_by(|a, b| area(a.1).total_cmp(&area(b.1)));
        parts.push(match region {
            Some((name, _)) => format!("in the {name}"),
            None => "not inside any named pane".to_string(),
        });
        // And the row inside it, which is the widget someone has to go and look at.
        if let Ok(rows) = UI_ROWS.lock() {
            if let Some((label, _)) = rows
                .iter()
                .filter(|(_, r)| inside(rect, *r))
                .min_by(|a, b| area(a.1).total_cmp(&area(b.1)))
            {
                parts.push(format!("in the {label:?} row"));
            }
        }
    }
    if !context.selection.is_empty() {
        parts.push(format!("selected: {}", context.selection));
    }
    if !context.windows.is_empty() {
        parts.push(format!("open: {}", context.windows.join(", ")));
    }
    Some(format!(
        "{message} — {} (please report this line at {ISSUE_URL})",
        parts.join(", ")
    ))
}

/// Where a warning tells the reader to take it.
const ISSUE_URL: &str = "https://github.com/iffy/BearCAD/issues";

/// How many egui "Widget rect … changed id between passes" warnings this process has logged.
///
/// Scripts read this via `bearcad.debug.widget_id_warnings()` so a layout that flashes red
/// fails the interaction test that produced it (#1614).
pub fn widget_id_change_warnings() -> u64 {
    WIDGET_ID_CHANGE_WARNINGS.load(Ordering::Relaxed)
}

/// How much of an action's payload a log line carries (#1654).
const ACTION_DETAIL_MAX: usize = 160;

/// An action and what it acted on, for the log (#1023/#1654): `SetTool(Dimension)`, not a
/// bare `SetTool`. Someone reading a session back to describe a bug has to be able to tell
/// one of a kind from another, which the variant alone can't do. Cut short at
/// [`ACTION_DETAIL_MAX`] so the actions that carry whole meshes still cost one line.
pub fn action_detail(action: &impl std::fmt::Debug) -> String {
    use std::fmt::Write as _;
    // A sink that gives up once it has enough. An action carrying an imported mesh would
    // otherwise format millions of vertices only to throw all but the first line away.
    struct Clip {
        out: String,
        chars: usize,
    }
    impl std::fmt::Write for Clip {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for c in s.chars() {
                // Debug of a nested struct can run over several lines; a log line is one line.
                let c = if c == '\n' || c == '\t' { ' ' } else { c };
                if self.out.ends_with(' ') && c == ' ' {
                    continue;
                }
                if self.chars == ACTION_DETAIL_MAX {
                    // Stops the Debug impl mid-write: the caller adds the ellipsis.
                    return Err(std::fmt::Error);
                }
                self.out.push(c);
                self.chars += 1;
            }
            Ok(())
        }
    }
    let mut clip = Clip { out: String::new(), chars: 0 };
    if write!(clip, "{action:?}").is_err() {
        clip.out.push('…');
    }
    clip.out
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

    /// #1654: the run keeps its own log in memory, so what the user did can be read back
    /// (and attached to a bug report) whether or not a log file was ever opened.
    #[test]
    fn the_session_log_reads_back_what_the_run_did() {
        let marker = format!("marker-{}", std::process::id());
        log(&marker);
        info(format!("{marker} refused: nope"));
        let session = session_log();
        assert!(session.contains(&marker), "the session log keeps what was logged");
        assert!(
            session.contains("trace") && session.contains("info"),
            "with the level of each line: {session}"
        );
    }

    /// #1654: the buffer is bounded — a long session drops its oldest lines and says so,
    /// rather than growing without limit.
    #[test]
    fn the_session_log_drops_its_oldest_lines() {
        for i in 0..(RECENT_LINES + 50) {
            log(format!("filler-{i}"));
        }
        let session = session_log();
        assert!(
            session.lines().count() <= RECENT_LINES + 1,
            "bounded, got {} lines",
            session.lines().count()
        );
        assert!(session.starts_with("… earlier lines dropped"), "{session:.80}");
        assert!(session.contains(&format!("filler-{}", RECENT_LINES + 49)), "keeps the newest");
    }

    /// #1654: a log line has to say *which* thing was done — "SetTool" alone can't tell a
    /// report reader whether the user picked the Dimension tool or the Rectangle one.
    #[test]
    fn an_action_line_names_what_it_acted_on() {
        #[derive(Debug)]
        #[allow(dead_code)]
        enum Sample {
            Plain,
            Tuple(u32),
            Struct { path: String },
        }
        assert_eq!(action_detail(&Sample::Plain), "Plain");
        assert_eq!(action_detail(&Sample::Tuple(7)), "Tuple(7)");
        assert_eq!(
            action_detail(&Sample::Struct { path: "a.bearcad".into() }),
            "Struct { path: \"a.bearcad\" }"
        );
        // Actions carrying whole meshes are cut short rather than flooding the log.
        let long = Sample::Struct { path: "x".repeat(1000) };
        let line = action_detail(&long);
        assert!(
            line.chars().count() <= ACTION_DETAIL_MAX + 1,
            "cut to length, got {}",
            line.chars().count()
        );
        assert!(line.ends_with('…'), "and says it was cut: {line:.40}");
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

    /// The published UI context is process-wide, so the tests that set it take turns.
    static UI_CONTEXT_TESTS: Mutex<()> = Mutex::new(());

    /// #1823: an egui id-clash warning on its own says nothing anyone can act on — a rect and
    /// two hashes. The line has to carry what the app was doing and which part of the window
    /// the rect landed in, or it can't be turned into a bug report.
    #[test]
    fn a_widget_id_warning_says_where_and_when_it_happened() {
        let _turn = UI_CONTEXT_TESTS.lock();
        set_ui_context(UiContext {
            frame: 512,
            workbench: "drawing",
            tool: "select",
            regions: vec![
                ("Elements pane".into(), [0.0, 40.0, 260.0, 800.0]),
                ("Context pane".into(), [660.0, 40.0, 900.0, 800.0]),
            ],
            windows: vec!["main".into(), "report_issue".into()],
            selection: String::new(),
        });
        let line = annotate_warning(
            "egui::context: Widget rect [[673.8 187.3] - [699.8 207.3]] changed id between \
             passes: prev ids: [\"3982\"], new ids: [\"CFD1\"]",
        )
        .expect("an id-clash warning is annotated");
        assert!(line.contains("frame 512"), "when it happened: {line}");
        assert!(line.contains("drawing"), "which workbench: {line}");
        assert!(line.contains("select"), "which tool: {line}");
        assert!(line.contains("Context pane"), "which part of the window: {line}");
        assert!(!line.contains("Elements pane"), "and not one the rect misses: {line}");
        assert!(line.contains("report_issue"), "what else was open: {line}");

        // A rect in no named region still gets the rest of the context.
        let elsewhere = annotate_warning(
            "egui::context: Widget rect [[400.0 900.0] - [420.0 920.0]] changed id between passes",
        )
        .expect("annotated");
        assert!(elsewhere.contains("frame 512"), "{elsewhere}");

        // Other warnings are left exactly as they came.
        assert_eq!(annotate_warning("wgpu: surface lost"), None);
        set_ui_context(UiContext::default());
    }

    /// #1828: "in the Context pane" was as far as the line went, and a pane is fifty
    /// widgets. The row a rect lies in, and what was selected, are what turn the warning
    /// into something reproducible — and an overlay drawn *over* a pane must not be
    /// reported as the pane.
    #[test]
    fn a_widget_id_warning_names_the_row_and_the_selection() {
        let _turn = UI_CONTEXT_TESTS.lock();
        set_ui_context(UiContext {
            frame: 900,
            workbench: "drawing",
            tool: "select",
            regions: vec![("Context pane".into(), [660.0, 40.0, 900.0, 800.0])],
            windows: vec!["main".into()],
            selection: "drawing_view 1 (aligned)".into(),
        });
        note_ui_row("Style", [664.0, 180.0, 896.0, 200.0]);
        note_ui_row("Projection lines", [664.0, 210.0, 896.0, 228.0]);

        let line = annotate_warning(
            "egui::context: Widget rect [[670.0 212.0] - [856.0 226.0]] changed id between \
             passes: prev ids: [\"D3AC\"], new ids: [\"300F\"]",
        )
        .expect("annotated");
        assert!(line.contains("Projection lines"), "the row the rect lies in: {line}");
        assert!(!line.contains("Style"), "not a row it misses: {line}");
        assert!(line.contains("drawing_view 1 (aligned)"), "what was selected: {line}");

        // An overlay (a combo popup, a tooltip) covering part of a pane is the smaller
        // region, and the one worth naming.
        set_ui_context(UiContext {
            frame: 901,
            workbench: "drawing",
            tool: "select",
            regions: vec![
                ("Context pane".into(), [660.0, 40.0, 900.0, 800.0]),
                ("drawing_view_style popup".into(), [664.0, 170.0, 890.0, 300.0]),
            ],
            windows: vec!["main".into()],
            selection: String::new(),
        });
        let line = annotate_warning(
            "egui::context: Widget rect [[670.0 200.0] - [856.0 220.0]] changed id between passes",
        )
        .expect("annotated");
        assert!(line.contains("popup"), "the innermost region wins: {line}");
        set_ui_context(UiContext::default());
    }

    /// #1614: the egui multipass id-clash line is counted so a script can fail on it.
    #[test]
    fn widget_id_change_warnings_count_the_egui_multipass_line() {
        let before = widget_id_change_warnings();
        warn("something else");
        assert_eq!(widget_id_change_warnings(), before, "unrelated warnings stay out");
        warn("egui::context: Widget rect [[0.0 73.0] - [220.0 768.0]] changed id between passes: prev ids: [\"0A96\"], new ids: [\"81FE\"]");
        assert_eq!(
            widget_id_change_warnings(),
            before + 1,
            "the egui line should increment the counter"
        );
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
