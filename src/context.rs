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
    /// True while a technical drawing is open (#317): the model-only "Selection" element picker
    /// is suppressed, since drawing projections/annotations have their own selection state.
    pub in_drawing_workbench: bool,
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
    /// One label per picked extrude profile face, shown in the Extrude tool's face element
    /// picker (#268); `None` when the Extrude tool isn't active.
    pub extrude_faces: Option<Vec<String>>,
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
    /// In-sketch Repeat tool control (#232).
    pub sketch_repeat: Option<SketchRepeatControl>,
    /// In-sketch Slice tool control (#238).
    pub sketch_slice: Option<SketchSliceControl>,
    /// Selected sketch-text editor (#286).
    pub sketch_text: Option<SketchTextControl>,
    /// Selected drawing-projection editor (#289).
    pub drawing_view: Option<DrawingViewControl>,
    /// Selected drawing text annotation editor (#312).
    pub drawing_annotation: Option<DrawingAnnotationControl>,
    /// The Add-view tool is active with nothing placed yet (#289): renders its pick hint.
    pub drawing_add_active: bool,
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
    /// One label per picked profile face, shown in the face element picker (#261).
    pub face_rows: Vec<String>,
    /// Which picker shows the focus ring (#304): exactly one at a time — Profile until a
    /// face is picked, then Axis until the axis is set, then back to Profile.
    pub axis_focused: bool,
    pub axis_label: Option<String>,
    pub symmetric: bool,
    pub body_choice: crate::actions::RevolveBodyChoice,
    /// In Cut mode, the picked bodies to cut (rendered through the unified element picker, #213).
    pub cut_bodies: Vec<usize>,
}

/// What the Combine tool's context section shows: the operation kind, both picker
/// sides (labels), which side the next viewport click lands on, and the keep-B toggle.
#[derive(Clone, Debug, PartialEq)]
pub struct BooleanControl {
    pub kind: crate::model::BooleanOpKind,
    /// Side-A / side-B picked bodies (rendered through the unified element picker, #213).
    pub a: Vec<usize>,
    pub b: Vec<usize>,
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
    /// Picked bodies to move (rendered through the unified element picker, #213).
    pub targets: Vec<usize>,
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
    Commit,
}

/// What the Repeat tool's context section shows.
#[derive(Clone, Debug, PartialEq)]
pub struct RepeatControl {
    /// Picked bodies to repeat (rendered through the unified element picker, #213).
    pub targets: Vec<usize>,
    /// Picked construction planes to repeat as offset copies (#221).
    pub plane_targets: Vec<usize>,
    /// Picked sketches to repeat as offset copies (#231/#234).
    pub sketch_targets: Vec<usize>,
    /// Picked cut/add extrusions whose effect is replayed at each offset (#220/#235).
    pub extrusion_targets: Vec<usize>,
    pub axis_label: String,
    pub mode: crate::model::RepeatMode,
    pub count: String,
    /// The gap field (start-to-start pitch when `gap_is_offset`, else clear gap).
    pub spacing: String,
    /// The distance field (to the end of the last item when `distance_is_end`, else to its start).
    pub length: String,
    /// Which of count/gap/distance is currently **computed** (#257).
    pub computed_var: crate::model::RepeatVar,
    pub gap_is_offset: bool,
    pub distance_is_end: bool,
    /// Formatted value of the computed variable, shown read-only in its field (`None` if it
    /// doesn't evaluate).
    pub computed_value: Option<String>,
    /// Live instance count the current configuration produces (`None` = doesn't evaluate).
    pub preview_instances: Option<usize>,
    pub editing: bool,
    pub can_commit: bool,
}

/// What the in-sketch Repeat tool's context section shows (#232): the picked entities, the
/// repeat direction, and the count/gap/distance fields (which map onto the same variables as the
/// 3D repeat).
#[derive(Clone, Debug, PartialEq)]
pub struct SketchRepeatControl {
    pub entity_count: usize,
    /// The direction source: a picked edge's name, or "the U axis".
    pub direction_label: String,
    pub direction_is_edge: bool,
    pub count: String,
    pub spacing: String,
    pub length: String,
    pub computed_var: crate::model::RepeatVar,
    pub gap_is_offset: bool,
    pub distance_is_end: bool,
    pub can_commit: bool,
    pub editing: bool,
}

/// One edit from the in-sketch Repeat context section (#232).
#[derive(Clone, Debug, PartialEq)]
pub enum SketchRepeatEdit {
    Count(String),
    Gap(String),
    Distance(String),
    ToggleGapOffset,
    ToggleDistanceEnd,
    /// Clear the picked direction edge (fall back to the U axis).
    ClearDirection,
    Commit,
}

/// One edit from the Repeat context section (#257): the three interlinked variables and the two
/// measurement toggles. Editing a variable marks it as one of the two "set" ones (the third is
/// then computed).
#[derive(Clone, Debug, PartialEq)]
pub enum RepeatEdit {
    Axis(crate::model::RevolveAxis),
    Count(String),
    Gap(String),
    Distance(String),
    /// Toggle the gap field between a clear gap and a start-to-start offset (pitch).
    ToggleGapOffset,
    /// Toggle the distance field between start-to-end and start-to-start.
    ToggleDistanceEnd,
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

/// In-sketch Slice control (#238): the two-role picker for slicing sketch lines/circles/faces by
/// cutter lines. Mirrors [`SliceControl`] but without the 3D extend-to-infinity toggle.
#[derive(Clone, Debug, PartialEq)]
pub struct SketchSliceControl {
    pub target_rows: Vec<String>,
    pub cutter_rows: Vec<String>,
    /// `true` while the cutter picker is active (the next viewport click adds a cutter line).
    pub picking_cutter: bool,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the in-sketch Slice context section (#238).
#[derive(Clone, Debug, PartialEq)]
pub enum SketchSliceEdit {
    /// Choose which picker the next viewport click lands on (`true` = cutter).
    PickingCutter(bool),
    /// Clear the target set.
    ClearTargets,
    /// Clear the cutter set.
    ClearCutters,
    Commit,
}

/// Editor for a selected sketch text (#282/#286): the string, font, size, style, and rotation.
/// Editor for the selected drawing projection (#289): shown while a view card is selected on
/// the open drawing page (or right after the Add-view tool places one).
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingViewControl {
    pub view: usize,
    /// The projected source ("Body 0", "Sketch 1", …).
    pub source: String,
    pub orientation: crate::model::DrawingOrientation,
    /// The stored print scale text (`"1:20"`), empty for auto-fit (#300).
    pub scale: String,
    /// True when this view is an aligned child (#296): its scale is inherited from the parent, so
    /// it's read-only here.
    pub aligned: bool,
    /// For an aligned child (#332): the orthographic orientations it may take while staying in
    /// line with its base. Empty for a non-aligned view (or a child of an Isometric parent), which
    /// keeps the full orientation bear/picker.
    pub inline_orientations: Vec<crate::model::DrawingOrientation>,
    /// Whether this view is at a **free** (arbitrary) angle (#345): the widget spins to any angle
    /// instead of picking a preset from the bear.
    pub free_angle: bool,
    /// The source body/sketch's world edges (#358), shown as a live wireframe in the orientation
    /// widget while in free mode. Only populated when `free_angle` (empty otherwise).
    pub source_edges: Vec<(glam::Vec3, glam::Vec3)>,
    /// How the projection renders (#301).
    pub style: crate::model::DrawingViewStyle,
}

/// Editor for a selected drawing text annotation (#312).
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingAnnotationControl {
    pub text: String,
}

/// A drawing element highlighted on the open page (#328/#341): a projection, a text note, or a
/// shown dimension. Used to mark the element the Elements-pane row is hovering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingElementRef {
    Projection(usize),
    Text(usize),
    Dimension { view: usize, a: [i32; 3], b: [i32; 3] },
}

/// One edit from the drawing-annotation context section (#312).
#[derive(Clone, Debug, PartialEq)]
pub enum DrawingAnnotationEdit {
    Text(String),
    Remove,
}


/// One edit from the drawing-view context section (#289).
#[derive(Clone, Debug, PartialEq)]
pub enum DrawingViewEdit {
    Orientation(crate::model::DrawingOrientation),
    /// Display style (#301): visible edges / wireframe / shaded.
    Style(crate::model::DrawingViewStyle),
    /// A valid print-scale text (`"1:20"`), or `None` for auto-fit (#300). Only ever emitted
    /// with text that parses — invalid drafts stay local to the field.
    Scale(Option<String>),
    /// Show every length/diameter dimension (`true`) or hide them all (`false`) for this view
    /// (#331). Views start with none shown; these two buttons flip the whole set at once.
    SetAllDimensions(bool),
    /// Switch between preset (bear) orientations and a free (arbitrary) angle (#345).
    SetFreeAngle(bool),
    Remove,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SketchTextControl {
    pub index: usize,
    pub text: String,
    pub font_family: String,
    /// Installed font families for the chooser.
    pub families: Vec<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub size_expr: String,
    /// Rotation in degrees (the model stores radians).
    pub rotation_deg: String,
    /// Wrap width in mm, empty when unwrapped (#282).
    pub wrap: String,
}

/// One edit from the sketch-text context section (#286). Each re-bakes the text.
#[derive(Clone, Debug, PartialEq)]
pub enum SketchTextEdit {
    Text(String),
    Font(String),
    Bold(bool),
    Italic(bool),
    Underline(bool),
    Size(String),
    Rotation(String),
    /// Wrap width in mm (#282): empty clears wrapping (a growing single-line box).
    Wrap(String),
}

/// One edit from the Combine context section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BooleanEdit {
    Kind(crate::model::BooleanOpKind),
    KeepB(bool),
    Commit,
}

/// One edit from the Revolve context section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RevolveEdit {
    Symmetric(bool),
    BodyChoice(crate::actions::RevolveBodyChoice),
    /// Remove profile face row `i` from the face picker (`None` clears them all) (#261).
    RemoveFace(Option<usize>),
    /// Clear the picked revolve axis (#261).
    ClearAxis,
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
        crate::model::ExtrudeFace::TextGlyph { .. } => "text glyph".to_string(),
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
    /// Picked extrude profile faces, shown as an element picker (#268).
    pub extrude_faces: Option<Vec<String>>,
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
    /// Tool-owned element pickers (#213): the sets a construction tool is gathering (e.g. the
    /// Revolve tool's cut bodies), each rendered by the same combo-box widget. Extensible: a
    /// tool may show several (Combine's A/B sides). Empty for tools not yet migrated.
    pub tool_pickers: Vec<ToolPickerView>,
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
    /// In-sketch Repeat tool control (#232).
    pub sketch_repeat: Option<SketchRepeatControl>,
    /// In-sketch Slice tool control (#238).
    pub sketch_slice: Option<SketchSliceControl>,
    /// Selected sketch-text editor (#286).
    pub sketch_text: Option<SketchTextControl>,
    /// Selected drawing-projection editor (#289).
    pub drawing_view: Option<DrawingViewControl>,
    /// Selected drawing text annotation editor (#312).
    pub drawing_annotation: Option<DrawingAnnotationControl>,
    /// The Add-view tool is active with nothing placed yet (#289).
    pub drawing_add_active: bool,
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
    /// Hint shown while the set is empty (the picker's placeholder).
    pub hint: &'static str,
    /// Icon shown for every row/summary chip (these sets are single-kind).
    pub icon: crate::icons::IconId,
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

/// One tool-owned element picker to render in the context pane (#213): its heading, the
/// [`ElementPicker`] state built from the tool's in-progress set, and which set it edits so
/// removals route back correctly.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPickerView {
    pub heading: &'static str,
    pub picker: ElementPicker,
    pub target: PickerTarget,
}

/// Which tool-owned set a [`ToolPickerView`]'s removals apply to. Grows as tools migrate onto
/// the unified picker; the active tool disambiguates, but this stays explicit so a tool with
/// several pickers (e.g. Combine's two sides) routes each correctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerTarget {
    /// The Revolve tool's cut bodies (`CreatingRevolve::cut_bodies`).
    RevolveCut,
    /// The Move tool's target bodies (`CreatingMove::targets`).
    MoveTargets,
    /// The Repeat tool's target bodies (`CreatingRepeat::targets`).
    RepeatTargets,
    /// The Combine tool's side-A bodies (`CreatingBoolean::a`).
    CombineA,
    /// The Combine tool's side-B bodies (`CreatingBoolean::b`).
    CombineB,
}

/// An interaction with a [`ToolPickerView`] to apply to its backing tool set (#213).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolPickerAction {
    /// The user clicked the picker input; make it the active one (for tools whose viewport
    /// clicks land on one of several pickers, e.g. Combine's A/B sides).
    Focus,
    /// Remove the picked element at this row index.
    Remove(usize),
    /// Clear the whole set.
    Clear,
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

/// Build a Body-filtered tool picker (#213) from a tool's picked body-index set. `selected_color`
/// overrides the highlight (e.g. red for bodies that get cut). Focused, since it's the set the
/// active tool's viewport clicks feed.
fn body_tool_picker(
    heading: &'static str,
    target: PickerTarget,
    bodies: &[usize],
    placeholder: &str,
    selected_color: Option<eframe::egui::Color32>,
    focused: bool,
) -> ToolPickerView {
    let mut picker = ElementPicker::new(ElementFilter::kind(ElementKind::Body), PickLimit::Infinite)
        .with_placeholder(placeholder.to_string());
    if let Some(color) = selected_color {
        picker = picker.with_selected_color(color);
    }
    picker.set_focused(focused);
    picker.set_picked(bodies.iter().map(|&bi| SceneElement::Body(bi)));
    ToolPickerView {
        heading,
        picker,
        target,
    }
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
    let extrude_faces = input.extrude_faces.clone();
    // The Repeat tool's own context is busy enough; its distances are plain lengths, so the
    // Default-units section isn't shown while it's active (#257). The Text tool has nothing to do
    // with the document's default units either, so it's suppressed there too (#330).
    let units = (input.tool != Tool::Repeat && input.tool != Tool::Text)
        .then(|| units_control_from_selection(input.doc, input.selection))
        .flatten();
    let edge_picker = input
        .edge_treatment_rows
        .clone()
        .map(|rows| EdgePickerControl {
            heading: "Edges",
            hint: "Click an edge — Shift+click adds more",
            icon: crate::icons::IconId::Line,
            rows,
        })
        .or_else(|| {
            input.loft_rows.clone().map(|rows| EdgePickerControl {
                heading: "Sections",
                hint: "Click a closed profile (circle or loop)",
                icon: crate::icons::IconId::Circle,
                rows,
            })
        });
    // The unified selection element picker (#213), mirroring the live selection for the tools
    // that operate on it. Suppressed while a draw construction owns the pane.
    let drawing = input.draw_rect_construction.is_some()
        || input.draw_line_construction.is_some()
        || input.draw_circle_construction.is_some();
    let selection_picker = (!drawing && !input.in_drawing_workbench)
        .then(|| selection_picker_for(input.tool, input.selection))
        .flatten();
    // Tool-owned element pickers (#213). Each is a Body-filtered picker built from the tool's
    // in-progress set. Bodies consumed destructively (Revolve cut) get the red highlight override.
    let mut tool_pickers = Vec::new();
    if let Some(r) = input.revolve.as_ref() {
        if r.body_choice == crate::actions::RevolveBodyChoice::Cut {
            tool_pickers.push(body_tool_picker(
                "Cut bodies",
                PickerTarget::RevolveCut,
                &r.cut_bodies,
                "Click a body to cut",
                Some(crate::theme::CUT_ACCENT),
                true,
            ));
        }
    }
    if let Some(m) = input.move_op.as_ref() {
        tool_pickers.push(body_tool_picker(
            "Bodies",
            PickerTarget::MoveTargets,
            &m.targets,
            "Click bodies in the viewport",
            None,
            true,
        ));
    }
    if let Some(r) = input.repeat_op.as_ref() {
        tool_pickers.push(body_tool_picker(
            "Bodies",
            PickerTarget::RepeatTargets,
            &r.targets,
            "Click bodies in the viewport",
            None,
            true,
        ));
    }
    if let Some(b) = input.boolean_op.as_ref() {
        // Combine mode uses one picker (side A only); Cut/Intersect/Difference use two sides.
        // The focused side is the one the next viewport click lands on, toggled by clicking a
        // picker (its Focus event). Side B (the tool that gets consumed in Cut) is styled red.
        let two_sided = b.kind != crate::model::BooleanOpKind::Combine;
        tool_pickers.push(body_tool_picker(
            if two_sided { "Side A" } else { "Bodies" },
            PickerTarget::CombineA,
            &b.a,
            "Click bodies in the viewport",
            None,
            !b.picking_b,
        ));
        if two_sided {
            tool_pickers.push(body_tool_picker(
                "Side B",
                PickerTarget::CombineB,
                &b.b,
                "Click bodies in the viewport",
                (b.kind == crate::model::BooleanOpKind::Cut).then_some(crate::theme::CUT_ACCENT),
                b.picking_b,
            ));
        }
    }
    let calibrate_image = input.calibrate_image;
    let revolve = input.revolve.clone();
    let boolean_op = input.boolean_op.clone();
    let boolean_edit_start = input.boolean_edit_start;
    let move_op = input.move_op.clone();
    let move_edit_start = input.move_edit_start;
    let repeat_op = input.repeat_op.clone();
    let sketch_repeat = input.sketch_repeat.clone();
    let sketch_slice = input.sketch_slice.clone();
    let sketch_text = input.sketch_text.clone();
    // With the Text tool active, the pane belongs to placing/editing text — a projection that
    // happens to still be selected must not show its editor here (#329). The Dimension/Select
    // tools keep the projection editor.
    let drawing_view = if input.tool == Tool::Text {
        None
    } else {
        input.drawing_view.clone()
    };
    let drawing_annotation = input.drawing_annotation.clone();
    let drawing_add_active = input.drawing_add_active;
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
            extrude_faces: extrude_faces.clone(),
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            tool_pickers: Vec::new(),
            calibrate_image,
            revolve: revolve.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            repeat_op: repeat_op.clone(),
            sketch_repeat: sketch_repeat.clone(),
            sketch_slice: sketch_slice.clone(),
            sketch_text: sketch_text.clone(),
            drawing_view: drawing_view.clone(),
            drawing_annotation: drawing_annotation.clone(),
            drawing_add_active,
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
            extrude_faces: extrude_faces.clone(),
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            tool_pickers: Vec::new(),
            calibrate_image,
            revolve: revolve.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            repeat_op: repeat_op.clone(),
            sketch_repeat: sketch_repeat.clone(),
            sketch_slice: sketch_slice.clone(),
            sketch_text: sketch_text.clone(),
            drawing_view: drawing_view.clone(),
            drawing_annotation: drawing_annotation.clone(),
            drawing_add_active,
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
            extrude_faces: extrude_faces.clone(),
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            tool_pickers: Vec::new(),
            calibrate_image,
            revolve: revolve.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            repeat_op: repeat_op.clone(),
            sketch_repeat: sketch_repeat.clone(),
            sketch_slice: sketch_slice.clone(),
            sketch_text: sketch_text.clone(),
            drawing_view: drawing_view.clone(),
            drawing_annotation: drawing_annotation.clone(),
            drawing_add_active,
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
        extrude_faces: extrude_faces.clone(),
        units,
        edge_picker,
        selection_picker,
        tool_pickers,
        calibrate_image,
        revolve,
        boolean_op,
        boolean_edit_start,
        move_op,
        move_edit_start,
        repeat_op,
        sketch_repeat,
        sketch_slice,
        sketch_text,
        drawing_view,
        drawing_annotation,
        drawing_add_active,
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
    on_extrude_face_remove: &mut impl FnMut(Option<usize>),
    on_units_changed: &mut impl FnMut(UnitsChoice),
    on_edge_picker_edit: &mut impl FnMut(Option<usize>),
    on_selection_edit: &mut impl FnMut(SelectionEdit),
    on_tool_picker_edit: &mut impl FnMut(PickerTarget, ToolPickerAction),
    on_revolve_edit: &mut impl FnMut(RevolveEdit),
    on_boolean_edit: &mut impl FnMut(BooleanEdit),
    on_boolean_edit_start: &mut impl FnMut(usize),
    on_move_edit: &mut impl FnMut(MoveEdit),
    on_move_edit_start: &mut impl FnMut(usize),
    on_repeat_edit: &mut impl FnMut(RepeatEdit),
    on_sketch_repeat_edit: &mut impl FnMut(SketchRepeatEdit),
    on_sketch_slice_edit: &mut impl FnMut(SketchSliceEdit),
    on_sketch_text_edit: &mut impl FnMut(SketchTextEdit),
    on_drawing_view_edit: &mut impl FnMut(DrawingViewEdit),
    on_drawing_annotation_edit: &mut impl FnMut(DrawingAnnotationEdit),
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

    // The element picker is the primary control for the Select tool, so it renders first (#246).
    if let Some(picker) = &content.selection_picker {
        any_control = true;
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
    }

    // Tool-owned element pickers (#213) render at the top of the active tool's section, above
    // its parameter controls — the picked set is the tool's primary input.
    for view in &content.tool_pickers {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new(view.heading).strong());
        ui.add_enabled_ui(controls_enabled, |ui| {
            if let Some(event) = crate::element_picker::show(ui, &view.picker, doc, view.heading) {
                match event {
                    crate::element_picker::PickerEvent::Focus => {
                        on_tool_picker_edit(view.target, ToolPickerAction::Focus)
                    }
                    // Tool-owned sets are ordered vectors, so a row index maps straight through.
                    crate::element_picker::PickerEvent::Remove(i) => {
                        on_tool_picker_edit(view.target, ToolPickerAction::Remove(i))
                    }
                    crate::element_picker::PickerEvent::Clear => {
                        on_tool_picker_edit(view.target, ToolPickerAction::Clear)
                    }
                }
            }
        });
    }

    if let Some(control) = &content.revolve {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Revolve").strong());

        // Face element picker (#261): the picked profile faces, click one's ✕ to drop it. Faces
        // are still added by clicking them in the viewport.
        ui.label("Profile");
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "revolve_faces",
            !control.axis_focused,
            "Click a profile face",
            crate::icons::IconId::Sketch,
            &control.face_rows,
        ) {
            match event {
                crate::element_picker::PickerEvent::Focus => {}
                crate::element_picker::PickerEvent::Remove(i) => {
                    on_revolve_edit(RevolveEdit::RemoveFace(Some(i)))
                }
                crate::element_picker::PickerEvent::Clear => {
                    on_revolve_edit(RevolveEdit::RemoveFace(None))
                }
            }
        }

        // Axis element picker (#261): the picked edge/axis, click its ✕ to clear. Set it by
        // clicking a straight line or a global axis in the viewport.
        ui.label("Axis");
        let axis_rows: Vec<String> = control.axis_label.iter().cloned().collect();
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "revolve_axis",
            control.axis_focused,
            "Click an axis line",
            crate::icons::IconId::Line,
            &axis_rows,
        ) {
            match event {
                crate::element_picker::PickerEvent::Focus => {}
                crate::element_picker::PickerEvent::Remove(_)
                | crate::element_picker::PickerEvent::Clear => {
                    on_revolve_edit(RevolveEdit::ClearAxis)
                }
            }
        }

        let mut symmetric = control.symmetric;
        if ui.checkbox(&mut symmetric, "Symmetric").changed() {
            on_revolve_edit(RevolveEdit::Symmetric(symmetric));
        }
        // A segmented icon group (#261): New body / Add to touching / Cut, one highlighted —
        // the same icons the Extrude "into" picker uses. A cut needs the kernel, so it's only
        // offered on an `occt` build (mirrors the Extrude cut option).
        let choice = control.body_choice;
        ui.horizontal(|ui| {
            let mut choices = vec![
                (
                    crate::actions::RevolveBodyChoice::NewBody,
                    crate::icons::IconId::NewBody,
                    "New body",
                ),
                (
                    crate::actions::RevolveBodyChoice::AddTouching,
                    crate::icons::IconId::AddToBody,
                    "Add to touching bodies",
                ),
            ];
            if cfg!(feature = "occt") {
                choices.push((
                    crate::actions::RevolveBodyChoice::Cut,
                    crate::icons::IconId::CutBody,
                    "Cut bodies",
                ));
            }
            for (value, icon, tooltip) in choices {
                if crate::icons::selectable_icon_button(ui, icon, choice == value, tooltip)
                    .clicked()
                    && choice != value
                {
                    on_revolve_edit(RevolveEdit::BodyChoice(value));
                }
            }
        });
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
        // A segmented icon group (#267): two-circle boolean icons with kept regions solid and
        // removed regions faint red.
        let kind = control.kind;
        ui.horizontal(|ui| {
            for (value, icon) in [
                (crate::model::BooleanOpKind::Combine, crate::icons::IconId::BooleanUnion),
                (crate::model::BooleanOpKind::Cut, crate::icons::IconId::BooleanCut),
                (
                    crate::model::BooleanOpKind::Intersect,
                    crate::icons::IconId::BooleanIntersect,
                ),
                (
                    crate::model::BooleanOpKind::Difference,
                    crate::icons::IconId::BooleanDifference,
                ),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, kind == value, value.label())
                    .clicked()
                    && kind != value
                {
                    on_boolean_edit(BooleanEdit::Kind(value));
                }
            }
        });
        let two_sided = control.kind != crate::model::BooleanOpKind::Combine;
        // The side-A / side-B body sets render as element pickers above (see `tool_pickers`);
        // clicking a picker makes it the active side. Only the "keep B" toggle stays here.
        if two_sided {
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
        // The picked bodies render through the unified element picker (see `tool_pickers`).
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
        // The picked bodies render through the unified element picker (see `tool_pickers`).
        // Construction-plane targets (#221) are picked via the Elements pane / viewport, like the
        // Move tool's planes — surfaced here as a count so the picked set is visible.
        let mut pending: Option<RepeatEdit> = None;
        if !control.plane_targets.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{} construction plane(s) — copied along the axis",
                    control.plane_targets.len()
                ))
                .color(egui::Color32::from_gray(140))
                .size(11.0),
            );
        }
        if !control.sketch_targets.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{} sketch(es) — copied along the axis",
                    control.sketch_targets.len()
                ))
                .color(egui::Color32::from_gray(140))
                .size(11.0),
            );
        }
        if !control.extrusion_targets.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "{} operation(s) — replayed along the axis",
                    control.extrusion_targets.len()
                ))
                .color(egui::Color32::from_gray(140))
                .size(11.0),
            );
        }
        // Axis element picker (#257): shows the picked edge/axis. Set it by clicking a straight
        // line or a global axis in the viewport; the ✕ resets to the X axis. Quick X/Y/Z buttons
        // stay for the global axes (which are awkward to click).
        ui.label("Axis");
        let axis_rows = vec![format!("Along {}", control.axis_label)];
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "repeat_axis",
            true,
            "Click an axis line",
            crate::icons::IconId::Line,
            &axis_rows,
        ) {
            if matches!(
                event,
                crate::element_picker::PickerEvent::Remove(_) | crate::element_picker::PickerEvent::Clear
            ) {
                pending = Some(RepeatEdit::Axis(crate::model::RevolveAxis::X));
            }
        }
        ui.horizontal(|ui| {
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
        // Count / gap / distance (#257): the user edits two, the third is computed and shown
        // read-only in its field. Gap and distance each have a picture toggle to switch how
        // they're measured.
        use crate::model::RepeatVar;
        {
            // One variable row: `label` + a text field (read-only + showing the computed value
            // when this is the computed variable), optionally preceded by a toggle button.
            let mut var_row = |ui: &mut egui::Ui,
                               var: RepeatVar,
                               label: &str,
                               value: &str,
                               toggle: Option<(crate::icons::IconId, RepeatEdit)>,
                               make: &dyn Fn(String) -> RepeatEdit| {
                let computed = control.computed_var == var;
                ui.horizontal(|ui| {
                    if let Some((icon, edit)) = toggle {
                        // A clickable picture that toggles how this variable is measured (#257).
                        if crate::icons::icon_button(ui, icon, "Click to toggle how this is measured")
                            .clicked()
                        {
                            pending = Some(edit);
                        }
                    }
                    ui.label(label);
                    if computed {
                        let shown = control.computed_value.clone().unwrap_or_else(|| "—".to_string());
                        ui.add_enabled(
                            false,
                            egui::TextEdit::singleline(&mut shown.clone()).desired_width(90.0),
                        )
                        .on_hover_text("Computed from the other two");
                        ui.label(egui::RichText::new("auto").color(egui::Color32::from_gray(130)).size(10.0));
                    } else {
                        let mut text = value.to_string();
                        if ui
                            .add(egui::TextEdit::singleline(&mut text).desired_width(90.0))
                            .changed()
                        {
                            pending = Some(make(text));
                        }
                    }
                });
            };
            var_row(ui, RepeatVar::Count, "Count", &control.count, None, &RepeatEdit::Count);
            let gap_icon = if control.gap_is_offset {
                crate::icons::IconId::RepeatGapOffset
            } else {
                crate::icons::IconId::RepeatGapBetween
            };
            var_row(
                ui,
                RepeatVar::Gap,
                if control.gap_is_offset { "Offset" } else { "Gap" },
                &control.spacing,
                Some((gap_icon, RepeatEdit::ToggleGapOffset)),
                &RepeatEdit::Gap,
            );
            let dist_icon = if control.distance_is_end {
                crate::icons::IconId::RepeatDistEnd
            } else {
                crate::icons::IconId::RepeatDistStart
            };
            var_row(
                ui,
                RepeatVar::Distance,
                "Distance",
                &control.length,
                Some((dist_icon, RepeatEdit::ToggleDistanceEnd)),
                &RepeatEdit::Distance,
            );
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

    // In-sketch Repeat tool (#232): entities + direction + count/gap/distance.
    if let Some(control) = &content.sketch_repeat {
        use crate::model::RepeatVar;
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Repeat (in sketch)").strong());
        ui.label(
            egui::RichText::new(format!(
                "{} entities · direction: {} (Shift+click an edge)",
                control.entity_count, control.direction_label
            ))
            .color(egui::Color32::from_gray(140))
            .size(11.0),
        );
        let mut pending: Option<SketchRepeatEdit> = None;
        if control.direction_is_edge && ui.small_button("Use U axis").clicked() {
            pending = Some(SketchRepeatEdit::ClearDirection);
        }
        let mut var_row = |ui: &mut egui::Ui,
                           var: RepeatVar,
                           label: &str,
                           value: &str,
                           toggle: Option<(crate::icons::IconId, SketchRepeatEdit)>,
                           make: &dyn Fn(String) -> SketchRepeatEdit| {
            let computed = control.computed_var == var;
            ui.horizontal(|ui| {
                if let Some((icon, edit)) = toggle {
                    if crate::icons::icon_button(ui, icon, "Toggle how this is measured").clicked() {
                        pending = Some(edit);
                    }
                }
                ui.label(label);
                if computed {
                    ui.label(egui::RichText::new("(auto)").color(egui::Color32::from_gray(130)).size(10.0));
                } else {
                    let mut text = value.to_string();
                    if ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0)).changed() {
                        pending = Some(make(text));
                    }
                }
            });
        };
        var_row(ui, RepeatVar::Count, "Count", &control.count, None, &SketchRepeatEdit::Count);
        let gap_icon = if control.gap_is_offset {
            crate::icons::IconId::RepeatGapOffset
        } else {
            crate::icons::IconId::RepeatGapBetween
        };
        var_row(
            ui,
            RepeatVar::Gap,
            if control.gap_is_offset { "Offset" } else { "Gap" },
            &control.spacing,
            Some((gap_icon, SketchRepeatEdit::ToggleGapOffset)),
            &SketchRepeatEdit::Gap,
        );
        let dist_icon = if control.distance_is_end {
            crate::icons::IconId::RepeatDistEnd
        } else {
            crate::icons::IconId::RepeatDistStart
        };
        var_row(
            ui,
            RepeatVar::Distance,
            "Distance",
            &control.length,
            Some((dist_icon, SketchRepeatEdit::ToggleDistanceEnd)),
            &SketchRepeatEdit::Distance,
        );
        if let Some(edit) = pending {
            on_sketch_repeat_edit(edit);
        }
        ui.add_space(2.0);
        if ui
            .add_enabled(
                control.can_commit && controls_enabled,
                egui::Button::new(if control.editing { "Apply changes" } else { "Repeat" }),
            )
            .clicked()
        {
            on_sketch_repeat_edit(SketchRepeatEdit::Commit);
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
        // Two element pickers; the focused one is the side the next viewport click lands on
        // (clicking a picker makes it active, replacing the old Bodies/Cutters toggle).
        ui.label(egui::RichText::new("Bodies").strong());
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "slice_targets",
            !control.picking_cutter,
            "Click bodies in the viewport",
            crate::icons::IconId::Body,
            &control.target_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SliceEdit::PickingCutter(false),
                crate::element_picker::PickerEvent::Remove(i) => SliceEdit::RemoveTarget(Some(i)),
                crate::element_picker::PickerEvent::Clear => SliceEdit::RemoveTarget(None),
            });
        }
        ui.label(egui::RichText::new("Cutters").strong());
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "slice_cutters",
            control.picking_cutter,
            "Click planes or faces to cut with",
            crate::icons::IconId::Plane,
            &control.cutter_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SliceEdit::PickingCutter(true),
                crate::element_picker::PickerEvent::Remove(i) => SliceEdit::RemoveCutter(Some(i)),
                crate::element_picker::PickerEvent::Clear => SliceEdit::RemoveCutter(None),
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

    // In-sketch Slice (#238): two-role pickers for sketch targets (lines/circles/faces) and cutter
    // lines, like the Combine tool's A/B pickers. Clicking a picker makes it the active side.
    if let Some(control) = &content.sketch_slice {
        any_control = true;
        ui.separator();
        ui.label(
            egui::RichText::new(if control.editing { "Edit slice" } else { "Slice (in sketch)" })
                .strong(),
        );
        let mut pending: Option<SketchSliceEdit> = None;
        ui.label(egui::RichText::new("Targets").strong());
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "sketch_slice_targets",
            !control.picking_cutter,
            "Click sketch lines, circles, or faces",
            crate::icons::IconId::Line,
            &control.target_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SketchSliceEdit::PickingCutter(false),
                crate::element_picker::PickerEvent::Remove(_)
                | crate::element_picker::PickerEvent::Clear => SketchSliceEdit::ClearTargets,
            });
        }
        ui.label(egui::RichText::new("Cutters").strong());
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "sketch_slice_cutters",
            control.picking_cutter,
            "Click sketch lines to cut with",
            crate::icons::IconId::Line,
            &control.cutter_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SketchSliceEdit::PickingCutter(true),
                crate::element_picker::PickerEvent::Remove(_)
                | crate::element_picker::PickerEvent::Clear => SketchSliceEdit::ClearCutters,
            });
        }
        if let Some(edit) = pending {
            on_sketch_slice_edit(edit);
        }
        ui.add_space(2.0);
        if ui
            .add_enabled(
                control.can_commit && controls_enabled,
                egui::Button::new(if control.editing { "Apply changes" } else { "Slice" }),
            )
            .clicked()
        {
            on_sketch_slice_edit(SketchSliceEdit::Commit);
        }
    }

    // Sketch-text editor (#286): edit the selected text's string, font, size, style, rotation.
    if let Some(control) = &content.sketch_text {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Text").strong());
        let mut edit_text = control.text.clone();
        // {…} variable autocomplete (#338): handle Tab/arrows before the field, dropdown after.
        let text_id = ui.make_persistent_id("sketch_text_edit_field");
        let ectx = ui.ctx().clone();
        if ectx.memory(|m| m.focused()) == Some(text_id)
            && crate::expression_input::interp_autocomplete_handle_keys(
                ui, &ectx, text_id, &mut edit_text, doc, &[],
            )
        {
            on_sketch_text_edit(SketchTextEdit::Text(edit_text.clone()));
        }
        let text_resp = ui.add(
            egui::TextEdit::multiline(&mut edit_text)
                .id(text_id)
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if text_resp.changed() {
            on_sketch_text_edit(SketchTextEdit::Text(edit_text.clone()));
        }
        if text_resp.has_focus() {
            let cursor =
                crate::expression_input::text_edit_cursor_char_index(&ectx, text_id, &edit_text);
            if crate::expression_input::interp_autocomplete_show_dropdown(
                ui, &ectx, &text_resp, text_id, &mut edit_text, doc, &[], cursor,
            ) {
                on_sketch_text_edit(SketchTextEdit::Text(edit_text.clone()));
            }
        }
        // Font family chooser.
        egui::ComboBox::from_id_salt("sketch_text_font")
            .selected_text(control.font_family.clone())
            .show_ui(ui, |ui| {
                for fam in &control.families {
                    if ui
                        .selectable_label(fam == &control.font_family, fam)
                        .clicked()
                    {
                        on_sketch_text_edit(SketchTextEdit::Font(fam.clone()));
                    }
                }
            });
        ui.horizontal(|ui| {
            let mut bold = control.bold;
            if ui.selectable_label(bold, egui::RichText::new("B").strong()).clicked() {
                bold = !bold;
                on_sketch_text_edit(SketchTextEdit::Bold(bold));
            }
            let mut italic = control.italic;
            if ui.selectable_label(italic, egui::RichText::new("I").italics()).clicked() {
                italic = !italic;
                on_sketch_text_edit(SketchTextEdit::Italic(italic));
            }
            let mut underline = control.underline;
            if ui.selectable_label(underline, egui::RichText::new("U").underline()).clicked() {
                underline = !underline;
                on_sketch_text_edit(SketchTextEdit::Underline(underline));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Size");
            let mut size = control.size_expr.clone();
            if ui.add(egui::TextEdit::singleline(&mut size).desired_width(70.0)).changed() {
                on_sketch_text_edit(SketchTextEdit::Size(size));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Rotation°");
            let mut rot = control.rotation_deg.clone();
            if ui.add(egui::TextEdit::singleline(&mut rot).desired_width(70.0)).changed() {
                on_sketch_text_edit(SketchTextEdit::Rotation(rot));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Wrap width");
            let mut wrap = control.wrap.clone();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut wrap)
                        .hint_text("grow")
                        .desired_width(70.0),
                )
                .on_hover_text("mm to wrap to; empty grows the box to fit")
                .changed()
            {
                on_sketch_text_edit(SketchTextEdit::Wrap(wrap));
            }
        });
    }

    // Drawing-projection editor (#289): the selected view card's source, orientation, and a
    // remove button; the Add-view tool shows its pick hint until something is placed.
    if let Some(control) = &content.drawing_view {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("View").strong());
        ui.label(&control.source);
        // An aligned child's orientation is derived from its base and placement direction — it
        // unfolds (and rotates) to stay in line (#351) — so it's shown read-only. Change the view
        // by placing it in a different direction.
        if control.aligned {
            ui.label(
                egui::RichText::new(format!("{} · aligned", control.orientation.label()))
                    .color(egui::Color32::from_gray(150)),
            );
        } else {
            // Toggle between preset (bear) orientations and a free spin (#345).
            let mut free = control.free_angle;
            if ui.checkbox(&mut free, "Free angle").changed() {
                on_drawing_view_edit(DrawingViewEdit::SetFreeAngle(free));
            }
            // Interactive orientation bear (#315): drag to spin, click a face for that view or
            // a corner/edge for isometric; focus it and press 4/5/6/8/2/0 for
            // left/front/right/top/bottom/back. In free mode, spinning sets an arbitrary angle.
            let seed = drawing_orientation_to_standard(control.orientation);
            // Highlight the current view on the bear (#323/#340): a face, a corner (Isometric),
            // or a cube edge (a diagonal edge view, #339). Drawn even when behind the bear.
            let selected = drawing_orientation_to_cube_pick(control.orientation);
            let body_edges = control.free_angle.then_some(control.source_edges.as_slice());
            if let Some(pick) = crate::view_cube::show_orientation_picker(
                ui,
                "drawing_view_bear",
                seed,
                selected,
                control.free_angle,
                body_edges,
                None,
                false,
            ) {
                on_drawing_view_edit(DrawingViewEdit::Orientation(orientation_pick_to_drawing(pick)));
            }
        }
        egui::ComboBox::from_id_salt("drawing_view_style")
            .selected_text(control.style.label())
            .show_ui(ui, |ui| {
                for style in crate::model::DrawingViewStyle::ALL {
                    if ui.selectable_label(control.style == style, style.label()).clicked() {
                        on_drawing_view_edit(DrawingViewEdit::Style(style));
                    }
                }
            });
        ui.horizontal(|ui| {
            ui.label("Scale");
            if control.aligned {
                // An aligned child inherits the parent's scale and can't change it (#296/#300).
                let shown = if control.scale.is_empty() { "auto (inherited)".to_string() } else { control.scale.clone() };
                ui.label(egui::RichText::new(shown).color(egui::Color32::from_gray(150)));
            } else {
                // The field drafts locally while focused (#300): only text that parses as
                // `page:model` commits, so the view keeps its last valid scale; empty = auto-fit.
                let draft_id = egui::Id::new(("drawing_view_scale_draft", control.view));
                let mut draft = ui
                    .data(|d| d.get_temp::<String>(draft_id))
                    .unwrap_or_else(|| control.scale.clone());
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut draft)
                        .hint_text("1:20")
                        .desired_width(70.0),
                );
                if resp.changed() {
                    let trimmed = draft.trim();
                    if trimmed.is_empty() {
                        on_drawing_view_edit(DrawingViewEdit::Scale(None));
                    } else if crate::model::parse_drawing_scale(trimmed).is_some() {
                        on_drawing_view_edit(DrawingViewEdit::Scale(Some(trimmed.to_string())));
                    }
                }
                if resp.has_focus() {
                    ui.data_mut(|d| d.insert_temp(draft_id, draft));
                } else {
                    ui.data_mut(|d| d.remove::<String>(draft_id));
                }
            }
        });
        // Dimensions are off by default (#331); these flip the whole set on or off at once.
        ui.horizontal(|ui| {
            if ui.button("Show all dimensions").clicked() {
                on_drawing_view_edit(DrawingViewEdit::SetAllDimensions(true));
            }
            if ui.button("Hide all dimensions").clicked() {
                on_drawing_view_edit(DrawingViewEdit::SetAllDimensions(false));
            }
        });
        if ui.button("Remove view").clicked() {
            on_drawing_view_edit(DrawingViewEdit::Remove);
        }
    } else if content.drawing_add_active {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Add view").strong());
        ui.label(
            egui::RichText::new(
                "Click a body or sketch in the Elements pane to place it on the page",
            )
            .color(egui::Color32::from_gray(140)),
        );
    }

    // Drawing text annotation editor (#312): a multiline textarea + remove button.
    if let Some(control) = &content.drawing_annotation {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new("Text").strong());
        let mut edit_text = control.text.clone();
        // {…} variable autocomplete (#338): handle Tab/arrows before the field, dropdown after.
        let text_id = ui.make_persistent_id("drawing_annotation_edit_field");
        let ectx = ui.ctx().clone();
        if ectx.memory(|m| m.focused()) == Some(text_id)
            && crate::expression_input::interp_autocomplete_handle_keys(
                ui, &ectx, text_id, &mut edit_text, doc, &[],
            )
        {
            on_drawing_annotation_edit(DrawingAnnotationEdit::Text(edit_text.clone()));
        }
        let text_resp = ui.add(
            egui::TextEdit::multiline(&mut edit_text)
                .id(text_id)
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if text_resp.changed() {
            on_drawing_annotation_edit(DrawingAnnotationEdit::Text(edit_text.clone()));
        }
        if text_resp.has_focus() {
            let cursor =
                crate::expression_input::text_edit_cursor_char_index(&ectx, text_id, &edit_text);
            if crate::expression_input::interp_autocomplete_show_dropdown(
                ui, &ectx, &text_resp, text_id, &mut edit_text, doc, &[], cursor,
            ) {
                on_drawing_annotation_edit(DrawingAnnotationEdit::Text(edit_text.clone()));
            }
        }
        if ui.button("Remove text").clicked() {
            on_drawing_annotation_edit(DrawingAnnotationEdit::Remove);
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

    if let Some(picker) = &content.edge_picker {
        any_control = true;
        ui.separator();
        ui.label(egui::RichText::new(picker.heading).strong());
        ui.add_enabled_ui(controls_enabled, |ui| {
            // The active tool's picker is focused (its viewport clicks feed this set).
            if let Some(event) = crate::element_picker::show_labeled(
                ui,
                picker.heading,
                true,
                picker.hint,
                picker.icon,
                &picker.rows,
            ) {
                match event {
                    crate::element_picker::PickerEvent::Focus => {}
                    crate::element_picker::PickerEvent::Remove(i) => on_edge_picker_edit(Some(i)),
                    crate::element_picker::PickerEvent::Clear => on_edge_picker_edit(None),
                }
            }
        });
    }

    if let Some(faces) = &content.extrude_faces {
        any_control = true;
        // Extrude face element picker (#268): the picked profile faces, each with a ✕ to drop
        // it. Faces are added by clicking them in the viewport.
        ui.label("Faces");
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "extrude_faces",
            true,
            "Click a face to extrude",
            crate::icons::IconId::Sketch,
            faces,
        ) {
            match event {
                crate::element_picker::PickerEvent::Focus => {}
                crate::element_picker::PickerEvent::Remove(i) => on_extrude_face_remove(Some(i)),
                crate::element_picker::PickerEvent::Clear => on_extrude_face_remove(None),
            }
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

/// Map a drawing orientation to the bear's selected-pose highlight (#340): a face for the six
/// orthographic views, the top-front-right corner for Isometric, or the matching cube edge for a
/// diagonal edge view (#339).
fn drawing_orientation_to_cube_pick(
    o: crate::model::DrawingOrientation,
) -> Option<crate::view_cube::CubePick> {
    use crate::model::{DrawingOrientation as O, EdgeView as E};
    use crate::view_cube::{CubeCornerId, CubeEdgeId, CubePick};
    match o {
        O::Front | O::Back | O::Left | O::Right | O::Top | O::Bottom => {
            Some(CubePick::Face(drawing_orientation_to_standard(o)))
        }
        O::Isometric => Some(CubePick::Corner(CubeCornerId::FrontRightTop)),
        O::Corner(c) => {
            use crate::model::CornerView as CV;
            let id = match c {
                CV::FrontLeftBottom => CubeCornerId::FrontLeftBottom,
                CV::FrontRightBottom => CubeCornerId::FrontRightBottom,
                CV::BackRightBottom => CubeCornerId::BackRightBottom,
                CV::BackLeftBottom => CubeCornerId::BackLeftBottom,
                CV::FrontLeftTop => CubeCornerId::FrontLeftTop,
                CV::FrontRightTop => CubeCornerId::FrontRightTop,
                CV::BackRightTop => CubeCornerId::BackRightTop,
                CV::BackLeftTop => CubeCornerId::BackLeftTop,
            };
            Some(CubePick::Corner(id))
        }
        O::Edge(e) => {
            let id = match e {
                E::FrontRight => CubeEdgeId::FrontRight,
                E::BackRight => CubeEdgeId::BackRight,
                E::BackLeft => CubeEdgeId::BackLeft,
                E::FrontLeft => CubeEdgeId::FrontLeft,
                E::FrontTop => CubeEdgeId::FrontTop,
                E::RightTop => CubeEdgeId::RightTop,
                E::BackTop => CubeEdgeId::BackTop,
                E::LeftTop => CubeEdgeId::LeftTop,
                E::FrontBottom => CubeEdgeId::FrontBottom,
                E::RightBottom => CubeEdgeId::RightBottom,
                E::BackBottom => CubeEdgeId::BackBottom,
                E::LeftBottom => CubeEdgeId::LeftBottom,
            };
            Some(CubePick::Edge(id))
        }
        // A free angle (#345) isn't a cube face/edge/corner, so nothing is highlighted.
        O::Free { .. } => None,
    }
}

/// Map a drawing orientation to the bear picker's `StandardView` for seeding its pose (#315).
/// Isometric has no straight-on equivalent, so it seeds to Front.
fn drawing_orientation_to_standard(o: crate::model::DrawingOrientation) -> crate::camera::StandardView {
    use crate::camera::StandardView as S;
    use crate::model::DrawingOrientation as O;
    match o {
        O::Front | O::Isometric => S::Front,
        O::Back => S::Back,
        O::Left => S::Left,
        O::Right => S::Right,
        O::Top => S::Top,
        O::Bottom => S::Bottom,
        // An edge/corner view (#339/#344) has no single straight-on face; seed from its first.
        O::Edge(e) => drawing_orientation_to_standard(e.faces().0),
        O::Corner(c) => drawing_orientation_to_standard(c.faces().0),
        // A free angle (#345) seeds the bear to Front (the widget then follows the stored basis).
        O::Free { .. } => S::Front,
    }
}

/// Map a bear-picker choice back to a drawing orientation (#315).
fn orientation_pick_to_drawing(
    pick: crate::view_cube::OrientationPick,
) -> crate::model::DrawingOrientation {
    use crate::camera::StandardView as S;
    use crate::model::DrawingOrientation as O;
    use crate::model::{CornerView as CV, EdgeView as EV};
    use crate::view_cube::{CubeCornerId as CC, CubeEdgeId as CE};
    match pick {
        crate::view_cube::OrientationPick::Standard(v) => match v {
            S::Front => O::Front,
            S::Back => O::Back,
            S::Left => O::Left,
            S::Right => O::Right,
            S::Top => O::Top,
            S::Bottom => O::Bottom,
        },
        // A bear edge/corner click now picks that specific view (#344), not a fixed isometric.
        crate::view_cube::OrientationPick::Edge(id) => O::Edge(match id {
            CE::FrontRight => EV::FrontRight,
            CE::BackRight => EV::BackRight,
            CE::BackLeft => EV::BackLeft,
            CE::FrontLeft => EV::FrontLeft,
            CE::FrontTop => EV::FrontTop,
            CE::RightTop => EV::RightTop,
            CE::BackTop => EV::BackTop,
            CE::LeftTop => EV::LeftTop,
            CE::FrontBottom => EV::FrontBottom,
            CE::RightBottom => EV::RightBottom,
            CE::BackBottom => EV::BackBottom,
            CE::LeftBottom => EV::LeftBottom,
        }),
        crate::view_cube::OrientationPick::Corner(id) => O::Corner(match id {
            CC::FrontLeftBottom => CV::FrontLeftBottom,
            CC::FrontRightBottom => CV::FrontRightBottom,
            CC::BackRightBottom => CV::BackRightBottom,
            CC::BackLeftBottom => CV::BackLeftBottom,
            CC::FrontLeftTop => CV::FrontLeftTop,
            CC::FrontRightTop => CV::FrontRightTop,
            CC::BackRightTop => CV::BackRightTop,
            CC::BackLeftTop => CV::BackLeftTop,
        }),
        // A free-angle spin (#345) carries its own basis.
        crate::view_cube::OrientationPick::Free { right, up } => O::Free { right, up },
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// #315: the bear orientation picker's StandardView ↔ DrawingOrientation mapping round-trips
    /// for the six straight-on views, and isometric picks map to Isometric.
    #[test]
    fn orientation_bear_mappings_round_trip() {
        use crate::camera::StandardView as S;
        use crate::model::DrawingOrientation as O;
        use crate::view_cube::OrientationPick;
        for (o, s) in [
            (O::Front, S::Front),
            (O::Back, S::Back),
            (O::Left, S::Left),
            (O::Right, S::Right),
            (O::Top, S::Top),
            (O::Bottom, S::Bottom),
        ] {
            assert_eq!(drawing_orientation_to_standard(o), s);
            assert_eq!(orientation_pick_to_drawing(OrientationPick::Standard(s)), o);
        }
        // Isometric seeds to Front; a bear edge/corner click now picks that specific view (#344).
        assert_eq!(drawing_orientation_to_standard(O::Isometric), S::Front);
        assert_eq!(
            orientation_pick_to_drawing(OrientationPick::Edge(crate::view_cube::CubeEdgeId::FrontRight)),
            O::Edge(crate::model::EdgeView::FrontRight)
        );
        assert_eq!(
            orientation_pick_to_drawing(OrientationPick::Corner(
                crate::view_cube::CubeCornerId::BackLeftTop
            )),
            O::Corner(crate::model::CornerView::BackLeftTop)
        );
    }

    /// #340: every orientation maps to a bear pose highlight — a face, a corner (Isometric), or a
    /// cube edge (diagonal edge views), so the chosen view is always marked.
    #[test]
    fn orientation_to_cube_pick_covers_faces_edges_corners() {
        use crate::model::{DrawingOrientation as O, EdgeView};
        use crate::view_cube::{CubeCornerId, CubeEdgeId, CubePick};
        assert_eq!(
            drawing_orientation_to_cube_pick(O::Front),
            Some(CubePick::Face(crate::camera::StandardView::Front))
        );
        assert_eq!(
            drawing_orientation_to_cube_pick(O::Isometric),
            Some(CubePick::Corner(CubeCornerId::FrontRightTop))
        );
        assert_eq!(
            drawing_orientation_to_cube_pick(O::Edge(EdgeView::FrontRight)),
            Some(CubePick::Edge(CubeEdgeId::FrontRight))
        );
        // Every orientation resolves to some highlight.
        for o in O::ALL {
            assert!(drawing_orientation_to_cube_pick(*o).is_some(), "{o:?} has a pose");
        }
    }
    use crate::model::{Document, FaceId, Line};
    use crate::selection::click_scene_selection;

    fn input<'a>(doc: &'a Document, selection: &'a SceneSelection) -> ContextInput<'a> {
        ContextInput {
            doc,
            selection,
            tool: Tool::Select,
            in_drawing_workbench: false,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
        }
    }

    /// #257: the Default-units section is suppressed while the Repeat tool is active (its
    /// distances are plain lengths), but present for other tools.
    #[test]
    fn repeat_tool_hides_the_units_control() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let select = context_pane_content(&input(&doc, &selection));
        assert!(select.units.is_some(), "non-repeat tools still show units");
        let repeat = context_pane_content(&ContextInput {
            tool: Tool::Repeat,
            in_drawing_workbench: false,
            ..input(&doc, &selection)
        });
        assert!(repeat.units.is_none(), "Repeat tool hides the units control");
    }

    /// #329/#330: with the Text tool active, the projection editor and the Default-units section
    /// are suppressed — the pane belongs to placing/editing text, not to a projection that
    /// happens to still be selected. The Dimension tool keeps the projection editor.
    #[test]
    fn text_tool_hides_projection_editor_and_units() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let view_control = DrawingViewControl {
            view: 0,
            source: "Body 0".to_string(),
            orientation: crate::model::DrawingOrientation::Front,
            scale: String::new(),
            aligned: false,
            inline_orientations: Vec::new(),
            free_angle: false,
            source_edges: Vec::new(),
            style: crate::model::DrawingViewStyle::default(),
        };
        // Dimension tool: projection editor and units both present.
        let dim = context_pane_content(&ContextInput {
            tool: Tool::Dimension,
            in_drawing_workbench: true,
            drawing_view: Some(view_control.clone()),
            ..input(&doc, &selection)
        });
        assert!(dim.drawing_view.is_some(), "Dimension tool keeps the projection editor");
        assert!(dim.units.is_some(), "Dimension tool still shows units");
        // Text tool: both suppressed.
        let text = context_pane_content(&ContextInput {
            tool: Tool::Text,
            in_drawing_workbench: true,
            drawing_view: Some(view_control),
            ..input(&doc, &selection)
        });
        assert!(text.drawing_view.is_none(), "Text tool hides the projection editor (#329)");
        assert!(text.units.is_none(), "Text tool hides the Default-units section (#330)");
    }

    /// #328: the drawing-element picker only shows under the Select tool.
    /// #268: the Extrude tool surfaces its picked profile faces as an element picker.
    #[test]
    fn extrude_tool_surfaces_a_face_picker() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let content = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            in_drawing_workbench: false,
            extrude_faces: Some(vec!["Circle 1".to_string(), "Region 2".to_string()]),
            ..input(&doc, &selection)
        });
        assert_eq!(
            content.extrude_faces.as_deref(),
            Some(["Circle 1".to_string(), "Region 2".to_string()].as_slice())
        );
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
            in_drawing_workbench: false,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: Some(vec!["Block — vertical 0".to_string()]),
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
            icon: crate::icons::IconId::Line,
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
            in_drawing_workbench: false,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
            in_drawing_workbench: false,
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
    fn revolve_cut_mode_yields_a_red_body_picker() {
        use crate::hierarchy::SceneElement;
        let doc = Document::default();
        let selection = SceneSelection::default();
        let cut_input = ContextInput {
            tool: Tool::Revolve,
            in_drawing_workbench: false,
            revolve: Some(RevolveControl {
                face_count: 1,
                face_rows: vec!["Circle 1".to_string()],
                axis_focused: false,
                axis_label: Some("the Y axis".to_string()),
                symmetric: false,
                body_choice: crate::actions::RevolveBodyChoice::Cut,
                cut_bodies: vec![2, 5],
            }),
            ..input(&doc, &selection)
        };
        let content = context_pane_content(&cut_input);
        assert_eq!(content.tool_pickers.len(), 1);
        let view = &content.tool_pickers[0];
        assert_eq!(view.target, PickerTarget::RevolveCut);
        assert_eq!(
            view.picker.picked(),
            &[SceneElement::Body(2), SceneElement::Body(5)]
        );
        // Body-only filter, and the red "cut" highlight override in place of the default.
        assert!(view.picker.accepts(&SceneElement::Body(0)));
        assert!(!view.picker.accepts(&SceneElement::Line(0)));
        assert_eq!(
            view.picker.selected_color(crate::theme::FOCUS_ACCENT),
            crate::theme::CUT_ACCENT
        );

        // Non-Cut mode shows no tool picker.
        let new_body_input = ContextInput {
            tool: Tool::Revolve,
            in_drawing_workbench: false,
            revolve: Some(RevolveControl {
                body_choice: crate::actions::RevolveBodyChoice::NewBody,
                face_count: 1,
                face_rows: vec!["Circle 1".to_string()],
                axis_focused: false,
                axis_label: None,
                symmetric: false,
                cut_bodies: vec![],
            }),
            ..input(&doc, &selection)
        };
        assert!(context_pane_content(&new_body_input).tool_pickers.is_empty());
    }

    #[test]
    fn move_and_repeat_yield_body_pickers_without_cut_override() {
        use crate::hierarchy::SceneElement;
        let doc = Document::default();
        let selection = SceneSelection::default();

        let move_input = ContextInput {
            tool: Tool::Move,
            in_drawing_workbench: false,
            move_op: Some(MoveControl {
                targets: vec![1, 4],
                tx: String::new(),
                ty: String::new(),
                tz: String::new(),
                axis_label: None,
                angle: String::new(),
                editing: false,
                can_commit: true,
            }),
            ..input(&doc, &selection)
        };
        let pickers = context_pane_content(&move_input).tool_pickers;
        assert_eq!(pickers.len(), 1);
        assert_eq!(pickers[0].target, PickerTarget::MoveTargets);
        assert_eq!(
            pickers[0].picker.picked(),
            &[SceneElement::Body(1), SceneElement::Body(4)]
        );
        assert!(!pickers[0].picker.accepts(&SceneElement::Line(0)));
        // Move doesn't consume its bodies, so it keeps the default (non-red) highlight.
        assert_eq!(
            pickers[0].picker.selected_color(crate::theme::FOCUS_ACCENT),
            crate::theme::FOCUS_ACCENT
        );

        let repeat_input = ContextInput {
            tool: Tool::Repeat,
            in_drawing_workbench: false,
            repeat_op: Some(RepeatControl {
                targets: vec![7],
                plane_targets: Vec::new(),
                sketch_targets: Vec::new(),
                extrusion_targets: Vec::new(),
                axis_label: "the X axis".to_string(),
                mode: crate::model::RepeatMode::CountGap,
                count: "3".to_string(),
                spacing: String::new(),
                length: String::new(),
                computed_var: crate::model::RepeatVar::Distance,
                gap_is_offset: false,
                distance_is_end: true,
                computed_value: None,
                preview_instances: Some(3),
                editing: false,
                can_commit: true,
            }),
            ..input(&doc, &selection)
        };
        let pickers = context_pane_content(&repeat_input).tool_pickers;
        assert_eq!(pickers.len(), 1);
        assert_eq!(pickers[0].target, PickerTarget::RepeatTargets);
        assert_eq!(pickers[0].picker.picked(), &[SceneElement::Body(7)]);
    }

    #[test]
    fn combine_shows_one_or_two_body_pickers_by_kind() {
        use crate::hierarchy::SceneElement;
        let doc = Document::default();
        let selection = SceneSelection::default();
        let make = |kind, a: Vec<usize>, b: Vec<usize>, picking_b| ContextInput {
            tool: Tool::Combine,
            in_drawing_workbench: false,
            boolean_op: Some(BooleanControl {
                kind,
                a,
                b,
                picking_b,
                keep_b: false,
                editing: false,
                can_commit: false,
            }),
            ..input(&doc, &selection)
        };

        // Combine kind: a single side-A picker, default highlight, focused.
        let single = context_pane_content(&make(
            crate::model::BooleanOpKind::Combine,
            vec![0, 1],
            vec![],
            false,
        ))
        .tool_pickers;
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].target, PickerTarget::CombineA);
        assert!(single[0].picker.is_focused());

        // Cut kind, picking B: two pickers; B is focused and red (it gets consumed).
        let cut = context_pane_content(&make(
            crate::model::BooleanOpKind::Cut,
            vec![0],
            vec![2],
            true,
        ))
        .tool_pickers;
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].target, PickerTarget::CombineA);
        assert!(!cut[0].picker.is_focused());
        assert_eq!(cut[1].target, PickerTarget::CombineB);
        assert!(cut[1].picker.is_focused());
        assert_eq!(cut[1].picker.picked(), &[SceneElement::Body(2)]);
        assert_eq!(
            cut[1].picker.selected_color(crate::theme::FOCUS_ACCENT),
            crate::theme::CUT_ACCENT
        );
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
                extrude_faces: None,
                edge_picker: None,
                selection_picker: Some(ElementPicker::select_everything()),
                tool_pickers: Vec::new(),
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
            in_drawing_workbench: false,
            draw_rect_construction: Some(true),
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
                extrude_faces: None,
                edge_picker: None,
                selection_picker: None,
            tool_pickers: Vec::new(),
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
            in_drawing_workbench: false,
            draw_rect_construction: None,
            draw_line_construction: Some(false),
            draw_circle_construction: None,
            draw_line_curve_mode: Some(true),
            draw_line_tangent_constraint: Some(false),
            in_sketch: true,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
                extrude_faces: None,
                // #213: the Select tool surfaces the selection through the unified element picker.
                edge_picker: None,
                selection_picker: Some({
                    let mut p = ElementPicker::select_everything();
                    p.set_picked([SceneElement::Line(0)]);
                    p
                }),
                tool_pickers: Vec::new(),
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
            in_drawing_workbench: false,
            draw_rect_construction: Some(false),
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
            in_drawing_workbench: false,
            draw_rect_construction: Some(true),
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
                extrude_faces: None,
                edge_picker: None,
                selection_picker: None,
            tool_pickers: Vec::new(),
                calibrate_image: None,
                revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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
            in_drawing_workbench: false,
            draw_rect_construction: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_faces: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_add_active: false,
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