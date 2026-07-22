//! Document parameters: named length or angle expressions that drive sketch dimensions.

use crate::actions::{Action, ActionResult, AppState};
use crate::constraints::{
    find_distance_constraint, propagate_parameter_rename_to_constraints, solve_document_constraints,
};
use crate::icons::{icon_button, IconId};
use crate::document_health::HealthStatus;
use crate::model::{effective_length_unit, DistanceTarget, Document, Parameter, ParameterSource};
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

fn param_name_id(index: usize) -> Id {
    Id::new(("bearcad_parameters_name", index))
}

fn param_value_id(index: usize) -> Id {
    Id::new(("bearcad_parameters_value", index))
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
pub fn format_parameter_autocomplete_value(doc: &Document, index: usize) -> String {
    let Some(param) = doc.parameters.get(index) else {
        return String::new();
    };
    if param.deleted {
        return String::new();
    }
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

/// Name of the parameter whose name/value field currently holds keyboard focus, if any.
pub fn focused_parameter_name(ctx: &egui::Context, doc: &Document) -> Option<String> {
    let focused = ctx.memory(|m| m.focused())?;
    doc.parameters.iter().enumerate().find_map(|(index, param)| {
        if param.deleted {
            return None;
        }
        (focused == param_name_id(index) || focused == param_value_id(index))
            .then(|| param.name.clone())
    })
}

fn pane_element_for_constraint_line(line: crate::model::ConstraintLine) -> crate::hierarchy::SceneElement {
    use crate::hierarchy::SceneElement;
    use crate::model::ConstraintLine;
    match line {
        ConstraintLine::Line(index) => SceneElement::Line(index),
        // A face's own edge tracks the extrusion that produced its face, same as elsewhere.
        ConstraintLine::FaceEdge { face, .. } => {
            SceneElement::Extrusion(face.extrusion_index().unwrap_or(usize::MAX))
        }
        ConstraintLine::OriginAxis(_) => SceneElement::ConstructionPlane(0),
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
            SceneElement::Extrusion(face.extrusion_index().unwrap_or(usize::MAX))
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
    for param in doc.parameters.iter().filter(|p| !p.deleted && p.name == name) {
        if let Some(source) = &param.source {
            elements.extend(derived_source_elements(source));
        }
    }
    for (index, constraint) in doc.constraints.iter().enumerate() {
        if constraint.deleted {
            continue;
        }
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
    elements
}

pub fn parameter_field_focused(ctx: &egui::Context, doc: &Document) -> bool {
    ctx.memory(|m| {
        m.focused().is_some_and(|id| {
            if id == Id::new(NEW_NAME_ID) || id == Id::new(NEW_VALUE_ID) {
                return true;
            }
            doc.parameters.iter().enumerate().any(|(index, _)| {
                id == param_name_id(index) || id == param_value_id(index)
            })
        })
    })
}

/// Which cell is being edited in the parameters table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterEditCell {
    Name(usize),
    Value(usize),
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

pub fn parameter_index_by_name(doc: &Document, name: &str) -> Option<usize> {
    doc.parameters
        .iter()
        .position(|p| p.name == name)
}

pub fn duplicate_parameter_name(doc: &Document, name: &str, except: Option<usize>) -> bool {
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
pub fn line_eligible_for_computed_length_parameter(doc: &Document, line_index: usize) -> bool {
    crate::document_lifecycle::line_alive(doc, line_index)
        && find_distance_constraint(doc, DistanceTarget::LineLength(line_index)).is_none()
}

pub fn computed_parameter_index_for_line(doc: &Document, line_index: usize) -> Option<usize> {
    doc.parameters.iter().position(|param| {
        !param.deleted
            && matches!(
                param.source,
                Some(ParameterSource::LineLength(index)) if index == line_index
            )
    })
}

pub fn parameter_value_is_readonly(param: &Parameter) -> bool {
    param.source.is_some()
}

pub fn parameter_source_description(doc: &Document, param: &Parameter) -> Option<String> {
    let gone = |alive: bool| if alive { "" } else { " (deleted)" };
    match param.source.as_ref()? {
        ParameterSource::LineLength(index) => Some(format!(
            "Driven by line {index} length{}",
            gone(crate::document_lifecycle::line_alive(doc, *index))
        )),
        ParameterSource::PointDistance(..) => Some(format!(
            "Driven by point-to-point distance{}",
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
        ParameterSource::LineDistance(a, b) => Some(format!(
            "Driven by distance between lines {a} and {b}{}",
            gone(derived_source_value(doc, param.source.as_ref().unwrap()).is_some())
        )),
        ParameterSource::LineAngle(a, b) => Some(format!(
            "Driven by angle between lines {a} and {b}{}",
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
            let line = doc.lines.get(*index).filter(|l| !l.deleted)?;
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
    }
}

fn line_world_segment(doc: &Document, index: usize) -> Option<(glam::Vec3, glam::Vec3)> {
    let line = doc.lines.get(index).filter(|l| !l.deleted)?;
    let frame = crate::face::sketch_geometry_frame(doc, line.sketch)?;
    Some((
        crate::face::local_to_world(&frame, line.x0, line.y0),
        crate::face::local_to_world(&frame, line.x1, line.y1),
    ))
}

pub fn default_computed_parameter_name_for_line(doc: &Document, line_index: usize) -> String {
    unique_parameter_name(doc, &format!("line{line_index}_length"))
}

/// Update read-only parameter expressions from their geometry sources.
pub fn sync_computed_parameters(doc: &mut Document) {
    // Values are computed against an immutable view first (the derived evaluators walk
    // sketches/frames), then written back.
    let updates: Vec<(usize, String)> = doc
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.deleted)
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
    line_index: usize,
    name: Option<String>,
) -> Result<usize, String> {
    if !crate::document_lifecycle::line_alive(doc, line_index) {
        return Err(format!("Line {line_index} not found"));
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
    let index = doc.parameters.len();
    doc.parameters.push(Parameter {
        name,
        expression: format_length_display_in(length, unit),
        deleted: false,
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
        _ => None,
    }
}

/// The derived parameter a Dimension-tool click should create in 3D mode (#453).
/// Pair measurements fire as soon as the pair completes; a lone line's length only
/// fires on a plain click, since an additive (shift) click is building a pair.
pub fn dimension_click_derived_source(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
    additive: bool,
) -> Option<ParameterSource> {
    let source = derived_source_from_selection(doc, selection)?;
    if additive && matches!(source, ParameterSource::LineLength(_)) {
        return None;
    }
    Some(source)
}

/// Create a read-only parameter driven by `source` (#432). The generalization of
/// [`add_computed_parameter_from_line_length`] to every derived-source kind.
pub fn add_derived_parameter(
    doc: &mut Document,
    source: ParameterSource,
    name: Option<String>,
) -> Result<usize, String> {
    if let ParameterSource::LineLength(line_index) = source {
        return add_computed_parameter_from_line_length(doc, line_index, name);
    }
    let (value, is_angle) =
        derived_source_value(doc, &source).ok_or("Selection doesn't measure anything")?;
    if doc
        .parameters
        .iter()
        .any(|p| !p.deleted && p.source.as_ref() == Some(&source))
    {
        return Err("A parameter already tracks this measurement".to_string());
    }
    let base = match &source {
        ParameterSource::LineLength(_) => unreachable!(),
        ParameterSource::PointDistance(..) => "distance".to_string(),
        ParameterSource::LineDistance(a, b) => format!("line{a}_line{b}_distance"),
        ParameterSource::LineAngle(a, b) => format!("line{a}_line{b}_angle"),
    };
    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| unique_parameter_name(doc, &base));
    validate_new_parameter_name(doc, &name, None)?;
    let expression = if is_angle {
        crate::value::format_angle_display_in(value.to_radians(), doc.default_angle_unit)
    } else {
        format_length_display_in(value, doc.default_length_unit)
    };
    let index = doc.parameters.len();
    doc.parameters.push(Parameter {
        name,
        expression,
        deleted: false,
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
    }
}

/// Selected unconstrained line that can drive a computed length parameter.
pub fn line_for_computed_parameter_context_menu(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
) -> Option<usize> {
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
    on_create: &mut impl FnMut(usize),
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
    for param in &mut doc.parameters {
        param.expression = substitute_parameter_name(&param.expression, old, new);
    }
    for line in &mut doc.lines {
        if let Some(expr) = &mut line.length_expr {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
    for circle in &mut doc.circles {
        if let Some(expr) = &mut circle.diameter_expr {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
    propagate_parameter_rename_to_constraints(doc, old, new);
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
    result
}

/// Re-bake sketch-text glyph outlines from their raw templates (#338), so `{expr}` fields and
/// `size_expr` follow parameter edits. Text with no `{` and a constant/blank `size_expr` still
/// re-bakes harmlessly (identical result); a font that's since gone leaves the existing outlines.
pub fn rebake_sketch_texts(doc: &mut Document) {
    for i in 0..doc.sketch_texts.len() {
        let t = &doc.sketch_texts[i];
        if t.deleted {
            continue;
        }
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
    for i in 0..doc.extrusions.len() {
        let (deleted, expr, dist) = {
            let e = &doc.extrusions[i];
            (e.deleted, e.expression.clone(), e.distance)
        };
        if deleted || expr.trim().is_empty() {
            continue;
        }
        if let Some(mag) = crate::value::eval_length_mm_in_doc(&expr, doc) {
            let sign = if dist < 0.0 { -1.0 } else { 1.0 };
            doc.extrusions[i].distance = mag.abs() * sign;
        }
    }
}

pub fn validate_new_parameter_name(doc: &Document, name: &str, except: Option<usize>) -> Result<(), String> {
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
    existing_index: Option<usize>,
) -> Vec<(String, String)> {
    let mut bindings: Vec<(String, String)> = doc
        .parameters
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let expr = if existing_index == Some(index) {
                expression.to_string()
            } else {
                param.expression.clone()
            };
            (param.name.clone(), expr)
        })
        .collect();
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
    existing_index: Option<usize>,
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
    existing_index: Option<usize>,
) -> Option<String> {
    let expression = expression.trim();
    if expression.is_empty() || param_name.trim().is_empty() {
        return None;
    }
    find_parameter_dependency_cycle(doc, param_name, expression, existing_index)
        .map(|cycle| format_circular_dependency_error(&cycle))
}

pub fn validate_document_parameters_no_cycles(doc: &Document) -> Result<(), String> {
    for (index, param) in doc.parameters.iter().enumerate() {
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
    existing_index: Option<usize>,
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
            if let Some(index) = parameter_index_by_name(doc, &name) {
                if !doc.parameters[index].deleted {
                    *text = name.clone();
                    return Ok(Some(InlineParameterCommit::Reused(name)));
                }
            }
        }
    }
    let Some((name, value)) = parse_inline_parameter_definition(text) else {
        return Ok(None);
    };
    // `name=value` on an existing (live) name redefines that parameter's expression; a
    // deleted parameter still reserves its name, so it falls through to `add_parameter`'s
    // duplicate-name error rather than silently editing an invisible row.
    if let Some(index) = parameter_index_by_name(doc, &name) {
        if !doc.parameters[index].deleted {
            set_parameter_expression(doc, index, value)?;
            *text = name.clone();
            return Ok(Some(InlineParameterCommit::Redefined(name)));
        }
    }
    add_parameter(doc, name.clone(), value)?;
    *text = name.clone();
    Ok(Some(InlineParameterCommit::Created(name)))
}

pub fn add_parameter(doc: &mut Document, name: String, expression: String) -> Result<usize, String> {
    let name = name.trim().to_string();
    let expression = expression.trim().to_string();
    validate_new_parameter_name(doc, &name, None)?;
    validate_parameter_expression_for(doc, &name, &expression, None)?;
    let index = doc.parameters.len();
    doc.parameters.push(Parameter {
        name,
        expression,
        deleted: false,
        source: None,
    });
    doc.shape_order.push(crate::model::ShapeKind::Parameter);
    recompute_document_geometry(doc)?;
    Ok(index)
}

pub fn set_parameter_name(doc: &mut Document, index: usize, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    let old = doc
        .parameters
        .get(index)
        .ok_or_else(|| format!("Parameter {index} not found"))?
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
    index: usize,
    expression: String,
) -> Result<(), String> {
    let expression = expression.trim().to_string();
    let param = doc
        .parameters
        .get(index)
        .ok_or_else(|| format!("Parameter {index} not found"))?;
    require_parameter_value_editable(param)?;
    let param_name = param.name.clone();
    validate_parameter_expression_for(doc, &param_name, &expression, Some(index))?;
    doc.parameters[index].expression = expression;
    recompute_document_geometry(doc)
}

pub fn delete_parameter(doc: &mut Document, index: usize) -> Result<(), String> {
    if index >= doc.parameters.len() {
        return Err(format!("Parameter {index} not found"));
    }
    if !crate::document_lifecycle::tombstone_parameter(doc, index) {
        return Err(format!("Parameter {index} already deleted"));
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

pub fn show_pane(ui: &mut egui::Ui, app: &mut AppState) {
    use crate::expression_input::ParameterExpressionContext;
    use egui::{Grid, ScrollArea, TextEdit};

    ui.heading(PANE_TITLE);
    ui.add_space(4.0);

    // A row's ✕ delete button queues here and is applied after the grid, so the loop keeps its
    // borrow of `app` (#270).
    let mut delete_index: Option<usize> = None;

    ScrollArea::vertical().show(ui, |ui| {
        Grid::new("parameters_table")
            .num_columns(3)
            .spacing([8.0, 4.0])
            .min_col_width(72.0)
            .show(ui, |ui| {
                ui.label("Name");
                ui.label("Value");
                ui.label("");
                ui.end_row();

                let count = app.doc.parameters.len();
                let enter = ui.input(|i| i.key_pressed(Key::Enter));

                for index in 0..count {
                    if !crate::document_lifecycle::parameter_alive(&app.doc, index) {
                        continue;
                    }
                    let (param_name, param_expression, param_display, param_status, value_readonly, source_description) = {
                        let param = &app.doc.parameters[index];
                        (
                            param.name.clone(),
                            param.expression.clone(),
                            format_parameter_value_display(&app.doc, &param.expression),
                            app.document_health.parameter_status(index),
                            parameter_value_is_readonly(param),
                            parameter_source_description(&app.doc, param),
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

                    ui.horizontal(|ui| {
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

                    ui.horizontal(|ui| {
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
                    ui.horizontal(|ui| {
                        // Delete button (#270): a muted-red ✕ that removes the parameter.
                        let remove = ui.add(
                            egui::ImageButton::new(crate::icons::sized_texture(
                                ui.ctx(),
                                crate::icons::IconId::Close,
                            ))
                            .frame(false)
                            .tint(egui::Color32::from_rgb(0xC9, 0x6F, 0x66)),
                        );
                        if remove.on_hover_text("Delete parameter").clicked() {
                            delete_index = Some(index);
                        }
                        if let Some(reason) = source_description {
                            ui.label(
                                RichText::new(reason)
                                    .color(egui::Color32::from_gray(140))
                                    .size(11.0),
                            );
                        } else if param_frozen {
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
                    ui.end_row();
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

    // Derived parameter from the selection (#432): when the selection measures something
    // (a line, two points, two parallel lines, two same-plane lines), show the value it
    // would capture next to a create button.
    if let Some(source) = derived_source_from_selection(&app.doc, &app.scene_selection) {
        if let Some((value, is_angle)) = derived_source_value(&app.doc, &source) {
            ui.add_space(6.0);
            ui.separator();
            let display = if is_angle {
                crate::value::format_angle_display_in(
                    value.to_radians(),
                    app.doc.default_angle_unit,
                )
            } else {
                format_length_display_in(value, app.doc.default_length_unit)
            };
            ui.horizontal(|ui| {
                if ui
                    .button("Derive from selection")
                    .on_hover_text(
                        "Create a read-only parameter that tracks this measurement",
                    )
                    .clicked()
                {
                    apply_parameter_action(
                        app,
                        Action::CreateDerivedParameter { source: source.clone(), name: None },
                    );
                }
                ui.add_enabled(
                    false,
                    egui::TextEdit::singleline(&mut display.clone()).desired_width(80.0),
                );
            });
        }
    }

    if let Some(message) = &app.parameters_pane.message {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(message)
                .color(egui::Color32::from_rgb(255, 140, 100))
                .size(12.0),
        );
    } else if app.doc.parameters.is_empty() {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Type name and value (e.g. A and 10mm or 45deg), then press Enter or +")
                .color(egui::Color32::from_gray(140))
                .size(12.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::AppState;
    use crate::constraints::add_distance_constraint;
    use crate::document_lifecycle::tombstone_element;
    use crate::hierarchy::SceneElement;
    use crate::model::{DistanceTarget, Document, FaceId, Line, ShapeKind};

    fn doc_with_param_a() -> Document {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "5mm".to_string()).unwrap();
        doc
    }

    #[test]
    fn add_multiple_parameters_in_sequence() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "5mm".to_string()).unwrap();
        add_parameter(&mut doc, "B".to_string(), "A + 5in".to_string()).unwrap();
        add_parameter(&mut doc, "width".to_string(), "2 * B".to_string()).unwrap();
        assert_eq!(doc.parameters.len(), 3);
        assert_eq!(doc.parameters[2].expression, "2 * B");
    }

    #[test]
    fn add_parameter_stores_name_and_expression() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "width".to_string(), "2in".to_string()).unwrap();
        assert_eq!(doc.parameters.len(), 1);
        assert_eq!(doc.parameters[0].name, "width");
        assert_eq!(doc.parameters[0].expression, "2in");
        assert!(doc.shape_order.contains(&ShapeKind::Parameter));
    }

    #[test]
    fn parameter_rename_updates_dependent_expressions() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "B".to_string(), "A + 5in".to_string()).unwrap();
        set_parameter_name(&mut doc, 0, "Len".to_string()).unwrap();
        assert_eq!(doc.parameters[1].expression, "Len + 5in");
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
        assert_eq!(doc.parameters[0].name, "width");
        assert_eq!(doc.parameters[0].expression, "10mm");
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
        assert_eq!(doc.parameters[0].expression, "30");
        assert_eq!(doc.parameters.len(), 1, "redefine must not add a second parameter");

        let mut text = "dia=".to_string();
        let outcome = try_commit_inline_parameter_definition(&mut doc, &mut text).unwrap();
        assert_eq!(outcome, Some(InlineParameterCommit::Reused("dia".to_string())));
        assert_eq!(text, "dia");
        assert_eq!(doc.parameters[0].expression, "30", "reuse leaves the value unchanged");
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
        let err = set_parameter_expression(&mut doc, 0, "Missing".to_string()).unwrap_err();
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
        let err = set_parameter_expression(&mut doc, 0, "B".to_string()).unwrap_err();
        assert!(err.contains("Circular dependency"));
        assert!(err.contains("A"));
        assert!(err.contains("B"));
    }

    #[test]
    fn rejects_three_parameter_cycle() {
        let mut doc = doc_with_param_a();
        add_parameter(&mut doc, "C".to_string(), "A".to_string()).unwrap();
        add_parameter(&mut doc, "B".to_string(), "C".to_string()).unwrap();
        let err = set_parameter_expression(&mut doc, 0, "B".to_string()).unwrap_err();
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
        let warning = parameter_expression_cycle_warning(&doc, "A", "B", Some(0)).unwrap();
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
        assert_eq!(doc.parameters[0].expression, "16.7deg");
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
            format_parameter_value_display(&doc, &doc.parameters[1].expression),
            "35.0 deg (base + 5deg)"
        );
    }

    #[test]
    fn angle_parameter_drives_angle_constraint() {
        use crate::constraints::{add_angle_constraint_with_sign, angle_constraint_natural_sign};
        use crate::model::{ConstraintLine, Line, ShapeKind};

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        add_parameter(&mut doc, "corner".to_string(), "16.7deg".to_string()).unwrap();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 0.0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 100.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.shape_order.push(ShapeKind::Line);
        let rotation_sign =
            angle_constraint_natural_sign(&doc, ConstraintLine::Line(0), ConstraintLine::Line(1))
                .unwrap();
        add_angle_constraint_with_sign(
            &mut doc,
            sketch,
            ConstraintLine::Line(0),
            ConstraintLine::Line(1),
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
        assert_eq!(state.doc.parameters[1].expression, "A + 5mm");
    }

    /// #453: the Dimension tool measures in 3D mode — a plain click on a line captures
    /// its length; an additive click defers (a pair is being built); completed pairs
    /// fire regardless of the modifier.
    #[test]
    fn dimension_tool_click_measures_in_3d_mode() {
        use crate::hierarchy::SceneElement;
        use crate::model::{FaceId, Line, ParameterSource};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 40.0, 0.0)); // 0
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 10.0, 40.0, 10.0)); // 1 ∥ 0
        let mut sel = crate::selection::SceneSelection::default();

        sel.insert(SceneElement::Line(0));
        assert_eq!(
            dimension_click_derived_source(&doc, &sel, false),
            Some(ParameterSource::LineLength(0)),
            "plain click on a line should capture its length"
        );
        assert_eq!(
            dimension_click_derived_source(&doc, &sel, true),
            None,
            "shift+click on a lone line is building a pair — don't fire yet"
        );

        sel.insert(SceneElement::Line(1));
        assert_eq!(
            dimension_click_derived_source(&doc, &sel, true),
            Some(ParameterSource::LineDistance(0, 1)),
            "a completed pair fires regardless of the modifier"
        );
    }

    /// #453: with a measuring selection already made, switching to the Dimension tool
    /// in 3D mode captures the derived parameter immediately and clears the selection.
    #[test]
    fn set_dimension_tool_with_selection_creates_derived_parameter_in_3d() {
        use crate::actions::{Action, Tool};
        use crate::hierarchy::SceneElement;
        use crate::model::{FaceId, Line, ParameterSource, ShapeKind};
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 40.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.scene_selection.insert(SceneElement::Line(0));
        state.apply(Action::SetTool(Tool::Dimension));
        assert_eq!(state.doc.parameters.len(), 1);
        assert_eq!(
            state.doc.parameters[0].source,
            Some(ParameterSource::LineLength(0))
        );
        assert!(state.scene_selection.is_empty());
        assert!(state.status.contains("Added derived parameter"));
    }

    fn doc_with_unconstrained_line(length: f32) -> (Document, usize) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, length, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        (doc, 0)
    }

    /// #432: the selection classifies into a derived source, the derived value tracks
    /// geometry, and the focused-parameter highlight covers the defining elements.
    #[test]
    fn derived_parameters_from_selection_kinds() {
        use crate::hierarchy::SceneElement;
        use crate::model::{ConstraintPoint, FaceId, Line, LineEnd, ParameterSource};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 40.0, 0.0)); // 0
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 10.0, 40.0, 10.0)); // 1 ∥ 0
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 30.0, 30.0)); // 2 diagonal
        let mut sel = crate::selection::SceneSelection::default();

        // Two parallel lines → distance.
        sel.insert(SceneElement::Line(0));
        sel.insert(SceneElement::Line(1));
        let source = derived_source_from_selection(&doc, &sel).expect("parallel pair");
        assert_eq!(source, ParameterSource::LineDistance(0, 1));
        let (value, is_angle) = derived_source_value(&doc, &source).unwrap();
        assert!(!is_angle);
        assert!((value - 10.0).abs() < 1e-3);
        let index = add_derived_parameter(&mut doc, source.clone(), None).unwrap();
        assert!(parameter_value_is_readonly(&doc.parameters[index]));
        // A second parameter for the same measurement is refused.
        assert!(add_derived_parameter(&mut doc, source, None).is_err());

        // Two non-parallel same-sketch lines → angle (degrees).
        sel.clear();
        sel.insert(SceneElement::Line(0));
        sel.insert(SceneElement::Line(2));
        let source = derived_source_from_selection(&doc, &sel).expect("angle pair");
        assert_eq!(source, ParameterSource::LineAngle(0, 2));
        let (value, is_angle) = derived_source_value(&doc, &source).unwrap();
        assert!(is_angle);
        assert!((value - 45.0).abs() < 0.1, "angle {value}");
        let index = add_derived_parameter(&mut doc, source, None).unwrap();
        assert!(doc.parameters[index].expression.contains("deg"));

        // Two points → distance; moving the geometry re-syncs the value.
        sel.clear();
        sel.insert(SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: 0,
            end: LineEnd::Start,
        }));
        sel.insert(SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: 0,
            end: LineEnd::End,
        }));
        let source = derived_source_from_selection(&doc, &sel).expect("point pair");
        let _ = add_derived_parameter(&mut doc, source.clone(), Some("span".into())).unwrap();
        assert!((crate::value::eval_length_mm_in_doc("span", &doc).unwrap() - 40.0).abs() < 1e-2);
        doc.lines[0].x1 = 60.0;
        sync_computed_parameters(&mut doc);
        assert!((crate::value::eval_length_mm_in_doc("span", &doc).unwrap() - 60.0).abs() < 1e-2);

        // The focused derived parameter highlights its defining elements.
        let highlighted = elements_using_parameter(&doc, "span");
        assert!(highlighted.contains(&SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: 0,
            end: LineEnd::Start,
        })));
        assert!(highlighted.contains(&SceneElement::Point(ConstraintPoint::LineEndpoint {
            line: 0,
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
            Some(ParameterSource::LineLength(0))
        ));
    }

    #[test]
    fn computed_parameter_updates_when_line_length_changes() {
        let (mut doc, line_index) = doc_with_unconstrained_line(10.0);
        add_computed_parameter_from_line_length(&mut doc, line_index, None).unwrap();
        doc.lines[0].x1 = 25.0;
        recompute_document_geometry(&mut doc).unwrap();
        assert_eq!(doc.parameters[0].expression, "25.0 mm");
    }

    #[test]
    fn computed_parameter_rejects_constrained_line() {
        let (mut doc, line_index) = doc_with_unconstrained_line(10.0);
        let sketch = doc.lines[0].sketch;
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
        tombstone_element(&mut doc, SceneElement::Line(line_index));
        assert_eq!(doc.parameters.len(), 1);
        assert_eq!(doc.parameters[0].expression, "10.0 mm");
        let health = crate::document_health::recompute_document_health(&doc);
        assert_eq!(
            health.parameter_status(0),
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