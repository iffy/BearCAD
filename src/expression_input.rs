//! Shared validation and UI for length expression text fields.

use crate::model::Document;
use crate::parameters::parameter_expression_cycle_warning;
use crate::value::{
    document_parameter_names, format_unknown_variable_error,
    unknown_variables_in_expression, unknown_variables_in_parameter_expression,
};
use eframe::egui::{self, Frame, Id, Margin, Response, Stroke, TextEdit};

pub const ERROR_TOOLTIP_GAP: f32 = 4.0;
pub const INVALID_BORDER: egui::Color32 = egui::Color32::from_rgb(220, 100, 90);
pub const INVALID_BG: egui::Color32 = egui::Color32::from_rgb(52, 30, 30);
pub const INVALID_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 190, 170);
pub const ERROR_TOOLTIP_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 180, 120);

/// Context for validating a parameter definition expression (cycle detection).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterExpressionContext {
    pub param_name: String,
    pub existing_index: Option<usize>,
}

/// Live validation errors for a length expression field.
pub fn length_expression_field_errors(
    text: &str,
    doc: &Document,
    parameter_context: Option<&ParameterExpressionContext>,
) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let mut errors = Vec::new();
    if let Some(ctx) = parameter_context {
        errors.extend(
            unknown_variables_in_parameter_expression(
                text,
                doc,
                &ctx.param_name,
                ctx.existing_index,
            )
            .into_iter()
            .map(|name| format_unknown_variable_error(&name)),
        );
        if let Some(warning) = parameter_expression_cycle_warning(
            doc,
            &ctx.param_name,
            text,
            ctx.existing_index,
        ) {
            errors.push(warning);
        }
    } else {
        let known_names = document_parameter_names(doc);
        errors.extend(
            unknown_variables_in_expression(text, &known_names)
                .into_iter()
                .map(|name| format_unknown_variable_error(&name)),
        );
    }

    errors
}

pub fn show_expression_error_tooltips_above(ui: &egui::Ui, anchor: &Response, errors: &[String]) {
    if errors.is_empty() {
        return;
    }

    use egui::{Align2, Area, Frame, Order};

    Area::new(anchor.id.with("expression_error_tooltip"))
        .order(Order::Tooltip)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(anchor.rect.left_top() - egui::vec2(0.0, ERROR_TOOLTIP_GAP))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            Frame::popup(&ui.style()).show(ui, |ui| {
                for error in errors {
                    ui.label(egui::RichText::new(error).color(ERROR_TOOLTIP_TEXT));
                }
            });
        });
}

/// Frame matching default [`TextEdit`] metrics so error styling only changes colors.
fn length_expression_text_edit_frame(ui: &egui::Ui, id: Id, invalid: bool) -> Frame {
    let visuals = &ui.style().visuals;
    let focused = ui.ctx().memory(|m| m.focused()) == Some(id);
    let widget = if focused {
        &visuals.widgets.active
    } else {
        &visuals.widgets.inactive
    };
    let stroke = if invalid {
        Stroke::new(widget.bg_stroke.width, INVALID_BORDER)
    } else {
        widget.bg_stroke
    };

    Frame::default()
        .fill(if invalid {
            INVALID_BG
        } else {
            visuals.extreme_bg_color
        })
        .stroke(stroke)
        .inner_margin(Margin::symmetric(4.0, 2.0))
        .rounding(widget.rounding)
}

/// Parameters-pane style length expression input with shared validation UI.
pub fn show_length_expression_text_edit(
    ui: &mut egui::Ui,
    text: &mut String,
    id: Id,
    hint_text: &str,
    errors: &[String],
) -> Response {
    let invalid = !errors.is_empty();
    let response = length_expression_text_edit_frame(ui, id, invalid)
        .show(ui, |ui| {
            let mut edit = TextEdit::singleline(text)
                .id(id)
                .hint_text(hint_text)
                .desired_width(f32::INFINITY)
                .frame(false)
                .margin(Margin::ZERO);
            if invalid {
                edit = edit.text_color(INVALID_TEXT);
            }
            ui.add(edit)
        })
        .inner;

    show_expression_error_tooltips_above(ui, &response, errors);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameters::add_parameter;

    #[test]
    fn length_expression_field_errors_reports_unknown_variable() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "10mm".to_string()).unwrap();
        let errors = length_expression_field_errors("A + B", &doc, None);
        assert_eq!(errors, vec!["Unknown variable: B".to_string()]);
    }

    #[test]
    fn length_expression_field_errors_reports_cycle_for_parameter_context() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "5mm".to_string()).unwrap();
        add_parameter(&mut doc, "B".to_string(), "A".to_string()).unwrap();
        let errors = length_expression_field_errors(
            "B",
            &doc,
            Some(&ParameterExpressionContext {
                param_name: "A".to_string(),
                existing_index: Some(0),
            }),
        );
        assert_eq!(errors, vec!["Circular dependency: A → B → A".to_string()]);
    }

    #[test]
    fn length_expression_field_errors_reports_unknown_before_cycle() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "5mm".to_string()).unwrap();
        let errors = length_expression_field_errors(
            "Missing + B",
            &doc,
            Some(&ParameterExpressionContext {
                param_name: "A".to_string(),
                existing_index: Some(0),
            }),
        );
        assert_eq!(
            errors,
            vec![
                "Unknown variable: Missing".to_string(),
                "Unknown variable: B".to_string(),
            ]
        );
    }
}