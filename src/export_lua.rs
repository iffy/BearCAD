//! Export the current document as a deterministic Lua script (#1159).
//!
//! Walks the element graph (nesting + input edges) in topological order and emits the
//! declarative `bearcad.*` minimum needed to recreate the model — no `bearcad.ui` module.

use crate::hierarchy::{build_element_list, HierarchyNode, SceneElement};
use crate::model::{
    ConstraintEntity, ConstraintKind, ConstraintLine, ConstraintPoint, DistanceTarget, Document,
    FaceId, LineEnd, SketchId,
};
use crate::script::Instruction;
use std::collections::HashSet;

/// Serialize `doc` as a replayable Lua script that rebuilds the same model without UI calls.
pub fn document_to_lua(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str("-- BearCAD document as Lua\n");
    out.push_str("-- Deterministic recreate (no bearcad.ui). Replay: cargo run -- --script <file> --exit\n\n");
    out.push_str("bearcad.new()\n");

    // Document units when non-default.
    let default_doc = Document::default();
    if doc.default_length_unit != default_doc.default_length_unit
        || doc.default_angle_unit != default_doc.default_angle_unit
    {
        out.push_str(&format!(
            "bearcad.set_units{{ length = {:?}, angle = {:?} }}\n",
            doc.default_length_unit.script_name(),
            doc.default_angle_unit.script_name()
        ));
    }

    // Free parameters first (derived ones need geometry that may not exist yet — emit after).
    let mut derived_params = Vec::new();
    for (_key, param) in doc.parameters.iter() {
        match &param.source {
            None => {
                out.push_str(
                    &Instruction::AddParameter {
                        name: param.name.clone(),
                        expression: param.expression.clone(),
                    }
                    .as_lua_in(Some(doc)),
                );
                out.push('\n');
                // #1176/#1180: options (private, min, max, step) after the add, by name.
                let default_primary =
                    crate::parameters::new_parameter_primary_default(&param.expression);
                if param.primary != default_primary {
                    out.push_str(&format!(
                        "bearcad.edit_parameter{{ name = {:?}, private = {} }}\n",
                        param.name, !param.primary
                    ));
                }
                for (which, bound) in [
                    (crate::parameters::ParameterBound::Minimum, &param.minimum),
                    (crate::parameters::ParameterBound::Maximum, &param.maximum),
                    (crate::parameters::ParameterBound::Step, &param.step),
                ] {
                    if let Some(expression) = bound {
                        out.push_str(&format!(
                            "bearcad.edit_parameter{{ name = {:?}, {} = {:?} }}\n",
                            param.name,
                            which.script_name(),
                            expression
                        ));
                    }
                }
            }
            Some(source) => derived_params.push((param.name.clone(), source.clone())),
        }
    }

    let mut ctx = EmitCtx::new(doc);
    let nodes = build_element_list(doc, None);
    for node in nodes {
        ctx.emit_node(node, &mut out);
    }
    ctx.close_sketch(&mut out);

    // Derived parameters after geometry they reference exists.
    for (name, source) in derived_params {
        out.push_str(
            &Instruction::CreateDerivedParameter {
                source,
                name: Some(name),
            }
            .as_lua_in(Some(doc)),
        );
        out.push('\n');
    }

    // Components and their memberships, once every element they can hold exists (#1517).
    emit_components(doc, &mut out);

    // Custom materials beyond the defaults (by name/color), assigned after bodies exist.
    emit_materials(doc, &mut out);

    out
}

/// Normalize ledger / ephemeral fields so two docs that model the same part compare equal.
pub fn normalize_for_compare(doc: &mut Document) {
    doc.mesh_rev = 0;
    doc.shape_order.clear();
    doc.undo_groups.clear();
    for c in doc.constraints.values_mut() {
        c.dim_offset = None;
        canonicalize_constraint_kind(&mut c.kind);
    }
    for line in doc.lines.values_mut() {
        line.length_dim_offset = None;
    }
    for circle in doc.circles.values_mut() {
        circle.diameter_dim_offset = None;
    }
}

/// Coincident / parallel / equal / tangent don't care which side is `a`. The solver
/// and the geometric-constraint verb don't store them in the same order, so a
/// round-trip that rebuilds the same constraint can fail `PartialEq` on swap alone.
fn canonicalize_constraint_kind(kind: &mut ConstraintKind) {
    let swap = |a: &str, b: &str| a > b;
    match kind {
        ConstraintKind::Coincident { a, b } => {
            if swap(&format!("{a:?}"), &format!("{b:?}")) {
                std::mem::swap(a, b);
            }
        }
        ConstraintKind::Parallel { line_a, line_b }
        | ConstraintKind::Perpendicular { line_a, line_b }
        | ConstraintKind::Equal { line_a, line_b } => {
            if swap(&format!("{line_a:?}"), &format!("{line_b:?}")) {
                std::mem::swap(line_a, line_b);
            }
        }
        ConstraintKind::Tangent { a, b } => {
            if swap(&format!("{a:?}"), &format!("{b:?}")) {
                std::mem::swap(a, b);
            }
        }
        _ => {}
    }
}

/// Trim float noise so `30deg` round-trips as `30`, not `30.000002`.
fn script_f32(v: f32) -> f32 {
    (v * 1e4).round() / 1e4
}

/// `"x"`/`"y"`/`"z"` when this axis is the world origin triad (#1876).
fn global_axis_script_name(origin: glam::Vec3, direction: glam::Vec3) -> Option<&'static str> {
    if origin.length() >= 1e-4 {
        return None;
    }
    let d = direction.normalize_or_zero();
    use crate::construction::GlobalAxis;
    for axis in [GlobalAxis::X, GlobalAxis::Y, GlobalAxis::Z] {
        if (d - axis.direction()).length() < 1e-3 {
            return Some(axis.script_name());
        }
    }
    None
}

/// Live line ordinal whose world segment matches this stored axis.
fn sketch_line_axis_ordinal(
    doc: &Document,
    origin: glam::Vec3,
    direction: glam::Vec3,
) -> Option<usize> {
    let dir = direction.normalize_or_zero();
    if dir.length_squared() < 1e-8 {
        return None;
    }
    doc.lines.iter().enumerate().find_map(|(ord, (_, line))| {
        let (a, b) = crate::face::line_world_endpoints(doc, line)?;
        let line_dir = (b - a).normalize_or_zero();
        if line_dir.length_squared() < 1e-8 {
            return None;
        }
        let collinear = line_dir.dot(dir).abs() > 0.999;
        let on_start = (a - origin).length() < 1e-3;
        let on_end = (b - origin).length() < 1e-3;
        (collinear && (on_start || on_end)).then_some(ord)
    })
}

/// True when `doc` has no user content beyond a fresh default (for import-Lua warning, #1160).
pub fn document_is_blank(doc: &Document) -> bool {
    let mut a = doc.clone();
    let mut b = Document::default();
    normalize_for_compare(&mut a);
    normalize_for_compare(&mut b);
    a == b
}

/// Human-readable differences between two documents. Empty means equal after normalize.
pub fn document_diff(a: &Document, b: &Document) -> Vec<String> {
    let mut a = a.clone();
    let mut b = b.clone();
    normalize_for_compare(&mut a);
    normalize_for_compare(&mut b);
    if a == b {
        return Vec::new();
    }
    let mut diffs = Vec::new();
    macro_rules! count_diff {
        ($field:ident, $label:expr) => {
            let na = a.$field.len();
            let nb = b.$field.len();
            if na != nb {
                diffs.push(format!("{}: {} vs {}", $label, na, nb));
            } else if a.$field != b.$field {
                // #1520: name the first record that differs — "content differs (N entries)"
                // on its own costs a bisect every time an export gap shows up.
                let mut detail = String::new();
                for (i, (va, vb)) in a.$field.values().zip(b.$field.values()).enumerate() {
                    if va != vb {
                        detail = format!("\n    #{i} was {va:?}\n    #{i} got {vb:?}");
                        break;
                    }
                }
                diffs.push(format!(
                    "{} content differs ({} entries){}",
                    $label, na, detail
                ));
            }
        };
    }
    count_diff!(parameters, "parameters");
    count_diff!(sketches, "sketches");
    count_diff!(lines, "lines");
    count_diff!(circles, "circles");
    count_diff!(constraints, "constraints");
    count_diff!(construction_planes, "construction_planes");
    count_diff!(extrusions, "extrusions");
    count_diff!(bodies, "bodies");
    count_diff!(materials, "materials");
    count_diff!(lofts, "lofts");
    count_diff!(revolutions, "revolutions");
    count_diff!(primitives, "primitives");
    count_diff!(sweeps, "sweeps");
    count_diff!(boolean_ops, "boolean_ops");
    count_diff!(move_ops, "move_ops");
    count_diff!(mirror_ops, "mirror_ops");
    count_diff!(repeat_ops, "repeat_ops");
    count_diff!(slice_ops, "slice_ops");
    count_diff!(shell_ops, "shell_ops");
    count_diff!(edge_treatment_ops, "edge_treatment_ops");
    count_diff!(sketch_repeat_ops, "sketch_repeat_ops");
    count_diff!(sketch_offset_ops, "sketch_offset_ops");
    count_diff!(sketch_mirror_ops, "sketch_mirror_ops");
    count_diff!(sketch_vertex_treatment_ops, "sketch_vertex_treatment_ops");
    count_diff!(sketch_slice_ops, "sketch_slice_ops");
    count_diff!(sketch_texts, "sketch_texts");
    count_diff!(drawings, "drawings");
    count_diff!(joints, "joints");
    count_diff!(components, "components");
    count_diff!(units, "units");
    count_diff!(unit_instances, "unit_instances");
    count_diff!(tracing_images, "tracing_images");
    count_diff!(imported_meshes, "imported_meshes");
    if a.default_length_unit != b.default_length_unit {
        diffs.push(format!(
            "default_length_unit: {:?} vs {:?}",
            a.default_length_unit, b.default_length_unit
        ));
    }
    if a.default_angle_unit != b.default_angle_unit {
        diffs.push(format!(
            "default_angle_unit: {:?} vs {:?}",
            a.default_angle_unit, b.default_angle_unit
        ));
    }
    if a.component_members != b.component_members {
        diffs.push("component_members differ".into());
    }
    if diffs.is_empty() {
        diffs.push("documents differ (field-level PartialEq; see normalize_for_compare)".into());
    }
    diffs
}

// ─── emission state ───────────────────────────────────────────────────────────

struct EmitCtx<'a> {
    doc: &'a Document,
    open_sketch: Option<SketchId>,
    /// Geometry produced by ops — skip free emit; the op recreates them.
    generated_lines: HashSet<crate::model::LineKey>,
    generated_circles: HashSet<crate::model::CircleKey>,
    generated_constraints: HashSet<crate::model::ConstraintKey>,
    /// Constraints absorbed into CreateRect / CreateLine / CreateCircle.
    absorbed_constraints: HashSet<crate::model::ConstraintKey>,
    /// Lines already emitted as part of a CreateRect.
    emitted_lines: HashSet<crate::model::LineKey>,
    emitted_circles: HashSet<crate::model::CircleKey>,
    /// Datum planes present at `bearcad.new()` — don't re-emit.
    default_planes: HashSet<crate::model::ConstructionPlaneKey>,
    /// Sketches whose free geometry has been emitted (#1511) — ops must not run first.
    sketch_contents_done: HashSet<SketchId>,
}

impl<'a> EmitCtx<'a> {
    fn new(doc: &'a Document) -> Self {
        let mut generated_lines = HashSet::new();
        let mut generated_circles = HashSet::new();
        let mut generated_constraints = HashSet::new();
        for op in doc.sketch_repeat_ops.values() {
            generated_lines.extend(op.line_outputs.iter().copied());
            generated_circles.extend(op.circle_outputs.iter().copied());
        }
        for op in doc.sketch_offset_ops.values() {
            generated_lines.extend(op.line_outputs.iter().copied());
            generated_circles.extend(op.circle_outputs.iter().copied());
        }
        for op in doc.sketch_mirror_ops.values() {
            generated_lines.extend(op.line_outputs.iter().copied());
            generated_circles.extend(op.circle_outputs.iter().copied());
            generated_constraints.extend(op.constraint_outputs.iter().copied());
        }
        for op in doc.sketch_slice_ops.values() {
            generated_lines.extend(op.line_outputs.iter().copied());
            generated_constraints.extend(op.constraint_outputs.iter().copied());
        }
        for op in doc.sketch_vertex_treatment_ops.values() {
            generated_lines.extend(op.line_outputs.iter().copied());
            generated_lines.extend(op.bridge_outputs.iter().copied());
            generated_constraints.extend(op.constraint_outputs.iter().copied());
        }
        // Default datum planes: first three from a fresh document, matched by frame.
        let mut default_planes = HashSet::new();
        let fresh = Document::default();
        for (k, p) in doc.construction_planes.iter() {
            if p.repeat_instance.is_some() {
                continue;
            }
            for fp in fresh.construction_planes.values() {
                if planes_same_datum(p, fp) {
                    default_planes.insert(k);
                    break;
                }
            }
        }
        Self {
            doc,
            open_sketch: None,
            generated_lines,
            generated_circles,
            generated_constraints,
            absorbed_constraints: HashSet::new(),
            emitted_lines: HashSet::new(),
            emitted_circles: HashSet::new(),
            default_planes,
            sketch_contents_done: HashSet::new(),
        }
    }

    fn close_sketch(&mut self, out: &mut String) {
        if self.open_sketch.take().is_some() {
            out.push_str("bearcad.exit_sketch()\n");
        }
    }

    fn ensure_sketch(&mut self, sketch: SketchId, out: &mut String) {
        if self.open_sketch == Some(sketch) {
            return;
        }
        self.close_sketch(out);
        let Some(s) = self.doc.sketches.get(sketch) else {
            return;
        };
        // Prefer open_sketch when the sketch already exists (re-enter); for a brand-new
        // sketch begin_sketch creates it. At export time every sketch already exists in
        // `doc`, but at *replay* begin_sketch creates. Use begin_sketch for first enter
        // of each sketch (no prior geometry-creating call in this sketch yet).
        let ordinal = self.doc.sketches.keys().position(|k| k == sketch).unwrap_or(0);
        // If this is the first time we touch this sketch and it has a host face, begin_sketch.
        // open_sketch needs the sketch to already exist in the *replay* document, so we
        // always begin_sketch for the first enter; subsequent geometry stays in-session.
        out.push_str(&format!(
            "bearcad.begin_sketch({})\n",
            face_table(&s.face, self.doc)
        ));
        // begin_sketch may create a *new* sketch even when one already exists on the face
        // — for re-exports of multi-geometry sketches we need open_sketch after first
        // geometry batch. The first begin_sketch on a face creates sketch 0, 1, … in order
        // of first visit. If the document already had that sketch as ordinal N and we're
        // visiting sketches in topo order, ordinals match.
        let _ = ordinal;
        if let Some(name) = &s.name {
            out.push_str(&format!(
                "bearcad.set_name({{ kind = \"sketch\", index = {ordinal} }}, {name:?})\n"
            ));
        }
        self.open_sketch = Some(sketch);
    }

    fn emit_node(&mut self, node: HierarchyNode, out: &mut String) {
        match node {
            HierarchyNode::Document
            | HierarchyNode::Drawings
            // A cross-section view is a way of looking, not modelled geometry (#1671); the
            // Lua export describes the model.
            | HierarchyNode::Views
            | HierarchyNode::CrossSection(_)
            | HierarchyNode::SectionPlane { .. }
            | HierarchyNode::UnitChild { .. }
            | HierarchyNode::DrawingDimension { .. }
            | HierarchyNode::DrawingPointDim { .. }
            | HierarchyNode::DrawingLoupe { .. }
            | HierarchyNode::DrawingProjection { .. }
            | HierarchyNode::DrawingAnnotation { .. }
            | HierarchyNode::EdgeTreatment { .. }
            // Components and their memberships are emitted last, once every element they
            // can hold exists (see `emit_components`).
            | HierarchyNode::Component(_) => {}

            // A body is made by the operation above it, but what the *user* set on it
            // afterwards is the body's own (#1517).
            HierarchyNode::Body(key) => {
                let Some(body) = self.doc.bodies.get(key) else {
                    return;
                };
                // A body an operation consumed carries the same `shadow` flag the user's
                // "make this a shadow body" sets, and re-emitting it would make the op
                // refuse its own input ("already consumed by another operation").
                let shadow = body.shadow && !body_is_op_input(self.doc, key);
                if !shadow && body.name.is_none() {
                    return;
                }
                self.close_sketch(out);
                let ord = self.doc.bodies.keys().position(|k| k == key).unwrap_or(0);
                if shadow {
                    out.push_str(&format!(
                        "bearcad.set_body_shadow{{ body = {ord}, shadow = true }}\n"
                    ));
                }
                if let Some(name) = &body.name {
                    out.push_str(&format!(
                        "bearcad.set_name({{ kind = \"body\", index = {ord} }}, {name:?})\n"
                    ));
                }
            }

            HierarchyNode::ConstructionPlane(key) => {
                if self.default_planes.contains(&key) {
                    return;
                }
                let Some(plane) = self.doc.construction_planes.get(key) else {
                    return;
                };
                if plane.repeat_instance.is_some() {
                    return; // produced by a repeat op
                }
                // #1510: `bearcad.plane` adds `offset` on top of `origin`, so the plane's
                // *world* origin is the wrong anchor to emit — replaying it moved the plane
                // by another `offset`. Emit what the definition actually says: the
                // offset-from-parent form when the anchor is another construction plane,
                // otherwise the anchor face's own origin and normal.
                let offset = plane.definition.offset_mm;
                match &plane.definition.anchor {
                    crate::model::PlaneAnchor::Face {
                        origin,
                        normal,
                        label,
                    } => {
                        let parent = (label == "Construction plane")
                            .then(|| {
                                self.doc.construction_planes.iter().position(|(_, p)| {
                                    (p.origin - *origin).length() < 1e-5
                                        && (p.normal - *normal).length() < 1e-5
                                })
                            })
                            .flatten();
                        match parent {
                            Some(from) => out.push_str(&format!(
                                "bearcad.plane{{ offset = {offset}, from = {from} }}\n"
                            )),
                            None => out.push_str(&format!(
                                "bearcad.plane{{ offset = {offset}, origin = {{{}, {}, {}}}, normal = {{{}, {}, {}}} }}\n",
                                origin.x, origin.y, origin.z, normal.x, normal.y, normal.z
                            )),
                        }
                    }
                    crate::model::PlaneAnchor::Axis {
                        origin,
                        direction,
                        ..
                    } => {
                        let angle = script_f32(plane.definition.angle_deg);
                        // A sketch line through the origin along X/Y/Z is still that
                        // line, not the world triad — match the line first (#1876).
                        if let Some(line) =
                            sketch_line_axis_ordinal(self.doc, *origin, *direction)
                        {
                            out.push_str(&format!(
                                "bearcad.plane{{ offset = {offset}, axis = {{ line = {line} }}, angle = {angle} }}\n"
                            ));
                        } else if let Some(name) = global_axis_script_name(*origin, *direction) {
                            out.push_str(&format!(
                                "bearcad.plane{{ offset = {offset}, axis = {name:?}, angle = {angle} }}\n"
                            ));
                        } else {
                            out.push_str(
                                "-- skipped: construction plane anchored on an unmatched axis\n",
                            );
                            return;
                        }
                    }
                }
                if let Some(name) = &plane.name {
                    let ord = self
                        .doc
                        .construction_planes
                        .keys()
                        .position(|k| k == key)
                        .unwrap_or(0);
                    out.push_str(&format!(
                        "bearcad.set_name({{ kind = \"plane\", index = {ord} }}, {name:?})\n"
                    ));
                }
            }

            HierarchyNode::Sketch(sketch) => {
                // Outside an active sketch session the hierarchy hides free lines/circles/
                // constraints (they're only listed while editing). Emit them here when we
                // first visit the sketch node (#1159).
                self.emit_sketch_contents(sketch, out);
            }

            HierarchyNode::Line(key) => {
                self.emit_line(key, out);
            }

            HierarchyNode::Circle(key) => {
                self.emit_circle(key, out);
            }

            HierarchyNode::Constraint(key) => {
                self.emit_constraint_node(key, out);
            }

            HierarchyNode::SketchText(key) => {
                self.emit_sketch_text(key, out);
            }

            HierarchyNode::Extrusion(key) => {
                self.close_sketch(out);
                let Some(instr) = instruction_for_extrusion(self.doc, key) else {
                    return;
                };
                out.push_str(&instr.as_lua_in(Some(self.doc)));
                out.push('\n');
                if let Some(e) = self.doc.extrusions.get(key) {
                    if let Some(name) = &e.name {
                        let ord = self
                            .doc
                            .extrusions
                            .keys()
                            .position(|k| k == key)
                            .unwrap_or(0);
                        out.push_str(&format!(
                            "bearcad.set_name({{ kind = \"extrusion\", index = {ord} }}, {name:?})\n"
                        ));
                    }
                    // Legacy per-extrusion edge treatments.
                    if !e.edge_treatments.is_empty() {
                        for et in &e.edge_treatments {
                            let edges = vec![(
                                crate::script::TreatableSolidRef::Extrusion(
                                    self.doc
                                        .extrusions
                                        .keys()
                                        .position(|k| k == key)
                                        .unwrap_or(0),
                                ),
                                et.edge,
                            )];
                            let instr = Instruction::EdgeTreatment {
                                edges,
                                kind: et.kind,
                                amount: et.amount,
                                expression: String::new(),
                            };
                            out.push_str(&instr.as_lua_in(Some(self.doc)));
                            out.push('\n');
                        }
                    }
                }
            }

            HierarchyNode::Loft(key) => {
                self.close_sketch(out);
                if let Some(instr) = instruction_for_loft(self.doc, key) {
                    out.push_str(&instr.as_lua_in(Some(self.doc)));
                    out.push('\n');
                }
            }
            HierarchyNode::Revolution(key) => {
                self.close_sketch(out);
                if let Some(instr) = instruction_for_revolution(self.doc, key) {
                    out.push_str(&instr.as_lua_in(Some(self.doc)));
                    out.push('\n');
                }
            }
            HierarchyNode::SweepOp(key) => {
                self.close_sketch(out);
                if let Some(instr) = instruction_for_sweep(self.doc, key) {
                    out.push_str(&instr.as_lua_in(Some(self.doc)));
                    out.push('\n');
                }
            }
            HierarchyNode::Shape(key) => {
                self.close_sketch(out);
                if let Some(shape) = self.doc.primitives.get(key) {
                    // Reuse Instruction::Shape via as_lua path — Shape renders via shape_lua_call
                    // only when Instruction is Shape { shape }.
                    out.push_str(
                        &Instruction::Shape {
                            shape: shape.clone(),
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }

            HierarchyNode::BooleanOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.boolean_ops.get(key) {
                    let a = body_ords(self.doc, &op.a);
                    let b = body_ords(self.doc, &op.b);
                    out.push_str(
                        &Instruction::CreateBooleanOp {
                            kind: op.kind,
                            a,
                            b,
                            keep_b: op.keep_b,
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }
            HierarchyNode::MoveOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.move_ops.get(key) {
                    out.push_str(
                        &Instruction::CreateMoveOp {
                            targets: body_ords(self.doc, &op.targets),
                            images: op
                                .image_targets
                                .iter()
                                .filter_map(|k| {
                                    self.doc.tracing_images.keys().position(|i| i == *k)
                                })
                                .collect(),
                            tx: op.tx.clone(),
                            ty: op.ty.clone(),
                            tz: op.tz.clone(),
                            rx: op.rx.clone(),
                            ry: op.ry.clone(),
                            rz: op.rz.clone(),
                            roll_angle: op.roll_angle.clone(),
                            face_flip: op.face_flip,
                            face_spin: op.face_spin.clone(),
                            face_offset: op.face_offset.clone(),
                            start_point_a: op.start_point_a.clone(),
                            end_point_a: op.end_point_a.clone(),
                            start_point_b: op.start_point_b.clone(),
                            end_point_b: op.end_point_b.clone(),
                            start_point_c: op.start_point_c.clone(),
                            end_point_c: op.end_point_c.clone(),
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }
            HierarchyNode::MirrorOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.mirror_ops.get(key) {
                    out.push_str(
                        &Instruction::CreateMirrorOp {
                            plane: op.plane.clone(),
                            targets: body_ords(self.doc, &op.targets),
                            mode: op.mode,
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }
            HierarchyNode::RepeatOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.repeat_ops.get(key) {
                    out.push_str(
                        &Instruction::CreateRepeatOp {
                            targets: body_ords(self.doc, &op.targets),
                            axis: op.axis,
                            around_axis: op.around_axis,
                            flip: op.flip,
                            mode: op.mode,
                            count: op.count.clone(),
                            spacing: op.spacing.clone(),
                            length: op.length.clone(),
                            length_target: op.length_target.clone(),
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }
            HierarchyNode::SliceOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.slice_ops.get(key) {
                    out.push_str(
                        &Instruction::CreateSliceOp {
                            targets: body_ords(self.doc, &op.targets),
                            cutters: op.cutters.clone(),
                            extend_infinite: op.extend_infinite,
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }
            HierarchyNode::ShellOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.shell_ops.get(key) {
                    out.push_str(
                        &Instruction::CreateShellOp {
                            targets: body_ords(self.doc, &op.targets),
                            open_faces: op.open_faces.clone(),
                            thickness: op.thickness.clone(),
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                    if let Some(name) = &op.name {
                        let ord = self
                            .doc
                            .shell_ops
                            .keys()
                            .position(|k| k == key)
                            .unwrap_or(0);
                        out.push_str(&format!(
                            "bearcad.set_name({{ kind = \"shell_op\", index = {ord} }}, {name:?})\n"
                        ));
                    }
                }
            }
            HierarchyNode::EdgeTreatmentOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.edge_treatment_ops.get(key) {
                    let edges: Vec<_> = op
                        .edges
                        .iter()
                        .filter_map(|te| {
                            let host = match te.solid {
                                crate::model::TreatableSolid::Extrusion(e) => {
                                    crate::script::TreatableSolidRef::Extrusion(
                                        self.doc.extrusions.keys().position(|k| k == e)?,
                                    )
                                }
                                crate::model::TreatableSolid::Primitive(p) => {
                                    crate::script::TreatableSolidRef::Primitive(
                                        self.doc.primitives.keys().position(|k| k == p)?,
                                    )
                                }
                            };
                            Some((host, te.edge))
                        })
                        .collect();
                    if !edges.is_empty() {
                        out.push_str(
                            &Instruction::EdgeTreatment {
                                edges,
                                kind: op.kind,
                                amount: op.amount,
                                expression: op.expression.clone(),
                            }
                            .as_lua_in(Some(self.doc)),
                        );
                        out.push('\n');
                    }
                }
            }
            HierarchyNode::Joint(key) => {
                self.close_sketch(out);
                if let Some(joint) = self.doc.joints.get(key) {
                    out.push_str(
                        &Instruction::CreateJointOp {
                            members: joint.members.clone(),
                            base: joint.base,
                            kind: joint.kind.clone(),
                            placement: joint.placement.clone(),
                            frame: joint.frame.clone(),
                            position: joint.position.clone(),
                            position2: joint.position2.clone(),
                            position3: joint.position3.clone(),
                            limits: joint.limits.clone(),
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }

            HierarchyNode::SketchOffsetOp(key) => {
                if let Some(op) = self.doc.sketch_offset_ops.get(key) {
                    self.enter_sketch(op.sketch, out);
                    let sketch = self
                        .doc
                        .sketches
                        .keys()
                        .position(|k| k == op.sketch)
                        .unwrap_or(0);
                    let lines = keys_ords(self.doc.lines.keys(), &op.line_targets);
                    let circles = keys_ords(self.doc.circles.keys(), &op.circle_targets);
                    let constr = if op.construction {
                        ", construction = true"
                    } else {
                        ""
                    };
                    out.push_str(&format!(
                        "bearcad.offset_sketch{{ sketch = {sketch}, lines = {{{}}}, circles = {{{}}}, distance = {:?}{constr} }}\n",
                        list_usizes(&lines),
                        list_usizes(&circles),
                        op.distance
                    ));
                }
            }
            HierarchyNode::SketchRepeatOp(key) => {
                if let Some(op) = self.doc.sketch_repeat_ops.get(key) {
                    self.enter_sketch(op.sketch, out);
                    let sketch = self
                        .doc
                        .sketches
                        .keys()
                        .position(|k| k == op.sketch)
                        .unwrap_or(0);
                    let lines = keys_ords(self.doc.lines.keys(), &op.line_targets);
                    let circles = keys_ords(self.doc.circles.keys(), &op.circle_targets);
                    let mode = match op.mode {
                        crate::model::RepeatMode::CountGap => "count_gap",
                        crate::model::RepeatMode::CountFitEnds => "count_fit_ends",
                        crate::model::RepeatMode::CountFitCenters => "count_fit_centers",
                        crate::model::RepeatMode::FillGap => "fill_gap",
                        crate::model::RepeatMode::FillPitch => "fill_pitch",
                        crate::model::RepeatMode::FillMaxPitch => "fill_max_pitch",
                        crate::model::RepeatMode::CountPitch => "count_pitch",
                        crate::model::RepeatMode::FillGapSpan => "fill_gap_span",
                        crate::model::RepeatMode::FillPitchSpan => "fill_pitch_span",
                    };
                    // #1513: the reader takes `dir = {du, dv}`, not two scalar keys, and an
                    // empty expression string is not the same as an omitted field.
                    let mut parts = format!(
                        "sketch = {sketch}, lines = {{{}}}, circles = {{{}}}, dir = {{{}, {}}}, mode = {mode:?}",
                        list_usizes(&lines),
                        list_usizes(&circles),
                        op.dir_u,
                        op.dir_v,
                    );
                    for (key, expr) in [
                        ("count", &op.count),
                        ("spacing", &op.spacing),
                        ("length", &op.length),
                    ] {
                        if !expr.trim().is_empty() {
                            parts.push_str(&format!(", {key} = {expr:?}"));
                        }
                    }
                    out.push_str(&format!("bearcad.repeat_sketch{{ {parts} }}\n"));
                }
            }
            HierarchyNode::SketchMirrorOp(key) => {
                if let Some(op) = self.doc.sketch_mirror_ops.get(key) {
                    self.enter_sketch(op.sketch, out);
                    let sketch = self
                        .doc
                        .sketches
                        .keys()
                        .position(|k| k == op.sketch)
                        .unwrap_or(0);
                    let mirror = sketch_mirror_axis_lua(self.doc, op.line);
                    let lines = keys_ords(self.doc.lines.keys(), &op.line_targets);
                    let circles = keys_ords(self.doc.circles.keys(), &op.circle_targets);
                    out.push_str(&format!(
                        "bearcad.mirror_sketch{{ sketch = {sketch}, line = {mirror}, lines = {{{}}}, circles = {{{}}} }}\n",
                        list_usizes(&lines),
                        list_usizes(&circles)
                    ));
                }
            }
            HierarchyNode::SketchSliceOp(key) => {
                if let Some(op) = self.doc.sketch_slice_ops.get(key) {
                    self.enter_sketch(op.sketch, out);
                    let sketch = self
                        .doc
                        .sketches
                        .keys()
                        .position(|k| k == op.sketch)
                        .unwrap_or(0);
                    let lines = keys_ords(self.doc.lines.keys(), &op.line_targets);
                    let cutters = keys_ords(self.doc.lines.keys(), &op.cutter_lines);
                    let circles = keys_ords(self.doc.circles.keys(), &op.circle_targets);
                    out.push_str(&format!(
                        "bearcad.slice_sketch{{ sketch = {sketch}, lines = {{{}}}, cutters = {{{}}}, circles = {{{}}} }}\n",
                        list_usizes(&lines),
                        list_usizes(&cutters),
                        list_usizes(&circles)
                    ));
                }
            }
            HierarchyNode::SketchVertexTreatmentOp(key) => {
                // One call per op (#1519): `point` for a single corner, `points` for several.
                // Mixed kind/amount corners (a connected chamfer+fillet region) still emit
                // one call per homogeneous group so replay can rebuild the same op.
                if let Some(op) = self.doc.sketch_vertex_treatment_ops.get(key) {
                    self.enter_sketch(op.sketch, out);
                    let mut groups: Vec<(
                        crate::model::VertexTreatmentKind,
                        String,
                        Vec<ConstraintPoint>,
                    )> = Vec::new();
                    for corner in &op.corners {
                        let Some(&la) = op.line_targets.get(corner.a) else {
                            continue;
                        };
                        let point = ConstraintPoint::LineEndpoint {
                            line: la,
                            end: corner.a_end,
                        };
                        if let Some(group) = groups.iter_mut().find(|(k, a, _)| {
                            *k == corner.kind && a == &corner.amount
                        }) {
                            group.2.push(point);
                        } else {
                            groups.push((corner.kind, corner.amount.clone(), vec![point]));
                        }
                    }
                    for (kind, amount, points) in groups {
                        out.push_str(
                            &Instruction::VertexTreatment {
                                points,
                                kind,
                                amount,
                            }
                            .as_lua_in(Some(self.doc)),
                        );
                        out.push('\n');
                    }
                }
            }

            HierarchyNode::Drawing(key) => {
                self.close_sketch(out);
                let Some(d) = self.doc.drawings.get(key) else {
                    return;
                };
                let dord = self.doc.drawings.keys().position(|k| k == key).unwrap_or(0);
                out.push_str(
                    &Instruction::CreateDrawing {
                        name: d.name.clone(),
                    }
                    .as_lua_in(Some(self.doc)),
                );
                out.push('\n');
                out.push_str(
                    &Instruction::SetDrawingPage {
                        drawing: dord,
                        width_mm: Some(d.page_width_mm),
                        height_mm: Some(d.page_height_mm),
                        margin_mm: Some(d.margin_mm),
                    }
                    .as_lua_in(Some(self.doc)),
                );
                out.push('\n');
                for (vi, view) in d.views.iter().enumerate() {
                    if let Some(sketch) = view.sketch {
                        let sord = self
                            .doc
                            .sketches
                            .keys()
                            .position(|k| k == sketch)
                            .unwrap_or(0);
                        out.push_str(
                            &Instruction::AddDrawingSketchView {
                                drawing: dord,
                                sketch: sord,
                                orientation: view.orientation,
                            }
                            .as_lua_in(Some(self.doc)),
                        );
                        out.push('\n');
                    } else {
                        let bords: Vec<usize> = view
                            .bodies
                            .iter()
                            .map(|bi| {
                                self.doc
                                    .bodies
                                    .keys()
                                    .position(|k| k == *bi)
                                    .unwrap_or(0)
                            })
                            .collect();
                        out.push_str(
                            &Instruction::AddDrawingView {
                                drawing: dord,
                                bodies: bords,
                                orientation: view.orientation,
                            }
                            .as_lua_in(Some(self.doc)),
                        );
                        out.push('\n');
                    }
                    out.push_str(
                        &Instruction::MoveDrawingView {
                            drawing: dord,
                            view: vi,
                            x: view.pos_x,
                            y: view.pos_y,
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                    // Card size (#1207): only emit when it differs from the default.
                    let def = crate::drawing::CELL_FRAC;
                    if (view.size_x - def).abs() > 1e-4 || (view.size_y - def).abs() > 1e-4 {
                        out.push_str(
                            &Instruction::SetDrawingViewSize {
                                drawing: dord,
                                view: vi,
                                size_x: view.size_x,
                                size_y: view.size_y,
                            }
                            .as_lua_in(Some(self.doc)),
                        );
                        out.push('\n');
                    }
                }
                // #1516: `bearcad.drawing{}` seeds its own title annotation. Re-emitting it
                // as a `drawing_text` left the replayed drawing with two overlapping titles,
                // and every further round trip added another.
                let seeded = seeded_drawing_title(d, dord);
                for (ai, ann) in d.annotations.iter() {
                    if Some(ai) == seeded {
                        continue;
                    }
                    out.push_str(
                        &Instruction::AddDrawingAnnotation {
                            drawing: dord,
                            text: ann.text.clone(),
                            x: ann.pos_x,
                            y: ann.pos_y,
                            wrap: ann.wrap_frac,
                        }
                        .as_lua_in(Some(self.doc)),
                    );
                    out.push('\n');
                }
            }

            HierarchyNode::Image(key) => {
                // Path-based import can't be reconstructed without the original file bytes.
                let _ = key;
                out.push_str(
                    "-- skipped: tracing image (needs original file path; import_image not reconstructable from document alone)\n",
                );
            }
            HierarchyNode::UnitInstance(key) => {
                let _ = key;
                out.push_str(
                    "-- skipped: unit instance (needs source .bearcad path)\n",
                );
            }
        }
    }

    /// All free geometry in a sketch: lines (with rect folding), circles, texts, then
    /// remaining constraints. Sketch ops that nest under the sketch are still visited as
    /// their own hierarchy nodes later.
    fn emit_sketch_contents(&mut self, sketch: SketchId, out: &mut String) {
        if !self.sketch_contents_done.insert(sketch) {
            return;
        }
        // Rectangles first (each consumes four lines).
        let line_keys: Vec<_> = self
            .doc
            .lines
            .iter()
            .filter(|(k, l)| {
                l.sketch == sketch
                    && !self.generated_lines.contains(k)
                    && !self.emitted_lines.contains(k)
                    && l.projection.is_none()
            })
            .map(|(k, _)| k)
            .collect();
        for key in line_keys {
            if self.emitted_lines.contains(&key) {
                continue;
            }
            if let Some(rect) = find_rect_from_line(self.doc, key, &self.generated_lines) {
                self.emit_rect(rect, out);
            }
        }
        // Remaining free lines.
        let line_keys: Vec<_> = self
            .doc
            .lines
            .iter()
            .filter(|(k, l)| l.sketch == sketch && !self.generated_lines.contains(k))
            .map(|(k, _)| k)
            .collect();
        for key in line_keys {
            self.emit_line(key, out);
        }
        self.emit_projections(sketch, out);
        let circle_keys: Vec<_> = self
            .doc
            .circles
            .iter()
            .filter(|(k, c)| c.sketch == sketch && !self.generated_circles.contains(k))
            .map(|(k, _)| k)
            .collect();
        for key in circle_keys {
            self.emit_circle(key, out);
        }
        let text_keys: Vec<_> = self
            .doc
            .sketch_texts
            .iter()
            .filter(|(_, t)| t.sketch == sketch)
            .map(|(k, _)| k)
            .collect();
        for key in text_keys {
            self.emit_sketch_text(key, out);
        }
        let constraint_keys: Vec<_> = self
            .doc
            .constraints
            .iter()
            .filter(|(k, c)| {
                c.sketch == sketch
                    && !self.generated_constraints.contains(k)
                    && !self.absorbed_constraints.contains(k)
            })
            .map(|(k, _)| k)
            .collect();
        for key in constraint_keys {
            self.emit_constraint_node(key, out);
        }
        // Empty sketch still needs to exist.
        if self.open_sketch != Some(sketch)
            && !self.doc.lines.values().any(|l| l.sketch == sketch)
            && !self.doc.circles.values().any(|c| c.sketch == sketch)
            && !self.doc.sketch_texts.values().any(|t| t.sketch == sketch)
        {
            self.ensure_sketch(sketch, out);
        }
    }

    /// `bearcad.project{...}` for whatever outside geometry a sketch references (#1517).
    /// Projected lines are *derived*, so they can't be emitted as `bearcad.line` calls — the
    /// export used to skip them and lose a projection-only sketch entirely.
    fn emit_projections(&mut self, sketch: SketchId, out: &mut String) {
        use crate::model::ProjectionSource;
        let mut bodies: Vec<usize> = Vec::new();
        let mut planes: Vec<usize> = Vec::new();
        let mut unresolved = false;
        for line in self.doc.lines.values() {
            if line.sketch != sketch {
                continue;
            }
            match &line.projection {
                None => {}
                Some(ProjectionSource::BodyEdge { body, .. }) => {
                    if let Some(ord) = self.doc.bodies.keys().position(|k| k == *body) {
                        if !bodies.contains(&ord) {
                            bodies.push(ord);
                        }
                    }
                }
                Some(ProjectionSource::Plane { plane }) => {
                    if let Some(ord) = self.doc.construction_planes.keys().position(|k| k == *plane)
                    {
                        if !planes.contains(&ord) {
                            planes.push(ord);
                        }
                    }
                }
                // A unit instance's face edge has no `bearcad.project` spelling.
                Some(ProjectionSource::UnitEdge { .. }) => unresolved = true,
            }
        }
        if bodies.is_empty() && planes.is_empty() && !unresolved {
            return;
        }
        self.ensure_sketch(sketch, out);
        if !bodies.is_empty() {
            out.push_str(&format!(
                "bearcad.project{{ bodies = {{{}}} }}\n",
                list_usizes(&bodies)
            ));
        }
        if !planes.is_empty() {
            out.push_str(&format!(
                "bearcad.project{{ planes = {{{}}} }}\n",
                list_usizes(&planes)
            ));
        }
        if unresolved {
            out.push_str("-- skipped: projection of an imported unit's edge (no scripting verb)\n");
        }
    }

    /// Open `sketch` with everything it holds already created (#1511). A sketch-level op
    /// is a separate hierarchy node that can be visited before the Sketch node, and calling
    /// `mirror_sketch`/`fillet_vertex`/… before its lines exist makes the replay die on
    /// "no line 0". Emitting the sketch's contents on first reference fixes the order
    /// whichever way the element list happens to run.
    fn enter_sketch(&mut self, sketch: SketchId, out: &mut String) {
        self.emit_sketch_contents(sketch, out);
        self.ensure_sketch(sketch, out);
    }

    fn emit_line(&mut self, key: crate::model::LineKey, out: &mut String) {
        if self.generated_lines.contains(&key) || self.emitted_lines.contains(&key) {
            return;
        }
        let Some(line) = self.doc.lines.get(key) else {
            return;
        };
        if line.projection.is_some() {
            return;
        }
        if let Some(rect) = find_rect_from_line(self.doc, key, &self.generated_lines) {
            self.emit_rect(rect, out);
            return;
        }
        self.ensure_sketch(line.sketch, out);
        let dim = line_length_expr(self.doc, key);
        if let Some(ck) = line_length_constraint_key(self.doc, key) {
            self.absorbed_constraints.insert(ck);
        }
        // #1518: export the pre-solve seed so replay follows the same solve trajectory.
        let (x0, y0, x1, y1) = line.export_endpoints();
        let instr = Instruction::CreateLine {
            x0,
            y0,
            x1,
            y1,
            bezier: line.bezier,
            dimension: dim,
        };
        out.push_str(&instr.as_lua_in(Some(self.doc)));
        out.push('\n');
        self.emitted_lines.insert(key);
        if line.construction {
            let ord = self.doc.lines.keys().position(|k| k == key).unwrap_or(0);
            out.push_str(&format!(
                "bearcad.set_construction({{ kind = \"line\", index = {ord} }}, true)\n"
            ));
        }
        if let Some(name) = &line.name {
            let ord = self.doc.lines.keys().position(|k| k == key).unwrap_or(0);
            out.push_str(&format!(
                "bearcad.set_name({{ kind = \"line\", index = {ord} }}, {name:?})\n"
            ));
        }
    }

    fn emit_circle(&mut self, key: crate::model::CircleKey, out: &mut String) {
        if self.generated_circles.contains(&key) || self.emitted_circles.contains(&key) {
            return;
        }
        let Some(circle) = self.doc.circles.get(key) else {
            return;
        };
        self.ensure_sketch(circle.sketch, out);
        let (cx, cy, r) = circle.export_center_radius();
        let diameter_expr =
            circle_diameter_expr(self.doc, key).or_else(|| Some((r * 2.0).to_string()));
        if let Some(ck) = circle_diameter_constraint_key(self.doc, key) {
            self.absorbed_constraints.insert(ck);
        }
        let instr = Instruction::CreateCircle {
            cx,
            cy,
            r,
            diameter_expr,
        };
        out.push_str(&instr.as_lua_in(Some(self.doc)));
        out.push('\n');
        self.emitted_circles.insert(key);
        if circle.construction {
            let ord = self.doc.circles.keys().position(|k| k == key).unwrap_or(0);
            out.push_str(&format!(
                "bearcad.set_construction({{ kind = \"circle\", index = {ord} }}, true)\n"
            ));
        }
        if let Some(name) = &circle.name {
            let ord = self.doc.circles.keys().position(|k| k == key).unwrap_or(0);
            out.push_str(&format!(
                "bearcad.set_name({{ kind = \"circle\", index = {ord} }}, {name:?})\n"
            ));
        }
    }

    fn emit_constraint_node(&mut self, key: crate::model::ConstraintKey, out: &mut String) {
        if self.generated_constraints.contains(&key) || self.absorbed_constraints.contains(&key) {
            return;
        }
        let Some(c) = self.doc.constraints.get(key) else {
            return;
        };
        match &c.kind {
            ConstraintKind::Distance {
                target: DistanceTarget::LineLength(li),
            } if self.emitted_lines.contains(li) => {
                self.absorbed_constraints.insert(key);
                return;
            }
            ConstraintKind::Distance {
                target: DistanceTarget::CircleDiameter(ci),
            } if self.emitted_circles.contains(ci) => {
                self.absorbed_constraints.insert(key);
                return;
            }
            _ => {}
        }
        // #1514: `offset_sketch`/`repeat_sketch` have no `constraint_outputs` field, so the
        // coincidences chaining their generated lines look like free constraints. Anything
        // that only touches op-generated geometry belongs to the op, not to the script.
        if self.constraint_is_generated(c) {
            return;
        }
        self.ensure_sketch(c.sketch, out);
        emit_constraint(self.doc, c, out);
        self.absorbed_constraints.insert(key);
    }

    /// True when every line/circle a constraint names was produced by a sketch op — the op
    /// re-creates the constraint on replay, and the entities don't exist before it runs.
    fn constraint_is_generated(&self, c: &crate::model::Constraint) -> bool {
        let (lines, circles) = constraint_refs(&c.kind);
        if lines.is_empty() && circles.is_empty() {
            return false;
        }
        lines.iter().all(|l| self.generated_lines.contains(l))
            && circles.iter().all(|c| self.generated_circles.contains(c))
    }

    fn emit_sketch_text(&mut self, key: crate::model::SketchTextKey, out: &mut String) {
        let Some(t) = self.doc.sketch_texts.get(key) else {
            return;
        };
        self.ensure_sketch(t.sketch, out);
        let size = if t.size_expr.trim().is_empty() {
            t.size.to_string()
        } else {
            t.size_expr.clone()
        };
        let instr = Instruction::CreateSketchText {
            text: t.text.clone(),
            font: Some(t.font_family.clone()),
            bold: t.bold,
            italic: t.italic,
            underline: t.underline,
            size,
            x: t.origin.0,
            y: t.origin.1,
            rotation_deg: t.rotation.to_degrees(),
            wrap: t.wrap_width,
            flip: t.flip,
        };
        out.push_str(&instr.as_lua_in(Some(self.doc)));
        out.push('\n');
    }

    fn emit_rect(&mut self, rect: RectGroup, out: &mut String) {
        let Some(line0) = self.doc.lines.get(rect.lines[0]) else {
            return;
        };
        self.ensure_sketch(line0.sketch, out);
        let w_expr = symbolic_expr(line_length_expr(self.doc, rect.lines[0]));
        let h_expr = symbolic_expr(line_length_expr(self.doc, rect.lines[1]));
        for &lk in &rect.lines {
            self.emitted_lines.insert(lk);
            if let Some(ck) = line_length_constraint_key(self.doc, lk) {
                self.absorbed_constraints.insert(ck);
            }
        }
        for ck in &rect.constraints {
            self.absorbed_constraints.insert(*ck);
        }
        // Prefer the bottom/right edges' seeds so a later constraint solve doesn't
        // bake post-solve coordinates into the rect call (#1518).
        let (x, y, w, h) = rect_export_xywh(self.doc, &rect);
        let instr = Instruction::CreateRect {
            x,
            y,
            width: w,
            height: h,
            width_expr: w_expr,
            height_expr: h_expr,
        };
        out.push_str(&instr.as_lua_in(Some(self.doc)));
        out.push('\n');
        // #1515: `bearcad.rect` folds four lines into one call, so anything set on an
        // individual edge — its name, its construction flag — has to follow the call.
        for &lk in &rect.lines {
            let Some(line) = self.doc.lines.get(lk) else {
                continue;
            };
            if !line.construction && line.name.is_none() {
                continue;
            }
            let ord = self.doc.lines.keys().position(|k| k == lk).unwrap_or(0);
            if line.construction {
                out.push_str(&format!(
                    "bearcad.set_construction({{ kind = \"line\", index = {ord} }}, true)\n"
                ));
            }
            if let Some(name) = &line.name {
                out.push_str(&format!(
                    "bearcad.set_name({{ kind = \"line\", index = {ord} }}, {name:?})\n"
                ));
            }
        }
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Every line and circle a constraint refers to, for deciding whether it is op-generated.
fn constraint_refs(
    kind: &ConstraintKind,
) -> (Vec<crate::model::LineKey>, Vec<crate::model::CircleKey>) {
    let mut lines = Vec::new();
    let mut circles = Vec::new();
    fn point(
        p: &ConstraintPoint,
        lines: &mut Vec<crate::model::LineKey>,
        circles: &mut Vec<crate::model::CircleKey>,
    ) {
        match p {
            ConstraintPoint::LineEndpoint { line, .. } => lines.push(*line),
            ConstraintPoint::CircleCenter(c) => circles.push(*c),
            _ => {}
        }
    }
    fn line_ref(l: &ConstraintLine, lines: &mut Vec<crate::model::LineKey>) {
        if let ConstraintLine::Line(i) = l {
            lines.push(*i);
        }
    }
    fn entity(
        e: &ConstraintEntity,
        lines: &mut Vec<crate::model::LineKey>,
        circles: &mut Vec<crate::model::CircleKey>,
    ) {
        match e {
            ConstraintEntity::Point(p) => point(p, lines, circles),
            ConstraintEntity::Line(l) => line_ref(l, lines),
            ConstraintEntity::Circle(c) => circles.push(*c),
            // Body-fixed geometry: no sketch entity of its own to emit.
            ConstraintEntity::FaceCircle { .. } | ConstraintEntity::Origin => {}
        }
    }
    match kind {
        ConstraintKind::Distance { target } => match target {
            DistanceTarget::LineLength(i) => lines.push(*i),
            DistanceTarget::CircleDiameter(i) => circles.push(*i),
            DistanceTarget::PointPointDistance { anchor, mover, .. } => {
                point(anchor, &mut lines, &mut circles);
                point(mover, &mut lines, &mut circles);
            }
            DistanceTarget::PointLineDistance { point: p, line, .. } => {
                point(p, &mut lines, &mut circles);
                line_ref(line, &mut lines);
            }
            DistanceTarget::LineLineDistance { line_a, line_b, .. } => {
                line_ref(line_a, &mut lines);
                line_ref(line_b, &mut lines);
            }
        },
        ConstraintKind::Angle { line_a, line_b, .. }
        | ConstraintKind::Parallel { line_a, line_b }
        | ConstraintKind::Perpendicular { line_a, line_b }
        | ConstraintKind::Equal { line_a, line_b } => {
            line_ref(line_a, &mut lines);
            line_ref(line_b, &mut lines);
        }
        ConstraintKind::Coincident { a, b } => {
            entity(a, &mut lines, &mut circles);
            entity(b, &mut lines, &mut circles);
        }
        ConstraintKind::Midpoint { point: p, line } => {
            point(p, &mut lines, &mut circles);
            line_ref(line, &mut lines);
        }
        ConstraintKind::Tangent { a, b } => {
            point(a, &mut lines, &mut circles);
            point(b, &mut lines, &mut circles);
        }
        ConstraintKind::TangentCircle { circle, other } => {
            circles.push(*circle);
            match other {
                crate::model::TangentTarget::Circle(o) => circles.push(*o),
                crate::model::TangentTarget::Line(line) => line_ref(line, &mut lines),
            }
        }
    }
    (lines, circles)
}

/// True when some operation takes `body` as an input. Consumption shadows a body, so this
/// is what separates "the user made it a shadow body" from "an op used it up" (#1517).
/// The annotation `CreateDrawing` seeded as this drawing's title, when it is still exactly
/// what that action produces (#1516). Anything the user has since changed about it is a real
/// annotation and exports like one.
fn seeded_drawing_title(
    d: &crate::model::Drawing,
    ordinal: usize,
) -> Option<crate::model::AnnotationKey> {
    let (key, first) = d.annotations.iter().next()?;
    let fresh = crate::model::Drawing::default();
    let text = d
        .name
        .clone()
        .unwrap_or_else(|| format!("Drawing {ordinal}"));
    let pos_x = (fresh.margin_mm / fresh.page_width_mm).clamp(0.0, 0.4);
    let same = first.text == text
        && (first.pos_x - pos_x).abs() < 1e-5
        && (first.pos_y - 0.02).abs() < 1e-5
        && (first.size_frac - 0.028).abs() < 1e-5
        && first.wrap_frac.is_none();
    same.then_some(key)
}

fn body_is_op_input(doc: &Document, body: crate::model::BodyKey) -> bool {
    let listed = |v: &[crate::model::BodyKey]| v.contains(&body);
    if doc
        .boolean_ops
        .values()
        .any(|op| listed(&op.a) || listed(&op.b))
    {
        return true;
    }
    if doc.move_ops.values().any(|op| listed(&op.targets))
        || doc.mirror_ops.values().any(|op| listed(&op.targets))
        || doc.repeat_ops.values().any(|op| listed(&op.targets))
        || doc.slice_ops.values().any(|op| listed(&op.targets))
        || doc.shell_ops.values().any(|op| listed(&op.targets))
    {
        return true;
    }
    let loft_bodies = doc.lofts.values().any(|op| match &op.mode {
        crate::model::LoftMode::NewBody => false,
        crate::model::LoftMode::AddTo(b) | crate::model::LoftMode::Cut(b) => listed(b),
    });
    let revolve_bodies = doc.revolutions.values().any(|op| match &op.mode {
        crate::model::RevolveMode::NewBody => false,
        crate::model::RevolveMode::AddTo(b) | crate::model::RevolveMode::Cut(b) => listed(b),
    });
    let sweep_bodies = doc.sweeps.values().any(|op| match &op.mode {
        crate::model::SweepMode::NewBody => false,
        crate::model::SweepMode::AddTo(b) | crate::model::SweepMode::Cut(b) => listed(b),
    });
    loft_bodies
        || revolve_bodies
        || sweep_bodies
        || crate::model::body_is_fuse_host(doc, body)
}

fn planes_same_datum(a: &crate::model::ConstructionPlane, b: &crate::model::ConstructionPlane) -> bool {
    (a.normal - b.normal).length() < 1e-5
        && (a.origin - b.origin).length() < 1e-5
        && (a.u_axis - b.u_axis).length() < 1e-5
        && a.extent == b.extent
}

fn face_table(face: &FaceId, doc: &Document) -> String {
    // Use the same rendering as Instruction::as_lua_in for FaceId.
    // BeginSketch currently uses face_lua_parts; we emit the full table form.
    crate::script::face_id_lua_ref_for_export(face, doc)
}

fn body_ords(doc: &Document, keys: &[crate::model::BodyKey]) -> Vec<usize> {
    keys.iter()
        .filter_map(|k| doc.bodies.keys().position(|x| x == *k))
        .collect()
}

fn sketch_mirror_axis_lua(doc: &Document, axis: crate::model::SketchMirrorAxis) -> String {
    use crate::model::{SketchAxis, SketchMirrorAxis};
    match axis {
        SketchMirrorAxis::Line(li) => doc
            .lines
            .keys()
            .position(|k| k == li)
            .unwrap_or(0)
            .to_string(),
        SketchMirrorAxis::OriginAxis(SketchAxis::X) => "\"x\"".to_string(),
        SketchMirrorAxis::OriginAxis(SketchAxis::Y) => "\"y\"".to_string(),
        SketchMirrorAxis::X => "\"gx\"".to_string(),
        SketchMirrorAxis::Y => "\"gy\"".to_string(),
        SketchMirrorAxis::Z => "\"gz\"".to_string(),
    }
}

fn keys_ords<T>(
    live: impl Iterator<Item = crate::arena::Key<T>>,
    targets: &[crate::arena::Key<T>],
) -> Vec<usize> {
    let live: Vec<_> = live.collect();
    targets
        .iter()
        .filter_map(|t| live.iter().position(|k| k == t))
        .collect()
}

fn list_usizes(v: &[usize]) -> String {
    v.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn line_length_expr(doc: &Document, line: crate::model::LineKey) -> Option<String> {
    doc.constraints.values().find_map(|c| match &c.kind {
        ConstraintKind::Distance {
            target: DistanceTarget::LineLength(i),
        } if *i == line => Some(c.expression.clone()),
        _ => None,
    })
}

fn line_length_constraint_key(
    doc: &Document,
    line: crate::model::LineKey,
) -> Option<crate::model::ConstraintKey> {
    doc.constraints.iter().find_map(|(k, c)| match &c.kind {
        ConstraintKind::Distance {
            target: DistanceTarget::LineLength(i),
        } if *i == line => Some(k),
        _ => None,
    })
}

fn circle_diameter_expr(doc: &Document, circle: crate::model::CircleKey) -> Option<String> {
    doc.constraints.values().find_map(|c| match &c.kind {
        ConstraintKind::Distance {
            target: DistanceTarget::CircleDiameter(i),
        } if *i == circle => Some(c.expression.clone()),
        _ => None,
    })
}

fn circle_diameter_constraint_key(
    doc: &Document,
    circle: crate::model::CircleKey,
) -> Option<crate::model::ConstraintKey> {
    doc.constraints.iter().find_map(|(k, c)| match &c.kind {
        ConstraintKind::Distance {
            target: DistanceTarget::CircleDiameter(i),
        } if *i == circle => Some(k),
        _ => None,
    })
}

struct RectGroup {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    lines: [crate::model::LineKey; 4],
    constraints: Vec<crate::model::ConstraintKey>,
}

/// If `start` is the bottom edge of an axis-aligned rectangle of four free lines, return it.
fn find_rect_from_line(
    doc: &Document,
    start: crate::model::LineKey,
    generated: &HashSet<crate::model::LineKey>,
) -> Option<RectGroup> {
    let l0 = doc.lines.get(start)?;
    if l0.bezier.is_some() || l0.projection.is_some() || generated.contains(&start) {
        return None;
    }
    // Bottom edge: horizontal, left→right.
    let eps = 1e-3;
    if (l0.y0 - l0.y1).abs() > eps || l0.x1 <= l0.x0 {
        return None;
    }
    let x = l0.x0;
    let y = l0.y0;
    let w = l0.x1 - l0.x0;
    // Find right, top, left edges among same-sketch free lines.
    let sketch = l0.sketch;
    let mut right = None;
    let mut top = None;
    let mut left = None;
    for (k, l) in doc.lines.iter() {
        // Skip op-generated lines: a sketch fillet leaves trimmed copies of the rect's own
        // edges in the sketch, and grouping those would emit a `bearcad.rect` for geometry
        // the op re-creates (#1511).
        if l.sketch != sketch
            || k == start
            || l.bezier.is_some()
            || l.projection.is_some()
            || generated.contains(&k)
        {
            continue;
        }
        // right: (x+w,y) → (x+w,y+h)
        if (l.x0 - (x + w)).abs() < eps
            && (l.y0 - y).abs() < eps
            && (l.x1 - (x + w)).abs() < eps
            && l.y1 > y + eps
        {
            right = Some((k, l.y1 - l.y0));
        }
        // top: (x+w,y+h) → (x,y+h)
        if (l.x1 - x).abs() < eps
            && (l.x0 - (x + w)).abs() < eps
            && (l.y0 - l.y1).abs() < eps
            && l.y0 > y + eps
        {
            top = Some(k);
        }
        // left: (x,y+h) → (x,y)
        if (l.x0 - x).abs() < eps
            && (l.x1 - x).abs() < eps
            && (l.y1 - y).abs() < eps
            && l.y0 > y + eps
        {
            left = Some(k);
        }
    }
    let (right_k, h) = right?;
    let top_k = top?;
    let left_k = left?;
    // Confirm top/left heights match.
    let top_l = doc.lines.get(top_k)?;
    let left_l = doc.lines.get(left_k)?;
    if (top_l.y0 - (y + h)).abs() > eps || (left_l.y0 - (y + h)).abs() > eps {
        return None;
    }
    if w < 0.5 || h < 0.5 {
        return None;
    }
    // Collect constraints that CreateRect would recreate (coincidences + axis parallels).
    let lines = [start, right_k, top_k, left_k];
    let mut constraints = Vec::new();
    for (ck, c) in doc.constraints.iter() {
        if c.sketch != sketch {
            continue;
        }
        match &c.kind {
            ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint { line: la, .. }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint { line: lb, .. }),
            } if lines.contains(la) && lines.contains(lb) => constraints.push(ck),
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(la),
                line_b: ConstraintLine::OriginAxis(_),
            } if lines.contains(la) => constraints.push(ck),
            ConstraintKind::Parallel {
                line_a: ConstraintLine::OriginAxis(_),
                line_b: ConstraintLine::Line(lb),
            } if lines.contains(lb) => constraints.push(ck),
            _ => {}
        }
    }
    // A `bearcad.rect` plants four coincidences and four axis-parallels. Four lines
    // that merely look rectangular (an extrude_face wall, a hand-drawn box) must
    // stay as four `bearcad.line` calls — folding them would invent length dims
    // the original document never had.
    let coincidences = constraints.iter().filter(|ck| {
        matches!(
            doc.constraints.get(**ck).map(|c| &c.kind),
            Some(ConstraintKind::Coincident { .. })
        )
    }).count();
    let axis_parallels = constraints.len() - coincidences;
    if coincidences < 4 || axis_parallels < 4 {
        return None;
    }
    Some(RectGroup {
        x,
        y,
        w,
        h,
        lines,
        constraints,
    })
}

/// Keep parameter/unit expressions (`"w"`, `"2in"`); drop a bare number so the
/// seed millimetres are what `bearcad.rect` plants. A document in inches that
/// was authored as `width = 2` stores the expression `"2"` (2 inches) on a
/// 2 mm seed, and re-evaluating that string as the create size would skip the
/// original solve.
fn symbolic_expr(expr: Option<String>) -> Option<String> {
    expr.filter(|e| e.trim().parse::<f64>().is_err())
}

fn rect_export_xywh(doc: &Document, rect: &RectGroup) -> (f32, f32, f32, f32) {
    let Some(bottom) = doc.lines.get(rect.lines[0]) else {
        return (rect.x, rect.y, rect.w, rect.h);
    };
    let Some(right) = doc.lines.get(rect.lines[1]) else {
        return (rect.x, rect.y, rect.w, rect.h);
    };
    let (x0, y0, x1, _) = bottom.export_endpoints();
    let (_, y0r, _, y1r) = right.export_endpoints();
    let w = x1 - x0;
    let h = y1r - y0r;
    if w < 0.5 || h < 0.5 {
        return (rect.x, rect.y, rect.w, rect.h);
    }
    (x0, y0, w, h)
}

fn emit_constraint(doc: &Document, c: &crate::model::Constraint, out: &mut String) {
    match &c.kind {
        ConstraintKind::Distance { target } => match target {
            DistanceTarget::LineLength(i) => {
                let ord = doc.lines.keys().position(|k| k == *i).unwrap_or(0);
                out.push_str(&format!(
                    "bearcad.dimension{{ kind = \"line\", index = {ord}, value = {:?} }}\n",
                    c.expression
                ));
            }
            DistanceTarget::CircleDiameter(i) => {
                let ord = doc.circles.keys().position(|k| k == *i).unwrap_or(0);
                out.push_str(&format!(
                    "bearcad.dimension{{ kind = \"circle\", index = {ord}, value = {:?} }}\n",
                    c.expression
                ));
            }
            DistanceTarget::PointPointDistance { anchor, mover, .. } => {
                out.push_str(&format!(
                    "bearcad.dimension{{ kind = \"point_point\", anchor = {}, mover = {}, value = {:?} }}\n",
                    constraint_point_table(doc, anchor),
                    constraint_point_table(doc, mover),
                    c.expression
                ));
            }
            DistanceTarget::PointLineDistance { point, line, .. } => {
                out.push_str(&format!(
                    "bearcad.dimension{{ kind = \"point_line\", point = {}, line = {}, value = {:?} }}\n",
                    constraint_point_table(doc, point),
                    constraint_line_table(doc, line),
                    c.expression
                ));
            }
            DistanceTarget::LineLineDistance { line_a, line_b, .. } => {
                out.push_str(&format!(
                    "bearcad.dimension{{ kind = \"line_line\", a = {}, b = {}, value = {:?} }}\n",
                    constraint_line_table(doc, line_a),
                    constraint_line_table(doc, line_b),
                    c.expression
                ));
            }
        },
        ConstraintKind::Angle {
            line_a,
            line_b,
            rotation_sign,
        } => {
            let a = match line_a {
                ConstraintLine::Line(i) => doc.lines.keys().position(|k| k == *i).unwrap_or(0),
                _ => 0,
            };
            let b = match line_b {
                ConstraintLine::Line(i) => doc.lines.keys().position(|k| k == *i).unwrap_or(0),
                _ => 0,
            };
            out.push_str(&format!(
                "bearcad.dimension{{ kind = \"angle\", a = {a}, b = {b}, sign = {rotation_sign}, value = {:?} }}\n",
                c.expression
            ));
        }
        ConstraintKind::Parallel {
            line_a,
            line_b,
        } => {
            // Axis-parallel collapses to horizontal/vertical when one side is an origin axis.
            match (line_a, line_b) {
                (ConstraintLine::Line(i), ConstraintLine::OriginAxis(axis))
                | (ConstraintLine::OriginAxis(axis), ConstraintLine::Line(i)) => {
                    let ord = doc.lines.keys().position(|k| k == *i).unwrap_or(0);
                    let name = match axis {
                        crate::model::SketchAxis::X => "horizontal",
                        crate::model::SketchAxis::Y => "vertical",
                    };
                    out.push_str(&format!(
                        "bearcad.constrain({name:?}, {{ kind = \"line\", index = {ord} }})\n"
                    ));
                }
                _ => {
                    emit_geo_pair(doc, line_a, line_b, "parallel", out);
                }
            }
        }
        ConstraintKind::Perpendicular { line_a, line_b } => {
            emit_geo_pair(doc, line_a, line_b, "perpendicular", out);
        }
        ConstraintKind::Equal { line_a, line_b } => {
            emit_geo_pair(doc, line_a, line_b, "equal", out);
        }
        ConstraintKind::Coincident { a, b } => {
            if let (Some(ea), Some(eb)) = (entity_select(doc, a), entity_select(doc, b)) {
                out.push_str(&format!("bearcad.constrain(\"coincident\", {ea}, {eb})\n"));
            }
        }
        ConstraintKind::Midpoint { point, line } => {
            out.push_str(&format!(
                "bearcad.constrain(\"midpoint\", {}, {})\n",
                constraint_point_table(doc, point),
                constraint_line_table(doc, line)
            ));
        }
        ConstraintKind::Tangent { a, b } => {
            out.push_str(&format!(
                "bearcad.constrain(\"tangent\", {}, {})\n",
                constraint_point_table(doc, a),
                constraint_point_table(doc, b)
            ));
        }
        ConstraintKind::TangentCircle { circle, other } => {
            let ord = doc.circles.keys().position(|k| k == *circle).unwrap_or(0);
            let circle = format!("{{ kind = \"circle\", index = {ord} }}");
            let other = match other {
                crate::model::TangentTarget::Circle(o) => {
                    let ord = doc.circles.keys().position(|k| k == *o).unwrap_or(0);
                    format!("{{ kind = \"circle\", index = {ord} }}")
                }
                crate::model::TangentTarget::Line(line) => constraint_line_table(doc, line),
            };
            out.push_str(&format!("bearcad.constrain(\"tangent\", {circle}, {other})\n"));
        }
    }
}

fn emit_geo_pair(
    doc: &Document,
    a: &ConstraintLine,
    b: &ConstraintLine,
    kind: &str,
    out: &mut String,
) {
    out.push_str(&format!(
        "bearcad.constrain({kind:?}, {}, {})\n",
        constraint_line_table(doc, a),
        constraint_line_table(doc, b)
    ));
}

fn entity_select(doc: &Document, e: &ConstraintEntity) -> Option<String> {
    match e {
        ConstraintEntity::Point(p) => Some(constraint_point_table(doc, p)),
        ConstraintEntity::Line(l) => Some(constraint_line_table(doc, l)),
        ConstraintEntity::Circle(i) => {
            let ord = doc.circles.keys().position(|k| k == *i).unwrap_or(0);
            Some(format!("{{ kind = \"circle\", index = {ord} }}"))
        }
        ConstraintEntity::FaceCircle { .. } => None,
        ConstraintEntity::Origin => Some("{ kind = \"origin\" }".into()),
    }
}

fn constraint_point_table(doc: &Document, p: &ConstraintPoint) -> String {
    match p {
        ConstraintPoint::LineEndpoint { line, end } => {
            let ord = doc.lines.keys().position(|k| k == *line).unwrap_or(0);
            let end = match end {
                LineEnd::Start => "start",
                LineEnd::End => "end",
            };
            format!("{{ kind = \"line\", index = {ord}, endpoint = \"{end}\" }}")
        }
        ConstraintPoint::CircleCenter(c) => {
            let ord = doc.circles.keys().position(|k| k == *c).unwrap_or(0);
            format!("{{ kind = \"circle\", index = {ord}, point = true }}")
        }
        ConstraintPoint::Origin => "{ kind = \"origin\" }".into(),
        other => {
            // Fall back to Instruction rendering for face/text/image points.
            Instruction::SelectSceneElement {
                element: SceneElement::Point(other.clone()),
                additive: false,
            }
            .as_lua_in(Some(doc))
            .trim_start_matches("bearcad.select(")
            .trim_end_matches(')')
            .to_string()
        }
    }
}

fn constraint_line_table(doc: &Document, l: &ConstraintLine) -> String {
    match l {
        ConstraintLine::Line(i) => {
            let ord = doc.lines.keys().position(|k| k == *i).unwrap_or(0);
            format!("{{ kind = \"line\", index = {ord} }}")
        }
        ConstraintLine::OriginAxis(axis) => {
            let a = match axis {
                crate::model::SketchAxis::X => "x",
                crate::model::SketchAxis::Y => "y",
            };
            format!("{{ kind = \"axis\", axis = \"{a}\" }}")
        }
        ConstraintLine::FaceEdge { face, index } => {
            format!(
                "{{ kind = \"face\", face = {}, index = {index}, edge = true }}",
                face_table(face, doc)
            )
        }
        ConstraintLine::ImageEdge { image, edge } => {
            let ord = doc.tracing_images.keys().position(|k| k == *image).unwrap_or(0);
            format!(
                "{{ kind = \"image\", index = {ord}, edge = \"{}\" }}",
                edge.lua_name()
            )
        }
    }
}

fn instruction_for_extrusion(
    doc: &Document,
    key: crate::model::ExtrusionKey,
) -> Option<Instruction> {
    let extrusion = doc.extrusions.get(key)?;
    let body = match crate::model::body_index_for_extrusion(doc, key).and_then(|bi| doc.bodies.get(bi))
    {
        Some(body) if body.source.cut_extrusion_indices().contains(&key) => {
            crate::actions::ExtrudeBodyChoice::Cut
        }
        Some(body) if body.source.extrusion_indices().len() > 1 => {
            crate::actions::ExtrudeBodyChoice::Merge
        }
        _ => crate::actions::ExtrudeBodyChoice::New,
    };
    let sketch = doc.sketches.keys().position(|k| k == extrusion.sketch)?;
    Some(Instruction::Extrude {
        sketch,
        faces: extrusion.faces.clone(),
        distance: extrusion.distance,
        body,
        target: extrusion.target.clone(),
        expression: (!extrusion.expression.trim().is_empty()).then(|| extrusion.expression.clone()),
        symmetric: extrusion.symmetric,
        taper: extrusion.taper,
        taper_mode: extrusion.taper_mode,
        taper_expression: (!extrusion.taper_expression.trim().is_empty())
            .then(|| extrusion.taper_expression.clone()),
    })
}

fn instruction_for_loft(doc: &Document, key: crate::model::LoftKey) -> Option<Instruction> {
    let loft = doc.lofts.get(key)?;
    let (body, bodies) = match &loft.mode {
        crate::model::LoftMode::NewBody => (crate::actions::RevolveBodyChoice::NewBody, Vec::new()),
        crate::model::LoftMode::AddTo(b) => {
            (crate::actions::RevolveBodyChoice::AddTouching, b.clone())
        }
        crate::model::LoftMode::Cut(b) => (crate::actions::RevolveBodyChoice::Cut, b.clone()),
    };
    Some(Instruction::Loft {
        faces: loft.sections.iter().map(|s| s.face.clone()).collect(),
        body,
        bodies: body_ords(doc, &bodies),
    })
}

fn instruction_for_revolution(
    doc: &Document,
    key: crate::model::RevolutionKey,
) -> Option<Instruction> {
    let rev = doc.revolutions.get(key)?;
    let (body, bodies) = match &rev.mode {
        crate::model::RevolveMode::NewBody => {
            (crate::actions::RevolveBodyChoice::NewBody, Vec::new())
        }
        crate::model::RevolveMode::AddTo(b) => {
            (crate::actions::RevolveBodyChoice::AddTouching, b.clone())
        }
        crate::model::RevolveMode::Cut(b) => (crate::actions::RevolveBodyChoice::Cut, b.clone()),
    };
    Some(Instruction::Revolve {
        faces: rev.faces.clone(),
        axis: rev.axis,
        angle_deg: rev.angle_deg,
        angle_expression: rev.angle_expression.clone(),
        angle_is_revolutions: rev.angle_is_revolutions,
        pitch_mm: rev.pitch_mm,
        pitch_expression: rev.pitch_expression.clone(),
        symmetric: rev.symmetric,
        body,
        bodies: body_ords(doc, &bodies),
    })
}

fn instruction_for_sweep(doc: &Document, key: crate::model::SweepKey) -> Option<Instruction> {
    let fp = doc.sweeps.get(key)?;
    let (body, bodies) = match &fp.mode {
        crate::model::SweepMode::NewBody => {
            (crate::actions::RevolveBodyChoice::NewBody, Vec::new())
        }
        crate::model::SweepMode::AddTo(b) => {
            (crate::actions::RevolveBodyChoice::AddTouching, b.clone())
        }
        crate::model::SweepMode::Cut(b) => (crate::actions::RevolveBodyChoice::Cut, b.clone()),
    };
    Some(Instruction::Sweep {
        faces: fp.faces.clone(),
        path: fp.path.clone(),
        body,
        bodies: body_ords(doc, &bodies),
    })
}

/// Components and which top-level elements live in them (#1517). Both have scripting verbs
/// (`bearcad.component`, `bearcad.move_to_component`); neither used to be exported, so a
/// component tree vanished on the round trip.
fn emit_components(doc: &Document, out: &mut String) {
    for (key, component) in doc.components.iter() {
        let mut parts = Vec::new();
        if let Some(name) = &component.name {
            parts.push(format!("name = {name:?}"));
        }
        if let Some(parent) = component.parent {
            if let Some(ord) = doc.components.keys().position(|k| k == parent) {
                parts.push(format!("parent = {ord}"));
            }
        }
        out.push_str(&format!("bearcad.component{{ {} }}\n", parts.join(", ")));
        let _ = key;
    }
    for (member, component) in &doc.component_members {
        let Some(cord) = doc.components.keys().position(|k| k == *component) else {
            continue;
        };
        let Some((kind, index)) = component_member_ref(doc, member) else {
            out.push_str(&format!(
                "-- skipped: {member:?} is in a component, but move_to_component can't name it\n"
            ));
            continue;
        };
        out.push_str(&format!(
            "bearcad.move_to_component{{ kind = {kind:?}, index = {index}, component = {cord} }}\n"
        ));
    }
}

/// The `kind`/`index` pair `bearcad.move_to_component` names a component member by.
fn component_member_ref(
    doc: &Document,
    member: &crate::model::ComponentMember,
) -> Option<(&'static str, usize)> {
    use crate::model::ComponentMember as M;
    macro_rules! ord {
        ($coll:ident, $key:expr, $name:expr) => {
            doc.$coll.keys().position(|k| k == *$key).map(|i| ($name, i))
        };
    }
    match member {
        M::ConstructionPlane(k) => ord!(construction_planes, k, "plane"),
        M::Extrusion(k) => ord!(extrusions, k, "extrusion"),
        M::Body(k) => ord!(bodies, k, "body"),
        M::Loft(k) => ord!(lofts, k, "loft"),
        M::Drawing(k) => ord!(drawings, k, "drawing"),
        M::BooleanOp(k) => ord!(boolean_ops, k, "boolean_op"),
        M::MoveOp(k) => ord!(move_ops, k, "move_op"),
        M::MirrorOp(k) => ord!(mirror_ops, k, "mirror_op"),
        M::RepeatOp(k) => ord!(repeat_ops, k, "repeat_op"),
        M::SliceOp(k) => ord!(slice_ops, k, "slice_op"),
        M::ShellOp(k) => ord!(shell_ops, k, "shell_op"),
        M::EdgeTreatmentOp(k) => ord!(edge_treatment_ops, k, "edge_treatment_op"),
        M::Revolution(k) => ord!(revolutions, k, "revolution"),
        M::Sweep(k) => ord!(sweeps, k, "sweep"),
    }
}

fn emit_materials(doc: &Document, out: &mut String) {
    let defaults: HashSet<_> = crate::model::Material::DEFAULTS
        .iter()
        .map(|(n, c)| ((*n).to_string(), *c))
        .collect();
    // Custom materials: anything not in the default palette.
    for (mi, mat) in doc.materials.iter() {
        if defaults.contains(&(mat.name.clone(), mat.color)) {
            continue;
        }
        // Bodies using this material.
        let bodies: Vec<usize> = doc
            .bodies
            .iter()
            .filter(|(_, b)| b.material == Some(mi))
            .filter_map(|(bk, _)| doc.bodies.keys().position(|k| k == bk))
            .collect();
        out.push_str(
            &Instruction::AddMaterial {
                name: Some(mat.name.clone()),
                color: Some(mat.color),
                bodies,
            }
            .as_lua_in(Some(doc)),
        );
        out.push('\n');
    }
    // Assignments of default materials (non-first) that AddMaterial wouldn't cover.
    for (bi, body) in doc.bodies.iter() {
        let Some(mk) = body.material else { continue };
        let Some(mat) = doc.materials.get(mk) else { continue };
        if !defaults.contains(&(mat.name.clone(), mat.color)) {
            continue; // custom — already assigned via AddMaterial
        }
        // Skip Unobtainium (first default) — that's the implicit default.
        if mat.name == crate::model::Material::DEFAULTS[0].0 {
            continue;
        }
        let bord = doc.bodies.keys().position(|k| k == bi).unwrap_or(0);
        let mord = doc.materials.keys().position(|k| k == mk).unwrap_or(0);
        out.push_str(&format!(
            "bearcad.set_material{{ body = {bord}, material = {mord} }}\n"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::AppState;
    use crate::script::{ScriptRunner, SyntheticInput};

    fn run_lua(source: &str) -> AppState {
        let mut runner = ScriptRunner::from_lua_source(source).unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
        state
    }

    /// Like `run_lua`, but hands back the script error instead of panicking (#1520).
    fn try_run_lua(source: &str) -> Result<AppState, String> {
        let mut runner = ScriptRunner::from_lua_source(source).map_err(|e| e.to_string())?;
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        match runner.error {
            Some(e) => Err(e),
            None => Ok(state),
        }
    }

    /// Build a document from `source`, export it, replay the export, and report every
    /// difference. Empty result means the export is a faithful recipe for the document.
    fn round_trip(source: &str) -> Result<Vec<String>, String> {
        let state = try_run_lua(source).map_err(|e| format!("source script failed: {e}"))?;
        let script = document_to_lua(&state.doc);
        if script.contains("bearcad.ui.") {
            return Err(format!("export used the ui module:\n{script}"));
        }
        let rebuilt = try_run_lua(&script)
            .map_err(|e| format!("replay of export failed: {e}\n--- script ---\n{script}"))?;
        let diffs = document_diff(&state.doc, &rebuilt.doc);
        if diffs.is_empty() {
            Ok(diffs)
        } else {
            Err(format!(
                "round-trip diffs: {}\n--- script ---\n{script}",
                diffs.join("\n  ")
            ))
        }
    }

    /// #1520: one case per declarative verb. Adding a verb means adding a case here, the
    /// same way the pointer path requires a `tests/interaction/*.lua` script.
    const ROUND_TRIP_CASES: &[(&str, &str)] = &[
        (
            "cuboid",
            r#"bearcad.new()
               bearcad.cuboid{ width = 20, depth = 15, height = 10 }"#,
        ),
        (
            "cylinder",
            r#"bearcad.new()
               bearcad.cylinder{ radius = 6, height = 25 }"#,
        ),
        (
            "sphere",
            r#"bearcad.new()
               bearcad.sphere{ radius = 9 }"#,
        ),
        (
            "boolean",
            r#"bearcad.new()
               bearcad.cuboid{ width = 20, depth = 20, height = 20 }
               bearcad.cuboid{ width = 10, depth = 10, height = 40 }
               bearcad.combine{ op = "cut", a = {0}, b = {1} }"#,
        ),
        (
            "move",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10 }
               bearcad.move_bodies{ bodies = {0}, x = 5, y = 3, z = 1 }"#,
        ),
        (
            "mirror",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10, at = {20, 0, 0} }
               bearcad.mirror_bodies{ bodies = {0}, plane = 0 }"#,
        ),
        (
            "repeat",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10 }
               bearcad.repeat_bodies{ bodies = {0}, axis = "x", mode = "count_gap", count = 3, spacing = 8 }"#,
        ),
        (
            "shell",
            r#"bearcad.new()
               bearcad.circle{ x = 0, y = 0, r = 10 }
               bearcad.extrude{ circle = 0, distance = 20 }
               bearcad.shell{ bodies = {0}, faces = {{ kind = "extrude_cap", extrusion = 0, profile = "circle", profile_index = 0, top = true }}, thickness = "1" }"#,
        ),
        (
            "edge fillet",
            r#"bearcad.new()
               bearcad.cuboid{ width = 20, depth = 20, height = 20 }
               bearcad.fillet_edge{ primitive = 0, edge = { kind = "vertical", face = 0, edge = 0 }, radius = 2 }"#,
        ),
        (
            "sketch fillet",
            r#"bearcad.new()
               bearcad.rect{ width = 40, height = 30 }
               bearcad.fillet_vertex{ point = { kind = "line", index = 0, endpoint = "end" }, radius = 3 }"#,
        ),
        (
            "sketch fillet two corners",
            r#"bearcad.new()
               bearcad.rect{ width = 40, height = 30 }
               bearcad.fillet_vertex{ points = {
                 { kind = "line", index = 0, endpoint = "end" },
                 { kind = "line", index = 1, endpoint = "end" },
               }, radius = 3 }"#,
        ),
        (
            "sketch offset",
            r#"bearcad.new()
               bearcad.rect{ width = 40, height = 30 }
               bearcad.offset_sketch{ lines = {0,1,2,3}, distance = 3 }"#,
        ),
        (
            "sketch mirror",
            r#"bearcad.new()
               bearcad.line{ x = 5, y = 0, x1 = 20, y1 = 10 }
               bearcad.line{ x = 0, y = -20, x1 = 0, y1 = 20 }
               bearcad.mirror_sketch{ lines = {0}, line = 1 }"#,
        ),
        (
            "sketch repeat",
            r#"bearcad.new()
               bearcad.circle{ x = 0, y = 0, r = 3 }
               bearcad.repeat_sketch{ circles = {0}, count = 4, angle = 0, spacing = "10" }"#,
        ),
        (
            "sketch slice",
            r#"bearcad.new()
               bearcad.line{ x = -20, y = 0, x1 = 20, y1 = 0 }
               bearcad.line{ x = 0, y = -20, x1 = 0, y1 = 20 }
               bearcad.slice_sketch{ lines = {0}, cutters = {1} }"#,
        ),
        (
            "revolve",
            r#"bearcad.new()
               bearcad.rect{ x = 5, y = 0, width = 10, height = 20 }
               bearcad.revolve{ polygon = {0,1,2,3}, axis = "y", angle = 360 }"#,
        ),
        (
            "text",
            r#"bearcad.new()
               bearcad.text{ text = "Hi", size = 10, x = 0, y = 0 }"#,
        ),
        (
            "bezier",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 30, y1 = 0, bezier = { {10, 12}, {20, -12} } }"#,
        ),
        (
            "construction geometry",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 30, y1 = 0 }
               bearcad.set_construction({ kind = "line", index = 0 }, true)
               bearcad.circle{ x = 0, y = 10, r = 4 }
               bearcad.set_construction({ kind = "circle", index = 0 }, true)"#,
        ),
        (
            "named elements",
            r#"bearcad.new()
               bearcad.rect{ width = 40, height = 20 }
               bearcad.set_name(bearcad.element("line", 2), "Back edge")"#,
        ),
        (
            "shadow body",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10 }
               bearcad.set_body_shadow{ body = 0, shadow = true }"#,
        ),
        (
            "non-default units",
            r#"bearcad.new()
               bearcad.set_units{ length = "in", angle = "rad" }
               bearcad.rect{ width = 2, height = 1 }"#,
        ),
        (
            "parameter bounds",
            r#"bearcad.new()
               bearcad.add_parameter("w", "24")
               bearcad.edit_parameter{ name = "w", min = "5", max = "50" }
               bearcad.rect{ width = "w", height = 12 }"#,
        ),
        (
            "construction plane offset",
            r#"bearcad.new()
               bearcad.plane{ offset = 12 }"#,
        ),
        (
            "chained construction planes",
            r#"bearcad.new()
               bearcad.plane{ offset = 10 }
               bearcad.plane{ offset = 5, from = 3 }"#,
        ),
        (
            "projection",
            r#"bearcad.new()
               bearcad.cuboid{ width = 20, depth = 20, height = 20 }
               bearcad.plane{ offset = 40 }
               bearcad.begin_sketch("construction_plane", 3)
               bearcad.project{ body = 0 }"#,
        ),
        (
            "components",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10 }
               bearcad.component{ name = "Sub" }
               bearcad.move_to_component{ kind = "body", index = 0, component = 0 }"#,
        ),
        (
            "loft in a component",
            r#"bearcad.new()
               bearcad.circle{ r = 5 }
               bearcad.plane{ offset = 10 }
               bearcad.begin_sketch{ kind = "plane", index = 3 }
               bearcad.circle{ r = 2 }
               bearcad.exit_sketch()
               bearcad.loft{ circles = {0, 1} }
               bearcad.component{ name = "Sub" }
               bearcad.move_to_component{ kind = "loft", index = 0, component = 0 }"#,
        ),
        (
            "drawing",
            r#"bearcad.new()
               bearcad.cuboid{ width = 20, depth = 20, height = 20 }
               bearcad.drawing{}
               bearcad.drawing_view{ drawing = 0, body = 0, orientation = "front" }"#,
        ),
        (
            "drawing in a component",
            r#"bearcad.new()
               bearcad.cuboid{ width = 20, depth = 20, height = 20 }
               bearcad.drawing{}
               bearcad.drawing_view{ drawing = 0, body = 0, orientation = "front" }
               bearcad.component{ name = "Sub" }
               bearcad.move_to_component{ kind = "drawing", index = 0, component = 0 }"#,
        ),
        (
            "sketch on a body face",
            r#"bearcad.new()
               bearcad.rect{ width = 40, height = 40 }
               bearcad.extrude{ polygon = {0,1,2,3}, distance = 20 }
               bearcad.exit_sketch()
               bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0,1,2,3}, top = true }
               bearcad.rect{ width = 10, height = 10 }"#,
        ),
        (
            "cut extrude",
            r#"bearcad.new()
               bearcad.rect{ width = 40, height = 40 }
               bearcad.extrude{ polygon = {0,1,2,3}, distance = 20 }
               bearcad.exit_sketch()
               bearcad.begin_sketch{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0,1,2,3}, top = true }
               bearcad.circle{ x = 0, y = 0, r = 5 }
               bearcad.extrude{ circle = 0, distance = -10, body = "cut" }"#,
        ),
        (
            "constraint parallel",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 1 }
               bearcad.line{ x = 0, y = 10, x1 = 20, y1 = 12 }
               bearcad.constrain("parallel",
                   { kind = "line", index = 0 }, { kind = "line", index = 1 })"#,
        ),
        (
            "constraint perpendicular",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 1 }
               bearcad.line{ x = 0, y = 10, x1 = 1, y1 = 30 }
               bearcad.constrain("perpendicular",
                   { kind = "line", index = 0 }, { kind = "line", index = 1 })"#,
        ),
        (
            "constraint line length",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0 }
               bearcad.dimension{ kind = "line", index = 0, value = "leg = 40mm" }"#,
        ),
        (
            "constraint point_point",
            r#"bearcad.new()
               bearcad.circle{ x = 0, y = 0, r = 3 }
               bearcad.circle{ x = 20, y = 1, r = 3 }
               bearcad.dimension{ kind = "point_point",
                   anchor = { kind = "circle", index = 0, point = true },
                   mover = { kind = "circle", index = 1, point = true },
                   value = "25" }"#,
        ),
        (
            "constraint angle",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 0 }
               bearcad.line{ x = 0, y = 0, x1 = 18, y1 = 8 }
               bearcad.dimension{ kind = "angle", a = 0, b = 1, value = "30deg" }"#,
        ),
        (
            "constraint coincident",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 0 }
               bearcad.line{ x = 21, y = 1, x1 = 30, y1 = 15 }
               bearcad.constrain("coincident",
                   { kind = "line", index = 0, endpoint = "end" },
                   { kind = "line", index = 1, endpoint = "start" })"#,
        ),
        (
            "constraint equal",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 0 }
               bearcad.line{ x = 0, y = 10, x1 = 12, y1 = 10 }
               bearcad.constrain("equal",
                   { kind = "line", index = 0 }, { kind = "line", index = 1 })"#,
        ),
        (
            "constraint horizontal",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 2 }
               bearcad.constrain("horizontal", { kind = "line", index = 0 })"#,
        ),
        (
            "constraint circle diameter",
            r#"bearcad.new()
               bearcad.circle{ x = 0, y = 0, r = 8 }"#,
        ),
        (
            "slice",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10 }
               bearcad.plane{ offset = 5 }
               bearcad.slice{ bodies = {0}, cutters = {{ kind = "construction_plane", index = 3 }} }"#,
        ),
        (
            "edge chamfer",
            r#"bearcad.new()
               bearcad.rect{ x = 0, y = 0, width = 10, height = 10 }
               bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
               bearcad.chamfer_edge{
                   extrusion = 0,
                   edge = { kind = "vertical", face = 0, edge = 0 },
                   distance = 2
               }"#,
        ),
        (
            "loft",
            r#"bearcad.new()
               bearcad.circle{ r = 5 }
               bearcad.plane{ offset = 10 }
               bearcad.begin_sketch{ kind = "plane", index = 3 }
               bearcad.circle{ r = 2 }
               bearcad.exit_sketch()
               bearcad.loft{ circles = {0, 1} }"#,
        ),
        (
            "sweep",
            r#"bearcad.new()
               bearcad.circle{ x = 0, y = 0, r = 3 }
               bearcad.exit_sketch()
               bearcad.plane{ origin = {0, 0, 0}, normal = {0, 1, 0} }
               bearcad.begin_sketch{ kind = "plane", index = 3 }
               bearcad.line{ x = 0, y = 0, x1 = 0, y1 = 20 }
               bearcad.exit_sketch()
               bearcad.sweep{ circle = 0, path = {0} }"#,
        ),
        (
            "materials",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10 }
               bearcad.set_material{ body = 0, material = 1 }"#,
        ),
        (
            "derived parameter",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 30, y1 = 0 }
               bearcad.derive_parameter{ kind = "line_length", a = 0, name = "L" }"#,
        ),
        (
            "anchored construction plane",
            r#"bearcad.new()
               bearcad.plane{ origin = {0, 0, 0}, normal = {1, 0, 0} }"#,
        ),
        (
            "axis construction plane",
            r#"bearcad.new()
               bearcad.plane{ axis = "x", angle = 45 }"#,
        ),
        (
            "line-axis construction plane",
            r#"bearcad.new()
               bearcad.line{ x = 0, y = 0, x1 = 20, y1 = 0 }
               bearcad.plane{ axis = { line = 0 }, angle = 30, offset = 5 }"#,
        ),
        (
            "extrude_face",
            r#"bearcad.new()
               bearcad.rect{ x = 0, y = 0, width = 20, height = 20 }
               bearcad.exit_sketch()
               bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 20 }
               bearcad.extrude_face{
                   face = { kind = "extrude_side", extrusion = 0, profile = "polygon", profile_lines = {0, 1, 2, 3}, edge = 0 },
                   distance = 10
               }"#,
        ),
        (
            "polygon-profile shell",
            r#"bearcad.new()
               bearcad.rect{ width = 40, height = 40 }
               bearcad.extrude{ polygon = {0,1,2,3}, distance = 20 }
               bearcad.shell{ bodies = {0}, faces = {{ kind = "extrude_cap", extrusion = 0, profile = "polygon", profile_lines = {0,1,2,3}, top = true }}, thickness = "1" }"#,
        ),
        (
            "joint",
            r#"bearcad.new()
               bearcad.cuboid{ width = 10, depth = 10, height = 10 }
               bearcad.cuboid{ width = 10, depth = 10, height = 10, at = {20, 0, 0} }
               bearcad.joint{ a = 0, b = 1, kind = "rigid" }"#,
        ),
    ];

    /// #1518: export the coordinates the user placed, not the post-solve ones.
    #[test]
    fn export_uses_pre_solve_seed_coordinates() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 50, y1 = 0 }
            bearcad.dimension{ kind = "line", index = 0, value = "leg = 40mm" }
            "#,
        );
        let line = state.doc.lines.values().next().expect("one line");
        assert!(
            (line.x1 - 50.0).abs() > 0.1,
            "solver should have moved the endpoint, got x1={}",
            line.x1
        );
        let script = document_to_lua(&state.doc);
        assert!(
            script.contains("x1 = 50"),
            "export must emit the pre-solve seed, got:\n{script}"
        );
    }

    /// #1519: a multi-corner sketch fillet exports as one `fillet_vertex{ points = ... }`.
    #[test]
    fn multi_corner_fillet_exports_one_call() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 30 }
            bearcad.fillet_vertex{ points = {
                { kind = "line", index = 0, endpoint = "end" },
                { kind = "line", index = 1, endpoint = "end" },
            }, radius = 3 }
            "#,
        );
        assert_eq!(state.doc.sketch_vertex_treatment_ops.len(), 1);
        let script = document_to_lua(&state.doc);
        let calls = script.matches("fillet_vertex").count();
        assert_eq!(
            calls, 1,
            "exporter must emit one call per op, got {calls} in:\n{script}"
        );
        assert!(
            script.contains("points ="),
            "multi-corner op must emit `points`, got:\n{script}"
        );
    }

    /// #1525: a drawing filed in a component exports as `move_to_component{ kind = "drawing" }`.
    #[test]
    fn drawing_in_component_exports_membership() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.cuboid{ width = 20, depth = 20, height = 20 }
            bearcad.drawing{}
            bearcad.component{ name = "Sub" }
            bearcad.move_to_component{ kind = "drawing", index = 0, component = 0 }
            "#,
        );
        assert_eq!(state.doc.component_members.len(), 1);
        let script = document_to_lua(&state.doc);
        assert!(
            !script.contains("skipped:"),
            "drawing membership must not be skipped, got:\n{script}"
        );
        assert!(
            script.contains("kind = \"drawing\""),
            "export must name the drawing, got:\n{script}"
        );
    }

    /// #1520: every case exports to a script that replays into the same document.
    #[test]
    fn declarative_verbs_round_trip() {
        let mut failures = Vec::new();
        for (label, source) in ROUND_TRIP_CASES {
            if let Err(e) = round_trip(source) {
                failures.push(format!("\n=== {label} ===\n{e}"));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} round-trip cases failed:{}",
            failures.len(),
            ROUND_TRIP_CASES.len(),
            failures.join("")
        );
    }

    #[test]
    fn empty_document_exports_new_only() {
        let doc = Document::default();
        let script = document_to_lua(&doc);
        assert!(script.contains("bearcad.new()"));
        assert!(!script.contains("bearcad.ui."));
        // No geometry calls.
        assert!(!script.contains("bearcad.rect"));
        assert!(!script.contains("bearcad.line"));
        assert!(!script.contains("bearcad.extrude"));
    }

    #[test]
    fn rect_extrude_round_trips_without_ui() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 20, x = 0, y = 0 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            "#,
        );
        let script = document_to_lua(&state.doc);
        assert!(
            !script.contains("bearcad.ui."),
            "export must not use ui module:\n{script}"
        );
        assert!(
            script.contains("bearcad.rect") || script.contains("bearcad.line"),
            "expected geometry in export:\n{script}"
        );
        assert!(
            script.contains("bearcad.extrude"),
            "expected extrude in export:\n{script}"
        );

        let rebuilt = run_lua(&script);
        let diffs = document_diff(&state.doc, &rebuilt.doc);
        assert!(
            diffs.is_empty(),
            "round-trip diffs: {diffs:?}\n--- script ---\n{script}"
        );
    }

    /// #1857: a tangency exports as `constrain("tangent", …)`, and replaying the
    /// export rebuilds the same two tangencies.
    #[test]
    fn tangent_circles_round_trip() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 20 }
            bearcad.circle{ x = 70, y = 0, r = 10 }
            bearcad.line{ x = -60, y = 50, x1 = 60, y1 = 50 }
            bearcad.constrain("tangent",
                { kind = "circle", index = 0 }, { kind = "circle", index = 1 })
            bearcad.constrain("tangent",
                { kind = "circle", index = 0 }, { kind = "line", index = 0 })
            "#,
        );
        let script = document_to_lua(&state.doc);
        assert!(!script.contains("bearcad.ui."));
        let rebuilt = run_lua(&script);
        let kinds = |doc: &Document| -> Vec<crate::model::ConstraintKind> {
            doc.constraints
                .values()
                .filter(|c| matches!(c.kind, ConstraintKind::TangentCircle { .. }))
                .map(|c| c.kind.clone())
                .collect()
        };
        // The document is under-constrained (two tangencies leave the pair free to slide),
        // so replaying lands on some valid pose — what must survive is the tangencies.
        assert_eq!(kinds(&state.doc).len(), 2, "two tangencies:\n{script}");
        assert_eq!(kinds(&rebuilt.doc), kinds(&state.doc), "\n{script}");
    }

    /// #1859: a circular repeat about a circle's normal exports with that axis and rebuilds
    /// the same six copies.
    #[test]
    fn repeat_about_a_circle_normal_round_trips() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 40 }
            bearcad.rect{ width = 8, height = 8, x = 36, y = -4 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 6 }
            bearcad.exit_sketch()
            bearcad.repeat_bodies{ bodies = {0}, axis = { circle_normal = 0 }, around = true,
                                   mode = "count_gap", count = 6, spacing = "60deg" }
            "#,
        );
        let script = document_to_lua(&state.doc);
        assert!(
            script.contains("circle_normal"),
            "the axis should export as the circle's normal:\n{script}"
        );
        let rebuilt = run_lua(&script);
        // (Full `document_diff` equality is out of reach here: the export emits the rect
        // before the circle, so the sketch's constraints come back in a different order.)
        assert_eq!(rebuilt.doc.bodies.len(), state.doc.bodies.len(), "\n{script}");
        let axis = |d: &Document| d.repeat_ops.values().next().map(|r| r.axis.clone());
        assert_eq!(axis(&rebuilt.doc), axis(&state.doc), "\n{script}");
    }

    #[test]
    fn line_circle_round_trips() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 30, y1 = 0, dimension = "30" }
            bearcad.circle{ x = 10, y = 10, r = 5 }
            "#,
        );
        let script = document_to_lua(&state.doc);
        assert!(!script.contains("bearcad.ui."));
        let rebuilt = run_lua(&script);
        let diffs = document_diff(&state.doc, &rebuilt.doc);
        assert!(
            diffs.is_empty(),
            "round-trip diffs: {diffs:?}\n--- script ---\n{script}"
        );
    }

    #[test]
    fn document_diff_reports_mismatch() {
        let a = run_lua("bearcad.new()\nbearcad.rect{ width = 10, height = 10 }\n").doc;
        let b = run_lua("bearcad.new()\nbearcad.rect{ width = 20, height = 10 }\n").doc;
        let diffs = document_diff(&a, &b);
        assert!(!diffs.is_empty(), "expected a mismatch");
    }

    #[test]
    fn parameter_and_rect_round_trip() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.add_parameter("w", "24")
            bearcad.rect{ width = "w", height = 12 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            "#,
        );
        let script = document_to_lua(&state.doc);
        assert!(script.contains("bearcad.add_parameter"));
        assert!(!script.contains("bearcad.ui."));
        let rebuilt = run_lua(&script);
        let diffs = document_diff(&state.doc, &rebuilt.doc);
        assert!(
            diffs.is_empty(),
            "round-trip diffs: {diffs:?}\n--- script ---\n{script}"
        );
    }

    /// #1162: shell ops export as `bearcad.shell{...}` and round-trip.
    #[test]
    fn shell_op_exports_and_round_trips() {
        // Circle profile so open-face refs use round-trippable `profile = "circle"`.
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 10 }
            bearcad.extrude{ circle = 0, distance = 20 }
            bearcad.shell{
                bodies = {0},
                faces = {{ kind = "extrude_cap", extrusion = 0, profile = "circle", profile_index = 0, top = true }},
                thickness = "1"
            }
            "#,
        );
        assert_eq!(state.doc.shell_ops.len(), 1);
        let script = document_to_lua(&state.doc);
        assert!(
            !script.contains("bearcad.ui."),
            "export must not use ui module:\n{script}"
        );
        assert!(
            script.contains("bearcad.shell"),
            "expected shell in export:\n{script}"
        );
        assert!(
            script.contains("thickness"),
            "expected thickness in export:\n{script}"
        );
        assert!(
            script.contains("extrude_cap") || script.contains("faces"),
            "expected open faces in export:\n{script}"
        );

        let rebuilt = run_lua(&script);
        let diffs = document_diff(&state.doc, &rebuilt.doc);
        assert!(
            diffs.is_empty(),
            "round-trip diffs: {diffs:?}\n--- script ---\n{script}"
        );
    }

    #[test]
    fn blank_document_is_blank() {
        assert!(document_is_blank(&Document::default()));
    }

    #[test]
    fn geometry_makes_document_non_blank() {
        let state = run_lua(
            r#"
            bearcad.new()
            bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }
            "#,
        );
        assert!(!document_is_blank(&state.doc));
    }

    /// #1160: File → Import → Lua Script… (and `bearcad.import_lua`) replays an export.
    #[test]
    fn import_lua_into_blank_rebuilds_document() {
        let exported = run_lua(
            r#"
            bearcad.new()
            bearcad.rect{ width = 40, height = 20, x = 0, y = 0 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 10 }
            "#,
        );
        let script = document_to_lua(&exported.doc);
        let path = std::env::temp_dir().join(format!(
            "bearcad_import_lua_blank_{}.lua",
            std::process::id()
        ));
        std::fs::write(&path, &script).unwrap();
        let path_str = path.to_string_lossy().replace('\\', "\\\\");
        let imported = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.import_lua("{path_str}")
            "#
        ));
        let _ = std::fs::remove_file(&path);
        let diffs = document_diff(&exported.doc, &imported.doc);
        assert!(
            diffs.is_empty(),
            "import_lua into blank must match export: {diffs:?}\n--- script ---\n{script}"
        );
    }

    #[test]
    fn import_lua_refuses_non_blank_without_force() {
        let path = std::env::temp_dir().join(format!(
            "bearcad_import_lua_refuse_{}.lua",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "bearcad.new()\nbearcad.rect{ width = 10, height = 10 }\n",
        )
        .unwrap();
        let path_str = path.to_string_lossy().replace('\\', "\\\\");
        let mut runner = ScriptRunner::from_lua_source(&format!(
            r#"
            bearcad.new()
            bearcad.line{{ x = 0, y = 0, x1 = 5, y1 = 0 }}
            local ok, err = pcall(function() bearcad.import_lua("{path_str}") end)
            assert(not ok, "import into non-blank without force must raise")
            assert(tostring(err):find("not blank") or tostring(err):find("force"),
                   "error should mention blank/force: " .. tostring(err))
            assert(bearcad.count("line") == 1, "original geometry must remain")
            "#
        ))
        .unwrap();
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        let vp = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(960.0, 560.0));
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, Some(vp), &ctx);
        }
        let _ = std::fs::remove_file(&path);
        assert!(runner.error.is_none(), "script error: {:?}", runner.error);
        assert_eq!(state.doc.lines.len(), 1);
    }

    #[test]
    fn import_lua_force_replaces_non_blank() {
        let exported = run_lua(
            r#"
            bearcad.new()
            bearcad.circle{ x = 0, y = 0, r = 5 }
            "#,
        );
        let script = document_to_lua(&exported.doc);
        let path = std::env::temp_dir().join(format!(
            "bearcad_import_lua_force_{}.lua",
            std::process::id()
        ));
        std::fs::write(&path, &script).unwrap();
        let path_str = path.to_string_lossy().replace('\\', "\\\\");
        let imported = run_lua(&format!(
            r#"
            bearcad.new()
            bearcad.rect{{ width = 99, height = 99 }}
            bearcad.import_lua{{ path = "{path_str}", force = true }}
            "#
        ));
        let _ = std::fs::remove_file(&path);
        assert_eq!(imported.doc.circles.len(), 1);
        assert!(imported.doc.lines.is_empty(), "force import replaces prior geometry");
        let diffs = document_diff(&exported.doc, &imported.doc);
        assert!(diffs.is_empty(), "force import must match export: {diffs:?}");
    }
}
