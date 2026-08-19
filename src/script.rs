//! Lua script runner and internal instruction dispatch (SPEC §8).
//!
//! Scripts are `.lua` files that call the global `bearcad` API. They drive the
//! live UI via synthetic pointer/keyboard events and headless actions.

use crate::actions::{
    dim_label_target_in_sketch, Action, ActionResult, AppState, DimLabelAxis, Pane, RectAxis,
    Tool,
};
use crate::command_palette::{best_match, commands_for_state, PaletteOutcome};
use crate::constraints::apply_dimension_expression;
use crate::hierarchy::SceneElement;
use crate::model::{
    ConstraintLine, ConstraintPoint, DistanceTarget, ExtrudeFace, FaceId,
    VertexTreatmentKind,
};
use crate::value::{AngleUnit, LengthUnit};

use crate::construction::PlaneDim;
use crate::camera::{GroundDisplay, ProjectionMode, ShadingMode, StandardView};
use crate::view_cube::{CubeCornerId, CubeEdgeId};

#[cfg(not(target_arch = "wasm32"))]
use crate::lua_script::{load_script, ScriptTickData};
use eframe::egui::{self, Key, Modifiers, PointerButton};
use glam::Vec3;
#[cfg(not(target_arch = "wasm32"))]
use mlua::Lua;
use std::path::Path;
use crate::time::Instant;
use std::time::Duration;

/// What part of the window a scripted screenshot captures.
///
/// Panes are capturable so documentation can show one pane on its own (#672) — a
/// whole-window shot of, say, the Context pane is mostly viewport, and cropping it
/// afterwards would need the pane's position, which only the running app knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScreenshotRegion {
    /// The 3D viewport only, with the view-cube HUD suppressed for that frame.
    #[default]
    Viewport,
    /// The entire window, panes and toolbar included.
    Window,
    /// A single pane. Captures nothing if that pane is hidden.
    Pane(crate::actions::Pane),
    /// The Settings window (#737), help notes included when help mode is on.
    Settings,
}

impl ScreenshotRegion {
    /// Parse a region name as written in a script: `"viewport"`, `"window"`, or any
    /// pane name [`crate::actions::Pane::from_name`] accepts (`"context"`, …).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "viewport" | "view" | "3d" => Some(Self::Viewport),
            "window" | "whole" | "whole_window" | "all" => Some(Self::Window),
            "settings" => Some(Self::Settings),
            other => crate::actions::Pane::from_name(other).map(Self::Pane),
        }
    }

    /// The name this region is written as in a script.
    pub fn script_name(self) -> &'static str {
        match self {
            Self::Viewport => "viewport",
            Self::Window => "window",
            Self::Settings => "settings",
            Self::Pane(pane) => pane.script_name(),
        }
    }
}

/// Modifier keys a scripted click holds down (#835/#984). **Shift** is the modifier several
/// tools read for their second role (multi-select, the in-sketch repeat direction);
/// **Control** narrows an edge pick to the one edge under the cursor rather than its whole
/// tangent-continuous run.
///
/// **Cmd** is the platform primary modifier (⌘ on macOS, Ctrl elsewhere). It is spelled out
/// separately because a scripted `ctrl` stays literally Ctrl — egui's `command` field follows
/// Mac's Cmd, not Ctrl (#984) — so the copy/paste shortcuts (which read `Modifiers::COMMAND`)
/// need a dedicated option (#1408).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClickMods {
    pub shift: bool,
    pub ctrl: bool,
    pub cmd: bool,
}

impl ClickMods {
    /// The Lua options table this reads back as, e.g. `, { ctrl = true }` — empty when no
    /// modifier is held, so a plain click round-trips as a plain call.
    fn lua_opts(self) -> String {
        let mut parts = Vec::new();
        if self.shift {
            parts.push("shift = true");
        }
        if self.ctrl {
            parts.push("ctrl = true");
        }
        if self.cmd {
            parts.push("cmd = true");
        }
        match parts.is_empty() {
            true => String::new(),
            false => format!(", {{ {} }}", parts.join(", ")),
        }
    }

    fn egui(self) -> Modifiers {
        Modifiers {
            shift: self.shift,
            ctrl: self.ctrl,
            // #1408: the `command` flag is the platform primary (⌘ on macOS, Ctrl
            // elsewhere) that the copy/paste shortcuts match on. Set separately from
            // `ctrl` so a scripted cmd reads as `command` — a scripted Ctrl stays Ctrl and
            // never doubles as the primary (#984).
            command: self.cmd,
            ..Modifiers::NONE
        }
    }
}

/// Script-level host of a treatable edge (#1329): an extrusion or Shape-tool primitive,
/// named by its live ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreatableSolidRef {
    Extrusion(usize),
    Primitive(usize),
}

/// A single script instruction.
#[derive(Clone, Debug, PartialEq)]
pub enum Instruction {
    // Document / tool actions
    New,
    Open(String),
    Save(Option<String>),
    /// Discard persisted tessellation and rebuild (SPEC §4.4 / #1343).
    RebuildGeometry,
    /// Export bodies to an STL file at `path`; `body` names a single body (`None` = all).
    ExportStl { path: String, body: Option<String> },
    /// Export bodies to a 3MF package at `path`; `body` names a single body (`None` = all) (#1284).
    Export3mf { path: String, body: Option<String> },
    /// Export bodies to a STEP file at `path`; `body` names a single body (`None` = all).
    ExportStep { path: String, body: Option<String> },
    /// Write a Home zoom-to-fit PNG preview of the document (#1223).
    ExportPreview { path: String },
    /// Import an STL file at `path` as a new body (#70).
    ImportStl { path: String },
    /// Import another BearCAD document as a unit with a first instance (#721).
    ImportUnit {
        path: String,
        link: Option<crate::model::LinkMode>,
        name: Option<String>,
    },
    /// Import a tracing image (#169).
    ImportImage { path: String, plane: Option<usize> },
    /// Calibrate a tracing image's scale (#171).
    /// Move one calibration reference point (#424), plane-local mm.
    SetCalibrationPoint {
        image: usize,
        index: usize,
        x: f32,
        y: f32,
    },
    /// Delete one calibration reference point (#424).
    RemoveCalibrationPoint { image: usize, index: usize },
    CalibrateImage {
        image: usize,
        a: Option<(f32, f32)>,
        b: Option<(f32, f32)>,
        length: f32,
        expression: String,
    },
    /// Set a tracing image's draw opacity (#1548).
    SetImageOpacity {
        image: usize,
        opacity: f32,
        expression: String,
    },
    /// Import a STEP file at `path` as a new body (#71).
    ImportStep { path: String },
    /// Import a document Lua script (#1160): run the file against the live document.
    /// Refuses a non-blank document unless `force` is true (GUI warns interactively).
    ImportLua { path: String, force: bool },
    Clear,
    Undo,
    /// Copy the selection onto the session clipboard (#1236).
    CopySelection,
    /// Paste clipboard contents at an explicit offset (#1236). `linked` is Paste Linked.
    PasteAt {
        linked: bool,
        x: f32,
        y: f32,
        z: f32,
    },
    Tool(Tool),
    BeginSketch { face: FaceId },
    OpenSketch { sketch: usize },
    ExitSketch,
    /// Create a rectangle directly in the active sketch (face-local mm) with locked dimensions.
    /// `width_expr`/`height_expr` (#402) lock the dimension to a parameter expression instead
    /// of the plain number — when set, they win over `width`/`height`.
    CreateRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        width_expr: Option<String>,
        height_expr: Option<String>,
    },
    /// Create a line directly in the active sketch (face-local mm endpoints). Like a
    /// click-drawn line it is unconstrained; `dimension` (an expression, e.g. "50" or "leg")
    /// locks its length the way typing a length while drawing does.
    /// `bezier` (#54) makes it a curve: `[handle near (x0,y0), handle near (x1,y1)]`.
    CreateLine {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        bezier: Option<[(f32, f32); 2]>,
        dimension: Option<String>,
    },
    /// Create a circle directly in the active sketch (face-local mm) with a locked diameter.
    /// `diameter_expr` (#402) locks the diameter to a parameter expression; it wins over `r`.
    CreateCircle {
        cx: f32,
        cy: f32,
        r: f32,
        diameter_expr: Option<String>,
    },
    /// Place a text element in the active sketch (#282/#286): glyph outlines baked from a
    /// system font, the same as the Text tool. `size` is an expression (parameters work).
    CreateSketchText {
        text: String,
        /// Font family; `None` picks the same default the Text tool uses.
        font: Option<String>,
        bold: bool,
        italic: bool,
        underline: bool,
        size: String,
        x: f32,
        y: f32,
        rotation_deg: f32,
        wrap: Option<f32>,
        /// Mirror the glyphs about the box's vertical centre (#1571).
        flip: bool,
    },
    /// Project outside geometry into the active sketch (`bearcad.project{ ... }`, #1351).
    /// Empty `elements` means the current scene selection (including un-project).
    Project { elements: Vec<SceneElement> },
    /// Extrude coplanar sketch faces into a solid.
    Extrude {
        sketch: usize,
        faces: Vec<crate::model::ExtrudeFace>,
        distance: f32,
        /// How the extrusion attaches to bodies (#32/#35): new body, add to the extruded
        /// face's body, or cut it from that body.
        body: crate::actions::ExtrudeBodyChoice,
        /// Extrude up to this object's extended plane instead of the fixed distance —
        /// the scripted "pull the gizmo and snap to a surface" (#114).
        target: Option<crate::model::ExtrudeTarget>,
        /// Distance as a parameter expression (#402): wins over `distance` and is stored
        /// on the extrusion so it re-bakes when parameters change.
        expression: Option<String>,
        /// Extrude half the distance each way from the sketch plane (#504).
        symmetric: bool,
        /// End-face size change vs the start face (#1243); units depend on `taper_mode`.
        taper: f32,
        /// Whether `taper` is a length (mm per side) or a draft angle in degrees (#1243).
        taper_mode: crate::model::ExtrudeTaperMode,
        /// Optional expression driving `taper` (#1243).
        taper_expression: Option<String>,
    },
    /// Scripted push/pull of a bare body face (#130/#122): the declarative equivalent of
    /// clicking the face with the Extrude tool and pulling it (optionally onto `target`).
    ExtrudeBodyFace {
        face: FaceId,
        distance: f32,
        body: crate::actions::ExtrudeBodyChoice,
        target: Option<crate::model::ExtrudeTarget>,
    },
    /// Semantic push/pull of an existing extrusion (#114): a new fixed distance
    /// (clearing any snap target) and/or a new snap target. `expression` (#402) sets a
    /// parameter-expression distance, winning over `distance`.
    UpdateExtrusion {
        extrusion: usize,
        distance: Option<f32>,
        target: Option<crate::model::ExtrudeTarget>,
        expression: Option<String>,
    },
    /// Loft a solid through two or more closed cross-section profiles (SPEC §3.5).
    /// Each face's owning sketch is inferred at execution time, like `bearcad.extrude`.
    Loft {
        faces: Vec<crate::model::ExtrudeFace>,
        body: crate::actions::RevolveBodyChoice,
        bodies: Vec<usize>,
    },
    /// Create a technical drawing (#180), optionally named.
    CreateDrawing { name: Option<String> },
    /// Set a drawing's page size and margin, in millimetres (#273/#406). `None` keeps the
    /// drawing's current value.
    SetDrawingPage {
        drawing: usize,
        width_mm: Option<f32>,
        height_mm: Option<f32>,
        margin_mm: Option<f32>,
    },
    /// Export a technical drawing to a vector SVG file.
    ExportDrawingSvg { drawing: usize, path: String },
    /// Export a technical drawing to a single-page vector PDF file.
    ExportDrawingPdf { drawing: usize, path: String },
    /// Add a body (or multi-body) view in an orientation to a drawing (#1190/#1191).
    AddDrawingView {
        drawing: usize,
        bodies: Vec<usize>,
        orientation: crate::model::DrawingOrientation,
    },
    /// Append bodies to an existing body projection (#1191).
    AddBodiesToDrawingView {
        drawing: usize,
        view: usize,
        bodies: Vec<usize>,
    },
    /// Add a sketch projection to a drawing (#278/#403) — `bearcad.drawing_view{ sketch = i }`.
    AddDrawingSketchView {
        drawing: usize,
        sketch: usize,
        orientation: crate::model::DrawingOrientation,
    },
    /// Add a free text annotation to a drawing page (#312).
    AddDrawingAnnotation {
        drawing: usize,
        text: String,
        x: f32,
        y: f32,
        wrap: Option<f32>,
    },
    /// Add an aligned child projection (#296): parent view index, direction, free-axis pos.
    AddAlignedDrawingView {
        drawing: usize,
        parent: usize,
        dir: crate::model::AlignDir,
        pos: f32,
    },
    /// Move a placed view to a page position (fractions 0..1) (#297/#309).
    MoveDrawingView {
        drawing: usize,
        view: usize,
        x: f32,
        y: f32,
    },
    /// Resize a placed view's card (page fractions) (#1207).
    SetDrawingViewSize {
        drawing: usize,
        view: usize,
        size_x: f32,
        size_y: f32,
    },
    /// Toggle the length dimension of a view's edge, named by its two world endpoints.
    ToggleDrawingDimension {
        drawing: usize,
        view: usize,
        a: (f32, f32, f32),
        b: (f32, f32, f32),
    },
    /// Toggle a detected circle's diameter dimension, named by its world centre (#373).
    ToggleDrawingCircleDimension {
        drawing: usize,
        view: usize,
        center: (f32, f32, f32),
    },
    /// Set (or clear) a drawing edge dimension's label offset (#294/#1228).
    /// `offset = None` restores the auto-placed default.
    SetDrawingDimensionOffset {
        drawing: usize,
        view: usize,
        a: (f32, f32, f32),
        b: (f32, f32, f32),
        offset: Option<f32>,
    },
    /// Set (or clear) a drawing circle Ø-label offset (#397/#1228).
    SetDrawingCircleDimOffset {
        drawing: usize,
        view: usize,
        center: (f32, f32, f32),
        offset: Option<f32>,
    },
    /// Show/hide an aligned child's dashed projection lines to its base view (#377).
    SetDrawingViewAlignLines {
        drawing: usize,
        view: usize,
        show: bool,
    },
    /// Edit a view's caption label (#372): each `Some` overrides that aspect; an empty
    /// `text` returns to the automatic caption.
    SetDrawingViewLabel {
        drawing: usize,
        view: usize,
        hidden: Option<bool>,
        pos: Option<String>,
        text: Option<String>,
    },
    /// Toggle the angle dimension between two of a view's edges, each named by its endpoints.
    ToggleDrawingAngle {
        drawing: usize,
        view: usize,
        edge1: ((f32, f32, f32), (f32, f32, f32)),
        edge2: ((f32, f32, f32), (f32, f32, f32)),
    },
    /// Place a primitive shape (#909): a cuboid, cylinder, or sphere, straight into 3D.
    Shape { shape: crate::model::Primitive },
    /// Re-point an existing shape (#909).
    EditShape {
        index: usize,
        shape: crate::model::Primitive,
    },
    /// Revolve profiles around an axis (SPEC §3.5 Revolve). Sketch inferred per face.
    /// `pitch_mm` is helical pitch (mm per full turn); 0 is a pure revolve (#1242).
    Revolve {
        faces: Vec<crate::model::ExtrudeFace>,
        axis: crate::model::RevolveAxis,
        angle_deg: f32,
        angle_expression: String,
        angle_is_revolutions: bool,
        pitch_mm: f32,
        pitch_expression: String,
        symmetric: bool,
        body: crate::actions::RevolveBodyChoice,
        bodies: Vec<usize>,
    },
    /// Sweep profiles along a path of sketch lines (SPEC §3.5 Sweep). Sketch
    /// inferred per face.
    Sweep {
        faces: Vec<crate::model::ExtrudeFace>,
        path: Vec<crate::model::LineKey>,
        body: crate::actions::RevolveBodyChoice,
        bodies: Vec<usize>,
    },
    /// Boolean operation between whole bodies (the Combine tool).
    CreateBooleanOp {
        kind: crate::model::BooleanOpKind,
        a: Vec<usize>,
        b: Vec<usize>,
        keep_b: bool,
    },
    /// Arm the Combine tool with picked sides **without committing** them, so the tool's
    /// live result preview (#1033) can be driven from a script. `bearcad.combine` is the
    /// committing counterpart.
    BeginBooleanOp {
        kind: crate::model::BooleanOpKind,
        a: Vec<usize>,
        b: Vec<usize>,
        keep_b: bool,
    },
    /// Re-point an existing boolean operation.
    EditBooleanOp {
        op: usize,
        kind: crate::model::BooleanOpKind,
        a: Vec<usize>,
        b: Vec<usize>,
        keep_b: bool,
    },
    /// Move bodies (Move tool): translation + optional rotation, expressions allowed.
    CreateMoveOp {
        targets: Vec<usize>,
        /// Tracing images to slide on their host plane (#217/#1587).
        images: Vec<usize>,
        tx: String,
        ty: String,
        tz: String,
        /// Free-mode turns about the world X/Y/Z axes (#1076), degree expressions.
        rx: String,
        ry: String,
        rz: String,
        /// Point Snap's third pair set as an angle instead of a target point (#1078).
        roll_angle: String,
        /// Face Snap's side flip, its turn about the target normal (#1077), and its gap
        /// off that face (#1079).
        face_flip: bool,
        face_spin: String,
        face_offset: String,
        /// Snap-translate points (#649/#650): with both set the move snaps `source` onto
        /// `target` and the tx/ty/tz expressions are ignored.
        start_point_a: Option<crate::model::MovePointRef>,
        end_point_a: Option<crate::model::MovePointRef>,
        /// The optional B pair (#669), which adds the rotation.
        start_point_b: Option<crate::model::MovePointRef>,
        end_point_b: Option<crate::model::MovePointRef>,
        /// The optional C pair, which pins the spin B leaves free.
        start_point_c: Option<crate::model::MovePointRef>,
        end_point_c: Option<crate::model::MovePointRef>,
    },
    /// Re-point an existing move operation.
    EditMoveOp {
        op: usize,
        targets: Vec<usize>,
        /// Tracing images to slide on their host plane (#217/#1587).
        images: Vec<usize>,
        tx: String,
        ty: String,
        tz: String,
        /// Free-mode turns about the world X/Y/Z axes (#1076), degree expressions.
        rx: String,
        ry: String,
        rz: String,
        /// Point Snap's third pair set as an angle instead of a target point (#1078).
        roll_angle: String,
        /// Face Snap's side flip, its turn about the target normal (#1077), and its gap
        /// off that face (#1079).
        face_flip: bool,
        face_spin: String,
        face_offset: String,
        start_point_a: Option<crate::model::MovePointRef>,
        end_point_a: Option<crate::model::MovePointRef>,
        /// The optional B pair (#669), which adds the rotation.
        start_point_b: Option<crate::model::MovePointRef>,
        end_point_b: Option<crate::model::MovePointRef>,
        /// The optional C pair, which pins the spin B leaves free.
        start_point_c: Option<crate::model::MovePointRef>,
        end_point_c: Option<crate::model::MovePointRef>,
    },
    /// Arm the Move tool with a set of picks **without committing** them, so the tool's
    /// live preview — the destination ghost, the A pair's connector, the B and C paths —
    /// can be driven from a script. `bearcad.move_bodies` is the committing counterpart.
    BeginMoveOp {
        targets: Vec<usize>,
        /// Tracing images to slide on their host plane (#217/#1587).
        images: Vec<usize>,
        tx: String,
        ty: String,
        tz: String,
        /// Free-mode turns about the world X/Y/Z axes (#1076), degree expressions.
        rx: String,
        ry: String,
        rz: String,
        /// Point Snap's third pair set as an angle instead of a target point (#1078).
        roll_angle: String,
        /// Face Snap's side flip, its turn about the target normal (#1077), and its gap
        /// off that face (#1079).
        face_flip: bool,
        face_spin: String,
        face_offset: String,
        start_point_a: Option<crate::model::MovePointRef>,
        end_point_a: Option<crate::model::MovePointRef>,
        start_point_b: Option<crate::model::MovePointRef>,
        end_point_b: Option<crate::model::MovePointRef>,
        start_point_c: Option<crate::model::MovePointRef>,
        end_point_c: Option<crate::model::MovePointRef>,
    },
    /// Join parts with a kinematic relationship (Joint tool, #891/#894).
    CreateJointOp {
        members: Vec<crate::model::JointRef>,
        base: usize,
        kind: crate::model::JointKind,
        /// Where the parts start out (#1021/#1079): an ordinary move.
        placement: crate::model::MoveOperation,
        /// How the joint's freedoms are oriented (#1079).
        frame: crate::model::JointFrame,
        position: String,
        position2: String,
        position3: String,
        limits: crate::model::JointLimits,
    },
    /// Re-point an existing joint.
    EditJointOp {
        op: usize,
        members: Vec<crate::model::JointRef>,
        base: usize,
        kind: crate::model::JointKind,
        /// Where the parts start out (#1021/#1079): an ordinary move.
        placement: crate::model::MoveOperation,
        /// How the joint's freedoms are oriented (#1079).
        frame: crate::model::JointFrame,
        position: String,
        position2: String,
        position3: String,
        limits: crate::model::JointLimits,
    },
    /// Arm the Joint tool with a set of picks **without committing** them, so the tool's
    /// live preview can be driven from a script — `bearcad.joint` is the committing
    /// counterpart, exactly as `begin_move` is to `move_bodies`.
    BeginJointOp {
        members: Vec<crate::model::JointRef>,
        base: usize,
        kind: crate::model::JointKind,
        /// Where the parts start out (#1021/#1079): an ordinary move.
        placement: crate::model::MoveOperation,
        /// How the joint's freedoms are oriented (#1079).
        frame: crate::model::JointFrame,
        position: String,
        position2: String,
        position3: String,
        limits: crate::model::JointLimits,
    },
    /// Capture a joint's current position as its rest pose (#898).
    SetJointRest { op: usize },
    /// Put a joint back to its rest pose (#898).
    RevertJoint { op: usize },
    /// Put every joint back to its rest pose (#898).
    RevertAllJoints,
    /// Mirror bodies across a plane/face (Mirror tool, #523).
    CreateMirrorOp {
        plane: FaceId,
        targets: Vec<usize>,
        /// How the reflections land (#639).
        mode: crate::model::MirrorMode,
    },
    /// Re-point an existing mirror operation (#523).
    EditMirrorOp {
        op: usize,
        plane: FaceId,
        targets: Vec<usize>,
        mode: crate::model::MirrorMode,
    },
    /// Linear repeat of bodies along an axis (Repeat tool).
    CreateRepeatOp {
        targets: Vec<usize>,
        axis: crate::model::RevolveAxis,
        /// Turn the copies about the axis instead of sliding them along it (#839).
        around_axis: bool,
        /// Run the pattern the other way along the path (#989).
        flip: bool,
        mode: crate::model::RepeatMode,
        count: String,
        spacing: String,
        length: String,
        /// A face/plane/vertex the fill length is measured to, overriding `length` (#645).
        length_target: Option<crate::model::ExtrudeTarget>,
    },
    /// Re-point an existing repeat operation.
    EditRepeatOp {
        op: usize,
        targets: Vec<usize>,
        axis: crate::model::RevolveAxis,
        around_axis: bool,
        /// Run the pattern the other way along the path (#989).
        flip: bool,
        mode: crate::model::RepeatMode,
        count: String,
        spacing: String,
        length: String,
        length_target: Option<crate::model::ExtrudeTarget>,
    },
    /// Slice bodies with planar and/or line cutters (Slice tool, #1126).
    CreateSliceOp {
        targets: Vec<usize>,
        cutters: Vec<crate::model::SliceCutter>,
        extend_infinite: bool,
    },
    /// Re-point an existing slice operation.
    EditSliceOp {
        op: usize,
        targets: Vec<usize>,
        cutters: Vec<crate::model::SliceCutter>,
        extend_infinite: bool,
    },
    /// Hollow bodies to a wall thickness (Shell tool, #1156).
    CreateShellOp {
        targets: Vec<usize>,
        open_faces: Vec<crate::model::FaceId>,
        thickness: String,
    },
    /// Re-point an existing shell operation.
    EditShellOp {
        op: usize,
        targets: Vec<usize>,
        open_faces: Vec<crate::model::FaceId>,
        thickness: String,
    },
    SetElementVisible {
        element: SceneElement,
        visible: Option<bool>,
    },
    /// Add a material and give it to `bodies` (#834).
    AddMaterial {
        name: Option<String>,
        color: Option<[u8; 3]>,
        bodies: Vec<usize>,
    },
    /// Assign (or clear) a body's material (#834).
    SetBodyMaterial {
        body: usize,
        material: Option<usize>,
    },
    /// Mark a body as a shadow body or restore it as live (#1218).
    SetBodyShadow {
        body: usize,
        shadow: bool,
    },
    /// Click a tree row: replaces selection unless `additive` is true.
    SelectSceneElement {
        element: SceneElement,
        additive: bool,
    },
    ClearSceneSelection,
    SetShapeConstruction {
        element: SceneElement,
        construction: bool,
    },
    /// Set construction/substantial on draw op or all constructable selected targets.
    ApplyConstruction {
        construction: bool,
    },
    /// Toggle construction/substantial on draw op or each constructable selected target.
    ToggleConstruction,
    /// Set visibility of every hideable target in the selection (#1152).
    ApplySelectionVisibility {
        visible: bool,
    },
    /// Toggle visibility of every hideable target in the selection (#1152).
    ToggleSelectionVisibility,
    SetElementName {
        element: SceneElement,
        name: String,
    },
    FocusElementName,
    /// Set the document-wide default length/angle units (#52).
    SetDocumentUnits { length: LengthUnit, angle: AngleUnit },
    /// Set (or clear, via `None`) a per-sketch length/angle unit override (#52).
    SetSketchUnits {
        sketch: usize,
        length: Option<LengthUnit>,
        angle: Option<AngleUnit>,
    },
    /// Create a component (#423).
    CreateComponent {
        name: Option<String>,
        parent: Option<usize>,
    },
    /// Move an element (or component) into a component, or with `None` to the root (#423).
    MoveToComponent {
        element: SceneElement,
        component: Option<usize>,
    },
    /// Set a component's unit overrides (#423).
    SetComponentUnits {
        component: usize,
        length: Option<LengthUnit>,
        angle: Option<AngleUnit>,
    },
    /// Toggle auto-zoom (#438).
    SetAutoZoom { on: bool },
    /// Toggle snapping (#913).
    SetSnapping { on: bool },
    /// Arm one of the active tool's element pickers by heading (#963/#968).
    FocusPicker { name: String },
    /// The Move tool's rotation-candidate spacing, in degrees (#917).
    SetMoveAngleSnap { degrees: f32 },
    /// Toggle the joint preview's animation (#906).
    SetJointAnimation { on: bool },
    /// Toggle Zoom to Fit's glide (#1276). Off snaps instantly.
    SetAnimateZoomToFit { on: bool },
    /// Auto-update channel (#1288): `"release"` or `"pre_release"`.
    SetUpdateChannel { channel: crate::settings::UpdateChannel },
    /// Force touch mode on/off (auto-detected from real touches otherwise).
    SetTouchMode { on: bool },
    /// Start / advance / end an interactive tutorial.
    StartTutorial { index: usize },
    TutorialNext,
    /// Press the current tutorial step's "do it for me" button.
    TutorialAssist,
    EndTutorial,
    SetDim { axis: RectAxis, value: String },
    SetDimLabelOffset { axis: DimLabelAxis, offset: f32 },
    BeginEditCommittedDim { axis: DimLabelAxis },
    CommitCommittedDim,
    /// Angle dimension between two sketch lines (the scripted Dimension-tool angle flow).
    AddAngleConstraint {
        line_a: usize,
        line_b: usize,
        rotation_sign: crate::model::ConstraintSign,
        expression: String,
    },
    AddDistanceConstraint {
        target: DistanceTarget,
        expression: String,
    },
    AddGeometricConstraint(crate::geometric_constraints::GeometricConstraintType),
    ApplyConstraintShortcut(char),
    DragVertex {
        point: ConstraintPoint,
        u: f32,
        v: f32,
    },
    DragLineSegment {
        target: crate::model::ConstraintLine,
        anchor_u: f32,
        anchor_v: f32,
        u: f32,
        v: f32,
    },
    /// Chamfer or fillet sketch vertices where exactly two plain lines meet (#37/#38/#1519):
    /// truncates both lines back from the vertex and bridges them with a new line (straight
    /// for a chamfer, single-cubic-bezier arc for a fillet). `amount` is the chamfer distance
    /// or fillet radius depending on `kind`. One instruction is one operation: `points` holds
    /// every corner in the call (`point` in Lua is the single-corner shorthand).
    VertexTreatment {
        points: Vec<ConstraintPoint>,
        kind: VertexTreatmentKind,
        /// Chamfer distance / fillet radius as a parametric expression (mm), so tying it to a
        /// parameter keeps the bevel following that parameter (#538/#554).
        amount: String,
    },
    /// Chamfer or fillet analytic edges of extrusions' 3D solids (#77) — a mesh-bevel
    /// approximation scoped to the vertical and side/cap edges of a `Rect`/`Polygon`-profiled
    /// extrusion (see `crate::model::ExtrusionEdgeRef`, SPEC §3.4). `amount` is the chamfer
    /// distance or fillet radius depending on `kind`.
    ///
    /// `edges` is the whole set treated by *one* operation (#672). Treating four edges is not
    /// the same as four one-edge operations: each operation bevels the extrusion's own body, so
    /// a second one would start over from the sharp box and the two outputs would overlap.
    EdgeTreatment {
        edges: Vec<(TreatableSolidRef, crate::model::ExtrusionEdgeRef)>,
        kind: VertexTreatmentKind,
        amount: f32,
        expression: String,
    },
    SetLineLength { value: String },
    SetCircleDiameter { value: String },
    BeginEditConstructionPlane { index: usize },
    CommitConstructionPlane,
    SetPlaneOffset { value: String },
    SetPlaneAngle { value: String },
    /// Declaratively add a new construction plane offset from plane `from` (#116).
    CreatePlane { offset: f32, from: usize },
    /// #465: a plane anchored on an arbitrary face (origin + normal), offset along the
    /// normal — the scripted equivalent of clicking a body face with the Plane tool.
    CreateFacePlane { offset: f32, origin: Vec3, normal: Vec3 },
    FocusDim(RectAxis),
    FocusLineLength,
    FocusCircleDiameter,
    FocusPlaneDim(PlaneDim),
    Orbit { dx: f32, dy: f32 },
    Pan { dx: f32, dy: f32 },
    Zoom { scroll: f32 },
    /// First-person mode (#91): toggle (`None`) or force on/off.
    FpsMode { on: Option<bool> },
    /// Turn the FPS player's head, degrees: positive `dx` looks right, positive `dy` up.
    FpsLook { dx: f32, dy: f32 },
    /// Walk the FPS player along the ground, mm: `forward` along the view heading,
    /// `strafe` to the right (instant, not physics-integrated).
    FpsMove { forward: f32, strafe: f32 },
    /// Press the FPS jump key once.
    FpsJump,
    /// Toggle (`None`) or set Minecraft-style flying.
    FpsFly { on: Option<bool> },
    /// Integrate FPS physics for this many seconds with no keys held (lands jumps).
    FpsAdvance { seconds: f32 },
    /// Set the FPS player's scale directly (#120), clamped to
    /// [`crate::fps::MIN_SCALE`, `crate::fps::MAX_SCALE`].
    FpsScale { scale: f32 },
    View(StandardView),
    ViewEdge(CubeEdgeId),
    ViewCorner(CubeCornerId),
    ViewHome,
    SetHomeView,
    ProjectionMode(ProjectionMode),
    /// Ground plane display (#159): grid lines or a solid plane.
    GroundDisplay(GroundDisplay),
    ToggleProjectionMode,
    ShadingMode(ShadingMode),
    /// Set any subset of the camera pose instantly — no transition animation, for
    /// deterministic scripted screenshots (`bearcad.ui.camera{...}`, #108).
    SetCamera {
        yaw: Option<f32>,
        pitch: Option<f32>,
        distance: Option<f32>,
        target: Option<(f32, f32, f32)>,
    },
    /// Frame the whole document (bodies + sketch geometry) in the viewport, instantly (#108).
    ZoomFit,
    /// Switch the Elements pane's layout (`bearcad.ui.elements_view(...)`, #34/#108).
    SetElementsView { mode: crate::hierarchy::HierarchyViewMode },
    /// Show/hide a UI pane. `None` toggles.
    SetPane { pane: Pane, visible: Option<bool> },
    /// Add an AI backend (#1595).
    AddAiBackend { backend: crate::ai::backends::Backend },
    /// Replace a configured AI backend's settings, keeping its id.
    UpdateAiBackend { id: String, backend: crate::ai::backends::Backend },
    /// Remove a configured AI backend.
    RemoveAiBackend { id: String },
    /// Choose the AI backend the conversation uses.
    SelectAiBackend { id: String },
    /// Send a chat message to the selected backend (#1598).
    SendAiMessage { text: String },
    /// Stop a reply in progress.
    CancelAiMessage,
    /// Empty the conversation.
    ClearAiConversation,
    /// Choose how much of the workspace a message carries (#1597).
    SetAiContextScope { scope: crate::ai::context::ContextScope },
    /// Start a backend's running cost total from zero (#1599).
    ResetAiBackendSpend { id: String },
    /// Run one Lua block from the latest reply (#1600).
    RunAiBlock { index: usize },
    /// Put a canned reply in the conversation, for screenshots and tests (#1600).
    SeedAiReply { question: String, reply: String },
    AddParameter { name: String, expression: String },
    CreateParameterFromLineLength { line_index: usize, name: Option<String> },
    /// Create a derived (measured) parameter from a geometry source (#432).
    CreateDerivedParameter {
        source: crate::model::ParameterSource,
        name: Option<String>,
    },
    SetParameterName { index: usize, name: String },
    SetParameterExpression { index: usize, expression: String },
    /// Flip a parameter's primary flag (#727).
    SetParameterPrimary { index: usize, primary: bool },
    /// Set or clear a parameter min/max/step bound (#1176).
    SetParameterBound {
        index: usize,
        which: crate::parameters::ParameterBound,
        expression: Option<String>,
    },
    /// Override (or clear) one unit instance's parameter (#728).
    SetUnitParameterOverride {
        instance: usize,
        name: String,
        expression: Option<String>,
    },
    /// Re-sync one unit's embedded copy from its source file (#732).
    SyncUnit { unit: usize },
    /// Switch a unit's link mode (#734).
    SetUnitLink { unit: usize, link: crate::model::LinkMode },
    /// Add another instance of an embedded unit (#736).
    AddUnitInstance { unit: usize, name: Option<String> },
    /// Clone an existing unit instance with its parameter overrides (#1404).
    CloneUnitInstance { instance: usize },
    /// Show/hide/toggle the Settings window (#737).
    SetSettingsWindow { open: Option<bool> },
    /// Show/hide/toggle the Changelog window (#1328).
    SetChangelogWindow { open: Option<bool> },
    /// Show/hide/toggle the Tutorials pane (#1241).
    SetTutorialPane { open: Option<bool> },
    /// Mark every registered tutorial complete.
    CompleteAllTutorials,
    /// Clear every tutorial completion check.
    UnstartAllTutorials,
    /// Open/close the McMaster-Carr catalog window (#1022).
    SetMcMasterWindow { open: Option<bool>, part: Option<String> },
    /// Open/close the DEV Report issue window (#627 / #1477).
    SetReportIssueWindow { open: Option<bool> },
    /// Open a new blank document tab (`bearcad.ui.new_tab()`).
    NewTab,
    /// Open a new tab on the same document as the current one (`bearcad.ui.new_tab{ same = true }`).
    NewTabSameDocument,
    /// Close a tab by index, or the active tab when `None` (`bearcad.ui.close_tab([i])`).
    CloseTab { index: Option<usize> },
    /// Activate a main-window tab by index (`bearcad.ui.tab(i)`).
    SelectTab { index: usize },
    /// Reorder main-window tabs (`bearcad.ui.reorder_tab(from, to)`).
    ReorderTab { from: usize, to: usize },
    /// Detach a tab into its own window (`bearcad.ui.detach_tab([i])`).
    DetachTab { index: Option<usize> },
    DeleteParameter { index: usize },
    DeleteSelection,
    /// Show/hide the command palette. `None` toggles.
    SetCommandPalette { open: Option<bool> },
    /// Run the best-matching palette command for a query, with the argument a command that
    /// asks for one would have prompted for (#1022).
    RunPaletteCommand { query: String, argument: Option<String> },
    // Synthetic input (viewport-local pixel coordinates)
    Move { x: f32, y: f32 },
    Click { x: f32, y: f32, mods: ClickMods },
    /// Move/click at ground-plane world coordinates (millimetres, z = 0).
    MoveGround { x: f32, y: f32 },
    /// A click at ground coordinates, optionally with modifiers held (#835/#984).
    ClickGround { x: f32, y: f32, mods: ClickMods },
    /// Primary-drag between two ground-plane points (world mm), like [`Self::Drag`].
    DragGround { x0: f32, y0: f32, x1: f32, y1: f32 },
    Drag {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    },
    RightDrag { dx: f32, dy: f32 },
    RightDragShift { dx: f32, dy: f32 },
    /// A key tap, optionally with modifiers held for that press (#1198: Shift+Space opens
    /// the Selection Exploder in one-shot additive mode).
    Key { key: Key, mods: ClickMods },
    KeyDown(Key),
    KeyUp(Key),
    Type(String),

    /// Set (or nudge) a viewport gizmo's scalar (#214): drives the in-progress gizmo the same
    /// way a drag would, so gizmo tools are scriptable/testable. `relative` adds to the current
    /// value instead of replacing it.
    SetGizmo {
        name: String,
        value: f32,
        relative: bool,
    },
    /// Put the active tool into one of its modes (#672) — the Context pane's mode row,
    /// which scripted pointer input can't reach (#130).
    SetToolMode(String),
    /// Help mode (#672): `None` toggles. Documentation captures turn it on.
    HelpMode { on: Option<bool> },
    /// Viewport tool-hint overlay (#1509): `None` toggles. Docs captures turn it off.
    ToolHints { on: Option<bool> },

    // Sequencing
    WaitMs(u64),
    WaitFrames(u32),
    /// Save a screenshot of [`ScreenshotRegion`].
    Screenshot {
        path: String,
        region: ScreenshotRegion,
    },
    Quit,
}

impl Instruction {
    /// Format this instruction as a Lua API call, with no document to resolve against.
    ///
    /// Anything naming an element then falls back to its arena **slot**, which is only the
    /// ordinal a script wants while nothing of that kind has been deleted. Prefer
    /// [`as_lua_in`](Self::as_lua_in) wherever a document is at hand (#1070).
    pub fn as_lua(&self) -> String {
        self.as_lua_in(None)
    }

    /// Format this instruction as a Lua API call, naming elements by their **ordinal** among
    /// the live ones of their kind — what a replay resolves (#1055/#1070).
    pub fn as_lua_in(&self, doc: Option<&crate::model::Document>) -> String {
        match self {
            Instruction::New => "bearcad.new()".to_string(),
            Instruction::Open(path) => format!("bearcad.open({path:?})"),
            Instruction::Save(None) => "bearcad.save()".to_string(),
            Instruction::Save(Some(path)) => format!("bearcad.save({path:?})"),
            Instruction::RebuildGeometry => "bearcad.rebuild_geometry()".to_string(),
            Instruction::ExportPreview { path } => format!("bearcad.export_preview({path:?})"),
            Instruction::ExportStl { path, body: None } => format!("bearcad.export_stl({path:?})"),
            Instruction::ExportStl {
                path,
                body: Some(body),
            } => format!("bearcad.export_stl({path:?}, {body:?})"),
            Instruction::Export3mf { path, body: None } => format!("bearcad.export_3mf({path:?})"),
            Instruction::Export3mf {
                path,
                body: Some(body),
            } => format!("bearcad.export_3mf({path:?}, {body:?})"),
            Instruction::ExportStep { path, body: None } => format!("bearcad.export_step({path:?})"),
            Instruction::ExportStep {
                path,
                body: Some(body),
            } => format!("bearcad.export_step({path:?}, {body:?})"),
            Instruction::ImportStl { path } => format!("bearcad.import_stl({path:?})"),
            Instruction::ImportUnit { path, link, name } => {
                let mut args = format!("path = {path:?}");
                if let Some(link) = link {
                    args.push_str(&format!(
                        ", link = \"{}\"",
                        match link {
                            crate::model::LinkMode::Static => "static",
                            crate::model::LinkMode::Dynamic => "dynamic",
                        }
                    ));
                }
                if let Some(name) = name {
                    args.push_str(&format!(", name = {name:?}"));
                }
                format!("bearcad.import_unit{{ {args} }}")
            }
            Instruction::ImportImage { path, plane } => match plane {
                Some(p) => format!("bearcad.import_image{{ path = {path:?}, plane = {p} }}"),
                None => format!("bearcad.import_image({path:?})"),
            },
            Instruction::SetCalibrationPoint { image, index, x, y } => format!(
                "bearcad.calibration_point{{ image = {image}, index = {index}, x = {x}, y = {y} }}"
            ),
            Instruction::RemoveCalibrationPoint { image, index } => format!(
                "bearcad.remove_calibration_point{{ image = {image}, index = {index} }}"
            ),
            Instruction::CalibrateImage { image, a, b, length, expression } => {
                let length_arg = if expression.is_empty() {
                    length.to_string()
                } else {
                    format!("{expression:?}")
                };
                match (a, b) {
                    (Some(a), Some(b)) => format!(
                        "bearcad.calibrate_image{{ image = {image}, from = {{ {}, {} }}, to = {{ {}, {} }}, length = {length_arg} }}",
                        a.0, a.1, b.0, b.1
                    ),
                    _ => format!(
                        "bearcad.calibrate_image{{ image = {image}, length = {length_arg} }}"
                    ),
                }
            }
            Instruction::SetImageOpacity { image, opacity, expression } => {
                let opacity_arg = if expression.is_empty() {
                    opacity.to_string()
                } else {
                    format!("{expression:?}")
                };
                format!("bearcad.image_opacity{{ image = {image}, opacity = {opacity_arg} }}")
            }
            Instruction::ImportStep { path } => format!("bearcad.import_step({path:?})"),
            Instruction::ImportLua { path, force } => {
                if *force {
                    format!("bearcad.import_lua{{ path = {path:?}, force = true }}")
                } else {
                    format!("bearcad.import_lua({path:?})")
                }
            }
            Instruction::Clear => "bearcad.clear()".to_string(),
            Instruction::Undo => "bearcad.undo()".to_string(),
            Instruction::CopySelection => "bearcad.copy()".to_string(),
            Instruction::PasteAt { linked, x, y, z } => {
                if *linked {
                    format!("bearcad.paste{{ linked = true, x = {x}, y = {y}, z = {z} }}")
                } else {
                    format!("bearcad.paste{{ x = {x}, y = {y}, z = {z} }}")
                }
            }
            Instruction::Tool(tool) => format!("bearcad.ui.tool({:?})", tool_lua_name(*tool)),
            Instruction::BeginSketch { face } => {
                // Full face table so body faces (extrude caps/sides, etc.) round-trip (#1159).
                format!("bearcad.begin_sketch({})", face_id_lua_ref(face, doc))
            }
            Instruction::OpenSketch { sketch } => format!("bearcad.open_sketch({sketch})"),
            Instruction::ExitSketch => "bearcad.exit_sketch()".to_string(),
            Instruction::CreateRect {
                x,
                y,
                width,
                height,
                width_expr,
                height_expr,
            } => {
                let w = match width_expr {
                    Some(e) => format!("{e:?}"),
                    None => width.to_string(),
                };
                let h = match height_expr {
                    Some(e) => format!("{e:?}"),
                    None => height.to_string(),
                };
                format!("bearcad.rect{{ x = {x}, y = {y}, width = {w}, height = {h} }}")
            }
            Instruction::CreateLine { x0, y0, x1, y1, bezier, dimension } => {
                let bezier_arg = match bezier {
                    Some([(c0x, c0y), (c1x, c1y)]) => format!(
                        ", bezier = {{ {{ {c0x}, {c0y} }}, {{ {c1x}, {c1y} }} }}"
                    ),
                    None => String::new(),
                };
                let dim_arg = match dimension {
                    Some(expr) => format!(", dimension = \"{expr}\""),
                    None => String::new(),
                };
                format!(
                    "bearcad.line{{ x = {x0}, y = {y0}, x1 = {x1}, y1 = {y1}{bezier_arg}{dim_arg} }}"
                )
            }
            Instruction::CreateCircle { cx, cy, r, diameter_expr } => match diameter_expr {
                Some(e) => format!("bearcad.circle{{ x = {cx}, y = {cy}, diameter = {e:?} }}"),
                None => format!("bearcad.circle{{ x = {cx}, y = {cy}, r = {r} }}"),
            },
            Instruction::CreateSketchText {
                text,
                font,
                bold,
                italic,
                underline,
                size,
                x,
                y,
                rotation_deg,
                wrap,
                flip,
            } => {
                let mut args = format!("text = {:?}, x = {x}, y = {y}, size = {:?}", text, size);
                if let Some(font) = font {
                    args.push_str(&format!(", font = {font:?}"));
                }
                for (flag, name) in [
                    (*bold, "bold"),
                    (*italic, "italic"),
                    (*underline, "underline"),
                    (*flip, "flip"),
                ] {
                    if flag {
                        args.push_str(&format!(", {name} = true"));
                    }
                }
                if *rotation_deg != 0.0 {
                    args.push_str(&format!(", rotation = {rotation_deg}"));
                }
                if let Some(wrap) = wrap {
                    args.push_str(&format!(", wrap = {wrap}"));
                }
                format!("bearcad.text{{ {args} }}")
            }
            Instruction::Project { elements } => {
                if elements.is_empty() {
                    "bearcad.project()".to_string()
                } else {
                    let ents = elements
                        .iter()
                        .map(|e| element_lua_ref(e, doc))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("bearcad.project{{ entities = {{ {ents} }} }}")
                }
            }
            Instruction::Extrude {
                faces,
                distance,
                body,
                target,
                expression,
                symmetric,
                taper,
                taper_mode,
                taper_expression,
                ..
            } => {
                let body = match body {
                    crate::actions::ExtrudeBodyChoice::New => "",
                    crate::actions::ExtrudeBodyChoice::Merge => ", body = \"merge\"",
                    crate::actions::ExtrudeBodyChoice::Cut => ", body = \"cut\"",
                    crate::actions::ExtrudeBodyChoice::JoinNew => ", body = \"join\"",
                };
                let to = target
                    .as_ref()
                    .map(|t| format!(", to = {}", extrude_target_lua_table(t, doc)))
                    .unwrap_or_default();
                let distance = match expression {
                    Some(e) => format!("{e:?}"),
                    None => distance.to_string(),
                };
                let sym = if *symmetric { ", symmetric = true" } else { "" };
                let taper_s = if taper.abs() > 1e-12
                    || taper_expression.is_some()
                    || *taper_mode != crate::model::ExtrudeTaperMode::Distance
                {
                    let t = match taper_expression {
                        Some(e) => format!("{e:?}"),
                        None => taper.to_string(),
                    };
                    let mode = if *taper_mode != crate::model::ExtrudeTaperMode::Distance {
                        format!(", taper_mode = {:?}", taper_mode.as_str())
                    } else {
                        String::new()
                    };
                    format!(", taper = {t}{mode}")
                } else {
                    String::new()
                };
                format!(
                    "bearcad.extrude{{ {}, distance = {distance}{body}{to}{sym}{taper_s} }}",
                    extrude_face_args(faces, doc)
                )
            }
            Instruction::ExtrudeBodyFace { face, distance, body, target } => {
                let body = match body {
                    crate::actions::ExtrudeBodyChoice::New => "",
                    crate::actions::ExtrudeBodyChoice::Merge => ", body = \"merge\"",
                    crate::actions::ExtrudeBodyChoice::Cut => ", body = \"cut\"",
                    crate::actions::ExtrudeBodyChoice::JoinNew => ", body = \"join\"",
                };
                let to = target
                    .as_ref()
                    .map(|t| format!(", to = {}", extrude_target_lua_table(t, doc)))
                    .unwrap_or_default();
                format!(
                    "bearcad.extrude_face{{ face = {}, distance = {distance}{body}{to} }}",
                    face_id_lua_ref(face, doc)
                )
            }
            Instruction::UpdateExtrusion { extrusion, distance, target, expression } => {
                let d = match (expression, distance) {
                    (Some(e), _) => format!(", distance = {e:?}"),
                    (None, Some(d)) => format!(", distance = {d}"),
                    (None, None) => String::new(),
                };
                let to = target
                    .as_ref()
                    .map(|t| format!(", to = {}", extrude_target_lua_table(t, doc)))
                    .unwrap_or_default();
                format!("bearcad.edit_extrusion{{ extrusion = {extrusion}{d}{to} }}")
            }
            Instruction::Loft { faces, body, bodies } => {
                use crate::model::ExtrudeFace;
                let index_list = |indices: &[usize]| -> String {
                    indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                };
                // The lines' arena slots, not their ordinals (#1070).
                let line_list = |lines: &[crate::model::LineKey]| -> String {
                    lines.iter().map(|i| line_ord(doc, *i).to_string()).collect::<Vec<_>>().join(", ")
                };
                let mut circles = Vec::new();
                let mut polygons = Vec::new();
                for face in faces {
                    match face {
                        ExtrudeFace::Circle(i) => circles.push(circle_ord(doc, *i)),
                        ExtrudeFace::Polygon(lines) => polygons.push(lines),
                        // Boolean regions aren't loftable sections (no interactive path
                        // constructs one), so nothing to render.
                        ExtrudeFace::Boolean { .. }
                        | ExtrudeFace::TextGlyph { .. }
                        | ExtrudeFace::SketchRegion { .. } => {}
                    }
                }
                let mut parts = Vec::new();
                if !circles.is_empty() {
                    parts.push(format!("circles = {{{}}}", index_list(&circles)));
                }
                if !polygons.is_empty() {
                    parts.push(format!(
                        "polygons = {{{}}}",
                        polygons
                            .iter()
                            .map(|lines| format!("{{{}}}", line_list(lines)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                match body {
                    crate::actions::RevolveBodyChoice::NewBody => {}
                    crate::actions::RevolveBodyChoice::AddTouching => {
                        parts.push("body = \"add\"".to_string());
                        if !bodies.is_empty() {
                            parts.push(format!("bodies = {{{}}}", index_list(bodies)));
                        }
                    }
                    crate::actions::RevolveBodyChoice::Cut => {
                        parts.push("body = \"cut\"".to_string());
                        parts.push(format!("bodies = {{{}}}", index_list(bodies)));
                    }
                }
                format!("bearcad.loft{{ {} }}", parts.join(", "))
            }
            Instruction::SetDrawingPage { drawing, width_mm, height_mm, margin_mm } => {
                let field = |name: &str, v: &Option<f32>| {
                    v.map(|v| format!(", {name} = {v}")).unwrap_or_default()
                };
                format!(
                    "bearcad.drawing_page{{ drawing = {drawing}{}{}{} }}",
                    field("width", width_mm),
                    field("height", height_mm),
                    field("margin", margin_mm),
                )
            }
            Instruction::CreateDrawing { name } => match name {
                Some(n) => format!("bearcad.drawing{{ name = {:?} }}", n),
                None => "bearcad.drawing{}".to_string(),
            },
            Instruction::ExportDrawingPdf { drawing, path } => {
                format!("bearcad.export_drawing_pdf{{ drawing = {drawing}, path = {path:?} }}")
            }
            Instruction::ExportDrawingSvg { drawing, path } => {
                format!("bearcad.export_drawing_svg{{ drawing = {drawing}, path = {path:?} }}")
            }
            Instruction::AddDrawingView {
                drawing,
                bodies,
                orientation,
            } => {
                let orient = orientation.label().to_ascii_lowercase();
                match bodies.as_slice() {
                    [b] => format!(
                        "bearcad.drawing_view{{ drawing = {drawing}, body = {b}, orientation = {orient:?} }}"
                    ),
                    many => format!(
                        "bearcad.drawing_view{{ drawing = {drawing}, bodies = {{{}}}, orientation = {orient:?} }}",
                        many.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")
                    ),
                }
            }
            Instruction::AddBodiesToDrawingView {
                drawing,
                view,
                bodies,
            } => match bodies.as_slice() {
                [b] => format!(
                    "bearcad.drawing_view_add{{ drawing = {drawing}, view = {view}, body = {b} }}"
                ),
                many => format!(
                    "bearcad.drawing_view_add{{ drawing = {drawing}, view = {view}, bodies = {{{}}} }}",
                    many.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")
                ),
            },
            Instruction::AddDrawingSketchView {
                drawing,
                sketch,
                orientation,
            } => format!(
                "bearcad.drawing_view{{ drawing = {drawing}, sketch = {sketch}, orientation = {:?} }}",
                orientation.label().to_ascii_lowercase()
            ),
            Instruction::AddDrawingAnnotation { drawing, text, x, y, wrap } => {
                let wrap = wrap.map(|w| format!(", wrap = {w}")).unwrap_or_default();
                format!("bearcad.drawing_text{{ drawing = {drawing}, text = {text:?}, x = {x}, y = {y}{wrap} }}")
            }
            Instruction::AddAlignedDrawingView { drawing, parent, dir, pos } => format!(
                "bearcad.drawing_align_view{{ drawing = {drawing}, parent = {parent}, dir = {:?}, pos = {pos} }}",
                format!("{dir:?}").to_ascii_lowercase()
            ),
            Instruction::MoveDrawingView { drawing, view, x, y } => format!(
                "bearcad.drawing_move_view{{ drawing = {drawing}, view = {view}, x = {x}, y = {y} }}"
            ),
            Instruction::SetDrawingViewSize {
                drawing,
                view,
                size_x,
                size_y,
            } => format!(
                "bearcad.drawing_view_size{{ drawing = {drawing}, view = {view}, \
                 width = {size_x}, height = {size_y} }}"
            ),
            Instruction::ToggleDrawingDimension {
                drawing,
                view,
                a,
                b,
            } => format!(
                "bearcad.drawing_dimension{{ drawing = {drawing}, view = {view}, \
                 a = {{ {}, {}, {} }}, b = {{ {}, {}, {} }} }}",
                a.0, a.1, a.2, b.0, b.1, b.2
            ),
            Instruction::ToggleDrawingCircleDimension { drawing, view, center } => format!(
                "bearcad.drawing_circle_dimension{{ drawing = {drawing}, view = {view}, \
                 center = {{ {}, {}, {} }} }}",
                center.0, center.1, center.2
            ),
            Instruction::SetDrawingDimensionOffset {
                drawing,
                view,
                a,
                b,
                offset,
            } => {
                let off = match offset {
                    Some(o) => format!("{o}"),
                    None => "nil".into(),
                };
                format!(
                    "bearcad.drawing_dim_offset{{ drawing = {drawing}, view = {view}, \
                     a = {{ {}, {}, {} }}, b = {{ {}, {}, {} }}, offset = {off} }}",
                    a.0, a.1, a.2, b.0, b.1, b.2
                )
            }
            Instruction::SetDrawingCircleDimOffset {
                drawing,
                view,
                center,
                offset,
            } => {
                let off = match offset {
                    Some(o) => format!("{o}"),
                    None => "nil".into(),
                };
                format!(
                    "bearcad.drawing_circle_dim_offset{{ drawing = {drawing}, view = {view}, \
                     center = {{ {}, {}, {} }}, offset = {off} }}",
                    center.0, center.1, center.2
                )
            }
            Instruction::SetDrawingViewAlignLines { drawing, view, show } => format!(
                "bearcad.drawing_view_align_lines{{ drawing = {drawing}, view = {view}, \
                 show = {show} }}"
            ),
            Instruction::SetDrawingViewLabel { drawing, view, hidden, pos, text } => {
                let mut args = format!("drawing = {drawing}, view = {view}");
                if let Some(h) = hidden {
                    args.push_str(&format!(", hidden = {h}"));
                }
                if let Some(p) = pos {
                    args.push_str(&format!(", pos = {p:?}"));
                }
                if let Some(t) = text {
                    args.push_str(&format!(", text = {t:?}"));
                }
                format!("bearcad.drawing_view_label{{ {args} }}")
            }
            Instruction::ToggleDrawingAngle {
                drawing,
                view,
                edge1,
                edge2,
            } => {
                let pt = |p: (f32, f32, f32)| format!("{{ {}, {}, {} }}", p.0, p.1, p.2);
                let edge = |e: ((f32, f32, f32), (f32, f32, f32))| {
                    format!("{{ a = {}, b = {} }}", pt(e.0), pt(e.1))
                };
                format!(
                    "bearcad.drawing_angle{{ drawing = {drawing}, view = {view}, edge1 = {}, edge2 = {} }}",
                    edge(*edge1),
                    edge(*edge2)
                )
            }
            // A shape replays as its own call, dimensions and frame spelled out (#909).
            Instruction::Shape { shape } => shape_lua_call(shape, None),
            Instruction::EditShape { index, shape } => shape_lua_call(shape, Some(*index)),
            Instruction::Revolve {
                faces,
                axis,
                angle_deg,
                angle_expression,
                angle_is_revolutions,
                pitch_mm,
                pitch_expression,
                symmetric,
                body,
                bodies,
            } => {
                use crate::model::ExtrudeFace;
                let index_list = |indices: &[usize]| -> String {
                    indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                };
                // The lines' arena slots, not their ordinals (#1070).
                let line_list = |lines: &[crate::model::LineKey]| -> String {
                    lines.iter().map(|i| line_ord(doc, *i).to_string()).collect::<Vec<_>>().join(", ")
                };
                let mut parts = Vec::new();
                let circles: Vec<usize> = faces
                    .iter()
                    .filter_map(|f| match f {
                        ExtrudeFace::Circle(i) => Some(circle_ord(doc, *i)),
                        _ => None,
                    })
                    .collect();
                if !circles.is_empty() {
                    parts.push(format!("circles = {{{}}}", index_list(&circles)));
                }
                for f in faces {
                    if let ExtrudeFace::Polygon(lines) = f {
                        parts.push(format!("polygon = {{{}}}", line_list(lines)));
                    }
                }
                parts.push(format!("axis = {}", revolve_axis_lua(*axis)));
                if !angle_expression.trim().is_empty() {
                    if *angle_is_revolutions {
                        parts.push(format!("revolutions = {:?}", angle_expression));
                    } else {
                        parts.push(format!("angle = {:?}", angle_expression));
                    }
                } else {
                    parts.push(format!("angle = {angle_deg}"));
                }
                if !pitch_expression.trim().is_empty() {
                    parts.push(format!("pitch = {:?}", pitch_expression));
                } else if pitch_mm.abs() > 1e-9 {
                    parts.push(format!("pitch = {pitch_mm}"));
                }
                if *symmetric {
                    parts.push("symmetric = true".to_string());
                }
                match body {
                    crate::actions::RevolveBodyChoice::NewBody => {}
                    crate::actions::RevolveBodyChoice::AddTouching => {
                        parts.push("body = \"add\"".to_string());
                        if !bodies.is_empty() {
                            parts.push(format!("bodies = {{{}}}", index_list(bodies)));
                        }
                    }
                    crate::actions::RevolveBodyChoice::Cut => {
                        parts.push("body = \"cut\"".to_string());
                        parts.push(format!("bodies = {{{}}}", index_list(bodies)));
                    }
                }
                format!("bearcad.revolve{{ {} }}", parts.join(", "))
            }
            Instruction::Sweep { faces, path, body, bodies } => {
                use crate::model::ExtrudeFace;
                let index_list = |indices: &[usize]| -> String {
                    indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                };
                // The lines' arena slots, not their ordinals (#1070).
                let line_list = |lines: &[crate::model::LineKey]| -> String {
                    lines.iter().map(|i| line_ord(doc, *i).to_string()).collect::<Vec<_>>().join(", ")
                };
                let mut parts = Vec::new();
                let circles: Vec<usize> = faces
                    .iter()
                    .filter_map(|f| match f {
                        ExtrudeFace::Circle(i) => Some(circle_ord(doc, *i)),
                        _ => None,
                    })
                    .collect();
                if !circles.is_empty() {
                    parts.push(format!("circles = {{{}}}", index_list(&circles)));
                }
                for f in faces {
                    if let ExtrudeFace::Polygon(lines) = f {
                        parts.push(format!("polygon = {{{}}}", line_list(lines)));
                    }
                }
                parts.push(format!("path = {{{}}}", line_list(path)));
                match body {
                    crate::actions::RevolveBodyChoice::NewBody => {}
                    crate::actions::RevolveBodyChoice::AddTouching => {
                        parts.push("body = \"add\"".to_string());
                        if !bodies.is_empty() {
                            parts.push(format!("bodies = {{{}}}", index_list(bodies)));
                        }
                    }
                    crate::actions::RevolveBodyChoice::Cut => {
                        parts.push("body = \"cut\"".to_string());
                        parts.push(format!("bodies = {{{}}}", index_list(bodies)));
                    }
                }
                format!("bearcad.sweep{{ {} }}", parts.join(", "))
            }
            Instruction::CreateBooleanOp { kind, a, b, keep_b } => {
                boolean_op_lua("bearcad.combine", None, *kind, a, b, *keep_b)
            }
            Instruction::BeginBooleanOp { kind, a, b, keep_b } => {
                boolean_op_lua("bearcad.begin_combine", None, *kind, a, b, *keep_b)
            }
            Instruction::EditBooleanOp { op, kind, a, b, keep_b } => {
                boolean_op_lua("bearcad.edit_boolean", Some(*op), *kind, a, b, *keep_b)
            }
            Instruction::CreateMoveOp { targets, images, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c } => {
                move_op_lua("bearcad.move_bodies", None, targets, images, tx, ty, tz, rx, ry, rz, roll_angle, *face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c)
            }
            Instruction::BeginMoveOp { targets, images, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c } => {
                move_op_lua("bearcad.begin_move", None, targets, images, tx, ty, tz, rx, ry, rz, roll_angle, *face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c)
            }
            Instruction::EditMoveOp { op, targets, images, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c } => {
                move_op_lua("bearcad.edit_move", Some(*op), targets, images, tx, ty, tz, rx, ry, rz, roll_angle, *face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c)
            }
            Instruction::CreateJointOp { members, base, kind, placement, frame, position, position2, position3, limits } => {
                joint_op_lua("bearcad.joint", None, doc, members, *base, kind, placement, frame, position, position2, position3, limits)
            }
            Instruction::BeginJointOp { members, base, kind, placement, frame, position, position2, position3, limits } => {
                joint_op_lua("bearcad.begin_joint", None, doc, members, *base, kind, placement, frame, position, position2, position3, limits)
            }
            Instruction::EditJointOp { op, members, base, kind, placement, frame, position, position2, position3, limits } => {
                joint_op_lua("bearcad.edit_joint", Some(*op), doc, members, *base, kind, placement, frame, position, position2, position3, limits)
            }
            Instruction::SetJointRest { op } => format!("bearcad.set_joint_rest({op})"),
            Instruction::RevertJoint { op } => format!("bearcad.revert_joint({op})"),
            Instruction::RevertAllJoints => "bearcad.revert_joints()".to_string(),
            Instruction::CreateMirrorOp { plane, targets, mode } => {
                mirror_op_lua("bearcad.mirror_bodies", None, doc, plane, targets, *mode)
            }
            Instruction::EditMirrorOp { op, plane, targets, mode } => {
                mirror_op_lua("bearcad.edit_mirror", Some(*op), doc, plane, targets, *mode)
            }
            Instruction::CreateRepeatOp { targets, axis, around_axis, flip, mode, count, spacing, length, length_target } => {
                repeat_op_lua("bearcad.repeat_bodies", None, doc, targets, *axis, *around_axis, *flip, *mode, count, spacing, length, length_target.as_ref())
            }
            Instruction::EditRepeatOp { op, targets, axis, around_axis, flip, mode, count, spacing, length, length_target } => {
                repeat_op_lua("bearcad.edit_repeat", Some(*op), doc, targets, *axis, *around_axis, *flip, *mode, count, spacing, length, length_target.as_ref())
            }
            Instruction::CreateSliceOp { targets, cutters, extend_infinite } => {
                slice_op_lua("bearcad.slice", None, doc, targets, cutters, *extend_infinite)
            }
            Instruction::EditSliceOp { op, targets, cutters, extend_infinite } => {
                slice_op_lua("bearcad.edit_slice", Some(*op), doc, targets, cutters, *extend_infinite)
            }
            Instruction::CreateShellOp {
                targets,
                open_faces,
                thickness,
            } => shell_op_lua("bearcad.shell", None, doc, targets, open_faces, thickness),
            Instruction::EditShellOp {
                op,
                targets,
                open_faces,
                thickness,
            } => shell_op_lua(
                "bearcad.edit_shell",
                Some(*op),
                doc,
                targets,
                open_faces,
                thickness,
            ),
            Instruction::SetElementVisible { element, visible } => {
                let target = element_lua_ref(element, doc);
                let verb = match visible {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.set_visible({target}, {verb:?})")
            }
            Instruction::AddMaterial { name, color, bodies } => {
                let name = name
                    .as_ref()
                    .map(|n| format!("name = {n:?}, "))
                    .unwrap_or_default();
                let color = color
                    .map(|c| format!("color = \"#{:02x}{:02x}{:02x}\", ", c[0], c[1], c[2]))
                    .unwrap_or_default();
                let bodies = bodies
                    .iter()
                    .map(|b| b.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("bearcad.material{{ {name}{color}bodies = {{{bodies}}} }}")
            }
            Instruction::SetBodyMaterial { body, material } => {
                let material = material
                    .map(|m| m.to_string())
                    .unwrap_or_else(|| "nil".to_string());
                format!("bearcad.set_material{{ body = {body}, material = {material} }}")
            }
            Instruction::SetBodyShadow { body, shadow } => {
                format!("bearcad.set_body_shadow{{ body = {body}, shadow = {shadow} }}")
            }
            Instruction::SelectSceneElement { element, additive } => {
                let target = element_lua_ref(element, doc);
                if *additive {
                    format!("bearcad.select({target}, {{ additive = true }})")
                } else {
                    format!("bearcad.select({target})")
                }
            }
            Instruction::ClearSceneSelection => "bearcad.clear_selection()".to_string(),
            Instruction::SetShapeConstruction { element, construction } => {
                format!(
                    "bearcad.set_construction({}, {})",
                    element_lua_ref(element, doc),
                    construction
                )
            }
            Instruction::ApplyConstruction { construction } => {
                format!("bearcad.apply_construction({construction})")
            }
            Instruction::ToggleConstruction => "bearcad.toggle_construction()".to_string(),
            Instruction::ApplySelectionVisibility { visible } => {
                format!("bearcad.apply_visibility({visible})")
            }
            Instruction::ToggleSelectionVisibility => "bearcad.toggle_visibility()".to_string(),
            Instruction::SetElementName { element, name } => {
                format!(
                    "bearcad.set_name({}, {name:?})",
                    element_lua_ref(element, doc)
                )
            }
            Instruction::FocusElementName => "bearcad.ui.focus_name()".to_string(),
            Instruction::SetDocumentUnits { length, angle } => {
                format!(
                    "bearcad.set_units{{ length = {:?}, angle = {:?} }}",
                    length.script_name(),
                    angle.script_name()
                )
            }
            Instruction::CreateComponent { name, parent } => {
                let mut args = String::new();
                if let Some(n) = name {
                    args.push_str(&format!("name = {n:?}"));
                }
                if let Some(p) = parent {
                    if !args.is_empty() {
                        args.push_str(", ");
                    }
                    args.push_str(&format!("parent = {p}"));
                }
                format!("bearcad.component{{ {args} }}")
            }
            Instruction::MoveToComponent { element, component } => {
                let target = match component {
                    Some(c) => c.to_string(),
                    None => "false".to_string(),
                };
                let tokens = element_script_tokens(element.clone(), doc);
                format!(
                    "bearcad.move_to_component{{ kind = {:?}, index = {}, component = {target} }}",
                    tokens.kind, tokens.index
                )
            }
            Instruction::SetComponentUnits { component, length, angle } => {
                let mut args = format!("component = {component}");
                if let Some(l) = length {
                    args.push_str(&format!(", length = {:?}", l.script_name()));
                }
                if let Some(a) = angle {
                    args.push_str(&format!(", angle = {:?}", a.script_name()));
                }
                format!("bearcad.set_units{{ {args} }}")
            }
            Instruction::SetSketchUnits { sketch, length, angle } => {
                let length_arg = match length {
                    Some(length) => format!(", length = {:?}", length.script_name()),
                    None => String::new(),
                };
                let angle_arg = match angle {
                    Some(angle) => format!(", angle = {:?}", angle.script_name()),
                    None => String::new(),
                };
                format!("bearcad.set_units{{ sketch = {sketch}{length_arg}{angle_arg} }}")
            }
            Instruction::SetAutoZoom { on } => {
                format!("bearcad.ui.auto_zoom({on})")
            }
            Instruction::SetSnapping { on } => {
                format!("bearcad.ui.snapping({on})")
            }
            Instruction::FocusPicker { name } => {
                format!("bearcad.ui.picker_focus({name:?})")
            }
            Instruction::SetMoveAngleSnap { degrees } => {
                format!("bearcad.ui.angle_snap({degrees})")
            }
            Instruction::SetJointAnimation { on } => {
                format!("bearcad.ui.animate_joints({on})")
            }
            Instruction::SetAnimateZoomToFit { on } => {
                format!("bearcad.ui.animate_zoom_to_fit({on})")
            }
            Instruction::SetUpdateChannel { channel } => {
                format!("bearcad.ui.update_channel({:?})", channel.as_str())
            }
            Instruction::SetTouchMode { on } => {
                format!("bearcad.ui.touch({on})")
            }
            Instruction::StartTutorial { index } => {
                format!(
                    "bearcad.ui.tutorial({:?})",
                    crate::tutorial::TUTORIALS[*index].name
                )
            }
            Instruction::TutorialNext => "bearcad.ui.tutorial_next()".to_string(),
            Instruction::TutorialAssist => "bearcad.ui.tutorial_assist()".to_string(),
            Instruction::EndTutorial => "bearcad.ui.tutorial_end()".to_string(),
            Instruction::SetDim { axis, value } => {
                format!(
                    "bearcad.set_dim({:?}, {value:?})",
                    rect_axis_lua_name(*axis)
                )
            }
            Instruction::SetDimLabelOffset { axis, offset } => {
                format!(
                    "bearcad.set_dim_label_offset({:?}, {offset})",
                    dim_label_axis_lua_name(*axis)
                )
            }
            Instruction::BeginEditCommittedDim { axis } => {
                format!(
                    "bearcad.edit_dim({:?})",
                    dim_label_axis_lua_name(*axis)
                )
            }
            Instruction::CommitCommittedDim => "bearcad.commit_dim()".to_string(),
            Instruction::AddAngleConstraint {
                line_a,
                line_b,
                rotation_sign,
                expression,
            } => format!(
                "bearcad.add_angle_constraint{{ a = {line_a}, b = {line_b}, sign = {rotation_sign}, value = {expression:?} }}"
            ),
            Instruction::AddDistanceConstraint { target, expression } => {
                format!(
                    "bearcad.add_constraint({}, {expression:?})",
                    distance_target_lua_ref(target)
                )
            }
            Instruction::AddGeometricConstraint(kind) => {
                format!(
                    "bearcad.add_geometric_constraint({:?})",
                    geometric_constraint_lua_name(*kind)
                )
            }
            Instruction::ApplyConstraintShortcut(key) => {
                format!("bearcad.constraint_shortcut({key:?})")
            }
            Instruction::DragVertex { point, u, v } => {
                format!(
                    "bearcad.ui.drag_vertex({}, {u}, {v})",
                    constraint_point_lua_ref(point, doc)
                )
            }
            Instruction::DragLineSegment {
                target,
                anchor_u,
                anchor_v,
                u,
                v,
            } => format!(
                "bearcad.ui.drag_line({}, {anchor_u}, {anchor_v}, {u}, {v})",
                constraint_line_lua_ref(target, doc)
            ),
            Instruction::VertexTreatment { points, kind, amount } => {
                let (fname, amount_key) = match kind {
                    VertexTreatmentKind::Chamfer => ("chamfer_vertex", "distance"),
                    VertexTreatmentKind::Fillet => ("fillet_vertex", "radius"),
                };
                // A plain number records bare; a parametric expression records as a quoted string.
                let amount_lua = if amount.trim().parse::<f32>().is_ok() {
                    amount.clone()
                } else {
                    format!("{amount:?}")
                };
                match points.as_slice() {
                    [point] => format!(
                        "bearcad.{fname}{{ point = {}, {amount_key} = {amount_lua} }}",
                        constraint_point_lua_ref(point, doc)
                    ),
                    many => {
                        let list = many
                            .iter()
                            .map(|p| constraint_point_lua_ref(p, doc))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!(
                            "bearcad.{fname}{{ points = {{ {list} }}, {amount_key} = {amount_lua} }}"
                        )
                    }
                }
            }
            Instruction::EdgeTreatment { edges, kind, amount, expression } => {
                let (fname, amount_key) = match kind {
                    VertexTreatmentKind::Chamfer => ("chamfer_edge", "distance"),
                    VertexTreatmentKind::Fillet => ("fillet_edge", "radius"),
                };
                let host_lua = |host: TreatableSolidRef| match host {
                    TreatableSolidRef::Extrusion(i) => format!("extrusion = {i}"),
                    TreatableSolidRef::Primitive(i) => format!("primitive = {i}"),
                };
                let amount_lua = if !expression.trim().is_empty() && expression.trim().parse::<f32>().is_err()
                {
                    format!("{expression:?}")
                } else {
                    amount.to_string()
                };
                // One edge keeps the singular, readable form; a set spells out `edges`.
                match edges.as_slice() {
                    [(host, edge)] => format!(
                        "bearcad.{fname}{{ {}, edge = {}, {amount_key} = {amount_lua} }}",
                        host_lua(*host),
                        extrusion_edge_lua_ref(*edge)
                    ),
                    many => {
                        let list = many
                            .iter()
                            .map(|(host, edge)| {
                                let e = extrusion_edge_lua_ref(*edge);
                                format!("{{ {}, edge = {e} }}", host_lua(*host))
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("bearcad.{fname}{{ edges = {{ {list} }}, {amount_key} = {amount_lua} }}")
                    }
                }
            }
            Instruction::SetLineLength { value } => {
                format!("bearcad.set_dim(\"length\", {value:?})")
            }
            Instruction::SetCircleDiameter { value } => {
                format!("bearcad.set_dim(\"diameter\", {value:?})")
            }
            Instruction::BeginEditConstructionPlane { index } => {
                format!("bearcad.edit_plane({index})")
            }
            Instruction::CommitConstructionPlane => "bearcad.commit_plane()".to_string(),
            Instruction::SetPlaneOffset { value } => {
                format!("bearcad.set_dim(\"offset\", {value:?})")
            }
            Instruction::SetPlaneAngle { value } => {
                format!("bearcad.set_dim(\"angle\", {value:?})")
            }
            Instruction::CreatePlane { offset, from } => {
                format!("bearcad.plane{{ offset = {offset}, from = {from} }}")
            }
            Instruction::CreateFacePlane { offset, origin, normal } => {
                format!(
                    "bearcad.plane{{ offset = {offset}, origin = {{{}, {}, {}}}, normal = {{{}, {}, {}}} }}",
                    origin.x, origin.y, origin.z, normal.x, normal.y, normal.z
                )
            }
            Instruction::FocusDim(axis) => {
                format!("bearcad.ui.focus_dim({:?})", rect_axis_lua_name(*axis))
            }
            Instruction::FocusLineLength => "bearcad.ui.focus_dim(\"length\")".to_string(),
            Instruction::FocusCircleDiameter => "bearcad.ui.focus_dim(\"diameter\")".to_string(),
            Instruction::FocusPlaneDim(dim) => {
                format!("bearcad.ui.focus_dim({:?})", plane_dim_lua_name(*dim))
            }
            Instruction::FpsMode { on } => match on {
                Some(on) => format!("bearcad.ui.fps({on})"),
                None => "bearcad.ui.fps()".to_string(),
            },
            Instruction::FpsLook { dx, dy } => format!("bearcad.ui.fps_look({dx}, {dy})"),
            Instruction::FpsMove { forward, strafe } => {
                format!("bearcad.ui.fps_move{{ forward = {forward}, strafe = {strafe} }}")
            }
            Instruction::FpsJump => "bearcad.ui.fps_jump()".to_string(),
            Instruction::FpsFly { on } => match on {
                Some(on) => format!("bearcad.ui.fps_fly({on})"),
                None => "bearcad.ui.fps_fly()".to_string(),
            },
            Instruction::FpsAdvance { seconds } => {
                format!("bearcad.ui.fps_advance({seconds})")
            }
            Instruction::FpsScale { scale } => format!("bearcad.ui.fps_scale({scale})"),
            Instruction::Orbit { dx, dy } => format!("bearcad.ui.orbit({dx}, {dy})"),
            Instruction::Pan { dx, dy } => format!("bearcad.ui.pan({dx}, {dy})"),
            Instruction::Zoom { scroll } => format!("bearcad.ui.wheel({scroll})"),
            Instruction::View(view) => format!("bearcad.ui.view({:?})", view_script_name(*view)),
            Instruction::ViewEdge(edge) => {
                format!("bearcad.ui.view(\"edge\", {:?})", edge_script_name(*edge))
            }
            Instruction::ViewCorner(corner) => format!(
                "bearcad.ui.view(\"corner\", {:?})",
                corner_script_name(*corner)
            ),
            Instruction::ViewHome => "bearcad.ui.view_home()".to_string(),
            Instruction::SetHomeView => "bearcad.ui.set_home_view()".to_string(),
            Instruction::ProjectionMode(mode) => {
                format!("bearcad.ui.view({:?})", projection_mode_script_name(*mode))
            }
            Instruction::ToggleProjectionMode => "bearcad.ui.toggle_projection()".to_string(),
            Instruction::ShadingMode(mode) => {
                format!("bearcad.ui.shading({:?})", mode.script_name())
            }
            Instruction::GroundDisplay(mode) => {
                format!("bearcad.ui.ground({:?})", mode.script_name())
            }
            Instruction::SetCamera {
                yaw,
                pitch,
                distance,
                target,
            } => {
                let mut parts = Vec::new();
                if let Some(yaw) = yaw {
                    parts.push(format!("yaw = {yaw}"));
                }
                if let Some(pitch) = pitch {
                    parts.push(format!("pitch = {pitch}"));
                }
                if let Some(distance) = distance {
                    parts.push(format!("distance = {distance}"));
                }
                if let Some((x, y, z)) = target {
                    parts.push(format!("target = {{{x}, {y}, {z}}}"));
                }
                format!("bearcad.ui.camera{{ {} }}", parts.join(", "))
            }
            Instruction::ZoomFit => "bearcad.ui.zoom_fit()".to_string(),
            Instruction::SetElementsView { mode } => {
                format!("bearcad.ui.elements_view({:?})", mode.script_name())
            }
            Instruction::SetPane { pane, visible } => {
                let verb = match visible {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.ui.pane({:?}, {verb:?})", pane.script_name())
            }
            Instruction::AddAiBackend { backend } => {
                // `key_description` rather than the key: --show-commands output is pasted
                // into bug reports (#1595).
                format!(
                    "bearcad.ai.add_backend{{ name = {:?}, provider = {:?}, model = {:?}, key = {:?} }}",
                    backend.name,
                    backend.provider.as_str(),
                    backend.model,
                    backend.key_description()
                )
            }
            Instruction::UpdateAiBackend { id, backend } => {
                format!(
                    "bearcad.ai.update_backend({id:?}, {{ model = {:?}, key = {:?} }})",
                    backend.model,
                    backend.key_description()
                )
            }
            Instruction::RemoveAiBackend { id } => {
                format!("bearcad.ai.remove_backend({id:?})")
            }
            Instruction::SendAiMessage { text } => {
                format!("bearcad.ai.ask({text:?})")
            }
            Instruction::CancelAiMessage => "bearcad.ai.stop()".to_string(),
            Instruction::ClearAiConversation => "bearcad.ai.clear()".to_string(),
            Instruction::SetAiContextScope { scope } => {
                format!("bearcad.ai.context_scope({:?})", scope.as_str())
            }
            Instruction::ResetAiBackendSpend { id } => {
                format!("bearcad.ai.reset_usage({id:?})")
            }
            Instruction::RunAiBlock { index } => {
                format!("bearcad.ai.run_block({index})")
            }
            Instruction::SeedAiReply { question, reply } => {
                format!("bearcad.ai.seed_reply({question:?}, {reply:?})")
            }
            Instruction::SelectAiBackend { id } => {
                format!("bearcad.ai.set_backend({id:?})")
            }
            Instruction::AddParameter { name, expression } => {
                format!("bearcad.parameter(\"add\", {name:?}, {expression:?})")
            }
            Instruction::CreateDerivedParameter { source, name } => {
                use crate::model::ParameterSource as PS;
                let src = match source {
                    PS::LineLength(i) => {
                        format!("kind = \"line_length\", a = {}", line_ord(doc, *i))
                    }
                    PS::PointDistance(a, b) => format!(
                        "kind = \"point_distance\", a = {{ {} }}, b = {{ {} }}",
                        point_lua_fields(a, doc),
                        point_lua_fields(b, doc)
                    ),
                    PS::LineDistance(a, b) => {
                        format!(
                            "kind = \"line_distance\", a = {}, b = {}",
                            a.index(),
                            b.index()
                        )
                    }
                    PS::LineAngle(a, b) => {
                        format!("kind = \"line_angle\", a = {}, b = {}", line_ord(doc, *a), line_ord(doc, *b))
                    }
                    // Body geometry (#647) is keyed on quantized world points; scripts spell
                    // them as plain **mm** coordinates, which the parser re-quantizes.
                    PS::BodyEdgeLength { body, a, b } => format!(
                        "kind = \"body_edge_length\", body = {}, a = {}, b = {}",
                        body_ord(doc, *body),
                        mm_point_lua(*a),
                        mm_point_lua(*b)
                    ),
                    PS::BodyVertexDistance { body_a, a, body_b, b } => format!(
                        "kind = \"body_vertex_distance\", body = {}, a = {}, body_b = {}, b = {}",
                        body_ord(doc, *body_a),
                        mm_point_lua(*a),
                        body_ord(doc, *body_b),
                        mm_point_lua(*b)
                    ),
                    // Analytic unit edge (#724): the face has no flat Lua spelling, so it
                    // rides as its JSON encoding; the parser feeds it back through serde.
                    PS::UnitEdgeLength { instance, face, edge } => format!(
                        "kind = \"unit_edge_length\", instance = {}, face = {:?}, edge = {edge}",
                        instance_ord(doc, *instance),
                        serde_json::to_string(face).unwrap_or_default()
                    ),
                };
                match name {
                    Some(name) => {
                        format!("bearcad.derive_parameter{{ {src}, name = {name:?} }}")
                    }
                    None => format!("bearcad.derive_parameter{{ {src} }}"),
                }
            }
            Instruction::CreateParameterFromLineLength { line_index, name } => match name {
                Some(name) => format!(
                    "bearcad.parameter(\"from_line_length\", {line_index}, {name:?})"
                ),
                None => format!("bearcad.parameter(\"from_line_length\", {line_index})"),
            },
            Instruction::SetParameterName { index, name } => {
                format!("bearcad.parameter(\"name\", {index}, {name:?})")
            }
            Instruction::SetParameterExpression { index, expression } => {
                format!("bearcad.parameter(\"value\", {index}, {expression:?})")
            }
            // #1180: script surface is `private` (inverse of stored primary).
            Instruction::SetParameterPrimary { index, primary } => {
                format!("bearcad.parameter(\"private\", {index}, {})", !primary)
            }
            Instruction::SetParameterBound { index, which, expression } => {
                let action = which.label();
                match expression {
                    Some(expression) => {
                        format!("bearcad.parameter({action:?}, {index}, {expression:?})")
                    }
                    None => format!("bearcad.parameter({action:?}, {index})"),
                }
            }
            Instruction::SyncUnit { unit } => format!("bearcad.sync_unit({unit})"),
            Instruction::AddUnitInstance { unit, name } => match name {
                Some(name) => {
                    format!("bearcad.add_unit_instance{{ unit = {unit}, name = {name:?} }}")
                }
                None => format!("bearcad.add_unit_instance{{ unit = {unit} }}"),
            },
            Instruction::CloneUnitInstance { instance } => {
                format!("bearcad.clone_unit_instance{{ instance = {instance} }}")
            }
            Instruction::SetUnitLink { unit, link } => format!(
                "bearcad.unit_link({unit}, \"{}\")",
                match link {
                    crate::model::LinkMode::Static => "static",
                    crate::model::LinkMode::Dynamic => "dynamic",
                }
            ),
            Instruction::SetUnitParameterOverride { instance, name, expression } => {
                match expression {
                    Some(expression) => format!(
                        "bearcad.unit_override{{ instance = {instance}, name = {name:?}, value = {expression:?} }}"
                    ),
                    None => format!(
                        "bearcad.unit_override{{ instance = {instance}, name = {name:?} }}"
                    ),
                }
            }
            Instruction::DeleteParameter { index } => {
                format!("bearcad.parameter(\"delete\", {index})")
            }
            Instruction::DeleteSelection => "bearcad.delete_selection()".to_string(),
            Instruction::SetCommandPalette { open } => {
                let verb = match open {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.ui.palette({verb:?})")
            }
            Instruction::SetSettingsWindow { open } => {
                let verb = match open {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.ui.settings({verb:?})")
            }
            Instruction::SetChangelogWindow { open } => {
                let verb = match open {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.ui.changelog({verb:?})")
            }
            Instruction::SetTutorialPane { open } => {
                let verb = match open {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.ui.tutorial_pane({verb:?})")
            }
            Instruction::CompleteAllTutorials => {
                "bearcad.ui.complete_all_tutorials()".to_string()
            }
            Instruction::UnstartAllTutorials => {
                "bearcad.ui.unstart_all_tutorials()".to_string()
            }
            Instruction::SetMcMasterWindow { open, part } => {
                let verb = match open {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                match part {
                    Some(part) => format!("bearcad.ui.mcmaster({verb:?}, {part:?})"),
                    None => format!("bearcad.ui.mcmaster({verb:?})"),
                }
            }
            Instruction::SetReportIssueWindow { open } => {
                let verb = match open {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.ui.report_issue({verb:?})")
            }
            Instruction::NewTab => "bearcad.ui.new_tab()".to_string(),
            Instruction::NewTabSameDocument => "bearcad.ui.new_tab{ same = true }".to_string(),
            Instruction::CloseTab { index: None } => "bearcad.ui.close_tab()".to_string(),
            Instruction::CloseTab { index: Some(i) } => format!("bearcad.ui.close_tab({i})"),
            Instruction::SelectTab { index } => format!("bearcad.ui.tab({index})"),
            Instruction::ReorderTab { from, to } => {
                format!("bearcad.ui.reorder_tab({from}, {to})")
            }
            Instruction::DetachTab { index: None } => "bearcad.ui.detach_tab()".to_string(),
            Instruction::DetachTab { index: Some(i) } => format!("bearcad.ui.detach_tab({i})"),
            Instruction::RunPaletteCommand { query, argument } => match argument {
                Some(argument) => {
                    format!("bearcad.ui.palette(\"run\", {query:?}, {argument:?})")
                }
                None => format!("bearcad.ui.palette(\"run\", {query:?})"),
            },
            Instruction::Move { x, y } => format!("bearcad.ui.move({x}, {y})"),
            Instruction::Click { x, y, mods } => {
                format!("bearcad.ui.click({x}, {y}{})", mods.lua_opts())
            }
            Instruction::MoveGround { x, y } => format!("bearcad.ui.move_ground({x}, {y})"),
            Instruction::ClickGround { x, y, mods } => {
                format!("bearcad.ui.click_ground({x}, {y}{})", mods.lua_opts())
            }
            Instruction::DragGround { x0, y0, x1, y1 } => {
                format!("bearcad.ui.drag_ground({x0}, {y0}, {x1}, {y1})")
            }
            Instruction::Drag { x0, y0, x1, y1 } => {
                format!("bearcad.ui.drag({x0}, {y0}, {x1}, {y1})")
            }
            Instruction::RightDrag { dx, dy } => format!("bearcad.ui.right_drag({dx}, {dy})"),
            Instruction::RightDragShift { dx, dy } => {
                format!("bearcad.ui.right_drag_pan({dx}, {dy})")
            }
            Instruction::Key { key, mods } => {
                format!("bearcad.ui.key({:?}{})", key_name(*key), mods.lua_opts())
            }
            Instruction::KeyDown(key) => format!("bearcad.ui.keydown({:?})", key_name(*key)),
            Instruction::KeyUp(key) => format!("bearcad.ui.keyup({:?})", key_name(*key)),
            Instruction::Type(text) => format!("bearcad.ui.type({text:?})"),
            Instruction::WaitMs(ms) => format!("bearcad.ui.wait_ms({ms})"),
            Instruction::WaitFrames(n) => format!("bearcad.ui.wait({n})"),
            Instruction::Screenshot { path, region } => match region {
                ScreenshotRegion::Viewport => format!("bearcad.ui.screenshot({path:?})"),
                other => format!("bearcad.ui.screenshot({path:?}, {:?})", other.script_name()),
            },
            Instruction::SetToolMode(mode) => format!("bearcad.ui.tool_mode({mode:?})"),
            Instruction::HelpMode { on } => match on {
                Some(on) => format!("bearcad.ui.help({on})"),
                None => "bearcad.ui.help()".to_string(),
            },
            Instruction::ToolHints { on } => match on {
                Some(on) => format!("bearcad.ui.tool_hints({on})"),
                None => "bearcad.ui.tool_hints()".to_string(),
            },
            Instruction::SetGizmo { name, value, relative } => {
                if *relative {
                    format!("bearcad.drag_gizmo{{ name = {name:?}, by = {value} }}")
                } else {
                    format!("bearcad.set_gizmo{{ name = {name:?}, value = {value} }}")
                }
            }
            Instruction::Quit => "bearcad.quit()".to_string(),
        }
    }
}

/// Script load / execution errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub message: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ScriptError {}

/// Map a human-readable key name to an egui [`Key`].
pub fn parse_key(name: &str) -> Result<Key, String> {
    match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Ok(Key::Enter),
        "tab" => Ok(Key::Tab),
        "escape" | "esc" => Ok(Key::Escape),
        "backspace" => Ok(Key::Backspace),
        "delete" | "del" => Ok(Key::Delete),
        "left" => Ok(Key::ArrowLeft),
        "right" => Ok(Key::ArrowRight),
        "up" => Ok(Key::ArrowUp),
        "down" => Ok(Key::ArrowDown),
        "space" => Ok(Key::Space),
        "r" => Ok(Key::R),
        "a" => Ok(Key::A),
        "b" => Ok(Key::B),
        "c" => Ok(Key::C),
        "d" => Ok(Key::D),
        "e" => Ok(Key::E),
        "f" => Ok(Key::F),
        "g" => Ok(Key::G),
        "h" => Ok(Key::H),
        "i" => Ok(Key::I),
        "j" => Ok(Key::J),
        "k" => Ok(Key::K),
        "l" => Ok(Key::L),
        "m" => Ok(Key::M),
        "n" => Ok(Key::N),
        "o" => Ok(Key::O),
        "p" => Ok(Key::P),
        "q" => Ok(Key::Q),
        "s" => Ok(Key::S),
        "t" => Ok(Key::T),
        "u" => Ok(Key::U),
        "v" => Ok(Key::V),
        "w" => Ok(Key::W),
        "x" => Ok(Key::X),
        "y" => Ok(Key::Y),
        "z" => Ok(Key::Z),
        "0" => Ok(Key::Num0),
        "1" => Ok(Key::Num1),
        "2" => Ok(Key::Num2),
        "3" => Ok(Key::Num3),
        "4" => Ok(Key::Num4),
        "5" => Ok(Key::Num5),
        "6" => Ok(Key::Num6),
        "7" => Ok(Key::Num7),
        "8" => Ok(Key::Num8),
        "9" => Ok(Key::Num9),
        "`" | "backtick" | "grave" => Ok(Key::Backtick),
        _ => Err(format!("unknown key '{name}'")),
    }
}

struct ElementScriptTokens {
    kind: &'static str,
    index: usize,
    point: Option<crate::model::ConstraintPoint>,
}

/// Resolve a scripted size that may be a parameter expression (#402): the expression,
/// when present, wins over the plain number and must evaluate against the document's
/// parameters.
fn eval_scalar_input(
    doc: &crate::model::Document,
    number: f32,
    expr: &Option<String>,
    what: &str,
) -> Result<f32, String> {
    match expr {
        None => Ok(number),
        Some(e) => crate::value::eval_length_mm_in_doc(e, doc)
            .ok_or_else(|| format!("{what} expression {e:?} doesn't evaluate to a length")),
    }
}

/// An element's **ordinal** among the live ones of its kind when a document was available to
/// count them, else its arena **slot** (#1070). The two agree until something of that kind is
/// deleted, at which point only the ordinal is what a replay resolves — so an export with no
/// document to hand says the slot and is wrong exactly then.
fn ordinal_or_slot(found: Option<Option<usize>>, slot: u32) -> usize {
    found.flatten().unwrap_or(slot as usize)
}

/// The ordinal helpers the renderers use, one per collection a script can name (#1070).
macro_rules! ordinal_fn {
    ($name:ident, $key:ty, $coll:ident) => {
        fn $name(doc: Option<&crate::model::Document>, key: $key) -> usize {
            ordinal_or_slot(doc.map(|d| d.$coll.keys().position(|k| k == key)), key.index())
        }
    };
}
ordinal_fn!(line_ord, crate::model::LineKey, lines);
ordinal_fn!(circle_ord, crate::model::CircleKey, circles);
ordinal_fn!(body_ord, crate::model::BodyKey, bodies);
ordinal_fn!(instance_ord, crate::model::UnitInstanceKey, unit_instances);

fn element_script_tokens(
    element: SceneElement,
    doc: Option<&crate::model::Document>,
) -> ElementScriptTokens {
    match element {
        // A drawing's three item types script by their own kind names (#363/#967); a
        // dimension has no index of its own, so it reports the view it is shown on.
        SceneElement::DrawingElement { drawing, element } => {
            use crate::context::DrawingElementRef as D;
            let (kind, index) = match element {
                D::Projection(i) => ("projection", i),
                D::Text(key) => (
                    "annotation",
                    ordinal_or_slot(
                        doc.and_then(|d| d.drawings.get(drawing))
                            .map(|dr| dr.annotations.keys().position(|k| k == key)),
                        key.index(),
                    ),
                ),
                D::Dimension { view, .. } => ("drawing_dimension", view),
            };
            ElementScriptTokens {
                kind,
                index,
                point: None,
            }
        }
        SceneElement::ConstructionPlane(i) => ElementScriptTokens {
            kind: "construction_plane",
            index: ordinal_or_slot(doc.map(|d| d.construction_planes.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Sketch(i) => ElementScriptTokens {
            kind: "sketch",
            index: ordinal_or_slot(doc.map(|d| d.sketches.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Line(i) => ElementScriptTokens {
            kind: "line",
            index: ordinal_or_slot(doc.map(|d| d.lines.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Circle(i) => ElementScriptTokens {
            kind: "circle",
            index: ordinal_or_slot(doc.map(|d| d.circles.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Constraint(i) => ElementScriptTokens {
            kind: "constraint",
            index: ordinal_or_slot(doc.map(|d| d.constraints.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Point(point) => ElementScriptTokens {
            kind: "point",
            index: 0,
            point: Some(point),
        },
        SceneElement::Extrusion(i) => ElementScriptTokens {
            kind: "extrusion",
            index: ordinal_or_slot(doc.map(|d| d.extrusions.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Body(i) => ElementScriptTokens {
            kind: "body",
            index: ordinal_or_slot(doc.map(|d| d.bodies.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // Handled directly in `element_lua_ref` before this is reached (a `FaceEdge` doesn't
        // fit the `kind`/`index`/`edge`/`point` shape the other variants share).
        SceneElement::FaceEdge(_) => ElementScriptTokens {
            kind: "face_edge",
            index: 0,
            point: None,
        },
        // Geometry-keyed 3D sub-elements (#156): no stable scripting identity yet, so the
        // recorded-script export falls back to a placeholder like `FaceEdge` does.
        SceneElement::BodyEdge { .. } => ElementScriptTokens {
            kind: "body_edge",
            index: 0,
            point: None,
        },
        SceneElement::BodyVertex { .. } => ElementScriptTokens {
            kind: "body_vertex",
            index: 0,
            point: None,
        },
        SceneElement::SketchFace(_) => ElementScriptTokens {
            kind: "face",
            index: 0,
            point: None,
        },
        SceneElement::MovePoint(_) => ElementScriptTokens {
            kind: "move_point",
            index: 0,
            point: None,
        },
        SceneElement::ExtrusionEdge { extrusion, .. } => ElementScriptTokens {
            kind: "extrusion_edge",
            index: extrusion.index() as usize,
            point: None,
        },
        SceneElement::PrimitiveEdge { primitive, .. } => ElementScriptTokens {
            kind: "primitive_edge",
            index: primitive.index() as usize,
            point: None,
        },
        SceneElement::RepeatedFace { instance, .. } => ElementScriptTokens {
            kind: "repeated_face",
            index: instance,
            point: None,
        },
        SceneElement::BodyFace { .. } => ElementScriptTokens {
            kind: "body_face",
            index: 0,
            point: None,
        },
        // A cylinder and its centre line (#1013) are keyed by geometry, not by an index.
        SceneElement::BodyCylinder { .. } => ElementScriptTokens {
            kind: "body_cylinder",
            index: 0,
            point: None,
        },
        SceneElement::BodyAxis { .. } => ElementScriptTokens {
            kind: "body_axis",
            index: 0,
            point: None,
        },
        // The image's **slot**, not its ordinal (#1055): `as_lua` has no document to count
        // live images against. The two agree until an image is deleted.
        SceneElement::Image(i) => ElementScriptTokens {
            kind: "image",
            index: ordinal_or_slot(doc.map(|d| d.tracing_images.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::BooleanOp(i) => ElementScriptTokens {
            kind: "boolean_op",
            index: ordinal_or_slot(doc.map(|d| d.boolean_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::MoveOp(i) => ElementScriptTokens {
            kind: "move_op",
            index: ordinal_or_slot(doc.map(|d| d.move_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::MirrorOp(i) => ElementScriptTokens {
            kind: "mirror_op",
            index: ordinal_or_slot(doc.map(|d| d.mirror_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::RepeatOp(i) => ElementScriptTokens {
            kind: "repeat_op",
            index: ordinal_or_slot(doc.map(|d| d.repeat_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::SketchOffsetOp(i) => ElementScriptTokens {
            kind: "sketch_offset_op",
            index: ordinal_or_slot(doc.map(|d| d.sketch_offset_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::SketchMirrorOp(i) => ElementScriptTokens {
            kind: "sketch_mirror_op",
            index: ordinal_or_slot(doc.map(|d| d.sketch_mirror_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::SketchVertexTreatmentOp(i) => ElementScriptTokens {
            kind: "sketch_vertex_treatment_op",
            index: ordinal_or_slot(doc.map(|d| d.sketch_vertex_treatment_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::SketchRepeatOp(i) => ElementScriptTokens {
            kind: "sketch_repeat_op",
            index: ordinal_or_slot(doc.map(|d| d.sketch_repeat_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::SketchSliceOp(i) => ElementScriptTokens {
            kind: "sketch_slice_op",
            index: ordinal_or_slot(doc.map(|d| d.sketch_slice_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The text's arena slot, not its ordinal (#1070).
        SceneElement::SketchText(i) => ElementScriptTokens {
            kind: "sketch_text",
            index: ordinal_or_slot(doc.map(|d| d.sketch_texts.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::SliceOp(i) => ElementScriptTokens {
            kind: "slice_op",
            index: ordinal_or_slot(doc.map(|d| d.slice_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::ShellOp(i) => ElementScriptTokens {
            kind: "shell_op",
            index: ordinal_or_slot(doc.map(|d| d.shell_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The op's arena slot, not its ordinal (#1070).
        SceneElement::EdgeTreatmentOp(i) => ElementScriptTokens {
            kind: "edge_treatment_op",
            index: ordinal_or_slot(doc.map(|d| d.edge_treatment_ops.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The revolve's arena slot, not its ordinal (#1070).
        SceneElement::Revolution(i) => ElementScriptTokens {
            kind: "revolution",
            index: ordinal_or_slot(doc.map(|d| d.revolutions.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The shape's arena slot, not its ordinal (#1070).
        SceneElement::Shape(i) => ElementScriptTokens {
            kind: "shape",
            index: ordinal_or_slot(doc.map(|d| d.primitives.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The sweep's arena slot, not its ordinal (#1070).
        SceneElement::SweepOp(i) => ElementScriptTokens {
            kind: "sweep",
            index: ordinal_or_slot(doc.map(|d| d.sweeps.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Loft(i) => ElementScriptTokens {
            kind: "loft",
            index: ordinal_or_slot(doc.map(|d| d.lofts.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Drawing(i) => ElementScriptTokens {
            kind: "drawing",
            index: ordinal_or_slot(doc.map(|d| d.drawings.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The component's arena slot, not its ordinal (#1070).
        SceneElement::Component(i) => ElementScriptTokens {
            kind: "component",
            index: ordinal_or_slot(doc.map(|d| d.components.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The joint's arena slot, not its ordinal (#1070).
        SceneElement::Joint(i) => ElementScriptTokens {
            kind: "joint",
            index: ordinal_or_slot(doc.map(|d| d.joints.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        // The instance's arena slot, not its ordinal (#1070).
        SceneElement::UnitInstance(i) => ElementScriptTokens {
            kind: "unit_instance",
            index: ordinal_or_slot(doc.map(|d| d.unit_instances.keys().position(|k| k == i)), i.index()),
            point: None,
        },
        SceneElement::Origin => ElementScriptTokens {
            kind: "origin",
            index: 0,
            point: None,
        },
        // The world axes script by name, not index (#952): `axis`/0,1,2 for X/Y/Z.
        SceneElement::GlobalAxis(axis) => ElementScriptTokens {
            kind: "axis",
            index: match axis {
                crate::construction::GlobalAxis::X => 0,
                crate::construction::GlobalAxis::Y => 1,
                crate::construction::GlobalAxis::Z => 2,
            },
            point: None,
        },
    }
}

fn geometric_constraint_script_name(
    kind: crate::geometric_constraints::GeometricConstraintType,
) -> &'static str {
    use crate::geometric_constraints::GeometricConstraintType;
    match kind {
        GeometricConstraintType::Parallel => "parallel",
        GeometricConstraintType::Perpendicular => "perpendicular",
        GeometricConstraintType::Equal => "equal",
        GeometricConstraintType::Coincident => "coincident",
        GeometricConstraintType::Midpoint => "midpoint",
        GeometricConstraintType::AlongXAxis => "horizontal",
        GeometricConstraintType::AlongYAxis => "vertical",
    }
}

/// Map an applied [`Action`] to a script [`Instruction`] when one exists.
/// A joint's ordinal among the live ones — what a script writes (#1055).
fn joint_ordinal(doc: &crate::model::Document, key: crate::model::JointKey) -> Option<usize> {
    doc.joints.keys().position(|k| k == key)
}

/// The joint an ordinal names — the inverse of [`joint_ordinal`].
fn joint_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::JointKey> {
    doc.joints.keys().nth(ordinal)
}

/// An extrusion's ordinal among the live ones — what a script writes (#1055).
fn extrusion_ordinal(
    doc: &crate::model::Document,
    key: crate::model::ExtrusionKey,
) -> Option<usize> {
    doc.extrusions.keys().position(|k| k == key)
}

/// The extrusion an ordinal names — the inverse of [`extrusion_ordinal`].
fn extrusion_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::ExtrusionKey> {
    doc.extrusions.keys().nth(ordinal)
}

fn primitive_ordinal(
    doc: &crate::model::Document,
    key: crate::model::PrimitiveKey,
) -> Option<usize> {
    doc.primitives.keys().position(|k| k == key)
}

fn primitive_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::PrimitiveKey> {
    doc.primitives.keys().nth(ordinal)
}

/// A line's ordinal among the live ones — what a script writes (#1055).
fn line_ordinal(doc: &crate::model::Document, key: crate::model::LineKey) -> Option<usize> {
    doc.lines.keys().position(|k| k == key)
}

/// The line an ordinal names — the inverse of [`line_ordinal`].
fn line_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::LineKey> {
    doc.lines.keys().nth(ordinal)
}

/// A construction plane's ordinal among the live ones — what a script writes (#1055).
fn plane_ordinal(
    doc: &crate::model::Document,
    key: crate::model::ConstructionPlaneKey,
) -> Option<usize> {
    doc.construction_planes.keys().position(|k| k == key)
}

/// The plane an ordinal names — the inverse of [`plane_ordinal`].
fn plane_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::ConstructionPlaneKey> {
    doc.construction_planes.keys().nth(ordinal)
}

/// A sketch's ordinal among the live ones — what a script writes (#1055).
fn sketch_ordinal(doc: &crate::model::Document, key: crate::model::SketchId) -> Option<usize> {
    doc.sketches.keys().position(|k| k == key)
}

/// The sketch an ordinal names — the inverse of [`sketch_ordinal`].
fn sketch_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::SketchId> {
    doc.sketches.keys().nth(ordinal)
}

/// A unit's ordinal among the live ones — what a script writes (#1055).
fn unit_ordinal(doc: &crate::model::Document, key: crate::model::UnitKey) -> Option<usize> {
    doc.units.keys().position(|k| k == key)
}

/// The unit an ordinal names — the inverse of [`unit_ordinal`].
fn unit_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::UnitKey> {
    doc.units.keys().nth(ordinal)
}

/// The drawing an ordinal names (#1055) — a script counts the live drawings.
fn drawing_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::DrawingKey> {
    doc.drawings.keys().nth(ordinal)
}

/// A component's ordinal among the live ones — what a script writes (#1055).
fn component_ordinal(
    doc: &crate::model::Document,
    key: crate::model::ComponentKey,
) -> Option<usize> {
    doc.components.keys().position(|k| k == key)
}

/// The component an ordinal names — the inverse of [`component_ordinal`].
fn component_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::ComponentKey> {
    doc.components.keys().nth(ordinal)
}

/// A unit instance's ordinal among the live ones — what a script writes (#1055).
fn unit_instance_ordinal(
    doc: &crate::model::Document,
    key: crate::model::UnitInstanceKey,
) -> Option<usize> {
    doc.unit_instances.keys().position(|k| k == key)
}

/// The unit instance an ordinal names — the inverse of [`unit_instance_ordinal`].
fn unit_instance_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::UnitInstanceKey> {
    doc.unit_instances.keys().nth(ordinal)
}

/// A slice operation's ordinal among the live ones — what a script writes (#1055).
fn slice_op_ordinal(doc: &crate::model::Document, key: crate::model::SliceOpKey) -> Option<usize> {
    doc.slice_ops.keys().position(|k| k == key)
}

/// The slice operation an ordinal names — the inverse of [`slice_op_ordinal`].
fn slice_op_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::SliceOpKey> {
    doc.slice_ops.keys().nth(ordinal)
}

/// A shell operation's ordinal among the live ones (#1156).
fn shell_op_ordinal(doc: &crate::model::Document, key: crate::model::ShellOpKey) -> Option<usize> {
    doc.shell_ops.keys().position(|k| k == key)
}

/// The shell operation an ordinal names — the inverse of [`shell_op_ordinal`].
fn shell_op_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::ShellOpKey> {
    doc.shell_ops.keys().nth(ordinal)
}

/// A repeat's ordinal among the live ones — what a script writes (#1055).
fn repeat_op_ordinal(
    doc: &crate::model::Document,
    key: crate::model::RepeatOpKey,
) -> Option<usize> {
    doc.repeat_ops.keys().position(|k| k == key)
}

/// The repeat an ordinal names — the inverse of [`repeat_op_ordinal`].
fn repeat_op_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::RepeatOpKey> {
    doc.repeat_ops.keys().nth(ordinal)
}

/// A mirror operation's ordinal among the live ones — what a script writes (#1055).
fn mirror_op_ordinal(
    doc: &crate::model::Document,
    key: crate::model::MirrorOpKey,
) -> Option<usize> {
    doc.mirror_ops.keys().position(|k| k == key)
}

/// The mirror operation an ordinal names — the inverse of [`mirror_op_ordinal`].
fn mirror_op_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::MirrorOpKey> {
    doc.mirror_ops.keys().nth(ordinal)
}

/// A move operation's ordinal among the live ones — what a script writes (#1055).
fn move_op_ordinal(doc: &crate::model::Document, key: crate::model::MoveOpKey) -> Option<usize> {
    doc.move_ops.keys().position(|k| k == key)
}

/// The move operation an ordinal names — the inverse of [`move_op_ordinal`].
fn move_op_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::MoveOpKey> {
    doc.move_ops.keys().nth(ordinal)
}

/// A boolean operation's ordinal among the live ones — what a script writes (#1055).
fn boolean_op_ordinal(
    doc: &crate::model::Document,
    key: crate::model::BooleanOpKey,
) -> Option<usize> {
    doc.boolean_ops.keys().position(|k| k == key)
}

/// The boolean operation an ordinal names — the inverse of [`boolean_op_ordinal`].
fn boolean_op_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::BooleanOpKey> {
    doc.boolean_ops.keys().nth(ordinal)
}

/// A body's ordinal among the live ones — what a script writes (#1055).
fn body_ordinal(doc: &crate::model::Document, key: crate::model::BodyKey) -> Option<usize> {
    doc.bodies.keys().position(|k| k == key)
}

/// The same for a whole list; `None` if any entry has stopped resolving.
fn body_ordinals(
    doc: &crate::model::Document,
    keys: &[crate::model::BodyKey],
) -> Option<Vec<usize>> {
    keys.iter().map(|k| body_ordinal(doc, *k)).collect()
}

/// The body an ordinal names — the inverse of [`body_ordinal`].
fn body_key(doc: &crate::model::Document, ordinal: usize) -> Option<crate::model::BodyKey> {
    doc.body_at(ordinal)
}

/// The same for a whole list, dropping ordinals that name nothing.
fn body_keys(doc: &crate::model::Document, ordinals: &[usize]) -> Vec<crate::model::BodyKey> {
    ordinals.iter().filter_map(|o| body_key(doc, *o)).collect()
}

/// Split Move targets into body `targets` and unit `instance_targets` (#1406): a unit's
/// materialized body moves as its instance, the same way the GUI routes it, so the
/// geometry stays nested under the imported unit rather than producing a detached output.
fn split_move_bodies(
    doc: &crate::model::Document,
    bodies: &[crate::model::BodyKey],
) -> (Vec<crate::model::BodyKey>, Vec<crate::model::UnitInstanceKey>) {
    use crate::model::BodySource;
    let mut body_targets: Vec<crate::model::BodyKey> = Vec::new();
    let mut instances: Vec<crate::model::UnitInstanceKey> = Vec::new();
    for &bi in bodies {
        match doc.bodies.get(bi).map(|b| &b.source) {
            Some(BodySource::UnitInstance(ui)) => {
                if !instances.contains(ui) {
                    instances.push(*ui);
                }
            }
            _ => {
                if !body_targets.contains(&bi) {
                    body_targets.push(bi);
                }
            }
        }
    }
    (body_targets, instances)
}

/// A parameter's ordinal among the live ones — what a script writes (#1055).
fn parameter_ordinal(
    doc: &crate::model::Document,
    key: crate::model::ParameterKey,
) -> Option<usize> {
    doc.parameters.keys().position(|k| k == key)
}

/// The parameter an ordinal names — the inverse of [`parameter_ordinal`].
fn parameter_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::ParameterKey> {
    doc.parameters.keys().nth(ordinal)
}

/// A tracing image's ordinal among the live ones — what a script writes (#1055).
fn image_ordinal(
    doc: &crate::model::Document,
    key: crate::model::TracingImageKey,
) -> Option<usize> {
    doc.tracing_images.keys().position(|k| k == key)
}

/// The image an ordinal names — the inverse of [`image_ordinal`].
fn image_key(
    doc: &crate::model::Document,
    ordinal: usize,
) -> Option<crate::model::TracingImageKey> {
    doc.tracing_images.keys().nth(ordinal)
}

fn image_keys(
    doc: &crate::model::Document,
    ordinals: &[usize],
) -> Vec<crate::model::TracingImageKey> {
    ordinals.iter().filter_map(|o| image_key(doc, *o)).collect()
}

fn image_ordinals(
    doc: &crate::model::Document,
    keys: &[crate::model::TracingImageKey],
) -> Option<Vec<usize>> {
    keys.iter().map(|k| image_ordinal(doc, *k)).collect()
}

pub fn instruction_from_action(action: &Action, doc: &crate::model::Document) -> Option<Instruction> {
    use crate::actions::dim_label_axis_for_target;
    match action {
        Action::CreateBooleanOperation { kind, a, b, keep_b, solid_count: _ } => {
            Some(Instruction::CreateBooleanOp {
                kind: *kind,
                a: body_ordinals(doc, a)?,
                b: body_ordinals(doc, b)?,
                keep_b: *keep_b,
            })
        }
        Action::EditBooleanOperation { op, kind, a, b, keep_b } => {
            Some(Instruction::EditBooleanOp {
                op: boolean_op_ordinal(doc, *op)?,
                kind: *kind,
                a: body_ordinals(doc, a)?,
                b: body_ordinals(doc, b)?,
                keep_b: *keep_b,
            })
        }
        Action::CreateMoveOperation { targets, image_targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c, .. } => {
            Some(Instruction::CreateMoveOp {
                targets: body_ordinals(doc, targets)?,
                images: image_ordinals(doc, image_targets)?,
                tx: tx.clone(),
                ty: ty.clone(),
                tz: tz.clone(),
                rx: rx.clone(),
                ry: ry.clone(),
                rz: rz.clone(),
                roll_angle: roll_angle.clone(),
                face_flip: *face_flip,
                face_spin: face_spin.clone(),
                face_offset: face_offset.clone(),
                start_point_a: *start_point_a,
                end_point_a: *end_point_a,
                start_point_b: *start_point_b,
                end_point_b: *end_point_b,
                start_point_c: *start_point_c,
                end_point_c: *end_point_c,
            })
        }
        Action::EditMoveOperation { op, targets, image_targets, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c, .. } => {
            Some(Instruction::EditMoveOp {
                op: move_op_ordinal(doc, *op)?,
                targets: body_ordinals(doc, targets)?,
                images: image_ordinals(doc, image_targets)?,
                tx: tx.clone(),
                ty: ty.clone(),
                tz: tz.clone(),
                rx: rx.clone(),
                ry: ry.clone(),
                rz: rz.clone(),
                roll_angle: roll_angle.clone(),
                face_flip: *face_flip,
                face_spin: face_spin.clone(),
                face_offset: face_offset.clone(),
                start_point_a: *start_point_a,
                end_point_a: *end_point_a,
                start_point_b: *start_point_b,
                end_point_b: *end_point_b,
                start_point_c: *start_point_c,
                end_point_c: *end_point_c,
            })
        }
        Action::CreateMirrorOperation { plane, targets, mode } => Some(Instruction::CreateMirrorOp {
            plane: plane.clone(),
            targets: body_ordinals(doc, targets)?,
            mode: *mode,
        }),
        Action::EditMirrorOperation { op, plane, targets, mode } => Some(Instruction::EditMirrorOp {
            op: mirror_op_ordinal(doc, *op)?,
            plane: plane.clone(),
            targets: body_ordinals(doc, targets)?,
            mode: *mode,
        }),
        // The scripting Instruction DSL doesn't carry plane targets (#221), same as it omits
        // the Move op's plane/image targets — they replay as body-only operations.
        Action::CreateRepeatOperation { targets, plane_targets: _, extrusion_targets: _, sketch_targets: _, axis, path_circle: _, around_axis, flip, mode, count, spacing, length, length_target } => {
            Some(Instruction::CreateRepeatOp {
                targets: body_ordinals(doc, targets)?,
                axis: *axis,
                around_axis: *around_axis,
                flip: *flip,
                mode: *mode,
                count: count.clone(),
                spacing: spacing.clone(),
                length: length.clone(),
                length_target: length_target.clone(),
            })
        }
        Action::EditRepeatOperation { op, targets, plane_targets: _, extrusion_targets: _, sketch_targets: _, axis, path_circle: _, around_axis, flip, mode, count, spacing, length, length_target } => {
            Some(Instruction::EditRepeatOp {
                op: repeat_op_ordinal(doc, *op)?,
                targets: body_ordinals(doc, targets)?,
                axis: *axis,
                around_axis: *around_axis,
                flip: *flip,
                mode: *mode,
                count: count.clone(),
                spacing: spacing.clone(),
                length: length.clone(),
                length_target: length_target.clone(),
            })
        }
        Action::CreateJointOperation { members, base, kind, placement, frame, position, position2, position3, limits } => {
            Some(Instruction::CreateJointOp {
                members: members.clone(),
                base: *base,
                kind: kind.clone(),
                placement: placement.clone(),
                frame: frame.clone(),
                position: position.clone(),
                position2: position2.clone(),
                position3: position3.clone(),
                limits: limits.clone(),
            })
        }
        Action::EditJointOperation { op, members, base, kind, placement, frame, position, position2, position3, limits } => {
            Some(Instruction::EditJointOp {
                op: joint_ordinal(doc, *op)?,
                members: members.clone(),
                base: *base,
                kind: kind.clone(),
                placement: placement.clone(),
                frame: frame.clone(),
                position: position.clone(),
                position2: position2.clone(),
                position3: position3.clone(),
                limits: limits.clone(),
            })
        }
        Action::SetJointRest { joint } => Some(Instruction::SetJointRest {
            op: joint_ordinal(doc, *joint)?,
        }),
        Action::RevertJoint { joint } => Some(Instruction::RevertJoint {
            op: joint_ordinal(doc, *joint)?,
        }),
        Action::RevertAllJoints => Some(Instruction::RevertAllJoints),
        Action::CreateSliceOperation { targets, cutters, extend_infinite } => {
            Some(Instruction::CreateSliceOp {
                targets: body_ordinals(doc, targets)?,
                cutters: cutters.clone(),
                extend_infinite: *extend_infinite,
            })
        }
        Action::EditSliceOperation { op, targets, cutters, extend_infinite } => {
            Some(Instruction::EditSliceOp {
                op: slice_op_ordinal(doc, *op)?,
                targets: body_ordinals(doc, targets)?,
                cutters: cutters.clone(),
                extend_infinite: *extend_infinite,
            })
        }
        Action::CreateShellOperation {
            targets,
            open_faces,
            thickness,
        } => Some(Instruction::CreateShellOp {
            targets: body_ordinals(doc, targets)?,
            open_faces: open_faces.clone(),
            thickness: thickness.clone(),
        }),
        Action::EditShellOperation {
            op,
            targets,
            open_faces,
            thickness,
        } => Some(Instruction::EditShellOp {
            op: shell_op_ordinal(doc, *op)?,
            targets: body_ordinals(doc, targets)?,
            open_faces: open_faces.clone(),
            thickness: thickness.clone(),
        }),
        Action::NewDocument => Some(Instruction::New),
        Action::Open { path } => Some(Instruction::Open(path.clone())),
        Action::Save { path } => Some(Instruction::Save(path.clone())),
        Action::ForceRebuildGeometry => Some(Instruction::RebuildGeometry),
        Action::ExportStl { path, body } => Some(Instruction::ExportStl {
            path: path.clone(),
            body: body.clone(),
        }),
        Action::Export3mf { path, body } => Some(Instruction::Export3mf {
            path: path.clone(),
            body: body.clone(),
        }),
        Action::ExportStep { path, body } => Some(Instruction::ExportStep {
            path: path.clone(),
            body: body.clone(),
        }),
        Action::ImportStl { path } => Some(Instruction::ImportStl { path: path.clone() }),
        Action::ImportUnit { path, link, name } => Some(Instruction::ImportUnit {
            path: path.clone(),
            link: *link,
            name: name.clone(),
        }),
        Action::ImportImage { path, plane } => Some(Instruction::ImportImage {
            path: path.clone(),
            plane: match plane {
                Some(p) => Some(plane_ordinal(doc, *p)?),
                None => None,
            },
        }),
        Action::SetCalibrationPoint { image, index, x, y } => {
            Some(Instruction::SetCalibrationPoint {
                image: image_ordinal(doc, *image)?,
                index: *index,
                x: *x,
                y: *y,
            })
        }
        Action::RemoveCalibrationPoint { image, index } => {
            Some(Instruction::RemoveCalibrationPoint {
                image: image_ordinal(doc, *image)?,
                index: *index,
            })
        }
        Action::CalibrateImage { image, a, b, length, expression } => {
            Some(Instruction::CalibrateImage {
                image: image_ordinal(doc, *image)?,
                a: Some(*a),
                b: Some(*b),
                length: *length,
                expression: expression.clone(),
            })
        }
        Action::SetImageOpacity { image, opacity } => Some(Instruction::SetImageOpacity {
            image: image_ordinal(doc, *image)?,
            opacity: *opacity,
            expression: String::new(),
        }),
        Action::ImportStep { path } => Some(Instruction::ImportStep { path: path.clone() }),
        Action::UpdateExtrusion { extrusion, distance, target, expression } => {
            Some(Instruction::UpdateExtrusion {
                extrusion: extrusion_ordinal(doc, *extrusion)?,
                distance: *distance,
                target: target.clone(),
                expression: expression.clone(),
            })
        }
        Action::ToggleFpsMode => Some(Instruction::FpsMode { on: None }),
        Action::Clear => Some(Instruction::Clear),
        Action::UndoLast => Some(Instruction::Undo),
        Action::CopySelection => Some(Instruction::CopySelection),
        Action::PasteAt { linked, offset } => Some(Instruction::PasteAt {
            linked: *linked,
            x: offset.x,
            y: offset.y,
            z: offset.z,
        }),
        Action::SetTool(tool) => Some(Instruction::Tool(*tool)),
        // The interactive draw tools commit straight to `doc` without going through the
        // declarative Create*/Extrude actions (#59); replay them as the equivalent call
        // using the as-committed geometry. A failed commit (e.g. "too small") returns
        // `ActionResult::Err`, so `after_apply` never reaches here for those.
        // A rectangle is now four plain lines (#66 polygon); reconstruct its origin/extent
        // from the bounding box of the four lines just appended by the commit.
        Action::CommitRectangle => {
            let keys: Vec<_> = doc.lines.keys().collect();
            let n = keys.len();
            (n >= 4).then(|| {
                let rect_keys = &keys[n - 4..];
                let rect_lines: Vec<_> = rect_keys.iter().map(|&k| &doc.lines[k]).collect();
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for l in rect_lines {
                    for (x, y) in [(l.x0, l.y0), (l.x1, l.y1)] {
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                }
                // Typed width/height land as LineLength dims on the bottom (n-4) and right
                // (n-3) edges; carry their expressions so a parametric rect replays
                // parametrically (#402).
                let dim_expr = |line: crate::model::LineKey| {
                    doc.constraints.values().collect::<Vec<_>>().into_iter().rev().find_map(|c| match &c.kind {
                        crate::model::ConstraintKind::Distance {
                            target: crate::model::DistanceTarget::LineLength(i),
                        } if *i == line => Some(c.expression.clone()),
                        _ => None,
                    })
                };
                Instruction::CreateRect {
                    x: min_x,
                    y: min_y,
                    width: max_x - min_x,
                    height: max_y - min_y,
                    width_expr: dim_expr(rect_keys[0]),
                    height_expr: dim_expr(rect_keys[1]),
                }
            })
        }
        Action::CommitLine => doc.lines.keys().last().map(|index| {
            let l = &doc.lines[index];
            // A typed-while-drawing length lands as a LineLength dim inside CommitLine;
            // carry its expression so replaying the log recreates the same constraint
            // (and click-drawn lines replay unconstrained, as drawn).
            let dimension = doc.constraints.values().collect::<Vec<_>>().into_iter().rev().find_map(|c| match &c.kind {
                crate::model::ConstraintKind::Distance {
                    target: crate::model::DistanceTarget::LineLength(i),
                } if *i == index => Some(c.expression.clone()),
                _ => None,
            });
            Instruction::CreateLine {
                x0: l.x0,
                y0: l.y0,
                x1: l.x1,
                y1: l.y1,
                bezier: l.bezier,
                dimension,
            }
        }),
        Action::CommitCircle => doc.circles.keys().last().map(|index| {
            let c = &doc.circles[index];
            // Carry a typed diameter's expression like CommitLine does (#402).
            let diameter_expr = doc.constraints.values().collect::<Vec<_>>().into_iter().rev().find_map(|c| match &c.kind {
                crate::model::ConstraintKind::Distance {
                    target: crate::model::DistanceTarget::CircleDiameter(i),
                } if *i == index => Some(c.expression.clone()),
                _ => None,
            });
            Instruction::CreateCircle {
                cx: c.cx,
                cy: c.cy,
                r: c.r,
                diameter_expr,
            }
        }),
        Action::SetRectDimension { axis, value } => Some(Instruction::SetDim {
            axis: *axis,
            value: value.clone(),
        }),
        Action::FocusRectDimension { axis } => Some(Instruction::FocusDim(*axis)),
        Action::SetLineLength { value } => Some(Instruction::SetLineLength {
            value: value.clone(),
        }),
        Action::FocusLineLength => Some(Instruction::FocusLineLength),
        Action::SetCircleDiameter { value } => Some(Instruction::SetCircleDiameter {
            value: value.clone(),
        }),
        Action::FocusCircleDiameter => Some(Instruction::FocusCircleDiameter),
        Action::SetDimLabelOffset { target, offset } => {
            dim_label_axis_for_target(doc, *target).map(|axis| {
                Instruction::SetDimLabelOffset {
                    axis,
                    offset: *offset,
                }
            })
        }
        Action::BeginEditCommittedDim { target } => {
            dim_label_axis_for_target(doc, *target).map(|axis| {
                Instruction::BeginEditCommittedDim { axis }
            })
        }
        Action::CommitCommittedDim => Some(Instruction::CommitCommittedDim),
        Action::BeginEditConstructionPlane { index } => {
            Some(Instruction::BeginEditConstructionPlane { index: plane_ordinal(doc, *index)? })
        }
        Action::CommitConstructionPlane => Some(Instruction::CommitConstructionPlane),
        Action::SetPlaneOffset { value } => Some(Instruction::SetPlaneOffset {
            value: value.clone(),
        }),
        Action::SetPlaneAngle { value } => Some(Instruction::SetPlaneAngle {
            value: value.clone(),
        }),
        Action::FocusPlaneDim { dim } => Some(Instruction::FocusPlaneDim(*dim)),
        Action::BeginSketch { face, .. } => Some(Instruction::BeginSketch { face: face.clone() }),
        Action::OpenSketch { sketch, .. } => Some(Instruction::OpenSketch {
            sketch: sketch_ordinal(doc, *sketch)?,
        }),
        Action::ExitSketch => Some(Instruction::ExitSketch),
        Action::SetElementVisible { element, visible } => Some(Instruction::SetElementVisible {
            element: element.clone(),
            visible: Some(*visible),
        }),
        Action::ToggleElementVisibility(element) => Some(Instruction::SetElementVisible {
            element: element.clone(),
            visible: None,
        }),
        Action::SetHomeView => Some(Instruction::SetHomeView),
        Action::SetElementsViewMode { mode } => {
            Some(Instruction::SetElementsView { mode: *mode })
        }
        Action::SetPaneVisible { pane, visible } => Some(Instruction::SetPane {
            pane: *pane,
            visible: Some(*visible),
        }),
        Action::TogglePane(pane) => Some(Instruction::SetPane {
            pane: *pane,
            visible: None,
        }),
        Action::AddParameter { name, expression } => Some(Instruction::AddParameter {
            name: name.clone(),
            expression: expression.clone(),
        }),
        Action::CreateDerivedParameter { source, name } => {
            Some(Instruction::CreateDerivedParameter {
                source: source.clone(),
                name: name.clone(),
            })
        }
        Action::CreateParameterFromLineLength { line_index, name } => {
            Some(Instruction::CreateParameterFromLineLength {
                line_index: line_ordinal(doc, *line_index)?,
                name: name.clone(),
            })
        }
        Action::SetParameterPrimary { index, primary } => {
            Some(Instruction::SetParameterPrimary {
                index: parameter_ordinal(doc, *index)?,
                primary: *primary,
            })
        }
        Action::SetParameterBound { index, which, expression } => {
            Some(Instruction::SetParameterBound {
                index: parameter_ordinal(doc, *index)?,
                which: *which,
                expression: expression.clone(),
            })
        }
        Action::SetUnitParameterOverride { instance, name, expression } => {
            Some(Instruction::SetUnitParameterOverride {
                instance: unit_instance_ordinal(doc, *instance)?,
                name: name.clone(),
                expression: expression.clone(),
            })
        }
        Action::SyncUnit { unit } => {
            Some(Instruction::SyncUnit { unit: unit_ordinal(doc, *unit)? })
        }
        Action::SetUnitLink { unit, link } => {
            Some(Instruction::SetUnitLink { unit: unit_ordinal(doc, *unit)?, link: *link })
        }
        Action::AddUnitInstance { unit, name } => {
            Some(Instruction::AddUnitInstance {
                unit: unit_ordinal(doc, *unit)?,
                name: name.clone(),
            })
        }
        Action::CloneUnitInstance { instance } => {
            Some(Instruction::CloneUnitInstance { instance: unit_instance_ordinal(doc, *instance)? })
        }
        Action::SetSettingsWindow { open } => Some(Instruction::SetSettingsWindow { open: *open }),
        Action::SetChangelogWindow { open } => Some(Instruction::SetChangelogWindow { open: *open }),
        Action::SetTutorialPane { open } => Some(Instruction::SetTutorialPane { open: *open }),
        Action::CompleteAllTutorials => Some(Instruction::CompleteAllTutorials),
        Action::UnstartAllTutorials => Some(Instruction::UnstartAllTutorials),
        Action::SetMcMasterWindow { open, part } => Some(Instruction::SetMcMasterWindow {
            open: *open,
            part: part.clone(),
        }),
        Action::SetReportIssueWindow { open } => {
            Some(Instruction::SetReportIssueWindow { open: *open })
        },
        Action::CommitParameterName { index, name } => Some(Instruction::SetParameterName {
            index: parameter_ordinal(doc, *index)?,
            name: name.clone(),
        }),
        Action::CommitParameterExpression { index, expression } => {
            Some(Instruction::SetParameterExpression {
                index: parameter_ordinal(doc, *index)?,
                expression: expression.clone(),
            })
        }
        Action::DeleteParameter { index } => Some(Instruction::DeleteParameter {
            index: parameter_ordinal(doc, *index)?,
        }),
        Action::DeleteSelection => Some(Instruction::DeleteSelection),
        Action::SetCommandPaletteOpen { open } => Some(Instruction::SetCommandPalette {
            open: Some(*open),
        }),
        Action::ToggleCommandPalette => Some(Instruction::SetCommandPalette { open: None }),
        Action::ClickSceneElement { element, additive } => Some(Instruction::SelectSceneElement {
            element: element.clone(),
            additive: *additive,
        }),
        Action::ClearSceneSelection => Some(Instruction::ClearSceneSelection),
        Action::SetShapeConstruction {
            element,
            construction,
        } => Some(Instruction::SetShapeConstruction {
            element: element.clone(),
            construction: *construction,
        }),
        Action::ApplyConstruction { construction } => Some(Instruction::ApplyConstruction {
            construction: *construction,
        }),
        Action::ToggleConstruction => Some(Instruction::ToggleConstruction),
        Action::ApplySelectionVisibility { visible } => {
            Some(Instruction::ApplySelectionVisibility {
                visible: *visible,
            })
        }
        Action::ToggleSelectionVisibility => Some(Instruction::ToggleSelectionVisibility),
        Action::AddGeometricConstraint(kind) => Some(Instruction::AddGeometricConstraint(*kind)),
        Action::ApplyConstraintShortcut(key) => Some(Instruction::ApplyConstraintShortcut(*key)),
        Action::DragVertex { point, u, v } => Some(Instruction::DragVertex {
            point: point.clone(),
            u: *u,
            v: *v,
        }),
        Action::CommitElementName { element, name } => Some(Instruction::SetElementName {
            element: element.clone(),
            name: name.clone(),
        }),
        Action::FocusElementName => Some(Instruction::FocusElementName),
        Action::SetDocumentUnits { length, angle } => {
            Some(Instruction::SetDocumentUnits { length: *length, angle: *angle })
        }
        Action::CreateComponent { name, parent } => Some(Instruction::CreateComponent {
            name: name.clone(),
            parent: match parent {
                Some(p) => Some(component_ordinal(doc, *p)?),
                None => None,
            },
        }),
        Action::MoveToComponent { element, component } => Some(Instruction::MoveToComponent {
            element: element.clone(),
            component: match component {
                Some(c) => Some(component_ordinal(doc, *c)?),
                None => None,
            },
        }),
        Action::SetComponentUnits { component, length, angle } => {
            Some(Instruction::SetComponentUnits {
                component: component_ordinal(doc, *component)?,
                length: *length,
                angle: *angle,
            })
        }
        Action::SetSketchUnits { sketch, length, angle } => Some(Instruction::SetSketchUnits {
            sketch: sketch_ordinal(doc, *sketch)?,
            length: *length,
            angle: *angle,
        }),
        Action::CommitVertexTreatment { point, kind, amount } => {
            Some(Instruction::VertexTreatment {
                points: vec![point.clone()],
                kind: *kind,
                amount: amount.clone(),
            })
        }
        Action::ZoomToFit => Some(Instruction::ZoomFit),
        Action::CommitEdgeTreatments { edges, kind, amount, expression } => Some(Instruction::EdgeTreatment {
            edges: edges
                .iter()
                .map(|(solid, edge)| {
                    let host = match solid {
                        crate::model::TreatableSolid::Extrusion(e) => {
                            TreatableSolidRef::Extrusion(extrusion_ordinal(doc, *e)?)
                        }
                        crate::model::TreatableSolid::Primitive(p) => {
                            TreatableSolidRef::Primitive(primitive_ordinal(doc, *p)?)
                        }
                    };
                    Some((host, *edge))
                })
                .collect::<Option<Vec<_>>>()?,
            kind: *kind,
            amount: *amount,
            expression: expression.clone(),
        }),
        Action::ProjectSelection => Some(Instruction::Project { elements: vec![] }),
        Action::ProjectElement { element } => Some(Instruction::Project {
            elements: vec![element.clone()],
        }),
        Action::ProjectSources { sources } => Some(Instruction::Project {
            elements: sources
                .iter()
                .filter_map(|source| match source {
                    crate::model::ProjectionSource::BodyEdge { body, a, b } => {
                        Some(SceneElement::BodyEdge {
                            body: *body,
                            a: *a,
                            b: *b,
                        })
                    }
                    crate::model::ProjectionSource::Plane { plane } => {
                        Some(SceneElement::ConstructionPlane(*plane))
                    }
                    crate::model::ProjectionSource::UnitEdge { .. } => None,
                })
                .collect(),
        }),
        _ => None,
    }
}

/// Replay instructions for a constraint added as a side effect of committing sketch geometry
/// (e.g. a line endpoint snapping onto an existing vertex/line while drawing, #37/#41) —
/// `crate::actions::AppState::add_snap_constraint` mutates `doc.constraints` directly, without
/// going through `Action::AddGeometricConstraint`, so the command log otherwise has nothing to
/// replay it with. Mirrors the "select both, then apply" flow the constraint pane itself uses:
/// `bearcad.select(...)` for each side (second call `additive`), then
/// `bearcad.add_geometric_constraint(...)`. Best-effort — a `ConstraintEntity::Origin` side (the
/// sketch origin, #21) isn't a selectable `SceneElement`, so that case (and any kind without a
/// direct `GeometricConstraintType`) returns `None` rather than emitting an unreplayable stub.
pub fn instructions_for_snap_constraint(kind: &crate::model::ConstraintKind) -> Option<Vec<Instruction>> {
    use crate::geometric_constraints::GeometricConstraintType;
    use crate::model::{ConstraintEntity, ConstraintKind};

    fn element_for_entity(entity: &ConstraintEntity) -> Option<SceneElement> {
        match entity {
            ConstraintEntity::Point(point) => Some(SceneElement::Point(point.clone())),
            ConstraintEntity::Line(ConstraintLine::Line(index)) => Some(SceneElement::Line(*index)),
            ConstraintEntity::Line(
                line @ (ConstraintLine::FaceEdge { .. }
                | ConstraintLine::OriginAxis(_)
                | ConstraintLine::ImageEdge { .. }),
            ) => Some(SceneElement::FaceEdge(line.clone())),
            ConstraintEntity::Circle(index) => Some(SceneElement::Circle(*index)),
            ConstraintEntity::Origin => None,
        }
    }

    let (a, b, geometric_kind) = match kind {
        ConstraintKind::Coincident { a, b } => (
            element_for_entity(a)?,
            element_for_entity(b)?,
            GeometricConstraintType::Coincident,
        ),
        ConstraintKind::Midpoint { point, line } => (
            SceneElement::Point(point.clone()),
            element_for_entity(&ConstraintEntity::Line(line.clone()))?,
            GeometricConstraintType::Midpoint,
        ),
        _ => return None,
    };
    Some(vec![
        Instruction::SelectSceneElement { element: a, additive: false },
        Instruction::SelectSceneElement { element: b, additive: true },
        Instruction::AddGeometricConstraint(geometric_kind),
    ])
}

/// Build a replayable `Instruction::Extrude` for the extrusion the interactive Extrude tool
/// just created (the last entry in `doc.extrusions`). Used by the command log instead of
/// `instruction_from_action`, since `Action::CommitExtrusion` carries no fields to read the
/// committed faces/distance/body choice from — only `doc`'s post-commit state has them (#59).
pub fn instruction_for_new_extrusion(doc: &crate::model::Document) -> Option<Instruction> {
    let ei = doc.extrusions.keys().last()?;
    let extrusion = doc.extrusions.get(ei)?;
    let body = match crate::model::body_index_for_extrusion(doc, ei).and_then(|bi| doc.bodies.get(bi))
    {
        // Subtracted from its body → a cut (#35).
        Some(body) if body.source.cut_extrusion_indices().contains(&ei) => {
            crate::actions::ExtrudeBodyChoice::Cut
        }
        // Added alongside other extrusions → merged into an existing body (#32).
        Some(body) if body.source.extrusion_indices().len() > 1 => {
            crate::actions::ExtrudeBodyChoice::Merge
        }
        _ => crate::actions::ExtrudeBodyChoice::New,
    };
    Some(Instruction::Extrude {
        sketch: sketch_ordinal(doc, extrusion.sketch)?,
        faces: extrusion.faces.clone(),
        distance: extrusion.distance,
        body,
        target: extrusion.target.clone(),
        expression: (!extrusion.expression.trim().is_empty())
            .then(|| extrusion.expression.clone()),
        symmetric: extrusion.symmetric,
        taper: extrusion.taper,
        taper_mode: extrusion.taper_mode,
        taper_expression: (!extrusion.taper_expression.trim().is_empty())
            .then(|| extrusion.taper_expression.clone()),
    })
}

/// Build a replayable `Instruction::Loft` for the loft the interactive Loft tool just
/// created (the last entry in `doc.lofts`) — `Action::CommitLoft` carries no fields, so
/// like `instruction_for_new_extrusion` the sections come from post-commit state.
pub fn instruction_for_new_loft(doc: &crate::model::Document) -> Option<Instruction> {
    // Slot order is creation order here, so the newest loft is the last live one.
    let loft = doc.lofts.values().last()?;
    let (body, bodies) = match &loft.mode {
        crate::model::LoftMode::NewBody => {
            (crate::actions::RevolveBodyChoice::NewBody, Vec::new())
        }
        crate::model::LoftMode::AddTo(b) => {
            (crate::actions::RevolveBodyChoice::AddTouching, b.clone())
        }
        crate::model::LoftMode::Cut(b) => (crate::actions::RevolveBodyChoice::Cut, b.clone()),
    };
    Some(Instruction::Loft {
        faces: loft.sections.iter().map(|sec| sec.face.clone()).collect(),
        body,
        bodies: body_ordinals(doc, &bodies)?,
    })
}

/// A shape as its Lua call (#909): `bearcad.cuboid{...}` / `cylinder` / `sphere`, or
/// `bearcad.edit_shape{ index = i, ... }` when re-pointing one.
fn shape_lua_call(shape: &crate::model::Primitive, edit: Option<usize>) -> String {
    use crate::model::PrimitiveKind as K;
    let num = |v: f32| format!("{}", (v * 1000.0).round() / 1000.0);
    let point = |p: [f32; 3]| format!("{{{}, {}, {}}}", num(p[0]), num(p[1]), num(p[2]));
    let mut parts = Vec::new();
    if let Some(index) = edit {
        parts.push(format!("index = {index}"));
        parts.push(format!("shape = {:?}", shape.kind.script_name()));
    }
    parts.push(format!("at = {}", point(shape.origin)));
    if shape.normal != [0.0, 0.0, 1.0] {
        parts.push(format!("normal = {}", point(shape.normal)));
    }
    if shape.u_axis != [1.0, 0.0, 0.0] {
        parts.push(format!("u_axis = {}", point(shape.u_axis)));
    }
    let dim = |name: &str, expression: &String| -> Option<String> {
        (!expression.trim().is_empty()).then(|| format!("{name} = {expression:?}"))
    };
    match shape.kind {
        K::Cuboid => {
            parts.extend(dim("width", &shape.width));
            parts.extend(dim("depth", &shape.depth));
            parts.extend(dim("height", &shape.height));
        }
        K::Cylinder => {
            parts.extend(dim("radius", &shape.radius));
            parts.extend(dim("height", &shape.height));
        }
        K::Sphere => parts.extend(dim("radius", &shape.radius)),
    }
    if let Some(name) = &shape.name {
        parts.push(format!("name = {name:?}"));
    }
    let call = match edit {
        Some(_) => "edit_shape".to_string(),
        None => shape.kind.script_name().to_string(),
    };
    format!("bearcad.{call}{{ {} }}", parts.join(", "))
}

/// Replayable `Instruction::Revolve` for the revolution the interactive tool just created
/// (mirrors `instruction_for_new_loft`).
pub fn instruction_for_new_revolution(doc: &crate::model::Document) -> Option<Instruction> {
    let rev = doc.revolutions.values().last()?;
    let (body, bodies) = match &rev.mode {
        crate::model::RevolveMode::NewBody => {
            (crate::actions::RevolveBodyChoice::NewBody, Vec::new())
        }
        crate::model::RevolveMode::AddTo(b) => {
            (crate::actions::RevolveBodyChoice::AddTouching, b.clone())
        }
        crate::model::RevolveMode::Cut(b) => (crate::actions::RevolveBodyChoice::Cut, b.clone()),
    };
    Some(Instruction::Revolve {
        faces: rev.faces.clone(),
        axis: rev.axis,
        angle_deg: rev.angle_deg,
        angle_expression: rev.angle_expression.clone(),
        angle_is_revolutions: rev.angle_is_revolutions,
        pitch_mm: rev.pitch_mm,
        pitch_expression: rev.pitch_expression.clone(),
        symmetric: rev.symmetric,
        body,
        bodies: body_ordinals(doc, &bodies)?,
    })
}

/// Replayable `Instruction::Sweep` for the sweep the interactive tool just created
/// (mirrors `instruction_for_new_revolution`).
pub fn instruction_for_new_sweep(doc: &crate::model::Document) -> Option<Instruction> {
    let fp = doc.sweeps.values().last()?;
    let (body, bodies) = match &fp.mode {
        crate::model::SweepMode::NewBody => {
            (crate::actions::RevolveBodyChoice::NewBody, Vec::new())
        }
        crate::model::SweepMode::AddTo(b) => {
            (crate::actions::RevolveBodyChoice::AddTouching, b.clone())
        }
        crate::model::SweepMode::Cut(b) => (crate::actions::RevolveBodyChoice::Cut, b.clone()),
    };
    Some(Instruction::Sweep {
        faces: fp.faces.clone(),
        path: fp.path.clone(),
        body,
        bodies: body_ordinals(doc, &bodies)?,
    })
}

/// Command-log instructions for a just-committed edge-treatment operation (#531): a single
/// `chamfer_edge`/`fillet_edge` call carrying every treated edge, so replaying the log rebuilds
/// the one operation the user committed rather than one operation per edge (#672).
pub fn instructions_for_new_edge_treatment_op(
    doc: &crate::model::Document,
) -> Vec<Instruction> {
    let Some(op) = doc.edge_treatment_ops.values().last() else {
        return Vec::new();
    };
    let Some(edges) = op
        .edges
        .iter()
        .map(|te| {
            let host = match te.solid {
                crate::model::TreatableSolid::Extrusion(e) => {
                    TreatableSolidRef::Extrusion(extrusion_ordinal(doc, e)?)
                }
                crate::model::TreatableSolid::Primitive(p) => {
                    TreatableSolidRef::Primitive(primitive_ordinal(doc, p)?)
                }
            };
            Some((host, te.edge))
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Vec::new();
    };
    vec![Instruction::EdgeTreatment {
        edges,
        kind: op.kind,
        amount: op.amount,
        expression: op.expression.clone(),
    }]
}

/// Render a boolean-operation call (`bearcad.combine{}` / `bearcad.edit_boolean{}`).
fn boolean_op_lua(
    call: &str,
    op: Option<usize>,
    kind: crate::model::BooleanOpKind,
    a: &[usize],
    b: &[usize],
    keep_b: bool,
) -> String {
    let list = |v: &[usize]| {
        v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    };
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!("op = \"{}\"", match kind {
        crate::model::BooleanOpKind::Combine => "combine",
        crate::model::BooleanOpKind::Cut => "cut",
        crate::model::BooleanOpKind::Intersect => "intersect",
        crate::model::BooleanOpKind::Difference => "difference",
    }));
    parts.push(format!("a = {{{}}}", list(a)));
    if !b.is_empty() {
        parts.push(format!("b = {{{}}}", list(b)));
    }
    if keep_b {
        parts.push("keep_b = true".to_string());
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

/// Render a move-operation call (`bearcad.move_bodies{}` / `bearcad.edit_move{}`).
#[allow(clippy::too_many_arguments)]
fn move_op_lua(
    call: &str,
    op: Option<usize>,
    targets: &[usize],
    images: &[usize],
    tx: &str,
    ty: &str,
    tz: &str,
    rx: &str,
    ry: &str,
    rz: &str,
    roll_angle: &str,
    face_flip: bool,
    face_spin: &str,
    face_offset: &str,
    start_point_a: &Option<crate::model::MovePointRef>,
    end_point_a: &Option<crate::model::MovePointRef>,
    start_point_b: &Option<crate::model::MovePointRef>,
    end_point_b: &Option<crate::model::MovePointRef>,
    start_point_c: &Option<crate::model::MovePointRef>,
    end_point_c: &Option<crate::model::MovePointRef>,
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!(
        "bodies = {{{}}}",
        targets.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    ));
    if !images.is_empty() {
        parts.push(format!(
            "images = {{{}}}",
            images.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
        ));
    }
    // Naming both points makes it a snap translation (#648); the x/y/z components below are
    // then ignored, so they're left out.
    if let (Some(start), Some(end)) = (start_point_a, end_point_a) {
        parts.push(format!("from = {}", move_point_lua(start)));
        parts.push(format!("to = {}", move_point_lua(end)));
    }
    // The optional B pair (#669) adds the rotation.
    if let (Some(start), Some(end)) = (start_point_b, end_point_b) {
        parts.push(format!("from_b = {}", move_point_lua(start)));
        parts.push(format!("to_b = {}", move_point_lua(end)));
    }
    // The optional C pair pins the spin B leaves free.
    if let (Some(start), Some(end)) = (start_point_c, end_point_c) {
        parts.push(format!("from_c = {}", move_point_lua(start)));
        parts.push(format!("to_c = {}", move_point_lua(end)));
    }
    for (name, value) in [
        ("x", tx),
        ("y", ty),
        ("z", tz),
        ("rx", rx),
        ("ry", ry),
        ("rz", rz),
        ("roll", roll_angle),
        ("spin", face_spin),
        ("gap", face_offset),
    ] {
        if !value.trim().is_empty() {
            parts.push(format!("{name} = \"{value}\""));
        }
    }
    // Only mentioned when set: the default puts the surfaces together (#1077).
    if face_flip {
        parts.push("flip = true".to_string());
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

/// Render a mirror-operation call (`bearcad.mirror_bodies{}` / `bearcad.edit_mirror{}`, #523).
fn mirror_op_lua(
    call: &str,
    op: Option<usize>,
    doc: Option<&crate::model::Document>,
    plane: &FaceId,
    targets: &[usize],
    mode: crate::model::MirrorMode,
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!("plane = {}", face_id_lua_ref(plane, doc)));
    parts.push(format!(
        "bodies = {{{}}}",
        targets.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    ));
    // The default mode stays implicit so existing scripts render unchanged (#639).
    if let Some(name) = mirror_mode_script_name(mode) {
        parts.push(format!("output = {name:?}"));
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

/// One joint member as script text (#894): a bare body index, or a `{ kind = …, index = … }`
/// table for components and unit instances.
fn joint_member_lua(member: &crate::model::JointRef) -> String {
    match member {
        crate::model::JointRef::Body(i) => i.index().to_string(),
        // The component's arena slot, not its ordinal — this form has no document (#1070).
        crate::model::JointRef::Component(i) => {
            format!("{{ kind = \"component\", index = {} }}", i.index())
        }
        // The instance's arena slot, not its ordinal — this form has no document (#1070).
        crate::model::JointRef::UnitInstance(i) => {
            format!("{{ kind = \"unit_instance\", index = {} }}", i.index())
        }
    }
}

/// One side of a mate pick as script text (#1020). The point spellings are the Move tool's,
/// except that an edge **midpoint** is `midpoint` — `edge` names the whole edge.
pub fn mate_ref_lua(r: &crate::model::MateRef) -> String {
    match r {
        crate::model::MateRef::Face { body, centroid, normal } => format!(
            "{{ body = {}, face = {}, normal = {} }}",
            body.index(),
            mm_point_lua(*centroid),
            mm_point_lua(*normal)
        ),
        crate::model::MateRef::Plane(i) => format!("{{ plane = {} }}", i.index()),
        crate::model::MateRef::Edge { body, a, b } => format!(
            "{{ body = {}, edge = {{ {}, {} }} }}",
            body.index(),
            mm_point_lua(*a),
            mm_point_lua(*b)
        ),
        crate::model::MateRef::Axis(a) => format!(
            "{{ axis = \"{}\" }}",
            match a {
                crate::construction::GlobalAxis::X => "x",
                crate::construction::GlobalAxis::Y => "y",
                crate::construction::GlobalAxis::Z => "z",
            }
        ),
        crate::model::MateRef::HoleAxis { body, origin, dir } => format!(
            "{{ body = {}, hole_axis = {}, direction = {} }}",
            body.index(),
            mm_point_lua(*origin),
            mm_point_lua(*dir)
        ),
        crate::model::MateRef::Point(crate::model::MovePointRef::EdgeMidpoint { body, a, b }) => {
            format!(
                "{{ body = {}, midpoint = {{ {}, {} }} }}",
                body.index(),
                mm_point_lua(*a),
                mm_point_lua(*b)
            )
        }
        crate::model::MateRef::Point(p) => move_point_lua(p),
    }
}

/// The `face = {…}` / `line_up = {…}` arguments of a joint call (#1020).
fn mate_lua(placement: &crate::model::MoveOperation) -> Vec<String> {
    let face = |p: &Option<crate::model::MovePointRef>| {
        p.as_ref()
            .and_then(crate::model::move_point_host_mate_ref)
            .map(|r| mate_ref_lua(&r))
    };
    let (moving, fixed) = (face(&placement.start_point_a), face(&placement.end_point_a));
    if moving.is_none() && fixed.is_none() {
        return Vec::new();
    }
    let mut inner = Vec::new();
    if let Some(r) = moving {
        inner.push(format!("moving = {r}"));
    }
    if let Some(r) = fixed {
        inner.push(format!("fixed = {r}"));
    }
    if placement.face_flip {
        inner.push("flip = true".to_string());
    }
    if !placement.face_offset.trim().is_empty() {
        inner.push(format!("offset = \"{}\"", placement.face_offset));
    }
    if !placement.face_spin.trim().is_empty() {
        inner.push(format!("spin = \"{}\"", placement.face_spin));
    }
    vec![format!("face = {{ {} }}", inner.join(", "))]
}

/// Render a joint call (`bearcad.joint{}` / `bearcad.edit_joint{}` / `bearcad.begin_joint{}`).
#[allow(clippy::too_many_arguments)]
fn joint_op_lua(
    call: &str,
    op: Option<usize>,
    doc: Option<&crate::model::Document>,
    members: &[crate::model::JointRef],
    base: usize,
    kind: &crate::model::JointKind,
    placement: &crate::model::MoveOperation,
    frame: &crate::model::JointFrame,
    position: &str,
    position2: &str,
    position3: &str,
    limits: &crate::model::JointLimits,
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    if members.len() > 2 {
        parts.push(format!(
            "parts = {{{}}}",
            members.iter().map(joint_member_lua).collect::<Vec<_>>().join(", ")
        ));
    } else {
        if let Some(a) = members.first() {
            parts.push(format!("a = {}", joint_member_lua(a)));
        }
        if let Some(b) = members.get(1) {
            parts.push(format!("b = {}", joint_member_lua(b)));
        }
    }
    parts.push(format!("kind = \"{}\"", kind.name()));
    if let crate::model::JointKind::Screw { lead } = kind {
        if !lead.trim().is_empty() {
            parts.push(format!("lead = \"{lead}\""));
        }
    }
    if base == 1 {
        parts.push("base = \"b\"".to_string());
    }
    parts.extend(mate_lua(placement));
    // How the joint works (#1079): only mentioned when set, since a mate usually seeds it.
    if let Some(p) = &frame.origin {
        parts.push(format!("frame_origin = {}", move_point_lua(p)));
    }
    if let Some(r) = &frame.primary {
        parts.push(format!("frame_axis = {}", mate_ref_lua(r)));
    }
    if let Some(r) = &frame.secondary {
        parts.push(format!("frame_axis2 = {}", mate_ref_lua(r)));
    }
    for (name, value) in [
        ("position", position),
        ("position2", position2),
        ("position3", position3),
        // Travel limits (#896): expressions ride as strings, stop targets as face specs.
        ("slide_min", &limits.slide_min),
        ("slide_max", &limits.slide_max),
        ("turn_min", &limits.turn_min),
        ("turn_max", &limits.turn_max),
    ] {
        if !value.trim().is_empty() {
            parts.push(format!("{name} = \"{value}\""));
        }
    }
    for (name, target) in [
        ("slide_min_to", &limits.slide_min_target),
        ("slide_max_to", &limits.slide_max_target),
    ] {
        if let Some(target) = target {
            parts.push(format!("{name} = {}", extrude_target_lua_table(target, doc)));
        }
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

/// The `output = …` script name for a non-default [`crate::model::MirrorMode`] (#639).
/// `None` for the default, which scripts leave out.
pub fn mirror_mode_script_name(mode: crate::model::MirrorMode) -> Option<&'static str> {
    match mode {
        crate::model::MirrorMode::NewBody => None,
        crate::model::MirrorMode::Join => Some("join"),
        crate::model::MirrorMode::Cut => Some("cut"),
    }
}

/// Render a repeat-operation call (`bearcad.repeat_bodies{}` / `bearcad.edit_repeat{}`).
#[allow(clippy::too_many_arguments)]
fn repeat_op_lua(
    call: &str,
    op: Option<usize>,
    doc: Option<&crate::model::Document>,
    targets: &[usize],
    axis: crate::model::RevolveAxis,
    around_axis: bool,
    flip: bool,
    mode: crate::model::RepeatMode,
    count: &str,
    spacing: &str,
    length: &str,
    length_target: Option<&crate::model::ExtrudeTarget>,
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!(
        "bodies = {{{}}}",
        targets.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    ));
    parts.push(format!("axis = {}", revolve_axis_lua(axis)));
    if around_axis {
        parts.push("around = true".to_string());
    }
    if flip {
        parts.push("flip = true".to_string());
    }
    parts.push(format!("mode = \"{}\"", match mode {
        crate::model::RepeatMode::CountGap => "count_gap",
        crate::model::RepeatMode::CountFitEnds => "count_fit_ends",
        crate::model::RepeatMode::CountFitCenters => "count_fit_centers",
        crate::model::RepeatMode::FillGap => "fill_gap",
        crate::model::RepeatMode::FillPitch => "fill_pitch",
        crate::model::RepeatMode::FillMaxPitch => "fill_max_pitch",
        crate::model::RepeatMode::CountPitch => "count_pitch",
        crate::model::RepeatMode::FillGapSpan => "fill_gap_span",
        crate::model::RepeatMode::FillPitchSpan => "fill_pitch_span",
    }));
    // A picked length target (#645) replaces the fill-length expression.
    if let Some(target) = length_target {
        parts.push(format!("to = {}", extrude_target_lua_table(target, doc)));
    }
    for (name, value) in [("count", count), ("spacing", spacing), ("length", length)] {
        if !value.trim().is_empty() {
            parts.push(format!("{name} = \"{value}\""));
        }
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

/// Render a slice-operation call (`bearcad.slice{}` / `bearcad.edit_slice{}`).
fn slice_op_lua(
    call: &str,
    op: Option<usize>,
    doc: Option<&crate::model::Document>,
    targets: &[usize],
    cutters: &[crate::model::SliceCutter],
    extend_infinite: bool,
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!(
        "bodies = {{{}}}",
        targets.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    ));
    parts.push(format!(
        "cutters = {{{}}}",
        cutters
            .iter()
            .map(|c| slice_cutter_lua_ref(c, doc))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    if extend_infinite {
        parts.push("extend = true".to_string());
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

fn shell_op_lua(
    call: &str,
    op: Option<usize>,
    doc: Option<&crate::model::Document>,
    targets: &[usize],
    open_faces: &[crate::model::FaceId],
    thickness: &str,
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!(
        "bodies = {{{}}}",
        targets.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    ));
    if !open_faces.is_empty() {
        parts.push(format!(
            "faces = {{{}}}",
            open_faces
                .iter()
                .map(|f| face_id_lua_ref(f, doc))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    parts.push(format!("thickness = {thickness:?}"));
    format!("{call}{{ {} }}", parts.join(", "))
}

/// Script table for one slice cutter: a face-spec, or `{ kind = "line", index = i }` (#1126).
fn slice_cutter_lua_ref(
    cutter: &crate::model::SliceCutter,
    doc: Option<&crate::model::Document>,
) -> String {
    match cutter {
        crate::model::SliceCutter::Face(face) => face_id_lua_ref(face, doc),
        crate::model::SliceCutter::Line { line } => {
            let index = ordinal_or_slot(
                doc.map(|d| d.lines.keys().position(|k| k == *line)),
                line.index(),
            );
            format!("{{ kind = \"line\", index = {index} }}")
        }
    }
}

/// Render an extrusion's faces as `bearcad.extrude{}` keyword arguments
/// (`rect=`/`rects=`, `circle=`/`circles=`, `polygon=`). A single rect or circle uses the
/// singular field to match how `bearcad.extrude` is normally called by hand; multiple of a
/// kind use the plural array form. Only the first polygon face is kept — the Lua API has no
/// way to extrude more than one closed-loop face alongside the others in one call.
fn extrude_face_args(
    faces: &[crate::model::ExtrudeFace],
    doc: Option<&crate::model::Document>,
) -> String {
    use crate::model::ExtrudeFace;
    let mut circles = Vec::new();
    let mut polygon = None;
    let mut boolean = None;
    for face in faces {
        match face {
            ExtrudeFace::Circle(i) => circles.push(circle_ord(doc, *i)),
            ExtrudeFace::Polygon(lines) => {
                polygon.get_or_insert(lines);
            }
            // Only the first is kept, same "one non-rect/circle profile per call" limitation
            // as `polygon` above — the Lua API has no way to extrude more than one alongside
            // the others in a single call.
            ExtrudeFace::Boolean { op, a, b } => {
                boolean.get_or_insert((*op, a.as_ref(), b.as_ref()));
            }
            // Text glyphs aren't reconstructable from a flat script arg (they reference baked
            // outlines); nor is a plane region, whose seed the flat arg shape has nowhere to
            // put (#993). The script round-trip skips both.
            ExtrudeFace::TextGlyph { .. } | ExtrudeFace::SketchRegion { .. } => {}
        };
    }
    let index_list = |indices: &[usize]| -> String {
        indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    };
    let mut parts = Vec::new();
    match circles.as_slice() {
        [] => {}
        [single] => parts.push(format!("circle = {single}")),
        many => parts.push(format!("circles = {{{}}}", index_list(many))),
    }
    if let Some(lines) = polygon {
        // The lines' arena slots, not their ordinals (#1070).
        let idx = lines.iter().map(|i| line_ord(doc, *i).to_string()).collect::<Vec<_>>().join(", ");
        parts.push(format!("polygon = {{{idx}}}"));
    }
    if let Some((op, a, b)) = boolean {
        parts.push(format!("boolean = {}", boolean_face_lua_table(op, a, b, doc)));
    }
    parts.join(", ")
}

/// Lua table literal for a boolean-combined face's inner fields (#16/#62): `{op = "...",
/// a = <face spec>, b = <face spec>}`, matching the shape `lua_boolean_face_from_table`
/// (src/lua_script.rs) parses back.
fn boolean_face_lua_table(
    op: crate::model::BooleanOp,
    a: &crate::model::ExtrudeFace,
    b: &crate::model::ExtrudeFace,
    doc: Option<&crate::model::Document>,
) -> String {
    let op_str = match op {
        crate::model::BooleanOp::Intersection => "intersection",
        crate::model::BooleanOp::Difference => "difference",
    };
    format!(
        "{{op = \"{op_str}\", a = {}, b = {}}}",
        extrude_face_spec_table(a, doc),
        extrude_face_spec_table(b, doc)
    )
}

/// Lua face-spec table for any `ExtrudeFace` (`{rect = i}`, `{circle = i}`,
/// `{polygon = {..}}`, or a nested `{boolean = {...}}`) — the shape
/// `lua_extrude_face_from_table` (src/lua_script.rs) parses back into an `ExtrudeFace`.
fn extrude_face_spec_table(
    face: &crate::model::ExtrudeFace,
    doc: Option<&crate::model::Document>,
) -> String {
    use crate::model::ExtrudeFace;
    match face {
        ExtrudeFace::Circle(i) => format!("{{circle = {}}}", circle_ord(doc, *i)),
        ExtrudeFace::Polygon(lines) => {
            let idx = lines.iter().map(|i| line_ord(doc, *i).to_string()).collect::<Vec<_>>().join(", ");
            format!("{{polygon = {{{idx}}}}}")
        }
        ExtrudeFace::Boolean { op, a, b } => {
            format!("{{boolean = {}}}", boolean_face_lua_table(*op, a, b, doc))
        }
        ExtrudeFace::TextGlyph { text, glyph } => {
            format!("{{text_glyph = {{text = {}, glyph = {glyph}}}}}", text.index())
        }
        // A plane region (#993) names its sketch and the seed point that picks it out.
        ExtrudeFace::SketchRegion { sketch, seed_u, seed_v } => format!("{{region = {{sketch = {}, u = {}, v = {}}}}}", sketch.index(),
            *seed_u as f32 / crate::model::SKETCH_REGION_SEED_SCALE,
            *seed_v as f32 / crate::model::SKETCH_REGION_SEED_SCALE
        ),
    }
}

/// Render an [`crate::model::ExtrudeTarget`] as the `to = {...}` table
/// `bearcad.extrude`/`bearcad.edit_extrusion` accept (#114).
fn extrude_target_lua_table(
    target: &crate::model::ExtrudeTarget,
    doc: Option<&crate::model::Document>,
) -> String {
    use crate::model::ExtrudeTarget;
    match target {
        ExtrudeTarget::Plane(i) => format!("{{ plane = {} }}", i.index()),
        ExtrudeTarget::Face(face) => format!("{{ face = {} }}", extrude_face_spec_table(face, doc)),
        ExtrudeTarget::BodyFace(face_id) => format!("{{ face = {} }}", face_id_lua_ref(face_id, doc)),
        ExtrudeTarget::RepeatedFace { face, op, instance } => format!(
            // The repeat's arena slot, not its ordinal (#1070).
            "{{ face = {}, repeat_op = {}, instance = {instance} }}",
            face_id_lua_ref(face, doc),
            op.index()
        ),
        ExtrudeTarget::Vertex(point) => {
            format!("{{ vertex = {} }}", constraint_point_lua_ref(point, doc))
        }
    }
}

fn view_script_name(view: StandardView) -> &'static str {
    match view {
        StandardView::Front => "front",
        StandardView::Back => "back",
        StandardView::Left => "left",
        StandardView::Right => "right",
        StandardView::Top => "top",
        StandardView::Bottom => "bottom",
    }
}

fn projection_mode_script_name(mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Orthographic => "orthographic",
        ProjectionMode::Natural => "natural",
    }
}

fn edge_script_name(edge: CubeEdgeId) -> &'static str {
    match edge {
        CubeEdgeId::FrontBottom => "front_bottom",
        CubeEdgeId::RightBottom => "right_bottom",
        CubeEdgeId::BackBottom => "back_bottom",
        CubeEdgeId::LeftBottom => "left_bottom",
        CubeEdgeId::FrontTop => "front_top",
        CubeEdgeId::RightTop => "right_top",
        CubeEdgeId::BackTop => "back_top",
        CubeEdgeId::LeftTop => "left_top",
        CubeEdgeId::FrontLeft => "front_left",
        CubeEdgeId::FrontRight => "front_right",
        CubeEdgeId::BackRight => "back_right",
        CubeEdgeId::BackLeft => "back_left",
    }
}

fn corner_script_name(corner: CubeCornerId) -> &'static str {
    match corner {
        CubeCornerId::FrontLeftBottom => "front_left_bottom",
        CubeCornerId::FrontRightBottom => "front_right_bottom",
        CubeCornerId::BackRightBottom => "back_right_bottom",
        CubeCornerId::BackLeftBottom => "back_left_bottom",
        CubeCornerId::FrontLeftTop => "front_left_top",
        CubeCornerId::FrontRightTop => "front_right_top",
        CubeCornerId::BackRightTop => "back_right_top",
        CubeCornerId::BackLeftTop => "back_left_top",
    }
}

fn key_name(key: Key) -> &'static str {
    match key {
        Key::Enter => "enter",
        Key::Tab => "tab",
        Key::Escape => "escape",
        Key::Backspace => "backspace",
        Key::Delete => "delete",
        Key::ArrowLeft => "left",
        Key::ArrowRight => "right",
        Key::ArrowUp => "up",
        Key::ArrowDown => "down",
        Key::Space => "space",
        Key::R => "r",
        Key::A => "a",
        Key::B => "b",
        Key::C => "c",
        Key::D => "d",
        Key::E => "e",
        Key::F => "f",
        Key::G => "g",
        Key::H => "h",
        Key::I => "i",
        Key::J => "j",
        Key::K => "k",
        Key::L => "l",
        Key::M => "m",
        Key::N => "n",
        Key::O => "o",
        Key::P => "p",
        Key::Q => "q",
        Key::S => "s",
        Key::T => "t",
        Key::U => "u",
        Key::V => "v",
        Key::W => "w",
        Key::X => "x",
        Key::Y => "y",
        Key::Z => "z",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::Backtick => "`",
        _ => "?",
    }
}

fn tool_lua_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "select",
        Tool::Rectangle => "rectangle",
        Tool::Line => "line",
        Tool::Circle => "circle",
        Tool::ConstructionPlane => "construction_plane",
        Tool::Sketch => "sketch",
        Tool::Dimension => "dimension",
        Tool::Project => "project",
        Tool::Constraint => "constraint",
        Tool::Extrude => "extrude",
        Tool::Chamfer => "chamfer",
        Tool::Fillet => "fillet",
        Tool::Offset => "offset",
        Tool::Loft => "loft",
        Tool::Revolve => "revolve",
        Tool::Shape => "shape",
        Tool::Sweep => "sweep",
        Tool::Combine => "combine",
        Tool::Move => "move",
        Tool::Mirror => "mirror",
        Tool::Repeat => "repeat",
        Tool::Slice => "slice",
        Tool::Shell => "shell",
        Tool::Joint => "joint",
        Tool::Text => "text",
        Tool::DrawingAdd => "drawing_add",
        Tool::DrawingAlign => "drawing_align",
    }
}

#[allow(dead_code)] // kept for simple kind/index diagnostics; export uses face_id_lua_ref
fn face_lua_parts(face: &FaceId) -> (&'static str, usize) {
    match face {
        FaceId::Circle(i) => ("circle", i.index() as usize),
        FaceId::ConstructionPlane(i) => ("construction_plane", i.index() as usize),
        // Cap/side faces aren't yet addressable from the two-argument script form.
        FaceId::ExtrudeCap { extrusion, .. } => ("extrude_cap", extrusion.index() as usize),
        FaceId::ExtrudeSide { extrusion, .. } => ("extrude_side", extrusion.index() as usize),
        // A unit face isn't fully addressable from the two-argument form either (#725):
        // the inner face rides only in session recordings via its instance.
        FaceId::UnitFace { instance, .. } => ("unit_face", instance.index() as usize),
        // A polygon's full line list isn't expressible as a single index; same limitation
        // as cap/side faces above (#66).
        FaceId::Polygon(lines) => (
            "polygon",
            lines.first().map(|l| l.index() as usize).unwrap_or(0),
        ),
        // The revolve's arena slot, not its ordinal — this form has no document (#1070).
        FaceId::RevolveCap { revolution, .. } => ("revolve_cap", revolution.index() as usize),
        FaceId::RevolveSide { revolution, .. } => ("revolve_side", revolution.index() as usize),
        FaceId::PrimitiveFace { primitive, .. } => ("primitive_face", primitive.index() as usize),
        FaceId::RepeatedFace { op, instance, .. } => ("repeated_face", op.index() as usize + instance),
        FaceId::BodyMeshFace { body, .. } => ("body_mesh_face", body.index() as usize),
    }
}

fn rect_axis_lua_name(axis: RectAxis) -> &'static str {
    match axis {
        RectAxis::Width => "width",
        RectAxis::Height => "height",
    }
}

fn dim_label_axis_lua_name(axis: DimLabelAxis) -> &'static str {
    match axis {
        DimLabelAxis::Width => "width",
        DimLabelAxis::Height => "height",
        DimLabelAxis::Length => "length",
        DimLabelAxis::Diameter => "diameter",
    }
}

fn plane_dim_lua_name(dim: PlaneDim) -> &'static str {
    match dim {
        PlaneDim::Offset => "offset",
        PlaneDim::Angle => "angle",
    }
}

fn geometric_constraint_lua_name(
    kind: crate::geometric_constraints::GeometricConstraintType,
) -> &'static str {
    geometric_constraint_script_name(kind)
}

fn element_lua_ref(element: &SceneElement, doc: Option<&crate::model::Document>) -> String {
    // #26/#27: a face's own edge, matching `lua_script::parse_element_table`'s
    // `{ kind = "face", face = {...}, index = N, edge = true }` shape.
    if let SceneElement::FaceEdge(line) = element {
        match line {
            ConstraintLine::FaceEdge { face, index } => {
                return format!(
                    "{{ kind = \"face\", face = {}, index = {index}, edge = true }}",
                    face_id_lua_ref(face, doc)
                );
            }
            ConstraintLine::OriginAxis(axis) => {
                return format!("{{ kind = \"axis\", axis = \"{}\" }}", sketch_axis_lua_name(*axis));
            }
            ConstraintLine::ImageEdge { image, edge } => {
                let ordinal = doc
                    .and_then(|d| d.tracing_images.keys().position(|k| k == *image))
                    .unwrap_or(image.index() as usize);
                return format!(
                    "{{ kind = \"image\", index = {ordinal}, edge = \"{}\" }}",
                    edge.lua_name()
                );
            }
            ConstraintLine::Line(index) => {
                let ordinal = doc
                    .and_then(|d| d.lines.keys().position(|k| k == *index))
                    .unwrap_or(index.index() as usize);
                return format!("{{ kind = \"line\", index = {ordinal} }}");
            }
        }
    }
    let tokens = element_script_tokens(element.clone(), doc);
    if let Some(point) = tokens.point {
        return format!("{{ kind = \"point\", {} }}", point_lua_fields(&point, doc));
    }
    format!("{{ kind = \"{}\", index = {} }}", tokens.kind, tokens.index)
}

fn point_lua_fields(point: &ConstraintPoint, doc: Option<&crate::model::Document>) -> String {
    use crate::model::{ConstraintPoint, LineEnd};
    match point {
        ConstraintPoint::LineEndpoint { line, end } => {
            let end_name = match end {
                LineEnd::Start => "start",
                LineEnd::End => "end",
            };
            // `end` is a Lua reserved word, so it can't be a bareword table key; bracket it.
            format!(
                "kind = \"line\", index = {}, [\"end\"] = \"{end_name}\"",
                line.index()
            )
        }
        ConstraintPoint::CircleCenter(circle) => {
            let ordinal = ordinal_or_slot(
                doc.map(|d| d.circles.keys().position(|k| k == *circle)),
                circle.index(),
            );
            format!("kind = \"circle\", index = {ordinal}")
        }
        ConstraintPoint::Origin => "kind = \"origin\"".to_string(),
        // #26/#27: mirrors `lua_script::parse_constraint_point_table`'s `"face"` shape.
        ConstraintPoint::FaceVertex { face, index } => {
            format!("kind = \"face\", face = {}, index = {index}", face_id_lua_ref(face, doc))
        }
        // #408: mirrors `lua_script::parse_constraint_point_table`'s `"sketch_text"` shape.
        ConstraintPoint::TextAnchor { text, anchor } => {
            let anchor = anchor.lua_name();
            format!(
                "kind = \"sketch_text\", index = {}, anchor = \"{anchor}\"",
                text.index()
            )
        }
        // #425: mirrors the `"image"` + `point` shape.
        ConstraintPoint::ImageCalibrationPoint { image, index } => {
            let ordinal = ordinal_or_slot(
                doc.map(|d| d.tracing_images.keys().position(|k| k == *image)),
                image.index(),
            );
            format!("kind = \"image\", index = {ordinal}, point = {index}")
        }
        ConstraintPoint::ImageAnchor { image, anchor } => {
            let ordinal = ordinal_or_slot(
                doc.map(|d| d.tracing_images.keys().position(|k| k == *image)),
                image.index(),
            );
            let anchor = anchor.lua_name();
            format!("kind = \"image\", index = {ordinal}, anchor = \"{anchor}\"")
        }
    }
}

fn constraint_line_lua_ref(line: &ConstraintLine, doc: Option<&crate::model::Document>) -> String {
    match line {
        ConstraintLine::Line(index) => {
            format!("{{ kind = \"line\", index = {} }}", index.index())
        }
        // #26/#27: mirrors `lua_script::parse_constraint_line_table`'s `"face"` shape.
        ConstraintLine::FaceEdge { face, index } => format!(
            "{{ kind = \"face\", face = {}, index = {index} }}",
            face_id_lua_ref(face, doc)
        ),
        ConstraintLine::OriginAxis(axis) => {
            format!("{{ kind = \"axis\", axis = \"{}\" }}", sketch_axis_lua_name(*axis))
        }
        ConstraintLine::ImageEdge { image, edge } => {
            let ordinal = ordinal_or_slot(
                doc.map(|d| d.tracing_images.keys().position(|k| k == *image)),
                image.index(),
            );
            format!(
                "{{ kind = \"image\", index = {ordinal}, edge = \"{}\" }}",
                edge.lua_name()
            )
        }
    }
}

/// Lua name for a sketch origin axis (#189).
fn sketch_axis_lua_name(axis: crate::model::SketchAxis) -> &'static str {
    match axis {
        crate::model::SketchAxis::X => "x",
        crate::model::SketchAxis::Y => "y",
    }
}

fn constraint_point_lua_ref(point: &ConstraintPoint, doc: Option<&crate::model::Document>) -> String {
    format!("{{ {} }}", point_lua_fields(point, doc))
}

/// A scripted move is a **snap** exactly when it names both A points (#648) — the terse form,
/// so a plain `move_bodies{x = …}` stays a free translation.
///
/// It is a **Face Snap** (#1077) when both of those points sit on faces and no B pair is
/// named: naming two faces and nothing else is asking for one to be put on the other. A B
/// pair says the turn is coming from a second point pair instead, which is Point Snap — so a
/// script written before Face Snap existed keeps meaning what it meant.
pub fn move_translate_mode(
    source: &Option<crate::model::MovePointRef>,
    target: &Option<crate::model::MovePointRef>,
    source_b: &Option<crate::model::MovePointRef>,
) -> crate::model::MoveTranslateMode {
    use crate::model::{MovePointRef, MoveTranslateMode};
    match (source, target) {
        (Some(a), Some(b)) => {
            let on_faces = matches!(a, MovePointRef::OnFace { .. })
                && matches!(b, MovePointRef::OnFace { .. });
            if on_faces && source_b.is_none() {
                MoveTranslateMode::FaceSnap
            } else {
                MoveTranslateMode::PointSnap
            }
        }
        _ => MoveTranslateMode::Free,
    }
}

/// A [`crate::model::MovePointRef`] as the Lua table scripts use (#649/#650): a body plus
/// either a `vertex` in millimetres or the two ends of an `edge` (its midpoint is the point),
/// or `{ origin = true }` for the world origin (#946).
pub fn move_point_lua(point: &crate::model::MovePointRef) -> String {
    match point {
        crate::model::MovePointRef::Vertex { body, p } => {
            format!("{{ body = {}, vertex = {} }}", body.index(), mm_point_lua(*p))
        }
        crate::model::MovePointRef::EdgeMidpoint { body, a, b } => format!(
            "{{ body = {}, edge = {{ {}, {} }} }}",
            body.index(),
            mm_point_lua(*a),
            mm_point_lua(*b)
        ),
        // A point along an edge (#670) is spelled by its position, like a corner — the
        // parser doesn't need to know which edge it came from.
        crate::model::MovePointRef::OnEdge { body, p } => {
            format!("{{ body = {}, on_edge = {} }}", body.index(), mm_point_lua(*p))
        }
        // A point on a face (#738/#1074) spells its selection key — the face's centroid plus
        // normal — and, when it isn't the middle, how far across the face it sits.
        crate::model::MovePointRef::OnFace { body, centroid, normal, uv } => {
            let head = format!(
                "{{ body = {}, on_face = {}, normal = {}",
                body.index(),
                mm_point_lua(*centroid),
                mm_point_lua(*normal)
            );
            if *uv == [0, 0] {
                format!("{head} }}")
            } else {
                format!(
                    "{head}, uv = {{ {}, {} }} }}",
                    uv[0] as f32 / 100.0,
                    uv[1] as f32 / 100.0
                )
            }
        }
        // The world origin (#946): no body, so it spells itself.
        crate::model::MovePointRef::Origin => "{ origin = true }".to_string(),
    }
}

/// A quantized body point (#647) as the `{x, y, z}` **millimetre** table scripts use — the
/// inverse of the parser's re-quantization, so `derive_parameter` round-trips.
fn mm_point_lua(p: [i32; 3]) -> String {
    let v = crate::hierarchy::dequantize_body_point(p);
    format!("{{ {}, {}, {} }}", v.x, v.y, v.z)
}

/// The `axis = …` argument for a [`crate::model::RevolveAxis`], matching what
/// `lua_script::parse_revolve_axis` accepts. Shared by the revolve, move, and repeat calls so
/// every axis kind — including a picked body edge (#643) — round-trips through a script.
pub fn revolve_axis_lua(axis: crate::model::RevolveAxis) -> String {
    match axis {
        crate::model::RevolveAxis::X => "\"x\"".to_string(),
        crate::model::RevolveAxis::Y => "\"y\"".to_string(),
        crate::model::RevolveAxis::Z => "\"z\"".to_string(),
        crate::model::RevolveAxis::Line(li) => format!("{{ line = {} }}", li.index()),
        crate::model::RevolveAxis::BodyEdge { body, a, b } => format!(
            "{{ body = {}, from = {{ {}, {}, {} }}, to = {{ {}, {}, {} }} }}",
            body.index(),
            a.x, a.y, a.z, b.x, b.y, b.z
        ),
    }
}

/// Lua table literal for a `FaceId`, matching `lua_script::parse_face_id_table`'s shape.
/// Cap/side profiles are limited to `rect`/`circle` (same limitation as `face_lua_parts` and
/// `parse_face_id_table` — a polygon profile isn't a single index, #66).
/// Public for document→Lua export (#1159).
pub fn face_id_lua_ref_for_export(
    face: &FaceId,
    doc: &crate::model::Document,
) -> String {
    face_id_lua_ref(face, Some(doc))
}

fn face_id_lua_ref(face: &FaceId, doc: Option<&crate::model::Document>) -> String {
    // Every index below is an **ordinal** among the live elements of its kind when a document
    // was available to count them, and the arena slot otherwise (#1070).
    let circle = |i: crate::model::CircleKey| {
        ordinal_or_slot(doc.map(|d| d.circles.keys().position(|k| k == i)), i.index())
    };
    let plane = |i: crate::model::ConstructionPlaneKey| {
        ordinal_or_slot(
            doc.map(|d| d.construction_planes.keys().position(|k| k == i)),
            i.index(),
        )
    };
    let line = |i: crate::model::LineKey| {
        ordinal_or_slot(doc.map(|d| d.lines.keys().position(|k| k == i)), i.index())
    };
    let extrusion = |i: crate::model::ExtrusionKey| {
        ordinal_or_slot(doc.map(|d| d.extrusions.keys().position(|k| k == i)), i.index())
    };
    let revolution = |i: crate::model::RevolutionKey| {
        ordinal_or_slot(doc.map(|d| d.revolutions.keys().position(|k| k == i)), i.index())
    };
    let instance = |i: crate::model::UnitInstanceKey| {
        ordinal_or_slot(
            doc.map(|d| d.unit_instances.keys().position(|k| k == i)),
            i.index(),
        )
    };
    match face {
        FaceId::Circle(i) => format!("{{ kind = \"circle\", index = {} }}", circle(*i)),
        FaceId::ConstructionPlane(i) => {
            format!("{{ kind = \"construction_plane\", index = {} }}", plane(*i))
        }
        FaceId::Polygon(lines) => format!(
            "{{ kind = \"polygon\", index = {} }}",
            lines.first().map(|l| line(*l)).unwrap_or(0)
        ),
        FaceId::ExtrudeCap { extrusion: e, profile, top } => format!(
            "{{ kind = \"extrude_cap\", extrusion = {}, {}, top = {top} }}",
            extrusion(*e),
            extrude_face_profile_lua_fields(profile, doc)
        ),
        FaceId::ExtrudeSide { extrusion: e, profile, edge } => format!(
            "{{ kind = \"extrude_side\", extrusion = {}, {}, edge = {edge} }}",
            extrusion(*e),
            extrude_face_profile_lua_fields(profile, doc)
        ),
        FaceId::RevolveCap { revolution: r, profile, end } => format!(
            "{{ kind = \"revolve_cap\", revolution = {}, {}, [\"end\"] = {end} }}",
            revolution(*r),
            extrude_face_profile_lua_fields(profile, doc)
        ),
        FaceId::RevolveSide { revolution: r, profile, edge } => format!(
            "{{ kind = \"revolve_side\", revolution = {}, {}, edge = {edge} }}",
            revolution(*r),
            extrude_face_profile_lua_fields(profile, doc)
        ),
        // The inner face rides as its JSON encoding (#725), like unit_edge_length's face.
        FaceId::UnitFace { instance: i, face } => format!(
            "{{ kind = \"unit_face\", instance = {}, face = {:?} }}",
            instance(*i),
            serde_json::to_string(face.as_ref()).unwrap_or_default()
        ),
        FaceId::PrimitiveFace { primitive, face } => format!(
            "{{ kind = \"primitive_face\", primitive = {}, face = {:?} }}",
            primitive.index(),
            serde_json::to_string(face).unwrap_or_default()
        ),
        FaceId::RepeatedFace { face, op, instance } => format!(
            "{{ kind = \"repeated_face\", repeat_op = {}, instance = {instance}, face = {:?} }}",
            op.index(),
            serde_json::to_string(face.as_ref()).unwrap_or_default()
        ),
        FaceId::BodyMeshFace {
            body,
            centroid,
            normal,
        } => format!(
            "{{ kind = \"body_mesh_face\", body = {}, centroid = {:?}, normal = {:?} }}",
            body.index(),
            centroid,
            normal
        ),
    }
}

fn extrude_face_profile_lua_fields(profile: &ExtrudeFace, doc: Option<&crate::model::Document>) -> String {
    match profile {
        ExtrudeFace::Circle(i) => {
            let ordinal =
                ordinal_or_slot(doc.map(|d| d.circles.keys().position(|k| k == *i)), i.index());
            format!("profile = \"circle\", profile_index = {ordinal}")
        }
        // #1512: `parse_face_id_table` reads a polygon's loop from `profile_lines`, as
        // ordinals — a single raw arena index named no profile the reader could resolve.
        ExtrudeFace::Polygon(lines) => format!(
            "profile = \"polygon\", profile_lines = {{{}}}",
            lines
                .iter()
                .map(|l| ordinal_or_slot(
                    doc.map(|d| d.lines.keys().position(|k| k == *l)),
                    l.index()
                )
                .to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        // Round-trippable since #406: `parse_face_id_table` accepts
        // `profile = "boolean", boolean = {...}`.
        ExtrudeFace::Boolean { op, a, b } => format!(
            "profile = \"boolean\", boolean = {}",
            boolean_face_lua_table(*op, a, b, doc)
        ),
        // #1512: both name their host by *ordinal* so the reader can resolve them.
        ExtrudeFace::SketchRegion { sketch, seed_u, seed_v } => {
            let (u, v) = crate::model::sketch_region_seed_point(*seed_u, *seed_v);
            format!(
                "profile = \"region\", sketch = {}, seed = {{{u}, {v}}}",
                ordinal_or_slot(
                    doc.map(|d| d.sketches.keys().position(|k| k == *sketch)),
                    sketch.index()
                )
            )
        }
        ExtrudeFace::TextGlyph { text, glyph } => format!(
            "profile = \"text_glyph\", text = {}, glyph = {glyph}",
            ordinal_or_slot(
                doc.map(|d| d.sketch_texts.keys().position(|k| k == *text)),
                text.index()
            )
        ),
    }
}

/// Lua table literal for an `ExtrusionEdgeRef`, matching `parse_extrusion_edge_table`'s shape
/// (#77): `{ kind = "vertical", face = N, edge = N }` or `{ kind = "cap", face = N, edge = N,
/// top = true/false }`.
fn extrusion_edge_lua_ref(edge: crate::model::ExtrusionEdgeRef) -> String {
    use crate::model::ExtrusionEdgeRef;
    match edge {
        ExtrusionEdgeRef::Vertical { face, edge } => {
            format!("{{ kind = \"vertical\", face = {face}, edge = {edge} }}")
        }
        ExtrusionEdgeRef::Cap { face, edge, top } => {
            format!("{{ kind = \"cap\", face = {face}, edge = {edge}, top = {top} }}")
        }
    }
}

fn distance_target_lua_ref(target: &DistanceTarget) -> String {
    match target {
        DistanceTarget::LineLength(index) => {
            format!("{{ kind = \"line\", index = {} }}", index.index())
        }
        DistanceTarget::CircleDiameter(index) => {
            format!("{{ kind = \"circle\", index = {} }}", index.index())
        }
        DistanceTarget::LineLineDistance { .. }
        | DistanceTarget::PointPointDistance { .. }
        | DistanceTarget::PointLineDistance { .. } => {
            "{ kind = \"selection\" }".to_string()
        }
    }
}

/// Queued synthetic pointer/keyboard events injected into egui each frame.
#[derive(Default)]
pub struct SyntheticInput {
    /// Event batches, one per frame, delivered through eframe's `raw_input_hook` —
    /// so synthetic pointer input builds *real* egui pointer state (presses, drags,
    /// hover) exactly like OS events, and spreads across frames the way tool
    /// handlers expect (press one frame, move the next, release after).
    frames: std::collections::VecDeque<Vec<egui::Event>>,
    pointer_pos: Option<egui::Pos2>,
    /// When set, secondary-button drag deltas are applied via events.
    pending_right_drag: Option<(egui::Vec2, Modifiers)>,
}

impl SyntheticInput {
    /// The next frame's synthetic events, consumed by `raw_input_hook`.
    pub fn take_raw_frame(&mut self) -> Option<Vec<egui::Event>> {
        self.frames.pop_front()
    }

    fn push_batch(&mut self, events: Vec<egui::Event>) {
        self.frames.push_back(events);
    }

    fn push_event(&mut self, event: egui::Event) {
        self.frames.push_back(vec![event]);
    }

    /// Apply secondary-button drag after egui has processed pointer state.
    pub fn apply_pending_drag(&mut self, viewport: egui::Rect, on_drag: impl FnMut(egui::Vec2, Modifiers, f32)) {
        if let Some((delta, modifiers)) = self.pending_right_drag.take() {
            let mut callback = on_drag;
            callback(delta, modifiers, viewport.height());
        }
    }

    fn viewport_pos(viewport: egui::Rect, x: f32, y: f32) -> egui::Pos2 {
        viewport.min + egui::vec2(x, y)
    }

    pub fn move_to(&mut self, viewport: egui::Rect, x: f32, y: f32) {
        let pos = Self::viewport_pos(viewport, x, y);
        self.pointer_pos = Some(pos);
        self.push_event(egui::Event::PointerMoved(pos));
    }

    /// A click with modifiers optionally held (#835/#984) — Shift for a tool's second role
    /// (multi-select, the in-sketch repeat's direction edge), Control for a single-edge pick.
    pub fn click_with(&mut self, viewport: egui::Rect, x: f32, y: f32, mods: ClickMods) {
        let pos = Self::viewport_pos(viewport, x, y);
        self.pointer_pos = Some(pos);
        let modifiers = mods.egui();
        // Hover one frame, press the next, release the one after — the exact shape
        // tool handlers (press-frame logic, select-then-drag) are written against.
        self.push_event(egui::Event::PointerMoved(pos));
        self.push_event(egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        });
        self.push_event(egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers,
        });
    }

    pub fn drag(&mut self, viewport: egui::Rect, x0: f32, y0: f32, x1: f32, y1: f32) {
        let p0 = Self::viewport_pos(viewport, x0, y0);
        let p1 = Self::viewport_pos(viewport, x1, y1);
        self.pointer_pos = Some(p1);
        self.push_event(egui::Event::PointerMoved(p0));
        self.push_event(egui::Event::PointerButton {
            pos: p0,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        // Several interpolated moves: drag handlers integrate per-frame deltas.
        for step in 1..=4 {
            let t = step as f32 / 4.0;
            self.push_event(egui::Event::PointerMoved(p0 + (p1 - p0) * t));
        }
        self.push_event(egui::Event::PointerButton {
            pos: p1,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
    }

    pub fn right_drag(&mut self, viewport: egui::Rect, dx: f32, dy: f32, shift: bool) {
        let pos = self
            .pointer_pos
            .unwrap_or_else(|| viewport.center());
        let modifiers = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
        self.pending_right_drag = Some((egui::vec2(dx, dy), modifiers));
        self.push_batch(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Secondary,
                pressed: true,
                modifiers,
            },
            egui::Event::PointerButton {
                pos: pos + egui::vec2(dx, dy),
                button: PointerButton::Secondary,
                pressed: false,
                modifiers,
            },
        ]);
    }

    /// A key tap with modifiers held for both the press and the release (#1198).
    /// Pass [`ClickMods::default`] for a plain key.
    pub fn key_with(&mut self, key: Key, mods: ClickMods) {
        self.push_key(key, true, mods);
        self.push_key(key, false, mods);
    }

    pub fn key_down(&mut self, key: Key) {
        self.push_key(key, true, ClickMods::default());
    }

    pub fn key_up(&mut self, key: Key) {
        self.push_key(key, false, ClickMods::default());
    }

    fn push_key(&mut self, key: Key, pressed: bool, mods: ClickMods) {
        self.push_event(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: mods.egui(),
        });
    }

    pub fn type_text(&mut self, text: &str) {
        self.push_event(egui::Event::Text(text.to_string()));
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct LuaRunner {
    lua: Lua,
    thread: mlua::Thread,
    finished: bool,
}

/// Web builds ship without the Lua runtime (mlua's bundled C doesn't compile for
/// wasm32-unknown-unknown); this stub keeps `ScriptRunner`'s shape identical.
#[cfg(target_arch = "wasm32")]
struct LuaRunner {
    finished: bool,
}

/// Interactive Lua REPL state (`--repl`): one persistent `Lua` for the whole session (so
/// globals survive between entries, like a normal Lua REPL), fed complete input chunks over a
/// channel by a stdin reader thread, each chunk run as a coroutine through the same per-frame
/// tick machinery scripts use (so `bearcad.ui.wait`/screenshots work from the REPL too).
///
/// The reader thread and the app hand off with a ready/prompt protocol: the app sends the
/// prompt to print when it's ready for input ([`REPL_PROMPT`], or [`REPL_CONT_PROMPT`] while a
/// multi-line entry is incomplete), the reader prints it, blocks on a line, sends it back, and
/// nudges the event loop awake via the installed `egui::Context`.
#[cfg(not(target_arch = "wasm32"))]
struct ReplRunner {
    lua: Lua,
    /// The coroutine for the entry currently executing, if any.
    active: Option<mlua::Thread>,
    /// Accumulated multi-line input (kept until it parses as a complete chunk).
    buffer: String,
    lines_rx: std::sync::mpsc::Receiver<String>,
    ready_tx: std::sync::mpsc::Sender<&'static str>,
    /// Wakes the winit event loop when input arrives while the app is idle; installed once
    /// the eframe context exists (see [`ScriptRunner::install_repaint_context`]).
    repaint_ctx: std::sync::Arc<std::sync::OnceLock<egui::Context>>,
}

/// Primary REPL prompt.
pub const REPL_PROMPT: &str = "bearcad> ";
/// Continuation prompt while a multi-line entry is syntactically incomplete.
pub const REPL_CONT_PROMPT: &str = "    ...> ";

/// What the REPL's accumulated input buffer parses to.
#[cfg(not(target_arch = "wasm32"))]
enum ChunkOutcome {
    /// A complete chunk, ready to execute as a coroutine.
    Ready(mlua::Thread),
    /// Syntactically incomplete (e.g. an unclosed `function`): keep buffering lines.
    Incomplete,
    /// A real syntax error: report it and reset the buffer.
    SyntaxError(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl ReplRunner {
    /// Compile the buffered input. Tries `return <input>` first (so a bare expression like
    /// `1 + 2` or `bearcad.find("Main box")` echoes its value, as in the standalone Lua
    /// REPL), then the plain chunk. Lua reports unfinished constructs distinctly
    /// (`incomplete_input`), which is what drives multi-line entry.
    fn load_buffered_chunk(&self) -> ChunkOutcome {
        let as_expression = format!("return {}", self.buffer);
        let func = match self.lua.load(&as_expression).into_function() {
            Ok(f) => Ok(f),
            Err(_) => self.lua.load(&self.buffer).into_function(),
        };
        match func {
            Ok(f) => match self.lua.create_thread(f) {
                Ok(t) => ChunkOutcome::Ready(t),
                Err(e) => ChunkOutcome::SyntaxError(e.to_string()),
            },
            Err(mlua::Error::SyntaxError {
                incomplete_input: true,
                ..
            }) => ChunkOutcome::Incomplete,
            Err(e) => ChunkOutcome::SyntaxError(e.to_string()),
        }
    }

    /// Echo an entry's returned values, `tostring`-rendered and tab-separated (nothing for
    /// statements, which return no values).
    fn print_values(&self, values: &mlua::MultiValue) {
        if values.is_empty() {
            return;
        }
        let tostring: mlua::Function = match self.lua.globals().get("tostring") {
            Ok(f) => f,
            Err(_) => return,
        };
        let rendered: Vec<String> = values
            .iter()
            .map(|v| {
                tostring
                    .call::<mlua::String>(v.clone())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|_| format!("{v:?}"))
            })
            .collect();
        println!("{}", rendered.join("\t"));
    }
}

#[cfg(target_arch = "wasm32")]
struct ReplRunner {}

/// egui data key under which a pane records where it landed this frame, so a
/// scripted screenshot can crop to it (#672). `shell_id` is the pane's panel id.
pub fn pane_rect_id(shell_id: &str) -> egui::Id {
    egui::Id::new(("bearcad_pane_rect", shell_id))
}

/// The panel id a pane draws itself under, or `None` for the view-cube HUD, which
/// is drawn inside the viewport rather than as a pane of its own.
fn pane_shell_id(pane: crate::actions::Pane) -> Option<&'static str> {
    use crate::actions::Pane;
    match pane {
        Pane::Hierarchy => Some("tree"),
        Pane::Parameters => Some("parameters"),
        Pane::Context => Some("context"),
        Pane::Tutorials => Some("tutorials"),
        Pane::Ai => Some("ai"),
        Pane::ViewCube => None,
    }
}

/// Where `pane` was drawn last frame, in logical points. `None` when it is hidden.
pub fn pane_rect(ctx: &egui::Context, pane: crate::actions::Pane) -> Option<egui::Rect> {
    let id = pane_rect_id(pane_shell_id(pane)?);
    ctx.data(|data| data.get_temp::<egui::Rect>(id))
}

/// A pending screenshot request, resolved when egui delivers the captured frame.
struct ScreenshotRequest {
    path: String,
    /// `Some` crops the captured framebuffer to the 3D viewport; `None` keeps the whole window.
    crop: Option<ScreenshotCrop>,
    /// Frames waited since the capture command was last sent (#872).
    frames_waited: u32,
    /// How many times the command has been sent, the first included.
    attempts: u32,
}

/// Frames to wait for a captured frame before asking again (#872): an occluded window
/// skips its paint, and wgpu ≥29 reports that as `CurrentSurfaceTexture::Occluded` — the
/// capture request is dropped along with the frame, so it has to be re-sent.
/// Shared with the DEV report-issue capture path (#1177).
pub(crate) const SCREENSHOT_RETRY_FRAMES: u32 = 10;
/// How many times to ask before giving up, so a permanently hidden window fails the
/// script instead of hanging until `--timeout`.
pub(crate) const SCREENSHOT_MAX_ATTEMPTS: u32 = 12;

struct ScreenshotCrop {
    /// 3D viewport rect in logical points.
    rect: egui::Rect,
    /// Logical-to-physical pixel ratio of the captured framebuffer.
    pixels_per_point: f32,
}

/// Drives a script through the live application, one step at a time.
pub struct ScriptRunner {
    instructions: Vec<Instruction>,
    lua: Option<LuaRunner>,
    repl: Option<ReplRunner>,
    pc: usize,
    wait_until: Option<Instant>,
    wait_frames_remaining: u32,
    screenshot_pending: Option<ScreenshotRequest>,
    waiting_view_transition: bool,
    /// Prevents re-printing an instruction while waiting (e.g. for viewport layout).
    logged_pc: Option<usize>,
    /// Set when a declarative modeling instruction's underlying action is rejected
    /// (#104/#109/#110/#112); the Lua bindings (`ScriptTickData::exec`) raise it as a
    /// script error so invalid input fails loudly instead of silently doing nothing.
    /// Instruction-list playback ignores it (the GUI status bar already reports it).
    pub(crate) last_action_error: Option<String>,
    pub verbose: bool,
    pub done: bool,
    pub error: Option<String>,
    pub should_quit: bool,
    /// Tab / document-id ops that need the `App` workspace (not just `AppState`).
    pub(crate) pending_tab_ops: Vec<TabOp>,
    /// File→New / Open replaced the document in the active tab — rebind its document id.
    pub(crate) rebind_active_document: bool,
}

/// Workspace-level tab operations queued by script instructions and applied by `App`.
#[derive(Clone, Debug)]
pub(crate) enum TabOp {
    NewBlank,
    NewSameDocument,
    Close { index: Option<usize> },
    Select { index: usize },
    Reorder { from: usize, to: usize },
    Detach { index: Option<usize> },
}

impl ScriptRunner {
    pub fn from_instructions(instructions: Vec<Instruction>) -> Self {
        Self {
            instructions,
            lua: None,
            repl: None,
            pc: 0,
            wait_until: None,
            wait_frames_remaining: 0,
            screenshot_pending: None,
            waiting_view_transition: false,
            logged_pc: None,
            last_action_error: None,
            verbose: true,
            done: false,
            error: None,
            should_quit: false,
            pending_tab_ops: Vec::new(),
            rebind_active_document: false,
        }
    }

    /// Load a Lua source string (tests + DEV verify export #1159).
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub fn from_lua_source(source: &str) -> Result<Self, ScriptError> {
        let lua = Lua::new();
        crate::lua_script::register_api(&lua).map_err(|e| ScriptError {
            message: e.to_string(),
        })?;
        let func = lua.load(source).into_function().map_err(|e| ScriptError {
            message: e.to_string(),
        })?;
        let thread = lua.create_thread(func).map_err(|e| ScriptError {
            message: e.to_string(),
        })?;
        let mut runner = Self::from_instructions(vec![]);
        runner.lua = Some(LuaRunner {
            lua,
            thread,
            finished: false,
        });
        Ok(runner)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_file(path: &Path) -> Result<Self, ScriptError> {
        if path.extension().and_then(|e| e.to_str()) != Some("lua") {
            return Err(ScriptError {
                message: format!(
                    "scripts must use the .lua extension: {}",
                    path.display()
                ),
            });
        }
        let lua = Lua::new();
        let thread = load_script(&lua, path).map_err(|e| ScriptError {
            message: e.to_string(),
        })?;
        let mut runner = Self::from_instructions(vec![]);
        runner.lua = Some(LuaRunner {
            lua,
            thread,
            finished: false,
        });
        if runner.verbose {
            println!("Running script: {}", path.display());
            println!("---");
        }
        Ok(runner)
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Interactive Lua REPL on stdin against the live app (`--repl`). Spawns the stdin
    /// reader thread; entries evaluate in one persistent Lua state (globals survive between
    /// entries), errors print and the session continues, and EOF (Ctrl-D) ends it.
    pub fn repl() -> Result<Self, ScriptError> {
        let (lines_tx, lines_rx) = std::sync::mpsc::channel::<String>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<&'static str>();
        let runner = Self::repl_from_channels(lines_rx, ready_tx)?;
        let repaint_ctx = runner
            .repl
            .as_ref()
            .expect("repl_from_channels sets repl")
            .repaint_ctx
            .clone();

        std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let stdin = std::io::stdin();
            let mut input = stdin.lock();
            // The app sends the prompt to print whenever it's ready for the next line.
            while let Ok(prompt) = ready_rx.recv() {
                print!("{prompt}");
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                match input.read_line(&mut line) {
                    // EOF (Ctrl-D): drop `lines_tx` by leaving the loop, which the app sees
                    // as a disconnect and ends the REPL.
                    Ok(0) | Err(_) => {
                        println!();
                        break;
                    }
                    Ok(_) => {
                        if lines_tx.send(line).is_err() {
                            break;
                        }
                        // The app may be idle (no repaints scheduled); wake it to evaluate.
                        if let Some(ctx) = repaint_ctx.get() {
                            ctx.request_repaint();
                        }
                    }
                }
            }
            if let Some(ctx) = repaint_ctx.get() {
                ctx.request_repaint();
            }
        });

        println!("BearCAD Lua REPL — the `bearcad` API is available; globals persist between");
        println!("entries; Ctrl-D ends the session.");
        Ok(runner)
    }

    /// REPL core without the stdin thread: complete lines arrive on `lines_rx`, and the
    /// runner sends the next prompt on `ready_tx` whenever it's ready for input. Split out
    /// so tests can drive a REPL session without a terminal.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn repl_from_channels(
        lines_rx: std::sync::mpsc::Receiver<String>,
        ready_tx: std::sync::mpsc::Sender<&'static str>,
    ) -> Result<Self, ScriptError> {
        let lua = Lua::new();
        crate::lua_script::register_api(&lua).map_err(|e| ScriptError {
            message: e.to_string(),
        })?;
        // First prompt: the reader prints it as soon as it starts.
        let _ = ready_tx.send(REPL_PROMPT);
        let mut runner = Self::from_instructions(vec![]);
        runner.repl = Some(ReplRunner {
            lua,
            active: None,
            buffer: String::new(),
            lines_rx,
            ready_tx,
            repaint_ctx: std::sync::Arc::new(std::sync::OnceLock::new()),
        });
        Ok(runner)
    }

    /// Whether this runner is an interactive REPL session.
    pub fn is_repl(&self) -> bool {
        self.repl.is_some()
    }

    /// Give the REPL's stdin reader thread a way to wake the event loop when input arrives
    /// while the app is idle. Called once the eframe context exists; a no-op for scripts.
    pub fn install_repaint_context(&self, ctx: egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(repl) = &self.repl {
            let _ = repl.repaint_ctx.set(ctx);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    fn log_instruction(&mut self, instr: &Instruction) {
        if self.verbose && self.logged_pc != Some(self.pc) {
            println!("{}", instr.as_lua());
            self.logged_pc = Some(self.pc);
        }
    }

    pub fn is_waiting(&self) -> bool {
        self.wait_until.is_some()
            || self.wait_frames_remaining > 0
            || self.screenshot_pending.is_some()
            || self.waiting_view_transition
    }

    fn clear_instruction_wait(&mut self) {
        self.wait_until = None;
        self.pc += 1;
        self.logged_pc = None;
    }

    fn advance_after_wait(&mut self) {
        if self.lua.is_some() {
            self.logged_pc = None;
        } else {
            self.clear_instruction_wait();
        }
    }

    /// Advance the script. Returns true if a repaint should be requested.
    pub fn tick(
        &mut self,
        state: &mut AppState,
        synthetic: &mut SyntheticInput,
        viewport: Option<egui::Rect>,
        ctx: &egui::Context,
    ) -> bool {
        // Finder / `bearcad.ui.os_open` queue (#1326): same drain the GUI uses.
        #[cfg(not(target_arch = "wasm32"))]
        {
            for path in crate::file_association::take_os_open_documents() {
                let _ = state.apply(Action::Open { path });
            }
        }
        if self.repl.is_some() {
            return self.tick_repl_mode(state, synthetic, viewport, ctx);
        }
        if self.lua.is_some() {
            return self.tick_lua_mode(state, synthetic, viewport, ctx);
        }
        self.tick_instructions(state, synthetic, viewport, ctx)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn tick_repl_mode(
        &mut self,
        state: &mut AppState,
        synthetic: &mut SyntheticInput,
        viewport: Option<egui::Rect>,
        ctx: &egui::Context,
    ) -> bool {
        if self.done {
            return false;
        }

        // Same wait handling as scripts (`bearcad.ui.wait`, view transitions, screenshots),
        // so an in-flight REPL entry can use every yielding API.
        if let Some(until) = self.wait_until {
            if Instant::now() < until {
                return true;
            }
            self.wait_until = None;
        }
        if self.wait_frames_remaining > 0 {
            self.wait_frames_remaining -= 1;
            return true;
        }
        if self.waiting_view_transition {
            if state.cam.is_transitioning() {
                return true;
            }
            self.waiting_view_transition = false;
        }
        if self.tick_pending_screenshot(ctx) {
            return true;
        }

        let runner_ptr = self as *mut ScriptRunner;
        let repl = self.repl.as_mut().unwrap();

        // An entry is executing: resume its coroutine one step.
        if let Some(thread) = repl.active.clone() {
            repl.lua.set_app_data(ScriptTickData {
                runner: runner_ptr,
                state: state as *mut AppState,
                synthetic: synthetic as *mut SyntheticInput,
                viewport,
                ctx: ctx as *const egui::Context as *mut egui::Context,
            });
            match thread.resume::<mlua::MultiValue>(()) {
                Ok(values) => match thread.status() {
                    mlua::ThreadStatus::Resumable | mlua::ThreadStatus::Running => true,
                    mlua::ThreadStatus::Finished | mlua::ThreadStatus::Error => {
                        // Entry finished: echo any returned values (expression results),
                        // then hand the prompt back. Errors were surfaced by resume::Err.
                        repl.print_values(&values);
                        repl.active = None;
                        let _ = repl.ready_tx.send(REPL_PROMPT);
                        false
                    }
                },
                Err(e) => {
                    // A REPL survives errors: report and hand the prompt back.
                    println!("error: {e}");
                    repl.active = None;
                    let _ = repl.ready_tx.send(REPL_PROMPT);
                    false
                }
            }
        } else {
            // Idle: look for the next complete input chunk.
            match repl.lines_rx.try_recv() {
                Ok(line) => {
                    repl.buffer.push_str(&line);
                    match repl.load_buffered_chunk() {
                        ChunkOutcome::Ready(thread) => {
                            repl.buffer.clear();
                            repl.active = Some(thread);
                            // Start executing on the next tick.
                            true
                        }
                        ChunkOutcome::Incomplete => {
                            let _ = repl.ready_tx.send(REPL_CONT_PROMPT);
                            false
                        }
                        ChunkOutcome::SyntaxError(msg) => {
                            println!("error: {msg}");
                            repl.buffer.clear();
                            let _ = repl.ready_tx.send(REPL_PROMPT);
                            false
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Stdin closed (Ctrl-D): the session is over.
                    self.done = true;
                    false
                }
            }
        }
    }

    /// Web stubs: no Lua/REPL runners exist on wasm, so these branches are unreachable.
    #[cfg(target_arch = "wasm32")]
    fn tick_repl_mode(
        &mut self,
        _state: &mut AppState,
        _synthetic: &mut SyntheticInput,
        _viewport: Option<egui::Rect>,
        _ctx: &egui::Context,
    ) -> bool {
        self.done = true;
        false
    }

    #[cfg(target_arch = "wasm32")]
    fn tick_lua_mode(
        &mut self,
        _state: &mut AppState,
        _synthetic: &mut SyntheticInput,
        _viewport: Option<egui::Rect>,
        _ctx: &egui::Context,
    ) -> bool {
        self.done = true;
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn tick_lua_mode(
        &mut self,
        state: &mut AppState,
        synthetic: &mut SyntheticInput,
        viewport: Option<egui::Rect>,
        ctx: &egui::Context,
    ) -> bool {
        if self.done {
            return false;
        }

        if let Some(until) = self.wait_until {
            if Instant::now() < until {
                return true;
            }
            self.wait_until = None;
            self.advance_after_wait();
        }

        if self.wait_frames_remaining > 0 {
            self.wait_frames_remaining -= 1;
            if self.wait_frames_remaining == 0 {
                self.advance_after_wait();
            }
            return true;
        }

        if self.waiting_view_transition {
            if state.cam.is_transitioning() {
                return true;
            }
            self.waiting_view_transition = false;
            self.advance_after_wait();
        }

        if self.tick_pending_screenshot(ctx) {
            return true;
        }

        let runner_ptr = self as *mut ScriptRunner;
        let lua_runner = self.lua.as_mut().unwrap();
        if lua_runner.finished {
            self.done = true;
            return false;
        }

        lua_runner.lua.set_app_data(ScriptTickData {
            runner: runner_ptr,
            state: state as *mut AppState,
            synthetic: synthetic as *mut SyntheticInput,
            viewport,
            ctx: ctx as *const egui::Context as *mut egui::Context,
        });

        match lua_runner.thread.resume::<()>(()) {
            Ok(_) => match lua_runner.thread.status() {
                mlua::ThreadStatus::Finished => {
                    lua_runner.finished = true;
                    self.done = true;
                    if self.verbose {
                        println!("---");
                        println!("Script complete.");
                    }
                    false
                }
                mlua::ThreadStatus::Resumable => true,
                mlua::ThreadStatus::Running => true,
                mlua::ThreadStatus::Error => {
                    self.error = Some("Lua thread error".to_string());
                    if self.verbose {
                        eprintln!("Script error: Lua thread error");
                    }
                    lua_runner.finished = true;
                    self.done = true;
                    false
                }
            },
            Err(e) => {
                self.error = Some(e.to_string());
                // Surface the failure on the terminal too — without this the error only
                // lands in the status bar, which reads as a silent hang when running
                // headless without `--exit`.
                if self.verbose {
                    eprintln!("Script error: {e}");
                }
                lua_runner.finished = true;
                self.done = true;
                false
            }
        }
    }

    fn tick_instructions(
        &mut self,
        state: &mut AppState,
        synthetic: &mut SyntheticInput,
        viewport: Option<egui::Rect>,
        ctx: &egui::Context,
    ) -> bool {
        if self.done {
            return false;
        }

        if let Some(until) = self.wait_until {
            if Instant::now() < until {
                return true;
            }
            self.clear_instruction_wait();
        }

        if self.wait_frames_remaining > 0 {
            self.wait_frames_remaining -= 1;
            if self.wait_frames_remaining == 0 {
                self.clear_instruction_wait();
            }
            return true;
        }

        if self.waiting_view_transition {
            if state.cam.is_transitioning() {
                return true;
            }
            self.waiting_view_transition = false;
            self.clear_instruction_wait();
        }

        if self.tick_pending_screenshot(ctx) {
            return true;
        }

        while self.pc < self.instructions.len() {
            let instr = self.instructions[self.pc].clone();
            self.log_instruction(&instr);
            match self.execute_instruction(instr, state, synthetic, viewport, ctx) {
                StepResult::Continue => {
                    self.pc += 1;
                }
                StepResult::Wait => return true,
                StepResult::Done => {
                    self.done = true;
                    return false;
                }
            }
        }

        self.done = true;
        if self.verbose {
            println!("---");
            println!("Script complete.");
        }
        false
    }

    pub(crate) fn execute_instruction(
        &mut self,
        instr: Instruction,
        state: &mut AppState,
        synthetic: &mut SyntheticInput,
        viewport: Option<egui::Rect>,
        ctx: &egui::Context,
    ) -> StepResult {
        let result = self.execute_one(instr, state, synthetic, viewport, ctx);
        if self.should_quit {
            if let Some(lua_runner) = self.lua.as_mut() {
                lua_runner.finished = true;
            }
            self.done = true;
            return StepResult::Done;
        }
        result
    }

    /// Keep a pending capture alive: `true` while the script is still waiting for one.
    ///
    /// A frame the window server skips takes the pending capture request down with it, so
    /// nothing ever comes back (#872) — on macOS wgpu skips every frame while the window
    /// is fully covered or the display is asleep. Ask again every few frames, and fail the
    /// script with a reason rather than hang until `--timeout` if it never lands.
    fn tick_pending_screenshot(&mut self, ctx: &egui::Context) -> bool {
        let Some(request) = self.screenshot_pending.as_mut() else {
            return false;
        };
        request.frames_waited += 1;
        if request.frames_waited < SCREENSHOT_RETRY_FRAMES {
            return true;
        }
        request.frames_waited = 0;
        if request.attempts >= SCREENSHOT_MAX_ATTEMPTS {
            let path = request.path.clone();
            self.screenshot_pending = None;
            let message = format!(
                "screenshot '{path}' was never delivered — the window never painted \
                 (fully covered, minimized, or the display is asleep)"
            );
            eprintln!("Script error: {message}");
            self.error = Some(message);
            self.done = true;
            return true;
        }
        request.attempts += 1;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        true
    }

    /// Called when egui delivers a screenshot response for a pending request.
    pub fn on_screenshot(&mut self, image: &egui::ColorImage) -> Result<(), String> {
        let Some(request) = self.screenshot_pending.take() else {
            return Ok(());
        };
        match request.crop {
            Some(crop) => {
                save_screenshot_cropped(&request.path, image, crop.rect, crop.pixels_per_point)?
            }
            None => save_screenshot(&request.path, image)?,
        }
        if self.lua.is_none() {
            self.pc += 1;
        }
        Ok(())
    }

    /// Whether the view-cube HUD should be hidden this frame for a pending viewport screenshot.
    pub fn screenshot_suppresses_hud(&self) -> bool {
        self.screenshot_pending
            .as_ref()
            .is_some_and(|request| request.crop.is_some())
    }

    /// Hide the viewport tool-hint line while any screenshot is in flight (#1509).
    pub fn screenshot_suppresses_tool_hints(&self) -> bool {
        self.screenshot_pending.is_some()
    }
}

pub(crate) enum StepResult {
    Continue,
    Wait,
    Done,
}

impl ScriptRunner {
    fn ground_pointer(
        synthetic: &mut SyntheticInput,
        state: &AppState,
        viewport: Option<egui::Rect>,
        x: f32,
        y: f32,
        // `Some(mods)` clicks (holding those modifiers); `None` only moves the pointer.
        click: Option<ClickMods>,
    ) {
        let Some(vp) = viewport else { return };
        let world = Vec3::new(x, y, 0.0);
        let mat = state.cam.view_proj(vp);
        let Some(screen) = state.cam.project(world, vp, &mat) else {
            return;
        };
        let local_x = screen.x - vp.min.x;
        let local_y = screen.y - vp.min.y;
        match click {
            Some(mods) => synthetic.click_with(vp, local_x, local_y, mods),
            None => synthetic.move_to(vp, local_x, local_y),
        }
    }

    /// Stashes a rejected declarative-modeling or file-I/O action's message in
    /// [`ScriptRunner::last_action_error`] so `ScriptTickData::exec` can raise it as a Lua
    /// error (#104/#109/#110/#112, #106 for open/save/import/export).
    fn record_action_error(&mut self, result: ActionResult) {
        if let ActionResult::Err(e) = result {
            self.last_action_error = Some(e);
        }
    }

    fn execute_one(
        &mut self,
        instr: Instruction,
        state: &mut AppState,
        synthetic: &mut SyntheticInput,
        viewport: Option<egui::Rect>,
        ctx: &egui::Context,
    ) -> StepResult {
        match instr {
            Instruction::New => {
                state.apply(Action::NewDocument);
                self.rebind_active_document = true;
                StepResult::Continue
            }
            Instruction::Open(path) => {
                let r = state.apply(Action::Open { path });
                self.record_action_error(r);
                self.rebind_active_document = true;
                StepResult::Continue
            }
            Instruction::NewTab => {
                self.pending_tab_ops.push(TabOp::NewBlank);
                StepResult::Continue
            }
            Instruction::NewTabSameDocument => {
                self.pending_tab_ops.push(TabOp::NewSameDocument);
                StepResult::Continue
            }
            Instruction::CloseTab { index } => {
                self.pending_tab_ops.push(TabOp::Close { index });
                StepResult::Continue
            }
            Instruction::SelectTab { index } => {
                self.pending_tab_ops.push(TabOp::Select { index });
                StepResult::Continue
            }
            Instruction::ReorderTab { from, to } => {
                self.pending_tab_ops.push(TabOp::Reorder { from, to });
                StepResult::Continue
            }
            Instruction::DetachTab { index } => {
                self.pending_tab_ops.push(TabOp::Detach { index });
                StepResult::Continue
            }
            Instruction::Save(path) => {
                let r = state.apply(Action::Save { path });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::RebuildGeometry => {
                let r = state.apply(Action::ForceRebuildGeometry);
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ExportStl { path, body } => {
                let r = state.apply(Action::ExportStl { path, body });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::Export3mf { path, body } => {
                let r = state.apply(Action::Export3mf { path, body });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ExportStep { path, body } => {
                let r = state.apply(Action::ExportStep { path, body });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ExportPreview { path } => {
                match crate::file_preview::export_preview_png(&state.doc, &path) {
                    Ok(()) => {}
                    Err(e) => self.record_action_error(crate::actions::ActionResult::Err(e)),
                }
                StepResult::Continue
            }
            Instruction::ImportStl { path } => {
                let r = state.apply(Action::ImportStl { path });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ImportUnit { path, link, name } => {
                let r = state.apply(Action::ImportUnit { path, link, name });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ImportImage { path, plane } => {
                let plane = match plane.map(|p| plane_key(&state.doc, p)) {
                    Some(None) => {
                        self.last_action_error = Some("No such construction plane".to_string());
                        return StepResult::Continue;
                    }
                    other => other.flatten(),
                };
                let r = state.apply(Action::ImportImage { path, plane });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetImageOpacity { image, opacity, expression } => {
                let Some(image) = image_key(&state.doc, image) else {
                    self.last_action_error = Some(format!("Image {image} not found"));
                    return StepResult::Continue;
                };
                let mut expression = expression;
                if let Err(e) = crate::actions::commit_inline_parameter_defs(
                    &mut state.doc,
                    [&mut expression],
                ) {
                    self.record_action_error(crate::actions::ActionResult::Err(e));
                    return StepResult::Continue;
                }
                let opacity = if !expression.trim().is_empty() {
                    match crate::value::eval_count_in_doc(&expression, &state.doc) {
                        Some(v) => v,
                        None => {
                            self.last_action_error =
                                Some(format!("Not a usable opacity: {expression}"));
                            return StepResult::Continue;
                        }
                    }
                } else {
                    opacity
                };
                let result = state.apply(Action::SetImageOpacity { image, opacity });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetCalibrationPoint { image, index, x, y } => {
                let Some(image) = image_key(&state.doc, image) else {
                    self.last_action_error = Some(format!("Image {image} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::SetCalibrationPoint { image, index, x, y });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::RemoveCalibrationPoint { image, index } => {
                let Some(image) = image_key(&state.doc, image) else {
                    self.last_action_error = Some(format!("Image {image} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::RemoveCalibrationPoint { image, index });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CalibrateImage { image, a, b, length, expression } => {
                let Some(image) = image_key(&state.doc, image) else {
                    self.last_action_error = Some(format!("Image {image} not found"));
                    return StepResult::Continue;
                };
                let mut expression = expression;
                if let Err(e) = crate::actions::commit_inline_parameter_defs(
                    &mut state.doc,
                    [&mut expression],
                ) {
                    self.record_action_error(crate::actions::ActionResult::Err(e));
                    return StepResult::Continue;
                }
                let length = if !expression.trim().is_empty() {
                    match crate::value::eval_parameter_in_doc(&expression, &state.doc) {
                        Some(crate::value::EvaluatedParameter::LengthMm(v)) if v > 0.0 => v,
                        _ => {
                            self.last_action_error =
                                Some(format!("Not a usable length: {expression}"));
                            return StepResult::Continue;
                        }
                    }
                } else {
                    length
                };
                let (a, b) = match (a, b) {
                    (Some(a), Some(b)) => (a, b),
                    _ => {
                        let Some(img) = state.doc.tracing_images.get_mut(image) else {
                            self.last_action_error = Some(format!("Image {image:?} not found"));
                            return StepResult::Continue;
                        };
                        crate::model::ensure_image_calibration(img);
                        let img = &state.doc.tracing_images[image];
                        let Some(a) = crate::model::image_calibration_point_uv(img, 0) else {
                            self.last_action_error =
                                Some("Image has no calibration points".to_string());
                            return StepResult::Continue;
                        };
                        let Some(b) = crate::model::image_calibration_point_uv(img, 1) else {
                            self.last_action_error =
                                Some("Image has no calibration points".to_string());
                            return StepResult::Continue;
                        };
                        (a, b)
                    }
                };
                let r = state.apply(Action::CalibrateImage {
                    image,
                    a,
                    b,
                    length,
                    expression,
                });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ImportStep { path } => {
                let r = state.apply(Action::ImportStep { path });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ImportLua { path, force } => {
                // #1160: replay a document Lua export (File → Export → Lua Script…).
                // Nested runner to completion so the next script line sees the result.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if !force && !crate::export_lua::document_is_blank(&state.doc) {
                        self.last_action_error = Some(
                            "import_lua: document is not blank; pass force = true to replace it"
                                .to_string(),
                        );
                        return StepResult::Continue;
                    }
                    let p = std::path::Path::new(&path);
                    if p.extension().and_then(|e| e.to_str()) != Some("lua") {
                        self.last_action_error = Some(format!(
                            "import_lua: scripts must use the .lua extension: {path}"
                        ));
                        return StepResult::Continue;
                    }
                    let source = match std::fs::read_to_string(p) {
                        Ok(s) => s,
                        Err(e) => {
                            self.last_action_error =
                                Some(format!("import_lua: could not read {path}: {e}"));
                            return StepResult::Continue;
                        }
                    };
                    let mut nested = match ScriptRunner::from_lua_source(&source) {
                        Ok(r) => r,
                        Err(e) => {
                            self.last_action_error =
                                Some(format!("import_lua: {}", e.message));
                            return StepResult::Continue;
                        }
                    };
                    nested.verbose = false;
                    while !nested.done {
                        nested.tick(state, synthetic, viewport, ctx);
                    }
                    if let Some(err) = nested.error {
                        self.last_action_error = Some(format!("import_lua: {err}"));
                    } else {
                        state.status = format!("Imported Lua script: {path}");
                    }
                    self.rebind_active_document = true;
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = (path, force);
                    self.last_action_error = Some(
                        "import_lua: no filesystem to read a path from".to_string(),
                    );
                }
                StepResult::Continue
            }
            Instruction::Clear => {
                state.apply(Action::Clear);
                StepResult::Continue
            }
            Instruction::CopySelection => {
                let result = state.apply(Action::CopySelection);
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::PasteAt { linked, x, y, z } => {
                let result = state.apply(Action::PasteAt {
                    linked,
                    offset: glam::Vec3::new(x, y, z),
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::Undo => {
                state.apply(Action::UndoLast);
                StepResult::Continue
            }
            Instruction::Tool(tool) => {
                state.apply(Action::SetTool(tool));
                StepResult::Continue
            }
            Instruction::BeginSketch { face } => {
                state.apply(Action::BeginSketch {
                    face,
                    viewport,
                });
                StepResult::Continue
            }
            Instruction::OpenSketch { sketch } => {
                let Some(sketch) = sketch_key(&state.doc, sketch) else {
                    self.last_action_error = Some(format!("Unknown sketch {sketch}"));
                    return StepResult::Continue;
                };
                state.apply(Action::OpenSketch {
                    sketch,
                    viewport,
                });
                StepResult::Continue
            }
            Instruction::ExitSketch => {
                state.apply(Action::ExitSketch);
                StepResult::Continue
            }
            Instruction::CreateRect {
                x,
                y,
                width,
                height,
                width_expr,
                height_expr,
            } => {
                let (width, height) = match (
                    eval_scalar_input(&state.doc, width, &width_expr, "rect width"),
                    eval_scalar_input(&state.doc, height, &height_expr, "rect height"),
                ) {
                    (Ok(w), Ok(h)) => (w, h),
                    (Err(e), _) | (_, Err(e)) => {
                        self.record_action_error(crate::actions::ActionResult::Err(e));
                        return StepResult::Continue;
                    }
                };
                let result = state.apply(Action::CreateRectangle {
                    x,
                    y,
                    width,
                    height,
                    width_expr,
                    height_expr,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateLine { x0, y0, x1, y1, bezier, dimension } => {
                let result =
                    state.apply(Action::CreateLineSegment { x0, y0, x1, y1, bezier, dimension });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateCircle { cx, cy, r, diameter_expr } => {
                let r = match &diameter_expr {
                    Some(_) => {
                        match eval_scalar_input(&state.doc, r, &diameter_expr, "circle diameter") {
                            Ok(d) => d * 0.5,
                            Err(e) => {
                                self.record_action_error(crate::actions::ActionResult::Err(e));
                                return StepResult::Continue;
                            }
                        }
                    }
                    None => r,
                };
                let result = state.apply(Action::CreateCircle { cx, cy, r, diameter_expr });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateSketchText {
                text,
                font,
                bold,
                italic,
                underline,
                size,
                x,
                y,
                rotation_deg,
                wrap,
                flip,
            } => {
                let Some(session) = state.sketch_session else {
                    self.last_action_error = Some("text needs an open sketch".to_string());
                    return StepResult::Continue;
                };
                let Some(font_family) = font.or_else(crate::default_text_font) else {
                    self.last_action_error =
                        Some("no usable system font found for text".to_string());
                    return StepResult::Continue;
                };
                let Some(size_mm) =
                    crate::value::eval_length_mm_in_doc(&size, &state.doc).filter(|s| *s > 0.0)
                else {
                    self.last_action_error =
                        Some(format!("text size {size:?} doesn't evaluate to a positive length"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::CreateSketchText {
                    sketch: session.sketch,
                    text,
                    font_family,
                    bold,
                    italic,
                    underline,
                    size: size_mm,
                    size_expr: size,
                    origin: (x, y),
                    rotation: rotation_deg.to_radians(),
                    wrap_width: wrap,
                    flip,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::Project { elements } => {
                if !elements.is_empty() {
                    state.scene_selection.clear();
                    for el in elements {
                        state.scene_selection.insert(el);
                    }
                }
                let result = state.apply(Action::ProjectSelection);
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::Extrude {
                sketch,
                faces,
                distance,
                body,
                target,
                expression,
                symmetric,
                taper,
                taper_mode,
                taper_expression,
            } => {
                let distance = match eval_scalar_input(
                    &state.doc,
                    distance,
                    &expression,
                    "extrude distance",
                ) {
                    Ok(d) => d,
                    Err(e) => {
                        self.record_action_error(crate::actions::ActionResult::Err(e));
                        return StepResult::Continue;
                    }
                };
                let Some(sketch) = sketch_key(&state.doc, sketch) else {
                    self.last_action_error = Some(format!("Unknown sketch {sketch}"));
                    return StepResult::Continue;
                };
                let taper = match &taper_expression {
                    Some(e) => match taper_mode {
                        crate::model::ExtrudeTaperMode::Distance => {
                            crate::value::eval_length_mm_in_doc(e, &state.doc).unwrap_or(taper)
                        }
                        crate::model::ExtrudeTaperMode::Angle => crate::value::eval_angle_rad_in_doc(
                            e,
                            &state.doc,
                        )
                        .map(|r| r.to_degrees())
                        .unwrap_or(taper),
                    },
                    None => taper,
                };
                let result = state.apply(Action::CreateExtrusion {
                    sketch,
                    faces,
                    distance,
                    body,
                    target,
                    expression,
                    symmetric,
                    taper,
                    taper_mode,
                    taper_expression,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ExtrudeBodyFace { face, distance, body, target } => {
                let result = state.apply(Action::CreateBodyFaceExtrusion {
                    face_id: face,
                    distance,
                    target,
                    body,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::UpdateExtrusion { extrusion, distance, target, expression } => {
                let distance = match &expression {
                    Some(e) => match crate::value::eval_length_mm_in_doc(e, &state.doc) {
                        Some(d) => Some(d),
                        None => {
                            self.record_action_error(crate::actions::ActionResult::Err(format!(
                                "extrusion distance expression {e:?} doesn't evaluate to a length"
                            )));
                            return StepResult::Continue;
                        }
                    },
                    None => distance,
                };
                let Some(extrusion) = extrusion_key(&state.doc, extrusion) else {
                    self.last_action_error = Some(format!("No extrusion {extrusion}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::UpdateExtrusion {
                    extrusion,
                    distance,
                    target,
                    expression,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::Loft { faces, body, bodies } => {
                // Rebuild sections from the faces (sketch inferred per face), then drive the
                // same action pair the interactive tool uses.
                state.creating_loft = None;
                for face in faces {
                    let Some(sketch) = crate::actions::extrude_face_sketch(&state.doc, &face)
                    else {
                        self.record_action_error(crate::actions::ActionResult::Err(
                            "loft section face does not exist".to_string(),
                        ));
                        continue;
                    };
                    let result = state.apply(Action::ToggleLoftSection {
                        section: crate::model::LoftSection { sketch, face },
                    });
                    self.record_action_error(result);
                }
                let bodies = body_keys(&state.doc, &bodies);
                if let Some(cl) = state.creating_loft.as_mut() {
                    cl.body_choice = body;
                    cl.cut_bodies = bodies;
                }
                let result = state.apply(Action::CommitLoft);
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingPage { drawing, width_mm, height_mm, margin_mm } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::SetDrawingPage {
                    drawing,
                    width_mm,
                    height_mm,
                    margin_mm,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateDrawing { name } => {
                let result = state.apply(Action::CreateDrawing { name });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ExportDrawingSvg { drawing, path } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::ExportDrawingSvg { drawing, path });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ExportDrawingPdf { drawing, path } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::ExportDrawingPdf { drawing, path });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddDrawingView {
                drawing,
                bodies,
                orientation,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let resolved = body_keys(&state.doc, &bodies);
                if resolved.len() != bodies.len() {
                    let missing = bodies
                        .iter()
                        .find(|&&b| body_key(&state.doc, b).is_none())
                        .copied()
                        .unwrap_or(0);
                    self.last_action_error = Some(format!("No body {missing}"));
                    return StepResult::Continue;
                }
                let result = state.apply(Action::AddDrawingView {
                    drawing,
                    bodies: resolved,
                    orientation,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddBodiesToDrawingView {
                drawing,
                view,
                bodies,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let resolved = body_keys(&state.doc, &bodies);
                if resolved.len() != bodies.len() {
                    let missing = bodies
                        .iter()
                        .find(|&&b| body_key(&state.doc, b).is_none())
                        .copied()
                        .unwrap_or(0);
                    self.last_action_error = Some(format!("No body {missing}"));
                    return StepResult::Continue;
                }
                let result = state.apply(Action::AddBodiesToDrawingView {
                    drawing,
                    view,
                    bodies: resolved,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddDrawingSketchView {
                drawing,
                sketch,
                orientation,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let Some(sketch) = sketch_key(&state.doc, sketch) else {
                    self.last_action_error = Some(format!("Unknown sketch {sketch}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::AddDrawingSketchView {
                    drawing,
                    sketch,
                    orientation,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddDrawingAnnotation { drawing, text, x, y, wrap } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::AddDrawingAnnotation {
                    drawing,
                    text,
                    pos_x: x,
                    pos_y: y,
                    wrap_frac: wrap,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddAlignedDrawingView { drawing, parent, dir, pos } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::AddAlignedDrawingView { drawing, parent, dir, pos });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::MoveDrawingView { drawing, view, x, y } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::MoveDrawingView {
                    drawing,
                    view,
                    pos_x: x,
                    pos_y: y,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingViewSize {
                drawing,
                view,
                size_x,
                size_y,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::SetDrawingViewSize {
                    drawing,
                    view,
                    size_x,
                    size_y,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ToggleDrawingDimension {
                drawing,
                view,
                a,
                b,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let q = |p: (f32, f32, f32)| {
                    crate::hierarchy::quantize_body_point(glam::Vec3::new(p.0, p.1, p.2))
                };
                let result = state.apply(Action::ToggleDrawingDimension {
                    drawing,
                    view,
                    a: q(a),
                    b: q(b),
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingDimensionOffset {
                drawing,
                view,
                a,
                b,
                offset,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let q = |p: (f32, f32, f32)| {
                    crate::hierarchy::quantize_body_point(glam::Vec3::new(p.0, p.1, p.2))
                };
                let result = state.apply(Action::SetDrawingDimensionOffset {
                    drawing,
                    view,
                    a: q(a),
                    b: q(b),
                    offset,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingCircleDimOffset {
                drawing,
                view,
                center,
                offset,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let center = crate::hierarchy::quantize_body_point(glam::Vec3::new(
                    center.0, center.1, center.2,
                ));
                let result = state.apply(Action::SetDrawingCircleDimOffset {
                    drawing,
                    view,
                    center,
                    offset,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ToggleDrawingCircleDimension { drawing, view, center } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::ToggleDrawingCircleDimension {
                    drawing,
                    view,
                    center: crate::hierarchy::quantize_body_point(glam::Vec3::new(
                        center.0, center.1, center.2,
                    )),
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingViewAlignLines { drawing, view, show } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let result =
                    state.apply(Action::SetDrawingViewAlignLines { drawing, view, show });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingViewLabel { drawing, view, hidden, pos, text } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let pos = match pos.as_deref() {
                    Some(name) => match crate::model::DrawingLabelPos::from_name(name) {
                        Some(p) => Some(p),
                        None => {
                            self.record_action_error(ActionResult::Err(format!(
                                "unknown label position '{name}'"
                            )));
                            return StepResult::Continue;
                        }
                    },
                    None => None,
                };
                let result = state.apply(Action::SetDrawingViewLabel {
                    drawing,
                    view,
                    hidden,
                    pos,
                    text: text.map(Some),
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ToggleDrawingAngle {
                drawing,
                view,
                edge1,
                edge2,
            } => {
                let Some(drawing) = drawing_key(&state.doc, drawing) else {
                    self.last_action_error = Some(format!("No drawing {drawing}"));
                    return StepResult::Continue;
                };
                let q = |p: (f32, f32, f32)| {
                    crate::hierarchy::quantize_body_point(glam::Vec3::new(p.0, p.1, p.2))
                };
                let key = |e: ((f32, f32, f32), (f32, f32, f32))| {
                    crate::model::normalized_edge_key(q(e.0), q(e.1))
                };
                let result = state.apply(Action::ToggleDrawingAngle {
                    drawing,
                    view,
                    edge1: key(edge1),
                    edge2: key(edge2),
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::Shape { shape } => {
                let result = state.apply(Action::CreateShape { shape });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditShape { index, shape } => {
                // The instruction's `index` is a script ordinal (#1055).
                let Some(key) = state.doc.primitives.keys().nth(index) else {
                    self.last_action_error = Some(format!("No shape {index}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::EditShape { index: key, shape });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::Revolve {
                faces,
                axis,
                angle_deg,
                angle_expression,
                angle_is_revolutions,
                pitch_mm,
                pitch_expression,
                symmetric,
                body,
                bodies,
            } => {
                let Some(sketch) = faces
                    .first()
                    .and_then(|f| crate::actions::extrude_face_sketch(&state.doc, f))
                else {
                    self.record_action_error(crate::actions::ActionResult::Err(
                        "revolve face does not exist".to_string(),
                    ));
                    return StepResult::Continue;
                };
                let bodies = body_keys(&state.doc, &bodies);
                let result = state.apply(Action::CreateRevolution {
                    sketch,
                    faces,
                    axis,
                    angle_deg,
                    angle_expression,
                    angle_is_revolutions,
                    pitch_mm,
                    pitch_expression,
                    gap_is_offset: true,
                    symmetric,
                    body,
                    bodies,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::Sweep { faces, path, body, bodies } => {
                let Some(sketch) = faces
                    .first()
                    .and_then(|f| crate::actions::extrude_face_sketch(&state.doc, f))
                else {
                    self.record_action_error(crate::actions::ActionResult::Err(
                        "sweep face does not exist".to_string(),
                    ));
                    return StepResult::Continue;
                };
                let bodies = body_keys(&state.doc, &bodies);
                let result = state.apply(Action::CreateSweep {
                    sketch,
                    faces,
                    path,
                    body,
                    bodies,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::VertexTreatment { points, kind, amount } => {
                for point in points {
                    let result = state.apply(Action::CommitVertexTreatment {
                        point,
                        kind,
                        amount: amount.clone(),
                    });
                    self.record_action_error(result);
                }
                StepResult::Continue
            }
            Instruction::EdgeTreatment { edges, kind, amount, expression } => {
                let Some(edges) = edges
                    .iter()
                    .map(|(host, edge)| {
                        let solid = match host {
                            TreatableSolidRef::Extrusion(o) => {
                                crate::model::TreatableSolid::Extrusion(extrusion_key(
                                    &state.doc, *o,
                                )?)
                            }
                            TreatableSolidRef::Primitive(o) => {
                                crate::model::TreatableSolid::Primitive(primitive_key(
                                    &state.doc, *o,
                                )?)
                            }
                        };
                        Some((solid, *edge))
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    self.last_action_error = Some("No such extrusion or primitive".to_string());
                    return StepResult::Continue;
                };
                let result = state.apply(Action::CommitEdgeTreatments { edges, kind, amount, expression });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateBooleanOp { kind, a, b, keep_b } => {
                let (a, b) = (body_keys(&state.doc, &a), body_keys(&state.doc, &b));
                let result = state.apply(Action::CreateBooleanOperation {
                    kind,
                    a,
                    b,
                    keep_b,
                    solid_count: None,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::BeginBooleanOp { kind, a, b, keep_b } => {
                let (a, b) = (body_keys(&state.doc, &a), body_keys(&state.doc, &b));
                state.apply(crate::actions::Action::SetTool(crate::actions::Tool::Combine));
                state.creating_boolean = Some(crate::actions::CreatingBoolean {
                    kind,
                    a,
                    b,
                    picking_b: kind != crate::model::BooleanOpKind::Combine,
                    keep_b,
                    editing: None,
                });
                StepResult::Continue
            }
            Instruction::EditBooleanOp { op, kind, a, b, keep_b } => {
                let (a, b) = (body_keys(&state.doc, &a), body_keys(&state.doc, &b));
                let Some(op) = boolean_op_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Boolean operation {op} not found"));
                    return StepResult::Continue;
                };
                let result =
                    state.apply(Action::EditBooleanOperation { op, kind, a, b, keep_b });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateMoveOp { targets, images, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c } => {
                let (targets, instance_targets) = split_move_bodies(&state.doc, &body_keys(&state.doc, &targets));
                let image_targets = image_keys(&state.doc, &images);
                let result = state.apply(Action::CreateMoveOperation {
                    translate_mode: move_translate_mode(&start_point_a, &end_point_a, &start_point_b),
                    start_point_a,
                    end_point_a,
                    start_point_b,
                    end_point_b,
                    start_point_c,
                    end_point_c,
                    targets,
                    plane_targets: Vec::new(),
                    image_targets,
                    instance_targets,
                    tx,
                    ty,
                    tz,
                    rx,
                    ry,
                    rz,
                    roll_angle,
                    face_flip,
                    face_spin,
                    face_offset,
                    keep_inputs: false,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditMoveOp { op, targets, images, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c } => {
                let targets = body_keys(&state.doc, &targets);
                let image_targets = image_keys(&state.doc, &images);
                let Some(op) = move_op_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Move operation {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::EditMoveOperation {
                    op,
                    translate_mode: move_translate_mode(&start_point_a, &end_point_a, &start_point_b),
                    start_point_a,
                    end_point_a,
                    start_point_b,
                    end_point_b,
                    start_point_c,
                    end_point_c,
                    targets,
                    plane_targets: Vec::new(),
                    image_targets,
                    instance_targets: Vec::new(),
                    tx,
                    ty,
                    tz,
                    rx,
                    ry,
                    rz,
                    roll_angle,
                    face_flip,
                    face_spin,
                    face_offset,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::BeginMoveOp { targets, images, tx, ty, tz, rx, ry, rz, roll_angle, face_flip, face_spin, face_offset, start_point_a, end_point_a, start_point_b, end_point_b, start_point_c, end_point_c } => {
                let (targets, instance_targets) = split_move_bodies(&state.doc, &body_keys(&state.doc, &targets));
                let image_targets = image_keys(&state.doc, &images);
                state.apply(crate::actions::Action::SetTool(crate::actions::Tool::Move));
                state.creating_move = Some(crate::actions::CreatingMove {
                    targets,
                    translate_mode: move_translate_mode(&start_point_a, &end_point_a, &start_point_b),
                    start_point_a,
                    end_point_a,
                    start_point_b,
                    end_point_b,
                    start_point_c,
                    end_point_c,
                    plane_targets: Vec::new(),
                    image_targets,
                    instance_targets,
                    tx,
                    ty,
                    tz,
                    rx,
                    ry,
                    rz,
                    roll_angle,
                    face_flip,
                    face_spin,
                    face_offset,
                    editing: None,
                    pending_face_a: None,
                    pending_face_b: None,
                    pending_gizmo_focus_axis: None,
                });
                StepResult::Continue
            }
            Instruction::CreateJointOp { members, base, kind, placement, frame, position, position2, position3, limits } => {
                let result = state.apply(Action::CreateJointOperation {
                    members,
                    base,
                    kind,
                    placement,
                    frame,
                    position,
                    position2,
                    position3,
                    limits,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditJointOp { op, members, base, kind, placement, frame, position, position2, position3, limits } => {
                let Some(op) = joint_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Joint {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::EditJointOperation {
                    op,
                    members,
                    base,
                    kind,
                    placement,
                    frame,
                    position,
                    position2,
                    position3,
                    limits,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::BeginJointOp { members, base, kind, placement, frame, position, position2, position3, limits } => {
                state.apply(crate::actions::Action::SetTool(crate::actions::Tool::Joint));
                let probe = crate::model::Joint {
                    members,
                    base,
                    kind,
                    placement,
                    frame,
                    position,
                    position2,
                    position3,
                    rest: String::new(),
                    rest2: String::new(),
                    rest3: String::new(),
                    limits,
                    name: None,
                };
                // A begin-joint probe is not editing anything yet; the key never resolves.
                let mut cj = crate::actions::CreatingJoint::from_joint(
                    &probe,
                    crate::arena::Key::from_bits(u64::MAX),
                );
                cj.editing = None;
                state.creating_joint = Some(cj);
                StepResult::Continue
            }
            Instruction::SetJointRest { op } => {
                let Some(joint) = joint_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Joint {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::SetJointRest { joint });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::RevertJoint { op } => {
                let Some(joint) = joint_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Joint {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::RevertJoint { joint });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::RevertAllJoints => {
                let result = state.apply(Action::RevertAllJoints);
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateMirrorOp { plane, targets, mode } => {
                let targets = body_keys(&state.doc, &targets);
                let result = state.apply(Action::CreateMirrorOperation { plane, targets, mode });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditMirrorOp { op, plane, targets, mode } => {
                let targets = body_keys(&state.doc, &targets);
                let Some(op) = mirror_op_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Mirror operation {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::EditMirrorOperation { op, plane, targets, mode });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateRepeatOp { targets, axis, around_axis, flip, mode, count, spacing, length, length_target } => {
                let targets = body_keys(&state.doc, &targets);
                let result = state.apply(Action::CreateRepeatOperation {
                    path_circle: None,
                    around_axis,
                    flip,
                    targets,
                    plane_targets: Vec::new(),
                    extrusion_targets: Vec::new(),
                    sketch_targets: Vec::new(),
                    axis,
                    mode,
                    count,
                    spacing,
                    length,
                    length_target,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditRepeatOp { op, targets, axis, around_axis, flip, mode, count, spacing, length, length_target } => {
                let targets = body_keys(&state.doc, &targets);
                let Some(op) = repeat_op_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Repeat operation {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::EditRepeatOperation {
                    op,
                    targets,
                    plane_targets: Vec::new(),
                    extrusion_targets: Vec::new(),
                    sketch_targets: Vec::new(),
                    axis,
                    path_circle: None,
                    around_axis,
                    flip,
                    mode,
                    count,
                    spacing,
                    length,
                    length_target,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateSliceOp { targets, cutters, extend_infinite } => {
                let targets = body_keys(&state.doc, &targets);
                let result = state.apply(Action::CreateSliceOperation {
                    targets,
                    cutters,
                    extend_infinite,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditSliceOp { op, targets, cutters, extend_infinite } => {
                let targets = body_keys(&state.doc, &targets);
                let Some(op) = slice_op_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Slice operation {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::EditSliceOperation {
                    op,
                    targets,
                    cutters,
                    extend_infinite,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateShellOp {
                targets,
                open_faces,
                thickness,
            } => {
                let targets = body_keys(&state.doc, &targets);
                let result = state.apply(Action::CreateShellOperation {
                    targets,
                    open_faces,
                    thickness,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditShellOp {
                op,
                targets,
                open_faces,
                thickness,
            } => {
                let targets = body_keys(&state.doc, &targets);
                let Some(op) = shell_op_key(&state.doc, op) else {
                    self.last_action_error = Some(format!("Shell operation {op} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::EditShellOperation {
                    op,
                    targets,
                    open_faces,
                    thickness,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetElementVisible { element, visible } => {
                match visible {
                    Some(v) => state.apply(Action::SetElementVisible { element, visible: v }),
                    None => state.apply(Action::ToggleElementVisibility(element)),
                };
                StepResult::Continue
            }
            Instruction::AddMaterial { name, color, bodies } => {
                let bodies = body_keys(&state.doc, &bodies);
                let result = state.apply(Action::AddMaterial { name, color, bodies });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetBodyMaterial { body, material } => {
                // A script names a material by its **ordinal** among the live ones (#1055):
                // keys are not something you can write by hand, and every example and doc
                // page says `material = 0`. Resolved here, at the boundary.
                let key = match material {
                    Some(ordinal) => match state.doc.materials.keys().nth(ordinal) {
                        Some(key) => Some(key),
                        None => {
                            self.last_action_error =
                                Some(format!("Unknown material {ordinal}"));
                            return StepResult::Continue;
                        }
                    },
                    None => None,
                };
                let Some(body) = body_key(&state.doc, body) else {
                    self.last_action_error = Some(format!("Unknown body {body}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::SetBodyMaterial { body, material: key });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetBodyShadow { body, shadow } => {
                let Some(body) = body_key(&state.doc, body) else {
                    self.last_action_error = Some(format!("Unknown body {body}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::SetBodyShadow { body, shadow });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SelectSceneElement { element, additive } => {
                state.apply(Action::ClickSceneElement { element, additive });
                StepResult::Continue
            }
            Instruction::ClearSceneSelection => {
                state.apply(Action::ClearSceneSelection);
                StepResult::Continue
            }
            Instruction::SetShapeConstruction { element, construction } => {
                let _ = state.apply(Action::SetShapeConstruction {
                    element,
                    construction,
                });
                StepResult::Continue
            }
            Instruction::ApplyConstruction { construction } => {
                let _ = state.apply(Action::ApplyConstruction { construction });
                StepResult::Continue
            }
            Instruction::ToggleConstruction => {
                let _ = state.apply(Action::ToggleConstruction);
                StepResult::Continue
            }
            Instruction::ApplySelectionVisibility { visible } => {
                let _ = state.apply(Action::ApplySelectionVisibility { visible });
                StepResult::Continue
            }
            Instruction::ToggleSelectionVisibility => {
                let _ = state.apply(Action::ToggleSelectionVisibility);
                StepResult::Continue
            }
            Instruction::SetElementName { element, name } => {
                state.apply(Action::CommitElementName { element, name });
                StepResult::Continue
            }
            Instruction::FocusElementName => {
                state.apply(Action::FocusElementName);
                StepResult::Continue
            }
            Instruction::SetDocumentUnits { length, angle } => {
                let _ = state.apply(Action::SetDocumentUnits { length, angle });
                StepResult::Continue
            }
            Instruction::CreateComponent { name, parent } => {
                let parent = match parent.map(|p| component_key(&state.doc, p)) {
                    Some(None) => {
                        self.last_action_error = Some("Component not found".to_string());
                        return StepResult::Continue;
                    }
                    other => other.flatten(),
                };
                let result = state.apply(Action::CreateComponent { name, parent });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::MoveToComponent { element, component } => {
                let component = match component.map(|c| component_key(&state.doc, c)) {
                    Some(None) => {
                        self.last_action_error = Some("Component not found".to_string());
                        return StepResult::Continue;
                    }
                    other => other.flatten(),
                };
                let result = state.apply(Action::MoveToComponent { element, component });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetComponentUnits { component, length, angle } => {
                let Some(component) = component_key(&state.doc, component) else {
                    self.last_action_error = Some(format!("Component {component} not found"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::SetComponentUnits { component, length, angle });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetSketchUnits { sketch, length, angle } => {
                let Some(sketch) = sketch_key(&state.doc, sketch) else {
                    self.last_action_error = Some(format!("Unknown sketch {sketch}"));
                    return StepResult::Continue;
                };
                let _ = state.apply(Action::SetSketchUnits { sketch, length, angle });
                StepResult::Continue
            }
            Instruction::SetAutoZoom { on } => {
                state.auto_zoom = on;
                StepResult::Continue
            }
            Instruction::SetSnapping { on } => {
                let _ = state.apply(Action::SetSnapping(on));
                StepResult::Continue
            }
            Instruction::FocusPicker { name } => {
                // #1485: arming a picker that isn't there fails the script.
                let result = state.apply(Action::FocusPicker(name));
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetMoveAngleSnap { degrees } => {
                let _ = state.apply(Action::SetMoveAngleSnap(degrees));
                StepResult::Continue
            }
            Instruction::SetJointAnimation { on } => {
                state.animate_joints = on;
                StepResult::Continue
            }
            Instruction::SetAnimateZoomToFit { on } => {
                state.animate_zoom_to_fit = on;
                StepResult::Continue
            }
            Instruction::SetUpdateChannel { channel } => {
                state.update_channel = channel;
                StepResult::Continue
            }
            Instruction::SetTouchMode { on } => {
                crate::touch::set_active(on);
                StepResult::Continue
            }
            Instruction::StartTutorial { index } => {
                let _ = state.apply(Action::StartTutorial { index });
                StepResult::Continue
            }
            Instruction::TutorialNext => {
                let _ = state.apply(Action::TutorialNext);
                StepResult::Continue
            }
            Instruction::TutorialAssist => {
                let _ = state.apply(Action::TutorialAssist);
                StepResult::Continue
            }
            Instruction::EndTutorial => {
                let _ = state.apply(Action::EndTutorial);
                StepResult::Continue
            }
            Instruction::SetDim { axis, value } => {
                let r = state.apply(Action::SetRectDimension { axis, value });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetDimLabelOffset { axis, offset } => {
                if let Some(session) = state.sketch_session {
                    if let Some(target) =
                        dim_label_target_in_sketch(&state.doc, session.sketch, axis)
                    {
                        let r = state.apply(Action::SetDimLabelOffset { target, offset });
                        self.record_action_error(r);
                    }
                }
                StepResult::Continue
            }
            Instruction::BeginEditCommittedDim { axis } => {
                if axis == DimLabelAxis::Length {
                    let mut only_image = None;
                    let mut extras = false;
                    for element in state.scene_selection.iter() {
                        match (element, only_image) {
                            (SceneElement::Image(i), None) => only_image = Some(i),
                            _ => extras = true,
                        }
                    }
                    if extras {
                        only_image = None;
                    }
                    if let Some(image) = only_image.filter(|&i| state.doc.tracing_images.contains(i))
                    {
                        let r = state.apply(Action::BeginEditImageCalibration { image });
                        self.record_action_error(r);
                        return StepResult::Continue;
                    }
                }
                let Some(session) = state.sketch_session else {
                    self.last_action_error = Some("Not in sketch mode".to_string());
                    return StepResult::Continue;
                };
                let Some(target) = dim_label_target_in_sketch(&state.doc, session.sketch, axis)
                else {
                    self.last_action_error = Some(format!(
                        "No committed {} dimension to edit",
                        dim_label_axis_lua_name(axis)
                    ));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::BeginEditCommittedDim { target });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::CommitCommittedDim => {
                let r = state.apply(Action::CommitCommittedDim);
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::AddAngleConstraint {
                line_a,
                line_b,
                rotation_sign,
                expression,
            } => {
                if let Some(session) = state.sketch_session {
                    // `name = value` defines the parameter on the spot and dimensions with
                    // it, exactly as typing it into the GUI's value field does (#797).
                    let mut expression = expression;
                    if let Err(e) = crate::actions::commit_inline_parameter_defs(
                        &mut state.doc,
                        [&mut expression],
                    ) {
                        self.record_action_error(crate::actions::ActionResult::Err(e));
                        return StepResult::Continue;
                    }
                    let (Some(line_a), Some(line_b)) =
                        (line_key(&state.doc, line_a), line_key(&state.doc, line_b))
                    else {
                        self.last_action_error = Some("No such line".to_string());
                        return StepResult::Continue;
                    };
                    let result = crate::constraints::apply_dimension_expression(
                        &mut state.doc,
                        session.sketch,
                        crate::model::DimensionTarget::Angle {
                            line_a: crate::model::ConstraintLine::Line(line_a),
                            line_b: crate::model::ConstraintLine::Line(line_b),
                            rotation_sign,
                        },
                        &expression,
                    );
                    if let Err(e) = result {
                        self.record_action_error(crate::actions::ActionResult::Err(e));
                    } else {
                        let _ = crate::constraints::solve_document_constraints(&mut state.doc);
                    }
                }
                StepResult::Continue
            }
            Instruction::AddDistanceConstraint { target, expression } => {
                if let Some(session) = state.sketch_session {
                    // `name = value` defines the parameter on the spot and dimensions with
                    // it, exactly as typing it into the GUI's value field does (#797).
                    let mut expression = expression;
                    if let Err(e) = crate::actions::commit_inline_parameter_defs(
                        &mut state.doc,
                        [&mut expression],
                    ) {
                        state.status = e.clone();
                        self.record_action_error(crate::actions::ActionResult::Err(e));
                        return StepResult::Continue;
                    }
                    let existed = crate::constraints::find_distance_constraint(
                        &state.doc,
                        target.clone(),
                    )
                    .is_some();
                    match apply_dimension_expression(
                        &mut state.doc,
                        session.sketch,
                        crate::model::DimensionTarget::Distance(target),
                        &expression,
                    ) {
                        Ok(_) => {
                            state.status = if existed {
                                format!("Updated dimension ({expression})")
                            } else {
                                format!("Added dimension ({expression})")
                            };
                        }
                        Err(e) => {
                            state.status = e.clone();
                            self.record_action_error(crate::actions::ActionResult::Err(e));
                        }
                    }
                }
                StepResult::Continue
            }
            Instruction::AddGeometricConstraint(kind) => {
                let _ = state.apply(Action::AddGeometricConstraint(kind));
                StepResult::Continue
            }
            Instruction::ApplyConstraintShortcut(key) => {
                let _ = state.apply(Action::ApplyConstraintShortcut(key));
                StepResult::Continue
            }
            Instruction::DragVertex { point, u, v } => {
                let result = state.apply(Action::DragVertex { point, u, v });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::DragLineSegment {
                target,
                anchor_u,
                anchor_v,
                u,
                v,
            } => {
                let result = state.apply(Action::BeginLineDrag {
                    target,
                    anchor_u,
                    anchor_v,
                });
                self.record_action_error(result);
                let _ = state.apply(Action::DragLine { u, v });
                let _ = state.apply(Action::EndLineDrag);
                StepResult::Continue
            }
            Instruction::SetLineLength { value } => {
                let r = state.apply(Action::SetLineLength { value });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetCircleDiameter { value } => {
                let r = state.apply(Action::SetCircleDiameter { value });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::BeginEditConstructionPlane { index } => {
                let Some(index) = plane_key(&state.doc, index) else {
                    self.last_action_error = Some(format!("Unknown construction plane {index}"));
                    return StepResult::Continue;
                };
                state.apply(Action::BeginEditConstructionPlane { index });
                StepResult::Continue
            }
            Instruction::CommitConstructionPlane => {
                state.apply(Action::CommitConstructionPlane);
                StepResult::Continue
            }
            Instruction::SetPlaneOffset { value } => {
                let _ = state.apply(Action::SetPlaneOffset { value });
                StepResult::Continue
            }
            Instruction::SetPlaneAngle { value } => {
                let _ = state.apply(Action::SetPlaneAngle { value });
                StepResult::Continue
            }
            Instruction::CreatePlane { offset, from } => {
                let Some(from) = plane_key(&state.doc, from) else {
                    self.last_action_error = Some(format!("Unknown construction plane {from}"));
                    return StepResult::Continue;
                };
                let result = state.apply(Action::AddConstructionPlane { from, offset_mm: offset });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateFacePlane { offset, origin, normal } => {
                // The same Begin → typed offset → Commit path the Plane tool takes when a
                // face is clicked (#465).
                let result = state.apply(Action::BeginConstructionPlane {
                    reference: crate::construction::PlaneReference::Face {
                        origin,
                        normal: normal.normalize_or_zero(),
                        label: "Face".to_string(),
                    },
                    parent: crate::model::ConstructionPlaneParent::Root,
                });
                self.record_action_error(result);
                let _ = state.apply(Action::SetPlaneOffset {
                    value: format!("{offset}mm"),
                });
                let result = state.apply(Action::CommitConstructionPlane);
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::FocusDim(axis) => {
                let _ = state.apply(Action::FocusRectDimension { axis });
                StepResult::Continue
            }
            Instruction::FocusLineLength => {
                let _ = state.apply(Action::FocusLineLength);
                StepResult::Continue
            }
            Instruction::FocusCircleDiameter => {
                let _ = state.apply(Action::FocusCircleDiameter);
                StepResult::Continue
            }
            Instruction::FocusPlaneDim(dim) => {
                let _ = state.apply(Action::FocusPlaneDim { dim });
                StepResult::Continue
            }
            Instruction::FpsMode { on } => {
                let active = state.fps.is_some();
                if on != Some(active) {
                    let result = state.apply(Action::ToggleFpsMode);
                    self.record_action_error(result);
                }
                StepResult::Continue
            }
            Instruction::FpsLook { dx, dy } => {
                match state.fps.as_mut() {
                    Some(player) => {
                        player.look_by_angles(-dx.to_radians(), dy.to_radians());
                        player.clone().apply_to_camera(&mut state.cam);
                    }
                    None => self.record_action_error(crate::actions::ActionResult::Err(
                        "Not in FPS mode".to_string(),
                    )),
                }
                StepResult::Continue
            }
            Instruction::FpsMove { forward, strafe } => {
                match state.fps.as_mut() {
                    Some(player) => {
                        let step = player.ground_forward() * forward
                            + player.ground_right() * strafe;
                        player.eye += step;
                        player.clone().apply_to_camera(&mut state.cam);
                    }
                    None => self.record_action_error(crate::actions::ActionResult::Err(
                        "Not in FPS mode".to_string(),
                    )),
                }
                StepResult::Continue
            }
            Instruction::FpsJump => {
                match state.fps.as_mut() {
                    Some(player) => {
                        player.tick(
                            0.0,
                            crate::fps::FpsInput {
                                jump_pressed: true,
                                ..Default::default()
                            },
                        );
                        player.clone().apply_to_camera(&mut state.cam);
                    }
                    None => self.record_action_error(crate::actions::ActionResult::Err(
                        "Not in FPS mode".to_string(),
                    )),
                }
                StepResult::Continue
            }
            Instruction::FpsFly { on } => {
                match state.fps.as_mut() {
                    Some(player) => {
                        let want = on.unwrap_or(!player.flying);
                        if want != player.flying {
                            player.flying = want;
                            player.vertical_speed = 0.0;
                        }
                    }
                    None => self.record_action_error(crate::actions::ActionResult::Err(
                        "Not in FPS mode".to_string(),
                    )),
                }
                StepResult::Continue
            }
            Instruction::FpsAdvance { seconds } => {
                match state.fps.as_mut() {
                    Some(player) => {
                        let mut remaining = seconds.clamp(0.0, 60.0);
                        while remaining > 0.0 {
                            let dt = remaining.min(0.01);
                            player.tick(dt, crate::fps::FpsInput::default());
                            remaining -= dt;
                        }
                        player.clone().apply_to_camera(&mut state.cam);
                    }
                    None => self.record_action_error(crate::actions::ActionResult::Err(
                        "Not in FPS mode".to_string(),
                    )),
                }
                StepResult::Continue
            }
            Instruction::FpsScale { scale } => {
                match state.fps.as_mut() {
                    Some(player) => {
                        player.set_scale(scale);
                        player.clone().apply_to_camera(&mut state.cam);
                    }
                    None => self.record_action_error(crate::actions::ActionResult::Err(
                        "Not in FPS mode".to_string(),
                    )),
                }
                StepResult::Continue
            }
            Instruction::Orbit { dx, dy } => {
                state.apply(Action::OrbitCamera { delta: (dx, dy) });
                StepResult::Continue
            }
            Instruction::Pan { dx, dy } => {
                let h = viewport.map(|r| r.height()).unwrap_or(640.0);
                state.apply(Action::PanCamera {
                    delta: (dx, dy),
                    viewport_height: h,
                });
                StepResult::Continue
            }
            Instruction::Zoom { scroll } => {
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                state.apply(Action::ZoomCamera {
                    scroll,
                    focal: vp.center(),
                    viewport: vp,
                });
                StepResult::Continue
            }
            Instruction::View(view) => {
                state.apply(Action::SetStandardView(view));
                self.waiting_view_transition = true;
                StepResult::Wait
            }
            Instruction::ViewEdge(edge) => {
                state.apply(Action::SetViewEdge(edge));
                self.waiting_view_transition = true;
                StepResult::Wait
            }
            Instruction::ViewCorner(corner) => {
                state.apply(Action::SetViewCorner(corner));
                self.waiting_view_transition = true;
                StepResult::Wait
            }
            Instruction::ViewHome => {
                state.apply(Action::ViewHome);
                self.waiting_view_transition = true;
                StepResult::Wait
            }
            Instruction::SetHomeView => {
                state.apply(Action::SetHomeView);
                StepResult::Continue
            }
            Instruction::ProjectionMode(mode) => {
                state.apply(Action::SetProjectionMode(mode));
                StepResult::Continue
            }
            Instruction::ToggleProjectionMode => {
                state.apply(Action::ToggleProjectionMode);
                StepResult::Continue
            }
            Instruction::ShadingMode(mode) => {
                state.apply(Action::SetShadingMode(mode));
                StepResult::Continue
            }
            Instruction::GroundDisplay(mode) => {
                state.apply(Action::SetGroundDisplay(mode));
                StepResult::Continue
            }
            Instruction::SetCamera {
                yaw,
                pitch,
                distance,
                target,
            } => {
                state.cam.set_pose_instant(
                    yaw,
                    pitch,
                    distance,
                    target.map(|(x, y, z)| Vec3::new(x, y, z)),
                );
                StepResult::Continue
            }
            Instruction::ZoomFit => {
                if let Some(vp) = viewport {
                    state.viewport_aspect = (vp.width() / vp.height().max(1.0)).max(0.01);
                    state.viewport_height = vp.height();
                }
                // Same path as the Z key / menu: includes in-progress operation previews (#1114).
                // When animation is on (#1276), wait out the glide like ViewHome does.
                state.apply(Action::ZoomToFit);
                if state.cam.is_transitioning() {
                    self.waiting_view_transition = true;
                    StepResult::Wait
                } else {
                    StepResult::Continue
                }
            }
            Instruction::SetElementsView { mode } => {
                state.apply(Action::SetElementsViewMode { mode });
                StepResult::Continue
            }
            Instruction::SetPane { pane, visible } => {
                match visible {
                    Some(v) => state.apply(Action::SetPaneVisible { pane, visible: v }),
                    None => state.apply(Action::TogglePane(pane)),
                };
                StepResult::Continue
            }
            // Every one of these records its action error: a script that names a backend
            // that does not exist must fail loudly, not select nothing quietly (#1599).
            Instruction::AddAiBackend { backend } => {
                let result = state.apply(Action::AddAiBackend { backend });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::UpdateAiBackend { id, backend } => {
                let result = state.apply(Action::UpdateAiBackend { id, backend });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::RemoveAiBackend { id } => {
                let result = state.apply(Action::RemoveAiBackend { id });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SelectAiBackend { id } => {
                let result = state.apply(Action::SelectAiBackend { id });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SendAiMessage { text } => {
                let result = state.apply(Action::SendAiMessage { text });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CancelAiMessage => {
                let result = state.apply(Action::CancelAiMessage);
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ClearAiConversation => {
                state.apply(Action::ClearAiConversation);
                StepResult::Continue
            }
            Instruction::SetAiContextScope { scope } => {
                state.apply(Action::SetAiContextScope { scope });
                StepResult::Continue
            }
            Instruction::ResetAiBackendSpend { id } => {
                let result = state.apply(Action::ResetAiBackendSpend { id });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::RunAiBlock { index } => {
                let result = state.apply(Action::RunAiBlock { index });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SeedAiReply { question, reply } => {
                let result = state.apply(Action::SeedAiReply { question, reply });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddParameter { name, expression } => {
                state.apply(Action::AddParameter { name, expression });
                StepResult::Continue
            }
            Instruction::CreateDerivedParameter { source, name } => {
                let result = state.apply(Action::CreateDerivedParameter { source, name });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateParameterFromLineLength { line_index, name } => {
                let Some(line_index) = line_key(&state.doc, line_index) else {
                    self.last_action_error = Some(format!("No line {line_index}"));
                    return StepResult::Continue;
                };
                state.apply(Action::CreateParameterFromLineLength { line_index, name });
                StepResult::Continue
            }
            // A script names a parameter by its ordinal among the live ones (#1055).
            Instruction::SetParameterName { index, name } => {
                if let Some(key) = parameter_key(&state.doc, index) {
                    state.apply(Action::CommitParameterName { index: key, name });
                } else {
                    self.last_action_error = Some(format!("Parameter {index} not found"));
                }
                StepResult::Continue
            }
            Instruction::SetParameterExpression { index, expression } => {
                if let Some(key) = parameter_key(&state.doc, index) {
                    state.apply(Action::CommitParameterExpression { index: key, expression });
                } else {
                    self.last_action_error = Some(format!("Parameter {index} not found"));
                }
                StepResult::Continue
            }
            Instruction::SetParameterPrimary { index, primary } => {
                let Some(key) = parameter_key(&state.doc, index) else {
                    self.last_action_error = Some(format!("Parameter {index} not found"));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::SetParameterPrimary { index: key, primary });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetParameterBound { index, which, expression } => {
                let Some(key) = parameter_key(&state.doc, index) else {
                    self.last_action_error = Some(format!("Parameter {index} not found"));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::SetParameterBound {
                    index: key,
                    which,
                    expression,
                });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetUnitParameterOverride { instance, name, expression } => {
                let Some(instance) = unit_instance_key(&state.doc, instance) else {
                    self.last_action_error = Some(format!("Unit instance {instance} not found"));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::SetUnitParameterOverride { instance, name, expression });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SyncUnit { unit } => {
                let Some(unit) = unit_key(&state.doc, unit) else {
                    self.last_action_error = Some(format!("Unit {unit} not found"));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::SyncUnit { unit });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetUnitLink { unit, link } => {
                let Some(unit) = unit_key(&state.doc, unit) else {
                    self.last_action_error = Some(format!("Unit {unit} not found"));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::SetUnitLink { unit, link });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::AddUnitInstance { unit, name } => {
                let Some(unit) = unit_key(&state.doc, unit) else {
                    self.last_action_error = Some(format!("Unit {unit} not found"));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::AddUnitInstance { unit, name });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::CloneUnitInstance { instance } => {
                let Some(instance) = unit_instance_key(&state.doc, instance) else {
                    self.last_action_error =
                        Some(format!("Unit instance {instance} not found"));
                    return StepResult::Continue;
                };
                let r = state.apply(Action::CloneUnitInstance { instance });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetMcMasterWindow { open, part } => {
                state.apply(Action::SetMcMasterWindow { open, part });
                StepResult::Continue
            }
            Instruction::SetReportIssueWindow { open } => {
                state.apply(Action::SetReportIssueWindow { open });
                StepResult::Continue
            }
            Instruction::SetSettingsWindow { open } => {
                state.apply(Action::SetSettingsWindow { open });
                StepResult::Continue
            }
            Instruction::SetChangelogWindow { open } => {
                state.apply(Action::SetChangelogWindow { open });
                StepResult::Continue
            }
            Instruction::SetTutorialPane { open } => {
                state.apply(Action::SetTutorialPane { open });
                StepResult::Continue
            }
            Instruction::CompleteAllTutorials => {
                state.apply(Action::CompleteAllTutorials);
                StepResult::Continue
            }
            Instruction::UnstartAllTutorials => {
                state.apply(Action::UnstartAllTutorials);
                StepResult::Continue
            }
            Instruction::DeleteParameter { index } => {
                if let Some(key) = parameter_key(&state.doc, index) {
                    state.apply(Action::DeleteParameter { index: key });
                } else {
                    self.last_action_error = Some(format!("Parameter {index} not found"));
                }
                StepResult::Continue
            }
            Instruction::DeleteSelection => {
                state.apply(Action::DeleteSelection);
                StepResult::Continue
            }
            Instruction::SetCommandPalette { open } => {
                match open {
                    Some(true) => state.apply(Action::SetCommandPaletteOpen { open: true }),
                    Some(false) => state.apply(Action::SetCommandPaletteOpen { open: false }),
                    None => state.apply(Action::ToggleCommandPalette),
                };
                StepResult::Continue
            }
            Instruction::RunPaletteCommand { query, argument } => {
                let commands = commands_for_state(state);
                if let Some(cmd) = best_match(&query, &commands) {
                    match cmd.outcome(argument.as_deref().unwrap_or_default()) {
                        PaletteOutcome::Action(action) => {
                            state.apply(action);
                        }
                        PaletteOutcome::ImportImageOnThisPlane { path } => {
                            let Some(plane) =
                                crate::command_palette::selected_construction_plane(state)
                            else {
                                state.status = "No construction plane selected".to_string();
                                return StepResult::Continue;
                            };
                            match path {
                                Some(path) => {
                                    let r = state.apply(Action::ImportImage {
                                        path,
                                        plane: Some(plane),
                                    });
                                    self.record_action_error(r);
                                }
                                None => {
                                    state.status =
                                        "Palette file commands require the GUI".to_string();
                                }
                            }
                        }
                        PaletteOutcome::OpenFile | PaletteOutcome::SaveFile
                        | PaletteOutcome::SaveFileAs
                        | PaletteOutcome::ExportLua
                        | PaletteOutcome::ImportLua
                        | PaletteOutcome::DocumentJson
                        | PaletteOutcome::OpenExploder
                        | PaletteOutcome::ShowShortcuts
                        | PaletteOutcome::ShowSettings
                        | PaletteOutcome::ImportUnit => {
                            state.status =
                                "Palette file commands require the GUI".to_string();
                        }
                    }
                } else {
                    state.status = format!("No palette command matches '{query}'");
                }
                StepResult::Continue
            }

            Instruction::Move { x, y } => {
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                synthetic.move_to(vp, x, y);
                StepResult::Continue
            }
            Instruction::Click { x, y, mods } => {
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                synthetic.click_with(vp, x, y, mods);
                StepResult::Continue
            }
            Instruction::MoveGround { x, y } => {
                if viewport.is_none() || state.cam.is_transitioning() {
                    return StepResult::Wait;
                }
                Self::ground_pointer(synthetic, state, viewport, x, y, None);
                StepResult::Continue
            }
            Instruction::ClickGround { x, y, mods } => {
                if viewport.is_none() || state.cam.is_transitioning() {
                    return StepResult::Wait;
                }
                Self::ground_pointer(synthetic, state, viewport, x, y, Some(mods));
                StepResult::Continue
            }
            Instruction::DragGround { x0, y0, x1, y1 } => {
                if state.cam.is_transitioning() {
                    return StepResult::Wait;
                }
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                let mat = state.cam.view_proj(vp);
                let (Some(a), Some(b)) = (
                    state.cam.project(Vec3::new(x0, y0, 0.0), vp, &mat),
                    state.cam.project(Vec3::new(x1, y1, 0.0), vp, &mat),
                ) else {
                    return StepResult::Continue;
                };
                synthetic.drag(vp, a.x - vp.min.x, a.y - vp.min.y, b.x - vp.min.x, b.y - vp.min.y);
                StepResult::Continue
            }
            Instruction::Drag { x0, y0, x1, y1 } => {
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                synthetic.drag(vp, x0, y0, x1, y1);
                StepResult::Continue
            }
            Instruction::RightDrag { dx, dy } => {
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                synthetic.right_drag(vp, dx, dy, false);
                StepResult::Continue
            }
            Instruction::RightDragShift { dx, dy } => {
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                synthetic.right_drag(vp, dx, dy, true);
                StepResult::Continue
            }
            Instruction::Key { key, mods } => {
                synthetic.key_with(key, mods);
                StepResult::Continue
            }
            Instruction::KeyDown(key) => {
                synthetic.key_down(key);
                StepResult::Continue
            }
            Instruction::KeyUp(key) => {
                synthetic.key_up(key);
                StepResult::Continue
            }
            Instruction::Type(text) => {
                synthetic.type_text(&text);
                StepResult::Continue
            }

            Instruction::WaitMs(ms) => {
                self.wait_until = Some(Instant::now() + Duration::from_millis(ms));
                StepResult::Wait
            }
            Instruction::WaitFrames(n) => {
                if n == 0 {
                    StepResult::Continue
                } else {
                    self.wait_frames_remaining = n;
                    StepResult::Wait
                }
            }
            Instruction::Screenshot { path, region } => {
                let rect = match region {
                    ScreenshotRegion::Window => None,
                    ScreenshotRegion::Viewport => Some(viewport),
                    ScreenshotRegion::Pane(pane) => match pane_rect(ctx, pane) {
                        Some(rect) => Some(Some(rect)),
                        None => {
                            self.record_action_error(crate::actions::ActionResult::Err(format!(
                                "the {} pane is not on screen to capture",
                                pane.label()
                            )));
                            return StepResult::Continue;
                        }
                    },
                    ScreenshotRegion::Settings => {
                        match ctx.data(|d| d.get_temp::<egui::Rect>(pane_rect_id("settings"))) {
                            Some(rect) => Some(Some(rect)),
                            None => {
                                self.record_action_error(crate::actions::ActionResult::Err(
                                    "the Settings window is not on screen to capture".to_string(),
                                ));
                                return StepResult::Continue;
                            }
                        }
                    }
                };
                let crop = rect.flatten().map(|rect| ScreenshotCrop {
                    rect,
                    pixels_per_point: ctx.pixels_per_point(),
                });
                self.screenshot_pending = Some(ScreenshotRequest {
                    path,
                    crop,
                    frames_waited: 0,
                    attempts: 1,
                });
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                StepResult::Wait
            }
            Instruction::HelpMode { on } => {
                state.apply(crate::actions::Action::SetHelpMode(on));
                StepResult::Continue
            }
            Instruction::ToolHints { on } => {
                state.apply(crate::actions::Action::SetToolHints(on));
                StepResult::Continue
            }
            Instruction::SetToolMode(mode) => {
                if let Err(err) = crate::actions::set_tool_mode(state, &mode) {
                    self.record_action_error(crate::actions::ActionResult::Err(err));
                }
                StepResult::Continue
            }
            Instruction::SetGizmo { name, value, relative } => {
                let target = if relative {
                    match crate::actions::gizmo_value(state, &name) {
                        Some(current) => current + value,
                        None => {
                            self.record_action_error(crate::actions::ActionResult::Err(format!(
                                "no gizmo named '{name}' is active"
                            )));
                            return StepResult::Continue;
                        }
                    }
                } else {
                    value
                };
                if !crate::actions::set_gizmo(state, &name, target) {
                    self.record_action_error(crate::actions::ActionResult::Err(format!(
                        "no gizmo named '{name}' is active"
                    )));
                }
                StepResult::Continue
            }
            Instruction::Quit => {
                self.should_quit = true;
                StepResult::Done
            }
        }
    }
}

/// Save an egui [`egui::ColorImage`] to a PNG file.
pub fn save_screenshot(path: &str, image: &egui::ColorImage) -> Result<(), String> {
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| [c.r(), c.g(), c.b(), c.a()])
        .collect();
    save_rgba(path, image.width() as u32, image.height() as u32, &rgba)
}

/// Save the portion of `image` covered by `rect` (logical points), scaled by `pixels_per_point`.
fn save_screenshot_cropped(
    path: &str,
    image: &egui::ColorImage,
    rect: egui::Rect,
    pixels_per_point: f32,
) -> Result<(), String> {
    let (x0, y0, x1, y1) = crop_bounds(image.width(), image.height(), rect, pixels_per_point);
    let (w, h) = (x1 - x0, y1 - y0);
    if w == 0 || h == 0 {
        // Degenerate crop (e.g. viewport rect unknown): fall back to the whole frame.
        return save_screenshot(path, image);
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in y0..y1 {
        let row = y * image.width();
        for x in x0..x1 {
            let c = image.pixels[row + x];
            rgba.extend_from_slice(&[c.r(), c.g(), c.b(), c.a()]);
        }
    }
    save_rgba(path, w as u32, h as u32, &rgba)
}

/// Physical-pixel `(x0, y0, x1, y1)` crop bounds, clamped to the image.
fn crop_bounds(
    img_w: usize,
    img_h: usize,
    rect: egui::Rect,
    pixels_per_point: f32,
) -> (usize, usize, usize, usize) {
    let to_px = |v: f32, max: usize| ((v * pixels_per_point).round() as i32).clamp(0, max as i32) as usize;
    let x0 = to_px(rect.min.x, img_w);
    let y0 = to_px(rect.min.y, img_h);
    let x1 = to_px(rect.max.x, img_w).max(x0);
    let y1 = to_px(rect.max.y, img_h).max(y0);
    (x0, y0, x1, y1)
}

fn save_rgba(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)
        .map_err(|e| format!("failed to save screenshot to {path}: {e}"))
}

/// CLI launch options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptOptions {
    pub script_path: Option<String>,
    pub document_path: Option<String>,
    pub exit_on_complete: bool,
    pub show_commands: bool,
    /// Force-exit (non-zero) if the app hasn't closed on its own within this many
    /// seconds — a watchdog for unattended/CI launches. See #61.
    pub timeout_secs: Option<u64>,
    /// Run an interactive Lua REPL on stdin against the live app (`--repl`).
    pub repl: bool,
    /// Start a tutorial by registry name on launch (`--tutorial cube`, #765) — the same
    /// thing the web build's `?tutorial=` parameter does.
    pub tutorial: Option<String>,
    /// Discard `geometry_cache` after opening a document (`--rebuild`, SPEC §4.4).
    pub rebuild: bool,
}

/// Parsed command-line outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliOutcome {
    Help,
    /// Install the `bearcad` CLI symlink onto PATH (`bearcad install-cli`). See #49.
    InstallCli,
    /// Remove the `bearcad` CLI symlink (`bearcad uninstall-cli`).
    UninstallCli,
    /// Show the McMaster-Carr catalog in a window of its own (`bearcad mcmaster [part]`,
    /// #1022). The app runs itself under this to host the web view in a second process.
    McMaster { part: Option<String> },
    /// Print every tool operation's input/output/shadow element types (`bearcad opsigs`).
    /// `html` is true when `--html` was passed (HTML document instead of markdown).
    OpSigs { html: bool },
    /// Print an exhaustive `[ ]` test checklist (`bearcad testplan`).
    Testplan,
    Run(ScriptOptions),
}

/// Print usage information to stdout.
pub fn print_usage() {
    print!("{}", usage_text());
}

fn usage_text() -> &'static str {
    "\
BearCAD — parametric CAD prototype

Usage:
  bearcad [options] [script.lua]
  bearcad <command>

Commands:
  install-cli           Symlink this executable onto PATH as `bearcad`
                        (default /usr/local/bin; use sudo if it is not writable)
                        and register `.bearcad` so double-click opens BearCAD
  uninstall-cli         Remove the `bearcad` PATH symlink and unregister `.bearcad`
  mcmaster [part]       Browse the McMaster-Carr catalog in a window, printing each CAD
                        file it downloads. The app runs this itself when you import a part
  opsigs [--html]       Print every tool operation's inputs, outputs, and shadows
                        (markdown; pass --html for HTML). Also: `cargo opsigs`
  testplan              Print a [ ] checklist of tools, variants, tutorials, and
                        features for exhaustive manual or AI testing. Tutorials
                        come from TUTORIALS; extra items live in src/testplan.rs
                        (`CUSTOM_ITEMS`)

Options:
  --script <path>       Run a Lua script
  --repl                Interactive Lua REPL on stdin against the live app
                        (globals persist between entries; Ctrl-D ends it)
  --exit, --exit-on-complete
                        Exit after startup, or after the script finishes
  --show-commands       Print each user action as a script line on stdout
  --tutorial <name>     Start a guided tutorial on launch (e.g. `cube`)
  --timeout <seconds>   Force-exit with an error if the app hasn't closed on
                        its own within this many seconds
  --rebuild             Discard cached tessellation and rebuild geometry
  -h, --help            Show this help and exit

Examples:
  bearcad
  bearcad --exit
  bearcad drawing.bearcad --exit
  bearcad --script demo.lua
  bearcad demo.lua --exit
  bearcad --repl
  bearcad --tutorial cube
  bearcad --exit --timeout 30
  bearcad install-cli
  bearcad testplan

Diagnostics:
  Every run writes a log; the path is printed on startup. Warnings and notable
  events also go to stderr, so a terminal narrates the session.
  BEARCAD_LOG=1          Put the full trace on stderr too, not just in the log
  BEARCAD_LOG_FILE=path  Write the log here instead of the default
"
}

/// Parse command-line arguments.
pub fn parse_cli(args: impl IntoIterator<Item = impl AsRef<str>>) -> CliOutcome {
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.as_ref().to_string())
        .collect();
    if args
        .iter()
        .any(|arg| arg == "--help" || arg == "-h")
    {
        return CliOutcome::Help;
    }
    // Subcommands (args[0] is the program name).
    match args.get(1).map(String::as_str) {
        Some("install-cli") => return CliOutcome::InstallCli,
        Some("uninstall-cli") => return CliOutcome::UninstallCli,
        Some(crate::mcmaster::SUBCOMMAND) => {
            return CliOutcome::McMaster { part: args.get(2).cloned() }
        }
        Some("opsigs") => {
            let html = args.iter().skip(2).any(|a| a == "--html");
            return CliOutcome::OpSigs { html };
        }
        Some("testplan") => return CliOutcome::Testplan,
        _ => {}
    }
    CliOutcome::Run(parse_args_from_vec(&args))
}

/// Parse command-line arguments for script mode (without handling `--help`).
#[allow(dead_code)] // public API; exercised by unit tests
pub fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> ScriptOptions {
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.as_ref().to_string())
        .collect();
    parse_args_from_vec(&args)
}

fn parse_args_from_vec(args: &[String]) -> ScriptOptions {
    let mut opts = ScriptOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--script" => {
                i += 1;
                if i < args.len() {
                    opts.script_path = Some(args[i].clone());
                }
            }
            "--exit" | "--exit-on-complete" => {
                opts.exit_on_complete = true;
            }
            "--repl" => {
                opts.repl = true;
            }
            "--show-commands" => {
                opts.show_commands = true;
            }
            "--tutorial" => {
                i += 1;
                if i < args.len() {
                    opts.tutorial = Some(args[i].clone());
                }
            }
            "--timeout" => {
                i += 1;
                if i < args.len() {
                    opts.timeout_secs = args[i].parse::<u64>().ok();
                }
            }
            "--rebuild" | "--force-rebuild" => {
                opts.rebuild = true;
            }
            arg if !arg.starts_with('-') => {
                if opts.script_path.is_none()
                    && (arg.ends_with(".lua")
                        || Path::new(arg).extension().is_some_and(|e| e == "lua"))
                {
                    opts.script_path = Some(arg.to_string());
                } else if opts.document_path.is_none() {
                    if let Some(path) = crate::file_association::path_from_os_open_spec(arg) {
                        if crate::file_association::is_document_path(&path) {
                            opts.document_path = Some(path);
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    opts
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::sketch_key_for_slot as skey;
    use crate::model::extrusion_key_for_slot as xkey;
    use super::*;
    use crate::model::ConstraintLine;

    /// Set up a channel-driven REPL session (no terminal): returns the runner, the sender
    /// that plays the role of stdin lines, and the receiver for ready-prompt handoffs. The
    /// initial [`REPL_PROMPT`] handoff is consumed here.
    fn repl_session() -> (
        ScriptRunner,
        std::sync::mpsc::Sender<String>,
        std::sync::mpsc::Receiver<&'static str>,
    ) {
        let (lines_tx, lines_rx) = std::sync::mpsc::channel::<String>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<&'static str>();
        let runner = ScriptRunner::repl_from_channels(lines_rx, ready_tx).expect("repl runner");
        assert_eq!(ready_rx.try_recv(), Ok(REPL_PROMPT), "initial prompt handoff");
        (runner, lines_tx, ready_rx)
    }

    /// Tick the REPL until it hands the next prompt back (i.e. the pending entry finished),
    /// returning that prompt. Panics if it never does.
    fn drive_to_prompt(
        runner: &mut ScriptRunner,
        state: &mut AppState,
        synthetic: &mut SyntheticInput,
        ctx: &egui::Context,
        ready_rx: &std::sync::mpsc::Receiver<&'static str>,
    ) -> &'static str {
        for _ in 0..100 {
            runner.tick(state, synthetic, None, ctx);
            if let Ok(prompt) = ready_rx.try_recv() {
                return prompt;
            }
        }
        panic!("REPL never handed the prompt back");
    }

    /// #404: `add_constraint` sets a status of its own (it used to leave the previous
    /// message lingering), and a creation call's `name=` doesn't clobber the creation
    /// status with "Renamed to …".
    #[test]
    fn scripted_calls_leave_an_accurate_status() {
        let (mut runner, lines_tx, ready_rx) = repl_session();
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();

        lines_tx
            .send("bearcad.rect{ width = 40, height = 20 }\n".to_string())
            .unwrap();
        drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx);
        lines_tx
            .send("bearcad.line{ x = 0, y = 30, x1 = 25, y1 = 30 }\n".to_string())
            .unwrap();
        drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx);
        lines_tx
            .send("bearcad.add_constraint({ kind = \"line\", index = 4 }, \"25mm\")\n".to_string())
            .unwrap();
        drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx);
        assert_eq!(state.status, "Added dimension (25mm)");

        lines_tx
            .send(
                "bearcad.extrude{ polygon = {0, 1, 2, 3}, distance = 12, name = \"Base\" }\n"
                    .to_string(),
            )
            .unwrap();
        drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx);
        assert_eq!(state.doc.extrusions.len(), 1);
        assert_eq!(state.doc.extrusions[xkey(0)].name.as_deref(), Some("Base"));
        assert!(
            state.status.starts_with("Added extrusion ("),
            "name= must not clobber the creation status, got: {}",
            state.status
        );
    }

    #[test]
    fn repl_runs_entries_and_persists_globals_between_them() {
        let (mut runner, lines_tx, ready_rx) = repl_session();
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();

        // First entry sets a global; second uses it to draw. If `x` didn't persist, the
        // rect call would error out with a nil width and create nothing.
        lines_tx.send("x = 4\n".to_string()).unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_PROMPT
        );
        lines_tx
            .send("bearcad.rect{ width = x * 10, height = x }\n".to_string())
            .unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_PROMPT
        );

        // A rectangle is 4 lines (#56); its width came from the persisted global.
        assert_eq!(state.doc.lines.len(), 4, "rect should have created 4 lines");
        let max_x = state
            .doc
            .lines
            .values()
            .flat_map(|l| [l.x0, l.x1])
            .fold(f32::MIN, f32::max);
        let min_x = state
            .doc
            .lines
            .values()
            .flat_map(|l| [l.x0, l.x1])
            .fold(f32::MAX, f32::min);
        assert!((max_x - min_x - 40.0).abs() < 1e-3, "width {}", max_x - min_x);
        assert!(!runner.done, "REPL stays alive between entries");
    }

    /// #214: `bearcad.set_gizmo`/`drag_gizmo` drive the in-progress extrude push/pull depth
    /// through the Lua → Instruction → Action path (there's no Lua entry to *start* an
    /// in-progress extrusion yet, so it's pre-seeded via the same action the tool uses).
    #[test]
    fn lua_gizmo_functions_drive_the_extrude_depth() {
        let (mut runner, lines_tx, ready_rx) = repl_session();
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();

        let sketch = state.doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let lines = crate::construction::add_line_rectangle(
            &mut state.doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4],
        );
        state.apply(crate::actions::Action::ToggleExtrudeFace {
            face: crate::model::ExtrudeFace::Polygon(lines.to_vec()),
        });
        assert!(state.creating_extrusion.is_some());

        lines_tx
            .send("bearcad.set_gizmo{ name = 'extrude', value = 15 }\n".to_string())
            .unwrap();
        drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx);
        assert_eq!(state.creating_extrusion.as_ref().unwrap().distance, 15.0);

        lines_tx
            .send("bearcad.drag_gizmo{ name = 'extrude', by = 5 }\n".to_string())
            .unwrap();
        drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx);
        assert_eq!(state.creating_extrusion.as_ref().unwrap().distance, 20.0);
    }

    #[test]
    fn repl_buffers_multiline_input_until_complete() {
        let (mut runner, lines_tx, ready_rx) = repl_session();
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();

        lines_tx.send("function add(a, b)\n".to_string()).unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_CONT_PROMPT,
            "unclosed function should ask for more input"
        );
        lines_tx.send("  return a + b\n".to_string()).unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_CONT_PROMPT
        );
        lines_tx.send("end\n".to_string()).unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_PROMPT,
            "closing the function completes the entry"
        );

        // The function defined across three lines is callable in a later entry.
        lines_tx.send("sum = add(2, 3)\n".to_string()).unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_PROMPT
        );
        lines_tx
            .send("bearcad.rect{ width = sum, height = sum }\n".to_string())
            .unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_PROMPT
        );
        assert_eq!(state.doc.lines.len(), 4);
    }

    #[test]
    fn repl_survives_errors_and_ends_on_disconnect() {
        let (mut runner, lines_tx, ready_rx) = repl_session();
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();

        // A runtime error is reported and the session continues.
        lines_tx.send("error('boom')\n".to_string()).unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_PROMPT
        );
        assert!(!runner.done, "an error must not end the REPL");
        assert!(runner.error.is_none(), "REPL errors are not fatal script errors");

        // A syntax error likewise.
        lines_tx.send("this is not lua ][\n".to_string()).unwrap();
        assert_eq!(
            drive_to_prompt(&mut runner, &mut state, &mut synthetic, &ctx, &ready_rx),
            REPL_PROMPT
        );
        assert!(!runner.done);

        // Dropping the sender (stdin EOF / Ctrl-D) ends the session.
        drop(lines_tx);
        for _ in 0..10 {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
            if runner.done {
                break;
            }
        }
        assert!(runner.done, "REPL ends when stdin closes");
        assert!(runner.error.is_none());
    }

    #[test]
    fn parse_args_recognizes_repl_flag() {
        let opts = parse_args(["bearcad", "--repl"]);
        assert!(opts.repl);
        assert!(parse_args(["bearcad"]).repl == false);
    }

    #[test]
    fn screenshot_regions_round_trip_through_their_script_names() {
        use crate::actions::Pane;
        for region in [
            ScreenshotRegion::Viewport,
            ScreenshotRegion::Window,
            ScreenshotRegion::Pane(Pane::Context),
            ScreenshotRegion::Pane(Pane::Hierarchy),
            ScreenshotRegion::Pane(Pane::Parameters),
        ] {
            assert_eq!(ScreenshotRegion::from_name(region.script_name()), Some(region));
        }

        // Pane aliases are accepted, and the viewport is still the default.
        assert_eq!(
            ScreenshotRegion::from_name("elements"),
            Some(ScreenshotRegion::Pane(Pane::Hierarchy))
        );
        assert_eq!(ScreenshotRegion::default(), ScreenshotRegion::Viewport);
        assert_eq!(ScreenshotRegion::from_name("nonsense"), None);
    }

    #[test]
    fn screenshot_instruction_writes_its_region_back_out() {
        let viewport = Instruction::Screenshot {
            path: "out.png".to_string(),
            region: ScreenshotRegion::Viewport,
        };
        // The default region stays implicit, exactly as it was written before regions.
        assert_eq!(viewport.as_lua(), "bearcad.ui.screenshot(\"out.png\")");
    }

    #[test]
    fn tool_hints_instruction_writes_back_out() {
        assert_eq!(
            Instruction::ToolHints { on: Some(false) }.as_lua(),
            "bearcad.ui.tool_hints(false)"
        );
        assert_eq!(
            Instruction::ToolHints { on: None }.as_lua(),
            "bearcad.ui.tool_hints()"
        );
    }

    /// #736: every unit instruction exports as a replayable `bearcad.*` call.
    #[test]
    fn unit_instructions_export_as_lua() {
        assert_eq!(
            Instruction::ImportUnit {
                path: "a.bearcad".to_string(),
                link: Some(crate::model::LinkMode::Static),
                name: Some("bracket".to_string()),
            }
            .as_lua(),
            "bearcad.import_unit{ path = \"a.bearcad\", link = \"static\", name = \"bracket\" }"
        );
        assert_eq!(
            Instruction::AddUnitInstance { unit: 0, name: Some("b2".to_string()) }.as_lua(),
            "bearcad.add_unit_instance{ unit = 0, name = \"b2\" }"
        );
        assert_eq!(
            Instruction::CloneUnitInstance { instance: 1 }.as_lua(),
            "bearcad.clone_unit_instance{ instance = 1 }"
        );
        assert_eq!(
            Instruction::SetUnitParameterOverride {
                instance: 1,
                name: "width".to_string(),
                expression: Some("20".to_string()),
            }
            .as_lua(),
            "bearcad.unit_override{ instance = 1, name = \"width\", value = \"20\" }"
        );
        assert_eq!(Instruction::SyncUnit { unit: 2 }.as_lua(), "bearcad.sync_unit(2)");
        assert_eq!(
            Instruction::SetUnitLink { unit: 0, link: crate::model::LinkMode::Dynamic }
                .as_lua(),
            "bearcad.unit_link(0, \"dynamic\")"
        );

        let pane = Instruction::Screenshot {
            path: "out.png".to_string(),
            region: ScreenshotRegion::Pane(crate::actions::Pane::Context),
        };
        assert_eq!(pane.as_lua(), "bearcad.ui.screenshot(\"out.png\", \"context\")");
    }

    #[test]
    fn create_line_instruction_renders_bezier_when_present() {
        let straight = Instruction::CreateLine {
            x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0, bezier: None, dimension: None,
        };
        assert_eq!(straight.as_lua(), "bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0 }");

        let curved = Instruction::CreateLine {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 0.0,
            bezier: Some([(3.0, 4.0), (7.0, 4.0)]),
            dimension: None,
        };
        assert_eq!(
            curved.as_lua(),
            "bearcad.line{ x = 0, y = 0, x1 = 10, y1 = 0, bezier = { { 3, 4 }, { 7, 4 } } }"
        );
    }

    #[test]
    fn set_units_instructions_render_replayable_lua() {
        let doc_units = Instruction::SetDocumentUnits { length: LengthUnit::In, angle: AngleUnit::Rad };
        assert_eq!(
            doc_units.as_lua(),
            "bearcad.set_units{ length = \"in\", angle = \"rad\" }"
        );

        let sketch_override = Instruction::SetSketchUnits {
            sketch: 2,
            length: Some(LengthUnit::Cm),
            angle: None,
        };
        assert_eq!(
            sketch_override.as_lua(),
            "bearcad.set_units{ sketch = 2, length = \"cm\" }"
        );

        let sketch_inherit =
            Instruction::SetSketchUnits { sketch: 0, length: None, angle: None };
        assert_eq!(sketch_inherit.as_lua(), "bearcad.set_units{ sketch = 0 }");
    }

    #[test]
    fn parse_key_names() {
        assert_eq!(parse_key("enter").unwrap(), Key::Enter);
        assert_eq!(parse_key("ESC").unwrap(), Key::Escape);
        assert_eq!(parse_key("`").unwrap(), Key::Backtick);
        assert_eq!(parse_key("backtick").unwrap(), Key::Backtick);
        assert!(parse_key("notakey").is_err());
    }

    #[test]
    fn screenshot_crop_bounds_scale_by_pixels_per_point() {
        // 800x600 logical window at 2x DPI -> 1600x1200 framebuffer.
        let rect = egui::Rect::from_min_max(egui::pos2(220.0, 40.0), egui::pos2(800.0, 600.0));
        let (x0, y0, x1, y1) = crop_bounds(1600, 1200, rect, 2.0);
        assert_eq!((x0, y0, x1, y1), (440, 80, 1600, 1200));
    }

    #[test]
    fn screenshot_crop_bounds_clamp_to_image() {
        // Viewport extends past the framebuffer; bounds clamp instead of overflowing.
        let rect = egui::Rect::from_min_max(egui::pos2(-10.0, -10.0), egui::pos2(2000.0, 2000.0));
        let (x0, y0, x1, y1) = crop_bounds(1600, 1200, rect, 1.0);
        assert_eq!((x0, y0, x1, y1), (0, 0, 1600, 1200));
    }

    #[test]
    fn screenshot_crop_produces_subimage_dimensions() {
        // 4x4 image, crop the bottom-right 2x2 (logical rect at 1x DPI).
        let pixels = vec![egui::Color32::WHITE; 16];
        let image = egui::ColorImage {
            size: [4, 4],
            pixels,
            ..Default::default()
        };
        let rect = egui::Rect::from_min_max(egui::pos2(2.0, 2.0), egui::pos2(4.0, 4.0));
        let (x0, y0, x1, y1) = crop_bounds(image.width(), image.height(), rect, 1.0);
        assert_eq!((x1 - x0, y1 - y0), (2, 2));
    }

    #[test]
    fn parse_cli_help_flags() {
        assert_eq!(parse_cli(["bearcad", "--help"]), CliOutcome::Help);
        assert_eq!(parse_cli(["bearcad", "-h"]), CliOutcome::Help);
    }

    #[test]
    fn parse_cli_testplan_subcommand() {
        assert_eq!(parse_cli(["bearcad", "testplan"]), CliOutcome::Testplan);
    }

    #[test]
    fn usage_lists_testplan() {
        let usage = usage_text();
        assert!(
            usage.contains("testplan"),
            "help should list the testplan command"
        );
        assert!(
            usage.contains("CUSTOM_ITEMS"),
            "help should point at the custom-items hook"
        );
        assert!(
            usage.to_ascii_lowercase().contains("tutorial"),
            "help should mention that testplan lists tutorials"
        );
    }

    #[test]
    fn parse_show_commands_flag() {
        let opts = parse_args(["bearcad", "--show-commands"]);
        assert!(opts.show_commands);
    }

    #[test]
    fn instruction_from_action_preserves_a_curved_committed_line() {
        let mut doc = crate::model::Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let mut line = crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        line.bezier = Some([(3.0, 4.0), (7.0, 4.0)]);
        doc.lines.insert(line);
        let instruction = instruction_from_action(&Action::CommitLine, &doc).unwrap();
        assert_eq!(
            instruction,
            Instruction::CreateLine {
                x0: 0.0,
                y0: 0.0,
                x1: 10.0,
                y1: 0.0,
                bezier: Some([(3.0, 4.0), (7.0, 4.0)]),
                dimension: None,
            }
        );
    }

    #[test]
    fn vertex_treatment_instruction_renders_as_the_matching_lua_call() {
        let point = ConstraintPoint::LineEndpoint { line: lkey(0), end: crate::model::LineEnd::End };
        let chamfer = Instruction::VertexTreatment {
            points: vec![point.clone()],
            kind: VertexTreatmentKind::Chamfer,
            amount: "3".to_string(),
        };
        assert_eq!(
            chamfer.as_lua(),
            "bearcad.chamfer_vertex{ point = { kind = \"line\", index = 0, [\"end\"] = \"end\" }, distance = 3 }"
        );
        let fillet = Instruction::VertexTreatment {
            points: vec![point.clone()],
            kind: VertexTreatmentKind::Fillet,
            amount: "2.5".to_string(),
        };
        assert_eq!(
            fillet.as_lua(),
            "bearcad.fillet_vertex{ point = { kind = \"line\", index = 0, [\"end\"] = \"end\" }, radius = 2.5 }"
        );
        // A parametric amount records as a quoted string so it survives replay.
        let parametric = Instruction::VertexTreatment {
            points: vec![point.clone()],
            kind: VertexTreatmentKind::Fillet,
            amount: "r".to_string(),
        };
        assert_eq!(
            parametric.as_lua(),
            "bearcad.fillet_vertex{ point = { kind = \"line\", index = 0, [\"end\"] = \"end\" }, radius = \"r\" }"
        );
        let two = Instruction::VertexTreatment {
            points: vec![
                point.clone(),
                ConstraintPoint::LineEndpoint { line: lkey(1), end: crate::model::LineEnd::End },
            ],
            kind: VertexTreatmentKind::Fillet,
            amount: "3".to_string(),
        };
        assert_eq!(
            two.as_lua(),
            "bearcad.fillet_vertex{ points = { { kind = \"line\", index = 0, [\"end\"] = \"end\" }, { kind = \"line\", index = 1, [\"end\"] = \"end\" } }, radius = 3 }"
        );
    }

    #[test]
    fn instruction_from_action_maps_commit_vertex_treatment() {
        let doc = crate::model::Document::default();
        let point = ConstraintPoint::LineEndpoint { line: lkey(2), end: crate::model::LineEnd::Start };
        let action = Action::CommitVertexTreatment {
            point: point.clone(),
            kind: VertexTreatmentKind::Fillet,
            amount: "4".to_string(),
        };
        assert_eq!(
            instruction_from_action(&action, &doc),
            Some(Instruction::VertexTreatment {
                points: vec![point],
                kind: VertexTreatmentKind::Fillet,
                amount: "4".to_string(),
            })
        );
    }

    #[test]
    fn edge_treatment_instruction_renders_as_the_matching_lua_call() {
        use crate::model::ExtrusionEdgeRef;
        let edge = ExtrusionEdgeRef::Vertical { face: 0, edge: 2 };
        let chamfer = Instruction::EdgeTreatment {
            edges: vec![(TreatableSolidRef::Extrusion(1), edge)],
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
            expression: String::new(),
        };
        assert_eq!(
            chamfer.as_lua(),
            "bearcad.chamfer_edge{ extrusion = 1, edge = { kind = \"vertical\", face = 0, edge = 2 }, distance = 3 }"
        );
        let cap_edge = ExtrusionEdgeRef::Cap { face: 1, edge: 3, top: true };
        let fillet = Instruction::EdgeTreatment {
            edges: vec![(TreatableSolidRef::Extrusion(0), cap_edge)],
            kind: VertexTreatmentKind::Fillet,
            amount: 1.5,
            expression: String::new(),
        };
        assert_eq!(
            fillet.as_lua(),
            "bearcad.fillet_edge{ extrusion = 0, edge = { kind = \"cap\", face = 1, edge = 3, top = true }, radius = 1.5 }"
        );
        // A whole set (#672) renders as the plural `edges` list — one call, one operation.
        let set = Instruction::EdgeTreatment {
            edges: vec![
                (TreatableSolidRef::Extrusion(0), edge),
                (TreatableSolidRef::Extrusion(0), cap_edge),
            ],
            kind: VertexTreatmentKind::Fillet,
            amount: 8.0,
            expression: String::new(),
        };
        assert_eq!(
            set.as_lua(),
            concat!(
                "bearcad.fillet_edge{ edges = ",
                "{ { extrusion = 0, edge = { kind = \"vertical\", face = 0, edge = 2 } }, ",
                "{ extrusion = 0, edge = { kind = \"cap\", face = 1, edge = 3, top = true } } }, ",
                "radius = 8 }"
            )
        );
    }

    #[test]
    fn instruction_from_action_maps_commit_edge_treatment() {
        use crate::model::ExtrusionEdgeRef;
        // An extrusion is named by its ordinal among the live ones (#1055), so the document
        // has to actually hold that many.
        let mut doc = crate::model::Document::default();
        for _ in 0..3 {
            doc.extrusions.insert(crate::model::Extrusion {
                sketch: skey(0),
                faces: Vec::new(),
                distance: 1.0,
                target: None,
                expression: String::new(),
                symmetric: false,
                name: None,
                taper: 0.0,
                taper_mode: crate::model::ExtrudeTaperMode::Distance,
                taper_expression: String::new(),
                edge_treatments: Vec::new(),
            });
        }
        let edge = ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: false };
        let action = Action::CommitEdgeTreatments {
            edges: vec![(crate::model::TreatableSolid::Extrusion(xkey(2)), edge)],
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.5,
            expression: String::new(),
        };
        assert_eq!(
            instruction_from_action(&action, &doc),
            Some(Instruction::EdgeTreatment {
                edges: vec![(TreatableSolidRef::Extrusion(2), edge)],
                kind: VertexTreatmentKind::Chamfer,
                amount: 2.5,
                expression: String::new(),
            })
        );
    }

    #[test]
    fn instruction_from_action_maps_tool_changes() {
        let state = AppState::default();
        let instruction =
            instruction_from_action(&Action::SetTool(Tool::Rectangle), &state.doc).unwrap();
        assert_eq!(instruction, Instruction::Tool(Tool::Rectangle));
    }

    /// #1351: Project actions replay as `bearcad.project{ ... }`.
    #[test]
    fn project_instruction_renders_as_lua() {
        assert_eq!(
            Instruction::Project { elements: vec![] }.as_lua(),
            "bearcad.project()"
        );
        assert_eq!(
            Instruction::Project {
                elements: vec![SceneElement::ConstructionPlane(pkey(2))],
            }
            .as_lua_in(Some(&crate::model::Document::default())),
            "bearcad.project{ entities = { { kind = \"construction_plane\", index = 2 } } }"
        );
    }

    #[test]
    fn instruction_from_action_maps_project() {
        let doc = crate::model::Document::default();
        assert_eq!(
            instruction_from_action(&Action::ProjectSelection, &doc),
            Some(Instruction::Project { elements: vec![] })
        );
        assert_eq!(
            instruction_from_action(
                &Action::ProjectElement {
                    element: SceneElement::ConstructionPlane(pkey(2)),
                },
                &doc
            ),
            Some(Instruction::Project {
                elements: vec![SceneElement::ConstructionPlane(pkey(2))],
            })
        );
    }

    #[test]
    fn parse_cli_run_delegates_to_script_options() {
        assert_eq!(
            parse_cli(["bearcad", "--script", "test.lua", "--exit"]),
            CliOutcome::Run(ScriptOptions {
                script_path: Some("test.lua".to_string()),
                document_path: None,
                exit_on_complete: true,
                show_commands: false,
                timeout_secs: None,
                repl: false,
                tutorial: None,
                rebuild: false,
            })
        );
    }

    #[test]
    fn parse_args_finds_timeout_flag() {
        let opts = parse_args(["bearcad", "--exit", "--timeout", "30"]);
        assert_eq!(opts.timeout_secs, Some(30));
    }

    /// #765: `--tutorial <name>` is the desktop twin of the web `?tutorial=` link.
    #[test]
    fn parse_args_finds_the_tutorial_flag() {
        let opts = parse_args(["bearcad", "--tutorial", "cube"]);
        assert_eq!(opts.tutorial.as_deref(), Some("cube"));
        assert_eq!(parse_args(["bearcad"]).tutorial, None);
    }

    #[test]
    fn parse_args_ignores_invalid_timeout_value() {
        let opts = parse_args(["bearcad", "--timeout", "soon"]);
        assert_eq!(opts.timeout_secs, None);
    }

    #[test]
    fn parse_args_finds_script_flag() {
        let opts = parse_args(["bearcad", "--script", "test.lua", "--exit"]);
        assert_eq!(opts.script_path.as_deref(), Some("test.lua"));
        assert!(opts.exit_on_complete);
    }

    #[test]
    fn parse_args_finds_positional_script() {
        let opts = parse_args(["bearcad", "demo.lua"]);
        assert_eq!(opts.script_path.as_deref(), Some("demo.lua"));
    }

    #[test]
    fn parse_args_finds_positional_document_and_exit() {
        let opts = parse_args(["bearcad", "/tmp/test.bearcad", "--exit"]);
        assert_eq!(opts.document_path.as_deref(), Some("/tmp/test.bearcad"));
        assert!(opts.exit_on_complete);
        assert!(opts.script_path.is_none());
    }

    /// Some launchers pass a `file://` URL instead of a path (#1326).
    #[test]
    fn parse_args_decodes_file_url_document() {
        let opts = parse_args(["bearcad", "file:///tmp/My%20Part.bearcad", "--exit"]);
        assert_eq!(opts.document_path.as_deref(), Some("/tmp/My Part.bearcad"));
    }

    #[test]
    fn parse_args_exit_without_paths_exits_after_startup() {
        let opts = parse_args(["bearcad", "--exit"]);
        assert!(opts.exit_on_complete);
        assert!(opts.script_path.is_none());
        assert!(opts.document_path.is_none());
    }

    /// #1343: `--rebuild` discards persisted tessellation after open.
    #[test]
    fn parse_args_finds_rebuild_flag() {
        let opts = parse_args(["bearcad", "part.bearcad", "--rebuild"]);
        assert!(opts.rebuild);
        assert_eq!(opts.document_path.as_deref(), Some("part.bearcad"));
        assert!(!parse_args(["bearcad"]).rebuild);
    }

    /// #1074: an exported script spells a point on a face by the face's key, and only mentions
    /// how far across the face it sits when that isn't the middle — so the common case reads
    /// exactly as it did when a face point could only ever be the centre (#738).
    #[test]
    fn move_point_on_a_face_exports_its_offset_only_when_it_has_one() {
        let on_face = |uv: [i32; 2]| crate::model::MovePointRef::OnFace {
            body: crate::model::body_key_for_slot(2),
            centroid: [0, 0, 500],
            normal: [0, 0, 100],
            uv,
        };
        assert_eq!(
            move_point_lua(&on_face([0, 0])),
            "{ body = 2, on_face = { 0, 0, 5 }, normal = { 0, 0, 1 } }"
        );
        assert_eq!(
            move_point_lua(&on_face([300, -250])),
            "{ body = 2, on_face = { 0, 0, 5 }, normal = { 0, 0, 1 }, uv = { 3, -2.5 } }"
        );
    }

    #[test]
    fn instruction_as_lua_formats_click() {
        let ins = Instruction::Click { x: 100.0, y: 200.0, mods: ClickMods::default() };
        assert_eq!(ins.as_lua(), "bearcad.ui.click(100, 200)");
    }

    #[test]
    fn script_drag_line_translates_segment() {
        let mut runner = ScriptRunner::from_instructions(vec![
            Instruction::Tool(Tool::Line),
            Instruction::Tool(Tool::Select),
            Instruction::DragLineSegment {
                target: ConstraintLine::Line(lkey(0)),
                anchor_u: 0.0,
                anchor_v: 0.0,
                u: 4.0,
                v: 0.0,
            },
        ]);
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        state.creating_line = Some(crate::actions::CreatingLine {
            origin: glam::Vec3::ZERO,
            text: String::new(),
            last_mouse: glam::Vec3::new(10.0, 0.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(crate::actions::Action::CommitLine);
        while !runner.done {
            runner.tick(
                &mut state,
                &mut synthetic,
                None,
                &egui::Context::default(),
            );
        }
        let line = &state.doc.lines[lkey(0)];
        assert!((line.x0 - 4.0).abs() < 1e-2);
        assert!((line.y0).abs() < 1e-2);
        assert!((line.x1 - 14.0).abs() < 1e-2);
    }

    #[test]
    fn script_palette_run_sets_top_view() {
        let mut runner = ScriptRunner::from_instructions(vec![Instruction::RunPaletteCommand {
            query: "view top".into(),
            argument: None,
        }]);
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        while !runner.done {
            runner.tick(
                &mut state,
                &mut synthetic,
                None,
                &egui::Context::default(),
            );
        }
        assert!(state.cam.is_transitioning());
    }

    #[test]
    fn script_delete_selection_removes_a_line() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(crate::model::FaceId::default());
        state.doc.lines.insert(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 0.0, 5.0, 0.0,
        ));
        state.doc.shape_order.push(crate::model::ShapeKind::Line);
        let mut runner = ScriptRunner::from_instructions(vec![
            Instruction::SelectSceneElement {
                element: SceneElement::Line(lkey(0)),
                additive: false,
            },
            Instruction::DeleteSelection,
        ]);
        runner.verbose = false;
        let mut synthetic = SyntheticInput::default();
        let ctx = egui::Context::default();
        while !runner.done {
            runner.tick(&mut state, &mut synthetic, None, &ctx);
        }
        assert!(!state.doc.lines.contains(lkey(0)));
    }

    #[test]
    fn script_adds_and_renames_parameters() {
        let mut runner = ScriptRunner::from_instructions(vec![
            Instruction::AddParameter {
                name: "A".into(),
                expression: "5mm".into(),
            },
            Instruction::AddParameter {
                name: "B".into(),
                expression: "A+5in".into(),
            },
            Instruction::SetParameterName {
                index: 0,
                name: "Len".into(),
            },
        ]);
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        while !runner.done {
            runner.tick(
                &mut state,
                &mut synthetic,
                None,
                &egui::Context::default(),
            );
        }
        assert_eq!(state.doc.parameters.len(), 2);
        assert_eq!(state.doc.parameters.values().next().unwrap().name, "Len");
        assert_eq!(state.doc.parameters.values().nth(1).unwrap().expression, "Len+5in");
    }

    #[test]
    fn script_adds_angle_parameter() {
        let mut runner = ScriptRunner::from_instructions(vec![Instruction::AddParameter {
            name: "corner".into(),
            expression: "16.7deg".into(),
        }]);
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        while !runner.done {
            runner.tick(
                &mut state,
                &mut synthetic,
                None,
                &egui::Context::default(),
            );
        }
        assert_eq!(state.doc.parameters.values().next().unwrap().expression, "16.7deg");
        let angle = crate::value::eval_parameter_in_doc("corner", &state.doc).unwrap();
        match angle {
            crate::value::EvaluatedParameter::AngleRad(v) => {
                assert!((v.to_degrees() - 16.7).abs() < 1e-2);
            }
            _ => panic!("expected angle parameter"),
        }
    }

    #[test]
    fn runner_set_dim_expression_evaluates_length() {
        let mut runner = ScriptRunner::from_instructions(vec![
            Instruction::Tool(Tool::Line),
            Instruction::SetLineLength {
                value: "2in + 5mm / 2".into(),
            },
        ]);
        runner.verbose = false;
        let mut state = AppState::default();
        let mut synthetic = SyntheticInput::default();
        state.apply(crate::actions::Action::BeginSketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            viewport: None,
        });
        state.creating_line = Some(crate::actions::CreatingLine {
            origin: glam::Vec3::ZERO,
            text: String::new(),
            last_mouse: glam::Vec3::new(10.0, 10.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });

        while !runner.done {
            runner.tick(
                &mut state,
                &mut synthetic,
                None,
                &egui::Context::default(),
            );
        }

        let cl = state.creating_line.as_ref().unwrap();
        assert_eq!(cl.text, "2in + 5mm / 2");
        let sketch = state.sketch_session.unwrap().sketch;
        let frame = crate::face::sketch_geometry_frame(&state.doc, sketch).unwrap();
        let end = cl.end_point(&frame, &state.doc);
        let (u0, v0) = crate::face::world_to_local(&frame, cl.origin);
        let (u1, v1) = crate::face::world_to_local(&frame, end);
        let len = crate::model::Line::from_local_endpoints(sketch, u0, v0, u1, v1).length();
        assert!((len - 53.3).abs() < 1e-2);
    }
}