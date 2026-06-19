//! Document parameters: named length expressions that drive sketch dimensions.

use crate::actions::{Action, ActionResult, AppState};
use crate::model::{Document, Parameter};
use crate::value::{
    eval_length_mm_in_doc, expression_references_document_parameter, format_length_display,
    is_valid_parameter_name, substitute_parameter_name,
};
use eframe::egui::{self, Id, Key};

pub const PANE_TITLE: &str = "Parameters";

const NEW_NAME_ID: &str = "le3_parameters_new_name";
const NEW_VALUE_ID: &str = "le3_parameters_new_value";

fn param_name_id(index: usize) -> Id {
    Id::new(("le3_parameters_name", index))
}

fn param_value_id(index: usize) -> Id {
    Id::new(("le3_parameters_value", index))
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
    expression_references_document_parameter(doc, expr)
}

/// Value-column label for a stored parameter expression.
pub fn format_parameter_value_display(doc: &Document, expression: &str) -> String {
    let expr = expression.trim();
    if !parameter_value_is_expression(doc, expr) {
        return expr.to_string();
    }
    match eval_length_mm_in_doc(expr, doc) {
        Some(v) => format!("{} ({expr})", format_length_display(v)),
        None => expr.to_string(),
    }
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

/// Rename `old` to `new` in every expression that references it.
pub fn propagate_parameter_rename(doc: &mut Document, old: &str, new: &str) {
    if old == new {
        return;
    }
    for param in &mut doc.parameters {
        param.expression = substitute_parameter_name(&param.expression, old, new);
    }
    for rect in &mut doc.rects {
        if let Some(expr) = &mut rect.width_expr {
            *expr = substitute_parameter_name(expr, old, new);
        }
        if let Some(expr) = &mut rect.height_expr {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
    for line in &mut doc.lines {
        if let Some(expr) = &mut line.length_expr {
            *expr = substitute_parameter_name(expr, old, new);
        }
    }
}

/// Re-evaluate locked sketch dimensions from their stored expressions.
pub fn recompute_document_geometry(doc: &mut Document) -> Result<(), String> {
    for i in 0..doc.rects.len() {
        if doc.rects[i].width_locked {
            if let Some(expr) = doc.rects[i].width_expr.clone() {
                let w = eval_length_mm_in_doc(&expr, doc)
                    .ok_or_else(|| format!("Invalid width expression '{expr}'"))?;
                if w <= 0.0 {
                    return Err(format!("Width expression '{expr}' must be positive"));
                }
                doc.rects[i].w = w;
            }
        }
        if doc.rects[i].height_locked {
            if let Some(expr) = doc.rects[i].height_expr.clone() {
                let h = eval_length_mm_in_doc(&expr, doc)
                    .ok_or_else(|| format!("Invalid height expression '{expr}'"))?;
                if h <= 0.0 {
                    return Err(format!("Height expression '{expr}' must be positive"));
                }
                doc.rects[i].h = h;
            }
        }
    }
    for i in 0..doc.lines.len() {
        if !doc.lines[i].length_locked {
            continue;
        }
        let Some(expr) = doc.lines[i].length_expr.clone() else {
            continue;
        };
        let len = eval_length_mm_in_doc(&expr, doc)
            .ok_or_else(|| format!("Invalid length expression '{expr}'"))?;
        if len <= 0.0 {
            return Err(format!("Length expression '{expr}' must be positive"));
        }
        let du = doc.lines[i].x1 - doc.lines[i].x0;
        let dv = doc.lines[i].y1 - doc.lines[i].y0;
        let dist = (du * du + dv * dv).sqrt();
        if dist < 1e-6 {
            doc.lines[i].x1 = doc.lines[i].x0 + len;
            doc.lines[i].y1 = doc.lines[i].y0;
        } else {
            let scale = len / dist;
            doc.lines[i].x1 = doc.lines[i].x0 + du * scale;
            doc.lines[i].y1 = doc.lines[i].y0 + dv * scale;
        }
    }
    Ok(())
}

pub fn validate_new_parameter_name(doc: &Document, name: &str, except: Option<usize>) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Parameter name is required".to_string());
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

pub fn validate_parameter_expression(doc: &Document, expression: &str) -> Result<(), String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("Parameter value is required".to_string());
    }
    eval_length_mm_in_doc(expression, doc)
        .ok_or_else(|| format!("Invalid expression '{expression}'"))?;
    Ok(())
}

pub fn add_parameter(doc: &mut Document, name: String, expression: String) -> Result<usize, String> {
    let name = name.trim().to_string();
    let expression = expression.trim().to_string();
    validate_new_parameter_name(doc, &name, None)?;
    validate_parameter_expression(doc, &expression)?;
    let index = doc.parameters.len();
    doc.parameters.push(Parameter { name, expression });
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
    if doc.parameters.get(index).is_none() {
        return Err(format!("Parameter {index} not found"));
    }
    validate_parameter_expression(doc, &expression)?;
    doc.parameters[index].expression = expression;
    recompute_document_geometry(doc)
}

pub fn delete_parameter(doc: &mut Document, index: usize) -> Result<(), String> {
    if index >= doc.parameters.len() {
        return Err(format!("Parameter {index} not found"));
    }
    if let Some(pos) = doc
        .shape_order
        .iter()
        .enumerate()
        .filter(|(_, k)| **k == crate::model::ShapeKind::Parameter)
        .nth(index)
        .map(|(i, _)| i)
    {
        doc.shape_order.remove(pos);
    }
    doc.parameters.remove(index);
    Ok(())
}

fn apply_parameter_action(state: &mut AppState, action: Action) {
    match state.apply(action) {
        ActionResult::Ok => state.parameters_pane.message = None,
        ActionResult::Err(e) => state.parameters_pane.message = Some(e),
        ActionResult::NeedsDialog => {
            state.parameters_pane.message = Some("Unexpected dialog request".to_string());
        }
    }
}

pub fn show_pane(ui: &mut egui::Ui, app: &mut AppState) {
    use egui::{Grid, ScrollArea, TextEdit};

    ui.heading(PANE_TITLE);
    ui.add_space(4.0);

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
                    let (param_name, param_expression, param_display) = {
                        let param = &app.doc.parameters[index];
                        (
                            param.name.clone(),
                            param.expression.clone(),
                            format_parameter_value_display(&app.doc, &param.expression),
                        )
                    };
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
                            if app.parameters_pane.editing_focus {
                                response.request_focus();
                                app.parameters_pane.editing_focus = false;
                            }
                            if enter && response.has_focus() {
                                let draft = app.parameters_pane.draft.clone();
                                apply_parameter_action(
                                    app,
                                    Action::CommitParameterName {
                                        index,
                                        name: draft,
                                    },
                                );
                                app.parameters_pane.cancel_edit();
                                ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, Key::Enter);
                                });
                            }
                        } else if ui
                            .selectable_label(false, &param_name)
                            .clicked()
                        {
                            app.parameters_pane
                                .begin_edit(ParameterEditCell::Name(index), &param_name);
                        }
                    });

                    ui.horizontal(|ui| {
                        if editing_value {
                            let response = ui.add(
                                TextEdit::singleline(&mut app.parameters_pane.draft)
                                    .id(param_value_id(index))
                                    .desired_width(f32::INFINITY),
                            );
                            if app.parameters_pane.editing_focus {
                                response.request_focus();
                                app.parameters_pane.editing_focus = false;
                            }
                            if enter && response.has_focus() {
                                let draft = app.parameters_pane.draft.clone();
                                apply_parameter_action(
                                    app,
                                    Action::CommitParameterExpression {
                                        index,
                                        expression: draft,
                                    },
                                );
                                app.parameters_pane.cancel_edit();
                                ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, Key::Enter);
                                });
                            }
                        } else if ui
                            .selectable_label(false, &param_display)
                            .clicked()
                        {
                            app.parameters_pane.begin_edit(
                                ParameterEditCell::Value(index),
                                &param_expression,
                            );
                        }
                    });
                    ui.label("");
                    ui.end_row();
                }

                let name_response = ui.add(
                    TextEdit::singleline(&mut app.parameters_pane.new_name)
                        .id(Id::new(NEW_NAME_ID))
                        .hint_text("name")
                        .desired_width(f32::INFINITY),
                );
                if app.parameters_pane.focus_new_name {
                    name_response.request_focus();
                    app.parameters_pane.focus_new_name = false;
                }
                let value_response = ui.add(
                    TextEdit::singleline(&mut app.parameters_pane.new_value)
                        .id(Id::new(NEW_VALUE_ID))
                        .hint_text("value")
                        .desired_width(f32::INFINITY),
                );
                if app.parameters_pane.focus_new_value {
                    value_response.request_focus();
                    app.parameters_pane.focus_new_value = false;
                }

                let add_clicked = ui
                    .button("+")
                    .on_hover_text("Add parameter")
                    .clicked();

                if name_response.gained_focus() || value_response.gained_focus() {
                    app.parameters_pane.cancel_edit();
                }

                let mut commit_new = add_clicked;
                if enter && name_response.has_focus() {
                    if !app.parameters_pane.new_name.trim().is_empty()
                        && app.parameters_pane.new_value.trim().is_empty()
                    {
                        app.parameters_pane.focus_new_value = true;
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
                    } else if new_parameter_row_ready(&app.parameters_pane) {
                        commit_new = true;
                    }
                } else if enter
                    && value_response.has_focus()
                    && new_parameter_row_ready(&app.parameters_pane)
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
            egui::RichText::new("Type name and value (e.g. A and 10mm), then press Enter or +")
                .color(egui::Color32::from_gray(140))
                .size(12.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Document, FaceId, ShapeKind};
    use crate::Rect;

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
    fn parameter_value_change_recomputes_rectangle_width() {
        let mut doc = doc_with_param_a();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let mut rect = Rect::from_local_corners(sketch, 0.0, 0.0, 5.0, 10.0);
        rect.width_locked = true;
        rect.width_expr = Some("A".to_string());
        doc.rects.push(rect);
        doc.shape_order.push(ShapeKind::Rect);

        set_parameter_expression(&mut doc, 0, "10mm".to_string()).unwrap();
        assert!((doc.rects[0].w - 10.0).abs() < 1e-3);
    }

    #[test]
    fn rectangle_with_parameter_expression_evaluates_on_recompute() {
        let mut doc = doc_with_param_a();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let mut rect = Rect::from_local_corners(sketch, 0.0, 0.0, 1.0, 10.0);
        rect.width_locked = true;
        rect.width_expr = Some("A + 5in".to_string());
        doc.rects.push(rect);
        recompute_document_geometry(&mut doc).unwrap();
        assert!((doc.rects[0].w - (5.0 + 5.0 * 25.4)).abs() < 1e-2);
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
    fn format_parameter_value_display_shows_literal_unchanged() {
        let doc = Document::default();
        assert_eq!(format_parameter_value_display(&doc, "10mm"), "10mm");
        assert_eq!(format_parameter_value_display(&doc, "50"), "50");
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
}