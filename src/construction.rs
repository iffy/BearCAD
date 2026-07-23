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
    PlaneAnchor, PlaneDefinition, SketchId,
};
use crate::value::{eval_length_mm, parse_length_or};
use eframe::egui;
use glam::{Quat, Vec3};
/// Shared stroke/fill colour for all construction geometry.
pub const CONSTRUCTION_RGBA: egui::Color32 = egui::Color32::from_rgb(230, 120, 40);
/// Brighter yellow fill for construction planes (semi-transparent in the viewport).
pub const PLANE_FILL_RGBA: egui::Color32 = egui::Color32::from_rgb(241, 196, 15);

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
        | PickTargetKind::Circle(_) => PlaneAnchorSource::Axis,
        PickTargetKind::BodyFace { .. }
        | PickTargetKind::ConstructionPlane(_)
        | PickTargetKind::Ground(_) => PlaneAnchorSource::Face,
        // A constraint badge is never a plane anchor (it only reaches the exploder, #568); classify
        // it as a point so the arm is total.
        PickTargetKind::Constraint(_) => PlaneAnchorSource::Point,
    }
}

/// Whether a sketch line is a curve (has bezier handles), not a straight edge.
///
/// Straight edges alone are a complete plane-anchor set (line *in* the plane). A curve
/// alone is not — it needs a point for the line+point set (#483): plane through the
/// point, normal along the curve (tangent at an endpoint).
pub fn sketch_line_is_curve(doc: &Document, line_index: usize) -> bool {
    doc.lines
        .get(line_index)
        .is_some_and(|l| !l.deleted && l.bezier.is_some())
}

/// Outward world-space tangent of `line_index` at `point` when the point (or a
/// coincidence partner) is an endpoint of that line — the same direction #474 uses.
/// `None` when the point is not on this line's ends.
pub fn line_outward_tangent_at_point(
    doc: &Document,
    line_index: usize,
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
    line_index: usize,
    end: crate::model::LineEnd,
) -> Option<Vec3> {
    let line = doc.lines.get(line_index)?;
    if line.deleted {
        return None;
    }
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
    line_index: Option<usize>,
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
    axis_line: Option<usize>,
    anchor_point: Option<&crate::model::ConstraintPoint>,
    next_kind: &PickTargetKind,
    next_reference: &PlaneReference,
) -> Option<(PlaneReference, PlaneAnchorSource, Vec<String>, Option<usize>, Option<crate::model::ConstraintPoint>)>
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
pub fn descendant_plane_indices(doc: &Document, root_plane: usize) -> Vec<usize> {
    let mut descendants = Vec::new();
    let mut faces = vec![FaceId::ConstructionPlane(root_plane)];
    let mut seen_faces = std::collections::HashSet::new();

    while let Some(face) = faces.pop() {
        if !seen_faces.insert(face.clone()) {
            continue;
        }
        for sketch in doc.sketches_on_face(face) {
            for (pi, plane) in doc.construction_planes.iter().enumerate() {
                if matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
                    descendants.push(pi);
                    faces.push(FaceId::ConstructionPlane(pi));
                }
            }
            for (ci, circle) in doc.circles.iter().enumerate() {
                if circle.sketch == sketch {
                    faces.push(FaceId::Circle(ci));
                }
            }
        }
    }

    descendants
}

/// Faces hosted on or nested under sketches on `root_plane` (including the root plane).
pub fn descendant_faces(doc: &Document, root_plane: usize) -> Vec<FaceId> {
    let mut faces = vec![FaceId::ConstructionPlane(root_plane)];
    let mut seen_faces = std::collections::HashSet::new();
    let mut collected = Vec::new();

    while let Some(face) = faces.pop() {
        if !seen_faces.insert(face.clone()) {
            continue;
        }
        collected.push(face.clone());
        for sketch in doc.sketches_on_face(face) {
            for (pi, plane) in doc.construction_planes.iter().enumerate() {
                if matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
                    faces.push(FaceId::ConstructionPlane(pi));
                }
            }
            for (ci, circle) in doc.circles.iter().enumerate() {
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
    pub planes: Vec<(usize, ConstructionPlane)>,
    pub lines: Vec<(Vec3, Vec3)>,
}

/// Where dependent planes and hosted sketch geometry will land after `preview_plane` is committed.
pub fn preview_plane_edit_dependents(
    doc: &Document,
    plane_index: usize,
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
        for line in &doc.lines {
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
    plane_index: usize,
    definition: &PlaneDefinition,
    parent: ConstructionPlaneParent,
) -> Result<(), String> {
    if doc.construction_planes.get(plane_index).is_none() {
        return Err(format!("Unknown construction plane {plane_index}"));
    }

    let old_frame = sketch_frame(doc, FaceId::ConstructionPlane(plane_index))
        .ok_or_else(|| format!("Construction plane {plane_index} has no sketch frame"))?;
    let descendants = descendant_plane_indices(doc, plane_index);

    let plane = plane_from_definition(definition, parent);
    doc.construction_planes[plane_index] = plane;

    let new_frame = sketch_frame(doc, FaceId::ConstructionPlane(plane_index))
        .ok_or_else(|| format!("Construction plane {plane_index} has no sketch frame"))?;

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
        deleted: false,
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
        deleted: false,
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
        PickTargetKind::BodyEdge { .. }
        | PickTargetKind::BodyFace { .. }
        | PickTargetKind::BodyVertex { .. }
        | PickTargetKind::GlobalAxis(_)
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
        ConstraintPoint::ImageCalibrationPoint { .. } => None,
        // A face's own vertex has no owning sketch of its own — it's referenced *from*
        // whichever sketch a constraint projects it into, not owned by one.
        ConstraintPoint::FaceVertex { .. } => None,
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

/// Corners of the visible plane quad in world space.
pub fn plane_corners(plane: &ConstructionPlane, half: f32) -> [Vec3; 4] {
    let o = plane.origin;
    let u = plane.u_axis * half;
    let v = plane.v_axis * half;
    [
        o - u - v,
        o + u - v,
        o + u + v,
        o - u + v,
    ]
}

/// Live offset for a face reference from a world-space hover point.
#[cfg(test)]
mod pick_path_tests {
    use super::*;
    use crate::model::{Document, FaceId, Line};

    /// #459 diagnosis: the full press-path pick — project a line endpoint through a
    /// real camera and ask the picker for it at that exact screen position.
    #[test]
    fn endpoint_picks_at_its_projected_position() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, -30.0, -20.0, 30.0, 20.0));
        let cam = crate::camera::Camera::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        let (a, _b) = crate::face::line_world_endpoints(&doc, &doc.lines[0]).unwrap();
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

/// Which axis gizmo handle is under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisGizmoHit {
    Offset,
    Angle,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    let tangent = axis.cross(handle_dir).normalize_or_zero();
    let angle_color = if angle_hovered {
        GIZMO_HANDLE_HOVER_RGBA
    } else {
        color
    };
    if let Some(sp) = project(handle) {
        if angle_hovered {
            draw_gizmo_handle_hover(painter, sp, GIZMO_HANDLE_HOVER_RGBA);
        } else {
            painter.circle_filled(sp, 6.0, color);
        }
        if let (Some(ta), Some(tb)) = (
            project(handle + tangent * 6.0),
            project(handle - tangent * 6.0),
        ) {
            let t_screen = (ta - tb).normalized();
            if t_screen.length_sq() > 1e-4 {
                for sign in [-1.0f32, 1.0] {
                    draw_gizmo_arrow_2d(painter, sp, t_screen * sign, 12.0, 5.0, 3.0, angle_color);
                }
            }
        }
    }
}

/// Which geometry would be selected at a viewport position.
#[derive(Clone, Debug, PartialEq)]
pub enum PickTargetKind {
    /// A sketch point (line endpoint, rect corner, or circle center).
    Point(ConstraintPoint),
    /// A standalone sketch line segment.
    Line(usize),
    /// A sketch circle (picked on its perimeter).
    Circle(usize),
    /// One feature edge of a 3D body's solid mesh (#31) — a mesh boundary or crease between
    /// two non-coplanar triangles, the same edges `ShadingMode::Wireframe` draws, extracted via
    /// `solid_mesh_unique_edges`. Works for any body (extrusion-sourced or STL/STEP-imported),
    /// since it's derived from the triangle mesh rather than an analytic profile.
    BodyEdge {
        body: usize,
        a: Vec3,
        b: Vec3,
    },
    /// A planar face of a 3D body's solid mesh (#144): the maximal edge-connected group of
    /// coplanar triangles under the cursor (see `solid_mesh_coplanar_faces`), in world space.
    /// Lets any face of any body — extrusion-sourced, boolean-cut, or imported — be hover-
    /// highlighted and referenced in 3D. `normal` orients the highlight fill toward the camera.
    BodyFace {
        body: usize,
        triangles: Vec<[Vec3; 3]>,
        normal: Vec3,
    },
    /// A vertex (corner) of a 3D body's solid mesh (#144), for 3D hover/selection.
    BodyVertex {
        body: usize,
        position: Vec3,
    },
    GlobalAxis(GlobalAxis),
    ConstructionPlane(usize),
    Ground(Vec3),
    /// A sketch constraint's annotation icon (#568), by its index into `Document::constraints`.
    /// Constraints have no world geometry of their own — the icon is a screen-space glyph placed
    /// near the geometry it governs — so this is only ever produced for the Selection Exploder
    /// crowd (never by `resolve_pick_target`), letting a constraint icon buried under overlapping
    /// geometry be fanned out and selected like anything else.
    Constraint(usize),
}

/// A resolved pick target with its plane reference and screen-space distance.
#[derive(Clone, Debug, PartialEq)]
pub struct PickTarget {
    pub kind: PickTargetKind,
    pub reference: PlaneReference,
    distance_px: f32,
    priority: u8,
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
}

impl PickOcclusion {
    /// The camera eye this occlusion context was built with — used to depth-sort face picks (#565).
    pub fn eye(&self) -> Vec3 {
        self.eye
    }

    pub fn new(doc: &Document, visibility: &crate::hierarchy::ElementVisibility, eye: Vec3) -> Self {
        let meshes = doc
            .bodies
            .iter()
            .enumerate()
            .filter(|(bi, body)| {
                // Shadow bodies neither render nor occlude/catch picks.
                !body.deleted
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
                    | ConstraintPoint::ImageCalibrationPoint { .. } => false,
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
            | PickTargetKind::BodyVertex { body, .. } => {
                doc.bodies.get(*body).is_some_and(|b| !b.shadow)
                    && vis.effective_visible(doc, SceneElement::Body(*body))
            }
            PickTargetKind::ConstructionPlane(i) => {
                vis.effective_visible(doc, SceneElement::ConstructionPlane(*i))
            }
            // A constraint badge is pickable when it is visible (its icon is only drawn for visible
            // constraints anyway, #568).
            PickTargetKind::Constraint(i) => {
                vis.effective_visible(doc, SceneElement::Constraint(*i))
            }
            PickTargetKind::GlobalAxis(_) | PickTargetKind::Ground(_) => true,
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
                priority: 0,
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
                priority: 0,
            });
        }
    }

    if let Some((kind, a, b, label, dist)) = nearest_body_edge(screen, project, doc) {
        if pickable(&kind) && visible(segment_point_nearest_screen(screen, project, a, b)) {
            consider(PickTarget {
                kind,
                reference: PlaneReference::Axis {
                    origin: segment_midpoint(a, b),
                    direction: segment_direction(a, b),
                    label,
                },
                distance_px: dist,
                priority: 0,
            });
        }
    }

    // A body **face** is selectable too (#565), but only where no edge/vertex is under the cursor:
    // it's ranked below them (priority 1 vs 0), so clicking near an edge still picks the edge and
    // clicking the face interior picks the face. Needs the camera eye to pick the front-most face,
    // so it's only offered when an occlusion context (which carries the eye) is present.
    if let Some(occ) = occlusion {
        if let Some(kind) = crate::face::pick_body_face(screen, project, doc, occ.eye()) {
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
                        priority: 1,
                    });
                }
            }
        }
    }

    if let Some((axis, dist)) = nearest_global_axis(screen, project) {
        consider(PickTarget {
            kind: PickTargetKind::GlobalAxis(axis),
            reference: PlaneReference::Axis {
                origin: Vec3::ZERO,
                direction: axis.direction(),
                label: axis.label().to_string(),
            },
            distance_px: dist,
            priority: 0,
        });
    }

    if let Some((index, dist)) = nearest_construction_plane(screen, project, &doc.construction_planes)
    {
        let plane = &doc.construction_planes[index];
        let origin = ground_point.unwrap_or(plane.origin);
        let projected = project_point_on_plane(origin, plane);
        if pickable(&PickTargetKind::ConstructionPlane(index)) {
        consider(PickTarget {
            kind: PickTargetKind::ConstructionPlane(index),
            reference: PlaneReference::Face {
                origin: projected,
                normal: plane.normal,
                label: "Construction plane".to_string(),
            },
            distance_px: dist,
            priority: 2,
        });
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
            priority: 3,
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
        priority: 1,
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
    if base.as_ref().is_some_and(|t| t.priority == 0) {
        return base;
    }
    body_face_pick_target(screen, project, doc, eye, occlusion).or(base)
}

impl PickTarget {
    fn beats(&self, other: &PickTarget) -> bool {
        if self.priority != other.priority {
            return self.priority < other.priority;
        }
        // A vertex within its (fixed-radius) pick zone beats an edge that merely passes under the
        // cursor, so hovering near a corner selects the corner, not the edge through it (#242).
        // Only vertex-vs-edge is reordered here; every other same-priority pair still goes by
        // pixel distance.
        let is_vertex = |k: &PickTargetKind| {
            matches!(k, PickTargetKind::Point(_) | PickTargetKind::BodyVertex { .. })
        };
        let is_edge = |k: &PickTargetKind| {
            matches!(
                k,
                PickTargetKind::Line(_) | PickTargetKind::Circle(_) | PickTargetKind::BodyEdge { .. }
            )
        };
        if is_vertex(&self.kind) && is_edge(&other.kind) {
            return true;
        }
        if is_edge(&self.kind) && is_vertex(&other.kind) {
            return false;
        }
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
    let mut ends: Vec<(usize, crate::model::LineEnd)> =
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
        if line.deleted {
            continue;
        }
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
            .unwrap_or_else(|| format!("line {li}"));
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
        PickTargetKind::Constraint(index) => Some(SceneElement::Constraint(*index)),
        _ => None,
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
        PickTargetKind::BodyEdge { a, b, .. } => {
            draw_segment_highlight(painter, project, a, b, color);
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
        PickTargetKind::ConstructionPlane(index) => {
            if let Some(plane) = doc.construction_planes.get(index) {
                draw_plane_face_highlight(painter, project, plane, color);
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
    let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
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
        // Already a world-space point (#26/#27) — no sketch frame to project through.
        ConstraintPoint::FaceVertex { face, index } => {
            crate::extrude::face_boundary_loop_world(doc, &face)?.get(index).copied()
        }
        ConstraintPoint::TextAnchor { text, anchor } => {
            let entity = doc.sketch_texts.get(text).filter(|t| !t.deleted)?;
            let frame = sketch_geometry_frame(doc, entity.sketch)?;
            let (u, v) = crate::text::sketch_text_anchor_uv(entity, anchor);
            Some(local_to_world(&frame, u, v))
        }
        ConstraintPoint::ImageCalibrationPoint { image, index } => {
            let img = doc.tracing_images.get(image).filter(|i| !i.deleted)?;
            let (u, v) = crate::model::image_calibration_point_uv(img, index)?;
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

    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted || line.sketch != sketch {
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

    for (ci, circle) in doc.circles.iter().enumerate() {
        if circle.deleted || circle.sketch != sketch {
            continue;
        }
        if let Some(center) = crate::face::circle_world_center(doc, circle) {
            consider(ConstraintPoint::CircleCenter(ci), center);
        }
    }

    // A text's nine anchor points (#408) are constrainable vertices too.
    for (ti, text) in doc.sketch_texts.iter().enumerate() {
        if text.deleted || text.sketch != sketch {
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
    // A calibrated image's two reference points (#425), for images on this sketch's plane.
    for (ii, img) in doc.tracing_images.iter().enumerate() {
        if img.deleted
            || doc.sketch_face(sketch) != Some(FaceId::ConstructionPlane(img.plane))
        {
            continue;
        }
        if let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) {
            for index in 0..2 {
                if let Some((u, v)) = crate::model::image_calibration_point_uv(img, index) {
                    consider(
                        ConstraintPoint::ImageCalibrationPoint { image: ii, index },
                        crate::face::local_to_world(&frame, u, v),
                    );
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

    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted || line.sketch != sketch {
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

    // The origin axes (#189) are pickable everywhere as fixed reference lines, so a point or
    // line can be constrained onto one from the constraint tool (not only by snapping).
    // Measured as an **infinite line in screen space** from two nearby projected points
    // (#394): the old ±10 m segment endpoints usually fail to project (behind the camera /
    // outside the frustum), which silently made the axes unpickable and unhoverable.
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
            if dist <= crate::touch::hit(LINE_PICK_RADIUS_PX)
                && best.as_ref().is_none_or(|(_, best_d)| dist < *best_d)
            {
                best = Some((ConstraintLine::OriginAxis(axis), dist));
            }
        };
        consider_axis(crate::model::SketchAxis::X, frame.u_axis);
        consider_axis(crate::model::SketchAxis::Y, frame.v_axis);
    }

    best
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

    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted {
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

    for (ci, circle) in doc.circles.iter().enumerate() {
        if circle.deleted {
            continue;
        }
        if let Some(center) = crate::face::circle_world_center(doc, circle) {
            consider(ConstraintPoint::CircleCenter(ci), center);
        }
    }

    // A text's nine anchor points (#408): pickable like any vertex, so the constraint tool
    // can hold a text's corner or centre to other geometry.
    for (ti, text) in doc.sketch_texts.iter().enumerate() {
        if text.deleted {
            continue;
        }
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
    // A calibrated image's two reference points (#425).
    for (ii, img) in doc.tracing_images.iter().enumerate() {
        if img.deleted {
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

    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted {
            continue;
        }
        let Some(points) = line_world_polyline(doc, line) else {
            continue;
        };
        for pair in points.windows(2) {
            consider(PickTargetKind::Line(li), pair[0], pair[1], "Line");
        }
    }

    for (ci, circle) in doc.circles.iter().enumerate() {
        if circle.deleted {
            continue;
        }
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

/// Nearest feature edge of any 3D body's solid mesh (#31) — lets a construction plane be
/// referenced from any edge on any shape, not just 2D sketch geometry.
fn nearest_body_edge(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
) -> Option<(PickTargetKind, Vec3, Vec3, String, f32)> {
    let mut best: Option<(PickTargetKind, Vec3, Vec3, String, f32)> = None;

    let mut consider = |kind: PickTargetKind, a: Vec3, b: Vec3| {
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            return;
        };
        if best.as_ref().is_none_or(|(_, _, _, _, d)| dist < *d) {
            best = Some((kind, a, b, "Body edge".to_string(), dist));
        }
    };

    for (bi, body) in doc.bodies.iter().enumerate() {
        if body.deleted || body.shadow {
            continue;
        }
        let Some(solid) = crate::extrude::body_solid_mesh(doc, bi) else {
            continue;
        };
        for (a, b) in crate::gpu_viewport::solid_mesh_unique_edges(&solid) {
            consider(PickTargetKind::BodyEdge { body: bi, a, b }, a, b);
        }
    }

    best
}

/// Nearest solid-mesh vertex (#144) of any 3D body within the point pick radius, for 3D
/// hover/selection — so any corner of any body (extrusion-sourced or imported) can be picked.
pub fn nearest_body_vertex(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
) -> Option<(PickTargetKind, f32)> {
    let mut best: Option<(PickTargetKind, f32)> = None;
    for (bi, body) in doc.bodies.iter().enumerate() {
        if body.deleted || body.shadow {
            continue;
        }
        let Some(solid) = crate::extrude::body_solid_mesh(doc, bi) else {
            continue;
        };
        for tri in &solid.triangles {
            for &p in tri {
                let Some(sp) = project(p) else {
                    continue;
                };
                let dist = (screen - sp).length();
                if dist <= crate::touch::hit(POINT_PICK_RADIUS_PX) && best.as_ref().is_none_or(|(_, d)| dist < *d) {
                    best = Some((PickTargetKind::BodyVertex { body: bi, position: p }, dist));
                }
            }
        }
    }
    best
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
/// now maps to a `SceneElement::BodyFace` keyed by its quantized centroid+normal, so two distinct
/// faces of the same body get two distinct keys (and two loupes) rather than collapsing to one.
fn crowd_key(kind: &PickTargetKind) -> String {
    match scene_element_from_pick(kind) {
        Some(el) => format!("{el:?}"),
        None => match kind {
            PickTargetKind::ConstructionPlane(i) => format!("plane:{i}"),
            other => format!("{other:?}"),
        },
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
    // Retained for call-site symmetry with `resolve_pick_target`; the crowd no longer
    // depth-orders faces by eye distance (it enumerates every near face, #556).
    _eye: Vec3,
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
    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted {
            continue;
        }
        let Some((a, b)) = line_world_endpoints(doc, line) else {
            continue;
        };
        push_point(&mut raw, ConstraintPoint::LineEndpoint { line: li, end: LineEnd::Start }, a);
        push_point(&mut raw, ConstraintPoint::LineEndpoint { line: li, end: LineEnd::End }, b);
    }
    for (ci, circle) in doc.circles.iter().enumerate() {
        if circle.deleted {
            continue;
        }
        if let Some(center) = crate::face::circle_world_center(doc, circle) {
            push_point(&mut raw, ConstraintPoint::CircleCenter(ci), center);
        }
    }
    for (ti, text) in doc.sketch_texts.iter().enumerate() {
        if text.deleted {
            continue;
        }
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
    for (ii, img) in doc.tracing_images.iter().enumerate() {
        if img.deleted {
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
    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted {
            continue;
        }
        if let Some(points) = line_world_polyline(doc, line) {
            for pair in points.windows(2) {
                push_edge(&mut raw, PickTargetKind::Line(li), pair[0], pair[1]);
            }
        }
    }
    for (ci, circle) in doc.circles.iter().enumerate() {
        if circle.deleted {
            continue;
        }
        if let Some(pts) = crate::face::circle_world_perimeter(doc, circle, 32) {
            for w in pts.windows(2) {
                push_edge(&mut raw, PickTargetKind::Circle(ci), w[0], w[1]);
            }
        }
    }
    for (bi, body) in doc.bodies.iter().enumerate() {
        if body.deleted || body.shadow {
            continue;
        }
        let Some(solid) = crate::extrude::body_solid_mesh(doc, bi) else {
            continue;
        };
        for (a, b) in crate::gpu_viewport::solid_mesh_unique_edges(&solid) {
            push_edge(&mut raw, PickTargetKind::BodyEdge { body: bi, a, b }, a, b);
        }
        for tri in &solid.triangles {
            for &p in tri {
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

    // Every body face near the cursor (#555/#556): not just the nearest ray-hit face, but every
    // face — front and back — whose projected area is within the pick radius, so a narrow face
    // seen edge-on (a thin sliver between its two edges) and buried back faces both get loupes.
    for (kind, centroid, dist) in crate::face::body_faces_near(screen, project, doc, point_r) {
        if pickable(&kind) {
            raw.push((kind, centroid, dist));
        }
    }

    // Dedupe per distinct thing (keeping the nearest touch), then order nearest-first.
    let mut best: std::collections::HashMap<String, (PickTargetKind, Vec3, f32)> =
        std::collections::HashMap::new();
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
    out.sort_by(|a, b| a.dist_px.partial_cmp(&b.dist_px).unwrap_or(std::cmp::Ordering::Equal));
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

/// Nearest currently-treatable analytic extrusion edge (#77): the chamfer/fillet tool's own
/// picking path when no sketch is open, used instead of the generic [`nearest_body_edge`]
/// (mesh-feature-edge) picking above since it needs the structured `ExtrusionEdgeRef`, not just
/// two raw points — see `crate::extrude::treatable_edges`.
pub fn nearest_treatable_edge(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    doc: &Document,
) -> Option<(usize, crate::model::ExtrusionEdgeRef, Vec3, Vec3, f32)> {
    let mut best: Option<(usize, crate::model::ExtrusionEdgeRef, Vec3, Vec3, f32)> = None;
    for (extrusion, edge, a, b) in crate::extrude::treatable_edges(doc) {
        let Some(dist) = segment_pick_distance(screen, project, a, b) else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, _, _, _, d)| dist < *d) {
            best = Some((extrusion, edge, a, b, dist));
        }
    }
    best
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

fn nearest_construction_plane(
    screen: egui::Pos2,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    planes: &[ConstructionPlane],
) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    for (index, plane) in planes.iter().enumerate().rev() {
        let corners = plane_corners(plane, PLANE_DISPLAY_HALF);
        let pts: Option<Vec<egui::Pos2>> = corners.iter().map(|&c| project(c)).collect();
        let Some(pts) = pts else { continue };
        let quad = [pts[0], pts[1], pts[2], pts[3]];
        let dist = if point_in_screen_quad(screen, quad) {
            0.0
        } else {
            dist_point_to_quad_edges(screen, quad)
        };
        if dist <= FACE_PICK_MARGIN_PX {
            if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                best = Some((index, dist));
            }
        }
    }
    best
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
pub fn add_line_polygon(doc: &mut Document, sketch: SketchId, points: &[(f32, f32)]) -> Vec<usize> {
    use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ShapeKind};
    let n = points.len();
    let base = doc.lines.len();
    for i in 0..n {
        let (u0, v0) = points[i];
        let (u1, v1) = points[(i + 1) % n];
        doc.lines.push(Line::from_local_endpoints(sketch, u0, v0, u1, v1));
        doc.shape_order.push(ShapeKind::Line);
    }
    let idx: Vec<usize> = (base..base + n).collect();
    for i in 0..n {
        doc.constraints.push(Constraint {
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
            deleted: false,
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
) -> [usize; 4] {
    use crate::model::{
        Constraint, ConstraintEntity, ConstraintKind, ConstraintLine, ShapeKind,
    };
    let corners = [
        (x, y),
        (x + w, y),
        (x + w, y + h),
        (x, y + h),
    ];
    let base = doc.lines.len();
    for i in 0..4 {
        let (u0, v0) = corners[i];
        let (u1, v1) = corners[(i + 1) % 4];
        let mut line = Line::from_local_endpoints(sketch, u0, v0, u1, v1);
        line.construction = construction_edges[i];
        doc.lines.push(line);
        doc.shape_order.push(ShapeKind::Line);
    }
    let idx = [base, base + 1, base + 2, base + 3];
    let mut push = |kind: ConstraintKind| {
        doc.constraints.push(Constraint {
            sketch,
            kind,
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
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

    #[test]
    fn plane_corners_are_centered_on_origin() {
        let plane = plane_from_face(0.0, Vec3::new(10.0, 20.0, 0.0), Vec3::Z);
        let corners = plane_corners(&plane, 10.0);
        let center = corners.iter().fold(Vec3::ZERO, |acc, c| acc + *c) / 4.0;
        assert!((center.x - 10.0).abs() < 1e-3);
        assert!((center.y - 20.0).abs() < 1e-3);
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

    /// #551: unlike `resolve_pick_target` (which keeps only the nearest), `collect_pick_candidates`
    /// returns the whole crowd within the hitbox — every endpoint and edge under the cursor — so
    /// the Selection Exploder can fan them out. Deduped per element and ordered nearest-first.
    #[test]
    fn collect_pick_candidates_returns_the_whole_crowd() {
        use crate::model::{ConstraintPoint, Line, LineEnd};
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0)); // line 0
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 0.0, 10.0)); // line 1
        // XY-plane sketch → world (x, y, 0); project drops z. The cursor sits on the shared corner.
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let cands = collect_pick_candidates(Pos2::new(0.0, 0.0), &project, &doc, Vec3::ZERO, None);
        let kinds: Vec<&PickTargetKind> = cands.iter().map(|c| &c.kind).collect();
        // Both segments and both coincident start endpoints are within the hitbox.
        assert!(kinds.iter().any(|k| matches!(k, PickTargetKind::Line(0))), "{kinds:?}");
        assert!(kinds.iter().any(|k| matches!(k, PickTargetKind::Line(1))), "{kinds:?}");
        assert!(kinds.iter().any(|k| matches!(
            k,
            PickTargetKind::Point(ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::Start })
        )));
        assert!(kinds.iter().any(|k| matches!(
            k,
            PickTargetKind::Point(ConstraintPoint::LineEndpoint { line: 1, end: LineEnd::Start })
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
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        assert!(
            collect_pick_candidates(Pos2::new(500.0, 500.0), &project, &doc, Vec3::ZERO, None)
                .is_empty()
        );
    }

    fn doc_with_plane_sketch() -> (Document, usize) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        (doc, sketch)
    }

    #[test]
    fn parent_from_line_pick_is_owning_sketch() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines = vec![Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0)];
        assert_eq!(
            parent_from_pick_target(&doc, PickTargetKind::Line(0)),
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
            &PickTargetKind::Point(crate::model::ConstraintPoint::CircleCenter(0)),
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
            &PickTargetKind::Line(1),
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
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        // Start (6,4), near handle (6,12) → outward at start is -Y (away from handle).
        // Mid-segment direction along the chord is roughly +X/+Y — not the end tangent.
        let mut curve = Line::from_local_endpoints(sketch, 6.0, 4.0, 26.0, 14.0);
        curve.bezier = Some([(6.0, 12.0), (18.0, 14.0)]);
        doc.lines.push(curve);

        let mid_segment_dir = Vec3::new(1.0, 0.5, 0.0).normalize();
        let axis = PlaneReference::Axis {
            origin: Vec3::new(16.0, 9.0, 0.0),
            direction: mid_segment_dir,
            label: "Curve".to_string(),
        };
        let point = ConstraintPoint::LineEndpoint {
            line: 0,
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
            Some(0),
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
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut curve = Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 10.0);
        curve.bezier = Some([(0.0, 5.0), (5.0, 10.0)]);
        doc.lines.push(curve);
        assert!(!sketch_line_is_curve(&doc, 0));
        assert!(sketch_line_is_curve(&doc, 1));
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
            &PickTargetKind::Line(0),
            &line_ref,
        )
        .is_none());
    }

    #[test]
    fn vertex_normal_candidates_follow_line_and_curve_tangents() {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, Line, LineEnd};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        // A straight line along +X and a curve leaving the shared vertex along +Y
        // (its near handle sits at (10, 5) above the vertex (10, 0)).
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut curve = Line::from_local_endpoints(sketch, 10.0, 0.0, 20.0, 10.0);
        curve.bezier = Some([(10.0, 5.0), (15.0, 10.0)]);
        doc.lines.push(curve);
        doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 0,
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });

        let candidates = vertex_normal_candidates(
            &doc,
            &ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::End },
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
            &ConstraintPoint::LineEndpoint { line: 1, end: LineEnd::End },
        );
        assert_eq!(solo.len(), 1);
        // Outward at the curve's end = away from its near handle (15,10) -> (20,10): +X.
        assert!((solo[0].1 - Vec3::X).length() < 1e-4, "{:?}", solo[0]);
    }

    #[test]
    fn pick_reference_prefers_line_over_ground() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines = vec![Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 0.0)];
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let reference = resolve_pick_target(Pos2::new(50.0, 2.0), &project, Some(Vec3::ZERO), &doc, None)
            .map(|t| t.reference);
        assert!(matches!(reference, Some(PlaneReference::Axis { .. })));
    }

    #[test]
    fn nearest_treatable_edge_finds_circle_cap_rims() {
        use crate::actions::{Action, AppState, Tool};
        use crate::model::{Circle, ExtrudeFace, ExtrusionEdgeRef, FaceId};

        let mut state = AppState::default();
        state.apply(Action::BeginSketch { face: FaceId::ConstructionPlane(0), viewport: None });
        let sketch = state.sketch_session.unwrap().sketch;
        state.doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        state.doc.shape_order.push(crate::model::ShapeKind::Circle);
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace { face: ExtrudeFace::Circle(0) });
        state.apply(Action::SetExtrudeDistance { distance: 6.0 });
        state.apply(Action::CommitExtrusion);

        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let hit = nearest_treatable_edge(Pos2::new(5.0, 0.0), &project, &state.doc);
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

    fn doc_with_imported_triangle_body() -> Document {
        let mut doc = Document::default();
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles: vec![[
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(0.0, 10.0, 0.0),
            ]],
            source_name: "tri".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
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
            PickTargetKind::BodyVertex { body: 0, position } if (position - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4
        ));
    }

    #[test]
    fn nearest_body_vertex_misses_when_cursor_far_from_any_corner() {
        let doc = doc_with_imported_triangle_body();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        assert!(nearest_body_vertex(Pos2::new(50.0, 50.0), &project, &doc).is_none());
    }

    /// #155: with an occlusion context, a line hidden behind a visible body is not pickable;
    /// without one (or with the body hidden), it still is.
    #[test]
    fn occluded_line_is_not_picked() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines = vec![Line::from_local_endpoints(sketch, 20.0, 40.0, 60.0, 40.0)];
        // A blocker body (imported soup, so no kernel needed): its top face at z = 10
        // stands between the eye (z = +100) and the line (z = 0).
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let triangles = vec![
            [c(0.0, 0.0, 10.0), c(80.0, 0.0, 10.0), c(80.0, 80.0, 10.0)],
            [c(0.0, 0.0, 10.0), c(80.0, 80.0, 10.0), c(0.0, 80.0, 10.0)],
        ];
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles,
            source_name: "blocker".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
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
        assert!(matches!(picked.map(|t| t.kind), Some(PickTargetKind::Line(0))));

        // Hiding the body restores pickability: an invisible body must not occlude.
        let mut visibility = crate::hierarchy::ElementVisibility::default();
        visibility.set_visible(crate::hierarchy::SceneElement::Body(0), false);
        let occ = PickOcclusion::new(&doc, &visibility, eye);
        let picked = resolve_pick_target(cursor, &project, None, &doc, Some(&occ));
        assert!(matches!(picked.map(|t| t.kind), Some(PickTargetKind::Line(0))));
    }

    /// #258: a hidden or shadow sketch line is neither selectable nor hoverable — it drops out
    /// of the pick candidates whenever a visibility/occlusion context is present.
    #[test]
    fn hidden_or_shadow_line_is_not_picked() {
        let (mut doc, sketch) = doc_with_plane_sketch();
        doc.lines = vec![Line::from_local_endpoints(sketch, 20.0, 40.0, 60.0, 40.0)];
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let eye = Vec3::new(40.0, 40.0, 100.0);
        let cursor = Pos2::new(40.0, 40.0);

        // Visible: the line is picked.
        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);
        assert!(matches!(
            resolve_pick_target(cursor, &project, None, &doc, Some(&occ)).map(|t| t.kind),
            Some(PickTargetKind::Line(0))
        ));

        // Hiding its sketch makes the line (and its endpoints) effectively hidden → not picked.
        let mut vis = crate::hierarchy::ElementVisibility::default();
        vis.set_visible(crate::hierarchy::SceneElement::Sketch(sketch), false);
        let occ = PickOcclusion::new(&doc, &vis, eye);
        assert!(
            !matches!(
                resolve_pick_target(cursor, &project, None, &doc, Some(&occ)).map(|t| t.kind),
                Some(PickTargetKind::Line(0)) | Some(PickTargetKind::Point(_))
            ),
            "a hidden line and its endpoints must not be picked"
        );

        // A shadow line is not picked even while visible.
        doc.lines[0].shadow = true;
        let vis = crate::hierarchy::ElementVisibility::default();
        let occ = PickOcclusion::new(&doc, &vis, eye);
        assert!(
            !matches!(
                resolve_pick_target(cursor, &project, None, &doc, Some(&occ)).map(|t| t.kind),
                Some(PickTargetKind::Line(0)) | Some(PickTargetKind::Point(_))
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
            scene_element_from_pick(&PickTargetKind::Constraint(4)),
            Some(SceneElement::Constraint(4))
        );
    }

    #[test]
    fn body_edge_and_vertex_picks_become_selectable_elements() {
        use crate::hierarchy::SceneElement;

        let a = Vec3::new(0.0, 0.0, 10.0);
        let b = Vec3::new(80.0, 0.0, 10.0);
        let forward = scene_element_from_pick(&PickTargetKind::BodyEdge { body: 0, a, b });
        let backward = scene_element_from_pick(&PickTargetKind::BodyEdge { body: 0, a: b, b: a });
        assert!(matches!(forward, Some(SceneElement::BodyEdge { body: 0, .. })));
        assert_eq!(forward, backward, "edge identity must not depend on direction");

        let vertex =
            scene_element_from_pick(&PickTargetKind::BodyVertex { body: 2, position: a });
        assert!(matches!(vertex, Some(SceneElement::BodyVertex { body: 2, .. })));

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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let lines = add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.extrusions.push(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(lines.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            deleted: false,
            edge_treatments: Vec::new(),
        });
        doc.bodies.push(Body {
            source: BodySource::Extrusion(0),
            name: None,
            deleted: false,
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
            body: 3,
            triangles: triangles.clone(),
            normal,
        });
        assert!(matches!(a, Some(SceneElement::BodyFace { body: 3, .. })));
        // Same face, triangles listed in a different order → same centroid/normal → equal key.
        let mut reordered = triangles.clone();
        reordered.reverse();
        let b = scene_element_from_pick(&PickTargetKind::BodyFace {
            body: 3,
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
            body: 3,
            triangles: lower,
            normal: -Vec3::Z,
        });
        assert_ne!(a, c, "parallel faces at different depths must be distinct");
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

    /// #555: a narrow face seen edge-on (its projected area a thin sliver / line between its two
    /// edges) is caught by `body_faces_near` via edge distance, where a strict inside-triangle
    /// ray hit would miss it. Looking down -Z, the x=0 side face collapses to the line x=0.
    #[test]
    fn body_faces_near_catches_edge_on_narrow_face() {
        let doc = box_body_doc();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        // Cursor on the projected line of the x=0 side face (which has zero projected area).
        let near = crate::face::body_faces_near(Pos2::new(0.0, 5.0), &project, &doc, 12.0);
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
        doc.lines = vec![Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 0.0)];
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
        doc.lines = vec![Line::from_local_endpoints(sketch, 50.0, 50.0, 150.0, 50.0)];
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
        doc.lines = vec![Line::from_local_endpoints(sketch, 100.0, 50.0, 200.0, 50.0)];
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let target = resolve_pick_target(Pos2::new(100.0, 59.0), &project, None, &doc, None);
        assert!(matches!(
            target.map(|t| t.kind),
            Some(PickTargetKind::Point(ConstraintPoint::LineEndpoint {
                line: 0,
                end: LineEnd::Start,
            }))
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
        assert_eq!(target.kind, PickTargetKind::ConstructionPlane(0));
        assert!(matches!(target.reference, PlaneReference::Face { .. }));
    }

    #[test]
    fn pick_reference_uses_ground_when_empty() {
        let doc = Document::default();
        let project = |w: Vec3| Some(Pos2::new(w.x, w.y));
        let reference = resolve_pick_target(
            Pos2::new(80.0, 80.0),
            &project,
            Some(Vec3::new(80.0, 80.0, 0.0)),
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
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
        doc.construction_planes.push(child);
        let child_origin_before = doc.construction_planes[1].origin.z;

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
            0,
            &definition,
            ConstructionPlaneParent::Root,
        )
        .unwrap();

        let child_origin_after = doc.construction_planes[1].origin.z;
        assert!((child_origin_after - child_origin_before - 15.0).abs() < 1e-3);
    }

    // ---- Rectangle-as-four-lines (#66) ----

    #[test]
    fn add_line_rectangle_drops_four_lines_axis_parallel_and_coincident_constraints() {
        use crate::model::{ConstraintKind, ConstraintLine, Document, FaceId, SketchAxis};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let lines = add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        // Four plain lines forming a closed loop (bottom, right, top, left).
        assert_eq!(doc.lines.len(), 4);
        assert_eq!(lines, [0, 1, 2, 3]);
        // #577: the edges are constrained parallel to the sketch axes (X for bottom/top, Y for
        // left/right) rather than the old Horizontal/Vertical constraints.
        let parallel_to = |axis: SketchAxis| {
            doc.constraints
                .iter()
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
            .iter()
            .filter(|c| matches!(c.kind, ConstraintKind::Coincident { .. }))
            .count();
        assert_eq!(coincident, 4, "four shared corners join the loop");
        // Bottom edge (0) is parallel to X; right edge (1) parallel to Y.
        assert!(doc.constraints.iter().any(|c| matches!(
            &c.kind,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(0),
                line_b: ConstraintLine::OriginAxis(SketchAxis::X)
            }
        )));
        assert!(doc.constraints.iter().any(|c| matches!(
            &c.kind,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(1),
                line_b: ConstraintLine::OriginAxis(SketchAxis::Y)
            }
        )));
    }

    #[test]
    fn add_line_rectangle_forms_a_recognized_polygon_face() {
        use crate::model::{Document, FaceId};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        let loops = crate::polygon::closed_line_loops(&doc, sketch);
        assert_eq!(loops.len(), 1, "the four lines are one closed loop");
        let mut sorted = loops[0].clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3]);
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
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles,
            source_name: "box".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
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
                PickTargetKind::Ground(_) | PickTargetKind::ConstructionPlane(0)
            ),
            "ground fallback, got {:?}",
            target.kind
        );
    }

    #[test]
    fn typed_width_height_drive_the_rectangle_under_solving() {
        use crate::constraints::{add_distance_constraint, solve_document_constraints};
        use crate::model::{DistanceTarget, Document, FaceId};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
}