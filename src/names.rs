//! Custom names for scene elements shown in the Elements pane.

use crate::constraints::constraint_label;
use crate::hierarchy::{HierarchyNode, SceneElement};
use crate::model::{effective_length_unit, Document};
use crate::value::format_length_display_in;

/// Map a selected element to the object that owns a user-visible name.
pub fn nameable_element(element: SceneElement) -> Option<SceneElement> {
    match element {
        // A drawing's items aren't renameable (#967); the drawing itself is.
        SceneElement::DrawingElement { .. } => None,
        SceneElement::ConstructionPlane(_)
        | SceneElement::Sketch(_)
        | SceneElement::Line(_)
        | SceneElement::Circle(_)
        | SceneElement::Constraint(_)
        | SceneElement::Extrusion(_)
        | SceneElement::Body(_)
        | SceneElement::Image(_)
        | SceneElement::BooleanOp(_)
        | SceneElement::MoveOp(_)
        | SceneElement::MirrorOp(_)
        | SceneElement::RepeatOp(_)
        | SceneElement::SketchRepeatOp(_)
        | SceneElement::SketchOffsetOp(_)
        | SceneElement::SketchMirrorOp(_)
        | SceneElement::SketchVertexTreatmentOp(_)
        | SceneElement::SketchSliceOp(_)
        | SceneElement::SketchText(_)
        | SceneElement::SliceOp(_)
        | SceneElement::EdgeTreatmentOp(_)
        | SceneElement::Revolution(_)
        | SceneElement::Shape(_)
        | SceneElement::SweepOp(_)
        | SceneElement::Component(_)
        | SceneElement::UnitInstance(_)
        | SceneElement::Joint(_) => Some(element),
        SceneElement::Point(_)
        | SceneElement::FaceEdge(_)
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
        | SceneElement::RepeatedFace { .. } => None,
    }
}

/// When exactly one nameable element is selected, return it.
pub fn single_nameable_from_selection(
    selection: &crate::selection::SceneSelection,
) -> Option<SceneElement> {
    selection.single().and_then(nameable_element)
}

fn name_matches(stored: Option<&str>, query: &str) -> bool {
    stored
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some_and(|s| s == query)
}

/// Find the first scene element with the given custom name (case-sensitive).
pub fn find_element_by_name(doc: &Document, name: &str) -> Option<SceneElement> {
    let query = name.trim();
    if query.is_empty() {
        return None;
    }
    for (index, plane) in doc.construction_planes.iter().enumerate() {
        if name_matches(plane.name.as_deref(), query) {
            return Some(SceneElement::ConstructionPlane(index));
        }
    }
    for (index, sketch) in doc.sketches.iter().enumerate() {
        if name_matches(sketch.name.as_deref(), query) {
            return Some(SceneElement::Sketch(index));
        }
    }
    for (index, line) in doc.lines.iter().enumerate() {
        if line.deleted {
            continue;
        }
        if name_matches(line.name.as_deref(), query) {
            return Some(SceneElement::Line(index));
        }
    }
    for (index, circle) in doc.circles.iter().enumerate() {
        if circle.deleted {
            continue;
        }
        if name_matches(circle.name.as_deref(), query) {
            return Some(SceneElement::Circle(index));
        }
    }
    for (index, constraint) in doc.constraints.iter().enumerate() {
        if constraint.deleted {
            continue;
        }
        if name_matches(constraint.name.as_deref(), query) {
            return Some(SceneElement::Constraint(index));
        }
    }
    for (index, extrusion) in doc.extrusions.iter() {
        if name_matches(extrusion.name.as_deref(), query) {
            return Some(SceneElement::Extrusion(index));
        }
    }
    // Imported unit instances (#728): findable/selectable by their instance name.
    for (index, instance) in doc.unit_instances.iter() {
        if name_matches(instance.name.as_deref(), query) {
            return Some(SceneElement::UnitInstance(index));
        }
    }
    // Joints (#891): findable/selectable by name like any operation.
    for (index, joint) in doc.joints.iter() {
        if name_matches(joint.name.as_deref(), query) {
            return Some(SceneElement::Joint(index));
        }
    }
    None
}

/// How a picked [`crate::model::RevolveAxis`] reads in a context pane's axis picker — the
/// sketch line's name, the body's name for an edge picked on it (#643), or the origin axis.
pub fn revolve_axis_label(doc: &Document, axis: crate::model::RevolveAxis) -> String {
    use crate::model::RevolveAxis;
    match axis {
        RevolveAxis::Line(li) => element_name(doc, SceneElement::Line(li))
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("line {li}")),
        RevolveAxis::BodyEdge { body, .. } => element_name(doc, SceneElement::Body(body))
            .map(|n| format!("an edge of {n}"))
            .unwrap_or_else(|| format!("an edge of body {}", body.index())),
        RevolveAxis::X => "the X axis".to_string(),
        RevolveAxis::Y => "the Y axis".to_string(),
        RevolveAxis::Z => "the Z axis".to_string(),
    }
}

pub fn element_name(doc: &Document, element: SceneElement) -> Option<&str> {
    let name = match element {
        SceneElement::ConstructionPlane(index) => doc.construction_planes.get(index)?.name.as_deref(),
        SceneElement::Sketch(index) => doc.sketches.get(index)?.name.as_deref(),
        SceneElement::Line(index) => doc.lines.get(index)?.name.as_deref(),
        SceneElement::Circle(index) => doc.circles.get(index)?.name.as_deref(),
        SceneElement::Constraint(index) => doc.constraints.get(index)?.name.as_deref(),
        // A drawing item's name is the page's own (#967) — a view or a note has no stored
        // name of its own to rename.
        SceneElement::DrawingElement { .. } => None,
        SceneElement::Extrusion(index) => doc.extrusions.get(index)?.name.as_deref(),
        SceneElement::Body(index) => doc.bodies.get(index)?.name.as_deref(),
        SceneElement::Image(index) => doc.tracing_images.get(index)?.name.as_deref(),
        SceneElement::BooleanOp(index) => doc.boolean_ops.get(index)?.name.as_deref(),
        SceneElement::MoveOp(index) => doc.move_ops.get(index)?.name.as_deref(),
        SceneElement::MirrorOp(index) => doc.mirror_ops.get(index)?.name.as_deref(),
        SceneElement::RepeatOp(index) => doc.repeat_ops.get(index)?.name.as_deref(),
        SceneElement::SketchRepeatOp(index) => doc.sketch_repeat_ops.get(index)?.name.as_deref(),
        SceneElement::SketchOffsetOp(index) => doc.sketch_offset_ops.get(index)?.name.as_deref(),
        SceneElement::SketchMirrorOp(index) => doc.sketch_mirror_ops.get(index)?.name.as_deref(),
        SceneElement::SketchVertexTreatmentOp(index) => {
            doc.sketch_vertex_treatment_ops.get(index)?.name.as_deref()
        }
        SceneElement::SketchSliceOp(index) => doc.sketch_slice_ops.get(index)?.name.as_deref(),
        SceneElement::SketchText(index) => doc.sketch_texts.get(index)?.name.as_deref(),
        SceneElement::SliceOp(index) => doc.slice_ops.get(index)?.name.as_deref(),
        SceneElement::EdgeTreatmentOp(index) => {
            doc.edge_treatment_ops.get(index)?.name.as_deref()
        }
        SceneElement::Revolution(index) => doc.revolutions.get(index)?.name.as_deref(),
        SceneElement::Shape(index) => doc.primitives.get(index)?.name.as_deref(),
        SceneElement::SweepOp(index) => doc.sweeps.get(index)?.name.as_deref(),
        SceneElement::Component(index) => doc.components.get(index)?.name.as_deref(),
        SceneElement::UnitInstance(index) => doc.unit_instances.get(index)?.name.as_deref(),
        SceneElement::Joint(index) => doc.joints.get(index)?.name.as_deref(),
        SceneElement::Point(_)
        | SceneElement::FaceEdge(_)
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
        | SceneElement::RepeatedFace { .. } => None,
    }?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub fn set_element_name(doc: &mut Document, element: SceneElement, name: String) -> Result<(), String> {
    let stored = {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };
    match element {
        SceneElement::ConstructionPlane(index) => {
            let plane = doc
                .construction_planes
                .get_mut(index)
                .ok_or_else(|| format!("construction plane {index} not found"))?;
            plane.name = stored;
        }
        SceneElement::Sketch(index) => {
            let sketch = doc
                .sketches
                .get_mut(index)
                .ok_or_else(|| format!("sketch {index} not found"))?;
            sketch.name = stored;
        }
        SceneElement::Line(index) => {
            let line = doc
                .lines
                .get_mut(index)
                .ok_or_else(|| format!("line {index} not found"))?;
            line.name = stored;
        }
        SceneElement::Circle(index) => {
            let circle = doc
                .circles
                .get_mut(index)
                .ok_or_else(|| format!("circle {index} not found"))?;
            circle.name = stored;
        }
        SceneElement::DrawingElement { .. } => {
            return Err("a drawing's items have no name of their own".to_string())
        }
        SceneElement::Constraint(index) => {
            let constraint = doc
                .constraints
                .get_mut(index)
                .ok_or_else(|| format!("constraint {index} not found"))?;
            constraint.name = stored;
        }
        SceneElement::Extrusion(index) => {
            let extrusion = doc
                .extrusions
                .get_mut(index)
                .ok_or_else(|| format!("extrusion {} not found", index.index()))?;
            extrusion.name = stored;
        }
        SceneElement::Body(index) => {
            let body = doc
                .bodies
                .get_mut(index)
                .ok_or_else(|| format!("body {index:?} not found"))?;
            body.name = stored;
        }
        SceneElement::BooleanOp(index) => {
            let op = doc
                .boolean_ops
                .get_mut(index)
                .ok_or_else(|| format!("boolean operation {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::MoveOp(index) => {
            let op = doc
                .move_ops
                .get_mut(index)
                .ok_or_else(|| format!("move operation {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::MirrorOp(index) => {
            let op = doc
                .mirror_ops
                .get_mut(index)
                .ok_or_else(|| format!("mirror operation {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::RepeatOp(index) => {
            let op = doc
                .repeat_ops
                .get_mut(index)
                .ok_or_else(|| format!("repeat operation {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::SketchRepeatOp(index) => {
            let op = doc
                .sketch_repeat_ops
                .get_mut(index)
                .ok_or_else(|| format!("sketch repeat {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::SketchOffsetOp(index) => {
            let op = doc
                .sketch_offset_ops
                .get_mut(index)
                .ok_or_else(|| format!("sketch offset {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::SketchMirrorOp(index) => {
            let op = doc
                .sketch_mirror_ops
                .get_mut(index)
                .ok_or_else(|| format!("sketch mirror {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::SketchVertexTreatmentOp(index) => {
            let op = doc
                .sketch_vertex_treatment_ops
                .get_mut(index)
                .ok_or_else(|| format!("sketch vertex treatment {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::SketchSliceOp(index) => {
            let op = doc
                .sketch_slice_ops
                .get_mut(index)
                .ok_or_else(|| format!("sketch slice {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::SketchText(index) => {
            let t = doc
                .sketch_texts
                .get_mut(index)
                .ok_or_else(|| format!("sketch text {} not found", index.index()))?;
            t.name = stored;
        }
        SceneElement::SliceOp(index) => {
            let op = doc
                .slice_ops
                .get_mut(index)
                .ok_or_else(|| format!("slice operation {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::EdgeTreatmentOp(index) => {
            let op = doc
                .edge_treatment_ops
                .get_mut(index)
                .ok_or_else(|| format!("edge treatment operation {index:?} not found"))?;
            op.name = stored;
        }
        SceneElement::Component(index) => {
            let component = doc
                .components
                .get_mut(index)
                .ok_or_else(|| format!("component {} not found", index.index()))?;
            component.name = stored;
        }
        SceneElement::Revolution(index) => {
            let rev = doc
                .revolutions
                .get_mut(index)
                .ok_or_else(|| format!("revolution {index:?} not found"))?;
            rev.name = stored;
        }
        SceneElement::Shape(index) => {
            let shape = doc
                .primitives
                .get_mut(index)
                .ok_or_else(|| format!("shape {index:?} not found"))?;
            shape.name = stored;
        }
        SceneElement::SweepOp(index) => {
            let fp = doc
                .sweeps
                .get_mut(index)
                .ok_or_else(|| format!("sweep {index:?} not found"))?;
            fp.name = stored;
        }
        SceneElement::Image(index) => {
            let image = doc
                .tracing_images
                .get_mut(index)
                .ok_or_else(|| format!("image {index:?} not found"))?;
            image.name = stored;
        }
        SceneElement::UnitInstance(index) => {
            let old = doc
                .unit_instances
                .get(index)
                .ok_or_else(|| format!("unit instance {} not found", index.index()))?
                .name
                .clone();
            // Another instance already answering to this name would make `name.param`
            // ambiguous (#731): refuse rather than silently shadow.
            if let Some(new_name) = stored.as_deref() {
                let taken = doc.unit_instances.iter().any(|(i, inst)| {
                    i != index && inst.name.as_deref() == Some(new_name)
                });
                if taken {
                    return Err(format!("Another instance is already named '{new_name}'"));
                }
            }
            let unit = doc.unit_instances[index].unit;
            doc.unit_instances[index].name = stored.clone();
            // The rename rewrites every `old.param` reference in the same step (#731), so
            // the (snapshot-based) undo restores both name and references together.
            if let (Some(old), Some(new)) = (old.as_deref(), stored.as_deref()) {
                crate::parameters::propagate_instance_rename(doc, unit, old, new);
                let _ = crate::parameters::recompute_document_geometry(doc);
            }
        }
        SceneElement::Joint(index) => {
            let joint = doc
                .joints
                .get_mut(index)
                .ok_or_else(|| format!("joint {index:?} not found"))?;
            joint.name = stored;
        }
        SceneElement::Point(_) => {
            return Err("points cannot be renamed".to_string());
        }
        SceneElement::FaceEdge(_) => {
            return Err("face edges cannot be renamed".to_string());
        }
        SceneElement::Origin => {
            return Err("the origin cannot be renamed".to_string());
        }
        SceneElement::GlobalAxis(_) => {
            return Err("the world axes cannot be renamed".to_string());
        }
        SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::BodyFace { .. }
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_)
        | SceneElement::ExtrusionEdge { .. }
        | SceneElement::RepeatedFace { .. } => {
            return Err("body edges, vertices, and faces cannot be renamed".to_string());
        }
    }
    Ok(())
}

/// A primitive shape's kind as it reads in the app (#909).
pub fn primitive_kind_label(kind: crate::model::PrimitiveKind) -> &'static str {
    use crate::model::PrimitiveKind as K;
    match kind {
        K::Cuboid => "Cuboid",
        K::Cylinder => "Cylinder",
        K::Sphere => "Sphere",
    }
}

pub fn default_node_label(doc: &Document, node: HierarchyNode) -> String {
    match node {
        // The synthetic root has no stored filename/title to draw on (#87) — `Document`
        // doesn't carry one — so it always gets this fixed label.
        HierarchyNode::Document => "Document".to_string(),
        HierarchyNode::Component(i) => format!("Component {}", i.index()),
        // The instance's own name (set at import from the file stem) lives in
        // `element_name`; this is only the fallback for an unnamed instance (#723).
        HierarchyNode::UnitInstance(i) => format!("Unit {}", i.index()),
        HierarchyNode::UnitChild { instance, ordinal } => {
            crate::hierarchy::unit_child_rows(doc, instance)
                .get(ordinal)
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| format!("Unit content {ordinal}"))
        }
        HierarchyNode::ConstructionPlane(i) => {
            if i == 0 {
                "Construction plane (XY)".to_string()
            } else {
                format!("Construction plane {i}")
            }
        }
        HierarchyNode::Sketch(i) => format!("Sketch {i}"),
        HierarchyNode::Line(i) => {
            let line = &doc.lines[i];
            let len = line.length();
            let unit = effective_length_unit(doc, line.sketch);
            let len_label = format_length_display_in(len, unit);
            // A chamfer/fillet bridging line (#76) gets a more recognizable default label than
            // a generic "Line N" — fillet vs. chamfer is distinguishable by whether the bridge
            // is curved (a fillet always sets `bezier`; a chamfer's bridge is always straight).
            if line.chamfer_fillet_parent.is_some() {
                let kind = if line.bezier.is_some() { "Fillet" } else { "Chamfer" };
                format!("{kind} {i} ({len_label})")
            } else {
                format!("Line {i} ({len_label})")
            }
        }
        HierarchyNode::Circle(i) => {
            let circle = &doc.circles[i];
            let diameter = circle.diameter();
            let unit = effective_length_unit(doc, circle.sketch);
            format!("Circle {i} ({})", crate::value::format_diameter_display_in(diameter, unit))
        }
        HierarchyNode::Constraint(i) => constraint_label(doc, i),
        HierarchyNode::Extrusion(i) => {
            let extrusion = doc.extrusions.get(i);
            let distance = extrusion.map(|e| e.distance).unwrap_or(0.0);
            let unit = extrusion
                .map(|e| effective_length_unit(doc, e.sketch))
                .unwrap_or(doc.default_length_unit);
            format!(
                "Extrusion {} ({})",
                i.index(),
                format_length_display_in(distance, unit)
            )
        }
        HierarchyNode::Body(i) => format!("Body {}", i.index()),
        HierarchyNode::Image(i) => doc
            .tracing_images
            .get(i)
            .map(|img| img.source_name.clone())
            .unwrap_or_else(|| format!("Image {}", i.index())),
        HierarchyNode::BooleanOp(i) => {
            let kind = doc
                .boolean_ops
                .get(i)
                .map(|op| op.kind.label())
                .unwrap_or("Boolean");
            format!("{kind} {}", i.index())
        }
        HierarchyNode::MoveOp(i) => format!("Move {}", i.index()),
        HierarchyNode::MirrorOp(i) => format!("Mirror {}", i.index()),
        HierarchyNode::RepeatOp(i) => format!("Repeat {}", i.index()),
        HierarchyNode::SketchRepeatOp(i) => format!("Sketch repeat {}", i.index()),
        HierarchyNode::SketchOffsetOp(i) => format!("Offset {}", i.index()),
        HierarchyNode::SketchMirrorOp(i) => format!("Sketch mirror {}", i.index()),
        HierarchyNode::SketchVertexTreatmentOp(i) => {
            format!("{} {}", sketch_vertex_treatment_label(doc, i), i.index())
        }
        HierarchyNode::SketchSliceOp(i) => format!("Sketch slice {}", i.index()),
        HierarchyNode::SketchText(i) => {
            // Show the string itself (first line, trimmed to keep rows compact).
            let content = doc
                .sketch_texts
                .get(i)
                .map(|t| t.text.lines().next().unwrap_or("").to_string())
                .unwrap_or_default();
            let mut short: String = content.chars().take(16).collect();
            if short.len() < content.len() {
                short.push('…');
            }
            format!("Text {} (\"{short}\")", i.index())
        }
        HierarchyNode::SliceOp(i) => format!("Slice {}", i.index()),
        HierarchyNode::EdgeTreatmentOp(i) => {
            match doc.edge_treatment_ops.get(i).map(|o| o.kind) {
                Some(crate::model::VertexTreatmentKind::Fillet) => format!("Fillet {}", i.index()),
                _ => format!("Chamfer {}", i.index()),
            }
        }
        HierarchyNode::Revolution(i) => format!("Revolve {}", i.index()),
        // A shape reads by what it is: "Cuboid 0", "Sphere 2" (#909).
        HierarchyNode::Shape(i) => match doc.primitives.get(i) {
            Some(shape) => {
                format!("{} {}", crate::names::primitive_kind_label(shape.kind), i.index())
            }
            None => format!("Shape {}", i.index()),
        },
        HierarchyNode::SweepOp(i) => format!("Sweep {}", i.index()),
        // A joint reads by its kind (#891): "Revolute 0", or "Rigid group 2" once a rigid
        // joint ties more than two things (#900).
        HierarchyNode::Joint(i) => match doc.joints.get(i) {
            Some(j) if matches!(j.kind, crate::model::JointKind::Rigid) && j.members.len() > 2 => {
                format!("Rigid group {}", i.index())
            }
            Some(j) => format!("{} {}", joint_kind_label(&j.kind), i.index()),
            None => format!("Joint {}", i.index()),
        },
        HierarchyNode::Drawing(i) => doc
            .drawings
            .get(i)
            .and_then(|d| d.name.as_deref())
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Drawing {}", i.index())),
        HierarchyNode::EdgeTreatment { extrusion, index } => {
            match doc
                .extrusions
                .get(extrusion)
                .and_then(|ext| ext.edge_treatments.get(index))
            {
                Some(t) => {
                    let unit = doc
                        .extrusions
                        .get(extrusion)
                        .map(|e| effective_length_unit(doc, e.sketch))
                        .unwrap_or(doc.default_length_unit);
                    let kind = match t.kind {
                        crate::model::VertexTreatmentKind::Chamfer => "Chamfer",
                        crate::model::VertexTreatmentKind::Fillet => "Fillet",
                    };
                    format!("{kind} ({})", format_length_display_in(t.amount, unit))
                }
                None => "Fillet".to_string(),
            }
        }
        HierarchyNode::Loft(i) => doc
            .lofts
            .get(i)
            .and_then(|l| l.name.as_deref())
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Loft {}", i.index())),
        HierarchyNode::DrawingProjection { drawing, view } => {
            match doc.drawings.get(drawing).and_then(|d| d.views.get(view)) {
                Some(v) => {
                    let source = match v.sketch {
                        Some(si) => node_label(doc, HierarchyNode::Sketch(si)),
                        None => node_label(doc, HierarchyNode::Body(v.body)),
                    };
                    format!("{source} — {}", v.orientation.label())
                }
                None => "Projection".to_string(),
            }
        }
        HierarchyNode::DrawingAnnotation { drawing, annotation } => {
            match doc.drawings.get(drawing).and_then(|d| d.annotations.get(annotation)) {
                Some(a) => {
                    let first = a.text.lines().next().unwrap_or("").trim();
                    if first.is_empty() {
                        format!("Text {annotation}")
                    } else {
                        let mut s: String = first.chars().take(20).collect();
                        if first.chars().count() > 20 {
                            s.push('…');
                        }
                        format!("Text: {s}")
                    }
                }
                None => "Text".to_string(),
            }
        }
        HierarchyNode::DrawingDimension { drawing, view, a, b } => {
            let unit = doc.default_length_unit;
            let len = (crate::hierarchy::dequantize_body_point(a)
                - crate::hierarchy::dequantize_body_point(b))
            .length();
            let _ = (drawing, view);
            format!("Dim: {}", crate::value::format_length_display_in(len, unit))
        }
    }
}

/// A short display label for a selected element, for the Select tool's selection picker
/// (#202). A custom name wins; otherwise a compact type + index label. Kept index-safe (no
/// direct slice indexing) so a stale selection can't panic the picker.
/// The type word for an in-sketch chamfer/fillet op (#538), derived from its first corner's
/// kind: "Chamfer" or "Fillet" (falling back to "Chamfer/Fillet" for an empty op).
/// A joint kind's display word (#891).
pub fn joint_kind_label(kind: &crate::model::JointKind) -> &'static str {
    match kind {
        crate::model::JointKind::Rigid => "Rigid",
        crate::model::JointKind::Slider => "Slider",
        crate::model::JointKind::Revolute => "Revolute",
        crate::model::JointKind::Cylindrical => "Cylindrical",
        crate::model::JointKind::Planar => "Planar",
        crate::model::JointKind::Ball => "Ball",
        crate::model::JointKind::PinSlot => "Pin-slot",
        crate::model::JointKind::Screw { .. } => "Screw",
    }
}

fn sketch_vertex_treatment_label(
    doc: &Document,
    index: crate::model::SketchVertexTreatmentOpKey,
) -> &'static str {
    match doc
        .sketch_vertex_treatment_ops
        .get(index)
        .and_then(|o| o.corners.first())
        .map(|c| c.kind)
    {
        Some(crate::model::VertexTreatmentKind::Chamfer) => "Chamfer",
        Some(crate::model::VertexTreatmentKind::Fillet) => "Fillet",
        None => "Chamfer/Fillet",
    }
}

/// A body's user-visible name, or `Body N` when it hasn't been renamed. Sub-element labels
/// ("Corner of …", "Edge midpoint of …") read better against the name the user chose.
fn body_label(doc: &Document, body: crate::model::BodyKey) -> String {
    element_name(doc, SceneElement::Body(body))
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("Body {}", body.index()))
}

pub fn scene_element_label(doc: &Document, element: &SceneElement) -> String {
    if let Some(name) = element_name(doc, element.clone()) {
        return name.to_string();
    }
    match element {
        SceneElement::ConstructionPlane(i) => {
            if *i == 0 {
                "Construction plane (XY)".to_string()
            } else {
                format!("Construction plane {i}")
            }
        }
        SceneElement::Component(i) => format!("Component {}", i.index()),
        SceneElement::UnitInstance(i) => format!("Unit {}", i.index()),
        SceneElement::Sketch(i) => format!("Sketch {i}"),
        SceneElement::Line(i) => format!("Line {i}"),
        SceneElement::Circle(i) => format!("Circle {i}"),
        SceneElement::Origin => "Origin".to_string(),
        SceneElement::GlobalAxis(axis) => axis.label().to_string(),
        SceneElement::Point(_) => "Point".to_string(),
        SceneElement::Constraint(i) => format!("Constraint {i}"),
        // The same label the Elements pane gives the row (#967), so the picker and the pane
        // name the thing identically.
        SceneElement::DrawingElement { drawing, element } => {
            use crate::context::DrawingElementRef as D;
            node_label(
                doc,
                match element {
                    D::Projection(view) => HierarchyNode::DrawingProjection {
                        drawing: *drawing,
                        view: *view,
                    },
                    D::Text(annotation) => HierarchyNode::DrawingAnnotation {
                        drawing: *drawing,
                        annotation: *annotation,
                    },
                    D::Dimension { view, a, b } => HierarchyNode::DrawingDimension {
                        drawing: *drawing,
                        view: *view,
                        a: *a,
                        b: *b,
                    },
                },
            )
        }
        SceneElement::Extrusion(i) => format!("Extrusion {}", i.index()),
        SceneElement::Body(i) => format!("Body {}", i.index()),
        SceneElement::FaceEdge(_) => "Edge".to_string(),
        SceneElement::BodyEdge { body, .. } => format!("Edge of {}", body_label(doc, *body)),
        SceneElement::BodyVertex { body, .. } => {
            format!("Corner of {}", body_label(doc, *body))
        }
        SceneElement::BodyFace { body, .. } => format!("Face of {}", body_label(doc, *body)),
        // A hole, a boss, a shaft (#1013) — and its centre line.
        SceneElement::BodyCylinder { body, .. } => format!("Cylinder of {}", body_label(doc, *body)),
        SceneElement::BodyAxis { body, .. } => format!("Axis of {}", body_label(doc, *body)),
        SceneElement::SketchFace(face) => crate::face::face_label(doc, face.clone()),
        // An analytic edge reads as its owner plus where it sits in the profile (#955) — the
        // wording the Chamfer/Fillet picker's own row builder used before it became a picker.
        // A repeat instance's face names the source face and which copy (#955).
        SceneElement::RepeatedFace { face, instance, .. } => format!(
            "{} (copy {instance})",
            crate::face::face_label(doc, face.clone())
        ),
        SceneElement::ExtrusionEdge { extrusion, edge } => {
            let owner = element_name(doc, SceneElement::Extrusion(*extrusion))
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("Extrusion {}", extrusion.index()));
            let which = match edge {
                crate::model::ExtrusionEdgeRef::Vertical { edge, .. } => {
                    format!("vertical {edge}")
                }
                crate::model::ExtrusionEdgeRef::Cap { edge, top: true, .. } => {
                    format!("top {edge}")
                }
                crate::model::ExtrusionEdgeRef::Cap { edge, top: false, .. } => {
                    format!("base {edge}")
                }
            };
            format!("{owner} — {which}")
        }
        // A snap point reads by which feature of the body it sits on (#955), the wording the
        // Move tool's own picker rows used before they became real pickers.
        SceneElement::MovePoint(point) => match point.body() {
            Some(body) => {
                let body = body_label(doc, body);
                match point {
                    crate::model::MovePointRef::Vertex { .. } => format!("Corner of {body}"),
                    crate::model::MovePointRef::EdgeMidpoint { .. } => {
                        format!("Edge midpoint of {body}")
                    }
                    crate::model::MovePointRef::OnEdge { .. } => format!("On an edge of {body}"),
                    crate::model::MovePointRef::FaceCenter { .. } => {
                        format!("Middle of a face of {body}")
                    }
                    crate::model::MovePointRef::Origin => "Origin".to_string(),
                }
            }
            None => "Origin".to_string(),
        },
        SceneElement::Image(i) => format!("Image {}", i.index()),
        SceneElement::BooleanOp(i) => format!("Boolean {}", i.index()),
        SceneElement::MoveOp(i) => format!("Move {}", i.index()),
        SceneElement::MirrorOp(i) => format!("Mirror {}", i.index()),
        SceneElement::RepeatOp(i) => format!("Repeat {}", i.index()),
        SceneElement::SketchRepeatOp(i) => format!("Sketch repeat {}", i.index()),
        SceneElement::SketchOffsetOp(i) => format!("Offset {}", i.index()),
        SceneElement::SketchMirrorOp(i) => format!("Sketch mirror {}", i.index()),
        SceneElement::SketchVertexTreatmentOp(i) => {
            format!("{} {}", sketch_vertex_treatment_label(doc, *i), i.index())
        }
        SceneElement::SketchSliceOp(i) => format!("Sketch slice {}", i.index()),
        SceneElement::SketchText(i) => format!("Text {}", i.index()),
        SceneElement::SliceOp(i) => format!("Slice {}", i.index()),
        SceneElement::EdgeTreatmentOp(i) => match doc.edge_treatment_ops.get(*i).map(|o| o.kind) {
            Some(crate::model::VertexTreatmentKind::Fillet) => format!("Fillet {}", i.index()),
            _ => format!("Chamfer {}", i.index()),
        },
        SceneElement::Revolution(i) => format!("Revolve {}", i.index()),
        SceneElement::Shape(i) => format!("Shape {}", i.index()),
        SceneElement::SweepOp(i) => format!("Sweep {}", i.index()),
        SceneElement::Joint(i) => format!("Joint {}", i.index()),
    }
}

pub fn node_label(doc: &Document, node: HierarchyNode) -> String {
    // HierarchyNode::Document has no SceneElement, and thus no custom-name storage — it
    // always falls through to its fixed default label.
    crate::hierarchy::scene_element_for_node(node)
        .and_then(|element| element_name(doc, element))
        .map(str::to_string)
        .unwrap_or_else(|| default_node_label(doc, node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::add_distance_constraint;
    use crate::model::{Document, FaceId, Line};

    #[test]
    fn chamfer_fillet_bridge_line_gets_a_recognizable_default_label() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        // A straight bridge (chamfer): default label says "Chamfer", not "Line".
        let mut chamfer_bridge = Line::from_local_endpoints(sketch, 10.0, 0.0, 15.0, 5.0);
        chamfer_bridge.chamfer_fillet_parent = Some(0);
        doc.lines.push(chamfer_bridge);
        assert!(node_label(&doc, HierarchyNode::Line(1)).starts_with("Chamfer 1"));
        // A curved bridge (fillet): default label says "Fillet".
        let mut fillet_bridge = Line::from_local_endpoints(sketch, 10.0, 0.0, 15.0, 5.0);
        fillet_bridge.chamfer_fillet_parent = Some(0);
        fillet_bridge.bezier = Some([(11.0, 0.0), (14.0, 4.0)]);
        doc.lines.push(fillet_bridge);
        assert!(node_label(&doc, HierarchyNode::Line(2)).starts_with("Fillet 2"));
        // An ordinary line (no chamfer/fillet parent) keeps the generic label.
        assert!(node_label(&doc, HierarchyNode::Line(0)).starts_with("Line 0"));
    }

    #[test]
    fn custom_name_replaces_default_label() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        set_element_name(&mut doc, SceneElement::Line(0), "Guide".to_string()).unwrap();
        assert_eq!(node_label(&doc, HierarchyNode::Line(0)), "Guide");
        assert_eq!(
            element_name(&doc, SceneElement::Line(0)),
            Some("Guide")
        );
    }

    #[test]
    fn constraint_custom_name_shown_in_elements_pane() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        add_distance_constraint(
            &mut doc,
            sketch,
            crate::model::DistanceTarget::LineLength(0),
            "10mm".to_string(),
        )
        .unwrap();
        set_element_name(&mut doc, SceneElement::Constraint(0), "Length lock".to_string())
            .unwrap();
        assert_eq!(
            node_label(&doc, HierarchyNode::Constraint(0)),
            "Length lock"
        );
    }
}