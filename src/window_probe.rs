//! What AppKit says about the app's window (#1032).
//!
//! A grey launch that the app itself cannot see is the hard case: frames are built at the
//! right size, the viewport's blit lands, the window reports itself maximized and not
//! minimized — and the screen still shows the OS window background rather than the app's
//! own dark canvas. Everything egui can tell us is already healthy, so the remaining
//! questions are ones only the window server can answer: is the window on screen at all, is
//! it occluded, is it transparent, does its backing layer have a real size?

use std::sync::Mutex;

// `diag` is a sibling module; bring it in so the launch-settle trace lines below reach it
// without a `crate::` prefix on every call.
use crate::diag;

/// A short one-line summary of what AppKit currently sees, for the launch-settle trace
/// (#1108). This is sampled every frame during the settle and logged only on change, so the
/// log shows a narrative of state transitions (active, occluded, visible) instead of one
/// line per frame. `None` off macOS (or when called off the main thread).
#[cfg(target_os = "macos")]
fn short_appkit_summary() -> Option<String> {
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSWindowOcclusionState,
    };
    use objc2::MainThreadMarker;

    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    let total = windows.count();
    let visible = windows.iter().filter(|w| w.isVisible()).count();
    let occluded = windows
        .iter()
        .any(|w| !w.occlusionState().contains(NSWindowOcclusionState::Visible));
    let policy = match app.activationPolicy() {
        NSApplicationActivationPolicy::Regular => "regular",
        NSApplicationActivationPolicy::Accessory => "accessory",
        NSApplicationActivationPolicy::Prohibited => "prohibited",
        _ => "other",
    };
    Some(format!(
        "app active {}, policy {}, {} window(s), {} visible, occluded {}",
        app.isActive(),
        policy,
        total,
        visible,
        occluded,
    ))
}

/// The last summary we logged, so [`log_state_if_changed`] only writes a line when something
/// actually moves. A grey launch where the window stays occluded the whole settle would
/// otherwise produce a thousand identical lines.
#[cfg(target_os = "macos")]
static LAST_LOGGED: Mutex<Option<String>> = Mutex::new(None);

/// The last `activate_app()` outcome we logged, so [`log_activation_outcome`] writes a line
/// only on the true↔false boundary (#1108). A settle that stays refused or stays granted
/// gets one line, not one per frame.
#[cfg(target_os = "macos")]
static LAST_OUTCOME: Mutex<Option<bool>> = Mutex::new(None);

/// Log a change in whether `activate_app` could reach the window server (#1108).
///
/// `activate_app` is called every frame during the settle; whether it succeeded is a single
/// bool that flips at most a couple of times. Logging it on the change (and the first call)
/// turns the trace's "asked the window server" spam into two meaningful lines: when asking
/// started working, and when it stopped.
#[cfg(target_os = "macos")]
pub fn log_activation_outcome(activated: bool) {
    if let Ok(mut slot) = LAST_OUTCOME.lock() {
        if *slot == Some(activated) {
            return;
        }
        diag::log(format!(
            "launch: activate_app {} -> {}",
            slot.map(|b| if b { "true" } else { "false" }).unwrap_or("(none)"),
            activated,
        ));
        *slot = Some(activated);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn log_activation_outcome(_activated: bool) {}

/// Log a change to the activation policy the first time `activate_app` corrects it (#1108).
/// A binary run from the terminal starts with `Prohibited` (or `Accessory`), which cannot
/// become frontmost — that is the root cause of the grey launch for unbundled runs, and a
/// line that says "policy was X, set to regular" is what distinguishes that case from one
/// where the policy was already correct and the window is still occluded for another reason.
#[cfg(target_os = "macos")]
fn log_policy_change(from: objc2_app_kit::NSApplicationActivationPolicy) {
    use objc2_app_kit::NSApplicationActivationPolicy;
    let name = match from {
        NSApplicationActivationPolicy::Regular => "regular",
        NSApplicationActivationPolicy::Accessory => "accessory",
        NSApplicationActivationPolicy::Prohibited => "prohibited",
        _ => "other",
    };
    diag::log(format!("launch: activation policy was {name}, set to regular"));
}

/// Log the AppKit state only when it differs from the last call (#1108).
///
/// During the launch settle this is called every frame; the interesting events are
/// *transitions* (the app becoming active, the window losing occlusion), and a per-frame log
/// of "still occluded, still inactive" hides them behind a wall of identical lines. This
/// writes one line per change, plus the first sample, so the trace reads as a timeline.
#[cfg(target_os = "macos")]
pub fn log_state_if_changed() {
    let Some(summary) = short_appkit_summary() else {
        return;
    };
    if let Ok(mut slot) = LAST_LOGGED.lock() {
        if slot.as_deref() == Some(summary.as_str()) {
            return;
        }
        diag::log(format!("launch: AppKit — {summary}"));
        *slot = Some(summary);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn log_state_if_changed() {}

/// One line describing the app's window as AppKit sees it, or `None` off macOS (and when
/// there is no window yet). Must be called from the main thread — AppKit is not thread-safe,
/// which is why this is sampled on the UI thread and left for the watchdog rather than read
/// from it.
#[cfg(target_os = "macos")]
pub fn appkit_window_state() -> Option<String> {
    use objc2_app_kit::{NSApplication, NSWindowOcclusionState};
    use objc2::MainThreadMarker;

    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    if windows.count() == 0 {
        return Some("AppKit: the app has no windows".to_string());
    }
    // Every window, not just the first: an app can hold more than one, and picking one to
    // report risks describing the wrong thing in exactly the case this exists to diagnose.
    let mut described = Vec::new();
    for (i, window) in windows.iter().enumerate().take(4) {
        let frame = window.frame();
        let occluded = !window
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible);
        let content = window
            .contentView()
            .map(|view| {
                let b = view.bounds();
                format!("{:.0}×{:.0}", b.size.width, b.size.height)
            })
            .unwrap_or_else(|| "no content view".to_string());
        described.push(format!(
            "[{i}] visible {}, occluded {}, alpha {:.2}, frame {:.0}×{:.0} at ({:.0}, {:.0}), \
             content {content}",
            window.isVisible(),
            occluded,
            window.alphaValue(),
            frame.size.width,
            frame.size.height,
            frame.origin.x,
            frame.origin.y,
        ));
    }
    Some(format!(
        "AppKit: app active {}, {} window(s) — {}",
        app.isActive(),
        windows.count(),
        described.join("; ")
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn appkit_window_state() -> Option<String> {
    None
}

/// Bring the app to the front at launch (#1032).
///
/// An unbundled binary — which is what `cargo run` and every scripted run produce — does not
/// activate itself on macOS. The window is created and mapped, but stays behind whatever was
/// frontmost, and the window server then reports it **occluded** and stops giving it drawable
/// updates. The app goes on building frames into a surface nothing displays, so the window
/// shows its uninitialised backing: a grey rectangle, with every app-side signal healthy.
///
/// This is also why scripted screenshots intermittently failed with "the window never
/// painted" — same occlusion, same suppressed updates.
#[cfg(target_os = "macos")]
pub fn activate_app() -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    // **Regular** first (#1082). A binary run straight from the terminal has no bundle, so
    // AppKit gives it an activation policy that cannot become frontmost at all — and then
    // `activate()` is simply refused, which is why the window stayed occluded however many
    // times we asked. Setting the policy is what makes the request answerable.
    //
    // Log the policy change once (#1108): it is the one action `activate_app` takes that is
    // not visible in the state summary, and a grey launch where the policy stayed
    // "accessory" reads differently from one where it was corrected and the window still
    // stayed occluded.
    if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
        log_policy_change(app.activationPolicy());
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    }
    app.activate();
    // Activation is **cooperative** on macOS 14+: it defers to whatever app is currently
    // active, and `ignoringOtherApps` is documented as having no effect there — so a launch
    // from a terminal that keeps focus cannot be made active at all. That is the system's
    // call and not ours to override.
    //
    // Ordering the *window* front is a different request, and one the window server does
    // honour: the window comes out from behind whatever was covering it even while the app
    // stays inactive. That is what matters here, because it is **occlusion** — not focus —
    // that makes the window server stop handing out drawable updates and leaves the surface
    // showing its uninitialised contents (#1032/#1082).
    //
    // Order every visible window to the front, not just the first one (#1091). Once there
    // are multiple windows (e.g. a popped-out drawing, #276), `app.windows()` may return
    // them in creation order with the main window first — but we order every visible one
    // anyway so the main viewport window is never missed. `orderFrontRegardless` alone
    // does not make the window the key window, so call `makeKeyAndOrderFront` as well —
    // even though macOS 14+ makes activation cooperative, making the window key *and*
    // ordering it front is a stronger signal to the window server than ordering alone.
    // Skip invisible windows (e.g. a closed drawing popout) — making them visible would
    // pop them onto the screen unexpectedly.
    let windows = app.windows();
    for w in windows.iter() {
        if w.isVisible() {
            w.orderFrontRegardless();
            w.makeKeyAndOrderFront(None);
        }
    }
    !windows.is_empty() || app.isActive()
}

#[cfg(not(target_os = "macos"))]
pub fn activate_app() -> bool {
    false
}
