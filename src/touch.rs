//! Touch-device support: gesture navigation (pinch zoom, two-finger pan, one-finger
//! orbit), finger-sized pick targets, and the compact phone layout.
//!
//! Touch mode flips on automatically the first time a touch arrives (and can be
//! forced from scripts via `bearcad.ui.touch(...)`). It is a global, frame-coherent
//! flag: picking helpers deep in the call tree scale their hit radii through
//! [`hit`] without threading state everywhere.

use std::sync::atomic::{AtomicBool, Ordering};

static TOUCH_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether touch mode is on (a touch has been seen, or a script forced it).
pub fn active() -> bool {
    TOUCH_ACTIVE.load(Ordering::Relaxed)
}

pub fn set_active(on: bool) {
    TOUCH_ACTIVE.store(on, Ordering::Relaxed);
}

/// Latch touch mode on when any touch arrives this frame.
pub fn detect(ctx: &eframe::egui::Context) {
    if !active() && ctx.input(|i| i.any_touches()) {
        set_active(true);
    }
}

/// How much bigger pick targets get under a finger than under a mouse cursor.
const TOUCH_HIT_SCALE: f32 = 1.7;

/// A pick radius/tolerance in px, finger-sized when touch mode is on.
pub fn hit(base_px: f32) -> f32 {
    if active() {
        base_px * TOUCH_HIT_SCALE
    } else {
        base_px
    }
}

/// Below this logical screen width the layout goes compact (phone-sized): side panes
/// float as closable windows over the viewport instead of docking beside it.
const COMPACT_WIDTH: f32 = 700.0;

pub fn compact(ctx: &eframe::egui::Context) -> bool {
    ctx.screen_rect().width() < COMPACT_WIDTH
}

/// Convert a proportional zoom factor (pinch / trackpad, `>1` = zoom in) into the
/// scroll-pixel units [`crate::camera::Camera::zoom`] expects, so a pinch of factor
/// `f` lands the camera exactly at `distance / f`.
pub fn zoom_factor_to_scroll(factor: f32) -> f32 {
    (1.0 - 1.0 / factor.max(1e-3)) * 1000.0
}

static VALUE_FIELD_FOCUSED: AtomicBool = AtomicBool::new(false);

/// Whether the currently focused text widget is a *value* field (expression/dimension
/// input). Set during rendering by the value widgets, cleared each frame; drives which
/// keyboard a touch device gets — the app's keypad for values, the OS keyboard for
/// free text like names.
pub fn set_value_field_focused(on: bool) {
    VALUE_FIELD_FOCUSED.store(on, Ordering::Relaxed);
}

pub fn value_field_focused() -> bool {
    VALUE_FIELD_FOCUSED.load(Ordering::Relaxed)
}

/// Long-press duration before a touch press counts as a right-click.
pub const LONG_PRESS_SECS: f64 = 0.6;
/// How far the finger may wander (px) and still count as a long press, not a drag.
pub const LONG_PRESS_SLOP_PX: f32 = 8.0;

/// Whether a held touch press should fire as a synthetic secondary click (pure
/// decision; the caller injects the events and latches `already_fired`).
pub fn long_press_fires(held_secs: f64, moved_px: f32, already_fired: bool) -> bool {
    !already_fired && held_secs >= LONG_PRESS_SECS && moved_px <= LONG_PRESS_SLOP_PX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_radius_grows_only_in_touch_mode() {
        set_active(false);
        assert_eq!(hit(10.0), 10.0);
        set_active(true);
        assert!(hit(10.0) > 15.0);
        set_active(false);
    }

    #[test]
    fn long_press_fires_once_when_held_still() {
        assert!(!long_press_fires(0.3, 0.0, false), "too short");
        assert!(!long_press_fires(1.0, 20.0, false), "wandered: it's a drag");
        assert!(long_press_fires(0.7, 3.0, false));
        assert!(!long_press_fires(0.7, 3.0, true), "fires once per press");
    }

    /// The scroll equivalent of a pinch factor must land the camera at exactly
    /// `distance / factor` given `Camera::zoom`'s `distance * (1 - scroll * 0.001)`.
    #[test]
    fn pinch_factor_round_trips_through_camera_zoom_formula() {
        for factor in [0.5f32, 0.9, 1.0, 1.1, 2.0] {
            let scroll = zoom_factor_to_scroll(factor);
            let new_distance = 100.0 * (1.0 - scroll * 0.001);
            assert!(
                (new_distance - 100.0 / factor).abs() < 1e-3,
                "factor {factor}: got {new_distance}, want {}",
                100.0 / factor
            );
        }
    }
}
