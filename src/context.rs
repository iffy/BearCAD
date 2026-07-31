//! Context pane: union of editable properties for the current selection or draw op.

use crate::actions::{ExtrudeBodyMode, Tool};
use crate::document_health::{health_status_label, selection_frozen_summary, DocumentHealth, HealthStatus};
use crate::element_picker::{ElementFilter, ElementKind, ElementPicker, PickLimit, PickRule};
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
    /// The open drawing page (#967), so a drawing item's element can name which page it is on.
    pub open_drawing: Option<usize>,
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
    /// The open sketch, when there is one (#953) — what the sketch-scoped pickers restrict
    /// themselves to. `in_sketch` is `open_sketch.is_some()`; both are carried because most
    /// callers only need the flag.
    pub open_sketch: Option<crate::model::SketchId>,
    /// The open sketch's local X and Y axis directions as they currently project on
    /// screen (#751), so the axis-parallel constraint buttons can draw their glyphs at
    /// the angle the user actually sees. `None` outside a sketch (or edge-on views).
    pub sketch_axis_screen_dirs: Option<(egui::Vec2, egui::Vec2)>,
    /// Current snapping on/off state (shown as a toggle for snapping tools).
    pub snapping_enabled: bool,
    /// Body an in-progress/edited extrusion would join by default, if any (#32).
    pub extrude_merge_candidate: Option<usize>,
    /// Whether the in-progress extrusion's profiles form more than one disjoint solid (#837).
    pub extrude_disjoint_profiles: bool,
    /// Current new-body/merge-into choice for the in-progress/edited extrusion.
    pub extrude_body_mode: Option<ExtrudeBodyMode>,
    /// Symmetric extrude toggle while an extrusion is in progress (#504).
    pub extrude_symmetric: Option<bool>,
    /// The picked extrude profile faces (#268/#955), shown through the Extrude tool's face
    /// element picker; `None` when the Extrude tool isn't active.
    pub extrude_faces: Option<Vec<crate::model::ExtrudeFace>>,
    /// The Extrude tool's in-context distance/target/commit controls (#584); `Some` while an
    /// extrusion is in progress.
    pub extrude: Option<ExtrudeControl>,
    /// Selection-picker rows for the active tool (#157/#167): `Some` whenever the tool
    /// collects a selection set (Chamfer/Fillet outside a sketch — one row per edge in the
    /// in-progress treatment, empty while nothing is picked yet), `None` for other tools.
    pub edge_treatment_edges: Option<Vec<(usize, crate::model::ExtrusionEdgeRef)>>,
    /// Selection-picker rows for the Loft tool (#loft): one row per picked cross section,
    /// `Some` (possibly empty) whenever the Loft tool is active outside a sketch.
    pub loft_sections: Option<Vec<crate::model::LoftSection>>,
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
    /// Create Shape tool state (#909): `Some` while the Shape tool is active.
    pub shape: Option<ShapeControl>,
    /// Joint tool state (#894): `Some` while the Joint tool is active.
    pub joint: Option<JointControl>,
    /// "Edit joint" entry point: `Some(op)` when exactly one joint is selected (#894).
    pub joint_edit_start: Option<usize>,
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
    /// The in-progress dimension value (#775), mirrored into the pane.
    pub dimension_edit: Option<DimensionEditControl>,
    /// The in-progress chamfer/fillet amount (#792).
    pub treatment: Option<TreatmentControl>,
}

/// What the Revolve tool's context section shows (#revolve): the picked axis (if any),
/// the symmetric toggle, the body mode, and — in Cut mode — the picked bodies (rendered
/// through the shared selection picker).
#[derive(Clone, Debug, PartialEq)]
pub struct RevolveControl {
    /// The picked profile faces and the sweep axis (#955), rendered through real
    /// [`ElementPicker`]s rather than as label rows.
    pub faces: Vec<crate::model::ExtrudeFace>,
    pub axis: Option<crate::model::RevolveAxis>,
    /// Which picker shows the focus ring (#304): exactly one at a time — Profile until a
    /// face is picked, then Axis until the axis is set, then back to Profile.
    pub axis_focused: bool,
    pub symmetric: bool,
    pub body_choice: crate::actions::RevolveBodyChoice,
    /// In Cut mode, the picked bodies to cut (rendered through the unified element picker, #213).
    pub cut_bodies: Vec<usize>,
}

/// What the Sweep tool's context section shows (#sweep): the picked profile
/// faces, the picked path lines, the body mode, and — in Cut mode — the picked bodies.
#[derive(Clone, Debug, PartialEq)]
pub struct SweepControl {
    /// The picked profile faces and the path lines they sweep along (#955), rendered through
    /// real [`ElementPicker`]s rather than as label rows.
    pub faces: Vec<crate::model::ExtrudeFace>,
    pub path: Vec<usize>,
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
    /// What each anchor row was picked from, in row order (#955). Empty while re-opening a
    /// committed plane, whose stored definition keeps only the derived frame.
    pub anchor_elements: Vec<SceneElement>,
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
    /// Drop the `i`-th anchor row (#955). With one row that is the whole anchor, so it starts
    /// over; with a line+point set the surviving half becomes the anchor on its own.
    RemoveAnchor(usize),
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
    /// Construction planes (#217) and tracing images (#217) moving with them. They share the
    /// **one** Bodies picker rather than getting rows of their own — a Move takes "the things
    /// that move", and splitting them by kind would be three near-empty inputs (#963).
    pub plane_targets: Vec<usize>,
    pub image_targets: Vec<usize>,
    /// Snap (default) or free translation (#648) — the Translate dropdown.
    pub translate_mode: crate::model::MoveTranslateMode,
    /// Whether the Bodies picker is the focused one (#658) — false while any of the tool's
    /// other pickers is armed.
    pub bodies_focused: bool,
    /// **Start point A** (#668): the picked point, if any, and whether its picker is armed.
    /// Start points sit on a **moving** body, end points on stationary geometry (#953) — each
    /// picker enforces that itself.
    pub start_a: Option<crate::model::MovePointRef>,
    pub start_a_focused: bool,
    /// **End point A** (#668): the picked point, if any, and whether its picker is armed.
    pub end_a: Option<crate::model::MovePointRef>,
    pub end_a_focused: bool,
    /// Angle snap (#917): how far apart the rotation's candidate dots sit, in degrees
    /// (0–90). The row shows a slider and a value field side by side.
    pub angle_snap_deg: f32,
    /// The optional **B pair** (#669), which adds the rotation.
    pub start_b: Option<crate::model::MovePointRef>,
    pub start_b_focused: bool,
    pub end_b: Option<crate::model::MovePointRef>,
    pub end_b_focused: bool,
    /// The optional **C pair**, which pins the spin B leaves free.
    pub start_c: Option<crate::model::MovePointRef>,
    pub start_c_focused: bool,
    pub end_c: Option<crate::model::MovePointRef>,
    pub end_c_focused: bool,
    pub tx: String,
    pub ty: String,
    pub tz: String,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Move context section.
#[derive(Clone, Debug, PartialEq)]
pub enum MoveEdit {
    Tx(String),
    Ty(String),
    Tz(String),
    /// Translate dropdown (#648).
    TranslateMode(crate::model::MoveTranslateMode),
    /// Arm / clear the source-point picker (#649).
    StartAFocus,
    ClearStartA,
    /// Arm / clear the target-point picker (#650).
    EndAFocus,
    ClearEndA,
    /// Arm / clear the optional B-pair pickers (#669).
    StartBFocus,
    ClearStartB,
    EndBFocus,
    ClearEndB,
    /// Arm / clear the optional C-pair pickers.
    StartCFocus,
    ClearStartC,
    EndCFocus,
    ClearEndC,
    /// The rotation's candidate spacing in degrees (#917), clamped to 0–90 by the caller.
    AngleSnap(f32),
    Commit,
}

/// What the Create Shape tool's context section shows (#909): which shape, its labelled
/// dimensions, and whether it can be committed.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeControl {
    pub kind: crate::model::PrimitiveKind,
    /// The dimension the current placement phase is asking for (#912): its field takes the
    /// keyboard, so a size can be typed the moment a click lands.
    pub focus_field: Option<crate::actions::ShapeDimension>,
    pub width: String,
    pub depth: String,
    pub height: String,
    pub radius: String,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Shape context section (#909).
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeEdit {
    Kind(crate::model::PrimitiveKind),
    Dimension(crate::actions::ShapeDimension, String),
    Commit,
}

/// What the Joint tool's context section shows (#894): the picked parts, the joint-type
/// dropdown, the mating-point pickers, and the position expressions the kind offers.
#[derive(Clone, Debug, PartialEq)]
pub struct JointControl {
    /// The picked parts, in pick order (#955).
    pub members: Vec<crate::model::JointRef>,
    pub members_focused: bool,
    /// The two sides of a non-Rigid joint (#991), picked as named slots rather than as a list
    /// plus a swap button: which part moves and which holds it *is* the joint. Rigid keeps the
    /// plain Parts list — it joins any number of parts and none of them moves.
    pub mobile: Option<crate::model::JointRef>,
    pub fixed: Option<crate::model::JointRef>,
    /// Which of the two side slots the next click fills. Only one is ever armed.
    pub mobile_focused: bool,
    pub fixed_focused: bool,
    pub kind: crate::model::JointKind,
    /// The held side's label, shown on the Base row; clicking swaps sides.
    pub base_label: String,
    /// Every mate pick sits on one side or the other (#953/#1014): moving picks on the
    /// driven part, fixed picks on the base — each picker holds that rule, so a click on the
    /// wrong part is simply not a pick.
    pub driven_bodies: Vec<usize>,
    pub base_bodies: Vec<usize>,
    /// The face pair (#1014) and what it holds the part off by.
    pub moving_face: Option<crate::model::MateRef>,
    pub moving_face_focused: bool,
    pub fixed_face: Option<crate::model::MateRef>,
    pub fixed_face_focused: bool,
    pub flip: bool,
    pub offset: String,
    /// The line-up rows (#1015), each with its two picks and which of them is armed. The
    /// last is the one being picked into; there is none once nothing is left to pin.
    pub line_up: Vec<JointLineUpRow>,
    pub position: String,
    pub position2: String,
    pub position3: String,
    /// Travel limits (#896): expressions, plus the picked slide stops' labels.
    pub slide_min: String,
    pub slide_max: String,
    pub turn_min: String,
    pub turn_max: String,
    /// The travel stops (#896/#955): a plane or flat face the slide ends at.
    pub slide_min_stop: Option<crate::hierarchy::SceneElement>,
    pub slide_min_stop_focused: bool,
    pub slide_max_stop: Option<crate::hierarchy::SceneElement>,
    pub slide_max_stop_focused: bool,
    pub editing: bool,
    pub can_commit: bool,
    /// Whether the preview sweep animates (#906) — one app-wide switch, shown on every
    /// joint's pane.
    pub animate: bool,
}

/// A line-up row picker's registered name (#1015/#968): rows are numbered so a script can
/// name one, and the mate rows don't collide with the part slots above them. Beyond the
/// numbered few a row keeps the plain name — a mate is fully placed in two or three.
fn line_up_heading(row: usize, moving: bool) -> &'static str {
    const MOVING: [&str; 4] = ["Line up 1 moving", "Line up 2 moving", "Line up 3 moving", "Line up 4 moving"];
    const FIXED: [&str; 4] = ["Line up 1 fixed", "Line up 2 fixed", "Line up 3 fixed", "Line up 4 fixed"];
    let table = if moving { &MOVING } else { &FIXED };
    table.get(row).copied().unwrap_or(if moving { "Line up moving" } else { "Line up fixed" })
}

/// One line-up row of the Mate section (#1015).
#[derive(Clone, Debug, PartialEq)]
pub struct JointLineUpRow {
    pub moving: Option<crate::model::MateRef>,
    pub moving_focused: bool,
    pub fixed: Option<crate::model::MateRef>,
    pub fixed_focused: bool,
}

/// One edit from the Joint context section (#894).
#[derive(Clone, Debug, PartialEq)]
pub enum JointEdit {
    Kind(crate::model::JointKind),
    /// The screw's lead expression (mm per turn).
    Lead(String),
    /// Swap which side is held.
    SwapBase,
    /// Arm / clear the two side slots of a non-Rigid joint (#991).
    MobileFocus,
    ClearMobile,
    FixedFocus,
    ClearFixed,
    Position(String),
    Position2(String),
    Position3(String),
    /// Travel limits (#896).
    SlideMin(String),
    SlideMax(String),
    TurnMin(String),
    TurnMax(String),
    SlideMinStopFocus,
    ClearSlideMinStop,
    SlideMaxStopFocus,
    ClearSlideMaxStop,
    /// Turn the preview sweep's animation on/off for every joint (#906).
    Animate(bool),
    /// Capture the committed joint's current position as its rest pose (#898).
    SetRest,
    /// Put the committed joint back to its rest pose (#898).
    Revert,
    MembersFocus,
    RemoveMember(usize),
    ClearMembers,
    /// The face pair (#1014).
    MovingFaceFocus,
    ClearMovingFace,
    FixedFaceFocus,
    ClearFixedFace,
    /// Which way the moving part ends up facing, and the gap it's held off by.
    Flip(bool),
    Offset(String),
    /// A line-up row (#1015), by its index.
    LineUpMovingFocus(usize),
    ClearLineUpMoving(usize),
    LineUpFixedFocus(usize),
    ClearLineUpFixed(usize),
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
    /// How each reflection lands (#639): its own body, or joined to / cut from its source.
    pub mode: crate::model::MirrorMode,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Mirror context section (#523). The plane and the picked bodies are both
/// handled through the unified element pickers (`PickerTarget::MirrorPlane` /
/// `PickerTarget::MirrorTargets`), so this only covers the commit button.
#[derive(Clone, Debug, PartialEq)]
pub enum MirrorEdit {
    /// Output-row click (#639): New body / Join / Cut.
    Mode(crate::model::MirrorMode),
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
    /// The picked path (#439/#955): a straight reference, or a circle to ride round (#840).
    pub path: Option<crate::hierarchy::SceneElement>,
    /// Repeat **around** the path instead of along it (#839). While set, Distance becomes an
    /// Angle and the distance-target picker stands down.
    pub around_axis: bool,
    /// Whether the picked path can be turned about at all (#840): a curved one is only ever
    /// followed, so its "around" option is disabled.
    pub can_turn_about_path: bool,
    /// Run the pattern the **other way** along the path (#989). A path has two directions and
    /// picking one says nothing about which you meant, so this is how you say it.
    pub flip: bool,
    /// Label of the picked distance target (#645), if any — the face/plane/vertex the fill
    /// length is measured to. Empty means the Distance expression governs.
    /// The "Distance to" target (#645) the fill length is measured to, as an element (#955).
    pub length_target: Option<crate::hierarchy::SceneElement>,
    /// Whether the distance-target picker is armed (the next viewport click sets it, #645).
    pub length_target_focused: bool,
    /// The distance the picked target works out to, formatted — shown read-only in the
    /// Distance field while a target is set (#645).
    pub length_target_value: Option<String>,
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
    pub editing: bool,
    pub can_commit: bool,
}

/// What the in-sketch Repeat tool's context section shows (#232): the picked entities, the
/// repeat direction, and the count/gap/distance fields (which map onto the same variables as the
/// 3D repeat). Laid out like the 3D section (#835), one dimension down: element pickers for the
/// entities and the direction line, and the same three interlinked value rows.
#[derive(Clone, Debug, PartialEq)]
pub struct SketchRepeatControl {
    /// The sketch entities being copied, as element-picker rows.
    pub picked: Vec<SceneElement>,
    /// The picked direction line; `None` means the sketch's U axis.
    pub direction: Option<SceneElement>,
    /// Whether the direction picker is armed (the next viewport click sets it, #835).
    pub direction_focused: bool,
    /// Whether one of the section's value fields holds keyboard focus — while it does,
    /// neither picker reads as focused (mirrors the 3D section, #646).
    pub value_field_focused: bool,
    pub count: String,
    pub spacing: String,
    pub length: String,
    pub computed_var: crate::model::RepeatVar,
    pub gap_is_offset: bool,
    pub distance_is_end: bool,
    /// Formatted value of the computed variable, shown read-only in its field.
    pub computed_value: Option<String>,
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
    /// Move the green lock — compute this variable from the other two (#835).
    SetComputed(crate::model::RepeatVar),
    /// Drop one picked entity.
    Remove(SceneElement),
    /// Drop every picked entity.
    Clear,
    /// Arm the direction picker: the next viewport click sets the direction line (#835).
    DirectionFocus,
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
    /// Arm the distance-target picker (#645): the next viewport click sets it.
    LengthTargetFocus,
    /// Clear the picked distance target, handing Distance back to its expression (#645).
    ClearLengthTarget,
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
    /// Repeat along the picked path, or around it as an axis of rotation (#839).
    SetAroundAxis(bool),
    /// Run the pattern the other way along the picked path (#989).
    SetFlip(bool),
    Commit,
}

/// What the Slice tool's context section shows: the picked target bodies, the planar
/// cutters, which picker the next viewport click lands on, and the extend-to-infinity flag.
#[derive(Clone, Debug, PartialEq)]
pub struct SliceControl {
    /// The bodies being sliced, and the planar faces/planes doing the slicing (#955). The
    /// pane renders both through real [`ElementPicker`]s, so they carry their own filters,
    /// focus, and — for the cutters, which are consumed — the red highlight.
    pub targets: Vec<usize>,
    pub cutters: Vec<crate::model::FaceId>,
    /// `true` while the cutter picker is active (the next viewport click adds a cutter).
    pub picking_cutter: bool,
    pub extend_infinite: bool,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the Slice context section.
#[derive(Clone, Debug, PartialEq)]
pub enum SliceEdit {
    ExtendInfinite(bool),
    Commit,
}

/// In-sketch Slice control (#238): the two-role picker for slicing sketch lines/circles/faces by
/// cutter lines. Mirrors [`SliceControl`] but without the 3D extend-to-infinity toggle.
#[derive(Clone, Debug, PartialEq)]
pub struct SketchSliceControl {
    /// The sketch entities being sliced and the lines doing the slicing (#955). Targets are
    /// listed lines-then-circles-then-faces, which is the order the removal handler unpacks.
    pub targets: Vec<crate::hierarchy::SceneElement>,
    pub cutters: Vec<usize>,
    /// `true` while the cutter picker is active (the next viewport click adds a cutter line).
    pub picking_cutter: bool,
    pub editing: bool,
    pub can_commit: bool,
}

/// One edit from the in-sketch Slice context section (#238).
#[derive(Clone, Debug, PartialEq)]
pub enum SketchSliceEdit {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// The blue primary button / Enter — commit the revolve (#586).
    Commit,
}

/// One edit from the Sweep context section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SweepEdit {
    BodyChoice(crate::actions::RevolveBodyChoice),
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

/// Tools that snap while drawing or moving sketch geometry.
pub fn tool_uses_snapping(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Select | Tool::Line | Tool::Rectangle | Tool::Circle | Tool::Shape
    )
}

/// The egui id of one of the Repeat section's value fields — the same id
/// [`crate::expression_input::ValueInput`] is built with when the row renders.
pub fn repeat_value_field_id(label: &str) -> egui::Id {
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
    /// The selected unit instance's link/source/placement section (#734).
    pub unit_instance: Option<UnitInstanceControl>,
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
    /// On-screen directions of the sketch's local X/Y axes (#751), for the axis-parallel
    /// constraint buttons' rotated glyphs.
    pub constraint_axis_dirs: Option<(egui::Vec2, egui::Vec2)>,
    /// `Some(enabled)` when the current tool snaps; renders an enable/disable toggle.
    pub snapping: Option<bool>,
    /// New-body/merge-into choice for an in-progress or edited extrusion (#32).
    pub extrude_body: Option<ExtrudeBodyControl>,
    /// In-context distance/target/commit controls for the Extrude tool (#584).
    pub extrude: Option<ExtrudeControl>,
    /// Default length/angle unit picker: document-level when nothing is selected, or
    /// per-sketch (with a "follow document" inherit option) when a single sketch is
    /// selected (#52).
    pub units: Option<UnitsControl>,
    /// Material picker for the selected bodies (#834).
    pub material: Option<MaterialControl>,
    /// The unified element-picker control (#213). Populated for tools already migrated to
    /// [`ElementPicker`] — currently the Select tool's "select everything" picker, which is
    /// always shown (placeholder when empty) and never loses focus.
    pub selection_picker: Option<ElementPicker>,
    /// Dimension tool in 3D mode (#618): the derived-parameter name/value/commit block,
    /// rendered right under the selection picker.
    pub dimension_derive: Option<DimensionDeriveView>,
    /// The dimension being typed right now (#775).
    pub dimension_edit: Option<DimensionEditControl>,
    /// The chamfer/fillet being set right now (#792).
    pub treatment: Option<TreatmentControl>,
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
    /// Create Shape tool state (#909): `Some` while the Shape tool is active.
    pub shape: Option<ShapeControl>,
    /// Joint tool state (#894): `Some` while the Joint tool is active.
    pub joint: Option<JointControl>,
    /// "Edit joint" entry point: `Some(op)` when exactly one joint is selected (#894).
    pub joint_edit_start: Option<usize>,
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

/// What the units picker in the context pane should show and let the user change.
///
/// The material picker for the selected bodies (#834): what they're made of, and the way in
/// to naming/recolouring that material.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialControl {
    /// The selected bodies this assigns to.
    pub bodies: Vec<usize>,
    /// The material they all share; `None` when they disagree. `Some(None)` is the default
    /// material — no material assigned.
    pub current: Option<Option<usize>>,
    /// Every live material: index, name, colour.
    pub materials: Vec<(usize, String, [u8; 3])>,
}

/// One edit from the material picker (#834).
#[derive(Clone, Debug, PartialEq)]
pub enum MaterialEdit {
    /// Assign this material (or the default, with `None`) to the selected bodies.
    Assign(Option<usize>),
    /// Create a material and give it to the selected bodies.
    New,
    Rename(usize, String),
    Recolor(usize, [u8; 3]),
}

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
    /// Whether the picked profiles fall into more than one disjoint solid (#837). With no host
    /// body, that's what **Join** joins: one body instead of one per profile.
    pub can_join_profiles: bool,
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
    /// The extrude-to target (#584/#955): the plane, face, vertex, or repeat-instance face the
    /// depth runs up to, as the element the picker holds.
    pub target: Option<crate::hierarchy::SceneElement>,
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

/// The selected unit instance's section (#734): link mode, source (with staleness), and
/// placement values. Moving an instance is the Move tool's job (#735) — the pane shows
/// where it sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitInstanceControl {
    pub instance: usize,
    pub unit: usize,
    pub link: crate::model::LinkMode,
    /// The source file's name plus how it's referenced ("library" / "relative").
    pub source: String,
    /// Translation summary, e.g. "5, 0, 12.5 mm".
    pub position: String,
    /// Rotation summary, e.g. "90° about 0, 0, 1"; "—" when unrotated.
    pub rotation: String,
}

/// Edits the unit-instance section can make (#734), applied by the frame loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitPaneEdit {
    SetLink { unit: usize, link: crate::model::LinkMode },
    /// Update the embedded copy from the source file (#732).
    Sync { unit: usize },
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
    /// Where this picker draws. **Every** picker belongs in `tool_pickers` regardless (#958):
    /// focus, hover, the tool-switch handoff, the Exploder's fan and `bearcad.pickers()` all
    /// read that list, and a picker missing from it is invisible to every one of them.
    pub render: PickerRender,
}

/// Where a [`ToolPickerView`] draws (#958).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerRender {
    /// In the shared picker block at the top of the tool's section — the default.
    Shared,
    /// By the tool's own pane block, in place among its other controls: the Move tool's point
    /// rows sit between the Rotation heading and the Angle-snap slider, so they can't be
    /// hoisted to the top. The shared block skips these.
    Inline,
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
    /// The Extrude tool's profile faces (`CreatingExtrusion::faces`, #268/#955).
    ExtrudeProfile,
    /// The 3D Chamfer/Fillet tool's analytic edges (`CreatingEdgeTreatment::edges`, #955).
    TreatmentEdges,
    /// The Loft tool's cross sections (`CreatingLoft::sections`, #955).
    LoftSections,
    /// The Joint tool's member parts (`CreatingJoint::members`, #894/#955).
    JointMembers,
    /// The **mobile** (driven) side of a two-sided joint (#991).
    JointMobile,
    /// The **fixed** (held) side — the base.
    JointFixed,
    /// The Move tool's six mating-point pickers (#649/#650/#958). Rendered inline among the
    /// tool's other controls, but registered like every other picker.
    MoveStartA,
    MoveEndA,
    MoveStartB,
    MoveEndB,
    MoveStartC,
    MoveEndC,
    /// The Joint tool's mate pickers (#1014/#1015) and its two travel stops (#896/#958),
    /// likewise inline but registered: the face pair, then a moving/fixed pair per line-up
    /// row (indexed, since the rows appear one at a time as the mate is pinned down).
    JointMovingFace,
    JointFixedFace,
    JointLineUpMoving(usize),
    JointLineUpFixed(usize),
    JointMinStop,
    JointMaxStop,
    /// Extrude's "Up to" target and Repeat's "Distance to" (#584/#645/#958): inline under
    /// their tools' Distance fields, registered like every other picker.
    ExtrudeUpTo,
    RepeatDistanceTo,
    /// The Sweep tool's profile faces (`CreatingSweep::faces`, #955).
    SweepProfile,
    /// The Sweep tool's path lines (`CreatingSweep::path`, #955), chained tip-to-tail at commit.
    SweepPath,
    /// The Revolve tool's profile faces (`CreatingRevolve::faces`, #955).
    RevolveProfile,
    /// The Revolve tool's sweep axis (`CreatingRevolve::axis`, #955): a straight reference —
    /// a sketch line, a body edge, or a world axis. Single-pick.
    RevolveAxis,
    /// The in-sketch Slice tool's target entities (#955): lines, circles, and faces.
    SketchSliceTargets,
    /// The in-sketch Slice tool's cutter lines (#955).
    SketchSliceCutters,
    /// The Slice tool's target bodies (`CreatingSlice::targets`, #955).
    SliceTargets,
    /// The Slice tool's cutter faces/planes (`CreatingSlice::cutters`, #955). Consumed
    /// destructively, so they carry the red highlight override.
    SliceCutters,
    /// The Mirror tool's mirror plane (`CreatingMirror::plane`, #566): a plane or flat face.
    MirrorPlane,
    /// The Mirror tool's target bodies (`CreatingMirror::targets`, #523).
    MirrorTargets,
    /// The Repeat tool's target bodies (`CreatingRepeat::targets`).
    RepeatTargets,
    /// The in-sketch Repeat tool's entities and its direction line (#232/#835/#958).
    SketchRepeatEntities,
    SketchRepeatDirection,
    /// The in-sketch Offset tool's entity set (#493/#958).
    SketchOffsetEntities,
    /// The in-sketch Mirror tool's mirror line and the shapes it reflects (#534/#958).
    SketchMirrorLine,
    SketchMirrorShapes,
    /// The Repeat tool's path (`CreatingRepeat::path`, #840/#958): a straight reference to
    /// travel along, or a circle to ride round. Inline, under the tool's own controls.
    RepeatPath,
    /// The Combine tool's side-A bodies (`CreatingBoolean::a`).
    CombineA,
    /// The Combine tool's side-B bodies (`CreatingBoolean::b`).
    CombineB,
    /// The drawing Select tool's picked page items (#346/#967).
    DrawingSelection,
    /// The Aligned-view tool's base projection (#365/#967): the view a new aligned view lines
    /// up with. Single-pick, projections only.
    DrawingAlignBase,
    /// The unified selection picker (#213): what the Select, Constraint, Dimension,
    /// Chamfer/Fillet, Sketch and Project tools pick into.
    Selection,
    /// The Construction Plane tool's anchor set (`CreatingConstructionPlane::anchor_elements`,
    /// #474/#483/#955): a face, a straight edge or axis, a vertex, or a line **and** a point.
    PlaneAnchor,
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

/// The dimension being typed (#775): the pane mirrors the floating value input so a
/// dimension can be set from either place, and offers the same blue **Go** button every
/// other tool commits with.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionEditControl {
    pub text: String,
    /// Angles get an angle-flavoured input (and the "Angle" label); lengths get "Span".
    pub is_angle: bool,
}

/// A user edit from the Dimension tool's value block (#775).
#[derive(Clone, Debug, PartialEq)]
pub enum DimensionEditEdit {
    SetText(String),
    Commit,
}

/// The in-progress chamfer/fillet amount (#792): the pane mirrors the floating amount field
/// and offers the same blue **Go** button the other tools commit with.
#[derive(Clone, Debug, PartialEq)]
pub struct TreatmentControl {
    pub text: String,
    pub kind: crate::model::VertexTreatmentKind,
}

impl TreatmentControl {
    /// A fillet is set by radius, a chamfer by how far back it cuts.
    pub fn label(&self) -> &'static str {
        match self.kind {
            crate::model::VertexTreatmentKind::Fillet => "Radius",
            crate::model::VertexTreatmentKind::Chamfer => "Distance",
        }
    }
}

/// A user edit from the chamfer/fillet amount block (#792).
#[derive(Clone, Debug, PartialEq)]
pub enum TreatmentEdit {
    SetText(String),
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
    doc: &Document,
    tool: Tool,
    open_sketch: Option<crate::model::SketchId>,
    selection: &SceneSelection,
) -> Option<ElementPicker> {
    let in_sketch = open_sketch.is_some();
    // While a sketch is open the sketch-scoped pickers take only what that sketch owns (#742) —
    // its own geometry, the origin and its axes, and the sketched-on face's edges and corners.
    // One rule (#953), shared with the hover and click paths, instead of three copies.
    let scoped = |filter: ElementFilter| match open_sketch {
        Some(sketch) => filter.rule(PickRule::InSketch(sketch)),
        None => filter,
    };
    let mut picker = match tool {
        // Select: the selection **is** this picker (#966), so it's handed back rather than
        // rebuilt — that's what gives the popup's rows a stable order without sorting by each
        // element's debug string, and what makes any rule it carries real rather than applied
        // after the fact. While a sketch is open it carries the sketch-only rule (#742) like
        // every other sketch-scoped picker, so the Exploder's fan and the hover path can never
        // offer what the click path would refuse (#982).
        Tool::Select => {
            let picker = selection.picker().clone();
            return Some(match open_sketch {
                Some(sketch) => picker.with_rule(PickRule::InSketch(sketch)),
                None => picker,
            });
        }
        // Constraint / Dimension: sketch geometry only (points, lines, circles, body/face
        // edges). Dimension's picker mirrors the live selection so a pre-selected line or
        // pair shows up and the tool can proceed as if those were just picked (#486).
        Tool::Constraint | Tool::Dimension if in_sketch => {
            let mut p = ElementPicker::new(
                scoped(ElementFilter::kinds(&[
                    ElementKind::Vertex,
                    ElementKind::Line,
                    ElementKind::Circle,
                    ElementKind::Edge,
                    // A placed constraint's badge is selectable too (#568) — that is how one is
                    // reached to edit or delete it, and it is why these tools' fans offer badges.
                    ElementKind::Constraint,
                ])),
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
                scoped(ElementFilter::kind(ElementKind::Vertex)),
                PickLimit::Infinite,
            );
            p.set_focused(true);
            p
        }
        // Sketch / Text outside a sketch: pick a single face plane to open (#497). A body's own
        // cap or side wall is sketchable too (#465), and that is the *analytic* face — the
        // plane the sketch sits on, not the triangles it renders as (#957).
        //
        // The draw tools join them (#958): outside a sketch, a Rectangle/Line/Circle/Offset
        // click picks the face to sketch on before it draws anything, which is the same pick.
        // Saying so gives all six the same hover and the same Exploder fan — reaching a datum
        // plane buried behind a body is exactly what the fan is for.
        Tool::Sketch
        | Tool::Text
        | Tool::Rectangle
        | Tool::Line
        | Tool::Circle
        | Tool::Offset
            if !in_sketch =>
        {
            let mut p = ElementPicker::new(
                ElementFilter::kinds(&[ElementKind::Plane, ElementKind::Profile]),
                PickLimit::Finite(1),
            );
            p.set_focused(true);
            p
        }
        // Project in a sketch (#498/#983): only what a projection can source — outside body
        // corners/edges/bodies and planes that cross the sketch — plus this sketch's already-
        // projected lines, picked to un-project them. The rule is what keeps the sketch's own
        // drawn geometry out of the fan and the hover.
        Tool::Project if in_sketch => {
            let sketch = open_sketch.expect("in_sketch guard");
            let mut p = ElementPicker::new(
                ElementFilter::kinds(&[
                    ElementKind::Plane,
                    ElementKind::Vertex,
                    ElementKind::Line,
                    ElementKind::Edge,
                    ElementKind::Body,
                ])
                .rule(PickRule::ProjectableInto(sketch)),
                PickLimit::Infinite,
            );
            p.set_focused(true);
            p
        }
        _ => return None,
    };
    // Mirror the live selection, keeping only what this picker accepts (its filter drops the
    // rest); `set_picked` preserves order so the popup rows line up with `picked()`.
    picker.set_picked(doc, selection.ordered());
    Some(picker)
}

/// Build a Body-filtered tool picker (#213) from a tool's picked body-index set. `selected_color`
/// overrides the highlight (e.g. red for bodies that get cut). Focused, since it's the set the
/// active tool's viewport clicks feed.
fn body_tool_picker(
    doc: &Document,
    heading: &'static str,
    target: PickerTarget,
    bodies: &[usize],
    selected_color: Option<eframe::egui::Color32>,
    focused: bool,
) -> ToolPickerView {
    // Every body-set picker refuses a body that's deleted or already consumed by another
    // operation (#953) — the `!deleted && !shadow` gate the click paths each re-checked by hand.
    let mut picker = ElementPicker::new(
        ElementFilter::kind(ElementKind::Body).rule(PickRule::LiveBody),
        PickLimit::Infinite,
    );
    if let Some(color) = selected_color {
        picker = picker.with_selected_color(color);
    }
    picker.set_focused(focused);
    picker.set_picked(doc, bodies.iter().map(|&bi| SceneElement::Body(bi)));
    ToolPickerView {
        heading,
        picker,
        target,
        separator_above: true,
        render: PickerRender::Shared,
    }
}

/// The active tool's title for the top of the context pane (#608). Every modelling/sketch tool
/// gets a title; the Select tool and drawing workbench return `None` (they show selection info
/// or their own section headings). The "Edit …" variants surface when a committed operation is
/// being re-edited through its tool.
fn tool_context_title(input: &ContextInput<'_>) -> Option<&'static str> {
    use crate::actions::Tool;
    // The drawing workbench has its own titled sections (View / Projection), not a tool title.
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
        Tool::Project => "Projection",
        Tool::Loft => "Loft",
        Tool::Revolve => "Revolve",
        Tool::Shape => match (input.shape.as_ref().map(|c| c.kind), editing) {
            (Some(kind), true) => return Some(match kind {
                crate::model::PrimitiveKind::Cuboid => "Edit cuboid",
                crate::model::PrimitiveKind::Cylinder => "Edit cylinder",
                crate::model::PrimitiveKind::Sphere => "Edit sphere",
            }),
            (Some(kind), false) => crate::names::primitive_kind_label(kind),
            (None, _) => "Shape",
        },
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
        Tool::Joint => {
            if input.joint.as_ref().is_some_and(|c| c.editing) {
                "Edit joint"
            } else {
                "Joint"
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
            // The title says which way the copies run (#839).
            (false, false) if input.repeat_op.as_ref().is_some_and(|r| r.around_axis) => {
                "Rotational repeat"
            }
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

/// Test hook for [`unit_instance_control`] (#734).
#[cfg(test)]
pub fn unit_instance_control_for_tests(
    doc: &Document,
    instance: usize,
) -> Option<UnitInstanceControl> {
    unit_instance_control(doc, instance)
}

/// Build the selected unit instance's section (#734) from the document.
fn unit_instance_control(doc: &Document, instance: usize) -> Option<UnitInstanceControl> {
    let inst = doc.unit_instances.get(instance).filter(|i| !i.deleted)?;
    let unit = doc.units.get(inst.unit)?;
    let (path, kind) = match &unit.source {
        crate::model::UnitSource::RelativePath(p) => (p, "relative"),
        crate::model::UnitSource::Library(p) => (p, "library"),
    };
    let file = std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let axis_len = |expr: &str| {
        let expr = expr.trim();
        if expr.is_empty() {
            0.0
        } else {
            crate::value::eval_length_mm_in_doc(expr, doc).unwrap_or(0.0)
        }
    };
    let p = &inst.placement;
    let position = format!(
        "{}, {}, {} mm",
        axis_len(&p.tx),
        axis_len(&p.ty),
        axis_len(&p.tz)
    );
    let angle_rad = if p.angle.trim().is_empty() {
        0.0
    } else {
        crate::value::eval_angle_rad_in_doc(&p.angle, doc).unwrap_or(0.0)
    };
    let rotation = if angle_rad.abs() < 1e-6 {
        "—".to_string()
    } else {
        format!(
            "{:.1}° about {}, {}, {}",
            angle_rad.to_degrees(),
            p.axis[0],
            p.axis[1],
            p.axis[2]
        )
    };
    Some(UnitInstanceControl {
        instance,
        unit: inst.unit,
        link: unit.link,
        source: format!("{file} — {kind}"),
        position,
        rotation,
    })
}

/// The axis-parallel constraint buttons' hand-painted glyph (#751): a double-headed
/// arrow in the axis's own colour, drawn along `dir` — the axis's current on-screen
/// direction (already projected, so a tilted view rotates the glyph but never skews it).
fn axis_constraint_button(
    ui: &mut egui::Ui,
    enabled: bool,
    dir: egui::Vec2,
    color: egui::Color32,
) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let icon = crate::icons::ICON_DISPLAY_SIZE;
    let size = egui::vec2(icon, icon) + padding * 2.0;
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter()
            .rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        ui.painter().rect_stroke(
            rect,
            visuals.corner_radius,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let color = if enabled { color } else { color.gamma_multiply(0.35) };
        let d = dir.normalized();
        let half = icon / 2.0 - 1.5;
        let (a, b) = (rect.center() - d * half, rect.center() + d * half);
        let stroke = egui::Stroke::new(1.8, color);
        let painter = ui.painter();
        painter.line_segment([a, b], stroke);
        // Arrowheads on both ends, so the glyph reads as the axis, not just a slash.
        let barb = |tip: egui::Pos2, back: egui::Vec2| {
            for angle in [0.5f32, -0.5] {
                let wing = egui::emath::Rot2::from_angle(angle) * back;
                painter.line_segment([tip, tip + wing * 4.5], stroke);
            }
        };
        barb(b, -d);
        barb(a, d);
    }
    response
}

/// Every element picker the active tool has (#963), in order — its **primary** first.
///
/// Built on demand from the tool's state rather than cached from the last pane frame: a pick
/// arriving before such a frame (right after a tool switch, or in a headless test) would
/// otherwise find no pickers and silently do nothing. The context pane renders from this; so
/// does the click routing, the viewport highlight, the hover fallback and `bearcad.pickers()`.
/// Scope a filter to the open sketch (#742): an in-sketch tool picks that sketch's own
/// geometry and nothing else. One rule rather than the same `element_in_sketch` check written
/// out again in each of the hover and click paths (#958).
fn in_sketch(filter: ElementFilter, sketch: Option<crate::model::SketchId>) -> ElementFilter {
    match sketch {
        Some(sketch) => filter.rule(PickRule::InSketch(sketch)),
        None => filter,
    }
}

pub fn tool_picker_views(input: &ContextInput<'_>) -> Vec<ToolPickerView> {
    let mut tool_pickers = Vec::new();
    // The selection picker (#213) is a tool picker like any other — it is what Select,
    // Constraint, Dimension, Chamfer/Fillet, Sketch and Project pick into. Registering it here
    // is what lets hover, the handoff, the Exploder's fan and `bearcad.pickers()` see it (#958);
    // it draws where it always has, at the top of the pane. Suppressed while a draw
    // construction owns the pane, or in the drawing workbench.
    // A draw construction owns the pane while its tool is drawing *in a sketch*. Outside one,
    // those tools are picking the face to sketch on, and that pick wants a picker (#958).
    let drawing = input.open_sketch.is_some()
        && (input.draw_rect_construction.is_some()
            || input.draw_line_construction.is_some()
            || input.draw_circle_construction.is_some());
    if !drawing && !input.in_drawing_workbench {
        if let Some(picker) =
            selection_picker_for(input.doc, input.tool, input.open_sketch, input.selection)
        {
            tool_pickers.push(ToolPickerView {
                heading: "Selection",
                picker,
                target: PickerTarget::Selection,
                separator_above: true,
                render: PickerRender::Inline,
            });
        }
    }
    if let Some(r) = input.revolve.as_ref() {
        // Revolve's own two inputs (#955): the profile faces it sweeps, and the axis it sweeps
        // them about. Exactly one shows the focus ring (#304).
        let mut profile = ElementPicker::new(
            ElementFilter::kind(ElementKind::Profile),
            PickLimit::Infinite,
        );
        // Profile is the primary, so it stays armed unless another picker's turn has come:
        // the Axis while there's a profile and no axis, or the Cut bodies once both are
        // settled in Cut mode. Otherwise nothing would be armed and a click would do nothing
        // (#962/#970).
        let cut_armed = r.body_choice == crate::actions::RevolveBodyChoice::Cut
            && !r.faces.is_empty()
            && r.axis.is_some();
        profile.set_focused(!r.axis_focused && !cut_armed);
        profile.set_picked(
            input.doc,
            r.faces
                .iter()
                .map(crate::extrude::extrude_face_scene_element),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Profile",
            picker: profile,
            target: PickerTarget::RevolveProfile,
            separator_above: true,
            render: PickerRender::Shared,
        });
        // A straight reference only (#953): a sketch line with no curve to it, a body's feature
        // edge, or a world axis — never a circle.
        let mut axis = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Line, ElementKind::Edge, ElementKind::Axis])
                .rule(PickRule::Straight),
            PickLimit::Finite(1),
        );
        axis.set_focused(r.axis_focused);
        axis.set_picked(
            input.doc,
            r.axis.map(SceneElement::from_revolve_axis),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Axis",
            picker: axis,
            target: PickerTarget::RevolveAxis,
            separator_above: false,
            render: PickerRender::Shared,
        });
        if r.body_choice == crate::actions::RevolveBodyChoice::Cut {
            // Only once the profile and axis are settled — until then one of those wears the
            // ring, and two rings claim two places for the next click (#962).
            let ready = !r.faces.is_empty() && r.axis.is_some();
            let mut cut = body_tool_picker(
                input.doc,
                "Cut bodies",
                PickerTarget::RevolveCut,
                &r.cut_bodies,
                Some(crate::theme::CUT_ACCENT),
                ready,
            );
            cut.separator_above = false;
            tool_pickers.push(cut);
        }
    }
    if let Some(edges) = input.edge_treatment_edges.as_ref() {
        // The 3D Chamfer/Fillet edge set (#166/#955): the analytic edges one amount applies to.
        let mut picker =
            ElementPicker::new(ElementFilter::kind(ElementKind::Edge), PickLimit::Infinite);
        picker.set_focused(true);
        picker.set_picked(
            input.doc,
            edges.iter().map(|&(extrusion, edge)| {
                SceneElement::ExtrusionEdge { extrusion, edge }
            }),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Edges",
            picker,
            target: PickerTarget::TreatmentEdges,
            separator_above: true,
            render: PickerRender::Shared,
        });
    }
    if let Some(sections) = input.loft_sections.as_ref() {
        // A loft section is a closed profile, so it needs no element of its own (#952): the
        // analytic face already names it.
        let cutting = input
            .loft_body
            .as_ref()
            .is_some_and(|l| l.body_choice == crate::actions::RevolveBodyChoice::Cut);
        let mut picker =
            ElementPicker::new(ElementFilter::kind(ElementKind::Profile), PickLimit::Infinite);
        picker.set_focused(!(cutting && sections.len() >= 2));
        picker.set_picked(
            input.doc,
            sections
                .iter()
                .map(|s| crate::extrude::extrude_face_scene_element(&s.face)),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Sections",
            picker,
            target: PickerTarget::LoftSections,
            separator_above: true,
            render: PickerRender::Shared,
        });
    }
    if let Some(j) = input.joint.as_ref() {
        // The Joint tool's mate pickers (#1014/#1015): the face pair, then a moving/fixed
        // pair per line-up row. Inline, like the Move tool's point rows — registered so
        // focus, hover and scripts see them.
        //
        // The moving side is narrowed to the driven part's bodies; the fixed side to the
        // base's, plus the document's own geometry (#1018) — `OffBodies` already counts a
        // datum plane, a world axis and the origin as stationary, so grounding the first
        // part of an assembly against the world falls out.
        let mate_picker = |kinds: &[ElementKind], on_moving: bool, focused: bool| {
            let rule = if on_moving {
                PickRule::OnBodies(j.driven_bodies.clone())
            } else {
                PickRule::OffBodies(j.driven_bodies.clone())
            };
            let mut picker =
                ElementPicker::new(ElementFilter::kinds(kinds).rule(rule), PickLimit::Finite(1));
            picker.set_focused(focused);
            picker
        };
        const FACE_KINDS: [ElementKind; 2] = [ElementKind::Face, ElementKind::Plane];
        const LINE_UP_KINDS: [ElementKind; 3] =
            [ElementKind::Vertex, ElementKind::Edge, ElementKind::Axis];
        for (heading, target, pick, on_moving, focused) in [
            (
                "Moving face",
                PickerTarget::JointMovingFace,
                j.moving_face,
                true,
                j.moving_face_focused,
            ),
            (
                "Fixed face",
                PickerTarget::JointFixedFace,
                j.fixed_face,
                false,
                j.fixed_face_focused,
            ),
        ] {
            let mut picker = mate_picker(&FACE_KINDS, on_moving, focused);
            picker.set_picked(input.doc, pick.as_ref().map(SceneElement::from_mate_ref));
            tool_pickers.push(ToolPickerView {
                heading,
                picker,
                target,
                separator_above: false,
                render: PickerRender::Inline,
            });
        }
        for (i, row) in j.line_up.iter().enumerate() {
            for (heading, target, pick, on_moving, focused) in [
                (
                    line_up_heading(i, true),
                    PickerTarget::JointLineUpMoving(i),
                    row.moving,
                    true,
                    row.moving_focused,
                ),
                (
                    line_up_heading(i, false),
                    PickerTarget::JointLineUpFixed(i),
                    row.fixed,
                    false,
                    row.fixed_focused,
                ),
            ] {
                let mut picker = mate_picker(&LINE_UP_KINDS, on_moving, focused);
                picker.set_picked(input.doc, pick.as_ref().map(SceneElement::from_mate_ref));
                tool_pickers.push(ToolPickerView {
                    heading,
                    picker,
                    target,
                    separator_above: false,
                    render: PickerRender::Inline,
                });
            }
        }
        for (heading, target, stop, focused) in [
            ("Min stop", PickerTarget::JointMinStop, j.slide_min_stop.clone(), j.slide_min_stop_focused),
            ("Max stop", PickerTarget::JointMaxStop, j.slide_max_stop.clone(), j.slide_max_stop_focused),
        ] {
            let mut picker = ElementPicker::new(
                ElementFilter::kinds(&[ElementKind::Plane, ElementKind::Profile]),
                PickLimit::Finite(1),
            );
            picker.set_focused(focused);
            picker.set_picked(input.doc, stop);
            tool_pickers.push(ToolPickerView {
                heading,
                picker,
                target,
                separator_above: false,
                render: PickerRender::Inline,
            });
        }
        // The Joint tool's two parts (#894/#955). It renders in the Joint block (between the
        // Base row and the kind dropdown), but it belongs here so focus, hover, the handoff
        // and `bearcad.pickers()` can see it (#958).
        // **Rigid** joins any number of parts and none of them moves, so it keeps a plain list.
        if matches!(j.kind, crate::model::JointKind::Rigid) {
            let mut members = ElementPicker::new(
                ElementFilter::kinds(&[
                    ElementKind::Body,
                    ElementKind::Component,
                    ElementKind::Joint,
                ])
                .rule(PickRule::LiveBody),
                PickLimit::Finite(2),
            );
            members.set_focused(j.members_focused);
            members.set_picked(
                input.doc,
                j.members.iter().map(|m| SceneElement::from_joint_ref(*m)),
            );
            tool_pickers.push(ToolPickerView {
                heading: "Parts",
                picker: members,
                target: PickerTarget::JointMembers,
                separator_above: true,
                render: PickerRender::Inline,
            });
        }
        // Every other kind joins exactly **two** parts, and which one moves is the whole
        // meaning of the joint — so those are picked as two named single-slot inputs (#991),
        // the mobile part first and the part it is held against second. They **replace** the
        // Parts list rather than sitting beside it: two pickers claiming the same picks would
        // put two focus rings on the pane and send the click to whichever was registered first.
        // Registered like every other picker so focus, hover, the handoff and
        // `bearcad.pickers()` see them (#958).
        else {
            let side = |focused: bool, part: Option<crate::model::JointRef>| {
                let mut p = ElementPicker::new(
                    ElementFilter::kinds(&[
                        ElementKind::Body,
                        ElementKind::Component,
                        ElementKind::Joint,
                    ])
                    .rule(PickRule::LiveBody),
                    PickLimit::Finite(1),
                );
                p.set_focused(focused);
                p.set_picked(input.doc, part.map(SceneElement::from_joint_ref));
                p
            };
            tool_pickers.push(ToolPickerView {
                heading: "Moving part",
                picker: side(j.mobile_focused, j.mobile),
                target: PickerTarget::JointMobile,
                separator_above: false,
                render: PickerRender::Inline,
            });
            tool_pickers.push(ToolPickerView {
                heading: "Fixed part",
                picker: side(j.fixed_focused, j.fixed),
                target: PickerTarget::JointFixed,
                separator_above: false,
                render: PickerRender::Inline,
            });
        }
    }
    if let Some(e) = input.extrude.as_ref() {
        // Extrude's "Up to" (#584/#958): a single plane, face or vertex the depth runs to.
        // Inline — it sits under the Distance field — but registered like every other picker.
        let mut target = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Plane,
                ElementKind::Profile,
                ElementKind::Vertex,
            ]),
            PickLimit::Finite(1),
        );
        target.set_focused(e.target_focused);
        target.set_picked(input.doc, e.target.clone());
        tool_pickers.push(ToolPickerView {
            heading: "Up to",
            picker: target,
            target: PickerTarget::ExtrudeUpTo,
            separator_above: false,
            render: PickerRender::Inline,
        });
    }
    if let Some(faces) = input.extrude_faces.as_ref() {
        // The Extrude tool's profile faces (#268/#955). Always shown while the tool is active,
        // empty or not, and focused unless the "Up to" picker is armed — exactly one picker
        // wears the focus ring (#962).
        let target_armed = input.extrude.as_ref().is_some_and(|e| e.target_focused);
        let mut profile =
            ElementPicker::new(ElementFilter::kind(ElementKind::Profile), PickLimit::Infinite);
        profile.set_focused(!target_armed);
        profile.set_picked(
            input.doc,
            faces.iter().map(crate::extrude::extrude_face_scene_element),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Faces",
            picker: profile,
            target: PickerTarget::ExtrudeProfile,
            separator_above: true,
            render: PickerRender::Shared,
        });
    }
    if let Some(f) = input.sweep.as_ref() {
        // Sweep's two inputs (#955), the Revolve pair's twin: the profiles, and the path they
        // travel. Exactly one shows the focus ring.
        let mut profile =
            ElementPicker::new(ElementFilter::kind(ElementKind::Profile), PickLimit::Infinite);
        profile.set_focused(!f.path_focused && !(!f.faces.is_empty() && !f.path.is_empty()));
        profile.set_picked(
            input.doc,
            f.faces
                .iter()
                .map(crate::extrude::extrude_face_scene_element),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Profile",
            picker: profile,
            target: PickerTarget::SweepProfile,
            separator_above: true,
            render: PickerRender::Shared,
        });
        let mut path =
            ElementPicker::new(ElementFilter::kind(ElementKind::Line), PickLimit::Infinite);
        path.set_focused(f.path_focused);
        path.set_picked(input.doc, f.path.iter().map(|&li| SceneElement::Line(li)));
        tool_pickers.push(ToolPickerView {
            heading: "Path",
            picker: path,
            target: PickerTarget::SweepPath,
            separator_above: false,
            render: PickerRender::Shared,
        });
        if f.body_choice == crate::actions::RevolveBodyChoice::Cut {
            // As for Revolve: not until the profile and path are settled (#962).
            let ready = !f.faces.is_empty() && !f.path.is_empty();
            let mut cut = body_tool_picker(
                input.doc,
                "Cut bodies",
                PickerTarget::SweepCut,
                &f.cut_bodies,
                Some(crate::theme::CUT_ACCENT),
                ready,
            );
            cut.separator_above = false;
            tool_pickers.push(cut);
        }
    }
    if let Some(l) = input.loft_body.as_ref() {
        if l.body_choice == crate::actions::RevolveBodyChoice::Cut {
            // Not until there are enough sections to loft — the Sections picker wears the ring
            // until then, and two rings claim two places for the next click (#962).
            let ready = input.loft_sections.as_ref().is_some_and(|s| s.len() >= 2);
            let mut cut = body_tool_picker(
                input.doc,
                "Cut bodies",
                PickerTarget::LoftCut,
                &l.cut_bodies,
                Some(crate::theme::CUT_ACCENT),
                ready,
            );
            cut.separator_above = false;
            tool_pickers.push(cut);
        }
    }
    if let Some(m) = input.move_op.as_ref() {
        // Exactly one Move picker reads as focused (#658): the Bodies picker only while the
        // step-through hasn't moved on to a point/axis/alignment picker.
        //
        // It takes planes and tracing images alongside bodies (#217/#963) — a Move moves "the
        // things that move", and they were previously routed by a per-kind arm in the pane
        // click cascade with no picker showing them at all.
        let mut moving = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Body,
                ElementKind::Plane,
                ElementKind::Image,
            ])
            .rule(PickRule::LiveBody),
            PickLimit::Infinite,
        );
        moving.set_focused(m.bodies_focused);
        moving.set_picked(
            input.doc,
            m.targets
                .iter()
                .map(|&bi| SceneElement::Body(bi))
                .chain(m.plane_targets.iter().map(|&pi| {
                    SceneElement::ConstructionPlane(pi)
                }))
                .chain(m.image_targets.iter().map(|&ii| SceneElement::Image(ii))),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Bodies",
            picker: moving,
            target: PickerTarget::MoveTargets,
            separator_above: true,
            render: PickerRender::Shared,
        });
    }
    if let Some(m) = input.move_op.as_ref() {
        // The Move tool's six point pickers (#649/#650/#955). They draw between the Rotation
        // heading and the Angle-snap slider, so they're `Inline` — but they belong in this list
        // like every other picker (#958), or focus, hover and scripts can't see them.
        let moving = m.targets.clone();
        for (heading, target, point, on_moving, focused) in [
            ("Start point A", PickerTarget::MoveStartA, m.start_a, true, m.start_a_focused),
            ("End point A", PickerTarget::MoveEndA, m.end_a, false, m.end_a_focused),
            ("Start point B", PickerTarget::MoveStartB, m.start_b, true, m.start_b_focused),
            ("End point B", PickerTarget::MoveEndB, m.end_b, false, m.end_b_focused),
            ("Start point C", PickerTarget::MoveStartC, m.start_c, true, m.start_c_focused),
            ("End point C", PickerTarget::MoveEndC, m.end_c, false, m.end_c_focused),
        ] {
            let rule = if on_moving {
                PickRule::OnBodies(moving.clone())
            } else {
                PickRule::OffBodies(moving.clone())
            };
            let mut picker = ElementPicker::new(
                ElementFilter::kind(ElementKind::Vertex).rule(rule),
                PickLimit::Finite(1),
            );
            picker.set_focused(focused);
            picker.set_picked(input.doc, point.map(SceneElement::from_move_point));
            tool_pickers.push(ToolPickerView {
                heading,
                picker,
                target,
                separator_above: false,
                render: PickerRender::Inline,
            });
        }
    }
    if let Some(sl) = input.sketch_slice.as_ref() {
        // The in-sketch Slice tool's two sides (#238/#955): the entities being cut, and the
        // lines cutting them. Cutters are consumed by the operation, so they read red.
        let mut targets = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Line,
                ElementKind::Circle,
                ElementKind::Profile,
            ]),
            PickLimit::Infinite,
        );
        targets.set_focused(!sl.picking_cutter);
        targets.set_picked(input.doc, sl.targets.iter().cloned());
        tool_pickers.push(ToolPickerView {
            heading: "Targets",
            picker: targets,
            target: PickerTarget::SketchSliceTargets,
            separator_above: true,
            render: PickerRender::Shared,
        });
        let mut cutters =
            ElementPicker::new(ElementFilter::kind(ElementKind::Line), PickLimit::Infinite)
                .with_selected_color(crate::theme::CUT_ACCENT);
        cutters.set_focused(sl.picking_cutter);
        cutters.set_picked(input.doc, sl.cutters.iter().map(|&li| SceneElement::Line(li)));
        tool_pickers.push(ToolPickerView {
            heading: "Cutters",
            picker: cutters,
            target: PickerTarget::SketchSliceCutters,
            separator_above: false,
            render: PickerRender::Shared,
        });
    }
    if let Some(sl) = input.slice_op.as_ref() {
        // Slice's two pickers (#955): the bodies it splits, and the planes/flat faces doing the
        // splitting. Exactly one is focused — whichever the next viewport click feeds.
        tool_pickers.push(body_tool_picker(
            input.doc,
            "Targets",
            PickerTarget::SliceTargets,
            &sl.targets,
            None,
            !sl.picking_cutter,
        ));
        // The cutters are consumed by the operation, so they take the red override — the
        // example SPEC has always cited for it (#213/#961).
        let mut cutters = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Plane, ElementKind::Profile]),
            PickLimit::Infinite,
        )
        .with_selected_color(crate::theme::CUT_ACCENT);
        cutters.set_focused(sl.picking_cutter);
        cutters.set_picked(
            input.doc,
            sl.cutters.iter().cloned().map(SceneElement::from_face_id),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Cutters",
            picker: cutters,
            target: PickerTarget::SliceCutters,
            // No divider between the two pickers and the toggle below — one Slice block (#602).
            separator_above: false,
            render: PickerRender::Shared,
        });
    }
    if let Some(m) = input.mirror_op.as_ref() {
        // Primary picker: the mirror plane — a construction plane or a flat body face (#566).
        // Single-pick, and focused (the pick target) until a plane is chosen.
        let mut plane_picker = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Plane, ElementKind::Profile]),
            PickLimit::Finite(1),
        );
        plane_picker.set_focused(m.plane.is_none());
        if let Some(element) = m.plane.clone() {
            plane_picker.set_picked(input.doc, [element]);
        }
        tool_pickers.push(ToolPickerView {
            heading: "Mirror plane",
            picker: plane_picker,
            target: PickerTarget::MirrorPlane,
            separator_above: true,
            render: PickerRender::Shared,
        });
        // Secondary picker: the bodies picker reads as focused only once a mirror plane is
        // chosen — the plane is the first pick (#523). No divider between the plane picker,
        // the bodies picker, and the Do button — they read as one Mirror block (#602).
        let mut bodies = body_tool_picker(
            input.doc,
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
        let axis_is_next = r.path.is_none() && has_targets;
        // Repeat's "Distance to" (#645/#958): inline under the Distance field, registered here.
        let mut length_target = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Plane,
                ElementKind::Profile,
                ElementKind::Vertex,
            ]),
            PickLimit::Finite(1),
        );
        length_target.set_focused(r.length_target_focused && !r.around_axis);
        length_target.set_picked(input.doc, r.length_target.clone());
        // Like Move's, the Repeat set takes more than bodies (#220/#221/#231/#963): planes,
        // sketches and cut extrusions are all repeated alongside them, and each used to be a
        // per-kind arm in the pane click cascade with no picker showing it.
        let mut repeated = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Body,
                ElementKind::Plane,
                ElementKind::Sketch,
            ])
            .operations(&[crate::element_picker::OperationKind::Extrude])
            .rule(PickRule::LiveBody),
            PickLimit::Infinite,
        );
        repeated.set_focused(!axis_is_next && !r.value_field_focused && !r.length_target_focused);
        repeated.set_picked(
            input.doc,
            r.targets
                .iter()
                .map(|&bi| SceneElement::Body(bi))
                .chain(r.plane_targets.iter().map(|&pi| {
                    SceneElement::ConstructionPlane(pi)
                }))
                .chain(r.sketch_targets.iter().map(|&si| SceneElement::Sketch(si)))
                .chain(r.extrusion_targets.iter().map(|&ei| {
                    SceneElement::Extrusion(ei)
                })),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Bodies",
            picker: repeated,
            target: PickerTarget::RepeatTargets,
            separator_above: true,
            render: PickerRender::Shared,
        });
        // A straight reference to travel along, or a **circle** to ride round (#840). Whether
        // the copies follow the path or turn about it is the Repeat toggle right below, so the
        // row names the path itself rather than repeating "Along"/"Around" (#955).
        let mut path = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Line,
                ElementKind::Edge,
                ElementKind::Axis,
                ElementKind::Circle,
            ]),
            PickLimit::Finite(1),
        );
        path.set_focused(axis_is_next && !r.value_field_focused);
        path.set_picked(input.doc, r.path.clone());
        tool_pickers.push(ToolPickerView {
            heading: "Path",
            picker: path,
            target: PickerTarget::RepeatPath,
            separator_above: false,
            render: PickerRender::Inline,
        });
        tool_pickers.push(ToolPickerView {
            heading: "Distance to",
            picker: length_target,
            target: PickerTarget::RepeatDistanceTo,
            separator_above: false,
            render: PickerRender::Inline,
        });
    }
    if let Some(r) = input.sketch_repeat.as_ref() {
        // The in-sketch Repeat tool (#232/#835): the entities being copied, and the line whose
        // direction they march along — empty meaning the sketch's own U axis.
        let mut entities = ElementPicker::new(
            in_sketch(
                ElementFilter::kinds(&[ElementKind::Line, ElementKind::Circle]),
                input.open_sketch,
            ),
            PickLimit::Infinite,
        );
        entities.set_focused(!r.direction_focused && !r.value_field_focused);
        entities.set_picked(input.doc, r.picked.iter().cloned());
        tool_pickers.push(ToolPickerView {
            heading: "Entities",
            picker: entities,
            target: PickerTarget::SketchRepeatEntities,
            separator_above: true,
            render: PickerRender::Inline,
        });
        let mut direction = ElementPicker::new(
            in_sketch(ElementFilter::kinds(&[ElementKind::Line]), input.open_sketch),
            PickLimit::Finite(1),
        );
        direction.set_focused(r.direction_focused);
        direction.set_picked(input.doc, r.direction.clone());
        tool_pickers.push(ToolPickerView {
            heading: "Direction",
            picker: direction,
            target: PickerTarget::SketchRepeatDirection,
            separator_above: false,
            render: PickerRender::Inline,
        });
    }
    if let Some(o) = input.sketch_offset.as_ref() {
        // The in-sketch Offset tool's entity set (#493) — the tool's only pick, so always armed.
        let mut entities = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Line, ElementKind::Circle]),
            PickLimit::Infinite,
        );
        entities.set_focused(true);
        entities.set_picked(input.doc, o.picked.iter().cloned());
        tool_pickers.push(ToolPickerView {
            heading: "Entities",
            picker: entities,
            target: PickerTarget::SketchOffsetEntities,
            separator_above: true,
            render: PickerRender::Inline,
        });
    }
    if let Some(m) = input.sketch_mirror.as_ref() {
        // The in-sketch Mirror tool (#534): the mirror line comes first, then the shapes — so
        // exactly one of the two is armed, whichever the next click should feed.
        let mut line = ElementPicker::new(
            in_sketch(ElementFilter::kinds(&[ElementKind::Line]), input.open_sketch),
            PickLimit::Finite(1),
        );
        line.set_focused(m.line.is_none());
        line.set_picked(input.doc, m.line.map(SceneElement::Line));
        tool_pickers.push(ToolPickerView {
            heading: "Mirror line",
            picker: line,
            target: PickerTarget::SketchMirrorLine,
            separator_above: true,
            render: PickerRender::Inline,
        });
        let mut shapes = ElementPicker::new(
            in_sketch(
                ElementFilter::kinds(&[ElementKind::Line, ElementKind::Circle]),
                input.open_sketch,
            ),
            PickLimit::Infinite,
        );
        shapes.set_focused(m.line.is_some());
        shapes.set_picked(input.doc, m.picked.iter().cloned());
        tool_pickers.push(ToolPickerView {
            heading: "Shapes",
            picker: shapes,
            target: PickerTarget::SketchMirrorShapes,
            separator_above: false,
            render: PickerRender::Inline,
        });
    }
    if input.in_drawing_workbench && input.tool == Tool::Select {
        let rows = &input.drawing_selection;
        // The drawing workbench's Select tool (#346/#967): its page items are ordinary
        // elements now, so its input is an ordinary picker — one that takes the three drawing
        // kinds, each keeping the icon the Elements pane gives it (#363).
        let mut picker = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Projection,
                ElementKind::Annotation,
                ElementKind::Dimension,
            ]),
            PickLimit::Infinite,
        );
        picker.set_focused(true);
        picker.set_picked(
            input.doc,
            rows.iter().map(|(drawing, element, _)| SceneElement::DrawingElement {
                drawing: *drawing,
                element: *element,
            }),
        );
        tool_pickers.push(ToolPickerView {
            heading: "Selection",
            picker,
            target: PickerTarget::DrawingSelection,
            separator_above: true,
            render: PickerRender::Inline,
        });
    }
    if input.drawing_align_active {
        let base = &input.drawing_align_base;
        // The Aligned-view tool's base view (#365/#967): a single projection, which the
        // `Projection` kind expresses directly — it used to be an `Option<Option<..>>` standing
        // in for a `Finite(1)` picker.
        let mut picker = ElementPicker::new(
            ElementFilter::kind(ElementKind::Projection),
            PickLimit::Finite(1),
        );
        picker.set_focused(true);
        if let (Some((view, _)), Some(drawing)) = (base.as_ref(), input.open_drawing) {
            picker.set_picked(
                input.doc,
                [SceneElement::DrawingElement {
                    drawing,
                    element: DrawingElementRef::Projection(*view),
                }],
            );
        }
        tool_pickers.push(ToolPickerView {
            heading: "Base view",
            picker,
            target: PickerTarget::DrawingAlignBase,
            separator_above: true,
            render: PickerRender::Inline,
        });
    }
    if let Some(p) = input.plane_tool.as_ref() {
        // The plane's anchor (#474/#483/#955). One row for a face, straight edge, axis or
        // vertex; two when a line and a point together fix the frame — hence a limit of two
        // rather than one. Focused whenever the tool is up: the anchor is the only thing this
        // tool picks, so every viewport click either sets it or complements it.
        let mut anchor = ElementPicker::new(
            ElementFilter::kinds(&[
                ElementKind::Face,
                ElementKind::Profile,
                ElementKind::Plane,
                ElementKind::Line,
                ElementKind::Edge,
                ElementKind::Axis,
                ElementKind::Circle,
                ElementKind::Vertex,
            ]),
            PickLimit::Finite(2),
        );
        anchor.set_focused(true);
        anchor.set_picked(input.doc, p.anchor_elements.iter().cloned());
        tool_pickers.push(ToolPickerView {
            heading: "Anchor",
            picker: anchor,
            target: PickerTarget::PlaneAnchor,
            separator_above: true,
            render: PickerRender::Inline,
        });
    }
    if let Some(b) = input.boolean_op.as_ref() {
        // Combine mode uses one picker (side A only); Cut/Intersect/Difference use two sides.
        // The focused side is the one the next viewport click lands on, toggled by clicking a
        // picker (its Focus event). Side B (the tool that gets consumed in Cut) is styled red.
        let two_sided = b.kind != crate::model::BooleanOpKind::Combine;
        tool_pickers.push(body_tool_picker(
            input.doc,
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
                input.doc,
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
    tool_pickers
}
pub fn context_pane_content(input: &ContextInput<'_>) -> ContextPaneContent {
    let tool_title = tool_context_title(input);
    let name = single_nameable_from_selection(input.selection).map(|element| NameControl { element });
    // The selected unit instance's own section (#734).
    let unit_instance = match input.selection.single() {
        Some(SceneElement::UnitInstance(instance)) => {
            unit_instance_control(input.doc, instance)
        }
        _ => None,
    };
    // Snapping shows for the drawing tools in 3D as well as in a sketch (#636): the
    // Rectangle/Line/Circle sections read identically either way, and the toggle is sticky,
    // so setting it in 3D carries into the sketch the first click opens. The Select tool
    // keeps its sketch-only toggle — there's nothing to snap while picking in 3D.
    let snapping = (tool_uses_snapping(input.tool)
        && (input.in_sketch || is_draw_tool(input.tool) || input.tool == Tool::Shape))
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
            can_join_profiles: input.extrude_disjoint_profiles,
            symmetric: input.extrude_symmetric.unwrap_or(false),
        }
    });
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
            // The Joint tool has a busy section of its own (#998), and a joint's units are
            // whatever its parts' already are — nothing about a joint is the document's default.
            | Tool::Joint
    );
    let units = (!units_suppressed)
        .then(|| units_control_from_selection(input.doc, input.selection))
        .flatten();
    // Material picker (#834): shown whenever the selection is bodies, so what a body is made
    // of sits right where its name does.
    // Materials belong to the Select tool's pane (#934): while a tool is running, the pane
    // is that tool's controls, and a body's material isn't one of them.
    let material = (input.tool == Tool::Select)
        .then(|| material_control_from_selection(input.doc, input.selection))
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

    let calibrate_image = input.calibrate_image;
    let revolve = input.revolve.clone();
    let sweep = input.sweep.clone();
    let plane_tool = input.plane_tool.clone();
    let loft_body = input.loft_body.clone();
    let boolean_op = input.boolean_op.clone();
    let boolean_edit_start = input.boolean_edit_start;
    let move_op = input.move_op.clone();
    let move_edit_start = input.move_edit_start;
    let joint = input.joint.clone();
    let joint_edit_start = input.joint_edit_start;
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
    let shape = input.shape.clone();
    let slice_op = input.slice_op.clone();
    let slice_edit_start = input.slice_edit_start;
    let revolve_edit_start = input.revolve_edit_start;
    let sweep_edit_start = input.sweep_edit_start;
    let calibrate_start = input.calibrate_start;
    let calibrate_pending = input.calibrate_pending;
    // Built before the draw tools' early returns below: outside a sketch their first click
    // picks the face to sketch on, which is a pick like any other and shows a picker (#958).
    let tool_pickers = tool_picker_views(input);
    // The unified selection element picker (#213), mirroring the live selection for the tools
    // that operate on it — registered with the rest (#958) and drawn at the top of the pane.
    let selection_picker = tool_pickers
        .iter()
        .find(|v| v.target == PickerTarget::Selection)
        .map(|v| v.picker.clone());

    if let Some(construction) = input.draw_rect_construction {
        return ContextPaneContent {
            tool_title,
            name,
            unit_instance: None,
            curve_mode: None,
            rect_anchor: input.rect_anchor,
            circle_anchor: input.circle_anchor,
            tangent_constraint: None,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            constraint_axis_dirs: None,
            snapping,
            extrude_body,
            extrude: extrude.clone(),
            units,
            material: material.clone(),
            selection_picker: selection_picker.clone(),
            dimension_derive: None,
            dimension_edit: None,
            treatment: None,
            tool_pickers: tool_pickers.clone(),
            calibrate_image,
            revolve: revolve.clone(),
            sweep: sweep.clone(),
            plane_tool: plane_tool.clone(),
            loft_body: loft_body.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            shape: shape.clone(),
            joint: joint.clone(),
            joint_edit_start,
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
            unit_instance: None,
            curve_mode: input.draw_line_curve_mode,
            rect_anchor: input.rect_anchor,
            circle_anchor: input.circle_anchor,
            tangent_constraint: input.draw_line_tangent_constraint,
            construction: Some(ConstructionControl {
                value: tri_state_from_bool(construction),
                target_count: 1,
            }),
            constraints: None,
            constraint_axis_dirs: None,
            snapping,
            extrude_body,
            extrude: extrude.clone(),
            units,
            material: material.clone(),
            selection_picker: selection_picker.clone(),
            dimension_derive: None,
            dimension_edit: None,
            treatment: None,
            tool_pickers: tool_pickers.clone(),
            calibrate_image,
            revolve: revolve.clone(),
            sweep: sweep.clone(),
            plane_tool: plane_tool.clone(),
            loft_body: loft_body.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            shape: shape.clone(),
            joint: joint.clone(),
            joint_edit_start,
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
            unit_instance: None,
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
            constraint_axis_dirs: None,
            snapping,
            extrude_body,
            extrude: extrude.clone(),
            units,
            material: material.clone(),
            selection_picker: selection_picker.clone(),
            dimension_derive: None,
            dimension_edit: None,
            treatment: None,
            tool_pickers: tool_pickers.clone(),
            calibrate_image,
            revolve: revolve.clone(),
            sweep: sweep.clone(),
            plane_tool: plane_tool.clone(),
            loft_body: loft_body.clone(),
            boolean_op: boolean_op.clone(),
            boolean_edit_start,
            move_op: move_op.clone(),
            move_edit_start,
            shape: shape.clone(),
            joint: joint.clone(),
            joint_edit_start,
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
        unit_instance,
        curve_mode: None,
        rect_anchor: input.rect_anchor,
        circle_anchor: input.circle_anchor,
        tangent_constraint: None,
        construction: (!targets.is_empty()).then(|| ConstructionControl {
            value: construction_tri_state(input.doc, &targets),
            target_count: targets.len(),
        }),
        constraints,
        constraint_axis_dirs: input.sketch_axis_screen_dirs,
        snapping,
        extrude_body,
        extrude: extrude.clone(),
        units,
        material,
        selection_picker,
        dimension_derive,
        dimension_edit: input.dimension_edit.clone(),
        treatment: input.treatment.clone(),
        tool_pickers,
        calibrate_image,
        revolve,
        sweep,
        plane_tool,
        loft_body,
        boolean_op,
        boolean_edit_start,
        move_op,
        shape,
        joint,
        joint_edit_start,
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
/// The material picker for the selected bodies (#834): `None` unless every selected element
/// is a live body.
fn material_control_from_selection(
    doc: &Document,
    selection: &SceneSelection,
) -> Option<MaterialControl> {
    let mut bodies = Vec::new();
    for element in selection.iter() {
        match element {
            SceneElement::Body(bi) if doc.bodies.get(bi).is_some_and(|b| !b.deleted) => {
                bodies.push(bi)
            }
            _ => return None,
        }
    }
    if bodies.is_empty() {
        return None;
    }
    // A body with no material of its own is made of the document's first one (#924), so
    // the picker shows that material selected — swatch, name and colour included — rather
    // than a "Default" entry standing in for it.
    let material_of = |bi: &usize| {
        doc.bodies[*bi]
            .material
            .filter(|mi| doc.materials.get(*mi).is_some_and(|m| !m.deleted))
            .or_else(|| {
                doc.materials
                    .get(crate::model::DEFAULT_MATERIAL)
                    .filter(|m| !m.deleted)
                    .map(|_| crate::model::DEFAULT_MATERIAL)
            })
    };
    let first = material_of(&bodies[0]);
    let agreed = bodies.iter().all(|bi| material_of(bi) == first);
    Some(MaterialControl {
        materials: doc
            .materials
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.deleted)
            .map(|(i, m)| (i, m.name.clone(), m.color))
            .collect(),
        current: agreed.then_some(first),
        bodies,
    })
}

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

// --- Help mode (#672) --------------------------------------------------------------------
//
// With help mode on, every row of the pane grows a floating note beside it saying what that
// control wants. The notes are collected as the pane lays itself out — each row helper
// records its own rect — and drawn in one pass afterwards, so they can be spaced apart
// without overlapping each other.
//
// The collector rides in egui's per-frame data rather than being threaded through
// `show_pane`'s (already enormous) parameter list and every row call site.

#[derive(Clone, Default)]
struct HelpNotes {
    tool: Option<Tool>,
    entries: Vec<(egui::Rect, &'static str)>,
}

fn help_notes_id() -> egui::Id {
    egui::Id::new("context_help_notes")
}

/// Start collecting help notes for this frame's pane. Call before laying the pane out; a
/// missing collector is what tells the row helpers help mode is off.
pub fn begin_help_notes(ctx: &egui::Context, tool: Option<Tool>) {
    ctx.data_mut(|d| d.insert_temp(help_notes_id(), HelpNotes { tool, entries: Vec::new() }));
}

/// Stop collecting (help mode off, or the pane is done).
pub fn end_help_notes(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<HelpNotes>(help_notes_id()));
}

/// [`note_help`] for rows outside this module (#727): the Parameters pane's controls
/// participate in help mode with an explicit label + rect.
pub(crate) fn note_help_rect(ui: &egui::Ui, label: &str, rect: egui::Rect) {
    note_help(ui, label, rect);
}

/// Record a row's rect against its help text, if help mode is on and this row has any.
fn note_help(ui: &egui::Ui, label: &str, rect: egui::Rect) {
    let ctx = ui.ctx();
    let Some(mut notes) = ctx.data(|d| d.get_temp::<HelpNotes>(help_notes_id())) else {
        return;
    };
    let Some(text) = row_help(notes.tool, label) else {
        return;
    };
    notes.entries.push((rect, text));
    ctx.data_mut(|d| d.insert_temp(help_notes_id(), notes));
}

/// Draw the notes collected this frame beside `pane_rect`, returning the rectangle they
/// cover (so a scripted pane capture can widen to include them).
///
/// Notes sit to the pane's left — the pane lives against the window's right edge — each
/// aimed at its own row by a leader line. Where two would overlap, the lower one slides
/// down, so a dense pane fans its notes out rather than stacking them on top of each other.
pub fn draw_help_notes(ctx: &egui::Context, pane_rect: egui::Rect) -> Option<egui::Rect> {
    let notes = ctx.data(|d| d.get_temp::<HelpNotes>(help_notes_id()))?;
    if notes.entries.is_empty() {
        return None;
    }

    const WIDTH: f32 = 230.0;
    const GAP: f32 = 14.0; // between a note and the pane
    const SPACING: f32 = 6.0; // between stacked notes
    let right = pane_rect.left() - GAP;
    let left = right - WIDTH;

    // Lay every note's text out first: the placement pass needs all the heights.
    let galleys: Vec<_> = notes
        .entries
        .iter()
        .map(|(_, text)| {
            ctx.fonts_mut(|fonts| {
                fonts.layout(
                    text.to_string(),
                    egui::FontId::proportional(11.5),
                    egui::Color32::from_gray(225),
                    WIDTH - 16.0,
                )
            })
        })
        .collect();

    // Each note wants to sit level with its row; where that would overlap the one above, it
    // slides down. That alone walks the whole column downwards, so the finished stack is then
    // lifted back to sit within the window — and centred on its rows if it is taller than they
    // are.
    let mut tops = Vec::with_capacity(galleys.len());
    let mut lowest = f32::NEG_INFINITY;
    for ((row, _), galley) in notes.entries.iter().zip(&galleys) {
        let height = galley.size().y + 12.0;
        let top = (row.center().y - height / 2.0).max(lowest + SPACING);
        lowest = top + height;
        tops.push(top);
    }
    let screen = ctx.content_rect();
    let overflow = lowest - (screen.bottom() - 8.0);
    if overflow > 0.0 {
        let headroom = tops[0] - (screen.top() + 8.0);
        let lift = overflow.min(headroom.max(0.0));
        for top in &mut tops {
            *top -= lift;
        }
    }

    let mut bounds: Option<egui::Rect> = None;
    for (i, ((row, _), galley)) in notes.entries.iter().zip(galleys).enumerate() {
        let height = galley.size().y + 12.0;
        let note =
            egui::Rect::from_min_size(egui::pos2(left, tops[i]), egui::vec2(WIDTH, height));

        egui::Area::new(egui::Id::new(("context_help_note", i)))
            .order(egui::Order::Foreground)
            .fixed_pos(note.min)
            // No fade-in: a note appearing mid-animation would make captures
            // non-deterministic (SPEC §9.3).
            .fade_in(false)
            .interactable(false)
            .show(ctx, |ui| {
                let painter = ui.painter();
                painter.rect_filled(note, 4.0, egui::Color32::from_black_alpha(230));
                painter.rect_stroke(
                    note,
                    4.0,
                    egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
                    egui::StrokeKind::Inside,
                );
                painter.galley(note.min + egui::vec2(8.0, 6.0), galley, egui::Color32::WHITE);
                // A leader from the note to the row it explains.
                painter.line_segment(
                    [
                        egui::pos2(note.right(), note.center().y),
                        egui::pos2(row.left(), row.center().y),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(110)),
                );
                ui.allocate_space(note.size());
            });

        bounds = Some(bounds.map_or(note, |b: egui::Rect| b.union(note)));
    }
    bounds
}

/// What a pane row means, keyed by the tool it belongs to and the row's label.
///
/// Rows that mean the same thing under every tool (the document's default units, an
/// element's name) are matched on the label alone, after the per-tool lookup misses.
fn row_help(tool: Option<Tool>, label: &str) -> Option<&'static str> {
    let per_tool = match (tool, label) {
        (Some(Tool::Move), "Bodies") => {
            Some("The bodies that will move. Click one to add it, click it again to drop it.")
        }
        (Some(Tool::Move), "Translate") => Some(
            "How you say where the bodies go: Snap lands a point on a point, Free takes X/Y/Z \
             amounts. M switches between them.",
        ),
        (Some(Tool::Move), "Start point A") => Some(
            "The corner or edge midpoint on a moving body that you are aiming with. In Free \
             mode it is where the drag arrows sit.",
        ),
        (Some(Tool::Move), "End point A") => Some(
            "Where start point A lands — a corner or edge midpoint on something that isn't \
             moving.",
        ),
        (Some(Tool::Move), "Angle snap") => Some(
            "How far apart the rotation's candidate dots sit, in degrees. 90° gives the six \
             axis directions; smaller values give more to choose from.",
        ),
        (Some(Tool::Move), "Start point B") => Some(
            "Optional. A second point on a moving body, to turn the bodies as well as slide \
             them.",
        ),
        (Some(Tool::Move), "End point B") => Some(
            "Optional. Where start point B swings to, about end point A. Only the spots it can \
             actually reach are offered.",
        ),
        (Some(Tool::Move), "X") => Some(
            "How far along X, as an expression — 25, gap * 2, 10mm — so the move stays \
             parametric.",
        ),
        (Some(Tool::Move), "Y") => Some("How far along Y."),
        (Some(Tool::Move), "Z") => Some("How far along Z."),

        (Some(Tool::Shape), "Shape") => Some(
            "Which shape to place: cuboid, cylinder, or sphere. B cycles them.",
        ),
        (Some(Tool::Shape), "Width") => Some("The cuboid's size across the plane's first direction."),
        (Some(Tool::Shape), "Depth") => Some("The cuboid's size across the other direction."),
        (Some(Tool::Shape), "Height") => Some("How far the shape rises off the plane it sits on."),
        (Some(Tool::Shape), "Radius") => Some("The cylinder's or sphere's radius."),
        (Some(Tool::Joint), "Parts") => Some(
            "The two parts to join — bodies, components, or imported units. Click one to \
             add it, click it again to drop it.",
        ),
        (Some(Tool::Joint), "Type") => Some(
            "How the parts may move relative to each other: rigid, slider, revolute, \
             cylindrical, planar, ball, pin-slot, or screw.",
        ),
        (Some(Tool::Joint), "Lead") => Some(
            "How far the screw travels per full turn, as an expression.",
        ),
        (Some(Tool::Joint), "Base") => Some(
            "The side that stays put; the other side moves through the joint. Click to swap.",
        ),
        (Some(Tool::Joint), "Start point A") => Some(
            "The mating origin on the driven part. Leave the pairs empty to join the parts \
             right where they are.",
        ),
        (Some(Tool::Joint), "End point A") => Some(
            "Where start point A mates on the base part — the joint's origin.",
        ),
        (Some(Tool::Joint), "Start point B") => Some(
            "Optional. Aims the driven side's axis: it runs from start point A toward here.",
        ),
        (Some(Tool::Joint), "End point B") => Some(
            "Optional. Aims the base side's axis — the slide direction or the turn axis.",
        ),
        (Some(Tool::Joint), "Start point C") => Some(
            "Optional. Pins the driven side's spin about its axis.",
        ),
        (Some(Tool::Joint), "End point C") => Some(
            "Optional. Pins the base side's spin about its axis.",
        ),
        (Some(Tool::Joint), "Slide") => Some(
            "How far along the axis, as an expression, so the pose stays parametric.",
        ),
        (Some(Tool::Joint), "Slide min") => Some(
            "How far back the slide may travel, as an expression. Empty leaves it open.",
        ),
        (Some(Tool::Joint), "Slide max") => Some(
            "How far forward the slide may travel. Empty leaves it open.",
        ),
        (Some(Tool::Joint), "Turn min") => Some(
            "How far the joint may turn one way, in degrees. Empty leaves it open.",
        ),
        (Some(Tool::Joint), "Turn max") => Some(
            "How far the joint may turn the other way. Empty leaves it open.",
        ),
        (Some(Tool::Joint), "Animate") => Some(
            "Whether the preview sweeps through the joint's range. Applies to every joint.",
        ),
        (Some(Tool::Joint), "Rest") => Some(
            "The pose the assembly is meant to sit in. Set captures the current position; \
             Revert goes back to it.",
        ),
        (Some(Tool::Joint), "Min stop") => Some(
            "A face or plane the slide stops at, instead of a number — the limit follows \
             the model.",
        ),
        (Some(Tool::Joint), "Max stop") => Some(
            "A face or plane the slide stops at going forward.",
        ),
        (Some(Tool::Joint), "Angle") => Some("How far around the axis, in degrees."),
        (Some(Tool::Joint), "U") => Some("How far across the plane's first direction."),
        (Some(Tool::Joint), "V") => Some("How far across the plane's second direction."),
        (Some(Tool::Joint), "Spin") => Some("The turn about the plane's normal."),
        (Some(Tool::Joint), "Yaw") => Some("The turn about the frame's first axis."),
        (Some(Tool::Joint), "Pitch") => Some("The turn about the frame's second axis."),
        (Some(Tool::Joint), "Roll") => Some("The turn about the frame's third axis."),

        (Some(Tool::Extrude), "Faces") => Some(
            "The sketch or solid faces being pulled. Click a face to add it, click it again to \
             drop it.",
        ),
        (Some(Tool::Extrude), "Distance") => Some(
            "How deep, as an expression. Mirrors the drag handle in the 3D view — moving \
             either updates the other.",
        ),
        (Some(Tool::Extrude), "Up to") => Some(
            "A plane, face, or vertex to stop at instead of a fixed depth; the extrusion then \
             follows it if it moves. Setting one clears Distance.",
        ),
        (Some(Tool::Extrude), "Output") => Some(
            "Whether this becomes a new body, fuses into the body it grows from, or cuts into \
             it. Profiles that don't touch make a body each; Join puts them in one.",
        ),
        (Some(Tool::Extrude), "Symmetric") => {
            Some("Grows the same depth either side of the sketch plane instead of one way.")
        }

        (Some(Tool::Revolve), "Profile") => Some(
            "The sketch faces to sweep around the axis. Click a face to add it, click it again \
             to drop it.",
        ),
        (Some(Tool::Revolve), "Axis") => Some(
            "The line the profile turns about — a straight sketch line or one of the global \
             axes. The angle is dragged in the 3D view.",
        ),
        (Some(Tool::Revolve), "Symmetric") => {
            Some("Sweeps the same angle either side of the profile instead of one way.")
        }
        (Some(Tool::Revolve), "Output") => Some(
            "Whether this becomes a new body, joins the body it touches, or cuts into it.",
        ),

        (_, "Calibrate scale") => Some(
            "Sets the image's real-world size: click two points over a feature of known \
             size, then type its length.",
        ),
        (_, "Real length") => Some(
            "How long the marked span really is. Apply rescales the whole image so \
             that span measures this.",
        ),
        (Some(Tool::Text), "Text") => Some(
            "What the text says. {curly braces} interpolate an expression — {w * 2} — \
             and re-render when parameters change.",
        ),
        (Some(Tool::Text), "Font") => Some(
            "The typeface, from the fonts on this machine. It embeds in the file, so \
             the text renders the same anywhere.",
        ),
        (Some(Tool::Text), "Size") => Some(
            "Letter height as an expression — parametric text sizes work.",
        ),
        (Some(Tool::Text), "Rotation°") => Some(
            "The text's turn about its anchor, in degrees — the round handle in the \
             viewport drags it too.",
        ),
        (Some(Tool::Text), "Wrap width") => Some(
            "Where lines break. Empty grows one line; drag the side handle in the \
             viewport to set it.",
        ),
        (Some(Tool::ConstructionPlane), "Anchor") => Some(
            "What the plane hangs on: a face, an edge, a vertex, or a line plus a \
             point. Click it in the viewport; the clear button starts the pick over.",
        ),
        (Some(Tool::ConstructionPlane), "Normal") => Some(
            "Which way the plane faces when the anchor leaves more than one choice — \
             pick among the directions at that corner.",
        ),
        (Some(Tool::ConstructionPlane), "Offset") => Some(
            "How far off the anchor the plane sits, as an expression. Mirrors the drag \
             handle in the 3D view.",
        ),
        (Some(Tool::ConstructionPlane), "Tilt") => Some(
            "The tilt about the anchored axis, in degrees — for an edge or axis anchor.",
        ),
        (Some(Tool::DrawingAdd), "Projection") => Some(
            "Click a body or sketch — in the Elements pane or the 3D view — and a \
             projection of it lands on the page.",
        ),
        (Some(Tool::DrawingAlign), "Base view") => Some(
            "The view the new projection aligns to: click a view card, then a side of \
             it, and the aligned view lands there.",
        ),
        (_, "Shows") => Some("The body or sketch this view projects, and from which side."),
        (_, "Style") => Some("How the projection draws — hidden lines shown, hidden, or dashed."),
        (_, "Scale") => Some(
            "The view's drawing scale. Views print at this ratio; the label can show it.",
        ),
        (_, "Label") => Some("The caption under the view — shown or hidden, with its text."),
        (_, "Dimensions") => Some(
            "Measurements shown on this view. Click an edge on the page to add or \
             remove its dimension.",
        ),
        (_, "Text") => Some("The caption's wording."),
        (_, "Position") => Some(
            "Where it sits on the page, as fractions of the sheet — drag it there too.",
        ),
        (Some(Tool::Select), "Selection") => Some(
            "Everything currently selected, one row each — click things in the viewport \
             or the Elements pane; a row's clear drops it.",
        ),
        (Some(Tool::Constraint), "Selection") => Some(
            "The geometry the constraint applies to — click lines, points, or edges in \
             the viewport; Shift+click adds more.",
        ),
        (Some(Tool::Constraint), "Constraints") => Some(
            "The relationships the current selection can take — hover a button for its \
             name, and a greyed one says what it still needs. Clicking applies it.",
        ),
        (Some(Tool::Dimension), "Selection") => Some(
            "What to measure: one edge for its length, two for the distance or angle \
             between them, two corners for their distance.",
        ),
        (Some(Tool::Dimension), "Parameter name") => Some(
            "The name the measurement is saved under — a read-only parameter that \
             follows the geometry.",
        ),
        (Some(Tool::Dimension), "Value") => Some(
            "What the current selection measures, live.",
        ),
        (Some(Tool::Line), "Curve") => Some(
            "Draws a curved (bezier) line instead of a straight one — Cmd/Ctrl+B \
             switches mid-draw too.",
        ),
        (Some(Tool::Line), "Tangent") => Some(
            "Starts the next line tangent to the one it leaves from, and keeps them \
             tangent afterwards.",
        ),
        (Some(Tool::Rectangle), "Anchor") => Some(
            "Whether the rectangle grows from a corner or from its center. Pressing R \
             again flips it.",
        ),
        (Some(Tool::Circle), "Anchor") => Some(
            "Whether the circle grows from its center or from a point on its edge. \
             Pressing O again flips it.",
        ),
        (Some(Tool::Project), "Selection") => Some(
            "The outside geometry to pull onto this sketch plane — click a body edge to \
             project it, a face or corner to take the whole body's edges, or a plane for \
             its crossing line. Clicking a projected line removes it.",
        ),
        (Some(Tool::Sketch), "Selection") => Some(
            "The face the new sketch opens on — a construction plane, a flat body face, \
             or a unit's face. Click it in the viewport.",
        ),
        (Some(Tool::Offset), "Entities") => Some(
            "The lines and circles to copy at a distance. Click one to add it, click it \
             again to drop it.",
        ),
        (Some(Tool::Offset), "Distance") => Some(
            "How far the copies sit from their sources, as an expression. Negative \
             flips the side.",
        ),
        (Some(Tool::Offset), "Construction") => Some(
            "Whether the copies land as construction geometry — guides that don't \
             become part of a profile.",
        ),
        (Some(Tool::Slice), "Bodies") => Some(
            "The bodies to cut apart. Click one to add it, click it again to drop it.",
        ),
        (Some(Tool::Slice), "Cutters") => Some(
            "What does the cutting — construction planes or flat body faces; in a \
             sketch, the lines that split the shapes.",
        ),
        (Some(Tool::Slice), "Infinite cut") => Some(
            "Whether each cutter extends without bound, or only cuts as far as the \
             face itself reaches.",
        ),
        (Some(Tool::Slice), "Targets") => Some(
            "The sketch lines and circles to split. Click one to add it, click it \
             again to drop it.",
        ),
        (Some(Tool::Repeat), "Bodies") => Some(
            "The bodies to copy along the axis. Click one to add it, click it again to \
             drop it.",
        ),
        (Some(Tool::Repeat), "Planes") => Some(
            "How many construction planes ride along as offset copies.",
        ),
        (Some(Tool::Repeat), "Sketches") => Some(
            "How many sketches ride along as offset copies.",
        ),
        (Some(Tool::Repeat), "Cuts") => Some(
            "How many cut operations are replayed at each step — a row of holes from one.",
        ),
        (Some(Tool::Repeat), "Path") => Some(
            "What the pattern follows — a straight edge, a sketch line, or a global axis.",
        ),
        (Some(Tool::Repeat), "Repeat") => Some(
            "Lay the copies out along the path, or turn them around it as an axis.",
        ),
        (Some(Tool::Repeat), "Angle") => Some(
            "How far around the axis the pattern sweeps. The green lock marks the value \
             computed from the other two.",
        ),
        (Some(Tool::Repeat), "Count") => Some(
            "How many copies. The green lock marks the value computed from the other two.",
        ),
        (Some(Tool::Repeat), "Gap") => Some(
            "Clear space between one copy's end and the next one's start. The icon \
             switches it to Offset (start-to-start).",
        ),
        (Some(Tool::Repeat), "Offset") => Some(
            "Start-to-start spacing between copies. The icon switches it back to Gap.",
        ),
        (Some(Tool::Repeat), "Distance") => Some(
            "How far the whole pattern runs. The icon toggles whether the last copy \
             starts or ends there.",
        ),
        (Some(Tool::Repeat), "Distance to") => Some(
            "A face, plane, or corner the pattern runs out to instead of a typed \
             distance — the copies follow it if it moves.",
        ),
        (Some(Tool::Repeat), "Entities") => Some(
            "The sketch lines and circles to copy. Click one to add it, click it again \
             to drop it.",
        ),
        (Some(Tool::Repeat), "Direction") => Some(
            "The direction the copies run — a sketch line, or the sketch's U axis while \
             this is empty.",
        ),
        (Some(Tool::Mirror), "Mirror plane") => Some(
            "The plane the reflection flips across — a construction plane or a flat \
             body face.",
        ),
        (Some(Tool::Mirror), "Bodies") => Some(
            "The bodies to reflect. Click one to add it, click it again to drop it.",
        ),
        (Some(Tool::Mirror), "Output") => Some(
            "Whether each reflection is its own body, fuses with its source, or cuts \
             into it.",
        ),
        (Some(Tool::Mirror), "Mirror line") => Some(
            "The straight sketch line the copies flip across.",
        ),
        (Some(Tool::Mirror), "Shapes") => Some(
            "The lines and circles to reflect. Click one to add it, click it again to \
             drop it.",
        ),
        (Some(Tool::Loft), "Sections") => Some(
            "The closed profiles the loft blends through, in order — one per level. \
             Click a circle or closed loop to add it, click it again to drop it.",
        ),
        (Some(Tool::Loft), "Output") => Some(
            "Whether this becomes a new body, joins the body it touches, or cuts into it.",
        ),
        (Some(Tool::Sweep), "Profile") => Some(
            "The closed sketch faces to push along the path. Click a face to add it, \
             click it again to drop it.",
        ),
        (Some(Tool::Sweep), "Path") => Some(
            "The line(s) the profile travels along, chained end to end — straight or \
             curved. Click lines in the viewport in order.",
        ),
        (Some(Tool::Sweep), "Output") => Some(
            "Whether this becomes a new body, joins the body it touches, or cuts into it.",
        ),
        (Some(Tool::Chamfer), "Selection") => Some(
            "The sketch corners to cut flat. Click a corner where two lines meet; the cut \
             distance is typed in the 3D view.",
        ),
        (Some(Tool::Chamfer), "Edges") => Some(
            "The body edges to cut flat, one row each. Shift+click for several; the cut \
             distance is typed in the 3D view.",
        ),
        (Some(Tool::Fillet), "Selection") => Some(
            "The sketch corners to round. Click a corner where two lines meet; the radius is \
             typed in the 3D view.",
        ),
        (Some(Tool::Fillet), "Edges") => Some(
            "The body edges to round, one row each. Shift+click for several; the radius is \
             typed in the 3D view.",
        ),

        (Some(Tool::Combine), "Bodies") => {
            Some("The bodies to fuse into one. Click a body to add it, click it again to drop it.")
        }
        (Some(Tool::Combine), "Mode") => Some(
            "Which boolean to perform: combine, cut, intersect, or difference. The pickers \
             below follow — a two-sided operation asks for side A and side B.",
        ),
        (Some(Tool::Combine), "Side A") => {
            Some("The bodies kept. For a cut, the one being carved into.")
        }
        (Some(Tool::Combine), "Side B") => {
            Some("The bodies applied to side A. For a cut, the ones carved away.")
        }
        (Some(Tool::Combine), "Keep B bodies") => Some(
            "Leaves the side B bodies as real bodies afterwards; by default every input \
             becomes a shadow body.",
        ),
        _ => None,
    };
    per_tool.or_else(|| match label {
        // Revolve, Sweep and Loft all grow this picker when their output is set to Cut.
        "Cut bodies" => Some(
            "The bodies this cuts into. They are consumed — click a body to add it, click it \
             again to drop it.",
        ),
        "Length" => Some("The length unit a value you type is read in when you don't write one."),
        "Angle" => Some("The angle unit a value you type is read in when you don't write one."),
        "Construction" => Some(
            "Whether this lands as construction geometry — dashed guides to measure and \
             snap against that never become part of a profile.",
        ),
        "Snapping" => {
            Some("Whether drawing snaps to nearby geometry — vertices, midpoints, and axes.")
        }
        "Points" => Some(
            "How many of the two calibration points are placed. Click the image over a \
             feature of known size, once at each end.",
        ),
        "Link" => Some(
            "Whether this part follows its source file: Dynamic picks up the file's \
             saves; Static keeps the copy as-is until you update it.",
        ),
        "Source" => Some(
            "The file this part came from — found through the library, or by a path \
             relative to this document. A dot means the file has moved on; Update picks \
             it up.",
        ),
        "Placement" => Some(
            "Where this instance sits. The Move tool moves it; these are the numbers.",
        ),
        "Rotation" => Some("How this instance is turned, about the axis shown."),
        "Unit parameters" => Some(
            "The selected imported part's own knobs. Editing a value here changes this \
             one instance — never the part's file, never its other instances.",
        ),
        "Unit parameter" => Some(
            "One of the imported part's values. Click the number to type a new one for \
             this instance.",
        ),
        "Override" => Some(
            "This value is overridden for this instance (it reads gold). The button \
             beside it goes back to the part's own value.",
        ),
        "Internals" => Some(
            "The part's secondary parameters — internals its author didn't put at the \
             front door. The eye shows or hides them.",
        ),
        "Primary" => Some(
            "Whether this parameter is a knob for whoever imports this file: eye open — \
             offered first; eye closed — an internal value. Nothing is blocked either way.",
        ),
        "Library directory" => Some(
            "Where your reusable parts live. A document can import a file under this folder \
             by name, so the import is found again on any machine whose library holds the \
             same parts.",
        ),
        _ => None,
    })
}

/// A two-column field row (#371): `label` in the fixed-width left column (vertically centred
/// against the input), the input(s) from `add_input` in the aligned right column. Shared
/// with the Settings window (#720) so its rows line up — and note help (#672) — the same way.
pub(crate) fn labeled_row<R>(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    add_input: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let label = label.into();
    let help_key = label.text().to_string();
    let out = ui.horizontal(|ui| {
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
    });
    note_help(ui, &help_key, out.response.rect);
    out.inner
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
    let row = ui.horizontal(|ui| {
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
    note_help(ui, label, row.response.rect);
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
    let help_key = label.text().to_string();
    let out = ui.horizontal_top(|ui| {
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
    });
    note_help(ui, &help_key, out.response.rect);
    out.inner.inner
}

/// One row of the extrude "into" picker (#32/#35): the mode's icon followed by a radio button.
/// Selecting the radio mutates `current`, which the caller diffs to fire the change callback.
/// Egui-memory key for where the pane drew the constraint button for `kind` this frame.
fn constraint_button_rect_id(
    kind: crate::geometric_constraints::GeometricConstraintType,
) -> egui::Id {
    egui::Id::new(("constraint_button_rect", kind.label()))
}

/// Egui-memory key for the extrude Output buttons (New body / Join / Cut). Keyed by the
/// mode's *kind*, since Join/Cut carry a body index the caller may not know.
fn extrude_output_button_rect_id(mode: &ExtrudeBodyMode) -> egui::Id {
    let kind = match mode {
        ExtrudeBodyMode::NewBody => "new",
        ExtrudeBodyMode::JoinNew | ExtrudeBodyMode::MergeInto(_) => "join",
        ExtrudeBodyMode::Cut(_) => "cut",
    };
    egui::Id::new(("extrude_output_button_rect", kind))
}

/// Where the pane drew an extrude **Output** button this frame (#804) — the tutorial's orb
/// points at "pick Cut" there.
pub fn extrude_output_button_rect(
    ctx: &egui::Context,
    mode: &ExtrudeBodyMode,
) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<egui::Rect>(extrude_output_button_rect_id(mode)))
}

/// Where the Context pane's constraint button for `kind` sits on screen, if it drew one
/// this frame (#770) — the tutorial points its orb there once a step's picks are made.
pub fn constraint_button_rect(
    ctx: &egui::Context,
    kind: crate::geometric_constraints::GeometricConstraintType,
) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<egui::Rect>(constraint_button_rect_id(kind)))
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
    on_unit_edit: &mut impl FnMut(UnitPaneEdit),
    on_curve_mode_changed: &mut impl FnMut(bool),
    on_tangent_constraint_changed: &mut impl FnMut(bool),
    on_construction_changed: &mut impl FnMut(bool),
    on_rect_anchor_changed: &mut impl FnMut(crate::actions::RectAnchor),
    on_circle_anchor_changed: &mut impl FnMut(crate::actions::CircleAnchor),
    on_constraint_clicked: &mut impl FnMut(crate::geometric_constraints::GeometricConstraintType),
    on_snapping_changed: &mut impl FnMut(bool),
    on_extrude_body_mode_changed: &mut impl FnMut(ExtrudeBodyMode),
    on_extrude_symmetric_changed: &mut impl FnMut(bool),
    on_extrude_edit: &mut impl FnMut(ExtrudeEdit),
    on_units_changed: &mut impl FnMut(UnitsChoice),
    on_material_edit: &mut impl FnMut(MaterialEdit),
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
    on_shape_edit: &mut impl FnMut(ShapeEdit),
    on_joint_edit: &mut impl FnMut(JointEdit),
    on_joint_edit_start: &mut impl FnMut(usize),
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
    on_dimension_edit: &mut impl FnMut(DimensionEditEdit),
    on_treatment_edit: &mut impl FnMut(TreatmentEdit),
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

    // The dimension being typed (#775): the same value, editable here as well as in the
    // floating input on the drawing, and the blue Go button every other tool commits with.
    if let Some(control) = &content.dimension_edit {
        any_control = true;
        let label = if control.is_angle { "Angle" } else { "Span" };
        let kind = if control.is_angle {
            crate::expression_input::ValueKind::Angle
        } else {
            crate::expression_input::ValueKind::Length
        };
        let mut pending: Option<DimensionEditEdit> = None;
        labeled_row(ui, label, |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                let mut text = control.text.clone();
                crate::expression_input::ValueInput::new("dimension_value", kind)
                    .width(110.0)
                    .show(ui, &mut text, doc);
                // Emit on any buffer difference, not just `changed()` — autocomplete
                // rewrites the buffer behind egui's back (#517).
                if text != control.text {
                    pending = Some(DimensionEditEdit::SetText(text));
                }
            });
        });
        if let Some(edit) = pending {
            on_dimension_edit(edit);
        }
        if primary_button(ui, controls_enabled, "Set dimension") {
            on_dimension_edit(DimensionEditEdit::Commit);
        }
        ui.add_space(4.0);
    }

    // The chamfer/fillet amount, mirrored from the floating field with the usual Go
    // button (#792).
    if let Some(control) = &content.treatment {
        any_control = true;
        let mut pending: Option<TreatmentEdit> = None;
        labeled_row(ui, control.label(), |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                let mut text = control.text.clone();
                crate::expression_input::ValueInput::new(
                    "treatment_amount",
                    crate::expression_input::ValueKind::Length,
                )
                .width(110.0)
                .show(ui, &mut text, doc);
                if text != control.text {
                    pending = Some(TreatmentEdit::SetText(text));
                }
            });
        });
        if let Some(edit) = pending {
            on_treatment_edit(edit);
        }
        let action = match control.kind {
            crate::model::VertexTreatmentKind::Fillet => "Fillet",
            crate::model::VertexTreatmentKind::Chamfer => "Chamfer",
        };
        if primary_button(ui, controls_enabled, action) {
            on_treatment_edit(TreatmentEdit::Commit);
        }
        ui.add_space(4.0);
    }

    // The drawing workbench's Select tool has its own always-visible element picker (#346): a
    // label-only combo box over the selected projections/text/dimensions, kept in sync with the
    // Elements pane and the page.
    if let Some(rows) = &content.drawing_selection {
        any_control = true;
        // A real picker (#967), drawn here where it belongs in the drawing block. Each row
        // carries the icon its kind gives it (#363), which is why the three drawing kinds are
        // separate `ElementKind`s rather than one.
        let view = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::DrawingSelection);
        labeled_row_top(ui, "Selection", |ui| {
        ui.add_enabled_ui(controls_enabled, |ui| {
            if let Some(event) = view.and_then(|view| {
                crate::element_picker::show(ui, &view.picker, doc, "drawing_selection_picker")
            }) {
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
        let _ = base;
        let view = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::DrawingAlignBase);
        labeled_row_top(ui, "Base view", |ui| {
        ui.add_enabled_ui(controls_enabled, |ui| {
            if let Some(event) = view.and_then(|view| {
                crate::element_picker::show(ui, &view.picker, doc, "drawing_align_base_picker")
            }) {
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

    // The selected unit instance (#734): link, source (with staleness + update), and
    // placement values. Name is the shared Name row above; moving is the Move tool's job.
    if let Some(control) = &content.unit_instance {
        any_control = true;
        labeled_row(ui, "Link", |ui| {
            for (mode, label) in [
                (crate::model::LinkMode::Dynamic, "Dynamic"),
                (crate::model::LinkMode::Static, "Static"),
            ] {
                if ui.selectable_label(control.link == mode, label).clicked()
                    && control.link != mode
                {
                    on_unit_edit(UnitPaneEdit::SetLink { unit: control.unit, link: mode });
                }
            }
        });
        labeled_row(ui, "Source", |ui| {
            ui.add(egui::Label::new(egui::RichText::new(&control.source).size(11.0)).truncate());
            if health.stale_units.contains(&control.unit) {
                let (dot, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    dot.center(),
                    3.0,
                    crate::document_health::UNSTABLE_DISPLAY,
                );
                if ui.button("Update").clicked() {
                    on_unit_edit(UnitPaneEdit::Sync { unit: control.unit });
                }
            }
        });
        labeled_row(ui, "Placement", |ui| {
            ui.label(egui::RichText::new(&control.position).size(11.0));
        });
        labeled_row(ui, "Rotation", |ui| {
            ui.label(egui::RichText::new(&control.rotation).size(11.0));
        });
    }

    if let Some(rows) = &content.constraints {
        any_control = true;
        let header = ui.label(
            egui::RichText::new("Constraints")
                .color(egui::Color32::from_gray(130))
                .size(11.5),
        );
        note_help(ui, "Constraints", header.rect);
        for row in rows {
            ui.horizontal(|ui| {
                let enabled = controls_enabled && row.enabled;
                shortcuts::show_constraint_shortcut_left(
                    ui,
                    shortcuts::geometric_constraint_shortcut(row.kind),
                    enabled,
                );
                // The axis-parallel buttons draw their own glyph (#751): an arrow in the
                // axis's colour, rotated to the axis's current on-screen direction — so
                // "which way will this line snap" always matches the view.
                use crate::geometric_constraints::GeometricConstraintType as G;
                let axis = match row.kind {
                    G::AlongXAxis => Some((
                        content.constraint_axis_dirs.map(|d| d.0),
                        crate::col::X_AXIS,
                        egui::vec2(1.0, 0.0),
                    )),
                    G::AlongYAxis => Some((
                        content.constraint_axis_dirs.map(|d| d.1),
                        crate::col::Y_AXIS,
                        egui::vec2(0.0, -1.0),
                    )),
                    _ => None,
                };
                let response = match axis {
                    Some((dir, color, fallback)) => {
                        axis_constraint_button(ui, enabled, dir.unwrap_or(fallback), color)
                    }
                    None => ui.add_enabled(
                        enabled,
                        egui::Button::new(egui::Image::new(crate::icons::sized_texture(
                            ui.ctx(),
                            icon_for_constraint(row.kind),
                        )))
                        .frame(true),
                    ),
                }
                .on_hover_text(row.kind.label());
                // Where the tutorial's orb points once both picks are made (#770).
                ctx.data_mut(|d| {
                    d.insert_temp(constraint_button_rect_id(row.kind), response.rect)
                });
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
    for view in content
        .tool_pickers
        .iter()
        .filter(|v| v.render == PickerRender::Shared)
    {
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

    if let Some(control) = &content.revolve {
        any_control = true;
        // Profile and Axis are real `ToolPickerView`s now (#955), rendered with every other
        // tool picker above; only the parameters and the commit button live here.
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
        let ready = !control.faces.is_empty() && control.axis.is_some();
        if primary_button(ui, ready && controls_enabled, "Revolve") {
            on_revolve_edit(RevolveEdit::Commit);
        }
    }

    if let Some(control) = &content.sweep {
        any_control = true;
        // Profile and Path are real `ToolPickerView`s now (#955), rendered with every other
        // tool picker above; only the output choice and the commit button live here.
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
        let ready = !control.faces.is_empty() && !control.path.is_empty();
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

        // The picked anchor set — face, edge, vertex, or line+point (#474/#483). Backed by a
        // real picker (#955) but drawn from the tool's rows, which name each half of a
        // line+point set the way the plane reads it ("Vertex ⊥ Edge") rather than by the
        // element's own name.
        let anchor = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::PlaneAnchor);
        labeled_row_top(ui, "Anchor", |ui| {
            let rows: Vec<(crate::icons::IconId, String)> = control
                .anchor_labels
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let icon = control
                        .anchor_elements
                        .get(i)
                        .map(|e| crate::element_picker::ElementKind::of(e).icon())
                        .unwrap_or(crate::icons::IconId::Plane);
                    (icon, label.clone())
                })
                .collect();
            let pickable = anchor
                .map(|v| v.picker.filter().pickable_icons())
                .unwrap_or_else(|| vec![crate::icons::IconId::Plane]);
            if let Some(event) = crate::element_picker::show_rows(
                ui,
                "plane_anchor",
                anchor.is_some_and(|v| v.picker.is_focused()),
                &pickable,
                false,
                &rows,
            ) {
                match event {
                    crate::element_picker::PickerEvent::Focus => {}
                    crate::element_picker::PickerEvent::Remove(i) => {
                        on_plane_tool_edit(PlaneToolEdit::RemoveAnchor(i))
                    }
                    crate::element_picker::PickerEvent::Clear => {
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
                labeled_row(ui, "Tilt", |ui| {
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
            // Two-column like every other checkbox row (#933): the label in the left
            // column, the box itself in the right one.
            let mut keep_b = control.keep_b;
            let changed = labeled_row(ui, "Keep B bodies", |ui| {
                ui.checkbox(&mut keep_b, "")
                    .on_hover_text("Leave the B-side inputs as real bodies instead of shadows")
                    .changed()
            });
            if changed {
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
    }

    if let Some(control) = &content.move_op {
        any_control = true;
        ui.separator();
        // The picked bodies render through the unified element picker (see `tool_pickers`).
        let mut pending: Option<MoveEdit> = None;
        // Translate mode (#648): snapping a picked point onto another (the default), or typing
        // and dragging X/Y/Z outright.
        {
            use crate::model::MoveTranslateMode as M;
            let mut mode = control.translate_mode;
            labeled_row(ui, "Translate", |ui| {
                egui::ComboBox::from_id_salt("move_translate_mode")
                    .selected_text(match mode {
                        M::Snap => "Snap",
                        M::Free => "Free",
                    })
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut mode, M::Snap, "Snap");
                        ui.selectable_value(&mut mode, M::Free, "Free");
                    });
            });
            if mode != control.translate_mode {
                pending = Some(MoveEdit::TranslateMode(mode));
            }
        }
        // Start point A is picked in both modes (#649/#668): it's the handle a snap moves
        // *from*.
        // Each point picker is built with the other tool pickers (#958) and drawn here, where
        // it belongs among the tool's controls — between the Rotation heading and the
        // Angle-snap slider, which is why it can't be hoisted into the shared block. Its rule
        // (a **start** point on one of the moving bodies, an **end** point on anything else,
        // #953) lives in the picker, so pane, hover and click path agree.
        let tool_pickers = &content.tool_pickers;
        let mut picker_row = |ui: &mut egui::Ui,
                              label: &'static str,
                              id: &'static str,
                              target: PickerTarget,
                              on_focus: MoveEdit,
                              on_clear: MoveEdit| {
            let Some(view) = tool_pickers.iter().find(|v| v.target == target) else {
                return;
            };
            labeled_row_top(ui, label, |ui| {
                if let Some(event) = crate::element_picker::show(ui, &view.picker, doc, id) {
                    pending = Some(match event {
                        crate::element_picker::PickerEvent::Focus => on_focus,
                        crate::element_picker::PickerEvent::Remove(_)
                        | crate::element_picker::PickerEvent::Clear => on_clear,
                    });
                }
            });
        };
        picker_row(
            ui,
            "Start point A",
            "move_start_point_a",
            PickerTarget::MoveStartA,
            MoveEdit::StartAFocus,
            MoveEdit::ClearStartA,
        );
        // Snap (#650/#668): end point A on stationary geometry; the offset is derived from
        // the pair, so there are no X/Y/Z fields. The optional B pair below it adds the
        // rotation (#669) — start B on a moving body, end B on the constraint sphere.
        if control.translate_mode == crate::model::MoveTranslateMode::Snap {
            picker_row(
                ui,
                "End point A",
                "move_end_point_a",
                PickerTarget::MoveEndA,
                MoveEdit::EndAFocus,
                MoveEdit::ClearEndA,
            );
            // The B and C pairs are the rotation (#915): the label says so, since the
            // four points after it turn the part rather than move it.
            section_label(ui, "Rotation");
            // How far apart the candidate dots sit on the sphere/circle (#917): a slider
            // and a value field, both clamped to 0–90°.
            let mut angle_snap: Option<f32> = None;
            labeled_row(ui, "Angle snap", |ui| {
                let mut degrees = control.angle_snap_deg;
                // Both controls have to fit the pane's right column beside each other.
                ui.spacing_mut().slider_width = 46.0;
                let slider = ui.add(
                    egui::Slider::new(&mut degrees, 0.0..=crate::actions::MAX_ANGLE_SNAP_DEG)
                        .show_value(false),
                );
                let mut text = format!("{}", (degrees * 100.0).round() / 100.0);
                let typed = crate::expression_input::ValueInput::new(
                    ("move_field", "Angle snap"),
                    crate::expression_input::ValueKind::Angle,
                )
                .width(62.0)
                .show(ui, &mut text, doc);
                if typed.changed() {
                    if let Some(v) = crate::value::eval_angle_rad_in_doc(&text, doc) {
                        degrees = v.to_degrees();
                    }
                }
                if slider.changed() || typed.changed() {
                    angle_snap = Some(degrees.clamp(0.0, crate::actions::MAX_ANGLE_SNAP_DEG));
                }
            });
            if let Some(degrees) = angle_snap {
                on_move_edit(MoveEdit::AngleSnap(degrees));
            }
            picker_row(
                ui,
                "Start point B",
                "move_start_point_b",
                PickerTarget::MoveStartB,
                MoveEdit::StartBFocus,
                MoveEdit::ClearStartB,
            );
            picker_row(
                ui,
                "End point B",
                "move_end_point_b",
                PickerTarget::MoveEndB,
                MoveEdit::EndBFocus,
                MoveEdit::ClearEndB,
            );
            picker_row(
                ui,
                "Start point C",
                "move_start_point_c",
                PickerTarget::MoveStartC,
                MoveEdit::StartCFocus,
                MoveEdit::ClearStartC,
            );
            picker_row(
                ui,
                "End point C",
                "move_end_point_c",
                PickerTarget::MoveEndC,
                MoveEdit::EndCFocus,
                MoveEdit::ClearEndC,
            );
        }
        drop(picker_row);
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
            if control.translate_mode == crate::model::MoveTranslateMode::Free {
                field(ui, "X", &control.tx, ValueKind::Length, &MoveEdit::Tx);
                field(ui, "Y", &control.ty, ValueKind::Length, &MoveEdit::Ty);
                field(ui, "Z", &control.tz, ValueKind::Length, &MoveEdit::Tz);
            }
        }
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
    }

    if let Some(op) = content.move_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit move").clicked() {
            on_move_edit_start(op);
        }
    }

    // The Create Shape tool (#909): which shape, then that shape's own dimensions.
    if let Some(control) = &content.shape {
        use crate::actions::ShapeDimension as D;
        use crate::model::PrimitiveKind as K;
        any_control = true;
        ui.separator();
        let mut pending: Option<ShapeEdit> = None;
        let mut enter_commit = false;
        labeled_row(ui, "Shape", |ui| {
            for (value, icon, tooltip) in [
                (K::Cuboid, crate::icons::IconId::ShapeCuboid, "Cuboid (B cycles)"),
                (K::Cylinder, crate::icons::IconId::ShapeCylinder, "Cylinder (B cycles)"),
                (K::Sphere, crate::icons::IconId::ShapeSphere, "Sphere (B cycles)"),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, control.kind == value, tooltip)
                    .clicked()
                    && control.kind != value
                {
                    pending = Some(ShapeEdit::Kind(value));
                }
            }
        });
        let mut dimension = |ui: &mut egui::Ui, label: &str, field: D, value: &str| {
            labeled_row(ui, label, |ui| {
                let mut text = value.to_string();
                let id = egui::Id::new(("shape_field", label));
                let resp = crate::expression_input::ValueInput::from_id(
                    id,
                    crate::expression_input::ValueKind::Length,
                )
                .width(90.0)
                .show(ui, &mut text, doc);
                // The phase's own field takes the keyboard, so its size can be typed
                // straight after the click that asked for it (#912).
                if control.focus_field == Some(field) && !resp.has_focus() {
                    resp.request_focus();
                }
                if resp.changed() {
                    pending = Some(ShapeEdit::Dimension(field, text.clone()));
                }
                // Enter in a shape field creates the shape, like the sketch Rectangle's
                // typed dimensions do (#912) — the field holds the keyboard, so the
                // viewport's own Enter never sees it.
                let has_keyboard = ui.ctx().memory(|m| m.focused()) == Some(id);
                if has_keyboard && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    enter_commit = true;
                }
            });
        };
        match control.kind {
            K::Cuboid => {
                dimension(ui, "Width", D::Width, &control.width);
                dimension(ui, "Depth", D::Depth, &control.depth);
                dimension(ui, "Height", D::Height, &control.height);
            }
            K::Cylinder => {
                dimension(ui, "Radius", D::Radius, &control.radius);
                dimension(ui, "Height", D::Height, &control.height);
            }
            K::Sphere => dimension(ui, "Radius", D::Radius, &control.radius),
        }
        if let Some(edit) = pending {
            on_shape_edit(edit);
        }
        ui.add_space(2.0);
        let create = primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Create" },
        );
        if create || (enter_commit && control.can_commit) {
            on_shape_edit(ShapeEdit::Commit);
        }
    }

    if let Some(control) = &content.joint {
        any_control = true;
        ui.separator();
        let mut pending: Option<JointEdit> = None;
        // The joint-type dropdown (#894).
        {
            use crate::model::JointKind as K;
            let mut kind = control.kind.clone();
            labeled_row(ui, "Type", |ui| {
                crate::icons::show_icon(ui, crate::icons::icon_for_joint_kind(&kind));
                egui::ComboBox::from_id_salt("joint_kind")
                    .selected_text(crate::names::joint_kind_label(&kind))
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for value in [
                            K::Rigid,
                            K::Slider,
                            K::Revolute,
                            K::Cylindrical,
                            K::Planar,
                            K::Ball,
                            K::PinSlot,
                            K::Screw { lead: String::new() },
                        ] {
                            let label = crate::names::joint_kind_label(&value);
                            let selected =
                                std::mem::discriminant(&kind) == std::mem::discriminant(&value);
                            // Each kind reads by its icon as well as its name (#921), the
                            // same glyph the pane row and the 3D badge use — laid out beside
                            // the label rather than painted over it (#999).
                            let row = crate::icons::selectable_icon_label(
                                ui,
                                crate::icons::icon_for_joint_kind(&value),
                                selected,
                                label,
                            )
                            .on_hover_text(label);
                            if row.clicked() && !selected {
                                kind = value.clone();
                            }
                        }
                    });
            });
            if std::mem::discriminant(&kind) != std::mem::discriminant(&control.kind) {
                pending = Some(JointEdit::Kind(kind));
            }
        }
        // Which parts, and which of them moves. Every kind but Rigid joins exactly **two**, and
        // which is held is the whole meaning of the joint — so those are two named slots (#991),
        // the mobile part first and the part holding it second. Rigid keeps the plain list: it
        // joins any number and nothing moves.
        let side_row = |ui: &mut egui::Ui,
                        pending: &mut Option<JointEdit>,
                        target: PickerTarget,
                        label: &'static str,
                        id: &'static str,
                        on_focus: JointEdit,
                        on_clear: JointEdit| {
            let Some(view) = content.tool_pickers.iter().find(|v| v.target == target) else {
                return;
            };
            labeled_row_top(ui, label, |ui| {
                if let Some(event) = crate::element_picker::show(ui, &view.picker, doc, id) {
                    *pending = Some(match event {
                        crate::element_picker::PickerEvent::Focus => on_focus.clone(),
                        _ => on_clear.clone(),
                    });
                }
            });
        };
        if matches!(control.kind, crate::model::JointKind::Rigid) {
            let members = content
                .tool_pickers
                .iter()
                .find(|v| v.target == PickerTarget::JointMembers);
            if let Some(members) = members {
                labeled_row_top(ui, "Parts", |ui| {
                    if let Some(event) =
                        crate::element_picker::show(ui, &members.picker, doc, "joint_members")
                    {
                        pending = Some(match event {
                            crate::element_picker::PickerEvent::Focus => JointEdit::MembersFocus,
                            crate::element_picker::PickerEvent::Remove(i) => {
                                JointEdit::RemoveMember(i)
                            }
                            crate::element_picker::PickerEvent::Clear => JointEdit::ClearMembers,
                        });
                    }
                });
            }
        } else {
            side_row(
                ui,
                &mut pending,
                PickerTarget::JointMobile,
                "Moving",
                "joint_mobile",
                JointEdit::MobileFocus,
                JointEdit::ClearMobile,
            );
            side_row(
                ui,
                &mut pending,
                PickerTarget::JointFixed,
                "Fixed",
                "joint_fixed",
                JointEdit::FixedFocus,
                JointEdit::ClearFixed,
            );
        }
        // The screw's lead (#894): mm of travel per full turn.
        if let crate::model::JointKind::Screw { lead } = &control.kind {
            labeled_row(ui, "Lead", |ui| {
                let mut text = lead.clone();
                let resp = crate::expression_input::ValueInput::new(
                    ("joint_field", "Lead"),
                    crate::expression_input::ValueKind::Length,
                )
                .width(90.0)
                .show(ui, &mut text, doc);
                if resp.changed() {
                    pending = Some(JointEdit::Lead(text));
                }
            });
        }
        // Which side is held (#894): the base. Clicking swaps it. Only for **Rigid** now — the
        // other kinds name their two sides outright in the Mobile/Fixed slots above (#991), so
        // a swap button would be a second, vaguer way to say the same thing.
        if control.members.len() >= 2 && matches!(control.kind, crate::model::JointKind::Rigid) {
            labeled_row(ui, "Base", |ui| {
                if ui
                    .button(&control.base_label)
                    .on_hover_text("Swap which side is held")
                    .clicked()
                {
                    pending = Some(JointEdit::SwapBase);
                }
            });
        }
        // Built with the other tool pickers (#958), drawn here. Moving picks sit on the
        // driven part, fixed picks on the base (#953) — the picker refuses a pick on the
        // other side rather than letting a wrong one land.
        let tool_pickers = &content.tool_pickers;
        // The **mate** — where the parts start out (#1021): *put this face on that face,
        // then line this up with that.* Alternating labeled rows (#1024), not two columns:
        // the two-column layout made the pane too wide and re-used one widget id per line-up
        // row (#1025). Each picker gets a unique id salted by role and row index.
        section_label(ui, "Mate");
        let mate_row = |ui: &mut egui::Ui,
                        pending: &mut Option<JointEdit>,
                        label: &'static str,
                        id: (&str, &str, usize),
                        target: PickerTarget,
                        on_focus: JointEdit,
                        on_clear: JointEdit| {
            let Some(view) = tool_pickers.iter().find(|v| v.target == target) else {
                return;
            };
            labeled_row_top(ui, label, |ui| {
                if let Some(event) = crate::element_picker::show(ui, &view.picker, doc, id) {
                    *pending = Some(match event {
                        crate::element_picker::PickerEvent::Focus => on_focus.clone(),
                        _ => on_clear.clone(),
                    });
                }
            });
        };
        mate_row(
            ui,
            &mut pending,
            "Moving face",
            ("joint_mate", "moving_face", 0),
            PickerTarget::JointMovingFace,
            JointEdit::MovingFaceFocus,
            JointEdit::ClearMovingFace,
        );
        mate_row(
            ui,
            &mut pending,
            "Fixed face",
            ("joint_mate", "fixed_face", 0),
            PickerTarget::JointFixedFace,
            JointEdit::FixedFaceFocus,
            JointEdit::ClearFixedFace,
        );
        // How the face pair lands: which way round, and the gap it's held off by (#1014).
        if control.moving_face.is_some() || control.fixed_face.is_some() {
            labeled_row(ui, "Flip", |ui| {
                let mut flip = control.flip;
                if ui.checkbox(&mut flip, "").changed() {
                    pending = Some(JointEdit::Flip(flip));
                }
            });
            labeled_row(ui, "Offset", |ui| {
                let mut text = control.offset.clone();
                let resp = crate::expression_input::ValueInput::new(
                    ("joint_field", "Offset"),
                    crate::expression_input::ValueKind::Length,
                )
                .width(90.0)
                .show(ui, &mut text, doc);
                if resp.changed() {
                    pending = Some(JointEdit::Offset(text));
                }
            });
        }
        for i in 0..control.line_up.len() {
            mate_row(
                ui,
                &mut pending,
                line_up_heading(i, true),
                ("joint_mate", "line_up_moving", i),
                PickerTarget::JointLineUpMoving(i),
                JointEdit::LineUpMovingFocus(i),
                JointEdit::ClearLineUpMoving(i),
            );
            mate_row(
                ui,
                &mut pending,
                line_up_heading(i, false),
                ("joint_mate", "line_up_fixed", i),
                PickerTarget::JointLineUpFixed(i),
                JointEdit::LineUpFixedFocus(i),
                JointEdit::ClearLineUpFixed(i),
            );
        }
        // What this kind of joint can do, under its own name (#997): the freedoms it has and
        // the limits on them. Rigid has neither, so it gets no section at all.
        if !matches!(control.kind, crate::model::JointKind::Rigid) {
            section_label(ui, crate::names::joint_kind_label(&control.kind));
        }
        // Position fields per kind (#894): what each freedom is called and measures.
        {
            use crate::expression_input::ValueKind;
            use crate::model::JointKind as K;
            let mut field = |ui: &mut egui::Ui,
                             label: &str,
                             value: &str,
                             kind: ValueKind,
                             make: &dyn Fn(String) -> JointEdit| {
                labeled_row(ui, label, |ui| {
                    let mut text = value.to_string();
                    let resp =
                        crate::expression_input::ValueInput::new(("joint_field", label), kind)
                            .width(90.0)
                            .show(ui, &mut text, doc);
                    if resp.changed() {
                        pending = Some(make(text));
                    }
                });
            };
            match &control.kind {
                K::Rigid => {}
                K::Slider => {
                    field(ui, "Slide", &control.position, ValueKind::Length, &JointEdit::Position)
                }
                K::Revolute | K::Screw { .. } => {
                    field(ui, "Angle", &control.position, ValueKind::Angle, &JointEdit::Position)
                }
                K::Cylindrical | K::PinSlot => {
                    field(ui, "Slide", &control.position, ValueKind::Length, &JointEdit::Position);
                    field(ui, "Angle", &control.position2, ValueKind::Angle, &JointEdit::Position2);
                }
                K::Planar => {
                    field(ui, "U", &control.position, ValueKind::Length, &JointEdit::Position);
                    field(ui, "V", &control.position2, ValueKind::Length, &JointEdit::Position2);
                    field(ui, "Spin", &control.position3, ValueKind::Angle, &JointEdit::Position3);
                }
                K::Ball => {
                    field(ui, "Yaw", &control.position, ValueKind::Angle, &JointEdit::Position);
                    field(ui, "Pitch", &control.position2, ValueKind::Angle, &JointEdit::Position2);
                    field(ui, "Roll", &control.position3, ValueKind::Angle, &JointEdit::Position3);
                }
            }
            // Travel limits (#896): slide bounds for the sliding kinds — as expressions
            // or a picked stop face/plane — and turn bounds for the turning kinds.
            let slides = matches!(
                control.kind,
                K::Slider | K::Cylindrical | K::Planar | K::PinSlot | K::Screw { .. }
            );
            let turns = matches!(
                control.kind,
                K::Revolute | K::Cylindrical | K::Ball | K::PinSlot | K::Screw { .. }
            );
            if slides {
                field(ui, "Slide min", &control.slide_min, ValueKind::Length, &JointEdit::SlideMin);
                field(ui, "Slide max", &control.slide_max, ValueKind::Length, &JointEdit::SlideMax);
            }
            if turns {
                field(ui, "Turn min", &control.turn_min, ValueKind::Angle, &JointEdit::TurnMin);
                field(ui, "Turn max", &control.turn_max, ValueKind::Angle, &JointEdit::TurnMax);
            }
            drop(field);
            if slides {
                // A stop is a plane or a flat face the travel ends at (#896/#955), built with
                // the other tool pickers (#958) and drawn here.
                let mut stop_row = |ui: &mut egui::Ui,
                                    label: &'static str,
                                    id: &'static str,
                                    target: PickerTarget,
                                    on_focus: JointEdit,
                                    on_clear: JointEdit| {
                    let Some(view) = tool_pickers.iter().find(|v| v.target == target) else {
                        return;
                    };
                    labeled_row_top(ui, label, |ui| {
                        if let Some(event) =
                            crate::element_picker::show(ui, &view.picker, doc, id)
                        {
                            pending = Some(match event {
                                crate::element_picker::PickerEvent::Focus => on_focus,
                                crate::element_picker::PickerEvent::Remove(_)
                                | crate::element_picker::PickerEvent::Clear => on_clear,
                            });
                        }
                    });
                };
                stop_row(
                    ui,
                    "Min stop",
                    "joint_slide_min_stop",
                    PickerTarget::JointMinStop,
                    JointEdit::SlideMinStopFocus,
                    JointEdit::ClearSlideMinStop,
                );
                stop_row(
                    ui,
                    "Max stop",
                    "joint_slide_max_stop",
                    PickerTarget::JointMaxStop,
                    JointEdit::SlideMaxStopFocus,
                    JointEdit::ClearSlideMaxStop,
                );
            }
        }
        // The preview sweep's animation (#906): one switch for every joint.
        {
            let mut animate = control.animate;
            labeled_row(ui, "Animate", |ui| {
                if ui.checkbox(&mut animate, "").changed() {
                    pending = Some(JointEdit::Animate(animate));
                }
            });
        }
        // The rest pose (#898): capture the current position, or go back to it — only
        // meaningful once the joint exists.
        if control.editing {
            labeled_row(ui, "Rest", |ui| {
                if ui
                    .button("Set")
                    .on_hover_text("Set the current position as the rest position")
                    .clicked()
                {
                    pending = Some(JointEdit::SetRest);
                }
                if ui
                    .button("Revert")
                    .on_hover_text("Put the joint back to its rest position")
                    .clicked()
                {
                    pending = Some(JointEdit::Revert);
                }
            });
        }
        if let Some(edit) = pending {
            on_joint_edit(edit);
        }
        ui.add_space(2.0);
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Joint" },
        ) {
            on_joint_edit(JointEdit::Commit);
        }
    }

    if let Some(op) = content.joint_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit joint").clicked() {
            on_joint_edit_start(op);
        }
    }

    if let Some(control) = &content.mirror_op {
        any_control = true;
        // No divider between the pickers above and this Do button — the Mirror plane picker,
        // the Bodies picker, and the button read as one contiguous block (#602).
        // The mirror plane and the bodies to mirror both render above through the unified
        // element pickers (see `tool_pickers`: `MirrorPlane` then `MirrorTargets`, #566).
        // Output row (#639): the same segmented icon group, labels, and placement the Revolve
        // tool uses — New body / Join body / Cut — so the two panes read alike.
        let mode = control.mode;
        labeled_row(ui, "Output", |ui| {
            for (value, icon, tooltip) in [
                (
                    crate::model::MirrorMode::NewBody,
                    crate::icons::IconId::NewBody,
                    "New body",
                ),
                (
                    crate::model::MirrorMode::Join,
                    crate::icons::IconId::AddToBody,
                    "Join body",
                ),
                (
                    crate::model::MirrorMode::Cut,
                    crate::icons::IconId::CutBody,
                    "Cut",
                ),
            ] {
                if crate::icons::selectable_icon_button(ui, icon, mode == value, tooltip)
                    .clicked()
                    && mode != value
                {
                    on_mirror_edit(MirrorEdit::Mode(value));
                }
            }
        });
        ui.add_space(2.0);
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Mirror" },
        ) {
            on_mirror_edit(MirrorEdit::Commit);
        }
    }

    if let Some(op) = content.mirror_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit mirror").clicked() {
            on_mirror_edit_start(op);
        }
    }

    if let Some(control) = &content.repeat_op {
        any_control = true;
        ui.separator();
        // The picked bodies render through the unified element picker (see `tool_pickers`).
        // Construction-plane targets (#221) are picked via the Elements pane / viewport, like the
        // Move tool's planes — surfaced here as a count so the picked set is visible.
        let mut pending: Option<RepeatEdit> = None;
        // Values, not sentences (#662): the picked plane/sketch/cut sets as counts.
        if !control.plane_targets.is_empty() {
            let count = control.plane_targets.len();
            labeled_row(ui, "Planes", |ui| {
                ui.label(count.to_string());
            });
        }
        if !control.sketch_targets.is_empty() {
            let count = control.sketch_targets.len();
            labeled_row(ui, "Sketches", |ui| {
                ui.label(count.to_string());
            });
        }
        if !control.extrusion_targets.is_empty() {
            let count = control.extrusion_targets.len();
            labeled_row(ui, "Cuts", |ui| {
                ui.label(count.to_string());
            });
        }
        // Axis element picker (#257/#439): empty until an axis is picked — click a straight
        // edge, a sketch line, or an origin axis in the viewport; the ✕ clears it. It reads
        // as the focused picker exactly while unset — once targets are seeded, the axis is
        // the next thing to pick. The X/Y/Z shortcut buttons are gone (#643): the origin axes
        // are pickable in the viewport like everything else, so the buttons were a second,
        // inconsistent way in.
        // Built with the other tool pickers so the rest of the app can see it (#954/#958);
        // drawn here, where it belongs among the Repeat block's own controls.
        if let Some(view) = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::RepeatPath)
        {
            labeled_row_top(ui, "Path", |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &view.picker, doc, "repeat_axis")
                {
                    if matches!(
                        event,
                        crate::element_picker::PickerEvent::Remove(_)
                            | crate::element_picker::PickerEvent::Clear
                    ) {
                        pending = Some(RepeatEdit::ClearAxis);
                    }
                }
            });
            // Which way along it (#989). Sits with the path because it is a property of the
            // path, not of the spacing — and it is only answerable once one is picked.
            let mut flip = control.flip;
            if ui
                .add_enabled_ui(controls_enabled, |ui| checkbox_row(ui, "Flip", &mut flip, None))
                .inner
            {
                pending = Some(RepeatEdit::SetFlip(flip));
            }
        }
        // Along the path, or around it as an axis of rotation (#839) — the same segmented
        // icon pair the other tools' mode choices use.
        labeled_row(ui, "Repeat", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                ui.horizontal(|ui| {
                    for (around, icon, tip) in [
                        (
                            false,
                            crate::icons::IconId::RepeatAlongPath,
                            "Along the path".to_string(),
                        ),
                        (
                            true,
                            crate::icons::IconId::RepeatAroundAxis,
                            if control.can_turn_about_path {
                                "Around the path, as an axis of rotation".to_string()
                            } else {
                                "A curved path is followed, not turned about".to_string()
                            },
                        ),
                    ] {
                        // A curved path can only be followed (#840).
                        let enabled = !around || control.can_turn_about_path;
                        let clicked = ui
                            .add_enabled_ui(enabled, |ui| {
                                crate::icons::selectable_icon_button(
                                    ui,
                                    icon,
                                    control.around_axis == around,
                                    tip,
                                )
                                .clicked()
                            })
                            .inner;
                        if clicked && control.around_axis != around {
                            pending = Some(RepeatEdit::SetAroundAxis(around));
                        }
                    }
                });
            });
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
                let row = ui.horizontal(|ui| {
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
                    // A read-only value renders through the very same widget as an editable
                    // one, just disabled (#654) — a hand-rolled `TextEdit` sized its *text*
                    // rather than its box, coming out both wider and shorter than its
                    // neighbours.
                    let read_only = |ui: &mut egui::Ui, shown: Option<String>, hover: &str| {
                        let mut text = shown.unwrap_or_default();
                        ui.add_enabled_ui(false, |ui| {
                            crate::expression_input::ValueInput::from_id(
                                repeat_value_field_id(label).with("computed"),
                                crate::expression_input::ValueKind::Length,
                            )
                            .width(VAR_FIELD_W)
                            .show(ui, &mut text, doc)
                            .on_hover_text(hover.to_string());
                        });
                    };
                    // A picked distance target (#645) drives the Distance value, so its field
                    // reads back the derived length instead of an expression.
                    let target_driven = var == RepeatVar::Distance
                        && control.length_target.is_some();
                    if target_driven {
                        read_only(
                            ui,
                            control.length_target_value.clone(),
                            "Measured to the picked target",
                        );
                    } else if computed {
                        read_only(
                            ui,
                            control.computed_value.clone(),
                            "Computed from the other two",
                        );
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
                note_help(ui, label, row.response.rect);
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
            // Turning about the axis measures a sweep, not a length (#839): the row becomes
            // **Angle** and loses the start/end measure toggle, which means nothing for a turn.
            if control.around_axis {
                var_row(ui, RepeatVar::Distance, "Angle", &control.length, None, &RepeatEdit::Distance);
            } else {
                var_row(
                    ui,
                    RepeatVar::Distance,
                    "Distance",
                    &control.length,
                    Some((dist_icon, RepeatEdit::ToggleDistanceEnd)),
                    &RepeatEdit::Distance,
                );
            }
        }
        // Distance-target picker (#645): a face, construction plane, or vertex the pattern
        // runs out to, so the distance follows that geometry instead of a typed number —
        // the Repeat tool's version of the Extrude tool's "Up to" picker. Focus it, then
        // click the target in the viewport; the ✕ hands Distance back to its expression.
        // A sweep has nothing to measure to, so the distance-target picker stands down (#839).
        let length_target = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::RepeatDistanceTo);
        if let Some(length_target) = length_target {
        labeled_row_top(ui, "Distance to", |ui| {
            ui.add_enabled_ui(!control.around_axis, |ui| {
            if let Some(event) =
                crate::element_picker::show(ui, &length_target.picker, doc, "repeat_length_target")
            {
                pending = Some(match event {
                    crate::element_picker::PickerEvent::Focus => RepeatEdit::LengthTargetFocus,
                    crate::element_picker::PickerEvent::Remove(_)
                    | crate::element_picker::PickerEvent::Clear => RepeatEdit::ClearLengthTarget,
                });
            }
            });
        });
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

    // In-sketch Repeat tool (#232): entities + direction + count/gap/distance, laid out like
    // the 3D section one dimension down (#835) — pickers for both, locks on the value rows.
    if let Some(control) = &content.sketch_repeat {
        use crate::model::RepeatVar;
        any_control = true;
        ui.separator();
        let mut pending: Option<SketchRepeatEdit> = None;
        // The entities being copied (#835): the same unified picker every other in-sketch
        // tool uses, so rows can be dropped individually or cleared. Built with the other tool
        // pickers so the rest of the app can see it (#958); drawn here, in the tool's block.
        if let Some(view) = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::SketchRepeatEntities)
        {
        labeled_row_top(ui, "Entities", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &view.picker, doc, "sketch_repeat_picker")
                {
                    match event {
                        crate::element_picker::PickerEvent::Focus => {}
                        crate::element_picker::PickerEvent::Remove(i) => {
                            if let Some(el) = control.picked.get(i).cloned() {
                                pending = Some(SketchRepeatEdit::Remove(el));
                            }
                        }
                        crate::element_picker::PickerEvent::Clear => {
                            pending = Some(SketchRepeatEdit::Clear);
                        }
                    }
                }
            });
        });
        }
        // The direction line (#835), the in-sketch counterpart of the 3D section's Axis
        // picker: empty means the sketch's U axis. Focus it and the next viewport click sets
        // it; the ✕ hands the direction back to the U axis.
        if let Some(view) = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::SketchRepeatDirection)
        {
        labeled_row_top(ui, "Direction", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &view.picker, doc, "sketch_repeat_direction")
                {
                    pending = Some(match event {
                        crate::element_picker::PickerEvent::Focus => {
                            SketchRepeatEdit::DirectionFocus
                        }
                        crate::element_picker::PickerEvent::Remove(_)
                        | crate::element_picker::PickerEvent::Clear => {
                            SketchRepeatEdit::ClearDirection
                        }
                    });
                }
            });
        });
        }
        let mut var_row = |ui: &mut egui::Ui,
                           var: RepeatVar,
                           label: &str,
                           value: &str,
                           toggle: Option<(crate::icons::IconId, SketchRepeatEdit)>,
                           make: &dyn Fn(String) -> SketchRepeatEdit| {
            let computed = control.computed_var == var;
            let row = ui.horizontal(|ui| {
                // Icon + label share the fixed label column (#371) so the inputs align.
                ui.allocate_ui_with_layout(
                    egui::vec2(FIELD_LABEL_W, 18.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_size(egui::vec2(FIELD_LABEL_W, 18.0));
                        match toggle {
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
                // Both states render at the same width (#641) so the column doesn't jump as
                // the computed one moves between rows.
                const VAR_FIELD_W: f32 = 110.0;
                if computed {
                    let mut text = control.computed_value.clone().unwrap_or_default();
                    ui.add_enabled_ui(false, |ui| {
                        crate::expression_input::ValueInput::new(
                            ("sketch_repeat_var_field", label, "computed"),
                            crate::expression_input::ValueKind::Length,
                        )
                        .width(VAR_FIELD_W)
                        .show(ui, &mut text, doc)
                        .on_hover_text("Computed from the other two");
                    });
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
                    .width(VAR_FIELD_W)
                    .show(ui, &mut text, doc);
                    if resp.changed() {
                        pending = Some(make(text));
                    }
                }
                // Lock (#642/#835): green on the value the app computes, grey on the two the
                // user sets; clicking a grey lock moves the green one there.
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
                    pending = Some(SketchRepeatEdit::SetComputed(var));
                }
            });
            note_help(ui, label, row.response.rect);
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
        // The blue primary button, in the input column, like every other tool's commit.
        if primary_button(
            ui,
            control.can_commit && controls_enabled,
            if control.editing { "Apply changes" } else { "Repeat" },
        ) {
            on_sketch_repeat_edit(SketchRepeatEdit::Commit);
        }
    }

    if let Some(control) = &content.sketch_offset {
        any_control = true;
        ui.separator();
        // Element picker of lines/circles in the offset set (#493). Registered with the other
        // tool pickers (#958), drawn here.
        if let Some(view) = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::SketchOffsetEntities)
        {
        labeled_row_top(ui, "Entities", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &view.picker, doc, "sketch_offset_picker")
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
        }
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
    }

    if let Some(control) = &content.sketch_mirror {
        any_control = true;
        ui.separator();
        // Primary: the mirror line, as a single-line element picker (#534). Removing it lets
        // the next viewport click pick a new mirror line. Registered with the other tool
        // pickers (#958), drawn here.
        if let Some(view) = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::SketchMirrorLine)
        {
        labeled_row_top(ui, "Mirror line", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &view.picker, doc, "sketch_mirror_line_picker")
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
        }
        // Secondary: the reflected shapes (unified element picker).
        if let Some(view) = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::SketchMirrorShapes)
        {
        labeled_row_top(ui, "Shapes", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &view.picker, doc, "sketch_mirror_picker")
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
        }
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
    }

    if let Some(op) = content.repeat_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit repeat").clicked() {
            on_repeat_edit_start(op);
        }
    }

    if let Some(control) = &content.slice_op {
        any_control = true;
        let mut pending: Option<SliceEdit> = None;
        // The Targets and Cutters pickers are real `ToolPickerView`s now (#955), rendered with
        // every other tool picker above; only the toggle and the commit button live here.
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

    // In-sketch Slice (#238/#955): Targets and Cutters are real `ToolPickerView`s now,
    // rendered with every other tool picker above; only the commit button lives here.
    if let Some(control) = &content.sketch_slice {
        any_control = true;
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
        labeled_row(ui, "Shows", |ui| {
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
        let header = ui.label(
            egui::RichText::new("Projection")
                .color(egui::Color32::from_gray(130))
                .size(11.5),
        );
        note_help(ui, "Projection", header.rect);
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
    }

    if let Some(op) = content.revolve_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit revolve").clicked() {
            on_revolve_edit_start(op);
        }
    }

    if let Some(op) = content.sweep_edit_start {
        any_control = true;
        ui.separator();
        if ui.button("Edit sweep").clicked() {
            on_sweep_edit_start(op);
        }
    }

    if let Some(image) = content.calibrate_start {
        any_control = true;
        ui.separator();
        let resp = ui.button("Calibrate scale");
        note_help(ui, "Calibrate scale", resp.rect);
        if resp.clicked() {
            on_calibrate_start(image);
        }
    }

    if let Some(placed) = content.calibrate_pending {
        any_control = true;
        ui.separator();
        section_label(ui, "Calibrate scale");
        // A value, not prose (#662): how the two-point placement is going. What the
        // points mean lives in help mode.
        labeled_row(ui, "Points", |ui| {
            ui.label(format!("{placed} / 2"));
        });
    }

    if let Some(control) = content.calibrate_image {
        any_control = true;
        ui.separator();
        section_label(ui, "Calibrate scale");
        labeled_row(ui, "Real length", |ui| {
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
            let target = content
                .tool_pickers
                .iter()
                .find(|v| v.target == PickerTarget::ExtrudeUpTo);
            if let Some(target) = target {
            labeled_row_top(ui, "Up to", |ui| {
                if let Some(event) =
                    crate::element_picker::show(ui, &target.picker, doc, "extrude_target")
                {
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
                    // Join means "into the host body" when the sketch sits on one, and
                    // "into a single body" when it doesn't and there's more than one
                    // profile to join (#837).
                    let (join_mode, join_tooltip, join_enabled) = match control.merge_body {
                        Some(bi) => (
                            ExtrudeBodyMode::MergeInto(bi),
                            format!("Join {}", control.merge_body_label),
                            true,
                        ),
                        None => (
                            ExtrudeBodyMode::JoinNew,
                            if control.can_join_profiles {
                                "Join the profiles into one body".to_string()
                            } else {
                                "Join body (sketch must sit on a body face)".to_string()
                            },
                            control.can_join_profiles,
                        ),
                    };
                    for (value, icon, tooltip, enabled) in [
                        (
                            ExtrudeBodyMode::NewBody,
                            crate::icons::IconId::NewBody,
                            if control.can_join_profiles {
                                "One new body per profile".to_string()
                            } else {
                                "New body".to_string()
                            },
                            true,
                        ),
                        (
                            join_mode,
                            crate::icons::IconId::AddToBody,
                            join_tooltip,
                            join_enabled,
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
                            let response = crate::icons::selectable_icon_button(
                                ui,
                                icon,
                                mode == value,
                                tooltip,
                            );
                            // Where the tutorial's orb points at "pick Cut" (#804).
                            ctx.data_mut(|d| {
                                d.insert_temp(
                                    extrude_output_button_rect_id(&value),
                                    response.rect,
                                )
                            });
                            if response.clicked() && mode != value && enabled {
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

    // Material picker (#834): what the selected body is made of, with the way in to naming
    // and recolouring that material.
    if let Some(control) = &content.material {
        any_control = true;
        ui.separator();
        let mut pending: Option<MaterialEdit> = None;
        let selected_text = match control.current {
            None => "Mixed".to_string(),
            Some(None) => "None".to_string(),
            Some(Some(mi)) => control
                .materials
                .iter()
                .find(|(i, _, _)| *i == mi)
                .map(|(_, name, _)| name.clone())
                .unwrap_or_else(|| "None".to_string()),
        };
        labeled_row(ui, "Material", |ui| {
            ui.add_enabled_ui(controls_enabled, |ui| {
                egui::ComboBox::from_id_salt("context_material")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for (index, name, color) in &control.materials {
                            let selected = control.current == Some(Some(*index));
                            if ui
                                .horizontal(|ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(12.0, 12.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        rect,
                                        2.0,
                                        egui::Color32::from_rgb(color[0], color[1], color[2]),
                                    );
                                    ui.selectable_label(selected, name)
                                })
                                .inner
                                .clicked()
                            {
                                pending = Some(MaterialEdit::Assign(Some(*index)));
                            }
                        }
                        ui.separator();
                        if ui.selectable_label(false, "New material…").clicked() {
                            pending = Some(MaterialEdit::New);
                        }
                    });
            });
        });
        // The chosen material's own name and colour, editable in place.
        if let Some(Some(mi)) = control.current {
            if let Some((_, name, color)) =
                control.materials.iter().find(|(i, _, _)| *i == mi)
            {
                ui.add_enabled_ui(controls_enabled, |ui| {
                    labeled_row(ui, "Name", |ui| {
                        let mut text = name.clone();
                        if ui.text_edit_singleline(&mut text).changed() {
                            pending = Some(MaterialEdit::Rename(mi, text));
                        }
                    });
                    labeled_row(ui, "Colour", |ui| {
                        let mut rgb = *color;
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            pending = Some(MaterialEdit::Recolor(mi, rgb));
                        }
                    });
                });
            }
        }
        if let Some(edit) = pending {
            on_material_edit(edit);
        }
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

    /// #982: with a sketch open, the Select tool's picker view carries the sketch-only rule
    /// (#742) — the one the click path enforces — so the Exploder's fan and every other
    /// picker-driven path refuse a datum plane or outside body exactly as a click does.
    #[test]
    fn select_picker_is_sketch_scoped_while_a_sketch_is_open() {
        use crate::hierarchy::SceneElement;

        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines
            .push(crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let selection = crate::selection::SceneSelection::default();

        let open = selection_picker_for(&doc, Tool::Select, Some(sketch), &selection)
            .expect("Select always has its picker");
        assert!(open.accepts(&doc, &SceneElement::Line(0)));
        assert!(!open.accepts(&doc, &SceneElement::ConstructionPlane(1)));
        assert!(!open.accepts(&doc, &SceneElement::Body(0)));

        // Outside a sketch it takes everything again.
        let closed = selection_picker_for(&doc, Tool::Select, None, &selection)
            .expect("Select always has its picker");
        assert!(closed.accepts(&doc, &SceneElement::ConstructionPlane(1)));
    }

    /// #392: registering system fonts for the chooser preview must never crash — every face
    /// handed to egui parses (ab_glyph-validated, correct .ttc index), and the family is only
    /// used on a pass after its atlas rebuild. Runs real passes over a sample of the
    /// installed fonts; a bad face panics right here instead of in the running app.
    #[test]
    fn font_preview_registration_never_panics() {
        let ctx = egui::Context::default();
        let families = crate::text::system_font_families();
        for fam in families.iter().take(40) {
            let _ = ctx.run_ui(Default::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    if let Some(ff) = preview_font_family(ui.ctx(), fam) {
                        ui.label(egui::RichText::new(fam).family(ff));
                    }
                });
            });
        }
        // One more pass so every family registered on the last iteration builds its atlas
        // (the #392 panic site) and lays out in its own face.
        let _ = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
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

    /// A document with one sketch (id 0) holding a rectangle's four lines. The sketch-scoped
    /// pickers refuse geometry that isn't in the open sketch (#953, `PickRule::InSketch`), so a
    /// test that picks line 0 needs line 0 to be in sketch 0.
    fn doc_with_a_sketch() -> Document {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc
    }

    /// A document holding `n` plain solid bodies. The body pickers refuse a body that isn't
    /// there (#953, `PickRule::LiveBody`), so a test that picks body 4 needs body 4 to exist.
    fn doc_with_bodies(n: usize) -> Document {
        let mut doc = Document::default();
        for _ in 0..n {
            doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Imported(0),
                material: None,
                name: None,
                deleted: false,
                shadow: false,
            });
        }
        doc
    }

    fn input<'a>(doc: &'a Document, selection: &'a SceneSelection) -> ContextInput<'a> {
        ContextInput {
            doc,
            selection,
            tool: Tool::Select,
            in_drawing_workbench: false,
            open_drawing: None,
            draw_rect_construction: None,
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            open_sketch: None,
            sketch_axis_screen_dirs: None,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_disjoint_profiles: false,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_edges: None,
            loft_sections: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            dimension_edit: None,
            treatment: None,
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
            open_drawing: None,
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
            open_drawing: None,
            drawing_view: Some(view_control.clone()),
            ..input(&doc, &selection)
        });
        assert!(dim.drawing_view.is_some(), "Dimension tool keeps the projection editor");
        assert!(dim.units.is_none(), "Dimension tool no longer shows units (#585)");
        // Text tool: both suppressed.
        let text = context_pane_content(&ContextInput {
            tool: Tool::Text,
            in_drawing_workbench: true,
            open_drawing: None,
            drawing_view: Some(view_control),
            ..input(&doc, &selection)
        });
        assert!(text.drawing_view.is_none(), "Text tool hides the projection editor (#329)");
        assert!(text.units.is_none(), "Text tool hides the Default-units section (#330)");
        // Joint tool: a joint has a busy section of its own, and its units are whatever its
        // parts' already are — nothing about it is the document's default (#998).
        let joint = context_pane_content(&ContextInput {
            tool: Tool::Joint,
            ..input(&doc, &selection)
        });
        assert!(joint.units.is_none(), "Joint tool hides the Default-units section (#998)");
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
            open_sketch: Some(0),
            sketch_axis_screen_dirs: None,
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
        use crate::hierarchy::SceneElement;
        let doc = doc_with_a_sketch();
        let selection = SceneSelection::default();
        let content = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            in_drawing_workbench: false,
            open_drawing: None,
            extrude_faces: Some(vec![
                crate::model::ExtrudeFace::Circle(0),
                crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3]),
            ]),
            ..input(&doc, &selection)
        });
        let picker = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::ExtrudeProfile)
            .expect("the Extrude face picker");
        assert_eq!(
            picker.picker.picked(),
            &[
                SceneElement::SketchFace(crate::model::FaceId::Circle(0)),
                SceneElement::SketchFace(crate::model::FaceId::Polygon(vec![0, 1, 2, 3])),
            ],
            "profiles keep their analytic-face identity (#955)"
        );
        assert!(picker.picker.is_focused(), "Extrude has only the one picker");
        assert!(!picker.picker.accepts(&doc, &SceneElement::Body(0)));
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
                target: None,
                target_focused: false,
                can_commit: true,
                has_extrusion: true,
            }),
            ..input(&doc, &selection)
        });
        let control = content.extrude.expect("extrude control present");
        assert_eq!(control.distance, "15 mm");
        assert!(control.target.is_none());
        assert!(control.can_commit);

        // Target-driven: a picked "Up to" target with the distance field nulled.
        let content = context_pane_content(&ContextInput {
            tool: Tool::Extrude,
            extrude: Some(ExtrudeControl {
                distance: String::new(),
                target: Some(crate::hierarchy::SceneElement::ConstructionPlane(2)),
                target_focused: false,
                can_commit: true,
                has_extrusion: true,
            }),
            ..input(&doc, &selection)
        });
        let control = content.extrude.expect("extrude control present");
        assert!(control.distance.is_empty(), "distance is null while a target drives the depth");
        assert_eq!(
            control.target,
            Some(crate::hierarchy::SceneElement::ConstructionPlane(2))
        );
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
            extrude_disjoint_profiles: false,
            extrude_faces: Some(Vec::new()),
            ..input(&doc, &selection)
        });
        let body = content.extrude_body.expect("Extrude-into control shows before a face");
        assert_eq!(body.mode, crate::actions::ExtrudeBodyMode::NewBody);
        assert!(body.merge_body.is_none(), "Add/Cut stay disabled with no host body");
    }

    /// #166/#955: the Chamfer/Fillet edge set is a real element picker, present (empty or
    /// not) whenever the tool is active.
    #[test]
    fn the_treatment_edge_set_is_an_element_picker() {
        use crate::hierarchy::SceneElement;
        let doc = Document::default();
        let selection = SceneSelection::default();
        let edge = crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        let base = ContextInput {
            tool: Tool::Chamfer,
            edge_treatment_edges: Some(vec![(0, edge)]),
            ..input(&doc, &selection)
        };
        let picker = |input: &ContextInput<'_>| {
            context_pane_content(input)
                .tool_pickers
                .into_iter()
                .find(|v| v.target == PickerTarget::TreatmentEdges)
        };
        let view = picker(&base).expect("the edge picker");
        assert_eq!(
            view.picker.picked(),
            &[SceneElement::ExtrusionEdge { extrusion: 0, edge }]
        );
        assert!(view.picker.accepts(
            &doc,
            &SceneElement::BodyEdge { body: 0, a: [0; 3], b: [1; 3] }
        ));
        assert!(!view.picker.accepts(&doc, &SceneElement::Body(0)));

        // An empty set still shows the picker — that's how you see what to pick.
        let empty = ContextInput { edge_treatment_edges: Some(Vec::new()), ..base };
        assert!(picker(&empty).expect("shown while empty").picker.is_empty());
        // With the tool inactive there is no picker at all.
        let off = ContextInput { edge_treatment_edges: None, ..empty };
        assert!(picker(&off).is_none());
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
            open_drawing: None,
            draw_rect_construction: None,
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            open_sketch: None,
            sketch_axis_screen_dirs: None,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_disjoint_profiles: false,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_edges: None,
            loft_sections: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            dimension_edit: None,
            treatment: None,
        };
        let picker = context_pane_content(&input)
            .selection_picker
            .expect("selection picker");
        // The picker *is* the selection (#966), so its rows are in pick order — Line first,
        // because that's what was clicked first. That order is stable across frames, which is
        // what the ✕ needed and what the debug-string sort used to fake.
        assert_eq!(
            picker.picked(),
            &[SceneElement::Line(0), SceneElement::Circle(1)]
        );
        assert!(picker.is_focused(), "the selection picker is the Select tool's only one");
        assert!(picker.accepts(&doc, &SceneElement::Body(0)), "Select accepts everything");

        // Empty selection: the picker is still shown (an always-present input), just empty.
        let empty_selection = SceneSelection::default();
        let empty = ContextInput { selection: &empty_selection, ..input };
        let empty_picker = context_pane_content(&empty)
            .selection_picker
            .expect("always-present select picker");
        assert!(empty_picker.is_empty());
    }

    #[test]
    fn constraint_tool_picker_filters_to_constrainable_geometry() {
        use crate::hierarchy::SceneElement;
        let doc = doc_with_a_sketch();
        let mut selection = SceneSelection::default();
        // A constrainable line plus a body (which the constraint picker should reject).
        crate::selection::click_scene_selection(&mut selection, SceneElement::Line(0), true);
        crate::selection::click_scene_selection(&mut selection, SceneElement::Body(3), true);
        let input = ContextInput {
            tool: Tool::Constraint,
            in_sketch: true,
            open_sketch: Some(0),
            sketch_axis_screen_dirs: None,
            in_drawing_workbench: false,
            open_drawing: None,
            ..input(&doc, &selection)
        };
        let picker = context_pane_content(&input)
            .selection_picker
            .expect("constraint picker");
        assert_eq!(picker.picked(), &[SceneElement::Line(0)], "body filtered out");
        assert!(picker.is_focused(), "active tool's picker is focused");
        assert!(!picker.accepts(&doc, &SceneElement::Body(0)));
    }

    #[test]
    fn revolve_cut_mode_yields_a_red_body_picker() {
        use crate::hierarchy::SceneElement;
        let doc = doc_with_bodies(8);
        let selection = SceneSelection::default();
        let cut_input = ContextInput {
            tool: Tool::Revolve,
            in_drawing_workbench: false,
            open_drawing: None,
            revolve: Some(RevolveControl {
                faces: vec![crate::model::ExtrudeFace::Circle(0)],
                axis: Some(crate::model::RevolveAxis::Y),
                axis_focused: false,
                symmetric: false,
                body_choice: crate::actions::RevolveBodyChoice::Cut,
                cut_bodies: vec![2, 5],
            }),
            ..input(&doc, &selection)
        };
        let content = context_pane_content(&cut_input);
        // Profile and Axis (#955) plus the cut bodies; find the one under test by its target
        // rather than by position.
        assert_eq!(content.tool_pickers.len(), 3);
        let view = content
            .tool_pickers
            .iter()
            .find(|v| v.target == PickerTarget::RevolveCut)
            .expect("the cut-bodies picker");
        assert_eq!(
            view.picker.picked(),
            &[SceneElement::Body(2), SceneElement::Body(5)]
        );
        // Body-only filter, and the red "cut" highlight override in place of the default.
        assert!(view.picker.accepts(&doc, &SceneElement::Body(0)));
        assert!(!view.picker.accepts(&doc, &SceneElement::Line(0)));
        assert_eq!(
            view.picker.selected_color(crate::theme::FOCUS_ACCENT),
            crate::theme::CUT_ACCENT
        );

        // Non-Cut mode shows no tool picker.
        let new_body_input = ContextInput {
            tool: Tool::Revolve,
            in_drawing_workbench: false,
            open_drawing: None,
            revolve: Some(RevolveControl {
                body_choice: crate::actions::RevolveBodyChoice::NewBody,
                faces: vec![crate::model::ExtrudeFace::Circle(0)],
                axis: None,
                axis_focused: false,
                symmetric: false,
                cut_bodies: vec![],
            }),
            ..input(&doc, &selection)
        };
        // Outside Cut mode the tool still shows its own two inputs, just no cut picker.
        let pickers = context_pane_content(&new_body_input).tool_pickers;
        assert!(
            !pickers.iter().any(|v| v.target == PickerTarget::RevolveCut),
            "no cut picker outside Cut mode"
        );
        assert_eq!(pickers.len(), 2, "the profile and axis pickers remain");
    }

    /// #834: the material picker shows for a body selection, reports what they share, and
    /// stays away when the selection isn't bodies.
    #[test]
    fn material_picker_follows_the_body_selection() {
        use crate::hierarchy::SceneElement;
        let mut doc = Document::default();
        // The document already carries the default palette (#928); Brass lands after it.
        let brass = doc.materials.len();
        doc.materials.push(crate::model::Material {
            name: "Brass".to_string(),
            color: [1, 2, 3],
            deleted: false,
        });
        for material in [Some(brass), None] {
            doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Extrusion(0),
                material,
                name: None,
                deleted: false,
                shadow: false,
            });
        }

        let mut selection = SceneSelection::default();
        assert!(context_pane_content(&input(&doc, &selection)).material.is_none());

        selection.insert(SceneElement::Body(0));
        let control = context_pane_content(&input(&doc, &selection)).material.unwrap();
        assert_eq!(control.bodies, vec![0]);
        assert_eq!(control.current, Some(Some(brass)));
        assert_eq!(
            control.materials.last(),
            Some(&(brass, "Brass".to_string(), [1, 2, 3]))
        );
        assert_eq!(
            control.materials.first().map(|(i, n, _)| (*i, n.clone())),
            Some((0, "Unobtainium".to_string())),
            "the whole palette is offered (#928)"
        );

        // Two bodies that disagree read as mixed — the second has no material of its own,
        // which reads as Unobtainium (#924).
        selection.insert(SceneElement::Body(1));
        let control = context_pane_content(&input(&doc, &selection)).material.unwrap();
        assert_eq!(control.current, None);

        // A body with no material of its own reads as Unobtainium, the first material
        // (#924) — the picker shows it selected, not a "Default" stand-in.
        let mut lone = SceneSelection::default();
        lone.insert(SceneElement::Body(1));
        let control = context_pane_content(&input(&doc, &lone)).material.unwrap();
        assert_eq!(control.current, Some(Some(crate::model::DEFAULT_MATERIAL)));

        // A non-body in the selection takes the picker away.
        selection.insert(SceneElement::Line(0));
        assert!(context_pane_content(&input(&doc, &selection)).material.is_none());

        // And so does running a tool (#934): the pane is that tool's controls then.
        let mut bodies = SceneSelection::default();
        bodies.insert(SceneElement::Body(0));
        let mut tool_input = input(&doc, &bodies);
        tool_input.tool = Tool::Move;
        assert!(context_pane_content(&tool_input).material.is_none());
    }

    #[test]
    fn move_and_repeat_yield_body_pickers_without_cut_override() {
        use crate::hierarchy::SceneElement;
        let doc = doc_with_bodies(8);
        let selection = SceneSelection::default();

        let move_input = ContextInput {
            tool: Tool::Move,
            in_drawing_workbench: false,
            open_drawing: None,
            move_op: Some(MoveControl {
                plane_targets: Vec::new(),
                image_targets: Vec::new(),
                angle_snap_deg: crate::actions::MAX_ANGLE_SNAP_DEG,
                translate_mode: crate::model::MoveTranslateMode::Free,
                bodies_focused: true,
                start_a: None,
                start_a_focused: false,
                end_a: None,
                end_a_focused: false,
                start_b: None,
                start_b_focused: false,
                end_b: None,
                end_b_focused: false,
                start_c: None,
                start_c_focused: false,
                end_c: None,
                end_c_focused: false,
                targets: vec![1, 4],
                tx: String::new(),
                ty: String::new(),
                tz: String::new(),
                editing: false,
                can_commit: true,
            }),
            ..input(&doc, &selection)
        };
        let pickers = context_pane_content(&move_input).tool_pickers;
        // Bodies plus the six point pickers (#958): they render inline among the tool's own
        // controls but are registered like every other picker, so find this one by target.
        assert_eq!(pickers.len(), 7, "Bodies plus the six point pickers");
        assert_eq!(
            pickers
                .iter()
                .filter(|v| v.render == PickerRender::Shared)
                .count(),
            1,
            "only Bodies draws in the shared block"
        );
        let pickers: Vec<&ToolPickerView> = pickers
            .iter()
            .filter(|v| v.target == PickerTarget::MoveTargets)
            .collect();
        assert_eq!(pickers[0].target, PickerTarget::MoveTargets);
        assert_eq!(
            pickers[0].picker.picked(),
            &[SceneElement::Body(1), SceneElement::Body(4)]
        );
        assert!(!pickers[0].picker.accepts(&doc, &SceneElement::Line(0)));
        // Move doesn't consume its bodies, so it keeps the default (non-red) highlight.
        assert_eq!(
            pickers[0].picker.selected_color(crate::theme::FOCUS_ACCENT),
            crate::theme::FOCUS_ACCENT
        );

        let repeat_input = ContextInput {
            tool: Tool::Repeat,
            in_drawing_workbench: false,
            open_drawing: None,
            repeat_op: Some(RepeatControl {
                around_axis: false,
                flip: false,
                can_turn_about_path: true,
                targets: vec![7],
                plane_targets: Vec::new(),
                sketch_targets: Vec::new(),
                extrusion_targets: Vec::new(),
                path: Some(crate::hierarchy::SceneElement::GlobalAxis(
                    crate::construction::GlobalAxis::X,
                )),
                value_field_focused: false,
                length_target: None,
                length_target_focused: false,
                length_target_value: None,
                mode: crate::model::RepeatMode::CountGap,
                count: "3".to_string(),
                spacing: String::new(),
                length: String::new(),
                computed_var: crate::model::RepeatVar::Distance,
                gap_is_offset: false,
                distance_is_end: true,
                computed_value: None,
                editing: false,
                can_commit: true,
            }),
            ..input(&doc, &selection)
        };
        let pickers = context_pane_content(&repeat_input).tool_pickers;
        // Bodies plus the inline "Path" and "Distance to" (#958).
        assert_eq!(pickers.len(), 3);
        assert_eq!(pickers[0].target, PickerTarget::RepeatTargets);
        assert_eq!(pickers[0].picker.picked(), &[SceneElement::Body(7)]);
    }

    /// #646: typing in the Repeat section's Count/Offset/Distance fields blurs the Bodies
    /// picker (and the Axis picker) — the focus ring belongs where the keyboard is.
    #[test]
    fn repeat_value_field_focus_blurs_the_pickers() {
        let doc = Document::default();
        let selection = SceneSelection::default();
        let control = |value_field_focused,
                       path: Option<crate::hierarchy::SceneElement>|
         -> RepeatControl {
            RepeatControl {
            around_axis: false,
            flip: false,
            can_turn_about_path: true,
            targets: vec![7],
            plane_targets: Vec::new(),
            sketch_targets: Vec::new(),
            extrusion_targets: Vec::new(),
            path,
            value_field_focused,
            length_target: None,
            length_target_focused: false,
            length_target_value: None,
            mode: crate::model::RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: String::new(),
            length: String::new(),
            computed_var: crate::model::RepeatVar::Distance,
            gap_is_offset: false,
            distance_is_end: true,
            computed_value: None,
            editing: false,
            can_commit: true,
            }
        };
        let x_axis = || {
            Some(crate::hierarchy::SceneElement::GlobalAxis(
                crate::construction::GlobalAxis::X,
            ))
        };
        let pane = |c: RepeatControl| {
            context_pane_content(&ContextInput {
                tool: Tool::Repeat,
                in_drawing_workbench: false,
                open_drawing: None,
                repeat_op: Some(c),
                ..input(&doc, &selection)
            })
        };
        // Axis already picked: the Bodies picker normally reads as focused…
        assert!(pane(control(false, x_axis())).tool_pickers[0].picker.is_focused());
        // …but not while a value field has the keyboard.
        assert!(!pane(control(true, x_axis())).tool_pickers[0].picker.is_focused());
        // With no axis yet, the Bodies picker defers to the Axis picker either way.
        assert!(!pane(control(false, None)).tool_pickers[0].picker.is_focused());
        assert!(!pane(control(true, None)).tool_pickers[0].picker.is_focused());
    }

    /// #991: every joint kind but Rigid joins exactly **two** parts, and which one moves is the
    /// whole meaning of the joint — so those are two named single-slot pickers, the **mobile**
    /// part first and the **fixed** one it is held against second. Rigid keeps the plain Parts
    /// list: it joins any number and nothing moves.
    #[test]
    fn a_two_sided_joint_picks_its_mobile_and_fixed_parts_separately() {
        use crate::model::{JointKind, JointRef};
        // A real body: the side pickers carry `PickRule::LiveBody`, so a part that isn't there
        // is refused — as it should be.
        let mut doc = Document::default();
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            material: None,
            name: None,
            deleted: false,
            shadow: false,
        });
        let doc = doc;
        let selection = SceneSelection::default();
        let control = |kind: JointKind,
                       mobile: Option<JointRef>,
                       fixed: Option<JointRef>|
         -> JointControl {
            JointControl {
                members: [mobile, fixed].into_iter().flatten().collect(),
                members_focused: true,
                mobile,
                fixed,
                mobile_focused: mobile.is_none(),
                fixed_focused: mobile.is_some() && fixed.is_none(),
                kind,
                base_label: String::new(),
                driven_bodies: Vec::new(),
                base_bodies: Vec::new(),
                moving_face: None, moving_face_focused: false,
                fixed_face: None, fixed_face_focused: false,
                flip: false,
                offset: String::new(),
                line_up: Vec::new(),
                position: String::new(),
                position2: String::new(),
                position3: String::new(),
                slide_min: String::new(),
                slide_max: String::new(),
                turn_min: String::new(),
                turn_max: String::new(),
                slide_min_stop: None,
                slide_min_stop_focused: false,
                slide_max_stop: None,
                slide_max_stop_focused: false,
                editing: false,
                can_commit: false,
                animate: true,
            }
        };
        let pickers = |c: JointControl| {
            context_pane_content(&ContextInput {
                tool: Tool::Joint,
                joint: Some(c),
                ..input(&doc, &selection)
            })
            .tool_pickers
        };

        // Rigid: one Parts list, and no side slots at all.
        let rigid = pickers(control(JointKind::Rigid, None, None));
        assert!(rigid.iter().any(|v| v.target == PickerTarget::JointMembers));
        assert!(
            !rigid.iter().any(|v| matches!(
                v.target,
                PickerTarget::JointMobile | PickerTarget::JointFixed
            )),
            "a rigid group has no moving side to name"
        );

        // A slider names both sides, mobile before fixed.
        let slider = pickers(control(JointKind::Slider, None, None));
        let mobile_at = slider.iter().position(|v| v.target == PickerTarget::JointMobile);
        let fixed_at = slider.iter().position(|v| v.target == PickerTarget::JointFixed);
        let (Some(mobile_at), Some(fixed_at)) = (mobile_at, fixed_at) else {
            panic!("a slider should offer both side pickers");
        };
        assert!(mobile_at < fixed_at, "the mobile part is picked first");
        assert!(
            slider[mobile_at].picker.limit().is_single()
                && slider[fixed_at].picker.limit().is_single(),
            "each side takes exactly one part"
        );

        // One ring at a time, and it steps mobile → fixed as they fill.
        assert!(slider[mobile_at].picker.is_focused() && !slider[fixed_at].picker.is_focused());
        let half = pickers(control(JointKind::Slider, Some(JointRef::Body(0)), None));
        let m = half.iter().find(|v| v.target == PickerTarget::JointMobile).unwrap();
        let f = half.iter().find(|v| v.target == PickerTarget::JointFixed).unwrap();
        assert_eq!(m.picker.picked().len(), 1, "the mobile slot holds its part");
        assert!(!m.picker.is_focused() && f.picker.is_focused(), "the ring moves on to Fixed");
    }

    #[test]
    fn no_tool_ever_shows_two_focused_pickers() {
        // #954: "only one EP can have focus at a time". The pickers are rebuilt each frame from
        // tool state, so the invariant is maintained by construction rather than by a stored
        // registry — which only holds if every tool's focus rules stay mutually exclusive.
        // Walk every tool and assert it.
        let doc = doc_with_bodies(4);
        let selection = SceneSelection::default();
        let mut seen = 0usize;
        for tool in Tool::ALL {
            for in_sketch in [false, true] {
                // Populate the active tool's control — a tool with no control contributes no
                // pickers, which would make this assert nothing. Gated by tool because that is
                // how the app builds them: `context_pane_content` trusts its caller to pass
                // only the active tool's controls, so the invariant is a property of the pair.
                let input = ContextInput {
                    tool,
                    in_sketch,
                    open_sketch: in_sketch.then_some(0),
                    slice_op: (tool == Tool::Slice).then_some(SliceControl {
                        targets: vec![1],
                        cutters: vec![crate::model::FaceId::ConstructionPlane(0)],
                        picking_cutter: false,
                        extend_infinite: true,
                        editing: false,
                        can_commit: true,
                    }),
                    revolve: (tool == Tool::Revolve).then_some(RevolveControl {
                        faces: vec![crate::model::ExtrudeFace::Circle(0)],
                        axis: None,
                        axis_focused: true,
                        symmetric: false,
                        body_choice: crate::actions::RevolveBodyChoice::Cut,
                        cut_bodies: vec![2],
                    }),
                    sweep: (tool == Tool::Sweep).then_some(SweepControl {
                        faces: vec![crate::model::ExtrudeFace::Circle(0)],
                        path: Vec::new(),
                        path_focused: true,
                        body_choice: crate::actions::RevolveBodyChoice::Cut,
                        cut_bodies: vec![2],
                    }),
                    extrude_faces: (tool == Tool::Extrude)
                        .then_some(vec![crate::model::ExtrudeFace::Circle(0)]),
                    loft_sections: (tool == Tool::Loft).then_some(vec![
                        crate::model::LoftSection {
                            sketch: 0,
                            face: crate::model::ExtrudeFace::Circle(0),
                        },
                        crate::model::LoftSection {
                            sketch: 0,
                            face: crate::model::ExtrudeFace::Circle(1),
                        },
                    ]),
                    loft_body: (tool == Tool::Loft).then_some(LoftBodyControl {
                        body_choice: crate::actions::RevolveBodyChoice::Cut,
                        cut_bodies: vec![2],
                        can_commit: true,
                    }),
                    plane_tool: (tool == Tool::ConstructionPlane).then_some(PlaneToolControl {
                        anchor_labels: vec!["Origin".to_string()],
                        anchor_elements: vec![crate::hierarchy::SceneElement::Origin],
                        normal_labels: Vec::new(),
                        normal_choice: 0,
                        has_anchor: true,
                        show_angle: false,
                        offset_text: String::new(),
                        angle_text: String::new(),
                        offset_focused: true,
                        angle_focused: false,
                    }),
                    ..input(&doc, &selection)
                };
                let focused: Vec<&'static str> = context_pane_content(&input)
                    .tool_pickers
                    .iter()
                    .filter(|v| v.picker.is_focused())
                    .map(|v| v.heading)
                    .collect();
                assert!(
                    focused.len() <= 1,
                    "{tool:?} (in_sketch={in_sketch}) focuses {focused:?}"
                );
                seen += context_pane_content(&input).tool_pickers.len();
            }
        }
        assert!(
            seen > 0,
            "the walk saw no pickers at all — the tool controls below must be populated for \
             this to be asserting anything"
        );
    }

    #[test]
    fn exactly_one_picker_wears_the_focus_ring() {
        // #962: arming a single-pick input must blur the tool's other picker — two focus rings
        // at once says two places will take the next click, and only one will.
        let doc = doc_with_a_sketch();
        let selection = SceneSelection::default();
        let extrude = |target_focused| ContextInput {
            tool: Tool::Extrude,
            extrude_faces: Some(vec![crate::model::ExtrudeFace::Circle(0)]),
            extrude: Some(ExtrudeControl {
                distance: "10".to_string(),
                target: None,
                target_focused,
                can_commit: true,
                has_extrusion: true,
            }),
            ..input(&doc, &selection)
        };
        let focused = |input: &ContextInput<'_>| {
            context_pane_content(input)
                .tool_pickers
                .iter()
                .filter(|v| v.picker.is_focused())
                .count()
        };
        assert_eq!(focused(&extrude(false)), 1, "the Faces picker alone");
        // Arming "Up to" blurs Faces — still exactly one ring, now on the armed input, which
        // is registered like every other picker (#958) even though it draws inline.
        assert_eq!(focused(&extrude(true)), 1);
        assert_eq!(
            context_pane_content(&extrude(true))
                .tool_pickers
                .iter()
                .find(|v| v.picker.is_focused())
                .map(|v| v.heading),
            Some("Up to")
        );
    }

    #[test]
    fn the_plane_tools_anchor_is_a_real_picker() {
        // #955: the Anchor input was the last label-only one. The plane's `reference` is a
        // derived frame, so the tool now keeps what was clicked and the picker holds it.
        use crate::hierarchy::SceneElement;
        let doc = doc_with_a_sketch();
        let selection = SceneSelection::default();
        let point = SceneElement::Origin;
        let line = SceneElement::Line(0);
        let plane_input = ContextInput {
            tool: Tool::ConstructionPlane,
            plane_tool: Some(PlaneToolControl {
                anchor_labels: vec!["Origin".to_string(), "Line 0".to_string()],
                anchor_elements: vec![point.clone(), line.clone()],
                normal_labels: Vec::new(),
                normal_choice: 0,
                has_anchor: true,
                show_angle: false,
                offset_text: String::new(),
                angle_text: String::new(),
                offset_focused: false,
                angle_focused: false,
            }),
            ..input(&doc, &selection)
        };
        let pickers = context_pane_content(&plane_input).tool_pickers;
        let anchor = pickers
            .iter()
            .find(|v| v.target == PickerTarget::PlaneAnchor)
            .expect("the plane tool registers its Anchor picker");
        assert_eq!(anchor.heading, "Anchor");
        assert!(anchor.picker.is_focused(), "the anchor is what the tool picks");
        assert_eq!(anchor.picker.picked(), &[point, line]);
        // Two, because a line and a point together fix one frame (#483).
        assert_eq!(anchor.picker.limit(), PickLimit::Finite(2));
        // It takes what a plane can be anchored on, and nothing else.
        let accepts = anchor.picker.filter().accepted_kinds();
        assert!(accepts.contains(&ElementKind::Face));
        assert!(accepts.contains(&ElementKind::Vertex));
        assert!(accepts.contains(&ElementKind::Edge));
        assert!(!accepts.contains(&ElementKind::Body));
    }

    #[test]
    fn revolve_yields_a_profile_picker_and_a_straight_axis_picker() {
        // #955: Revolve's Profile and Axis were label-only. The axis picker is the first real
        // consumer of both the world-axis element and the `Straight` rule (#952/#953).
        use crate::hierarchy::SceneElement;
        let doc = doc_with_a_sketch();
        let selection = SceneSelection::default();
        let revolve_input = ContextInput {
            tool: Tool::Revolve,
            revolve: Some(RevolveControl {
                faces: vec![crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3])],
                axis: Some(crate::model::RevolveAxis::Z),
                axis_focused: false,
                symmetric: false,
                body_choice: crate::actions::RevolveBodyChoice::NewBody,
                cut_bodies: Vec::new(),
            }),
            ..input(&doc, &selection)
        };
        let pickers = context_pane_content(&revolve_input).tool_pickers;
        assert_eq!(pickers.len(), 2, "profile and axis; no cut picker outside Cut mode");

        let profile = &pickers[0];
        assert_eq!(profile.target, PickerTarget::RevolveProfile);
        assert_eq!(
            profile.picker.picked(),
            &[SceneElement::SketchFace(crate::model::FaceId::Polygon(vec![
                0, 1, 2, 3
            ]))],
            "the profile keeps its analytic-face identity"
        );

        let axis = &pickers[1];
        assert_eq!(axis.target, PickerTarget::RevolveAxis);
        assert_eq!(
            axis.picker.picked(),
            &[SceneElement::GlobalAxis(crate::construction::GlobalAxis::Z)],
            "the world Z axis is an element now"
        );
        assert!(matches!(axis.picker.limit(), PickLimit::Finite(1)), "one axis");
        assert!(
            axis.picker.accepts(&doc, &SceneElement::Line(0)),
            "a straight sketch line is a valid axis"
        );
        assert!(
            !axis.picker.accepts(&doc, &SceneElement::Circle(0)),
            "a revolve axis has to be straight"
        );
        assert!(
            !axis.picker.accepts(&doc, &SceneElement::Body(0)),
            "a whole body is not an axis"
        );
    }

    #[test]
    fn slice_yields_a_body_picker_and_a_red_cutter_picker() {
        // #955: Slice's Targets and Cutters were label-only, so neither had a filter, a focus,
        // or the red highlight SPEC names Slice cutters as *the* example of.
        use crate::hierarchy::SceneElement;
        let doc = doc_with_bodies(4);
        let selection = SceneSelection::default();
        let slice_input = ContextInput {
            tool: Tool::Slice,
            slice_op: Some(SliceControl {
                targets: vec![1],
                cutters: vec![crate::model::FaceId::ConstructionPlane(0)],
                picking_cutter: true,
                extend_infinite: true,
                editing: false,
                can_commit: true,
            }),
            ..input(&doc, &selection)
        };
        let pickers = context_pane_content(&slice_input).tool_pickers;
        assert_eq!(pickers.len(), 2, "targets and cutters");

        let targets = &pickers[0];
        assert_eq!(targets.target, PickerTarget::SliceTargets);
        assert_eq!(targets.picker.picked(), &[SceneElement::Body(1)]);
        assert!(targets.picker.accepts(&doc, &SceneElement::Body(0)));
        assert!(!targets.picker.is_focused(), "the cutter picker has focus");

        let cutters = &pickers[1];
        assert_eq!(cutters.target, PickerTarget::SliceCutters);
        assert_eq!(
            cutters.picker.picked(),
            &[SceneElement::ConstructionPlane(0)],
            "a plane cutter keeps its plane identity"
        );
        assert!(cutters.picker.is_focused());
        assert!(
            !cutters.picker.accepts(&doc, &SceneElement::Body(0)),
            "a whole body is not a cutter"
        );
        assert_eq!(
            cutters.picker.selected_color(crate::theme::FOCUS_ACCENT),
            crate::theme::CUT_ACCENT,
            "cutters are consumed destructively, so they read red"
        );
    }

    #[test]
    fn combine_shows_one_or_two_body_pickers_by_kind() {
        use crate::hierarchy::SceneElement;
        let doc = doc_with_bodies(8);
        let selection = SceneSelection::default();
        let make = |kind, a: Vec<usize>, b: Vec<usize>, picking_b| ContextInput {
            tool: Tool::Combine,
            in_drawing_workbench: false,
            open_drawing: None,
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
    fn help_text_is_keyed_by_tool_so_a_shared_label_reads_correctly() {
        // "Bodies" means different things to Move and to Combine.
        let move_bodies = row_help(Some(Tool::Move), "Bodies").unwrap();
        let combine_bodies = row_help(Some(Tool::Combine), "Bodies").unwrap();
        assert_ne!(move_bodies, combine_bodies);
        assert!(move_bodies.contains("move"), "{move_bodies}");
        assert!(combine_bodies.contains("fuse"), "{combine_bodies}");
    }

    #[test]
    fn help_text_falls_back_to_rows_that_mean_the_same_everywhere() {
        // The default-units rows belong to no tool in particular.
        for tool in [None, Some(Tool::Chamfer), Some(Tool::Move)] {
            assert!(row_help(tool, "Length").is_some());
            assert!(row_help(tool, "Angle").is_some());
        }
    }

    #[test]
    fn rows_without_help_text_get_no_note() {
        assert_eq!(row_help(Some(Tool::Move), "Nonexistent row"), None);
        assert_eq!(row_help(None, "Bodies"), None);
    }

    #[test]
    fn edge_treatment_row_labels_name_the_extrusion_and_edge() {
        // The wording moved into `scene_element_label` with the picker (#955), so the row and
        // any other place that names the edge read the same.
        use crate::hierarchy::SceneElement;
        let doc = Document::default();
        assert_eq!(
            crate::names::scene_element_label(
                &doc,
                &SceneElement::ExtrusionEdge {
                    extrusion: 3,
                    edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 2 },
                }
            ),
            "Extrusion 3 — vertical 2"
        );
        assert_eq!(
            crate::names::scene_element_label(
                &doc,
                &SceneElement::ExtrusionEdge {
                    extrusion: 0,
                    edge: crate::model::ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: true },
                }
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
                unit_instance: None,
                dimension_derive: None,
            dimension_edit: None,
            treatment: None,
                name: None,
                curve_mode: None,
            rect_anchor: None,
            circle_anchor: None,
                tangent_constraint: None,
                construction: None,
                constraints: None,
                constraint_axis_dirs: None,
                snapping: None,
                extrude_body: None,
                extrude: None,
                selection_picker: Some(ElementPicker::select_everything()),
                tool_pickers: vec![ToolPickerView {
                    heading: "Selection",
                    picker: ElementPicker::select_everything(),
                    target: PickerTarget::Selection,
                    separator_above: true,
                    render: PickerRender::Inline,
                }],
                calibrate_image: None,
                revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
                material: None,
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
            open_drawing: None,
            draw_rect_construction: Some(true),
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            open_sketch: None,
            sketch_axis_screen_dirs: None,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_disjoint_profiles: false,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_edges: None,
            loft_sections: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            dimension_edit: None,
            treatment: None,
        });
        assert_eq!(
            content,
            ContextPaneContent {
                tool_title: None,
                unit_instance: None,
                dimension_derive: None,
            dimension_edit: None,
            treatment: None,
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
                constraint_axis_dirs: None,
                snapping: None,
                extrude_body: None,
                extrude: None,
                // A draw tool outside a sketch picks the face to sketch on, so it has a
                // picker like every other tool (#958).
                selection_picker: Some(ElementPicker::select_everything()),
            tool_pickers: vec![ToolPickerView {
                heading: "Selection",
                picker: ElementPicker::select_everything(),
                target: PickerTarget::Selection,
                separator_above: true,
                render: PickerRender::Inline,
            }],
                calibrate_image: None,
                revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
                material: None,
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
            open_drawing: None,
            draw_rect_construction: None,
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: Some(false),
            draw_circle_construction: None,
            draw_line_curve_mode: Some(true),
            draw_line_tangent_constraint: Some(false),
            in_sketch: true,
            open_sketch: Some(0),
            sketch_axis_screen_dirs: None,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_disjoint_profiles: false,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_edges: None,
            loft_sections: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            dimension_edit: None,
            treatment: None,
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
                unit_instance: None,
                dimension_derive: None,
            dimension_edit: None,
            treatment: None,
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
                constraint_axis_dirs: None,
                snapping: None,
                extrude_body: None,
                extrude: None,
                // #213: the Select tool surfaces the selection through the unified element picker.
                selection_picker: Some({
                    let mut p = ElementPicker::select_everything();
                    p.set_picked(&doc, [SceneElement::Line(0)]);
                    p
                }),
                tool_pickers: vec![ToolPickerView {
                    heading: "Selection",
                    picker: {
                        let mut p = ElementPicker::select_everything();
                        p.set_picked(&doc, [SceneElement::Line(0)]);
                        p
                    },
                    target: PickerTarget::Selection,
                    separator_above: true,
                    render: PickerRender::Inline,
                }],
                calibrate_image: None,
                revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            material: None,
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
            open_drawing: None,
            draw_rect_construction: Some(false),
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            open_sketch: None,
            sketch_axis_screen_dirs: None,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_disjoint_profiles: false,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_edges: None,
            loft_sections: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            dimension_edit: None,
            treatment: None,
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
            open_drawing: None,
            draw_rect_construction: Some(true),
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            open_sketch: None,
            sketch_axis_screen_dirs: None,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_disjoint_profiles: false,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_edges: None,
            loft_sections: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            dimension_edit: None,
            treatment: None,
        });
        assert_eq!(
            content,
            ContextPaneContent {
                tool_title: None,
                unit_instance: None,
                dimension_derive: None,
            dimension_edit: None,
            treatment: None,
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
                constraint_axis_dirs: None,
                snapping: None,
                extrude_body: None,
                extrude: None,
                // A draw tool outside a sketch picks the face to sketch on, so it has a
                // picker like every other tool (#958).
                selection_picker: Some({
                    let mut p = ElementPicker::select_everything();
                    p.set_picked(&doc, [SceneElement::Line(0)]);
                    p
                }),
            tool_pickers: vec![ToolPickerView {
                heading: "Selection",
                picker: {
                    let mut p = ElementPicker::select_everything();
                    p.set_picked(&doc, [SceneElement::Line(0)]);
                    p
                },
                target: PickerTarget::Selection,
                separator_above: true,
                render: PickerRender::Inline,
            }],
                calibrate_image: None,
                revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            material: None,
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
            open_drawing: None,
            draw_rect_construction: None,
            rect_anchor: None,
            circle_anchor: None,
            draw_line_construction: None,
            draw_circle_construction: None,
            draw_line_curve_mode: None,
            draw_line_tangent_constraint: None,
            in_sketch: false,
            open_sketch: None,
            sketch_axis_screen_dirs: None,
            snapping_enabled: true,
            extrude_merge_candidate: None,
            extrude_disjoint_profiles: false,
            extrude_body_mode: None,
            extrude_symmetric: None,
            extrude_faces: None,
            extrude: None,
            edge_treatment_edges: None,
            loft_sections: None,
            calibrate_image: None,
            revolve: None,
            sweep: None,
            plane_tool: None,
            loft_body: None,
            boolean_op: None,
            boolean_edit_start: None,
            move_op: None,
            move_edit_start: None,
            shape: None,
            joint: None,
            joint_edit_start: None,
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
            dimension_edit: None,
            treatment: None,
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
            extrude_disjoint_profiles: false,
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