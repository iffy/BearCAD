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
    /// Rectangle anchor mode (#532): `Some` while the Rectangle tool is active.
    pub rect_anchor: Option<crate::actions::RectAnchor>,
    pub draw_line_construction: Option<bool>,
    pub draw_circle_construction: Option<bool>,
    /// Circle anchor mode: `Some` while the Circle tool is active.
    pub circle_anchor: Option<crate::actions::CircleAnchor>,
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
    /// Symmetric extrude toggle while an extrusion is in progress (#504).
    pub extrude_symmetric: Option<bool>,
    /// One label per picked extrude profile face, shown in the Extrude tool's face element
    /// picker (#268); `None` when the Extrude tool isn't active.
    pub extrude_faces: Option<Vec<String>>,
    /// The Extrude tool's in-context distance/target/commit controls (#584); `Some` while an
    /// extrusion is in progress.
    pub extrude: Option<ExtrudeControl>,
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
    /// Sweep tool state (#sweep): `Some` while the Sweep tool is active.
    pub sweep: Option<SweepControl>,
    /// Construction Plane tool state (#474): `Some` while the Plane tool is active.
    pub plane_tool: Option<PlaneToolControl>,
    /// Loft tool body-mode state (#479): `Some` while the Loft tool is active.
    pub loft_body: Option<LoftBodyControl>,
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
    /// Mirror tool state (#523): `Some` while the Mirror tool is active.
    pub mirror_op: Option<MirrorControl>,
    /// "Edit mirror" entry point: `Some(op)` when exactly one mirror operation is selected.
    pub mirror_edit_start: Option<usize>,
    /// Repeat tool state: `Some` while the Repeat tool is active.
    pub repeat_op: Option<RepeatControl>,
    /// In-sketch Repeat tool control (#232).
    pub sketch_repeat: Option<SketchRepeatControl>,
    /// In-sketch Offset tool control.
    pub sketch_offset: Option<SketchOffsetControl>,
    /// "Edit offset" entry point: the selected committed offset op.
    pub sketch_offset_edit_start: Option<usize>,
    /// In-sketch Mirror tool control (#523/#528).
    pub sketch_mirror: Option<SketchMirrorControl>,
    /// "Edit sketch mirror" entry point: the selected committed sketch-mirror op.
    pub sketch_mirror_edit_start: Option<usize>,
    /// In-sketch Slice tool control (#238).
    pub sketch_slice: Option<SketchSliceControl>,
    /// Selected sketch-text editor (#286).
    pub sketch_text: Option<SketchTextControl>,
    /// Selected drawing-projection editor (#289).
    pub drawing_view: Option<DrawingViewControl>,
    /// Selected drawing text annotation editor (#312).
    pub drawing_annotation: Option<DrawingAnnotationControl>,
    /// The Select tool's drawing element picker rows (#346): one `(drawing, element, label)` per
    /// selected projection/text/dimension, in selection order. Populated only in the drawing
    /// workbench with the Select tool active; drives the always-visible combo-box picker.
    pub drawing_selection: Vec<(usize, DrawingElementRef, String)>,
    /// The Add-view tool is active with nothing placed yet (#289): renders its pick hint.
    pub drawing_add_active: bool,
    /// The Aligned-view tool is active (#365): renders its "Base view" element picker.
    pub drawing_align_active: bool,
    /// The Aligned-view tool's current base projection `(view, label)`, if one is chosen (#365).
    pub drawing_align_base: Option<(usize, String)>,
    /// "Edit repeat" entry point.
    pub repeat_edit_start: Option<usize>,
    /// Slice tool state: `Some` while the Slice tool is active.
    pub slice_op: Option<SliceControl>,
    /// "Edit slice" entry point.
    pub slice_edit_start: Option<usize>,
    /// "Edit revolve" entry point (#211): `Some(op)` when exactly one revolution is selected.
    pub revolve_edit_start: Option<usize>,
    /// "Edit sweep" entry point: `Some(op)` when exactly one sweep is selected.
    pub sweep_edit_start: Option<usize>,
    /// Guided calibration entry point (#163): `Some(image)` when exactly one tracing image
    /// is selected and no calibration is running — renders the "Calibrate scale" button.
    pub calibrate_start: Option<usize>,
    /// Guided calibration in progress with fewer than two points placed: how many are
    /// placed so far (renders the click-two-points hint).
    pub calibrate_pending: Option<usize>,
    /// Dimension tool in 3D mode (#618): the derived-parameter name/value/commit block.
    pub dimension_derive: Option<DimensionDeriveControl>,
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

/// What the Sweep tool's context section shows (#sweep): the picked profile
/// faces, the picked path lines, the body mode, and — in Cut mode — the picked bodies.
#[derive(Clone, Debug, PartialEq)]
pub struct SweepControl {
    /// One label per picked profile face, shown in the face element picker.
    pub face_rows: Vec<String>,
    /// One label per picked path line, shown in the path element picker.
    pub path_rows: Vec<String>,
    /// Which picker shows the focus ring: Profile until a face is picked, then Path
    /// until a line is picked, then back to Profile.
    pub path_focused: bool,
    pub body_choice: crate::actions::RevolveBodyChoice,
    /// In Cut mode, the picked bodies to cut (rendered through the unified element picker).
    pub cut_bodies: Vec<usize>,
}

/// What the Construction Plane tool's context section shows (#474 / #483): the picked
/// anchor set (face; edge; or line+point — with a ✕ to clear and repick) and, for a
/// vertex where several lines/curves meet, the normal-direction choices.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaneToolControl {
    /// Anchor row labels; empty while nothing is picked yet. One row for face/edge/vertex;
    /// two rows when the set is line+point (#483).
    pub anchor_labels: Vec<String>,
    /// One label per normal candidate at a picked vertex (empty or 1 when unambiguous).
    pub normal_labels: Vec<String>,
    pub normal_choice: usize,
    /// An anchor is picked, so the offset/angle inputs and the Do button show (#611).
    pub has_anchor: bool,
    /// The anchor is an axis (edge or global axis), so an **angle** input shows alongside the
    /// offset (#613). Face/plane/vertex anchors only offset (#614).
    pub show_angle: bool,
    /// Offset expression mirroring the 3D field (#613/#614).
    pub offset_text: String,
    /// Angle expression (degrees) mirroring the 3D field, when `show_angle` (#613).
    pub angle_text: String,
    pub offset_focused: bool,
    pub angle_focused: bool,
}

/// The Loft tool's body-mode state (#479): the New/Add/Cut choice plus Cut's picked
/// bodies (rendered through the unified element picker like Revolve's).
#[derive(Clone, Debug, PartialEq)]
pub struct LoftBodyControl {
    pub body_choice: crate::actions::RevolveBodyChoice,
    pub cut_bodies: Vec<usize>,
    /// Ready to commit — at least two sections picked (#586).
    pub can_commit: bool,
}

/// One edit from the Construction Plane tool's context section (#474).
#[derive(Clone, Debug, PartialEq)]
pub enum PlaneToolEdit {
    /// Clear the picked anchor (start over).
    ClearAnchor,
    /// Anchor the plane on the `i`-th normal candidate at the picked vertex.
    NormalChoice(usize),
    /// Set the offset expression (mirrors the 3D field, #613/#614).
    SetOffset(String),
    /// Set the angle expression in degrees (mirrors the 3D field, #613).
    SetAngle(String),
    /// Focus the offset / angle field.
    FocusOffset,
    FocusAngle,
    /// Create the plane (the blue primary button / Enter, #611).
    Commit,
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

/// What the Mirror tool's context section shows (#523/#566): the mirror plane (rendered through
/// the unified element picker), the picked bodies, and whether it's an edit / ready to commit.
#[derive(Clone, Debug, PartialEq)]
pub struct MirrorControl {
    /// The picked mirror plane/face as a scene element (a construction plane or a flat body
    /// face, #566), or `None` until one is picked. Drives the plane element picker.
    pub plane: Option<SceneElement>,
    /// Picked bodies to mirror (rendered through the unified element picker).
    pub targets: Vec<usize>,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Mirror context section (#523). The plane and the picked bodies are both
/// handled through the unified element pickers (`PickerTarget::MirrorPlane` /
/// `PickerTarget::MirrorTargets`), so this only covers the commit button.
#[derive(Clone, Debug, PartialEq)]
pub enum MirrorEdit {
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
    /// Picked axis label; `None` until an axis is picked (#439).
    pub axis_label: Option<String>,
    /// Whether one of the section's value fields (Count / Offset / Distance) holds keyboard
    /// focus (#646). While it does, neither element picker reads as focused — the pane's
    /// focus ring belongs where the keyboard is, not on a picker the user isn't using.
    pub value_field_focused: bool,
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

/// The in-sketch Offset tool's context section.
#[derive(Clone, Debug, PartialEq)]
pub struct SketchOffsetControl {
    pub entity_count: usize,
    /// Lines/circles currently in the offset set (#493), for the element picker.
    pub picked: Vec<SceneElement>,
    /// Signed distance expression (positive grows a closed loop/circle).
    pub distance: String,
    pub construction: bool,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the in-sketch Offset context section.
#[derive(Clone, Debug, PartialEq)]
pub enum SketchOffsetEdit {
    Distance(String),
    Construction(bool),
    Commit,
    /// Re-open a committed offset op for editing.
    EditStart(usize),
    /// Remove one picked entity from the offset set (#493).
    Remove(SceneElement),
    /// Clear all picked entities (#493).
    Clear,
}

/// The in-sketch Mirror tool's context section (#523/#528).
#[derive(Clone, Debug, PartialEq)]
pub struct SketchMirrorControl {
    /// The picked mirror line's index, or `None` until one is chosen.
    pub line: Option<usize>,
    /// Lines/circles currently in the reflected set, for the element picker.
    pub picked: Vec<SceneElement>,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the in-sketch Mirror context section (#523/#528).
#[derive(Clone, Debug, PartialEq)]
pub enum SketchMirrorEdit {
    /// Clear the picked mirror line so a new one can be clicked.
    ClearLine,
    /// Remove one picked source from the reflected set.
    Remove(SceneElement),
    /// Clear all picked sources.
    Clear,
    Commit,
    /// Re-open a committed sketch-mirror op for editing.
    EditStart(usize),
}

/// One edit from the Repeat context section (#257): the three interlinked variables and the two
/// measurement toggles. Editing a variable marks it as one of the two "set" ones (the third is
/// then computed).
#[derive(Clone, Debug, PartialEq)]
pub enum RepeatEdit {
    /// Clear the picked axis (#439): the picker's ✕ empties it instead of resetting to X.
    ClearAxis,
    /// Grey-lock click (#443/#642): make this variable the computed one, freeing whichever
    /// was computed before to be edited.
    SetComputed(crate::model::RepeatVar),
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
    /// Whether the aligned child draws dashed projection lines to its base view (#377); only
    /// meaningful while `aligned` is true.
    pub align_lines: bool,
    /// For an aligned child (#332): the orthographic orientations it may take while staying in
    /// line with its base. Empty for a non-aligned view (or a child of an Isometric parent), which
    /// keeps the full orientation bear/picker.
    pub inline_orientations: Vec<crate::model::DrawingOrientation>,
    /// How the projection renders (#301).
    pub style: crate::model::DrawingViewStyle,
    /// Caption label state (#372): visibility, position in the card, and the custom text
    /// template (empty = the automatic caption, shown as the field's hint).
    pub label_hidden: bool,
    pub label_pos: crate::model::DrawingLabelPos,
    pub label_text: String,
    /// The automatic caption ("Body 0 — Front (1:20)"), hinted in the empty text field.
    pub auto_label: String,
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

/// The icon for a drawing element, matching the one the Elements pane uses for it (#363).
pub fn drawing_element_icon(element: DrawingElementRef) -> crate::icons::IconId {
    match element {
        DrawingElementRef::Projection(_) => crate::icons::IconId::Projection,
        DrawingElementRef::Text(_) => crate::icons::IconId::Text,
        DrawingElementRef::Dimension { .. } => crate::icons::IconId::Dimension,
    }
}

/// One edit from the drawing-annotation context section (#312).
#[derive(Clone, Debug, PartialEq)]
pub enum DrawingAnnotationEdit {
    Text(String),
    Remove,
}


/// One edit from the Select tool's drawing element picker (#346): remove one element from the
/// selection, or clear it entirely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingSelectionEdit {
    Remove(usize, DrawingElementRef),
    Clear,
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
    /// Set the projection to the current 3D viewport angle (#366).
    UseCurrentView,
    /// Show or hide an aligned child's dashed projection lines to its base view (#377).
    AlignLines(bool),
    /// Show or hide the view's caption label (#372).
    LabelHidden(bool),
    /// Move the caption label within the card (#372).
    LabelPos(crate::model::DrawingLabelPos),
    /// Override the caption text (#372); `None` returns to the automatic caption.
    LabelText(Option<String>),
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
    /// The evaluated size in mm — what the ± steppers add to / subtract from (#385).
    pub size_mm: f32,
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
    /// The blue primary button / Enter — commit the revolve (#586).
    Commit,
}

/// One edit from the Sweep context section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SweepEdit {
    BodyChoice(crate::actions::RevolveBodyChoice),
    /// Remove profile face row `i` from the face picker (`None` clears them all).
    RemoveFace(Option<usize>),
    /// Remove path line row `i` from the path picker (`None` clears them all).
    RemovePath(Option<usize>),
    /// The blue primary button / Enter — commit the sweep (#586).
    Commit,
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

/// The egui id of one of the Repeat section's value fields — the same id
/// [`crate::expression_input::ValueInput`] is built with when the row renders.
fn repeat_value_field_id(label: &str) -> egui::Id {
    egui::Id::new(("repeat_var_field", label))
}

/// Whether one of the Repeat section's Count / Offset / Distance fields holds keyboard focus
/// (#646). Both labels the gap row can carry ("Gap" and "Offset") count, since the row's id
/// follows its display label.
pub fn repeat_value_field_focused(ctx: &egui::Context) -> bool {
    let focused = ctx.memory(|m| m.focused());
    focused.is_some_and(|id| {
        ["Count", "Gap", "Offset", "Distance"]
            .iter()
            .any(|l| repeat_value_field_id(l) == id)
    })
}

/// The sketch-entity drawing tools (#636). Their context sections are identical in 3D and
/// inside a sketch — in 3D the first click just opens the sketch they draw into.
pub fn is_draw_tool(tool: Tool) -> bool {
    matches!(tool, Tool::Line | Tool::Rectangle | Tool::Circle)
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
    /// The active tool's title, shown once at the very top of the pane so every tool's context
    /// section is labelled (#608). `None` for the Select tool and the drawing workbench, which
    /// have their own selection/section headings instead.
    pub tool_title: Option<&'static str>,
    pub name: Option<NameControl>,
    /// Curve-mode (`B`) checkbox while the line tool is active (#73).
    pub curve_mode: Option<bool>,
    /// Tangent-constraint (`T`) checkbox while the line tool is active (#73).
    pub tangent_constraint: Option<bool>,
    pub construction: Option<ConstructionControl>,
    /// Rectangle anchor radio (#532): `Some` while the Rectangle tool is active.
    pub rect_anchor: Option<crate::actions::RectAnchor>,
    /// Circle anchor radio: `Some` while the Circle tool is active.
    pub circle_anchor: Option<crate::actions::CircleAnchor>,
    pub constraints: Option<Vec<ConstraintPaneRow>>,
    /// `Some(enabled)` when the current tool snaps; renders an enable/disable toggle.
    pub snapping: Option<bool>,
    /// New-body/merge-into choice for an in-progress or edited extrusion (#32).
    pub extrude_body: Option<ExtrudeBodyControl>,
    /// Picked extrude profile faces, shown as an element picker (#268).
    pub extrude_faces: Option<Vec<String>>,
    /// In-context distance/target/commit controls for the Extrude tool (#584).
    pub extrude: Option<ExtrudeControl>,
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
    /// Dimension tool in 3D mode (#618): the derived-parameter name/value/commit block,
    /// rendered right under the selection picker.
    pub dimension_derive: Option<DimensionDeriveView>,
    /// Tool-owned element pickers (#213): the sets a construction tool is gathering (e.g. the
    /// Revolve tool's cut bodies), each rendered by the same combo-box widget. Extensible: a
    /// tool may show several (Combine's A/B sides). Empty for tools not yet migrated.
    pub tool_pickers: Vec<ToolPickerView>,
    /// Image scale calibration (#171).
    pub calibrate_image: Option<CalibrateImageControl>,
    /// Revolve tool controls (#revolve).
    pub revolve: Option<RevolveControl>,
    /// Sweep tool controls (#sweep).
    pub sweep: Option<SweepControl>,
    /// Construction Plane tool state (#474): `Some` while the Plane tool is active.
    pub plane_tool: Option<PlaneToolControl>,
    /// Loft tool body-mode state (#479): `Some` while the Loft tool is active.
    pub loft_body: Option<LoftBodyControl>,
    /// Combine tool controls.
    pub boolean_op: Option<BooleanControl>,
    /// "Edit operation" button target.
    pub boolean_edit_start: Option<usize>,
    /// Move tool state: `Some` while the Move tool is active.
    pub move_op: Option<MoveControl>,
    /// "Edit move" entry point: `Some(op)` when exactly one move operation is selected.
    pub move_edit_start: Option<usize>,
    /// Mirror tool state (#523): `Some` while the Mirror tool is active.
    pub mirror_op: Option<MirrorControl>,
    /// "Edit mirror" entry point: `Some(op)` when exactly one mirror operation is selected.
    pub mirror_edit_start: Option<usize>,
    /// Repeat tool state: `Some` while the Repeat tool is active.
    pub repeat_op: Option<RepeatControl>,
    /// In-sketch Repeat tool control (#232).
    pub sketch_repeat: Option<SketchRepeatControl>,
    /// In-sketch Offset tool control.
    pub sketch_offset: Option<SketchOffsetControl>,
    /// "Edit offset" entry point: the selected committed offset op.
    pub sketch_offset_edit_start: Option<usize>,
    /// In-sketch Mirror tool control (#523/#528).
    pub sketch_mirror: Option<SketchMirrorControl>,
    /// "Edit sketch mirror" entry point: the selected committed sketch-mirror op.
    pub sketch_mirror_edit_start: Option<usize>,
    /// In-sketch Slice tool control (#238).
    pub sketch_slice: Option<SketchSliceControl>,
    /// Selected sketch-text editor (#286).
    pub sketch_text: Option<SketchTextControl>,
    /// Selected drawing-projection editor (#289).
    pub drawing_view: Option<DrawingViewControl>,
    /// Selected drawing text annotation editor (#312).
    pub drawing_annotation: Option<DrawingAnnotationControl>,
    /// The Select tool's always-visible drawing element picker (#346): `(drawing, element, label)`
    /// per selected projection/text/dimension. `Some` (possibly empty) whenever the Select tool is
    /// active in the drawing workbench.
    pub drawing_selection: Option<Vec<(usize, DrawingElementRef, String)>>,
    /// The Add-view tool is active with nothing placed yet (#289).
    pub drawing_add_active: bool,
    /// The Aligned-view tool's "Base view" picker (#365): `Some` when the tool is active; the inner
    /// option is the chosen base projection `(view, label)` or `None` while none is picked.
    pub drawing_align: Option<Option<(usize, String)>>,
    /// "Edit repeat" entry point.
    pub repeat_edit_start: Option<usize>,
    /// Slice tool controls.
    pub slice_op: Option<SliceControl>,
    /// "Edit slice" button target.
    pub slice_edit_start: Option<usize>,
    /// "Edit revolve" button target (#211).
    pub revolve_edit_start: Option<usize>,
    /// "Edit sweep" button target.
    pub sweep_edit_start: Option<usize>,
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
    /// Component this control edits (#423); mutually exclusive with `sketch`.
    pub component: Option<usize>,
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
    /// A component's overrides (#423); `None` inherits from the parent chain.
    Component {
        component: usize,
        length: Option<LengthUnit>,
        angle: Option<AngleUnit>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtrudeBodyControl {
    pub mode: ExtrudeBodyMode,
    /// Host body for Add/Cut when the sketch sits on a body face; `None` disables those modes.
    pub merge_body: Option<usize>,
    pub merge_body_label: String,
    /// Symmetric extrude (#504).
    pub symmetric: bool,
}

/// The Extrude tool's in-context distance field, extrude-to target picker, and commit button
/// (#584): a full alternative to driving the extrusion from the 3D gizmo/value field.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtrudeControl {
    /// Distance value-input text — mirrors the 3D field. **Empty ("" → null)** while an
    /// extrude-to target is set, since the depth then comes from the target plane/face.
    pub distance: String,
    /// The extrude-to target picker's rows: one label when a plane/face target is set, else empty.
    pub target_rows: Vec<String>,
    /// Whether the target picker shows the focus ring (armed so the next viewport click on a
    /// plane/face sets the target).
    pub target_focused: bool,
    /// Whether an extrusion is currently committable (at least one profile face picked).
    pub can_commit: bool,
    /// Whether an extrusion is actually in progress (a face is picked). When false the Distance and
    /// "Up to" rows are hidden but the (disabled) primary button still shows (#601).
    pub has_extrusion: bool,
}

/// Edits driven by the Extrude tool's context section (#584).
#[derive(Clone, Debug, PartialEq)]
pub enum ExtrudeEdit {
    /// The distance value-input text changed (clears any extrude-to target).
    Distance(String),
    /// The target picker was focused — arm target-pick mode.
    TargetFocus,
    /// Clear the extrude-to target (depth reverts to the distance field).
    ClearTarget,
    /// The "Extrude" button was pressed — commit the extrusion.
    Commit,
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
    /// Focus the drawing-annotation text field with everything selected (#379) — set when a
    /// page textbox is double-clicked, so typing immediately replaces its text.
    pub focus_annotation_field: bool,
    pub synced_element: Option<SceneElement>,
    /// Length draft for the image scale calibration control (#171).
    pub calibrate_length_draft: String,
    /// Which calibration span the draft was last pre-filled for (#424): the control's
    /// image + quantized endpoints. When the span changes (a point placed or dragged) the
    /// draft re-syncs to the span's current measured length.
    pub calibrate_synced: Option<(usize, [i32; 4])>,
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
    /// Whether to draw a divider above this picker. Tools whose pickers form one contiguous
    /// block with the following controls (e.g. Mirror, #602) suppress the inner dividers.
    pub separator_above: bool,
}

/// Which tool-owned set a [`ToolPickerView`]'s removals apply to. Grows as tools migrate onto
/// the unified picker; the active tool disambiguates, but this stays explicit so a tool with
/// several pickers (e.g. Combine's two sides) routes each correctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerTarget {
    /// The Revolve tool's cut bodies (`CreatingRevolve::cut_bodies`).
    RevolveCut,
    /// The Sweep tool's cut bodies (`CreatingSweep::cut_bodies`).
    SweepCut,
    /// The Loft tool's cut bodies (`CreatingLoft::cut_bodies`, #479).
    LoftCut,
    /// The Move tool's target bodies (`CreatingMove::targets`).
    MoveTargets,
    /// The Mirror tool's mirror plane (`CreatingMirror::plane`, #566): a plane or flat face.
    MirrorPlane,
    /// The Mirror tool's target bodies (`CreatingMirror::targets`, #523).
    MirrorTargets,
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

/// Derived-parameter controls for the Dimension tool in 3D mode (#618): the name box for
/// the parameter about to be recorded (owned by `AppState::dimension_param_name`).
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionDeriveControl {
    pub name_text: String,
}

/// A user edit from the Dimension tool's derived-parameter controls (#618).
#[derive(Clone, Debug, PartialEq)]
pub enum DimensionDeriveEdit {
    SetName(String),
    Commit,
}

/// Rendered state of the Dimension tool's derived-parameter block (#618): the measured
/// value of the current selection (one line → its length; two parallel lines → the
/// distance between them; two non-parallel lines → the angle; two vertices → the
/// distance), formatted for display, and whether "Derive parameter" can fire.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionDeriveView {
    pub name_text: String,
    pub value: Option<String>,
    pub can_commit: bool,
}

/// The selection element picker to show for `tool`, if any — the unified control every
/// selection-driven tool uses. Both variants mirror the live `selection`; they differ only in
/// which kinds they accept and their placeholder, demonstrating the per-instance configuration.
fn selection_picker_for(
    tool: Tool,
    in_sketch: bool,
    selection: &SceneSelection,
) -> Option<ElementPicker> {
    let mut picker = match tool {
        // Select: accepts everything, always shown, never loses focus.
        Tool::Select => ElementPicker::select_everything(),
        // Constraint / Dimension: sketch geometry only (points, lines, circles, body/face
        // edges). Dimension's picker mirrors the live selection so a pre-selected line or
        // pair shows up and the tool can proceed as if those were just picked (#486).
        Tool::Constraint | Tool::Dimension if in_sketch => {
            let mut p = ElementPicker::new(
                ElementFilter::kinds(&[
                    ElementKind::Vertex,
                    ElementKind::Line,
                    ElementKind::Circle,
                    ElementKind::Edge,
                ]),
                PickLimit::Infinite,
            );
            p.set_focused(true);
            p
        }
        // Dimension outside a sketch: lines / points for derived measures (#499).
        Tool::Dimension if !in_sketch => {
            let mut p = ElementPicker::new(
                ElementFilter::kinds(&[
                    ElementKind::Line,
                    ElementKind::Vertex,
                    ElementKind::Edge,
                ]),
                PickLimit::Finite(2),
            );
            p.set_focused(true);
            p
        }
        // Chamfer/Fillet in-sketch: vertices only (#492).
        Tool::Chamfer | Tool::Fillet if in_sketch => {
            let mut p = ElementPicker::new(
                ElementFilter::kind(ElementKind::Vertex),
                PickLimit::Infinite,
            );
            p.set_focused(true);
            p
        }
        // Sketch / Text outside a sketch: pick a single face plane to open (#497).
        Tool::Sketch | Tool::Text if !in_sketch => {
            let mut p = ElementPicker::new(
                ElementFilter::kind(ElementKind::Plane),
                PickLimit::Finite(1),
            );
            p.set_focused(true);
            p
        }
        // Project in a sketch: points, lines, edges (#498).
        Tool::Project if in_sketch => {
            let mut p = ElementPicker::new(
                ElementFilter::kinds(&[
                    ElementKind::Vertex,
                    ElementKind::Line,
                    ElementKind::Edge,
                    ElementKind::Body,
                ]),
                PickLimit::Infinite,
            );
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
    selected_color: Option<eframe::egui::Color32>,
    focused: bool,
) -> ToolPickerView {
    let mut picker =
        ElementPicker::new(ElementFilter::kind(ElementKind::Body), PickLimit::Infinite);
    if let Some(color) = selected_color {
        picker = picker.with_selected_color(color);
    }
    picker.set_focused(focused);
    picker.set_picked(bodies.iter().map(|&bi| SceneElement::Body(bi)));
    ToolPickerView {
        heading,
        picker,
        target,
        separator_above: true,
    }
}

/// The active tool's title for the top of the context pane (#608). Every modelling/sketch tool
/// gets a title; the Select tool and drawing workbench return `None` (they show selection info
/// or their own section headings). The "Edit …" variants surface when a committed operation is
/// being re-edited through its tool.
fn tool_context_title(input: &ContextInput<'_>) -> Option<&'static str> {
    use crate::actions::Tool;
    // The drawing workbench has its own titled sections (View / Add view), not a tool title.
    if input.in_drawing_workbench {
        return None;
    }
    let editing = input.move_op.as_ref().is_some_and(|c| c.editing)
        || input.mirror_op.as_ref().is_some_and(|c| c.editing)
        || input.boolean_op.as_ref().is_some_and(|c| c.editing)
        || input.repeat_op.as_ref().is_some_and(|c| c.editing)
        || input.slice_op.as_ref().is_some_and(|c| c.editing)
        || input.sketch_offset.as_ref().is_some_and(|c| c.editing);
    Some(match input.tool {
        Tool::Select => return None,
        Tool::Rectangle => "Rectangle",
        Tool::Line => "Line",
        Tool::Circle => "Circle",
        Tool::ConstructionPlane => "Construction plane",
        Tool::Sketch => "Sketch",
        Tool::Dimension => "Dimension",
        Tool::Constraint => "Constraint",
        Tool::Extrude => "Extrude",
        Tool::Chamfer => "Chamfer",
        Tool::Fillet => "Fillet",
        Tool::Offset => {
            if editing {
                "Edit offset"
            } else {
                "Offset"
            }
        }
        Tool::Project => "Project",
        Tool::Loft => "Loft",
        Tool::Revolve => "Revolve",
        Tool::Sweep => "Sweep",
        Tool::Combine => {
            if editing {
                "Edit boolean operation"
            } else {
                "Combine"
            }
        }
        Tool::Move => {
            if editing {
                "Edit move"
            } else {
                "Move"
            }
        }
        Tool::Mirror => {
            if editing {
                "Edit mirror"
            } else {
                "Mirror"
            }
        }
        Tool::Repeat => match (input.in_sketch, editing) {
            (true, _) => "Repeat (in sketch)",
            (false, true) => "Edit repeat",
            (false, false) => "Linear repeat",
        },
        Tool::Slice => match (input.in_sketch, editing) {
            (true, true) => "Edit slice",
            (true, false) => "Slice (in sketch)",
            (false, true) => "Edit slice",
            (false, false) => "Slice",
        },
        Tool::Text => "Text",
        Tool::DrawingAdd | Tool::DrawingAlign => return None,
    })
}

pub fn context_pane_content(input: &ContextInput<'_>) -> ContextPaneContent {
    let tool_title = tool_context_title(input);
    let name = single_nameable_from_selection(input.selection).map(|element| NameControl { element });
    // Snapping shows for the drawing tools in 3D as well as in a sketch (#636): the
    // Rectangle/Line/Circle sections read identically either way, and the toggle is sticky,
    // so setting it in 3D carries into the sketch the first click opens. The Select tool
    // keeps its sketch-only toggle — there's nothing to snap while picking in 3D.
    let snapping = (tool_uses_snapping(input.tool)
        && (input.in_sketch || is_draw_tool(input.tool)))
    .then_some(input.snapping_enabled);
    // #505: always show New/Add/Cut while extruding (Add/Cut need a host body candidate).
    let extrude_body = input.extrude_body_mode.map(|mode| {
        let merge_body = input.extrude_merge_candidate;
        let merge_body_label = merge_body
            .and_then(|bi| element_name(input.doc, SceneElement::Body(bi)).map(|n| n.to_string()))
            .unwrap_or_else(|| "body".to_string());
        ExtrudeBodyControl {
            mode,
            merge_body,
            merge_body_label,
            symmetric: input.extrude_symmetric.unwrap_or(false),
        }
    });
    let extrude_faces = input.extrude_faces.clone();
    let extrude = input.extrude.clone();
    // The Default-units section is only relevant to selection/sketch editing, not to the modeling,
    // transform, dimension, or constraint tools whose own busy context sections don't need it
    // (#257/#330/#585). It's suppressed while any of those tools is active.
    let units_suppressed = matches!(
        input.tool,
        Tool::Repeat
            | Tool::Text
            | Tool::Extrude
            | Tool::Sweep
            | Tool::Loft
            | Tool::Revolve
            | Tool::Combine
            | Tool::Move
            | Tool::Mirror
            | Tool::Slice
            | Tool::Dimension
            | Tool::Constraint
    );
    let units = (!units_suppressed)
        .then(|| units_control_from_selection(input.doc, input.selection))
        .flatten();
    let edge_picker = input
        .edge_treatment_rows
        .clone()
        .map(|rows| EdgePickerControl {
            heading: "Edges",
            icon: crate::icons::IconId::Line,
            rows,
        })
        .or_else(|| {
            input.loft_rows.clone().map(|rows| EdgePickerControl {
                heading: "Sections",
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
        .then(|| selection_picker_for(input.tool, input.in_sketch, input.selection))
        .flatten();
    // Dimension tool in 3D (#618): measure the current selection for the derive block —
    // one line → its length; two parallel lines → the distance between them; two
    // non-parallel lines → the angle; two vertices → the distance.
    let dimension_derive = input.dimension_derive.as_ref().map(|c| {
        let source =
            crate::parameters::derived_source_from_selection(input.doc, input.selection);
        let value = source.as_ref().and_then(|s| {
            crate::parameters::derived_source_value(input.doc, s).map(|(v, is_angle)| {
                if is_angle {
                    crate::value::format_angle_display_in(
                        v.to_radians(),
                        input.doc.default_angle_unit,
                    )
                } else {
                    crate::value::format_length_display_in(v, input.doc.default_length_unit)
                }
            })
        });
        DimensionDeriveView {
            name_text: c.name_text.clone(),
            can_commit: value.is_some(),
            value,
        }
    });
    // The drawing workbench's Select tool gets its own always-visible element picker (#346),
    // mirroring the multi-selection of projections/text/dimensions.
    let drawing_selection = (input.in_drawing_workbench && input.tool == Tool::Select)
        .then(|| input.drawing_selection.clone());
    // The Aligned-view tool shows a "Base view" picker (#365) for the projection to align to.
    let drawing_align = input.drawing_align_active.then(|| input.drawing_align_base.clone());
    // Tool-owned element pickers (#213). Each is a Body-filtered picker built from the tool's
    // in-progress set. Bodies consumed destructively (Revolve cut) get the red highlight override.
    let mut tool_pickers = Vec::new();
    if let Some(r) = input.revolve.as_ref() {
        if r.body_choice == crate::actions::RevolveBodyChoice::Cut {
            tool_pickers.push(body_tool_picker(
                "Cut bodies",
                PickerTarget::RevolveCut,
                &r.cut_bodies,
                    Some(crate::theme::CUT_ACCENT),
                true,
            ));
        }
    }
    if let Some(f) = input.sweep.as_ref() {
        if f.body_choice == crate::actions::RevolveBodyChoice::Cut {
            tool_pickers.push(body_tool_picker(
                "Cut bodies",
                PickerTarget::SweepCut,
                &f.cut_bodies,
                Some(crate::theme::CUT_ACCENT),
                true,
            ));
        }
    }
    if let Some(l) = input.loft_body.as_ref() {
        if l.body_choice == crate::actions::RevolveBodyChoice::Cut {
            tool_pickers.push(body_tool_picker(
                "Cut bodies",
                PickerTarget::LoftCut,
                &l.cut_bodies,
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
            None,
            true,
        ));
    }
    if let Some(m) = input.mirror_op.as_ref() {
        // Primary picker: the mirror plane — a construction plane or a flat body face (#566).
        // Single-pick, and focused (the pick target) until a plane is chosen.
        let mut plane_picker = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Plane, ElementKind::Face]),
            PickLimit::Finite(1),
        );
        plane_picker.set_focused(m.plane.is_none());
        if let Some(element) = m.plane.clone() {
            plane_picker.set_picked([element]);
        }
        tool_pickers.push(ToolPickerView {
            heading: "Mirror plane",
            picker: plane_picker,
            target: PickerTarget::MirrorPlane,
            separator_above: true,
        });
        // Secondary picker: the bodies picker reads as focused only once a mirror plane is
        // chosen — the plane is the first pick (#523). No divider between the plane picker,
        // the bodies picker, and the Do button — they read as one Mirror block (#602).
        let mut bodies = body_tool_picker(
            "Bodies",
            PickerTarget::MirrorTargets,
            &m.targets,
            None,
            m.plane.is_some(),
        );
        bodies.separator_above = false;
        tool_pickers.push(bodies);
    }
    if let Some(r) = input.repeat_op.as_ref() {
        // Only one Repeat picker reads as focused (#439): the axis while it's unset and
        // there's already something to repeat (the axis is the next pick), the bodies
        // otherwise. Typing in Count/Offset/Distance blurs both (#646) — the pane's focus
        // ring should sit where the keyboard is, not on a picker the user isn't using.
        let has_targets = !r.targets.is_empty()
            || !r.plane_targets.is_empty()
            || !r.sketch_targets.is_empty()
            || !r.extrusion_targets.is_empty();
        let axis_is_next = r.axis_label.is_none() && has_targets;
        tool_pickers.push(body_tool_picker(
            "Bodies",
            PickerTarget::RepeatTargets,
            &r.targets,
            None,
            !axis_is_next && !r.value_field_focused,
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
            None,
            !b.picking_b,
        ));
        if two_sided {
            // No divider between the two sides and the mode/Do controls below — the Combine
            // pickers and the section read as one contiguous block (#606).
            let mut side_b = body_tool_picker(
                "Side B",
                PickerTarget::CombineB,
                &b.b,
                    (b.kind == crate::model::BooleanOpKind::Cut).then_some(crate::theme::CUT_ACCENT),
                b.picking_b,
            );
            side_b.separator_above = false;
            tool_pickers.push(side_b);
        }
    }
    let calibrate_image = input.calibrate_image;
    let revolve = input.revolve.clone();
    let sweep = input.sweep.clone();
    let plane_tool = input.plane_tool.clone();
    let loft_body = input.loft_body.clone();
    let boolean_op = input.boolean_op.clone();
    let boolean_edit_start = input.boolean_edit_start;
    let move_op = input.move_op.clone();
    let move_edit_start = input.move_edit_start;
    let mirror_op = input.mirror_op.clone();
    let mirror_edit_start = input.mirror_edit_start;
    let repeat_op = input.repeat_op.clone();
    let sketch_repeat = input.sketch_repeat.clone();
    let sketch_offset = input.sketch_offset.clone();
    let sketch_offset_edit_start = input.sketch_offset_edit_start;
    let sketch_mirror = input.sketch_mirror.clone();
    let sketch_mirror_edit_start = input.sketch_mirror_edit_start;
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
    let sweep_edit_start = input.sweep_edit_start;
    let calibrate_start = input.calibrate_start;
    let calibrate_pending = input.calibrate_pending;

    if let Some(construction) = input.draw_rect_construction {
        return ContextPaneContent {
            tool_title,
            name,
            curve_mode: None,
            rect_anchor: input.rect_anchor,
            circle_anchor: input.circle_anchor,
            tangent_constraint: None,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            snapping,
            extrude_body,
            extrude_faces: extrude_faces.clone(),
            extrude: extrude.clone(),
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            dimension_derive: None,
            tool_pickers: Vec::new(),
            calibrate_image,
            revolve: revolve.clone(),
            sweep: sweep.clone(),
            plane_tool: plane_tool.clone(),
            loft_body: loft_body.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            mirror_op: mirror_op.clone(),
            mirror_edit_start,
            repeat_op: repeat_op.clone(),
            sketch_repeat: sketch_repeat.clone(),
            sketch_offset: sketch_offset.clone(),
            sketch_offset_edit_start,
            sketch_mirror: sketch_mirror.clone(),
            sketch_mirror_edit_start,
            sketch_slice: sketch_slice.clone(),
            sketch_text: sketch_text.clone(),
            drawing_view: drawing_view.clone(),
            drawing_annotation: drawing_annotation.clone(),
            drawing_selection: None,
            drawing_align: None,
            drawing_add_active,
            repeat_edit_start,
            slice_op: slice_op.clone(),
            slice_edit_start,
            revolve_edit_start,
            sweep_edit_start,
        calibrate_start,
            calibrate_pending,
        };
    }
    if let Some(construction) = input.draw_line_construction {
        return ContextPaneContent {
            tool_title,
            name,
            curve_mode: input.draw_line_curve_mode,
            rect_anchor: input.rect_anchor,
            circle_anchor: input.circle_anchor,
            tangent_constraint: input.draw_line_tangent_constraint,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            snapping,
            extrude_body,
            extrude_faces: extrude_faces.clone(),
            extrude: extrude.clone(),
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            dimension_derive: None,
            tool_pickers: Vec::new(),
            calibrate_image,
            revolve: revolve.clone(),
            sweep: sweep.clone(),
            plane_tool: plane_tool.clone(),
            loft_body: loft_body.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            mirror_op: mirror_op.clone(),
            mirror_edit_start,
            repeat_op: repeat_op.clone(),
            sketch_repeat: sketch_repeat.clone(),
            sketch_offset: sketch_offset.clone(),
            sketch_offset_edit_start,
            sketch_mirror: sketch_mirror.clone(),
            sketch_mirror_edit_start,
            sketch_slice: sketch_slice.clone(),
            sketch_text: sketch_text.clone(),
            drawing_view: drawing_view.clone(),
            drawing_annotation: drawing_annotation.clone(),
            drawing_selection: None,
            drawing_align: None,
            drawing_add_active,
            repeat_edit_start,
            slice_op: slice_op.clone(),
            slice_edit_start,
            revolve_edit_start,
            sweep_edit_start,
        calibrate_start,
            calibrate_pending,
        };
    }
    if let Some(construction) = input.draw_circle_construction {
        return ContextPaneContent {
            tool_title,
            name,
            curve_mode: None,
            // The Anchor row (centre+radius vs edge-to-edge) rides along here (#635) — it
            // used to be dropped, hiding a mode that `O` could still toggle blind.
            rect_anchor: input.rect_anchor,
            circle_anchor: input.circle_anchor,
            tangent_constraint: None,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            snapping,
            extrude_body,
            extrude_faces: extrude_faces.clone(),
            extrude: extrude.clone(),
            units,
            edge_picker: edge_picker.clone(),
            selection_picker: None,
            dimension_derive: None,
            tool_pickers: Vec::new(),
            calibrate_image,
            revolve: revolve.clone(),
            sweep: sweep.clone(),
            plane_tool: plane_tool.clone(),
            loft_body: loft_body.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            mirror_op: mirror_op.clone(),
            mirror_edit_start,
            repeat_op: repeat_op.clone(),
            sketch_repeat: sketch_repeat.clone(),
            sketch_offset: sketch_offset.clone(),
            sketch_offset_edit_start,
            sketch_mirror: sketch_mirror.clone(),
            sketch_mirror_edit_start,
            sketch_slice: sketch_slice.clone(),
            sketch_text: sketch_text.clone(),
            drawing_view: drawing_view.clone(),
            drawing_annotation: drawing_annotation.clone(),
            drawing_selection: None,
            drawing_align: None,
            drawing_add_active,
            repeat_edit_start,
            slice_op: slice_op.clone(),
            slice_edit_start,
            revolve_edit_start,
            sweep_edit_start,
        calibrate_start,
            calibrate_pending,
        };
    }

    // The Dimension tool in 3D measures the selection (#618) — its pane is the
    // derived-parameter block, not per-entity editing, so no Construction toggle (#630).
    let targets = if input.tool == Tool::Dimension && !input.in_sketch {
        Vec::new()
    } else {
        construction_targets_from_selection(input.selection)
    };
    let constraints = (input.tool == Tool::Constraint)
        .then(|| constraint_pane_rows(input.selection));
    ContextPaneContent {
        tool_title,
        name,
        curve_mode: None,
        rect_anchor: input.rect_anchor,
        circle_anchor: input.circle_anchor,
        tangent_constraint: None,
        construction: (!targets.is_empty()).then(|| ConstructionControl {
            value: construction_tri_state(input.doc, &targets),
            target_count: targets.len(),
        }),
        constraints,
        snapping,
        extrude_body,
        extrude_faces: extrude_faces.clone(),
        extrude: extrude.clone(),
        units,
        edge_picker,
        selection_picker,
        dimension_derive,
        tool_pickers,
        calibrate_image,
        revolve,
        sweep,
        plane_tool,
        loft_body,
        boolean_op,
        boolean_edit_start,
        move_op,
        move_edit_start,
        mirror_op,
        mirror_edit_start,
        repeat_op,
        sketch_repeat,
        sketch_offset,
        sketch_offset_edit_start,
        sketch_mirror,
        sketch_mirror_edit_start,
        sketch_slice,
        sketch_text,
        drawing_view,
        drawing_annotation,
        drawing_selection,
        drawing_align,
        drawing_add_active,
        repeat_edit_start,
        slice_op,
        slice_edit_start,
        revolve_edit_start,
        sweep_edit_start,
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
            component: None,
            effective_length: doc.default_length_unit,
            effective_angle: doc.default_angle_unit,
            length_override: None,
            angle_override: None,
            document_length: doc.default_length_unit,
            document_angle: doc.default_angle_unit,
        });
    }
    // A selected component gets its own units picker (#423): overrides inherit through the
    // parent chain to the document.
    if let Some(SceneElement::Component(ci)) = selection.single() {
        let component = doc.components.get(ci).filter(|c| !c.deleted)?;
        return Some(UnitsControl {
            sketch: None,
            component: Some(ci),
            effective_length: crate::model::effective_component_length_unit(doc, ci),
            effective_angle: crate::model::effective_component_angle_unit(doc, ci),
            length_override: component.length_unit,
            angle_override: component.angle_unit,
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
                component: None,
        effective_length: crate::model::effective_length_unit(doc, id),
        effective_angle: crate::model::effective_angle_unit(doc, id),
        length_override: sketch.length_unit,
        angle_override: sketch.angle_unit,
        document_length: doc.default_length_unit,
        document_angle: doc.default_angle_unit,
    })
}

/// Pre-fill the calibration length draft with the marked span's current measured length
/// (#424), re-syncing whenever the span changes (a point placed, dragged, or a different
/// image). A calibrated image's span measures its declared length, so re-opening shows it.
pub fn sync_calibrate_draft(
    state: &mut ContextPaneState,
    doc: &Document,
    content: &ContextPaneContent,
) {
    let Some(control) = &content.calibrate_image else {
        state.calibrate_synced = None;
        return;
    };
    let q = |v: f32| (v * 100.0).round() as i32;
    let key = (control.image, [q(control.a.0), q(control.a.1), q(control.b.0), q(control.b.1)]);
    if state.calibrate_synced == Some(key) {
        return;
    }
    let span = ((control.b.0 - control.a.0).powi(2) + (control.b.1 - control.a.1).powi(2)).sqrt();
    state.calibrate_length_draft = crate::value::format_length_display_in(
        span,
        doc.default_length_unit,
    );
    state.calibrate_synced = Some(key);
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

/// Lazily register `family`'s regular face with egui so its name can render **in that font**
/// in the font chooser (#384), returning the egui family to use. Fonts load on first sight
/// (the chooser virtualizes its rows, so only families scrolled into view load) and stay
/// registered for the session; a family whose face can't load renders in the default font
/// and isn't retried.
fn preview_font_family(ctx: &egui::Context, family: &str) -> Option<egui::FontFamily> {
    use std::collections::HashMap;
    // `None` = the face failed to load (never retried); `Some(pass)` = registered via
    // `set_fonts` during that pass. The family only becomes *usable* on a later pass —
    // laying out text in a family the atlas doesn't know yet panics inside egui (#392), so
    // the first frame renders the default font and repaints.
    thread_local! {
        static REGISTRY: std::cell::RefCell<(egui::FontDefinitions, HashMap<String, Option<u64>>)> =
            std::cell::RefCell::new((egui::FontDefinitions::default(), HashMap::new()));
    }
    REGISTRY.with(|reg| {
        let mut reg = reg.borrow_mut();
        let pass = ctx.cumulative_pass_nr();
        if let Some(state) = reg.1.get(family) {
            return match state {
                Some(registered) if pass > *registered => {
                    Some(egui::FontFamily::Name(family.into()))
                }
                Some(_) => {
                    ctx.request_repaint();
                    None
                }
                None => None,
            };
        }
        let Some((bytes, index)) = crate::text::font_bytes_indexed(family, false, false) else {
            reg.1.insert(family.to_string(), None);
            return None;
        };
        // Only register faces egui's own parser accepts (#392): an unparseable face would
        // panic inside the glyph-atlas build, taking the app down on the next frame.
        if ab_glyph::FontRef::try_from_slice_and_index(&bytes, index).is_err() {
            reg.1.insert(family.to_string(), None);
            return None;
        }
        // Carry the face index (#392): many macOS families live in .ttc collections, and
        // registering the collection as face 0 renders (or fails on) the wrong face.
        let key = format!("preview:{family}");
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        reg.0.font_data.insert(key.clone(), std::sync::Arc::new(data));
        // The family's own face first, then the default proportional stack so glyphs the
        // face lacks still render.
        let mut stack = vec![key];
        if let Some(default) = reg.0.families.get(&egui::FontFamily::Proportional) {
            stack.extend(default.iter().cloned());
        }
        reg.0.families.insert(egui::FontFamily::Name(family.into()), stack);
        ctx.set_fonts(reg.0.clone());
        reg.1.insert(family.to_string(), Some(pass));
        ctx.request_repaint();
        None
    })
}

/// The **primary button** (#586): the blue, no-text commit button that a tool's context section
/// shows to complete its action. It sits in the **right column** of the 2-column layout (empty
/// label) and also fires on **Enter** — but only while `enabled` and no widget has the keyboard, so
/// Enter goes to a focused field first. `enabled` is the tool's "ready" flag (all inputs valid);
/// when not ready the button stays visible but disabled. Returns true when it should commit.
fn primary_button(ui: &mut egui::Ui, enabled: bool, tooltip: &str) -> bool {
    let clicked = labeled_row(ui, "", |ui| {
        let blue = egui::Color32::from_rgb(56, 120, 224);
        let img = egui::Image::new(crate::icons::sized_texture_at(
            ui.ctx(),
            crate::icons::IconId::Confirm,
            16.0,
        ));
        // Fill the whole right column (#598).
        let w = ui.available_width().max(56.0);
        ui.add_enabled(
            enabled,
            egui::Button::image(img)
                .fill(blue)
                .min_size(egui::vec2(w, 24.0)),
        )
        .on_hover_text(format!("{tooltip} (Enter)"))
        .clicked()
    });
    let enter = enabled
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
        && ui.memory(|m| m.focused().is_none());
    clicked || enter
}

/// A primary action button with a **visible text label** (#629) — the same blue fill and
/// Enter-fires-it behavior as [`primary_button`], for actions whose name should read
/// without hovering (e.g. "Derive parameter").
fn primary_text_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> bool {
    let clicked = labeled_row(ui, "", |ui| {
        let blue = egui::Color32::from_rgb(56, 120, 224);
        let w = ui.available_width().max(56.0);
        ui.add_enabled(
            enabled,
            egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE))
                .fill(blue)
                .min_size(egui::vec2(w, 24.0)),
        )
        .on_hover_text(format!("{label} (Enter)"))
        .clicked()
    });
    let enter = enabled
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
        && ui.memory(|m| m.focused().is_none());
    clicked || enter
}

/// A faint section heading (#393): quieter than the field labels beneath it, so sections
/// read as grouping rather than competing with the label column.
fn section_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .color(egui::Color32::from_gray(130))
            .size(11.5),
    );
}

/// Width of the context pane's label column (#371): every label+input pair renders as a
/// two-column row — the label left-aligned in this fixed column, the input in the aligned
/// right column — so inputs line up down the whole pane.
const FIELD_LABEL_W: f32 = 78.0;

/// A two-column field row (#371): `label` in the fixed-width left column (vertically centred
/// against the input), the input(s) from `add_input` in the aligned right column.
fn labeled_row<R>(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    add_input: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let label = label.into();
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(FIELD_LABEL_W, 18.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                // The parent advances by the *used* rect, so pin the column width — and
                // wrap labels wider than it (#632: "Parameter name") onto a second line,
                // so every row's input starts at the same x.
                ui.set_min_size(egui::vec2(FIELD_LABEL_W, 18.0));
                ui.set_max_width(FIELD_LABEL_W);
                ui.add(egui::Label::new(label).wrap());
            },
        );
        add_input(ui)
    })
    .inner
}

/// A two-column **checkbox row** (#588): `label` (with an optional keyboard-shortcut hint) in the
/// left column, the checkbox in the right column. **Clicking either** the label or the box toggles
/// it — the whole left column is a click target. Returns whether the value changed.
fn checkbox_row(
    ui: &mut egui::Ui,
    label: &str,
    checked: &mut bool,
    shortcut: Option<crate::shortcuts::ShortcutHint>,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        // Left column: the clickable label.
        let resp = ui
            .allocate_ui_with_layout(
                egui::vec2(FIELD_LABEL_W, 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(egui::vec2(FIELD_LABEL_W, 18.0));
                    ui.add(egui::Label::new(label).sense(egui::Sense::click()))
                },
            )
            .inner;
        if resp.clicked() {
            *checked = !*checked;
            changed = true;
        }
        // Right column: the checkbox, with the shortcut hint to its **right** (#597).
        if ui.checkbox(checked, "").changed() {
            changed = true;
        }
        if let Some(hint) = shortcut {
            ui.add(egui::Label::new(
                egui::RichText::new(crate::shortcuts::format_shortcut(hint))
                    .weak()
                    .monospace()
                    .size(11.0),
            ));
        }
    });
    changed
}

/// A field label that is itself a click target (#640): it tints gold on hover, exactly like the
/// [`crate::icons::icon_button_hover_gold`] toggle beside it, so the label and the icon read as
/// one control. Used where a row's label names a mode the click cycles.
fn clickable_label(
    ui: &mut egui::Ui,
    label: &str,
    tooltip: impl Into<egui::WidgetText>,
) -> egui::Response {
    let hovered = ui
        .ctx()
        .read_response(ui.next_auto_id())
        .is_some_and(|r| r.hovered());
    let text = if hovered {
        egui::RichText::new(label).color(HOVER_GOLD)
    } else {
        egui::RichText::new(label)
    };
    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

/// The gold an interactive-on-hover control tints to (#440), shared by the icon toggles and the
/// clickable labels beside them (#640).
const HOVER_GOLD: egui::Color32 = egui::Color32::from_rgb(255, 210, 90);

/// [`labeled_row`] for tall inputs (pickers, multiline text): the label top-aligns with the
/// input, centred against its **first row** — 26 px, the height of an element picker's
/// collapsed strip (frame margins + one text row), so the label lines up with the picker's
/// own text (#387) and with a text area's first line.
fn labeled_row_top<R>(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    add_input: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let label = label.into();
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(FIELD_LABEL_W, 26.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                // The parent advances by the *used* rect, so pin the column width; wide
                // labels wrap within it (#632) so every input starts at the same x.
                ui.set_min_size(egui::vec2(FIELD_LABEL_W, 26.0));
                ui.set_max_width(FIELD_LABEL_W);
                ui.add(egui::Label::new(label).wrap());
            },
        );
        ui.vertical(add_input)
    })
    .inner
    .inner
}

/// One row of the extrude "into" picker (#32/#35): the mode's icon followed by a radio button.
/// Selecting the radio mutates `current`, which the caller diffs to fire the change callback.
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
    on_rect_anchor_changed: &mut impl FnMut(crate::actions::RectAnchor),
    on_circle_anchor_changed: &mut impl FnMut(crate::actions::CircleAnchor),
    on_constraint_clicked: &mut impl FnMut(crate::geometric_constraints::GeometricConstraintType),
    on_snapping_changed: &mut impl FnMut(bool),
    on_extrude_body_mode_changed: &mut impl FnMut(ExtrudeBodyMode),
    on_extrude_symmetric_changed: &mut impl FnMut(bool),
    on_extrude_face_remove: &mut impl FnMut(Option<usize>),
    on_extrude_edit: &mut impl FnMut(ExtrudeEdit),
    on_units_changed: &mut impl FnMut(UnitsChoice),
    on_edge_picker_edit: &mut impl FnMut(Option<usize>),
    on_selection_edit: &mut impl FnMut(SelectionEdit),
    on_tool_picker_edit: &mut impl FnMut(PickerTarget, ToolPickerAction),
    on_revolve_edit: &mut impl FnMut(RevolveEdit),
    on_sweep_edit: &mut impl FnMut(SweepEdit),
    on_plane_tool_edit: &mut impl FnMut(PlaneToolEdit),
    on_loft_body_choice: &mut impl FnMut(crate::actions::RevolveBodyChoice),
    on_loft_commit: &mut impl FnMut(),
    on_boolean_edit: &mut impl FnMut(BooleanEdit),
    on_boolean_edit_start: &mut impl FnMut(usize),
    on_move_edit: &mut impl FnMut(MoveEdit),
    on_move_edit_start: &mut impl FnMut(usize),
    on_mirror_edit: &mut impl FnMut(MirrorEdit),
    on_mirror_edit_start: &mut impl FnMut(usize),
    on_repeat_edit: &mut impl FnMut(RepeatEdit),
    on_sketch_repeat_edit: &mut impl FnMut(SketchRepeatEdit),
    on_sketch_offset_edit: &mut impl FnMut(SketchOffsetEdit),
    on_sketch_mirror_edit: &mut impl FnMut(SketchMirrorEdit),
    on_sketch_slice_edit: &mut impl FnMut(SketchSliceEdit),
    on_sketch_text_edit: &mut impl FnMut(SketchTextEdit),
    on_drawing_view_edit: &mut impl FnMut(DrawingViewEdit),
    on_drawing_annotation_edit: &mut impl FnMut(DrawingAnnotationEdit),
    on_drawing_selection_edit: &mut impl FnMut(DrawingSelectionEdit),
    on_drawing_align_clear: &mut impl FnMut(),
    on_repeat_edit_start: &mut impl FnMut(usize),
    on_slice_edit: &mut impl FnMut(SliceEdit),
    on_slice_edit_start: &mut impl FnMut(usize),
    on_revolve_edit_start: &mut impl FnMut(usize),
    on_sweep_edit_start: &mut impl FnMut(usize),
    on_calibrate_start: &mut impl FnMut(usize),
    on_calibrate_image: &mut impl FnMut(CalibrateImageControl, String),
    on_dimension_derive_edit: &mut impl FnMut(DimensionDeriveEdit),
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

    // Every tool's context section is headed by the tool's title at the very top of the pane,
    // above its pickers and controls (#608). The per-tool blocks below no longer draw their own
    // section labels — this single title covers them all.
    if let Some(title) = content.tool_title {
        any_control = true;
        section_label(ui, title);
    }

    // The element picker is the primary control for the Select tool, so it renders first (#246).
    // Pickers render as label-left / picker-right rows (#371), like every other field.
    if let Some(picker) = &content.selection_picker {
        any_control = true;
        labeled_row_top(ui, "Selection", |ui| {
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
        });
    }

    // Dimension tool in 3D mode (#618): name the measurement, see its current value, and
    // record it as a read-only derived parameter.
    if let Some(control) = &content.dimension_derive {
        any_control = true;
        labeled_row(ui, "Parameter name", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                let mut text = control.name_text.clone();
                let resp =
                    ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0));
                if resp.changed() {
                    on_dimension_derive_edit(DimensionDeriveEdit::SetName(text));
                }
            });
        });
        labeled_row(ui, "Value", |ui| {
            match &control.value {
                Some(value) => ui.label(value.clone()),
                None => ui.label(
                    egui::RichText::new("Pick 1–2 lines or 2 vertices")
                        .color(egui::Color32::from_gray(140))
                        .size(11.5),
                ),
            };
        });
        // A labeled button (#629): the action's name should be readable, not a bare ✓.
        if primary_text_button(ui, controls_enabled && control.can_commit, "Derive parameter") {
            on_dimension_derive_edit(DimensionDeriveEdit::Commit);
        }
        ui.add_space(4.0);
    }

    // The drawing workbench's Select tool has its own always-visible element picker (#346): a
    // label-only combo box over the selected projections/text/dimensions, kept in sync with the
    // Elements pane and the page.
    if let Some(rows) = &content.drawing_selection {
        any_control = true;
        // Each row carries the same icon the Elements pane uses for that element kind (#363).
        let icon_rows: Vec<(crate::icons::IconId, String)> = rows
            .iter()
            .map(|(_, element, label)| (drawing_element_icon(*element), label.clone()))
            .collect();
        labeled_row_top(ui, "Selection", |ui| {
        ui.add_enabled_ui(controls_enabled, |ui| {
            if let Some(event) = crate::element_picker::show_rows(
                ui,
                "drawing_selection_picker",
                true,
                &[
                    crate::icons::IconId::Projection,
                    crate::icons::IconId::Text,
                    crate::icons::IconId::Dimension,
                ],
                false,
                &icon_rows,
            ) {
                match event {
                    crate::element_picker::PickerEvent::Focus => {}
                    crate::element_picker::PickerEvent::Remove(i) => {
                        if let Some((drawing, element, _)) = rows.get(i) {
                            on_drawing_selection_edit(DrawingSelectionEdit::Remove(
                                *drawing, *element,
                            ));
                        }
                    }
                    crate::element_picker::PickerEvent::Clear => {
                        on_drawing_selection_edit(DrawingSelectionEdit::Clear)
                    }
                }
            }
        });
        });
    }

    // The Aligned-view tool's "Base view" picker (#365): the projection a new aligned view lines
    // up with. Seeded from a selected projection on tool entry; otherwise pick one by clicking a
    // projection (on the page or in the Elements pane). Always focused as a pick cue.
    if let Some(base) = &content.drawing_align {
        any_control = true;
        let rows: Vec<(crate::icons::IconId, String)> = base
            .iter()
            .map(|(_, label)| (crate::icons::IconId::Projection, label.clone()))
            .collect();
        labeled_row_top(ui, "Base view", |ui| {
        ui.add_enabled_ui(controls_enabled, |ui| {
            if let Some(event) = crate::element_picker::show_rows(
                ui,
                "drawing_align_base_picker",
                true,
                &[crate::icons::IconId::Projection],
                true,
                &rows,
            ) {
                if matches!(
                    event,
                    crate::element_picker::PickerEvent::Remove(_)
                        | crate::element_picker::PickerEvent::Clear
                ) {
                    on_drawing_align_clear();
                }
            }
        });
        });
    }

    if let Some(control) = &content.name {
        any_control = true;
        let id = egui::Id::new(("element_name", control.element.clone()));
        let mut committed = false;
        labeled_row(
            ui,
            shortcuts::compact_label("Name", Some(shortcuts::FOCUS_ELEMENT_NAME)),
            |ui| {
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
            },
        );
        if committed {
            on_name_committed(control.element.clone(), pane_state.name_draft.clone());
        }
        ui.add_space(4.0);
    }

    if let Some(rows) = &content.constraints {
        any_control = true;
        section_label(ui, "Constraints");
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
            if checkbox_row(ui, "Curve", &mut curve_mode, Some(shortcuts::TOGGLE_CURVE_MODE)) {
                on_curve_mode_changed(curve_mode);
            }
        });
    }

    if let Some(mut tangent_constraint) = content.tangent_constraint {
        any_control = true;
        ui.add_enabled_ui(controls_enabled, |ui| {
            if checkbox_row(
                ui,
                "Tangent",
                &mut tangent_constraint,
                Some(shortcuts::TOGGLE_TANGENT_CONSTRAINT),
            ) {
                on_tangent_constraint_changed(tangent_constraint);
            }
        });
        ui.add_space(4.0);
    }

    if let Some(anchor) = content.rect_anchor {
        use crate::actions::RectAnchor;
        any_control = true;
        // Two-column "Anchor" row (#589): label left, the mode buttons in the right column.
        labeled_row(ui, "Anchor", |ui| {
            for (value, icon, tooltip) in [
                (RectAnchor::Corner, crate::icons::IconId::RectCorner, "Corner-anchored (R toggles)"),
                (RectAnchor::Center, crate::icons::IconId::RectCenter, "Centre-anchored (R toggles)"),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, anchor == value, tooltip)
                    .clicked()
                    && anchor != value
                {
                    on_rect_anchor_changed(value);
                }
            }
        });
    }

    if let Some(anchor) = content.circle_anchor {
        use crate::actions::CircleAnchor;
        any_control = true;
        // Two-column "Anchor" row (#589), matching the Rectangle tool.
        labeled_row(ui, "Anchor", |ui| {
            for (value, icon, tooltip) in [
                (CircleAnchor::Center, crate::icons::IconId::CircleCenter, "Centre + radius (O toggles)"),
                (CircleAnchor::Edge, crate::icons::IconId::CircleEdge, "Edge to opposite edge (O toggles)"),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, anchor == value, tooltip)
                    .clicked()
                    && anchor != value
                {
                    on_circle_anchor_changed(value);
                }
            }
        });
    }

    if let Some(control) = &content.construction {
        any_control = true;
        let label = match control.value {
            TriState::Mixed => "Construction (mixed)",
            _ => "Construction",
        };
        let mut checked = control.value == TriState::On;
        ui.add_enabled_ui(controls_enabled, |ui| {
            if checkbox_row(ui, label, &mut checked, Some(shortcuts::TOGGLE_CONSTRUCTION)) {
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
        if checkbox_row(ui, "Snapping", &mut checked, None) {
            on_snapping_changed(checked);
        }
    }

    // Tool-owned element pickers (#213) render at the top of the active tool's section, above
    // its parameter controls — the picked set is the tool's primary input.
    for view in &content.tool_pickers {
        any_control = true;
        if view.separator_above {
            ui.separator();
        }
        labeled_row_top(ui, view.heading, |ui| {
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
        });
    }

    // Legacy row-list element picker (#loft Sections, Chamfer/Fillet Edges): render right after
    // the tool-owned pickers so the picked set is the first thing in the tool's section — e.g.
    // the Loft tool's "Sections" picker sits above its Output/Do controls (#609).
    if let Some(picker) = &content.edge_picker {
        any_control = true;
        ui.separator();
        labeled_row_top(ui, picker.heading, |ui| {
        ui.add_enabled_ui(controls_enabled, |ui| {
            // The active tool's picker is focused (its viewport clicks feed this set).
            if let Some(event) = crate::element_picker::show_labeled(
                ui,
                picker.heading,
                true,
                false,
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
        });
    }

    if let Some(control) = &content.revolve {
        any_control = true;
        ui.separator();

        // Face element picker (#261): the picked profile faces, click one's ✕ to drop it. Faces
        // are still added by clicking them in the viewport.
        labeled_row_top(ui, "Profile", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "revolve_faces",
            !control.axis_focused,
            false,
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
        });

        // Axis element picker (#261): the picked edge/axis, click its ✕ to clear. Set it by
        // clicking a straight line or a global axis in the viewport.
        let axis_rows: Vec<String> = control.axis_label.iter().cloned().collect();
        labeled_row_top(ui, "Axis", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "revolve_axis",
            control.axis_focused,
            true,
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
        });

        let mut symmetric = control.symmetric;
        if checkbox_row(ui, "Symmetric", &mut symmetric, None) {
            on_revolve_edit(RevolveEdit::Symmetric(symmetric));
        }
        // A segmented icon group (#261): New body / Add to touching / Cut, one highlighted —
        // the same icons the Extrude "into" picker uses. A cut needs the kernel, so it's only
        // offered on an `occt` build (mirrors the Extrude cut option).
        let choice = control.body_choice;
        labeled_row(ui, "Output", |ui| {
            for (value, icon, tooltip) in [
                (
                    crate::actions::RevolveBodyChoice::NewBody,
                    crate::icons::IconId::NewBody,
                    "New body",
                ),
                (
                    crate::actions::RevolveBodyChoice::AddTouching,
                    crate::icons::IconId::AddToBody,
                    "Join body",
                ),
                (
                    crate::actions::RevolveBodyChoice::Cut,
                    crate::icons::IconId::CutBody,
                    "Cut",
                ),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, choice == value, tooltip)
                    .clicked()
                    && choice != value
                {
                    on_revolve_edit(RevolveEdit::BodyChoice(value));
                }
            }
        });
        // Ready once a profile face and an axis are picked (#586).
        let ready = control.face_count > 0 && control.axis_label.is_some();
        if primary_button(ui, ready && controls_enabled, "Revolve") {
            on_revolve_edit(RevolveEdit::Commit);
        }
    }

    if let Some(control) = &content.sweep {
        any_control = true;
        ui.separator();

        // Face element picker: the picked profile faces, click one's ✕ to drop it. Faces
        // are still added by clicking them in the viewport.
        labeled_row_top(ui, "Profile", |ui| {
            if let Some(event) = crate::element_picker::show_labeled(
                ui,
                "sweep_faces",
                !control.path_focused,
                false,
                crate::icons::IconId::Sketch,
                &control.face_rows,
            ) {
                match event {
                    crate::element_picker::PickerEvent::Focus => {}
                    crate::element_picker::PickerEvent::Remove(i) => {
                        on_sweep_edit(SweepEdit::RemoveFace(Some(i)))
                    }
                    crate::element_picker::PickerEvent::Clear => {
                        on_sweep_edit(SweepEdit::RemoveFace(None))
                    }
                }
            }
        });

        // Path element picker: the picked path lines, click a row's ✕ to drop it. Lines
        // are added by clicking them in the viewport.
        labeled_row_top(ui, "Path", |ui| {
            if let Some(event) = crate::element_picker::show_labeled(
                ui,
                "sweep_path",
                control.path_focused,
                false,
                crate::icons::IconId::Line,
                &control.path_rows,
            ) {
                match event {
                    crate::element_picker::PickerEvent::Focus => {}
                    crate::element_picker::PickerEvent::Remove(i) => {
                        on_sweep_edit(SweepEdit::RemovePath(Some(i)))
                    }
                    crate::element_picker::PickerEvent::Clear => {
                        on_sweep_edit(SweepEdit::RemovePath(None))
                    }
                }
            }
        });

        // New body / Add to touching / Cut — the same segmented icon group as Revolve.
        // A cut needs the kernel, so it's only offered on an `occt` build.
        let choice = control.body_choice;
        labeled_row(ui, "Output", |ui| {
            for (value, icon, tooltip) in [
                (
                    crate::actions::RevolveBodyChoice::NewBody,
                    crate::icons::IconId::NewBody,
                    "New body",
                ),
                (
                    crate::actions::RevolveBodyChoice::AddTouching,
                    crate::icons::IconId::AddToBody,
                    "Join body",
                ),
                (
                    crate::actions::RevolveBodyChoice::Cut,
                    crate::icons::IconId::CutBody,
                    "Cut",
                ),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, choice == value, tooltip)
                    .clicked()
                    && choice != value
                {
                    on_sweep_edit(SweepEdit::BodyChoice(value));
                }
            }
        });
        // Ready once a profile face and a path are picked (#586).
        let ready = !control.face_rows.is_empty() && !control.path_rows.is_empty();
        if primary_button(ui, ready && controls_enabled, "Sweep") {
            on_sweep_edit(SweepEdit::Commit);
        }
    }

    if let Some(control) = &content.loft_body {
        any_control = true;
        ui.separator();
        // The same segmented icon group as Revolve/Sweep (#479), under a shared "Output" label.
        let choice = control.body_choice;
        labeled_row(ui, "Output", |ui| {
            for (value, icon, tooltip) in [
                (
                    crate::actions::RevolveBodyChoice::NewBody,
                    crate::icons::IconId::NewBody,
                    "New body",
                ),
                (
                    crate::actions::RevolveBodyChoice::AddTouching,
                    crate::icons::IconId::AddToBody,
                    "Join body",
                ),
                (
                    crate::actions::RevolveBodyChoice::Cut,
                    crate::icons::IconId::CutBody,
                    "Cut",
                ),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, choice == value, tooltip)
                    .clicked()
                    && choice != value
                {
                    on_loft_body_choice(value);
                }
            }
        });
        // Ready once at least two sections are picked (#586).
        if primary_button(ui, control.can_commit && controls_enabled, "Loft") {
            on_loft_commit();
        }
    }

    if let Some(control) = &content.plane_tool {
        any_control = true;
        ui.separator();

        // The picked anchor set — face, edge, vertex, or line+point — with ✕ to clear (#474/#483).
        labeled_row_top(ui, "Anchor", |ui| {
            if let Some(event) = crate::element_picker::show_labeled(
                ui,
                "plane_anchor",
                control.anchor_labels.is_empty(),
                true,
                crate::icons::IconId::Plane,
                &control.anchor_labels,
            ) {
                match event {
                    crate::element_picker::PickerEvent::Focus => {}
                    crate::element_picker::PickerEvent::Remove(_)
                    | crate::element_picker::PickerEvent::Clear => {
                        on_plane_tool_edit(PlaneToolEdit::ClearAnchor)
                    }
                }
            }
        });

        // Several lines meet the picked vertex: a single-select picker chooses which connected
        // line's direction is the plane's normal (#612), instead of a stack of "Along line X"
        // buttons.
        if control.normal_labels.len() > 1 {
            let selected = control
                .normal_labels
                .get(control.normal_choice)
                .cloned()
                .unwrap_or_default();
            labeled_row(ui, "Normal", |ui| {
                ui.add_enabled_ui(controls_enabled, |ui| {
                    egui::ComboBox::from_id_salt("plane_normal_line")
                        .selected_text(selected)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (i, label) in control.normal_labels.iter().enumerate() {
                                if ui
                                    .selectable_label(control.normal_choice == i, label)
                                    .clicked()
                                    && control.normal_choice != i
                                {
                                    on_plane_tool_edit(PlaneToolEdit::NormalChoice(i));
                                }
                            }
                        });
                });
            });
        }

        // Offset (and, for an edge/axis anchor, angle) inputs mirroring the 3D viewport fields
        // (#613/#614). Both edit the same in-progress plane, so the pane and the floating fields
        // stay in lock-step.
        if control.has_anchor {
            labeled_row(ui, "Offset", |ui| {
                ui.add_enabled_ui(controls_enabled, |ui| {
                    let mut text = control.offset_text.clone();
                    let resp = crate::expression_input::ValueInput::new(
                        "plane_offset_ctx",
                        crate::expression_input::ValueKind::Length,
                    )
                    .width(90.0)
                    .show(ui, &mut text, doc);
                    if resp.changed() {
                        on_plane_tool_edit(PlaneToolEdit::SetOffset(text));
                    }
                    if resp.gained_focus() {
                        on_plane_tool_edit(PlaneToolEdit::FocusOffset);
                    }
                });
            });
            if control.show_angle {
                labeled_row(ui, "Angle", |ui| {
                    ui.add_enabled_ui(controls_enabled, |ui| {
                        let mut text = control.angle_text.clone();
                        let resp = crate::expression_input::ValueInput::new(
                            "plane_angle_ctx",
                            crate::expression_input::ValueKind::Angle,
                        )
                        .width(90.0)
                        .show(ui, &mut text, doc);
                        if resp.changed() {
                            on_plane_tool_edit(PlaneToolEdit::SetAngle(text));
                        }
                        if resp.gained_focus() {
                            on_plane_tool_edit(PlaneToolEdit::FocusAngle);
                        }
                    });
                });
            }
            // The plane is only created when this fires (button or Enter) — never on a stray
            // viewport click (#611).
            if primary_button(ui, controls_enabled, "Create plane") {
                on_plane_tool_edit(PlaneToolEdit::Commit);
            }
        }
    }

    if let Some(control) = &content.boolean_op {
        any_control = true;
        // No divider between the Bodies picker above and this section — the pickers, the mode
        // row, and the Do button read as one contiguous Combine block (#606). The tool title
        // (#608) is drawn once at the top of the pane.
        // A segmented icon group (#267): two-circle boolean icons with kept regions solid and
        // removed regions faint red — in the right column under a "Mode" label (#606).
        let kind = control.kind;
        labeled_row(ui, "Mode", |ui| {
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
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Create" },
        ) {
            on_boolean_edit(BooleanEdit::Commit);
        }
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
        // The picked bodies render through the unified element picker (see `tool_pickers`).
        let mut pending: Option<MoveEdit> = None;
        {
            let mut field = |ui: &mut egui::Ui,
                             label: &str,
                             value: &str,
                             kind: crate::expression_input::ValueKind,
                             make: &dyn Fn(String) -> MoveEdit| {
                labeled_row(ui, label, |ui| {
                    let mut text = value.to_string();
                    let resp = crate::expression_input::ValueInput::new(("move_field", label), kind)
                        .width(90.0)
                        .show(ui, &mut text, doc);
                    if resp.changed() {
                        pending = Some(make(text));
                    }
                });
            };
            use crate::expression_input::ValueKind;
            field(ui, "X", &control.tx, ValueKind::Length, &MoveEdit::Tx);
            field(ui, "Y", &control.ty, ValueKind::Length, &MoveEdit::Ty);
            field(ui, "Z", &control.tz, ValueKind::Length, &MoveEdit::Tz);
            field(ui, "Angle", &control.angle, ValueKind::Angle, &MoveEdit::Angle);
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
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Move" },
        ) {
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

    if let Some(control) = &content.mirror_op {
        any_control = true;
        // No divider between the pickers above and this Do button — the Mirror plane picker,
        // the Bodies picker, and the button read as one contiguous block (#602).
        // The mirror plane and the bodies to mirror both render above through the unified
        // element pickers (see `tool_pickers`: `MirrorPlane` then `MirrorTargets`, #566).
        ui.add_space(2.0);
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Mirror" },
        ) {
            on_mirror_edit(MirrorEdit::Commit);
        }
        ui.label(
            egui::RichText::new("The originals stay; each reflection is a new body")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(op) = content.mirror_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit mirror").clicked() {
            on_mirror_edit_start(op);
        }
        ui.label(
            egui::RichText::new("Re-open the Mirror tool to change this operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(control) = &content.repeat_op {
        any_control = true;
        ui.separator();
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
        // Axis element picker (#257/#439): empty until an axis is picked — click a straight
        // edge, a sketch line, or an origin axis in the viewport; the ✕ clears it. It reads
        // as the focused picker exactly while unset — once targets are seeded, the axis is
        // the next thing to pick. The X/Y/Z shortcut buttons are gone (#643): the origin axes
        // are pickable in the viewport like everything else, so the buttons were a second,
        // inconsistent way in.
        let axis_rows: Vec<String> = control
            .axis_label
            .iter()
            .map(|l| format!("Along {l}"))
            .collect();
        let has_targets = !control.targets.is_empty()
            || !control.plane_targets.is_empty()
            || !control.sketch_targets.is_empty()
            || !control.extrusion_targets.is_empty();
        let axis_focused =
            control.axis_label.is_none() && has_targets && !control.value_field_focused;
        labeled_row_top(ui, "Axis", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "repeat_axis",
            axis_focused,
            true,
            crate::icons::IconId::Line,
            &axis_rows,
        ) {
            if matches!(
                event,
                crate::element_picker::PickerEvent::Remove(_) | crate::element_picker::PickerEvent::Clear
            ) {
                pending = Some(RepeatEdit::ClearAxis);
            }
        }
        });
        // Count / gap / distance (#257/#443/#444): two fields are editable, the third is
        // computed. A **green lock** marks the computed one and grey locks the other two
        // (#642) — clicking a grey lock moves the green one there. Editable fields are
        // expression inputs; the measure toggles (icon *and* label, #640) hover gold.
        use crate::model::RepeatVar;
        {
            let mut var_row = |ui: &mut egui::Ui,
                               var: RepeatVar,
                               label: &str,
                               value: &str,
                               toggle: Option<(crate::icons::IconId, RepeatEdit)>,
                               make: &dyn Fn(String) -> RepeatEdit| {
                let computed = control.computed_var == var;
                ui.horizontal(|ui| {
                    // Icon + label share the fixed label column (#371) so the inputs align.
                    ui.allocate_ui_with_layout(
                        egui::vec2(FIELD_LABEL_W, 18.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_size(egui::vec2(FIELD_LABEL_W, 18.0));
                            match toggle {
                                // The measure toggle hovers gold to read as clickable (#440),
                                // and its label is the same target (#640).
                                Some((icon, edit)) => {
                                    const TIP: &str = "Click to toggle how this is measured";
                                    if crate::icons::icon_button_hover_gold(ui, icon, TIP).clicked()
                                        || clickable_label(ui, label, TIP).clicked()
                                    {
                                        pending = Some(edit);
                                    }
                                }
                                None => {
                                    ui.label(label);
                                }
                            }
                        },
                    );
                    // Both states render at the same width (#641) so the column of inputs
                    // doesn't jump as the computed one moves between rows.
                    const VAR_FIELD_W: f32 = 110.0;
                    if computed {
                        let shown = control.computed_value.clone().unwrap_or_else(|| "—".to_string());
                        ui.add_enabled(
                            false,
                            egui::TextEdit::singleline(&mut shown.clone())
                                .desired_width(VAR_FIELD_W),
                        )
                        .on_hover_text("Computed from the other two");
                    } else {
                        let mut text = value.to_string();
                        let kind = if var == RepeatVar::Count {
                            crate::expression_input::ValueKind::Count
                        } else {
                            crate::expression_input::ValueKind::Length
                        };
                        let resp = crate::expression_input::ValueInput::from_id(
                            repeat_value_field_id(label),
                            kind,
                        )
                        .width(VAR_FIELD_W)
                        .show(ui, &mut text, doc);
                        if resp.changed() {
                            pending = Some(make(text.clone()));
                        }
                    }
                    // Lock (#642): green on the one value the app computes, grey on the two
                    // the user sets. Clicking a grey lock moves the green lock to it.
                    let lock = crate::icons::tinted_icon_button(
                        ui,
                        crate::icons::IconId::Lock,
                        if computed {
                            crate::theme::LOCKED_ACCENT
                        } else {
                            crate::theme::UNLOCKED_GRAY
                        },
                        if computed {
                            crate::theme::LOCKED_ACCENT
                        } else {
                            crate::theme::LOCKED_ACCENT.gamma_multiply(0.7)
                        },
                        if computed {
                            "Computed from the other two"
                        } else {
                            "Click to compute this from the other two instead"
                        },
                    );
                    if lock.clicked() && !computed {
                        pending = Some(RepeatEdit::SetComputed(var));
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
        // The Count field already shows the instance count (#446); only surface the
        // can't-evaluate case.
        if control.preview_instances.is_none() {
            ui.label(
                egui::RichText::new("Configuration doesn't evaluate yet")
                    .color(egui::Color32::from_gray(140))
                    .size(11.0),
            );
        }
        if let Some(edit) = pending {
            on_repeat_edit(edit);
        }
        ui.add_space(2.0);
        // The commit button sits in the input (right) column (#447), aligned with the fields.
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Repeat" },
        ) {
            on_repeat_edit(RepeatEdit::Commit);
        }
    }

    // In-sketch Repeat tool (#232): entities + direction + count/gap/distance.
    if let Some(control) = &content.sketch_repeat {
        use crate::model::RepeatVar;
        any_control = true;
        ui.separator();
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
                // Icon + label share the fixed label column (#371) so the inputs align.
                ui.allocate_ui_with_layout(
                    egui::vec2(FIELD_LABEL_W, 18.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_size(egui::vec2(FIELD_LABEL_W, 18.0));
                        if let Some((icon, edit)) = toggle {
                            if crate::icons::icon_button(ui, icon, "Toggle how this is measured")
                                .clicked()
                            {
                                pending = Some(edit);
                            }
                        }
                        ui.label(label);
                    },
                );
                if computed {
                    ui.label(egui::RichText::new("(auto)").color(egui::Color32::from_gray(130)).size(10.0));
                } else {
                    let mut text = value.to_string();
                    let kind = if var == RepeatVar::Count {
                        crate::expression_input::ValueKind::Count
                    } else {
                        crate::expression_input::ValueKind::Length
                    };
                    let resp = crate::expression_input::ValueInput::new(
                        ("sketch_repeat_var_field", label),
                        kind,
                    )
                    .width(80.0)
                    .show(ui, &mut text, doc);
                    if resp.changed() {
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

    if let Some(control) = &content.sketch_offset {
        any_control = true;
        ui.separator();
        // Element picker of lines/circles in the offset set (#493).
        let mut picker = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Line, ElementKind::Circle]),
            PickLimit::Infinite,
        );
        picker.set_focused(true);
        picker.set_picked(control.picked.iter().cloned());
        labeled_row_top(ui, "Entities", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &picker, doc, "sketch_offset_picker")
                {
                    match event {
                        crate::element_picker::PickerEvent::Focus => {}
                        crate::element_picker::PickerEvent::Remove(i) => {
                            if let Some(el) = control.picked.get(i).cloned() {
                                on_sketch_offset_edit(SketchOffsetEdit::Remove(el));
                            }
                        }
                        crate::element_picker::PickerEvent::Clear => {
                            on_sketch_offset_edit(SketchOffsetEdit::Clear);
                        }
                    }
                }
            });
        });
        let mut pending: Option<SketchOffsetEdit> = None;
        // Two-column Distance row (#592): label left, value input right.
        labeled_row(ui, "Distance", |ui| {
            let mut text = control.distance.clone();
            crate::expression_input::ValueInput::new(
                "sketch_offset_distance",
                crate::expression_input::ValueKind::Length,
            )
            .width(110.0)
            .show(ui, &mut text, doc);
            // Emit whenever the buffer differs, not only on `resp.changed()`: Tab/Space
            // parameter autocomplete rewrites the buffer before the text edit runs, so egui
            // doesn't flag it as a change and the completion would otherwise be lost (#517).
            if text != control.distance {
                pending = Some(SketchOffsetEdit::Distance(text.clone()));
            }
        });
        // Two-column Construction toggle with the shared `X` shortcut (#591).
        let mut construction = control.construction;
        if checkbox_row(ui, "Construction", &mut construction, Some(shortcuts::TOGGLE_CONSTRUCTION)) {
            pending = Some(SketchOffsetEdit::Construction(construction));
        }
        if let Some(edit) = pending {
            on_sketch_offset_edit(edit);
        }
        // The blue primary button / Enter commits the offset (#590).
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Offset" },
        ) {
            on_sketch_offset_edit(SketchOffsetEdit::Commit);
        }
    }

    if let Some(op) = content.sketch_offset_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit offset").clicked() {
            on_sketch_offset_edit(SketchOffsetEdit::EditStart(op));
        }
        ui.label(
            egui::RichText::new("Re-open the Offset tool to change this operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
    }

    if let Some(control) = &content.sketch_mirror {
        any_control = true;
        ui.separator();
        // Primary: the mirror line, as a single-line element picker (#534). Removing it lets
        // the next viewport click pick a new mirror line.
        let mut line_picker =
            ElementPicker::new(ElementFilter::kinds(&[ElementKind::Line]), PickLimit::Finite(1));
        line_picker.set_focused(control.line.is_none());
        line_picker.set_picked(control.line.map(SceneElement::Line));
        labeled_row_top(ui, "Mirror line", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &line_picker, doc, "sketch_mirror_line_picker")
                {
                    match event {
                        crate::element_picker::PickerEvent::Focus => {}
                        crate::element_picker::PickerEvent::Remove(_)
                        | crate::element_picker::PickerEvent::Clear => {
                            on_sketch_mirror_edit(SketchMirrorEdit::ClearLine);
                        }
                    }
                }
            });
        });
        // Secondary: the reflected shapes (unified element picker).
        let mut picker = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Line, ElementKind::Circle]),
            PickLimit::Infinite,
        );
        picker.set_focused(control.line.is_some());
        picker.set_picked(control.picked.iter().cloned());
        labeled_row_top(ui, "Shapes", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &picker, doc, "sketch_mirror_picker")
                {
                    match event {
                        crate::element_picker::PickerEvent::Focus => {}
                        crate::element_picker::PickerEvent::Remove(i) => {
                            if let Some(el) = control.picked.get(i).cloned() {
                                on_sketch_mirror_edit(SketchMirrorEdit::Remove(el));
                            }
                        }
                        crate::element_picker::PickerEvent::Clear => {
                            on_sketch_mirror_edit(SketchMirrorEdit::Clear);
                        }
                    }
                }
            });
        });
        if ui
            .add_enabled(
                control.can_commit && controls_enabled,
                egui::Button::new(if control.editing { "Apply changes" } else { "Mirror" }),
            )
            .clicked()
        {
            on_sketch_mirror_edit(SketchMirrorEdit::Commit);
        }
    }

    if let Some(op) = content.sketch_mirror_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit mirror").clicked() {
            on_sketch_mirror_edit(SketchMirrorEdit::EditStart(op));
        }
        ui.label(
            egui::RichText::new("Re-open the Mirror tool to change this operation")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
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
        let mut pending: Option<SliceEdit> = None;
        // Two element pickers; the focused one is the side the next viewport click lands on
        // (clicking a picker makes it active, replacing the old Bodies/Cutters toggle).
        labeled_row_top(ui, "Bodies", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "slice_targets",
            !control.picking_cutter,
            false,
            crate::icons::IconId::Body,
            &control.target_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SliceEdit::PickingCutter(false),
                crate::element_picker::PickerEvent::Remove(i) => SliceEdit::RemoveTarget(Some(i)),
                crate::element_picker::PickerEvent::Clear => SliceEdit::RemoveTarget(None),
            });
        }
        });
        labeled_row_top(ui, "Cutters", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "slice_cutters",
            control.picking_cutter,
            false,
            crate::icons::IconId::Plane,
            &control.cutter_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SliceEdit::PickingCutter(true),
                crate::element_picker::PickerEvent::Remove(i) => SliceEdit::RemoveCutter(Some(i)),
                crate::element_picker::PickerEvent::Clear => SliceEdit::RemoveCutter(None),
            });
        }
        });
        let mut extend = control.extend_infinite;
        if checkbox_row(ui, "Infinite cut", &mut extend, None) {
            pending = Some(SliceEdit::ExtendInfinite(extend));
        }
        if let Some(edit) = pending {
            on_slice_edit(edit);
        }
        ui.add_space(2.0);
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Slice" },
        ) {
            on_slice_edit(SliceEdit::Commit);
        }
    }

    // In-sketch Slice (#238): two-role pickers for sketch targets (lines/circles/faces) and cutter
    // lines, like the Combine tool's A/B pickers. Clicking a picker makes it the active side.
    if let Some(control) = &content.sketch_slice {
        any_control = true;
        ui.separator();
        let mut pending: Option<SketchSliceEdit> = None;
        labeled_row_top(ui, "Targets", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "sketch_slice_targets",
            !control.picking_cutter,
            false,
            crate::icons::IconId::Line,
            &control.target_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SketchSliceEdit::PickingCutter(false),
                crate::element_picker::PickerEvent::Remove(_)
                | crate::element_picker::PickerEvent::Clear => SketchSliceEdit::ClearTargets,
            });
        }
        });
        labeled_row_top(ui, "Cutters", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "sketch_slice_cutters",
            control.picking_cutter,
            false,
            crate::icons::IconId::Line,
            &control.cutter_rows,
        ) {
            pending = Some(match event {
                crate::element_picker::PickerEvent::Focus => SketchSliceEdit::PickingCutter(true),
                crate::element_picker::PickerEvent::Remove(_)
                | crate::element_picker::PickerEvent::Clear => SketchSliceEdit::ClearCutters,
            });
        }
        });
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
        let text_resp = labeled_row_top(ui, "Text", |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut edit_text)
                    .id(text_id)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
        });
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
        // Font family chooser: each name renders in its own font (#384). Rows are
        // virtualized so only the families scrolled into view load their face.
        labeled_row(ui, "Font", |ui| {
            egui::ComboBox::from_id_salt("sketch_text_font")
                .selected_text(control.font_family.clone())
                .show_ui(ui, |ui| {
                    let row_h = 20.0;
                    egui::ScrollArea::vertical().max_height(260.0).show_rows(
                        ui,
                        row_h,
                        control.families.len(),
                        |ui, range| {
                            for fam in &control.families[range] {
                                let label = match preview_font_family(ui.ctx(), fam) {
                                    Some(ff) => egui::RichText::new(fam)
                                        .family(ff)
                                        .size(14.0),
                                    None => egui::RichText::new(fam),
                                };
                                let resp = ui.add_sized(
                                    egui::vec2(ui.available_width(), row_h),
                                    egui::Button::selectable(
                                        fam == &control.font_family,
                                        label,
                                    ),
                                );
                                if resp.clicked() {
                                    on_sketch_text_edit(SketchTextEdit::Font(fam.clone()));
                                }
                            }
                        },
                    );
                });
        });
        labeled_row(ui, "", |ui| {
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
        labeled_row(ui, "Size", |ui| {
            let mut size = control.size_expr.clone();
            let resp = crate::expression_input::ValueInput::new(
                "sketch_text_size",
                crate::expression_input::ValueKind::Length,
            )
            .width(70.0)
            .show(ui, &mut size, doc);
            if resp.changed() {
                on_sketch_text_edit(SketchTextEdit::Size(size));
            }
            // ± steppers (#385): bump the evaluated size by 1 mm (replacing any expression
            // with the stepped literal), never below 1 mm.
            let stepped = |delta: f32| {
                let v = (control.size_mm + delta).max(1.0);
                let mut text = format!("{v:.2}");
                while text.ends_with('0') {
                    text.pop();
                }
                if text.ends_with('.') {
                    text.pop();
                }
                text
            };
            if ui.small_button("−").on_hover_text("Smaller by 1 mm").clicked() {
                on_sketch_text_edit(SketchTextEdit::Size(stepped(-1.0)));
            }
            if ui.small_button("+").on_hover_text("Larger by 1 mm").clicked() {
                on_sketch_text_edit(SketchTextEdit::Size(stepped(1.0)));
            }
        });
        labeled_row(ui, "Rotation°", |ui| {
            let mut rot = control.rotation_deg.clone();
            let resp = crate::expression_input::ValueInput::new(
                "sketch_text_rotation",
                crate::expression_input::ValueKind::Angle,
            )
            .width(70.0)
            .show(ui, &mut rot, doc);
            if resp.changed() {
                on_sketch_text_edit(SketchTextEdit::Rotation(rot));
            }
        });
        labeled_row(ui, "Wrap width", |ui| {
            let mut wrap = control.wrap.clone();
            if crate::expression_input::ValueInput::new(
                "sketch_text_wrap",
                crate::expression_input::ValueKind::Length,
            )
            .hint("grow")
            .width(70.0)
            .show(ui, &mut wrap, doc)
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
        section_label(ui, "View");
        labeled_row(ui, "Source", |ui| {
            ui.label(&control.source);
        });
        // An aligned child stays lined up with its base, but its **angle** can be adjusted within
        // the ring of orientations that keep the shared edge (#367). A child of an isometric
        // base has no such ring, so it stays read-only.
        if control.aligned && control.inline_orientations.is_empty() {
            ui.label(
                egui::RichText::new(format!("{} · aligned", control.orientation.label()))
                    .color(egui::Color32::from_gray(150)),
            );
        } else {
            // Interactive orientation bear (#315): drag to spin, click a face for that view or
            // a corner/edge for isometric; focus it and press 4/5/6/8/2/0 for
            // left/front/right/top/bottom/back. An aligned child gets the same bear (#370),
            // restricted to the faces/edges of its shared-edge ring — anything else neither
            // highlights nor clicks.
            let seed = drawing_orientation_to_standard(control.orientation);
            // Highlight the current view on the bear (#323/#340): a face, a corner (Isometric),
            // or a cube edge (a diagonal edge view, #339). Drawn even when behind the bear.
            let selected = drawing_orientation_to_cube_pick(control.orientation);
            let ring: Vec<crate::view_cube::CubePick> = control
                .inline_orientations
                .iter()
                .filter_map(|o| drawing_orientation_to_cube_pick(*o))
                .collect();
            let allowed = control.aligned.then_some(ring.as_slice());
            if let Some(pick) = crate::view_cube::show_orientation_picker(
                ui,
                "drawing_view_bear",
                seed,
                selected,
                false,
                None,
                None,
                false,
                allowed,
            ) {
                on_drawing_view_edit(DrawingViewEdit::Orientation(orientation_pick_to_drawing(pick)));
            }
            if control.aligned {
                ui.label(
                    egui::RichText::new(format!("{} · aligned", control.orientation.label()))
                        .color(egui::Color32::from_gray(150)),
                );
            } else {
                // Set the projection to whatever the 3D viewport is currently showing (#366) —
                // the way to get an arbitrary angle now that the free-spin toggle is gone.
                if ui.button("Use this view").clicked() {
                    on_drawing_view_edit(DrawingViewEdit::UseCurrentView);
                }
            }
        }
        labeled_row(ui, "Style", |ui| {
            egui::ComboBox::from_id_salt("drawing_view_style")
                .selected_text(control.style.label())
                .show_ui(ui, |ui| {
                    for style in crate::model::DrawingViewStyle::ALL {
                        if ui.selectable_label(control.style == style, style.label()).clicked() {
                            on_drawing_view_edit(DrawingViewEdit::Style(style));
                        }
                    }
                });
        });
        labeled_row(ui, "Scale", |ui| {
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
        // Aligned children can draw dashed projection lines to their base view (#377).
        if control.aligned {
            labeled_row(ui, "", |ui| {
                let mut lines = control.align_lines;
                if ui.checkbox(&mut lines, "Projection lines").changed() {
                    on_drawing_view_edit(DrawingViewEdit::AlignLines(lines));
                }
            });
        }
        // Caption label (#372): show/hide, custom text (with {expr} interpolation like any
        // label, #338), and a 2×3 position grid for where it sits on the card.
        labeled_row(ui, "Label", |ui| {
            let mut shown = !control.label_hidden;
            if ui.checkbox(&mut shown, "").changed() {
                on_drawing_view_edit(DrawingViewEdit::LabelHidden(!shown));
            }
        });
        if !control.label_hidden {
            labeled_row(ui, "Text", |ui| {
                let mut label_draft = control.label_text.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut label_draft)
                        .hint_text(control.auto_label.clone())
                        .desired_width(f32::INFINITY),
                );
                if resp.changed() {
                    let trimmed = label_draft.trim();
                    on_drawing_view_edit(DrawingViewEdit::LabelText(
                        (!trimmed.is_empty()).then(|| label_draft.clone()),
                    ));
                }
            });
            labeled_row_top(ui, "Position", |ui| {
                egui::Grid::new("drawing_view_label_pos")
                    .spacing(egui::vec2(2.0, 2.0))
                    .show(ui, |ui| {
                        for (i, pos) in
                            crate::model::DrawingLabelPos::ALL.into_iter().enumerate()
                        {
                            let selected = control.label_pos == pos;
                            if ui
                                .add_sized(
                                    egui::vec2(22.0, 16.0),
                                    egui::Button::selectable(selected, "▪"),
                                )
                                .on_hover_text(pos.label())
                                .clicked()
                            {
                                on_drawing_view_edit(DrawingViewEdit::LabelPos(pos));
                            }
                            if i == 2 {
                                ui.end_row();
                            }
                        }
                    });
            });
        }
        // Dimensions are off by default (#331); these flip the whole set on or off at once.
        // Laid out as label-left / buttons-right rows like every other field (#396).
        labeled_row(ui, "Dimensions", |ui| {
            if ui.button("Show all").clicked() {
                on_drawing_view_edit(DrawingViewEdit::SetAllDimensions(true));
            }
            if ui.button("Hide all").clicked() {
                on_drawing_view_edit(DrawingViewEdit::SetAllDimensions(false));
            }
        });
        labeled_row(ui, "", |ui| {
            if ui.button("Remove view").clicked() {
                on_drawing_view_edit(DrawingViewEdit::Remove);
            }
        });
    } else if content.drawing_add_active {
        any_control = true;
        ui.separator();
        section_label(ui, "Add view");
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
        let text_resp = labeled_row_top(ui, "Text", |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut edit_text)
                    .id(text_id)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
        });
        if text_resp.changed() {
            on_drawing_annotation_edit(DrawingAnnotationEdit::Text(edit_text.clone()));
        }
        // A double-clicked page textbox focuses this field with the text selected (#379),
        // so typing replaces it immediately (same pattern as the name field above).
        if pane_state.focus_annotation_field {
            text_resp.request_focus();
            if text_resp.has_focus() {
                let len = edit_text.chars().count();
                let mut state =
                    egui::TextEdit::load_state(&ectx, text_id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::default(),
                    egui::text::CCursor::new(len),
                )));
                state.store(&ectx, text_id);
                pane_state.focus_annotation_field = false;
            }
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

    if let Some(op) = content.sweep_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit sweep").clicked() {
            on_sweep_edit_start(op);
        }
        ui.label(
            egui::RichText::new("Re-open the Sweep tool to change this operation")
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
        section_label(ui, "Calibrate scale");
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
        section_label(ui, "Calibrate scale");
        ui.label(
            egui::RichText::new("Real length of the marked span on the image")
                .color(egui::Color32::from_gray(140))
                .size(11.0),
        );
        labeled_row(ui, "Length", |ui| {
            let mut draft = pane_state.calibrate_length_draft.clone();
            crate::expression_input::ValueInput::new(
                "calibrate_length",
                crate::expression_input::ValueKind::Length,
            )
            .hint("50mm")
            .width(80.0)
            .show(ui, &mut draft, doc);
            pane_state.calibrate_length_draft = draft;
            if ui.button("Apply").clicked()
                && !pane_state.calibrate_length_draft.trim().is_empty()
            {
                on_calibrate_image(control, pane_state.calibrate_length_draft.clone());
            }
        });
    }

    if let Some(faces) = &content.extrude_faces {
        any_control = true;
        // Extrude face element picker (#268): the picked profile faces, each with a ✕ to drop
        // it. Faces are added by clicking them in the viewport.
        labeled_row_top(ui, "Faces", |ui| {
        if let Some(event) = crate::element_picker::show_labeled(
            ui,
            "extrude_faces",
            true,
            false,
            crate::icons::IconId::Sketch,
            faces,
        ) {
            match event {
                crate::element_picker::PickerEvent::Focus => {}
                crate::element_picker::PickerEvent::Remove(i) => on_extrude_face_remove(Some(i)),
                crate::element_picker::PickerEvent::Clear => on_extrude_face_remove(None),
            }
        }
        });
    }

    if let Some(control) = &content.extrude {
        any_control = true;
        // The Distance and "Up to" rows only appear once an extrusion is in progress; the primary
        // "Extrude" button renders at the very bottom of the section (after Output/Symmetric),
        // matching Sweep/Loft/Revolve (#601).
        if control.has_extrusion {
            // Distance value input mirroring the 3D field (#584). Shows empty ("null") while an
            // extrude-to target drives the depth; typing here clears the target.
            labeled_row(ui, "Distance", |ui| {
                ui.add_enabled_ui(controls_enabled, |ui| {
                    let mut text = control.distance.clone();
                    let resp = crate::expression_input::ValueInput::new(
                        "extrude_distance",
                        crate::expression_input::ValueKind::Length,
                    )
                    .width(90.0)
                    .show(ui, &mut text, doc);
                    if resp.changed() {
                        on_extrude_edit(ExtrudeEdit::Distance(text));
                    }
                });
            });
            // Extrude-to target picker (#584): a plane or face to extrude up to. Focus it, then
            // click a plane/face in the viewport — or drag the gizmo onto one, which fills this in.
            labeled_row_top(ui, "Up to", |ui| {
                if let Some(event) = crate::element_picker::show_labeled(
                    ui,
                    "extrude_target",
                    control.target_focused,
                    true,
                    crate::icons::IconId::Plane,
                    &control.target_rows,
                ) {
                    match event {
                        crate::element_picker::PickerEvent::Focus => {
                            on_extrude_edit(ExtrudeEdit::TargetFocus)
                        }
                        crate::element_picker::PickerEvent::Remove(_)
                        | crate::element_picker::PickerEvent::Clear => {
                            on_extrude_edit(ExtrudeEdit::ClearTarget)
                        }
                    }
                }
            });
        }
    }

    if let Some(control) = &content.extrude_body {
        any_control = true;
        let mut mode = control.mode;
        // The same segmented icon group the Revolve/Sweep/Loft tools use (#479/#505), under a
        // shared "Output" label (#600).
        labeled_row(ui, "Output", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                ui.horizontal(|ui| {
                    let add_cut_enabled = control.merge_body.is_some();
                    for (value, icon, tooltip, enabled) in [
                        (
                            ExtrudeBodyMode::NewBody,
                            crate::icons::IconId::NewBody,
                            "New body".to_string(),
                            true,
                        ),
                        (
                            ExtrudeBodyMode::MergeInto(control.merge_body.unwrap_or(0)),
                            crate::icons::IconId::AddToBody,
                            if add_cut_enabled {
                                format!("Join {}", control.merge_body_label)
                            } else {
                                "Join body (sketch must sit on a body face)".to_string()
                            },
                            add_cut_enabled,
                        ),
                        (
                            ExtrudeBodyMode::Cut(control.merge_body.unwrap_or(0)),
                            crate::icons::IconId::CutBody,
                            if add_cut_enabled {
                                format!("Cut {}", control.merge_body_label)
                            } else {
                                "Cut body (sketch must sit on a body face)".to_string()
                            },
                            add_cut_enabled,
                        ),
                    ] {
                        ui.add_enabled_ui(enabled, |ui| {
                            if crate::icons::selectable_icon_button(
                                ui,
                                icon,
                                mode == value,
                                tooltip,
                            )
                            .clicked()
                                && mode != value
                                && enabled
                            {
                                mode = value;
                            }
                        });
                    }
                });
            });
        });
        if mode != control.mode {
            on_extrude_body_mode_changed(mode);
        }
        let mut symmetric = control.symmetric;
        if checkbox_row(ui, "Symmetric", &mut symmetric, None) {
            on_extrude_symmetric_changed(symmetric);
        }
        ui.add_space(4.0);
    }

    // The primary "Extrude" button sits at the bottom of the Extrude section — after the Faces
    // picker, Output, and Symmetric — so it reads as the final action, like Sweep/Loft/Revolve
    // (#601). Shown (disabled) as soon as the tool is selected, enabled once a face is picked.
    if let Some(control) = &content.extrude {
        if primary_button(ui, controls_enabled && control.can_commit, "Extrude") {
            on_extrude_edit(ExtrudeEdit::Commit);
        }
        ui.add_space(4.0);
    }

    if let Some(control) = &content.units {
        any_control = true;
        section_label(
            ui,
            if control.component.is_some() {
                "Component units"
            } else if control.sketch.is_some() {
                "Sketch units"
            } else {
                "Default units"
            },
        );
        ui.add_enabled_ui(controls_enabled, |ui| {
            labeled_row(ui, "Length", |ui| {
                let has_override_slot = control.sketch.is_some() || control.component.is_some();
                let follow_label = if control.component.is_some() {
                    format!("Inherit ({})", control.effective_length.label())
                } else {
                    format!("Follow document ({})", control.document_length.label())
                };
                let selected_text = if has_override_slot && control.length_override.is_none() {
                    follow_label.clone()
                } else {
                    control.effective_length.label().to_string()
                };
                egui::ComboBox::from_id_salt("context_length_unit")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        if has_override_slot
                            && ui
                                .selectable_label(control.length_override.is_none(), follow_label)
                                .clicked()
                        {
                            if let Some(component) = control.component {
                                on_units_changed(UnitsChoice::Component {
                                    component,
                                    length: None,
                                    angle: control.angle_override,
                                });
                            } else if let Some(sketch) = control.sketch {
                                on_units_changed(UnitsChoice::Sketch {
                                    sketch,
                                    length: None,
                                    angle: control.angle_override,
                                });
                            }
                        }
                        for unit in LengthUnit::ALL {
                            let selected = control.length_override == Some(unit)
                                || (!has_override_slot && control.effective_length == unit);
                            if ui.selectable_label(selected, unit.label()).clicked() {
                                if let Some(component) = control.component {
                                    on_units_changed(UnitsChoice::Component {
                                        component,
                                        length: Some(unit),
                                        angle: control.angle_override,
                                    });
                                } else if let Some(sketch) = control.sketch {
                                    on_units_changed(UnitsChoice::Sketch {
                                        sketch,
                                        length: Some(unit),
                                        angle: control.angle_override,
                                    });
                                } else {
                                    on_units_changed(UnitsChoice::Document {
                                        length: unit,
                                        angle: control.effective_angle,
                                    });
                                }
                            }
                        }
                    });
            });
            labeled_row(ui, "Angle", |ui| {
                let has_override_slot = control.sketch.is_some() || control.component.is_some();
                let follow_label = if control.component.is_some() {
                    format!("Inherit ({})", control.effective_angle.label())
                } else {
                    format!("Follow document ({})", control.document_angle.label())
                };
                let selected_text = if has_override_slot && control.angle_override.is_none() {
                    follow_label.clone()
                } else {
                    control.effective_angle.label().to_string()
                };
                egui::ComboBox::from_id_salt("context_angle_unit")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        if has_override_slot
                            && ui
                                .selectable_label(control.angle_override.is_none(), follow_label)
                                .clicked()
                        {
                            if let Some(component) = control.component {
                                on_units_changed(UnitsChoice::Component {
                                    component,
                                    length: control.length_override,
                                    angle: None,
                                });
                            } else if let Some(sketch) = control.sketch {
                                on_units_changed(UnitsChoice::Sketch {
                                    sketch,
                                    length: control.length_override,
                                    angle: None,
                                });
                            }
                        }
                        for unit in AngleUnit::ALL {
                            let selected = control.angle_override == Some(unit)
                                || (!has_override_slot && control.effective_angle == unit);
                            if ui.selectable_label(selected, unit.label()).clicked() {
                                if let Some(component) = control.component {
                                    on_units_changed(UnitsChoice::Component {
                                        component,
                                        length: control.length_override,
                                        angle: Some(unit),
                                    });
                                } else if let Some(sketch) = control.sketch {
                                    on_units_changed(UnitsChoice::Sketch {
                                        sketch,
                                        length: control.length_override,
                                        angle: Some(unit),
                                    });
                                } else {
                                    on_units_changed(UnitsChoice::Document {
                                        length: control.effective_length,
                                        angle: unit,
                                    });
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

    /// #392: registering system fonts for the chooser preview must never crash — every face
    /// handed to egui parses (ab_glyph-validated, correct .ttc index), and the family is only
    /// used on a pass after its atlas rebuild. Runs real passes over a sample of the
    /// installed fonts; a bad face panics right here instead of in the running app.
    #[test]
    fn font_preview_registration_never_panics() {
        let ctx = egui::Context::default();
        let families = crate::text::system_font_families();
        for fam in families.iter().take(40) {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(ff) = preview_font_family(ui.ctx(), fam) {
                        ui.label(egui::RichText::new(fam).family(ff));
                    }
                });
            });
        }
        // One more pass so every family registered on the last iteration builds its atlas
        // (the #392 panic site) and lays out in its own face.
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for fam in families.iter().take(40) {
                    if let Some(ff) = preview_font_family(ui.ctx(), fam) {
                        ui.label(egui::RichText::new(fam).family(ff));
                    }
                }
            });
        });
    }

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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
        }
    }

    /// #635: the Circle tool's Anchor row (centre+radius vs edge-to-edge) survives the
    /// circle branch of the pane builder — it used to be dropped, so the toggle never
    /// appeared even though the anchor mode itself worked.
    #[test]
    fn circle_tool_shows_the_anchor_row() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        for in_sketch in [false, true] {
            let content = context_pane_content(&ContextInput {
                tool: Tool::Circle,
                draw_circle_construction: Some(false),
                circle_anchor: Some(crate::actions::CircleAnchor::Edge),
                in_sketch,
                ..input(&doc, &selection)
            });
            assert_eq!(
                content.circle_anchor,
                Some(crate::actions::CircleAnchor::Edge),
                "Circle tool shows its Anchor row (in_sketch={in_sketch})"
            );
        }
    }

    /// #636: the Rectangle/Line/Circle context sections read the same in 3D as they do
    /// inside a sketch — the Snapping toggle used to be sketch-only.
    #[test]
    fn draw_tools_show_the_same_pane_in_3d_and_in_sketch() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        for (tool, ctor) in [
            (Tool::Rectangle, "rect"),
            (Tool::Line, "line"),
            (Tool::Circle, "circle"),
        ] {
            let build = |in_sketch: bool| {
                context_pane_content(&ContextInput {
                    tool,
                    draw_rect_construction: (tool == Tool::Rectangle).then_some(false),
                    draw_line_construction: (tool == Tool::Line).then_some(false),
                    draw_circle_construction: (tool == Tool::Circle).then_some(false),
                    rect_anchor: (tool == Tool::Rectangle)
                        .then_some(crate::actions::RectAnchor::Corner),
                    circle_anchor: (tool == Tool::Circle)
                        .then_some(crate::actions::CircleAnchor::Center),
                    draw_line_curve_mode: (tool == Tool::Line).then_some(false),
                    draw_line_tangent_constraint: (tool == Tool::Line).then_some(false),
                    in_sketch,
                    ..input(&doc, &selection)
                })
            };
            assert_eq!(build(false), build(true), "{ctor} tool pane matches in 3D");
            assert!(
                build(false).snapping.is_some(),
                "{ctor} tool shows Snapping in 3D"
            );
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
            align_lines: false,
            inline_orientations: Vec::new(),
            style: crate::model::DrawingViewStyle::default(),
            label_hidden: false,
            label_pos: Default::default(),
            label_text: String::new(),
            auto_label: "Body 0 — Front".to_string(),
        };
        // Dimension tool: keeps the projection editor, but the Default-units section is now
        // suppressed like the other modeling/transform tools (#585).
        let dim = context_pane_content(&ContextInput {
            tool: Tool::Dimension,
            in_drawing_workbench: true,
            drawing_view: Some(view_control.clone()),
            ..input(&doc, &selection)
        });
        assert!(dim.drawing_view.is_some(), "Dimension tool keeps the projection editor");
        assert!(dim.units.is_none(), "Dimension tool no longer shows units (#585)");
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

    /// #486: the Dimension tool shows the same sketch-geometry element picker as Constraint.
    #[test]
    fn dimension_tool_shows_selection_picker() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(crate::model::ShapeKind::Line);
        let mut selection = SceneSelection::default();
        click_scene_selection(&mut selection, SceneElement::Line(0), false);
        let content = context_pane_content(&ContextInput {
            tool: Tool::Dimension,
            in_sketch: true,
            ..input(&doc, &selection)
        });
        let picker = content
            .selection_picker
            .expect("Dimension tool should show a selection picker");
        assert!(
            picker.picked().iter().any(|e| *e == SceneElement::Line(0)),
            "pre-selected line should appear in the Dimension picker"
        );
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

    /// #584: the Extrude tool surfaces its in-context distance/target/commit controls.
    #[test]
    fn extrude_tool_surfaces_distance_and_target_controls() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        // Distance-driven: a distance value, empty target rows, committable.
        let content = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            extrude: Some(ExtrudeControl {
                distance: "15 mm".to_string(),
                target_rows: Vec::new(),
                target_focused: false,
                can_commit: true,
                has_extrusion: true,
            }),
            ..input(&doc, &selection)
        });
        let control = content.extrude.expect("extrude control present");
        assert_eq!(control.distance, "15 mm");
        assert!(control.target_rows.is_empty());
        assert!(control.can_commit);

        // Target-driven: a picked "Up to" target with the distance field nulled.
        let content = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            extrude: Some(ExtrudeControl {
                distance: String::new(),
                target_rows: vec!["Plane 2".to_string()],
                target_focused: false,
                can_commit: true,
                has_extrusion: true,
            }),
            ..input(&doc, &selection)
        });
        let control = content.extrude.expect("extrude control present");
        assert!(control.distance.is_empty(), "distance is null while a target drives the depth");
        assert_eq!(control.target_rows, vec!["Plane 2".to_string()]);
    }

    /// #587: "Extrude into" and "Symmetric" surface for the Extrude tool even before a face is
    /// picked (no face picker rows yet), with Add/Cut disabled until a host body is known.
    #[test]
    fn extrude_body_and_symmetric_show_before_a_face() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let content = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            extrude_body_mode: Some(crate::actions::ExtrudeBodyMode::NewBody),
            extrude_symmetric: Some(false),
            extrude_merge_candidate: None,
            extrude_faces: Some(Vec::new()),
            ..input(&doc, &selection)
        });
        let body = content.extrude_body.expect("Extrude-into control shows before a face");
        assert_eq!(body.mode, crate::actions::ExtrudeBodyMode::NewBody);
        assert!(body.merge_body.is_none(), "Add/Cut stay disabled with no host body");
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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: Some(vec!["Block — vertical 0".to_string()]),
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
        };
        let content = context_pane_content(&base);
        let edges_picker = |rows: Vec<String>| EdgePickerControl {
            heading: "Edges",
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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
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
            in_sketch: true,
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
                axis_label: Some("the X axis".to_string()),
                value_field_focused: false,
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

    /// #646: typing in the Repeat section's Count/Offset/Distance fields blurs the Bodies
    /// picker (and the Axis picker) — the focus ring belongs where the keyboard is.
    #[test]
    fn repeat_value_field_focus_blurs_the_pickers() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let control = |value_field_focused, axis_label: Option<&str>| RepeatControl {
            targets: vec![7],
            plane_targets: Vec::new(),
            sketch_targets: Vec::new(),
            extrusion_targets: Vec::new(),
            axis_label: axis_label.map(str::to_string),
            value_field_focused,
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
        };
        let pane = |c: RepeatControl| {
            context_pane_content(&ContextInput {
                tool: Tool::Repeat,
                in_drawing_workbench: false,
                repeat_op: Some(c),
                ..input(&doc, &selection)
            })
        };
        // Axis already picked: the Bodies picker normally reads as focused…
        assert!(pane(control(false, Some("the X axis"))).tool_pickers[0].picker.is_focused());
        // …but not while a value field has the keyboard.
        assert!(!pane(control(true, Some("the X axis"))).tool_pickers[0].picker.is_focused());
        // With no axis yet, the Bodies picker defers to the Axis picker either way.
        assert!(!pane(control(false, None)).tool_pickers[0].picker.is_focused());
        assert!(!pane(control(true, None)).tool_pickers[0].picker.is_focused());
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
                tool_title: None,
                dimension_derive: None,
                name: None,
                curve_mode: None,
            rect_anchor: None,
            circle_anchor: None,
                tangent_constraint: None,
                construction: None,
                constraints: None,
                snapping: None,
                extrude_body: None,
                extrude_faces: None,
                extrude: None,
                edge_picker: None,
                selection_picker: Some(ElementPicker::select_everything()),
                tool_pickers: Vec::new(),
                calibrate_image: None,
                revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: None,
            drawing_align: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
                calibrate_pending: None,
                units: Some(UnitsControl {
                    sketch: None,
                    component: None,
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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
        });
        assert_eq!(
            content,
            ContextPaneContent {
                tool_title: None,
                dimension_derive: None,
                name: None,
                curve_mode: None,
            rect_anchor: None,
            circle_anchor: None,
                tangent_constraint: None,
                construction: Some(ConstructionControl {
                    value: TriState::On,
                    target_count: 1,
                }),
                constraints: None,
                snapping: None,
                extrude_body: None,
                extrude_faces: None,
                extrude: None,
                edge_picker: None,
                selection_picker: None,
            tool_pickers: Vec::new(),
                calibrate_image: None,
                revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: None,
            drawing_align: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
                calibrate_pending: None,
                units: Some(UnitsControl {
                    sketch: None,
                    component: None,
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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: Some(false),
            draw_circle_construction: None,
            draw_line_curve_mode: Some(true),
            draw_line_tangent_constraint: Some(false),
            in_sketch: true,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
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
                tool_title: None,
                dimension_derive: None,
                name: Some(NameControl {
                    element: SceneElement::Line(0),
                }),
                curve_mode: None,
            rect_anchor: None,
            circle_anchor: None,
                tangent_constraint: None,
                construction: Some(ConstructionControl {
                    value: TriState::Off,
                    target_count: 1,
                }),
                constraints: None,
                snapping: None,
                extrude_body: None,
                extrude_faces: None,
                extrude: None,
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
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: None,
            drawing_align: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
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
                component: None,
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
                component: None,
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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
        });
        assert_eq!(
            content,
            ContextPaneContent {
                tool_title: None,
                dimension_derive: None,
                name: Some(NameControl {
                    element: SceneElement::Line(0),
                }),
                curve_mode: None,
            rect_anchor: None,
            circle_anchor: None,
                tangent_constraint: None,
                construction: Some(ConstructionControl {
                    value: TriState::On,
                    target_count: 1,
                }),
                constraints: None,
                snapping: None,
                extrude_body: None,
                extrude_faces: None,
                extrude: None,
                edge_picker: None,
                selection_picker: None,
            tool_pickers: Vec::new(),
                calibrate_image: None,
                revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: None,
            drawing_align: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
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
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_rows: None,
            loft_rows: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            mirror_op: None,
            mirror_edit_start: None,
            repeat_op: None,
            sketch_repeat: None,
            sketch_offset: None,
            sketch_offset_edit_start: None,
            sketch_mirror: None,
            sketch_mirror_edit_start: None,
            sketch_slice: None,
            sketch_text: None,
            drawing_view: None,
            drawing_annotation: None,
            drawing_selection: Vec::new(),
            drawing_align_active: false,
            drawing_align_base: None,
            drawing_add_active: false,
            repeat_edit_start: None,
            slice_op: None,
            slice_edit_start: None,
            revolve_edit_start: None,
            sweep_edit_start: None,
            calibrate_start: None,
            calibrate_pending: None,
            dimension_derive: None,
        });
        assert_eq!(
            content.constraints.as_ref().map(|rows| rows.len()),
            Some(crate::geometric_constraints::GeometricConstraintType::ALL.len())
        );
    }

    /// #505: New/Add/Cut stay visible while extruding even without a host body; Add/Cut
    /// simply have no merge target until the sketch sits on a body face.
    #[test]
    fn extrude_body_modes_always_shown_while_extruding() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let content = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            extrude_body_mode: Some(ExtrudeBodyMode::NewBody),
            extrude_merge_candidate: None,
            extrude_symmetric: Some(false),
            ..input(&doc, &selection)
        });
        let control = content.extrude_body.expect("body control while extruding");
        assert_eq!(control.mode, ExtrudeBodyMode::NewBody);
        assert!(control.merge_body.is_none());
        assert!(!control.symmetric);

        let with_host = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            extrude_body_mode: Some(ExtrudeBodyMode::MergeInto(0)),
            extrude_merge_candidate: Some(0),
            extrude_symmetric: Some(true),
            ..input(&doc, &selection)
        });
        let control = with_host.extrude_body.expect("body control with host");
        assert_eq!(control.merge_body, Some(0));
        assert!(control.symmetric);
    }
}