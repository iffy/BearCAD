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
                if !param.primary {
                    // New parameters default primary for plain values; force secondary when set.
                    let index = doc
                        .parameters
                        .iter()
                        .filter(|(_, p)| p.source.is_none())
                        .position(|(_, p)| p.name == param.name && p.expression == param.expression)
                        .unwrap_or(0);
                    // Count free parameters emitted so far for a stable ordinal at replay.
                    let _ = index;
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
    }
    for line in doc.lines.values_mut() {
        line.length_dim_offset = None;
    }
    for circle in doc.circles.values_mut() {
        circle.diameter_dim_offset = None;
    }
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
                diffs.push(format!("{} content differs ({} entries)", $label, na));
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
            | HierarchyNode::Body(_)
            | HierarchyNode::UnitChild { .. }
            | HierarchyNode::DrawingDimension { .. }
            | HierarchyNode::DrawingProjection { .. }
            | HierarchyNode::DrawingAnnotation { .. }
            | HierarchyNode::EdgeTreatment { .. }
            | HierarchyNode::Component(_) => {}

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
                // Prefer offset-from-parent form when the definition is a simple face offset.
                let offset = plane.definition.offset_mm;
                let origin = plane.origin;
                let normal = plane.normal;
                out.push_str(&format!(
                    "bearcad.plane{{ offset = {offset}, origin = {{{}, {}, {}}}, normal = {{{}, {}, {}}} }}\n",
                    origin.x, origin.y, origin.z, normal.x, normal.y, normal.z
                ));
                if let Some(name) = &plane.name {
                    let ord = self
                        .doc
                        .construction_planes
                        .keys()
                        .position(|k| k == key)
                        .unwrap_or(0);
                    out.push_str(&format!(
                        "bearcad.set_name({{ kind = \"construction_plane\", index = {ord} }}, {name:?})\n"
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
                                self.doc
                                    .extrusions
                                    .keys()
                                    .position(|k| k == key)
                                    .unwrap_or(0),
                                et.edge,
                            )];
                            let instr = Instruction::EdgeTreatment {
                                edges,
                                kind: et.kind,
                                amount: et.amount,
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
            HierarchyNode::EdgeTreatmentOp(key) => {
                self.close_sketch(out);
                if let Some(op) = self.doc.edge_treatment_ops.get(key) {
                    let edges: Vec<_> = op
                        .edges
                        .iter()
                        .filter_map(|te| {
                            let o = self
                                .doc
                                .extrusions
                                .keys()
                                .position(|k| k == te.extrusion)?;
                            Some((o, te.edge))
                        })
                        .collect();
                    if !edges.is_empty() {
                        out.push_str(
                            &Instruction::EdgeTreatment {
                                edges,
                                kind: op.kind,
                                amount: op.amount,
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
                    self.ensure_sketch(op.sketch, out);
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
                    self.ensure_sketch(op.sketch, out);
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
                    out.push_str(&format!(
                        "bearcad.repeat_sketch{{ sketch = {sketch}, lines = {{{}}}, circles = {{{}}}, du = {}, dv = {}, mode = {mode:?}, count = {:?}, spacing = {:?}, length = {:?} }}\n",
                        list_usizes(&lines),
                        list_usizes(&circles),
                        op.dir_u,
                        op.dir_v,
                        op.count,
                        op.spacing,
                        op.length
                    ));
                }
            }
            HierarchyNode::SketchMirrorOp(key) => {
                if let Some(op) = self.doc.sketch_mirror_ops.get(key) {
                    self.ensure_sketch(op.sketch, out);
                    let sketch = self
                        .doc
                        .sketches
                        .keys()
                        .position(|k| k == op.sketch)
                        .unwrap_or(0);
                    let mirror = self
                        .doc
                        .lines
                        .keys()
                        .position(|k| k == op.line)
                        .unwrap_or(0);
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
                    self.ensure_sketch(op.sketch, out);
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
                // Emit as individual chamfer_vertex/fillet_vertex per corner.
                if let Some(op) = self.doc.sketch_vertex_treatment_ops.get(key) {
                    self.ensure_sketch(op.sketch, out);
                    for corner in &op.corners {
                        let Some(&la) = op.line_targets.get(corner.a) else {
                            continue;
                        };
                        let point = ConstraintPoint::LineEndpoint {
                            line: la,
                            end: corner.a_end,
                        };
                        out.push_str(
                            &Instruction::VertexTreatment {
                                point,
                                kind: corner.kind,
                                amount: corner.amount.clone(),
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
                        let bord = self
                            .doc
                            .bodies
                            .keys()
                            .position(|k| k == view.body)
                            .unwrap_or(0);
                        out.push_str(
                            &Instruction::AddDrawingView {
                                drawing: dord,
                                body: bord,
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
                }
                for ann in d.annotations.values() {
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
            if let Some(rect) = find_rect_from_line(self.doc, key) {
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
        if let Some(rect) = find_rect_from_line(self.doc, key) {
            self.emit_rect(rect, out);
            return;
        }
        self.ensure_sketch(line.sketch, out);
        let dim = line_length_expr(self.doc, key);
        if let Some(ck) = line_length_constraint_key(self.doc, key) {
            self.absorbed_constraints.insert(ck);
        }
        let instr = Instruction::CreateLine {
            x0: line.x0,
            y0: line.y0,
            x1: line.x1,
            y1: line.y1,
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
        let diameter_expr =
            circle_diameter_expr(self.doc, key).or_else(|| Some((circle.r * 2.0).to_string()));
        if let Some(ck) = circle_diameter_constraint_key(self.doc, key) {
            self.absorbed_constraints.insert(ck);
        }
        let instr = Instruction::CreateCircle {
            cx: circle.cx,
            cy: circle.cy,
            r: circle.r,
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
        self.ensure_sketch(c.sketch, out);
        emit_constraint(self.doc, c, out);
        self.absorbed_constraints.insert(key);
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
        };
        out.push_str(&instr.as_lua_in(Some(self.doc)));
        out.push('\n');
    }

    fn emit_rect(&mut self, rect: RectGroup, out: &mut String) {
        let Some(line0) = self.doc.lines.get(rect.lines[0]) else {
            return;
        };
        self.ensure_sketch(line0.sketch, out);
        let w_expr = line_length_expr(self.doc, rect.lines[0]);
        let h_expr = line_length_expr(self.doc, rect.lines[1]);
        for &lk in &rect.lines {
            self.emitted_lines.insert(lk);
            if let Some(ck) = line_length_constraint_key(self.doc, lk) {
                self.absorbed_constraints.insert(ck);
            }
        }
        for ck in rect.constraints {
            self.absorbed_constraints.insert(ck);
        }
        let instr = Instruction::CreateRect {
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
            width_expr: w_expr,
            height_expr: h_expr,
        };
        out.push_str(&instr.as_lua_in(Some(self.doc)));
        out.push('\n');
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────────

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
fn find_rect_from_line(doc: &Document, start: crate::model::LineKey) -> Option<RectGroup> {
    let l0 = doc.lines.get(start)?;
    if l0.bezier.is_some() || l0.projection.is_some() || l0.shadow {
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
        if l.sketch != sketch || k == start || l.bezier.is_some() || l.projection.is_some() {
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
    Some(RectGroup {
        x,
        y,
        w,
        h,
        lines,
        constraints,
    })
}

fn emit_constraint(doc: &Document, c: &crate::model::Constraint, out: &mut String) {
    match &c.kind {
        ConstraintKind::Distance { target } => match target {
            DistanceTarget::LineLength(i) => {
                let ord = doc.lines.keys().position(|k| k == *i).unwrap_or(0);
                out.push_str(&format!(
                    "bearcad.add_constraint({{ kind = \"line\", index = {ord} }}, {:?})\n",
                    c.expression
                ));
            }
            DistanceTarget::CircleDiameter(i) => {
                let ord = doc.circles.keys().position(|k| k == *i).unwrap_or(0);
                out.push_str(&format!(
                    "bearcad.add_constraint({{ kind = \"circle\", index = {ord} }}, {:?})\n",
                    c.expression
                ));
            }
            DistanceTarget::PointPointDistance { anchor, mover, .. } => {
                out.push_str(&format!(
                    "bearcad.add_constraint({{ kind = \"point_point\", anchor = {}, mover = {} }}, {:?})\n",
                    constraint_point_table(doc, anchor),
                    constraint_point_table(doc, mover),
                    c.expression
                ));
            }
            DistanceTarget::PointLineDistance { point, line, .. } => {
                out.push_str(&format!(
                    "bearcad.add_constraint({{ kind = \"point_line\", point = {}, line = {} }}, {:?})\n",
                    constraint_point_table(doc, point),
                    constraint_line_table(doc, line),
                    c.expression
                ));
            }
            DistanceTarget::LineLineDistance { line_a, line_b, .. } => {
                out.push_str(&format!(
                    "bearcad.add_constraint({{ kind = \"line_line\", a = {}, b = {} }}, {:?})\n",
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
                "bearcad.add_angle_constraint{{ a = {a}, b = {b}, sign = {rotation_sign}, value = {:?} }}\n",
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
                        "bearcad.select({{ kind = \"line\", index = {ord} }})\n"
                    ));
                    out.push_str(&format!("bearcad.add_geometric_constraint({name:?})\n"));
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
                out.push_str(&format!("bearcad.select({ea})\n"));
                out.push_str(&format!("bearcad.select({eb}, true)\n"));
                out.push_str("bearcad.add_geometric_constraint(\"coincident\")\n");
            }
        }
        ConstraintKind::Midpoint { point, line } => {
            out.push_str(&format!(
                "bearcad.select({})\n",
                constraint_point_table(doc, point)
            ));
            out.push_str(&format!(
                "bearcad.select({}, true)\n",
                constraint_line_table(doc, line)
            ));
            out.push_str("bearcad.add_geometric_constraint(\"midpoint\")\n");
        }
        ConstraintKind::Tangent { a, b } => {
            out.push_str(&format!(
                "bearcad.select({})\n",
                constraint_point_table(doc, a)
            ));
            out.push_str(&format!(
                "bearcad.select({}, true)\n",
                constraint_point_table(doc, b)
            ));
            out.push_str("bearcad.add_geometric_constraint(\"tangent\")\n");
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
    out.push_str(&format!("bearcad.select({})\n", constraint_line_table(doc, a)));
    out.push_str(&format!(
        "bearcad.select({}, true)\n",
        constraint_line_table(doc, b)
    ));
    out.push_str(&format!("bearcad.add_geometric_constraint({kind:?})\n"));
}

fn entity_select(doc: &Document, e: &ConstraintEntity) -> Option<String> {
    match e {
        ConstraintEntity::Point(p) => Some(constraint_point_table(doc, p)),
        ConstraintEntity::Line(l) => Some(constraint_line_table(doc, l)),
        ConstraintEntity::Circle(i) => {
            let ord = doc.circles.keys().position(|k| k == *i).unwrap_or(0);
            Some(format!("{{ kind = \"circle\", index = {ord} }}"))
        }
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
            format!("{{ kind = \"line\", index = {ord}, [\"end\"] = \"{end}\" }}")
        }
        ConstraintPoint::CircleCenter(c) => {
            let ord = doc.circles.keys().position(|k| k == *c).unwrap_or(0);
            format!("{{ kind = \"circle\", index = {ord}, point = true }}")
        }
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
            bearcad.parameter("add", "w", "24")
            bearcad.rect{ width = "w", height = 12 }
            bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 5 }
            "#,
        );
        let script = document_to_lua(&state.doc);
        assert!(script.contains("bearcad.parameter"));
        assert!(!script.contains("bearcad.ui."));
        let rebuilt = run_lua(&script);
        let diffs = document_diff(&state.doc, &rebuilt.doc);
        assert!(
            diffs.is_empty(),
            "round-trip diffs: {diffs:?}\n--- script ---\n{script}"
        );
    }
}
