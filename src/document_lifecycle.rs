//! Tombstone deletion: entities are marked deleted but keep their indices so references stay valid.

use crate::hierarchy::SceneElement;
use crate::model::{
    ConstraintEntity, ConstraintKind, ConstraintLine, ConstraintPoint, DistanceTarget, Document,
    FaceId, ShapeKind, SketchId,
};
use crate::selection::SceneSelection;
use std::collections::HashSet;

pub fn sketch_alive(doc: &Document, sketch: SketchId) -> bool {
    doc.sketches.get(sketch).is_some_and(|s| !s.deleted)
}

pub fn line_alive(doc: &Document, index: usize) -> bool {
    doc.lines.get(index).is_some_and(|l| !l.deleted)
}

pub fn circle_alive(doc: &Document, index: usize) -> bool {
    doc.circles.get(index).is_some_and(|c| !c.deleted)
}

pub fn constraint_alive(doc: &Document, index: usize) -> bool {
    doc.constraints.get(index).is_some_and(|c| !c.deleted)
}

pub fn construction_plane_alive(doc: &Document, index: usize) -> bool {
    doc.construction_planes
        .get(index)
        .is_some_and(|p| !p.deleted)
}

/// Whether a scene element is present and not tombstoned.
pub fn element_alive(doc: &Document, element: SceneElement) -> bool {
    match element {
        // A drawing item lives as long as its page does; the page's own bookkeeping
        // renumbers or drops what's on it (#967).
        SceneElement::DrawingElement { drawing, .. } => doc
            .drawings
            .get(drawing)
            .is_some_and(|d| !d.deleted),
        SceneElement::ConstructionPlane(index) => construction_plane_alive(doc, index),
        SceneElement::Sketch(sketch) => sketch_alive(doc, sketch),
        SceneElement::Line(index) => line_alive(doc, index),
        SceneElement::Circle(index) => circle_alive(doc, index),
        SceneElement::Point(point) => point_owner_alive(doc, &point),
        SceneElement::Constraint(index) => constraint_alive(doc, index),
        SceneElement::Extrusion(index) => extrusion_alive(doc, index),
        SceneElement::Body(index) => body_alive(doc, index),
        SceneElement::FaceEdge(line) => constraint_line_alive(doc, &line),
        // The origin and the world axes always exist (#189/#952).
        SceneElement::Origin | SceneElement::GlobalAxis(_) => true,
        // Geometry-keyed 3D sub-elements (#156): alive as long as their body is (an exact
        // edge/vertex existence check would need a mesh rebuild; a stale selection is
        // harmless because it simply stops matching anything).
        SceneElement::BodyEdge { body, .. }
        | SceneElement::BodyVertex { body, .. }
        | SceneElement::BodyFace { body, .. }
        | SceneElement::BodyCylinder { body, .. }
        | SceneElement::BodyAxis { body, .. } => body_alive(doc, body),
        // An analytic face (#952) is alive while its plane still resolves — the same check the
        // geometry code makes before using one.
        SceneElement::SketchFace(face) => crate::face::sketch_frame(doc, face).is_some(),
        // A Move/Joint snap point (#952) lives as long as the body it sits on, like the other
        // geometry-keyed sub-elements; the world origin always does.
        // An extrusion's analytic edge (#952) lives as long as its extrusion.
        SceneElement::ExtrusionEdge { extrusion, .. } => extrusion_alive(doc, extrusion),
        // A repeat instance's face (#955) lives as long as both the repeat and the source face.
        SceneElement::RepeatedFace { face, op, .. } => {
            doc.repeat_ops.contains(op)
                && crate::face::sketch_frame(doc, face).is_some()
        }
        SceneElement::MovePoint(point) => match point {
            crate::model::MovePointRef::Origin => true,
            crate::model::MovePointRef::Vertex { body, .. }
            | crate::model::MovePointRef::EdgeMidpoint { body, .. }
            | crate::model::MovePointRef::OnEdge { body, .. }
            | crate::model::MovePointRef::FaceCenter { body, .. } => body_alive(doc, body),
        },
        SceneElement::UnitInstance(index) => doc
            .unit_instances
            .get(index)
            .is_some_and(|i| !i.deleted),
        SceneElement::Component(index) => doc
            .components
            .get(index)
            .is_some_and(|c| !c.deleted),
        SceneElement::BooleanOp(index) => doc.boolean_ops.contains(index),
        SceneElement::MoveOp(index) => doc.move_ops.contains(index),
        SceneElement::MirrorOp(index) => doc.mirror_ops.contains(index),
        SceneElement::RepeatOp(index) => doc.repeat_ops.contains(index),
        SceneElement::SketchOffsetOp(index) => doc
            .sketch_offset_ops
            .get(index)
            .is_some_and(|op| !op.deleted),
        SceneElement::SketchMirrorOp(index) => doc
            .sketch_mirror_ops
            .get(index)
            .is_some_and(|op| !op.deleted),
        SceneElement::SketchVertexTreatmentOp(index) => doc
            .sketch_vertex_treatment_ops
            .get(index)
            .is_some_and(|op| !op.deleted),
        SceneElement::SketchRepeatOp(index) => doc
            .sketch_repeat_ops
            .get(index)
            .is_some_and(|op| !op.deleted),
        SceneElement::SketchSliceOp(index) => doc
            .sketch_slice_ops
            .get(index)
            .is_some_and(|op| !op.deleted),
        SceneElement::SliceOp(index) => doc
            .slice_ops
            .get(index)
            .is_some_and(|op| !op.deleted),
        SceneElement::EdgeTreatmentOp(index) => doc
            .edge_treatment_ops
            .get(index)
            .is_some_and(|op| !op.deleted),
        SceneElement::Revolution(index) => doc.revolutions.contains(index),
        SceneElement::Shape(index) => doc.primitives.contains(index),
        SceneElement::SweepOp(index) => doc.sweeps.contains(index),
        SceneElement::Image(index) => doc.tracing_images.contains(index),
        SceneElement::SketchText(index) => doc
            .sketch_texts
            .get(index)
            .is_some_and(|t| !t.deleted),
        SceneElement::Joint(index) => doc
            .joints
            .get(index)
            .is_some_and(|j| !j.deleted),
    }
}

pub fn extrusion_alive(doc: &Document, index: usize) -> bool {
    doc.extrusions.get(index).is_some_and(|e| !e.deleted)
}

pub fn body_alive(doc: &Document, index: crate::model::BodyKey) -> bool {
    doc.bodies.contains(index)
}

fn point_owner_alive(
    doc: &Document,
    point: &crate::model::ConstraintPoint,
) -> bool {
    use crate::model::ConstraintPoint;
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => line_alive(doc, *line),
        ConstraintPoint::CircleCenter(circle) => circle_alive(doc, *circle),
        // A face's own vertex is "alive" exactly when it still resolves (extrusion present,
        // index still within its current boundary loop) — it has no owning scene entity.
        ConstraintPoint::FaceVertex { face, index } => {
            crate::extrude::face_boundary_loop_world(doc, face).is_some_and(|l| *index < l.len())
        }
        ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.get(*text).is_some_and(|t| !t.deleted)
        }
        ConstraintPoint::ImageCalibrationPoint { image, index } => doc
            .tracing_images
            .get(*image)
            .is_some_and(|i| crate::model::image_calibration_point_uv(i, *index).is_some()),
    }
}

/// Normalize a selection entry to the entity that should be tombstoned.
pub fn delete_target_for_element(element: SceneElement) -> SceneElement {
    match element {
        SceneElement::Point(point) => match point_owner_element(&point) {
            Some(owner) => owner,
            // A face's own vertex has no owning scene entity to delete instead — deleting it
            // is a no-op (it's fixed by the body, mirrors `ConstraintEntity::Origin`).
            None => SceneElement::Point(point),
        },
        other => other,
    }
}

fn point_owner_element(point: &crate::model::ConstraintPoint) -> Option<SceneElement> {
    use crate::model::ConstraintPoint;
    Some(match point {
        ConstraintPoint::LineEndpoint { line, .. } => SceneElement::Line(*line),
        ConstraintPoint::CircleCenter(circle) => SceneElement::Circle(*circle),
        ConstraintPoint::TextAnchor { text, .. } => SceneElement::SketchText(*text),
        ConstraintPoint::ImageCalibrationPoint { image, .. } => SceneElement::Image(*image),
        ConstraintPoint::FaceVertex { .. } => return None,
    })
}

/// Unique tombstone targets from the current selection (deduped).
pub fn delete_targets_from_selection(selection: &SceneSelection) -> Vec<SceneElement> {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for element in selection.iter() {
        let target = delete_target_for_element(element);
        if seen.insert(target.clone()) {
            targets.push(target);
        }
    }
    targets
}

/// Tombstone one element and any owned children. Returns true if anything changed.
pub fn tombstone_element(doc: &mut Document, element: SceneElement) -> bool {
    let mut changed = false;
    match element {
        // Deleting from a drawing page goes through the drawing's own actions (#967), which
        // renumber what's left; there is nothing to tombstone here.
        SceneElement::DrawingElement { .. } => {}
        // Deleting a component re-homes its members and child components to its parent
        // (#423) — grouping is organizational, so nothing inside is deleted.
        SceneElement::Component(index) => {
            if doc.components.get(index).is_some_and(|c| !c.deleted) {
                let parent = doc.components[index].parent;
                doc.components[index].deleted = true;
                for m in doc.component_members.iter_mut() {
                    if m.1 == index {
                        match parent {
                            Some(p) => m.1 = p,
                            None => m.1 = usize::MAX, // pruned below
                        }
                    }
                }
                doc.component_members.retain(|m| m.1 != usize::MAX);
                for c in doc.components.iter_mut() {
                    if c.parent == Some(index) {
                        c.parent = parent;
                    }
                }
                tombstone_joints_referencing(doc, crate::model::JointRef::Component(index));
                changed = true;
            }
        }
        // Deleting a unit instance removes that placement only (#723). The embedded copy
        // stays in `Document.units` even when the last instance goes: unit indices stay
        // stable and re-importing the same source stays cheap (it reuses the copy).
        SceneElement::UnitInstance(index) => {
            if doc.unit_instances.get(index).is_some_and(|i| !i.deleted) {
                doc.unit_instances[index].deleted = true;
                tombstone_joints_referencing(doc, crate::model::JointRef::UnitInstance(index));
                changed = true;
            }
        }
        SceneElement::ConstructionPlane(index) => {
            if tombstone_construction_plane(doc, index) {
                changed = true;
            }
        }
        SceneElement::Sketch(sketch) => {
            if tombstone_sketch(doc, sketch) {
                changed = true;
            }
        }
        SceneElement::Circle(index) => {
            if tombstone_circle(doc, index) {
                changed = true;
            }
        }
        SceneElement::Line(index) => {
            if tombstone_line(doc, index) {
                changed = true;
            }
        }
        SceneElement::Constraint(index) => {
            if tombstone_constraint(doc, index) {
                changed = true;
            }
        }
        SceneElement::Point(point) => {
            if let Some(owner) = point_owner_element(&point) {
                changed |= tombstone_element(doc, owner);
            }
        }
        SceneElement::Extrusion(index) => {
            if tombstone_extrusion(doc, index) {
                changed = true;
            }
        }
        SceneElement::Body(index) => {
            if tombstone_body(doc, index) {
                changed = true;
            }
        }
        // Fixed by the body's own geometry — deleting it is a no-op, same as `FaceVertex`.
        SceneElement::FaceEdge(_)
        | SceneElement::Origin
        | SceneElement::GlobalAxis(_)
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::BodyFace { .. }
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_)
        | SceneElement::ExtrusionEdge { .. }
        | SceneElement::RepeatedFace { .. } => {}
        SceneElement::Joint(index) => {
            if doc.joints.get(index).is_some_and(|j| !j.deleted) {
                doc.joints[index].deleted = true;
                remove_shape_order_entry(doc, ShapeKind::Joint, index);
                changed = true;
            }
        }
        SceneElement::RepeatOp(index) => {
            if let Some(removed) = doc.repeat_ops.remove(index) {
                {
                    let outputs = removed.outputs.clone();
                    for out in outputs {
                        doc.bodies.remove(out);
                    }
                    // Generated plane instances go with the op (#221).
                    let plane_outputs = removed.plane_outputs.clone();
                    for out in plane_outputs {
                        if let Some(plane) = doc.construction_planes.get_mut(out) {
                            plane.deleted = true;
                        }
                    }
                    // Repeated-sketch copies: their planes, sketches, and copied entities (#226).
                    let op = removed;
                    for out in &op.sketch_plane_outputs {
                        if let Some(plane) = doc.construction_planes.get_mut(*out) {
                            plane.deleted = true;
                        }
                    }
                    for &si in &op.sketch_outputs {
                        for l in doc.lines.iter_mut().filter(|l| l.sketch == si) {
                            l.deleted = true;
                        }
                        for c in doc.circles.iter_mut().filter(|c| c.sketch == si) {
                            c.deleted = true;
                        }
                        if let Some(s) = doc.sketches.get_mut(si) {
                            s.deleted = true;
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchRepeatOp(index) => {
            if let Some(op) = doc.sketch_repeat_ops.get_mut(index) {
                if !op.deleted {
                    op.deleted = true;
                    // The duplicated lines/circles go with the op (#222/#228).
                    let op = doc.sketch_repeat_ops[index].clone();
                    for &out in &op.line_outputs {
                        if let Some(l) = doc.lines.get_mut(out) {
                            l.deleted = true;
                        }
                    }
                    for &out in &op.circle_outputs {
                        if let Some(c) = doc.circles.get_mut(out) {
                            c.deleted = true;
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchOffsetOp(index) => {
            if let Some(op) = doc.sketch_offset_ops.get_mut(index) {
                if !op.deleted {
                    op.deleted = true;
                    // The parallel lines/circles go with the op.
                    let op = doc.sketch_offset_ops[index].clone();
                    for &out in &op.line_outputs {
                        if let Some(l) = doc.lines.get_mut(out) {
                            l.deleted = true;
                        }
                    }
                    for &out in &op.circle_outputs {
                        if let Some(c) = doc.circles.get_mut(out) {
                            c.deleted = true;
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchMirrorOp(index) => {
            if let Some(op) = doc.sketch_mirror_ops.get_mut(index) {
                if !op.deleted {
                    op.deleted = true;
                    // The reflected lines/circles go with the op (#523).
                    let op = doc.sketch_mirror_ops[index].clone();
                    for &out in &op.line_outputs {
                        if let Some(l) = doc.lines.get_mut(out) {
                            l.deleted = true;
                        }
                    }
                    for &out in &op.circle_outputs {
                        if let Some(c) = doc.circles.get_mut(out) {
                            c.deleted = true;
                        }
                    }
                    // The reflected corner-coincidences go with it too (#547).
                    for &ci in &op.constraint_outputs {
                        if let Some(c) = doc.constraints.get_mut(ci) {
                            c.deleted = true;
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchVertexTreatmentOp(index) => {
            if let Some(op) = doc.sketch_vertex_treatment_ops.get_mut(index) {
                if !op.deleted {
                    op.deleted = true;
                    // Deleting the chamfer/fillet (#538) un-shadows the source edges (they
                    // become live geometry again, sharp corner restored) and tombstones the
                    // generated trimmed copies, bridges, and stitch constraints.
                    let op = doc.sketch_vertex_treatment_ops[index].clone();
                    for &li in &op.line_targets {
                        if let Some(l) = doc.lines.get_mut(li) {
                            l.shadow = false;
                        }
                    }
                    for &out in op.line_outputs.iter().chain(op.bridge_outputs.iter()) {
                        if let Some(l) = doc.lines.get_mut(out) {
                            l.deleted = true;
                        }
                    }
                    for &ci in &op.constraint_outputs {
                        if let Some(c) = doc.constraints.get_mut(ci) {
                            c.deleted = true;
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchSliceOp(index) => {
            if let Some(op) = doc.sketch_slice_ops.get_mut(index) {
                if !op.deleted {
                    op.deleted = true;
                    let op = doc.sketch_slice_ops[index].clone();
                    // Un-shadow the originals and remove the fragments (#224/#229/#237).
                    for &t in &op.line_targets {
                        if let Some(l) = doc.lines.get_mut(t) {
                            l.shadow = false;
                        }
                    }
                    for &t in &op.circle_targets {
                        if let Some(c) = doc.circles.get_mut(t) {
                            c.shadow = false;
                        }
                    }
                    for &out in &op.line_outputs {
                        if let Some(l) = doc.lines.get_mut(out) {
                            l.deleted = true;
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::MoveOp(index) => {
            if let Some(op) = doc.move_ops.remove(index) {
                for &out in &op.outputs {
                    doc.bodies.remove(out);
                }
                for &input in &op.targets {
                    if !crate::model::body_shadowed_by_other_ops(doc, input, None, None, None, None)
                    {
                        if let Some(body) = doc.bodies.get_mut(input) {
                            body.shadow = false;
                        }
                    }
                }
                changed = true;
            }
        }
        SceneElement::MirrorOp(index) => {
            // A mirror keeps its inputs (never shadowed), so deleting it only removes its
            // reflected output bodies (#523).
            if let Some(op) = doc.mirror_ops.remove(index) {
                for &out in &op.outputs {
                    doc.bodies.remove(out);
                }
                changed = true;
            }
        }
        SceneElement::SliceOp(index) => {
            // Deleting the operation removes its fragments and releases its inputs from
            // shadow (unless another live operation still consumes them).
            if let Some(op) = doc.slice_ops.get_mut(index) {
                if !op.deleted {
                    op.deleted = true;
                    let op = doc.slice_ops[index].clone();
                    for &out in &op.outputs {
                        doc.bodies.remove(out);
                    }
                    for &input in &op.targets {
                        if !crate::model::body_shadowed_by_other_ops(doc, input, None, None, Some(index), None)
                        {
                            if let Some(body) = doc.bodies.get_mut(input) {
                                body.shadow = false;
                            }
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::EdgeTreatmentOp(index) => {
            // Deleting the chamfer/fillet removes its beveled outputs and releases its input
            // bodies from shadow (unless another live operation still consumes them) (#531).
            if let Some(op) = doc.edge_treatment_ops.get_mut(index) {
                if !op.deleted {
                    op.deleted = true;
                    let op = doc.edge_treatment_ops[index].clone();
                    for &out in &op.outputs {
                        doc.bodies.remove(out);
                    }
                    for &input in &op.targets {
                        if !crate::model::body_shadowed_by_other_ops(
                            doc,
                            input,
                            None,
                            None,
                            None,
                            Some(index),
                        ) {
                            if let Some(body) = doc.bodies.get_mut(input) {
                                body.shadow = false;
                            }
                        }
                    }
                    changed = true;
                }
            }
        }
        SceneElement::Revolution(index) => {
            // Deleting the revolution removes its output body (only NewBody mode has one;
            // AddTo/Cut fuse into existing bodies at recompute, so there's nothing else to
            // release — the revolve simply stops contributing).
            if doc.revolutions.remove(index).is_some() {
                let produced: Vec<crate::model::BodyKey> = doc
                    .bodies
                    .iter()
                    .filter(|(_, b)| b.source == crate::model::BodySource::Revolve(index))
                    .map(|(k, _)| k)
                    .collect();
                for key in produced {
                    doc.bodies.remove(key);
                }
                changed = true;
            }
        }
        SceneElement::Shape(index) => {
            // Deleting a shape (#909) takes its body with it.
            if doc.primitives.remove(index).is_some() {
                let produced: Vec<crate::model::BodyKey> = doc
                    .bodies
                    .iter()
                    .filter(|(_, b)| b.source == crate::model::BodySource::Primitive(index))
                    .map(|(k, _)| k)
                    .collect();
                for key in produced {
                    doc.bodies.remove(key);
                }
                changed = true;
            }
        }
        SceneElement::SweepOp(index) => {
            // Deleting the sweep removes its output body (only NewBody mode has one;
            // AddTo/Cut fuse into existing bodies at recompute).
            if doc.sweeps.remove(index).is_some() {
                let produced: Vec<crate::model::BodyKey> = doc
                    .bodies
                    .iter()
                    .filter(|(_, b)| b.source == crate::model::BodySource::Sweep(index))
                    .map(|(k, _)| k)
                    .collect();
                for key in produced {
                    doc.bodies.remove(key);
                }
                changed = true;
            }
        }
        SceneElement::BooleanOp(index) => {
            // Deleting the operation removes its outputs and releases its inputs from
            // shadow (unless another live operation still consumes them).
            if let Some(op) = doc.boolean_ops.remove(index) {
                for &out in &op.outputs {
                    doc.bodies.remove(out);
                }
                for &input in op.a.iter().chain(op.b.iter()) {
                    if !crate::model::body_shadowed_by_other_ops(doc, input, None, None, None, None)
                    {
                        if let Some(body) = doc.bodies.get_mut(input) {
                            body.shadow = false;
                        }
                    }
                }
                changed = true;
            }
        }
        // A tracing image is removed outright (#1055): its key stops resolving, so the
        // calibration constraints and move targets that named it read as gone rather than
        // sliding onto whichever image took its place.
        SceneElement::Image(index) => {
            changed = doc.tracing_images.remove(index).is_some();
        }
        SceneElement::SketchText(index) => {
            if let Some(t) = doc.sketch_texts.get_mut(index) {
                if !t.deleted {
                    t.deleted = true;
                    changed = true;
                }
            }
        }
    }
    changed
}

fn tombstone_extrusion(doc: &mut Document, index: usize) -> bool {
    let Some(extrusion) = doc.extrusions.get_mut(index) else {
        return false;
    };
    if extrusion.deleted {
        return false;
    }
    extrusion.deleted = true;
    remove_shape_order_entry(doc, ShapeKind::Extrusion, index);
    // A body that depends solely on this extrusion is removed with it; a body merging this
    // extrusion with others (#32) just drops this one and keeps the rest.
    let dependent: Vec<crate::model::BodyKey> = doc
        .bodies
        .iter()
        .filter(|(_, body)| body.source.owns_extrusion(index))
        .map(|(i, _)| i)
        .collect();
    for bi in dependent {
        let solely_owned = doc.bodies[bi].source.extrusion_indices() == [index];
        if solely_owned {
            tombstone_body(doc, bi);
        } else {
            doc.bodies[bi].source.remove_extrusion(index);
        }
    }
    true
}

fn tombstone_body(doc: &mut Document, index: crate::model::BodyKey) -> bool {
    // The history-tape marker to drop is the one for this body's place among the live ones,
    // read before the removal (#1055).
    let Some(ordinal) = doc.bodies.keys().position(|k| k == index) else {
        return false;
    };
    doc.bodies.remove(index);
    remove_shape_order_entry(doc, ShapeKind::Body, ordinal);
    tombstone_joints_referencing(doc, crate::model::JointRef::Body(index));
    true
}

/// A joint dies with any of the things it joins (#891): tombstone every live joint that
/// holds `member`.
fn tombstone_joints_referencing(doc: &mut Document, member: crate::model::JointRef) -> bool {
    let mut changed = false;
    for ji in 0..doc.joints.len() {
        if doc.joints[ji].deleted || !doc.joints[ji].members.contains(&member) {
            continue;
        }
        doc.joints[ji].deleted = true;
        remove_shape_order_entry(doc, ShapeKind::Joint, ji);
        changed = true;
    }
    changed
}

/// Tombstone every target in `elements`.
pub fn tombstone_elements(doc: &mut Document, elements: &[SceneElement]) -> usize {
    let mut count = 0usize;
    for element in elements {
        if tombstone_element(doc, element.clone()) {
            count += 1;
        }
    }
    count
}

fn tombstone_construction_plane(doc: &mut Document, index: usize) -> bool {
    let Some(plane) = doc.construction_planes.get_mut(index) else {
        return false;
    };
    if plane.deleted {
        return false;
    }
    plane.deleted = true;
    remove_shape_order_entry(doc, ShapeKind::ConstructionPlane, index);
    let face = FaceId::ConstructionPlane(index);
    for sketch in doc.sketches_on_face(face).collect::<Vec<_>>() {
        tombstone_sketch(doc, sketch);
    }
    true
}

fn tombstone_sketch(doc: &mut Document, sketch: SketchId) -> bool {
    let Some(entry) = doc.sketches.get_mut(sketch) else {
        return false;
    };
    if entry.deleted {
        return false;
    }
    entry.deleted = true;
    remove_shape_order_entry(doc, ShapeKind::Sketch, sketch);

    let lines: Vec<usize> = doc
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.sketch == sketch && !line.deleted)
        .map(|(i, _)| i)
        .collect();
    for li in lines {
        tombstone_line(doc, li);
    }
    let circles: Vec<usize> = doc
        .circles
        .iter()
        .enumerate()
        .filter(|(_, circle)| circle.sketch == sketch && !circle.deleted)
        .map(|(i, _)| i)
        .collect();
    for ci in circles {
        tombstone_circle(doc, ci);
    }
    let constraints: Vec<usize> = doc
        .constraints
        .iter()
        .enumerate()
        .filter(|(_, c)| c.sketch == sketch && !c.deleted)
        .map(|(i, _)| i)
        .collect();
    for ci in constraints {
        tombstone_constraint(doc, ci);
    }
    let planes: Vec<usize> = doc
        .construction_planes
        .iter()
        .enumerate()
        .filter(|(_, plane)| {
            matches!(plane.parent, crate::model::ConstructionPlaneParent::Sketch(s) if s == sketch)
                && !plane.deleted
        })
        .map(|(i, _)| i)
        .collect();
    for pi in planes {
        tombstone_construction_plane(doc, pi);
    }
    true
}

fn tombstone_circle(doc: &mut Document, index: usize) -> bool {
    let Some(circle) = doc.circles.get_mut(index) else {
        return false;
    };
    if circle.deleted {
        return false;
    }
    circle.deleted = true;
    remove_shape_order_entry(doc, ShapeKind::Circle, index);
    let face = FaceId::Circle(index);
    for sketch in doc.sketches_on_face(face).collect::<Vec<_>>() {
        tombstone_sketch(doc, sketch);
    }
    true
}

fn tombstone_line(doc: &mut Document, index: usize) -> bool {
    let Some(line) = doc.lines.get_mut(index) else {
        return false;
    };
    if line.deleted {
        return false;
    }
    line.deleted = true;
    remove_shape_order_entry(doc, ShapeKind::Line, index);
    // #502: detach from parametric ops that would re-create this line on recompute
    // (offset/repeat rebuild overwrites `deleted: false` for live outputs).
    detach_line_from_sketch_ops(doc, index);
    true
}

/// When a line is deleted, drop it from any sketch offset/repeat target or output
/// lists so rebuild does not revive it (#502). Empties the op if it has no sources left.
fn detach_line_from_sketch_ops(doc: &mut Document, line: usize) {
    let mut orphan_outputs: Vec<usize> = Vec::new();
    let mut empty_ops: Vec<usize> = Vec::new();
    for (oi, op) in doc.sketch_offset_ops.iter_mut().enumerate() {
        if op.deleted {
            continue;
        }
        // Parallel target/output slots: drop whichever end references this line.
        let mut i = 0;
        while i < op.line_targets.len() {
            let is_target = op.line_targets[i] == line;
            let is_output = op.line_outputs.get(i).copied() == Some(line);
            if is_target || is_output {
                // Deleting a source also tombstones its generated output.
                if is_target {
                    if let Some(&out) = op.line_outputs.get(i) {
                        if out != line {
                            orphan_outputs.push(out);
                        }
                    }
                }
                op.line_targets.remove(i);
                if i < op.line_outputs.len() {
                    op.line_outputs.remove(i);
                }
            } else {
                i += 1;
            }
        }
        op.line_outputs.retain(|&out| out != line);
        if op.line_targets.is_empty() && op.circle_targets.is_empty() {
            empty_ops.push(oi);
        }
    }
    for out in orphan_outputs {
        if let Some(l) = doc.lines.get_mut(out) {
            if !l.deleted {
                l.deleted = true;
                remove_shape_order_entry(doc, ShapeKind::Line, out);
            }
        }
    }
    for oi in empty_ops {
        if let Some(op) = doc.sketch_offset_ops.get_mut(oi) {
            if !op.deleted {
                op.deleted = true;
                remove_shape_order_entry(doc, ShapeKind::SketchOffsetOperation, oi);
            }
        }
    }
    for op in &mut doc.sketch_repeat_ops {
        if op.deleted {
            continue;
        }
        op.line_targets.retain(|&t| t != line);
        op.line_outputs.retain(|&out| out != line);
    }
}

fn tombstone_constraint(doc: &mut Document, index: usize) -> bool {
    let Some(constraint) = doc.constraints.get_mut(index) else {
        return false;
    };
    if constraint.deleted {
        return false;
    }
    constraint.deleted = true;
    remove_shape_order_entry(doc, ShapeKind::Constraint, index);
    true
}

/// Remove a parameter (used by `DeleteParameter` and selection delete). Its name is free
/// again the moment it goes, because it is actually gone (#1055).
pub fn tombstone_parameter(doc: &mut Document, index: crate::model::ParameterKey) -> bool {
    // The history-tape marker to drop is the one for this parameter's place among the live
    // ones, read before the removal.
    let Some(ordinal) = doc.parameters.keys().position(|k| k == index) else {
        return false;
    };
    doc.parameters.remove(index);
    remove_shape_order_entry(doc, ShapeKind::Parameter, ordinal);
    true
}

pub fn distance_target_alive(doc: &Document, target: &DistanceTarget) -> bool {
    match target {
        DistanceTarget::LineLength(index) => line_alive(doc, *index),
        DistanceTarget::CircleDiameter(index) => circle_alive(doc, *index),
        DistanceTarget::LineLineDistance {
            line_a,
            line_b,
            side: _,
        } => constraint_line_alive(doc, line_a) && constraint_line_alive(doc, line_b),
        DistanceTarget::PointPointDistance { anchor, mover, .. } => {
            constraint_point_alive(doc, anchor) && constraint_point_alive(doc, mover)
        }
        DistanceTarget::PointLineDistance { point, line, .. } => {
            constraint_point_alive(doc, point) && constraint_line_alive(doc, line)
        }
    }
}

pub fn constraint_line_alive(doc: &Document, line: &ConstraintLine) -> bool {
    match line {
        ConstraintLine::Line(index) => line_alive(doc, *index),
        ConstraintLine::FaceEdge { face, index } => {
            crate::extrude::face_boundary_loop_world(doc, face).is_some_and(|l| *index < l.len())
        }
        // The origin axes always exist (#189).
        ConstraintLine::OriginAxis(_) => true,
    }
}

pub fn constraint_entity_alive(doc: &Document, entity: &ConstraintEntity) -> bool {
    match entity {
        ConstraintEntity::Point(point) => constraint_point_alive(doc, point),
        ConstraintEntity::Line(line) => constraint_line_alive(doc, line),
        ConstraintEntity::Circle(circle) => circle_alive(doc, *circle),
        ConstraintEntity::Origin => true,
    }
}

pub fn constraint_point_alive(doc: &Document, point: &ConstraintPoint) -> bool {
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => line_alive(doc, *line),
        ConstraintPoint::CircleCenter(circle) => circle_alive(doc, *circle),
        ConstraintPoint::FaceVertex { face, index } => {
            crate::extrude::face_boundary_loop_world(doc, face).is_some_and(|l| *index < l.len())
        }
        ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.get(*text).is_some_and(|t| !t.deleted)
        }
        ConstraintPoint::ImageCalibrationPoint { image, index } => doc
            .tracing_images
            .get(*image)
            .is_some_and(|i| crate::model::image_calibration_point_uv(i, *index).is_some()),
    }
}

/// Whether a constraint can still be applied (all referenced geometry is alive).
pub fn constraint_kind_applicable(doc: &Document, kind: &ConstraintKind) -> bool {
    match kind {
        ConstraintKind::Distance { target } => distance_target_alive(doc, target),
        ConstraintKind::Parallel { line_a, line_b }
        | ConstraintKind::Perpendicular { line_a, line_b }
        | ConstraintKind::Equal { line_a, line_b } => {
            constraint_line_alive(doc, line_a) && constraint_line_alive(doc, line_b)
        }
        ConstraintKind::Coincident { a, b } => {
            constraint_entity_alive(doc, a) && constraint_entity_alive(doc, b)
        }
        ConstraintKind::Midpoint { point, line } => {
            constraint_point_alive(doc, point) && constraint_line_alive(doc, line)
        }
        ConstraintKind::Angle {
            line_a,
            line_b,
            rotation_sign: _,
        } => constraint_line_alive(doc, line_a) && constraint_line_alive(doc, line_b),
        ConstraintKind::Tangent { a, b } => {
            constraint_point_alive(doc, a) && constraint_point_alive(doc, b)
        }
    }
}

fn remove_shape_order_entry(doc: &mut Document, kind: ShapeKind, ordinal: usize) {
    if let Some(pos) = doc
        .shape_order
        .iter()
        .enumerate()
        .filter(|(_, k)| **k == kind)
        .nth(ordinal)
        .map(|(i, _)| i)
    {
        doc.shape_order.remove(pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Constraint, ConstraintKind, ConstraintLine, Document, Line};

    /// #1055: deleting a tracing image removes it rather than tombstoning it, and the
    /// image beside it keeps its identity — the whole reason positional identity had to go.
    #[test]
    fn deleting_a_tracing_image_removes_it_and_leaves_its_neighbour_alone() {
        let image = |name: &str| crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: name.to_string(),
            plane: 0,
            origin: (0.0, 0.0),
            base_origin: None,
            width_mm: 10.0,
            height_mm: 10.0,
            name: None,
            calibration: None,
        };
        let mut doc = Document::default();
        let first = doc.tracing_images.insert(image("first"));
        let second = doc.tracing_images.insert(image("second"));

        assert!(tombstone_element(&mut doc, SceneElement::Image(first)));
        assert_eq!(doc.tracing_images.len(), 1, "gone, not marked");
        assert_eq!(
            doc.tracing_images.get(second).map(|i| i.source_name.as_str()),
            Some("second"),
            "the survivor did not slide into the hole"
        );
        assert!(!element_alive(&doc, SceneElement::Image(first)));
        assert!(
            !tombstone_element(&mut doc, SceneElement::Image(first)),
            "deleting it twice changes nothing"
        );
    }

    fn push_test_body(doc: &mut Document) -> crate::model::BodyKey {
        let key = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            material: None,
            shadow: false,
        });
        doc.shape_order.push(ShapeKind::Body);
        key
    }

    fn push_test_joint(doc: &mut Document, members: Vec<crate::model::JointRef>) -> usize {
        doc.joints.push(crate::model::Joint {
            members,
            base: 0,
            kind: crate::model::JointKind::Revolute,
            mate: Default::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: Default::default(),
            name: None,
            deleted: false,
        });
        doc.shape_order.push(ShapeKind::Joint);
        doc.joints.len() - 1
    }

    /// #891: deleting a joint tombstones it and removes its shape-order entry; nothing it
    /// joins is touched.
    #[test]
    fn tombstone_joint_leaves_members_alone() {
        let mut doc = Document::default();
        let a = push_test_body(&mut doc);
        let b = push_test_body(&mut doc);
        let ji = push_test_joint(
            &mut doc,
            vec![crate::model::JointRef::Body(a), crate::model::JointRef::Body(b)],
        );
        let order_len = doc.shape_order.len();
        assert!(tombstone_element(&mut doc, SceneElement::Joint(ji)));
        assert!(doc.joints[ji].deleted);
        assert!(!element_alive(&doc, SceneElement::Joint(ji)));
        assert!(body_alive(&doc, a));
        assert!(body_alive(&doc, b));
        assert_eq!(doc.shape_order.len(), order_len - 1);
        // Already dead: a second delete is a no-op.
        assert!(!tombstone_element(&mut doc, SceneElement::Joint(ji)));
    }

    /// #891: a joint dies with either of the things it joins — here a member body.
    #[test]
    fn joint_dies_with_its_member_body() {
        let mut doc = Document::default();
        let a = push_test_body(&mut doc);
        let b = push_test_body(&mut doc);
        let ji = push_test_joint(
            &mut doc,
            vec![crate::model::JointRef::Body(a), crate::model::JointRef::Body(b)],
        );
        assert!(tombstone_element(&mut doc, SceneElement::Body(a)));
        assert!(doc.joints[ji].deleted, "joint must die with its member body");
        assert!(body_alive(&doc, b));
    }

    /// #891: a joint on a unit instance dies when that placement is deleted.
    #[test]
    fn joint_dies_with_its_unit_instance() {
        let mut doc = Document::default();
        let a = push_test_body(&mut doc);
        doc.units.push(crate::model::ImportedUnit {
            source: crate::model::UnitSource::RelativePath("x.bearcad".to_string()),
            link: Default::default(),
            document: Document::default(),
            source_mtime: None,
            source_hash: None,
        });
        doc.unit_instances.push(crate::model::UnitInstance {
            unit: 0,
            name: None,
            parameter_overrides: Vec::new(),
            placement: Default::default(),
            deleted: false,
        });
        let ji = push_test_joint(
            &mut doc,
            vec![
                crate::model::JointRef::Body(a),
                crate::model::JointRef::UnitInstance(0),
            ],
        );
        assert!(tombstone_element(&mut doc, SceneElement::UnitInstance(0)));
        assert!(doc.joints[ji].deleted, "joint must die with its unit instance");
    }

    fn sketch_with_two_lines() -> (Document, SketchId, usize, usize) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        let line_a = 0;
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        let line_b = 1;
        (doc, sketch, line_a, line_b)
    }

    #[test]
    fn tombstone_line_preserves_index_for_constraint_refs() {
        let (mut doc, sketch, line_a, line_b) = sketch_with_two_lines();
        doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(line_a),
                line_b: ConstraintLine::Line(line_b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        doc.shape_order.push(ShapeKind::Constraint);
        assert!(tombstone_line(&mut doc, line_a));
        assert!(doc.lines[line_a].deleted);
        assert!(!line_alive(&doc, line_a));
        assert!(line_alive(&doc, line_b));
        assert_eq!(doc.lines.len(), 2);
        let constraint = &doc.constraints[0];
        assert!(matches!(
            constraint.kind,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(0),
                ..
            }
        ));
    }

    #[test]
    fn tombstone_elements_counts_unique_targets() {
        let (mut doc, _, line_a, line_b) = sketch_with_two_lines();
        let count = tombstone_elements(
            &mut doc,
            &[
                SceneElement::Line(line_a),
                SceneElement::Line(line_b),
            ],
        );
        assert_eq!(count, 2);
        assert!(doc.lines[line_a].deleted);
        assert!(doc.lines[line_b].deleted);
    }
}