//! The drawing loupe (#755): while a finger drags a sketch shape out, a round magnifier
//! floats beside the fingertip showing the geometry underneath it, so the snap targets the
//! finger itself covers can still be aimed at.
//!
//! Only the placement maths lives here (and is unit-tested); the magnified geometry is
//! painted by the viewport, which is the only place that can project the document.

use eframe::egui;

/// Radius of the loupe disc, in points.
pub const RADIUS: f32 = 64.0;
/// How much the region under the fingertip is magnified.
pub const ZOOM: f32 = 2.6;
/// Clear space between the fingertip and the nearest edge of the disc — enough that a
/// finger (and the hand behind it) never covers the glass.
const GAP: f32 = 30.0;
/// Keep the disc this far inside the viewport edges.
const MARGIN: f32 = 6.0;

/// Where the loupe disc sits for a fingertip at `finger`.
///
/// Directly above the finger normally. When the finger is near the top of the view there's
/// no headroom, so the disc moves **beside** it — to the right, or to the left when the
/// right side has no room. The disc is always kept inside `viewport` (assuming it fits at
/// all), and never lands on top of the fingertip.
pub fn center(finger: egui::Pos2, viewport: egui::Rect, radius: f32) -> egui::Pos2 {
    let offset = radius + GAP;
    let x_range = (viewport.left() + radius + MARGIN, viewport.right() - radius - MARGIN);
    let clamp_x = |x: f32| {
        if x_range.0 <= x_range.1 {
            x.clamp(x_range.0, x_range.1)
        } else {
            viewport.center().x
        }
    };
    let above = finger.y - offset;
    if above - radius >= viewport.top() + MARGIN {
        return egui::pos2(clamp_x(finger.x), above);
    }
    // No headroom: sit beside the finger, on whichever side the disc actually fits — and
    // when neither does, on the roomier side.
    let (right, left) = (finger.x + offset, finger.x - offset);
    let fits_right = right + radius <= viewport.right() - MARGIN;
    let fits_left = left - radius >= viewport.left() + MARGIN;
    let x = if fits_right {
        right
    } else if fits_left {
        left
    } else if viewport.right() - finger.x >= finger.x - viewport.left() {
        right
    } else {
        left
    };
    let y_range = (viewport.top() + radius + MARGIN, viewport.bottom() - radius - MARGIN);
    let y = if y_range.0 <= y_range.1 {
        finger.y.clamp(y_range.0, y_range.1)
    } else {
        viewport.center().y
    };
    egui::pos2(clamp_x(x), y)
}

/// Map a screen point into the loupe: the fingertip lands at the disc's centre and
/// everything around it is blown up by `ZOOM`.
pub fn magnify(finger: egui::Pos2, center: egui::Pos2, p: egui::Pos2) -> egui::Pos2 {
    center + (p - finger) * ZOOM
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    #[test]
    fn loupe_floats_above_the_finger_when_there_is_room() {
        let finger = egui::pos2(400.0, 400.0);
        let c = center(finger, viewport(), RADIUS);
        assert_eq!(c.x, finger.x);
        assert!(c.y < finger.y - RADIUS, "the disc clears the fingertip");
        assert!(viewport().contains(c));
    }

    /// The reported case: a finger at the very top of the view has no headroom, so the
    /// loupe goes to the side instead of being clipped off-screen.
    #[test]
    fn loupe_moves_beside_the_finger_at_the_top_of_the_view() {
        let finger = egui::pos2(400.0, 8.0);
        let c = center(finger, viewport(), RADIUS);
        assert!(c.y >= RADIUS, "the whole disc is on-screen: {c:?}");
        assert!((c.x - finger.x).abs() >= RADIUS, "beside the finger, not over it");
    }

    /// Top-right corner: the right side has no room, so it swings to the left.
    #[test]
    fn loupe_swings_left_when_the_right_edge_is_in_the_way() {
        let finger = egui::pos2(790.0, 8.0);
        let c = center(finger, viewport(), RADIUS);
        assert!(c.x < finger.x, "left of the finger: {c:?}");
        assert!(c.x - RADIUS >= 0.0 && c.x + RADIUS <= 800.0);
    }

    /// Wherever the finger is, the disc stays fully inside the viewport and never covers
    /// the fingertip.
    #[test]
    fn loupe_stays_inside_the_viewport_and_off_the_fingertip() {
        let vp = viewport();
        for x in [2.0f32, 60.0, 400.0, 740.0, 798.0] {
            for y in [2.0f32, 40.0, 300.0, 560.0, 598.0] {
                let finger = egui::pos2(x, y);
                let c = center(finger, vp, RADIUS);
                assert!(
                    c.x - RADIUS >= vp.left() - 0.01
                        && c.x + RADIUS <= vp.right() + 0.01
                        && c.y - RADIUS >= vp.top() - 0.01
                        && c.y + RADIUS <= vp.bottom() + 0.01,
                    "disc off-screen for {finger:?}: {c:?}"
                );
                assert!(
                    (c - finger).length() >= RADIUS,
                    "disc covers the fingertip for {finger:?}: {c:?}"
                );
            }
        }
    }

    #[test]
    fn magnify_puts_the_fingertip_at_the_centre_and_scales_around_it() {
        let finger = egui::pos2(100.0, 200.0);
        let c = egui::pos2(100.0, 80.0);
        assert_eq!(magnify(finger, c, finger), c);
        let p = magnify(finger, c, finger + egui::vec2(10.0, 0.0));
        assert!((p.x - (c.x + 10.0 * ZOOM)).abs() < 1e-4);
    }
}
