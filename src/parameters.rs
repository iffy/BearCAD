//! Document parameters: named length or angle expressions that drive sketch dimensions.

use crate::actions::{Action, ActionResult, AppState};
use crate::constraints::{
    find_distance_constraint, propagate_parameter_rename_to_constraints, solve_document_constraints,
};
use crate::icons::{icon_button, IconId};
use crate::document_health::HealthStatus;
use crate::model::{
    effective_length_unit, DistanceTarget, Document, Parameter, ParameterKey, ParameterSource,
};
use crate::value::{
    eval_parameter_in_doc, expression_references_document_parameter,
    format_angle_display_in, format_length_display_in, format_unknown_variable_error,
    has_angle_unit_suffix, is_valid_parameter_name, parameter_name_conflicts_with_unit,
    parameter_names_referenced_in_expression, substitute_parameter_name,
    unknown_variables_in_parameter_expression, valid_parameter_expression_with_params,
    EvaluatedParameter,
};
use eframe::egui::{self, Color32, Id, Key, RichText};

pub const PANE_TITLE: &str = "Parameters";

const NEW_NAME_ID: &str = "bearcad_parameters_new_name";
const NEW_VALUE_ID: &str = "bearcad_parameters_new_value";
const INVALID_TEXT: Color32 = Color32::from_rgb(220, 80, 80);
const UNSTABLE_TEXT: Color32 = Color32::from_rgb(255, 180, 60);

fn styled_parameter_label(label: &str, status: HealthStatus) -> RichText {
    let text = RichText::new(label);
    match status {
        HealthStatus::Healthy => text,
        HealthStatus::Invalid => text.color(INVALID_TEXT),
        HealthStatus::Unstable => text.color(UNSTABLE_TEXT),
    }
}

fn param_name_id(key: ParameterKey) -> Id {
    Id::new(("bearcad_parameters_name", key.index(), key.generation()))
}

fn param_value_id(key: ParameterKey) -> Id {
    Id::new(("bearcad_parameters_value", key.index(), key.generation()))
}

/// Whether a stored parameter value should show computed + expression text.
pub fn parameter_value_is_expression(doc: &Document, expression: &str) -> bool {
    let expr = expression.trim();
    if expr.is_empty() {
        return false;
    }
    if expr.contains(['+', '*', '/', '(', ')']) {
        return true;
    }
    if expr.chars().skip(1).any(|c| c == '-') {
        return true;
    }
    has_angle_unit_suffix(expr) || expression_references_document_parameter(doc, expr)
}

/// Evaluated value label for parameter autocomplete rows.
pub fn format_parameter_autocomplete_value(doc: &Document, index: ParameterKey) -> String {
    let Some(param) = doc.parameters.get(index) else {
        return String::new();
    };
    match eval_parameter_in_doc(&param.expression, doc) {
        Some(EvaluatedParameter::LengthMm(v)) => {
            format_length_display_in(v, doc.default_length_unit)
        }
        Some(EvaluatedParameter::AngleRad(v)) => format_angle_display_in(v, doc.default_angle_unit),
        None => param.expression.clone(),
    }
}

/// Value-column label for a stored parameter expression.
pub fn format_parameter_value_display(doc: &Document, expression: &str) -> String {
    let expr = expression.trim();
    if !parameter_value_is_expression(doc, expr) {
        return expr.to_string();
    }
    match eval_parameter_in_doc(expr, doc) {
        Some(EvaluatedParameter::LengthMm(v)) => {
            let computed = format_length_display_in(v, doc.default_length_unit);
            // #484: when the typed text is numerically identical to the computed
            // display (e.g. `10mm` vs `10.0 mm`), show only the stored expression.
            if crate::expression_input::canonical_value_text(expr)
                == crate::expression_input::canonical_value_text(&computed)
            {
                expr.to_string()
            } else {
                format!("{} ({expr})", computed)
            }
        }
        Some(EvaluatedParameter::AngleRad(v)) => {
            let computed = format_angle_display_in(v, doc.default_angle_unit);
            if crate::expression_input::canonical_value_text(expr)
                == crate::expression_input::canonical_value_text(&computed)
            {
                expr.to_string()
            } else {
                format!("{} ({expr})", computed)
            }
        }
        None => expr.to_string(),
    }
}

/// Source geometry of the derived parameter whose **name** field currently holds keyboard
/// focus (#536): the elements its value is measured from, to green-highlight in the viewport.
/// Empty unless a derived parameter's name field (not its read-only value) is focused.
pub fn focused_derived_parameter_source(
    ctx: &egui::Context,
    doc: &Document,
) -> Vec<crate::hierarchy::SceneElement> {
    let Some(focused) = ctx.memory(|m| m.focused()) else {
        return Vec::new();
    };
    doc.parameters
        .iter()
        .find_map(|(index, param)| {
            if focused != param_name_id(index) {
                return None;
            }
            param.source.as_ref().map(derived_source_elements)
        })
        .unwrap_or_default()
}

/// Name of the parameter whose name/value field currently holds keyboard focus, if any.
pub fn focused_parameter_name(ctx: &egui::Context, doc: &Document) -> Option<String> {
    let focused = ctx.memory(|m| m.focused())?;
    doc.parameters.iter().find_map(|(index, param)| {
        (focused == param_name_id(index) || focused == param_value_id(index))
            .then(|| param.name.clone())
    })
}

fn pane_element_for_constraint_line(line: crate::model::ConstraintLine) -> crate::hierarchy::SceneElement {
    use crate::hierarchy::SceneElement;
    use crate::model::ConstraintLine;
    match line {
        ConstraintLine::Line(index) => SceneElement::Line(index),
        // A face's own edge tracks the feature that produced its face, same as elsewhere.
        // No owning feature means nothing to point the pane at (#1055): the origin stands in.
        ConstraintLine::FaceEdge { face, .. } => {
            crate::hierarchy::face_owner_element(&face).unwrap_or(SceneElement::Origin)
        }
        // A sketch axis belongs to no plane of its own; the origin stands in (#1055).
        ConstraintLine::OriginAxis(_) => SceneElement::Origin,
    }
}

fn pane_element_for_constraint_point(
    point: crate::model::ConstraintPoint,
) -> crate::hierarchy::SceneElement {
    use crate::hierarchy::SceneElement;
    use crate::model::ConstraintPoint;
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => SceneElement::Line(line),
        ConstraintPoint::CircleCenter(circle) => SceneElement::Circle(circle),
        ConstraintPoint::TextAnchor { text, .. } => SceneElement::SketchText(text),
        ConstraintPoint::ImageCalibrationPoint { image, .. } => SceneElement::Image(image),
        ConstraintPoint::FaceVertex { face, .. } => {
            crate::hierarchy::face_owner_element(&face).unwrap_or(SceneElement::Origin)
        }
    }
}

/// Elements (constraints and the geometry they drive) whose expression references `name`.
pub fn elements_using_parameter(
    doc: &Document,
    name: &str,
) -> std::collections::HashSet<crate::hierarchy::SceneElement> {
    use crate::hierarchy::SceneElement;
    use crate::model::{ConstraintKind, DistanceTarget};
    let mut elements = std::collections::HashSet::new();
    let known = [name];
    // A derived parameter highlights the geometry that defines its value (#432).
    for param in doc.parameters.values().filter(|p| p.name == name) {
        if let Some(source) = &param.source {
            elements.extend(derived_source_elements(source));
        }
    }
    for (index, constraint) in doc.constraints.iter() {
        if parameter_names_referenced_in_expression(&constraint.expression, &known).is_empty() {
            continue;
        }
        elements.insert(SceneElement::Constraint(index));
        match constraint.kind.clone() {
            ConstraintKind::Distance { target } => match target {
                DistanceTarget::LineLength(i) => {
                    elements.insert(SceneElement::Line(i));
                }
                DistanceTarget::CircleDiameter(i) => {
                    elements.insert(SceneElement::Circle(i));
                }
                DistanceTarget::LineLineDistance { line_a, line_b, .. } => {
                    elements.insert(pane_element_for_constraint_line(line_a));
                    elements.insert(pane_element_for_constraint_line(line_b));
                }
                DistanceTarget::PointPointDistance { anchor, mover, .. } => {
                    elements.insert(pane_element_for_constraint_point(anchor));
                    elements.insert(pane_element_for_constraint_point(mover));
                }
                DistanceTarget::PointLineDistance { point, line, .. } => {
                    elements.insert(pane_element_for_constraint_point(point));
                    elements.insert(pane_element_for_constraint_line(line));
                }
            },
            ConstraintKind::Angle { line_a, line_b, .. } => {
                elements.insert(pane_element_for_constraint_line(line_a));
                elements.insert(pane_element_for_constraint_line(line_b));
            }
            _ => {}
        }
    }
    // Extrusions whose distance expression references the parameter (#620).
    for (ei, ext) in doc.extrusions.iter() {
        if !parameter_names_referenced_in_expression(&ext.expression, &known).is_empty() {
            elements.insert(SceneElement::Extrusion(ei));
        }
    }
    elements
}

pub fn parameter_field_focused(ctx: &egui::Context, doc: &Document) -> bool {
    ctx.memory(|m| {
        m.focused().is_some_and(|id| {
            if id == Id::new(NEW_NAME_ID) || id == Id::new(NEW_VALUE_ID) {
                return true;
            }
            doc.parameters
                .keys()
                .any(|index| id == param_name_id(index) || id == param_value_id(index))
        })
    })
}

/// Which cell is being edited in the parameters table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterEditCell {
    Name(ParameterKey),
    Value(ParameterKey),
}

/// Transient UI state for the parameters pane.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParametersPaneState {
    pub editing: Option<ParameterEditCell>,
    pub draft: String,
    pub new_name: String,
    pub new_value: String,
    /// Focus the new-parameter name field on the next frame.
    pub focus_new_name: bool,
    /// Focus the new-parameter value field on the next frame.
    pub focus_new_value: bool,
    /// Focus the active edit cell once after [`begin_edit`].
    pub editing_focus: bool,
    /// Inline validation or action feedback shown under the table.
    pub message: Option<String>,
    /// Whether the new-parameter name field has focus (mirrored each frame for the
    /// tutorial's "tap the name box" predicate).
    pub new_name_focused: bool,
    /// The same for the value field, so the tutorial can ask for that tap on its own step
    /// (#861).
    pub new_value_focused: bool,
    /// Name of the parameter whose row the pointer is over, mirrored each frame (#620):
    /// the viewport highlights everything that parameter drives in green.
    pub hovered_name: Option<String>,
    /// Show the selected unit's secondary parameters too (#728). Ephemeral, off by
    /// default: a unit leads with its primary knobs.
    pub show_unit_secondary: bool,
    /// The unit parameter (by name) whose value cell is being edited (#728).
    pub unit_editing: Option<String>,
    pub unit_draft: String,
    pub unit_editing_focus: bool,
    /// Parameters whose gear-options panel is open (#1176). Multiple may be open at once;
    /// each gear toggles only its own row.
    pub options_open: std::collections::HashSet<ParameterKey>,
    /// Draft text for an options field being edited: (param, which bound, draft).
    /// Primary is a checkbox and commits immediately.
    pub options_editing: Option<(ParameterKey, ParameterBound)>,
    pub options_draft: String,
    pub options_editing_focus: bool,
}

/// Whether the new-parameter row has enough input to attempt a commit.
pub fn new_parameter_row_ready(pane: &ParametersPaneState) -> bool {
    !pane.new_name.trim().is_empty() && !pane.new_value.trim().is_empty()
}

/// Commit the new-parameter row; clears inputs only on success.
pub fn commit_new_parameter(state: &mut AppState) -> Result<(), String> {
    if !new_parameter_row_ready(&state.parameters_pane) {
        return Err("Enter a name and value".to_string());
    }
    let name = state.parameters_pane.new_name.trim().to_string();
    let expression = state.parameters_pane.new_value.trim().to_string();
    match state.apply(Action::AddParameter { name, expression }) {
        ActionResult::Ok => {
            state.parameters_pane.new_name.clear();
            state.parameters_pane.new_value.clear();
            state.parameters_pane.focus_new_name = true;
            state.parameters_pane.message = None;
            Ok(())
        }
        ActionResult::Err(e) => {
            state.parameters_pane.message = Some(e.clone());
            Err(e)
        }
        ActionResult::NeedsDialog => Err("Unexpected dialog request".to_string()),
    }
}

impl ParametersPaneState {
    pub fn begin_edit(&mut self, cell: ParameterEditCell, current: &str) {
        self.editing = Some(cell);
        self.draft = current.to_string();
        self.editing_focus = true;
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
        self.draft.clear();
        self.editing_focus = false;
    }
}

/// The parameter with this name, if any (#995). Deleting a parameter frees its name — it is
/// gone from the arena, so nothing is left to answer to it.
pub fn parameter_index_by_name(doc: &Document, name: &str) -> Option<ParameterKey> {
    doc.parameters
        .iter()
        .find_map(|(key, p)| (p.name == name).then_some(key))
}

pub fn duplicate_parameter_name(doc: &Document, name: &str, except: Option<ParameterKey>) -> bool {
    parameter_index_by_name(doc, name).is_some_and(|i| except != Some(i))
}

fn unique_parameter_name(doc: &Document, base: &str) -> String {
    if !duplicate_parameter_name(doc, base, None) {
        return base.to_string();
    }
    for suffix in 2..1000 {
        let candidate = format!("{base}{suffix}");
        if !duplicate_parameter_name(doc, &candidate, None) {
            return candidate;
        }
    }
    format!("{base}_{}", doc.parameters.len())
}

/// Whether a line may drive a computed length parameter (alive, no length constraint).
pub fn line_eligible_for_computed_length_parameter(doc: &Document, line_index: crate::model::LineKey) -> bool {
    crate::document_lifecycle::line_alive(doc, line_index)
        && find_distance_constraint(doc, DistanceTarget::LineLength(line_index)).is_none()
}

pub fn computed_parameter_index_for_line(
    doc: &Document,
    line_index: crate::model::LineKey,
) -> Option<ParameterKey> {
    doc.parameters.iter().find_map(|(key, param)| {
        matches!(
            param.source,
            Some(ParameterSource::LineLength(index)) if index == line_index
        )
        .then_some(key)
    })
}

pub fn parameter_value_is_readonly(param: &Parameter) -> bool {
    param.source.is_some()
}

pub fn parameter_source_description(doc: &Document, param: &Parameter) -> Option<String> {
    let gone = |alive: bool| if alive { "" } else { " (deleted)" };
    match param.source.as_ref()? {
        ParameterSource::LineLength(index) => Some(format!(
            "Driven by line {} length{}",
            index.index(),
            gone(crate::document_lifecycle::line_alive(doc, *index))
        )),
        ParameterSource::PointDistance(..) => Some(format!(
            "Driven by point-to-point distance{}",
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
        ParameterSource::LineDistance(a, b) => Some(format!(
            "Driven by distance between lines {} and {}{}",
            a.index(),
            b.index(),
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
        ParameterSource::LineAngle(a, b) => Some(format!(
            "Driven by angle between lines {} and {}{}",
            a.index(),
            b.index(),
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
        ParameterSource::BodyEdgeLength { body, .. } => Some(format!(
            "Driven by an edge of body {}{}",
            body.index(),
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
        ParameterSource::BodyVertexDistance { body_a, body_b, .. } => Some(format!(
            "Driven by the distance between two corners of {}{}",
            if body_a == body_b {
                format!("body {}", body_a.index())
            } else {
                format!("bodies {} and {}", body_a.index(), body_b.index())
            },
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
        ParameterSource::UnitEdgeLength { instance, .. } => Some(format!(
            "Driven by an edge of unit instance {}{}",
            instance.index(),
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
    }
}

/// Evaluate a derived parameter source's current value (#432): `(value, is_angle)` —
/// lengths in mm, angles in degrees. `None` when the referenced geometry is gone (or,
/// for a line pair, no longer classifies the same way).
pub fn derived_source_value(doc: &Document, source: &ParameterSource) -> Option<(f32, bool)> {
    match source {
        ParameterSource::LineLength(index) => {
            let line = doc.lines.get(*index)?;
            Some((line.length(), false))
        }
        ParameterSource::PointDistance(a, b) => {
            let pa = crate::construction::point_world_position(doc, a.clone())?;
            let pb = crate::construction::point_world_position(doc, b.clone())?;
            Some(((pb - pa).length(), false))
        }
        ParameterSource::LineDistance(a, b) => {
            let (a0, a1) = line_world_segment(doc, *a)?;
            let (b0, _) = line_world_segment(doc, *b)?;
            let dir = (a1 - a0).normalize_or_zero();
            if dir == glam::Vec3::ZERO {
                return None;
            }
            let offset = b0 - a0;
            Some(((offset - dir * offset.dot(dir)).length(), false))
        }
        ParameterSource::LineAngle(a, b) => {
            let (a0, a1) = line_world_segment(doc, *a)?;
            let (b0, b1) = line_world_segment(doc, *b)?;
            let da = (a1 - a0).normalize_or_zero();
            let db = (b1 - b0).normalize_or_zero();
            if da == glam::Vec3::ZERO || db == glam::Vec3::ZERO {
                return None;
            }
            Some((da.dot(db).clamp(-1.0, 1.0).acos().to_degrees(), true))
        }
        ParameterSource::BodyEdgeLength { body, a, b } => {
            let (p0, p1) = body_edge_world_segment(doc, *body, *a, *b)?;
            Some(((p1 - p0).length(), false))
        }
        ParameterSource::BodyVertexDistance { body_a, a, body_b, b } => {
            let pa = body_vertex_world_position(doc, *body_a, *a)?;
            let pb = body_vertex_world_position(doc, *body_b, *b)?;
            Some(((pb - pa).length(), false))
        }
        // Analytic unit edge (#724): re-resolves against the instance's current rebuild,
        // so the value follows the unit's parameter overrides.
        ParameterSource::UnitEdgeLength { instance, face, edge } => {
            let (p, q) = crate::units::unit_edge_world_segment(doc, *instance, face, *edge)?;
            Some(((q - p).length(), false))
        }
    }
}

/// The live world endpoints of a body feature edge identified by its quantized key (#647) —
/// the same identity `SceneElement::BodyEdge` carries, matched against the body's current
/// edge chains (either endpoint order). `None` once the mesh no longer has that edge.
pub fn body_edge_world_segment(
    doc: &Document,
    body: crate::model::BodyKey,
    a: [i32; 3],
    b: [i32; 3],
) -> Option<(glam::Vec3, glam::Vec3)> {
    // A shadow body still has real geometry — it's just consumed by an operation — and a Move
    // shadows its own inputs (#650), so shadows resolve here; only a deleted body doesn't.
    doc.bodies.get(body)?;
    // Reentrant un-posed access (#650/#897): the cached mesh when the cache is free, a
    // fresh build when resolving from inside the cache's own borrow. Un-posed on purpose —
    // Move snap points and joint frames are body-local references.
    let solid = crate::extrude::body_solid_mesh_unposed(doc, body)?;
    for chain in crate::gpu_viewport::solid_mesh_edge_chains(&solid) {
        let (ca, cb) = crate::gpu_viewport::chain_canonical_segment(&chain);
        let (ka, kb) = (
            crate::hierarchy::quantize_body_point(ca),
            crate::hierarchy::quantize_body_point(cb),
        );
        if (ka, kb) == (a, b) || (ka, kb) == (b, a) {
            return Some((ca, cb));
        }
    }
    None
}

/// The live world position of a body mesh corner identified by its quantized key (#647).
/// `None` once the mesh no longer has a corner there.
pub fn body_vertex_world_position(
    doc: &Document,
    body: crate::model::BodyKey,
    key: [i32; 3],
) -> Option<glam::Vec3> {
    // Shadow bodies resolve too, for the same reason as `body_edge_world_segment`.
    doc.bodies.get(body)?;
    // Reentrant un-posed access (#650/#897): the cached mesh when the cache is free, a
    // fresh build when resolving from inside the cache's own borrow. Un-posed on purpose —
    // Move snap points and joint frames are body-local references.
    let solid = crate::extrude::body_solid_mesh_unposed(doc, body)?;
    solid
        .triangles
        .iter()
        .flatten()
        .copied()
        .find(|p| crate::hierarchy::quantize_body_point(*p) == key)
}

fn line_world_segment(
    doc: &Document,
    index: crate::model::LineKey,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let line = doc.lines.get(index)?;
    let frame = crate::face::sketch_geometry_frame(doc, line.sketch)?;
    Some((
        crate::face::local_to_world(&frame, line.x0, line.y0),
        crate::face::local_to_world(&frame, line.x1, line.y1),
    ))
}

/// The default name a derived parameter for `source` would get (#629) — the same choices
/// [`add_derived_parameter`] makes — so the Dimension tool's name box can prefill with
/// editable text instead of an opaque "auto".
pub fn default_derived_parameter_name(doc: &Document, source: &ParameterSource) -> String {
    match source {
        ParameterSource::LineLength(line) => default_computed_parameter_name_for_line(doc, *line),
        ParameterSource::PointDistance(..) => unique_parameter_name(doc, "distance"),
        ParameterSource::LineDistance(a, b) => {
            unique_parameter_name(doc, &format!("line{}_line{}_distance", a.index(), b.index()))
        }
        ParameterSource::LineAngle(a, b) => {
            unique_parameter_name(doc, &format!("line{}_line{}_angle", a.index(), b.index()))
        }
        ParameterSource::BodyEdgeLength { body, .. } => {
            unique_parameter_name(doc, &format!("body{}_edge_length", body.index()))
        }
        ParameterSource::BodyVertexDistance { body_a, .. } => {
            unique_parameter_name(doc, &format!("body{}_corner_distance", body_a.index()))
        }
        ParameterSource::UnitEdgeLength { instance, .. } => {
            unique_parameter_name(doc, &format!("unit{}_edge_length", instance.index()))
        }
    }
}

pub fn default_computed_parameter_name_for_line(doc: &Document, line_index: crate::model::LineKey) -> String {
    unique_parameter_name(doc, &format!("line{}_length", line_index.index()))
}

/// Update read-only parameter expressions from their geometry sources.
pub fn sync_computed_parameters(doc: &mut Document) {
    // Values are computed against an immutable view first (the derived evaluators walk
    // sketches/frames), then written back.
    let updates: Vec<(ParameterKey, String)> = doc
        .parameters
        .iter()
        .filter_map(|(i, p)| {
            let source = p.source.as_ref()?;
            let (value, is_angle) = derived_source_value(doc, source)?;
            let expression = if is_angle {
                crate::value::format_angle_display_in(value.to_radians(), doc.default_angle_unit)
            } else {
                let unit = match source {
                    ParameterSource::LineLength(index) => doc
                        .lines
                        .get(*index)
                        .map(|l| effective_length_unit(doc, l.sketch))
                        .unwrap_or(doc.default_length_unit),
                    _ => doc.default_length_unit,
                };
                format_length_display_in(value, unit)
            };
            Some((i, expression))
        })
        .collect();
    for (i, expression) in updates {
        doc.parameters[i].expression = expression;
    }
}

pub fn require_parameter_value_editable(param: &Parameter) -> Result<(), String> {
    if parameter_value_is_readonly(param) {
        Err("Parameter value is read-only".to_string())
    } else {
        Ok(())
    }
}

pub fn add_computed_parameter_from_line_length(
    doc: &mut Document,
    line_index: crate::model::LineKey,
    name: Option<String>,
) -> Result<ParameterKey, String> {
    if !crate::document_lifecycle::line_alive(doc, line_index) {
        return Err(format!("Line {} not found", line_index.index()));
    }
    if find_distance_constraint(doc, DistanceTarget::LineLength(line_index)).is_some() {
        return Err("Line length is constrained".to_string());
    }
    if computed_parameter_index_for_line(doc, line_index).is_some() {
        return Err("A parameter already tracks this line's length".to_string());
    }
    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| default_computed_parameter_name_for_line(doc, line_index));
    validate_new_parameter_name(doc, &name, None)?;
    let length = doc.lines[line_index].length();
    let unit = effective_length_unit(doc, doc.lines[line_index].sketch);
    let index = doc.parameters.insert(Parameter {
        name,
        expression: format_length_display_in(length, unit),
        primary: false,
        minimum: None,
        maximum: None,
        step: None,
        source: Some(ParameterSource::LineLength(line_index)),
    });
    doc.shape_order.push(crate::model::ShapeKind::Parameter);
    recompute_document_geometry(doc)?;
    Ok(index)
}

/// Classify the current selection as a derived-parameter source (#432):
/// one line → its length; two points → their distance; two parallel lines → the distance
/// between them; two non-parallel lines in the same sketch → the angle between them.
pub fn derived_source_from_selection(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
) -> Option<ParameterSource> {
    use crate::hierarchy::SceneElement;
    let ordered = selection.ordered();
    match ordered.as_slice() {
        [SceneElement::Line(i)] => {
            line_eligible_for_computed_length_parameter(doc, *i).then(|| {
                ParameterSource::LineLength(*i)
            })
        }
        [SceneElement::Point(a), SceneElement::Point(b)] => {
            let source = ParameterSource::PointDistance(a.clone(), b.clone());
            derived_source_value(doc, &source).map(|_| source)
        }
        [SceneElement::Line(a), SceneElement::Line(b)] if a != b => {
            let (a0, a1) = line_world_segment(doc, *a)?;
            let (b0, b1) = line_world_segment(doc, *b)?;
            let da = (a1 - a0).normalize_or_zero();
            let db = (b1 - b0).normalize_or_zero();
            if da == glam::Vec3::ZERO || db == glam::Vec3::ZERO {
                return None;
            }
            if da.cross(db).length() < 1e-3 {
                Some(ParameterSource::LineDistance(*a, *b))
            } else if doc.lines.get(*a)?.sketch == doc.lines.get(*b)?.sketch {
                Some(ParameterSource::LineAngle(*a, *b))
            } else {
                None
            }
        }
        // Body geometry measures like sketch geometry does (#647): one feature edge gives its
        // length, two mesh corners give the distance between them.
        [SceneElement::BodyEdge { body, a, b }] => {
            // An edge picked on a unit's materialized body (#724) upgrades to its analytic
            // identity when one exists, so the dimension survives override changes; unit
            // geometry with no analytic face keeps the quantized key (STL-import parity).
            if let Some(crate::model::BodySource::UnitInstance(instance)) =
                doc.bodies.get(*body).map(|bd| &bd.source)
            {
                let (wa, wb) = (
                    crate::hierarchy::dequantize_body_point(*a),
                    crate::hierarchy::dequantize_body_point(*b),
                );
                if let Some((face, edge)) = crate::units::analytic_unit_edge(doc, *instance, wa, wb)
                {
                    let source = ParameterSource::UnitEdgeLength { instance: *instance, face, edge };
                    return derived_source_value(doc, &source).map(|_| source);
                }
            }
            let source = ParameterSource::BodyEdgeLength { body: *body, a: *a, b: *b };
            derived_source_value(doc, &source).map(|_| source)
        }
        [
            SceneElement::BodyVertex { body: body_a, p: a },
            SceneElement::BodyVertex { body: body_b, p: b },
        ] if (body_a, a) != (body_b, b) => {
            let source = ParameterSource::BodyVertexDistance {
                body_a: *body_a,
                a: *a,
                body_b: *body_b,
                b: *b,
            };
            derived_source_value(doc, &source).map(|_| source)
        }
        _ => None,
    }
}

/// Create a read-only parameter driven by `source` (#432). The generalization of
/// [`add_computed_parameter_from_line_length`] to every derived-source kind.
pub fn add_derived_parameter(
    doc: &mut Document,
    source: ParameterSource,
    name: Option<String>,
) -> Result<ParameterKey, String> {
    if let ParameterSource::LineLength(line_index) = source {
        return add_computed_parameter_from_line_length(doc, line_index, name);
    }
    let (value, is_angle) =
        derived_source_value(doc, &source).ok_or("Selection doesn't measure anything")?;
    if doc
        .parameters
        .values()
        .any(|p| p.source.as_ref() == Some(&source))
    {
        return Err("A parameter already tracks this measurement".to_string());
    }
    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| default_derived_parameter_name(doc, &source));
    validate_new_parameter_name(doc, &name, None)?;
    let expression = if is_angle {
        crate::value::format_angle_display_in(value.to_radians(), doc.default_angle_unit)
    } else {
        format_length_display_in(value, doc.default_length_unit)
    };
    let index = doc.parameters.insert(Parameter {
        name,
        expression,
        primary: false,
        minimum: None,
        maximum: None,
        step: None,
        source: Some(source),
    });
    doc.shape_order.push(crate::model::ShapeKind::Parameter);
    recompute_document_geometry(doc)?;
    Ok(index)
}

/// The scene elements a derived parameter's value comes from (#432), for highlighting.
pub fn derived_source_elements(
    source: &ParameterSource,
) -> Vec<crate::hierarchy::SceneElement> {
    use crate::hierarchy::SceneElement;
    match source {
        ParameterSource::LineLength(i) => vec![SceneElement::Line(*i)],
        ParameterSource::PointDistance(a, b) => vec![
            SceneElement::Point(a.clone()),
            SceneElement::Point(b.clone()),
        ],
        ParameterSource::LineDistance(a, b) | ParameterSource::LineAngle(a, b) => {
            vec![SceneElement::Line(*a), SceneElement::Line(*b)]
        }
        ParameterSource::BodyEdgeLength { body, a, b } => {
            vec![SceneElement::BodyEdge { body: *body, a: *a, b: *b }]
        }
        ParameterSource::BodyVertexDistance { body_a, a, body_b, b } => vec![
            SceneElement::BodyVertex { body: *body_a, p: *a },
            SceneElement::BodyVertex { body: *body_b, p: *b },
        ],
        ParameterSource::UnitEdgeLength { instance, .. } => {
            vec![SceneElement::UnitInstance(*instance)]
        }
    }
}

/// Selected unconstrained line that can drive a computed length parameter.
pub fn line_for_computed_parameter_context_menu(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
) -> Option<crate::model::LineKey> {
    let element = selection.single()?;
    let crate::hierarchy::SceneElement::Line(index) = element else {
        return None;
    };
    if computed_parameter_index_for_line(doc, index).is_some() {
        return None;
    }
    line_eligible_for_computed_length_parameter(doc, index).then_some(index)
}

pub fn show_computed_line_length_context_menu(
    response: &egui::Response,
    doc: &Document,
    selection: &crate::selection::SceneSelection,
    on_create: &mut impl FnMut(crate::model::LineKey),
) {
    let Some(line_index) = line_for_computed_parameter_context_menu(doc, selection) else {
        return;
    };
    response.context_menu(|ui| {
        if ui.button("Create parameter from length").clicked() {
            on_create(line_index);
            ui.close();
        }
    });
}

/// Rename `old` to `new` in every expression that references it.
pub fn propagate_parameter_rename(doc: &mut Document, old: &str, new: &str) {
    if old == new {
        return;
    }
    for param in doc.parameters.values_mut() {
        param.expression = substitute_parameter_name(&param.expression, old, new);
    }
    for line in doc.lines.values_mut() {
        if let Some(expr) = &mut line.length_expr {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
    for circle in doc.circles.values_mut() {
        if let Some(expr) = &mut circle.diameter_expr {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
    propagate_parameter_rename_to_constraints(doc, old, new);
}

/// Rewrite one whole (possibly qualified) name across every expression holder the app
/// evaluates (#731): parameters, sketch dimensions and constraints, extrusion depths,
/// Move/Repeat tool fields, text sizes, and unit placements/overrides.
fn substitute_name_everywhere(doc: &mut Document, old: &str, new: &str) {
    propagate_parameter_rename(doc, old, new);
    for extrusion in doc.extrusions.values_mut() {
        extrusion.expression = substitute_parameter_name(&extrusion.expression, old, new);
    }
    for op in doc.move_ops.values_mut() {
        for expr in [&mut op.tx, &mut op.ty, &mut op.tz] {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
    for op in doc.repeat_ops.values_mut() {
        for expr in [&mut op.count, &mut op.spacing, &mut op.length] {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
    for text in doc.sketch_texts.values_mut() {
        text.size_expr = substitute_parameter_name(&text.size_expr, old, new);
    }
    for instance in doc.unit_instances.values_mut() {
        let p = &mut instance.placement;
        for expr in [&mut p.tx, &mut p.ty, &mut p.tz, &mut p.angle] {
            *expr = substitute_parameter_name(expr, old, new);
        }
        for (_, expr) in &mut instance.parameter_overrides {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
}

/// Renaming a unit instance rewrites every qualified reference to it (#731):
/// `old.param` becomes `new.param` (in its backticked spelling where the new name needs
/// one) across everything holding an expression, so the rename never breaks a model.
pub fn propagate_instance_rename(doc: &mut Document, unit: crate::model::UnitKey, old: &str, new: &str) {
    let old = old.trim();
    let new = new.trim();
    if old.is_empty() || new.is_empty() || old == new {
        return;
    }
    let param_names: Vec<String> = doc
        .units
        .get(unit)
        .map(|u| {
            u.document.parameters.values().map(|p| p.name.clone()).collect()
        })
        .unwrap_or_default();
    let spelled_new = if crate::value::is_valid_parameter_name(new) {
        new.to_string()
    } else {
        format!("`{new}`")
    };
    for param in param_names {
        substitute_name_everywhere(
            doc,
            &format!("{old}.{param}"),
            &format!("{spelled_new}.{param}"),
        );
    }
}

/// Re-evaluate sketch constraints and apply solved geometry, then re-resolve associative
/// projections (#140) so they track their source bodies through the change.
pub fn recompute_document_geometry(doc: &mut Document) -> Result<(), String> {
    // Texts re-bake first so anchor constraints solve against current contours (#408).
    rebake_sketch_texts(doc);
    let result = solve_document_constraints(doc);
    crate::projection::refresh_projections(doc);
    rebake_extrusion_distances(doc);
    // Offset outputs track their sources and distance expressions.
    crate::actions::rebuild_sketch_offsets(doc);
    // Mirror outputs track their sources and mirror line (#523).
    crate::actions::rebuild_sketch_mirrors(doc);
    // Chamfer/fillet trimmed copies + bridges track the shadow sources and parametric amount (#538).
    crate::actions::rebuild_sketch_vertex_treatments(doc);
    result
}

/// Re-bake sketch-text glyph outlines from their raw templates (#338), so `{expr}` fields and
/// `size_expr` follow parameter edits. Text with no `{` and a constant/blank `size_expr` still
/// re-bakes harmlessly (identical result); a font that's since gone leaves the existing outlines.
pub fn rebake_sketch_texts(doc: &mut Document) {
    for i in doc.sketch_texts.keys().collect::<Vec<_>>() {
        let t = &doc.sketch_texts[i];
        let (template, family, bold, italic, wrap, size_expr, cur_size) = (
            t.text.clone(),
            t.font_family.clone(),
            t.bold,
            t.italic,
            t.wrap_width,
            t.size_expr.clone(),
            t.size,
        );
        // A parametric size follows its expression; a blank/constant expression keeps the value.
        let size = if size_expr.trim().is_empty() {
            cur_size
        } else {
            crate::value::eval_length_mm_in_doc(&size_expr, doc)
                .map(f32::abs)
                .filter(|s| *s > 0.0)
                .unwrap_or(cur_size)
        };
        let baked = crate::value::interpolate_text(&template, doc);
        if let Some((shaped, bytes)) =
            crate::text::shape_with_system_font_wrapped(&family, bold, italic, size, &baked, wrap)
        {
            let t = &mut doc.sketch_texts[i];
            t.size = size;
            t.contours = shaped.contours;
            t.font_bytes = bytes;
        }
    }
}

/// Re-evaluate each extrusion's stored `distance` from its `expression` (#251), so an extrusion
/// whose distance was typed as a parameter (or any expression) follows edits to that parameter.
/// Extrusions with no expression (plain gizmo-set distances) keep their baked value. The drag
/// direction (sign) is preserved; magnitude comes from the expression.
pub fn rebake_extrusion_distances(doc: &mut Document) {
    for i in doc.extrusions.keys().collect::<Vec<_>>() {
        let (expr, dist) = {
            let e = &doc.extrusions[i];
            (e.expression.clone(), e.distance)
        };
        if expr.trim().is_empty() {
            continue;
        }
        if let Some(mag) = crate::value::eval_length_mm_in_doc(&expr, doc) {
            let sign = if dist < 0.0 { -1.0 } else { 1.0 };
            doc.extrusions[i].distance = mag.abs() * sign;
        }
    }
}

pub fn validate_new_parameter_name(
    doc: &Document,
    name: &str,
    except: Option<ParameterKey>,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Parameter name is required".to_string());
    }
    if name.chars().any(|c| c.is_whitespace()) {
        return Err("Parameter name cannot contain spaces".to_string());
    }
    if parameter_name_conflicts_with_unit(name) {
        return Err(format!("Parameter name '{name}' conflicts with a known unit"));
    }
    if !is_valid_parameter_name(name) {
        return Err(format!(
            "Invalid parameter name '{name}' (use letters, digits, underscore; start with a letter)"
        ));
    }
    if duplicate_parameter_name(doc, name, except) {
        return Err(format!("Parameter '{name}' already exists"));
    }
    Ok(())
}

/// Parameter name/expression pairs for validation, optionally overriding one row or appending a new one.
fn parameter_bindings_for_check(
    doc: &Document,
    param_name: &str,
    expression: &str,
    existing_index: Option<ParameterKey>,
) -> Vec<(String, String)> {
    let mut bindings: Vec<(String, String)> = doc
        .parameters
        .iter()
        .map(|(index, param)| {
            let expr = if existing_index == Some(index) {
                expression.to_string()
            } else {
                param.expression.clone()
            };
            (param.name.clone(), expr)
        })
        .collect();
    // Qualified unit-instance bindings (#729), so `foo.bar` validates and evaluates in a
    // parameter expression like anywhere else. Their names carry a dot, so they can never
    // collide with the document's own names above.
    bindings.extend(
        crate::value::document_parameter_bindings(doc)
            .into_iter()
            .filter(|(name, _)| name.contains('.')),
    );
    if existing_index.is_none() && !bindings.iter().any(|(name, _)| name == param_name) {
        bindings.push((param_name.to_string(), expression.to_string()));
    }
    bindings
}

/// Cycle path starting and ending at the same parameter (e.g. `["A", "B", "C", "A"]`).
pub fn find_parameter_dependency_cycle(
    doc: &Document,
    param_name: &str,
    expression: &str,
    existing_index: Option<ParameterKey>,
) -> Option<Vec<String>> {
    let param_name = param_name.trim();
    if param_name.is_empty() {
        return None;
    }
    let bindings = parameter_bindings_for_check(doc, param_name, expression.trim(), existing_index);
    let known_names: Vec<&str> = bindings.iter().map(|(name, _)| name.as_str()).collect();
    let mut path = Vec::new();
    find_parameter_dependency_cycle_from(param_name, &bindings, &known_names, &mut path)
}

fn find_parameter_dependency_cycle_from(
    name: &str,
    bindings: &[(String, String)],
    known_names: &[&str],
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    if let Some(start) = path.iter().position(|n| n == name) {
        let mut cycle = path[start..].to_vec();
        cycle.push(name.to_string());
        return Some(cycle);
    }
    let expression = bindings
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, expr)| expr.as_str())?;
    path.push(name.to_string());
    for dep in parameter_names_referenced_in_expression(expression, known_names) {
        if let Some(cycle) =
            find_parameter_dependency_cycle_from(&dep, bindings, known_names, path)
        {
            return Some(cycle);
        }
    }
    path.pop();
    None
}

pub fn format_circular_dependency_error(cycle: &[String]) -> String {
    if cycle.is_empty() {
        return "Circular parameter dependency".to_string();
    }
    format!("Circular dependency: {}", cycle.join(" → "))
}

/// Live warning text for a draft expression, or `None` when no cycle is detected.
pub fn parameter_expression_cycle_warning(
    doc: &Document,
    param_name: &str,
    expression: &str,
    existing_index: Option<ParameterKey>,
) -> Option<String> {
    let expression = expression.trim();
    if expression.is_empty() || param_name.trim().is_empty() {
        return None;
    }
    find_parameter_dependency_cycle(doc, param_name, expression, existing_index)
        .map(|cycle| format_circular_dependency_error(&cycle))
}

pub fn validate_document_parameters_no_cycles(doc: &Document) -> Result<(), String> {
    for (index, param) in doc.parameters.iter() {
        if let Some(cycle) = find_parameter_dependency_cycle(
            doc,
            &param.name,
            &param.expression,
            Some(index),
        ) {
            return Err(format_circular_dependency_error(&cycle));
        }
    }
    Ok(())
}

pub fn validate_parameter_expression_for(
    doc: &Document,
    param_name: &str,
    expression: &str,
    existing_index: Option<ParameterKey>,
) -> Result<(), String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("Parameter value is required".to_string());
    }
    if let Some(name) =
        unknown_variables_in_parameter_expression(expression, doc, param_name, existing_index).first()
    {
        return Err(format_unknown_variable_error(name));
    }
    if let Some(cycle) =
        find_parameter_dependency_cycle(doc, param_name, expression, existing_index)
    {
        return Err(format_circular_dependency_error(&cycle));
    }
    let bindings = parameter_bindings_for_check(doc, param_name, expression, existing_index);
    let params: Vec<(&str, &str)> = bindings
        .iter()
        .map(|(name, expr)| (name.as_str(), expr.as_str()))
        .collect();
    if !valid_parameter_expression_with_params(expression, &params) {
        return Err(format!("Invalid expression '{expression}'"));
    }
    Ok(())
}

/// Parse `name=value` inline parameter definition syntax from a dimension field.
pub fn parse_inline_parameter_definition(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let (name, value) = text.split_once('=')?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return None;
    }
    if !is_valid_parameter_name(name) {
        return None;
    }
    Some((name.to_string(), value.to_string()))
}

/// What committing an inline `name=…` entry did (SPEC §5.1.1) — surfaced in the status bar
/// so it's unambiguous whether the name was created, redefined, or merely reused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineParameterCommit {
    /// `name=value` with a fresh name: the parameter was created.
    Created(String),
    /// `name=value` where `name` already existed: its expression was redefined to `value`.
    Redefined(String),
    /// Bare `name=` where `name` already exists: the field was bound to the parameter,
    /// which is left unchanged.
    Reused(String),
}

impl InlineParameterCommit {
    /// Status-bar message describing the outcome.
    pub fn status_message(&self) -> String {
        match self {
            Self::Created(name) => format!("Added parameter {name}"),
            Self::Redefined(name) => format!("Redefined parameter {name}"),
            Self::Reused(name) => format!("Using parameter {name}"),
        }
    }
}

/// Commit inline parameter syntax in a dimension field (SPEC §5.1.1): `name=value` creates
/// the parameter — or **redefines** it when `name` already exists — and bare `name=` of an
/// existing parameter **reuses** it. In every case `text` is replaced with `name` so the
/// field is left bound to the parameter.
pub fn try_commit_inline_parameter_definition(
    doc: &mut Document,
    text: &mut String,
) -> Result<Option<InlineParameterCommit>, String> {
    // Bare `name=`: bind the field to the existing parameter, unchanged.
    if let Some(name) = text.trim().strip_suffix('=') {
        let name = name.trim().to_string();
        if crate::value::is_valid_parameter_name(&name) {
            if parameter_index_by_name(doc, &name).is_some() {
                *text = name.clone();
                return Ok(Some(InlineParameterCommit::Reused(name)));
            }
        }
    }
    let Some((name, value)) = parse_inline_parameter_definition(text) else {
        return Ok(None);
    };
    // `name=value` on an existing name redefines that parameter's expression.
    if let Some(index) = parameter_index_by_name(doc, &name) {
        set_parameter_expression(doc, index, value)?;
        *text = name.clone();
        return Ok(Some(InlineParameterCommit::Redefined(name)));
    }
    add_parameter(doc, name.clone(), value)?;
    *text = name.clone();
    Ok(Some(InlineParameterCommit::Created(name)))
}

/// The `primary` flag a **newly created** parameter starts with (#727): primary when the
/// expression is a plain self-contained value (a bare number, with or without a unit — a
/// knob someone is meant to turn), secondary when it references anything else (derived,
/// usually internal). Computed once at creation; re-computing on later edits would fight
/// the user's own toggle.
pub fn new_parameter_primary_default(expression: &str) -> bool {
    crate::value::eval_length_mm(expression).is_some()
        || crate::value::eval_angle_rad(expression).is_some()
}

pub fn add_parameter(
    doc: &mut Document,
    name: String,
    expression: String,
) -> Result<ParameterKey, String> {
    let name = name.trim().to_string();
    let expression = expression.trim().to_string();
    validate_new_parameter_name(doc, &name, None)?;
    validate_parameter_expression_for(doc, &name, &expression, None)?;
    let index = doc.parameters.insert(Parameter {
        name,
        primary: new_parameter_primary_default(&expression),
        expression,
        minimum: None,
        maximum: None,
        step: None,
        source: None,
    });
    doc.shape_order.push(crate::model::ShapeKind::Parameter);
    recompute_document_geometry(doc)?;
    Ok(index)
}

pub fn set_parameter_name(
    doc: &mut Document,
    index: ParameterKey,
    name: String,
) -> Result<(), String> {
    let name = name.trim().to_string();
    let old = doc
        .parameters
        .get(index)
        .ok_or_else(|| format!("Parameter {index:?} not found"))?
        .name
        .clone();
    if name == old {
        return Ok(());
    }
    validate_new_parameter_name(doc, &name, Some(index))?;
    propagate_parameter_rename(doc, &old, &name);
    doc.parameters[index].name = name;
    recompute_document_geometry(doc)
}

pub fn set_parameter_expression(
    doc: &mut Document,
    index: ParameterKey,
    expression: String,
) -> Result<(), String> {
    let expression = expression.trim().to_string();
    let param = doc
        .parameters
        .get(index)
        .ok_or_else(|| format!("Parameter {index:?} not found"))?;
    require_parameter_value_editable(param)?;
    let param_name = param.name.clone();
    validate_parameter_expression_for(doc, &param_name, &expression, Some(index))?;
    doc.parameters[index].expression = expression;
    recompute_document_geometry(doc)
}

/// Set or clear a parameter bound option (`minimum` / `maximum` / `step`) (#1176).
/// Empty / `None` clears the option. Non-empty expressions must evaluate to the same
/// unit kind as the parameter's default value.
pub fn set_parameter_bound(
    doc: &mut Document,
    index: ParameterKey,
    which: ParameterBound,
    expression: Option<String>,
) -> Result<(), String> {
    let param = doc
        .parameters
        .get(index)
        .ok_or_else(|| format!("Parameter {index:?} not found"))?;
    let cleared = expression
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);
    if cleared {
        let slot = bound_slot_mut(&mut doc.parameters[index], which);
        *slot = None;
        return Ok(());
    }
    let expression = expression.unwrap().trim().to_string();
    let kind = parameter_value_kind(doc, &param.expression).ok_or_else(|| {
        "Parameter default value must evaluate so bounds know length vs angle".to_string()
    })?;
    // Evaluate the bound against the document, but ignore the parameter itself as a
    // binding so a bound can't circularly depend on the value it constrains.
    let bindings: Vec<(String, String)> = doc
        .parameters
        .iter()
        .filter(|(k, _)| *k != index)
        .map(|(_, p)| (p.name.clone(), p.expression.clone()))
        .collect();
    let params: Vec<(&str, &str)> = bindings
        .iter()
        .map(|(n, e)| (n.as_str(), e.as_str()))
        .collect();
    let bound_kind = match kind {
        ParameterValueKind::Length => {
            crate::value::eval_length_mm_with_params(&expression, &params)
                .map(|_| ParameterValueKind::Length)
        }
        ParameterValueKind::Angle => {
            crate::value::eval_angle_rad_with_params(&expression, &params)
                .map(|_| ParameterValueKind::Angle)
        }
    };
    match bound_kind {
        Some(k) if k == kind => {}
        Some(_) => {
            return Err(format!(
                "Bound must be a {} expression",
                kind.label()
            ));
        }
        None => {
            return Err(format!("Invalid {} expression '{expression}'", which.label()));
        }
    }
    // Step must be positive.
    if which == ParameterBound::Step {
        let step_v = match kind {
            ParameterValueKind::Length => {
                crate::value::eval_length_mm_with_params(&expression, &params)
            }
            ParameterValueKind::Angle => {
                crate::value::eval_angle_rad_with_params(&expression, &params)
            }
        };
        if step_v.is_none_or(|v| v <= 0.0) {
            return Err("Step must be a positive value".to_string());
        }
    }
    let slot = bound_slot_mut(&mut doc.parameters[index], which);
    *slot = Some(expression);
    Ok(())
}

/// Which optional bound field on a parameter (#1176).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterBound {
    Minimum,
    Maximum,
    Step,
}

impl ParameterBound {
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimum => "minimum",
            Self::Maximum => "maximum",
            Self::Step => "step",
        }
    }

    /// Status-bar noun for "Updated width minimum".
    pub fn label_status(self) -> &'static str {
        self.label()
    }
}

fn bound_slot_mut(param: &mut Parameter, which: ParameterBound) -> &mut Option<String> {
    match which {
        ParameterBound::Minimum => &mut param.minimum,
        ParameterBound::Maximum => &mut param.maximum,
        ParameterBound::Step => &mut param.step,
    }
}

/// Length vs angle, taken from a parameter's default expression (#1176).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterValueKind {
    Length,
    Angle,
}

impl ParameterValueKind {
    fn label(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Angle => "angle",
        }
    }
}

/// Unit kind of a parameter's default value expression, if it evaluates.
pub fn parameter_value_kind(doc: &Document, expression: &str) -> Option<ParameterValueKind> {
    match eval_parameter_in_doc(expression, doc) {
        Some(EvaluatedParameter::LengthMm(_)) => Some(ParameterValueKind::Length),
        Some(EvaluatedParameter::AngleRad(_)) => Some(ParameterValueKind::Angle),
        None => None,
    }
}

/// Resolved numeric bounds for a parameter in canonical units (mm or rad) (#1176).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParameterLimits {
    pub kind: ParameterValueKind,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,
}

/// Evaluate a parameter's min/max/step against `doc` (the document that owns the
/// parameter — for an import, the unit's embedded document).
pub fn parameter_limits(doc: &Document, param: &Parameter) -> Option<ParameterLimits> {
    let kind = parameter_value_kind(doc, &param.expression)?;
    let eval = |expr: &Option<String>| -> Option<f32> {
        let expr = expr.as_ref()?.trim();
        if expr.is_empty() {
            return None;
        }
        match kind {
            ParameterValueKind::Length => crate::value::eval_length_mm_in_doc(expr, doc),
            ParameterValueKind::Angle => crate::value::eval_angle_rad_in_doc(expr, doc),
        }
    };
    Some(ParameterLimits {
        kind,
        min: eval(&param.minimum),
        max: eval(&param.maximum),
        step: eval(&param.step).filter(|s| *s > 0.0),
    })
}

/// Snap `value` (canonical units) onto the parameter's step grid and clamp to min/max.
///
/// Step grid is `min + n·step` when min is defined, else `n·step` from zero. When both
/// min and max are set the result stays inside `[min, max]`.
pub fn clamp_and_snap_value(limits: &ParameterLimits, value: f32) -> f32 {
    let mut v = value;
    if let Some(min) = limits.min {
        v = v.max(min);
    }
    if let Some(max) = limits.max {
        v = v.min(max);
    }
    if let Some(step) = limits.step {
        let origin = limits.min.unwrap_or(0.0);
        let n = ((v - origin) / step).round();
        v = origin + n * step;
        // Re-clamp after snap so rounding past a bound lands on the bound's nearest step.
        if let Some(min) = limits.min {
            v = v.max(min);
        }
        if let Some(max) = limits.max {
            v = v.min(max);
            // If still past max after snap-up, step back once.
            if v > max + 1e-6 {
                v = origin + ((max - origin) / step).floor() * step;
            }
        }
        if let Some(min) = limits.min {
            if v < min - 1e-6 {
                v = origin + ((min - origin) / step).ceil() * step;
            }
        }
    }
    v
}

/// Format a canonical (mm / rad) value as a parameter expression in the document's
/// default unit (#1176).
pub fn format_canonical_as_expression(
    kind: ParameterValueKind,
    value: f32,
    doc: &Document,
) -> String {
    match kind {
        ParameterValueKind::Length => {
            format_length_display_in(value, doc.default_length_unit)
        }
        ParameterValueKind::Angle => {
            format_angle_display_in(value, doc.default_angle_unit)
        }
    }
}

/// Clamp/snap an override expression against a parameter's limits, returning the
/// (possibly adjusted) expression to store (#1176). Errors when the expression can't
/// evaluate as the parameter's unit kind.
pub fn clamp_and_snap_override_expression(
    unit_doc: &Document,
    param: &Parameter,
    expression: &str,
) -> Result<String, String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("Override value is required".to_string());
    }
    let Some(limits) = parameter_limits(unit_doc, param) else {
        // No evaluable default / bounds — accept the expression as typed if it parses.
        return Ok(expression.to_string());
    };
    let raw = match limits.kind {
        ParameterValueKind::Length => crate::value::eval_length_mm_in_doc(expression, unit_doc),
        ParameterValueKind::Angle => crate::value::eval_angle_rad_in_doc(expression, unit_doc),
    }
    .ok_or_else(|| {
        format!(
            "Override must be a {} expression",
            limits.kind.label()
        )
    })?;
    let snapped = clamp_and_snap_value(&limits, raw);
    // Keep the typed expression when it already matches (within float noise) so a
    // carefully written `width/2` isn't replaced with a bare number.
    if (snapped - raw).abs() < 1e-4 {
        return Ok(expression.to_string());
    }
    Ok(format_canonical_as_expression(limits.kind, snapped, unit_doc))
}

pub fn delete_parameter(doc: &mut Document, index: ParameterKey) -> Result<(), String> {
    if !crate::document_lifecycle::delete_parameter(doc, index) {
        return Err(format!("Parameter {index:?} not found"));
    }
    Ok(())
}

fn apply_parameter_action(state: &mut AppState, action: Action) -> ActionResult {
    let result = state.apply(action);
    match &result {
        ActionResult::Ok => state.parameters_pane.message = None,
        ActionResult::Err(e) => state.parameters_pane.message = Some(e.clone()),
        ActionResult::NeedsDialog => {
            state.parameters_pane.message = Some("Unexpected dialog request".to_string());
        }
    }
    result
}

/// Singleline [`TextEdit`] surrenders focus on Enter, so commit must treat `lost_focus` as active.
pub fn parameter_edit_enter_pressed(
    enter_pressed: bool,
    has_focus: bool,
    lost_focus: bool,
) -> bool {
    enter_pressed && (has_focus || lost_focus)
}

/// Whether a gear-options min/max/step field should commit its draft this frame (#1179).
///
/// Enter commits (via [`parameter_edit_enter_pressed`]). Any focus loss also commits —
/// blur must not discard the draft (that was the #1179 bug).
pub fn parameter_options_field_should_commit(
    enter_pressed: bool,
    has_focus: bool,
    lost_focus: bool,
) -> bool {
    lost_focus || parameter_edit_enter_pressed(enter_pressed, has_focus, lost_focus)
}

/// Draft text → expression for [`Action::SetParameterBound`]: empty clears the bound.
pub fn parameter_options_bound_expression(draft: &str) -> Option<String> {
    let t = draft.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// One row of the selected unit's parameter list (#728).
pub struct UnitParamRow {
    pub name: String,
    /// The value this instance evaluates with: its override when present, else the
    /// unit's own expression.
    pub expression: String,
    pub overridden: bool,
    pub primary: bool,
    /// Resolved min/max/step from the unit's own parameter definition (#1176). The
    /// importer cannot edit these — only override the value within them.
    pub limits: Option<ParameterLimits>,
    /// Current value in canonical units (mm / rad), evaluated in the unit document with
    /// this instance's override applied — drives the import slider (#1176).
    pub current_value: Option<f32>,
}

/// The selected unit instance's parameter rows (#728): the unit's primary parameters
/// first; its secondary ones only when `show_secondary`.
pub fn unit_parameter_rows(
    doc: &Document,
    instance: crate::model::UnitInstanceKey,
    show_secondary: bool,
) -> Vec<UnitParamRow> {
    let Some(inst) = doc.unit_instances.get(instance) else {
        return Vec::new();
    };
    let Some(unit) = doc.units.get(inst.unit) else {
        return Vec::new();
    };
    // Scratch unit doc with this instance's overrides so current values read correctly.
    let mut eval_doc = unit.document.clone();
    for (name, expression) in &inst.parameter_overrides {
        if let Some(p) = eval_doc.parameters.values_mut().find(|p| p.name == *name) {
            p.expression = expression.clone();
        }
    }
    let mut rows: Vec<UnitParamRow> = unit
        .document
        .parameters
        .values()
        .filter(|p| p.primary || show_secondary)
        .map(|p| {
            let over = inst.parameter_overrides.iter().find(|(n, _)| *n == p.name);
            let expression = over
                .map(|(_, e)| e.clone())
                .unwrap_or_else(|| p.expression.clone());
            let limits = parameter_limits(&unit.document, p);
            let current_value = match limits.map(|l| l.kind) {
                Some(ParameterValueKind::Length) => {
                    crate::value::eval_length_mm_in_doc(&expression, &eval_doc)
                }
                Some(ParameterValueKind::Angle) => {
                    crate::value::eval_angle_rad_in_doc(&expression, &eval_doc)
                }
                None => eval_parameter_in_doc(&expression, &eval_doc).map(|v| match v {
                    EvaluatedParameter::LengthMm(x) | EvaluatedParameter::AngleRad(x) => x,
                }),
            };
            UnitParamRow {
                name: p.name.clone(),
                expression,
                overridden: over.is_some(),
                primary: p.primary,
                limits,
                current_value,
            }
        })
        .collect();
    rows.sort_by_key(|r| !r.primary);
    rows
}

/// The selected unit's parameters at the top of the Parameters pane (#728): edits write
/// that one instance's overrides — never the source file, never other instances.
/// Min/max/step come from the unit and are not editable here; with both min and max the
/// row offers a slider (#1176).
fn show_unit_parameters_section(ui: &mut egui::Ui, app: &mut AppState) {
    use egui::TextEdit;
    let Some(crate::hierarchy::SceneElement::UnitInstance(instance)) =
        app.scene_selection.single()
    else {
        return;
    };
    if app.doc.unit_instances.get(instance).is_none() {
        return;
    }
    let heading = crate::names::scene_element_label(
        &app.doc,
        &crate::hierarchy::SceneElement::UnitInstance(instance),
    );
    let head = ui.label(RichText::new(heading).strong());
    crate::context::note_help_rect(ui, "Unit parameters", head.rect);

    let show_secondary = app.parameters_pane.show_unit_secondary;
    let rows = unit_parameter_rows(&app.doc, instance, show_secondary);
    // The unit document supplies display units for slider formatting.
    let unit_doc_defaults = app
        .doc
        .unit_instances
        .get(instance)
        .and_then(|inst| app.doc.units.get(inst.unit))
        .map(|u| (u.document.default_length_unit, u.document.default_angle_unit));
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let mut set_override: Option<(String, Option<String>)> = None;
    egui::Grid::new("unit_parameters_table")
        .num_columns(3)
        .spacing([8.0, 4.0])
        .min_col_width(72.0)
        .show(ui, |ui| {
            for row in &rows {
                let name_cell = ui.label(&row.name);
                crate::context::note_help_rect(ui, "Unit parameter", name_cell.rect);

                // Slider when both min and max resolve (#1176).
                let slider_range = row.limits.and_then(|lim| {
                    let min = lim.min?;
                    let max = lim.max?;
                    (max > min).then_some((lim, min, max))
                });

                ui.vertical(|ui| {
                    if let Some((lim, min, max)) = slider_range {
                        let current = row
                            .current_value
                            .unwrap_or(min)
                            .clamp(min, max);
                        let mut slider_v = current;
                        ui.spacing_mut().slider_width = 100.0;
                        let slider = ui.add(
                            egui::Slider::new(&mut slider_v, min..=max).show_value(false),
                        );
                        if slider.changed() {
                            let snapped = clamp_and_snap_value(&lim, slider_v);
                            let (len_u, ang_u) = unit_doc_defaults.unwrap_or((
                                app.doc.default_length_unit,
                                app.doc.default_angle_unit,
                            ));
                            let expr = match lim.kind {
                                ParameterValueKind::Length => {
                                    format_length_display_in(snapped, len_u)
                                }
                                ParameterValueKind::Angle => {
                                    format_angle_display_in(snapped, ang_u)
                                }
                            };
                            set_override = Some((row.name.clone(), Some(expr)));
                        }
                    }

                    let editing =
                        app.parameters_pane.unit_editing.as_deref() == Some(row.name.as_str());
                    if editing {
                        let resp = ui.add(
                            TextEdit::singleline(&mut app.parameters_pane.unit_draft)
                                .desired_width(f32::INFINITY),
                        );
                        if app.parameters_pane.unit_editing_focus {
                            resp.request_focus();
                            app.parameters_pane.unit_editing_focus = false;
                        }
                        if parameter_edit_enter_pressed(enter, resp.has_focus(), resp.lost_focus())
                        {
                            let draft = app.parameters_pane.unit_draft.trim().to_string();
                            if !draft.is_empty() {
                                set_override = Some((row.name.clone(), Some(draft)));
                            }
                            app.parameters_pane.unit_editing = None;
                        } else if resp.lost_focus() {
                            app.parameters_pane.unit_editing = None;
                        }
                    } else {
                        // An overridden value reads gold — this instance's own number, not
                        // the unit's.
                        let text = if row.overridden {
                            RichText::new(format_parameter_value_display(
                                &app.doc,
                                &row.expression,
                            ))
                            .color(egui::Color32::from_rgb(255, 210, 90))
                        } else {
                            RichText::new(format_parameter_value_display(
                                &app.doc,
                                &row.expression,
                            ))
                        };
                        let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                        if resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            app.parameters_pane.unit_editing = Some(row.name.clone());
                            app.parameters_pane.unit_draft = row.expression.clone();
                            app.parameters_pane.unit_editing_focus = true;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if row.overridden {
                        let revert = icon_button(
                            ui,
                            crate::icons::IconId::Close,
                            "Back to the unit's own value",
                        );
                        crate::context::note_help_rect(ui, "Override", revert.rect);
                        if revert.clicked() {
                            set_override = Some((row.name.clone(), None));
                        }
                    }
                });
                ui.end_row();
            }
        });
    let toggle = ui.horizontal(|ui| {
        if crate::icons::icon_button_hover_gold(
            ui,
            crate::icons::icon_for_visibility(show_secondary),
            if show_secondary {
                "Hide the unit's secondary parameters"
            } else {
                "Show the unit's secondary parameters"
            },
        )
        .clicked()
        {
            app.parameters_pane.show_unit_secondary = !show_secondary;
        }
        ui.label(RichText::new("Internals").weak().size(11.0));
    });
    crate::context::note_help_rect(ui, "Internals", toggle.response.rect);
    ui.separator();
    ui.add_space(2.0);

    if let Some((name, expression)) = set_override {
        apply_parameter_action(
            app,
            Action::SetUnitParameterOverride { instance, name, expression },
        );
    }
}

/// Min / max / step fields plus the Primary checkbox for one parameter's gear-options
/// panel (#1176). Indented under the parameter name with a tree gutter (#1178). Commits
/// queue into `set_primary` / `set_bound` so the caller can apply them after the grid
/// borrow ends. Bound fields commit on Enter **and** on blur (#1179).
fn show_parameter_options_fields(
    ui: &mut egui::Ui,
    app: &mut AppState,
    index: ParameterKey,
    primary: bool,
    minimum: &Option<String>,
    maximum: &Option<String>,
    step: &Option<String>,
    enter: bool,
    set_primary: &mut Option<(ParameterKey, bool)>,
    set_bound: &mut Option<(ParameterKey, ParameterBound, Option<String>)>,
) {
    use egui::{pos2, Stroke, TextEdit};

    // Tree rows under the parameter name (#1178): Primary, Min, Max, Step.
    // The gutter is a vertical line that turns a corner on the last row.
    const GUTTER: f32 = 12.0;
    let bounds: [(ParameterBound, &Option<String>, &str); 3] = [
        (ParameterBound::Minimum, minimum, "Min"),
        (ParameterBound::Maximum, maximum, "Max"),
        (ParameterBound::Step, step, "Step"),
    ];
    let n_rows = 1 + bounds.len(); // Primary + bounds
    let stroke = Stroke::new(
        1.0,
        ui.visuals().widgets.noninteractive.fg_stroke.color.gamma_multiply(0.45),
    );

    let row_gap = ui.spacing().item_spacing.y;
    for row_i in 0..n_rows {
        let is_last = row_i + 1 == n_rows;
        ui.horizontal(|ui| {
            // Tree gutter: continuous vertical stem under the name, elbow on each row
            // (and only the last row ends the stem so it "turns the corner").
            let row_h = ui.spacing().interact_size.y.max(14.0);
            let (gutter_rect, _) =
                ui.allocate_exact_size(egui::vec2(GUTTER, row_h), egui::Sense::hover());
            let x = gutter_rect.left() + 4.0;
            let mid_y = gutter_rect.center().y;
            let painter = ui.painter();
            // Reach across the inter-row gap so the stem reads continuous.
            let top_y = if row_i == 0 {
                // Peek up toward the parameter name above this block.
                gutter_rect.top() - row_gap
            } else {
                gutter_rect.top() - row_gap * 0.5
            };
            let bottom_y = if is_last {
                mid_y
            } else {
                gutter_rect.bottom() + row_gap * 0.5
            };
            painter.line_segment([pos2(x, top_y), pos2(x, bottom_y)], stroke);
            // Horizontal turn toward the label.
            painter.line_segment([pos2(x, mid_y), pos2(gutter_rect.right() - 1.0, mid_y)], stroke);

            if row_i == 0 {
                let mut primary_flag = primary;
                let resp = ui.checkbox(&mut primary_flag, "Primary");
                crate::context::note_help_rect(ui, "Primary", resp.rect);
                if resp.changed() {
                    *set_primary = Some((index, primary_flag));
                }
                resp.on_hover_text(
                    "Offered first when this file is imported. Unchecked = internal (secondary).",
                );
            } else {
                let (which, current, label) = bounds[row_i - 1];
                ui.label(RichText::new(label).size(11.0));
                let editing = app.parameters_pane.options_editing == Some((index, which));
                if editing {
                    let resp = ui.add(
                        TextEdit::singleline(&mut app.parameters_pane.options_draft)
                            .desired_width(96.0)
                            .hint_text("expression"),
                    );
                    if app.parameters_pane.options_editing_focus {
                        resp.request_focus();
                        app.parameters_pane.options_editing_focus = false;
                    }
                    if parameter_options_field_should_commit(
                        enter,
                        resp.has_focus(),
                        resp.lost_focus(),
                    ) {
                        *set_bound = Some((
                            index,
                            which,
                            parameter_options_bound_expression(&app.parameters_pane.options_draft),
                        ));
                        app.parameters_pane.options_editing = None;
                        app.parameters_pane.options_draft.clear();
                        if enter {
                            ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, Key::Enter);
                            });
                        }
                    }
                } else {
                    let display = current.as_deref().filter(|s| !s.is_empty()).unwrap_or("—");
                    let resp = ui.add(
                        egui::Label::new(RichText::new(display).size(11.0))
                            .sense(egui::Sense::click()),
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(format!("{label} (optional expression)"))
                        .clicked()
                    {
                        app.parameters_pane.options_editing = Some((index, which));
                        app.parameters_pane.options_draft =
                            current.clone().unwrap_or_default();
                        app.parameters_pane.options_editing_focus = true;
                    }
                }
            }
        });
    }
}

pub fn show_pane(ui: &mut egui::Ui, app: &mut AppState) {
    use crate::expression_input::ParameterExpressionContext;
    use egui::{Grid, ScrollArea, TextEdit};

    ui.heading(PANE_TITLE);
    ui.add_space(4.0);

    // A row's ✕ delete button queues here and is applied after the grid, so the loop keeps its
    // borrow of `app` (#270).
    let mut delete_index: Option<ParameterKey> = None;
    // Row-hover tracking (#620): re-derived every frame; queued here (like `delete_index`)
    // so the row loop keeps its borrow of `app`.
    let mut hovered_name: Option<String> = None;
    // Gear-options commits (#1176), applied after the grid like the delete.
    let mut set_primary: Option<(ParameterKey, bool)> = None;
    let mut set_bound: Option<(ParameterKey, ParameterBound, Option<String>)> = None;
    let mut toggle_options: Option<ParameterKey> = None;

    ScrollArea::vertical().show(ui, |ui| {
        // Selected unit instance (#728): its parameters lead the pane, unmistakably its
        // own section; the document's parameters follow below.
        show_unit_parameters_section(ui, app);
        Grid::new("parameters_table")
            .num_columns(3)
            .spacing([8.0, 4.0])
            .min_col_width(72.0)
            .show(ui, |ui| {
                ui.label("Name");
                ui.label("Value");
                ui.label("");
                ui.end_row();

                let enter = ui.input(|i| i.key_pressed(Key::Enter));

                let rows: Vec<ParameterKey> = app.doc.parameters.keys().collect();
                for index in rows {
                    let (
                        param_name,
                        param_expression,
                        param_display,
                        param_status,
                        value_readonly,
                        source_description,
                        param_primary,
                        param_minimum,
                        param_maximum,
                        param_step,
                    ) = {
                        let param = &app.doc.parameters[index];
                        (
                            param.name.clone(),
                            param.expression.clone(),
                            format_parameter_value_display(&app.doc, &param.expression),
                            app.document_health.parameter_status(index),
                            parameter_value_is_readonly(param),
                            parameter_source_description(&app.doc, param),
                            param.primary,
                            param.minimum.clone(),
                            param.maximum.clone(),
                            param.step.clone(),
                        )
                    };
                    let param_frozen = param_status.is_frozen();
                    if param_frozen {
                        match app.parameters_pane.editing {
                            Some(ParameterEditCell::Name(i) | ParameterEditCell::Value(i))
                                if i == index =>
                            {
                                app.parameters_pane.cancel_edit();
                            }
                            _ => {}
                        }
                    } else if value_readonly {
                        if matches!(
                            app.parameters_pane.editing,
                            Some(ParameterEditCell::Value(i)) if i == index
                        ) {
                            app.parameters_pane.cancel_edit();
                        }
                    }
                    let editing_name = matches!(
                        app.parameters_pane.editing,
                        Some(ParameterEditCell::Name(i)) if i == index
                    );
                    let editing_value = matches!(
                        app.parameters_pane.editing,
                        Some(ParameterEditCell::Value(i)) if i == index
                    );

                    let name_cell = ui.horizontal(|ui| {
                        // A derived (read-only) parameter shows a lock left of its name
                        // (#631); the measurement it tracks is the lock's hover text.
                        if value_readonly {
                            let lock = ui.add(egui::Image::new(crate::icons::sized_texture(
                                ui.ctx(),
                                crate::icons::IconId::Lock,
                            )));
                            if let Some(reason) = &source_description {
                                lock.on_hover_text(reason.clone());
                            }
                        }
                        if editing_name {
                            let response = ui.add(
                                TextEdit::singleline(&mut app.parameters_pane.draft)
                                    .id(param_name_id(index))
                                    .desired_width(f32::INFINITY),
                            );
                            if response.changed() {
                                app.parameters_pane
                                    .draft
                                    .retain(|c| !c.is_whitespace());
                            }
                            if app.parameters_pane.editing_focus {
                                response.request_focus();
                                app.parameters_pane.editing_focus = false;
                            }
                            if parameter_edit_enter_pressed(
                                enter,
                                response.has_focus(),
                                response.lost_focus(),
                            ) {
                                let draft = app.parameters_pane.draft.clone();
                                if apply_parameter_action(
                                    app,
                                    Action::CommitParameterName {
                                        index,
                                        name: draft,
                                    },
                                ) == ActionResult::Ok
                                {
                                    app.parameters_pane.cancel_edit();
                                }
                                ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, Key::Enter);
                                });
                            }
                        } else if ui
                            .selectable_label(
                                false,
                                styled_parameter_label(&param_name, param_status),
                            )
                            .clicked()
                            && !param_frozen
                        {
                            app.parameters_pane
                                .begin_edit(ParameterEditCell::Name(index), &param_name);
                        }
                    });

                    let value_cell = ui.horizontal(|ui| {
                        if editing_value {
                            let param_ctx = ParameterExpressionContext {
                                param_name: param_name.clone(),
                                existing_index: Some(index),
                            };
                            let exclude = [param_name.as_str()];
                            let mut draft = app.parameters_pane.draft.clone();
                            let response = crate::expression_input::ValueInput::from_id(
                                param_value_id(index),
                                crate::expression_input::ValueKind::Length,
                            )
                            .no_definitions()
                            .parameter_context(&param_ctx)
                            .exclude_names(&exclude)
                            .show(ui, &mut draft, &app.doc);
                            app.parameters_pane.draft = draft;
                            if app.parameters_pane.editing_focus {
                                response.request_focus();
                                app.parameters_pane.editing_focus = false;
                            }
                            if parameter_edit_enter_pressed(
                                enter,
                                response.has_focus(),
                                response.lost_focus(),
                            ) {
                                let draft = app.parameters_pane.draft.clone();
                                if apply_parameter_action(
                                    app,
                                    Action::CommitParameterExpression {
                                        index,
                                        expression: draft,
                                    },
                                ) == ActionResult::Ok
                                {
                                    app.parameters_pane.cancel_edit();
                                }
                                ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, Key::Enter);
                                });
                            }
                        } else if ui
                            .selectable_label(
                                false,
                                styled_parameter_label(&param_display, param_status),
                            )
                            .clicked()
                            && !param_frozen
                            && !value_readonly
                        {
                            app.parameters_pane.begin_edit(
                                ParameterEditCell::Value(index),
                                &param_expression,
                            );
                        }
                    });
                    let options_open = app.parameters_pane.options_open.contains(&index);
                    let extras_cell = ui.horizontal(|ui| {
                        // Gear cog toggles this parameter's options panel (#1176). Multiple
                        // can be open at once; each gear only affects its own row.
                        let gear = crate::icons::icon_button_hover_gold(
                            ui,
                            IconId::Gear,
                            if options_open {
                                "Hide parameter options"
                            } else {
                                "Parameter options (min, max, step, primary)"
                            },
                        );
                        crate::context::note_help_rect(ui, "Parameter options", gear.rect);
                        if gear.clicked() {
                            toggle_options = Some(index);
                        }
                        // Delete button (#270): a muted-red ✕ that removes the parameter.
                        let remove = ui.add(
                            egui::Button::new(
                                egui::Image::new(crate::icons::sized_texture(
                                    ui.ctx(),
                                    crate::icons::IconId::Close,
                                ))
                                .tint(egui::Color32::from_rgb(0xC9, 0x6F, 0x66)),
                            )
                            .frame(false),
                        );
                        if remove.on_hover_text("Delete parameter").clicked() {
                            delete_index = Some(index);
                        }
                        if param_frozen {
                            let reason = app
                                .document_health
                                .parameter_reason(index)
                                .unwrap_or("");
                            ui.label(
                                RichText::new(reason)
                                    .color(if param_status == HealthStatus::Invalid {
                                        INVALID_TEXT
                                    } else {
                                        UNSTABLE_TEXT
                                    })
                                    .size(11.0),
                            );
                        }
                    });
                    // Pointer over any of the row's cells marks the parameter hovered (#620).
                    if name_cell.response.contains_pointer()
                        || value_cell.response.contains_pointer()
                        || extras_cell.response.contains_pointer()
                    {
                        hovered_name = Some(param_name.clone());
                    }
                    ui.end_row();

                    // Expanded options under the name with a tree gutter (#1176/#1178) —
                    // not in the value column.
                    if options_open {
                        let options_cell = ui.vertical(|ui| {
                            ui.add_space(2.0);
                            show_parameter_options_fields(
                                ui,
                                app,
                                index,
                                param_primary,
                                &param_minimum,
                                &param_maximum,
                                &param_step,
                                enter,
                                &mut set_primary,
                                &mut set_bound,
                            );
                            ui.add_space(4.0);
                        });
                        ui.label("");
                        ui.label("");
                        if options_cell.response.contains_pointer() {
                            hovered_name = Some(param_name.clone());
                        }
                        ui.end_row();
                    }
                }

                let name_response = ui.add(
                    TextEdit::singleline(&mut app.parameters_pane.new_name)
                        .id(Id::new(NEW_NAME_ID))
                        .hint_text("name")
                        .desired_width(f32::INFINITY),
                );
                app.parameters_pane.new_name_focused = name_response.has_focus();
                app.tutorial_anchor_rects.insert(
                    crate::tutorial::UiAnchor::ParametersName,
                    name_response.rect,
                );
                if name_response.changed() {
                    app.parameters_pane
                        .new_name
                        .retain(|c| !c.is_whitespace());
                }
                if app.parameters_pane.focus_new_name {
                    name_response.request_focus();
                    app.parameters_pane.focus_new_name = false;
                }
                let new_param_context = (!app.parameters_pane.new_name.trim().is_empty()).then(|| {
                    ParameterExpressionContext {
                        param_name: app.parameters_pane.new_name.trim().to_string(),
                        existing_index: None,
                    }
                });
                let new_name = app.parameters_pane.new_name.trim().to_string();
                let exclude_new: Vec<&str> = if new_name.is_empty() {
                    Vec::new()
                } else {
                    vec![new_name.as_str()]
                };
                let mut new_value = app.parameters_pane.new_value.clone();
                let mut input = crate::expression_input::ValueInput::from_id(
                    Id::new(NEW_VALUE_ID),
                    crate::expression_input::ValueKind::Length,
                )
                .hint("value")
                .no_definitions()
                .exclude_names(&exclude_new);
                if let Some(ctx) = new_param_context.as_ref() {
                    input = input.parameter_context(ctx);
                }
                let value_response = input.show(ui, &mut new_value, &app.doc);
                app.parameters_pane.new_value = new_value;
                app.parameters_pane.new_value_focused = value_response.has_focus();
                app.tutorial_anchor_rects.insert(
                    crate::tutorial::UiAnchor::ParametersValue,
                    value_response.rect,
                );
                if app.parameters_pane.focus_new_value {
                    value_response.request_focus();
                    app.parameters_pane.focus_new_value = false;
                }

                let add_response = icon_button(ui, IconId::Plus, "Add parameter");
                app.tutorial_anchor_rects.insert(
                    crate::tutorial::UiAnchor::ParametersAdd,
                    add_response.rect,
                );
                let add_clicked = add_response.clicked();

                if name_response.gained_focus() || value_response.gained_focus() {
                    app.parameters_pane.cancel_edit();
                }

                let mut commit_new = add_clicked;
                if parameter_edit_enter_pressed(
                    enter,
                    name_response.has_focus(),
                    name_response.lost_focus(),
                ) {
                    if !app.parameters_pane.new_name.trim().is_empty()
                        && app.parameters_pane.new_value.trim().is_empty()
                    {
                        app.parameters_pane.focus_new_value = true;
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
                    } else if new_parameter_row_ready(&app.parameters_pane) {
                        commit_new = true;
                    }
                } else if parameter_edit_enter_pressed(
                    enter,
                    value_response.has_focus(),
                    value_response.lost_focus(),
                ) && new_parameter_row_ready(&app.parameters_pane)
                {
                    commit_new = true;
                }

                let lost_focus_commit = (name_response.lost_focus() || value_response.lost_focus())
                    && new_parameter_row_ready(&app.parameters_pane)
                    && !name_response.has_focus()
                    && !value_response.has_focus();

                if commit_new || lost_focus_commit {
                    let _ = commit_new_parameter(app);
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
                }

                ui.end_row();
            });
    });

    if let Some(index) = delete_index {
        apply_parameter_action(app, Action::DeleteParameter { index });
    }
    if let Some(index) = toggle_options {
        if !app.parameters_pane.options_open.insert(index) {
            app.parameters_pane.options_open.remove(&index);
            // Drop any in-flight options edit for this parameter.
            if app
                .parameters_pane
                .options_editing
                .is_some_and(|(k, _)| k == index)
            {
                app.parameters_pane.options_editing = None;
                app.parameters_pane.options_draft.clear();
            }
        }
    }
    if let Some((index, primary)) = set_primary {
        apply_parameter_action(app, Action::SetParameterPrimary { index, primary });
    }
    if let Some((index, which, expression)) = set_bound {
        apply_parameter_action(app, Action::SetParameterBound { index, which, expression });
    }
    app.parameters_pane.hovered_name = hovered_name;

    // Deriving a parameter from the selection lives in the Dimension tool's context-pane
    // block (#618/#629) — the pane's old "Derive from selection" button is gone.

    if let Some(message) = &app.parameters_pane.message {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(message)
                .color(egui::Color32::from_rgb(255, 140, 100))
                .size(12.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::body_key_for_slot as bkey;
    use super::*;
    use crate::actions::AppState;
    use crate::constraints::add_distance_constraint;
    use crate::document_lifecycle::delete_element;
    use crate::hierarchy::SceneElement;
    use crate::model::{DistanceTarget, Document, FaceId, Line, ShapeKind};

    fn doc_with_param_a() -> Document {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "5mm".to_string()).unwrap();
        doc
    }

    /// #727: a new parameter is primary when its expression is a plain self-contained
    /// value, secondary when it references anything; derived parameters are secondary.
    #[test]
    fn new_parameters_default_primary_from_their_expression() {
        let mut doc = Document::default();
        let plain = add_parameter(&mut doc, "width".to_string(), "10".to_string()).unwrap();
        let with_unit = add_parameter(&mut doc, "gap".to_string(), "2.5mm".to_string()).unwrap();
        let angle = add_parameter(&mut doc, "tilt".to_string(), "45deg".to_string()).unwrap();
        let derived = add_parameter(&mut doc, "half".to_string(), "width / 2".to_string()).unwrap();
        assert!(doc.parameters[plain].primary, "a bare number is a knob");
        assert!(doc.parameters[with_unit].primary, "a number with a unit is a knob");
        assert!(doc.parameters[angle].primary, "an angle literal is a knob");
        assert!(
            !doc.parameters[derived].primary,
            "an expression referencing another parameter is internal"
        );
    }

    /// #1176: min/max/step round-trip through save/load; empty clears; step must be positive;
    /// bounds share the default value's unit kind.
    #[test]
    fn parameter_bounds_round_trip_and_validate() {
        let mut doc = Document::default();
        let width = add_parameter(&mut doc, "width".to_string(), "10mm".to_string()).unwrap();
        set_parameter_bound(&mut doc, width, ParameterBound::Minimum, Some("5mm".into())).unwrap();
        set_parameter_bound(&mut doc, width, ParameterBound::Maximum, Some("50mm".into())).unwrap();
        set_parameter_bound(&mut doc, width, ParameterBound::Step, Some("2.5mm".into())).unwrap();
        assert_eq!(doc.parameters[width].minimum.as_deref(), Some("5mm"));
        assert_eq!(doc.parameters[width].maximum.as_deref(), Some("50mm"));
        assert_eq!(doc.parameters[width].step.as_deref(), Some("2.5mm"));

        let limits = parameter_limits(&doc, &doc.parameters[width]).unwrap();
        assert_eq!(limits.kind, ParameterValueKind::Length);
        assert!((limits.min.unwrap() - 5.0).abs() < 1e-4);
        assert!((limits.max.unwrap() - 50.0).abs() < 1e-4);
        assert!((limits.step.unwrap() - 2.5).abs() < 1e-4);

        // Angle bound on a length parameter is refused.
        let err = set_parameter_bound(&mut doc, width, ParameterBound::Minimum, Some("45deg".into()))
            .unwrap_err();
        assert!(err.contains("length") || err.contains("Invalid"), "{err}");
        // Non-positive step refused.
        assert!(
            set_parameter_bound(&mut doc, width, ParameterBound::Step, Some("0".into())).is_err()
        );
        // Clear.
        set_parameter_bound(&mut doc, width, ParameterBound::Step, None).unwrap();
        assert!(doc.parameters[width].step.is_none());

        let path = std::env::temp_dir().join("bearcad_param_bounds_roundtrip.bearcad");
        let _ = std::fs::remove_file(&path);
        crate::storage::save(&path.to_string_lossy(), &doc).unwrap();
        let loaded = crate::storage::open(&path.to_string_lossy()).unwrap();
        let p = loaded.parameters.values().next().unwrap();
        assert_eq!(p.minimum.as_deref(), Some("5mm"));
        assert_eq!(p.maximum.as_deref(), Some("50mm"));
        assert!(p.step.is_none());
        let _ = std::fs::remove_file(&path);

        // Legacy JSON without the fields loads as None.
        let mut value = serde_json::to_value(&doc).unwrap();
        let entry = value["parameters"]["entries"][0][2].as_object_mut().unwrap();
        entry.remove("minimum");
        entry.remove("maximum");
        entry.remove("step");
        let legacy = crate::storage::from_json_bytes(&serde_json::to_vec(&value).unwrap()).unwrap();
        let lp = legacy.parameters.values().next().unwrap();
        assert!(lp.minimum.is_none() && lp.maximum.is_none() && lp.step.is_none());
    }

    /// #1176: override values clamp to min/max and snap to the nearest step.
    #[test]
    fn override_clamp_and_snap_to_step() {
        let mut doc = Document::default();
        let width = add_parameter(&mut doc, "width".to_string(), "10mm".to_string()).unwrap();
        set_parameter_bound(&mut doc, width, ParameterBound::Minimum, Some("0mm".into())).unwrap();
        set_parameter_bound(&mut doc, width, ParameterBound::Maximum, Some("20mm".into())).unwrap();
        set_parameter_bound(&mut doc, width, ParameterBound::Step, Some("5mm".into())).unwrap();
        let param = &doc.parameters[width];

        // Below min → min.
        let out = clamp_and_snap_override_expression(&doc, param, "-3mm").unwrap();
        let v = crate::value::eval_length_mm_in_doc(&out, &doc).unwrap();
        assert!((v - 0.0).abs() < 1e-3, "got {out} ({v})");
        // Above max → max (or nearest step under max).
        let out = clamp_and_snap_override_expression(&doc, param, "99mm").unwrap();
        let v = crate::value::eval_length_mm_in_doc(&out, &doc).unwrap();
        assert!((v - 20.0).abs() < 1e-3, "got {out} ({v})");
        // Off-step snaps to closest: 12 → 10 or 15; closer is 10.
        let out = clamp_and_snap_override_expression(&doc, param, "12mm").unwrap();
        let v = crate::value::eval_length_mm_in_doc(&out, &doc).unwrap();
        assert!((v - 10.0).abs() < 1e-3, "got {out} ({v})");
        // Exactly on a step is kept as typed when numerically equal.
        let out = clamp_and_snap_override_expression(&doc, param, "15mm").unwrap();
        assert_eq!(out, "15mm");
    }

    /// #1176: numeric clamp/snap helper — mid-step, bounds, no-min origin.
    #[test]
    fn clamp_and_snap_value_math() {
        let with_min = ParameterLimits {
            kind: ParameterValueKind::Length,
            min: Some(2.0),
            max: Some(10.0),
            step: Some(2.0),
        };
        assert!((clamp_and_snap_value(&with_min, 5.0) - 6.0).abs() < 1e-4); // 2+2n: 4 or 6, closer 6? 5-4=1, 6-5=1 — round half...
        // 5.0 - 2.0 = 3.0 / 2.0 = 1.5 → rounds to 2 → 2+4=6. Yes.
        assert!((clamp_and_snap_value(&with_min, 0.0) - 2.0).abs() < 1e-4);
        assert!((clamp_and_snap_value(&with_min, 100.0) - 10.0).abs() < 1e-4);

        let no_min = ParameterLimits {
            kind: ParameterValueKind::Length,
            min: None,
            max: Some(10.0),
            step: Some(3.0),
        };
        assert!((clamp_and_snap_value(&no_min, 4.0) - 3.0).abs() < 1e-4);
        assert!((clamp_and_snap_value(&no_min, 11.0) - 9.0).abs() < 1e-4); // max 10, nearest step ≤10 is 9
    }

    /// #727: the flag round-trips through save/load; a document saved without the field
    /// (an existing file) loads secondary; the toggle action flips it.
    #[test]
    fn primary_flag_round_trips_and_defaults_secondary() {
        let mut state = AppState::default();
        state.apply(crate::actions::Action::AddParameter {
            name: "width".to_string(),
            expression: "10".to_string(),
        });
        let width = state.doc.parameters.keys().next().expect("the parameter");
        assert!(state.doc.parameters[width].primary);
        let r = state.apply(crate::actions::Action::SetParameterPrimary {
            index: width,
            primary: false,
        });
        assert_eq!(r, crate::actions::ActionResult::Ok);
        assert!(!state.doc.parameters[width].primary, "the toggle flips it");
        state.apply(crate::actions::Action::SetParameterPrimary { index: width, primary: true });

        let path = std::env::temp_dir().join("bearcad_primary_roundtrip.bearcad");
        let _ = std::fs::remove_file(&path);
        crate::storage::save(&path.to_string_lossy(), &state.doc).unwrap();
        let loaded = crate::storage::open(&path.to_string_lossy()).unwrap();
        assert!(loaded.parameters.values().next().unwrap().primary, "the flag round-trips");
        let _ = std::fs::remove_file(&path);

        // An existing document whose JSON has no `primary` field loads secondary.
        let mut value = serde_json::to_value(&state.doc).unwrap();
        // An arena serializes as `{ entries: [[slot, generation, value], ...] }` (#1055).
        value["parameters"]["entries"][0][2]
            .as_object_mut()
            .unwrap()
            .remove("primary");
        let bytes = serde_json::to_vec(&value).unwrap();
        let legacy = crate::storage::from_json_bytes(&bytes).unwrap();
        assert!(
            !legacy.parameters.values().next().unwrap().primary,
            "existing parameters load secondary — the front door is chosen deliberately"
        );
    }

    /// #647: a body's feature edge measures its length and two mesh corners measure the
    /// distance between them — the Dimension tool in 3D can now derive from body geometry,
    /// not just sketch geometry. Values come from the body's *live* mesh, so the source goes
    /// unavailable once the geometry no longer has that edge/corner.
    fn doc_with_one_triangle_body() -> Document {
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[
                glam::Vec3::new(0.0, 0.0, 0.0),
                glam::Vec3::new(30.0, 0.0, 0.0),
                glam::Vec3::new(0.0, 40.0, 0.0),
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
    fn body_edge_and_corner_pair_are_derivable_measurements() {
        use crate::hierarchy::quantize_body_point as q;
        let mut doc = doc_with_one_triangle_body();
        let (o, x, y) = (
            glam::Vec3::ZERO,
            glam::Vec3::new(30.0, 0.0, 0.0),
            glam::Vec3::new(0.0, 40.0, 0.0),
        );

        // One selected body edge → its length.
        let mut selection = crate::selection::SceneSelection::default();
        selection.insert(SceneElement::BodyEdge { body: bkey(0), a: q(o), b: q(x) });
        let source = derived_source_from_selection(&doc, &selection)
            .expect("a body edge is measurable");
        assert_eq!(
            source,
            ParameterSource::BodyEdgeLength { body: bkey(0), a: q(o), b: q(x) }
        );
        let (value, is_angle) = derived_source_value(&doc, &source).unwrap();
        assert!((value - 30.0).abs() < 1e-3, "edge length, got {value}");
        assert!(!is_angle);

        // Two selected corners → the distance between them (3-4-5 triangle).
        let mut selection = crate::selection::SceneSelection::default();
        selection.insert(SceneElement::BodyVertex { body: bkey(0), p: q(x) });
        selection.insert(SceneElement::BodyVertex { body: bkey(0), p: q(y) });
        let source = derived_source_from_selection(&doc, &selection)
            .expect("two body corners are measurable");
        let (value, _) = derived_source_value(&doc, &source).unwrap();
        assert!((value - 50.0).abs() < 1e-3, "corner distance, got {value}");

        // The same corner twice measures nothing.
        let mut selection = crate::selection::SceneSelection::default();
        selection.insert(SceneElement::BodyVertex { body: bkey(0), p: q(x) });
        selection.insert(SceneElement::BodyVertex { body: bkey(0), p: q(x) });
        assert!(derived_source_from_selection(&doc, &selection).is_none());

        // Geometry that no longer exists reads as unavailable, like a deleted line's.
        doc.bodies.remove(bkey(0));
        assert!(derived_source_value(
            &doc,
            &ParameterSource::BodyEdgeLength { body: bkey(0), a: q(o), b: q(x) }
        )
        .is_none());
        assert!(derived_source_value(
            &doc,
            &ParameterSource::BodyVertexDistance { body_a: bkey(0), a: q(x), body_b: bkey(0), b: q(y) }
        )
        .is_none());
    }

    #[test]
    fn add_multiple_parameters_in_sequence() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "5mm".to_string()).unwrap();
        add_parameter(&mut doc, "B".to_string(), "A + 5in".to_string()).unwrap();
        add_parameter(&mut doc, "width".to_string(), "2 * B".to_string()).unwrap();
        assert_eq!(doc.parameters.len(), 3);
        assert_eq!(doc.parameters.values().nth(2).unwrap().expression, "2 * B");
    }

    /// #995: deleting a parameter frees its name — it is removed outright, so nothing is left
    /// to raise "already exists" about a parameter that isn't there.
    #[test]
    fn a_deleted_parameters_name_can_be_used_again() {
        let mut doc = Document::default();
        let i = add_parameter(&mut doc, "slotwidth".into(), "12".into()).expect("created");
        assert!(
            add_parameter(&mut doc, "slotwidth".into(), "5".into()).is_err(),
            "a live parameter still owns its name"
        );

        delete_parameter(&mut doc, i).expect("deleted");
        let j = add_parameter(&mut doc, "slotwidth".into(), "5".into())
            .expect("the name is free once the parameter is gone");
        assert_ne!(i, j, "the reused slot carries a new generation, so the keys differ");
        assert!(doc.parameters.get(i).is_none(), "the old parameter is gone");
        assert!(doc.parameters.contains(j));

        // The name now resolves to the new one.
        assert_eq!(parameter_index_by_name(&doc, "slotwidth"), Some(j));
        assert_eq!(
            crate::value::eval_length_mm_in_doc("slotwidth", &doc),
            Some(5.0),
            "expressions read the live parameter"
        );
    }

    #[test]
    fn add_parameter_stores_name_and_expression() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "width".to_string(), "2in".to_string()).unwrap();
        assert_eq!(doc.parameters.len(), 1);
        assert_eq!(doc.parameters.values().next().unwrap().name, "width");
        assert_eq!(doc.parameters.values().next().unwrap().expression, "2in");
        assert!(doc.shape_order.contains(&ShapeKind::Parameter));
    }

    #[test]
    fn parameter_rename_updates_dependent_expressions() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "B".to_string(), "A + 5in".to_string()).unwrap();
        let a = doc.parameters.keys().next().unwrap();
        set_parameter_name(&mut doc, a, "Len".to_string()).unwrap();
        assert_eq!(doc.parameters.values().nth(1).unwrap().expression, "Len + 5in");
    }

    #[test]
    fn rejects_duplicate_parameter_names() {
        let mut doc = doc_with_param_a();
        assert!(add_parameter(&mut doc, "A".to_string(), "1mm".to_string()).is_err());
    }

    #[test]
    fn rejects_invalid_parameter_name() {
        let mut doc = Document::default();
        assert!(add_parameter(&mut doc, "1bad".to_string(), "5mm".to_string()).is_err());
    }

    #[test]
    fn parse_inline_parameter_definition_accepts_name_value() {
        assert_eq!(
            parse_inline_parameter_definition("width=5"),
            Some(("width".to_string(), "5".to_string()))
        );
        assert_eq!(
            parse_inline_parameter_definition(" corner = 45deg "),
            Some(("corner".to_string(), "45deg".to_string()))
        );
        assert!(parse_inline_parameter_definition("10mm").is_none());
        assert!(parse_inline_parameter_definition("1bad=5").is_none());
        assert!(parse_inline_parameter_definition("width=").is_none());
    }

    #[test]
    fn try_commit_inline_parameter_definition_creates_parameter() {
        let mut doc = Document::default();
        let mut text = "width=10mm".to_string();
        let outcome = try_commit_inline_parameter_definition(&mut doc, &mut text).unwrap();
        assert_eq!(outcome, Some(InlineParameterCommit::Created("width".to_string())));
        assert_eq!(text, "width");
        assert_eq!(doc.parameters.values().next().unwrap().name, "width");
        assert_eq!(doc.parameters.values().next().unwrap().expression, "10mm");
    }

    /// #147 / SPEC §5.1.1: `name=value` on an existing name **redefines** that parameter
    /// (no duplicate-name error), and bare `name=` **reuses** it unchanged.
    #[test]
    fn inline_definition_redefines_or_reuses_an_existing_parameter() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "dia".to_string(), "20mm".to_string()).unwrap();

        let mut text = "dia=30".to_string();
        let outcome = try_commit_inline_parameter_definition(&mut doc, &mut text).unwrap();
        assert_eq!(outcome, Some(InlineParameterCommit::Redefined("dia".to_string())));
        assert_eq!(text, "dia");
        assert_eq!(doc.parameters.values().next().unwrap().expression, "30");
        assert_eq!(doc.parameters.len(), 1, "redefine must not add a second parameter");

        let mut text = "dia=".to_string();
        let outcome = try_commit_inline_parameter_definition(&mut doc, &mut text).unwrap();
        assert_eq!(outcome, Some(InlineParameterCommit::Reused("dia".to_string())));
        assert_eq!(text, "dia");
        assert_eq!(
            doc.parameters.values().next().unwrap().expression,
            "30",
            "reuse leaves the value unchanged"
        );
    }

    /// A bare `name=` for a name that doesn't exist stays untouched (nothing to reuse) —
    /// the field's normal unknown-variable handling takes over.
    #[test]
    fn inline_bare_equals_without_existing_parameter_is_left_alone() {
        let mut doc = Document::default();
        let mut text = "dia=".to_string();
        let outcome = try_commit_inline_parameter_definition(&mut doc, &mut text).unwrap();
        assert_eq!(outcome, None);
        assert_eq!(text, "dia=");
        assert!(doc.parameters.is_empty());
    }

    #[test]
    fn rejects_parameter_names_with_spaces() {
        let mut doc = Document::default();
        let err = add_parameter(&mut doc, "my width".to_string(), "10mm".to_string()).unwrap_err();
        assert_eq!(err, "Parameter name cannot contain spaces");
    }

    #[test]
    fn rejects_parameter_names_that_match_units() {
        let mut doc = Document::default();
        for unit in ["deg", "mm", "rad", "in"] {
            let err = add_parameter(&mut doc, unit.to_string(), "1".to_string()).unwrap_err();
            assert!(
                err.contains("conflicts with a known unit"),
                "unit={unit} err={err}"
            );
        }
        let err = add_parameter(&mut doc, "Deg".to_string(), "45deg".to_string()).unwrap_err();
        assert!(err.contains("conflicts with a known unit"));
    }

    #[test]
    fn format_parameter_value_display_shows_literal_unchanged() {
        let doc = Document::default();
        assert_eq!(format_parameter_value_display(&doc, "10mm"), "10mm");
        assert_eq!(format_parameter_value_display(&doc, "50"), "50");
    }

    /// #484: a bare angle literal is numerically identical to its computed display, so
    /// show only one form — not `92.0 deg (92deg)`.
    #[test]
    fn format_parameter_value_display_hides_identical_angle_literal() {
        let doc = Document::default();
        assert_eq!(format_parameter_value_display(&doc, "92deg"), "92deg");
        assert_eq!(format_parameter_value_display(&doc, "45 deg"), "45 deg");
        assert_eq!(format_parameter_value_display(&doc, "90.0deg"), "90.0deg");
        // Unit conversion still dual-displays when the typed unit differs from default.
        assert_eq!(
            format_parameter_value_display(&doc, "1rad"),
            format!(
                "{} (1rad)",
                crate::value::format_angle_display_in(1.0, doc.default_angle_unit)
            )
        );
    }

    #[test]
    fn format_parameter_value_display_shows_computed_for_expressions() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "B".to_string(), "A + 5mm".to_string()).unwrap();
        add_parameter(&mut doc, "C".to_string(), "2 * B".to_string()).unwrap();
        assert_eq!(
            format_parameter_value_display(&doc, "A + 5mm"),
            "10.0 mm (A + 5mm)"
        );
        assert_eq!(format_parameter_value_display(&doc, "A"), "5.0 mm (A)");
        assert_eq!(
            format_parameter_value_display(&doc, "2 * B"),
            "20.0 mm (2 * B)"
        );
    }

    #[test]
    fn parameter_edit_enter_pressed_accepts_lost_focus_from_singleline_textedit() {
        assert!(parameter_edit_enter_pressed(true, false, true));
        assert!(parameter_edit_enter_pressed(true, true, false));
        assert!(!parameter_edit_enter_pressed(true, false, false));
        assert!(!parameter_edit_enter_pressed(false, false, true));
    }

    /// #1179: min/max/step must commit when the field loses focus, not only on Enter.
    #[test]
    fn parameter_options_field_commits_on_blur() {
        // Blur without Enter: commit (this was false before the fix — draft discarded).
        assert!(parameter_options_field_should_commit(false, false, true));
        // Enter (singleline surrenders focus): commit.
        assert!(parameter_options_field_should_commit(true, false, true));
        // Enter while still focused: commit.
        assert!(parameter_options_field_should_commit(true, true, false));
        // Still typing: do not commit.
        assert!(!parameter_options_field_should_commit(false, true, false));
        assert!(!parameter_options_field_should_commit(false, false, false));
    }

    #[test]
    fn parameter_options_bound_expression_empty_clears() {
        assert_eq!(parameter_options_bound_expression("  5mm  ").as_deref(), Some("5mm"));
        assert_eq!(parameter_options_bound_expression("   "), None);
        assert_eq!(parameter_options_bound_expression(""), None);
    }

    #[test]
    fn commit_new_parameter_clears_fields_only_on_success() {
        let mut state = AppState::default();
        state.parameters_pane.new_name = "A".to_string();
        state.parameters_pane.new_value = "10mm".to_string();
        commit_new_parameter(&mut state).unwrap();
        assert_eq!(state.doc.parameters.len(), 1);
        assert!(state.parameters_pane.new_name.is_empty());
        assert!(state.parameters_pane.new_value.is_empty());
        assert!(state.parameters_pane.message.is_none());
    }

    #[test]
    fn commit_new_parameter_keeps_fields_on_validation_error() {
        let mut state = AppState::default();
        state.parameters_pane.new_name = "1bad".to_string();
        state.parameters_pane.new_value = "10mm".to_string();
        assert!(commit_new_parameter(&mut state).is_err());
        assert_eq!(state.doc.parameters.len(), 0);
        assert_eq!(state.parameters_pane.new_name, "1bad");
        assert_eq!(state.parameters_pane.new_value, "10mm");
        assert!(state.parameters_pane.message.is_some());
    }

    #[test]
    fn rejects_unknown_variable_in_parameter_expression() {
        let mut doc = doc_with_param_a();
        let a = doc.parameters.keys().next().unwrap();
        let err = set_parameter_expression(&mut doc, a, "Missing".to_string()).unwrap_err();
        assert_eq!(err, "Unknown variable: Missing");
    }

    #[test]
    fn rejects_direct_self_referencing_parameter() {
        let mut doc = Document::default();
        assert!(add_parameter(&mut doc, "A".to_string(), "A".to_string()).is_err());
    }

    #[test]
    fn rejects_two_parameter_cycle() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "B".to_string(), "A".to_string()).unwrap();
        let a = doc.parameters.keys().next().unwrap();
        let err = set_parameter_expression(&mut doc, a, "B".to_string()).unwrap_err();
        assert!(err.contains("Circular dependency"));
        assert!(err.contains("A"));
        assert!(err.contains("B"));
    }

    #[test]
    fn rejects_three_parameter_cycle() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "C".to_string(), "A".to_string()).unwrap();
        add_parameter(&mut doc, "B".to_string(), "C".to_string()).unwrap();
        let a = doc.parameters.keys().next().unwrap();
        let err = set_parameter_expression(&mut doc, a, "B".to_string()).unwrap_err();
        assert_eq!(err, "Circular dependency: A → B → C → A");
    }

    #[test]
    fn rejects_add_parameter_that_references_itself() {
        let mut doc = Document::default();
        let err = add_parameter(&mut doc, "A".to_string(), "A".to_string()).unwrap_err();
        assert!(err.contains("Circular dependency"));
    }

    #[test]
    fn allows_non_circular_parameter_chain() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "B".to_string(), "A + 5mm".to_string()).unwrap();
        add_parameter(&mut doc, "C".to_string(), "2 * B".to_string()).unwrap();
        assert_eq!(doc.parameters.len(), 3);
    }

    #[test]
    fn parameter_expression_cycle_warning_for_draft_expression() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "B".to_string(), "A".to_string()).unwrap();
        let a = doc.parameters.keys().next();
        let warning = parameter_expression_cycle_warning(&doc, "A", "B", a).unwrap();
        assert_eq!(warning, "Circular dependency: A → B → A");
    }

    #[test]
    fn validate_document_parameters_no_cycles_accepts_healthy_document() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "B".to_string(), "A + 5mm".to_string()).unwrap();
        validate_document_parameters_no_cycles(&doc).unwrap();
    }

    #[test]
    fn add_angle_parameter_with_degrees() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "corner".to_string(), "16.7deg".to_string()).unwrap();
        assert_eq!(doc.parameters.values().next().unwrap().expression, "16.7deg");
        match eval_parameter_in_doc("corner", &doc).unwrap() {
            EvaluatedParameter::AngleRad(v) => {
                assert!((v.to_degrees() - 16.7).abs() < 1e-3);
            }
            _ => panic!("expected angle parameter"),
        }
    }

    #[test]
    fn add_angle_parameter_with_radians() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "slope".to_string(), "1.5708rad".to_string()).unwrap();
        match eval_parameter_in_doc("slope", &doc).unwrap() {
            EvaluatedParameter::AngleRad(v) => {
                assert!((v - 1.5708).abs() < 1e-3);
            }
            _ => panic!("expected angle parameter"),
        }
    }

    #[test]
    fn angle_parameter_chain_evaluates() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "base".to_string(), "30deg".to_string()).unwrap();
        add_parameter(&mut doc, "offset".to_string(), "base + 5deg".to_string()).unwrap();
        match eval_parameter_in_doc("offset", &doc).unwrap() {
            EvaluatedParameter::AngleRad(v) => {
                assert!((v.to_degrees() - 35.0).abs() < 1e-3);
            }
            _ => panic!("expected angle parameter"),
        }
        assert_eq!(
            format_parameter_value_display(
                &doc,
                &doc.parameters.values().nth(1).unwrap().expression
            ),
            "35.0 deg (base + 5deg)"
        );
    }

    #[test]
    fn angle_parameter_drives_angle_constraint() {
        use crate::constraints::{add_angle_constraint_with_sign, angle_constraint_natural_sign};
        use crate::model::{ConstraintLine, Line, ShapeKind};

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        add_parameter(&mut doc, "corner".to_string(), "16.7deg".to_string()).unwrap();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 0.0));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 100.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.shape_order.push(ShapeKind::Line);
        let rotation_sign =
            angle_constraint_natural_sign(&doc, ConstraintLine::Line(lkey(0)), ConstraintLine::Line(lkey(1)))
                .unwrap();
        add_angle_constraint_with_sign(
            &mut doc,
            sketch,
            ConstraintLine::Line(lkey(0)),
            ConstraintLine::Line(lkey(1)),
            rotation_sign,
            "corner".to_string(),
        )
        .unwrap();
        let angle = crate::value::eval_angle_rad_in_doc("corner", &doc).unwrap();
        assert!((angle.to_degrees() - 16.7).abs() < 1e-2);
    }

    #[test]
    fn commit_new_parameter_supports_multiple_adds_in_sequence() {
        let mut state = AppState::default();
        state.parameters_pane.new_name = "A".to_string();
        state.parameters_pane.new_value = "10mm".to_string();
        commit_new_parameter(&mut state).unwrap();
        state.parameters_pane.new_name = "B".to_string();
        state.parameters_pane.new_value = "A + 5mm".to_string();
        commit_new_parameter(&mut state).unwrap();
        assert_eq!(state.doc.parameters.len(), 2);
        assert_eq!(state.doc.parameters.values().nth(1).unwrap().expression, "A + 5mm");
    }

    /// #453: the Dimension tool measures in 3D mode — a plain click on a line captures
    /// its length; an additive click defers (a pair is being built); completed pairs
    /// fire regardless of the modifier.
    /// #618: switching to the Dimension tool in 3D mode with a measuring selection made
    /// keeps it — nothing fires until the pane's "Derive parameter" button commits, so
    /// the parameter can be named first; the explicit action then records it.
    #[test]
    fn set_dimension_tool_keeps_selection_until_derive_commits() {
        use crate::actions::{Action, Tool};
        use crate::hierarchy::SceneElement;
        use crate::model::{FaceId, Line, ParameterSource, ShapeKind};
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        state.doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 40.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.scene_selection.insert(SceneElement::Line(lkey(0)));
        state.apply(Action::SetTool(Tool::Dimension));
        assert!(state.doc.parameters.is_empty(), "no auto-created parameter");
        assert!(!state.scene_selection.is_empty(), "selection carries into the tool");
        let source =
            derived_source_from_selection(&state.doc, &state.scene_selection).expect("source");
        assert_eq!(source, ParameterSource::LineLength(lkey(0)));
        state.apply(Action::CreateDerivedParameter {
            source,
            name: Some("width".to_string()),
        });
        assert_eq!(state.doc.parameters.len(), 1);
        assert_eq!(state.doc.parameters.values().next().unwrap().name, "width");
        assert_eq!(
            state.doc.parameters.values().next().unwrap().source,
            Some(ParameterSource::LineLength(lkey(0)))
        );
    }

    fn doc_with_unconstrained_line(length: f32) -> (Document, crate::model::LineKey) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let line = doc
            .lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, length, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        (doc, line)
    }

    /// #432: the selection classifies into a derived source, the derived value tracks
    /// geometry, and the focused-parameter highlight covers the defining elements.
    #[test]
    fn derived_parameters_from_selection_kinds() {
        use crate::hierarchy::SceneElement;
        use crate::model::{ConstraintPoint, FaceId, Line, LineEnd, ParameterSource};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 40.0, 0.0)); // 0
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 10.0, 40.0, 10.0)); // 1 ∥ 0
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 30.0, 30.0)); // 2 diagonal
        let mut sel = crate::selection::SceneSelection::default();

        // Two parallel lines → distance.
        sel.insert(SceneElement::Line(lkey(0)));
        sel.insert(SceneElement::Line(lkey(1)));
        let source = derived_source_from_selection(&doc, &sel).expect("parallel pair");
        assert_eq!(source, ParameterSource::LineDistance(lkey(0), lkey(1)));
        let (value, is_angle) = derived_source_value(&doc, &source).unwrap();
        assert!(!is_angle);
        assert!((value - 10.0).abs() < 1e-3);
        let index = add_derived_parameter(&mut doc, source.clone(), None).unwrap();
        assert!(parameter_value_is_readonly(&doc.parameters[index]));
        // A second parameter for the same measurement is refused.
        assert!(add_derived_parameter(&mut doc, source, None).is_err());

        // Two non-parallel same-sketch lines → angle (degrees).
        sel.clear();
        sel.insert(SceneElement::Line(lkey(0)));
        sel.insert(SceneElement::Line(lkey(2)));
        let source = derived_source_from_selection(&doc, &sel).expect("angle pair");
        assert_eq!(source, ParameterSource::LineAngle(lkey(0), lkey(2)));
        let (value, is_angle) = derived_source_value(&doc, &source).unwrap();
        assert!(is_angle);
        assert!((value - 45.0).abs() < 0.1, "angle {value}");
        let index = add_derived_parameter(&mut doc, source, None).unwrap();
        assert!(doc.parameters[index].expression.contains("deg"));

        // Two points → distance; moving the geometry re-syncs the value.
        sel.clear();
        sel.insert(SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::Start,
        }));
        sel.insert(SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::End,
        }));
        let source = derived_source_from_selection(&doc, &sel).expect("point pair");
        let _ = add_derived_parameter(&mut doc, source.clone(), Some("span".into())).unwrap();
        assert!((crate::value::eval_length_mm_in_doc("span", &doc).unwrap() - 40.0).abs() < 1e-2);
        doc.lines[lkey(0)].x1 = 60.0;
        sync_computed_parameters(&mut doc);
        assert!((crate::value::eval_length_mm_in_doc("span", &doc).unwrap() - 60.0).abs() < 1e-2);

        // The focused derived parameter highlights its defining elements.
        let highlighted = elements_using_parameter(&doc, "span");
        assert!(highlighted.contains(&SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::Start,
        })));
        assert!(highlighted.contains(&SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::End,
        })));
    }

    #[test]
    fn add_computed_parameter_from_line_length_creates_readonly_parameter() {
        let (mut doc, line_index) = doc_with_unconstrained_line(12.5);
        let index =
            add_computed_parameter_from_line_length(&mut doc, line_index, None).unwrap();
        let param = &doc.parameters[index];
        assert_eq!(param.name, "line0_length");
        assert_eq!(param.expression, "12.5 mm");
        assert!(parameter_value_is_readonly(param));
        assert!(matches!(
            param.source,
            Some(ParameterSource::LineLength(l)) if l == lkey(0)
        ));
    }

    #[test]
    fn computed_parameter_updates_when_line_length_changes() {
        let (mut doc, line_index) = doc_with_unconstrained_line(10.0);
        add_computed_parameter_from_line_length(&mut doc, line_index, None).unwrap();
        doc.lines[lkey(0)].x1 = 25.0;
        recompute_document_geometry(&mut doc).unwrap();
        assert_eq!(doc.parameters.values().next().unwrap().expression, "25.0 mm");
    }

    #[test]
    fn computed_parameter_rejects_constrained_line() {
        let (mut doc, line_index) = doc_with_unconstrained_line(10.0);
        let sketch = doc.lines[lkey(0)].sketch;
        add_distance_constraint(
            &mut doc,
            sketch,
            DistanceTarget::LineLength(line_index),
            "10mm".to_string(),
        )
        .unwrap();
        let err = add_computed_parameter_from_line_length(&mut doc, line_index, None).unwrap_err();
        assert_eq!(err, "Line length is constrained");
    }

    #[test]
    fn computed_parameter_survives_line_deletion() {
        let (mut doc, line_index) = doc_with_unconstrained_line(10.0);
        add_computed_parameter_from_line_length(&mut doc, line_index, None).unwrap();
        delete_element(&mut doc, SceneElement::Line(line_index));
        assert_eq!(doc.parameters.len(), 1);
        assert_eq!(doc.parameters.values().next().unwrap().expression, "10.0 mm");
        let health = crate::document_health::recompute_document_health(&doc);
        let param = doc.parameters.keys().next().expect("the parameter");
        assert_eq!(
            health.parameter_status(param),
            crate::document_health::HealthStatus::Invalid
        );
    }

    #[test]
    fn set_parameter_expression_rejects_readonly_computed_parameter() {
        let (mut doc, line_index) = doc_with_unconstrained_line(10.0);
        let index =
            add_computed_parameter_from_line_length(&mut doc, line_index, None).unwrap();
        let err = set_parameter_expression(&mut doc, index, "20mm".to_string()).unwrap_err();
        assert_eq!(err, "Parameter value is read-only");
    }
}