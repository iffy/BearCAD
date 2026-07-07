//! Context pane: union of editable properties for the current selection or draw op.

use crate::actions::{ExtrudeBodyMode, Tool};
use crate::document_health::{health_status_label, selection_frozen_summary, DocumentHealth, HealthStatus};
use crate::element_picker::{ElementFilter, ElementKind, ElementPicker, PickLimit};
use crate::geometric_constraints::{constraint_pane_rows, ConstraintPaneRow};
use crate::hierarchy::SceneElement;
use crate::model::{Document, SketchId};
use crate::names::{element_name, single_nameable_from_selection};
use crate::selection::SceneSelection;
use crate::icons::icon_for_constraint;
use crate::shortcuts;
use crate::value::{AngleUnit, LengthUnit};
use eframe::egui::{self, Key, TextEdit};

pub const PANE_TITLE: &str = "Context";

/// Inputs needed to build context pane content (kept separate from [`AppState`] to avoid cycles).
pub struct ContextInput<'a> {
    pub doc: &'a Document,
    pub selection: &'a SceneSelection,
    pub tool: Tool,
    pub draw_rect_construction: Option<bool>,
    pub draw_line_construction: Option<bool>,
    pub draw_circle_construction: Option<bool>,
    /// Curve-mode (`B`) toggle while the line tool is active (#73): the next point drawn gets
    /// bezier handles on both sides (or one, if it's a chain's starting point).
    pub draw_line_curve_mode: Option<bool>,
    /// Tangent-constraint (`T`) toggle while the line tool is active (#73): only meaningful
    /// alongside curve mode.
    pub draw_line_tangent_constraint: Option<bool>,
    /// Whether a sketch is open (snapping only applies inside a sketch).
    pub in_sketch: bool,
    /// Current snapping on/off state (shown as a toggle for snapping tools).
    pub snapping_enabled: bool,
    /// Body an in-progress/edited extrusion would join by default, if any (#32).
    pub extrude_merge_candidate: Option<usize>,
    /// Current new-body/merge-into choice for the in-progress/edited extrusion.
    pub extrude_body_mode: Option<ExtrudeBodyMode>,
    /// Selection-picker rows for the active tool (#157/#167): `Some` whenever the tool
    /// collects a selection set (Chamfer/Fillet outside a sketch — one row per edge in the
    /// in-progress treatment, empty while nothing is picked yet), `None` for other tools.
    pub edge_treatment_rows: Option<Vec<String>>,
    /// Selection-picker rows for the Loft tool (#loft): one row per picked cross section,
    /// `Some` (possibly empty) whenever the Loft tool is active outside a sketch.
    pub loft_rows: Option<Vec<String>>,
    /// Image scale calibration (#171): `Some` when a reference segment is ready — either
    /// both guided calibration points are placed (#163), or the selection is exactly one
    /// tracing image plus one line on the image's host plane.
    pub calibrate_image: Option<CalibrateImageControl>,
    /// Revolve tool state (#revolve): `Some` while the Revolve tool is active.
    pub revolve: Option<RevolveControl>,
    /// Combine tool state: `Some` while the Combine tool is active (creating or editing
    /// a boolean operation).
    pub boolean_op: Option<BooleanControl>,
    /// "Edit operation" entry point: `Some(op)` when exactly one boolean operation is
    /// selected and the Combine tool isn't already active.
    pub boolean_edit_start: Option<usize>,
    /// Move tool state: `Some` while the Move tool is active.
    pub move_op: Option<MoveControl>,
    /// "Edit move" entry point: `Some(op)` when exactly one move operation is selected.
    pub move_edit_start: Option<usize>,
    /// Repeat tool state: `Some` while the Repeat tool is active.
    pub repeat_op: Option<RepeatControl>,
    /// "Edit repeat" entry point.
    pub repeat_edit_start: Option<usize>,
    /// Slice tool state: `Some` while the Slice tool is active.
    pub slice_op: Option<SliceControl>,
    /// "Edit slice" entry point.
    pub slice_edit_start: Option<usize>,
    /// "Edit revolve" entry point (#211): `Some(op)` when exactly one revolution is selected.
    pub revolve_edit_start: Option<usize>,
    /// Guided calibration entry point (#163): `Some(image)` when exactly one tracing image
    /// is selected and no calibration is running — renders the "Calibrate scale" button.
    pub calibrate_start: Option<usize>,
    /// Guided calibration in progress with fewer than two points placed: how many are
    /// placed so far (renders the click-two-points hint).
    pub calibrate_pending: Option<usize>,
}

/// What the Revolve tool's context section shows (#revolve): the picked axis (if any),
/// the symmetric toggle, the body mode, and — in Cut mode — the picked bodies (rendered
/// through the shared selection picker).
#[derive(Clone, Debug, PartialEq)]
pub struct RevolveControl {
    pub face_count: usize,
    pub axis_label: Option<String>,
    pub symmetric: bool,
    pub body_choice: crate::actions::RevolveBodyChoice,
    pub cut_rows: Vec<String>,
}

/// What the Combine tool's context section shows: the operation kind, both picker
/// sides (labels), which side the next viewport click lands on, and the keep-B toggle.
#[derive(Clone, Debug, PartialEq)]
pub struct BooleanControl {
    pub kind: crate::model::BooleanOpKind,
    pub a_rows: Vec<String>,
    pub b_rows: Vec<String>,
    pub picking_b: bool,
    pub keep_b: bool,
    /// `true` while re-editing a committed operation (changes the commit label).
    pub editing: bool,
    pub can_commit: bool,
}

/// What the Move tool's context section shows: the picked bodies, the translation
/// component expressions, the rotation axis + angle expression.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveControl {
    pub target_rows: Vec<String>,
    pub tx: String,
    pub ty: String,
    pub tz: String,
    pub axis_label: Option<String>,
    pub angle: String,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Move context section.
#[derive(Clone, Debug, PartialEq)]
pub enum MoveEdit {
    Tx(String),
    Ty(String),
    Tz(String),
    Angle(String),
    Axis(Option<crate::model::RevolveAxis>),
    RemoveTarget(Option<usize>),
    Commit,
}

/// What the Repeat tool's context section shows.
#[derive(Clone, Debug, PartialEq)]
pub struct RepeatControl {
    pub target_rows: Vec<String>,
    pub axis_label: String,
    pub mode: crate::model::RepeatMode,
    pub count: String,
    pub spacing: String,
    pub length: String,
    /// Live instance count the current configuration produces (`None` = doesn't evaluate).
    pub preview_instances: Option<usize>,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Repeat context section.
#[derive(Clone, Debug, PartialEq)]
pub enum RepeatEdit {
    Mode(crate::model::RepeatMode),
    Axis(crate::model::RevolveAxis),
    Count(String),
    Spacing(String),
    Length(String),
    RemoveTarget(Option<usize>),
    Commit,
}

/// What the Slice tool's context section shows: the picked target bodies, the planar
/// cutters, which picker the next viewport click lands on, and the extend-to-infinity flag.
#[derive(Clone, Debug, PartialEq)]
pub struct SliceControl {
    pub target_rows: Vec<String>,
    pub cutter_rows: Vec<String>,
    /// `true` while the cutter picker is active (the next viewport click adds a cutter).
    pub picking_cutter: bool,
    pub extend_infinite: bool,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Slice context section.
#[derive(Clone, Debug, PartialEq)]
pub enum SliceEdit {
    /// Choose which picker the next viewport click lands on (`true` = cutter).
    PickingCutter(bool),
    ExtendInfinite(bool),
    /// Remove target row `i` (`None` clears the target set).
    RemoveTarget(Option<usize>),
    /// Remove cutter row `i` (`None` clears the cutter set).
    RemoveCutter(Option<usize>),
    Commit,
}

/// One edit from the Combine context section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BooleanEdit {
    Kind(crate::model::BooleanOpKind),
    PickingB(bool),
    KeepB(bool),
    /// Remove row `i` from side A (`None` clears the side).
    RemoveA(Option<usize>),
    /// Remove row `i` from side B (`None` clears the side).
    RemoveB(Option<usize>),
    Commit,
}

/// One edit from the Revolve context section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RevolveEdit {
    Symmetric(bool),
    BodyChoice(crate::actions::RevolveBodyChoice),
}

/// The "Calibrate scale" control's inputs (#171): the target image and the reference
/// segment's plane-local endpoints (a line the user drew over a known image feature).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalibrateImageControl {
    pub image: usize,
    pub a: (f32, f32),
    pub b: (f32, f32),
}

/// One selection-picker row (#167) for a treated edge: the owning extrusion's display name
/// plus the analytic edge's position in its profile.
pub fn edge_treatment_row_label(
    doc: &Document,
    extrusion: usize,
    edge: crate::model::ExtrusionEdgeRef,
) -> String {
    let owner = element_name(doc, SceneElement::Extrusion(extrusion))
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("Extrusion {extrusion}"));
    let which = match edge {
        crate::model::ExtrusionEdgeRef::Vertical { edge, .. } => format!("vertical {edge}"),
        crate::model::ExtrusionEdgeRef::Cap { edge, top: true, .. } => format!("top {edge}"),
        crate::model::ExtrusionEdgeRef::Cap { edge, top: false, .. } => format!("base {edge}"),
    };
    format!("{owner} — {which}")
}

/// One selection-picker row for a loft cross section: the owning sketch's display name
/// plus what kind of profile it is.
pub fn loft_section_row_label(doc: &Document, section: &crate::model::LoftSection) -> String {
    let owner = element_name(doc, SceneElement::Sketch(section.sketch))
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("Sketch {}", section.sketch));
    let which = match &section.face {
        crate::model::ExtrudeFace::Circle(ci) => format!("circle {ci}"),
        crate::model::ExtrudeFace::Polygon(lines) => format!("loop ({} lines)", lines.len()),
        crate::model::ExtrudeFace::Boolean { .. } => "combined region".to_string(),
    };
    format!("{owner} — {which}")
}

/// Tools that snap while drawing or moving sketch geometry.
pub fn tool_uses_snapping(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Select | Tool::Line | Tool::Rectangle | Tool::Circle
    )
}

/// Tri-state value for a property shared by multiple targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriState {
    Off,
    On,
    Mixed,
}

/// What the context pane should display.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextPaneContent {
    pub name: Option<NameControl>,
    /// Curve-mode (`B`) checkbox while the line tool is active (#73).
    pub curve_mode: Option<bool>,
    /// Tangent-constraint (`T`) checkbox while the line tool is active (#73).
    pub tangent_constraint: Option<bool>,
    pub construction: Option<ConstructionControl>,
    pub constraints: Option<Vec<ConstraintPaneRow>>,
    /// `Some(enabled)` when the current tool snaps; renders an enable/disable toggle.
    pub snapping: Option<bool>,
    /// New-body/merge-into choice for an in-progress or edited extrusion (#32).
    pub extrude_body: Option<ExtrudeBodyControl>,
    /// Default length/angle unit picker: document-level when nothing is selected, or
    /// per-sketch (with a "follow document" inherit option) when a single sketch is
    /// selected (#52).
    pub units: Option<UnitsControl>,
    /// Generalized selection picker (#157/#167): the elements the active tool operates on.
    /// Legacy row-list form; being replaced tool-by-tool with [`ContextPaneContent::selection_picker`].
    pub edge_picker: Option<EdgePickerControl>,
    /// The unified element-picker control (#213). Populated for tools already migrated to
    /// [`ElementPicker`] — currently the Select tool's "select everything" picker, which is
    /// always shown (placeholder when empty) and never loses focus.
    pub selection_picker: Option<ElementPicker>,
    /// Image scale calibration (#171).
    pub calibrate_image: Option<CalibrateImageControl>,
    /// Revolve tool controls (#revolve).
    pub revolve: Option<RevolveControl>,
    /// Combine tool controls.
    pub boolean_op: Option<BooleanControl>,
    /// "Edit operation" button target.
    pub boolean_edit_start: Option<usize>,
    /// Move tool state: `Some` while the Move tool is active.
    pub move_op: Option<MoveControl>,
    /// "Edit move" entry point: `Some(op)` when exactly one move operation is selected.
    pub move_edit_start: Option<usize>,
    /// Repeat tool state: `Some` while the Repeat tool is active.
    pub repeat_op: Option<RepeatControl>,
    /// "Edit repeat" entry point.
    pub repeat_edit_start: Option<usize>,
    /// Slice tool controls.
    pub slice_op: Option<SliceControl>,
    /// "Edit slice" button target.
    pub slice_edit_start: Option<usize>,
    /// "Edit revolve" button target (#211).
    pub revolve_edit_start: Option<usize>,
    /// "Calibrate scale" start button (#163): the selected tracing image.
    pub calibrate_start: Option<usize>,
    /// Guided-calibration hint: points placed so far (of 2).
    pub calibrate_pending: Option<usize>,
}

/// The selection-picker input (#157/#167): the picked elements the active tool will operate
/// on, one label per row. Rendered with per-row remove buttons and a clear-all; an empty
/// picker shows a pick hint instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgePickerControl {
    /// Set-count heading, e.g. "Edges" or "Sections".
    pub heading: &'static str,
    /// Hint shown while the set is empty.
    pub hint: &'static str,
    pub rows: Vec<String>,
}

/// What the units picker in the context pane should show and let the user change.
///
/// NOTE (#52 scope): this control only reads/writes the stored default-unit choice. It
/// does not (yet) change how bare numbers are parsed or how any dimension is displayed —
/// see the doc comments on [`crate::model::Document::default_length_unit`] and SPEC §5.3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitsControl {
    /// Sketch this control edits; `None` for the document-level default (nothing selected).
    pub sketch: Option<SketchId>,
    /// Effective length unit: `length_override` if set, else the document default.
    pub effective_length: LengthUnit,
    /// Effective angle unit: `angle_override` if set, else the document default.
    pub effective_angle: AngleUnit,
    /// Explicit per-sketch length override; always `None` for the document-level control.
    pub length_override: Option<LengthUnit>,
    /// Explicit per-sketch angle override; always `None` for the document-level control.
    pub angle_override: Option<AngleUnit>,
    /// Document defaults, used to label the "Follow document" combo entry when `sketch.is_some()`.
    pub document_length: LengthUnit,
    pub document_angle: AngleUnit,
}

/// A user pick from the [`UnitsControl`] combo boxes, to be applied via
/// `Action::SetDocumentUnits` or `Action::SetSketchUnits` (#52).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitsChoice {
    Document { length: LengthUnit, angle: AngleUnit },
    Sketch {
        sketch: SketchId,
        /// `None` means "follow the document default".
        length: Option<LengthUnit>,
        angle: Option<AngleUnit>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtrudeBodyControl {
    pub mode: ExtrudeBodyMode,
    pub merge_body: usize,
    pub merge_body_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameControl {
    pub element: SceneElement,
}

/// Draft text and focus state for the name field in the context pane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextPaneState {
    pub name_draft: String,
    pub focus_name_field: bool,
    pub synced_element: Option<SceneElement>,
    /// Length draft for the image scale calibration control (#171).
    pub calibrate_length_draft: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionControl {
    pub value: TriState,
    pub target_count: usize,
}

/// A user edit from the unified selection element picker (#213): drop one element from the
/// selection, or clear it. Element-based (not row-index-based) so a filtered picker — whose
/// visible rows are a subset of the raw selection — always removes the right element.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectionEdit {
    Remove(SceneElement),
    Clear,
}

/// The selection element picker to show for `tool`, if any — the unified control every
/// selection-driven tool uses. Both variants mirror the live `selection`; they differ only in
/// which kinds they accept and their placeholder, demonstrating the per-instance configuration.
fn selection_picker_for(tool: Tool, selection: &SceneSelection) -> Option<ElementPicker> {
    let mut picker = match tool {
        // Select: accepts everything, always shown, never loses focus.
        Tool::Select => ElementPicker::select_everything(),
        // Constraint: only sketch geometry is constrainable, so restrict the picker to points,
        // lines, circles, and body/face edges (bodies, planes, operations are rejected).
        Tool::Constraint => {
            let mut p = ElementPicker::new(
                ElementFilter::kinds(&[
                    ElementKind::Vertex,
                    ElementKind::Line,
                    ElementKind::Circle,
                    ElementKind::Edge,
                ]),
                PickLimit::Infinite,
            )
            .with_placeholder("Pick geometry to constrain");
            p.set_focused(true);
            p
        }
        _ => return None,
    };
    // Mirror the live selection, keeping only what this picker accepts (its filter drops the
    // rest); `set_picked` preserves order so the popup rows line up with `picked()`.
    picker.set_picked(selection.ordered());
    Some(picker)
}

pub fn context_pane_content(input: &ContextInput<'_>) -> ContextPaneContent {
    let name = single_nameable_from_selection(input.selection).map(|element| NameControl { element });
    let snapping =
        (input.in_sketch && tool_uses_snapping(input.tool)).then_some(input.snapping_enabled);
    let extrude_body = match (input.extrude_merge_candidate, input.extrude_body_mode) {
        (Some(bi), Some(mode)) => Some(ExtrudeBodyControl {
            mode,
            merge_body: bi,
            merge_body_label: element_name(input.doc, SceneElement::Body(bi))
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("Body {bi}")),
        }),
        _ => None,
    };
    let units = units_control_from_selection(input.doc, input.selection);
    let edge_picker = input
        .edge_treatment_rows
        .clone()
        .map(|rows| EdgePickerControl {
            heading: "Edges",
            hint: "Click an edge — Shift+click adds more",
            rows,
        })
        .or_else(|| {
            input.loft_rows.clone().map(|rows| EdgePickerControl {
                heading: "Sections",
                hint: "Click a closed profile (circle or loop)",
                rows,
            })
        })
        .or_else(|| {
            input.revolve.as_ref().and_then(|r| {
                (r.body_choice == crate::actions::RevolveBodyChoice::Cut).then(|| {
                    EdgePickerControl {
                        heading: "Cut bodies",
                        hint: "Click a body to cut",
                        rows: r.cut_rows.clone(),
                    }
                })
            })
        });
    // The unified selection element picker (#213), mirroring the live selection for the tools
    // that operate on it. Suppressed while a draw construction owns the pane.
    let drawing = input.draw_rect_construction.is_some()
        || input.draw_line_construction.is_some()
        || input.draw_circle_construction.is_some();
    let selection_picker = (!drawing)
        .then(|| selection_picker_for(input.tool, input.selection))
        .flatten();
    let calibrate_image = input.calibrate_image;
    let revolve = input.revolve.clone();
    let boolean_op = input.boolean_op.clone();
    let boolean_edit_start = input.boolean_edit_start;
    let move_op = input.move_op.clone();
    let move_edit_start = input.move_edit_start;
    let repeat_op = input.repeat_op.clone();
    let repeat_edit_start = input.repeat_edit_start;
    let slice_op = input.slice_op.clone();
    let slice_edit_start = input.slice_edit_start;
    let revolve_edit_start = input.revolve_edit_start;
    let calibrate_start = input.calibrate_start;
    let calibrate_pending = input.calibrate_pending;

    if let Some(construction) = input.draw_rect_construction {
        return ContextPaneContent {
            name,
            curve_mode: None,
            tangent_constraint: None,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            snapping,
            extrude_body,
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            calibrate_image,
            revolve: revolve.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            repeat_op: repeat_op.clone(),
            repeat_edit_start,
            slice_op: slice_op.clone(),
            slice_edit_start,
            revolve_edit_start,
        calibrate_start,
            calibrate_pending,
        };
    }
    if let Some(construction) = input.draw_line_construction {
        return ContextPaneContent {
            name,
            curve_mode: input.draw_line_curve_mode,
            tangent_constraint: input.draw_line_tangent_constraint,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            snapping,
            extrude_body,
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            calibrate_image,
            revolve: revolve.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            repeat_op: repeat_op.clone(),
            repeat_edit_start,
            slice_op: slice_op.clone(),
            slice_edit_start,
            revolve_edit_start,
        calibrate_start,
            calibrate_pending,
        };
    }
    if let Some(construction) = input.draw_circle_construction {
        return ContextPaneContent {
            name,
            curve_mode: None,
            tangent_constraint: None,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            snapping,
            extrude_body,
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            calibrate_image,
            revolve: revolve.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            repeat_op: repeat_op.clone(),
            repeat_edit_start,
            slice_op: slice_op.clone(),
            slice_edit_start,
            revolve_edit_start,
        calibrate_start,
            calibrate_pending,
        };
    }

    let targets = construction_targets_from_selection(input.selection);
    let constraints = (input.tool == Tool::Constraint)
        .then(|| constraint_pane_rows(input.selection));
    ContextPaneContent {
        name,
        curve_mode: None,
        tangent_constraint: None,
        construction: (!targets.is_empty()).then(|| ConstructionControl {
            value: construction_tri_state(input.doc, &targets),
            target_count: targets.len(),
        }),
        constraints,
        snapping,
        extrude_body,
        units,
        edge_picker,
        selection_picker,
        calibrate_image,
        revolve,
        boolean_op,
        boolean_edit_start,
        move_op,
        move_edit_start,
        repeat_op,
        repeat_edit_start,
        slice_op,
        slice_edit_start,
        revolve_edit_start,
        calibrate_start,
        calibrate_pending,
    }
}

/// Build the units picker for the current selection: document-level when nothing is
/// selected, per-sketch (with an inherit option) when a single sketch is selected, and
/// hidden (`None`) for any other selection (#52).
fn units_control_from_selection(doc: &Document, selection: &SceneSelection) -> Option<UnitsControl> {
    if selection.is_empty() {
        return Some(UnitsControl {
            sketch: None,
            effective_length: doc.default_length_unit,
            effective_angle: doc.default_angle_unit,
            length_override: None,
            angle_override: None,
            document_length: doc.default_length_unit,
            document_angle: doc.default_angle_unit,
        });
    }
    let Some(SceneElement::Sketch(id)) = selection.single() else {
        return None;
    };
    let sketch = doc.sketches.get(id)?;
    Some(UnitsControl {
        sketch: Some(id),
        effective_length: crate::model::effective_length_unit(doc, id),
        effective_angle: crate::model::effective_angle_unit(doc, id),
        length_override: sketch.length_unit,
        angle_override: sketch.angle_unit,
        document_length: doc.default_length_unit,
        document_angle: doc.default_angle_unit,
    })
}

pub fn sync_name_draft(
    state: &mut ContextPaneState,
    doc: &Document,
    content: &ContextPaneContent,
) {
    let Some(control) = &content.name else {
        state.synced_element = None;
        return;
    };
    if state.synced_element == Some(control.element.clone()) {
        return;
    }
    state.synced_element = Some(control.element.clone());
    state.name_draft = element_name(doc, control.element.clone())
        .unwrap_or_default()
        .to_string();
}

pub fn construction_targets_from_selection(selection: &SceneSelection) -> Vec<SceneElement> {
    let mut targets = Vec::new();
    for element in selection.iter() {
        match element {
            SceneElement::Line(_) | SceneElement::Circle(_) => targets.push(element),
            _ => {}
        }
    }
    targets.sort_by_key(|element| scene_element_sort_key(element.clone()));
    targets.dedup();
    targets
}

fn scene_element_sort_key(element: SceneElement) -> (u8, usize, u8) {
    match element {
        SceneElement::Line(i) => (0, i, 0),
        SceneElement::Circle(i) => (1, i, 0),
        _ => (2, 0, 0),
    }
}

pub fn edge_construction_for_element(doc: &Document, element: SceneElement) -> Option<bool> {
    match element {
        SceneElement::Line(index) => doc.lines.get(index).map(|line| line.construction),
        SceneElement::Circle(index) => doc.circles.get(index).map(|circle| circle.construction),
        _ => None,
    }
}

/// Whether a selected line, edge, or curve uses dashed (construction) highlighting.
pub fn selection_highlight_dashed(doc: &Document, element: SceneElement) -> Option<bool> {
    edge_construction_for_element(doc, element)
}

pub fn construction_tri_state(doc: &Document, targets: &[SceneElement]) -> TriState {
    let mut any_on = false;
    let mut any_off = false;
    for element in targets {
        let Some(value) = edge_construction_for_element(doc, element.clone()) else {
            continue;
        };
        if value {
            any_on = true;
        } else {
            any_off = true;
        }
    }
    tri_state_from_flags(any_on, any_off)
}

fn tri_state_from_bool(value: bool) -> TriState {
    if value {
        TriState::On
    } else {
        TriState::Off
    }
}

fn tri_state_from_flags(any_on: bool, any_off: bool) -> TriState {
    match (any_on, any_off) {
        (true, false) => TriState::On,
        (false, true) => TriState::Off,
        (true, true) => TriState::Mixed,
        (false, false) => TriState::Off,
    }
}

pub fn set_edge_construction(
    doc: &mut Document,
    element: SceneElement,
    construction: bool,
) -> Result<(), String> {
    match element {
        SceneElement::Line(index) => {
            let line = doc
                .lines
                .get_mut(index)
                .ok_or_else(|| format!("Line {index} not found"))?;
            line.construction = construction;
            Ok(())
        }
        SceneElement::Circle(index) => {
            let circle = doc
                .circles
                .get_mut(index)
                .ok_or_else(|| format!("Circle {index} not found"))?;
            circle.construction = construction;
            Ok(())
        }
        _ => Err("Only lines, circles, and rectangle edges support construction mode".to_string()),
    }
}

pub fn set_construction_for_targets(
    doc: &mut Document,
    targets: &[SceneElement],
    construction: bool,
) -> Result<usize, String> {
    let mut updated = 0usize;
    for element in targets {
        set_edge_construction(doc, element.clone(), construction)?;
        updated += 1;
    }
    Ok(updated)
}

pub fn toggle_construction_for_targets(
    doc: &mut Document,
    targets: &[SceneElement],
) -> Result<usize, String> {
    let mut updated = 0usize;
    for element in targets {
        let Some(current) = edge_construction_for_element(doc, element.clone()) else {
            continue;
        };
        set_edge_construction(doc, element.clone(), !current)?;
        updated += 1;
    }
    Ok(updated)
}

/// One row of the extrude "into" picker (#32/#35): the mode's icon followed by a radio button.
/// Selecting the radio mutates `current`, which the caller diffs to fire the change callback.
fn extrude_body_mode_row(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    current: &mut ExtrudeBodyMode,
    value: ExtrudeBodyMode,
    icon: crate::icons::IconId,
    label: String,
) {
    ui.horizontal(|ui| {
        ui.image(crate::icons::sized_texture(ctx, icon));
        ui.radio_value(current, value, label);
    });
}

pub fn show_pane(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    content: &ContextPaneContent,
    pane_state: &mut ContextPaneState,
    health: &DocumentHealth,
    selection: &SceneSelection,
    doc: &Document,
    on_name_committed: &mut impl FnMut(SceneElement, String),
    on_curve_mode_changed: &mut impl FnMut(bool),
    on_tangent_constraint_changed: &mut impl FnMut(bool),
    on_construction_changed: &mut impl FnMut(bool),
    on_constraint_clicked: &mut impl FnMut(crate::geometric_constraints::GeometricConstraintType),
    on_snapping_changed: &mut impl FnMut(bool),
    on_extrude_body_mode_changed: &mut impl FnMut(ExtrudeBodyMode),
    on_units_changed: &mut impl FnMut(UnitsChoice),
    on_edge_picker_edit: &mut impl FnMut(Option<usize>),
    on_selection_edit: &mut impl FnMut(SelectionEdit),
    on_revolve_edit: &mut impl FnMut(RevolveEdit),
    on_boolean_edit: &mut impl FnMut(BooleanEdit),
    on_boolean_edit_start: &mut impl FnMut(usize),
    on_move_edit: &mut impl FnMut(MoveEdit),
    on_move_edit_start: &mut impl FnMut(usize),
    on_repeat_edit: &mut impl FnMut(RepeatEdit),
    on_repeat_edit_start: &mut impl FnMut(usize),
    on_slice_edit: &mut impl FnMut(SliceEdit),
    on_slice_edit_start: &mut impl FnMut(usize),
    on_revolve_edit_start: &mut impl FnMut(usize),
    on_calibrate_start: &mut impl FnMut(usize),
    on_calibrate_image: &mut impl FnMut(CalibrateImageControl, String),
) {
    ui.heading(PANE_TITLE);
    ui.separator();

    let frozen = selection_frozen_summary(health, selection);
    if let Some((status, reason)) = &frozen {
        let color = match status {
            HealthStatus::Invalid => egui::Color32::from_rgb(220, 80, 80),
            HealthStatus::Unstable => egui::Color32::from_rgb(255, 180, 60),
            HealthStatus::Healthy => egui::Color32::from_gray(140),
        };
        ui.label(
            egui::RichText::new(format!(
                "{} — editing frozen",
                health_status_label(*status).to_uppercase()
            ))
            .color(color)
            .strong(),
        );
        ui.label(
            egui::RichText::new(reason.as_str())
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
        ui.add_space(4.0);
    }

    let controls_enabled = frozen.is_none();
    let mut any_control = false;
    // Keep children from widening the side panel via egui's persisted PanelState.
    ui.set_width(ui.available_width());

    if let Some(control) = &content.name {
        any_control = true;
        ui.label(shortcuts::compact_label("Name", Some(shortcuts::FOCUS_ELEMENT_NAME)));
        let id = egui::Id::new(("element_name", control.element.clone()));
        let mut committed = false;
        ui.add_enabled_ui(controls_enabled, |ui| {
            let output = TextEdit::singleline(&mut pane_state.name_draft)
                .id(id)
                .desired_width(f32::INFINITY)
                .show(ui);
            let response = &output.response;
            let should_select_all = pane_state.focus_name_field;
            if should_select_all {
                response.request_focus();
            }
            if (should_select_all && response.has_focus()) || response.gained_focus() {
                let len = pane_state.name_draft.chars().count();
                let mut state = output.state;
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::default(),
                    egui::text::CCursor::new(len),
                )));
                state.store(ctx, id);
                pane_state.focus_name_field = false;
            }
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if (enter && response.has_focus()) || response.lost_focus() {
                committed = true;
                if enter && response.has_focus() {
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
                }
            }
        });
        if committed {
            on_name_committed(control.element.clone(), pane_state.name_draft.clone());
        }
        ui.add_space(4.0);
    }

    if let Some(rows) = &content.constraints {
        any_control = true;
        ui.label("Constraints");
        for row in rows {
            ui.horizontal(|ui| {
                let enabled = controls_enabled && row.enabled;
                shortcuts::show_constraint_shortcut_left(
                    ui,
                    shortcuts::geometric_constraint_shortcut(row.kind),
                    enabled,
                );
                let response = ui
                    .add_enabled(
                        enabled,
                        egui::ImageButton::new(crate::icons::sized_texture(
                            ui.ctx(),
                            icon_for_constraint(row.kind),
                        ))
                        .frame(true),
                    )
                    .on_hover_text(row.kind.label());
                if enabled && response.clicked() {
                    on_constraint_clicked(row.kind);
                }
                if !row.enabled && !row.missing.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("needs {}", row.missing.join(", ")))
                            .color(egui::Color32::from_gray(140))
                            .size(11.0),
                    );
                }
            });
        }
        ui.add_space(4.0);
    }

    if let Some(mut curve_mode) = content.curve_mode {
        any_control = true;
        ui.add_enabled_ui(controls_enabled, |ui| {
            if shortcuts::checkbox_with_shortcut(
                ui,
                &mut curve_mode,
                "Curve",
                Some(shortcuts::TOGGLE_CURVE_MODE),
            )
            .changed()
            {
                on_curve_mode_changed(curve_mode);
            }
        });
    }

    if let Some(mut tangent_constraint) = content.tangent_constraint {
        any_control = true;
        ui.add_enabled_ui(controls_enabled, |ui| {
            if shortcuts::checkbox_with_shortcut(
                ui,
                &mut tangent_constraint,
                "Tangent",
                Some(shortcuts::TOGGLE_TANGENT_CONSTRAINT),
            )
            .changed()
            {
                on_tangent_constraint_changed(tangent_constraint);
            }
        });
        ui.add_space(4.0);
    }

    if let Some(control) = &content.construction {
        any_control = true;
        let label = match control.value {
            TriState::Mixed => "Construction (mixed)",
            _ => "Construction",
        };
        let mut checked = control.value == TriState::On;
        ui.add_enabled_ui(controls_enabled, |ui| {
            if shortcuts::checkbox_with_shortcut(
                ui,
                &mut checked,
                label,
                Some(shortcuts::TOGGLE_CONSTRUCTION),
            )
            .changed()
            {
                on_construction_changed(checked);
            }
        });
        if control.target_count > 1 {
            ui.label(
                egui::RichText::new(format!("{} items", control.target_count))
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
        }
    }

    if let Some(enabled) = content.snapping {
        any_control = true;
        let mut checked = enabled;
        if ui.checkbox(&mut checked, "Snapping").changed() {
            on_snapping_changed(checked);
        }
        ui.label(
            egui::RichText::new("Snap to vertices, midpoints, and lines while drawing or moving")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(control) = &content.revolve {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Revolve").strong());
        ui.label(
            egui::RichText::new(match &control.axis_label {
                Some(label) => format!("{} face(s) around {}", control.face_count, label),
                None => format!("{} face(s) — click an axis line", control.face_count),
            })
            .color(egui::Color32::from_gray(140))
            .size(11.0),
        );
        let mut symmetric = control.symmetric;
        if ui.checkbox(&mut symmetric, "Symmetric").changed() {
            on_revolve_edit(RevolveEdit::Symmetric(symmetric));
        }
        let mut choice = control.body_choice;
        for (value, label) in [
            (crate::actions::RevolveBodyChoice::NewBody, "New body"),
            (crate::actions::RevolveBodyChoice::AddTouching, "Add to touching bodies"),
            (crate::actions::RevolveBodyChoice::Cut, "Cut bodies"),
        ] {
            if ui.radio_value(&mut choice, value, label).changed() {
                on_revolve_edit(RevolveEdit::BodyChoice(choice));
            }
        }
    }

    if let Some(control) = &content.boolean_op {
        any_control = true;
        ui.separator();
        ui.label(
            egui::RichText::new(if control.editing {
                "Edit boolean operation"
            } else {
                "Combine"
            })
            .strong(),
        );
        let mut kind = control.kind;
        for value in [
            crate::model::BooleanOpKind::Combine,
            crate::model::BooleanOpKind::Cut,
            crate::model::BooleanOpKind::Intersect,
            crate::model::BooleanOpKind::Difference,
        ] {
            if ui.radio_value(&mut kind, value, value.label()).changed() {
                on_boolean_edit(BooleanEdit::Kind(kind));
            }
        }
        let two_sided = control.kind != crate::model::BooleanOpKind::Combine;
        if two_sided {
            ui.horizontal(|ui| {
                ui.label("Picking");
                if ui.selectable_label(!control.picking_b, "A").clicked() {
                    on_boolean_edit(BooleanEdit::PickingB(false));
                }
                if ui.selectable_label(control.picking_b, "B").clicked() {
                    on_boolean_edit(BooleanEdit::PickingB(true));
                }
            });
        }
        let side = |ui: &mut egui::Ui,
                    heading: String,
                    rows: &[String],
                    hint: &str,
                    remove: &mut dyn FnMut(Option<usize>)| {
            ui.label(egui::RichText::new(heading).strong());
            if rows.is_empty() {
                ui.label(
                    egui::RichText::new(hint)
                        .color(egui::Color32::from_gray(140))
                        .size(11.0),
                );
            }
            for (i, row) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("✕").on_hover_text("Remove from set").clicked() {
                        remove(Some(i));
                    }
                    ui.label(row);
                });
            }
            if rows.len() > 1 && ui.small_button("Clear").clicked() {
                remove(None);
            }
        };
        side(
            ui,
            format!(
                "{} ({})",
                if two_sided { "Side A" } else { "Bodies" },
                control.a_rows.len()
            ),
            &control.a_rows,
            "Click bodies in the viewport",
            &mut |i| on_boolean_edit(BooleanEdit::RemoveA(i)),
        );
        if two_sided {
            side(
                ui,
                format!("Side B ({})", control.b_rows.len()),
                &control.b_rows,
                "Switch Picking to B, then click bodies",
                &mut |i| on_boolean_edit(BooleanEdit::RemoveB(i)),
            );
            let mut keep_b = control.keep_b;
            if ui
                .checkbox(&mut keep_b, "Keep B bodies")
                .on_hover_text("Leave the B-side inputs as real bodies instead of shadows")
                .changed()
            {
                on_boolean_edit(BooleanEdit::KeepB(keep_b));
            }
        }
        ui.add_space(2.0);
        if ui
            .add_enabled(
                control.can_commit && controls_enabled,
                egui::Button::new(if control.editing { "Apply changes" } else { "Create" }),
            )
            .clicked()
        {
            on_boolean_edit(BooleanEdit::Commit);
        }
        ui.label(
            egui::RichText::new("Inputs become shadow bodies; the result is one or more new bodies")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(op) = content.boolean_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit operation").clicked() {
            on_boolean_edit_start(op);
        }
        ui.label(
            egui::RichText::new("Re-open the pickers to change this boolean operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(control) = &content.move_op {
        any_control = true;
        ui.separator();
        ui.label(
            egui::RichText::new(if control.editing { "Edit move" } else { "Move" }).strong(),
        );
        ui.label(
            egui::RichText::new(format!("Bodies ({})", control.target_rows.len())).strong(),
        );
        if control.target_rows.is_empty() {
            ui.label(
                egui::RichText::new("Click bodies in the viewport")
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
        }
        for (i, row) in control.target_rows.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✕").on_hover_text("Remove from set").clicked() {
                    on_move_edit(MoveEdit::RemoveTarget(Some(i)));
                }
                ui.label(row);
            });
        }
        let mut pending: Option<MoveEdit> = None;
        {
            let mut field = |ui: &mut egui::Ui,
                             label: &str,
                             value: &str,
                             make: &dyn Fn(String) -> MoveEdit| {
                ui.horizontal(|ui| {
                    ui.label(label);
                    let mut text = value.to_string();
                    let resp =
                        ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
                    if resp.changed() {
                        pending = Some(make(text));
                    }
                });
            };
            field(ui, "X", &control.tx, &MoveEdit::Tx);
            field(ui, "Y", &control.ty, &MoveEdit::Ty);
            field(ui, "Z", &control.tz, &MoveEdit::Tz);
            field(ui, "Angle", &control.angle, &MoveEdit::Angle);
        }
        ui.horizontal(|ui| {
            ui.label("Axis");
            for (axis, label) in [
                (crate::model::RevolveAxis::X, "X"),
                (crate::model::RevolveAxis::Y, "Y"),
                (crate::model::RevolveAxis::Z, "Z"),
            ] {
                if ui.small_button(label).clicked() {
                    pending = Some(MoveEdit::Axis(Some(axis)));
                }
            }
            if ui.small_button("None").clicked() {
                pending = Some(MoveEdit::Axis(None));
            }
        });
        ui.label(
            egui::RichText::new(match &control.axis_label {
                Some(label) => format!("Rotating around {label} — or click a line"),
                None => "No rotation — pick an axis or click a line".to_string(),
            })
            .color(egui::Color32::from_gray(140))
            .size(11.0),
        );
        if let Some(edit) = pending {
            on_move_edit(edit);
        }
        ui.add_space(2.0);
        if ui
            .add_enabled(
                control.can_commit && controls_enabled,
                egui::Button::new(if control.editing { "Apply changes" } else { "Move" }),
            )
            .clicked()
        {
            on_move_edit(MoveEdit::Commit);
        }
        ui.label(
            egui::RichText::new("Inputs become shadow bodies; the moved copies are new bodies")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(op) = content.move_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit move").clicked() {
            on_move_edit_start(op);
        }
        ui.label(
            egui::RichText::new("Re-open the Move tool to change this operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(control) = &content.repeat_op {
        any_control = true;
        ui.separator();
        ui.label(
            egui::RichText::new(if control.editing { "Edit repeat" } else { "Linear repeat" })
                .strong(),
        );
        ui.label(
            egui::RichText::new(format!("Bodies ({})", control.target_rows.len())).strong(),
        );
        if control.target_rows.is_empty() {
            ui.label(
                egui::RichText::new("Click bodies in the viewport")
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
        }
        for (i, row) in control.target_rows.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✕").on_hover_text("Remove from set").clicked() {
                    on_repeat_edit(RepeatEdit::RemoveTarget(Some(i)));
                }
                ui.label(row);
            });
        }
        let mut pending: Option<RepeatEdit> = None;
        ui.horizontal(|ui| {
            ui.label("Axis");
            for (axis, label) in [
                (crate::model::RevolveAxis::X, "X"),
                (crate::model::RevolveAxis::Y, "Y"),
                (crate::model::RevolveAxis::Z, "Z"),
            ] {
                if ui.small_button(label).clicked() {
                    pending = Some(RepeatEdit::Axis(axis));
                }
            }
        });
        ui.label(
            egui::RichText::new(format!("Along {} — or click a line", control.axis_label))
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
        let mut mode = control.mode;
        for value in [
            crate::model::RepeatMode::CountGap,
            crate::model::RepeatMode::CountFitEnds,
            crate::model::RepeatMode::CountFitCenters,
            crate::model::RepeatMode::FillGap,
            crate::model::RepeatMode::FillPitch,
            crate::model::RepeatMode::FillMaxPitch,
        ] {
            if ui.radio_value(&mut mode, value, value.label()).changed() {
                pending = Some(RepeatEdit::Mode(mode));
            }
        }
        {
            let mut field = |ui: &mut egui::Ui,
                             label: &str,
                             value: &str,
                             enabled: bool,
                             make: &dyn Fn(String) -> RepeatEdit| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let mut text = value.to_string();
                        let resp =
                            ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
                        if resp.changed() {
                            pending = Some(make(text));
                        }
                    });
                });
            };
            field(ui, "Count", &control.count, control.mode.uses_count(), &RepeatEdit::Count);
            field(ui, "Spacing", &control.spacing, !matches!(control.mode, crate::model::RepeatMode::CountFitEnds | crate::model::RepeatMode::CountFitCenters), &RepeatEdit::Spacing);
            field(ui, "Length", &control.length, control.mode.uses_length(), &RepeatEdit::Length);
        }
        ui.label(
            egui::RichText::new(match control.preview_instances {
                Some(n) => format!("{n} instances"),
                None => "Configuration doesn't evaluate yet".to_string(),
            })
            .color(egui::Color32::from_gray(140))
            .size(11.0),
        );
        if let Some(edit) = pending {
            on_repeat_edit(edit);
        }
        ui.add_space(2.0);
        if ui
            .add_enabled(
                control.can_commit && controls_enabled,
                egui::Button::new(if control.editing { "Apply changes" } else { "Repeat" }),
            )
            .clicked()
        {
            on_repeat_edit(RepeatEdit::Commit);
        }
    }

    if let Some(op) = content.repeat_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit repeat").clicked() {
            on_repeat_edit_start(op);
        }
        ui.label(
            egui::RichText::new("Re-open the Repeat tool to change this operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(control) = &content.slice_op {
        any_control = true;
        ui.separator();
        ui.label(
            egui::RichText::new(if control.editing { "Edit slice" } else { "Slice" }).strong(),
        );
        let mut pending: Option<SliceEdit> = None;
        // Which picker the next viewport click lands on.
        ui.horizontal(|ui| {
            ui.label("Picking");
            let mut picking_cutter = control.picking_cutter;
            if ui
                .selectable_label(!picking_cutter, "Bodies")
                .clicked()
            {
                picking_cutter = false;
            }
            if ui.selectable_label(picking_cutter, "Cutters").clicked() {
                picking_cutter = true;
            }
            if picking_cutter != control.picking_cutter {
                pending = Some(SliceEdit::PickingCutter(picking_cutter));
            }
        });
        ui.label(
            egui::RichText::new(format!("Bodies ({})", control.target_rows.len())).strong(),
        );
        if control.target_rows.is_empty() {
            ui.label(
                egui::RichText::new("Click bodies in the viewport")
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
        }
        for (i, row) in control.target_rows.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✕").on_hover_text("Remove from set").clicked() {
                    pending = Some(SliceEdit::RemoveTarget(Some(i)));
                }
                ui.label(row);
            });
        }
        ui.label(
            egui::RichText::new(format!("Cutters ({})", control.cutter_rows.len())).strong(),
        );
        if control.cutter_rows.is_empty() {
            ui.label(
                egui::RichText::new("Switch to Cutters, then click planes or faces")
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
        }
        for (i, row) in control.cutter_rows.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✕").on_hover_text("Remove cutter").clicked() {
                    pending = Some(SliceEdit::RemoveCutter(Some(i)));
                }
                ui.label(row);
            });
        }
        let mut extend = control.extend_infinite;
        if ui
            .checkbox(&mut extend, "Extend cutters to infinity")
            .on_hover_text("Off: a cutter only separates material within its own footprint")
            .changed()
        {
            pending = Some(SliceEdit::ExtendInfinite(extend));
        }
        if let Some(edit) = pending {
            on_slice_edit(edit);
        }
        ui.add_space(2.0);
        if ui
            .add_enabled(
                control.can_commit && controls_enabled,
                egui::Button::new(if control.editing { "Apply changes" } else { "Slice" }),
            )
            .clicked()
        {
            on_slice_edit(SliceEdit::Commit);
        }
    }

    if let Some(op) = content.slice_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit slice").clicked() {
            on_slice_edit_start(op);
        }
        ui.label(
            egui::RichText::new("Re-open the Slice tool to change this operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(op) = content.revolve_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit revolve").clicked() {
            on_revolve_edit_start(op);
        }
        ui.label(
            egui::RichText::new("Re-open the Revolve tool to change this operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(image) = content.calibrate_start {
        any_control = true;
        ui.separator();
        if ui.button("Calibrate scale").clicked() {
            on_calibrate_start(image);
        }
        ui.label(
            egui::RichText::new("Set the image's real-world scale from a feature of known size")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(placed) = content.calibrate_pending {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Calibrate scale").strong());
        ui.label(
            egui::RichText::new(format!(
                "Click two points on the image over a feature of known size ({placed} of 2 placed)"
            ))
            .color(egui::Color32::from_gray(140))
            .size(11.0),
        );
    }

    if let Some(control) = content.calibrate_image {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Calibrate scale").strong());
        ui.label(
            egui::RichText::new("Real length of the marked span on the image")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut pane_state.calibrate_length_draft)
                    .desired_width(80.0)
                    .hint_text("50mm"),
            );
            if ui.button("Apply").clicked()
                && !pane_state.calibrate_length_draft.trim().is_empty()
            {
                on_calibrate_image(control, pane_state.calibrate_length_draft.clone());
            }
        });
    }

    if let Some(picker) = &content.selection_picker {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Selection").strong());
        ui.add_enabled_ui(controls_enabled, |ui| {
            if let Some(event) = crate::element_picker::show(ui, picker, doc, "selection_picker") {
                match event {
                    // A sticky-focused (Select) picker ignores focus; others take it on click.
                    crate::element_picker::PickerEvent::Focus => {}
                    crate::element_picker::PickerEvent::Remove(i) => {
                        if let Some(element) = picker.picked().get(i).cloned() {
                            on_selection_edit(SelectionEdit::Remove(element));
                        }
                    }
                    crate::element_picker::PickerEvent::Clear => {
                        on_selection_edit(SelectionEdit::Clear)
                    }
                }
            }
        });
    }

    if let Some(picker) = &content.edge_picker {
        any_control = true;
        ui.separator();
        ui.label(
            egui::RichText::new(format!("{} ({})", picker.heading, picker.rows.len())).strong(),
        );
        if picker.rows.is_empty() {
            ui.label(
                egui::RichText::new(picker.hint)
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
        }
        for (i, row) in picker.rows.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✕").on_hover_text("Remove from set").clicked() {
                    on_edge_picker_edit(Some(i));
                }
                ui.label(row);
            });
        }
        if picker.rows.len() > 1 && ui.small_button("Clear all").clicked() {
            on_edge_picker_edit(None);
        }
    }

    if let Some(control) = &content.extrude_body {
        any_control = true;
        ui.label("Extrude into");
        let mut mode = control.mode;
        ui.add_enabled_ui(controls_enabled, |ui| {
            extrude_body_mode_row(
                ui,
                ctx,
                &mut mode,
                ExtrudeBodyMode::MergeInto(control.merge_body),
                crate::icons::IconId::AddToBody,
                format!("Add to {}", control.merge_body_label),
            );
            extrude_body_mode_row(
                ui,
                ctx,
                &mut mode,
                ExtrudeBodyMode::NewBody,
                crate::icons::IconId::NewBody,
                "New body".to_string(),
            );
            // A cut needs the kernel to subtract solids; a non-`occt` build can't perform it,
            // so it isn't offered (avoids a dead control). See `body_solid_mesh` (#35).
            if cfg!(feature = "occt") {
                extrude_body_mode_row(
                    ui,
                    ctx,
                    &mut mode,
                    ExtrudeBodyMode::Cut(control.merge_body),
                    crate::icons::IconId::CutBody,
                    format!("Cut {}", control.merge_body_label),
                );
            }
        });
        if mode != control.mode {
            on_extrude_body_mode_changed(mode);
        }
        ui.add_space(4.0);
    }

    if let Some(control) = &content.units {
        any_control = true;
        ui.label(if control.sketch.is_some() {
            "Sketch units"
        } else {
            "Default units"
        });
        ui.add_enabled_ui(controls_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Length");
                let follow_document_label =
                    format!("Follow document ({})", control.document_length.label());
                let selected_text = match (control.sketch, control.length_override) {
                    (Some(_), None) => follow_document_label.clone(),
                    _ => control.effective_length.label().to_string(),
                };
                egui::ComboBox::from_id_salt("context_length_unit")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        if let Some(sketch) = control.sketch {
                            if ui
                                .selectable_label(control.length_override.is_none(), follow_document_label)
                                .clicked()
                            {
                                on_units_changed(UnitsChoice::Sketch {
                                    sketch,
                                    length: None,
                                    angle: control.angle_override,
                                });
                            }
                        }
                        for unit in LengthUnit::ALL {
                            let selected = control.length_override == Some(unit)
                                || (control.sketch.is_none() && control.effective_length == unit);
                            if ui.selectable_label(selected, unit.label()).clicked() {
                                match control.sketch {
                                    Some(sketch) => on_units_changed(UnitsChoice::Sketch {
                                        sketch,
                                        length: Some(unit),
                                        angle: control.angle_override,
                                    }),
                                    None => on_units_changed(UnitsChoice::Document {
                                        length: unit,
                                        angle: control.effective_angle,
                                    }),
                                }
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("Angle ");
                let follow_document_label =
                    format!("Follow document ({})", control.document_angle.label());
                let selected_text = match (control.sketch, control.angle_override) {
                    (Some(_), None) => follow_document_label.clone(),
                    _ => control.effective_angle.label().to_string(),
                };
                egui::ComboBox::from_id_salt("context_angle_unit")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        if let Some(sketch) = control.sketch {
                            if ui
                                .selectable_label(control.angle_override.is_none(), follow_document_label)
                                .clicked()
                            {
                                on_units_changed(UnitsChoice::Sketch {
                                    sketch,
                                    length: control.length_override,
                                    angle: None,
                                });
                            }
                        }
                        for unit in AngleUnit::ALL {
                            let selected = control.angle_override == Some(unit)
                                || (control.sketch.is_none() && control.effective_angle == unit);
                            if ui.selectable_label(selected, unit.label()).clicked() {
                                match control.sketch {
                                    Some(sketch) => on_units_changed(UnitsChoice::Sketch {
                                        sketch,
                                        length: control.length_override,
                                        angle: Some(unit),
                                    }),
                                    None => on_units_changed(UnitsChoice::Document {
                                        length: control.effective_length,
                                        angle: unit,
                                    }),
                                }
                            }
                        }
                    });
            });
        });
        ui.add_space(4.0);
    }

    if !any_control {
        ui.label(
            egui::RichText::new("Select geometry or draw to edit properties")
                .color(egui::Color32::from_gray(140))
                .size(12.0),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Document, FaceId, Line};
    use crate::selection::click_scene_selection;

    fn input<'a>(doc: &'a Document, selection: &'a SceneSelection) -> ContextInput<'a> {
        ContextInput {
            doc,
            selection,
            tool: Tool::Select,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        }
    }

    /// #157/#167: the selection picker surfaces whenever the input carries rows (the
    /// Chamfer/Fillet edge set), including an empty set (which renders the pick hint).
    #[test]
    fn edge_picker_control_follows_input_rows() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let base = ContextInput {
            doc: &doc,
            selection: &selection,
            tool: Tool::Fillet,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: Some(vec!["Block — vertical 0".to_string()]),
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        };
        let content = context_pane_content(&base);
        let edges_picker = |rows: Vec<String>| EdgePickerControl {
            heading: "Edges",
            hint: "Click an edge — Shift+click adds more",
            rows,
        };
        assert_eq!(
            content.edge_picker,
            Some(edges_picker(vec!["Block — vertical 0".to_string()]))
        );

        let empty = ContextInput { edge_treatment_rows: Some(Vec::new()), ..base };
        assert_eq!(
            context_pane_content(&empty).edge_picker,
            Some(edges_picker(Vec::new()))
        );
        let off = ContextInput { edge_treatment_rows: None, ..empty };
        assert_eq!(context_pane_content(&off).edge_picker, None);
    }

    /// #202: the Select tool presents the current selection as an element picker, ordered
    /// deterministically. No selection means no picker (nothing to manage).
    #[test]
    fn select_tool_selection_becomes_an_element_picker() {
        use crate::hierarchy::SceneElement;
        let doc = Document::default();
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(&mut selection, SceneElement::Line(0), true);
        crate::selection::click_scene_selection(&mut selection, SceneElement::Circle(1), true);
        let input = ContextInput {
            doc: &doc,
            selection: &selection,
            tool: Tool::Select,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        };
        let picker = context_pane_content(&input)
            .selection_picker
            .expect("selection picker");
        // Picked set follows `SceneSelection::ordered` (debug-string order): Circle before Line.
        assert_eq!(
            picker.picked(),
            &[SceneElement::Circle(1), SceneElement::Line(0)]
        );
        assert!(picker.has_sticky_focus(), "Select picker never loses focus");
        assert!(picker.accepts(&SceneElement::Body(0)), "Select accepts everything");

        // Empty selection: the picker is still shown (an always-present input), just empty.
        let empty_selection = SceneSelection::default();
        let empty = ContextInput { selection: &empty_selection, ..input };
        let empty_picker = context_pane_content(&empty)
            .selection_picker
            .expect("always-present select picker");
        assert!(empty_picker.is_empty());
        assert_eq!(context_pane_content(&empty).edge_picker, None);
    }

    #[test]
    fn constraint_tool_picker_filters_to_constrainable_geometry() {
        use crate::hierarchy::SceneElement;
        let doc = Document::default();
        let mut selection = SceneSelection::default();
        // A constrainable line plus a body (which the constraint picker should reject).
        crate::selection::click_scene_selection(&mut selection, SceneElement::Line(0), true);
        crate::selection::click_scene_selection(&mut selection, SceneElement::Body(3), true);
        let input = ContextInput {
            tool: Tool::Constraint,
            ..input(&doc, &selection)
        };
        let picker = context_pane_content(&input)
            .selection_picker
            .expect("constraint picker");
        assert_eq!(picker.picked(), &[SceneElement::Line(0)], "body filtered out");
        assert!(!picker.has_sticky_focus());
        assert!(picker.is_focused(), "active tool's picker is focused");
        assert!(!picker.accepts(&SceneElement::Body(0)));
    }

    #[test]
    fn edge_treatment_row_labels_name_the_extrusion_and_edge() {
        let doc = Document::default();
        assert_eq!(
            edge_treatment_row_label(
                &doc,
                3,
                crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 2 }
            ),
            "Extrusion 3 — vertical 2"
        );
        assert_eq!(
            edge_treatment_row_label(
                &doc,
                0,
                crate::model::ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: true }
            ),
            "Extrusion 0 — top 1"
        );
    }

    #[test]
    fn empty_when_nothing_selected() {
        let doc = Document::default();
        assert_eq!(
            context_pane_content(&input(&doc, &SceneSelection::default())),
            ContextPaneContent {
                name: None,
                curve_mode: None,
                tangent_constraint: None,
                construction: None,
                constraints: None,
                snapping: None,
                extrude_body: None,
                edge_picker: None,
                selection_picker: Some(ElementPicker::select_everything()),
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
                calibrate_pending: None,
                units: Some(UnitsControl {
                    sketch: None,
                    effective_length: LengthUnit::Mm,
                    effective_angle: AngleUnit::Deg,
                    length_override: None,
                    angle_override: None,
                    document_length: LengthUnit::Mm,
                    document_angle: AngleUnit::Deg,
                }),
            }
        );
    }

    #[test]
    fn shows_construction_while_drawing_rectangle() {
        let doc = Document::default();
        let content = context_pane_content(&ContextInput {
            doc: &doc,
            selection: &SceneSelection::default(),
            tool: Tool::Select,
            draw_rect_construction: Some(true),
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        });
        assert_eq!(
            content,
            ContextPaneContent {
                name: None,
                curve_mode: None,
                tangent_constraint: None,
                construction: Some(ConstructionControl {
                    value: TriState::On,
                    target_count: 1,
                }),
                constraints: None,
                snapping: None,
                extrude_body: None,
                edge_picker: None,
                selection_picker: None,
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
                calibrate_pending: None,
                units: Some(UnitsControl {
                    sketch: None,
                    effective_length: LengthUnit::Mm,
                    effective_angle: AngleUnit::Deg,
                    length_override: None,
                    angle_override: None,
                    document_length: LengthUnit::Mm,
                    document_angle: AngleUnit::Deg,
                }),
            }
        );
    }

    #[test]
    fn shows_curve_mode_and_tangent_constraint_while_drawing_a_line() {
        let doc = Document::default();
        let content = context_pane_content(&ContextInput {
            doc: &doc,
            selection: &SceneSelection::default(),
            tool: Tool::Line,
            draw_rect_construction: None,
            draw_line_construction: Some(false),
            draw_circle_construction: None,
            draw_line_curve_mode: Some(true),
            draw_line_tangent_constraint: Some(false),
            in_sketch: true,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        });
        assert_eq!(content.curve_mode, Some(true));
        assert_eq!(content.tangent_constraint, Some(false));
    }

    #[test]
    fn shows_name_when_single_element_selected() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 1.0, 0.0));
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Line(0), false);
        assert_eq!(
            context_pane_content(&input(&doc, &sel)),
            ContextPaneContent {
                name: Some(NameControl {
                    element: SceneElement::Line(0),
                }),
                curve_mode: None,
                tangent_constraint: None,
                construction: Some(ConstructionControl {
                    value: TriState::Off,
                    target_count: 1,
                }),
                constraints: None,
                snapping: None,
                extrude_body: None,
                // #213: the Select tool surfaces the selection through the unified element picker.
                edge_picker: None,
                selection_picker: Some({
                    let mut p = ElementPicker::select_everything();
                    p.set_picked([SceneElement::Line(0)]);
                    p
                }),
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
                calibrate_pending: None,
                units: None,
            }
        );
    }

    #[test]
    fn shows_inherited_units_when_sketch_selected() {
        let mut doc = Document::default();
        doc.default_length_unit = LengthUnit::In;
        doc.default_angle_unit = AngleUnit::Rad;
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Sketch(sketch), false);
        let content = context_pane_content(&input(&doc, &sel));
        assert_eq!(
            content.units,
            Some(UnitsControl {
                sketch: Some(sketch),
                effective_length: LengthUnit::In,
                effective_angle: AngleUnit::Rad,
                length_override: None,
                angle_override: None,
                document_length: LengthUnit::In,
                document_angle: AngleUnit::Rad,
            })
        );
    }

    #[test]
    fn shows_overridden_units_when_sketch_selected() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.sketches[sketch].length_unit = Some(LengthUnit::Cm);
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Sketch(sketch), false);
        let content = context_pane_content(&input(&doc, &sel));
        assert_eq!(
            content.units,
            Some(UnitsControl {
                sketch: Some(sketch),
                effective_length: LengthUnit::Cm,
                effective_angle: AngleUnit::Deg,
                length_override: Some(LengthUnit::Cm),
                angle_override: None,
                document_length: LengthUnit::Mm,
                document_angle: AngleUnit::Deg,
            })
        );
    }

    #[test]
    fn hides_units_control_when_non_sketch_element_selected() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 1.0, 0.0));
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Line(0), false);
        assert_eq!(context_pane_content(&input(&doc, &sel)).units, None);
    }

    #[test]
    fn shows_construction_before_drawing_when_rectangle_tool_active() {
        let doc = Document::default();
        let content = context_pane_content(&ContextInput {
            doc: &doc,
            selection: &SceneSelection::default(),
            tool: Tool::Select,
            draw_rect_construction: Some(false),
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        });
        assert_eq!(
            content.construction.unwrap().value,
            TriState::Off
        );
    }

    #[test]
    fn draw_mode_takes_precedence_over_selection() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 1.0, 0.0));
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Line(0), false);
        let content = context_pane_content(&ContextInput {
            doc: &doc,
            selection: &sel,
            tool: Tool::Select,
            draw_rect_construction: Some(true),
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        });
        assert_eq!(
            content,
            ContextPaneContent {
                name: Some(NameControl {
                    element: SceneElement::Line(0),
                }),
                curve_mode: None,
                tangent_constraint: None,
                construction: Some(ConstructionControl {
                    value: TriState::On,
                    target_count: 1,
                }),
                constraints: None,
                snapping: None,
                extrude_body: None,
                edge_picker: None,
                selection_picker: None,
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
                calibrate_pending: None,
                units: None,
            }
        );
    }

    #[test]
    fn constraint_tool_shows_constraint_rows() {
        let doc = Document::default();
        let content = context_pane_content(&ContextInput {
            doc: &doc,
            selection: &SceneSelection::default(),
            tool: Tool::Constraint,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        });
        assert_eq!(
            content.constraints.as_ref().map(|rows| rows.len()),
            Some(crate::geometric_constraints::GeometricConstraintType::ALL.len())
        );
    }
}