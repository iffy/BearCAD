//! Shared validation and UI for length expression text fields.

use crate::command_palette::fuzzy_score;
use crate::model::Document;
use crate::parameters::{format_parameter_autocomplete_value, parameter_expression_cycle_warning};
use crate::value::{
    document_parameter_names, format_unknown_variable_error,
    unknown_variables_in_expression, unknown_variables_in_parameter_expression,
};
use eframe::egui::{self, Frame, Id, Key, Margin, Order, Response, Stroke, TextEdit};
use egui::text::{CCursor, CCursorRange};
use egui::widgets::text_edit::TextEditState;

pub const ERROR_TOOLTIP_GAP: f32 = 4.0;
pub const INVALID_BORDER: egui::Color32 = egui::Color32::from_rgb(220, 100, 90);
pub const INVALID_BG: egui::Color32 = egui::Color32::from_rgb(52, 30, 30);
pub const INVALID_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 190, 170);
pub const ERROR_TOOLTIP_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 180, 120);
const AUTOCOMPLETE_MAX: usize = 8;

/// Context for validating a parameter definition expression (cycle detection).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterExpressionContext {
    pub param_name: String,
    pub existing_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutocompleteMatch {
    pub name: String,
    pub value: String,
    pub score: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AutocompleteUiState {
    highlight: usize,
    last_query: String,
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
        // Inline parameter syntax (SPEC §5.1.1, #147): in `name=value` the left side is the
        // name being *defined*, not a reference — only the right side can contain unknown
        // variables. While the value is still being typed (`name=` so far), there's nothing
        // to warn about yet.
        let expression = match text.split_once('=') {
            Some((_, value)) => value.trim(),
            None => text,
        };
        if !expression.is_empty() {
            let known_names = document_parameter_names(doc);
            errors.extend(
                unknown_variables_in_expression(expression, &known_names)
                    .into_iter()
                    .map(|name| format_unknown_variable_error(&name)),
            );
        }
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
            Frame::popup(ui.style()).show(ui, |ui| {
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
        // Match the element picker's vertical padding (4px) so value inputs and
        // element pickers are the same height when stacked in a tool pane (#599).
        .inner_margin(Margin::symmetric(4, 4))
        .corner_radius(widget.corner_radius)
}

fn is_identifier_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Character range `[start, end)` of the identifier token touching `cursor_char_index`.
pub fn identifier_token_at_cursor(text: &str, cursor_char_index: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let cursor = cursor_char_index.min(chars.len());
    let mut start = cursor;
    while start > 0 && is_identifier_part(chars[start - 1]) {
        start -= 1;
    }
    let mut end = cursor;
    while end < chars.len() && is_identifier_part(chars[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    let first = chars[start];
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if start > 0 {
        let before = chars[..start]
            .iter()
            .rposition(|c| !c.is_whitespace())
            .map(|idx| chars[idx]);
        if before.is_some_and(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }
    }
    Some((start, end))
}

fn token_query(text: &str, token: (usize, usize)) -> String {
    text.chars().skip(token.0).take(token.1 - token.0).collect()
}

/// Whether `cursor` (a char index) sits inside an open `{…}` interpolation field (#338), honoring
/// `{{`/`}}` escapes. Used to scope variable autocomplete to brace fields in free-text areas.
pub fn cursor_inside_interp_field(text: &str, cursor: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let end = cursor.min(chars.len());
    let mut i = 0;
    let mut in_field = false;
    while i < end {
        let c = chars[i];
        if in_field {
            if c == '}' {
                in_field = false;
            }
            i += 1;
        } else if c == '{' && chars.get(i + 1) == Some(&'{') {
            i += 2; // escaped literal brace
        } else if c == '}' && chars.get(i + 1) == Some(&'}') {
            i += 2; // escaped literal brace
        } else {
            if c == '{' {
                in_field = true;
            }
            i += 1;
        }
    }
    in_field
}

/// Like [`identifier_token_at_cursor`], but only when the cursor is inside a `{…}` field (#338),
/// so variable completion fires inside brace fields but not on ordinary words of free text.
pub fn interp_identifier_token_at_cursor(text: &str, cursor: usize) -> Option<(usize, usize)> {
    if !cursor_inside_interp_field(text, cursor) {
        return None;
    }
    identifier_token_at_cursor(text, cursor)
}

fn char_range_to_byte_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    let byte_start = text
        .char_indices()
        .nth(start)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let byte_end = text
        .char_indices()
        .nth(end)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    (byte_start, byte_end)
}

pub fn parameter_autocomplete_candidates(
    doc: &Document,
    query: &str,
    exclude_names: &[&str],
) -> Vec<AutocompleteMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for (index, param) in doc.parameters.iter().enumerate() {
        if param.deleted || exclude_names.iter().any(|name| *name == param.name) {
            continue;
        }
        let Some(score) = fuzzy_score(query, &param.name) else {
            continue;
        };
        matches.push(AutocompleteMatch {
            name: param.name.clone(),
            value: format_parameter_autocomplete_value(doc, index),
            score,
        });
    }
    matches.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    matches.truncate(AUTOCOMPLETE_MAX);
    matches
}

fn autocomplete_state_id(id: Id) -> Id {
    id.with("expression_autocomplete")
}

fn load_autocomplete_state(ctx: &egui::Context, id: Id) -> AutocompleteUiState {
    ctx.data_mut(|d| {
        d.get_temp::<AutocompleteUiState>(autocomplete_state_id(id))
            .unwrap_or_default()
    })
}

fn store_autocomplete_state(ctx: &egui::Context, id: Id, state: AutocompleteUiState) {
    ctx.data_mut(|d| d.insert_temp(autocomplete_state_id(id), state));
}

fn apply_parameter_completion(
    text: &mut String,
    token: (usize, usize),
    name: &str,
    text_state: &mut TextEditState,
) {
    let (byte_start, byte_end) = char_range_to_byte_range(text, token.0, token.1);
    text.replace_range(byte_start..byte_end, name);
    let cursor = token.0 + name.chars().count();
    text_state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(cursor))));
}

fn cursor_char_index(state: Option<&TextEditState>, text: &str) -> usize {
    state
        .and_then(|state| state.cursor.char_range())
        .map(|range| range.primary.index)
        .unwrap_or_else(|| text.chars().count())
}

/// The current caret char index for the text-edit widget `id`, for positioning `{…}` variable
/// autocomplete on free-text areas (#338).
pub fn text_edit_cursor_char_index(ctx: &egui::Context, id: Id, text: &str) -> usize {
    cursor_char_index(TextEditState::load(ctx, id).as_ref(), text)
}

/// Handle autocomplete keyboard input before the text edit runs.
pub fn expression_autocomplete_handle_keys(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    id: Id,
    text: &mut String,
    doc: &Document,
    exclude_names: &[&str],
) -> bool {
    autocomplete_handle_keys_with(ui, ctx, id, text, doc, exclude_names, identifier_token_at_cursor)
}

/// Like [`expression_autocomplete_handle_keys`], but scoped to `{…}` fields for free-text areas
/// with variable interpolation (#338).
pub fn interp_autocomplete_handle_keys(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    id: Id,
    text: &mut String,
    doc: &Document,
    exclude_names: &[&str],
) -> bool {
    autocomplete_handle_keys_with(
        ui,
        ctx,
        id,
        text,
        doc,
        exclude_names,
        interp_identifier_token_at_cursor,
    )
}

fn autocomplete_handle_keys_with(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    id: Id,
    text: &mut String,
    doc: &Document,
    exclude_names: &[&str],
    token_at: impl Fn(&str, usize) -> Option<(usize, usize)>,
) -> bool {
    let Some(mut text_state) = TextEditState::load(ctx, id) else {
        return false;
    };
    let cursor = cursor_char_index(Some(&text_state), text);
    let Some(token) = token_at(text, cursor) else {
        return false;
    };
    let query = token_query(text, token);
    let candidates = parameter_autocomplete_candidates(doc, &query, exclude_names);
    if candidates.is_empty() {
        return false;
    }

    let mut ui_state = load_autocomplete_state(ctx, id);
    if ui_state.last_query != query {
        ui_state.highlight = 0;
        ui_state.last_query = query;
    }
    ui_state.highlight = ui_state.highlight.min(candidates.len().saturating_sub(1));

    let up = ui.input(|i| i.key_pressed(Key::ArrowUp));
    let down = ui.input(|i| i.key_pressed(Key::ArrowDown));
    let space = ui.input(|i| i.key_pressed(Key::Space));
    let tab = ui.input(|i| i.key_pressed(Key::Tab));
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let mut changed = false;

    if up {
        ui_state.highlight = ui_state.highlight.saturating_sub(1);
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowUp));
    } else if down {
        ui_state.highlight = (ui_state.highlight + 1).min(candidates.len() - 1);
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowDown));
    } else if space || tab {
        // Space or Tab accepts the highlighted (top by default) candidate and keeps editing
        // with the caret at the end of the completed name (#50/#507).
        let name = candidates[ui_state.highlight].name.clone();
        apply_parameter_completion(text, token, &name, &mut text_state);
        text_state.store(ctx, id);
        let key = if space { Key::Space } else { Key::Tab };
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
        // Keep focus on this field even if Tab's focus-navigation already fired this frame.
        ctx.memory_mut(|m| m.request_focus(id));
        changed = true;
    } else if enter {
        // Enter accepts the highlighted candidate too, but is left unconsumed so the field's
        // own Enter handling still commits the (now completed) expression in one keystroke (#50).
        let name = candidates[ui_state.highlight].name.clone();
        apply_parameter_completion(text, token, &name, &mut text_state);
        text_state.store(ctx, id);
        changed = true;
    }

    store_autocomplete_state(ctx, id, ui_state);
    changed
}

/// Show the autocomplete dropdown below a focused expression field.
pub fn expression_autocomplete_show_dropdown(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anchor: &Response,
    id: Id,
    text: &mut String,
    doc: &Document,
    exclude_names: &[&str],
    cursor_char_index: usize,
) -> bool {
    autocomplete_show_dropdown_with(
        ui, ctx, anchor, id, text, doc, exclude_names, cursor_char_index,
        identifier_token_at_cursor,
    )
}

/// Like [`expression_autocomplete_show_dropdown`], scoped to `{…}` fields (#338).
pub fn interp_autocomplete_show_dropdown(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    anchor: &Response,
    id: Id,
    text: &mut String,
    doc: &Document,
    exclude_names: &[&str],
    cursor_char_index: usize,
) -> bool {
    autocomplete_show_dropdown_with(
        ui, ctx, anchor, id, text, doc, exclude_names, cursor_char_index,
        interp_identifier_token_at_cursor,
    )
}

#[allow(clippy::too_many_arguments)]
fn autocomplete_show_dropdown_with(
    _ui: &mut egui::Ui,
    ctx: &egui::Context,
    anchor: &Response,
    id: Id,
    text: &mut String,
    doc: &Document,
    exclude_names: &[&str],
    cursor_char_index: usize,
    token_at: impl Fn(&str, usize) -> Option<(usize, usize)>,
) -> bool {
    let Some(token) = token_at(text, cursor_char_index) else {
        return false;
    };
    let query = token_query(text, token);
    let candidates = parameter_autocomplete_candidates(doc, &query, exclude_names);
    if candidates.is_empty() {
        return false;
    }

    let mut ui_state = load_autocomplete_state(ctx, id);
    if ui_state.last_query != query {
        ui_state.highlight = 0;
        ui_state.last_query = query;
    }
    ui_state.highlight = ui_state.highlight.min(candidates.len().saturating_sub(1));
    store_autocomplete_state(ctx, id, ui_state.clone());

    let highlight = ui_state.highlight;
    let mut changed = false;
    let anchor_id = anchor.id;
    let token_for_click = token;

    egui::Area::new(anchor_id.with("expression_autocomplete"))
        .order(Order::Foreground)
        .fixed_pos(anchor.rect.left_bottom())
        .show(ctx, |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(anchor.rect.width().max(160.0));
                for (index, candidate) in candidates.iter().enumerate() {
                    let selected = index == highlight;
                    let label = format!("{}   {}", candidate.name, candidate.value);
                    let response = ui.selectable_label(selected, label);
                    if response.clicked() {
                        if let Some(mut text_state) = TextEditState::load(ctx, id) {
                            apply_parameter_completion(
                                text,
                                token_for_click,
                                &candidate.name,
                                &mut text_state,
                            );
                            text_state.store(ctx, id);
                            changed = true;
                        }
                    }
                }
            });
        });

    changed
}

/// Parameters-pane style length expression input with shared validation UI.
pub fn show_length_expression_text_edit(
    ui: &mut egui::Ui,
    text: &mut String,
    id: Id,
    hint_text: &str,
    errors: &[String],
    doc: &Document,
    exclude_names: &[&str],
) -> Response {
    let ctx = ui.ctx().clone();
    let had_focus = ctx.memory(|m| m.focused()) == Some(id);
    if had_focus {
        expression_autocomplete_handle_keys(ui, &ctx, id, text, doc, exclude_names);
    }

    let invalid = !errors.is_empty();
    let output = length_expression_text_edit_frame(ui, id, invalid)
        .show(ui, |ui| {
            // lock_focus so Tab is available for parameter autocomplete (#507) instead of
            // moving keyboard focus to the next widget (egui's singleline default).
            let mut edit = TextEdit::singleline(text)
                .id(id)
                .hint_text(hint_text)
                .desired_width(f32::INFINITY)
                .frame(false)
                .margin(Margin::ZERO)
                .lock_focus(true);
            if invalid {
                edit = edit.text_color(INVALID_TEXT);
            }
            edit.show(ui)
        })
        .inner;

    if output.response.has_focus() {
        // Touch devices: a focused value field gets the app keypad, not the OS keyboard.
        crate::touch::set_value_field_focused(true);
        let cursor = cursor_char_index(Some(&output.state), text);
        if expression_autocomplete_show_dropdown(
            ui,
            &ctx,
            &output.response,
            id,
            text,
            doc,
            exclude_names,
            cursor,
        ) {
            output.state.clone().store(&ctx, id);
        }
    }

    show_expression_error_tooltips_above(ui, &output.response, errors);
    output.response
}

/// What a [`ValueInput`] measures — picks the parser, the default unit added to the
/// computed value, and its display formatting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// A length in the document's default length unit.
    Length,
    /// An angle in the document's default angle unit.
    Angle,
    /// A unitless number (e.g. a count).
    Count,
}

/// The one standard numeric input (#456): a styled expression field that accepts
/// variables, expressions, functions, units, and inline `name=value` definitions,
/// with autocomplete, error tooltips, and the **computed value alongside** whenever
/// it differs from what was typed (including units — a bare number gains the default
/// unit in the preview; retyping exactly the computed value hides it).
pub struct ValueInput<'a> {
    pub id: Id,
    pub kind: ValueKind,
    pub hint: &'a str,
    /// Inline `name=value` parameter definitions allowed (off e.g. in the Parameters
    /// pane's value column, where the row *is* the definition).
    pub allow_definitions: bool,
    /// Field width; `None` fills the available width.
    pub width: Option<f32>,
    /// Cycle/self-reference checking context when editing an existing parameter.
    pub parameter_context: Option<&'a ParameterExpressionContext>,
    /// Parameter names excluded from autocomplete (e.g. the parameter being edited).
    pub exclude_names: &'a [&'a str],
}

impl<'a> ValueInput<'a> {
    pub fn new(id: impl std::hash::Hash, kind: ValueKind) -> Self {
        Self {
            id: Id::new(id),
            kind,
            hint: "",
            allow_definitions: true,
            width: None,
            parameter_context: None,
            exclude_names: &[],
        }
    }

    /// Like [`ValueInput::new`], with an already-built [`Id`].
    pub fn from_id(id: Id, kind: ValueKind) -> Self {
        Self { id, ..Self::new("", kind) }
    }

    pub fn exclude_names(mut self, names: &'a [&'a str]) -> Self {
        self.exclude_names = names;
        self
    }

    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    pub fn no_definitions(mut self) -> Self {
        self.allow_definitions = false;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn parameter_context(mut self, ctx: &'a ParameterExpressionContext) -> Self {
        self.parameter_context = Some(ctx);
        self
    }

    /// Render the field (and the computed-value label beside it, when it differs).
    /// Returns the field's response; `.changed()` reports edits as usual.
    pub fn show(self, ui: &mut egui::Ui, text: &mut String, doc: &Document) -> Response {
        let mut errors = length_expression_field_errors(text, doc, self.parameter_context);
        if !self.allow_definitions && text.contains('=') {
            errors.insert(0, "name=value definitions aren't allowed here".to_string());
        }
        let resp = match self.width {
            Some(w) => {
                ui.scope(|ui| {
                    ui.set_max_width(w);
                    show_length_expression_text_edit(
                        ui,
                        text,
                        self.id,
                        self.hint,
                        &errors,
                        doc,
                        self.exclude_names,
                    )
                })
                .inner
            }
            None => show_length_expression_text_edit(
                ui,
                text,
                self.id,
                self.hint,
                &errors,
                doc,
                self.exclude_names,
            ),
        };
        // The computed value floats *below* the field (#501) instead of sitting beside
        // it in the layout — so it appearing or disappearing while typing never shifts
        // anything around. Shown only while the field is focused (idle rows stay
        // unobscured); error tooltips use the same spot and win when present.
        if errors.is_empty() && resp.has_focus() {
            if let Some(computed) = value_input_computed_display(text, self.kind, doc) {
                egui::Area::new(self.id.with("value_input_computed"))
                    .order(egui::Order::Tooltip)
                    .pivot(egui::Align2::LEFT_TOP)
                    .fixed_pos(resp.rect.left_bottom() + egui::vec2(0.0, 2.0))
                    .interactable(false)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::default()
                            .fill(ui.style().visuals.extreme_bg_color.gamma_multiply(0.9))
                            .corner_radius(4.0)
                            .inner_margin(egui::Margin::symmetric(4, 1))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(format!("= {computed}"))
                                        .color(egui::Color32::from_gray(170))
                                        .size(11.0),
                                );
                            });
                    });
            }
        }
        resp
    }
}

/// The computed-value text a [`ValueInput`] shows beside the field, or `None` when the
/// input already reads identically (ignoring whitespace and case, so `12.5mm` matches
/// `12.5 mm`).
pub fn value_input_computed_display(
    text: &str,
    kind: ValueKind,
    doc: &Document,
) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let display = match kind {
        ValueKind::Length => {
            let v = crate::value::computed_length_in_doc(t, doc)?;
            crate::value::format_length_display_in(v, doc.default_length_unit)
        }
        ValueKind::Angle => {
            let v = crate::value::computed_angle_in_doc(t, doc)?;
            crate::value::format_angle_display_in(v, doc.default_angle_unit)
        }
        ValueKind::Count => {
            let v = crate::value::computed_length_in_doc(t, doc)?;
            let rounded = v.round();
            if (v - rounded).abs() > 1e-4 {
                format!("{v}")
            } else {
                format!("{}", rounded as i64)
            }
        }
    };
    // For inline definitions, compare the right-hand side against the value.
    let typed = match t.split_once('=') {
        Some((_, rhs)) => rhs,
        None => t,
    };
    if canonical_value_text(typed) == canonical_value_text(&display) {
        return None;
    }
    Some(display)
}

/// Canonical form for comparing a typed value against its computed display: lowercase,
/// whitespace dropped, and the leading number normalized (`45.0 deg` == `45deg`).
pub(crate) fn canonical_value_text(s: &str) -> String {
    let squashed: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let split = squashed
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(squashed.len());
    let (num, rest) = squashed.split_at(split);
    match num.parse::<f64>() {
        Ok(v) => format!("{v}{rest}"),
        Err(_) => squashed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameters::add_parameter;

    /// #456: the computed value shows exactly when it differs from the typed text —
    /// a bare number gains the default unit, an exact match (modulo spacing/case) hides.
    #[test]
    fn value_input_computed_display_semantics() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "gap".to_string(), "3mm".to_string()).unwrap();
        // Bare number: default unit added to the preview.
        assert_eq!(
            value_input_computed_display("12.5", ValueKind::Length, &doc),
            Some("12.5 mm".to_string())
        );
        // Identical including units (whitespace-insensitive): hidden.
        assert_eq!(value_input_computed_display("12.5 mm", ValueKind::Length, &doc), None);
        assert_eq!(value_input_computed_display("12.5mm", ValueKind::Length, &doc), None);
        // Unit conversion shows.
        assert_eq!(
            value_input_computed_display("1in", ValueKind::Length, &doc),
            Some("25.4 mm".to_string())
        );
        // Parameters and expressions show their value.
        assert_eq!(
            value_input_computed_display("gap", ValueKind::Length, &doc),
            Some("3.0 mm".to_string())
        );
        assert_eq!(
            value_input_computed_display("gap * 2", ValueKind::Length, &doc),
            Some("6.0 mm".to_string())
        );
        // Inline definitions preview their right-hand side.
        assert_eq!(
            value_input_computed_display("a = 2 + 3", ValueKind::Length, &doc),
            Some("5.0 mm".to_string())
        );
        // Counts stay unitless and hide when identical.
        assert_eq!(value_input_computed_display("4", ValueKind::Count, &doc), None);
        assert_eq!(
            value_input_computed_display("2*2", ValueKind::Count, &doc),
            Some("4".to_string())
        );
        // Angles use the angle parser and default angle unit.
        assert_eq!(value_input_computed_display("45deg", ValueKind::Angle, &doc), None);
        assert_eq!(
            value_input_computed_display("45deg + 45deg", ValueKind::Angle, &doc),
            Some("90.0 deg".to_string())
        );
        // Empty shows nothing.
        assert_eq!(value_input_computed_display("", ValueKind::Length, &doc), None);
    }

    #[test]
    fn length_expression_field_errors_reports_unknown_variable() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "10mm".to_string()).unwrap();
        let errors = length_expression_field_errors("A + B", &doc, None);
        assert_eq!(errors, vec!["Unknown variable: B".to_string()]);
    }

    /// #147: while typing an inline definition (`dia=10`, or the partial `dia=`), the name
    /// left of `=` is being *defined* — no unknown-variable warning may appear for it. Only
    /// the right side is checked as an expression.
    #[test]
    fn inline_definition_left_side_never_warns_unknown_variable() {
        let doc = Document::default();
        assert!(length_expression_field_errors("dia=10", &doc, None).is_empty());
        assert!(length_expression_field_errors("dia=", &doc, None).is_empty());
        assert_eq!(
            length_expression_field_errors("dia=foo", &doc, None),
            vec!["Unknown variable: foo".to_string()],
            "the right side of `=` is still checked"
        );
        assert_eq!(
            length_expression_field_errors("dia", &doc, None),
            vec!["Unknown variable: dia".to_string()],
            "without `=` the text is a plain expression and still warns"
        );
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

    #[test]
    fn identifier_token_at_cursor_finds_partial_name() {
        let text = "10mm + wid";
        let end = text.chars().count();
        assert_eq!(identifier_token_at_cursor(text, end), Some((7, 10)));
    }

    #[test]
    fn identifier_token_at_cursor_ignores_unit_suffix() {
        let text = "10mm";
        assert_eq!(identifier_token_at_cursor(text, 4), None);
    }

    /// #338: variable autocomplete in free text fires only inside a `{…}` field, honoring
    /// `{{`/`}}` escapes.
    #[test]
    fn interp_token_only_inside_brace_fields() {
        // Cursor after "wid" inside a field → token found.
        let inside = "Dim: {wid";
        let end = inside.chars().count();
        assert_eq!(interp_identifier_token_at_cursor(inside, end), Some((6, 9)));
        // Same word in plain text (no open brace) → no completion.
        let plain = "Dim: wid";
        assert_eq!(interp_identifier_token_at_cursor(plain, plain.chars().count()), None);
        // After the field closes → no completion.
        let closed = "{foo} bar";
        assert!(!cursor_inside_interp_field(closed, closed.chars().count()));
        // An escaped `{{` does not open a field.
        let escaped = "{{lit";
        assert!(!cursor_inside_interp_field(escaped, escaped.chars().count()));
        // A real field after an escape still opens.
        let mixed = "{{x}} {foo";
        assert!(cursor_inside_interp_field(mixed, mixed.chars().count()));
    }

    #[test]
    fn parameter_autocomplete_candidates_fuzzy_matches() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "width".to_string(), "10mm".to_string()).unwrap();
        add_parameter(&mut doc, "height".to_string(), "5mm".to_string()).unwrap();
        let matches = parameter_autocomplete_candidates(&doc, "wid", &[]);
        assert_eq!(matches.first().map(|m| m.name.as_str()), Some("width"));
    }

    #[test]
    fn apply_parameter_completion_replaces_partial_token() {
        // Backs the Tab/Enter completion in #50: "wid" -> "width".
        let mut text = "10mm + wid".to_string();
        let token = identifier_token_at_cursor(&text, text.chars().count()).unwrap();
        let mut state = TextEditState::default();
        apply_parameter_completion(&mut text, token, "width", &mut state);
        assert_eq!(text, "10mm + width");
    }

    /// #507: after Tab completes a parameter, the caret sits at the end so typing `/2`
    /// appends rather than replacing the completed name.
    #[test]
    fn apply_parameter_completion_places_cursor_at_end_of_name() {
        let mut text = "larg".to_string();
        let token = identifier_token_at_cursor(&text, text.chars().count()).unwrap();
        let mut state = TextEditState::default();
        apply_parameter_completion(&mut text, token, "largeWidth", &mut state);
        assert_eq!(text, "largeWidth");
        let cursor = state
            .cursor
            .char_range()
            .expect("cursor after completion")
            .primary
            .index;
        assert_eq!(cursor, "largeWidth".chars().count());
        // Simulate appending `/2` at that caret (what TextEdit does for unselected input).
        text.insert_str(cursor, "/2");
        assert_eq!(text, "largeWidth/2");
    }

    #[test]
    fn parameter_autocomplete_candidates_exclude_names() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "width".to_string(), "10mm".to_string()).unwrap();
        let matches = parameter_autocomplete_candidates(&doc, "wid", &["width"]);
        assert!(matches.is_empty());
    }
}