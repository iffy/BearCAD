//! What AppKit says about the app's window (#1032).
//!
//! A grey launch that the app itself cannot see is the hard case: frames are built at the
//! right size, the viewport's blit lands, the window reports itself maximized and not
//! minimized — and the screen still shows the OS window background rather than the app's
//! own dark canvas. Everything egui can tell us is already healthy, so the remaining
//! questions are ones only the window server can answer: is the window on screen at all, is
//! it occluded, is it transparent, does its backing layer have a real size?

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
    if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
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
    let ordered = app.windows().iter().next().inspect(|w| w.orderFrontRegardless()).is_some();
    ordered || app.isActive()
}

#[cfg(not(target_os = "macos"))]
pub fn activate_app() -> bool {
    false
}
