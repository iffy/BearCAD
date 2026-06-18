//! In-memory document model.
//!
//! This is the very first slice of LE3 (see SPEC.md): a document is a flat list
//! of rectangles and lines on a single 2D sketch. As the action-DAG, components,
//! and the OCCT kernel come online this will grow, but the persistence boundary
//! (`storage.rs`) is kept narrow so the file format can evolve underneath it.

use crate::face::default_xy_plane;
use serde::{Deserialize, Serialize};

/// A sketchable face that lines and rectangles can be drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceId {
    Rect(usize),
    ConstructionPlane(usize),
}

impl Default for FaceId {
    fn default() -> Self {
        FaceId::ConstructionPlane(0)
    }
}

impl FaceId {
    pub fn from_script(kind: &str, index: usize) -> Option<Self> {
        match kind.to_ascii_lowercase().as_str() {
            "rect" | "rectangle" => Some(FaceId::Rect(index)),
            "plane" | "construction_plane" | "constructionplane" => {
                Some(FaceId::ConstructionPlane(index))
            }
            _ => None,
        }
    }
}

/// An axis-aligned rectangle in face-local coordinates (millimetres, per SPEC §5.3).
///
/// Stored by its origin (`x`, `y`) and signed `w`/`h` extents in the local (u, v)
/// frame of [`FaceId::parent`]. We normalise on creation so width/height are always
/// positive, which keeps hit-testing simple.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    #[serde(default)]
    pub parent: FaceId,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Build a normalised rectangle from two opposite corners in face-local coords.
    pub fn from_local_corners(parent: FaceId, u0: f32, v0: f32, u1: f32, v1: f32) -> Self {
        Rect {
            parent,
            x: u0.min(u1),
            y: v0.min(v1),
            w: (u1 - u0).abs(),
            h: (v1 - v0).abs(),
        }
    }
}

/// A line segment in face-local coordinates (millimetres, per SPEC §5.3).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    #[serde(default)]
    pub parent: FaceId,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Line {
    pub fn from_local_endpoints(
        parent: FaceId,
        u0: f32,
        v0: f32,
        u1: f32,
        v1: f32,
    ) -> Self {
        Self {
            parent,
            x0: u0,
            y0: v0,
            x1: u1,
            y1: v1,
        }
    }

    pub fn length(&self) -> f32 {
        let du = self.x1 - self.x0;
        let dv = self.y1 - self.y0;
        (du * du + dv * dv).sqrt()
    }
}

/// A construction plane in world space (millimetres). Not exported to `.le3`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstructionPlane {
    pub origin: glam::Vec3,
    pub normal: glam::Vec3,
    pub u_axis: glam::Vec3,
    pub v_axis: glam::Vec3,
}

/// Which sketch primitive was created, in chronological order (for undo).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Rect,
    Line,
    ConstructionPlane,
}

/// The whole document: sketch primitives parented to faces, plus construction planes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub rects: Vec<Rect>,
    pub lines: Vec<Line>,
    /// Construction planes live in the document but are not written to `.le3`.
    #[serde(skip)]
    pub construction_planes: Vec<ConstructionPlane>,
    pub shape_order: Vec<ShapeKind>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            rects: Vec::new(),
            lines: Vec::new(),
            construction_planes: vec![default_xy_plane()],
            shape_order: Vec::new(),
        }
    }
}

impl Document {
    pub fn has_children(&self, face: FaceId) -> bool {
        self.rects.iter().any(|r| r.parent == face)
            || self.lines.iter().any(|l| l.parent == face)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_length_from_endpoints() {
        let line = Line::from_local_endpoints(FaceId::default(), 0.0, 0.0, 3.0, 4.0);
        assert!((line.length() - 5.0).abs() < 1e-4);
    }
}