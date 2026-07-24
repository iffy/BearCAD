//! A reusable element-picker control (#213): the single, consistent way every tool gathers
//! the scene elements it operates on.
//!
//! Historically each tool grew its own bespoke selection state (`creating_boolean.a`,
//! `creating_loft.sections`, the Chamfer/Fillet edge set, the constraint tool reading
//! `scene_selection` directly, …) with subtly different click, limit, and highlight rules.
//! [`ElementPicker`] replaces all of that with one configurable control:
//!
//! - it accepts a configurable **subset of element kinds** (planes, lines, bodies, operations,
//!   …), and can further restrict the [`OperationKind`]s it will take;
//! - it enforces a **pick limit** (a whole number, or [`PickLimit::Infinite`]);
//! - it renders like a focusable combo-box input with a generic empty state (the count plus
//!   the pickable kinds' icons, #388), a collapsed
//!   `N ⟨icon⟩` summary per kind, and an expandable popup with a remove button per row (the
//!   rendering lives in the context pane; this module is the state + rules it drives);
//! - it carries a **selected-highlight color** that defaults to the theme's selection color but
//!   can be overridden per picker (e.g. the Slice tool paints its cutters red).
//!
//! This module is deliberately free of egui widget code so the pick/limit/filter rules can be
//! unit-tested in isolation; only the small [`Color32`]/[`IconId`] value types are borrowed.

#![allow(dead_code)]

use crate::hierarchy::SceneElement;
use crate::icons::IconId;
use crate::model::Document;
use eframe::egui::{self, Color32};

/// A user-facing category of selectable scene element. Every [`SceneElement`] maps to exactly
/// one kind (see [`ElementKind::of`]); a picker accepts a configurable subset of kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementKind {
    /// A construction plane (or, for now, a tracing image sitting on one).
    Plane,
    Sketch,
    /// A straight sketch segment.
    Line,
    Circle,
    /// A point: a sketch/constraint point, a body corner, or the origin.
    Vertex,
    /// An edge of a body/face boundary (as opposed to a free sketch [`Line`](ElementKind::Line)).
    Edge,
    /// A flat face of a solid body (#555/#566), distinct from the whole [`Body`](ElementKind::Body):
    /// a picker can accept planes-or-faces without also taking whole bodies.
    Face,
    Constraint,
    /// A solid body.
    Body,
    Image,
    /// A history operation (extrude, boolean, move, repeat, slice, revolve). Restrict which ones
    /// with [`ElementFilter::operations`].
    Operation,
}

impl ElementKind {
    /// Kinds in the canonical order used for filter membership and the collapsed summary, so a
    /// picker accepting several kinds always renders them in the same, stable order.
    pub const ORDER: [ElementKind; 10] = [
        ElementKind::Plane,
        ElementKind::Sketch,
        ElementKind::Line,
        ElementKind::Circle,
        ElementKind::Vertex,
        ElementKind::Edge,
        ElementKind::Face,
        ElementKind::Constraint,
        ElementKind::Body,
        ElementKind::Operation,
    ];

    /// The kind an element belongs to. Total: every [`SceneElement`] has exactly one kind.
    pub fn of(element: &SceneElement) -> ElementKind {
        match element {
            SceneElement::ConstructionPlane(_) => ElementKind::Plane,
            SceneElement::Image(_) => ElementKind::Image,
            SceneElement::Sketch(_) => ElementKind::Sketch,
            SceneElement::Line(_) => ElementKind::Line,
            SceneElement::Circle(_) => ElementKind::Circle,
            SceneElement::Point(_) | SceneElement::BodyVertex { .. } | SceneElement::Origin => {
                ElementKind::Vertex
            }
            SceneElement::FaceEdge(_) | SceneElement::BodyEdge { .. } => ElementKind::Edge,
            SceneElement::Constraint(_) => ElementKind::Constraint,
            // A flat body face (#555/#566) is its own kind, so a "planes or faces" picker can
            // accept it without also accepting whole bodies.
            SceneElement::BodyFace { .. } => ElementKind::Face,
            SceneElement::Body(_) => ElementKind::Body,
            SceneElement::Component(_) => ElementKind::Operation,
            SceneElement::Extrusion(_)
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
            | SceneElement::SweepOp(_) => ElementKind::Operation,
        }
    }

    /// A representative icon for a collapsed summary chip of this kind.
    pub fn icon(self) -> IconId {
        match self {
            ElementKind::Plane => IconId::Plane,
            ElementKind::Image => IconId::Plane,
            ElementKind::Sketch => IconId::Sketch,
            ElementKind::Line => IconId::Line,
            ElementKind::Circle => IconId::Circle,
            // No dedicated point glyph; the coincident icon reads as "a point".
            ElementKind::Vertex => IconId::Coincident,
            ElementKind::Edge => IconId::Line,
            ElementKind::Face => IconId::Face,
            ElementKind::Constraint => IconId::Constraint,
            ElementKind::Body => IconId::Body,
            ElementKind::Operation => IconId::Gear,
        }
    }

    /// A short human label for hints and tooltips.
    pub fn label(self) -> &'static str {
        match self {
            ElementKind::Plane => "plane",
            ElementKind::Image => "image",
            ElementKind::Sketch => "sketch",
            ElementKind::Line => "line",
            ElementKind::Circle => "circle",
            ElementKind::Vertex => "vertex",
            ElementKind::Edge => "edge",
            ElementKind::Face => "face",
            ElementKind::Constraint => "constraint",
            ElementKind::Body => "body",
            ElementKind::Operation => "operation",
        }
    }
}

/// A history-operation sub-kind, so a picker can accept e.g. only bodies produced by a boolean
/// while rejecting move/repeat operations (the user's "limit it to selecting only certain
/// operations").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperationKind {
    Extrude,
    Boolean,
    Move,
    Mirror,
    Repeat,
    Slice,
    EdgeTreatment,
    Revolution,
}

impl OperationKind {
    /// The operation sub-kind of an element, or `None` if the element is not an operation.
    pub fn of(element: &SceneElement) -> Option<OperationKind> {
        Some(match element {
            SceneElement::Extrusion(_) => OperationKind::Extrude,
            SceneElement::BooleanOp(_) => OperationKind::Boolean,
            SceneElement::MoveOp(_) => OperationKind::Move,
            SceneElement::MirrorOp(_) => OperationKind::Mirror,
            SceneElement::RepeatOp(_) => OperationKind::Repeat,
            SceneElement::SliceOp(_) => OperationKind::Slice,
            SceneElement::EdgeTreatmentOp(_) => OperationKind::EdgeTreatment,
            SceneElement::Revolution(_) => OperationKind::Revolution,
            _ => return None,
        })
    }
}

/// Which elements a picker will accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementFilter {
    /// Accept every kind and every operation. The Select tool's "select everything" picker.
    everything: bool,
    /// Accepted kinds (ignored when `everything`). Ordered per [`ElementKind::ORDER`].
    kinds: Vec<ElementKind>,
    /// When `Some`, an [`ElementKind::Operation`] element is accepted only if its
    /// [`OperationKind`] is listed. `None` accepts every operation (subject to `kinds`).
    operations: Option<Vec<OperationKind>>,
}

impl ElementFilter {
    /// Accept anything selectable — used by the Select tool.
    pub fn everything() -> ElementFilter {
        ElementFilter {
            everything: true,
            kinds: Vec::new(),
            operations: None,
        }
    }

    /// Accept exactly the given kinds (deduplicated, canonically ordered).
    pub fn kinds(kinds: &[ElementKind]) -> ElementFilter {
        let mut ordered = Vec::new();
        for &k in ElementKind::ORDER.iter() {
            if kinds.contains(&k) {
                ordered.push(k);
            }
        }
        // Image shares the Plane row in ORDER; accept it explicitly when Plane is requested so a
        // "planes" picker also takes tracing images sitting on a plane.
        ElementFilter {
            everything: false,
            kinds: ordered,
            operations: None,
        }
    }

    /// A single-kind filter (the common case, e.g. "bodies only").
    pub fn kind(kind: ElementKind) -> ElementFilter {
        ElementFilter::kinds(&[kind])
    }

    /// Restrict accepted operations to the given sub-kinds. Implies [`ElementKind::Operation`].
    pub fn operations(mut self, ops: &[OperationKind]) -> ElementFilter {
        if !self.everything && !self.kinds.contains(&ElementKind::Operation) {
            self.kinds.push(ElementKind::Operation);
        }
        self.operations = Some(ops.to_vec());
        self
    }

    /// Whether a whole kind is (potentially) acceptable — drives hover styling of every element
    /// of that category while the picker is focused.
    /// The icons of the kinds this filter accepts, in canonical order, for the picker's
    /// generic empty state (#388). An accept-everything filter returns none — a bare count
    /// reads better than every icon at once.
    pub fn pickable_icons(&self) -> Vec<IconId> {
        if self.everything {
            return Vec::new();
        }
        let mut icons = Vec::new();
        for kind in &self.kinds {
            let icon = kind.icon();
            if !icons.contains(&icon) {
                icons.push(icon);
            }
        }
        icons
    }

    pub fn accepts_kind(&self, kind: ElementKind) -> bool {
        if self.everything {
            return true;
        }
        // Image is accepted wherever Plane is (see `kinds`).
        self.kinds.contains(&kind) || (kind == ElementKind::Image && self.kinds.contains(&ElementKind::Plane))
    }

    /// Whether a specific element is acceptable, honoring the operation restriction.
    pub fn accepts(&self, element: &SceneElement) -> bool {
        if self.everything {
            return true;
        }
        let kind = ElementKind::of(element);
        if !self.accepts_kind(kind) {
            return false;
        }
        if kind == ElementKind::Operation {
            if let Some(allowed) = &self.operations {
                return OperationKind::of(element).is_some_and(|op| allowed.contains(&op));
            }
        }
        true
    }
}

/// How many elements a picker will hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickLimit {
    /// At most `n` elements. `Finite(1)` is single-select: a new pick replaces the current one.
    Finite(usize),
    /// No cap.
    Infinite,
}

impl PickLimit {
    /// Whether one more element could be added when `current` are already picked.
    pub fn has_room(self, current: usize) -> bool {
        match self {
            PickLimit::Finite(n) => current < n,
            PickLimit::Infinite => true,
        }
    }

    /// Single-select pickers replace rather than reject on a new pick.
    pub fn is_single(self) -> bool {
        matches!(self, PickLimit::Finite(1))
    }
}

/// What happened when an element was offered to a picker via [`ElementPicker::pick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickOutcome {
    /// Newly added to the set.
    Added,
    /// Already present, so the click toggled it off.
    Removed,
    /// The picker replaced its single held element (a `Finite(1)` picker).
    Replaced,
    /// Rejected: wrong kind/operation for this picker's filter.
    NotAccepted,
    /// Rejected: the set is already at its (multi-element) limit.
    Full,
}

/// A configurable, focusable element-selection control. Holds both the configuration (filter,
/// limit, highlight color, focus stickiness) and the live picked set + focus state.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementPicker {
    filter: ElementFilter,
    limit: PickLimit,
    /// Overrides the theme selection color for this picker's highlights (e.g. Slice cutters red).
    selected_color: Option<Color32>,
    /// The Select tool's picker is always focused and cannot lose focus; `set_focused(false)` is
    /// a no-op for it.
    sticky_focus: bool,

    /// Picked elements in click order (stable for the popup rows and remove-by-index).
    picked: Vec<SceneElement>,
    focused: bool,
}

impl ElementPicker {
    /// A picker with the given filter and limit, unfocused, empty, default highlight color.
    pub fn new(filter: ElementFilter, limit: PickLimit) -> ElementPicker {
        ElementPicker {
            filter,
            limit,
            selected_color: None,
            sticky_focus: false,
            picked: Vec::new(),
            focused: false,
        }
    }

    /// The Select tool's picker: accepts everything, unbounded, and permanently focused.
    pub fn select_everything() -> ElementPicker {
        let mut picker = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        picker.sticky_focus = true;
        picker.focused = true;
        picker
    }

    // ---- builders -------------------------------------------------------------------------

    pub fn with_selected_color(mut self, color: Color32) -> ElementPicker {
        self.selected_color = Some(color);
        self
    }

    // ---- configuration accessors ----------------------------------------------------------

    pub fn filter(&self) -> &ElementFilter {
        &self.filter
    }

    pub fn limit(&self) -> PickLimit {
        self.limit
    }

    /// The highlight color for this picker's selected elements, resolving the per-picker override
    /// against the caller-supplied theme default.
    pub fn selected_color(&self, default: Color32) -> Color32 {
        self.selected_color.unwrap_or(default)
    }

    /// Whether this element is one this picker would accept (delegates to the filter).
    pub fn accepts(&self, element: &SceneElement) -> bool {
        self.filter.accepts(element)
    }

    // ---- focus ----------------------------------------------------------------------------

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Focus or blur the picker. A sticky (Select-tool) picker ignores blur requests.
    pub fn set_focused(&mut self, focused: bool) {
        if self.sticky_focus {
            self.focused = true;
        } else {
            self.focused = focused;
        }
    }

    pub fn has_sticky_focus(&self) -> bool {
        self.sticky_focus
    }

    // ---- picked set -----------------------------------------------------------------------

    pub fn picked(&self) -> &[SceneElement] {
        &self.picked
    }

    pub fn iter(&self) -> impl Iterator<Item = &SceneElement> {
        self.picked.iter()
    }

    pub fn len(&self) -> usize {
        self.picked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.picked.is_empty()
    }

    pub fn contains(&self, element: &SceneElement) -> bool {
        self.picked.contains(element)
    }

    /// Whether the set is at its limit (a `Finite` limit that's reached; never for `Infinite`).
    pub fn is_full(&self) -> bool {
        !self.limit.has_room(self.picked.len())
    }

    /// Offer an element to the picker. Toggles off if already present; otherwise adds it when the
    /// filter accepts it and there is room, replacing the sole element for a single-select picker.
    pub fn pick(&mut self, element: SceneElement) -> PickOutcome {
        if let Some(pos) = self.picked.iter().position(|e| e == &element) {
            self.picked.remove(pos);
            return PickOutcome::Removed;
        }
        if !self.filter.accepts(&element) {
            return PickOutcome::NotAccepted;
        }
        if self.is_full() {
            if self.limit.is_single() {
                self.picked.clear();
                self.picked.push(element);
                return PickOutcome::Replaced;
            }
            return PickOutcome::Full;
        }
        self.picked.push(element);
        PickOutcome::Added
    }

    /// Remove a specific element if present; returns whether it was there.
    pub fn remove(&mut self, element: &SceneElement) -> bool {
        if let Some(pos) = self.picked.iter().position(|e| e == element) {
            self.picked.remove(pos);
            true
        } else {
            false
        }
    }

    /// Remove the element at a popup-row index (the popup builds rows from [`picked`]).
    pub fn remove_index(&mut self, index: usize) -> Option<SceneElement> {
        (index < self.picked.len()).then(|| self.picked.remove(index))
    }

    pub fn clear(&mut self) {
        self.picked.clear();
    }

    /// Replace the whole picked set (e.g. re-syncing an edit session from a committed operation).
    /// Elements the filter rejects are dropped, and the limit is honored.
    pub fn set_picked(&mut self, elements: impl IntoIterator<Item = SceneElement>) {
        self.picked.clear();
        for element in elements {
            if self.is_full() {
                break;
            }
            if self.filter.accepts(&element) && !self.picked.contains(&element) {
                self.picked.push(element);
            }
        }
    }

    /// The collapsed summary: one `(icon, count)` chip per present kind, in canonical kind order.
    /// This is what the un-expanded control shows, e.g. `2 ⟨line⟩  1 ⟨body⟩`.
    pub fn summary(&self) -> Vec<(IconId, usize)> {
        let mut chips = Vec::new();
        for &kind in ElementKind::ORDER.iter() {
            let count = self
                .picked
                .iter()
                .filter(|e| ElementKind::of(e) == kind)
                .count();
            if count > 0 {
                chips.push((kind.icon(), count));
            }
        }
        chips
    }
}

/// A user interaction with the picker widget in a frame, applied by the caller against the
/// owning [`ElementPicker`] (the widget borrows the picker immutably so the caller keeps
/// control of tool-specific side effects of a removal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// The user clicked the input; it should take focus (and peers should blur).
    Focus,
    /// Remove the picked element at this popup-row index.
    Remove(usize),
    /// Clear the whole set.
    Clear,
}

const ROW_ICON_SIZE: f32 = 14.0;

fn row_icon(ui: &mut egui::Ui, icon: IconId) {
    ui.add(
        egui::Image::new(crate::icons::sized_texture(ui.ctx(), icon))
            .fit_to_exact_size(egui::vec2(ROW_ICON_SIZE, ROW_ICON_SIZE)),
    );
}

/// The shared combo-box rendering (#213) behind both the [`ElementPicker`] widget and the
/// label-only [`show_labeled`] path: a focusable input strip with a `N ⟨icon⟩` collapsed
/// summary and an expandable popup of `⟨icon⟩ label ✕` rows. Fully data-driven so any tool's
/// picked set renders identically.
fn render_combo(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    focused: bool,
    single: bool,
    empty_icons: &[IconId],
    summary: &[(IconId, usize)],
    rows: &[(IconId, String)],
) -> Option<PickerEvent> {
    let mut event = None;
    let ring = if focused {
        egui::Stroke::new(2.0, crate::theme::FOCUS_ACCENT)
    } else {
        egui::Stroke::new(1.0, crate::theme::INPUT_BORDER)
    };

    let frame = egui::Frame::NONE
        .fill(crate::theme::INPUT_BG)
        .stroke(ring)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .corner_radius(egui::CornerRadius::same(3));

    // The whole framed strip is one click target that toggles the popup.
    let inner = frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_width(ui.available_width().max(120.0));
            if rows.is_empty() {
                // Generic empty state (#388): the count ("0", or "0/1" for single-select)
                // plus dimmed icons of what this picker can take.
                let empty_count = if single { "0/1" } else { "0" };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(empty_count)
                            .color(Color32::from_gray(130))
                            .strong(),
                    )
                    .selectable(false),
                );
                for &icon in empty_icons {
                    ui.add(
                        egui::Image::new(crate::icons::sized_texture(ui.ctx(), icon))
                            .fit_to_exact_size(egui::vec2(ROW_ICON_SIZE, ROW_ICON_SIZE))
                            .tint(Color32::from_gray(120)),
                    );
                }
            } else {
                for &(icon, count) in summary {
                    // A single-select picker reads "1/1" (#388); the rest just count.
                    let text = if single { format!("{count}/1") } else { count.to_string() };
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).strong())
                            .selectable(false),
                    );
                    row_icon(ui, icon);
                    ui.add_space(4.0);
                }
            }
            // Right-aligned dropdown caret (painted — the ▾ glyph is missing from the font).
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                let c = rect.center();
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(c.x - 3.0, c.y - 2.0),
                        egui::pos2(c.x + 3.0, c.y - 2.0),
                        egui::pos2(c.x, c.y + 2.5),
                    ],
                    Color32::from_gray(150),
                    egui::Stroke::NONE,
                ));
            });
        });
    });

    // One interactable over the whole strip (click to focus + toggle popup).
    let response = ui
        .interact(inner.response.rect, ui.make_persistent_id(&id_source), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        event = Some(PickerEvent::Focus);
    }

    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(180.0);
            if rows.is_empty() {
                ui.label(egui::RichText::new("Nothing picked yet").weak().italics());
                return;
            }
            for (i, (icon, label)) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    // A muted-red ✕ icon (#256), soft enough not to jar against the dark theme.
                    let remove = ui.add(
                        egui::ImageButton::new(crate::icons::sized_texture(
                            ui.ctx(),
                            crate::icons::IconId::Close,
                        ))
                        .frame(false)
                        .tint(egui::Color32::from_rgb(0xC9, 0x6F, 0x66)),
                    );
                    if remove.on_hover_text("Remove").clicked() {
                        event = Some(PickerEvent::Remove(i));
                    }
                    row_icon(ui, *icon);
                    ui.label(label);
                });
            }
            if rows.len() > 1 {
                ui.separator();
                if ui.small_button("Clear all").clicked() {
                    event = Some(PickerEvent::Clear);
                }
            }
        });

    event
}

/// Render an [`ElementPicker`] as a focusable, combo-box-style input in `ui`.
///
/// Collapsed, it looks like a text input: the "no selection" placeholder when empty, otherwise
/// a `N ⟨icon⟩` chip per present kind. A focused picker draws an accent ring. Clicking opens a
/// popup listing each picked element (icon + label + ✕ remove) with a Clear-all footer.
pub fn show(
    ui: &mut egui::Ui,
    picker: &ElementPicker,
    doc: &Document,
    id_source: impl std::hash::Hash,
) -> Option<PickerEvent> {
    let rows: Vec<(IconId, String)> = picker
        .picked()
        .iter()
        .map(|element| {
            (
                ElementKind::of(element).icon(),
                crate::names::scene_element_label(doc, element),
            )
        })
        .collect();
    render_combo(
        ui,
        id_source,
        picker.is_focused(),
        picker.limit().is_single(),
        &picker.filter().pickable_icons(),
        &picker.summary(),
        &rows,
    )
}

/// Render a label picker (#213/#363) whose rows carry their own icons, for non-[`SceneElement`]
/// sets with mixed item types (e.g. the drawing Select tool's projections/text/dimensions). The
/// collapsed summary counts rows per icon in first-seen order.
pub fn show_rows(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    focused: bool,
    pickable: &[IconId],
    single: bool,
    rows: &[(IconId, String)],
) -> Option<PickerEvent> {
    let mut summary: Vec<(IconId, usize)> = Vec::new();
    for (icon, _) in rows {
        if let Some(entry) = summary.iter_mut().find(|(i, _)| i == icon) {
            entry.1 += 1;
        } else {
            summary.push((*icon, 1));
        }
    }
    render_combo(ui, id_source, focused, single, pickable, &summary, rows)
}

/// Render a label-only picker (#213) with the same combo-box look as [`show`], for tool sets
/// whose items are not [`SceneElement`]s (Chamfer/Fillet edges, Loft sections, Slice cutters,
/// …). All rows share one `icon`; `labels` are the popup rows in order.
pub fn show_labeled(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    focused: bool,
    single: bool,
    icon: IconId,
    labels: &[String],
) -> Option<PickerEvent> {
    let summary = if labels.is_empty() {
        Vec::new()
    } else {
        vec![(icon, labels.len())]
    };
    let rows: Vec<(IconId, String)> = labels.iter().map(|l| (icon, l.clone())).collect();
    render_combo(ui, id_source, focused, single, &[icon], &summary, &rows)
}

/// Apply a widget [`PickerEvent`] to a picker's own state. Focus is handled by the caller (it
/// also needs to blur peer pickers), so `Focus` is a no-op here and returns `false`; `Remove`
/// and `Clear` mutate the set and return `true` so the caller can react (e.g. re-preview).
pub fn apply_event(picker: &mut ElementPicker, event: PickerEvent) -> bool {
    match event {
        PickerEvent::Focus => false,
        PickerEvent::Remove(i) => picker.remove_index(i).is_some(),
        PickerEvent::Clear => {
            let had = !picker.is_empty();
            picker.clear();
            had
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(i: usize) -> SceneElement {
        SceneElement::Body(i)
    }
    fn line(i: usize) -> SceneElement {
        SceneElement::Line(i)
    }

    #[test]
    fn kind_of_covers_operations_and_geometry() {
        assert_eq!(ElementKind::of(&SceneElement::Body(0)), ElementKind::Body);
        assert_eq!(ElementKind::of(&SceneElement::Line(0)), ElementKind::Line);
        assert_eq!(ElementKind::of(&SceneElement::Origin), ElementKind::Vertex);
        assert_eq!(
            ElementKind::of(&SceneElement::BooleanOp(0)),
            ElementKind::Operation
        );
        assert_eq!(
            ElementKind::of(&SceneElement::ConstructionPlane(0)),
            ElementKind::Plane
        );
    }

    fn body_face(body: usize) -> SceneElement {
        SceneElement::BodyFace {
            body,
            centroid: [0, 0, 0],
            normal: [0, 0, 1],
        }
    }

    #[test]
    fn body_face_is_its_own_face_kind() {
        // #566: a flat body face is `Face`, not `Body`, so a planes-or-faces picker can take it
        // without also swallowing whole bodies.
        assert_eq!(ElementKind::of(&body_face(3)), ElementKind::Face);
        assert_eq!(ElementKind::of(&SceneElement::Body(3)), ElementKind::Body);
    }

    #[test]
    fn plane_or_face_filter_takes_planes_and_faces_not_bodies() {
        // The Mirror tool's plane picker (#566): construction planes and flat faces, never a
        // whole body.
        let f = ElementFilter::kinds(&[ElementKind::Plane, ElementKind::Face]);
        assert!(f.accepts(&SceneElement::ConstructionPlane(0)));
        assert!(f.accepts(&body_face(0)));
        assert!(!f.accepts(&SceneElement::Body(0)));
    }

    #[test]
    fn everything_filter_accepts_all() {
        let f = ElementFilter::everything();
        assert!(f.accepts(&body(0)));
        assert!(f.accepts(&SceneElement::Origin));
        assert!(f.accepts(&SceneElement::MoveOp(3)));
        assert!(f.accepts_kind(ElementKind::Constraint));
    }

    #[test]
    fn kind_filter_rejects_other_kinds() {
        let f = ElementFilter::kind(ElementKind::Body);
        assert!(f.accepts(&body(0)));
        assert!(!f.accepts(&line(0)));
        assert!(!f.accepts_kind(ElementKind::Line));
    }

    #[test]
    fn plane_filter_also_accepts_images() {
        let f = ElementFilter::kind(ElementKind::Plane);
        assert!(f.accepts(&SceneElement::ConstructionPlane(0)));
        assert!(f.accepts(&SceneElement::Image(0)));
    }

    #[test]
    fn operation_restriction_filters_by_sub_kind() {
        let f = ElementFilter::kinds(&[ElementKind::Body])
            .operations(&[OperationKind::Boolean, OperationKind::Slice]);
        assert!(f.accepts(&SceneElement::BooleanOp(0)));
        assert!(f.accepts(&SceneElement::SliceOp(0)));
        assert!(!f.accepts(&SceneElement::MoveOp(0)));
        // Body still accepted alongside the operations.
        assert!(f.accepts(&body(0)));
    }

    #[test]
    fn pick_toggles_and_respects_kind() {
        let mut p = ElementPicker::new(ElementFilter::kind(ElementKind::Body), PickLimit::Infinite);
        assert_eq!(p.pick(body(0)), PickOutcome::Added);
        assert_eq!(p.pick(line(0)), PickOutcome::NotAccepted);
        assert_eq!(p.pick(body(1)), PickOutcome::Added);
        assert_eq!(p.len(), 2);
        assert_eq!(p.pick(body(0)), PickOutcome::Removed);
        assert_eq!(p.len(), 1);
        assert!(p.contains(&body(1)));
    }

    #[test]
    fn finite_limit_blocks_when_full() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Finite(2));
        assert_eq!(p.pick(body(0)), PickOutcome::Added);
        assert_eq!(p.pick(body(1)), PickOutcome::Added);
        assert!(p.is_full());
        assert_eq!(p.pick(body(2)), PickOutcome::Full);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn single_select_replaces() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Finite(1));
        assert_eq!(p.pick(body(0)), PickOutcome::Added);
        assert_eq!(p.pick(body(1)), PickOutcome::Replaced);
        assert_eq!(p.picked(), &[body(1)]);
    }

    #[test]
    fn select_everything_picker_is_stuck_focused() {
        let mut p = ElementPicker::select_everything();
        assert!(p.is_focused());
        p.set_focused(false);
        assert!(p.is_focused(), "select-tool picker must not lose focus");
        assert!(p.accepts(&SceneElement::Sketch(0)));
    }

    #[test]
    fn summary_groups_by_kind_in_canonical_order() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        p.pick(body(0));
        p.pick(line(0));
        p.pick(line(1));
        // Canonical order puts lines before bodies.
        let summary = p.summary();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].1, 2, "two lines first");
        assert_eq!(summary[1].1, 1, "one body second");
    }

    #[test]
    fn selected_color_override_wins() {
        let default = Color32::from_rgb(1, 2, 3);
        let red = Color32::from_rgb(200, 0, 0);
        let plain = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        assert_eq!(plain.selected_color(default), default);
        let cutters = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite)
            .with_selected_color(red);
        assert_eq!(cutters.selected_color(default), red);
    }

    #[test]
    fn apply_event_removes_and_clears() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        p.pick(body(0));
        p.pick(line(0));
        assert!(apply_event(&mut p, PickerEvent::Remove(0)));
        assert_eq!(p.picked(), &[line(0)]);
        assert!(!apply_event(&mut p, PickerEvent::Focus));
        assert!(apply_event(&mut p, PickerEvent::Clear));
        assert!(p.is_empty());
    }

    #[test]
    fn set_picked_drops_rejected_and_honors_limit() {
        let mut p = ElementPicker::new(ElementFilter::kind(ElementKind::Body), PickLimit::Finite(2));
        p.set_picked([body(0), line(0), body(1), body(2)]);
        assert_eq!(p.picked(), &[body(0), body(1)]);
    }
}
