//! Construction geometry — helper objects that stay in-session but are not exported.
//!
//! Construction planes are defined by a reference face or axis/line, then an offset
//! (and optionally an angle around an axis).

use crate::face::{
    line_world_endpoints, line_world_polyline, sketch_frame,
    SketchFrame,
};
use crate::hierarchy::SceneElement;
use crate::model::{
    ConstructionPlane, ConstructionPlaneParent, ConstraintPoint, Document, FaceId, Line, LineEnd,
    PlaneAnchor, PlaneDefinition, PlaneExtent, SketchId,
};
use crate::value::{eval_length_mm, parse_length_or};
use eframe::egui;
use glam::{Quat, Vec3};
/// Shared stroke/fill colour for all construction geometry.
pub const CONSTRUCTION_RGBA: egui::Color32 = egui::Color32::from_rgb(230, 120, 40);
/// Pale yellow (#ffffc5) fill for construction planes (semi-transparent in the viewport, #628).
pub const PLANE_FILL_RGBA: egui::Color32 = egui::Color32::from_rgb(0xff, 0xff, 0xc5);

/// Screen-space dash and gap lengths for construction line strokes (pixels).
pub const CONSTRUCTION_DASH_LENGTH_PX: f32 = 6.0;
pub const CONSTRUCTION_DASH_GAP_PX: f32 = 4.0;

/// Half-edge length of the visible plane quad (millimetres).
pub const PLANE_DISPLAY_HALF: f32 = 50.0;

/// Screen-space pick tolerance for lines (pixels). The pointer need not land on the stroke.
pub const LINE_PICK_RADIUS_PX: f32 = 12.0;

/// Screen-space pick tolerance for points such as line endpoints (pixels).
pub const POINT_PICK_RADIUS_PX: f32 = 12.0;

/// Extra margin when picking faces by proximity to their projected edges (pixels).
pub const FACE_PICK_MARGIN_PX: f32 = 8.0;

/// Visual highlight for a pickable target under the cursor.
pub const PICK_HOVER_RGBA: egui::Color32 = egui::Color32::from_rgb(255, 210, 90);

/// The Selection Exploder's availability hint (#551): a light green, distinct from the yellow
/// pick-hover, that says "several things are stacked here — press Space to fan them out".
pub const EXPLODER_HINT_RGBA: egui::Color32 = egui::Color32::from_rgb(140, 226, 150);

/// Fill strength when highlighting a whole sketchable face on hover.
pub const FACE_HOVER_FILL_MULTIPLIER: f32 = 0.38;

/// Hover accent for axis gizmo drag handles.
pub const GIZMO_HANDLE_HOVER_RGBA: egui::Color32 = egui::Color32::from_rgb(255, 230, 120);

/// Visible length of the global X/Y/Z axes from the origin (millimetres).
pub const GLOBAL_AXIS_EXTENT_MM: f32 = 200.0;

/// Radius of the angle gizmo circle around an axis reference (millimetres).
pub const AXIS_ANGLE_GIZMO_RADIUS_MM: f32 = 25.0;

/// Screen-space hit radius for axis gizmo drag handles (pixels).
pub const AXIS_GIZMO_HANDLE_HIT_RADIUS_PX: f32 = 14.0;

/// Yellow arc colour for the axis angle dial, matching the Face Snap rotation arc (#1384).
pub const AXIS_ANGLE_ARC: egui::Color32 = egui::Color32::from_rgb(255, 225, 90);

/// What the user picked as the plane reference on the first click.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaneReference {
    /// A planar face: offset moves the plane along `normal`.
    Face {
        origin: Vec3,
        normal: Vec3,
        label: String,
    },
    /// A line or axis: offset is perpendicular distance; `angle_deg` spins the plane around the axis.
    Axis {
        origin: Vec3,
        direction: Vec3,
        label: String,
    },
}

impl PlaneReference {
    pub fn is_axis(&self) -> bool {
        matches!(self, PlaneReference::Axis { .. })
    }

    pub fn label(&self) -> &str {
        match self {
            PlaneReference::Face { label, .. } | PlaneReference::Axis { label, .. } => label,
        }
    }
}

/// How the Plane tool's current anchor was established (#474 / #483).
///
/// Valid complete sets for the Anchor picker:
/// - [`Face`](Self::Face): one planar face / ground / construction plane
/// - [`Axis`](Self::Axis): one straight edge (the line lies *in* the plane)
/// - [`LineAndPoint`](Self::LineAndPoint): one line/curve + one point (plane through
///   the point, normal along the line) — built by a complementary second pick
/// - [`Point`](Self::Point): a vertex alone (optionally with #474 normal candidates)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneAnchorSource {
    Face,
    Axis,
    Point,
    LineAndPoint,
}

/// Classify a viewport pick as a plane-anchor source kind.
pub fn plane_anchor_source_from_pick(kind: &PickTargetKind) -> PlaneAnchorSource {
    match kind {
        PickTargetKind::Point(_) | PickTargetKind::BodyVertex { .. } => PlaneAnchorSource::Point,
        PickTargetKind::Line(_)
        | PickTargetKind::BodyEdge { .. }
        | PickTargetKind::GlobalAxis(_)
        | PickTargetKind::OriginAxis(_)
        | PickTargetKind::Circle(_) => PlaneAnchorSource::Axis,
        // A cylinder's centre line is a straight reference like any other (#1013).
        PickTargetKind::BodyAxis { .. } => PlaneAnchorSource::Axis,
        PickTargetKind::BodyFace { .. }
        | PickTargetKind::ConstructionPlane(_)
        | PickTargetKind::TracingImage(_)
        | PickTargetKind::Ground(_)
        | PickTargetKind::SketchFace(_) => PlaneAnchorSource::Face,
        // A round wall is no plane to anchor against; classify it as a point so this is total.
        PickTargetKind::BodyCylinder { .. } => PlaneAnchorSource::Point,
        // Neither a constraint badge (#568), a whole body (#902), nor a drawing-page item
        // (#1641) is a plane anchor — all three only reach the exploder; classify them as a
        // point so the arm is total.
        PickTargetKind::Constraint(_)
        | PickTargetKind::Body(_)
        | PickTargetKind::DrawingElement { .. } => PlaneAnchorSource::Point,
    }
}

/// Whether a sketch line is a curve (has bezier handles), not a straight edge.
///
/// Straight edges alone are a complete plane-anchor set (line *in* the plane). A curve
/// alone is not — it needs a point for the line+point set (#483): plane through the
/// point, normal along the curve (tangent at an endpoint).
pub fn sketch_line_is_curve(doc: &Document, line_index: crate::model::LineKey) -> bool {
    doc.lines
        .get(line_index)
        .is_some_and(|l| l.bezier.is_some())
}

/// Outward world-space tangent of `line_index` at `point` when the point (or a
/// coincidence partner) is an endpoint of that line — the same direction #474 uses.
/// `None` when the point is not on this line's ends.
pub fn line_outward_tangent_at_point(
    doc: &Document,
    line_index: crate::model::LineKey,
    point: &crate::model::ConstraintPoint,
) -> Option<Vec3> {
    let sketch = point_sketch(doc, point.clone())?;
    let ends: Vec<crate::model::LineEnd> =
        crate::vertex_drag::coincident_group(doc, sketch, point.clone())
            .into_iter()
            .filter_map(|p| match p {
                crate::model::ConstraintPoint::LineEndpoint { line, end } if line == line_index => {
                    Some(end)
                }
                _ => None,
            })
            .collect();
    let end = *ends.first()?;
    line_outward_tangent_at_end(doc, line_index, end)
}

fn line_outward_tangent_at_end(
    doc: &Document,
    line_index: crate::model::LineKey,
    end: crate::model::LineEnd,
) -> Option<Vec3> {
    let line = doc.lines.get(line_index)?;
    let frame = crate::face::sketch_geometry_frame(doc, line.sketch)?;
    let (v, toward) = match end {
        crate::model::LineEnd::Start => {
            let toward = line.bezier.map(|b| b[0]).unwrap_or((line.x1, line.y1));
            ((line.x0, line.y0), toward)
        }
        crate::model::LineEnd::End => {
            let toward = line.bezier.map(|b| b[1]).unwrap_or((line.x0, line.y0));
            ((line.x1, line.y1), toward)
        }
    };
    let vw = crate::face::local_to_world(&frame, v.0, v.1);
    let tw = crate::face::local_to_world(&frame, toward.0, toward.1);
    let dir = (vw - tw).normalize_or_zero();
    (dir.length_squared() >= 1e-8).then_some(dir)
}

/// World normal for a line+point plane (#483): prefer the line's tangent at the point
/// when the point is an endpoint of that line (curves included); otherwise `fallback_dir`.
pub fn plane_normal_for_line_and_point(
    doc: &Document,
    line_index: Option<crate::model::LineKey>,
    point: Option<&crate::model::ConstraintPoint>,
    fallback_dir: Vec3,
) -> Vec3 {
    if let (Some(li), Some(pt)) = (line_index, point) {
        if let Some(dir) = line_outward_tangent_at_point(doc, li, pt) {
            return dir;
        }
    }
    let dir = fallback_dir.normalize_or_zero();
    if dir.length_squared() >= 1e-8 {
        dir
    } else {
        Vec3::Z
    }
}

/// Build the face-mode reference for a completed line+point anchor set.
pub fn line_and_point_plane_reference(
    origin: Vec3,
    normal: Vec3,
    point_label: &str,
    line_label: &str,
) -> (PlaneReference, Vec<String>) {
    let labels = vec![point_label.to_string(), line_label.to_string()];
    (
        PlaneReference::Face {
            origin,
            normal,
            label: format!("{point_label} ⊥ {line_label}"),
        },
        labels,
    )
}

/// Reorder the Anchor input's elements after a line+point complement (#483/#955).
///
/// The rows [`line_and_point_plane_reference`] produces are `[point, line]`, so the newly
/// clicked half replaces its own slot and the other half carries over from what was already
/// held. `current` is the anchor's rows before the complement — generic so the elements and
/// their per-row frames stay in step under one rule.
pub fn complemented_anchor_elements<T: Clone>(
    source: PlaneAnchorSource,
    current: &[T],
    next: Option<T>,
    next_is_point: bool,
) -> Vec<T> {
    let (point, line) = match source {
        PlaneAnchorSource::LineAndPoint => (current.first().cloned(), current.get(1).cloned()),
        PlaneAnchorSource::Axis => (None, current.first().cloned()),
        _ => (current.first().cloned(), None),
    };
    let (point, line) = if next_is_point {
        (next, line)
    } else {
        (point, next)
    };
    [point, line].into_iter().flatten().collect()
}

/// If `next` complements the current anchor into a line+point set (#483), return the
/// upgraded face-mode reference, new source, and Anchor row labels. Otherwise `None`
/// (caller may treat the click as a commit).
///
/// Complements:
/// - [`Axis`](PlaneAnchorSource::Axis) + point → through point, normal along the line
///   (endpoint tangent when the point is on the line)
/// - [`Point`](PlaneAnchorSource::Point) / [`LineAndPoint`](PlaneAnchorSource::LineAndPoint)
///   + line/edge/axis → keep origin, normal along the line (same tangent rule)
///
/// `axis_line` / `anchor_point` identify the geometry so curve endpoints use the true
/// tangent at the end rather than a mid-segment axis direction.
pub fn complement_plane_anchor(
    doc: &Document,
    source: PlaneAnchorSource,
    current: &PlaneReference,
    axis_line: Option<crate::model::LineKey>,
    anchor_point: Option<&crate::model::ConstraintPoint>,
    next_kind: &PickTargetKind,
    next_reference: &PlaneReference,
) -> Option<(
    PlaneReference,
    PlaneAnchorSource,
    Vec<String>,
    Option<crate::model::LineKey>,
    Option<crate::model::ConstraintPoint>,
)>
{
    let is_point = matches!(
        next_kind,
        PickTargetKind::Point(_) | PickTargetKind::BodyVertex { .. }
    );
    let next_line = match next_kind {
        PickTargetKind::Line(i) => Some(*i),
        _ => None,
    };
    let is_line = matches!(
        next_kind,
        PickTargetKind::Line(_)
            | PickTargetKind::BodyEdge { .. }
            | PickTargetKind::GlobalAxis(_)
            | PickTargetKind::OriginAxis(_)
            | PickTargetKind::Circle(_)
    );

    match source {
        PlaneAnchorSource::Axis if is_point => {
            let PlaneReference::Axis {
                direction,
                label: line_label,
                ..
            } = current
            else {
                return None;
            };
            let (origin, point_label) = match next_reference {
                PlaneReference::Face { origin, label, .. }
                | PlaneReference::Axis { origin, label, .. } => (*origin, label.clone()),
            };
            let pt = match next_kind {
                PickTargetKind::Point(p) => Some(p.clone()),
                _ => None,
            };
            let dir = plane_normal_for_line_and_point(
                doc,
                axis_line,
                pt.as_ref(),
                *direction,
            );
            let (reference, labels) =
                line_and_point_plane_reference(origin, dir, &point_label, line_label);
            Some((
                reference,
                PlaneAnchorSource::LineAndPoint,
                labels,
                axis_line,
                pt,
            ))
        }
        PlaneAnchorSource::Point | PlaneAnchorSource::LineAndPoint if is_line => {
            let PlaneReference::Face {
                origin,
                label: point_label,
                ..
            } = current
            else {
                return None;
            };
            let PlaneReference::Axis {
                direction,
                label: line_label,
                ..
            } = next_reference
            else {
                return None;
            };
            let point_row = if source == PlaneAnchorSource::LineAndPoint {
                point_label
                    .split(" ⊥ ")
                    .next()
                    .unwrap_or(point_label)
                    .to_string()
            } else {
                point_label.clone()
            };
            let line_idx = next_line.or(axis_line);
            let dir = plane_normal_for_line_and_point(
                doc,
                line_idx,
                anchor_point,
                *direction,
            );
            let (reference, labels) =
                line_and_point_plane_reference(*origin, dir, &point_row, line_label);
            Some((
                reference,
                PlaneAnchorSource::LineAndPoint,
                labels,
                line_idx,
                anchor_point.cloned(),
            ))
        }
        PlaneAnchorSource::LineAndPoint if is_point => {
            // Re-pick the point; recompute normal if we know the line (endpoint tangent).
            let PlaneReference::Face { normal, label, .. } = current else {
                return None;
            };
            let (origin, point_label) = match next_reference {
                PlaneReference::Face { origin, label, .. }
                | PlaneReference::Axis { origin, label, .. } => (*origin, label.clone()),
            };
            let line_row = label
                .split(" ⊥ ")
                .nth(1)
                .unwrap_or("Line")
                .to_string();
            let pt = match next_kind {
                PickTargetKind::Point(p) => Some(p.clone()),
                _ => None,
            };
            let dir = plane_normal_for_line_and_point(doc, axis_line, pt.as_ref(), *normal);
            let (reference, labels) =
                line_and_point_plane_reference(origin, dir, &point_label, &line_row);
            Some((
                reference,
                PlaneAnchorSource::LineAndPoint,
                labels,
                axis_line,
                pt,
            ))
        }
        _ => None,
    }
}

/// Which dimension field is focused while creating a plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneDim {
    Offset,
    Angle,
}

impl PlaneDim {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "offset" | "o" | "d" | "distance" => Some(PlaneDim::Offset),
            "angle" | "a" | "deg" | "degrees" => Some(PlaneDim::Angle),
            _ => None,
        }
    }

}

pub fn reference_from_definition(def: &PlaneDefinition) -> PlaneReference {
    match &def.anchor {
        PlaneAnchor::Face {
            origin,
            normal,
            label,
        } => PlaneReference::Face {
            origin: *origin,
            normal: *normal,
            label: label.clone(),
        },
        PlaneAnchor::Axis {
            origin,
            direction,
            label,
        } => PlaneReference::Axis {
            origin: *origin,
            direction: *direction,
            label: label.clone(),
        },
    }
}

pub fn definition_from_reference(
    reference: &PlaneReference,
    offset_mm: f32,
    angle_deg: f32,
) -> PlaneDefinition {
    let anchor = match reference {
        PlaneReference::Face {
            origin,
            normal,
            label,
        } => PlaneAnchor::Face {
            origin: *origin,
            normal: *normal,
            label: label.clone(),
        },
        PlaneReference::Axis {
            origin,
            direction,
            label,
        } => PlaneAnchor::Axis {
            origin: *origin,
            direction: *direction,
            label: label.clone(),
        },
    };
    PlaneDefinition {
        anchor,
        offset_mm,
        angle_deg,
        offset_expression: String::new(),
        angle_expression: String::new(),
    }
}

pub fn plane_from_definition(def: &PlaneDefinition, parent: ConstructionPlaneParent) -> ConstructionPlane {
    let reference = reference_from_definition(def);
    let mut plane = resolve_plane(
        &reference,
        &def.offset_mm.to_string(),
        &def.angle_deg.to_string(),
        def.offset_mm,
        def.angle_deg,
        true,
        true,
    );
    plane.parent = parent;
    plane.definition = def.clone();
    plane
}

/// Construction-plane indices nested under sketches hosted on `root_plane`.
pub fn descendant_plane_indices(doc: &Document, root_plane: crate::model::ConstructionPlaneKey) -> Vec<crate::model::ConstructionPlaneKey> {
    let mut descendants = Vec::new();
    let mut faces = vec![FaceId::ConstructionPlane(root_plane)];
    let mut seen_faces = std::collections::HashSet::new();

    while let Some(face) = faces.pop() {
        if !seen_faces.insert(face.clone()) {
            continue;
        }
        for sketch in doc.sketches_on_face(face) {
            for (pi, plane) in doc.construction_planes.iter() {
                if matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
                    descendants.push(pi);
                    faces.push(FaceId::ConstructionPlane(pi));
                }
            }
            for (ci, circle) in doc.circles.iter() {
                if circle.sketch == sketch {
                    faces.push(FaceId::Circle(ci));
                }
            }
        }
    }

    descendants
}

/// Faces hosted on or nested under sketches on `root_plane` (including the root plane).
pub fn descendant_faces(doc: &Document, root_plane: crate::model::ConstructionPlaneKey) -> Vec<FaceId> {
    let mut faces = vec![FaceId::ConstructionPlane(root_plane)];
    let mut seen_faces = std::collections::HashSet::new();
    let mut collected = Vec::new();

    while let Some(face) = faces.pop() {
        if !seen_faces.insert(face.clone()) {
            continue;
        }
        collected.push(face.clone());
        for sketch in doc.sketches_on_face(face) {
            for (pi, plane) in doc.construction_planes.iter() {
                if matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
                    faces.push(FaceId::ConstructionPlane(pi));
                }
            }
            for (ci, circle) in doc.circles.iter() {
                if circle.sketch == sketch {
                    faces.push(FaceId::Circle(ci));
                }
            }
        }
    }

    collected
}

/// World-space preview of geometry that moves when a construction plane is edited.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaneEditDependentPreview {
    pub planes: Vec<(crate::model::ConstructionPlaneKey, ConstructionPlane)>,
    pub lines: Vec<(Vec3, Vec3)>,
}

/// Where dependent planes and hosted sketch geometry will land after `preview_plane` is committed.
pub fn preview_plane_edit_dependents(
    doc: &Document,
    plane_index: crate::model::ConstructionPlaneKey,
    preview_plane: &ConstructionPlane,
) -> Option<PlaneEditDependentPreview> {
    let old_frame = sketch_frame(doc, FaceId::ConstructionPlane(plane_index))?;
    let new_frame = SketchFrame {
        origin: preview_plane.origin,
        u_axis: preview_plane.u_axis,
        v_axis: preview_plane.v_axis,
        normal: preview_plane.normal,
    };

    let mut planes = Vec::new();
    for index in descendant_plane_indices(doc, plane_index) {
        let mut plane = doc.construction_planes[index].clone();
        transform_plane_between_frames(&old_frame, &new_frame, &mut plane);
        planes.push((index, plane));
    }

    let mut sketches = std::collections::HashSet::new();
    for face in descendant_faces(doc, plane_index) {
        for sketch in doc.sketches_on_face(face) {
            sketches.insert(sketch);
        }
    }

    let mut lines = Vec::new();
    for sketch in sketches {
        for line in doc.lines.values() {
            if line.sketch != sketch {
                continue;
            }
            let Some((a, b)) = line_world_endpoints(doc, line) else {
                continue;
            };
            lines.push((
                transform_point_between_frames(&old_frame, &new_frame, a),
                transform_point_between_frames(&old_frame, &new_frame, b),
            ));
        }
    }

    Some(PlaneEditDependentPreview {
        planes,
        lines,
    })
}

pub fn transform_point_between_frames(old: &SketchFrame, new: &SketchFrame, point: Vec3) -> Vec3 {
    let relative = point - old.origin;
    let along_u = relative.dot(old.u_axis);
    let along_v = relative.dot(old.v_axis);
    let along_n = relative.dot(old.normal);
    new.origin + new.u_axis * along_u + new.v_axis * along_v + new.normal * along_n
}

pub fn transform_vector_between_frames(old: &SketchFrame, new: &SketchFrame, vector: Vec3) -> Vec3 {
    let along_u = vector.dot(old.u_axis);
    let along_v = vector.dot(old.v_axis);
    let along_n = vector.dot(old.normal);
    new.u_axis * along_u + new.v_axis * along_v + new.normal * along_n
}

pub fn transform_plane_between_frames(
    old: &SketchFrame,
    new: &SketchFrame,
    plane: &mut ConstructionPlane,
) {
    plane.origin = transform_point_between_frames(old, new, plane.origin);
    plane.normal = transform_vector_between_frames(old, new, plane.normal).normalize_or_zero();
    plane.u_axis = transform_vector_between_frames(old, new, plane.u_axis).normalize_or_zero();
    plane.v_axis = transform_vector_between_frames(old, new, plane.v_axis).normalize_or_zero();
}

pub fn transform_definition_between_frames(
    old: &SketchFrame,
    new: &SketchFrame,
    definition: &mut PlaneDefinition,
) {
    match &mut definition.anchor {
        PlaneAnchor::Face { origin, normal, .. } => {
            *origin = transform_point_between_frames(old, new, *origin);
            *normal = transform_vector_between_frames(old, new, *normal).normalize_or_zero();
        }
        PlaneAnchor::Axis {
            origin,
            direction,
            ..
        } => {
            *origin = transform_point_between_frames(old, new, *origin);
            *direction = transform_vector_between_frames(old, new, *direction).normalize_or_zero();
        }
    }
}

/// Rebuild a construction plane from its definition and move descendants with it.
pub fn apply_construction_plane_edit(
    doc: &mut Document,
    plane_index: crate::model::ConstructionPlaneKey,
    definition: &PlaneDefinition,
    parent: ConstructionPlaneParent,
) -> Result<(), String> {
    if doc.construction_planes.get(plane_index).is_none() {
        return Err(format!("Unknown construction plane {}", plane_index.index()));
    }

    let old_frame = sketch_frame(doc, FaceId::ConstructionPlane(plane_index))
        .ok_or_else(|| format!("Construction plane {} has no sketch frame", plane_index.index()))?;
    let descendants = descendant_plane_indices(doc, plane_index);

    let plane = plane_from_definition(definition, parent);
    doc.construction_planes[plane_index] = plane;

    let new_frame = sketch_frame(doc, FaceId::ConstructionPlane(plane_index))
        .ok_or_else(|| format!("Construction plane {} has no sketch frame", plane_index.index()))?;

    for index in descendants {
        let Some(child) = doc.construction_planes.get_mut(index) else {
            continue;
        };
        transform_plane_between_frames(&old_frame, &new_frame, child);
        transform_definition_between_frames(&old_frame, &new_frame, &mut child.definition);
    }

    Ok(())
}

/// Build an orthonormal (u, v) basis on a plane from its unit normal.
/// Stable in-plane axes for a face-anchored plane (#399): `u = up_hint × n`, `v = n × u`,
/// with `up_hint = +Z` (falling back to `+Y` for near-±Z normals). A plane offset from
/// Ground inherits Ground's axes exactly (u = +X, v = +Y for n = +Z), and a vertical
/// plane's v points world-up. The previous `n × hint` rule came out rotated 90° from the
/// parent plane, so identical sketch coordinates on an offset plane landed rotated relative
/// to the plane they were offset from — a loft between same-(u,v) circles leaned sideways.
pub fn plane_basis(normal: Vec3) -> (Vec3, Vec3) {
    let n = normal.normalize_or_zero();
    if n.length_squared() < 1e-8 {
        return (Vec3::X, Vec3::Y);
    }
    let up = if n.z.abs() < 0.9 { Vec3::Z } else { Vec3::Y };
    let u = up.cross(n).normalize_or_zero();
    let v = n.cross(u);
    (u, v)
}

/// Offset a face reference along its normal.
pub fn plane_from_face(offset: f32, origin: Vec3, normal: Vec3) -> ConstructionPlane {
    let n = normal.normalize_or_zero();
    let (u, v) = plane_basis(n);
    ConstructionPlane {
        origin: origin + n * offset,
        normal: n,
        u_axis: u,
        v_axis: v,
        parent: ConstructionPlaneParent::Root,
        definition: definition_from_reference(
            &PlaneReference::Face {
                origin,
                normal: n,
                label: String::new(),
            },
            offset,
            0.0,
        ),
        repeat_instance: None,
        name: None,
        extent: crate::model::PlaneExtent::default(),
    }
}

/// Build a plane from an axis reference, perpendicular distance, and rotation (degrees).
pub fn plane_from_axis(
    offset: f32,
    angle_deg: f32,
    origin: Vec3,
    direction: Vec3,
) -> ConstructionPlane {
    let axis = direction.normalize_or_zero();
    let n = axis_normal(direction, angle_deg);
    // Anchor the in-plane basis to the reference axis so the visible plane does not
    // flip when `plane_basis` switches its world-aligned hint (the Z/X threshold).
    let u = axis;
    let v = axis.cross(n).normalize_or_zero();
    ConstructionPlane {
        origin: origin + n * offset,
        normal: n,
        u_axis: u,
        v_axis: v,
        parent: ConstructionPlaneParent::Root,
        definition: definition_from_reference(
            &PlaneReference::Axis {
                origin,
                direction: axis,
                label: String::new(),
            },
            offset,
            angle_deg,
        ),
        repeat_instance: None,
        name: None,
        extent: crate::model::PlaneExtent::default(),
    }
}

/// Sketch that owns geometry used as a construction-plane reference, if any.
pub fn sketch_from_pick_target(doc: &Document, kind: PickTargetKind) -> Option<SketchId> {
    match kind {
        PickTargetKind::Line(index) => doc.lines.get(index).map(|line| line.sketch),
        PickTargetKind::Circle(index) => doc.circles.get(index).map(|circle| circle.sketch),
        PickTargetKind::ConstructionPlane(index) => doc.construction_planes.get(index).and_then(|plane| {
            match plane.parent {
                ConstructionPlaneParent::Sketch(sketch) => Some(sketch),
                ConstructionPlaneParent::Root => None,
            }
        }),
        PickTargetKind::Point(point) => point_sketch(doc, point),
        // A constraint's own sketch — though a constraint is never used as a plane reference (it
        // only reaches the exploder, #568), so this is here just to keep the match total.
        PickTargetKind::Constraint(index) => doc.constraints.get(index).map(|c| c.sketch),
        // A profile face's own sketch; body/revolve faces belong to no sketch (like the
        // body kinds below).
        PickTargetKind::SketchFace(face) => match face {
            crate::model::FaceId::Circle(i) => doc.circles.get(i).map(|c| c.sketch),
            crate::model::FaceId::Polygon(lines) => {
                doc.lines.get(*lines.first()?).map(|l| l.sketch)
            }
            _ => None,
        },
        PickTargetKind::BodyEdge { .. }
        | PickTargetKind::BodyFace { .. }
        | PickTargetKind::BodyCylinder { .. }
        | PickTargetKind::BodyAxis { .. }
        | PickTargetKind::BodyVertex { .. }
        | PickTargetKind::Body(_)
        | PickTargetKind::GlobalAxis(_)
        | PickTargetKind::OriginAxis(_)
        | PickTargetKind::TracingImage(_)
        | PickTargetKind::DrawingElement { .. }
        | PickTargetKind::Ground(_) => None,
    }
}

pub fn point_sketch(doc: &Document, point: ConstraintPoint) -> Option<SketchId> {
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => doc.lines.get(line).map(|l| l.sketch),
        ConstraintPoint::CircleCenter(circle) => doc.circles.get(circle).map(|c| c.sketch),
        ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.get(text).map(|t| t.sketch)
        }
        // A calibration point belongs to whichever sketch references it (the image sits on
        // a plane, not in a sketch) — no owning sketch, like a face vertex.
        ConstraintPoint::ImageCalibrationPoint { .. } | ConstraintPoint::ImageAnchor { .. } => None,
        // A face's own vertex has no owning sketch of its own — it's referenced *from*
        // whichever sketch a constraint projects it into, not owned by one.
        ConstraintPoint::FaceVertex { .. } => None,
        ConstraintPoint::Origin => None,
    }
}

/// Hierarchy parent for a new construction plane from a pick target.
pub fn parent_from_pick_target(doc: &Document, kind: PickTargetKind) -> ConstructionPlaneParent {
    sketch_from_pick_target(doc, kind)
        .map(ConstructionPlaneParent::Sketch)
        .unwrap_or(ConstructionPlaneParent::Root)
}

/// Resolve the final plane from a reference and dimension texts (typed or live).
pub fn resolve_plane(
    reference: &PlaneReference,
    offset_text: &str,
    angle_text: &str,
    live_offset: f32,
    live_angle_deg: f32,
    user_edited_offset: bool,
    user_edited_angle: bool,
) -> ConstructionPlane {
    match reference {
        PlaneReference::Face { origin, normal, .. } => {
            let offset = parse_or_live_signed(offset_text, live_offset, user_edited_offset);
            plane_from_face(offset, *origin, *normal)
        }
        PlaneReference::Axis {
            origin,
            direction,
            ..
        } => {
            let offset = parse_or_live_signed(offset_text, live_offset, user_edited_offset);
            let angle = parse_or_live(angle_text, live_angle_deg, user_edited_angle);
            plane_from_axis(offset, angle, *origin, *direction)
        }
    }
}

fn parse_or_live(text: &str, live: f32, user_edited: bool) -> f32 {
    if user_edited {
        eval_length_mm(text)
            .or_else(|| text.trim().parse::<f32>().ok())
            .unwrap_or(live)
            .max(0.0)
    } else {
        live.max(0.0)
    }
}

fn parse_or_live_signed(text: &str, live: f32, user_edited: bool) -> f32 {
    if user_edited {
        parse_length_or(text, live)
    } else {
        live
    }
}

/// A tracing image's four world corners, in UV order: (0,0), (1,0), (1,1), (0,1) —
/// v flipped, since image v grows downward and plane-local v grows up. Shared by
/// the textured quad, the hover outline, the Select-tool pick (#1561), and
/// zoom-to-fit (#1584).
pub fn tracing_image_corners(
    doc: &Document,
    image: crate::model::TracingImageKey,
) -> Option<[Vec3; 4]> {
    let img = doc.tracing_images.get(image)?;
    tracing_image_corners_at(doc, img.plane, img.origin, img.rotation, img.width_mm, img.height_mm)
}

/// The quad an image would occupy at its *pristine* pose — before any Move op touched it
/// (#1631). This is what a Free move's turn pivots about, so the pivot doesn't shift as the
/// move it belongs to is applied, and recomputing the document is idempotent.
pub fn tracing_image_base_corners(
    doc: &Document,
    image: crate::model::TracingImageKey,
) -> Option<[Vec3; 4]> {
    let img = doc.tracing_images.get(image)?;
    tracing_image_corners_at(
        doc,
        img.plane,
        img.base_origin.unwrap_or(img.origin),
        img.base_rotation.unwrap_or(img.rotation),
        img.width_mm,
        img.height_mm,
    )
}

/// Displayed-quad corners at an explicit origin/rotation, falling back to the stored pose.
pub fn tracing_image_live_corners(
    doc: &Document,
    image: crate::model::TracingImageKey,
    pose: Option<((f32, f32), f32)>,
) -> Option<[Vec3; 4]> {
    let img = doc.tracing_images.get(image)?;
    let (origin, rotation) = pose.unwrap_or((img.origin, img.rotation));
    tracing_image_corners_at(doc, img.plane, origin, rotation, img.width_mm, img.height_mm)
}

/// Displayed-quad corners for an explicit origin/rotation (#1601/#1611).
pub fn tracing_image_corners_at(
    doc: &Document,
    plane: crate::model::ConstructionPlaneKey,
    origin: (f32, f32),
    rotation: f32,
    width_mm: f32,
    height_mm: f32,
) -> Option<[Vec3; 4]> {
    let frame = crate::face::sketch_frame(doc, FaceId::ConstructionPlane(plane))?;
    let at = |u_frac: f32, v_frac: f32| {
        let (x, y) = crate::model::image_local_mm_at(origin, rotation, width_mm, height_mm, u_frac, v_frac);
        frame.origin + frame.u_axis * x + frame.v_axis * y
    };
    Some([at(0.0, 1.0), at(1.0, 1.0), at(1.0, 0.0), at(0.0, 0.0)])
}

/// The tracing image whose displayed quad contains `screen` (#1588). When several
/// overlap, the one nearer the eye wins; a coplanar image beats its host plane
/// because it is the thing you're pointing at.
pub fn tracing_image_under_cursor(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    eye: Vec3,
) -> Option<crate::model::TracingImageKey> {
    match nearest_plane_or_image(screen, project, doc, Some(eye)) {
        Some((PickTargetKind::TracingImage(index), _, _)) => Some(index),
        _ => None,
    }
}

/// Corners of the visible plane quad in world space.
/// The frame a cross-section cut draws as (#1687): its plane slid along its own normal by the
/// cut's offset and turned by its roll, expressed as a [`ConstructionPlane`] so the same quad
/// and outline drawing every other plane uses applies to it.
pub fn cross_section_cut_plane(cut: &crate::model::CrossSectionCut) -> ConstructionPlane {
    let normal = cut.normal.normalize_or_zero();
    let normal = if normal.length_squared() < 1e-8 { Vec3::Z } else { normal };
    // Any pair of axes spanning the plane; the roll turns them about the normal.
    let seed = if normal.dot(Vec3::Z).abs() > 0.9 { Vec3::X } else { Vec3::Z };
    let u = seed.cross(normal).normalize_or_zero();
    let u = if u.length_squared() < 1e-8 { Vec3::X } else { u };
    let v = normal.cross(u);
    let (sin, cos) = cut.roll.sin_cos();
    let u_axis = u * cos + v * sin;
    let v_axis = v * cos - u * sin;
    ConstructionPlane {
        origin: cut.origin + normal * cut.offset_mm,
        normal,
        u_axis,
        v_axis,
        parent: crate::model::ConstructionPlaneParent::Root,
        definition: crate::face::default_xy_plane_definition(),
        repeat_instance: None,
        name: None,
        extent: crate::model::PlaneExtent::default(),
    }
}

pub fn plane_corners(plane: &ConstructionPlane) -> [Vec3; 4] {
    plane_corners_of(plane, plane.extent)
}

/// The four world-space corners a plane would have with `extent` — used to preview a resize
/// drag before it is committed. Corner order is low-u/low-v first, then counter-clockwise in
/// the plane's own frame, so index 0 and index 2 are the opposite pair the handles use.
pub fn plane_corners_of(plane: &ConstructionPlane, extent: PlaneExtent) -> [Vec3; 4] {
    let o = plane.origin;
    let u = plane.u_axis;
    let v = plane.v_axis;
    [
        o + u * extent.u_min + v * extent.v_min,
        o + u * extent.u_max + v * extent.v_min,
        o + u * extent.u_max + v * extent.v_max,
        o + u * extent.u_min + v * extent.v_max,
    ]
}

/// Screen radius of a selected construction plane's corner resize handle (#833).
pub const PLANE_RESIZE_HANDLE_RADIUS_PX: f32 = 7.0;

/// The two opposite corners a selected plane offers as resize grips (#833): its low
/// (`u_min`, `v_min`) corner and its high (`u_max`, `v_max`) one — always that pair, so the
/// grips stay on the same two corners however the plane is dragged about.
pub const PLANE_RESIZE_CORNERS: [usize; 2] = [0, 2];

/// World positions of `plane`'s two resize grips, paired with their corner index.
pub fn plane_resize_handles(plane: &ConstructionPlane) -> [(usize, Vec3); 2] {
    let corners = plane_corners(plane);
    [
        (PLANE_RESIZE_CORNERS[0], corners[PLANE_RESIZE_CORNERS[0]]),
        (PLANE_RESIZE_CORNERS[1], corners[PLANE_RESIZE_CORNERS[1]]),
    ]
}

/// Which resize grip of `plane` sits under `pointer`, if any.
pub fn plane_resize_handle_hit(
    pointer: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    plane: &ConstructionPlane,
) -> Option<usize> {
    plane_resize_handles(plane)
        .into_iter()
        .filter_map(|(corner, world)| {
            let screen = project(world)?;
            let d = screen.distance(pointer);
            (d <= PLANE_RESIZE_HANDLE_RADIUS_PX * 1.8).then_some((corner, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(corner, _)| corner)
}

/// The extent `plane` takes when its `corner` grip is dragged onto the plane point `world`:
/// the dragged corner follows the pointer and the opposite one stays put.
pub fn plane_extent_from_corner_drag(
    plane: &ConstructionPlane,
    corner: usize,
    world: Vec3,
) -> PlaneExtent {
    let rel = world - plane.origin;
    let u = rel.dot(plane.u_axis);
    let v = rel.dot(plane.v_axis);
    let mut extent = plane.extent;
    if corner == PLANE_RESIZE_CORNERS[1] {
        extent.u_max = u;
        extent.v_max = v;
    } else {
        extent.u_min = u;
        extent.v_min = v;
    }
    extent.normalized()
}

/// Live offset for a face reference from a world-space hover point.
#[cfg(test)]
mod pick_path_tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use super::*;
    use crate::model::{Document, FaceId, Line};

    /// #459 diagnosis: the full press-path pick — project a line endpoint through a
    /// real camera and ask the picker for it at that exact screen position.
    #[test]
    fn endpoint_picks_at_its_projected_position() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(Line::from_local_endpoints(sketch, -30.0, -20.0, 30.0, 20.0));
        let cam = crate::camera::Camera::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        let (a, _b) = crate::face::line_world_endpoints(&doc, &doc.lines[lkey(0)]).unwrap();
        let screen = project(a).expect("endpoint projects");
        let hit = nearest_sketch_point_in_sketch(screen, &project, &doc, sketch);
        assert!(hit.is_some(), "endpoint under the cursor must pick");
        let health = crate::document_health::DocumentHealth::default();
        let element = crate::vertex_drag::scene_element_for_point(hit.unwrap().0);
        assert!(
            crate::document_health::require_element_editable(&health, element).is_ok(),
            "a fresh line must be editable"
        );
    }
}

#[cfg(test)]
pub fn live_face_offset(origin: Vec3, normal: Vec3, hover: Vec3) -> f32 {
    let n = normal.normalize_or_zero();
    (hover - origin).dot(n).max(0.0)
}

/// Reference perpendicular to an axis (stable when axis is nearly vertical).
pub fn axis_reference_perp(direction: Vec3) -> Vec3 {
    let axis = direction.normalize_or_zero();
    let mut perp = axis.cross(Vec3::Z);
    if perp.length_squared() < 1e-6 {
        perp = axis.cross(Vec3::X);
    }
    perp.normalize_or_zero()
}

/// Plane normal for an axis reference at the given angle (degrees around the axis).
pub fn axis_normal(direction: Vec3, angle_deg: f32) -> Vec3 {
    let axis = direction.normalize_or_zero();
    let perp = axis_reference_perp(axis);
    (Quat::from_axis_angle(axis, angle_deg.to_radians()) * perp).normalize_or_zero()
}

/// Minimum visual offset for the gizmo arrow when the live offset is near zero. Keeps the
/// handle clear of the anchor vertex/face (chamfer/fillet amounts are often smaller than this
/// floor) so it never renders on top of — or is hard to grab apart from — the geometry it's
/// anchored to.
pub fn gizmo_display_offset(offset: f32) -> f32 {
    if offset.abs() < 4.0 {
        if offset == 0.0 {
            4.0
        } else {
            offset.signum() * 4.0
        }
    } else {
        offset
    }
}

/// World position of the offset drag handle along a plane normal.
pub fn offset_handle(origin: Vec3, normal: Vec3, offset: f32) -> Vec3 {
    origin + normal.normalize_or_zero() * offset
}

/// World position of the offset drag handle for an axis-referenced plane.
pub fn axis_offset_handle(origin: Vec3, direction: Vec3, offset: f32, angle_deg: f32) -> Vec3 {
    offset_handle(origin, axis_normal(direction, angle_deg), offset)
}

/// World position of the angle drag handle on the gizmo circle.
pub fn axis_angle_handle(origin: Vec3, direction: Vec3, angle_deg: f32) -> Vec3 {
    origin + axis_normal(direction, angle_deg) * AXIS_ANGLE_GIZMO_RADIUS_MM
}

/// Angle (degrees) from a ray hit on the plane perpendicular to the axis through `origin`.
pub fn angle_from_axis_plane_hit(origin: Vec3, direction: Vec3, hit: Vec3) -> f32 {
    let axis = direction.normalize_or_zero();
    let rel = hit - origin;
    let radial = rel - axis * rel.dot(axis);
    if radial.length_squared() < 1e-8 {
        return 0.0;
    }
    let dir = radial.normalize_or_zero();
    let perp = axis_reference_perp(axis);
    let tangent = axis.cross(perp).normalize_or_zero();
    let cos = dir.dot(perp);
    let sin = dir.dot(tangent);
    sin.atan2(cos).to_degrees().rem_euclid(360.0)
}

/// Wrap a degree angle onto `(-180, 180]`.
pub fn wrap_signed_deg(deg: f32) -> f32 {
    if !deg.is_finite() {
        return 0.0;
    }
    let mut d = deg % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

/// Signed angle in degrees from `zero_dir` to the in-plane direction of `hit`, about `axis`
/// through `origin`. Right-hand rule, short path in `(-180, 180]` (#1432).
pub fn signed_angle_deg_about_axis(origin: Vec3, axis: Vec3, zero_dir: Vec3, hit: Vec3) -> f32 {
    let n = axis.normalize_or_zero();
    let z = (zero_dir - n * zero_dir.dot(n)).normalize_or_zero();
    let rel = hit - origin;
    let radial = (rel - n * rel.dot(n)).normalize_or_zero();
    if n == Vec3::ZERO || z == Vec3::ZERO || radial == Vec3::ZERO {
        return 0.0;
    }
    let sin = n.dot(z.cross(radial));
    let cos = z.dot(radial);
    wrap_signed_deg(sin.atan2(cos).to_degrees())
}

/// Incremental rotation-gizmo turn (#1432): add the shortest signed step from
/// `start_angle_deg` to `current_angle_deg` onto `start_value_deg`, then wrap
/// the result to `(-180, 180]`.
pub fn rotation_gizmo_drag_deg(
    start_value_deg: f32,
    start_angle_deg: f32,
    current_angle_deg: f32,
) -> f32 {
    wrap_signed_deg(start_value_deg + wrap_signed_deg(current_angle_deg - start_angle_deg))
}

/// Offset (mm) after dragging the normal arrow along its screen projection.
pub fn offset_from_normal_drag(
    origin: Vec3,
    normal: Vec3,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    start_offset: f32,
    start_screen: egui::Pos2,
    current_screen: egui::Pos2,
) -> f32 {
    let Some(p0) = project(origin) else {
        return start_offset;
    };
    let Some(p1) = project(origin + normal) else {
        return start_offset;
    };
    let screen_axis = p1 - p0;
    let len = screen_axis.length();
    if len < 1e-3 {
        return start_offset;
    }
    let delta_px = (current_screen - start_screen).dot(screen_axis) / len;
    start_offset + delta_px / len
}

/// Free-cursor offset along `normal` so the tip at `origin + normal * offset` sits even
/// with the pointer (#1196).
///
/// Intersects the mouse ray with a camera-facing plane through the height axis. Unlike
/// [`offset_from_normal_drag`] (screen-delta along a linearised normal, measured from a
/// drag start), this is absolute: where the pointer aims is the tip height, so perspective
/// and an off-centre phase start no longer leave the tip lagging below the mouse.
pub fn offset_along_normal_from_cursor(
    origin: Vec3,
    normal: Vec3,
    cam: &crate::camera::Camera,
    screen: egui::Pos2,
    viewport: egui::Rect,
    vp: &glam::Mat4,
) -> Option<f32> {
    let normal = normal.normalize_or_zero();
    if normal.length_squared() < 0.5 {
        return None;
    }
    // Plane contains the height axis and faces the camera: its normal is the component of
    // (eye − origin) perpendicular to `normal`. Looking straight along the axis makes the
    // plane degenerate — free-cursor height is undefined from a plan view.
    let to_cam = cam.eye() - origin;
    let mut plane_n = to_cam - normal * to_cam.dot(normal);
    if plane_n.length_squared() < 1e-8 {
        return None;
    }
    plane_n = plane_n.normalize();
    let hit = cam.ray_plane_hit(screen, viewport, vp, origin, plane_n)?;
    Some((hit - origin).dot(normal))
}

/// Which axis gizmo handle is under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisGizmoHit {
    Offset,
    Angle,
}

/// Hit-test a rotation gizmo's disc handle at a screen position (#1418). The fade arcs
/// and the rest of the ring are not grab targets.
pub fn rotation_handle_hit(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    handle: Vec3,
) -> bool {
    let Some(sp) = project(handle) else {
        return false;
    };
    (screen - sp).length() <= crate::touch::hit(AXIS_GIZMO_HANDLE_HIT_RADIUS_PX)
}

/// Hit-test the offset arrow handle at a screen position.
pub fn offset_gizmo_hit(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    origin: Vec3,
    normal: Vec3,
    offset: f32,
) -> bool {
    let Some(sp) = project(offset_handle(origin, normal, offset)) else {
        return false;
    };
    (screen - sp).length() <= crate::touch::hit(AXIS_GIZMO_HANDLE_HIT_RADIUS_PX)
}

/// Hit-test axis gizmo handles at a screen position.
pub fn axis_gizmo_hit(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    origin: Vec3,
    direction: Vec3,
    offset: f32,
    angle_deg: f32,
) -> Option<AxisGizmoHit> {
    let normal = axis_normal(direction, angle_deg);
    if offset_gizmo_hit(screen, project, origin, normal, offset) {
        return Some(AxisGizmoHit::Offset);
    }
    let angle_pos = axis_angle_handle(origin, direction, angle_deg);
    if let Some(sp) = project(angle_pos) {
        if (screen - sp).length() <= crate::touch::hit(AXIS_GIZMO_HANDLE_HIT_RADIUS_PX) {
            return Some(AxisGizmoHit::Angle);
        }
    }
    None
}

/// Active drag on an axis gizmo handle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisGizmoDrag {
    pub hit: AxisGizmoHit,
    pub start_offset: f32,
    pub start_angle_deg: f32,
    pub start_screen: egui::Pos2,
}

/// World coordinate axis (origin triad).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum GlobalAxis {
    X,
    Y,
    Z,
}

impl GlobalAxis {
    pub fn direction(self) -> Vec3 {
        match self {
            GlobalAxis::X => Vec3::X,
            GlobalAxis::Y => Vec3::Y,
            GlobalAxis::Z => Vec3::Z,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GlobalAxis::X => "X axis",
            GlobalAxis::Y => "Y axis",
            GlobalAxis::Z => "Z axis",
        }
    }

    pub fn color(self) -> egui::Color32 {
        match self {
            GlobalAxis::X => egui::Color32::from_rgb(200, 70, 70),
            GlobalAxis::Y => egui::Color32::from_rgb(70, 190, 90),
            GlobalAxis::Z => egui::Color32::from_rgb(80, 140, 230),
        }
    }
}

/// Segment from the origin along a global axis (for picking and highlight).
pub fn global_axis_segment(axis: GlobalAxis) -> (Vec3, Vec3) {
    let e = GLOBAL_AXIS_EXTENT_MM;
    (Vec3::ZERO, axis.direction() * e)
}

fn draw_gizmo_handle_hover(
    painter: &egui::Painter,
    screen: egui::Pos2,
    accent: egui::Color32,
) {
    painter.circle_filled(screen, 9.0, accent.gamma_multiply(0.35));
    painter.circle_stroke(screen, 9.0, egui::Stroke::new(2.5, accent));
    painter.circle_stroke(screen, 14.0, egui::Stroke::new(1.5, accent.gamma_multiply(0.75)));
}

/// Draw the offset arrow gizmo along a plane normal.
pub fn draw_offset_gizmo(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    origin: Vec3,
    normal: Vec3,
    offset: f32,
    color: egui::Color32,
    hovered: bool,
) {
    let n = normal.normalize_or_zero();
    let tip = origin + n * gizmo_display_offset(offset);

    let offset_stroke = if hovered { 4.0 } else { 2.5 };
    let offset_color = if hovered {
        GIZMO_HANDLE_HOVER_RGBA
    } else {
        color
    };

    if let (Some(base), Some(end)) = (project(origin), project(tip)) {
        painter.line_segment([base, end], egui::Stroke::new(offset_stroke, offset_color));
        let shaft = end - base;
        if shaft.length_sq() > 1.0 {
            // Direction cones (filled triangles in this 2D fallback), one along each way
            // the handle can drag, slightly offset from the handle disc — mirrors the GPU
            // path's `push_gizmo_cone`.
            let dir = shaft.normalized();
            for sign in [1.0f32, -1.0] {
                draw_gizmo_arrow_2d(painter, end, dir * sign, 14.0, 8.0, 4.0, offset_color);
            }
        }
        if hovered {
            draw_gizmo_handle_hover(painter, end, GIZMO_HANDLE_HOVER_RGBA);
        } else {
            painter.circle_filled(end, 6.0, color);
            painter.circle_stroke(end, 6.0, egui::Stroke::new(1.5, color.gamma_multiply(0.5)));
        }
    }
}

/// Screen-space direction arrow for the 2D painter gizmo fallback: a line-drawn V at
/// `handle + dir * (gap + head)` pointing along `dir` — mirrors the GPU path's
/// `push_gizmo_arrowhead`.
fn draw_gizmo_arrow_2d(
    painter: &egui::Painter,
    handle: egui::Pos2,
    dir: egui::Vec2,
    gap: f32,
    head: f32,
    wing: f32,
    color: egui::Color32,
) {
    let tip = handle + dir * (gap + head);
    let base = tip - dir * head;
    let side = egui::vec2(-dir.y, dir.x) * wing;
    painter.line_segment([tip, base + side], egui::Stroke::new(2.0, color));
    painter.line_segment([tip, base - side], egui::Stroke::new(2.0, color));
}

/// Draw offset arrow and angle circle handles for an axis-referenced plane.
pub fn draw_axis_plane_gizmo(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    origin: Vec3,
    direction: Vec3,
    offset: f32,
    angle_deg: f32,
    color: egui::Color32,
    hover: Option<AxisGizmoHit>,
) {
    let normal = axis_normal(direction, angle_deg);
    draw_offset_gizmo(
        painter,
        project,
        origin,
        normal,
        offset,
        color,
        hover == Some(AxisGizmoHit::Offset),
    );

    let axis = direction.normalize_or_zero();
    let perp = axis_reference_perp(axis);
    let segments = 48;
    let mut circle_pts = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let dir = Quat::from_axis_angle(axis, a) * perp;
        if let Some(sp) = project(origin + dir * AXIS_ANGLE_GIZMO_RADIUS_MM) {
            circle_pts.push(sp);
        }
    }
    let angle_hovered = hover == Some(AxisGizmoHit::Angle);
    let circle_color = if angle_hovered {
        GIZMO_HANDLE_HOVER_RGBA.gamma_multiply(0.9)
    } else {
        color.gamma_multiply(0.85)
    };
    if circle_pts.len() >= 2 {
        painter.add(egui::Shape::line(
            circle_pts,
            egui::Stroke::new(if angle_hovered { 2.5 } else { 1.5 }, circle_color),
        ));
    }

    let handle = axis_angle_handle(origin, direction, angle_deg);
    let handle_dir = (handle - origin).normalize_or_zero();
    let angle_color = if angle_hovered {
        GIZMO_HANDLE_HOVER_RGBA
    } else {
        color
    };
    // Unify with the Face Snap / Free Move rotation dial (#1384): a radial line from the
    // origin to the 0° reference, the yellow arc up to the current angle, a radial line
    // origin→handle and a single disc at the handle.
    if let Some(center) = project(origin) {
        if angle_deg.abs() > 1e-3 {
            let segs = 48;
            let mut arc_pts = Vec::with_capacity(segs + 1);
            for i in 0..=segs {
                let t = angle_deg.to_radians() * i as f32 / segs as f32;
                let dir = Quat::from_axis_angle(axis, t) * perp;
                if let Some(sp) = project(origin + dir * AXIS_ANGLE_GIZMO_RADIUS_MM) {
                    arc_pts.push(sp);
                }
            }
            if arc_pts.len() >= 2 {
                painter.add(egui::Shape::line(
                    arc_pts,
                    egui::Stroke::new(2.5, AXIS_ANGLE_ARC),
                ));
            }
        }
        if let Some(start) = project(origin + perp * AXIS_ANGLE_GIZMO_RADIUS_MM) {
            painter.line_segment([center, start], egui::Stroke::new(1.5, angle_color));
        }
    }
    if let Some(sp) = project(handle) {
        if handle_dir != Vec3::ZERO {
            if let Some(center) = project(origin) {
                painter.line_segment([center, sp], egui::Stroke::new(2.0, angle_color));
            }
        }
        if angle_hovered {
            draw_gizmo_handle_hover(painter, sp, GIZMO_HANDLE_HOVER_RGBA);
        } else {
            painter.circle_filled(sp, 6.0, color);
        }
    }
}

/// Which geometry would be selected at a viewport position.
#[derive(Clone, Debug, PartialEq)]
pub enum PickTargetKind {
    /// A sketch point (line endpoint, rect corner, or circle center).
    Point(ConstraintPoint),
    /// A standalone sketch line segment.
    Line(crate::model::LineKey),
    /// A sketch circle (picked on its perimeter).
    Circle(crate::model::CircleKey),
    /// One feature edge of a 3D body's solid mesh (#31) — a mesh boundary or crease between
    /// two non-coplanar triangles, the same edges `ShadingMode::Wireframe` draws, extracted via
    /// `solid_mesh_unique_edges`. Works for any body (extrusion-sourced or STL/STEP-imported),
    /// since it's derived from the triangle mesh rather than an analytic profile.
    BodyEdge {
        body: crate::model::BodyKey,
        a: Vec3,
        b: Vec3,
    },
    /// A planar face of a 3D body's solid mesh (#144): the maximal edge-connected group of
    /// coplanar triangles under the cursor (see `solid_mesh_coplanar_faces`), in world space.
    /// Lets any face of any body — extrusion-sourced, boolean-cut, or imported — be hover-
    /// highlighted and referenced in 3D. `normal` orients the highlight fill toward the camera.
    BodyFace {
        body: crate::model::BodyKey,
        triangles: Vec<[Vec3; 3]>,
        normal: Vec3,
    },
    /// A **cylindrical** surface of a 3D body's solid mesh (#1013): a hole's wall, a boss, a
    /// shaft — the whole round wall, not one facet of it. Boxed because it carries the fitted
    /// surface's triangles alongside its axis and radius.
    BodyCylinder {
        body: crate::model::BodyKey,
        cylinder: Box<crate::extrude::BodyCylinder>,
    },
    /// A cylindrical surface's centre line (#1013), as the world segment it spans.
    BodyAxis { body: crate::model::BodyKey, a: Vec3, b: Vec3 },
    /// A vertex (corner) of a 3D body's solid mesh (#144), for 3D hover/selection.
    BodyVertex {
        body: crate::model::BodyKey,
        position: Vec3,
    },
    /// A **whole** solid body (#902). The Select tool resolves a click on a body's flat face to
    /// this — bodies outrank faces, while edges and corners still outrank bodies — and the
    /// Selection Exploder fans it as a leaf of its own, so the face under it stays reachable.
    /// Carries only the body key: everything else resolves from the document.
    Body(crate::model::BodyKey),
    GlobalAxis(GlobalAxis),
    /// One of a sketch's own origin axes (#189 / #1538): local X (u) or Y (v). Distinct
    /// from [`Self::GlobalAxis`] so a 2D tool can take LX/LY even when they don't coincide
    /// with the world triad.
    OriginAxis(crate::model::SketchAxis),
    ConstructionPlane(crate::model::ConstructionPlaneKey),
    /// A tracing image's displayed quad (#1561). Same pick band as a construction
    /// plane; when both sit under the cursor, the nearer (or the image, if coplanar
    /// with its host) wins.
    TracingImage(crate::model::TracingImageKey),
    Ground(Vec3),
    /// A sketch constraint's annotation icon (#568), by its index into `Document::constraints`.
    /// Constraints have no world geometry of their own — the icon is a screen-space glyph placed
    /// near the geometry it governs — so this is only ever produced for the Selection Exploder
    /// crowd (never by `resolve_pick_target`), letting a constraint icon buried under overlapping
    /// geometry be fanned out and selected like anything else.
    Constraint(crate::model::ConstraintKey),
    /// An analytic sketchable face (#625) — exactly what `face::pick_sketch_face` picks: a
    /// sketch profile (circle/polygon), a body cap/side wall, or a revolve's flat face. Like
    /// `Constraint`, this is only ever produced for the Selection Exploder crowd, so tools
    /// that pick faces (e.g. Extrude) fan out the same faces their own pick path accepts,
    /// rather than raw mesh facet groups.
    SketchFace(crate::model::FaceId),
    /// A thing on a drawing page (#1641): a projected view, a text note, or one view's edge
    /// dimension. Produced only for the Selection Exploder crowd on the drawing workbench,
    /// where the fan works in **page space** — the `anchor` of such a candidate is a page-mm
    /// point (z = 0) and `project` is the page-to-screen transform, so the same loupes, the
    /// same packing, and the same leader lines all apply.
    DrawingElement {
        drawing: crate::model::DrawingKey,
        element: crate::context::DrawingElementRef,
        /// The element's page-space outline (mm, z = 0), drawn inside its loupe.
        outline: Vec<Vec3>,
    },
}

/// A resolved pick target with its plane reference and screen-space distance.
#[derive(Clone, Debug, PartialEq)]
pub struct PickTarget {
    pub kind: PickTargetKind,
    pub reference: PlaneReference,
    distance_px: f32,
}

impl PickTarget {
    /// Draw a hover highlight for this target.
    pub fn draw_highlight(
        &self,
        painter: &egui::Painter,
        project: &impl Fn(Vec3) -> Option<egui::Pos2>,
        doc: &Document,
    ) {
        draw_pick_highlight(painter, project, doc, self.kind.clone(), PICK_HOVER_RGBA);
    }
}

/// Occlusion context for picking (#155): the eye position plus the visible bodies' solid
/// meshes, so [`resolve_pick_target`] can reject candidates hidden *behind* a body under
/// the cursor. Build once per pick (it meshes each visible body); pass `None` to keep the
/// old X-ray behavior (tests, callers without a camera).
pub struct PickOcclusion {
    eye: Vec3,
    meshes: Vec<crate::extrude::SolidMesh>,
    /// Snapshot of user-hidden state so [`resolve_pick_target`] can reject candidates that are
    /// hidden (or shadow), not just occluded behind a body (#258).
    visibility: crate::hierarchy::ElementVisibility,
    /// Bodies treated as absent for hit-testing (#1336): they neither occlude nor pick.
    ignore: Vec<crate::model::BodyKey>,
}

impl PickOcclusion {
    /// The camera eye this occlusion context was built with — used to depth-sort face picks (#565).
    pub fn eye(&self) -> Vec3 {
        self.eye
    }

    pub fn new(doc: &Document, visibility: &crate::hierarchy::ElementVisibility, eye: Vec3) -> Self {
        Self::new_ignoring(doc, visibility, eye, &[])
    }

    /// Like [`Self::new`], but the listed bodies are not there: they do not occlude, and
    /// their geometry is not pickable. Destination picks during Move use this so a click
    /// goes through the body being moved (#1336).
    pub fn new_ignoring(
        doc: &Document,
        visibility: &crate::hierarchy::ElementVisibility,
        eye: Vec3,
        ignore: &[crate::model::BodyKey],
    ) -> Self {
        let meshes = doc
            .bodies
            .iter()
            .filter(|(bi, body)| {
                // Shadow bodies neither render nor occlude/catch picks. Ignored bodies
                // (a Move destination pick's moving set) are the same (#1336).
                !ignore.contains(bi)
                    && !body.shadow
                    && visibility
                        .effective_visible(doc, crate::hierarchy::SceneElement::Body(*bi))
            })
            .filter_map(|(bi, _)| crate::extrude::body_solid_mesh(doc, bi))
            .collect();
        Self {
            eye,
            meshes,
            visibility: visibility.clone(),
            ignore: ignore.to_vec(),
        }
    }

    /// Whether a pick candidate is eligible for hover/selection given user-hidden and shadow
    /// state (#258): hidden elements (and anything hidden by a hidden ancestor) and shadow
    /// geometry are neither selectable nor hoverable. World axes and the ground plane are
    /// always pickable.
    pub fn pickable(&self, doc: &Document, kind: &PickTargetKind) -> bool {
        use crate::hierarchy::SceneElement;
        let vis = &self.visibility;
        match kind {
            PickTargetKind::Point(point) => {
                let shadow = match point {
                    ConstraintPoint::LineEndpoint { line, .. } => {
                        doc.lines.get(*line).is_some_and(|l| l.shadow)
                    }
                    ConstraintPoint::CircleCenter(c) => {
                        doc.circles.get(*c).is_some_and(|c| c.shadow)
                    }
                    ConstraintPoint::FaceVertex { .. }
                    | ConstraintPoint::TextAnchor { .. }
                    | ConstraintPoint::ImageCalibrationPoint { .. }
                    | ConstraintPoint::ImageAnchor { .. }
                    | ConstraintPoint::Origin => false,
                };
                !shadow && vis.effective_visible(doc, SceneElement::Point(point.clone()))
            }
            PickTargetKind::Line(i) => {
                doc.lines.get(*i).is_some_and(|l| !l.shadow)
                    && vis.effective_visible(doc, SceneElement::Line(*i))
            }
            PickTargetKind::Circle(i) => {
                doc.circles.get(*i).is_some_and(|c| !c.shadow)
                    && vis.effective_visible(doc, SceneElement::Circle(*i))
            }
            PickTargetKind::BodyEdge { body, .. }
            | PickTargetKind::BodyFace { body, .. }
            | PickTargetKind::BodyCylinder { body, .. }
            | PickTargetKind::BodyAxis { body, .. }
            | PickTargetKind::BodyVertex { body, .. }
            | PickTargetKind::Body(body) => {
                !self.ignore.contains(body)
                    && doc.bodies.get(*body).is_some_and(|b| !b.shadow)
                    && vis.effective_visible(doc, SceneElement::Body(*body))
            }
            PickTargetKind::ConstructionPlane(i) => {
                vis.effective_visible(doc, SceneElement::ConstructionPlane(*i))
            }
            PickTargetKind::TracingImage(i) => {
                vis.effective_visible(doc, SceneElement::Image(*i))
            }
            // A constraint badge is pickable when it is visible (its icon is only drawn for visible
            // constraints anyway, #568).
            PickTargetKind::Constraint(i) => {
                vis.effective_visible(doc, SceneElement::Constraint(*i))
            }
            // An analytic face is pickable when the element that hosts/produces it is
            // visible — the same owner mapping sketches on it would depend on (#625).
            PickTargetKind::SketchFace(face) => {
                vis.effective_visible(doc, crate::hierarchy::face_element(face.clone()))
            }
            PickTargetKind::GlobalAxis(_)
            | PickTargetKind::OriginAxis(_)
            // A drawing-page item's visibility is the page's own (#1641).
            | PickTargetKind::DrawingElement { .. }
            | PickTargetKind::Ground(_) => true,
        }
    }

    /// Whether a solid stands strictly between the eye and `p` (with slack at both ends so
    /// a point *on* a body's own surface doesn't occlude itself).
    pub fn occluded(&self, p: Vec3) -> bool {
        let dir = p - self.eye;
        let len = dir.length();
        if len < 1e-6 {
            return false;
        }
        const SLACK: f32 = 1e-3;
        self.meshes.iter().any(|mesh| {
            mesh.triangles.iter().any(|tri| {
                ray_triangle_t(self.eye, dir, tri)
                    .is_some_and(|t| t > SLACK && t < 1.0 - SLACK)
            })
        })
    }
}

/// Möller–Trumbore ray/triangle intersection: the ray parameter `t` where `origin + t*dir`
/// hits `tri`, or `None` for a miss (or a parallel/degenerate triangle).
fn ray_triangle_t(origin: Vec3, dir: Vec3, tri: &[Vec3; 3]) -> Option<f32> {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - tri[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// Closest world point on segment `a`-`b` to `screen`, measured in projected screen space —
/// the point the cursor is actually "on", used as the occlusion probe for edge candidates
/// (a partially hidden edge stays pickable on its visible stretch).
fn segment_point_nearest_screen(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    a: Vec3,
    b: Vec3,
) -> Vec3 {
    let (Some(pa), Some(pb)) = (project(a), project(b)) else {
        return segment_midpoint(a, b);
    };
    let ab = pb - pa;
    if ab.length_sq() < 1e-6 {
        return a;
    }
    let t = ((screen - pa).dot(ab) / ab.length_sq()).clamp(0.0, 1.0);
    a + (b - a) * t
}

/// Resolve the best pick target under the cursor (shared by hover and click). With an
/// [`PickOcclusion`] context, candidates hidden behind a visible body are skipped (#155) —
/// clicking a body never selects a line buried behind it.
pub fn resolve_pick_target(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ground_point: Option<Vec3>,
    doc: &Document,
    occlusion: Option<&PickOcclusion>,
) -> Option<PickTarget> {
    let mut best: Option<PickTarget> = None;

    let mut consider = |candidate: PickTarget| {
        if best.as_ref().is_none_or(|b| candidate.beats(b)) {
            best = Some(candidate);
        }
    };
    let visible = |p: Vec3| occlusion.is_none_or(|occ| !occ.occluded(p));
    // Hidden and shadow elements are not selectable/hoverable (#258); only enforced when we
    // have an occlusion context (the picking tools build one — tests/X-ray callers pass None).
    let pickable = |kind: &PickTargetKind| occlusion.is_none_or(|occ| occ.pickable(doc, kind));

    if let Some((kind, dist)) = nearest_sketch_point(screen, project, doc) {
        let origin = match &kind {
            PickTargetKind::Point(point) => {
                point_world_position(doc, point.clone()).unwrap_or(Vec3::ZERO)
            }
            _ => Vec3::ZERO,
        };
        if pickable(&kind) && visible(origin) {
            // A vertex on a line/curve anchors a plane *normal to the curve* at that
            // point (#474): the first incident direction is the default; the Plane tool
            // offers the rest when several curves meet there. A bare point (no incident
            // lines) keeps the horizontal-plane fallback.
            let (normal, label) = match &kind {
                PickTargetKind::Point(point) => vertex_normal_candidates(doc, point)
                    .into_iter()
                    .next()
                    .map(|(l, d)| (d, format!("Vertex ({l})")))
                    .unwrap_or((Vec3::Z, "Point".to_string())),
                _ => (Vec3::Z, "Point".to_string()),
            };
            consider(PickTarget {
                kind,
                reference: PlaneReference::Face {
                    origin,
                    normal,
                    label,
                },
                distance_px: dist,
            });
        }
    }

    if let Some((kind, a, b, label, dist)) = nearest_sketch_edge(screen, project, doc) {
        if pickable(&kind) && visible(segment_point_nearest_screen(screen, project, a, b)) {
            consider(PickTarget {
                kind,
                reference: PlaneReference::Axis {
                    origin: segment_midpoint(a, b),
                    direction: segment_direction(a, b),
                    label,
                },
                distance_px: dist,
            });
        }
    }

    if let Some((kind, a, b, label, dist)) = nearest_body_edge(screen, project, doc, occlusion) {
        if pickable(&kind) && visible(segment_point_nearest_screen(screen, project, a, b)) {
            consider(PickTarget {
                kind,
                reference: PlaneReference::Axis {
                    origin: segment_midpoint(a, b),
                    direction: segment_direction(a, b),
                    label,
                },
                distance_px: dist,
            });
        }
    }

    // A body **face** is selectable too (#565), but only where no edge/vertex is under the cursor:
    // it's ranked below them (priority 1 vs 0), so clicking near an edge still picks the edge and
    // clicking the face interior picks the face. Needs the camera eye to pick the front-most face,
    // so it's only offered when an occlusion context (which carries the eye) is present.
    if let Some(occ) = occlusion {
        if let Some(kind) = crate::face::pick_body_face_where(screen, project, doc, occ.eye(), |bi| {
            occ.pickable(doc, &PickTargetKind::Body(bi))
        }) {
            // A round wall (#1013): the hole itself, not the flat face it never was. It
            // anchors like a face — a plane through its axis pointing at the camera is the
            // only sensible reference — but keeps its own identity.
            if let PickTargetKind::BodyCylinder { cylinder, .. } = &kind {
                if pickable(&kind) {
                    let (origin, normal) = (cylinder.origin, cylinder.dir);
                    consider(PickTarget {
                        kind: kind.clone(),
                        reference: PlaneReference::Face {
                            origin,
                            normal,
                            label: "Cylinder".to_string(),
                        },
                        distance_px: 0.0,
                    });
                }
            }
            if let PickTargetKind::BodyFace { triangles, normal, .. } = &kind {
                if pickable(&kind) {
                    let n = (triangles.len() * 3).max(1) as f32;
                    let centroid =
                        triangles.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / n;
                    let normal = *normal;
                    consider(PickTarget {
                        kind: kind.clone(),
                        reference: PlaneReference::Face {
                            origin: centroid,
                            normal,
                            label: "Face".to_string(),
                        },
                        distance_px: 0.0,
                    });
                }
            }
        }
    }

    // A cylinder's **centre line** (#1013), ranked with the edges: the thing "put this hole
    // on that shaft" and "slide along this bore" are actually about.
    if let Some((kind, a, b, dist)) = nearest_body_axis(screen, project, doc) {
        if pickable(&kind) && visible(segment_point_nearest_screen(screen, project, a, b)) {
            consider(PickTarget {
                kind,
                reference: PlaneReference::Axis {
                    origin: segment_midpoint(a, b),
                    direction: segment_direction(a, b),
                    label: "Axis".to_string(),
                },
                distance_px: dist,
            });
        }
    }

    // An axis is only pickable where it isn't buried behind a body (#1099): test the point
    // on the segment nearest the cursor, the same way edges do. Without an occlusion
    // context (X-ray callers) the axis stays pickable through anything — the Selection
    // Exploder reaches a buried axis through its own crowd path (`collect_pick_candidates`),
    // which is deliberately not occlusion-gated.
    if let Some((axis, dist)) = nearest_global_axis(screen, project) {
        let (a, b) = global_axis_segment(axis);
        let nearest = segment_point_nearest_screen(screen, project, a, b);
        if pickable(&PickTargetKind::GlobalAxis(axis)) && visible(nearest) {
            consider(PickTarget {
                kind: PickTargetKind::GlobalAxis(axis),
                reference: PlaneReference::Axis {
                    origin: Vec3::ZERO,
                    direction: axis.direction(),
                    label: axis.label().to_string(),
                },
                distance_px: dist,
            });
        }
    }

    // Prefer the frontmost plane *or tracing image* under the cursor (#1277/#1561):
    // when several display quads contain the pointer, only the one nearer the eye is
    // what a click would take. An image coplanar with its host plane beats the plane
    // (it's the thing you're pointing at). Eye comes from the occlusion context when
    // present; without it, ties fall back to iteration order, with images beating
    // coplanar planes.
    let eye = occlusion.map(|occ| occ.eye());
    if let Some((kind, dist, at)) = nearest_plane_or_image(screen, project, doc, eye) {
        if pickable(&kind) {
            match &kind {
                PickTargetKind::ConstructionPlane(index) => {
                    let plane = &doc.construction_planes[*index];
                    let origin = ground_point.unwrap_or(plane.origin);
                    let projected = project_point_on_plane(origin, plane);
                    consider(PickTarget {
                        kind: kind.clone(),
                        reference: PlaneReference::Face {
                            origin: projected,
                            normal: plane.normal,
                            label: "Construction plane".to_string(),
                        },
                        distance_px: dist,
                    });
                }
                PickTargetKind::TracingImage(_) => {
                    let normal = match &kind {
                        PickTargetKind::TracingImage(i) => doc
                            .tracing_images
                            .get(*i)
                            .and_then(|img| doc.construction_planes.get(img.plane))
                            .map(|p| p.normal)
                            .unwrap_or(Vec3::Z),
                        _ => Vec3::Z,
                    };
                    consider(PickTarget {
                        kind: kind.clone(),
                        reference: PlaneReference::Face {
                            origin: at,
                            normal,
                            label: "Image".to_string(),
                        },
                        distance_px: dist,
                    });
                }
                _ => {}
            }
        }
    }

    if let Some(p) = ground_point {
        consider(PickTarget {
            kind: PickTargetKind::Ground(p),
            reference: PlaneReference::Face {
                origin: p,
                normal: Vec3::Z,
                label: "Ground".to_string(),
            },
            distance_px: f32::MAX,
        });
    }

    best
}

/// Body-face pick candidate for the Plane tool (#465): the planar body face under the
/// cursor as an offset-plane reference — origin at the face centroid, normal the face
/// normal — so a new plane can be anchored on any face of any body.
pub fn body_face_pick_target(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    eye: Vec3,
    occlusion: Option<&PickOcclusion>,
) -> Option<PickTarget> {
    let kind = crate::face::pick_body_face(screen, project, doc, eye)
        .filter(|kind| occlusion.is_none_or(|occ| occ.pickable(doc, kind)))?;
    let PickTargetKind::BodyFace {
        ref triangles,
        normal,
        ..
    } = kind
    else {
        return None;
    };
    let count = (triangles.len() * 3).max(1) as f32;
    let origin = triangles.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
    Some(PickTarget {
        kind,
        reference: PlaneReference::Face {
            origin,
            normal,
            label: "Face".to_string(),
        },
        distance_px: 0.0,
        // Beats the construction-plane quads (2) and ground (3); loses to the sharp
        // targets — points, edges, axes (0).
    })
}

/// The Plane tool's full pick (#465): a sharp target from [`resolve_pick_target`]
/// (point, edge, axis) wins; otherwise a body face under the cursor; otherwise the
/// construction-plane quad or ground fallback.
pub fn resolve_plane_pick_target(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    ground_point: Option<Vec3>,
    doc: &Document,
    eye: Vec3,
    occlusion: Option<&PickOcclusion>,
) -> Option<PickTarget> {
    let base = resolve_pick_target(screen, project, ground_point, doc, occlusion);
    // A sharp target — a point, an edge, an axis — already won, so keep it. "Sharp" is
    // everything ranked above a face in the shared priority (#959).
    let sharp = crate::element_picker::default_pick_band(crate::element_picker::ElementKind::Face);
    if base.as_ref().is_some_and(|t| pick_band(&t.kind) < sharp) {
        return base;
    }
    body_face_pick_target(screen, project, doc, eye, occlusion).or(base)
}

/// The pick-priority band of a candidate (#959): the shared
/// [`element_picker::default_pick_band`] ranking of the element it resolves to. The ground
/// plane has no element and is the last resort, so it ranks behind everything.
///
/// This replaces a `u8` hand-assigned at each candidate's construction site, plus a
/// vertex-beats-edge special case bolted onto the comparison — the band ordering says that
/// outright (a corner is band 0, an edge band 1), so the special case is gone.
pub fn pick_band(kind: &PickTargetKind) -> usize {
    match scene_element_from_pick(kind) {
        Some(element) => crate::element_picker::default_pick_band(
            crate::element_picker::ElementKind::of(&element),
        ),
        // The ground: whatever is under the cursor when nothing else is.
        None => usize::MAX,
    }
}

impl PickTarget {
    fn beats(&self, other: &PickTarget) -> bool {
        let (mine, theirs) = (pick_band(&self.kind), pick_band(&other.kind));
        if mine != theirs {
            return mine < theirs;
        }
        // Same band — a sketch line against a body edge, say — so the nearer one wins.
        self.distance_px < other.distance_px
    }
}

/// The plane-normal candidates at a sketch vertex (#474): for every line/curve end
/// meeting the point (via its coincidence group), the world-space direction the curve
/// leaves the vertex along — a straight line contributes its own direction, a curve the
/// tangent at that endpoint (toward its near bezier handle), each pointing *away* from
/// the geometry so a positive offset walks out past the vertex. Labeled by line name.
pub fn vertex_normal_candidates(
    doc: &Document,
    point: &crate::model::ConstraintPoint,
) -> Vec<(String, Vec3)> {
    let Some(sketch) = point_sketch(doc, point.clone()) else {
        return Vec::new();
    };
    let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) else {
        return Vec::new();
    };
    let mut ends: Vec<(crate::model::LineKey, crate::model::LineEnd)> =
        crate::vertex_drag::coincident_group(doc, sketch, point.clone())
            .into_iter()
            .filter_map(|p| match p {
                crate::model::ConstraintPoint::LineEndpoint { line, end } => Some((line, end)),
                _ => None,
            })
            .collect();
    ends.sort_by_key(|&(line, end)| (line, matches!(end, crate::model::LineEnd::End)));
    ends.dedup();
    let mut out = Vec::new();
    for (li, end) in ends {
        let Some(line) = doc.lines.get(li) else { continue };
        let (v, toward) = match end {
            crate::model::LineEnd::Start => {
                let toward = line
                    .bezier
                    .map(|b| b[0])
                    .unwrap_or((line.x1, line.y1));
                ((line.x0, line.y0), toward)
            }
            crate::model::LineEnd::End => {
                let toward = line
                    .bezier
                    .map(|b| b[1])
                    .unwrap_or((line.x0, line.y0));
                ((line.x1, line.y1), toward)
            }
        };
        let vw = crate::face::local_to_world(&frame, v.0, v.1);
        let tw = crate::face::local_to_world(&frame, toward.0, toward.1);
        let dir = (vw - tw).normalize_or_zero();
        if dir.length_squared() < 1e-8 {
            continue;
        }
        let label = crate::names::element_name(doc, crate::hierarchy::SceneElement::Line(li))
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("line {}", li.index()));
        out.push((label, dir));
    }
    out
}

/// Map a viewport pick to a scene-tree selection target, when selectable.
pub fn scene_element_from_pick(kind: &PickTargetKind) -> Option<SceneElement> {
    match kind {
        PickTargetKind::Point(point) => Some(SceneElement::Point(point.clone())),
        PickTargetKind::Line(index) => Some(SceneElement::Line(*index)),
        PickTargetKind::Circle(index) => Some(SceneElement::Circle(*index)),
        // 3D body sub-elements are selectable outside sketch mode (#156). Their identity is
        // the quantized geometry, canonically ordered so either traversal direction of the
        // same edge maps to one selection key.
        PickTargetKind::BodyEdge { body, a, b } => {
            let (qa, qb) = (
                crate::hierarchy::quantize_body_point(*a),
                crate::hierarchy::quantize_body_point(*b),
            );
            let (qa, qb) = if qa <= qb { (qa, qb) } else { (qb, qa) };
            Some(SceneElement::BodyEdge { body: *body, a: qa, b: qb })
        }
        PickTargetKind::BodyVertex { body, position } => Some(SceneElement::BodyVertex {
            body: *body,
            p: crate::hierarchy::quantize_body_point(*position),
        }),
        // A body face (#555/#557) is keyed by its quantized centroid + normal, so a face can be
        // selected/highlighted directly rather than falling through to a positional edge pick.
        // The centroid is the average of every triangle vertex (deterministic for a deterministic
        // mesh), so two picks of the same face yield the same key.
        PickTargetKind::BodyFace { body, triangles, normal } => {
            let count = (triangles.len() * 3).max(1) as f32;
            let centroid = triangles.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
            Some(SceneElement::BodyFace {
                body: *body,
                centroid: crate::hierarchy::quantize_body_point(centroid),
                normal: crate::hierarchy::quantize_body_point(*normal),
            })
        }
        // A round wall and its centre line (#1013): keyed by the fitted axis (and radius),
        // so two picks of the same hole compare equal.
        PickTargetKind::BodyCylinder { body, cylinder } => {
            Some(crate::extrude::cylinder_scene_element(*body, cylinder))
        }
        PickTargetKind::BodyAxis { body, a, b } => {
            let q = crate::hierarchy::quantize_body_point;
            let dir = crate::extrude::canonical_axis_direction(*b - *a);
            Some(SceneElement::BodyAxis {
                body: *body,
                origin: q((*a + *b) * 0.5),
                dir: q(dir),
            })
        }
        PickTargetKind::Constraint(index) => Some(SceneElement::Constraint(*index)),
        // The whole body (#902).
        PickTargetKind::Body(index) => Some(SceneElement::Body(*index)),
        // A world axis (#952): fixed geometry with no owning entity, like the origin, but
        // pickable — so it needs an identity an element picker can hold.
        PickTargetKind::GlobalAxis(axis) => Some(SceneElement::GlobalAxis(*axis)),
        PickTargetKind::OriginAxis(axis) => Some(SceneElement::FaceEdge(
            crate::model::ConstraintLine::OriginAxis(*axis),
        )),
        // An analytic face (#952) — a sketch profile, a body cap/side wall, a revolve's flat
        // face, or a construction plane. `from_face_id` normalizes the plane case so a plane
        // keeps a single identity.
        PickTargetKind::SketchFace(face) => Some(SceneElement::from_face_id(face.clone())),
        PickTargetKind::ConstructionPlane(index) => Some(SceneElement::ConstructionPlane(*index)),
        PickTargetKind::TracingImage(index) => Some(SceneElement::Image(*index)),
        // A drawing-page item (#1641): the fan reports it like any other element.
        PickTargetKind::DrawingElement { drawing, element, .. } => {
            Some(SceneElement::DrawingElement { drawing: *drawing, element: element.clone() })
        }
        _ => None,
    }
}

/// Whether the viewport draws this candidate straight through solids (#1720). The world triad
/// and a sketch's origin axes are infinite datum lines painted over everything, so a click on
/// one is a click on what you can see -- the "take what isn't buried" gate (#1578) must not
/// throw them away just because a body happens to stand between them and the eye.
pub fn draws_through_solids(kind: &PickTargetKind) -> bool {
    matches!(
        kind,
        PickTargetKind::GlobalAxis(_) | PickTargetKind::OriginAxis(_)
    )
}

/// Every feature edge of a body's solid mesh (#902), in world space — what a whole-body
/// hover/loupe draws. Empty when the body doesn't mesh.
///
/// Uses the memoized analysis cache (#845/#1141): circular hole rims facet into hundreds of
/// segments, and recomputing them for every loupe/hover frame was measurable on holey parts.
pub fn body_feature_edges(doc: &Document, body: crate::model::BodyKey) -> Vec<(Vec3, Vec3)> {
    crate::extrude::body_feature_edges(doc, body).as_ref().clone()
}

/// World boundary loop of an analytic sketchable face (#625), for the exploder's highlights
/// and loupe previews: a circle profile's perimeter, a polygon profile's loop, a plane's
/// display quad, or a body/revolve face's analytic boundary.
pub fn sketch_face_boundary_world(doc: &Document, face: &FaceId) -> Option<Vec<Vec3>> {
    match face {
        FaceId::Circle(i) => doc
            .circles
            .get(*i)
            .and_then(|c| crate::face::circle_world_perimeter(doc, c, 48)),
        FaceId::Polygon(lines) => crate::extrude::face_profile_world(
            doc,
            &crate::model::ExtrudeFace::Polygon(lines.clone()),
        )
        .map(|(p, _)| p),
        FaceId::ConstructionPlane(i) => doc
            .construction_planes
            .get(*i)
            .map(|p| plane_corners(p).to_vec()),
        _ => crate::extrude::face_boundary_loop_world(doc, face),
    }
}

/// Draw a hover highlight for a pickable target.
pub fn draw_pick_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    kind: PickTargetKind,
    color: egui::Color32,
) {
    match kind {
        PickTargetKind::Point(point) => {
            if let Some(world) = point_world_position(doc, point) {
                if let Some(sp) = project(world) {
                    painter.circle_filled(sp, 6.0, color);
                    painter.circle_stroke(sp, 6.0, egui::Stroke::new(2.0, color));
                }
            }
        }
        PickTargetKind::Line(index) => {
            if let Some(line) = doc.lines.get(index) {
                draw_line_highlight(painter, project, doc, line, color);
            }
        }
        PickTargetKind::Circle(index) => {
            if let Some(circle) = doc.circles.get(index) {
                draw_circle_highlight(painter, project, doc, circle, color);
            }
        }
        PickTargetKind::BodyEdge { body, a, b } => {
            // The whole tangent-continuous curve, not just the picked facet (#626).
            let chain = crate::extrude::body_solid_mesh(doc, body)
                .map(|s| crate::gpu_viewport::body_edge_curve_chain(&s, a, b))
                .unwrap_or_else(|| vec![(a, b)]);
            for (sa, sb) in chain {
                draw_segment_highlight(painter, project, sa, sb, color);
            }
        }
        PickTargetKind::BodyFace { triangles, .. } => {
            let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
            for tri in &triangles {
                if let (Some(a), Some(b), Some(c)) =
                    (project(tri[0]), project(tri[1]), project(tri[2]))
                {
                    painter.add(egui::Shape::convex_polygon(
                        vec![a, b, c],
                        fill,
                        egui::Stroke::NONE,
                    ));
                }
            }
            for (a, b) in coplanar_face_boundary(&triangles) {
                draw_segment_highlight(painter, project, a, b, color);
            }
        }
        // A round wall (#1013): its facets filled, like a flat face's.
        PickTargetKind::BodyCylinder { cylinder, .. } => {
            let fill = color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER);
            for tri in &cylinder.triangles {
                if let (Some(a), Some(b), Some(c)) =
                    (project(tri[0]), project(tri[1]), project(tri[2]))
                {
                    painter.add(egui::Shape::convex_polygon(
                        vec![a, b, c],
                        fill,
                        egui::Stroke::NONE,
                    ));
                }
            }
        }
        // Its centre line, drawn as the segment it spans.
        PickTargetKind::BodyAxis { a, b, .. } => {
            draw_segment_highlight(painter, project, a, b, color);
        }
        PickTargetKind::BodyVertex { position, .. } => {
            if let Some(sp) = project(position) {
                painter.circle_filled(sp, 5.0, color);
                painter.circle_stroke(sp, 5.0, egui::Stroke::new(2.0, color));
            }
        }
        PickTargetKind::GlobalAxis(axis) => {
            let (a, b) = global_axis_segment(axis);
            let axis_color = axis.color().gamma_multiply(1.25);
            draw_segment_highlight(painter, project, a, b, axis_color);
        }
        PickTargetKind::OriginAxis(axis) => {
            // Highlight every sketch's matching origin axis — the pick identity is just
            // X or Y, and the open sketch's copy is what the GPU hover already draws.
            for (sketch, _) in doc.sketches.iter() {
                let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) else {
                    continue;
                };
                let dir = match axis {
                    crate::model::SketchAxis::X => frame.u_axis,
                    crate::model::SketchAxis::Y => frame.v_axis,
                };
                let half = GLOBAL_AXIS_EXTENT_MM;
                draw_segment_highlight(
                    painter,
                    project,
                    frame.origin - dir * half,
                    frame.origin + dir * half,
                    color,
                );
            }
        }
        PickTargetKind::ConstructionPlane(index) => {
            if let Some(plane) = doc.construction_planes.get(index) {
                draw_plane_face_highlight(painter, project, plane, color);
            }
        }
        PickTargetKind::TracingImage(index) => {
            if let Some(corners) = tracing_image_corners(doc, index) {
                for i in 0..corners.len() {
                    draw_segment_highlight(
                        painter,
                        project,
                        corners[i],
                        corners[(i + 1) % corners.len()],
                        color,
                    );
                }
            }
        }
        PickTargetKind::Ground(p) => {
            if let Some(sp) = project(p) {
                painter.circle_stroke(sp, 8.0, egui::Stroke::new(2.0, color));
                let r = 6.0;
                painter.line_segment(
                    [sp + egui::vec2(-r, 0.0), sp + egui::vec2(r, 0.0)],
                    egui::Stroke::new(2.0, color),
                );
                painter.line_segment(
                    [sp + egui::vec2(0.0, -r), sp + egui::vec2(0.0, r)],
                    egui::Stroke::new(2.0, color),
                );
            }
        }
        // A constraint's hover highlight is its badge lighting up in the annotation overlay (#568),
        // driven separately via `draw_constraint_icons`'s hovered set — nothing to draw in the
        // world-geometry layer here.
        PickTargetKind::Constraint(_) => {}
        // The whole body (#902): every feature edge of its mesh lights up, so it reads as the
        // body rather than one of its faces.
        PickTargetKind::Body(body) => {
            for (a, b) in body_feature_edges(doc, body) {
                draw_segment_highlight(painter, project, a, b, color);
            }
        }
        // An analytic face (#625): outline its boundary loop.
        PickTargetKind::SketchFace(face) => {
            if let Some(pts) = sketch_face_boundary_world(doc, &face) {
                for i in 0..pts.len() {
                    draw_segment_highlight(painter, project, pts[i], pts[(i + 1) % pts.len()], color);
                }
            }
        }
        // A drawing-page item (#1641): its page-space outline, drawn through the page's own
        // transform (which is what `project` is on the drawing workbench).
        PickTargetKind::DrawingElement { outline, .. } => {
            for w in outline.windows(2) {
                draw_segment_highlight(painter, project, w[0], w[1], color);
            }
        }
    }
}

fn draw_line_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    line: &Line,
    color: egui::Color32,
) {
    let Some(points) = line_world_polyline(doc, line) else {
        return;
    };
    for pair in points.windows(2) {
        if let (Some(pa), Some(pb)) = (project(pair[0]), project(pair[1])) {
            painter.line_segment([pa, pb], egui::Stroke::new(4.0, color));
        }
    }
    if let (Some(&a), Some(&b)) = (points.first(), points.last()) {
        for p in [a, b] {
            if let Some(sp) = project(p) {
                painter.circle_filled(sp, 5.0, color);
            }
        }
    }
}

fn draw_segment_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    a: Vec3,
    b: Vec3,
    color: egui::Color32,
) {
    if let (Some(pa), Some(pb)) = (project(a), project(b)) {
        painter.line_segment([pa, pb], egui::Stroke::new(4.0, color));
        for p in [pa, pb] {
            painter.circle_filled(p, 5.0, color);
        }
    }
}

/// Highlight a sketchable circle face with a filled overlay and border.
pub fn draw_circle_face_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    circle: &crate::model::Circle,
    color: egui::Color32,
) {
    let Some(pts_world) = crate::face::circle_world_perimeter(doc, circle, 48) else {
        return;
    };
    let pts: Option<Vec<egui::Pos2>> = pts_world.iter().map(|p| project(*p)).collect();
    let Some(pts) = pts else { return };
    painter.add(egui::Shape::convex_polygon(
        pts,
        color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER),
        egui::Stroke::new(2.0, color),
    ));
}

/// Highlight a sketchable face quad with a filled overlay and border.
pub fn draw_quad_face_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    corners: [Vec3; 4],
    color: egui::Color32,
) {
    let pts: Option<Vec<egui::Pos2>> = corners.iter().map(|&c| project(c)).collect();
    let Some(pts) = pts else { return };
    painter.add(egui::Shape::convex_polygon(
        pts,
        color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER),
        egui::Stroke::new(2.0, color),
    ));
}

/// Highlight an arbitrary planar face given by its world-space boundary loop.
pub fn draw_polygon_face_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    poly: &[Vec3],
    color: egui::Color32,
) {
    let pts: Option<Vec<egui::Pos2>> = poly.iter().map(|&p| project(p)).collect();
    let Some(pts) = pts else { return };
    if pts.len() < 3 {
        return;
    }
    let normal = (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
    for [a, b, c] in crate::polygon::triangulate_planar(poly, normal) {
        painter.add(egui::Shape::convex_polygon(
            vec![pts[a], pts[b], pts[c]],
            color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER),
            egui::Stroke::new(2.0, color),
        ));
    }
}

/// Like [`draw_polygon_face_highlight`], but with hole loops cut out (#519). A hovered
/// boolean-difference face (an inset border) or text glyph is an annular region: filling only
/// its outer ring painted a solid patch across the opening in the middle. This fills the true
/// holed region and outlines the outer ring and each hole boundary.
pub fn draw_region_face_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    outer: &[Vec3],
    holes: &[Vec<Vec3>],
    color: egui::Color32,
) {
    if outer.len() < 3 {
        return;
    }
    if holes.is_empty() {
        draw_polygon_face_highlight(painter, project, outer, color);
        return;
    }
    let normal = (outer[1] - outer[0]).cross(outer[2] - outer[0]).normalize_or_zero();
    // Fill the region between the outer ring and the holes.
    for tri in crate::polygon::triangulate_planar_with_holes(outer, holes, normal) {
        let pts: Option<Vec<egui::Pos2>> = tri.iter().map(|&p| project(p)).collect();
        if let Some(pts) = pts {
            painter.add(egui::Shape::convex_polygon(
                pts,
                color.gamma_multiply(FACE_HOVER_FILL_MULTIPLIER),
                egui::Stroke::NONE,
            ));
        }
    }
    // Outline the outer ring and every hole, so both boundaries of the border read.
    for ring in std::iter::once(outer).chain(holes.iter().map(|h| h.as_slice())) {
        let pts: Option<Vec<egui::Pos2>> = ring.iter().map(|&p| project(p)).collect();
        if let Some(pts) = pts {
            if pts.len() >= 2 {
                painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(2.0, color)));
            }
        }
    }
}

fn draw_plane_face_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    plane: &ConstructionPlane,
    color: egui::Color32,
) {
    let corners = plane_corners(plane);
    draw_quad_face_highlight(painter, project, corners, color);
}

fn project_point_on_plane(point: Vec3, plane: &ConstructionPlane) -> Vec3 {
    let n = plane.normal;
    let dist = (point - plane.origin).dot(n);
    point - n * dist
}

fn segment_midpoint(a: Vec3, b: Vec3) -> Vec3 {
    (a + b) * 0.5
}

fn segment_direction(a: Vec3, b: Vec3) -> Vec3 {
    (b - a).normalize_or_zero()
}

fn point_in_screen_quad(p: egui::Pos2, quad: [egui::Pos2; 4]) -> bool {
    // Split quad into two triangles and test barycentric inclusion.
    point_in_tri(p, quad[0], quad[1], quad[2]) || point_in_tri(p, quad[0], quad[2], quad[3])
}

fn point_in_tri(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2, c: egui::Pos2) -> bool {
    let v0 = c - a;
    let v1 = b - a;
    let v2 = p - a;
    let dot00 = v0.dot(v0);
    let dot01 = v0.dot(v1);
    let dot02 = v0.dot(v2);
    let dot11 = v1.dot(v1);
    let dot12 = v1.dot(v2);
    let denom = dot00 * dot11 - dot01 * dot01;
    if denom.abs() < 1e-8 {
        return false;
    }
    let inv = 1.0 / denom;
    let u = (dot11 * dot02 - dot01 * dot12) * inv;
    let v = (dot00 * dot12 - dot01 * dot02) * inv;
    u >= 0.0 && v >= 0.0 && (u + v) <= 1.0
}

fn dist_point_to_segment_px(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    if ab.length_sq() < 1e-4 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

fn segment_pick_distance(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    a: Vec3,
    b: Vec3,
) -> Option<f32> {
    let (Some(pa), Some(pb)) = (project(a), project(b)) else {
        return None;
    };
    let seg_dist = dist_point_to_segment_px(screen, pa, pb);
    let end_a = (screen - pa).length();
    let end_b = (screen - pb).length();
    let dist = seg_dist.min(end_a).min(end_b);
    let threshold = if end_a <= crate::touch::hit(POINT_PICK_RADIUS_PX)
        || end_b <= crate::touch::hit(POINT_PICK_RADIUS_PX)
    {
        crate::touch::hit(POINT_PICK_RADIUS_PX)
    } else {
        crate::touch::hit(LINE_PICK_RADIUS_PX)
    };
    if dist <= threshold {
        Some(dist)
    } else {
        None
    }
}

pub fn point_world_position(doc: &Document, point: ConstraintPoint) -> Option<Vec3> {
    use crate::face::{circle_world_center, local_to_world, sketch_geometry_frame};
    match point {
        ConstraintPoint::LineEndpoint { line, end } => {
            let entity = doc.lines.get(line)?;
            let frame = sketch_geometry_frame(doc, entity.sketch)?;
            let (u, v) = match end {
                LineEnd::Start => (entity.x0, entity.y0),
                LineEnd::End => (entity.x1, entity.y1),
            };
            Some(local_to_world(&frame, u, v))
        }
        ConstraintPoint::CircleCenter(circle) => {
            let entity = doc.circles.get(circle)?;
            circle_world_center(doc, entity)
        }
        ConstraintPoint::Origin => None,
        // Already a world-space point (#26/#27) — no sketch frame to project through.
        ConstraintPoint::FaceVertex { face, index } => {
            crate::extrude::face_boundary_loop_world(doc, &face)?.get(index).copied()
        }
        ConstraintPoint::TextAnchor { text, anchor } => {
            let entity = doc.sketch_texts.get(text)?;
            let frame = sketch_geometry_frame(doc, entity.sketch)?;
            let (u, v) = crate::text::sketch_text_anchor_uv(entity, anchor);
            Some(local_to_world(&frame, u, v))
        }
        ConstraintPoint::ImageCalibrationPoint { image, index } => {
            let img = doc.tracing_images.get(image)?;
            let (u, v) = crate::model::image_calibration_point_uv(img, index)?;
            let frame =
                crate::face::sketch_frame(doc, crate::model::FaceId::ConstructionPlane(img.plane))?;
            Some(frame.origin + frame.u_axis * u + frame.v_axis * v)
        }
        ConstraintPoint::ImageAnchor { image, anchor } => {
            let img = doc.tracing_images.get(image)?;
            let (u, v) = crate::model::image_anchor_uv(img, anchor);
            let frame =
                crate::face::sketch_frame(doc, crate::model::FaceId::ConstructionPlane(img.plane))?;
            Some(frame.origin + frame.u_axis * u + frame.v_axis * v)
        }
    }
}

/// Nearest sketch vertex in `sketch` under the cursor, if any.
pub fn nearest_sketch_point_in_sketch(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    sketch: SketchId,
) -> Option<(ConstraintPoint, f32)> {
    let mut best: Option<(ConstraintPoint, f32)> = None;

    let mut consider = |point: ConstraintPoint, world: Vec3| {
        if point_sketch(doc, point.clone()) != Some(sketch) {
            return;
        }
        let Some(sp) = project(world) else {
            return;
        };
        let dist = (screen - sp).length();
        if dist <= crate::touch::hit(POINT_PICK_RADIUS_PX) && best.as_ref().is_none_or(|(_, d)| dist < *d) {
            best = Some((point, dist));
        }
    };

    for (li, line) in doc.lines.iter() {
        if line.sketch != sketch {
            continue;
        }
        let Some((a, b)) = line_world_endpoints(doc, line) else {
            continue;
        };
        consider(
            ConstraintPoint::LineEndpoint {
                line: li,
                end: LineEnd::Start,
            },
            a,
        );
        consider(
            ConstraintPoint::LineEndpoint {
                line: li,
                end: LineEnd::End,
            },
            b,
        );
    }

    for (ci, circle) in doc.circles.iter() {
        if circle.sketch != sketch {
            continue;
        }
        if let Some(center) = crate::face::circle_world_center(doc, circle) {
            consider(ConstraintPoint::CircleCenter(ci), center);
        }
    }

    // A text's nine anchor points (#408) are constrainable vertices too.
    for (ti, text) in doc.sketch_texts.iter() {
        if text.sketch != sketch {
            continue;
        }
        if let Some(frame) = crate::face::sketch_geometry_frame(doc, text.sketch) {
            for anchor in crate::model::TextAnchor::ALL {
                let (u, v) = crate::text::sketch_text_anchor_uv(text, anchor);
                consider(
                    ConstraintPoint::TextAnchor { text: ti, anchor },
                    crate::face::local_to_world(&frame, u, v),
                );
            }
        }
    }
    // A calibrated image's two reference points (#425) and nine box anchors (#1589),
    // for images on this sketch's plane. `point_sketch` is `None` for these (the image
    // sits on a plane, not in a sketch), so they go through the FaceVertex-style
    // direct check rather than the `consider` sketch filter. Calibration first so they
    // win a tie with a coincident top/bottom-middle box point.
    if let Some(face) = doc.sketch_face(sketch) {
        if let FaceId::ConstructionPlane(plane) = face {
            if let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) {
                for (ii, img) in doc.tracing_images.iter() {
                    if img.plane != plane {
                        continue;
                    }
                    let consider_img = |point: ConstraintPoint, u: f32, v: f32, best: &mut Option<(ConstraintPoint, f32)>| {
                        let world = crate::face::local_to_world(&frame, u, v);
                        let Some(sp) = project(world) else {
                            return;
                        };
                        let dist = (screen - sp).length();
                        if dist <= crate::touch::hit(POINT_PICK_RADIUS_PX)
                            && best.as_ref().is_none_or(|(_, d)| dist < *d)
                        {
                            *best = Some((point, dist));
                        }
                    };
                    for index in 0..2 {
                        if let Some((u, v)) = crate::model::image_calibration_point_uv(img, index) {
                            consider_img(
                                ConstraintPoint::ImageCalibrationPoint { image: ii, index },
                                u,
                                v,
                                &mut best,
                            );
                        }
                    }
                    for anchor in crate::model::TextAnchor::ALL {
                        let (u, v) = crate::model::image_anchor_uv(img, anchor);
                        consider_img(
                            ConstraintPoint::ImageAnchor { image: ii, anchor },
                            u,
                            v,
                            &mut best,
                        );
                    }
                }
            }
        }
    }

    // A sketch open directly on a body's own extrusion cap/side face (#26/#27) can also
    // constrain to that face's own boundary vertices. `point_sketch` can't recognize these
    // (a `FaceVertex` has no owning sketch, unlike sketch-native entities above), so they're
    // considered directly rather than through the shared `consider` closure's sketch filter.
    // Scoped to the *active sketch's own face* only, per the issue — not arbitrary other faces.
    if let Some(face) = doc.sketch_face(sketch) {
        if matches!(face, FaceId::ExtrudeCap { .. } | FaceId::ExtrudeSide { .. }) {
            if let Some(loop_) = crate::extrude::face_boundary_loop_world(doc, &face) {
                for (index, world) in loop_.into_iter().enumerate() {
                    let Some(sp) = project(world) else {
                        continue;
                    };
                    let dist = (screen - sp).length();
                    if dist <= crate::touch::hit(POINT_PICK_RADIUS_PX) && best.as_ref().is_none_or(|(_, d)| dist < *d) {
                        best = Some((
                            ConstraintPoint::FaceVertex {
                                face: face.clone(),
                                index,
                            },
                            dist,
                        ));
                    }
                }
            }
        }
    }

    best
}

/// Nearest line or rectangle edge in `sketch` under the cursor (not vertices).
pub fn nearest_sketch_line_in_sketch(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    sketch: SketchId,
) -> Option<(crate::model::ConstraintLine, f32)> {
    use crate::model::ConstraintLine;
    let mut best: Option<(ConstraintLine, f32)> = None;

    let mut consider = |line: ConstraintLine, a: Vec3, b: Vec3| {
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            return;
        };
        if best.as_ref().is_none_or(|(_, d)| dist < *d) {
            best = Some((line, dist));
        }
    };

    for (li, line) in doc.lines.iter() {
        if line.sketch != sketch {
            continue;
        }
        let Some(points) = line_world_polyline(doc, line) else {
            continue;
        };
        for pair in points.windows(2) {
            consider(ConstraintLine::Line(li), pair[0], pair[1]);
        }
    }

    // Edges of the sketch's own body face (#26/#27), scoped exactly like the vertex loop in
    // `nearest_sketch_point_in_sketch` above. Vertices win over edges via the existing caller
    // precedence: callers already check `nearest_sketch_point_in_sketch` first and skip this
    // function on a hit (see e.g. `handle_vertex_drag`/`handle_line_drag` in main.rs).
    if let Some(face) = doc.sketch_face(sketch) {
        if matches!(face, FaceId::ExtrudeCap { .. } | FaceId::ExtrudeSide { .. }) {
            if let Some(loop_) = crate::extrude::face_boundary_loop_world(doc, &face) {
                let n = loop_.len();
                for index in 0..n {
                    consider(
                        ConstraintLine::FaceEdge {
                            face: face.clone(),
                            index,
                        },
                        loop_[index],
                        loop_[(index + 1) % n],
                    );
                }
            }
        }
    }

    // Tracing-image displayed-quad edges (#1589), for images on this sketch's plane.
    for (ii, img) in doc.tracing_images.iter() {
        if doc.sketch_face(sketch) != Some(FaceId::ConstructionPlane(img.plane)) {
            continue;
        }
        let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) else {
            continue;
        };
        for edge in crate::model::ImageEdge::ALL {
            let ((u0, v0), (u1, v1)) = crate::model::image_edge_uv(img, edge);
            consider(
                ConstraintLine::ImageEdge { image: ii, edge },
                crate::face::local_to_world(&frame, u0, v0),
                crate::face::local_to_world(&frame, u1, v1),
            );
        }
    }

    // The origin axes (#189) are pickable everywhere as fixed reference lines, so a point or
    // line can be constrained onto one from the constraint tool (not only by snapping).
    // Measured as an **infinite line in screen space** from two nearby projected points
    // (#394): the old ±10 m segment endpoints usually fail to project (behind the camera /
    // outside the frustum), which silently made the axes unpickable and unhoverable.
    //
    // Lose near-ties to real sketch geometry (#1183): a true plan view stacks face edges that
    // lie on the sketch u/v axes with those axes in projection; the body edge is the more
    // specific pick when both land under the cursor.
    if let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) {
        let mut consider_axis = |axis: crate::model::SketchAxis, dir: Vec3| {
            let (Some(p0), Some(p1)) = (
                project(frame.origin),
                project(frame.origin + dir * 10.0),
            ) else {
                return;
            };
            let d = p1 - p0;
            if d.length_sq() < 1e-6 {
                return;
            }
            let dn = d / d.length();
            let rel = screen - p0;
            let dist = (rel.x * dn.y - rel.y * dn.x).abs();
            const ORIGIN_AXIS_TIE_EPS_PX: f32 = 0.5;
            if dist <= crate::touch::hit(LINE_PICK_RADIUS_PX)
                && best
                    .as_ref()
                    .is_none_or(|(_, best_d)| dist + ORIGIN_AXIS_TIE_EPS_PX < *best_d)
            {
                best = Some((ConstraintLine::OriginAxis(axis), dist));
            }
        };
        consider_axis(crate::model::SketchAxis::X, frame.u_axis);
        consider_axis(crate::model::SketchAxis::Y, frame.v_axis);
    }

    best
}

/// Whether any sketch is hosted on this image's plane — the gate for treating
/// calibration endpoints as first-class constraint points (#425). Without a
/// hosted sketch they belong only to the Select-tool overlay (#1547/#1586).
fn image_hosts_a_sketch(doc: &Document, img: &crate::model::TracingImage) -> bool {
    doc.sketches.keys().any(|sketch| {
        doc.sketch_face(sketch) == Some(FaceId::ConstructionPlane(img.plane))
    })
}

fn nearest_sketch_point(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
) -> Option<(PickTargetKind, f32)> {
    let mut best: Option<(PickTargetKind, f32)> = None;

    let mut consider = |point: ConstraintPoint, world: Vec3| {
        let Some(sp) = project(world) else {
            return;
        };
        let dist = (screen - sp).length();
        if dist <= crate::touch::hit(POINT_PICK_RADIUS_PX)
            && best.as_ref().is_none_or(|(_, d)| dist < *d)
        {
            best = Some((PickTargetKind::Point(point), dist));
        }
    };

    for (li, line) in doc.lines.iter() {
        let Some((a, b)) = line_world_endpoints(doc, line) else {
            continue;
        };
        consider(
            ConstraintPoint::LineEndpoint {
                line: li,
                end: LineEnd::Start,
            },
            a,
        );
        consider(
            ConstraintPoint::LineEndpoint {
                line: li,
                end: LineEnd::End,
            },
            b,
        );
    }

    for (ci, circle) in doc.circles.iter() {
        if let Some(center) = crate::face::circle_world_center(doc, circle) {
            consider(ConstraintPoint::CircleCenter(ci), center);
        }
    }

    // A text's nine anchor points (#408): pickable like any vertex, so the constraint tool
    // can hold a text's corner or centre to other geometry.
    for (ti, text) in doc.sketch_texts.iter() {
        let Some(frame) = crate::face::sketch_geometry_frame(doc, text.sketch) else {
            continue;
        };
        for anchor in crate::model::TextAnchor::ALL {
            let (u, v) = crate::text::sketch_text_anchor_uv(text, anchor);
            consider(
                ConstraintPoint::TextAnchor { text: ti, anchor },
                crate::face::local_to_world(&frame, u, v),
            );
        }
    }
    // A calibrated image's two reference points (#425): only when a sketch is
    // hosted on the image's plane. Otherwise they are the Select-tool overlay
    // and must not steal the image pick (#1586).
    for (ii, img) in doc.tracing_images.iter() {
        if !image_hosts_a_sketch(doc, img) {
            continue;
        }
        let Some(frame) =
            crate::face::sketch_frame(doc, FaceId::ConstructionPlane(img.plane))
        else {
            continue;
        };
        for index in 0..2 {
            if let Some((u, v)) = crate::model::image_calibration_point_uv(img, index) {
                consider(
                    ConstraintPoint::ImageCalibrationPoint { image: ii, index },
                    frame.origin + frame.u_axis * u + frame.v_axis * v,
                );
            }
        }
        for anchor in crate::model::TextAnchor::ALL {
            let (u, v) = crate::model::image_anchor_uv(img, anchor);
            consider(
                ConstraintPoint::ImageAnchor { image: ii, anchor },
                frame.origin + frame.u_axis * u + frame.v_axis * v,
            );
        }
    }

    best
}

fn nearest_sketch_edge(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
) -> Option<(PickTargetKind, Vec3, Vec3, String, f32)> {
    let mut best: Option<(PickTargetKind, Vec3, Vec3, String, f32)> = None;

    let mut consider = |kind: PickTargetKind, a: Vec3, b: Vec3, label: &str| {
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            return;
        };
        if best.as_ref().is_none_or(|(_, _, _, _, d)| dist < *d) {
            best = Some((kind, a, b, label.to_string(), dist));
        }
    };

    for (li, line) in doc.lines.iter() {
        let Some(points) = line_world_polyline(doc, line) else {
            continue;
        };
        for pair in points.windows(2) {
            consider(PickTargetKind::Line(li), pair[0], pair[1], "Line");
        }
    }

    for (ci, circle) in doc.circles.iter() {
        let Some(pts) = crate::face::circle_world_perimeter(doc, circle, 32) else {
            continue;
        };
        for window in pts.windows(2) {
            consider(
                PickTargetKind::Circle(ci),
                window[0],
                window[1],
                "Circle",
            );
        }
    }

    best
}

/// Whether the cursor could possibly be inside a world box, judged in screen space (#1026).
///
/// The pick path runs every frame the camera moves and used to project **every triangle of
/// every body** to answer "what is under the cursor". This rejects a whole body — or a whole
/// face — with eight projections instead, which is what makes hover cheap enough to leave
/// running during a zoom.
///
/// **Conservative by construction.** A box's corners projected to screen bound the projection
/// of everything inside it, so a cursor outside that rectangle cannot be over the geometry.
/// Where a corner sits behind the camera it has no projection at all and the box is *accepted*
/// — a wrong rejection would silently drop a pick, which is far worse than a wasted test.
pub fn screen_bounds_hit(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    bounds: (Vec3, Vec3),
    margin: f32,
) -> bool {
    let (min, max) = bounds;
    let mut rect: Option<egui::Rect> = None;
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        // Anything we can't project makes the bound unreliable; take the test as passed.
        let Some(p) = project(corner) else { return true };
        rect = Some(match rect {
            Some(r) => r.union(egui::Rect::from_pos(p)),
            None => egui::Rect::from_pos(p),
        });
    }
    rect.is_none_or(|r| r.expand(margin).contains(screen))
}

/// Nearest feature edge of any 3D body's solid mesh (#31) — lets a construction plane be
/// referenced from any edge on any shape, not just 2D sketch geometry.
/// The cylinder centre line nearest the cursor (#1013), with its world segment and screen
/// distance — the axis twin of [`nearest_body_edge`].
fn nearest_body_axis(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
) -> Option<(PickTargetKind, Vec3, Vec3, f32)> {
    let mut best: Option<(PickTargetKind, Vec3, Vec3, f32)> = None;
    let bounds = crate::extrude::body_world_bounds_all(doc);
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        // A body nowhere near the cursor can't own the nearest axis (#1026).
        if !bounds.get(&bi).copied().flatten().is_some_and(|b| {
            screen_bounds_hit(screen, project, b, LINE_PICK_RADIUS_PX)
        }) {
            continue;
        }
        for cyl in crate::extrude::body_cylinders(doc, bi).iter() {
            let a = cyl.origin - cyl.dir * cyl.half_length;
            let b = cyl.origin + cyl.dir * cyl.half_length;
            let Some(dist) = segment_pick_distance(screen, project, a, b) else {
                continue;
            };
            if best.as_ref().is_none_or(|(_, _, _, d)| dist < *d) {
                best = Some((PickTargetKind::BodyAxis { body: bi, a, b }, a, b, dist));
            }
        }
    }
    best
}

fn nearest_body_edge(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    occlusion: Option<&PickOcclusion>,
) -> Option<(PickTargetKind, Vec3, Vec3, String, f32)> {
    // Track visibility alongside the screen distance: when two edges overlap in screen space (a
    // front and a back edge at the same cursor spot — e.g. the near and far top edges of a box seen
    // near-on), a **visible** edge must win, even if a hidden one is marginally closer (#581).
    // Otherwise the occluded back edge would be chosen and then rejected by the caller's visibility
    // gate, silently dropping the whole edge pick so a body **face** wins and the Plane tool loses
    // its angle/offset (hinge) reference.
    let mut best: Option<(PickTargetKind, Vec3, Vec3, String, f32, bool)> = None;

    let mut consider = |kind: PickTargetKind, a: Vec3, b: Vec3| {
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            return;
        };
        let anchor = segment_point_nearest_screen(screen, project, a, b);
        let visible = occlusion.is_none_or(|occ| !occ.occluded(anchor));
        // A visible edge beats an occluded one; within the same visibility, nearer screen wins.
        let better = best.as_ref().is_none_or(|(_, _, _, _, d, vis)| {
            (visible, -dist) > (*vis, -*d)
        });
        if better {
            best = Some((kind, a, b, "Body edge".to_string(), dist, visible));
        }
    };

    let bounds = crate::extrude::body_world_bounds_all(doc);
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        // Hidden or ignored bodies are not there: skip them inside the search so a
        // body behind them can still win (#1336), instead of taking their edge and
        // then dropping the whole pick.
        if occlusion.is_some_and(|occ| !occ.pickable(doc, &PickTargetKind::Body(bi))) {
            continue;
        }
        // No edge of a body can be nearer than the body is (#1026).
        if !bounds.get(&bi).copied().flatten().is_some_and(|b| {
            screen_bounds_hit(screen, project, b, LINE_PICK_RADIUS_PX)
        }) {
            continue;
        }
        // Segments of one smooth chain all carry the chain's canonical segment as their
        // pick identity (#626), so clicking any facet of a curved rim selects the whole
        // curve; the hovered segment itself still provides the axis anchor geometry.
        // Walk by reference (#1141): a circular rim is ~50–100 segments, and cloning each
        // chain every hover frame stacked up on bodies with several holes.
        for chain in crate::extrude::body_edge_chains(doc, bi).iter() {
            let (ca, cb) = crate::gpu_viewport::chain_canonical_segment(chain);
            for &(a, b) in chain {
                consider(PickTargetKind::BodyEdge { body: bi, a: ca, b: cb }, a, b);
            }
        }
    }

    best.map(|(kind, a, b, label, dist, _)| (kind, a, b, label, dist))
}

/// Nearest solid-mesh vertex (#144) of any 3D body within the point pick radius, for 3D
/// hover/selection — so any **feature corner** of any body can be picked. Tessellation
/// vertices on smooth surfaces (a sphere's mesh, a cylinder wall) are not features and are
/// not offered (#1101/#1118).
pub fn nearest_body_vertex(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
) -> Option<(PickTargetKind, f32)> {
    nearest_body_vertex_where(screen, project, doc, |_, _| true)
}

/// [`nearest_body_vertex`] restricted to the corners `accept` allows (#908).
///
/// The filter belongs *inside* the search, not after it: a box seen head-on projects its
/// near and far corners onto the same pixel, so filtering the single winner afterwards
/// throws the pick away whenever the hidden corner happened to be found first — the corner
/// then reads as unpickable and the click lands on an edge instead.
pub fn nearest_body_vertex_where(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    accept: impl Fn(&PickTargetKind, Vec3) -> bool,
) -> Option<(PickTargetKind, f32)> {
    let mut best: Option<(PickTargetKind, f32)> = None;
    let bounds = crate::extrude::body_world_bounds_all(doc);
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        // Every corner of a body far from the cursor is far from the cursor (#1026).
        if !bounds.get(&bi).copied().flatten().is_some_and(|b| {
            screen_bounds_hit(screen, project, b, POINT_PICK_RADIUS_PX)
        }) {
            continue;
        }
        // A pure sphere primitive has no feature corners at all (#1101).
        if crate::primitives::body_is_sphere(doc, bi) {
            continue;
        }
        let Some(solid) = crate::extrude::body_solid_mesh(doc, bi) else {
            continue;
        };
        // Only vertices that sit on a **feature edge** (crease or boundary) are corners
        // (#1118): a cut sphere's tessellation fans out hundreds of surface points that
        // are not real features — only the cut rim's corners are.
        let feature_verts = mesh_feature_vertex_keys(&solid);
        if feature_verts.is_empty() {
            continue;
        }
        for tri in &solid.triangles {
            for &p in tri {
                if !feature_verts.contains(&crate::gpu_viewport::quantize_vertex(p)) {
                    continue;
                }
                let Some(sp) = project(p) else {
                    continue;
                };
                let dist = (screen - sp).length();
                if dist > crate::touch::hit(POINT_PICK_RADIUS_PX)
                    || best.as_ref().is_some_and(|(_, d)| dist >= *d)
                {
                    continue;
                }
                let kind = PickTargetKind::BodyVertex { body: bi, position: p };
                if accept(&kind, p) {
                    best = Some((kind, dist));
                }
            }
        }
    }
    best
}

/// Quantized keys of every **sharp corner** on a mesh's feature edges (#1118/#1120).
///
/// A vertex on a feature edge is only a corner when it is not a smooth link in a curve
/// chain: free ends, T-junctions, 3+ edges meeting, or a degree-2 vertex where the chain
/// turns more than ~30°. Points along a tessellated circle or cut rim (degree 2, nearly
/// collinear) are not corners — the rim is selected as an edge, not a fan of vertices.
fn mesh_feature_vertex_keys(
    solid: &crate::extrude::SolidMesh,
) -> std::collections::HashSet<(i64, i64, i64)> {
    use crate::gpu_viewport::quantize_vertex;
    // Match [`crate::gpu_viewport::CURVE_CHAIN_COS_THRESHOLD`]: cos(30°). Two outward
    // directions with dot ≤ −that are nearly opposite → smooth continuation through the
    // vertex; a larger (less negative) dot is a real corner.
    const SMOOTH_DOT: f32 = -0.866_025; // -cos(30°)

    let mut adj: std::collections::HashMap<(i64, i64, i64), Vec<Vec3>> =
        std::collections::HashMap::new();
    for (a, b) in crate::gpu_viewport::solid_mesh_unique_edges(solid) {
        let (qa, qb) = (quantize_vertex(a), quantize_vertex(b));
        if qa == qb {
            continue;
        }
        adj.entry(qa).or_default().push((b - a).normalize_or_zero());
        adj.entry(qb).or_default().push((a - b).normalize_or_zero());
    }
    let mut keys = std::collections::HashSet::new();
    for (v, dirs) in adj {
        match dirs.as_slice() {
            [] => {}
            [_] => {
                // Free end of a feature edge — a corner.
                keys.insert(v);
            }
            [d0, d1] => {
                // Degree 2: keep only if the chain turns sharply (not a smooth rim facet).
                if d0.dot(*d1) > SMOOTH_DOT {
                    keys.insert(v);
                }
            }
            _ => {
                // 3+ feature edges meet — a real corner (e.g. where a cut meets a flat face).
                keys.insert(v);
            }
        }
    }
    keys
}

/// One selectable thing found in the crowd under the cursor, for the Selection Exploder (#551).
pub struct CrowdCandidate {
    /// The pick target this handle stands for — its screen anchor is where the exploder
    /// redirects the tool's pick, and its kind drives the handle icon.
    pub kind: PickTargetKind,
    /// A world point the exploder's connecting line attaches to (and where the tool re-picks) —
    /// the vertex itself, the point on an edge/line/circle nearest the cursor, or a face point.
    pub anchor: Vec3,
    /// Pixel distance from the cursor to the candidate.
    pub dist_px: f32,
}

/// A stable dedup key per crowd candidate (one handle per distinct thing). A body face (#555)
/// maps to a `SceneElement::BodyFace` keyed by its quantized centroid+normal, so two distinct
/// faces of the same body get two distinct keys (and two loupes) rather than collapsing to one.
///
/// Keying off the scene element is what makes "one handle per distinct thing" true: a
/// construction plane reached as a bare `ConstructionPlane` and the same plane reached as a
/// `SketchFace(FaceId::ConstructionPlane(_))` (#860) normalize to one element (#952), so the
/// plane fans out as a single loupe instead of two for the same thing.
fn crowd_key(kind: &PickTargetKind) -> String {
    match scene_element_from_pick(kind) {
        Some(el) => format!("{el:?}"),
        None => format!("{kind:?}"),
    }
}

/// Every selectable thing whose pick hitbox the cursor is within (#551) — the "crowd" the
/// Selection Exploder fans out. Unlike [`resolve_pick_target`] (which keeps only the nearest),
/// this returns all of them, deduped per thing, ordered nearest-first. Covers everything a tool
/// might pick at the cursor: sketch points/lines/circles, body vertices/edges, and the body face
/// under the cursor — so the exploder can redirect any tool's pick to the chosen one.
pub fn collect_pick_candidates(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    // Breaks ties in the crowd's order (#987): every face the cursor is inside sits at screen
    // distance 0, and the nearest of those to the eye is the one you can actually see. It only
    // *orders* — the crowd still enumerates every near face, buried ones included (#556).
    eye: Vec3,
    occlusion: Option<&PickOcclusion>,
) -> Vec<CrowdCandidate> {
    // The exploder exists to reach *buried* geometry, so — unlike `resolve_pick_target` — no
    // occlusion gate here (#556): things behind a body still appear in the crowd. The `pickable`
    // gate stays, so user-hidden and shadow geometry is still excluded.
    let pickable = |kind: &PickTargetKind| occlusion.is_none_or(|occ| occ.pickable(doc, kind));
    let point_r = crate::touch::hit(POINT_PICK_RADIUS_PX);
    let mut raw: Vec<(PickTargetKind, Vec3, f32)> = Vec::new();

    let push_point = |raw: &mut Vec<(PickTargetKind, Vec3, f32)>, cp: ConstraintPoint, world: Vec3| {
        let Some(sp) = project(world) else { return };
        let dist = (screen - sp).length();
        let kind = PickTargetKind::Point(cp);
        if dist <= point_r && pickable(&kind) {
            raw.push((kind, world, dist));
        }
    };

    // Sketch points: line endpoints, circle centres, text anchors, image calibration points.
    for (li, line) in doc.lines.iter() {
        let Some((a, b)) = line_world_endpoints(doc, line) else {
            continue;
        };
        push_point(&mut raw, ConstraintPoint::LineEndpoint { line: li, end: LineEnd::Start }, a);
        push_point(&mut raw, ConstraintPoint::LineEndpoint { line: li, end: LineEnd::End }, b);
    }
    for (ci, circle) in doc.circles.iter() {
        if let Some(center) = crate::face::circle_world_center(doc, circle) {
            push_point(&mut raw, ConstraintPoint::CircleCenter(ci), center);
        }
    }
    for (ti, text) in doc.sketch_texts.iter() {
        if let Some(frame) = crate::face::sketch_geometry_frame(doc, text.sketch) {
            for anchor in crate::model::TextAnchor::ALL {
                let (u, v) = crate::text::sketch_text_anchor_uv(text, anchor);
                push_point(
                    &mut raw,
                    ConstraintPoint::TextAnchor { text: ti, anchor },
                    crate::face::local_to_world(&frame, u, v),
                );
            }
        }
    }
    for (ii, img) in doc.tracing_images.iter() {
        if !image_hosts_a_sketch(doc, img) {
            continue;
        }
        if let Some(frame) = crate::face::sketch_frame(doc, FaceId::ConstructionPlane(img.plane)) {
            for index in 0..2 {
                if let Some((u, v)) = crate::model::image_calibration_point_uv(img, index) {
                    push_point(
                        &mut raw,
                        ConstraintPoint::ImageCalibrationPoint { image: ii, index },
                        frame.origin + frame.u_axis * u + frame.v_axis * v,
                    );
                }
            }
            for anchor in crate::model::TextAnchor::ALL {
                let (u, v) = crate::model::image_anchor_uv(img, anchor);
                push_point(
                    &mut raw,
                    ConstraintPoint::ImageAnchor { image: ii, anchor },
                    frame.origin + frame.u_axis * u + frame.v_axis * v,
                );
            }
        }
    }

    // Edges: sketch lines/circles and body mesh feature edges.
    let push_edge = |raw: &mut Vec<(PickTargetKind, Vec3, f32)>, kind: PickTargetKind, a: Vec3, b: Vec3| {
        let Some(dist) = segment_pick_distance(screen, project, a, b) else { return };
        let anchor = segment_point_nearest_screen(screen, project, a, b);
        if pickable(&kind) {
            raw.push((kind, anchor, dist));
        }
    };
    for (li, line) in doc.lines.iter() {
        if let Some(points) = line_world_polyline(doc, line) {
            for pair in points.windows(2) {
                push_edge(&mut raw, PickTargetKind::Line(li), pair[0], pair[1]);
            }
        }
    }
    for (ci, circle) in doc.circles.iter() {
        if let Some(pts) = crate::face::circle_world_perimeter(doc, circle, 32) {
            for w in pts.windows(2) {
                push_edge(&mut raw, PickTargetKind::Circle(ci), w[0], w[1]);
            }
        }
    }
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        let Some(solid) = crate::extrude::body_solid_mesh(doc, bi) else {
            continue;
        };
        // Chain-canonical identity (#626): the exploder's crowd then dedupes every facet of
        // one curve into a single candidate (the whole curve), keyed like selection.
        for chain in crate::gpu_viewport::solid_mesh_edge_chains(&solid) {
            let (ca, cb) = crate::gpu_viewport::chain_canonical_segment(&chain);
            for (a, b) in chain {
                push_edge(&mut raw, PickTargetKind::BodyEdge { body: bi, a: ca, b: cb }, a, b);
            }
        }
        // Feature corners only (#1101/#1118): a sphere's (or cut-sphere's) tessellation
        // vertices are not real features — only crease/boundary endpoints are.
        if crate::primitives::body_is_sphere(doc, bi) {
            continue;
        }
        let feature_verts = mesh_feature_vertex_keys(&solid);
        for tri in &solid.triangles {
            for &p in tri {
                if !feature_verts.contains(&crate::gpu_viewport::quantize_vertex(p)) {
                    continue;
                }
                let Some(sp) = project(p) else { continue };
                let dist = (screen - sp).length();
                if dist <= point_r {
                    let kind = PickTargetKind::BodyVertex { body: bi, position: p };
                    if pickable(&kind) {
                        raw.push((kind, p, dist));
                    }
                }
            }
        }
    }

    // Sketch origin axes (#189 / #1538): local LX/LY through each sketch origin. Same
    // infinite-line screen measure as `nearest_sketch_line_in_sketch` — a finite segment
    // usually fails to project. Identity is just X or Y; crowd_key collapses copies.
    for (sketch, _) in doc.sketches.iter() {
        let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) else {
            continue;
        };
        for (axis, dir) in [
            (crate::model::SketchAxis::X, frame.u_axis),
            (crate::model::SketchAxis::Y, frame.v_axis),
        ] {
            let (Some(p0), Some(p1)) = (
                project(frame.origin),
                project(frame.origin + dir * 10.0),
            ) else {
                continue;
            };
            let d = p1 - p0;
            if d.length_sq() < 1e-6 {
                continue;
            }
            let dn = d / d.length();
            let dist = ((screen - p0).x * dn.y - (screen - p0).y * dn.x).abs();
            let kind = PickTargetKind::OriginAxis(axis);
            if dist <= crate::touch::hit(LINE_PICK_RADIUS_PX) && pickable(&kind) {
                let t_mm = (screen - p0).dot(dn) * (10.0 / d.length());
                let anchor = frame.origin + dir * t_mm;
                raw.push((kind, anchor, dist));
            }
        }
    }

    // The world axes (#975). They are pickable — a Revolve axis, a Repeat path, a plane anchor
    // all take one — so they belong in the crowd like everything else pickable: an axis running
    // under a body or through a busy corner is exactly what the fan is for. All three, not the
    // nearest, since the crowd is the whole stack.
    for axis in [GlobalAxis::X, GlobalAxis::Y, GlobalAxis::Z] {
        let (a, b) = global_axis_segment(axis);
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            continue;
        };
        let kind = PickTargetKind::GlobalAxis(axis);
        if pickable(&kind) {
            // Anchored at the point on the axis nearest the cursor, so the loupe's leader line
            // points at the bit of it under the pointer rather than at the world origin.
            let anchor = match (project(a), project(b)) {
                (Some(pa), Some(pb)) if (pb - pa).length_sq() > 1e-4 => {
                    let t = ((screen - pa).dot(pb - pa) / (pb - pa).length_sq()).clamp(0.0, 1.0);
                    a.lerp(b, t)
                }
                _ => a,
            };
            raw.push((kind, anchor, dist));
        }
    }

    // Every construction plane near the cursor (#975). One reaches the crowd as an analytic
    // face too, and `crowd_key` collapses the pair — but only when `sketch_faces_near` offers
    // that plane, which it does not for one seen edge-on or one the pointer is merely near.
    // Anchor at the point under the cursor (#1277) so crowd depth order matches the ordinary
    // pick: a front plane sorts ahead of a big buried one that covers the same screen spot.
    for (index, plane) in doc.construction_planes.iter() {
        let corners = plane_corners(plane);
        let Some(pts) = corners.iter().map(|&c| project(c)).collect::<Option<Vec<_>>>() else {
            continue;
        };
        let quad = [pts[0], pts[1], pts[2], pts[3]];
        let dist = if point_in_screen_quad(screen, quad) {
            0.0
        } else {
            dist_point_to_quad_edges(screen, quad)
        };
        let kind = PickTargetKind::ConstructionPlane(index);
        if dist <= FACE_PICK_MARGIN_PX && pickable(&kind) {
            let at = plane_point_under_cursor(screen, &pts, &corners).unwrap_or(plane.origin);
            raw.push((kind, at, dist));
        }
    }

    // Tracing images (#1561): their displayed quad is pickable like a plane, so the
    // Select tool and the exploder can take one without going through the Elements pane.
    for (index, _) in doc.tracing_images.iter() {
        let Some(corners) = tracing_image_corners(doc, index) else {
            continue;
        };
        let Some(pts) = corners.iter().map(|&c| project(c)).collect::<Option<Vec<_>>>() else {
            continue;
        };
        let quad = [pts[0], pts[1], pts[2], pts[3]];
        let dist = if point_in_screen_quad(screen, quad) {
            0.0
        } else {
            dist_point_to_quad_edges(screen, quad)
        };
        let kind = PickTargetKind::TracingImage(index);
        if dist <= FACE_PICK_MARGIN_PX && pickable(&kind) {
            let at = plane_point_under_cursor(screen, &pts, &corners).unwrap_or(corners[0]);
            raw.push((kind, at, dist));
        }
    }

    // Every body face near the cursor (#555/#556): not just the nearest ray-hit face, but every
    // face — front and back — whose projected area is within the pick radius, so a narrow face
    // seen edge-on (a thin sliver between its two edges) and buried back faces both get loupes.
    for (kind, centroid, dist) in crate::face::body_faces_near(screen, project, doc, eye, point_r) {
        if pickable(&kind) {
            raw.push((kind, centroid, dist));
        }
    }

    // Analytic sketchable faces (#625): the faces the face-picking tools (Extrude, Sketch, …)
    // actually operate on — sketch profiles, extrusion caps/side walls, revolve flat faces —
    // offered alongside the mesh facet groups above so `exploder_tool_accepts` can hand each
    // tool the kind its own pick path accepts.
    for (face, centroid, dist) in crate::face::sketch_faces_near(screen, project, doc, point_r) {
        let kind = PickTargetKind::SketchFace(face);
        if pickable(&kind) {
            raw.push((kind, centroid, dist));
        }
    }

    // The whole bodies (#902): one candidate per body already represented in the crowd by a
    // face, edge, or corner, anchored at its nearest of those — so the exploder always offers
    // "the body" next to "this face of it".
    let mut bodies: std::collections::BTreeMap<crate::model::BodyKey, (Vec3, f32)> =
        std::collections::BTreeMap::new();
    for (kind, anchor, dist) in &raw {
        let bi = match kind {
            PickTargetKind::BodyEdge { body, .. }
            | PickTargetKind::BodyFace { body, .. }
            | PickTargetKind::BodyVertex { body, .. } => *body,
            _ => continue,
        };
        bodies
            .entry(bi)
            .and_modify(|e| {
                if *dist < e.1 {
                    *e = (*anchor, *dist);
                }
            })
            .or_insert((*anchor, *dist));
    }
    for (bi, (anchor, dist)) in bodies {
        raw.push((PickTargetKind::Body(bi), anchor, dist));
    }

    // Dedupe per distinct thing (keeping the nearest touch), then order nearest-first.
    //
    // A **`BTreeMap`**, not a `HashMap` (#987): `HashMap`'s iteration order is randomly seeded
    // per instance, so the same crowd came out in a different order on every call — and since
    // the sort below is stable, candidates tied on screen distance kept that random order. The
    // normal pick takes the first of them, so hovering a spot inside two faces thrashed
    // between them frame after frame.
    let mut best: std::collections::BTreeMap<String, (PickTargetKind, Vec3, f32)> =
        std::collections::BTreeMap::new();
    for (kind, anchor, dist) in raw {
        let key = crowd_key(&kind);
        best.entry(key)
            .and_modify(|e| {
                if dist < e.2 {
                    *e = (kind.clone(), anchor, dist);
                }
            })
            .or_insert((kind, anchor, dist));
    }
    let mut out: Vec<CrowdCandidate> = best
        .into_values()
        .map(|(kind, anchor, dist_px)| CrowdCandidate { kind, anchor, dist_px })
        .collect();
    // Nearest the cursor first; ties broken by **depth**, so the face you can see beats the one
    // hidden behind it and the ordinary pick takes the near one (#987). The cursor sits *inside*
    // every face it is over, all of them at distance 0, so screen distance alone cannot separate
    // them. This orders the crowd without pruning it: the exploder still fans every buried face,
    // which is the only way to reach one (#556). The key is the final tiebreak, so the order is
    // total and no two runs can disagree.
    out.sort_by(|a, b| {
        let depth = |c: &CrowdCandidate| (c.anchor - eye).length();
        a.dist_px
            .total_cmp(&b.dist_px)
            .then_with(|| depth(a).total_cmp(&depth(b)))
            .then_with(|| crowd_key(&a.kind).cmp(&crowd_key(&b.kind)))
    });
    out
}

/// Boundary edges of a coplanar face group (#144): the edges of the group's triangles that belong
/// to exactly one triangle. Interior edges (shared by two triangles, e.g. a quad's diagonal) are
/// dropped, leaving the outline of the whole face for the hover highlight.
pub fn coplanar_face_boundary(triangles: &[[Vec3; 3]]) -> Vec<(Vec3, Vec3)> {
    type Key = ((i64, i64, i64), (i64, i64, i64));
    let quant = |v: Vec3| {
        (
            (v.x * 1000.0).round() as i64,
            (v.y * 1000.0).round() as i64,
            (v.z * 1000.0).round() as i64,
        )
    };
    let mut counts: std::collections::HashMap<Key, (Vec3, Vec3, u32)> =
        std::collections::HashMap::new();
    for tri in triangles {
        for &(i, j) in &[(0usize, 1usize), (1, 2), (2, 0)] {
            let (a, b) = (tri[i], tri[j]);
            let (ka, kb) = (quant(a), quant(b));
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            counts.entry(key).or_insert((a, b, 0)).2 += 1;
        }
    }
    counts
        .into_values()
        .filter(|(_, _, n)| *n == 1)
        .map(|(a, b, _)| (a, b))
        .collect()
}

/// Ordered outline loop of a coplanar face group (#1219/#1220): the same edges as
/// [`coplanar_face_boundary`], chained into a closed polyline so a highlight border (and a
/// fan-fill) follows the real outline rather than the mesh's triangle-visit order — which
/// drew diagonals and crossing lines across cut/boolean faces.
///
/// When the face has holes or several disconnected outline components, returns the longest
/// component (the outer boundary for typical CAD faces). Empty when the group is degenerate.
pub fn coplanar_face_boundary_loop(triangles: &[[Vec3; 3]]) -> Vec<Vec3> {
    let boundary = coplanar_face_boundary(triangles);
    if boundary.is_empty() {
        return Vec::new();
    }
    // Adjacency: each quantized endpoint → neighbours (world points).
    type Q = (i64, i64, i64);
    let quant = |v: Vec3| -> Q {
        (
            (v.x * 1000.0).round() as i64,
            (v.y * 1000.0).round() as i64,
            (v.z * 1000.0).round() as i64,
        )
    };
    let mut adj: std::collections::HashMap<Q, Vec<Vec3>> = std::collections::HashMap::new();
    let mut world: std::collections::HashMap<Q, Vec3> = std::collections::HashMap::new();
    for &(a, b) in &boundary {
        let (ka, kb) = (quant(a), quant(b));
        world.entry(ka).or_insert(a);
        world.entry(kb).or_insert(b);
        adj.entry(ka).or_default().push(b);
        adj.entry(kb).or_default().push(a);
    }
    // The walk below must not depend on hash order (#1639): `ConstraintLine::FaceEdge`
    // stores an *index* into this loop, so starting at a different corner (or heading the
    // other way round) each call re-points every face-edge constraint at some other edge —
    // a snapped point jumps to a different edge on the next solve. Fixed corner order in,
    // same loop out.
    let mut starts: Vec<Q> = world.keys().copied().collect();
    starts.sort();
    for neighbours in adj.values_mut() {
        neighbours.sort_by_key(|&n| quant(n));
    }
    // Walk every unused edge; keep the longest **closed** walk (outer loop). An open
    // chain must not be treated as a loop — closing it invents a diagonal.
    let mut used: std::collections::HashSet<(Q, Q)> = std::collections::HashSet::new();
    let edge_key = |a: Q, b: Q| if a <= b { (a, b) } else { (b, a) };
    let mut best: Vec<Vec3> = Vec::new();
    for start_q in starts {
        let Some(neighbours) = adj.get(&start_q) else {
            continue;
        };
        for &first in neighbours {
            let first_q = quant(first);
            let start_edge = edge_key(start_q, first_q);
            if used.contains(&start_edge) {
                continue;
            }
            let mut walk_used: Vec<(Q, Q)> = Vec::new();
            let mut loop_pts = vec![world[&start_q], first];
            walk_used.push(start_edge);
            let mut prev = start_q;
            let mut cur = first_q;
            let mut closed = false;
            for _ in 0..boundary.len() + 2 {
                if cur == start_q {
                    closed = true;
                    break;
                }
                let Some(ns) = adj.get(&cur) else {
                    break;
                };
                let next = ns.iter().find_map(|&n| {
                    let nq = quant(n);
                    let ek = edge_key(cur, nq);
                    if nq != prev && !used.contains(&ek) && !walk_used.contains(&ek) {
                        Some((n, nq, ek))
                    } else {
                        None
                    }
                });
                let Some((n, nq, ek)) = next else {
                    break;
                };
                walk_used.push(ek);
                loop_pts.push(n);
                prev = cur;
                cur = nq;
            }
            if !closed {
                continue;
            }
            // Drop the duplicate close vertex.
            if loop_pts.len() >= 2 && quant(loop_pts[0]) == quant(*loop_pts.last().unwrap()) {
                loop_pts.pop();
            }
            if loop_pts.len() < 3 {
                continue;
            }
            for ek in walk_used {
                used.insert(ek);
            }
            if loop_pts.len() > best.len() {
                best = loop_pts;
            }
        }
    }
    best
}

/// Nearest currently-treatable analytic extrusion edge (#77): the chamfer/fillet tool's own
/// picking path when no sketch is open, used instead of the generic [`nearest_body_edge`]
/// (mesh-feature-edge) picking above since it needs the structured `ExtrusionEdgeRef`, not just
/// two raw points — see `crate::extrude::treatable_edges`.
pub fn nearest_treatable_edge(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    occlusion: Option<&PickOcclusion>,
) -> Option<(crate::model::TreatableSolid, crate::model::ExtrusionEdgeRef, Vec3, Vec3, f32)> {
    // Same visible-beats-occluded ranking as [`nearest_body_edge`] (#581/#1462): a
    // front edge must win when a back edge stacks under the cursor, instead of
    // the hidden one being chosen and then dropped.
    let mut best: Option<(
        crate::model::TreatableSolid,
        crate::model::ExtrusionEdgeRef,
        Vec3,
        Vec3,
        f32,
        bool,
    )> = None;
    for (solid, edge, a, b) in crate::extrude::treatable_edges(doc) {
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            continue;
        };
        let anchor = segment_point_nearest_screen(screen, project, a, b);
        let visible = occlusion.is_none_or(|occ| !occ.occluded(anchor));
        let better = best.as_ref().is_none_or(|(_, _, _, _, d, vis)| {
            (visible, -dist) > (*vis, -*d)
        });
        if better {
            best = Some((solid, edge, a, b, dist, visible));
        }
    }
    best.and_then(|(solid, edge, a, b, dist, visible)| {
        (visible || occlusion.is_none()).then_some((solid, edge, a, b, dist))
    })
}

/// Analytic chamfer/fillet edges a click at `screen` would take (#1462/#1463).
/// Goes through the shared pick (occlusion, posed bodies) first, then the
/// treatable-edge search, then a face's boundary.
pub fn pick_treatable_edges(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    eye: Vec3,
    occlusion: Option<&PickOcclusion>,
) -> Vec<(crate::model::TreatableSolid, crate::model::ExtrusionEdgeRef)> {
    let q = crate::hierarchy::quantize_body_point;
    if let Some(target) = resolve_pick_target(screen, project, None, doc, occlusion) {
        match &target.kind {
            PickTargetKind::BodyEdge { body, a, b } => {
                if let Some(resolved) =
                    crate::extrude::treatable_edge_for_selection(doc, *body, q(*a), q(*b))
                {
                    return vec![resolved];
                }
            }
            PickTargetKind::BodyFace { body, triangles, .. } => {
                let edges: Vec<_> = coplanar_face_boundary(triangles)
                    .into_iter()
                    .filter_map(|(a, b)| {
                        crate::extrude::treatable_edge_for_selection(doc, *body, q(a), q(b))
                    })
                    .collect();
                if !edges.is_empty() {
                    return edges;
                }
            }
            _ => {}
        }
    }
    if let Some((solid, edge, _, _, _)) = nearest_treatable_edge(screen, project, doc, occlusion) {
        return vec![(solid, edge)];
    }
    crate::face::pick_body_face(screen, project, doc, eye)
        .and_then(|kind| match kind {
            PickTargetKind::BodyFace { body, triangles, .. } => Some(
                coplanar_face_boundary(&triangles)
                    .into_iter()
                    .filter_map(|(a, b)| {
                        crate::extrude::treatable_edge_for_selection(doc, body, q(a), q(b))
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

fn draw_circle_highlight(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    circle: &crate::model::Circle,
    color: egui::Color32,
) {
    let Some(pts) = crate::face::circle_world_perimeter(doc, circle, 48) else {
        return;
    };
    let screen_pts: Option<Vec<egui::Pos2>> = pts.iter().map(|p| project(*p)).collect();
    if let Some(screen_pts) = screen_pts {
        if screen_pts.len() >= 2 {
            painter.add(egui::Shape::closed_line(
                screen_pts,
                egui::Stroke::new(3.0, color),
            ));
        }
    }
}

fn nearest_global_axis(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
) -> Option<(GlobalAxis, f32)> {
    let mut best: Option<(GlobalAxis, f32)> = None;
    for axis in [GlobalAxis::X, GlobalAxis::Y, GlobalAxis::Z] {
        let (a, b) = global_axis_segment(axis);
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            continue;
        };
        if best.map(|(_, d)| dist < d).unwrap_or(true) {
            best = Some((axis, dist));
        }
    }
    best
}

/// Screen-distance band within which two construction-plane picks count as the same depth
/// under the cursor, so the nearer (camera-facing) one wins (#1277). Mirrors the body-face
/// tie band in `face::consider_face_pick_sized`.
const PLANE_PICK_DEPTH_TIE_PX: f32 = 0.5;

/// World point on a plane's display quad under `screen`, via the same screen-space
/// barycentric blend body faces use — the point depth comparisons need, not the plane origin.
fn plane_point_under_cursor(
    screen: egui::Pos2,
    projected: &[egui::Pos2],
    corners: &[Vec3; 4],
) -> Option<Vec3> {
    if projected.len() != 4 {
        return None;
    }
    // Two triangles covering the display quad (same split as `point_in_screen_quad`).
    for tri in [[0usize, 1, 2], [0, 2, 3]] {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        let (pa, pb, pc) = (projected[a], projected[b], projected[c]);
        let area = (pb.x - pa.x) * (pc.y - pa.y) - (pc.x - pa.x) * (pb.y - pa.y);
        if area.abs() < 1e-6 {
            continue;
        }
        let w0 = ((pb.x - screen.x) * (pc.y - screen.y) - (pc.x - screen.x) * (pb.y - screen.y))
            / area;
        let w1 = ((pc.x - screen.x) * (pa.y - screen.y) - (pa.x - screen.x) * (pc.y - screen.y))
            / area;
        let w2 = 1.0 - w0 - w1;
        if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
            return Some(corners[a] * w0 + corners[b] * w1 + corners[c] * w2);
        }
    }
    None
}

/// Screen-space hit on a world quad: pixel distance (0 inside) and the world point
/// under the cursor. `None` when the quad is off-screen or farther than the face
/// pick margin.
fn screen_quad_hit(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    corners: &[Vec3; 4],
) -> Option<(f32, Vec3)> {
    let pts: Option<Vec<egui::Pos2>> = corners.iter().map(|&c| project(c)).collect();
    let pts = pts?;
    let quad = [pts[0], pts[1], pts[2], pts[3]];
    let dist = if point_in_screen_quad(screen, quad) {
        0.0
    } else {
        dist_point_to_quad_edges(screen, quad)
    };
    if dist > FACE_PICK_MARGIN_PX {
        return None;
    }
    let at = plane_point_under_cursor(screen, &pts, corners).unwrap_or(corners[0]);
    Some((dist, at))
}

/// The frontmost construction plane or tracing image under the cursor (#1561/#1562).
/// Same screen-distance / eye-depth ranking planes used to have on their own; a
/// coplanar image beats its host plane because it's the thing you're pointing at.
fn nearest_plane_or_image(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
    eye: Option<Vec3>,
) -> Option<(PickTargetKind, f32, Vec3)> {
    let mut best: Option<(PickTargetKind, f32, f32, Vec3)> = None;
    let consider = |best: &mut Option<(PickTargetKind, f32, f32, Vec3)>,
                    kind: PickTargetKind,
                    dist: f32,
                    at: Vec3| {
        let depth = eye.map(|e| (at - e).length()).unwrap_or(f32::MAX);
        let better = match best.as_ref() {
            None => true,
            Some((bk, d, dep, _)) => {
                if dist < d - PLANE_PICK_DEPTH_TIE_PX {
                    true
                } else if dist > d + PLANE_PICK_DEPTH_TIE_PX {
                    false
                } else if (depth - *dep).abs() > 1e-3 {
                    depth < *dep
                } else {
                    // Coplanar (or no-eye) tie: the image is the thing you're pointing at.
                    matches!(kind, PickTargetKind::TracingImage(_))
                        && matches!(bk, PickTargetKind::ConstructionPlane(_))
                }
            }
        };
        if better {
            *best = Some((kind, dist, depth, at));
        }
    };

    for (index, plane) in doc.construction_planes.iter() {
        let corners = plane_corners(plane);
        if let Some((dist, at)) = screen_quad_hit(screen, project, &corners) {
            consider(
                &mut best,
                PickTargetKind::ConstructionPlane(index),
                dist,
                at,
            );
        }
    }
    for (index, _) in doc.tracing_images.iter() {
        let Some(corners) = tracing_image_corners(doc, index) else {
            continue;
        };
        if let Some((dist, at)) = screen_quad_hit(screen, project, &corners) {
            consider(&mut best, PickTargetKind::TracingImage(index), dist, at);
        }
    }
    best.map(|(kind, dist, _, at)| (kind, dist, at))
}

fn dist_point_to_quad_edges(p: egui::Pos2, quad: [egui::Pos2; 4]) -> f32 {
    let edges = [(0, 1), (1, 2), (2, 3), (3, 0)];
    edges
        .iter()
        .map(|&(i, j)| dist_point_to_segment_px(p, quad[i], quad[j]))
        .fold(f32::MAX, f32::min)
}

/// Drop a closed loop of plain `Line`s from local (u, v) points, joined at their shared
/// corners by `Coincident` constraints — the general (not-necessarily-axis-aligned) form of
/// [`add_line_rectangle`], e.g. for mirroring an arbitrary body face's exact boundary into a
/// new implicit sketch (#122). No Horizontal/Vertical constraints (those only make sense for
/// an axis-aligned rectangle); `points.len()` must be at least 3.
///
/// Returns the line indices in the same order as `points`.
pub fn add_line_polygon(
    doc: &mut Document,
    sketch: SketchId,
    points: &[(f32, f32)],
) -> Vec<crate::model::LineKey> {
    use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ShapeKind};
    let n = points.len();
    let mut idx = Vec::with_capacity(n);
    for i in 0..n {
        let (u0, v0) = points[i];
        let (u1, v1) = points[(i + 1) % n];
        idx.push(doc.lines.insert(Line::from_local_endpoints(sketch, u0, v0, u1, v1)));
        doc.shape_order.push(ShapeKind::Line);
    }
    for i in 0..n {
        doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: idx[i],
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: idx[(i + 1) % n],
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        doc.shape_order.push(ShapeKind::Constraint);
    }
    idx
}

/// Drop a rectangle as four plain `Line`s forming a closed loop (bottom → right → top →
/// left), joined at their shared corners by `Coincident` constraints, with `Horizontal`
/// constraints on the two horizontal edges and `Vertical` on the two vertical edges — so
/// the loop stays a rectangle under solving. This is the geometry a rectangle *is* now
/// (SPEC §5.3): the four lines are auto-recognised as a `Polygon` face (#66). Corner `i`
/// is the shared endpoint of `lines[i-1].End`/`lines[i].Start` (wrapping): corners
/// 0=BL, 1=BR, 2=TR, 3=TL; edges bottom, right, top, left.
///
/// Returns the four line indices in edge order. Does **not** add width/height dimensions or
/// solve — callers add `DistanceTarget::LineLength` dims and solve as needed.
pub fn add_line_rectangle(
    doc: &mut Document,
    sketch: SketchId,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    construction_edges: [bool; 4],
) -> [crate::model::LineKey; 4] {
    use crate::model::{
        Constraint, ConstraintEntity, ConstraintKind, ConstraintLine, ShapeKind,
    };
    let corners = [
        (x, y),
        (x + w, y),
        (x + w, y + h),
        (x, y + h),
    ];
    let mut keys = Vec::with_capacity(4);
    for i in 0..4 {
        let (u0, v0) = corners[i];
        let (u1, v1) = corners[(i + 1) % 4];
        let mut line = Line::from_local_endpoints(sketch, u0, v0, u1, v1);
        line.construction = construction_edges[i];
        keys.push(doc.lines.insert(line));
        doc.shape_order.push(ShapeKind::Line);
    }
    let idx: [crate::model::LineKey; 4] = [keys[0], keys[1], keys[2], keys[3]];
    let mut push = |kind: ConstraintKind| {
        doc.constraints.insert(Constraint {
            sketch,
            kind,
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        doc.shape_order.push(ShapeKind::Constraint);
    };
    // Coincident: each line's End meets the next line's Start, closing the loop.
    for i in 0..4 {
        push(ConstraintKind::Coincident {
            a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                line: idx[i],
                end: LineEnd::End,
            }),
            b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                line: idx[(i + 1) % 4],
                end: LineEnd::Start,
            }),
        });
    }
    // Bottom (0) & top (2) parallel to the sketch X axis; right (1) & left (3) parallel to Y
    // (#577) — the axis-based replacement for the old Horizontal/Vertical constraints.
    use crate::model::SketchAxis;
    let x_axis = ConstraintLine::OriginAxis(SketchAxis::X);
    let y_axis = ConstraintLine::OriginAxis(SketchAxis::Y);
    push(ConstraintKind::Parallel { line_a: ConstraintLine::Line(idx[0]), line_b: x_axis.clone() });
    push(ConstraintKind::Parallel { line_a: ConstraintLine::Line(idx[2]), line_b: x_axis });
    push(ConstraintKind::Parallel { line_a: ConstraintLine::Line(idx[1]), line_b: y_axis.clone() });
    push(ConstraintKind::Parallel { line_a: ConstraintLine::Line(idx[3]), line_b: y_axis });
    idx
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::retain_ground_plane_only;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::constraint_key_for_slot as nkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::body_key_for_slot as bkey;
    use super::*;
    use eframe::egui::Pos2;

    #[test]
    fn face_plane_axes_match_the_ground_and_point_up(){
        // #399: a plane offset from Ground must inherit Ground's axes exactly — the old
        // basis came out rotated 90°, so same-(u,v) sketches on parent and offset plane
        // didn't line up (a loft between them leaned sideways).
        let plane = plane_from_face(30.0, Vec3::ZERO, Vec3::Z);
        assert!((plane.u_axis - Vec3::X).length() < 1e-5, "u = +X, got {:?}", plane.u_axis);
        assert!((plane.v_axis - Vec3::Y).length() < 1e-5, "v = +Y, got {:?}", plane.v_axis);
        // A vertical plane's v points world-up so sketches read upright.
        let wall = plane_from_face(0.0, Vec3::ZERO, -Vec3::Y);
        assert!((wall.v_axis - Vec3::Z).length() < 1e-5, "wall v = +Z, got {:?}", wall.v_axis);
        assert!(
            wall.u_axis.cross(wall.v_axis).dot(wall.normal) > 0.99,
            "basis stays right-handed"
        );
    }

    #[test]
    fn face_offset_moves_along_normal() {
        let plane = plane_from_face(10.0, Vec3::ZERO, Vec3::Z);
        assert!((plane.origin.z - 10.0).abs() < 1e-4);
        assert!((plane.normal.z - 1.0).abs() < 1e-4);
    }

    #[test]
    fn axis_offset_and_angle_produce_tilted_plane() {
        let plane = plane_from_axis(5.0, 90.0, Vec3::ZERO, Vec3::X);
        assert!(plane.normal.z.abs() > 0.9);
        assert!((plane.origin.length() - 5.0).abs() < 1e-3);
    }

    #[test]
    fn axis_plane_basis_stays_continuous_through_full_rotation() {
        let direction = Vec3::new(1.0, 0.5, 0.2);
        let axis = direction.normalize();
        let mut prev_v: Option<Vec3> = None;
        for deg in (0..=360).step_by(3) {
            let plane = plane_from_axis(0.0, deg as f32, Vec3::ZERO, direction);
            assert!(
                plane.u_axis.dot(axis).abs() > 0.99,
                "u_axis should follow the reference line at {deg}°"
            );
            if let Some(pv) = prev_v {
                assert!(
                    pv.dot(plane.v_axis).abs() > 0.99,
                    "v_axis jumped at {deg}° (dot={})",
                    pv.dot(plane.v_axis)
                );
            }
            prev_v = Some(plane.v_axis);
        }
    }

    #[test]
    fn axis_plane_basis_avoids_hint_flip_near_z_threshold() {
        // For an X-axis line, |normal.z| crosses 0.9 near 64° — the old `plane_basis`
        // hint switch caused a visible discontinuity in this range.
        let mut prev_v: Option<Vec3> = None;
        for deg in 55..=75 {
            let plane = plane_from_axis(0.0, deg as f32, Vec3::ZERO, Vec3::X);
            if let Some(pv) = prev_v {
                assert!(
                    pv.dot(plane.v_axis).abs() > 0.99,
                    "v_axis flipped at {deg}°"
                );
            }
            prev_v = Some(plane.v_axis);
        }
    }

    #[test]
    fn typed_offset_evaluates_unit_expression() {
        let reference = PlaneReference::Face {
            origin: Vec3::ZERO,
            normal: Vec3::Z,
            label: "Ground".to_string(),
        };
        let plane = resolve_plane(&reference, "1in + 2mm", "", 3.0, 0.0, true, false);
        assert!((plane.origin.z - 27.4).abs() < 1e-3);
    }

    #[test]
    fn typed_offset_overrides_live_value() {
        let reference = PlaneReference::Face {
            origin: Vec3::ZERO,
            normal: Vec3::Z,
            label: "Ground".to_string(),
        };
        let plane = resolve_plane(&reference, "12.5", "", 3.0, 0.0, true, false);
        assert!((plane.origin.z - 12.5).abs() < 1e-4);
    }

    #[test]
    fn live_offset_used_when_not_user_edited() {
        let reference = PlaneReference::Face {
            origin: Vec3::ZERO,
            normal: Vec3::Z,
            label: "Ground".to_string(),
        };
        let plane = resolve_plane(&reference, "", "", 7.0, 0.0, false, false);
        assert!((plane.origin.z - 7.0).abs() < 1e-4);
    }

    #[test]
    fn live_face_offset_is_signed_distance_along_normal() {
        let offset = live_face_offset(Vec3::ZERO, Vec3::Z, Vec3::new(1.0, 2.0, 15.0));
        assert!((offset - 15.0).abs() < 1e-4);
    }

    #[test]
    fn face_hover_fill_is_visible_but_translucent() {
        assert!(
            FACE_HOVER_FILL_MULTIPLIER > 0.2 && FACE_HOVER_FILL_MULTIPLIER < 0.6,
            "hover fill should read as a tint, not opaque or invisible"
        );
    }

    /// #833: dragging a corner grip moves that corner and leaves the opposite one where it
    /// was, so the rectangle resizes rather than shifting.
    #[test]
    fn dragging_a_corner_grip_moves_only_that_corner() {
        let mut plane = plane_from_face(0.0, Vec3::ZERO, Vec3::Z);
        plane.extent = PlaneExtent::quadrant(100.0, 0.0);
        let handles = plane_resize_handles(&plane);
        assert_eq!(handles[0].1, Vec3::ZERO, "low grip sits on the origin corner");
        assert_eq!(handles[1].1, Vec3::new(100.0, 100.0, 0.0));

        let high = plane_extent_from_corner_drag(&plane, handles[1].0, Vec3::new(140.0, 60.0, 0.0));
        assert_eq!(high, PlaneExtent { u_min: 0.0, u_max: 140.0, v_min: 0.0, v_max: 60.0 });

        let low = plane_extent_from_corner_drag(&plane, handles[0].0, Vec3::new(-30.0, 20.0, 0.0));
        assert_eq!(low, PlaneExtent { u_min: -30.0, u_max: 100.0, v_min: 20.0, v_max: 100.0 });
    }

    /// A grip dragged past its opposite number leaves a plane at least the minimum size,
    /// never an inside-out or zero-area one (#833).
    #[test]
    fn a_corner_dragged_past_its_opposite_keeps_a_minimum_size() {
        let mut plane = plane_from_face(0.0, Vec3::ZERO, Vec3::Z);
        plane.extent = PlaneExtent::quadrant(100.0, 0.0);
        let extent = plane_extent_from_corner_drag(&plane, 2, Vec3::new(-40.0, -40.0, 0.0));
        assert!(extent.u_max - extent.u_min >= crate::model::MIN_PLANE_EXTENT_MM - 1e-4);
        assert!(extent.v_max - extent.v_min >= crate::model::MIN_PLANE_EXTENT_MM - 1e-4);
    }

    /// The grips land on the plane's own corners whatever its orientation (#833).
    #[test]
    fn grips_follow_the_planes_own_axes() {
        let mut plane = plane_from_face(0.0, Vec3::ZERO, Vec3::Y);
        plane.extent = PlaneExtent::quadrant(10.0, 0.0);
        let corners = plane_corners(&plane);
        let handles = plane_resize_handles(&plane);
        assert_eq!(handles[0].1, corners[0]);
        assert_eq!(handles[1].1, corners[2]);
        assert!((corners[2] - corners[0]).dot(plane.normal).abs() < 1e-4);
    }

    #[test]
    fn plane_corners_are_centered_on_origin() {
        let plane = plane_from_face(0.0, Vec3::new(10.0, 20.0, 0.0), Vec3::Z);
        let corners = plane_corners(&plane);
        let center = corners.iter().fold(Vec3::ZERO, |acc, c| acc + *c) / 4.0;
        assert!((center.x - 10.0).abs() < 1e-3);
        assert!((center.y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn a_picked_world_axis_maps_to_a_scene_element() {
        // #952: the axes are pickable, so they need an identity an element picker can hold —
        // without one, an axis pick had nowhere to go and the Repeat/Revolve axis inputs had to
        // keep their own bespoke state.
        for axis in [GlobalAxis::X, GlobalAxis::Y, GlobalAxis::Z] {
            assert_eq!(
                scene_element_from_pick(&PickTargetKind::GlobalAxis(axis)),
                Some(SceneElement::GlobalAxis(axis))
            );
        }
    }

    #[test]
    fn global_x_axis_picked_near_positive_x() {
        let doc = Document::default();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let target = resolve_pick_target(
            Pos2::new(50.0, 2.0),
            &project,
            Some(Vec3::new(50.0, 2.0, 0.0)),
            &doc,
            None,
        )
        .unwrap();
        assert!(matches!(target.kind, PickTargetKind::GlobalAxis(GlobalAxis::X)));
        assert!(matches!(
            target.reference,
            PlaneReference::Axis { label, .. } if label == "X axis"
        ));
    }

    #[test]
    fn global_axis_beats_ground_when_near_origin_triad() {
        let doc = Document::default();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let target = resolve_pick_target(
            Pos2::new(3.0, 2.0),
            &project,
            Some(Vec3::new(3.0, 2.0, 0.0)),
            &doc,
            None,
        )
        .unwrap();
        assert!(matches!(target.kind, PickTargetKind::GlobalAxis(_)));
    }

    /// #975: the world axes and the datum planes are pickable — a Revolve axis, a Repeat path,
    /// a plane anchor and a Slice cutter all take one — so they belong in the crowd. They were
    /// missing from it, which meant the Exploder could not offer what the armed picker was
    /// asking for: with a Revolve's Axis picker armed, the fan over the X axis was empty.
    #[test]
    fn the_crowd_offers_the_world_axes_and_the_datum_planes() {
        let (doc, _) = doc_with_plane_sketch();
        // XY-plane sketch → world (x, y, 0); project drops z. The cursor sits on the +X axis,
        // 20mm out from the origin, well clear of the origin's own pick radius.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cands = collect_pick_candidates(Pos2::new(20.0, 0.0), &project, &doc, Vec3::ZERO, None);
        let kinds: Vec<&PickTargetKind> = cands.iter().map(|c| &c.kind).collect();
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, PickTargetKind::GlobalAxis(GlobalAxis::X))),
            "the X axis runs under the cursor: {kinds:?}"
        );
        // And it is anchored on the axis near the cursor, not back at the world origin, so its
        // loupe's leader line points at the bit of it being picked.
        let axis = cands
            .iter()
            .find(|c| matches!(c.kind, PickTargetKind::GlobalAxis(GlobalAxis::X)))
            .expect("the X axis");
        assert!((axis.anchor.x - 20.0).abs() < 1.0, "anchored at {:?}", axis.anchor);

        // A datum plane the cursor is over reaches the crowd as itself. The default document's
        // XY plane contains the point.
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, PickTargetKind::ConstructionPlane(_))),
            "the datum plane under the cursor: {kinds:?}"
        );

        // #1538: the sketch's own LX/LY belong in the crowd too — 2D Mirror takes them as a
        // mirror line, and the Exploder can only offer what collect_pick_candidates lists.
        assert!(
            kinds
                .iter()
                .any(|k| matches!(k, PickTargetKind::OriginAxis(crate::model::SketchAxis::X))),
            "the sketch X axis runs under the cursor: {kinds:?}"
        );
    }

    /// #551: unlike `resolve_pick_target` (which keeps only the nearest), `collect_pick_candidates`
    /// returns the whole crowd within the hitbox — every endpoint and edge under the cursor — so
    /// the Selection Exploder can fan them out. Deduped per element and ordered nearest-first.
    #[test]
    fn collect_pick_candidates_returns_the_whole_crowd() {
        use crate::model::{ConstraintPoint, Line, LineEnd};
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0)); // line 0
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 0.0, 10.0)); // line 1
        // XY-plane sketch → world (x, y, 0); project drops z. The cursor sits on the shared corner.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cands = collect_pick_candidates(Pos2::new(0.0, 0.0), &project, &doc, Vec3::ZERO, None);
        let kinds: Vec<&PickTargetKind> = cands.iter().map(|c| &c.kind).collect();
        // Both segments and both coincident start endpoints are within the hitbox.
        assert!(kinds.iter().any(|k| matches!(k, PickTargetKind::Line(l) if *l == lkey(0))), "{kinds:?}");
        assert!(kinds.iter().any(|k| matches!(k, PickTargetKind::Line(l) if *l == lkey(1))), "{kinds:?}");
        assert!(kinds.iter().any(|k| matches!(
            k,
            PickTargetKind::Point(ConstraintPoint::LineEndpoint { line, end: LineEnd::Start })
                if *line == lkey(0)
        )));
        assert!(kinds.iter().any(|k| matches!(
            k,
            PickTargetKind::Point(ConstraintPoint::LineEndpoint { line, end: LineEnd::Start })
                if *line == lkey(1)
        )));
        assert!(cands.len() >= 4, "a crowd, not just the nearest: {}", cands.len());
        // No duplicates (deduped per thing).
        let mut seen = std::collections::HashSet::new();
        assert!(cands.iter().all(|c| seen.insert(crowd_key(&c.kind))), "deduped per thing");
        // Ordered nearest-first.
        assert!(cands.windows(2).all(|w| w[0].dist_px <= w[1].dist_px));
    }

    /// #551: far from any geometry there is no crowd.
    #[test]
    fn collect_pick_candidates_empty_away_from_geometry() {
        use crate::model::Line;
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        assert!(
            collect_pick_candidates(Pos2::new(500.0, 500.0), &project, &doc, Vec3::ZERO, None)
                .is_empty()
        );
    }

    fn doc_with_plane_sketch() -> (Document, crate::model::SketchId) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        (doc, sketch)
    }

    #[test]
    fn parent_from_line_pick_is_owning_sketch() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        assert_eq!(
            parent_from_pick_target(&doc, PickTargetKind::Line(lkey(0))),
            ConstructionPlaneParent::Sketch(sketch)
        );
    }

    #[test]
    fn parent_from_ground_pick_is_root() {
        let doc = Document::default();
        assert_eq!(
            parent_from_pick_target(&doc, PickTargetKind::Ground(Vec3::ZERO)),
            ConstructionPlaneParent::Root
        );
    }

    #[test]
    fn complement_axis_plus_point_makes_plane_normal_to_line_through_point() {
        // #483: line first (axis), then a separate point → face through point, normal = line dir.
        let doc = Document::default();
        let axis = PlaneReference::Axis {
            origin: Vec3::new(0.0, 5.0, 0.0),
            direction: Vec3::X,
            label: "Line".to_string(),
        };
        let point_ref = PlaneReference::Face {
            origin: Vec3::new(10.0, 20.0, 0.0),
            normal: Vec3::Z,
            label: "Point".to_string(),
        };
        let (upgraded, source, labels, _, _) = complement_plane_anchor(
            &doc,
            PlaneAnchorSource::Axis,
            &axis,
            None,
            None,
            &PickTargetKind::Point(crate::model::ConstraintPoint::CircleCenter(rkey(0))),
            &point_ref,
        )
        .expect("axis + point should complement");
        assert_eq!(source, PlaneAnchorSource::LineAndPoint);
        assert_eq!(labels, vec!["Point".to_string(), "Line".to_string()]);
        match upgraded {
            PlaneReference::Face {
                origin, normal, ..
            } => {
                assert!((origin - Vec3::new(10.0, 20.0, 0.0)).length() < 1e-4);
                assert!((normal - Vec3::X).length() < 1e-4 || (normal + Vec3::X).length() < 1e-4);
            }
            other => panic!("expected Face, got {other:?}"),
        }
    }

    #[test]
    fn complement_point_plus_line_makes_plane_normal_to_line_through_point() {
        // #483: point first, then a line → same result, other pick order.
        let doc = Document::default();
        let point = PlaneReference::Face {
            origin: Vec3::new(10.0, 20.0, 0.0),
            normal: Vec3::Z,
            label: "Vertex (line 0)".to_string(),
        };
        let line_ref = PlaneReference::Axis {
            origin: Vec3::new(0.0, 5.0, 0.0),
            direction: Vec3::new(0.0, 1.0, 0.0),
            label: "Line".to_string(),
        };
        let (upgraded, source, labels, _, _) = complement_plane_anchor(
            &doc,
            PlaneAnchorSource::Point,
            &point,
            None,
            None,
            &PickTargetKind::Line(lkey(1)),
            &line_ref,
        )
        .expect("point + line should complement");
        assert_eq!(source, PlaneAnchorSource::LineAndPoint);
        assert_eq!(labels[1], "Line");
        match upgraded {
            PlaneReference::Face {
                origin, normal, ..
            } => {
                assert!((origin - Vec3::new(10.0, 20.0, 0.0)).length() < 1e-4);
                assert!((normal - Vec3::Y).length() < 1e-4 || (normal + Vec3::Y).length() < 1e-4);
            }
            other => panic!("expected Face, got {other:?}"),
        }
    }

    #[test]
    fn complement_curve_plus_endpoint_uses_endpoint_tangent_not_mid_segment() {
        // #483: a bezier curve picked as axis (wrong mid-segment direction) + its start
        // endpoint must use the curve tangent at that end (+Y toward the near handle).
        use crate::model::{ConstraintPoint, Line, LineEnd};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        // Start (6,4), near handle (6,12) → outward at start is -Y (away from handle).
        // Mid-segment direction along the chord is roughly +X/+Y — not the end tangent.
        let mut curve = Line::from_local_endpoints(sketch, 6.0, 4.0, 26.0, 14.0);
        curve.bezier = Some([(6.0, 12.0), (18.0, 14.0)]);
        doc.lines.insert(curve);

        let mid_segment_dir = Vec3::new(1.0, 0.5, 0.0).normalize();
        let axis = PlaneReference::Axis {
            origin: Vec3::new(16.0, 9.0, 0.0),
            direction: mid_segment_dir,
            label: "Curve".to_string(),
        };
        let point = ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::Start,
        };
        let point_ref = PlaneReference::Face {
            origin: Vec3::new(6.0, 4.0, 0.0),
            normal: Vec3::Z,
            label: "Vertex".to_string(),
        };
        let (upgraded, source, _, _, _) = complement_plane_anchor(
            &doc,
            PlaneAnchorSource::Axis,
            &axis,
            Some(lkey(0)),
            None,
            &PickTargetKind::Point(point),
            &point_ref,
        )
        .expect("curve + endpoint should complement");
        assert_eq!(source, PlaneAnchorSource::LineAndPoint);
        match upgraded {
            PlaneReference::Face { normal, origin, .. } => {
                assert!((origin - Vec3::new(6.0, 4.0, 0.0)).length() < 1e-3);
                // Outward at start = away from handle (6,12): -Y
                assert!(
                    (normal - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-3,
                    "expected -Y end tangent, got {normal:?} (not mid-segment {mid_segment_dir:?})"
                );
            }
            other => panic!("expected Face, got {other:?}"),
        }
    }

    #[test]
    fn sketch_line_is_curve_detects_bezier() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut curve = Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 10.0);
        curve.bezier = Some([(0.0, 5.0), (5.0, 10.0)]);
        doc.lines.insert(curve);
        assert!(!sketch_line_is_curve(&doc, lkey(0)));
        assert!(sketch_line_is_curve(&doc, lkey(1)));
    }

    #[test]
    fn complement_face_alone_rejects_second_pick() {
        // A face anchor is already a complete set; a further pick must not rewrite it.
        let doc = Document::default();
        let face = PlaneReference::Face {
            origin: Vec3::ZERO,
            normal: Vec3::Z,
            label: "Ground".to_string(),
        };
        let line_ref = PlaneReference::Axis {
            origin: Vec3::ZERO,
            direction: Vec3::X,
            label: "Line".to_string(),
        };
        assert!(complement_plane_anchor(
            &doc,
            PlaneAnchorSource::Face,
            &face,
            None,
            None,
            &PickTargetKind::Line(lkey(0)),
            &line_ref,
        )
        .is_none());
    }

    #[test]
    fn complemented_anchor_rows_stay_point_then_line() {
        let point = SceneElement::Origin;
        let line = SceneElement::Line(lkey(3));
        let other_point = SceneElement::Point(crate::model::ConstraintPoint::CircleCenter(rkey(1)));
        let other_line = SceneElement::Line(lkey(9));

        // A line held alone, completed by a point: the point leads.
        assert_eq!(
            complemented_anchor_elements(
                PlaneAnchorSource::Axis,
                &[line.clone()],
                Some(point.clone()),
                true,
            ),
            vec![point.clone(), line.clone()],
        );
        // A point held alone, completed by a line: same order.
        assert_eq!(
            complemented_anchor_elements(
                PlaneAnchorSource::Point,
                &[point.clone()],
                Some(line.clone()),
                false,
            ),
            vec![point.clone(), line.clone()],
        );
        // Re-picking one half of a settled set replaces only that half.
        assert_eq!(
            complemented_anchor_elements(
                PlaneAnchorSource::LineAndPoint,
                &[point.clone(), line.clone()],
                Some(other_point.clone()),
                true,
            ),
            vec![other_point, line.clone()],
        );
        assert_eq!(
            complemented_anchor_elements(
                PlaneAnchorSource::LineAndPoint,
                &[point.clone(), line],
                Some(other_line.clone()),
                false,
            ),
            vec![point, other_line],
        );
    }

    #[test]
    fn vertex_normal_candidates_follow_line_and_curve_tangents() {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, Line, LineEnd};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        // A straight line along +X and a curve leaving the shared vertex along +Y
        // (its near handle sits at (10, 5) above the vertex (10, 0)).
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut curve = Line::from_local_endpoints(sketch, 10.0, 0.0, 20.0, 10.0);
        curve.bezier = Some([(10.0, 5.0), (15.0, 10.0)]);
        doc.lines.insert(curve);
        doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: lkey(0),
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: lkey(1),
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });

        let candidates = vertex_normal_candidates(
            &doc,
            &ConstraintPoint::LineEndpoint { line: lkey(0), end: LineEnd::End },
        );
        assert_eq!(candidates.len(), 2, "one candidate per incident line");
        // Straight line 0: outward direction at its end is +X (away from (0,0)).
        assert!((candidates[0].1 - Vec3::X).length() < 1e-4, "{:?}", candidates[0]);
        // Curve 1: tangent at its start points toward its near handle (+Y), so the
        // outward direction is -Y.
        assert!((candidates[1].1 - Vec3::new(0.0, -1.0, 0.0)).length() < 1e-4, "{:?}", candidates[1]);

        // A lone endpoint (no coincidence) still yields its own line's direction.
        let solo = vertex_normal_candidates(
            &doc,
            &ConstraintPoint::LineEndpoint { line: lkey(1), end: LineEnd::End },
        );
        assert_eq!(solo.len(), 1);
        // Outward at the curve's end = away from its near handle (15,10) -> (20,10): +X.
        assert!((solo[0].1 - Vec3::X).length() < 1e-4, "{:?}", solo[0]);
    }

    #[test]
    fn pick_reference_prefers_line_over_ground() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 0.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let reference = resolve_pick_target(Pos2::new(50.0, 2.0), &project, Some(Vec3::ZERO), &doc, None)
            .map(|t| t.reference);
        assert!(matches!(reference, Some(PlaneReference::Axis { .. })));
    }

    /// #1462: a cuboid hidden behind another cuboid is not fillet-picked through the front one.
    #[test]
    fn treatable_pick_does_not_go_through_a_front_body() {
        let mut doc = Document::default();
        let mut hidden = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        hidden.width = "20".into();
        hidden.depth = "20".into();
        hidden.height = "20".into();
        let hpi = doc.primitives.insert(hidden);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(hpi),
            material: None,
            name: None,
            shadow: false,
        });
        let mut slab = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        slab.origin = [0.0, 30.0, 0.0];
        slab.width = "120".into();
        slab.depth = "30".into();
        slab.height = "100".into();
        let spi = doc.primitives.insert(slab);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(spi),
            material: None,
            name: None,
            shadow: false,
        });
        let eye = Vec3::new(162.0, 162.0, 172.0);
        let project = |w: Vec3| {
            let dir = (w - eye).normalize_or_zero();
            let z = dir.dot(Vec3::new(-1.0, -1.0, -1.0).normalize_or_zero()).max(0.01);
            Some(Pos2::new(
                dir.dot(Vec3::new(1.0, -1.0, 0.0).normalize()) / z * 200.0,
                dir.dot(Vec3::new(-1.0, -1.0, 2.0).normalize()) / z * 200.0,
            ))
        };
        let origin = project(Vec3::ZERO).unwrap();
        let visibility = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &visibility, eye);
        assert!(
            occ.occluded(Vec3::ZERO),
            "the origin sits behind the slab from this eye"
        );
        let picked = pick_treatable_edges(origin, &project, &doc, eye, Some(&occ));
        assert!(
            picked
                .iter()
                .all(|(solid, _)| *solid != crate::model::TreatableSolid::Primitive(hpi)),
            "must not pick the hidden cuboid, got {picked:?}"
        );
        assert!(!picked.is_empty(), "should pick the slab in front, got nothing");
    }

    /// #1462: an analytic treatable edge hidden behind a body is not the pick.
    /// From +X the near and far +Y verticals stack; occlusion must take the front one.
    #[test]
    fn treatable_pick_does_not_take_an_edge_through_a_body() {
        let mut doc = Document::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        shape.width = "40".into();
        shape.depth = "50".into();
        shape.height = "22".into();
        let pi = doc.primitives.insert(shape);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        // Shear X into screen-x so the near and far +Y verticals do not coincide:
        // far (x = −20) sits at screen 24, near (x = +20) at 26. A cursor on the far
        // edge would take that hidden edge without occlusion.
        let project = |w: Vec3| Some(Pos2::new(w.y + w.x * 0.05, w.z));
        let eye = Vec3::new(100.0, 0.0, 11.0);
        let cursor = Pos2::new(24.0, 11.0);
        let visibility = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &visibility, eye);

        let picked = resolve_pick_target(cursor, &project, None, &doc, Some(&occ));
        match picked.as_ref().map(|t| &t.kind) {
            Some(PickTargetKind::BodyEdge { a, b, .. }) => {
                let mid = (*a + *b) * 0.5;
                assert!(
                    mid.x > 0.0,
                    "must pick the near vertical (x > 0), got {a:?}–{b:?}"
                );
            }
            other => panic!("expected a body edge on the front, got {other:?}"),
        }

        let hit = nearest_treatable_edge(cursor, &project, &doc, Some(&occ));
        let (_, _, a, b, _) = hit.expect("a visible treatable edge");
        let mid = (a + b) * 0.5;
        assert!(
            mid.x > 0.0,
            "fillet must not take the far vertical through the body, got {a:?}–{b:?}"
        );
    }

    /// #1543: after a slice, hovering the original cuboid edge that now runs
    /// through the cut must not pick that uncut analytic segment.
    #[test]
    fn treatable_pick_does_not_take_a_sliced_away_original_edge() {
        use crate::actions::{Action, ActionResult, AppState};
        use crate::model::{FaceId, Primitive, PrimitiveKind, SliceCutter};
        use crate::model::body_key_for_slot as bkey;
        use crate::model::plane_key_for_slot as pkey;

        let mut state = AppState::default();
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "40".into();
        shape.depth = "50".into();
        shape.height = "22".into();
        assert!(matches!(
            state.apply(Action::CreateShape { shape }),
            ActionResult::Ok
        ));
        assert!(matches!(
            state.apply(Action::CreateSliceOperation {
                targets: vec![bkey(0)],
                cutters: vec![SliceCutter::Face(FaceId::ConstructionPlane(pkey(2)))],
                extend_infinite: true,
            }),
            ActionResult::Ok
        ));
        // Hide the x < 0 fragment so the original top +Y edge's left half is gone.
        let left = state
            .doc
            .bodies
            .iter()
            .find(|(bi, b)| {
                !b.shadow
                    && crate::extrude::body_solid_mesh(&state.doc, *bi)
                        .and_then(|m| m.bounds())
                        .is_some_and(|(min, max)| max.x < 1.0 && min.x < -1.0)
            })
            .map(|(bi, _)| bi);
        if let Some(left) = left {
            state.doc.bodies[left].shadow = true;
        }

        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(0.0, 0.0, 200.0);
        let visibility = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&state.doc, &visibility, eye);

        // The original top +Y edge ran through (−10, 25, 22) — now empty space.
        let cut_away = project(Vec3::new(-10.0, 25.0, 22.0)).unwrap();
        let hit = nearest_treatable_edge(cut_away, &project, &state.doc, Some(&occ));
        if let Some((_, _, a, b, _)) = hit {
            assert!(
                a.x * b.x > -1.0,
                "must not pick the original uncut cuboid edge through the slice, got {a:?}–{b:?}"
            );
            assert!(
                a.x.min(b.x) > -1.0,
                "a pick over the cut-away half must not reach x < 0, got {a:?}–{b:?}"
            );
        }
        let picked = pick_treatable_edges(cut_away, &project, &state.doc, eye, Some(&occ));
        for (solid, edge) in &picked {
            let segs: Vec<_> = crate::extrude::treatable_edges(&state.doc)
                .into_iter()
                .filter(|(s, r, _, _)| s == solid && r == edge)
                .map(|(_, _, a, b)| (a, b))
                .collect();
            for (a, b) in segs {
                assert!(
                    a.x.min(b.x) > -1.0,
                    "highlighting {solid:?} {edge:?} would still draw the cut-away original edge {a:?}–{b:?}"
                );
            }
        }

        // The remaining visible half of that edge is still pickable.
        let remain = project(Vec3::new(10.0, 25.0, 22.0)).unwrap();
        let hit = nearest_treatable_edge(remain, &project, &state.doc, Some(&occ))
            .expect("the remaining visible edge must still be pickable");
        let (_, _, a, b, _) = hit;
        let mid = (a + b) * 0.5;
        assert!(
            mid.x > 0.0 && a.x.min(b.x) > -1.0,
            "remaining pick must stay on the live fragment, got {a:?}–{b:?}"
        );
    }

    #[test]
    fn nearest_treatable_edge_finds_circle_cap_rims() {
        use crate::actions::{Action, AppState, Tool};
        use crate::model::{Circle, ExtrudeFace, ExtrusionEdgeRef, FaceId};

        let mut state = AppState::default();
        state.apply(Action::BeginSketch { face: FaceId::ConstructionPlane(pkey(0)), viewport: None });
        let sketch = state.sketch_session.unwrap().sketch;
        state.doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        state.doc.shape_order.push(crate::model::ShapeKind::Circle);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace { face: ExtrudeFace::Circle(rkey(0)) });
        state.apply(Action::SetExtrudeDistance { distance: 6.0 });
        state.apply(Action::CommitExtrusion);

        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let hit = nearest_treatable_edge(Pos2::new(5.0, 0.0), &project, &state.doc, None);
        // Cap rims of a cylinder are treatable analytic circle edges (#177).
        let (_, edge, _, _, _) = hit.expect("rim should be pickable");
        assert!(matches!(edge, ExtrusionEdgeRef::Cap { edge: 0, .. }));
    }

    #[test]
    fn coplanar_face_boundary_drops_the_shared_diagonal() {
        // A split-quad face's two triangles share their diagonal; the boundary is the 4
        // perimeter edges only (the interior diagonal, shared by both triangles, is dropped).
        let triangles = [
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
            ],
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
        ];
        assert_eq!(coplanar_face_boundary(&triangles).len(), 4);
    }

    #[test]
    fn coplanar_face_boundary_loop_orders_a_split_quad() {
        // Same split-quad as the boundary test; the loop must be 4 corners, every edge on boundary.
        let triangles = [
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
            ],
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
        ];
        let loop_pts = coplanar_face_boundary_loop(&triangles);
        assert_eq!(loop_pts.len(), 4, "quad outline has 4 corners, got {loop_pts:?}");
        let boundary = coplanar_face_boundary(&triangles);
        let quant = |v: Vec3| {
            (
                (v.x * 1000.0).round() as i64,
                (v.y * 1000.0).round() as i64,
                (v.z * 1000.0).round() as i64,
            )
        };
        let bset: std::collections::HashSet<_> = boundary
            .iter()
            .map(|(a, b)| {
                let (ka, kb) = (quant(*a), quant(*b));
                if ka <= kb { (ka, kb) } else { (kb, ka) }
            })
            .collect();
        for i in 0..4 {
            let a = loop_pts[i];
            let b = loop_pts[(i + 1) % 4];
            let (ka, kb) = (quant(a), quant(b));
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            assert!(bset.contains(&key), "loop edge missing from boundary");
        }
    }

    /// #1639: the loop is a document-facing *index* — `ConstraintLine::FaceEdge { index }`
    /// names `loop[index]`..`loop[index + 1]`. Walking it from a `HashMap`'s iteration order
    /// started it at a different corner on every call, so a point snapped onto one edge was
    /// re-solved onto another, and the line jumped somewhere else entirely.
    #[test]
    fn coplanar_face_boundary_loop_is_the_same_every_call() {
        // An L-shaped face (the reporter's column + arm), triangulated as a fan.
        let corners = [
            (0.0f32, 0.0f32),
            (20.0, 0.0),
            (20.0, 30.0),
            (80.0, 30.0),
            (80.0, 50.0),
            (20.0, 50.0),
            (20.0, 80.0),
            (0.0, 80.0),
        ];
        let pt = |(y, z): (f32, f32)| Vec3::new(20.0, y, z);
        let mut triangles = Vec::new();
        for i in 1..corners.len() - 1 {
            triangles.push([pt(corners[0]), pt(corners[i]), pt(corners[i + 1])]);
        }
        let first = coplanar_face_boundary_loop(&triangles);
        assert_eq!(first.len(), 8, "the L outline has 8 corners, got {first:?}");
        for round in 0..8 {
            let again = coplanar_face_boundary_loop(&triangles);
            assert_eq!(
                again, first,
                "round {round} walked the loop differently — face-edge indices must be stable"
            );
        }
    }

    #[test]
    fn coplanar_face_boundary_loop_handles_occt_style_diagonal() {
        // OCCT often triangulates A-B-D + B-C-D (diagonal B-D), not A-B-C + A-C-D.
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(2.0, 0.0, 0.0);
        let c = Vec3::new(2.0, 1.0, 0.0);
        let d = Vec3::new(0.0, 1.0, 0.0);
        let triangles = [[a, b, d], [b, c, d]];
        let loop_pts = coplanar_face_boundary_loop(&triangles);
        assert_eq!(loop_pts.len(), 4);
        let boundary = coplanar_face_boundary(&triangles);
        assert_eq!(boundary.len(), 4);
        let quant = |v: Vec3| {
            (
                (v.x * 1000.0).round() as i64,
                (v.y * 1000.0).round() as i64,
                (v.z * 1000.0).round() as i64,
            )
        };
        let bset: std::collections::HashSet<_> = boundary
            .iter()
            .map(|(a, b)| {
                let (ka, kb) = (quant(*a), quant(*b));
                if ka <= kb { (ka, kb) } else { (kb, ka) }
            })
            .collect();
        for i in 0..loop_pts.len() {
            let p = loop_pts[i];
            let q = loop_pts[(i + 1) % loop_pts.len()];
            let (ka, kb) = (quant(p), quant(q));
            let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
            assert!(bset.contains(&key), "bad edge {p:?}-{q:?}");
        }
    }


    fn doc_with_imported_triangle_body() -> Document {
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(0.0, 10.0, 0.0),
            ]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        doc
    }

    #[test]
    fn nearest_body_vertex_picks_a_mesh_corner() {
        let doc = doc_with_imported_triangle_body();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let (kind, _) = nearest_body_vertex(Pos2::new(10.0, 1.0), &project, &doc).unwrap();
        assert!(matches!(
            kind,
            PickTargetKind::BodyVertex { body, position }
                if body == bkey(0)
                    && (position - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4
        ));
    }

    #[test]
    fn nearest_body_vertex_misses_when_cursor_far_from_any_corner() {
        let doc = doc_with_imported_triangle_body();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        assert!(nearest_body_vertex(Pos2::new(50.0, 50.0), &project, &doc).is_none());
    }

    /// #1120: tessellated cut-rim points (smooth degree-2 chain) are not pickable vertices;
    /// only sharp corners (free ends, T-junctions, hard turns) are.
    #[test]
    fn nearest_body_vertex_skips_smooth_rim_tessellation_points() {
        // Regular 24-gon disc: every rim edge is a mesh boundary (feature), but each rim
        // vertex is a smooth link (turn 15°) — no pickable corners on the rim.
        let n = 24usize;
        let r = 10.0_f32;
        let mut rim = Vec::with_capacity(n);
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            rim.push(Vec3::new(r * a.cos(), r * a.sin(), 0.0));
        }
        let center = Vec3::ZERO;
        let mut triangles = Vec::with_capacity(n);
        for i in 0..n {
            triangles.push([center, rim[i], rim[(i + 1) % n]]);
        }
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles,
            source_name: "disc".to_string(),
            step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Cursor on a rim vertex: must not offer a body vertex (select the rim edge instead).
        let rim0 = rim[0];
        assert!(
            nearest_body_vertex(Pos2::new(rim0.x, rim0.y), &project, &doc).is_none(),
            "smooth rim tessellation points must not be pickable vertices"
        );
        // Same for a vertex halfway around the circle.
        let mid = rim[n / 2];
        assert!(
            nearest_body_vertex(Pos2::new(mid.x, mid.y), &project, &doc).is_none(),
            "opposite rim point also not a corner"
        );
    }

    /// #1120: a hard corner on a feature-edge chain remains pickable.
    #[test]
    fn nearest_body_vertex_keeps_hard_corners_on_feature_edges() {
        // L-shaped polyline extruded to a thin prism so the outer/inner 90° corners are
        // creases (3+ feature edges) — those stay pickable.
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        // Bottom L (z=0) and top L (z=1); side walls close the prism.
        let bottom = [
            c(0.0, 0.0, 0.0),
            c(10.0, 0.0, 0.0),
            c(10.0, 4.0, 0.0),
            c(4.0, 4.0, 0.0),
            c(4.0, 10.0, 0.0),
            c(0.0, 10.0, 0.0),
        ];
        let top: Vec<Vec3> = bottom.iter().map(|p| *p + Vec3::Z).collect();
        let mut triangles = Vec::new();
        // Cap fans (not coplanar diagonals as features — only the outer rim is boundary).
        for i in 1..bottom.len() - 1 {
            triangles.push([bottom[0], bottom[i], bottom[i + 1]]);
            triangles.push([top[0], top[i + 1], top[i]]);
        }
        for i in 0..bottom.len() {
            let j = (i + 1) % bottom.len();
            triangles.push([bottom[i], bottom[j], top[j]]);
            triangles.push([bottom[i], top[j], top[i]]);
        }
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles,
            source_name: "L".to_string(),
            step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Outer corner at (10, 0) — three creases meet.
        let hit = nearest_body_vertex(Pos2::new(10.0, 0.0), &project, &doc);
        assert!(
            matches!(
                hit,
                Some((PickTargetKind::BodyVertex { position, .. }, _))
                    if (position.truncate() - glam::Vec2::new(10.0, 0.0)).length() < 0.5
            ),
            "hard corner (10,0) must be pickable, got {hit:?}"
        );
        // Inner re-entrant corner at (4, 4).
        let hit = nearest_body_vertex(Pos2::new(4.0, 4.0), &project, &doc);
        assert!(
            matches!(
                hit,
                Some((PickTargetKind::BodyVertex { position, .. }, _))
                    if (position.truncate() - glam::Vec2::new(4.0, 4.0)).length() < 0.5
            ),
            "inner corner (4,4) must be pickable, got {hit:?}"
        );
    }

    /// #155: with an occlusion context, a line hidden behind a visible body is not pickable;
    /// without one (or with the body hidden), it still is.
    #[test]
    fn occluded_line_is_not_picked() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 20.0, 40.0, 60.0, 40.0));
        // A blocker body (imported soup, so no kernel needed): its top face at z = 10
        // stands between the eye (z = +100) and the line (z = 0).
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let triangles = vec![
            [c(0.0, 0.0, 10.0), c(80.0, 0.0, 10.0), c(80.0, 80.0, 10.0)],
            [c(0.0, 0.0, 10.0), c(80.0, 80.0, 10.0), c(0.0, 80.0, 10.0)],
        ];
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles,
            source_name: "blocker".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });

        // Top-down view: everything projects by (x, y); the eye is above the blocker.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(40.0, 40.0, 100.0);
        let cursor = Pos2::new(40.0, 40.0);

        let visibility = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &visibility, eye);
        let picked = resolve_pick_target(cursor, &project, None, &doc, Some(&occ));
        assert!(
            !matches!(picked.as_ref().map(|t| &t.kind), Some(PickTargetKind::Line(_))),
            "line behind the body must not be picked, got {:?}",
            picked.map(|t| t.kind)
        );

        // Without occlusion the line is picked (the old X-ray behavior).
        let picked = resolve_pick_target(cursor, &project, None, &doc, None);
        assert!(matches!(picked.map(|t| t.kind), Some(PickTargetKind::Line(l)) if l == lkey(0)));

        // Ignoring the blocker is the same as it not being there (#1336).
        let occ = PickOcclusion::new_ignoring(&doc, &visibility, eye, &[bkey(0)]);
        let picked = resolve_pick_target(cursor, &project, None, &doc, Some(&occ));
        assert!(
            matches!(picked.map(|t| t.kind), Some(PickTargetKind::Line(l)) if l == lkey(0)),
            "an ignored body must not occlude"
        );
        assert!(
            !occ.pickable(&doc, &PickTargetKind::Body(bkey(0))),
            "an ignored body is not pickable"
        );

        // Hiding the body restores pickability: an invisible body must not occlude.
        let mut visibility = crate::hierarchy::ElementVisibility::default();
        visibility.set_visible(crate::hierarchy::SceneElement::Body(bkey(0)), false);
        let occ = PickOcclusion::new(&doc, &visibility, eye);
        let picked = resolve_pick_target(cursor, &project, None, &doc, Some(&occ));
        assert!(matches!(picked.map(|t| t.kind), Some(PickTargetKind::Line(l)) if l == lkey(0)));
    }

    /// #1099: a world axis running through a body is not pickable on the Select tool (the body
    /// occludes it). Without an occlusion context it still is, and the Selection Exploder keeps
    /// reaching it through its own crowd path.
    #[test]
    fn occluded_global_axis_is_not_picked() {
        let mut doc = Document::default();
        // A blocker body whose top face at z = 10 stands between the eye (z = +100) and the
        // X axis (z = 0). Its footprint straddles y = 0 so the cursor, sat on the X axis, is
        // inside it.
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let triangles = vec![
            [c(0.0, -40.0, 10.0), c(80.0, -40.0, 10.0), c(80.0, 40.0, 10.0)],
            [c(0.0, -40.0, 10.0), c(80.0, 40.0, 10.0), c(0.0, 40.0, 10.0)],
        ];
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles,
            source_name: "blocker".to_string(),
            step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });

        // Top-down view: project drops z. The cursor sits on the +X axis, inside the blocker.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(40.0, 0.0, 100.0);
        let cursor = Pos2::new(40.0, 0.0);

        let visibility = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &visibility, eye);
        let picked = resolve_pick_target(cursor, &project, None, &doc, Some(&occ));
        assert!(
            !matches!(
                picked.as_ref().map(|t| &t.kind),
                Some(PickTargetKind::GlobalAxis(_))
            ),
            "an axis through a body must not be picked, got {:?}",
            picked.map(|t| t.kind)
        );

        // Without occlusion the axis is picked (the X-ray behavior the Exploder relies on).
        let picked = resolve_pick_target(cursor, &project, None, &doc, None);
        assert!(matches!(
            picked.map(|t| t.kind),
            Some(PickTargetKind::GlobalAxis(GlobalAxis::X))
        ));

        // The Exploder's crowd still offers the buried axis (collect_pick_candidates is not
        // occlusion-gated) — that is the documented way to reach it.
        let cands = collect_pick_candidates(cursor, &project, &doc, eye, Some(&occ));
        assert!(
            cands.iter().any(|c| matches!(
                c.kind,
                PickTargetKind::GlobalAxis(GlobalAxis::X)
            )),
            "the Exploder crowd must still offer the buried axis"
        );
    }

    /// #1101: a sphere primitive's tessellation vertices are not offered by the Select tool's
    /// crowd (`collect_pick_candidates`) — only the whole sphere body is selectable.
    #[test]
    fn sphere_primitive_vertices_are_not_in_the_crowd() {
        use crate::model::{Body, BodySource, Primitive};
        let mut doc = Document::default();
        // A sphere of radius 10 resting on the ground at the world origin: its centre is at
        // (0, 0, 10), so a top-down projection puts vertices all around (0, 10) on screen.
        let mut sphere = Primitive::new(crate::model::PrimitiveKind::Sphere);
        sphere.radius = "10".to_string();
        let pi = doc.primitives.insert(sphere);
        let _sphere_body = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });

        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Right at the sphere's north pole, where many tessellation vertices sit. The
        // Exploder crowd excludes a sphere's tessellation vertices (#1101).
        let on_sphere = Pos2::new(0.0, 10.0);
        let cands = collect_pick_candidates(on_sphere, &project, &doc, Vec3::new(0.0, 0.0, 100.0), None);
        assert!(
            cands.iter().all(|c| !matches!(
                c.kind,
                PickTargetKind::BodyVertex { .. }
            )),
            "the Exploder crowd must not offer a sphere's vertices"
        );
    }

    fn overlapping_sphere_on_cuboid_corner() -> (Document, crate::model::BodyKey, crate::model::BodyKey)
    {
        use crate::model::{Body, BodySource, Primitive, PrimitiveKind};
        let mut doc = Document::default();
        let mut cuboid = Primitive::new(PrimitiveKind::Cuboid);
        cuboid.width = "40".to_string();
        cuboid.depth = "40".to_string();
        cuboid.height = "20".to_string();
        let ci = doc.primitives.insert(cuboid);
        let cube = doc.bodies.insert(Body {
            source: BodySource::Primitive(ci),
            material: None,
            name: None,
            shadow: false,
        });
        let mut sphere = Primitive::new(PrimitiveKind::Sphere);
        sphere.origin = [20.0, 20.0, 0.0];
        sphere.radius = "12".to_string();
        let pi = doc.primitives.insert(sphere);
        let sphere_body = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        (doc, cube, sphere_body)
    }

    fn pick_kind_body(kind: &PickTargetKind) -> Option<crate::model::BodyKey> {
        match kind {
            PickTargetKind::BodyFace { body, .. }
            | PickTargetKind::BodyCylinder { body, .. }
            | PickTargetKind::BodyEdge { body, .. }
            | PickTargetKind::BodyVertex { body, .. }
            | PickTargetKind::Body(body) => Some(*body),
            _ => None,
        }
    }

    /// #1578: clicking the middle of a sphere's silhouette picks the sphere, even when that
    /// disc overlaps a cuboid whose edges run through the same pixel.
    #[test]
    fn a_sphere_is_picked_through_the_middle_of_its_disc() {
        let (doc, _cube, sphere) = overlapping_sphere_on_cuboid_corner();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(20.0, 20.0, 100.0);
        let cursor = Pos2::new(20.0, 20.0);
        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);

        let picked = resolve_pick_target(cursor, &project, None, &doc, Some(&occ))
            .expect("the sphere's disc should pick something");
        assert_eq!(
            pick_kind_body(&picked.kind),
            Some(sphere),
            "the middle of the sphere must take the sphere, not the cuboid, got {:?}",
            picked.kind
        );

        let cands = collect_pick_candidates(cursor, &project, &doc, eye, Some(&occ));
        assert!(
            cands.iter().any(|c| matches!(c.kind, PickTargetKind::Body(b) if b == sphere)),
            "the crowd must offer the sphere body at its centre, got {:?}",
            cands.iter().map(|c| &c.kind).collect::<Vec<_>>()
        );
    }

    /// #258: a hidden or shadow sketch line is neither selectable nor hoverable — it drops out
    /// of the pick candidates whenever a visibility/occlusion context is present.
    #[test]
    fn hidden_or_shadow_line_is_not_picked() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 20.0, 40.0, 60.0, 40.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(40.0, 40.0, 100.0);
        let cursor = Pos2::new(40.0, 40.0);

        // Visible: the line is picked.
        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);
        assert!(matches!(
            resolve_pick_target(cursor, &project, None, &doc, Some(&occ)).map(|t| t.kind),
            Some(PickTargetKind::Line(l)) if l == lkey(0)
        ));

        // Hiding its sketch makes the line (and its endpoints) effectively hidden → not picked.
        let mut vis = crate::hierarchy::ElementVisibility::default();
        vis.set_visible(crate::hierarchy::SceneElement::Sketch(sketch), false);
        let occ = PickOcclusion::new(&doc, &vis, eye);
        assert!(
            !matches!(
                resolve_pick_target(cursor, &project, None, &doc, Some(&occ)).map(|t| t.kind),
                Some(PickTargetKind::Line(_)) | Some(PickTargetKind::Point(_))
            ),
            "a hidden line and its endpoints must not be picked"
        );

        // A shadow line is not picked even while visible.
        doc.lines[lkey(0)].shadow = true;
        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);
        assert!(
            !matches!(
                resolve_pick_target(cursor, &project, None, &doc, Some(&occ)).map(|t| t.kind),
                Some(PickTargetKind::Line(_)) | Some(PickTargetKind::Point(_))
            ),
            "a shadow line must not be picked"
        );
    }

    /// #156: body edges and vertices map to selectable scene elements (outside sketch
    /// mode), with a canonical, direction-independent identity for edges.
    #[test]
    fn constraint_pick_becomes_selectable_constraint_element() {
        use crate::hierarchy::SceneElement;
        // A fanned-out constraint badge (#568) selects the constraint itself.
        assert_eq!(
            scene_element_from_pick(&PickTargetKind::Constraint(nkey(4))),
            Some(SceneElement::Constraint(nkey(4)))
        );
    }

    #[test]
    fn body_edge_and_vertex_picks_become_selectable_elements() {
        use crate::hierarchy::SceneElement;

        let a = Vec3::new(0.0, 0.0, 10.0);
        let b = Vec3::new(80.0, 0.0, 10.0);
        let forward = scene_element_from_pick(&PickTargetKind::BodyEdge { body: bkey(0), a, b });
        let backward = scene_element_from_pick(&PickTargetKind::BodyEdge { body: bkey(0), a: b, b: a });
        assert!(matches!(forward, Some(SceneElement::BodyEdge { body, .. }) if body == bkey(0)));
        assert_eq!(forward, backward, "edge identity must not depend on direction");

        let vertex =
            scene_element_from_pick(&PickTargetKind::BodyVertex { body: bkey(2), position: a });
        assert!(matches!(vertex, Some(SceneElement::BodyVertex { body, .. }) if body == bkey(2)));

        // Click round trip: selecting the picked edge lands in the scene selection.
        let mut state = crate::actions::AppState::default();
        state.apply(crate::actions::Action::ClickSceneElement {
            element: forward.clone().unwrap(),
            additive: false,
        });
        assert!(state.scene_selection.is_selected(forward.unwrap()));
    }

    /// A 10x10x5 box (extrusion 0) as body 0, on the XY construction plane.
    fn box_body_doc() -> Document {
        use crate::model::{Body, BodySource, ExtrudeFace, Extrusion, FaceId};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let lines = add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(lines.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        doc
    }

    /// #555/#557: a body face maps to a selectable `SceneElement::BodyFace`, keyed by quantized
    /// centroid+normal — deterministic, so the same face picked twice yields equal keys.
    #[test]
    fn body_face_pick_becomes_selectable_element() {
        use crate::hierarchy::SceneElement;
        let triangles = vec![
            [Vec3::new(0.0, 0.0, 5.0), Vec3::new(10.0, 0.0, 5.0), Vec3::new(10.0, 10.0, 5.0)],
            [Vec3::new(0.0, 0.0, 5.0), Vec3::new(10.0, 10.0, 5.0), Vec3::new(0.0, 10.0, 5.0)],
        ];
        let normal = Vec3::Z;
        let a = scene_element_from_pick(&PickTargetKind::BodyFace {
            body: bkey(3),
            triangles: triangles.clone(),
            normal,
        });
        assert!(matches!(a, Some(SceneElement::BodyFace { body, .. }) if body == bkey(3)));
        // Same face, triangles listed in a different order → same centroid/normal → equal key.
        let mut reordered = triangles.clone();
        reordered.reverse();
        let b = scene_element_from_pick(&PickTargetKind::BodyFace {
            body: bkey(3),
            triangles: reordered,
            normal,
        });
        assert_eq!(a, b, "two picks of the same face must produce equal keys");
        // A parallel face at a different height is a distinct key (centroid differs).
        let lower: Vec<[Vec3; 3]> = triangles
            .iter()
            .map(|t| [t[0].with_z(0.0), t[1].with_z(0.0), t[2].with_z(0.0)])
            .collect();
        let c = scene_element_from_pick(&PickTargetKind::BodyFace {
            body: bkey(3),
            triangles: lower,
            normal: -Vec3::Z,
        });
        assert_ne!(a, c, "parallel faces at different depths must be distinct");
    }

    /// #908: seen head-on, a box's near and far corners project onto the same pixel — the
    /// visible one must win, not whichever the mesh happens to list first.
    #[test]
    fn nearest_visible_body_vertex_beats_the_hidden_one_behind_it() {
        let doc = box_body_doc();
        // Looking straight down -Z from above: (0,0,0) and (0,0,5) share a screen point.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(0.0, 0.0, 500.0);
        let visibility = crate::hierarchy::ElementVisibility::default();
        let occlusion = PickOcclusion::new(&doc, &visibility, eye);
        let picked = nearest_body_vertex_where(Pos2::new(0.0, 0.0), &project, &doc, |kind, p| {
            occlusion.pickable(&doc, kind) && !occlusion.occluded(p)
        });
        match picked {
            Some((PickTargetKind::BodyVertex { position, .. }, _)) => assert!(
                (position.z - 5.0).abs() < 1e-4,
                "the top corner is the pickable one, got {position}"
            ),
            other => panic!("expected the visible corner, got {other:?}"),
        }
    }

    /// #902: a whole body is its own pick kind, mapping to `SceneElement::Body`.
    #[test]
    fn body_pick_becomes_the_whole_body_element() {
        use crate::hierarchy::SceneElement;
        assert_eq!(
            scene_element_from_pick(&PickTargetKind::Body(bkey(4))),
            Some(SceneElement::Body(bkey(4)))
        );
    }

    /// #902: the crowd fans the **whole body** alongside its faces/edges/corners, so the
    /// exploder can select either the body or the face under the cursor.
    #[test]
    fn collect_pick_candidates_includes_the_whole_body() {
        let doc = box_body_doc();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cands = collect_pick_candidates(Pos2::new(5.0, 5.0), &project, &doc, Vec3::ZERO, None);
        assert_eq!(
            cands
                .iter()
                .filter(|c| matches!(c.kind, PickTargetKind::Body(b) if b == bkey(0)))
                .count(),
            1,
            "exactly one whole-body candidate: {:?}",
            cands.iter().map(|c| &c.kind).collect::<Vec<_>>()
        );
    }

    /// #556: the crowd includes every body face near the cursor — front and back — as distinct
    /// candidates, not just the single nearest ray hit (and no occlusion gate drops buried ones).
    #[test]
    fn collect_pick_candidates_includes_multiple_distinct_faces() {
        let doc = box_body_doc();
        // Look straight down -Z: the top (z=5) and bottom (z=0) faces both project onto the same
        // square, so the bottom face is directly behind the top. Both must appear.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cands = collect_pick_candidates(Pos2::new(5.0, 5.0), &project, &doc, Vec3::ZERO, None);
        let faces: std::collections::HashSet<String> = cands
            .iter()
            .filter(|c| matches!(c.kind, PickTargetKind::BodyFace { .. }))
            .map(|c| crowd_key(&c.kind))
            .collect();
        assert!(
            faces.len() >= 2,
            "the crowd must fan out multiple distinct faces, got {}: {faces:?}",
            faces.len()
        );
    }

    /// #987: the crowd's order must be **deterministic** and put the face nearest the camera
    /// first. The dedupe used a `HashMap`, whose iteration order is randomly seeded per
    /// instance, and the sort by screen distance is stable — so two faces the cursor sits
    /// inside of (both at distance 0) came back in a different order every single call. The
    /// normal pick takes the first, so the hover thrashed frame to frame between the front
    /// face and the hidden one behind it.
    #[test]
    fn the_crowd_is_ordered_nearest_the_camera_and_never_varies() {
        let doc = box_body_doc();
        // Looking straight down -Z from high above: the top (z=5) and bottom (z=0) faces
        // project onto the same square, so the cursor is inside both and neither is nearer in
        // *screen* terms. The top is nearer the eye and must win, every time.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(5.0, 5.0, 1000.0);
        let first_keys: Vec<String> = {
            let cands = collect_pick_candidates(Pos2::new(5.0, 5.0), &project, &doc, eye, None);
            cands.iter().map(|c| crowd_key(&c.kind)).collect()
        };
        // Every call agrees, order included — a fresh HashMap seed must not be observable.
        for _ in 0..12 {
            let cands = collect_pick_candidates(Pos2::new(5.0, 5.0), &project, &doc, eye, None);
            let keys: Vec<String> = cands.iter().map(|c| crowd_key(&c.kind)).collect();
            assert_eq!(keys, first_keys, "the crowd's order must not vary between calls");
        }
        // And the first face in it is the one facing the camera, not the one behind it.
        let cands = collect_pick_candidates(Pos2::new(5.0, 5.0), &project, &doc, eye, None);
        let top_face = cands
            .iter()
            .filter(|c| matches!(c.kind, PickTargetKind::BodyFace { .. }))
            .map(|c| c.anchor.z)
            .next()
            .expect("the crowd holds the box's faces");
        assert!(
            top_face > 4.0,
            "the nearest face to the eye (the z=5 cap) must come first, got anchor z = {top_face}"
        );
    }

    /// #555: a narrow face seen edge-on (its projected area a thin sliver / line between its two
    /// edges) is caught by `body_faces_near` via edge distance, where a strict inside-triangle
    /// ray hit would miss it. Looking down -Z, the x=0 side face collapses to the line x=0.
    #[test]
    fn body_faces_near_catches_edge_on_narrow_face() {
        let doc = box_body_doc();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Cursor on the projected line of the x=0 side face (which has zero projected area).
        let near = crate::face::body_faces_near(Pos2::new(0.0, 5.0), &project, &doc, Vec3::ZERO, 12.0);
        assert!(
            near.iter().any(|(kind, _, _)| matches!(
                kind,
                PickTargetKind::BodyFace { normal, .. } if normal.x.abs() > 0.9
            )),
            "the edge-on x-facing side face must be reported: {:?}",
            near.iter().map(|(k, _, _)| k).collect::<Vec<_>>()
        );
    }

    #[test]
    fn line_picked_within_proximity_threshold() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 0.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let target = resolve_pick_target(Pos2::new(50.0, 8.0), &project, None, &doc, None);
        assert!(matches!(
            target.map(|t| t.kind),
            Some(PickTargetKind::Line(_))
        ));
    }

    /// #242: near a vertex the vertex wins even when the edge through it is a hair closer in
    /// pixels — so hovering a corner selects the corner, not the edge.
    #[test]
    fn vertex_beats_a_closer_edge_within_its_pick_radius() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        // Away from the world axes so only the line's vertex/edge compete.
        doc.lines.insert(Line::from_local_endpoints(sketch, 50.0, 50.0, 150.0, 50.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // (52, 55): 5px from the line (edge), 5.39px from the (50,50) endpoint — edge is closer,
        // but the vertex is within its radius, so it must win.
        let target = resolve_pick_target(Pos2::new(52.0, 55.0), &project, None, &doc, None);
        assert!(
            matches!(target.map(|t| t.kind), Some(PickTargetKind::Point(_))),
            "the vertex should win over the edge through it"
        );
    }

    #[test]
    fn line_endpoint_picked_within_point_threshold() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.insert(Line::from_local_endpoints(sketch, 100.0, 50.0, 200.0, 50.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let target = resolve_pick_target(Pos2::new(100.0, 59.0), &project, None, &doc, None);
        assert!(matches!(
            target.map(|t| t.kind),
            Some(PickTargetKind::Point(ConstraintPoint::LineEndpoint {
                line,
                end: LineEnd::Start,
            })) if line == lkey(0)
        ));
    }

    #[test]
    fn axis_normal_at_zero_angle_is_perpendicular_to_axis() {
        let normal = axis_normal(Vec3::X, 0.0);
        assert!(normal.dot(Vec3::X).abs() < 1e-4);
        assert!(normal.length() > 0.9);
    }

    #[test]
    fn gizmo_display_offset_never_collapses_to_zero() {
        assert!((gizmo_display_offset(0.0) - 4.0).abs() < 1e-4);
        assert!((gizmo_display_offset(0.5) - 4.0).abs() < 1e-4);
        assert!((gizmo_display_offset(-0.5) + 4.0).abs() < 1e-4);
        assert!((gizmo_display_offset(12.0) - 12.0).abs() < 1e-4);
    }

    #[test]
    fn offset_gizmo_hit_finds_face_offset_handle() {
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        assert!(offset_gizmo_hit(
            Pos2::new(0.0, 12.0),
            &project,
            Vec3::ZERO,
            Vec3::Z,
            12.0,
        ));
    }

    #[test]
    fn offset_from_normal_drag_moves_with_screen_motion() {
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let offset = offset_from_normal_drag(
            Vec3::ZERO,
            Vec3::Y,
            &project,
            0.0,
            Pos2::new(0.0, 0.0),
            Pos2::new(0.0, 10.0),
        );
        assert!((offset - 10.0).abs() < 1e-3);
    }

    /// #1296: free (non-target) gizmo pulls snap the live offset to 0.1 of the length unit.
    #[test]
    fn gizmo_length_drag_snaps_to_one_tenth_of_unit() {
        use crate::value::{snap_gizmo_length_mm, LengthUnit};
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Screen Y maps 1:1 to world Y (mm). Drag 12.34 mm along +Y.
        let raw = offset_from_normal_drag(
            Vec3::ZERO,
            Vec3::Y,
            &project,
            0.0,
            Pos2::new(0.0, 0.0),
            Pos2::new(0.0, 12.34),
        );
        assert!((raw - 12.34).abs() < 1e-3, "raw drag is continuous, got {raw}");
        let snapped_mm = snap_gizmo_length_mm(raw, LengthUnit::Mm);
        assert!(
            (snapped_mm - 12.3).abs() < 1e-4,
            "mm docs step by 0.1 mm, got {snapped_mm}"
        );
        // Inches: 0.1 in = 2.54 mm. A 12.34 mm pull → nearest 5 × 2.54 = 12.7.
        let snapped_in = snap_gizmo_length_mm(raw, LengthUnit::In);
        assert!(
            (snapped_in - 12.7).abs() < 1e-3,
            "inch docs step by 0.1 in, got {snapped_in}"
        );
    }

    #[test]
    fn offset_from_normal_drag_allows_negative_values() {
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let offset = offset_from_normal_drag(
            Vec3::ZERO,
            Vec3::Y,
            &project,
            5.0,
            Pos2::new(0.0, 5.0),
            Pos2::new(0.0, -5.0),
        );
        assert!((offset + 5.0).abs() < 1e-3);
    }

    /// #1196: pointing at the tip of a prospective height should resolve that height, under
    /// a perspective isometric camera — the Shape tool's free-cursor height path.
    #[test]
    fn offset_along_normal_from_cursor_matches_the_pointed_tip() {
        let mut cam = crate::camera::Camera::default(); // perspective + isometric
        cam.distance = 260.0;
        cam.target = Vec3::ZERO;
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let origin = Vec3::ZERO;
        let normal = Vec3::Z;
        for want in [10.0_f32, 25.0, 40.0, 80.0] {
            let tip = origin + normal * want;
            let screen = cam
                .project(tip, viewport, &vp)
                .expect("tip should project");
            let got = offset_along_normal_from_cursor(origin, normal, &cam, screen, viewport, &vp)
                .expect("cursor on the tip should resolve a height");
            assert!(
                (got - want).abs() < 0.25,
                "pointing at tip z={want} should yield ~{want}, got {got}"
            );
        }
    }

    /// #1196: the old screen-delta measurement from an off-centre base click leaves the tip
    /// short of the pointer under perspective — documenting why free-cursor height cannot
    /// use [`offset_from_normal_drag`] from `phase_screen`.
    #[test]
    fn offset_from_normal_drag_from_a_base_corner_lags_the_tip() {
        let mut cam = crate::camera::Camera::default();
        cam.distance = 260.0;
        cam.target = Vec3::ZERO;
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let origin = Vec3::ZERO;
        let normal = Vec3::Z;
        let want = 50.0_f32;
        let tip = origin + normal * want;
        let tip_screen = cam.project(tip, viewport, &vp).unwrap();
        // Base phase ends on an opposite corner, not the centre — that's the phase_screen.
        let corner_screen = cam
            .project(Vec3::new(20.0, 10.0, 0.0), viewport, &vp)
            .unwrap();
        let project = |w: Vec3| cam.project(w, viewport, &vp);
        let lagged = offset_from_normal_drag(
            origin,
            normal,
            &project,
            0.0,
            corner_screen,
            tip_screen,
        );
        // Must disagree with the true tip height by a visible amount (the bug).
        assert!(
            (lagged - want).abs() > 2.0,
            "expected the phase-screen relative drag to miss the tip (got {lagged}, want {want})"
        );
        // And the free-cursor helper must land on it.
        let tracked =
            offset_along_normal_from_cursor(origin, normal, &cam, tip_screen, viewport, &vp)
                .unwrap();
        assert!(
            (tracked - want).abs() < 0.25,
            "free-cursor height should track the tip, got {tracked}"
        );
    }

    /// #1196: looking straight down the normal, free-cursor height is undefined.
    #[test]
    fn offset_along_normal_from_cursor_is_none_in_plan_view() {
        let mut cam = crate::camera::Camera::default();
        let (yaw, pitch) = crate::camera::StandardView::Top.yaw_pitch();
        cam.yaw = yaw;
        cam.pitch = pitch;
        cam.distance = 260.0;
        cam.target = Vec3::ZERO;
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let screen = cam.project(Vec3::new(10.0, 10.0, 0.0), viewport, &vp).unwrap();
        assert!(offset_along_normal_from_cursor(
            Vec3::ZERO,
            Vec3::Z,
            &cam,
            screen,
            viewport,
            &vp
        )
        .is_none());
    }

    #[test]
    fn axis_offset_handle_supports_negative_offset() {
        let tip = axis_offset_handle(Vec3::ZERO, Vec3::Y, -10.0, 0.0);
        assert!(tip.x < -9.0);
    }

    #[test]
    fn signed_axis_offset_resolves_for_negative_text() {
        let reference = PlaneReference::Axis {
            origin: Vec3::ZERO,
            direction: Vec3::Y,
            label: "Line".to_string(),
        };
        let plane = resolve_plane(&reference, "-8", "", 0.0, 0.0, true, false);
        assert!(plane.origin.x < -7.0);
    }

    #[test]
    fn angle_from_axis_plane_hit_round_trips_gizmo_handle() {
        for deg in [0.0, 45.0, 90.0, 135.0, 180.0] {
            let hit = axis_angle_handle(Vec3::ZERO, Vec3::Y, deg);
            let angle = angle_from_axis_plane_hit(Vec3::ZERO, Vec3::Y, hit);
            let diff = (angle - deg).abs();
            assert!(
                diff < 1.0 || (diff - 360.0).abs() < 1.0,
                "deg={deg} got={angle}"
            );
        }
    }

    /// #1432: a long-way wrap like 298.6° is the short signed turn (−61.4°).
    #[test]
    fn wrap_signed_deg_keeps_the_short_turn() {
        let got = wrap_signed_deg(298.6);
        assert!(
            (got + 61.4).abs() < 0.05,
            "298.6° should wrap to −61.4°, got {got}"
        );
        assert!((wrap_signed_deg(-61.4) + 61.4).abs() < 0.05);
        assert!((wrap_signed_deg(0.0)).abs() < 1e-4);
        assert!((wrap_signed_deg(180.0) - 180.0).abs() < 1e-4);
        assert!(wrap_signed_deg(181.0) < 0.0);
    }

    /// #1432: the 3D ring-plane angle from +X toward −Y (clockwise about +Z) is negative,
    /// never the complementary 0–360 wrap.
    #[test]
    fn signed_angle_about_axis_is_the_short_turn() {
        let origin = Vec3::ZERO;
        let axis = Vec3::Z;
        let zero = Vec3::X;
        let clockwise = Quat::from_axis_angle(axis, (-61.4f32).to_radians()) * zero;
        let hit = origin + clockwise;
        let got = signed_angle_deg_about_axis(origin, axis, zero, hit);
        assert!(
            got < 0.0 && (got + 61.4).abs() < 0.2,
            "clockwise 61.4° should stay negative, got {got}"
        );
        let ccw = Quat::from_axis_angle(axis, 40f32.to_radians()) * zero;
        let pos = signed_angle_deg_about_axis(origin, axis, zero, origin + ccw);
        assert!(
            pos > 0.0 && (pos - 40.0).abs() < 0.2,
            "ccw 40° should stay positive, got {pos}"
        );
    }

    /// #1432: a drag that crosses the atan2 branch cut is a short step, not a ~300° jump.
    #[test]
    fn rotation_gizmo_drag_takes_the_short_signed_step() {
        let crossed = rotation_gizmo_drag_deg(0.0, 170.0, -170.0);
        assert!(
            (crossed - 20.0).abs() < 0.1,
            "170° → −170° is +20°, not −340°, got {crossed}"
        );
        let report = rotation_gizmo_drag_deg(0.0, 0.0, 298.6);
        assert!(
            report < 0.0 && (report + 61.4).abs() < 0.1,
            "a 298.6° landing is the short −61.4° turn, got {report}"
        );
    }

    /// #1418: rotation gizmos grab only at the handle disc, not along the ring.
    #[test]
    fn rotation_handle_hit_is_the_disc_not_the_ring() {
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let handle = Vec3::new(20.0, 0.0, 0.0);
        assert!(rotation_handle_hit(Pos2::new(20.0, 0.0), &project, handle));
        assert!(rotation_handle_hit(Pos2::new(22.0, 1.0), &project, handle));
        // A point on the same circle, well away from the handle, is not a grab.
        assert!(!rotation_handle_hit(Pos2::new(0.0, 20.0), &project, handle));
        assert!(!rotation_handle_hit(Pos2::new(14.1, 14.1), &project, handle));
    }

    #[test]
    fn axis_gizmo_hit_finds_offset_handle_near_tip() {
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let tip = axis_offset_handle(Vec3::ZERO, Vec3::X, 15.0, 0.0);
        let screen = project(tip).unwrap();
        let hit = axis_gizmo_hit(
            screen,
            &project,
            Vec3::ZERO,
            Vec3::X,
            15.0,
            0.0,
        );
        assert_eq!(hit, Some(AxisGizmoHit::Offset));
    }

    /// #124: a construction plane extends infinitely — its rendered border is a display
    /// artifact, not real geometry, so clicking right on that border must still resolve to
    /// the plane's *face* (an infinite-plane reference), never a fake edge/axis.
    #[test]
    fn pick_near_a_construction_planes_border_resolves_to_its_face_not_an_edge() {
        let doc = Document::default();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Plane 0 is the default XY ground plane; its rendered quad corners sit at
        // (±PLANE_DISPLAY_HALF, ±PLANE_DISPLAY_HALF, 0). Pick a point on the top edge away
        // from x=0/y=0 so it can't coincidentally land on the (legitimately pickable) global
        // X/Y axes instead.
        let on_the_border = Pos2::new(30.0, PLANE_DISPLAY_HALF);
        let target = resolve_pick_target(on_the_border, &project, None, &doc, None).unwrap();
        assert_eq!(target.kind, PickTargetKind::ConstructionPlane(pkey(0)));
        assert!(matches!(target.reference, PlaneReference::Face { .. }));
    }

    #[test]
    fn pick_reference_uses_ground_when_empty() {
        let doc = Document::default();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Well clear of the three datum planes' quadrants (#833), so nothing but the ground
        // is under the click.
        let reference = resolve_pick_target(
            Pos2::new(180.0, 180.0),
            &project,
            Some(Vec3::new(180.0, 180.0, 0.0)),
            &doc,
            None,
        )
        .map(|t| t.reference);
        assert!(matches!(
            reference,
            Some(PlaneReference::Face { label, .. }) if label == "Ground"
        ));
    }

    #[test]
    fn edit_plane_offset_moves_descendant_planes() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let child = plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Face {
                    origin: Vec3::ZERO,
                    normal: Vec3::Z,
                    label: "Ground".to_string(),
                },
                5.0,
                0.0,
            ),
            ConstructionPlaneParent::Sketch(sketch),
        );
        retain_ground_plane_only(&mut doc);
        doc.construction_planes.insert(child);
        let child_origin_before = doc.construction_planes[pkey(1)].origin.z;

        let definition = definition_from_reference(
            &PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: Vec3::Z,
                label: "Ground".to_string(),
            },
            15.0,
            0.0,
        );
        apply_construction_plane_edit(
            &mut doc,
            pkey(0),
            &definition,
            ConstructionPlaneParent::Root,
        )
        .unwrap();

        let child_origin_after = doc.construction_planes[pkey(1)].origin.z;
        assert!((child_origin_after - child_origin_before - 15.0).abs() < 1e-3);
    }

    // ---- Rectangle-as-four-lines (#66) ----

    #[test]
    fn add_line_rectangle_drops_four_lines_axis_parallel_and_coincident_constraints() {
        use crate::model::{ConstraintKind, ConstraintLine, Document, FaceId, SketchAxis};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let lines = add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        // Four plain lines forming a closed loop (bottom, right, top, left).
        assert_eq!(doc.lines.len(), 4);
        assert_eq!(lines, [lkey(0), lkey(1), lkey(2), lkey(3)]);
        // #577: the edges are constrained parallel to the sketch axes (X for bottom/top, Y for
        // left/right) rather than the old Horizontal/Vertical constraints.
        let parallel_to = |axis: SketchAxis| {
            doc.constraints
                .values()
                .filter(|c| {
                    matches!(&c.kind, ConstraintKind::Parallel { line_b, .. }
                        if *line_b == ConstraintLine::OriginAxis(axis))
                })
                .count()
        };
        assert_eq!(parallel_to(SketchAxis::X), 2, "bottom + top parallel to X");
        assert_eq!(parallel_to(SketchAxis::Y), 2, "left + right parallel to Y");
        let coincident = doc
            .constraints
            .values()
            .filter(|c| matches!(c.kind, ConstraintKind::Coincident { .. }))
            .count();
        assert_eq!(coincident, 4, "four shared corners join the loop");
        // Bottom edge (0) is parallel to X; right edge (1) parallel to Y.
        assert!(doc.constraints.values().any(|c| matches!(
            &c.kind,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(l),
                line_b: ConstraintLine::OriginAxis(SketchAxis::X)
            } if *l == lkey(0)
        )));
        assert!(doc.constraints.values().any(|c| matches!(
            &c.kind,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(l),
                line_b: ConstraintLine::OriginAxis(SketchAxis::Y)
            } if *l == lkey(1)
        )));
    }

    #[test]
    fn add_line_rectangle_forms_a_recognized_polygon_face() {
        use crate::model::{Document, FaceId};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        let loops = crate::polygon::closed_line_loops(&doc, sketch);
        assert_eq!(loops.len(), 1, "the four lines are one closed loop");
        let mut sorted = loops[0].clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![lkey(0), lkey(1), lkey(2), lkey(3)]);
    }

    /// #465: the Plane tool's pick prefers a body face under the cursor over the ground
    /// fallback, but a sharp target (a body edge) still beats the face.
    #[test]
    fn plane_pick_prefers_body_face_over_ground_but_not_edges() {
        // A 10x10x10 imported-mesh box, so face/edge picking works without the kernel.
        let c = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(10.0, 10.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::new(10.0, 0.0, 10.0),
            Vec3::new(10.0, 10.0, 10.0),
            Vec3::new(0.0, 10.0, 10.0),
        ];
        let quad = |a: usize, b: usize, d: usize, e: usize| {
            vec![[c[a], c[b], c[d]], [c[a], c[d], c[e]]]
        };
        let mut triangles = Vec::new();
        for face in [
            quad(0, 1, 2, 3),
            quad(4, 5, 6, 7),
            quad(0, 1, 5, 4),
            quad(1, 2, 6, 5),
            quad(2, 3, 7, 6),
            quad(3, 0, 4, 7),
        ] {
            triangles.extend(face);
        }
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles,
            source_name: "box".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        // ×20 scale so the box spans 200 px — a real on-screen size, keeping the face
        // center well clear of the edge pick radius.
        let project = |p: Vec3| Some(egui::pos2(p.x * 20.0, p.y * 20.0));
        let eye = Vec3::new(5.0, 5.0, 100.0);

        // Center of the top face: the face wins over the ground fallback.
        let target = resolve_plane_pick_target(
            egui::pos2(100.0, 100.0),
            &project,
            Some(Vec3::new(5.0, 5.0, 0.0)),
            &doc,
            eye,
            None,
        )
        .expect("something under the cursor");
        match &target.kind {
            PickTargetKind::BodyFace { .. } => match &target.reference {
                PlaneReference::Face { origin, normal, .. } => {
                    assert!((origin.z - 10.0).abs() < 1e-3, "top-face centroid, got {origin:?}");
                    assert!(normal.z.abs() > 0.99, "top-face normal, got {normal:?}");
                }
                other => panic!("face pick should anchor a Face reference, got {other:?}"),
            },
            other => panic!("expected a body face, got {other:?}"),
        }

        // On a box edge: the sharp edge target still beats the face.
        let target = resolve_plane_pick_target(
            egui::pos2(100.0, 0.0),
            &project,
            Some(Vec3::new(5.0, 0.0, 0.0)),
            &doc,
            eye,
            None,
        )
        .expect("something under the cursor");
        assert!(
            matches!(target.kind, PickTargetKind::BodyEdge { .. }),
            "edge should win, got {:?}",
            target.kind
        );

        // Far off the box: falls back to the ground plane (the quad when the cursor is
        // over its display extent, bare ground beyond it).
        let target = resolve_plane_pick_target(
            egui::pos2(500.0, 500.0),
            &project,
            Some(Vec3::new(25.0, 25.0, 0.0)),
            &doc,
            eye,
            None,
        )
        .expect("ground fallback");
        assert!(
            matches!(
                target.kind,
                PickTargetKind::Ground(_) | PickTargetKind::ConstructionPlane(_)
            ),
            "ground fallback, got {:?}",
            target.kind
        );
    }

    #[test]
    fn plane_pick_returns_edge_axis_reference_with_occlusion() {
        // #581: a body edge clicked with a real occlusion context must resolve to the edge with an
        // Axis reference (angle+offset), not fall through to a Face. Box offset from the world axes
        // so a world axis can't stand in for the edge.
        let (lo, hi, top) = (20.0f32, 40.0f32, 20.0f32);
        let c = [
            Vec3::new(lo, lo, 0.0), Vec3::new(hi, lo, 0.0), Vec3::new(hi, hi, 0.0), Vec3::new(lo, hi, 0.0),
            Vec3::new(lo, lo, top), Vec3::new(hi, lo, top), Vec3::new(hi, hi, top), Vec3::new(lo, hi, top),
        ];
        let quad = |a: usize, b: usize, d: usize, e: usize| vec![[c[a], c[b], c[d]], [c[a], c[d], c[e]]];
        let mut triangles = Vec::new();
        for face in [quad(0, 1, 2, 3), quad(4, 5, 6, 7), quad(0, 1, 5, 4), quad(1, 2, 6, 5), quad(2, 3, 7, 6), quad(3, 0, 4, 7)] {
            triangles.extend(face);
        }
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles,
            source_name: "box".to_string(),
            step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let project = |p: Vec3| Some(egui::pos2(p.x, p.y));
        let eye = Vec3::new(30.0, 30.0, 100.0);
        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);
        // The top edge along X at y=20: midpoint (30,20,20) → screen (30,20).
        let target = resolve_plane_pick_target(egui::pos2(30.0, 20.0), &project, Some(Vec3::new(30.0, 20.0, 0.0)), &doc, eye, Some(&occ)).expect("edge under cursor");
        assert!(matches!(target.kind, PickTargetKind::BodyEdge { .. }), "expected body edge, got {:?}", target.kind);
        assert!(matches!(target.reference, PlaneReference::Axis { .. }), "expected Axis reference, got {:?}", target.reference);
    }

    #[test]
    fn typed_width_height_drive_the_rectangle_under_solving() {
        use crate::constraints::{add_distance_constraint, solve_document_constraints};
        use crate::model::{DistanceTarget, Document, FaceId};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        // Start off-size, then lock width (bottom edge) and height (right edge).
        let lines = add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 3.0, 3.0, [false; 4]);
        add_distance_constraint(&mut doc, sketch, DistanceTarget::LineLength(lines[0]), "20mm".into())
            .unwrap();
        add_distance_constraint(&mut doc, sketch, DistanceTarget::LineLength(lines[1]), "8mm".into())
            .unwrap();
        solve_document_constraints(&mut doc).unwrap();
        let loop_lines = crate::polygon::closed_line_loops(&doc, sketch);
        let verts = crate::polygon::loop_vertices_uv(&doc, sketch, &loop_lines[0]).unwrap();
        let min_u = verts.iter().map(|v| v.0).fold(f32::INFINITY, f32::min);
        let max_u = verts.iter().map(|v| v.0).fold(f32::NEG_INFINITY, f32::max);
        let min_v = verts.iter().map(|v| v.1).fold(f32::INFINITY, f32::min);
        let max_v = verts.iter().map(|v| v.1).fold(f32::NEG_INFINITY, f32::max);
        assert!((max_u - min_u - 20.0).abs() < 1e-2, "width solved to 20mm");
        assert!((max_v - min_v - 8.0).abs() < 1e-2, "height solved to 8mm");
    }

    /// #1277: when two construction planes both contain the cursor in screen space, the pick
    /// must take the one nearer the camera — not whichever was considered first (the old
    /// reverse-iteration tie left a big buried plane winning over a smaller front one).
    #[test]
    fn construction_plane_pick_prefers_frontmost_when_overlapping() {
        let mut doc = Document::default();
        // Two parallel XY-oriented planes with the same display extent. Orthographic
        // (x, y) projection puts both under every cursor in the quad; only eye-depth
        // can tell them apart.
        let extent = crate::model::PlaneExtent {
            u_min: 0.0,
            u_max: 100.0,
            v_min: 0.0,
            v_max: 100.0,
        };
        doc.construction_planes[pkey(0)].extent = extent;
        // A second plane behind the first (lower z), inserted after the default XZ/YZ so
        // reverse-iteration would prefer it over XY when screen distances are equal.
        let mut behind = crate::face::default_xy_plane();
        behind.origin = Vec3::new(0.0, 0.0, -40.0);
        behind.extent = extent;
        behind.name = Some("Behind".to_string());
        let behind_key = doc.construction_planes.insert(behind);

        // Drop z: both quads map to the same screen rectangle.
        let project = |p: Vec3| Some(egui::pos2(p.x, p.y));
        let eye = Vec3::new(50.0, 50.0, 200.0);
        let world = Vec3::new(50.0, 50.0, 0.0);
        let sp = egui::pos2(50.0, 50.0);

        // Sanity: both display quads contain the cursor, and the buried plane is listed later.
        assert!(
            point_in_screen_quad(sp, {
                let c = plane_corners(&doc.construction_planes[pkey(0)]);
                let pts: Vec<_> = c.iter().map(|&p| project(p).unwrap()).collect();
                [pts[0], pts[1], pts[2], pts[3]]
            }),
            "front plane under cursor"
        );
        assert!(
            point_in_screen_quad(sp, {
                let c = plane_corners(&doc.construction_planes[behind_key]);
                let pts: Vec<_> = c.iter().map(|&p| project(p).unwrap()).collect();
                [pts[0], pts[1], pts[2], pts[3]]
            }),
            "buried plane also under cursor"
        );
        assert!(
            behind_key.index() > 0,
            "buried plane is later so reverse-iter would take it first without depth"
        );

        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);
        let t = resolve_pick_target(sp, &project, Some(world), &doc, Some(&occ))
            .expect("a plane under the cursor");
        assert!(
            matches!(t.kind, PickTargetKind::ConstructionPlane(i) if i == pkey(0)),
            "front plane (z=0) must win over the buried one (z=-40), got {:?}",
            t.kind
        );
    }

    fn tracing_image_on_xy(origin: (f32, f32), width: f32, height: f32) -> crate::model::TracingImage {
        crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: pkey(0),
            origin,
            base_origin: None,
            width_mm: width,
            height_mm: height,
            opacity: crate::model::DEFAULT_TRACING_IMAGE_OPACITY,
            name: None,
            calibration: None,
            rotation: 0.0,
            base_rotation: None,
        }
    }

    /// #1561: the Select tool picks a tracing image by clicking its quad — including the
    /// part of the image that is *not* sitting on a construction-plane display rectangle.
    #[test]
    fn clicking_a_tracing_image_selects_the_image() {
        let mut doc = Document::default();
        // Image in the -X/-Y quadrant, well clear of the 5..105 datum-plane quads and
        // the world axes along x=0 / y=0.
        let image = doc
            .tracing_images
            .insert(tracing_image_on_xy((-200.0, -200.0), 150.0, 150.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cursor = Pos2::new(-120.0, -120.0);
        let target = resolve_pick_target(
            cursor,
            &project,
            Some(Vec3::new(-120.0, -120.0, 0.0)),
            &doc,
            None,
        )
        .expect("something under the cursor");
        assert_eq!(
            scene_element_from_pick(&target.kind),
            Some(SceneElement::Image(image)),
            "clicking the image quad should select the image, got {:?}",
            target.kind
        );

        let cands = collect_pick_candidates(cursor, &project, &doc, Vec3::new(0.0, 0.0, 200.0), None);
        assert!(
            cands.iter().any(|c| {
                matches!(
                    scene_element_from_pick(&c.kind),
                    Some(SceneElement::Image(i)) if i == image
                )
            }),
            "the exploder crowd should include the tracing image"
        );
    }

    /// #1586: a calibrated image's endpoints are the Select-tool overlay, not
    /// independent 3D pick targets. Hovering / clicking one must take the image.
    #[test]
    fn calibration_endpoints_are_not_pickable_without_a_sketch() {
        use crate::model::{default_image_calibration, ConstraintPoint};
        let mut doc = Document::default();
        let mut img = tracing_image_on_xy((-200.0, -200.0), 150.0, 150.0);
        img.calibration = Some(default_image_calibration(150.0));
        let image = doc.tracing_images.insert(img);
        // Default top-middle: origin + (0.5·w, h) = (−125, −50).
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cursor = Pos2::new(-125.0, -50.0);
        let target = resolve_pick_target(
            cursor,
            &project,
            Some(Vec3::new(-125.0, -50.0, 0.0)),
            &doc,
            None,
        )
        .expect("something under the cursor");
        assert_eq!(
            scene_element_from_pick(&target.kind),
            Some(SceneElement::Image(image)),
            "an unselected image's calibration endpoint should pick the image, got {:?}",
            target.kind
        );
        let cands = collect_pick_candidates(cursor, &project, &doc, Vec3::new(0.0, 0.0, 200.0), None);
        assert!(
            cands.iter().all(|c| {
                !matches!(
                    c.kind,
                    PickTargetKind::Point(ConstraintPoint::ImageCalibrationPoint { .. })
                )
            }),
            "the exploder crowd should not include calibration endpoints when no sketch is hosted, got {:?}",
            cands.iter().map(|c| &c.kind).collect::<Vec<_>>()
        );
    }

    /// #1589: a hosted sketch can pick an image's box corner (distinct from the
    /// default top-middle calibration point).
    #[test]
    fn image_box_corners_are_pickable_in_a_hosted_sketch() {
        use crate::model::{default_image_calibration, ConstraintPoint, TextAnchor};
        let (mut doc, _sketch) = doc_with_plane_sketch();
        let mut img = tracing_image_on_xy((-200.0, -200.0), 150.0, 150.0);
        img.calibration = Some(default_image_calibration(150.0));
        let image = doc.tracing_images.insert(img);
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Bottom-left of the 150×150 image at origin (−200, −200).
        let cursor = Pos2::new(-200.0, -200.0);
        let target = resolve_pick_target(
            cursor,
            &project,
            Some(Vec3::new(-200.0, -200.0, 0.0)),
            &doc,
            None,
        )
        .expect("something under the cursor");
        assert!(
            matches!(
                &target.kind,
                PickTargetKind::Point(ConstraintPoint::ImageAnchor {
                    image: i,
                    anchor: TextAnchor::BottomLeft
                }) if *i == image
            ),
            "a hosted sketch should pick the image's bottom-left, got {:?}",
            target.kind
        );
        let picked = nearest_sketch_point_in_sketch(
            cursor,
            &project,
            &doc,
            doc.sketches.keys().next().unwrap(),
        );
        assert!(
            matches!(
                picked,
                Some((ConstraintPoint::ImageAnchor { image: i, anchor: TextAnchor::BottomLeft }, _))
                    if i == image
            ),
            "constraint-tool point pick should take the box corner, got {:?}",
            picked
        );
        let sketch = doc.sketches.keys().next().unwrap();
        let edge = nearest_sketch_line_in_sketch(Pos2::new(-200.0, -125.0), &project, &doc, sketch);
        assert!(
            matches!(
                edge,
                Some((crate::model::ConstraintLine::ImageEdge { image: i, edge: crate::model::ImageEdge::Left }, _))
                    if i == image
            ),
            "constraint-tool line pick should take the left image edge, got {:?}",
            edge
        );
    }

    /// #425: once a sketch sits on the image's plane the two reference points stay
    /// first-class constraint vertices.
    #[test]
    fn calibration_endpoints_are_pickable_in_a_hosted_sketch() {
        use crate::model::{default_image_calibration, ConstraintPoint};
        let (mut doc, _sketch) = doc_with_plane_sketch();
        let mut img = tracing_image_on_xy((-200.0, -200.0), 150.0, 150.0);
        img.calibration = Some(default_image_calibration(150.0));
        let image = doc.tracing_images.insert(img);
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cursor = Pos2::new(-125.0, -50.0);
        let target = resolve_pick_target(
            cursor,
            &project,
            Some(Vec3::new(-125.0, -50.0, 0.0)),
            &doc,
            None,
        )
        .expect("something under the cursor");
        assert!(
            matches!(
                &target.kind,
                PickTargetKind::Point(ConstraintPoint::ImageCalibrationPoint { image: i, index: 0 })
                    if *i == image
            ),
            "a hosted sketch should keep the calibration point pickable, got {:?}",
            target.kind
        );
    }

    /// #1561: on the overlap of an image and its host plane, the image is the thing
    /// you're pointing at.
    #[test]
    fn a_tracing_image_beats_its_coplanar_host_plane() {
        let mut doc = Document::default();
        // Covers the XY datum (5..105) plus some extra so the overlap is unambiguous.
        let image = doc
            .tracing_images
            .insert(tracing_image_on_xy((0.0, 0.0), 100.0, 100.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let target = resolve_pick_target(
            Pos2::new(50.0, 50.0),
            &project,
            Some(Vec3::new(50.0, 50.0, 0.0)),
            &doc,
            None,
        )
        .expect("something under the cursor");
        assert_eq!(
            scene_element_from_pick(&target.kind),
            Some(SceneElement::Image(image)),
            "image should win over its host plane, got {:?}",
            target.kind
        );
    }

    /// #1562 (pick): a construction plane that is nearer the eye than the image still
    /// wins — you can click the standing plane in front of the picture.
    #[test]
    fn a_front_plane_beats_a_buried_tracing_image() {
        let mut doc = Document::default();
        let _image = doc
            .tracing_images
            .insert(tracing_image_on_xy((0.0, 0.0), 100.0, 100.0));
        let mut front = crate::face::default_xy_plane();
        front.origin = Vec3::new(0.0, 0.0, 20.0);
        front.extent = crate::model::PlaneExtent {
            u_min: 0.0,
            u_max: 100.0,
            v_min: 0.0,
            v_max: 100.0,
        };
        front.name = Some("Front".to_string());
        let front_key = doc.construction_planes.insert(front);

        let project = |p: Vec3| Some(egui::pos2(p.x, p.y));
        let eye = Vec3::new(50.0, 50.0, 200.0);
        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);
        let t = resolve_pick_target(
            egui::pos2(50.0, 50.0),
            &project,
            Some(Vec3::new(50.0, 50.0, 0.0)),
            &doc,
            Some(&occ),
        )
        .expect("a plane or image under the cursor");
        assert!(
            matches!(t.kind, PickTargetKind::ConstructionPlane(i) if i == front_key),
            "front plane (z=20) must win over the image on z=0, got {:?}",
            t.kind
        );
    }

    /// #1466/#1467: sketches on a moved-with-adds body must not remesh the kernel on
    /// every hover/orbit frame. After a warmup, pick is a cache hit — do not assert a
    /// wall-clock budget; debug Linux CI can miss a 2 ms cutoff even when the cache works.
    #[test]
    fn issue_1466_hover_pick_does_not_remesh_every_frame() {
        for fixture in [
            include_bytes!("../tests/fixtures/issue_1466.json").as_slice(),
            include_bytes!("../tests/fixtures/issue_1467.json").as_slice(),
        ] {
            let mut doc = crate::storage::from_json_bytes(fixture).expect("load");
            doc.bump_mesh_rev();
            let visibility = crate::hierarchy::ElementVisibility::default();
            let eye = Vec3::new(200.0, 200.0, 200.0);
            let occ = PickOcclusion::new(&doc, &visibility, eye);
            let project = |w: Vec3| Some(Pos2::new(w.x + w.z * 0.3, w.y + w.z * 0.2));
            let live = doc
                .bodies
                .iter()
                .find(|(_, b)| !b.shadow)
                .map(|(bi, _)| bi)
                .expect("live body");
            let probe = {
                let mesh = crate::extrude::body_solid_mesh(&doc, live).expect("mesh");
                let tri = mesh.triangles.first().expect("tri");
                let c = (tri[0] + tri[1] + tri[2]) / 3.0;
                project(c).expect("project")
            };
            let _ = crate::face::pick_sketch_face(probe, &project, &doc, eye);
            let _ = resolve_pick_target(probe, &project, Some(Vec3::ZERO), &doc, Some(&occ));

            crate::extrude::FACE_KEY_MESH_BUILDS.with(|c| c.set(0));
            crate::extrude::reset_mesh_cache_stats();
            for _ in 0..40 {
                let _ = crate::face::pick_sketch_face(probe, &project, &doc, eye);
                let _ = resolve_pick_target(probe, &project, Some(Vec3::ZERO), &doc, Some(&occ));
            }
            let rebuilds = crate::extrude::FACE_KEY_MESH_BUILDS.with(|c| c.get());
            let stats = crate::extrude::mesh_cache_stats();
            assert_eq!(
                rebuilds, 0,
                "hover/orbit pick must reuse the cached pre-add solid, rebuilt {rebuilds} times"
            );
            assert_eq!(
                stats.misses, 0,
                "hover/orbit pick must not remesh OCCT bodies, cache {stats:?}"
            );
            assert!(
                stats.hits > 0,
                "hover/orbit pick should hit the body mesh cache, cache {stats:?}"
            );
        }
    }
}