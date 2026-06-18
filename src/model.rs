//! In-memory document model.
//!
//! This is the very first slice of LE3 (see SPEC.md): a document is a flat list
//! of rectangles on a single 2D sketch. As the action-DAG, components, and the
//! OCCT kernel come online this will grow, but the persistence boundary
//! (`storage.rs`) is kept narrow so the file format can evolve underneath it.

use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle in sketch coordinates (millimetres, per SPEC §5.3).
///
/// Stored by its origin (`x`, `y`) and signed `w`/`h` extents. We normalise on
/// creation so width/height are always positive, which keeps hit-testing simple.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Build a normalised rectangle from two opposite corners.
    pub fn from_corners(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Rect {
            x: x0.min(x1),
            y: y0.min(y1),
            w: (x1 - x0).abs(),
            h: (y1 - y0).abs(),
        }
    }
}

/// The whole document. Currently just the rectangles on one sketch.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Document {
    pub rects: Vec<Rect>,
}
