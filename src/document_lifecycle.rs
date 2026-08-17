//! Tombstone deletion: entities are marked deleted but keep their indices so references stay valid.

use crate::hierarchy::SceneElement;
use crate::model::{
    ConstraintEntity, ConstraintKind, ConstraintLine, ConstraintPoint, DistanceTarget, Document,
    FaceId, ShapeKind, SketchId,
};
use crate::selection::SceneSelection;
use std::collections::HashSet;

pub fn sketch_alive(doc: &Document, sketch: SketchId) -> bool {
    doc.sketches.contains(sketch)
}

pub fn line_alive(doc: &Document, index: crate::model::LineKey) -> bool {
    doc.lines.contains(index)
}

pub fn circle_alive(doc: &Document, index: crate::model::CircleKey) -> bool {
    doc.circles.contains(index)
}

pub fn constraint_alive(doc: &Document, index: crate::model::ConstraintKey) -> bool {
    doc.constraints.contains(index)
}

pub fn construction_plane_alive(
    doc: &Document,
    index: crate::model::ConstructionPlaneKey,
) -> bool {
    doc.construction_planes.contains(index)
}

/// Whether a scene element is still present in the document.
pub fn element_alive(doc: &Document, element: SceneElement) -> bool {
    match element {
        // A drawing item lives as long as its page does; the page's own bookkeeping
        // renumbers or drops what's on it (#967).
        SceneElement::DrawingElement { drawing, .. } => doc
            .drawings
            .get(drawing)
            .is_some(),
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
        SceneElement::PrimitiveEdge { primitive, .. } => doc.primitives.contains(primitive),
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
            | crate::model::MovePointRef::OnFace { body, .. } => body_alive(doc, body),
        },
        SceneElement::UnitInstance(index) => doc.unit_instances.contains(index),
        SceneElement::Component(index) => doc.components.contains(index),
        SceneElement::BooleanOp(index) => doc.boolean_ops.contains(index),
        SceneElement::MoveOp(index) => doc.move_ops.contains(index),
        SceneElement::MirrorOp(index) => doc.mirror_ops.contains(index),
        SceneElement::RepeatOp(index) => doc.repeat_ops.contains(index),
        SceneElement::SketchOffsetOp(index) => doc.sketch_offset_ops.contains(index),
        SceneElement::SketchMirrorOp(index) => doc.sketch_mirror_ops.contains(index),
        SceneElement::SketchVertexTreatmentOp(index) => doc.sketch_vertex_treatment_ops.contains(index),
        SceneElement::SketchRepeatOp(index) => doc.sketch_repeat_ops.contains(index),
        SceneElement::SketchSliceOp(index) => doc.sketch_slice_ops.contains(index),
        SceneElement::SliceOp(index) => doc.slice_ops.contains(index),
        SceneElement::ShellOp(index) => doc.shell_ops.contains(index),
        SceneElement::EdgeTreatmentOp(index) => doc.edge_treatment_ops.contains(index),
        SceneElement::Revolution(index) => doc.revolutions.contains(index),
        SceneElement::Shape(index) => doc.primitives.contains(index),
        SceneElement::SweepOp(index) => doc.sweeps.contains(index),
        SceneElement::Image(index) => doc.tracing_images.contains(index),
        SceneElement::SketchText(index) => doc.sketch_texts.contains(index),
        SceneElement::Joint(index) => doc.joints.contains(index),
    }
}

pub fn extrusion_alive(doc: &Document, index: crate::model::ExtrusionKey) -> bool {
    doc.extrusions.contains(index)
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
        ConstraintPoint::Origin => true,
        // A face's own vertex is "alive" exactly when it still resolves (extrusion present,
        // index still within its current boundary loop) — it has no owning scene entity.
        ConstraintPoint::FaceVertex { face, index } => {
            crate::extrude::face_boundary_loop_world(doc, face).is_some_and(|l| *index < l.len())
        }
        ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.contains(*text)
        }
        ConstraintPoint::ImageCalibrationPoint { image, index } => doc
            .tracing_images
            .get(*image)
            .is_some_and(|i| crate::model::image_calibration_point_uv(i, *index).is_some()),
    }
}

/// Normalize a selection entry to the entity that should be deleted.
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
        ConstraintPoint::Origin => SceneElement::Origin,
        ConstraintPoint::FaceVertex { .. } => return None,
    })
}

/// Unique delete targets from the current selection (deduped).
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

/// Delete one element and any owned children. Returns true if anything changed.
pub fn delete_element(doc: &mut Document, element: SceneElement) -> bool {
    let mut changed = false;
    match element {
        // Deleting from a drawing page goes through the drawing's own actions (#967), which
        // renumber what's left; there is nothing to delete here.
        SceneElement::DrawingElement { .. } => {}
        // Deleting a component re-homes its members and child components to its parent
        // (#423) — grouping is organizational, so nothing inside is deleted.
        SceneElement::Component(index) => {
            if let Some(gone) = doc.components.remove(index) {
                let parent = gone.parent;
                match parent {
                    Some(p) => {
                        for m in doc.component_members.iter_mut() {
                            if m.1 == index {
                                m.1 = p;
                            }
                        }
                    }
                    None => doc.component_members.retain(|m| m.1 != index),
                }
                for (_, c) in doc.components.iter_mut() {
                    if c.parent == Some(index) {
                        c.parent = parent;
                    }
                }
                delete_joints_referencing(doc, crate::model::JointRef::Component(index));
                changed = true;
            }
        }
        // Deleting a unit instance removes that placement only (#723). The embedded copy
        // stays in `Document.units` even when the last instance goes: unit indices stay
        // stable and re-importing the same source stays cheap (it reuses the copy).
        SceneElement::UnitInstance(index) => {
            if doc.unit_instances.remove(index).is_some() {
                delete_joints_referencing(doc, crate::model::JointRef::UnitInstance(index));
                changed = true;
            }
        }
        SceneElement::ConstructionPlane(index) => {
            if delete_construction_plane(doc, index) {
                changed = true;
            }
        }
        SceneElement::Sketch(sketch) => {
            if delete_sketch(doc, sketch) {
                changed = true;
            }
        }
        SceneElement::Circle(index) => {
            if delete_circle(doc, index) {
                changed = true;
            }
        }
        SceneElement::Line(index) => {
            if delete_line(doc, index) {
                changed = true;
            }
        }
        SceneElement::Constraint(index) => {
            if delete_constraint(doc, index) {
                changed = true;
            }
        }
        SceneElement::Point(point) => {
            if let Some(owner) = point_owner_element(&point) {
                changed |= delete_element(doc, owner);
            }
        }
        SceneElement::Extrusion(index) => {
            if delete_extrusion(doc, index) {
                changed = true;
            }
        }
        SceneElement::Body(index) => {
            if delete_body(doc, index) {
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
        | SceneElement::PrimitiveEdge { .. }
        | SceneElement::RepeatedFace { .. } => {}
        SceneElement::Joint(index) => {
            // The history-tape marker is this joint's place among the live ones (#1055).
            let ordinal = doc.joints.keys().position(|k| k == index);
            if doc.joints.remove(index).is_some() {
                if let Some(ordinal) = ordinal {
                    remove_shape_order_entry(doc, ShapeKind::Joint, ordinal);
                }
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
                        doc.construction_planes.remove(out);
                    }
                    // Repeated-sketch copies: their planes, sketches, and copied entities (#226).
                    let op = removed;
                    for out in &op.sketch_plane_outputs {
                        doc.construction_planes.remove(*out);
                    }
                    for &si in &op.sketch_outputs {
                        for key in doc.lines.keys().collect::<Vec<_>>() {
                            if doc.lines[key].sketch == si {
                                doc.lines.remove(key);
                            }
                        }
                        for c in doc.circles.keys().collect::<Vec<_>>() {
                            if doc.circles[c].sketch == si {
                                doc.circles.remove(c);
                            }
                        }
                        doc.sketches.remove(si);
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchRepeatOp(index) => {
            if let Some(removed) = doc.sketch_repeat_ops.remove(index) {
                {
                    // The duplicated lines/circles go with the op (#222/#228).
                    let op = removed.clone();
                    for &out in &op.line_outputs {
                        doc.lines.remove(out);
                    }
                    for &out in &op.circle_outputs {
                        doc.circles.remove(out);
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchOffsetOp(index) => {
            if let Some(removed) = doc.sketch_offset_ops.remove(index) {
                {
                    // The parallel lines/circles go with the op.
                    let op = removed.clone();
                    for &out in &op.line_outputs {
                        doc.lines.remove(out);
                    }
                    for &out in &op.circle_outputs {
                        doc.circles.remove(out);
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchMirrorOp(index) => {
            if let Some(removed) = doc.sketch_mirror_ops.remove(index) {
                {
                    // The reflected lines/circles go with the op (#523).
                    let op = removed.clone();
                    for &out in &op.line_outputs {
                        doc.lines.remove(out);
                    }
                    for &out in &op.circle_outputs {
                        doc.circles.remove(out);
                    }
                    // The reflected corner-coincidences go with it too (#547).
                    for &ci in &op.constraint_outputs {
                        doc.constraints.remove(ci);
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchVertexTreatmentOp(index) => {
            if let Some(removed) = doc.sketch_vertex_treatment_ops.remove(index) {
                {
                    // Deleting the chamfer/fillet (#538) un-shadows the source edges (they
                    // become live geometry again, sharp corner restored) and removes the
                    // generated trimmed copies, bridges, and stitch constraints.
                    let op = removed.clone();
                    for &li in &op.line_targets {
                        if let Some(l) = doc.lines.get_mut(li) {
                            l.shadow = false;
                        }
                    }
                    for &out in op.line_outputs.iter().chain(op.bridge_outputs.iter()) {
                        doc.lines.remove(out);
                    }
                    for &ci in &op.constraint_outputs {
                        doc.constraints.remove(ci);
                    }
                    changed = true;
                }
            }
        }
        SceneElement::SketchSliceOp(index) => {
            if let Some(removed) = doc.sketch_slice_ops.remove(index) {
                {
                    let op = removed.clone();
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
                        doc.lines.remove(out);
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
            if let Some(op) = doc.slice_ops.remove(index) {
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
        SceneElement::ShellOp(index) => {
            // Deleting the shell removes its hollowed outputs and un-shadows inputs (#1156).
            if let Some(op) = doc.shell_ops.remove(index) {
                for &out in &op.outputs {
                    doc.bodies.remove(out);
                }
                for &input in &op.targets {
                    if !crate::model::body_shadowed_by_other_ops_ex(
                        doc, input, None, None, None, None, None,
                    ) {
                        if let Some(body) = doc.bodies.get_mut(input) {
                            body.shadow = false;
                        }
                    }
                }
                changed = true;
            }
        }
        SceneElement::EdgeTreatmentOp(index) => {
            // Deleting the chamfer/fillet removes its beveled outputs and releases its input
            // bodies from shadow (unless another live operation still consumes them) (#531).
            if let Some(op) = doc.edge_treatment_ops.remove(index) {
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
            // Deleting a shape (#909) takes its body with it — including a Solid whose base
            // is that shape after an add-to-body / cut (#1104).
            if doc.primitives.remove(index).is_some() {
                let produced: Vec<crate::model::BodyKey> = doc
                    .bodies
                    .iter()
                    .filter(|(_, b)| b.source.primitive_base() == Some(index))
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
        // A sketch text is removed outright (#1055), like a tracing image.
        SceneElement::SketchText(index) => {
            changed = doc.sketch_texts.remove(index).is_some();
        }
    }
    changed
}

fn delete_extrusion(doc: &mut Document, index: crate::model::ExtrusionKey) -> bool {
    // The history-tape marker to drop is the one for this extrusion's place among the live
    // ones, read before the removal (#1055).
    let Some(ordinal) = doc.extrusions.keys().position(|k| k == index) else {
        return false;
    };
    doc.extrusions.remove(index);
    remove_shape_order_entry(doc, ShapeKind::Extrusion, ordinal);
    // A body that depends solely on this extrusion is removed with it; a body that is the
    // fused *output* of this extrusion (#1106) is removed too (releasing its host from
    // shadow); a body that only lists this extrusion among others just drops this one.
    let dependent: Vec<crate::model::BodyKey> = doc
        .bodies
        .iter()
        .filter(|(_, body)| body.source.owns_extrusion(index))
        .map(|(i, _)| i)
        .collect();
    let mut hosts_to_release = Vec::new();
    let mut doomed = Vec::new();
    for bi in dependent {
        let source = &doc.bodies[bi].source;
        let solely_owned = source.extrusion_indices() == [index]
            && source.cut_extrusion_indices().is_empty()
            && source.primitive_base().is_none();
        let is_producer = source.producing_extrusion() == Some(index);
        if solely_owned || is_producer {
            if let Some(h) = crate::model::fuse_host_of(doc, bi) {
                hosts_to_release.push(h);
            }
            doomed.push(bi);
        } else {
            doc.bodies[bi].source.remove_extrusion(index);
        }
    }
    // Cascade: any body fused from a doomed host dies with it (#1106 chain).
    let mut i = 0;
    while i < doomed.len() {
        let host = doomed[i];
        for (k, _) in doc.bodies.iter() {
            if !doomed.contains(&k) && crate::model::fuse_host_of(doc, k) == Some(host) {
                doomed.push(k);
            }
        }
        i += 1;
    }
    for bi in doomed {
        delete_body(doc, bi);
    }
    for h in hosts_to_release {
        if doc.bodies.contains(h)
            && !crate::model::body_shadowed_by_other_ops(doc, h, None, None, None, None)
        {
            if let Some(body) = doc.bodies.get_mut(h) {
                body.shadow = false;
            }
        }
    }
    true
}

fn delete_body(doc: &mut Document, index: crate::model::BodyKey) -> bool {
    // The history-tape marker to drop is the one for this body's place among the live ones,
    // read before the removal (#1055).
    let Some(ordinal) = doc.bodies.keys().position(|k| k == index) else {
        return false;
    };
    doc.bodies.remove(index);
    remove_shape_order_entry(doc, ShapeKind::Body, ordinal);
    delete_joints_referencing(doc, crate::model::JointRef::Body(index));
    true
}

/// A joint dies with any of the things it joins (#891): remove every live joint that
/// holds `member`.
fn delete_joints_referencing(doc: &mut Document, member: crate::model::JointRef) -> bool {
    let mut changed = false;
    let doomed: Vec<crate::model::JointKey> = doc
        .joints
        .iter()
        .filter(|(_, j)| j.members.contains(&member))
        .map(|(k, _)| k)
        .collect();
    for ji in doomed {
        let ordinal = doc.joints.keys().position(|k| k == ji);
        doc.joints.remove(ji);
        if let Some(ordinal) = ordinal {
            remove_shape_order_entry(doc, ShapeKind::Joint, ordinal);
        }
        changed = true;
    }
    changed
}

/// Tombstone every target in `elements`.
pub fn delete_elements(doc: &mut Document, elements: &[SceneElement]) -> usize {
    let mut count = 0usize;
    for element in elements {
        if delete_element(doc, element.clone()) {
            count += 1;
        }
    }
    count
}

fn delete_construction_plane(
    doc: &mut Document,
    index: crate::model::ConstructionPlaneKey,
) -> bool {
    // The history-tape marker to drop is the one for this plane's place among the live
    // ones, read before the removal (#1055).
    let Some(ordinal) = doc.construction_planes.keys().position(|k| k == index) else {
        return false;
    };
    doc.construction_planes.remove(index);
    remove_shape_order_entry(doc, ShapeKind::ConstructionPlane, ordinal);
    let face = FaceId::ConstructionPlane(index);
    for sketch in doc.sketches_on_face(face).collect::<Vec<_>>() {
        delete_sketch(doc, sketch);
    }
    true
}

fn delete_sketch(doc: &mut Document, sketch: SketchId) -> bool {
    // The history-tape marker to drop is the one for this sketch's place among the live
    // ones, read before the removal (#1055).
    let Some(ordinal) = doc.sketches.keys().position(|k| k == sketch) else {
        return false;
    };
    doc.sketches.remove(sketch);
    remove_shape_order_entry(doc, ShapeKind::Sketch, ordinal);

    let lines: Vec<crate::model::LineKey> = doc
        .lines
        .iter()
        .filter(|(_, line)| line.sketch == sketch)
        .map(|(i, _)| i)
        .collect();
    for li in lines {
        delete_line(doc, li);
    }
    let circles: Vec<crate::model::CircleKey> = doc
        .circles
        .iter()
        .filter(|(_, circle)| circle.sketch == sketch)
        .map(|(i, _)| i)
        .collect();
    for ci in circles {
        delete_circle(doc, ci);
    }
    let constraints: Vec<crate::model::ConstraintKey> = doc
        .constraints
        .iter()
        .filter(|(_, c)| c.sketch == sketch)
        .map(|(i, _)| i)
        .collect();
    for ci in constraints {
        delete_constraint(doc, ci);
    }
    let planes: Vec<crate::model::ConstructionPlaneKey> = doc
        .construction_planes
        .iter()
        .filter(|(_, plane)| {
            matches!(plane.parent, crate::model::ConstructionPlaneParent::Sketch(s) if s == sketch)
        })
        .map(|(i, _)| i)
        .collect();
    for pi in planes {
        delete_construction_plane(doc, pi);
    }
    true
}

fn delete_circle(doc: &mut Document, index: crate::model::CircleKey) -> bool {
    // The history-tape marker to drop is the one for this circle's place among the live
    // ones, read before the removal (#1055).
    let Some(ordinal) = doc.circles.keys().position(|k| k == index) else {
        return false;
    };
    doc.circles.remove(index);
    remove_shape_order_entry(doc, ShapeKind::Circle, ordinal);
    let face = FaceId::Circle(index);
    for sketch in doc.sketches_on_face(face).collect::<Vec<_>>() {
        delete_sketch(doc, sketch);
    }
    true
}

fn delete_line(doc: &mut Document, index: crate::model::LineKey) -> bool {
    // The history-tape marker to drop is the one for this line's place among the live ones,
    // read before the removal (#1055).
    let Some(ordinal) = doc.lines.keys().position(|k| k == index) else {
        return false;
    };
    doc.lines.remove(index);
    remove_shape_order_entry(doc, ShapeKind::Line, ordinal);
    // #502: detach from parametric ops that would re-create this line on recompute
    // (an offset/repeat rebuild re-inserts live outputs).
    detach_line_from_sketch_ops(doc, index);
    true
}

/// When a line is deleted, drop it from any sketch offset/repeat target or output
/// lists so rebuild does not revive it (#502). Empties the op if it has no sources left.
fn detach_line_from_sketch_ops(doc: &mut Document, line: crate::model::LineKey) {
    let mut orphan_outputs: Vec<crate::model::LineKey> = Vec::new();
    let mut empty_ops: Vec<crate::model::SketchOffsetOpKey> = Vec::new();
    for (oi, op) in doc.sketch_offset_ops.iter_mut() {
        // Parallel target/output slots: drop whichever end references this line.
        let mut i = 0;
        while i < op.line_targets.len() {
            let is_target = op.line_targets[i] == line;
            let is_output = op.line_outputs.get(i).copied() == Some(line);
            if is_target || is_output {
                // Deleting a source also removes its generated output.
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
        let ordinal = doc.lines.keys().position(|k| k == out);
        if let Some(ordinal) = ordinal {
            doc.lines.remove(out);
            remove_shape_order_entry(doc, ShapeKind::Line, ordinal);
        }
    }
    for oi in empty_ops {
        // The history-tape marker to drop is this op's place among the live ones (#1055).
        let ordinal = doc.sketch_offset_ops.keys().position(|k| k == oi);
        if doc.sketch_offset_ops.remove(oi).is_some() {
            if let Some(ordinal) = ordinal {
                remove_shape_order_entry(doc, ShapeKind::SketchOffsetOperation, ordinal);
            }
        }
    }
    for op in doc.sketch_repeat_ops.values_mut() {
        op.line_targets.retain(|&t| t != line);
        op.line_outputs.retain(|&out| out != line);
    }
}

fn delete_constraint(doc: &mut Document, index: crate::model::ConstraintKey) -> bool {
    // The history-tape marker to drop is the one for this constraint's place among the live
    // ones, read before the removal (#1055).
    let Some(ordinal) = doc.constraints.keys().position(|k| k == index) else {
        return false;
    };
    doc.constraints.remove(index);
    remove_shape_order_entry(doc, ShapeKind::Constraint, ordinal);
    true
}

/// Remove a parameter (used by `DeleteParameter` and selection delete). Its name is free
/// again the moment it goes, because it is actually gone (#1055).
pub fn delete_parameter(doc: &mut Document, index: crate::model::ParameterKey) -> bool {
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
        ConstraintPoint::Origin => true,
        ConstraintPoint::FaceVertex { face, index } => {
            crate::extrude::face_boundary_loop_world(doc, face).is_some_and(|l| *index < l.len())
        }
        ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.contains(*text)
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

/// Drop the `ordinal`-th `kind` marker from the history tape (#1055): `ordinal` is the
/// element's place among the live ones of its kind, not its storage key.
pub fn remove_shape_order_entry(doc: &mut Document, kind: ShapeKind, ordinal: usize) {
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
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::constraint_key_for_slot as nkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::unit_key_for_slot as ukey;
    use crate::model::unit_instance_key_for_slot as uikey;
    use super::*;
    use crate::model::{Constraint, ConstraintKind, ConstraintLine, Document, Line};

    /// #1055: deleting a tracing image removes it rather than tombstoning it, and the
    /// image beside it keeps its identity — the whole reason positional identity had to go.
    #[test]
    fn deleting_a_tracing_image_removes_it_and_leaves_its_neighbour_alone() {
        let image = |name: &str| crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: name.to_string(),
            plane: pkey(0),
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

        assert!(delete_element(&mut doc, SceneElement::Image(first)));
        assert_eq!(doc.tracing_images.len(), 1, "gone, not marked");
        assert_eq!(
            doc.tracing_images.get(second).map(|i| i.source_name.as_str()),
            Some("second"),
            "the survivor did not slide into the hole"
        );
        assert!(!element_alive(&doc, SceneElement::Image(first)));
        assert!(
            !delete_element(&mut doc, SceneElement::Image(first)),
            "deleting it twice changes nothing"
        );
    }

    fn push_test_body(doc: &mut Document) -> crate::model::BodyKey {
        let key = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            name: None,
            material: None,
            shadow: false,
        });
        doc.shape_order.push(ShapeKind::Body);
        key
    }

    fn push_test_joint(
        doc: &mut Document,
        members: Vec<crate::model::JointRef>,
    ) -> crate::model::JointKey {
        let key = doc.joints.insert(crate::model::Joint {
            members,
            base: 0,
            kind: crate::model::JointKind::Revolute,
            placement: Default::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: Default::default(),
            name: None,
            frame: Default::default(),
        });
        doc.shape_order.push(ShapeKind::Joint);
        key
    }

    /// #891: deleting a joint removes it along with its shape-order entry; nothing it
    /// joins is touched.
    #[test]
    fn deleting_a_joint_leaves_its_members_alone() {
        let mut doc = Document::default();
        let a = push_test_body(&mut doc);
        let b = push_test_body(&mut doc);
        let ji = push_test_joint(
            &mut doc,
            vec![crate::model::JointRef::Body(a), crate::model::JointRef::Body(b)],
        );
        let order_len = doc.shape_order.len();
        assert!(delete_element(&mut doc, SceneElement::Joint(ji)));
        assert!(!doc.joints.contains(ji));
        assert!(!element_alive(&doc, SceneElement::Joint(ji)));
        assert!(body_alive(&doc, a));
        assert!(body_alive(&doc, b));
        assert_eq!(doc.shape_order.len(), order_len - 1);
        // Already dead: a second delete is a no-op.
        assert!(!delete_element(&mut doc, SceneElement::Joint(ji)));
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
        assert!(delete_element(&mut doc, SceneElement::Body(a)));
        assert!(!doc.joints.contains(ji), "joint must die with its member body");
        assert!(body_alive(&doc, b));
    }

    /// #891: a joint on a unit instance dies when that placement is deleted.
    #[test]
    fn joint_dies_with_its_unit_instance() {
        let mut doc = Document::default();
        let a = push_test_body(&mut doc);
        doc.units.insert(crate::model::ImportedUnit {
            source: crate::model::UnitSource::RelativePath("x.bearcad".to_string()),
            link: Default::default(),
            document: Document::default(),
            source_mtime: None,
            source_hash: None,
        });
        doc.unit_instances.insert(crate::model::UnitInstance {
            unit: ukey(0),
            name: None,
            parameter_overrides: Vec::new(),
            placement: Default::default(),
        });
        let ji = push_test_joint(
            &mut doc,
            vec![
                crate::model::JointRef::Body(a),
                crate::model::JointRef::UnitInstance(uikey(0)),
            ],
        );
        assert!(delete_element(&mut doc, SceneElement::UnitInstance(uikey(0))));
        assert!(!doc.joints.contains(ji), "joint must die with its unit instance");
    }

    fn sketch_with_two_lines() -> (Document, SketchId, crate::model::LineKey, crate::model::LineKey) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let line_a = doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        let line_b = doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        (doc, sketch, line_a, line_b)
    }

    #[test]
    fn deleting_a_line_leaves_a_constraint_pointing_at_the_dead_key() {
        let (mut doc, sketch, line_a, line_b) = sketch_with_two_lines();
        doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(line_a),
                line_b: ConstraintLine::Line(line_b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        doc.shape_order.push(ShapeKind::Constraint);
        assert!(delete_line(&mut doc, line_a));
        assert!(!doc.lines.contains(line_a));
        assert!(!line_alive(&doc, line_a));
        assert!(line_alive(&doc, line_b));
        // The line is really gone now (#1055) — the constraint keeps the dead key, which is
        // what makes the constraint read as invalid instead of silently retargeting.
        assert_eq!(doc.lines.len(), 1);
        let constraint = &doc.constraints[nkey(0)];
        assert!(matches!(
            constraint.kind,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(l),
                ..
            } if l == line_a
        ));
    }

    #[test]
    fn delete_elements_counts_unique_targets() {
        let (mut doc, _, line_a, line_b) = sketch_with_two_lines();
        let count = delete_elements(
            &mut doc,
            &[
                SceneElement::Line(line_a),
                SceneElement::Line(line_b),
            ],
        );
        assert_eq!(count, 2);
        assert!(!doc.lines.contains(line_a));
        assert!(!doc.lines.contains(line_b));
    }
}