//! Lua script runner and internal instruction dispatch (SPEC §8).
//!
//! Scripts are `.lua` files that call the global `bearcad` API. They drive the
//! live UI via synthetic pointer/keyboard events and headless actions.

use crate::actions::{
    dim_label_target_in_sketch, Action, ActionResult, AppState, DimLabelAxis, Pane, RectAxis,
    Tool,
};
use crate::command_palette::{best_match, commands_for_state, PaletteOutcome};
use crate::constraints::add_distance_constraint;
use crate::hierarchy::SceneElement;
use crate::model::{
    ConstraintLine, ConstraintPoint, DistanceTarget, ExtrudeFace, FaceId, SketchId,
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
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// A single script instruction.
#[derive(Clone, Debug, PartialEq)]
pub enum Instruction {
    // Document / tool actions
    New,
    Open(String),
    Save(Option<String>),
    /// Export bodies to an STL file at `path`; `body` names a single body (`None` = all).
    ExportStl { path: String, body: Option<String> },
    /// Export bodies to a STEP file at `path`; `body` names a single body (`None` = all).
    ExportStep { path: String, body: Option<String> },
    /// Import an STL file at `path` as a new body (#70).
    ImportStl { path: String },
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
        a: (f32, f32),
        b: (f32, f32),
        length: f32,
    },
    /// Import a STEP file at `path` as a new body (#71).
    ImportStep { path: String },
    Clear,
    Undo,
    Tool(Tool),
    BeginSketch { face: FaceId },
    OpenSketch { sketch: SketchId },
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
    },
    /// Extrude coplanar sketch faces into a solid.
    Extrude {
        sketch: SketchId,
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
    /// Add a body view (in an orientation) to a drawing.
    AddDrawingView {
        drawing: usize,
        body: usize,
        orientation: crate::model::DrawingOrientation,
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
    /// Revolve profiles around an axis (SPEC §3.5 Revolve). Sketch inferred per face.
    Revolve {
        faces: Vec<crate::model::ExtrudeFace>,
        axis: crate::model::RevolveAxis,
        angle_deg: f32,
        symmetric: bool,
        body: crate::actions::RevolveBodyChoice,
        bodies: Vec<usize>,
    },
    /// Sweep profiles along a path of sketch lines (SPEC §3.5 Sweep). Sketch
    /// inferred per face.
    Sweep {
        faces: Vec<crate::model::ExtrudeFace>,
        path: Vec<usize>,
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
        tx: String,
        ty: String,
        tz: String,
        axis: Option<crate::model::RevolveAxis>,
        angle: String,
        /// Snap-translate points (#649/#650): with both set the move snaps `source` onto
        /// `target` and the tx/ty/tz expressions are ignored.
        source_point: Option<crate::model::MovePointRef>,
        target_point: Option<crate::model::MovePointRef>,
        /// The point the rotation turns about (#651); `None` follows `source_point`.
        rotation_point: Option<crate::model::MovePointRef>,
        /// Free Rotate's two extra axis+angle slots (#652).
        extra_rotations: [crate::model::MoveRotationSlot; 2],
    },
    /// Re-point an existing move operation.
    EditMoveOp {
        op: usize,
        targets: Vec<usize>,
        tx: String,
        ty: String,
        tz: String,
        axis: Option<crate::model::RevolveAxis>,
        angle: String,
        source_point: Option<crate::model::MovePointRef>,
        target_point: Option<crate::model::MovePointRef>,
        rotation_point: Option<crate::model::MovePointRef>,
        extra_rotations: [crate::model::MoveRotationSlot; 2],
    },
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
        mode: crate::model::RepeatMode,
        count: String,
        spacing: String,
        length: String,
        length_target: Option<crate::model::ExtrudeTarget>,
    },
    /// Slice bodies with planar cutters (Slice tool).
    CreateSliceOp {
        targets: Vec<usize>,
        cutters: Vec<FaceId>,
        extend_infinite: bool,
    },
    /// Re-point an existing slice operation.
    EditSliceOp {
        op: usize,
        targets: Vec<usize>,
        cutters: Vec<FaceId>,
        extend_infinite: bool,
    },
    SetElementVisible {
        element: SceneElement,
        visible: Option<bool>,
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
    SetElementName {
        element: SceneElement,
        name: String,
    },
    FocusElementName,
    /// Set the document-wide default length/angle units (#52).
    SetDocumentUnits { length: LengthUnit, angle: AngleUnit },
    /// Set (or clear, via `None`) a per-sketch length/angle unit override (#52).
    SetSketchUnits {
        sketch: SketchId,
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
    /// Force touch mode on/off (auto-detected from real touches otherwise).
    SetTouchMode { on: bool },
    /// Start / advance / end an interactive tutorial.
    StartTutorial { index: usize },
    TutorialNext,
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
    /// Chamfer or fillet a sketch vertex where exactly two plain lines meet (#37/#38):
    /// truncates both lines back from the vertex and bridges them with a new line (straight
    /// for a chamfer, single-cubic-bezier arc for a fillet). `amount` is the chamfer distance
    /// or fillet radius depending on `kind`.
    VertexTreatment {
        point: ConstraintPoint,
        kind: VertexTreatmentKind,
        /// Chamfer distance / fillet radius as a parametric expression (mm), so tying it to a
        /// parameter keeps the bevel following that parameter (#538/#554).
        amount: String,
    },
    /// Chamfer or fillet an analytic edge of an extrusion's 3D solid (#77) — a mesh-bevel
    /// approximation scoped to the vertical and side/cap edges of a `Rect`/`Polygon`-profiled
    /// extrusion (see `crate::model::ExtrusionEdgeRef`, SPEC §3.4). `amount` is the chamfer
    /// distance or fillet radius depending on `kind`.
    EdgeTreatment {
        extrusion: usize,
        edge: crate::model::ExtrusionEdgeRef,
        kind: VertexTreatmentKind,
        amount: f32,
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
    AddParameter { name: String, expression: String },
    CreateParameterFromLineLength { line_index: usize, name: Option<String> },
    /// Create a derived (measured) parameter from a geometry source (#432).
    CreateDerivedParameter {
        source: crate::model::ParameterSource,
        name: Option<String>,
    },
    SetParameterName { index: usize, name: String },
    SetParameterExpression { index: usize, expression: String },
    DeleteParameter { index: usize },
    DeleteSelection,
    /// Show/hide the command palette. `None` toggles.
    SetCommandPalette { open: Option<bool> },
    /// Run the best-matching palette command for a query.
    RunPaletteCommand { query: String },
    // Synthetic input (viewport-local pixel coordinates)
    Move { x: f32, y: f32 },
    Click { x: f32, y: f32 },
    /// Move/click at ground-plane world coordinates (millimetres, z = 0).
    MoveGround { x: f32, y: f32 },
    ClickGround { x: f32, y: f32 },
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
    Key(Key),
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

    // Sequencing
    WaitMs(u64),
    WaitFrames(u32),
    /// Save a screenshot. `whole_window` captures the full window; otherwise just the 3D
    /// viewport (with the view-cube HUD suppressed).
    Screenshot {
        path: String,
        whole_window: bool,
    },
    Quit,
}

impl Instruction {
    /// Format this instruction as a Lua API call (for `--show-commands` logging).
    pub fn as_lua(&self) -> String {
        match self {
            Instruction::New => "bearcad.new()".to_string(),
            Instruction::Open(path) => format!("bearcad.open({path:?})"),
            Instruction::Save(None) => "bearcad.save()".to_string(),
            Instruction::Save(Some(path)) => format!("bearcad.save({path:?})"),
            Instruction::ExportStl { path, body: None } => format!("bearcad.export_stl({path:?})"),
            Instruction::ExportStl {
                path,
                body: Some(body),
            } => format!("bearcad.export_stl({path:?}, {body:?})"),
            Instruction::ExportStep { path, body: None } => format!("bearcad.export_step({path:?})"),
            Instruction::ExportStep {
                path,
                body: Some(body),
            } => format!("bearcad.export_step({path:?}, {body:?})"),
            Instruction::ImportStl { path } => format!("bearcad.import_stl({path:?})"),
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
            Instruction::CalibrateImage { image, a, b, length } => format!(
                "bearcad.calibrate_image{{ image = {image}, from = {{ {}, {} }}, to = {{ {}, {} }}, length = {length} }}",
                a.0, a.1, b.0, b.1
            ),
            Instruction::ImportStep { path } => format!("bearcad.import_step({path:?})"),
            Instruction::Clear => "bearcad.clear()".to_string(),
            Instruction::Undo => "bearcad.undo()".to_string(),
            Instruction::Tool(tool) => format!("bearcad.ui.tool({:?})", tool_lua_name(*tool)),
            Instruction::BeginSketch { face } => {
                let (kind, index) = face_lua_parts(face);
                format!("bearcad.begin_sketch({kind:?}, {index})")
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
            } => {
                let mut args = format!("text = {:?}, x = {x}, y = {y}, size = {:?}", text, size);
                if let Some(font) = font {
                    args.push_str(&format!(", font = {font:?}"));
                }
                for (flag, name) in [(bold, "bold"), (italic, "italic"), (underline, "underline")] {
                    if *flag {
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
            Instruction::Extrude {
                faces,
                distance,
                body,
                target,
                expression,
                symmetric,
                ..
            } => {
                let body = match body {
                    crate::actions::ExtrudeBodyChoice::New => "",
                    crate::actions::ExtrudeBodyChoice::Merge => ", body = \"merge\"",
                    crate::actions::ExtrudeBodyChoice::Cut => ", body = \"cut\"",
                };
                let to = target
                    .as_ref()
                    .map(|t| format!(", to = {}", extrude_target_lua_table(t)))
                    .unwrap_or_default();
                let distance = match expression {
                    Some(e) => format!("{e:?}"),
                    None => distance.to_string(),
                };
                let sym = if *symmetric { ", symmetric = true" } else { "" };
                format!(
                    "bearcad.extrude{{ {}, distance = {distance}{body}{to}{sym} }}",
                    extrude_face_args(faces)
                )
            }
            Instruction::ExtrudeBodyFace { face, distance, body, target } => {
                let body = match body {
                    crate::actions::ExtrudeBodyChoice::New => "",
                    crate::actions::ExtrudeBodyChoice::Merge => ", body = \"merge\"",
                    crate::actions::ExtrudeBodyChoice::Cut => ", body = \"cut\"",
                };
                let to = target
                    .as_ref()
                    .map(|t| format!(", to = {}", extrude_target_lua_table(t)))
                    .unwrap_or_default();
                format!(
                    "bearcad.extrude_face{{ face = {}, distance = {distance}{body}{to} }}",
                    face_id_lua_ref(face)
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
                    .map(|t| format!(", to = {}", extrude_target_lua_table(t)))
                    .unwrap_or_default();
                format!("bearcad.edit_extrusion{{ extrusion = {extrusion}{d}{to} }}")
            }
            Instruction::Loft { faces, body, bodies } => {
                use crate::model::ExtrudeFace;
                let index_list = |indices: &[usize]| -> String {
                    indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                };
                let mut circles = Vec::new();
                let mut polygons = Vec::new();
                for face in faces {
                    match face {
                        ExtrudeFace::Circle(i) => circles.push(*i),
                        ExtrudeFace::Polygon(lines) => polygons.push(lines),
                        // Boolean regions aren't loftable sections (no interactive path
                        // constructs one), so nothing to render.
                        ExtrudeFace::Boolean { .. } | ExtrudeFace::TextGlyph { .. } => {}
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
                            .map(|lines| format!("{{{}}}", index_list(lines)))
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
                body,
                orientation,
            } => format!(
                "bearcad.drawing_view{{ drawing = {drawing}, body = {body}, orientation = {:?} }}",
                orientation.label().to_ascii_lowercase()
            ),
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
            Instruction::Revolve {
                faces,
                axis,
                angle_deg,
                symmetric,
                body,
                bodies,
            } => {
                use crate::model::ExtrudeFace;
                let index_list = |indices: &[usize]| -> String {
                    indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                };
                let mut parts = Vec::new();
                let circles: Vec<usize> = faces
                    .iter()
                    .filter_map(|f| match f {
                        ExtrudeFace::Circle(i) => Some(*i),
                        _ => None,
                    })
                    .collect();
                if !circles.is_empty() {
                    parts.push(format!("circles = {{{}}}", index_list(&circles)));
                }
                for f in faces {
                    if let ExtrudeFace::Polygon(lines) = f {
                        parts.push(format!("polygon = {{{}}}", index_list(lines)));
                    }
                }
                parts.push(format!("axis = {}", revolve_axis_lua(*axis)));
                parts.push(format!("angle = {angle_deg}"));
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
                let mut parts = Vec::new();
                let circles: Vec<usize> = faces
                    .iter()
                    .filter_map(|f| match f {
                        ExtrudeFace::Circle(i) => Some(*i),
                        _ => None,
                    })
                    .collect();
                if !circles.is_empty() {
                    parts.push(format!("circles = {{{}}}", index_list(&circles)));
                }
                for f in faces {
                    if let ExtrudeFace::Polygon(lines) = f {
                        parts.push(format!("polygon = {{{}}}", index_list(lines)));
                    }
                }
                parts.push(format!("path = {{{}}}", index_list(path)));
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
            Instruction::EditBooleanOp { op, kind, a, b, keep_b } => {
                boolean_op_lua("bearcad.edit_boolean", Some(*op), *kind, a, b, *keep_b)
            }
            Instruction::CreateMoveOp { targets, tx, ty, tz, axis, angle, source_point, target_point, rotation_point, extra_rotations } => {
                move_op_lua("bearcad.move_bodies", None, targets, tx, ty, tz, *axis, angle, source_point, target_point, rotation_point, extra_rotations)
            }
            Instruction::EditMoveOp { op, targets, tx, ty, tz, axis, angle, source_point, target_point, rotation_point, extra_rotations } => {
                move_op_lua("bearcad.edit_move", Some(*op), targets, tx, ty, tz, *axis, angle, source_point, target_point, rotation_point, extra_rotations)
            }
            Instruction::CreateMirrorOp { plane, targets, mode } => {
                mirror_op_lua("bearcad.mirror_bodies", None, plane, targets, *mode)
            }
            Instruction::EditMirrorOp { op, plane, targets, mode } => {
                mirror_op_lua("bearcad.edit_mirror", Some(*op), plane, targets, *mode)
            }
            Instruction::CreateRepeatOp { targets, axis, mode, count, spacing, length, length_target } => {
                repeat_op_lua("bearcad.repeat_bodies", None, targets, *axis, *mode, count, spacing, length, length_target.as_ref())
            }
            Instruction::EditRepeatOp { op, targets, axis, mode, count, spacing, length, length_target } => {
                repeat_op_lua("bearcad.edit_repeat", Some(*op), targets, *axis, *mode, count, spacing, length, length_target.as_ref())
            }
            Instruction::CreateSliceOp { targets, cutters, extend_infinite } => {
                slice_op_lua("bearcad.slice", None, targets, cutters, *extend_infinite)
            }
            Instruction::EditSliceOp { op, targets, cutters, extend_infinite } => {
                slice_op_lua("bearcad.edit_slice", Some(*op), targets, cutters, *extend_infinite)
            }
            Instruction::SetElementVisible { element, visible } => {
                let target = element_lua_ref(element);
                let verb = match visible {
                    Some(true) => "show",
                    Some(false) => "hide",
                    None => "toggle",
                };
                format!("bearcad.set_visible({target}, {verb:?})")
            }
            Instruction::SelectSceneElement { element, additive } => {
                let target = element_lua_ref(element);
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
                    element_lua_ref(element),
                    construction
                )
            }
            Instruction::ApplyConstruction { construction } => {
                format!("bearcad.apply_construction({construction})")
            }
            Instruction::ToggleConstruction => "bearcad.toggle_construction()".to_string(),
            Instruction::SetElementName { element, name } => {
                format!(
                    "bearcad.set_name({}, {name:?})",
                    element_lua_ref(element)
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
                let tokens = element_script_tokens(element.clone());
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
                    constraint_point_lua_ref(point)
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
                constraint_line_lua_ref(target)
            ),
            Instruction::VertexTreatment { point, kind, amount } => {
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
                format!(
                    "bearcad.{fname}{{ point = {}, {amount_key} = {amount_lua} }}",
                    constraint_point_lua_ref(point)
                )
            }
            Instruction::EdgeTreatment { extrusion, edge, kind, amount } => {
                let (fname, amount_key) = match kind {
                    VertexTreatmentKind::Chamfer => ("chamfer_edge", "distance"),
                    VertexTreatmentKind::Fillet => ("fillet_edge", "radius"),
                };
                format!(
                    "bearcad.{fname}{{ extrusion = {extrusion}, edge = {}, {amount_key} = {amount} }}",
                    extrusion_edge_lua_ref(*edge)
                )
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
            Instruction::AddParameter { name, expression } => {
                format!("bearcad.parameter(\"add\", {name:?}, {expression:?})")
            }
            Instruction::CreateDerivedParameter { source, name } => {
                use crate::model::ParameterSource as PS;
                let src = match source {
                    PS::LineLength(i) => format!("kind = \"line_length\", a = {i}"),
                    PS::PointDistance(a, b) => format!(
                        "kind = \"point_distance\", a = {{ {} }}, b = {{ {} }}",
                        point_lua_fields(a),
                        point_lua_fields(b)
                    ),
                    PS::LineDistance(a, b) => {
                        format!("kind = \"line_distance\", a = {a}, b = {b}")
                    }
                    PS::LineAngle(a, b) => format!("kind = \"line_angle\", a = {a}, b = {b}"),
                    // Body geometry (#647) is keyed on quantized world points; scripts spell
                    // them as plain **mm** coordinates, which the parser re-quantizes.
                    PS::BodyEdgeLength { body, a, b } => format!(
                        "kind = \"body_edge_length\", body = {body}, a = {}, b = {}",
                        mm_point_lua(*a),
                        mm_point_lua(*b)
                    ),
                    PS::BodyVertexDistance { body_a, a, body_b, b } => format!(
                        "kind = \"body_vertex_distance\", body = {body_a}, a = {}, body_b = {body_b}, b = {}",
                        mm_point_lua(*a),
                        mm_point_lua(*b)
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
            Instruction::RunPaletteCommand { query } => {
                format!("bearcad.ui.palette(\"run\", {query:?})")
            }
            Instruction::Move { x, y } => format!("bearcad.ui.move({x}, {y})"),
            Instruction::Click { x, y } => format!("bearcad.ui.click({x}, {y})"),
            Instruction::MoveGround { x, y } => format!("bearcad.ui.move_ground({x}, {y})"),
            Instruction::ClickGround { x, y } => format!("bearcad.ui.click_ground({x}, {y})"),
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
            Instruction::Key(key) => format!("bearcad.ui.key({:?})", key_name(*key)),
            Instruction::KeyDown(key) => format!("bearcad.ui.keydown({:?})", key_name(*key)),
            Instruction::KeyUp(key) => format!("bearcad.ui.keyup({:?})", key_name(*key)),
            Instruction::Type(text) => format!("bearcad.ui.type({text:?})"),
            Instruction::WaitMs(ms) => format!("bearcad.ui.wait_ms({ms})"),
            Instruction::WaitFrames(n) => format!("bearcad.ui.wait({n})"),
            Instruction::Screenshot { path, whole_window } => {
                if *whole_window {
                    format!("bearcad.ui.screenshot({path:?}, true)")
                } else {
                    format!("bearcad.ui.screenshot({path:?})")
                }
            }
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

fn element_script_tokens(element: SceneElement) -> ElementScriptTokens {
    match element {
        SceneElement::ConstructionPlane(i) => ElementScriptTokens {
            kind: "construction_plane",
            index: i,
            point: None,
        },
        SceneElement::Sketch(i) => ElementScriptTokens {
            kind: "sketch",
            index: i,
            point: None,
        },
        SceneElement::Line(i) => ElementScriptTokens {
            kind: "line",
            index: i,
            point: None,
        },
        SceneElement::Circle(i) => ElementScriptTokens {
            kind: "circle",
            index: i,
            point: None,
        },
        SceneElement::Constraint(i) => ElementScriptTokens {
            kind: "constraint",
            index: i,
            point: None,
        },
        SceneElement::Point(point) => ElementScriptTokens {
            kind: "point",
            index: 0,
            point: Some(point),
        },
        SceneElement::Extrusion(i) => ElementScriptTokens {
            kind: "extrusion",
            index: i,
            point: None,
        },
        SceneElement::Body(i) => ElementScriptTokens {
            kind: "body",
            index: i,
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
        SceneElement::BodyFace { .. } => ElementScriptTokens {
            kind: "body_face",
            index: 0,
            point: None,
        },
        SceneElement::Image(i) => ElementScriptTokens {
            kind: "image",
            index: i,
            point: None,
        },
        SceneElement::BooleanOp(i) => ElementScriptTokens {
            kind: "boolean_op",
            index: i,
            point: None,
        },
        SceneElement::MoveOp(i) => ElementScriptTokens {
            kind: "move_op",
            index: i,
            point: None,
        },
        SceneElement::MirrorOp(i) => ElementScriptTokens {
            kind: "mirror_op",
            index: i,
            point: None,
        },
        SceneElement::RepeatOp(i) => ElementScriptTokens {
            kind: "repeat_op",
            index: i,
            point: None,
        },
        SceneElement::SketchOffsetOp(i) => ElementScriptTokens {
            kind: "sketch_offset_op",
            index: i,
            point: None,
        },
        SceneElement::SketchMirrorOp(i) => ElementScriptTokens {
            kind: "sketch_mirror_op",
            index: i,
            point: None,
        },
        SceneElement::SketchVertexTreatmentOp(i) => ElementScriptTokens {
            kind: "sketch_vertex_treatment_op",
            index: i,
            point: None,
        },
        SceneElement::SketchRepeatOp(i) => ElementScriptTokens {
            kind: "sketch_repeat_op",
            index: i,
            point: None,
        },
        SceneElement::SketchSliceOp(i) => ElementScriptTokens {
            kind: "sketch_slice_op",
            index: i,
            point: None,
        },
        SceneElement::SketchText(i) => ElementScriptTokens {
            kind: "sketch_text",
            index: i,
            point: None,
        },
        SceneElement::SliceOp(i) => ElementScriptTokens {
            kind: "slice_op",
            index: i,
            point: None,
        },
        SceneElement::EdgeTreatmentOp(i) => ElementScriptTokens {
            kind: "edge_treatment_op",
            index: i,
            point: None,
        },
        SceneElement::Revolution(i) => ElementScriptTokens {
            kind: "revolution",
            index: i,
            point: None,
        },
        SceneElement::SweepOp(i) => ElementScriptTokens {
            kind: "sweep",
            index: i,
            point: None,
        },
        SceneElement::Component(i) => ElementScriptTokens {
            kind: "component",
            index: i,
            point: None,
        },
        SceneElement::Origin => ElementScriptTokens {
            kind: "origin",
            index: 0,
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
pub fn instruction_from_action(action: &Action, doc: &crate::model::Document) -> Option<Instruction> {
    use crate::actions::dim_label_axis_for_target;
    match action {
        Action::CreateBooleanOperation { kind, a, b, keep_b } => {
            Some(Instruction::CreateBooleanOp {
                kind: *kind,
                a: a.clone(),
                b: b.clone(),
                keep_b: *keep_b,
            })
        }
        Action::EditBooleanOperation { op, kind, a, b, keep_b } => {
            Some(Instruction::EditBooleanOp {
                op: *op,
                kind: *kind,
                a: a.clone(),
                b: b.clone(),
                keep_b: *keep_b,
            })
        }
        Action::CreateMoveOperation { targets, tx, ty, tz, axis, angle, source_point, target_point, rotation_point, extra_rotations, .. } => {
            Some(Instruction::CreateMoveOp {
                targets: targets.clone(),
                tx: tx.clone(),
                ty: ty.clone(),
                tz: tz.clone(),
                axis: *axis,
                angle: angle.clone(),
                source_point: *source_point,
                target_point: *target_point,
                rotation_point: *rotation_point,
                extra_rotations: extra_rotations.clone(),
            })
        }
        Action::EditMoveOperation { op, targets, tx, ty, tz, axis, angle, source_point, target_point, rotation_point, extra_rotations, .. } => {
            Some(Instruction::EditMoveOp {
                op: *op,
                targets: targets.clone(),
                tx: tx.clone(),
                ty: ty.clone(),
                tz: tz.clone(),
                axis: *axis,
                angle: angle.clone(),
                source_point: *source_point,
                target_point: *target_point,
                rotation_point: *rotation_point,
                extra_rotations: extra_rotations.clone(),
            })
        }
        Action::CreateMirrorOperation { plane, targets, mode } => Some(Instruction::CreateMirrorOp {
            plane: plane.clone(),
            targets: targets.clone(),
            mode: *mode,
        }),
        Action::EditMirrorOperation { op, plane, targets, mode } => Some(Instruction::EditMirrorOp {
            op: *op,
            plane: plane.clone(),
            targets: targets.clone(),
            mode: *mode,
        }),
        // The scripting Instruction DSL doesn't carry plane targets (#221), same as it omits
        // the Move op's plane/image targets — they replay as body-only operations.
        Action::CreateRepeatOperation { targets, plane_targets: _, extrusion_targets: _, sketch_targets: _, axis, mode, count, spacing, length, length_target } => {
            Some(Instruction::CreateRepeatOp {
                targets: targets.clone(),
                axis: *axis,
                mode: *mode,
                count: count.clone(),
                spacing: spacing.clone(),
                length: length.clone(),
                length_target: length_target.clone(),
            })
        }
        Action::EditRepeatOperation { op, targets, plane_targets: _, extrusion_targets: _, sketch_targets: _, axis, mode, count, spacing, length, length_target } => {
            Some(Instruction::EditRepeatOp {
                op: *op,
                targets: targets.clone(),
                axis: *axis,
                mode: *mode,
                count: count.clone(),
                spacing: spacing.clone(),
                length: length.clone(),
                length_target: length_target.clone(),
            })
        }
        Action::CreateSliceOperation { targets, cutters, extend_infinite } => {
            Some(Instruction::CreateSliceOp {
                targets: targets.clone(),
                cutters: cutters.clone(),
                extend_infinite: *extend_infinite,
            })
        }
        Action::EditSliceOperation { op, targets, cutters, extend_infinite } => {
            Some(Instruction::EditSliceOp {
                op: *op,
                targets: targets.clone(),
                cutters: cutters.clone(),
                extend_infinite: *extend_infinite,
            })
        }
        Action::NewDocument => Some(Instruction::New),
        Action::Open { path } => Some(Instruction::Open(path.clone())),
        Action::Save { path } => Some(Instruction::Save(path.clone())),
        Action::ExportStl { path, body } => Some(Instruction::ExportStl {
            path: path.clone(),
            body: body.clone(),
        }),
        Action::ExportStep { path, body } => Some(Instruction::ExportStep {
            path: path.clone(),
            body: body.clone(),
        }),
        Action::ImportStl { path } => Some(Instruction::ImportStl { path: path.clone() }),
        Action::ImportImage { path, plane } => Some(Instruction::ImportImage {
            path: path.clone(),
            plane: *plane,
        }),
        Action::SetCalibrationPoint { image, index, x, y } => {
            Some(Instruction::SetCalibrationPoint {
                image: *image,
                index: *index,
                x: *x,
                y: *y,
            })
        }
        Action::RemoveCalibrationPoint { image, index } => {
            Some(Instruction::RemoveCalibrationPoint { image: *image, index: *index })
        }
        Action::CalibrateImage { image, a, b, length } => Some(Instruction::CalibrateImage {
            image: *image,
            a: *a,
            b: *b,
            length: *length,
        }),
        Action::ImportStep { path } => Some(Instruction::ImportStep { path: path.clone() }),
        Action::UpdateExtrusion { extrusion, distance, target, expression } => {
            Some(Instruction::UpdateExtrusion {
                extrusion: *extrusion,
                distance: *distance,
                target: target.clone(),
                expression: expression.clone(),
            })
        }
        Action::ToggleFpsMode => Some(Instruction::FpsMode { on: None }),
        Action::Clear => Some(Instruction::Clear),
        Action::UndoLast => Some(Instruction::Undo),
        Action::SetTool(tool) => Some(Instruction::Tool(*tool)),
        // The interactive draw tools commit straight to `doc` without going through the
        // declarative Create*/Extrude actions (#59); replay them as the equivalent call
        // using the as-committed geometry. A failed commit (e.g. "too small") returns
        // `ActionResult::Err`, so `after_apply` never reaches here for those.
        // A rectangle is now four plain lines (#66 polygon); reconstruct its origin/extent
        // from the bounding box of the four lines just appended by the commit.
        Action::CommitRectangle => {
            let n = doc.lines.len();
            (n >= 4).then(|| {
                let rect_lines = &doc.lines[n - 4..];
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
                let dim_expr = |line: usize| {
                    doc.constraints.iter().rev().find_map(|c| match &c.kind {
                        crate::model::ConstraintKind::Distance {
                            target: crate::model::DistanceTarget::LineLength(i),
                        } if *i == line && !c.deleted => Some(c.expression.clone()),
                        _ => None,
                    })
                };
                Instruction::CreateRect {
                    x: min_x,
                    y: min_y,
                    width: max_x - min_x,
                    height: max_y - min_y,
                    width_expr: dim_expr(n - 4),
                    height_expr: dim_expr(n - 3),
                }
            })
        }
        Action::CommitLine => doc.lines.last().map(|l| {
            // A typed-while-drawing length lands as a LineLength dim inside CommitLine;
            // carry its expression so replaying the log recreates the same constraint
            // (and click-drawn lines replay unconstrained, as drawn).
            let index = doc.lines.len() - 1;
            let dimension = doc.constraints.iter().rev().find_map(|c| match &c.kind {
                crate::model::ConstraintKind::Distance {
                    target: crate::model::DistanceTarget::LineLength(i),
                } if *i == index && !c.deleted => Some(c.expression.clone()),
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
        Action::CommitCircle => doc.circles.last().map(|c| {
            // Carry a typed diameter's expression like CommitLine does (#402).
            let index = doc.circles.len() - 1;
            let diameter_expr = doc.constraints.iter().rev().find_map(|c| match &c.kind {
                crate::model::ConstraintKind::Distance {
                    target: crate::model::DistanceTarget::CircleDiameter(i),
                } if *i == index && !c.deleted => Some(c.expression.clone()),
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
            Some(Instruction::BeginEditConstructionPlane { index: *index })
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
        Action::OpenSketch { sketch, .. } => Some(Instruction::OpenSketch { sketch: *sketch }),
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
                line_index: *line_index,
                name: name.clone(),
            })
        }
        Action::CommitParameterName { index, name } => Some(Instruction::SetParameterName {
            index: *index,
            name: name.clone(),
        }),
        Action::CommitParameterExpression { index, expression } => {
            Some(Instruction::SetParameterExpression {
                index: *index,
                expression: expression.clone(),
            })
        }
        Action::DeleteParameter { index } => Some(Instruction::DeleteParameter { index: *index }),
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
            parent: *parent,
        }),
        Action::MoveToComponent { element, component } => Some(Instruction::MoveToComponent {
            element: element.clone(),
            component: *component,
        }),
        Action::SetComponentUnits { component, length, angle } => {
            Some(Instruction::SetComponentUnits {
                component: *component,
                length: *length,
                angle: *angle,
            })
        }
        Action::SetSketchUnits { sketch, length, angle } => Some(Instruction::SetSketchUnits {
            sketch: *sketch,
            length: *length,
            angle: *angle,
        }),
        Action::CommitVertexTreatment { point, kind, amount } => {
            Some(Instruction::VertexTreatment {
                point: point.clone(),
                kind: *kind,
                amount: amount.clone(),
            })
        }
        Action::ZoomToFit => Some(Instruction::ZoomFit),
        Action::CommitEdgeTreatment { extrusion, edge, kind, amount } => {
            Some(Instruction::EdgeTreatment {
                extrusion: *extrusion,
                edge: *edge,
                kind: *kind,
                amount: *amount,
            })
        }
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
                line @ (ConstraintLine::FaceEdge { .. } | ConstraintLine::OriginAxis(_)),
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
    let ei = doc.extrusions.len().checked_sub(1)?;
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
        sketch: extrusion.sketch,
        faces: extrusion.faces.clone(),
        distance: extrusion.distance,
        body,
        target: extrusion.target.clone(),
        expression: (!extrusion.expression.trim().is_empty())
            .then(|| extrusion.expression.clone()),
        symmetric: extrusion.symmetric,
    })
}

/// Build a replayable `Instruction::Loft` for the loft the interactive Loft tool just
/// created (the last entry in `doc.lofts`) — `Action::CommitLoft` carries no fields, so
/// like `instruction_for_new_extrusion` the sections come from post-commit state.
pub fn instruction_for_new_loft(doc: &crate::model::Document) -> Option<Instruction> {
    let loft = doc.lofts.last()?;
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
        bodies,
    })
}

/// Replayable `Instruction::Revolve` for the revolution the interactive tool just created
/// (mirrors `instruction_for_new_loft`).
pub fn instruction_for_new_revolution(doc: &crate::model::Document) -> Option<Instruction> {
    let rev = doc.revolutions.last()?;
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
        symmetric: rev.symmetric,
        body,
        bodies,
    })
}

/// Replayable `Instruction::Sweep` for the sweep the interactive tool just created
/// (mirrors `instruction_for_new_revolution`).
pub fn instruction_for_new_sweep(doc: &crate::model::Document) -> Option<Instruction> {
    let fp = doc.sweeps.last()?;
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
        bodies,
    })
}

/// Command-log instructions for a just-committed edge-treatment operation (#531): one
/// `chamfer_edge`/`fillet_edge` per treated edge on the last operation (an interactive
/// multi-edge commit records as several single-edge script calls).
pub fn instructions_for_new_edge_treatment_op(
    doc: &crate::model::Document,
) -> Vec<Instruction> {
    let Some(op) = doc.edge_treatment_ops.last() else {
        return Vec::new();
    };
    op.edges
        .iter()
        .map(|te| Instruction::EdgeTreatment {
            extrusion: te.extrusion,
            edge: te.edge,
            kind: op.kind,
            amount: op.amount,
        })
        .collect()
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
    tx: &str,
    ty: &str,
    tz: &str,
    axis: Option<crate::model::RevolveAxis>,
    angle: &str,
    source_point: &Option<crate::model::MovePointRef>,
    target_point: &Option<crate::model::MovePointRef>,
    rotation_point: &Option<crate::model::MovePointRef>,
    extra_rotations: &[crate::model::MoveRotationSlot; 2],
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!(
        "bodies = {{{}}}",
        targets.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    ));
    // Naming both points makes it a snap translation (#648); the x/y/z components below are
    // then ignored, so they're left out.
    if let (Some(source), Some(target)) = (source_point, target_point) {
        parts.push(format!("from = {}", move_point_lua(source)));
        parts.push(format!("to = {}", move_point_lua(target)));
    }
    // An explicit rotation point (#651); omitted, the rotation follows `from`.
    if let Some(pivot) = rotation_point {
        parts.push(format!("pivot = {}", move_point_lua(pivot)));
    }
    // Free Rotate's other two turns (#652), spelled `axis2`/`angle2` and `axis3`/`angle3`.
    for (i, slot) in extra_rotations.iter().enumerate() {
        if slot.angle.trim().is_empty() {
            continue;
        }
        let n = i + 2;
        if let Some(axis) = slot.axis {
            parts.push(format!("axis{n} = {}", revolve_axis_lua(axis)));
        }
        parts.push(format!("angle{n} = \"{}\"", slot.angle));
    }
    for (name, value) in [("x", tx), ("y", ty), ("z", tz)] {
        if !value.trim().is_empty() {
            parts.push(format!("{name} = \"{value}\""));
        }
    }
    if let Some(axis) = axis {
        parts.push(format!("axis = {}", revolve_axis_lua(axis)));
    }
    if !angle.trim().is_empty() {
        parts.push(format!("angle = \"{angle}\""));
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

/// Render a mirror-operation call (`bearcad.mirror_bodies{}` / `bearcad.edit_mirror{}`, #523).
fn mirror_op_lua(
    call: &str,
    op: Option<usize>,
    plane: &FaceId,
    targets: &[usize],
    mode: crate::model::MirrorMode,
) -> String {
    let mut parts = Vec::new();
    if let Some(op) = op {
        parts.push(format!("index = {op}"));
    }
    parts.push(format!("plane = {}", face_id_lua_ref(plane)));
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
    targets: &[usize],
    axis: crate::model::RevolveAxis,
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
        parts.push(format!("to = {}", extrude_target_lua_table(target)));
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
    targets: &[usize],
    cutters: &[FaceId],
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
        cutters.iter().map(face_id_lua_ref).collect::<Vec<_>>().join(", ")
    ));
    if extend_infinite {
        parts.push("extend = true".to_string());
    }
    format!("{call}{{ {} }}", parts.join(", "))
}

/// Render an extrusion's faces as `bearcad.extrude{}` keyword arguments
/// (`rect=`/`rects=`, `circle=`/`circles=`, `polygon=`). A single rect or circle uses the
/// singular field to match how `bearcad.extrude` is normally called by hand; multiple of a
/// kind use the plural array form. Only the first polygon face is kept — the Lua API has no
/// way to extrude more than one closed-loop face alongside the others in one call.
fn extrude_face_args(faces: &[crate::model::ExtrudeFace]) -> String {
    use crate::model::ExtrudeFace;
    let mut circles = Vec::new();
    let mut polygon = None;
    let mut boolean = None;
    for face in faces {
        match face {
            ExtrudeFace::Circle(i) => circles.push(*i),
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
            // outlines); the script round-trip skips them.
            ExtrudeFace::TextGlyph { .. } => {}
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
        parts.push(format!("polygon = {{{}}}", index_list(lines)));
    }
    if let Some((op, a, b)) = boolean {
        parts.push(format!("boolean = {}", boolean_face_lua_table(op, a, b)));
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
) -> String {
    let op_str = match op {
        crate::model::BooleanOp::Intersection => "intersection",
        crate::model::BooleanOp::Difference => "difference",
    };
    format!(
        "{{op = \"{op_str}\", a = {}, b = {}}}",
        extrude_face_spec_table(a),
        extrude_face_spec_table(b)
    )
}

/// Lua face-spec table for any `ExtrudeFace` (`{rect = i}`, `{circle = i}`,
/// `{polygon = {..}}`, or a nested `{boolean = {...}}`) — the shape
/// `lua_extrude_face_from_table` (src/lua_script.rs) parses back into an `ExtrudeFace`.
fn extrude_face_spec_table(face: &crate::model::ExtrudeFace) -> String {
    use crate::model::ExtrudeFace;
    match face {
        ExtrudeFace::Circle(i) => format!("{{circle = {i}}}"),
        ExtrudeFace::Polygon(lines) => {
            let idx = lines.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
            format!("{{polygon = {{{idx}}}}}")
        }
        ExtrudeFace::Boolean { op, a, b } => {
            format!("{{boolean = {}}}", boolean_face_lua_table(*op, a, b))
        }
        ExtrudeFace::TextGlyph { text, glyph } => {
            format!("{{text_glyph = {{text = {text}, glyph = {glyph}}}}}")
        }
    }
}

/// Render an [`crate::model::ExtrudeTarget`] as the `to = {...}` table
/// `bearcad.extrude`/`bearcad.edit_extrusion` accept (#114).
fn extrude_target_lua_table(target: &crate::model::ExtrudeTarget) -> String {
    use crate::model::ExtrudeTarget;
    match target {
        ExtrudeTarget::Plane(i) => format!("{{ plane = {i} }}"),
        ExtrudeTarget::Face(face) => format!("{{ face = {} }}", extrude_face_spec_table(face)),
        ExtrudeTarget::BodyFace(face_id) => format!("{{ face = {} }}", face_id_lua_ref(face_id)),
        ExtrudeTarget::RepeatedFace { face, op, instance } => format!(
            "{{ face = {}, repeat_op = {op}, instance = {instance} }}",
            face_id_lua_ref(face)
        ),
        ExtrudeTarget::Vertex(point) => {
            format!("{{ vertex = {} }}", constraint_point_lua_ref(point))
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
        Tool::Sweep => "sweep",
        Tool::Combine => "combine",
        Tool::Move => "move",
        Tool::Mirror => "mirror",
        Tool::Repeat => "repeat",
        Tool::Slice => "slice",
        Tool::Text => "text",
        Tool::DrawingAdd => "drawing_add",
        Tool::DrawingAlign => "drawing_align",
    }
}

fn face_lua_parts(face: &FaceId) -> (&'static str, usize) {
    match face {
        FaceId::Circle(i) => ("circle", *i),
        FaceId::ConstructionPlane(i) => ("construction_plane", *i),
        // Cap/side faces aren't yet addressable from the two-argument script form.
        FaceId::ExtrudeCap { extrusion, .. } => ("extrude_cap", *extrusion),
        FaceId::ExtrudeSide { extrusion, .. } => ("extrude_side", *extrusion),
        // A polygon's full line list isn't expressible as a single index; same limitation
        // as cap/side faces above (#66).
        FaceId::Polygon(lines) => ("polygon", *lines.first().unwrap_or(&0)),
        FaceId::RevolveCap { revolution, .. } => ("revolve_cap", *revolution),
        FaceId::RevolveSide { revolution, .. } => ("revolve_side", *revolution),
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

fn element_lua_ref(element: &SceneElement) -> String {
    // #26/#27: a face's own edge, matching `lua_script::parse_element_table`'s
    // `{ kind = "face", face = {...}, index = N, edge = true }` shape.
    if let SceneElement::FaceEdge(line) = element {
        match line {
            ConstraintLine::FaceEdge { face, index } => {
                return format!(
                    "{{ kind = \"face\", face = {}, index = {index}, edge = true }}",
                    face_id_lua_ref(face)
                );
            }
            ConstraintLine::OriginAxis(axis) => {
                return format!("{{ kind = \"axis\", axis = \"{}\" }}", sketch_axis_lua_name(*axis));
            }
            ConstraintLine::Line(index) => {
                return format!("{{ kind = \"line\", index = {index} }}");
            }
        }
    }
    let tokens = element_script_tokens(element.clone());
    if let Some(point) = tokens.point {
        return format!("{{ kind = \"point\", {} }}", point_lua_fields(&point));
    }
    format!("{{ kind = \"{}\", index = {} }}", tokens.kind, tokens.index)
}

fn point_lua_fields(point: &ConstraintPoint) -> String {
    use crate::model::{ConstraintPoint, LineEnd};
    match point {
        ConstraintPoint::LineEndpoint { line, end } => {
            let end_name = match end {
                LineEnd::Start => "start",
                LineEnd::End => "end",
            };
            // `end` is a Lua reserved word, so it can't be a bareword table key; bracket it.
            format!("kind = \"line\", index = {line}, [\"end\"] = \"{end_name}\"")
        }
        ConstraintPoint::CircleCenter(circle) => {
            format!("kind = \"circle\", index = {circle}")
        }
        // #26/#27: mirrors `lua_script::parse_constraint_point_table`'s `"face"` shape.
        ConstraintPoint::FaceVertex { face, index } => {
            format!("kind = \"face\", face = {}, index = {index}", face_id_lua_ref(face))
        }
        // #408: mirrors `lua_script::parse_constraint_point_table`'s `"sketch_text"` shape.
        ConstraintPoint::TextAnchor { text, anchor } => {
            let anchor = anchor.lua_name();
            format!("kind = \"sketch_text\", index = {text}, anchor = \"{anchor}\"")
        }
        // #425: mirrors the `"image"` + `point` shape.
        ConstraintPoint::ImageCalibrationPoint { image, index } => {
            format!("kind = \"image\", index = {image}, point = {index}")
        }
    }
}

fn constraint_line_lua_ref(line: &ConstraintLine) -> String {
    match line {
        ConstraintLine::Line(index) => format!("{{ kind = \"line\", index = {index} }}"),
        // #26/#27: mirrors `lua_script::parse_constraint_line_table`'s `"face"` shape.
        ConstraintLine::FaceEdge { face, index } => format!(
            "{{ kind = \"face\", face = {}, index = {index} }}",
            face_id_lua_ref(face)
        ),
        ConstraintLine::OriginAxis(axis) => {
            format!("{{ kind = \"axis\", axis = \"{}\" }}", sketch_axis_lua_name(*axis))
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

fn constraint_point_lua_ref(point: &ConstraintPoint) -> String {
    format!("{{ {} }}", point_lua_fields(point))
}

/// A scripted move is a **snap** translation exactly when it names both points (#648) — the
/// terse form, so a plain `move_bodies{x = …}` stays a free translation.
pub fn move_translate_mode(
    source: &Option<crate::model::MovePointRef>,
    target: &Option<crate::model::MovePointRef>,
) -> crate::model::MoveTranslateMode {
    if source.is_some() && target.is_some() {
        crate::model::MoveTranslateMode::Snap
    } else {
        crate::model::MoveTranslateMode::Free
    }
}

/// A [`crate::model::MovePointRef`] as the Lua table scripts use (#649/#650): a body plus
/// either a `vertex` in millimetres or the two ends of an `edge` (its midpoint is the point).
pub fn move_point_lua(point: &crate::model::MovePointRef) -> String {
    match point {
        crate::model::MovePointRef::Vertex { body, p } => {
            format!("{{ body = {body}, vertex = {} }}", mm_point_lua(*p))
        }
        crate::model::MovePointRef::EdgeMidpoint { body, a, b } => format!(
            "{{ body = {body}, edge = {{ {}, {} }} }}",
            mm_point_lua(*a),
            mm_point_lua(*b)
        ),
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
        crate::model::RevolveAxis::Line(li) => format!("{{ line = {li} }}"),
        crate::model::RevolveAxis::BodyEdge { body, a, b } => format!(
            "{{ body = {body}, from = {{ {}, {}, {} }}, to = {{ {}, {}, {} }} }}",
            a.x, a.y, a.z, b.x, b.y, b.z
        ),
    }
}

/// Lua table literal for a `FaceId`, matching `lua_script::parse_face_id_table`'s shape.
/// Cap/side profiles are limited to `rect`/`circle` (same limitation as `face_lua_parts` and
/// `parse_face_id_table` — a polygon profile isn't a single index, #66).
fn face_id_lua_ref(face: &FaceId) -> String {
    match face {
        FaceId::Circle(i) => format!("{{ kind = \"circle\", index = {i} }}"),
        FaceId::ConstructionPlane(i) => format!("{{ kind = \"construction_plane\", index = {i} }}"),
        FaceId::Polygon(lines) => format!(
            "{{ kind = \"polygon\", index = {} }}",
            lines.first().copied().unwrap_or(0)
        ),
        FaceId::ExtrudeCap { extrusion, profile, top } => format!(
            "{{ kind = \"extrude_cap\", extrusion = {extrusion}, {}, top = {top} }}",
            extrude_face_profile_lua_fields(profile)
        ),
        FaceId::ExtrudeSide { extrusion, profile, edge } => format!(
            "{{ kind = \"extrude_side\", extrusion = {extrusion}, {}, edge = {edge} }}",
            extrude_face_profile_lua_fields(profile)
        ),
        FaceId::RevolveCap { revolution, profile, end } => format!(
            "{{ kind = \"revolve_cap\", revolution = {revolution}, {}, [\"end\"] = {end} }}",
            extrude_face_profile_lua_fields(profile)
        ),
        FaceId::RevolveSide { revolution, profile, edge } => format!(
            "{{ kind = \"revolve_side\", revolution = {revolution}, {}, edge = {edge} }}",
            extrude_face_profile_lua_fields(profile)
        ),
    }
}

fn extrude_face_profile_lua_fields(profile: &ExtrudeFace) -> String {
    match profile {
        ExtrudeFace::Circle(i) => format!("profile = \"circle\", profile_index = {i}"),
        // Not round-trippable: `parse_face_id_table` only accepts `rect`/`circle` profiles
        // (same limitation as `face_lua_parts`'s polygon case, #66).
        ExtrudeFace::Polygon(lines) => format!(
            "profile = \"polygon\", profile_index = {}",
            lines.first().copied().unwrap_or(0)
        ),
        // Round-trippable since #406: `parse_face_id_table` accepts
        // `profile = "boolean", boolean = {...}`.
        ExtrudeFace::Boolean { op, a, b } => format!(
            "profile = \"boolean\", boolean = {}",
            boolean_face_lua_table(*op, a, b)
        ),
        ExtrudeFace::TextGlyph { text, glyph } => {
            format!("profile = \"text_glyph\", profile_index = {text}, glyph = {glyph}")
        }
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
            format!("{{ kind = \"line\", index = {index} }}")
        }
        DistanceTarget::CircleDiameter(index) => {
            format!("{{ kind = \"circle\", index = {index} }}")
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

    pub fn click(&mut self, viewport: egui::Rect, x: f32, y: f32) {
        let pos = Self::viewport_pos(viewport, x, y);
        self.pointer_pos = Some(pos);
        // Hover one frame, press the next, release the one after — the exact shape
        // tool handlers (press-frame logic, select-then-drag) are written against.
        self.push_event(egui::Event::PointerMoved(pos));
        self.push_event(egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        self.push_event(egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
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

    pub fn key(&mut self, key: Key) {
        self.push_key(key, true);
        self.push_key(key, false);
    }

    pub fn key_down(&mut self, key: Key) {
        self.push_key(key, true);
    }

    pub fn key_up(&mut self, key: Key) {
        self.push_key(key, false);
    }

    fn push_key(&mut self, key: Key, pressed: bool) {
        self.push_event(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
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

/// A pending screenshot request, resolved when egui delivers the captured frame.
struct ScreenshotRequest {
    path: String,
    /// `Some` crops the captured framebuffer to the 3D viewport; `None` keeps the whole window.
    crop: Option<ScreenshotCrop>,
}

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
        }
    }

    #[cfg(test)]
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
        if self.screenshot_pending.is_some() {
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

        if self.screenshot_pending.is_some() {
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

        if self.screenshot_pending.is_some() {
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
        click: bool,
    ) {
        let Some(vp) = viewport else { return };
        let world = Vec3::new(x, y, 0.0);
        let mat = state.cam.view_proj(vp);
        let Some(screen) = state.cam.project(world, vp, &mat) else {
            return;
        };
        let local_x = screen.x - vp.min.x;
        let local_y = screen.y - vp.min.y;
        if click {
            synthetic.click(vp, local_x, local_y);
        } else {
            synthetic.move_to(vp, local_x, local_y);
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
                StepResult::Continue
            }
            Instruction::Open(path) => {
                let r = state.apply(Action::Open { path });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::Save(path) => {
                let r = state.apply(Action::Save { path });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ExportStl { path, body } => {
                let r = state.apply(Action::ExportStl { path, body });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ExportStep { path, body } => {
                let r = state.apply(Action::ExportStep { path, body });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ImportStl { path } => {
                let r = state.apply(Action::ImportStl { path });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ImportImage { path, plane } => {
                let r = state.apply(Action::ImportImage { path, plane });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::SetCalibrationPoint { image, index, x, y } => {
                let result = state.apply(Action::SetCalibrationPoint { image, index, x, y });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::RemoveCalibrationPoint { image, index } => {
                let result = state.apply(Action::RemoveCalibrationPoint { image, index });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CalibrateImage { image, a, b, length } => {
                let r = state.apply(Action::CalibrateImage { image, a, b, length });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::ImportStep { path } => {
                let r = state.apply(Action::ImportStep { path });
                self.record_action_error(r);
                StepResult::Continue
            }
            Instruction::Clear => {
                state.apply(Action::Clear);
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
                });
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
                let result = state.apply(Action::CreateExtrusion {
                    sketch,
                    faces,
                    distance,
                    body,
                    target,
                    expression,
                    symmetric,
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
                if let Some(cl) = state.creating_loft.as_mut() {
                    cl.body_choice = body;
                    cl.cut_bodies = bodies;
                }
                let result = state.apply(Action::CommitLoft);
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingPage { drawing, width_mm, height_mm, margin_mm } => {
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
                let result = state.apply(Action::ExportDrawingSvg { drawing, path });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::ExportDrawingPdf { drawing, path } => {
                let result = state.apply(Action::ExportDrawingPdf { drawing, path });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddDrawingView {
                drawing,
                body,
                orientation,
            } => {
                let result = state.apply(Action::AddDrawingView {
                    drawing,
                    body,
                    orientation,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddDrawingSketchView {
                drawing,
                sketch,
                orientation,
            } => {
                let result = state.apply(Action::AddDrawingSketchView {
                    drawing,
                    sketch,
                    orientation,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::AddDrawingAnnotation { drawing, text, x, y, wrap } => {
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
                let result = state.apply(Action::AddAlignedDrawingView { drawing, parent, dir, pos });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::MoveDrawingView { drawing, view, x, y } => {
                let result = state.apply(Action::MoveDrawingView {
                    drawing,
                    view,
                    pos_x: x,
                    pos_y: y,
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
            Instruction::ToggleDrawingCircleDimension { drawing, view, center } => {
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
                let result =
                    state.apply(Action::SetDrawingViewAlignLines { drawing, view, show });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetDrawingViewLabel { drawing, view, hidden, pos, text } => {
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
            Instruction::Revolve {
                faces,
                axis,
                angle_deg,
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
                let result = state.apply(Action::CreateRevolution {
                    sketch,
                    faces,
                    axis,
                    angle_deg,
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
            Instruction::VertexTreatment { point, kind, amount } => {
                let result = state.apply(Action::CommitVertexTreatment { point, kind, amount });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EdgeTreatment { extrusion, edge, kind, amount } => {
                let result =
                    state.apply(Action::CommitEdgeTreatment { extrusion, edge, kind, amount });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateBooleanOp { kind, a, b, keep_b } => {
                let result = state.apply(Action::CreateBooleanOperation { kind, a, b, keep_b });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditBooleanOp { op, kind, a, b, keep_b } => {
                let result =
                    state.apply(Action::EditBooleanOperation { op, kind, a, b, keep_b });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateMoveOp { targets, tx, ty, tz, axis, angle, source_point, target_point, rotation_point, extra_rotations } => {
                let result = state.apply(Action::CreateMoveOperation {
                    translate_mode: move_translate_mode(&source_point, &target_point),
                    source_point,
                    target_point,
                    // A scripted move states its axes outright, so its rotation is free (#651).
                    rotate_mode: crate::model::MoveRotateMode::Free,
                    rotation_point,
                    extra_rotations,
                    targets,
                    plane_targets: Vec::new(),
                    image_targets: Vec::new(),
                    tx,
                    ty,
                    tz,
                    axis,
                    angle,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditMoveOp { op, targets, tx, ty, tz, axis, angle, source_point, target_point, rotation_point, extra_rotations } => {
                let result = state.apply(Action::EditMoveOperation {
                    op,
                    translate_mode: move_translate_mode(&source_point, &target_point),
                    source_point,
                    target_point,
                    rotate_mode: crate::model::MoveRotateMode::Free,
                    rotation_point,
                    extra_rotations,
                    targets,
                    plane_targets: Vec::new(),
                    image_targets: Vec::new(),
                    tx,
                    ty,
                    tz,
                    axis,
                    angle,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateMirrorOp { plane, targets, mode } => {
                let result = state.apply(Action::CreateMirrorOperation { plane, targets, mode });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditMirrorOp { op, plane, targets, mode } => {
                let result = state.apply(Action::EditMirrorOperation { op, plane, targets, mode });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::CreateRepeatOp { targets, axis, mode, count, spacing, length, length_target } => {
                let result = state.apply(Action::CreateRepeatOperation {
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
            Instruction::EditRepeatOp { op, targets, axis, mode, count, spacing, length, length_target } => {
                let result = state.apply(Action::EditRepeatOperation {
                    op,
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
            Instruction::CreateSliceOp { targets, cutters, extend_infinite } => {
                let result = state.apply(Action::CreateSliceOperation {
                    targets,
                    cutters,
                    extend_infinite,
                });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::EditSliceOp { op, targets, cutters, extend_infinite } => {
                let result = state.apply(Action::EditSliceOperation {
                    op,
                    targets,
                    cutters,
                    extend_infinite,
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
                let result = state.apply(Action::CreateComponent { name, parent });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::MoveToComponent { element, component } => {
                let result = state.apply(Action::MoveToComponent { element, component });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetComponentUnits { component, length, angle } => {
                let result = state.apply(Action::SetComponentUnits { component, length, angle });
                self.record_action_error(result);
                StepResult::Continue
            }
            Instruction::SetSketchUnits { sketch, length, angle } => {
                let _ = state.apply(Action::SetSketchUnits { sketch, length, angle });
                StepResult::Continue
            }
            Instruction::SetAutoZoom { on } => {
                state.auto_zoom = on;
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
            Instruction::EndTutorial => {
                let _ = state.apply(Action::EndTutorial);
                StepResult::Continue
            }
            Instruction::SetDim { axis, value } => {
                let _ = state.apply(Action::SetRectDimension { axis, value });
                StepResult::Continue
            }
            Instruction::SetDimLabelOffset { axis, offset } => {
                if let Some(session) = state.sketch_session {
                    if let Some(target) =
                        dim_label_target_in_sketch(&state.doc, session.sketch, axis)
                    {
                        let _ = state.apply(Action::SetDimLabelOffset { target, offset });
                    }
                }
                StepResult::Continue
            }
            Instruction::BeginEditCommittedDim { axis } => {
                if let Some(session) = state.sketch_session {
                    if let Some(target) =
                        dim_label_target_in_sketch(&state.doc, session.sketch, axis)
                    {
                        let _ = state.apply(Action::BeginEditCommittedDim { target });
                    }
                }
                StepResult::Continue
            }
            Instruction::CommitCommittedDim => {
                let _ = state.apply(Action::CommitCommittedDim);
                StepResult::Continue
            }
            Instruction::AddAngleConstraint {
                line_a,
                line_b,
                rotation_sign,
                expression,
            } => {
                if let Some(session) = state.sketch_session {
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
                    match add_distance_constraint(
                        &mut state.doc,
                        session.sketch,
                        target,
                        expression.clone(),
                    ) {
                        Ok(_) => {
                            state.status = format!("Added dimension ({expression})");
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
                let _ = state.apply(Action::SetLineLength { value });
                StepResult::Continue
            }
            Instruction::SetCircleDiameter { value } => {
                let _ = state.apply(Action::SetCircleDiameter { value });
                StepResult::Continue
            }
            Instruction::BeginEditConstructionPlane { index } => {
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
                if let Some((min, max)) = crate::extrude::document_world_bounds(&state.doc) {
                    if let Some(vp) = viewport {
                        state.viewport_aspect = (vp.width() / vp.height().max(1.0)).max(0.01);
                    }
                    state.cam.frame_bounds_instant(min, max, state.viewport_aspect);
                }
                StepResult::Continue
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
                state.apply(Action::CreateParameterFromLineLength { line_index, name });
                StepResult::Continue
            }
            Instruction::SetParameterName { index, name } => {
                state.apply(Action::CommitParameterName { index, name });
                StepResult::Continue
            }
            Instruction::SetParameterExpression { index, expression } => {
                state.apply(Action::CommitParameterExpression { index, expression });
                StepResult::Continue
            }
            Instruction::DeleteParameter { index } => {
                state.apply(Action::DeleteParameter { index });
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
            Instruction::RunPaletteCommand { query } => {
                let commands = commands_for_state(state);
                if let Some(cmd) = best_match(&query, &commands) {
                    match cmd.outcome() {
                        PaletteOutcome::Action(action) => {
                            state.apply(action);
                        }
                        PaletteOutcome::OpenFile | PaletteOutcome::SaveFile
                        | PaletteOutcome::SaveFileAs
                        | PaletteOutcome::ExportSessionCommands
                        | PaletteOutcome::DocumentJson
                        | PaletteOutcome::OpenExploder
                        | PaletteOutcome::ShowShortcuts => {
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
            Instruction::Click { x, y } => {
                let Some(vp) = viewport else {
                    return StepResult::Wait;
                };
                synthetic.click(vp, x, y);
                StepResult::Continue
            }
            Instruction::MoveGround { x, y } => {
                if viewport.is_none() || state.cam.is_transitioning() {
                    return StepResult::Wait;
                }
                Self::ground_pointer(synthetic, state, viewport, x, y, false);
                StepResult::Continue
            }
            Instruction::ClickGround { x, y } => {
                if viewport.is_none() || state.cam.is_transitioning() {
                    return StepResult::Wait;
                }
                Self::ground_pointer(synthetic, state, viewport, x, y, true);
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
            Instruction::Key(key) => {
                synthetic.key(key);
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
            Instruction::Screenshot { path, whole_window } => {
                let crop = if whole_window {
                    None
                } else {
                    viewport.map(|rect| ScreenshotCrop {
                        rect,
                        pixels_per_point: ctx.pixels_per_point(),
                    })
                };
                self.screenshot_pending = Some(ScreenshotRequest { path, crop });
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                StepResult::Wait
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
}

/// Parsed command-line outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliOutcome {
    Help,
    /// Install the `bearcad` CLI symlink onto PATH (`bearcad install-cli`). See #49.
    InstallCli,
    /// Remove the `bearcad` CLI symlink (`bearcad uninstall-cli`).
    UninstallCli,
    Run(ScriptOptions),
}

/// Print usage information to stdout.
pub fn print_usage() {
    println!(
        "\
BearCAD — parametric CAD prototype

Usage:
  bearcad [options] [script.lua]
  bearcad <command>

Commands:
  install-cli           Symlink this executable onto PATH as `bearcad`
                        (default /usr/local/bin; use sudo if it is not writable)
  uninstall-cli         Remove the `bearcad` PATH symlink

Options:
  --script <path>       Run a Lua script
  --repl                Interactive Lua REPL on stdin against the live app
                        (globals persist between entries; Ctrl-D ends it)
  --exit, --exit-on-complete
                        Exit after startup, or after the script finishes
  --show-commands       Print each user action as a script line on stdout
  --timeout <seconds>   Force-exit with an error if the app hasn't closed on
                        its own within this many seconds
  -h, --help            Show this help and exit

Examples:
  bearcad
  bearcad --exit
  bearcad drawing.bearcad --exit
  bearcad --script demo.lua
  bearcad demo.lua --exit
  bearcad --repl
  bearcad --exit --timeout 30
  bearcad install-cli
"
    );
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
            "--timeout" => {
                i += 1;
                if i < args.len() {
                    opts.timeout_secs = args[i].parse::<u64>().ok();
                }
            }
            arg if !arg.starts_with('-') => {
                if opts.script_path.is_none()
                    && (arg.ends_with(".lua")
                        || Path::new(arg).extension().is_some_and(|e| e == "lua"))
                {
                    opts.script_path = Some(arg.to_string());
                } else if opts.document_path.is_none()
                    && (arg.ends_with(".bearcad")
                        || Path::new(arg).extension().is_some_and(|e| e == "bearcad"))
                {
                    opts.document_path = Some(arg.to_string());
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
        assert_eq!(state.doc.extrusions[0].name.as_deref(), Some("Base"));
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
            .iter()
            .flat_map(|l| [l.x0, l.x1])
            .fold(f32::MIN, f32::max);
        let min_x = state
            .doc
            .lines
            .iter()
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

        let sketch = state.doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
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

        let sketch_inherit = Instruction::SetSketchUnits { sketch: 0, length: None, angle: None };
        assert_eq!(sketch_inherit.as_lua(), "bearcad.set_units{ sketch = 0 }");
    }

    #[test]
    fn parse_key_names() {
        assert_eq!(parse_key("enter").unwrap(), Key::Enter);
        assert_eq!(parse_key("ESC").unwrap(), Key::Escape);
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
    fn parse_show_commands_flag() {
        let opts = parse_args(["bearcad", "--show-commands"]);
        assert!(opts.show_commands);
    }

    #[test]
    fn instruction_from_action_preserves_a_curved_committed_line() {
        let mut doc = crate::model::Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        let mut line = crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        line.bezier = Some([(3.0, 4.0), (7.0, 4.0)]);
        doc.lines.push(line);
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
        let point = ConstraintPoint::LineEndpoint { line: 0, end: crate::model::LineEnd::End };
        let chamfer = Instruction::VertexTreatment {
            point: point.clone(),
            kind: VertexTreatmentKind::Chamfer,
            amount: "3".to_string(),
        };
        assert_eq!(
            chamfer.as_lua(),
            "bearcad.chamfer_vertex{ point = { kind = \"line\", index = 0, [\"end\"] = \"end\" }, distance = 3 }"
        );
        let fillet = Instruction::VertexTreatment {
            point: point.clone(),
            kind: VertexTreatmentKind::Fillet,
            amount: "2.5".to_string(),
        };
        assert_eq!(
            fillet.as_lua(),
            "bearcad.fillet_vertex{ point = { kind = \"line\", index = 0, [\"end\"] = \"end\" }, radius = 2.5 }"
        );
        // A parametric amount records as a quoted string so it survives replay.
        let parametric = Instruction::VertexTreatment {
            point,
            kind: VertexTreatmentKind::Fillet,
            amount: "r".to_string(),
        };
        assert_eq!(
            parametric.as_lua(),
            "bearcad.fillet_vertex{ point = { kind = \"line\", index = 0, [\"end\"] = \"end\" }, radius = \"r\" }"
        );
    }

    #[test]
    fn instruction_from_action_maps_commit_vertex_treatment() {
        let doc = crate::model::Document::default();
        let point = ConstraintPoint::LineEndpoint { line: 2, end: crate::model::LineEnd::Start };
        let action = Action::CommitVertexTreatment {
            point: point.clone(),
            kind: VertexTreatmentKind::Fillet,
            amount: "4".to_string(),
        };
        assert_eq!(
            instruction_from_action(&action, &doc),
            Some(Instruction::VertexTreatment {
                point,
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
            extrusion: 1,
            edge,
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
        };
        assert_eq!(
            chamfer.as_lua(),
            "bearcad.chamfer_edge{ extrusion = 1, edge = { kind = \"vertical\", face = 0, edge = 2 }, distance = 3 }"
        );
        let cap_edge = ExtrusionEdgeRef::Cap { face: 1, edge: 3, top: true };
        let fillet = Instruction::EdgeTreatment {
            extrusion: 0,
            edge: cap_edge,
            kind: VertexTreatmentKind::Fillet,
            amount: 1.5,
        };
        assert_eq!(
            fillet.as_lua(),
            "bearcad.fillet_edge{ extrusion = 0, edge = { kind = \"cap\", face = 1, edge = 3, top = true }, radius = 1.5 }"
        );
    }

    #[test]
    fn instruction_from_action_maps_commit_edge_treatment() {
        use crate::model::ExtrusionEdgeRef;
        let doc = crate::model::Document::default();
        let edge = ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: false };
        let action = Action::CommitEdgeTreatment {
            extrusion: 2,
            edge,
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.5,
        };
        assert_eq!(
            instruction_from_action(&action, &doc),
            Some(Instruction::EdgeTreatment {
                extrusion: 2,
                edge,
                kind: VertexTreatmentKind::Chamfer,
                amount: 2.5,
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
            })
        );
    }

    #[test]
    fn parse_args_finds_timeout_flag() {
        let opts = parse_args(["bearcad", "--exit", "--timeout", "30"]);
        assert_eq!(opts.timeout_secs, Some(30));
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

    #[test]
    fn parse_args_exit_without_paths_exits_after_startup() {
        let opts = parse_args(["bearcad", "--exit"]);
        assert!(opts.exit_on_complete);
        assert!(opts.script_path.is_none());
        assert!(opts.document_path.is_none());
    }

    #[test]
    fn instruction_as_lua_formats_click() {
        let ins = Instruction::Click { x: 100.0, y: 200.0 };
        assert_eq!(ins.as_lua(), "bearcad.ui.click(100, 200)");
    }

    #[test]
    fn script_drag_line_translates_segment() {
        let mut runner = ScriptRunner::from_instructions(vec![
            Instruction::Tool(Tool::Line),
            Instruction::Tool(Tool::Select),
            Instruction::DragLineSegment {
                target: ConstraintLine::Line(0),
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
            face: FaceId::ConstructionPlane(0),
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
        let line = &state.doc.lines[0];
        assert!((line.x0 - 4.0).abs() < 1e-2);
        assert!((line.y0).abs() < 1e-2);
        assert!((line.x1 - 14.0).abs() < 1e-2);
    }

    #[test]
    fn script_palette_run_sets_top_view() {
        let mut runner = ScriptRunner::from_instructions(vec![Instruction::RunPaletteCommand {
            query: "view top".into(),
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
    fn script_delete_selection_tombstones_line() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(crate::model::FaceId::default());
        state.doc.lines.push(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 0.0, 5.0, 0.0,
        ));
        state.doc.shape_order.push(crate::model::ShapeKind::Line);
        let mut runner = ScriptRunner::from_instructions(vec![
            Instruction::SelectSceneElement {
                element: SceneElement::Line(0),
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
        assert!(state.doc.lines[0].deleted);
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
        assert_eq!(state.doc.parameters[0].name, "Len");
        assert_eq!(state.doc.parameters[1].expression, "Len+5in");
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
        assert_eq!(state.doc.parameters[0].expression, "16.7deg");
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
            face: FaceId::ConstructionPlane(0),
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