//! Shared action layer (SPEC §8, §9, §11.2).
//!
//! GUI buttons, keyboard shortcuts, and instruction scripts all dispatch the
//! same [`Action`] values so behaviour stays in sync.

use crate::camera::{
    Camera, ProjectionMode, ShadingMode, StandardView, SKETCH_EDIT_FRAME_PADDING_PX,
    VIEW_TRANSITION_DURATION,
};
use crate::construction::{
    apply_construction_plane_edit, definition_from_reference, plane_from_definition,
    reference_from_definition, resolve_plane, AxisGizmoDrag, PlaneDim, PlaneReference,
};
use crate::model::ConstructionPlaneParent;
use crate::face::{
    sketch_camera_target, sketch_frame, sketch_geometry_frame, sketch_label, sketch_view_up,
    world_to_local,
};
use crate::context::{
    construction_targets_from_selection, set_construction_for_targets, set_edge_construction,
    toggle_construction_for_targets,
};
use crate::hierarchy::SceneElement;
use crate::hierarchy::ElementVisibility;
use crate::names::{element_name, set_element_name, single_nameable_from_selection};
use crate::document_health::{
    health_status_label, recompute_document_health, require_constraint_editable,
    require_dimension_target_editable, require_element_editable,
    require_parameter_editable,
    selection_frozen_summary, DocumentHealth,
};
use crate::document_lifecycle::{delete_targets_from_selection, tombstone_elements};
use crate::selection::{click_scene_selection, SceneSelection};
use crate::model::SketchId;
use crate::view_cube::{self, CubeCornerId, CubeEdgeId};
use crate::constraints::{
    add_distance_constraint, apply_dimension_expression, constraint_expression,
    default_dimension_expression, dimension_edit_from_selection, find_dimension_constraint,
    find_distance_constraint, set_constraint_dim_offset, set_constraint_expression, ConstraintId,
};
use crate::model::{
    independent_corner_handle, smooth_joint_bezier, vertex_treatment_geometry, Circle,
    ConstraintEntity, ConstraintLine, ConstraintKind, ConstructionPlane, ConstraintPoint,
    DimensionTarget, DistanceTarget, Document, EdgeTreatment, ExtrudeFace, Extrusion,
    ExtrusionEdgeRef, FaceId, Line, LineEnd, ShapeKind, VertexTreatmentKind,
};
use crate::vertex_drag;
use crate::face::SketchFrame;
use crate::parameters::{
    add_computed_parameter_from_line_length, add_parameter, delete_parameter,
    recompute_document_geometry, require_parameter_value_editable, set_parameter_expression,
    try_commit_inline_parameter_definition,
    set_parameter_name, ParametersPaneState,
};
use crate::value::{parse_positive_length_or_in_doc, AngleUnit, LengthUnit};
use eframe::egui;
use glam::Vec3;

/// The active viewport tool.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tool {
    /// Orbit/zoom only; no drawing.
    #[default]
    Select,
    /// Click to fix first corner of rectangle; move to position opposite corner;
    /// on-screen number inputs allow typing constraints; Enter commits.
    Rectangle,
    /// Click to fix first endpoint; move mouse for direction and length;
    /// on-screen length input allows typing a constraint; Enter commits.
    Line,
    /// Click to fix center; move mouse for radius; on-screen diameter input allows
    /// typing a constraint; Enter commits.
    Circle,
    /// Click a face or axis/line, then set offset (and angle for axes); Enter commits.
    ConstructionPlane,
    /// Pick a face to enter sketch mode; line/rectangle tools draw on that face.
    Sketch,
    /// Click a line segment to add or edit a distance constraint.
    Dimension,
    /// Select sketch entities and apply geometric constraints from the context pane.
    Constraint,
    /// Click coplanar faces to include them, then set a distance to extrude a solid.
    Extrude,
    /// Click a sketch vertex where exactly two plain lines meet, then set a straight-cut
    /// distance via gizmo/text input to truncate and bridge them (#37). 2D sketch vertices
    /// only — see SPEC §3.1/§3.4 for why there's no 3D solid-edge chamfer in this version.
    Chamfer,
    /// Same vertex-selection flow as [`Tool::Chamfer`], but bridges the truncated lines with a
    /// rounded single-cubic-bezier arc instead of a straight cut (#38).
    Fillet,
    /// Pick two or more closed sketch profiles (circles or line loops) as cross sections,
    /// then Enter blends them into a lofted solid (SPEC §3.5 Loft).
    Loft,
    /// Pick coplanar profile faces plus an axis (a sketch line — construction/projected
    /// fine — or a global axis), set a sweep angle with the gizmo or typed input, and
    /// revolve a solid (SPEC §3.5 Revolve). New body / fuse into touching bodies / cut
    /// picked bodies.
    Revolve,
    /// Boolean operations between whole bodies (Combine tool): union, cut, intersect,
    /// symmetric difference. Inputs become shadow bodies; outputs are new bodies.
    Combine,
    /// Move bodies (translate and/or rotate) into new bodies (Move tool, #176/#183).
    /// Inputs become shadow bodies; outputs are new bodies; the operation is editable.
    Move,
    /// Repeat bodies along an axis (Repeat tool, #182). The original stays; each further
    /// instance is a new body; the operation is editable.
    Repeat,
    /// Slice bodies with planar cutters (Slice tool, #181): each target splits into the
    /// fragments on either side of the cutter planes. Inputs become shadow bodies; the
    /// fragments are new bodies; the operation is editable.
    Slice,
}

impl Tool {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "select" => Some(Tool::Select),
            "rectangle" | "rect" => Some(Tool::Rectangle),
            "line" => Some(Tool::Line),
            "circle" => Some(Tool::Circle),
            "plane" | "construction_plane" | "constructionplane" | "construction plane" => {
                Some(Tool::ConstructionPlane)
            }
            "sketch" => Some(Tool::Sketch),
            "dimension" | "dim" => Some(Tool::Dimension),
            "constraint" | "constraints" => Some(Tool::Constraint),
            "extrude" => Some(Tool::Extrude),
            "chamfer" => Some(Tool::Chamfer),
            "fillet" => Some(Tool::Fillet),
            "loft" => Some(Tool::Loft),
            "revolve" => Some(Tool::Revolve),
            "combine" | "boolean" => Some(Tool::Combine),
            "move" => Some(Tool::Move),
            "repeat" | "linear_repeat" | "pattern" => Some(Tool::Repeat),
            "slice" | "split" => Some(Tool::Slice),
            _ => None,
        }
    }

    pub fn is_sketch_edit_tool(self) -> bool {
        matches!(
            self,
            Tool::Rectangle
                | Tool::Line
                | Tool::Circle
                | Tool::Dimension
                | Tool::Constraint
                | Tool::Chamfer
                | Tool::Fillet
        )
    }
}

/// State for the in-progress (pre-Enter) rectangle creation.
#[derive(Clone, Debug)]
pub struct CreatingRect {
    /// Fixed first corner in ground coords.
    pub origin: Vec3,
    /// Text content of the two dimension inputs (width, height).
    pub texts: [String; 2],
    /// 0 = width (horiz side), 1 = height (vert side)
    pub focused: usize,
    /// Current mouse projected ground point (drives free dimension + signs).
    pub last_mouse: Vec3,
    /// Tracks whether user has typed into each field.
    pub user_edited: [bool; 2],
    /// When true, the focused dimension input should claim keyboard focus.
    pub pending_focus: bool,
    /// New rectangle edges are committed as construction geometry when true.
    pub construction: bool,
}

impl CreatingRect {
    /// Current opposite corner in world space, respecting locked dimensions.
    pub fn end_point(&self, frame: &SketchFrame, doc: &Document) -> Vec3 {
        let (ou, ov) = world_to_local(frame, self.origin);
        let (mu, mv) = world_to_local(frame, self.last_mouse);
        let du = mu - ou;
        let dv = mv - ov;
        let w = if self.user_edited[0] {
            parse_positive_length_or_in_doc(&self.texts[0], doc, du.abs())
        } else {
            du.abs()
        };
        let h = if self.user_edited[1] {
            parse_positive_length_or_in_doc(&self.texts[1], doc, dv.abs())
        } else {
            dv.abs()
        };
        let su = if du < 0.0 { -1.0 } else { 1.0 };
        let sv = if dv < 0.0 { -1.0 } else { 1.0 };
        crate::face::local_to_world(frame, ou + su * w, ov + sv * h)
    }
}

/// State for the in-progress (pre-Enter) line creation.
#[derive(Clone, Debug)]
pub struct CreatingLine {
    /// Fixed first endpoint in ground coords.
    pub origin: Vec3,
    /// Text content of the length input.
    pub text: String,
    /// Current mouse projected ground point (drives free length + direction).
    pub last_mouse: Vec3,
    /// Tracks whether user has typed into the length field.
    pub user_edited: bool,
    /// When true, the length input should claim keyboard focus.
    pub pending_focus: bool,
    /// Committed line is construction geometry when true.
    pub construction: bool,
    /// When true, the vertex this segment starts from (if it has a previous chained
    /// segment) gets bezier handles on both sides — see [`Action::CommitLine`] and #73.
    pub curve_mode: bool,
    /// When curve-mode is on, whether the shared vertex's handles stay mirrored/tangent-
    /// continuous (via [`crate::model::smooth_joint_bezier`]) or are independent "corner"
    /// handles. Ignored when `curve_mode` is false.
    pub tangent_constraint: bool,
    /// Index into `doc.lines` of the previous segment this one chains from (its end is this
    /// segment's start), if any. `None` for the first segment of a fresh chain.
    pub chained_from: Option<usize>,
    /// Snapshot of `chained_from`'s line's `bezier` value taken the moment this segment
    /// started, before any live-preview smoothing touched it. Restored on cancel and used as
    /// the stable "existing far handle" baseline while curving the joint live (#73).
    pub chained_from_bezier: Option<[(f32, f32); 2]>,
}

/// State for the in-progress (pre-Enter) circle creation.
#[derive(Clone, Debug)]
pub struct CreatingCircle {
    /// Fixed center in ground coords.
    pub origin: Vec3,
    /// Text content of the diameter input.
    pub text: String,
    /// Current mouse projected ground point (drives free radius + direction).
    pub last_mouse: Vec3,
    /// Tracks whether user has typed into the diameter field.
    pub user_edited: bool,
    /// When true, the diameter input should claim keyboard focus.
    pub pending_focus: bool,
    /// Committed circle is construction geometry when true.
    pub construction: bool,
}

impl CreatingCircle {
    pub fn radius(&self, frame: &SketchFrame, doc: &Document) -> f32 {
        let (cu, cv) = world_to_local(frame, self.origin);
        let (mu, mv) = world_to_local(frame, self.last_mouse);
        let du = mu - cu;
        let dv = mv - cv;
        let dist = (du * du + dv * dv).sqrt();
        if self.user_edited {
            parse_positive_length_or_in_doc(&self.text, doc, dist * 2.0) / 2.0
        } else {
            dist
        }
    }

    pub fn diameter_dim_angle(&self, frame: &SketchFrame) -> f32 {
        let (cu, cv) = world_to_local(frame, self.origin);
        let (mu, mv) = world_to_local(frame, self.last_mouse);
        let du = mu - cu;
        let dv = mv - cv;
        if du * du + dv * dv < 1e-12 {
            0.0
        } else {
            dv.atan2(du)
        }
    }

    /// Point on the circle rim in world space, respecting any locked diameter.
    pub fn rim_point(&self, frame: &SketchFrame, doc: &Document) -> Vec3 {
        let r = self.radius(frame, doc);
        let angle = self.diameter_dim_angle(frame);
        let (cu, cv) = world_to_local(frame, self.origin);
        crate::face::local_to_world(
            frame,
            cu + angle.cos() * r,
            cv + angle.sin() * r,
        )
    }
}

/// Whether a committed extrusion creates a new body row, merges into (adds to) an existing
/// one, or is subtracted (cut) from one (#35). `Cut` is only *offered* in the GUI when the
/// OCCT kernel is present (a non-kernel build can't perform the subtraction), but the variant
/// and its attach logic exist in every build so documents round-trip regardless.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtrudeBodyMode {
    NewBody,
    MergeInto(usize),
    Cut(usize),
}

/// How a scripted / [`Action::CreateExtrusion`] extrude attaches to bodies, resolved against
/// the extrusion's merge candidate at commit time (#35). Mirrors the Lua `body =` argument:
/// omitted / `"new"` → [`New`](Self::New), `"merge"` → [`Merge`](Self::Merge),
/// `"cut"` → [`Cut`](Self::Cut). When there's no candidate body, `Merge`/`Cut` are a hard
/// error (#178) — the sketch must sit on a body face — rather than silently degrading to a
/// standalone new body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExtrudeBodyChoice {
    #[default]
    New,
    Merge,
    Cut,
}

/// In-progress (or being-edited) extrusion: selected faces + live signed distance.
#[derive(Clone, Debug)]
pub struct CreatingExtrusion {
    /// Sketch plane the faces lie on (all faces are coplanar).
    pub sketch: SketchId,
    pub faces: Vec<ExtrudeFace>,
    /// Live signed distance along the plane normal (gizmo-driven).
    pub distance: f32,
    /// Distance input text (magnitude); the sign follows `distance`.
    pub text: String,
    pub user_edited: bool,
    pub pending_focus: bool,
    /// When set, the depth is constrained to this object's extended plane.
    pub target: Option<crate::model::ExtrudeTarget>,
    /// `Some` when editing an existing extrusion rather than creating one.
    pub edit_index: Option<usize>,
    /// How this extrusion should attach to the document's bodies on commit.
    pub body_mode: ExtrudeBodyMode,
    /// Body that `body_mode` can merge into (the other option always being `NewBody`); `None`
    /// when there's no candidate body, in which case the context pane hides the choice.
    pub merge_candidate: Option<usize>,
}

impl CreatingExtrusion {
    /// Evaluated signed distance: typed magnitude (if edited) keeps the live sign.
    pub fn evaluated_distance(&self, doc: &Document) -> f32 {
        if self.user_edited {
            let magnitude = parse_positive_length_or_in_doc(&self.text, doc, self.distance.abs());
            let sign = if self.distance < 0.0 { -1.0 } else { 1.0 };
            magnitude * sign
        } else {
            self.distance
        }
    }
}

/// Default gizmo-driven chamfer distance / fillet radius when starting a new vertex treatment.
pub const DEFAULT_VERTEX_TREATMENT_AMOUNT: f32 = 2.0;

/// In-progress (pre-commit) chamfer/fillet: the vertex picked, which kind, and the live
/// gizmo-driven amount (chamfer distance or fillet radius). Mirrors [`CreatingExtrusion`]'s
/// shape closely — same click-to-grab gizmo drag and floating text-input pattern.
#[derive(Clone, Debug)]
pub struct CreatingVertexTreatment {
    pub point: ConstraintPoint,
    pub kind: VertexTreatmentKind,
    /// Live amount (mm), gizmo-driven; always clamped non-negative.
    pub amount_live: f32,
    /// Amount input text; the sign is always positive (chamfer/fillet can't go negative).
    pub text: String,
    pub user_edited: bool,
    pub pending_focus: bool,
}

impl CreatingVertexTreatment {
    /// Evaluated amount: typed magnitude (if edited), otherwise the live gizmo-driven value.
    /// Always non-negative.
    pub fn evaluated_amount(&self, doc: &Document) -> f32 {
        if self.user_edited {
            parse_positive_length_or_in_doc(&self.text, doc, self.amount_live.max(0.0))
        } else {
            self.amount_live.max(0.0)
        }
    }
}

/// In-progress (pre-commit) 3D solid-edge chamfer/fillet (#77): the extrusion + analytic edge
/// picked, which kind, and the live gizmo-driven amount. The 3D analogue of
/// [`CreatingVertexTreatment`] — kept as a parallel, separate state rather than folded into it,
/// since resolving the target/geometry is entirely different (an extrusion's own analytic
/// side/cap edge, via `ExtrusionEdgeRef`, not a sketch vertex).
/// State for an in-progress image scale calibration (#163/#171): after "Calibrate
/// scale" is clicked with a tracing image selected, the user places two reference points
/// on the image (plane-local mm) over a feature of known size, then types that real
/// length in the context pane to rescale the image.
#[derive(Clone, Debug)]
pub struct CreatingCalibration {
    pub image: usize,
    /// Placed reference points in host-plane-local mm (0..=2).
    pub points: Vec<(f32, f32)>,
}

/// Where a committed revolve lands (#revolve): its own body, fused into whatever bodies
/// it touches (resolved at commit), or cut from an explicitly picked body set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RevolveBodyChoice {
    #[default]
    NewBody,
    AddTouching,
    Cut,
}

/// State for the in-progress (pre-Enter) revolve: picked profile faces, the axis, the
/// live sweep angle (degrees; the text field also accepts `rad` expressions), and how
/// the result lands.
/// In-progress boolean operation (Combine tool): the op kind, the picked input bodies
/// per side, which side the next viewport body click lands on, and the keep-B toggle.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatingBoolean {
    pub kind: crate::model::BooleanOpKind,
    pub a: Vec<usize>,
    pub b: Vec<usize>,
    /// Which picker the next body click adds to (`Combine` only ever uses A).
    pub picking_b: bool,
    pub keep_b: bool,
    /// `Some(op)` while re-editing a committed operation (commit then updates in place).
    pub editing: Option<usize>,
}

impl Default for CreatingBoolean {
    fn default() -> Self {
        Self {
            kind: crate::model::BooleanOpKind::Combine,
            a: Vec::new(),
            b: Vec::new(),
            picking_b: false,
            keep_b: false,
            editing: None,
        }
    }
}

/// In-progress linear repeat (Repeat tool): the picked bodies, axis, spacing mode, and
/// the count/spacing/length expressions.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatingRepeat {
    pub targets: Vec<usize>,
    /// Picked source construction planes to repeat as offset copies (#221).
    pub plane_targets: Vec<usize>,
    pub axis: crate::model::RevolveAxis,
    pub mode: crate::model::RepeatMode,
    pub count: String,
    pub spacing: String,
    pub length: String,
    /// `Some(op)` while re-editing a committed operation.
    pub editing: Option<usize>,
}

impl Default for CreatingRepeat {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            plane_targets: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
            editing: None,
        }
    }
}

/// In-progress slice operation (Slice tool): the picked target bodies (A), planar cutters
/// (B), the extend-to-infinity toggle, and which picker the next click lands on.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatingSlice {
    pub targets: Vec<usize>,
    pub cutters: Vec<FaceId>,
    /// Which picker the next viewport click adds to: `false` = a target body, `true` = a
    /// cutter face/plane.
    pub picking_cutter: bool,
    pub extend_infinite: bool,
    /// `Some(op)` while re-editing a committed operation.
    pub editing: Option<usize>,
}

impl Default for CreatingSlice {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            cutters: Vec::new(),
            picking_cutter: false,
            extend_infinite: true,
            editing: None,
        }
    }
}

/// In-progress move operation (Move tool): the picked bodies, translation component
/// expressions, optional rotation axis + angle expression, and the op being re-edited.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CreatingMove {
    pub targets: Vec<usize>,
    /// Construction planes being moved (#217).
    pub plane_targets: Vec<usize>,
    /// Tracing images being moved (#217).
    pub image_targets: Vec<usize>,
    pub tx: String,
    pub ty: String,
    pub tz: String,
    pub axis: Option<crate::model::RevolveAxis>,
    pub angle: String,
    /// `Some(op)` while re-editing a committed operation.
    pub editing: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct CreatingRevolve {
    pub sketch: Option<SketchId>,
    pub faces: Vec<ExtrudeFace>,
    pub axis: Option<crate::model::RevolveAxis>,
    /// Live sweep angle in degrees (gizmo-driven when the text hasn't been edited).
    pub angle_live: f32,
    pub text: String,
    pub user_edited: bool,
    pub pending_focus: bool,
    pub symmetric: bool,
    pub body_choice: RevolveBodyChoice,
    /// Bodies picked for Cut mode.
    pub cut_bodies: Vec<usize>,
    /// `Some(op)` while re-editing a committed revolution (#211), else a fresh revolve.
    pub editing: Option<usize>,
}

impl Default for CreatingRevolve {
    fn default() -> Self {
        Self {
            sketch: None,
            faces: Vec::new(),
            axis: None,
            angle_live: 360.0,
            text: "360".to_string(),
            user_edited: false,
            pending_focus: false,
            symmetric: false,
            body_choice: RevolveBodyChoice::default(),
            cut_bodies: Vec::new(),
            editing: None,
        }
    }
}

impl CreatingRevolve {
    /// The effective sweep angle in degrees: the typed expression when edited (bare
    /// numbers are degrees; `rad`/`deg` suffixes and parameters work), else the live
    /// gizmo angle.
    pub fn evaluated_angle_deg(&self, doc: &Document) -> f32 {
        if self.user_edited {
            crate::value::eval_angle_rad_in_doc(&self.text, doc)
                .map(|r| r.to_degrees())
                .unwrap_or(self.angle_live)
        } else {
            self.angle_live
        }
    }
}

/// State for the in-progress (pre-Enter) loft: the cross sections picked so far.
/// Committing (Enter, >= 2 sections) creates a [`crate::model::Loft`] plus its body.
#[derive(Clone, Debug, Default)]
pub struct CreatingLoft {
    pub sections: Vec<crate::model::LoftSection>,
}

#[derive(Clone, Debug)]
pub struct CreatingEdgeTreatment {
    /// The analytic edges being treated together (#166): one shared amount/gizmo applies to
    /// all of them on commit. Non-empty; the first entry anchors the gizmo.
    pub edges: Vec<(usize, ExtrusionEdgeRef)>,
    pub kind: VertexTreatmentKind,
    /// Live amount (mm), gizmo-driven; always clamped non-negative.
    pub amount_live: f32,
    /// Amount input text; the sign is always positive (chamfer/fillet can't go negative).
    pub text: String,
    pub user_edited: bool,
    pub pending_focus: bool,
}

impl CreatingEdgeTreatment {
    /// The gizmo-anchoring edge (the first in the set).
    pub fn primary(&self) -> Option<(usize, ExtrusionEdgeRef)> {
        self.edges.first().copied()
    }

    /// Toggle an edge's membership in the set (#166; shift+click). Removing the last edge
    /// is refused — an in-progress treatment always keeps at least one edge.
    pub fn toggle_edge(&mut self, entry: (usize, ExtrusionEdgeRef)) {
        if let Some(pos) = self.edges.iter().position(|e| *e == entry) {
            if self.edges.len() > 1 {
                self.edges.remove(pos);
            }
        } else {
            self.edges.push(entry);
        }
    }

    /// Evaluated amount: typed magnitude (if edited), otherwise the live gizmo-driven value.
    /// Always non-negative.
    pub fn evaluated_amount(&self, doc: &Document) -> f32 {
        if self.user_edited {
            parse_positive_length_or_in_doc(&self.text, doc, self.amount_live.max(0.0))
        } else {
            self.amount_live.max(0.0)
        }
    }
}

impl CreatingLine {
    /// Current second endpoint in world space, respecting any locked length.
    pub fn end_point(&self, frame: &SketchFrame, doc: &Document) -> Vec3 {
        let (ou, ov) = world_to_local(frame, self.origin);
        let (mu, mv) = world_to_local(frame, self.last_mouse);
        let du = mu - ou;
        let dv = mv - ov;
        let dist = (du * du + dv * dv).sqrt();
        let len = if self.user_edited {
            parse_positive_length_or_in_doc(&self.text, doc, dist)
        } else {
            dist
        };
        if dist < 1e-6 {
            return crate::face::local_to_world(frame, ou + len, ov);
        }
        let scale = len / dist;
        crate::face::local_to_world(frame, ou + du * scale, ov + dv * scale)
    }
}

/// State for creating or editing a construction plane.
#[derive(Clone, Debug)]
pub struct CreatingConstructionPlane {
    /// When set, commit updates this plane instead of adding a new one.
    pub edit_index: Option<usize>,
    pub reference: PlaneReference,
    pub parent: ConstructionPlaneParent,
    pub offset_text: String,
    pub angle_text: String,
    pub focused: PlaneDim,
    /// Live offset (mm); updated by gizmo drag or wheel.
    pub offset_live: f32,
    /// Live angle for axis references (degrees); updated by gizmo drag.
    pub axis_angle_deg: f32,
    pub user_edited_offset: bool,
    pub user_edited_angle: bool,
    pub pending_focus: bool,
    pub axis_gizmo_drag: Option<AxisGizmoDrag>,
}

impl CreatingConstructionPlane {
    pub fn preview_plane(&self) -> ConstructionPlane {
        let (live_offset, live_angle) = self.live_dims();
        resolve_plane(
            &self.reference,
            &self.offset_text,
            &self.angle_text,
            live_offset,
            live_angle,
            self.user_edited_offset,
            self.user_edited_angle,
        )
    }

    pub fn resolved_definition(&self) -> crate::model::PlaneDefinition {
        let (live_offset, live_angle) = self.live_dims();
        let offset = if self.user_edited_offset {
            crate::value::parse_length_or(&self.offset_text, live_offset)
        } else {
            live_offset
        };
        let angle = if self.user_edited_angle {
            self.angle_text
                .trim()
                .parse::<f32>()
                .unwrap_or(live_angle)
                .rem_euclid(360.0)
        } else {
            live_angle
        };
        definition_from_reference(&self.reference, offset, angle)
    }

    pub fn live_dims(&self) -> (f32, f32) {
        match &self.reference {
            PlaneReference::Face { .. } => (self.offset_live, 0.0),
            PlaneReference::Axis { .. } => (self.offset_live, self.axis_angle_deg),
        }
    }
}

/// Every user-visible operation the app supports.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    NewDocument,
    Open { path: String },
    Save { path: Option<String> },
    /// Export bodies to an STL file. `body` names a single body; `None` exports all bodies.
    ExportStl { path: String, body: Option<String> },
    /// Export a technical drawing (#180) to a vector SVG file (prints to PDF).
    ExportDrawingSvg { drawing: usize, path: String },
    ExportDrawingPdf { drawing: usize, path: String },
    /// Export a single body (by index) to an STL file — used by the body row's context menu,
    /// which has the index in hand and works for unnamed bodies too.
    ExportStlBody { path: String, body: usize },
    /// Export bodies to a STEP file. `body` names a single body; `None` exports all bodies.
    ExportStep { path: String, body: Option<String> },
    /// Export a single body (by index) to a STEP file — used by the body row's context menu.
    ExportStepBody { path: String, body: usize },
    /// Import an STL file (ASCII or binary, #70) as a new body.
    ImportStl { path: String },
    /// Import a PNG/JPEG as a tracing image (#163/#169) on a construction plane (defaults
    /// to plane 0). Seeds 1 px = 1 mm, centered on the plane origin.
    ImportImage { path: String, plane: Option<usize> },
    /// Calibrate a tracing image's scale (#171): the plane-local segment `a`-`b` (drawn over
    /// a known feature) is assigned the real `length`; the image rescales uniformly about
    /// the segment midpoint so that span measures `length`.
    CalibrateImage {
        image: usize,
        a: (f32, f32),
        b: (f32, f32),
        length: f32,
    },
    /// Start the guided image calibration (#163): the user will click two points on the
    /// image over a feature of known size, then type its real length.
    BeginImageCalibration { image: usize },
    /// Place one calibration reference point (host-plane-local mm).
    AddCalibrationPoint { x: f32, y: f32 },
    /// Import a STEP file's `FACETED_BREP` geometry (#71) as a new body.
    ImportStep { path: String },
    Clear,
    UndoLast,
    SetTool(Tool),
    /// Enter/leave first-person mode (#91): WASD walking on the ground plane, mouse
    /// look, Space jump / double-tap fly, weapon-style tool slots. See [`crate::fps`].
    ToggleFpsMode,
    CancelOperation,
    CommitRectangle,
    SetRectDimension { axis: RectAxis, value: String },
    FocusRectDimension { axis: RectAxis },
    CommitLine,
    SetLineLength { value: String },
    FocusLineLength,
    CommitCircle,
    SetCircleDiameter { value: String },
    FocusCircleDiameter,
    SetDimLabelOffset {
        target: DimLabelTarget,
        offset: f32,
    },
    SetConstraintAngleValue {
        constraint_id: ConstraintId,
        angle_rad: f32,
    },
    BeginEditCommittedDim { target: DimLabelTarget },
    BeginDimensionEdit { target: DimensionTarget },
    CommitCommittedDim,
    BeginConstructionPlane {
        reference: PlaneReference,
        parent: ConstructionPlaneParent,
    },
    BeginEditConstructionPlane {
        index: usize,
    },
    CommitConstructionPlane,
    /// Declaratively add a new construction plane offset from an existing one, without the
    /// interactive begin/set-dim/commit flow (#116): the scripted equivalent of picking
    /// plane `from` in the viewport and typing `offset_mm`.
    AddConstructionPlane {
        from: usize,
        offset_mm: f32,
    },
    SetPlaneOffset { value: String },
    SetPlaneAngle { value: String },
    FocusPlaneDim { dim: PlaneDim },
    BeginSketch {
        face: FaceId,
        viewport: Option<egui::Rect>,
    },
    OpenSketch {
        sketch: SketchId,
        viewport: Option<egui::Rect>,
    },
    ExitSketch,
    SetElementVisible {
        element: SceneElement,
        visible: bool,
    },
    ToggleElementVisibility(SceneElement),
    OrbitCamera { delta: (f32, f32) },
    PanCamera { delta: (f32, f32), viewport_height: f32 },
    ZoomCamera {
        scroll: f32,
        focal: egui::Pos2,
        viewport: egui::Rect,
    },
    SetStandardView(StandardView),
    SetViewEdge(CubeEdgeId),
    SetViewCorner(CubeCornerId),
    ViewHome,
    SetHomeView,
    SetProjectionMode(ProjectionMode),
    ToggleProjectionMode,
    SetShadingMode(ShadingMode),
    /// Frame the current selection in the viewport, or the whole document (non-construction
    /// geometry) when nothing is selected (#164).
    ZoomToFit,
    /// Project the selected body edges (or whole bodies/extrusions) into the open sketch as
    /// associative construction-style lines (#140; the `Y` shortcut).
    ProjectSelection,
    /// Choose how the ground plane renders (#159; gear menu).
    SetGroundDisplay(crate::camera::GroundDisplay),
    /// Switch the Elements pane's layout (List/Tree/Graph, #34/#108).
    SetElementsViewMode { mode: crate::hierarchy::HierarchyViewMode },
    SetPaneVisible { pane: Pane, visible: bool },
    TogglePane(Pane),
    AddParameter { name: String, expression: String },
    /// Create a read-only parameter synced to an unconstrained line's length.
    CreateParameterFromLineLength { line_index: usize, name: Option<String> },
    CommitParameterName { index: usize, name: String },
    CommitParameterExpression { index: usize, expression: String },
    DeleteParameter { index: usize },
    /// Tombstone every element in the current scene selection.
    DeleteSelection,
    SetCommandPaletteOpen { open: bool },
    ToggleCommandPalette,
    ClickSceneElement {
        element: SceneElement,
        additive: bool,
    },
    ClearSceneSelection,
    SetShapeConstruction {
        element: SceneElement,
        construction: bool,
    },
    /// Set construction/substantial on the active draw op or all constructable selected targets.
    ApplyConstruction {
        construction: bool,
    },
    /// Toggle construction/substantial on the active draw op or each constructable selected target.
    ToggleConstruction,
    /// Set curve-mode (`B`) on the active line draw op, or the persisted default for the line
    /// tool (#73).
    ApplyCurveMode { curve_mode: bool },
    /// Toggle curve-mode (`B`): on the active line draw op / persisted line-tool default, or —
    /// in Select tool with sketch vertices selected — retroactively on each selected vertex
    /// (curves it if straight, straightens it if curved; see [`Action::ConvertVertexToBezier`]
    /// / [`Action::StraightenLine`]).
    ToggleCurveMode,
    /// Set the tangent-constraint toggle (`T`) on the active line draw op, or the persisted
    /// default for the line tool (#73).
    ApplyTangentConstraint { tangent_constraint: bool },
    /// Toggle the tangent-constraint (`T`): on the active line draw op / persisted line-tool
    /// default, or — in Select tool with sketch vertices selected — retroactively re-smooth
    /// vs. break tangency at each selected vertex (see [`Action::SetVertexTangent`]).
    ToggleTangentConstraint,
    CommitElementName {
        element: SceneElement,
        name: String,
    },
    FocusElementName,
    /// Set the document-wide default length/angle units (context pane, nothing selected; #52).
    /// Storage/display only for now — see [`crate::model::Document::default_length_unit`].
    SetDocumentUnits {
        length: LengthUnit,
        angle: AngleUnit,
    },
    /// Set (or clear, via `None`) a per-sketch length/angle unit override (context pane, sketch
    /// selected; #52). `None` means "follow the document default".
    SetSketchUnits {
        sketch: SketchId,
        length: Option<LengthUnit>,
        angle: Option<AngleUnit>,
    },
    /// Apply a geometric constraint type to the current selection (constraint tool).
    AddGeometricConstraint(crate::geometric_constraints::GeometricConstraintType),
    /// Apply the enabled constraint matching its mnemonic shortcut key (A/T/I/M/V/H).
    ApplyConstraintShortcut(char),
    /// Move a sketch vertex to local `(u, v)` while satisfying constraints.
    DragVertex {
        point: ConstraintPoint,
        u: f32,
        v: f32,
    },
    /// Start dragging a line or rectangle edge from an anchor in sketch-local coords.
    BeginLineDrag {
        target: crate::model::ConstraintLine,
        anchor_u: f32,
        anchor_v: f32,
    },
    /// Continue dragging the active line segment to sketch-local `(u, v)`.
    DragLine { u: f32, v: f32 },
    /// Finish an interactive line drag.
    EndLineDrag,
    /// Move a curved line's tangent handle (`near_start` selects the one near `(x0,y0)` vs.
    /// `(x1,y1)`) to sketch-local `(u, v)`. No-op-turned-error on a straight line.
    SetBezierHandle {
        line: usize,
        near_start: bool,
        u: f32,
        v: f32,
    },
    /// Right-click "convert to bezier curve": smooths the joint at `point` into a matched pair
    /// of tangent-continuous curves. Errors unless exactly two plain lines meet there.
    ConvertVertexToBezier { point: ConstraintPoint },
    /// Right-click "straighten curve": clears a curved line's tangent handles.
    StraightenLine { line: usize },
    /// Retroactive `T` shortcut on a selected sketch vertex (#73): when `continuous`, re-smooths
    /// both incident lines' handles at `point` via [`crate::model::smooth_joint_bezier`]
    /// (same computation as [`Action::ConvertVertexToBezier`]); when not, gives each line an
    /// independent "corner" handle at the vertex instead. Errors unless exactly two plain lines
    /// meet there.
    SetVertexTangent {
        point: ConstraintPoint,
        continuous: bool,
    },
    /// Chamfer or fillet a sketch vertex where exactly two plain lines meet (#37/#38):
    /// truncates both lines back from the vertex and bridges them with a new `Line` (straight
    /// for a chamfer, single-cubic-bezier arc for a fillet — see
    /// [`crate::model::vertex_treatment_geometry`]). `amount` is the chamfer distance or fillet
    /// radius depending on `kind`. Atomic and declarative: usable directly from Lua as well as
    /// from the interactive gizmo tool.
    CommitVertexTreatment {
        point: ConstraintPoint,
        kind: VertexTreatmentKind,
        amount: f32,
    },
    /// Chamfer or fillet an analytic edge of an extrusion's 3D solid (#77): a mesh-bevel
    /// approximation (flat bevel quad for a chamfer, an N-segment faceted bevel for a fillet —
    /// see `crate::extrude::corner_bevel_3d`/`extrude_profile_with_treatments`), scoped to the
    /// vertical side and side/cap edges of a `Rect`/`Polygon`-profiled extrusion (SPEC §3.4).
    /// Stores (or updates, if `edge` is already treated) an `EdgeTreatment` on the extrusion —
    /// parametric, re-evaluated every frame like everything else in this app, not a baked mesh
    /// edit. Rejects an edge that would share a corner with another already-treated edge on the
    /// same face (a vertex miter — this mesh-bevel approximation doesn't attempt to blend
    /// three-or-more bevels together). Atomic and declarative: usable directly from Lua
    /// (`bearcad.chamfer_edge`/`fillet_edge`) as well as from the interactive gizmo tool.
    /// Apply one chamfer/fillet amount to a whole set of edges as a single undo group
    /// (#166); each entry commits via [`Action::CommitEdgeTreatment`] internally.
    CommitEdgeTreatments {
        edges: Vec<(usize, ExtrusionEdgeRef)>,
        kind: VertexTreatmentKind,
        amount: f32,
    },
    CommitEdgeTreatment {
        extrusion: usize,
        edge: ExtrusionEdgeRef,
        kind: VertexTreatmentKind,
        amount: f32,
    },
    /// Create a rectangle directly in the active sketch (face-local mm) with locked dimensions.
    CreateRectangle {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    /// Create a line directly in the active sketch (face-local mm). Unconstrained like a
    /// click-drawn line ([`Action::CommitLine`] without a typed length); `dimension` locks
    /// its length with the given expression, like typing a length while drawing does.
    /// `bezier` (#54) makes it a curve: `[handle near (x0,y0), handle near (x1,y1)]`.
    CreateLineSegment {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        bezier: Option<[(f32, f32); 2]>,
        dimension: Option<String>,
    },
    /// Create a circle directly in the active sketch (face-local mm) with a locked diameter.
    CreateCircle {
        cx: f32,
        cy: f32,
        r: f32,
    },
    /// Create an extrusion solid from coplanar sketch faces.
    CreateExtrusion {
        sketch: SketchId,
        faces: Vec<ExtrudeFace>,
        distance: f32,
        /// How the extrusion attaches to bodies (#32/#35) — mirrors the context pane's
        /// New / Add-to-body / Cut choice for the GUI flow.
        body: ExtrudeBodyChoice,
        /// Extrude up to this object's extended plane instead of a fixed distance —
        /// the scripted equivalent of pulling the gizmo and snapping to a surface
        /// (#114). `distance` becomes the cached/fallback value.
        target: Option<crate::model::ExtrudeTarget>,
    },
    /// Semantic push/pull of an existing extrusion (#114): set a new fixed distance
    /// (clears any snap target — a plain typed distance is a blind extrude) and/or
    /// snap to a new target, re-evaluating the parametric geometry.
    UpdateExtrusion {
        extrusion: usize,
        distance: Option<f32>,
        target: Option<crate::model::ExtrudeTarget>,
    },
    /// Add/remove a face from the in-progress extrusion (starts one if needed).
    ToggleExtrudeFace { face: ExtrudeFace },
    /// Extrude a bare 3D body face directly, no separate sketch (#122): creates an implicit
    /// sketch mirroring `face_id`'s exact boundary and starts a fresh single-face extrusion
    /// from it (a body face is never grouped with other faces into one multi-face extrusion).
    ExtrudeBodyFace { face_id: FaceId },
    /// Scripted push/pull of a bare body face committed in one step (#130): builds the
    /// implicit sketch mirroring `face_id`, then creates the extrusion with `distance`,
    /// optional snap `target`, and body attachment — the declarative equivalent of clicking
    /// the face with the Extrude tool and pulling it onto another face.
    CreateBodyFaceExtrusion {
        face_id: FaceId,
        distance: f32,
        target: Option<crate::model::ExtrudeTarget>,
        body: ExtrudeBodyChoice,
    },
    /// Set the live (gizmo-driven) extrusion distance.
    SetExtrudeDistance { distance: f32 },
    /// Constrain (or unconstrain) the in-progress extrusion to an object's extended plane.
    SetExtrudeTarget {
        target: Option<crate::model::ExtrudeTarget>,
    },
    /// Begin editing an existing extrusion.
    EditExtrusion { index: usize },
    /// Finalize the in-progress extrusion (create or update).
    CommitExtrusion,
    /// Add/remove a cross section from the in-progress loft (starts one if needed).
    ToggleLoftSection { section: crate::model::LoftSection },
    /// Finalize the in-progress loft: blend the picked sections into a new body.
    CommitLoft,
    /// Create a new technical drawing (#180) and open it in the drawing pane.
    CreateDrawing { name: Option<String> },
    /// Add a body view (in a given orientation) to a drawing.
    AddDrawingView {
        drawing: usize,
        body: usize,
        orientation: crate::model::DrawingOrientation,
    },
    /// Remove a body view from a drawing by its index.
    RemoveDrawingView { drawing: usize, view: usize },
    /// Toggle the length dimension of one edge (by quantized world endpoints) in a drawing view.
    ToggleDrawingDimension {
        drawing: usize,
        view: usize,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// Toggle the angle dimension between two edges (each by quantized world endpoints).
    ToggleDrawingAngle {
        drawing: usize,
        view: usize,
        edge1: crate::model::DrawingEdgeKey,
        edge2: crate::model::DrawingEdgeKey,
    },
    /// Open a drawing in the drawing pane (`Some`) or close the pane (`None`).
    EditDrawing { drawing: Option<usize> },
    /// Finalize the in-progress revolve (reads `creating_revolve`).
    CommitRevolve,
    /// Scripted/replayed revolve creation with an explicit payload. `bodies` is the
    /// explicit Add/Cut body list; an empty list with `AddTouching` auto-resolves the
    /// touching bodies, like the interactive tool.
    CreateRevolution {
        sketch: SketchId,
        faces: Vec<ExtrudeFace>,
        axis: crate::model::RevolveAxis,
        angle_deg: f32,
        symmetric: bool,
        body: RevolveBodyChoice,
        bodies: Vec<usize>,
    },
    /// Commit the in-progress Combine-tool boolean operation.
    CommitBoolean,
    /// Scripted/replayed boolean operation with an explicit payload (also what
    /// `CommitBoolean` lowers to).
    CreateBooleanOperation {
        kind: crate::model::BooleanOpKind,
        a: Vec<usize>,
        b: Vec<usize>,
        keep_b: bool,
    },
    /// Re-point an existing boolean operation at new inputs / kind / keep-B. Outputs are
    /// preserved (their meshes recompute; the last output absorbs extra result solids).
    EditBooleanOperation {
        op: usize,
        kind: crate::model::BooleanOpKind,
        a: Vec<usize>,
        b: Vec<usize>,
        keep_b: bool,
    },
    /// Commit the in-progress Move-tool operation.
    CommitMove,
    /// Scripted/replayed move operation with an explicit payload.
    CreateMoveOperation {
        targets: Vec<usize>,
        #[allow(dead_code)]
        plane_targets: Vec<usize>,
        #[allow(dead_code)]
        image_targets: Vec<usize>,
        tx: String,
        ty: String,
        tz: String,
        axis: Option<crate::model::RevolveAxis>,
        angle: String,
    },
    /// Re-point an existing move operation.
    EditMoveOperation {
        op: usize,
        targets: Vec<usize>,
        #[allow(dead_code)]
        plane_targets: Vec<usize>,
        #[allow(dead_code)]
        image_targets: Vec<usize>,
        tx: String,
        ty: String,
        tz: String,
        axis: Option<crate::model::RevolveAxis>,
        angle: String,
    },
    /// Commit the in-progress Repeat-tool operation.
    CommitRepeat,
    /// Scripted/replayed linear repeat with an explicit payload.
    CreateRepeatOperation {
        targets: Vec<usize>,
        plane_targets: Vec<usize>,
        axis: crate::model::RevolveAxis,
        mode: crate::model::RepeatMode,
        count: String,
        spacing: String,
        length: String,
    },
    /// Re-point an existing repeat operation.
    EditRepeatOperation {
        op: usize,
        targets: Vec<usize>,
        plane_targets: Vec<usize>,
        axis: crate::model::RevolveAxis,
        mode: crate::model::RepeatMode,
        count: String,
        spacing: String,
        length: String,
    },
    /// Commit the in-progress Slice-tool operation.
    CommitSlice,
    /// Scripted/replayed slice with an explicit payload (also what `CommitSlice` lowers to).
    CreateSliceOperation {
        targets: Vec<usize>,
        cutters: Vec<FaceId>,
        extend_infinite: bool,
    },
    /// Re-point an existing slice operation at new targets / cutters / extend flag. Fragment
    /// bodies are resized to the new piece counts (grow pushes bodies, shrink tombstones).
    EditSliceOperation {
        op: usize,
        targets: Vec<usize>,
        cutters: Vec<FaceId>,
        extend_infinite: bool,
    },
    SetExtrudeBodyMode { mode: ExtrudeBodyMode },
    /// Enable or disable snapping while drawing/dragging.
    SetSnapping(bool),
    /// Add the constraint implied by leaving `point` on a snap target.
    ApplySnapConstraint {
        point: ConstraintPoint,
        target: crate::snapping::SnapTarget,
    },
    /// Redo the last undone action (#193): the inverse of [`Action::UndoLast`].
    RedoLast,
}

/// Maximum number of document snapshots the undo stack retains (#194). Old snapshots past
/// this are dropped, capping session memory for very long edit histories.
const MAX_UNDO_SNAPSHOTS: usize = 200;

impl Action {
    /// Whether [`AppState::apply`] should snapshot the document before this action for
    /// checkpoint undo (#194). Undo/redo never snapshot (they restore snapshots), and the
    /// pure navigation/selection/tool actions never touch the document, so cloning for them
    /// is pointless. Everything else defaults to `true` — a snapshot that turns out unused
    /// (the document didn't change) is discarded — so a new *mutating* action is undoable by
    /// default without having to remember to opt in.
    fn snapshots_for_undo(&self) -> bool {
        !self.resets_undo_history()
            && !matches!(
                self,
                Action::UndoLast
                    | Action::RedoLast
                    | Action::SetTool(_)
                    | Action::ClickSceneElement { .. }
                    | Action::ClearSceneSelection
                    | Action::SetElementVisible { .. }
                    | Action::ToggleElementVisibility(_)
                    | Action::ToggleFpsMode
                    | Action::EditDrawing { .. }
            )
    }

    /// Actions that replace or wipe the whole document, after which the previous edit
    /// history is meaningless — the undo/redo stacks are cleared rather than letting undo
    /// cross the boundary back into a different document (#194).
    fn resets_undo_history(&self) -> bool {
        matches!(
            self,
            Action::NewDocument | Action::Open { .. } | Action::Clear
        )
    }
}

/// A toggleable UI pane (SPEC §11.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The rotating orientation cube in the viewport corner ([`view_cube`]).
    ViewCube,
    /// Scene tree with visibility toggles and sketch editing.
    Hierarchy,
    /// Named parameters and expressions.
    Parameters,
    /// Properties for the current tree selection.
    Context,
}

impl Pane {
    /// All panes, in menu order.
    pub const ALL: &'static [Pane] = &[Pane::Hierarchy, Pane::Context, Pane::Parameters, Pane::ViewCube];

    /// Human-readable label for menus.
    pub fn label(self) -> &'static str {
        match self {
            Pane::ViewCube => "View Bear",
            Pane::Hierarchy => "Elements",
            Pane::Parameters => "Parameters",
            Pane::Context => "Context",
        }
    }

    /// Stable name used in instruction scripts.
    pub fn script_name(self) -> &'static str {
        match self {
            Pane::ViewCube => "view_bear",
            Pane::Hierarchy => "hierarchy",
            Pane::Parameters => "parameters",
            Pane::Context => "context",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "view_bear" | "viewbear" | "bear" | "view_cube" | "viewcube" | "cube" | "hud" => {
                Some(Pane::ViewCube)
            }
            "hierarchy" | "tree" | "dag" | "elements" => Some(Pane::Hierarchy),
            "parameters" | "params" | "param" => Some(Pane::Parameters),
            "context" | "properties" | "props" => Some(Pane::Context),
            _ => None,
        }
    }
}

/// Which panes are currently shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneVisibility {
    pub view_cube: bool,
    pub hierarchy: bool,
    pub parameters: bool,
    pub context: bool,
}

impl Default for PaneVisibility {
    fn default() -> Self {
        Self {
            view_cube: true,
            hierarchy: true,
            parameters: true,
            context: true,
        }
    }
}

impl PaneVisibility {
    pub fn is_visible(&self, pane: Pane) -> bool {
        match pane {
            Pane::ViewCube => self.view_cube,
            Pane::Hierarchy => self.hierarchy,
            Pane::Parameters => self.parameters,
            Pane::Context => self.context,
        }
    }

    pub fn set(&mut self, pane: Pane, visible: bool) {
        match pane {
            Pane::ViewCube => self.view_cube = visible,
            Pane::Hierarchy => self.hierarchy = visible,
            Pane::Parameters => self.parameters = visible,
            Pane::Context => self.context = visible,
        }
    }

    pub fn toggle(&mut self, pane: Pane) {
        let next = !self.is_visible(pane);
        self.set(pane, next);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimLabelAxis {
    Width,
    Height,
    Length,
}

impl DimLabelAxis {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "width" | "w" => Some(Self::Width),
            "height" | "h" => Some(Self::Height),
            "length" | "len" | "l" => Some(Self::Length),
            _ => None,
        }
    }
}

pub fn dim_label_target_in_sketch(
    doc: &Document,
    sketch: SketchId,
    axis: DimLabelAxis,
) -> Option<DimLabelTarget> {
    // Rectangle width/height are now ordinary line-length dimensions (#66); only the
    // `Length` axis resolves to a committed dimension.
    let target = match axis {
        DimLabelAxis::Width | DimLabelAxis::Height => None,
        DimLabelAxis::Length => doc
            .lines
            .iter()
            .enumerate()
            .rev()
            .find(|(_, l)| l.sketch == sketch)
            .map(|(index, _)| DistanceTarget::LineLength(index)),
    }?;
    find_distance_constraint(doc, target)
}

/// A committed sketch dimension label the user can reposition.
pub type DimLabelTarget = ConstraintId;

pub fn constraint_is_line_length(doc: &Document, target: DimLabelTarget) -> bool {
    doc.constraints.get(target).is_some_and(|c| {
        matches!(
            c.kind,
            crate::model::ConstraintKind::Distance {
                target: DistanceTarget::LineLength(_)
            }
        )
    })
}

pub fn constraint_is_circle_diameter(doc: &Document, target: DimLabelTarget) -> bool {
    doc.constraints.get(target).is_some_and(|c| {
        matches!(
            c.kind,
            crate::model::ConstraintKind::Distance {
                target: DistanceTarget::CircleDiameter(_)
            }
        )
    })
}

pub fn constraint_is_angle(doc: &Document, target: DimLabelTarget) -> bool {
    doc.constraints.get(target).is_some_and(|c| {
        matches!(c.kind, crate::model::ConstraintKind::Angle { .. })
    })
}

pub fn dim_label_axis_for_target(doc: &Document, target: DimLabelTarget) -> Option<DimLabelAxis> {
    if constraint_is_line_length(doc, target) {
        Some(DimLabelAxis::Length)
    } else {
        None
    }
}

/// In-progress edit of a sketch dimension (Select or Dimension tool).
#[derive(Clone, Debug, PartialEq)]
pub enum DimEditTarget {
    Constraint(ConstraintId),
    New(DimensionTarget),
}

impl DimEditTarget {
    pub fn dimension_target(&self, doc: &Document) -> Option<DimensionTarget> {
        match self {
            DimEditTarget::New(target) => Some(target.clone()),
            DimEditTarget::Constraint(id) => doc.constraints.get(*id).and_then(|c| match &c.kind {
                crate::model::ConstraintKind::Distance { target } => {
                    Some(DimensionTarget::Distance(target.clone()))
                }
                crate::model::ConstraintKind::Angle {
                    line_a,
                    line_b,
                    rotation_sign,
                } => Some(DimensionTarget::Angle {
                    line_a: line_a.clone(),
                    line_b: line_b.clone(),
                    rotation_sign: *rotation_sign,
                }),
                _ => None,
            }),
        }
    }

    pub fn distance_target(&self, doc: &Document) -> Option<DistanceTarget> {
        match self.dimension_target(doc)? {
            DimensionTarget::Distance(target) => Some(target),
            DimensionTarget::Angle { .. } => None,
        }
    }

    pub fn is_angle(&self, doc: &Document) -> bool {
        matches!(
            self.dimension_target(doc),
            Some(DimensionTarget::Angle { .. })
        )
    }
}

/// Committed angle constraint whose gizmo should be visible while its text field is open.
pub fn angle_gizmo_constraint_for_edit(
    edit: Option<&EditingCommittedDim>,
    doc: &Document,
) -> Option<ConstraintId> {
    let edit = edit?;
    if !edit.target.is_angle(doc) {
        return None;
    }
    match edit.target {
        DimEditTarget::Constraint(id) => Some(id),
        DimEditTarget::New(_) => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditingCommittedDim {
    pub target: DimEditTarget,
    pub text: String,
    pub pending_focus: bool,
    /// Arc radius (screen px) chosen while placing a *new* angle dimension, applied to the
    /// constraint's `dim_offset` on commit so the placed arc keeps the size the preview
    /// showed (#188). `None` for distance dims and edits of existing dimensions.
    pub dim_offset: Option<f32>,
}

/// Placement phase for a brand-new angle dimension: the preview follows the mouse,
/// snapping `rotation_sign` to whichever of the angle's two distinct magnitudes
/// (the natural one or its supplement) encloses the cursor; a click commits it and
/// moves on to typing the value (#40).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacingAngleDimension {
    pub line_a: ConstraintLine,
    pub line_b: ConstraintLine,
    pub rotation_sign: crate::model::ConstraintSign,
    /// Arc radius (screen px) the preview is currently drawn at — the cursor's distance
    /// from the vertex, so the arc grows/shrinks as you move the mouse (#188). Persisted to
    /// the constraint's `dim_offset` on commit.
    pub arc_offset: Option<f32>,
}

/// Expression text shown when editing a committed dimension.
pub fn committed_dim_expression(doc: &Document, target: DimLabelTarget) -> Option<String> {
    constraint_expression(doc, target)
}

fn apply_committed_dim_expression(
    doc: &mut Document,
    sketch: SketchId,
    target: DimEditTarget,
    expression: &str,
) -> Result<(), String> {
    match target {
        DimEditTarget::Constraint(id) => set_constraint_expression(doc, id, expression.to_string()),
        DimEditTarget::New(dimension_target) => {
            apply_dimension_expression(doc, sketch, dimension_target, expression)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectAxis {
    Width,
    Height,
}

impl RectAxis {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "width" | "w" => Some(RectAxis::Width),
            "height" | "h" => Some(RectAxis::Height),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        match self {
            RectAxis::Width => 0,
            RectAxis::Height => 1,
        }
    }
}

/// Active sketch session: new geometry is parented to this sketch until exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchSession {
    pub sketch: SketchId,
}

/// Transient UI state for the command palette (SPEC §11.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandPaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub request_focus: bool,
    /// Previous query text; used to reset selection when the filter changes.
    pub prior_query: String,
}

impl CommandPaletteState {
    pub fn open_palette(&mut self) {
        self.open = true;
        self.query.clear();
        self.prior_query.clear();
        self.selected = 0;
        self.request_focus = true;
    }

    pub fn close_palette(&mut self) {
        self.open = false;
        self.query.clear();
        self.prior_query.clear();
        self.selected = 0;
        self.request_focus = false;
    }
}

/// Application state that actions mutate.
pub struct AppState {
    pub doc: Document,
    pub path: Option<String>,
    pub tool: Tool,
    pub sketch_session: Option<SketchSession>,
    pub cam: Camera,
    pub creating_rect: Option<CreatingRect>,
    pub creating_line: Option<CreatingLine>,
    pub creating_circle: Option<CreatingCircle>,
    /// In-progress (or being-edited) extrusion: selected faces + live distance.
    pub creating_extrusion: Option<CreatingExtrusion>,
    /// In-progress chamfer/fillet: picked vertex + live gizmo-driven amount.
    pub creating_vertex_treatment: Option<CreatingVertexTreatment>,
    /// In-progress 3D solid-edge chamfer/fillet (#77): picked extrusion edge + live
    /// gizmo-driven amount. Parallel to `creating_vertex_treatment` — see
    /// [`CreatingEdgeTreatment`].
    pub creating_edge_treatment: Option<CreatingEdgeTreatment>,
    /// In-progress loft (Loft tool): the picked cross sections, shown in the context-pane
    /// selection picker.
    pub creating_loft: Option<CreatingLoft>,
    /// In-progress revolve (Revolve tool).
    pub creating_revolve: Option<CreatingRevolve>,
    /// In-progress boolean operation (Combine tool).
    pub creating_boolean: Option<CreatingBoolean>,
    /// In-progress move operation (Move tool).
    pub creating_move: Option<CreatingMove>,
    /// In-progress linear repeat (Repeat tool).
    pub creating_repeat: Option<CreatingRepeat>,
    /// In-progress slice operation (Slice tool).
    pub creating_slice: Option<CreatingSlice>,
    /// The technical drawing (#180) currently open in the drawing pane, if any. UI state
    /// (never persisted): while `Some`, the central area shows that drawing instead of the
    /// 3D viewport.
    pub editing_drawing: Option<usize>,
    /// In-progress image scale calibration (#163/#171): Some while the user is placing
    /// the two reference points / typing the real length.
    pub creating_calibration: Option<CreatingCalibration>,
    /// Viewport width/height, refreshed each frame by the UI — camera framing (ZoomToFit)
    /// needs it to fit the horizontal field of view.
    pub viewport_aspect: f32,
    /// Shared construction draw mode for rectangle, line, and circle tools.
    pub draw_construction: bool,
    /// Persisted "next point gets bezier handles" toggle for the line tool (`B`, #73); mirrors
    /// how `draw_construction` persists across chained segments.
    pub draw_curve_mode: bool,
    /// Persisted tangent-continuity toggle for the line tool (`T`, #73); only meaningful while
    /// `draw_curve_mode` is on.
    pub draw_tangent_constraint: bool,
    pub creating_plane: Option<CreatingConstructionPlane>,
    pub panes: PaneVisibility,
    pub parameters_pane: ParametersPaneState,
    pub command_palette: CommandPaletteState,
    pub element_visibility: ElementVisibility,
    pub scene_selection: SceneSelection,
    pub context_pane: crate::context::ContextPaneState,
    pub editing_committed_dim: Option<EditingCommittedDim>,
    /// Active placement phase for a new angle dimension (#40); see [`PlacingAngleDimension`].
    pub placing_angle_dimension: Option<PlacingAngleDimension>,
    pub status: String,
    pub command_log: Option<std::cell::RefCell<crate::command_log::CommandLog>>,
    /// Reframe sketch geometry once the viewport rect is known (e.g. hierarchy open before first paint).
    pub sketch_reframe_pending: bool,
    /// Camera pose captured when entering a sketch, restored when exiting it.
    pub pre_sketch_pose: Option<crate::camera::HomeView>,
    pub document_health: DocumentHealth,
    /// #103 part 2: `Some` while a cut-bearing body can't be built by the kernel, so the
    /// viewport is rendering the additive-only fallback with the cuts silently missing.
    /// Recomputed alongside `document_health` at every document mutation point; re-asserted
    /// into `status` at the end of every mutating [`AppState::apply`] so the warning stays
    /// visible for as long as the document is in that state. Always `None` without the
    /// kernel (`--no-default-features`): there the limitation is inherent and documented.
    pub kernel_fallback_warning: Option<String>,
    /// Whether `refresh_document_health` ran during the current `apply` call (i.e. the
    /// document just mutated) — consumed by `apply`'s tail to decide when to re-assert
    /// `kernel_fallback_warning` into `status`.
    pub(crate) kernel_fallback_warning_pending: bool,
    pub line_drag_session: Option<crate::vertex_drag::LineDragSession>,
    /// Snap a moved/drawn point to nearby geometry (and add a constraint when left there).
    pub snapping_enabled: bool,
    /// The point being dragged and what it is currently snapped to (committed on release).
    pub active_snap: Option<(ConstraintPoint, crate::snapping::SnapTarget)>,
    /// Snap targets for the start/end of a line being drawn (applied on commit).
    pub line_start_snap: Option<crate::snapping::SnapTarget>,
    pub line_end_snap: Option<crate::snapping::SnapTarget>,
    /// Snap targets for the origin/opposite corners of a rectangle being drawn.
    pub rect_origin_snap: Option<crate::snapping::SnapTarget>,
    pub rect_opposite_snap: Option<crate::snapping::SnapTarget>,
    /// Snap target for the center of a circle being drawn.
    pub circle_center_snap: Option<crate::snapping::SnapTarget>,
    /// Inference ("extension") snap guides: edges of the vertex the cursor most recently
    /// hovered while sketching. While these are active, pulling away from that vertex snaps
    /// the point onto the infinite extension of those edges (#21). Cleared on sketch exit.
    pub extension_anchors: Vec<crate::model::ConstraintLine>,
    /// Recursion depth of [`AppState::apply`]: undo-group boundaries (#105) are only
    /// recorded by the outermost call, so actions that delegate to other actions
    /// still undo as one gesture. Transient, never persisted; `pub` only so
    /// struct-update construction (`..AppState::default()`) works across modules.
    pub undo_group_depth: u8,
    /// Checkpoint-based undo/redo (#194): a snapshot of the whole document taken *before*
    /// each mutating user action. `UndoLast` restores the top snapshot; `RedoLast` re-applies
    /// it. Transient session state, never persisted. Capped at [`MAX_UNDO_SNAPSHOTS`].
    pub undo_stack: Vec<Document>,
    pub redo_stack: Vec<Document>,
    /// First-person mode player state (#91); `Some` while FPS mode is active.
    /// Transient, never persisted.
    pub fps: Option<crate::fps::FpsController>,
    /// The last active `fps` player state, kept after leaving FPS mode so the next entry
    /// resumes its player scale (#120/#135; position always restarts from the camera).
    /// Transient, never persisted.
    pub fps_memory: Option<crate::fps::FpsController>,
    /// Inference snap guide for #41: the line whose midpoint the cursor most recently touched
    /// while sketching. While set, pulling away from that midpoint snaps the point onto the
    /// infinite line normal to it, through its midpoint. Cleared on sketch exit.
    pub normal_inference_anchor: Option<crate::model::ConstraintLine>,
    /// Snapshots of `construction_planes` taken before each in-place plane edit, so that
    /// `UndoLast` can revert the edit. Kept in lockstep with `ShapeKind::ConstructionPlaneEdit`
    /// markers in `shape_order` (one payload per marker, same LIFO order).
    /// Snapshots of an extrusion's `edge_treatments` taken before each chamfer/fillet
    /// commit, so `UndoLast` can revert the edit (#168). Kept in lockstep with
    /// `ShapeKind::EdgeTreatmentEdit` markers, mirroring `construction_plane_edit_undo`.
    pub edge_treatment_undo: Vec<(usize, Vec<EdgeTreatment>)>,
    pub construction_plane_edit_undo: Vec<Vec<ConstructionPlane>>,
    /// Elements-pane layout (List/Tree/Graph, #34). Ephemeral UI view state like
    /// `extension_anchors` — never persisted; lives here (not on `App`) so scripts can
    /// drive it via `bearcad.ui.elements_view` (#108).
    pub hierarchy_view_mode: crate::hierarchy::HierarchyViewMode,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            doc: Document::default(),
            path: None,
            tool: Tool::default(),
            sketch_session: None,
            cam: Camera::default(),
            creating_rect: None,
            creating_line: None,
            creating_circle: None,
            creating_extrusion: None,
            creating_vertex_treatment: None,
            creating_edge_treatment: None,
            creating_loft: None,
            creating_revolve: None,
            creating_boolean: None,
            creating_move: None,
            creating_repeat: None,
            creating_slice: None,
            editing_drawing: None,
            creating_calibration: None,
            viewport_aspect: 16.0 / 9.0,
            draw_construction: false,
            draw_curve_mode: false,
            draw_tangent_constraint: true,
            creating_plane: None,
            panes: PaneVisibility::default(),
            parameters_pane: ParametersPaneState::default(),
            command_palette: CommandPaletteState::default(),
            element_visibility: ElementVisibility::default(),
            scene_selection: SceneSelection::default(),
            context_pane: crate::context::ContextPaneState::default(),
            editing_committed_dim: None,
            placing_angle_dimension: None,
            status: String::new(),
            command_log: None,
            sketch_reframe_pending: false,
            pre_sketch_pose: None,
            document_health: DocumentHealth::default(),
            kernel_fallback_warning: None,
            kernel_fallback_warning_pending: false,
            line_drag_session: None,
            snapping_enabled: true,
            active_snap: None,
            line_start_snap: None,
            line_end_snap: None,
            rect_origin_snap: None,
            rect_opposite_snap: None,
            circle_center_snap: None,
            extension_anchors: Vec::new(),
            undo_group_depth: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            fps: None,
            fps_memory: None,
            normal_inference_anchor: None,
            construction_plane_edit_undo: Vec::new(),
            edge_treatment_undo: Vec::new(),
            hierarchy_view_mode: crate::hierarchy::HierarchyViewMode::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeshExportFormat {
    Stl,
    Step,
}

impl MeshExportFormat {
    fn label(self) -> &'static str {
        match self {
            MeshExportFormat::Stl => "STL",
            MeshExportFormat::Step => "STEP",
        }
    }
}

impl AppState {
    pub fn refresh_document_health(&mut self) {
        self.document_health = recompute_document_health(&self.doc);
        // #103 part 2: this is the one seam every document mutation already goes through
        // (commits, open, undo, imports — never per-frame drags), so the "kernel fallback is
        // silently dropping this body's cuts" check lives here rather than being replicated
        // across every extrusion/treatment/cut action arm. `apply`'s tail turns the result
        // into a status-bar warning once the arm has finished writing its own status.
        #[cfg(feature = "occt")]
        {
            self.kernel_fallback_warning = crate::extrude::kernel_fallback_cut_warning(&self.doc);
            self.kernel_fallback_warning_pending = true;
        }
    }

    /// Move extrusion `ei` to wherever `mode` says it should live, used when editing an
    /// extrusion's body choice in the context pane (#32). The extrusion always already has a
    /// home (every committed extrusion gets one), so this only needs to detach it from there
    /// when the new home differs and attach it to the new one.
    fn apply_extrude_body_mode(&mut self, ei: usize, mode: ExtrudeBodyMode) {
        let current = crate::model::body_index_for_extrusion(&self.doc, ei);
        // The body is solely `ei`'s home (a lone added extrusion, no cuts) — removing `ei`
        // would leave it empty, so it should be tombstoned rather than emptied.
        let solely_owns = |doc: &Document, bi: usize| {
            doc.bodies.get(bi).is_some_and(|b| {
                b.source.extrusion_indices() == [ei] && b.source.cut_extrusion_indices().is_empty()
            })
        };
        // Whether `ei` is currently a *cut* of body `bi` (vs an added extrusion).
        let is_cut_in = |doc: &Document, bi: usize| {
            doc.bodies
                .get(bi)
                .is_some_and(|b| b.source.cut_extrusion_indices().contains(&ei))
        };
        let already_there = match (current, mode) {
            (Some(bi), ExtrudeBodyMode::MergeInto(target)) => {
                bi == target && !is_cut_in(&self.doc, bi)
            }
            (Some(bi), ExtrudeBodyMode::Cut(target)) => bi == target && is_cut_in(&self.doc, bi),
            (Some(bi), ExtrudeBodyMode::NewBody) => solely_owns(&self.doc, bi),
            (None, _) => true,
        };
        if already_there {
            return;
        }
        if let Some(bi) = current {
            if solely_owns(&self.doc, bi) {
                crate::document_lifecycle::tombstone_element(
                    &mut self.doc,
                    SceneElement::Body(bi),
                );
            } else if let Some(body) = self.doc.bodies.get_mut(bi) {
                body.source.remove_extrusion(ei);
            }
        }
        match mode {
            ExtrudeBodyMode::NewBody => {
                self.doc.bodies.push(crate::model::Body {
                    source: crate::model::BodySource::single(ei),
                    name: None,
                    deleted: false,
                    shadow: false,
                });
                self.doc.shape_order.push(ShapeKind::Body);
            }
            ExtrudeBodyMode::MergeInto(bi) => {
                if let Some(body) = self.doc.bodies.get_mut(bi).filter(|b| !b.deleted) {
                    body.source.append_extrusion(ei);
                } else {
                    self.doc.bodies.push(crate::model::Body {
                        source: crate::model::BodySource::single(ei),
                        name: None,
                        deleted: false,
                        shadow: false,
                    });
                    self.doc.shape_order.push(ShapeKind::Body);
                }
            }
            ExtrudeBodyMode::Cut(bi) => {
                if let Some(body) = self.doc.bodies.get_mut(bi).filter(|b| !b.deleted) {
                    body.source.append_cut_extrusion(ei);
                } else {
                    self.doc.bodies.push(crate::model::Body {
                        source: crate::model::BodySource::single(ei),
                        name: None,
                        deleted: false,
                        shadow: false,
                    });
                    self.doc.shape_order.push(ShapeKind::Body);
                }
            }
        }
    }

    /// Attach freshly-created extrusion `ei` (just pushed, owned by no body yet) to a body
    /// per `mode`, creating a new body if needed. Returns the resulting body's index.
    fn attach_new_extrusion_to_body(&mut self, ei: usize, mode: ExtrudeBodyMode) -> usize {
        match mode {
            ExtrudeBodyMode::MergeInto(bi) => {
                if let Some(body) = self.doc.bodies.get_mut(bi).filter(|b| !b.deleted) {
                    body.source.append_extrusion(ei);
                    return bi;
                }
            }
            ExtrudeBodyMode::Cut(bi) => {
                if let Some(body) = self.doc.bodies.get_mut(bi).filter(|b| !b.deleted) {
                    body.source.append_cut_extrusion(ei);
                    return bi;
                }
            }
            ExtrudeBodyMode::NewBody => {}
        }
        self.doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::single(ei),
            name: None,
            deleted: false,
            shadow: false,
        });
        self.doc.shape_order.push(ShapeKind::Body);
        self.doc.bodies.len() - 1
    }

    /// Add `triangles` from an imported file as a new body named after `path`'s file stem
    /// (shared by STL and STEP import, #70/#71).
    fn import_mesh_body(&mut self, path: &str, triangles: Vec<[Vec3; 3]>) -> ActionResult {
        let source_name = std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "import".to_string());
        let tri_count = triangles.len();
        self.doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles,
            source_name: source_name.clone(),
        });
        let mesh_index = self.doc.imported_meshes.len() - 1;
        self.doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh_index),
            name: Some(source_name),
            deleted: false,
            shadow: false,
        });
        self.doc.shape_order.push(ShapeKind::Body);
        self.refresh_document_health();
        self.status = format!("Imported {tri_count} triangle(s) from {path}");
        ActionResult::Ok
    }

    /// Write `mesh` to `path` as an ASCII STL named `name`, setting `self.status`.
    fn write_stl_file(
        &mut self,
        path: &str,
        name: &str,
        mesh: Option<crate::extrude::SolidMesh>,
    ) -> ActionResult {
        self.write_mesh_file(path, name, mesh, MeshExportFormat::Stl)
    }

    /// Write `mesh` to `path` as a STEP FACETED_BREP named `name`, setting `self.status`.
    fn write_step_file(
        &mut self,
        path: &str,
        name: &str,
        mesh: Option<crate::extrude::SolidMesh>,
    ) -> ActionResult {
        self.write_mesh_file(path, name, mesh, MeshExportFormat::Step)
    }

    /// Export a single body (by index) to `path` as STEP (#65). In `occt` builds, when the
    /// body has a kernel-representable OCCT solid, write **real BREP** (planar + curved
    /// surfaces) straight to the file via `STEPControl_Writer`; otherwise (non-`occt`, an
    /// imported-mesh body, non-representable geometry, or a kernel write failure) fall back
    /// to the hand-rolled faceted-BREP mesh path.
    fn write_step_body_file(&mut self, path: &str, name: &str, body: usize) -> ActionResult {
        #[cfg(feature = "occt")]
        {
            if let Some(shape) = crate::extrude::occt_body_shape(&self.doc, body) {
                if shape.write_step(std::path::Path::new(path)) {
                    self.status = format!("Exported body '{name}' to {path} (STEP BREP)");
                    return ActionResult::Ok;
                }
                // Kernel write failed — fall through to the faceted mesh path below.
            }
        }
        let mesh = crate::extrude::body_solid_mesh(&self.doc, body);
        self.write_step_file(path, name, mesh)
    }

    /// Byte-level document/import/export entry points for the **web build** (no
    /// filesystem — the browser hands us bytes from a picked file, and downloads bytes we
    /// hand back). Compiled everywhere so native tests can exercise them.
    ///
    /// Open a JSON-format document (see `storage::to_json_bytes`) from raw bytes.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn open_document_bytes(&mut self, bytes: &[u8], name: &str) -> ActionResult {
        match crate::storage::from_json_bytes(bytes) {
            Ok(mut doc) => {
                if let Err(e) = recompute_document_geometry(&mut doc) {
                    self.status = format!("Open failed: {e}");
                    return ActionResult::Err(e);
                }
                let n_lines = doc.lines.len();
                self.doc = doc;
                self.sketch_session = None;
                self.cam.set_view_up(None);
                self.refresh_document_health();
                self.path = None;
                self.status = format!("Opened {name} ({n_lines} line(s))");
                ActionResult::Ok
            }
            Err(e) => {
                self.status = format!("Open failed: {e}");
                ActionResult::Err(e)
            }
        }
    }

    /// Import an STL from raw bytes as a new body.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn import_stl_bytes(&mut self, name: &str, bytes: &[u8]) -> ActionResult {
        match crate::stl::parse_stl(bytes) {
            Ok(tris) => {
                self.import_mesh_body(name, tris.into_iter().map(|t| t.vertices).collect())
            }
            Err(e) => {
                self.status = format!("Import failed: {e}");
                ActionResult::Err(self.status.clone())
            }
        }
    }

    /// Import a STEP file's faceted geometry from raw bytes as a new body (the hand-rolled
    /// parser — the kernel reader is path-based and native-only).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn import_step_bytes(&mut self, name: &str, bytes: &[u8]) -> ActionResult {
        // Web kernel builds read real BREP (curved surfaces included) through the
        // bridged STEP reader, mirroring the native path-based arm.
        #[cfg(all(feature = "occt", target_arch = "wasm32"))]
        {
            if let Some(shape) = crate::kernel::Shape::read_step_bytes(bytes) {
                let tris = shape.tessellate(crate::extrude::OCCT_DEFLECTION as f64);
                if !tris.is_empty() {
                    return self.import_mesh_body(name, tris);
                }
            }
        }
        let text = match std::str::from_utf8(bytes) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Import failed: not UTF-8 ({e})");
                return ActionResult::Err(self.status.clone());
            }
        };
        match crate::step::parse_step_mesh(text) {
            Ok(triangles) => self.import_mesh_body(name, triangles),
            Err(e) => {
                self.status = format!("Import failed: {e}");
                ActionResult::Err(self.status.clone())
            }
        }
    }

    /// Import a PNG/JPEG from raw bytes as a tracing image on `plane` (default: ground).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn import_image_bytes(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        plane: Option<usize>,
    ) -> ActionResult {
        let dims = match image::load_from_memory(&bytes) {
            Ok(img) => (img.width() as f32, img.height() as f32),
            Err(e) => {
                self.status = format!("Import failed: not a readable image ({e})");
                return ActionResult::Err(self.status.clone());
            }
        };
        let plane = plane.unwrap_or(0);
        if !self
            .doc
            .construction_planes
            .get(plane)
            .is_some_and(|p| !p.deleted)
        {
            self.status = format!("Import failed: construction plane {plane} not found");
            return ActionResult::Err(self.status.clone());
        }
        let source_name = std::path::Path::new(name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "image".to_string());
        self.doc.tracing_images.push(crate::model::TracingImage {
            bytes,
            source_name: source_name.clone(),
            plane,
            origin: (-dims.0 / 2.0, -dims.1 / 2.0),
            width_mm: dims.0,
            height_mm: dims.1,
            name: None,
            deleted: false,
            calibration: None,
            base_origin: None,
        });
        self.doc.shape_order.push(crate::model::ShapeKind::Image);
        self.refresh_document_health();
        self.status = format!("Imported image {source_name}");
        ActionResult::Ok
    }

    /// ASCII STL of one body (or the whole document) as bytes.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn export_stl_bytes(&self, body: Option<usize>) -> Result<Vec<u8>, String> {
        let (name, mesh) = self.export_mesh_for(body)?;
        Ok(crate::stl::write_ascii_stl(&name, &mesh).into_bytes())
    }

    /// STEP of one body (or the whole document) as bytes. Web kernel builds write real
    /// BREP through the bridged writer when a single body is exportable (mirroring the
    /// native single-body path); everything else uses the faceted writer.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn export_step_bytes(&self, body: Option<usize>) -> Result<Vec<u8>, String> {
        #[cfg(all(feature = "occt", target_arch = "wasm32"))]
        {
            let single = body.or_else(|| {
                let mut live = self
                    .doc
                    .bodies
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| !b.deleted);
                match (live.next(), live.next()) {
                    (Some((bi, _)), None) => Some(bi),
                    _ => None,
                }
            });
            if let Some(bi) = single {
                if let Some(shape) = crate::extrude::occt_body_shape(&self.doc, bi) {
                    if let Some(bytes) = shape.write_step_bytes() {
                        return Ok(bytes);
                    }
                }
            }
        }
        let (name, mesh) = self.export_mesh_for(body)?;
        Ok(crate::step::write_step(&name, &mesh).into_bytes())
    }

    /// Resolve a revolve body choice into a concrete [`RevolveMode`]. `AddTouching` with
    /// an empty explicit list finds bodies whose mesh bounds intersect the revolve's;
    /// `Cut` requires a non-empty body list.
    #[allow(clippy::too_many_arguments)]
    fn resolve_revolve_mode(
        &mut self,
        sketch: SketchId,
        faces: &[ExtrudeFace],
        axis: crate::model::RevolveAxis,
        angle_deg: f32,
        symmetric: bool,
        choice: RevolveBodyChoice,
        bodies: &[usize],
    ) -> Result<crate::model::RevolveMode, String> {
        Ok(match choice {
            RevolveBodyChoice::NewBody => crate::model::RevolveMode::NewBody,
            RevolveBodyChoice::AddTouching => {
                if !bodies.is_empty() {
                    return Ok(crate::model::RevolveMode::AddTo(bodies.to_vec()));
                }
                let probe = crate::model::Revolution {
                    sketch,
                    faces: faces.to_vec(),
                    axis,
                    angle_deg,
                    symmetric,
                    mode: crate::model::RevolveMode::NewBody,
                    name: None,
                    deleted: false,
                };
                let touching = crate::extrude::revolve_mesh(&self.doc, &probe)
                    .and_then(|m| m.bounds())
                    .map(|rb| {
                        (0..self.doc.bodies.len())
                            .filter(|&bi| !self.doc.bodies[bi].deleted)
                            .filter(|&bi| {
                                crate::extrude::body_solid_mesh(&self.doc, bi)
                                    .and_then(|m| m.bounds())
                                    .is_some_and(|bb| {
                                        bb.0.cmple(rb.1).all() && rb.0.cmple(bb.1).all()
                                    })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if touching.is_empty() {
                    self.status = "No touching bodies — revolved into a new body".to_string();
                    crate::model::RevolveMode::NewBody
                } else {
                    crate::model::RevolveMode::AddTo(touching)
                }
            }
            RevolveBodyChoice::Cut => {
                if bodies.is_empty() {
                    return Err("Pick at least one body to cut".to_string());
                }
                crate::model::RevolveMode::Cut(bodies.to_vec())
            }
        })
    }

    /// Shared revolve commit: validates the solid builds, stores the [`Revolution`], and
    /// (in NewBody mode) its body — one `ShapeKind::Revolution` undo marker covers both.
    fn create_revolution(
        &mut self,
        sketch: SketchId,
        faces: Vec<ExtrudeFace>,
        axis: crate::model::RevolveAxis,
        angle_deg: f32,
        symmetric: bool,
        mode: crate::model::RevolveMode,
    ) -> ActionResult {
        let rev = crate::model::Revolution {
            sketch,
            faces,
            axis,
            angle_deg,
            symmetric,
            mode: mode.clone(),
            name: None,
            deleted: false,
        };
        if crate::extrude::revolve_mesh(&self.doc, &rev).is_none() {
            let e = "Revolve failed: profile must be a closed face and the axis a real line"
                .to_string();
            self.status = e.clone();
            return ActionResult::Err(e);
        }
        self.doc.revolutions.push(rev);
        if matches!(mode, crate::model::RevolveMode::NewBody) {
            self.doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Revolve(self.doc.revolutions.len() - 1),
                name: None,
                deleted: false,
                shadow: false,
            });
        }
        self.doc.shape_order.push(crate::model::ShapeKind::Revolution);
        self.creating_revolve = None;
        self.tool = Tool::Select;
        self.refresh_document_health();
        self.status = match &mode {
            crate::model::RevolveMode::NewBody => format!("Revolved ({angle_deg:.0}°)"),
            crate::model::RevolveMode::AddTo(b) => {
                format!("Revolved into {} body(ies) ({angle_deg:.0}°)", b.len())
            }
            crate::model::RevolveMode::Cut(b) => {
                format!("Revolve cut {} body(ies) ({angle_deg:.0}°)", b.len())
            }
        };
        ActionResult::Ok
    }

    /// Re-point an existing revolution (#211): replace its parameters in place (preserving its
    /// name), then reconcile its output body — a `NewBody`-mode revolve owns one body via
    /// `BodySource::Revolve`; `AddTo`/`Cut` own none (they fuse at recompute).
    #[allow(clippy::too_many_arguments)]
    fn edit_revolution(
        &mut self,
        op: usize,
        sketch: SketchId,
        faces: Vec<ExtrudeFace>,
        axis: crate::model::RevolveAxis,
        angle_deg: f32,
        symmetric: bool,
        mode: crate::model::RevolveMode,
    ) -> ActionResult {
        let Some(existing) = self.doc.revolutions.get(op).filter(|r| !r.deleted) else {
            return ActionResult::Err(format!("no revolution {op}"));
        };
        let candidate = crate::model::Revolution {
            sketch,
            faces,
            axis,
            angle_deg,
            symmetric,
            mode: mode.clone(),
            name: existing.name.clone(),
            deleted: false,
        };
        if crate::extrude::revolve_mesh(&self.doc, &candidate).is_none() {
            let e = "Revolve failed: profile must be a closed face and the axis a real line"
                .to_string();
            self.status = e.clone();
            return ActionResult::Err(e);
        }
        self.doc.revolutions[op] = candidate;
        // Reconcile the owned body with the (possibly changed) mode.
        let has_body = self
            .doc
            .bodies
            .iter()
            .any(|b| !b.deleted && b.source == crate::model::BodySource::Revolve(op));
        match (matches!(mode, crate::model::RevolveMode::NewBody), has_body) {
            (true, false) => self.doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Revolve(op),
                name: None,
                deleted: false,
                shadow: false,
            }),
            (false, true) => {
                for body in self.doc.bodies.iter_mut() {
                    if body.source == crate::model::BodySource::Revolve(op) {
                        body.deleted = true;
                    }
                }
            }
            _ => {}
        }
        self.creating_revolve = None;
        self.tool = Tool::Select;
        self.refresh_document_health();
        self.status = format!("Revolve updated ({angle_deg:.0}°)");
        ActionResult::Ok
    }

    fn export_mesh_for(
        &self,
        body: Option<usize>,
    ) -> Result<(String, crate::extrude::SolidMesh), String> {
        let (name, mesh) = match body {
            Some(bi) => {
                let b = self
                    .doc
                    .bodies
                    .get(bi)
                    .filter(|b| !b.deleted)
                    .ok_or_else(|| format!("no body {bi}"))?;
                let name = b.name.clone().unwrap_or_else(|| format!("body-{bi}"));
                (name, crate::extrude::body_solid_mesh(&self.doc, bi))
            }
            None => (
                "bearcad".to_string(),
                Some(crate::extrude::document_solid_mesh(&self.doc)),
            ),
        };
        match mesh {
            Some(m) if !m.is_empty() => Ok((name, m)),
            _ => Err("no solid geometry to export".to_string()),
        }
    }

    fn write_mesh_file(
        &mut self,
        path: &str,
        name: &str,
        mesh: Option<crate::extrude::SolidMesh>,
        format: MeshExportFormat,
    ) -> ActionResult {
        match mesh {
            Some(m) if !m.is_empty() => {
                let contents = match format {
                    MeshExportFormat::Stl => crate::stl::write_ascii_stl(name, &m),
                    MeshExportFormat::Step => crate::step::write_step(name, &m),
                };
                match std::fs::write(path, contents) {
                    Ok(()) => {
                        self.status = format!(
                            "Exported {} triangle(s) to {} ({})",
                            m.triangles.len(),
                            path,
                            format.label()
                        );
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = format!("Export failed: {e}");
                        ActionResult::Err(self.status.clone())
                    }
                }
            }
            _ => {
                self.status = "Export failed: no solid geometry to export".to_string();
                ActionResult::Err(self.status.clone())
            }
        }
    }
}

/// Default starting extrusion distance (mm).
pub const DEFAULT_EXTRUDE_DISTANCE: f32 = 10.0;

/// The sketch a face (rect/circle/polygon profile) belongs to.
pub(crate) fn extrude_face_sketch(doc: &Document, face: &ExtrudeFace) -> Option<SketchId> {
    match face {
        ExtrudeFace::Circle(i) => doc.circles.get(*i).map(|c| c.sketch),
        ExtrudeFace::Polygon(lines) => lines.first().and_then(|&i| doc.lines.get(i)).map(|l| l.sketch),
        // `a`/`b` always share the same sketch (that's the whole premise of combining them),
        // so either side resolves it.
        ExtrudeFace::Boolean { a, .. } => extrude_face_sketch(doc, a),
    }
}

/// Extrude a bare 3D body face directly, no separate sketch (#122) — the way many CAD tools
/// let you drag a face straight off a solid. Creates an implicit sketch hosted on `face_id`
/// (an `ExtrudeCap`/`ExtrudeSide` — anything else is rejected) and mirrors that face's exact
/// boundary into it: a circular cap gets a real `Circle` (same radius) rather than a
/// tessellated approximation, everything else (side walls, polygon caps) gets a
/// [`crate::construction::add_line_polygon`] loop matching
/// [`crate::extrude::face_boundary_loop_world`] point-for-point. Returns the new profile as
/// an `ExtrudeFace`.
fn create_implicit_extrude_sketch(
    doc: &mut Document,
    face_id: FaceId,
) -> Result<ExtrudeFace, String> {
    if !matches!(face_id, FaceId::ExtrudeCap { .. } | FaceId::ExtrudeSide { .. }) {
        return Err("Not a body face".to_string());
    }
    let frame = crate::face::sketch_frame(doc, face_id.clone())
        .ok_or_else(|| "Body face does not exist".to_string())?;

    // A circular cap keeps its exact circle, not a many-sided polygon approximation.
    if let FaceId::ExtrudeCap { profile: ExtrudeFace::Circle(i), .. } = &face_id {
        let radius = doc
            .circles
            .get(*i)
            .ok_or_else(|| "Source circle no longer exists".to_string())?
            .r;
        let world_loop = crate::extrude::face_boundary_loop_world(doc, &face_id)
            .ok_or_else(|| "Body face has no boundary".to_string())?;
        let center_world =
            world_loop.iter().copied().sum::<Vec3>() / world_loop.len().max(1) as f32;
        let (cx, cy) = crate::face::world_to_local(&frame, center_world);
        let sketch = doc.add_sketch(face_id);
        doc.circles
            .push(crate::model::Circle::from_local_center_radius(sketch, cx, cy, radius, 0.0));
        doc.shape_order.push(ShapeKind::Circle);
        return Ok(ExtrudeFace::Circle(doc.circles.len() - 1));
    }

    let world_loop = crate::extrude::face_boundary_loop_world(doc, &face_id)
        .ok_or_else(|| "Body face has no boundary".to_string())?;
    if world_loop.len() < 3 {
        return Err("Body face has no boundary".to_string());
    }
    let local_points: Vec<(f32, f32)> = world_loop
        .iter()
        .map(|&p| crate::face::world_to_local(&frame, p))
        .collect();
    let sketch = doc.add_sketch(face_id);
    let lines = crate::construction::add_line_polygon(doc, sketch, &local_points);
    Ok(ExtrudeFace::Polygon(lines))
}

/// The body that a fresh extrusion on `sketch` would join by default, if `sketch` lies on an
/// existing body's face (sketching on a face of a body continues that body unless the user
/// chooses otherwise in the context pane, #32).
/// Validate a scripted extrude snap target (#114): the referenced plane/face/vertex
/// must resolve to real geometry, mirroring how #112 validates faces at commit.
fn validate_extrude_target(
    doc: &Document,
    target: &crate::model::ExtrudeTarget,
) -> std::result::Result<(), String> {
    use crate::model::ExtrudeTarget;
    match target {
        ExtrudeTarget::Plane(i) => doc
            .construction_planes
            .get(*i)
            .filter(|p| !p.deleted)
            .map(|_| ())
            .ok_or_else(|| format!("Extrude target: no construction plane {i}")),
        ExtrudeTarget::Face(face) => crate::extrude::face_profile_world(doc, face)
            .map(|_| ())
            .ok_or_else(|| "Extrude target face does not exist".to_string()),
        ExtrudeTarget::Vertex(point) => {
            crate::extrude::constraint_point_world(doc, point.clone())
                .map(|_| ())
                .ok_or_else(|| "Extrude target vertex does not exist".to_string())
        }
        ExtrudeTarget::BodyFace(face_id) => {
            if !matches!(
                face_id,
                crate::model::FaceId::ExtrudeCap { .. } | crate::model::FaceId::ExtrudeSide { .. }
            ) {
                return Err(
                    "Extrude target: body face must be an extrusion cap or side wall".to_string(),
                );
            }
            crate::face::sketch_frame(doc, face_id.clone())
                .map(|_| ())
                .ok_or_else(|| "Extrude target body face does not exist".to_string())
        }
    }
}

fn extrude_merge_candidate(doc: &Document, sketch: SketchId) -> Option<usize> {
    let face = doc.sketch_face(sketch)?;
    let extrusion = match face {
        FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => extrusion,
        _ => return None,
    };
    crate::model::body_index_for_extrusion(doc, extrusion)
}

/// Corner index (0–3) of `rect` nearest to local point `(u, v)`.
/// Nearest rectangle corner (0=BL, 1=BR, 2=TR, 3=TL, matching `add_line_rectangle`) to a
/// local point, used to map a snapped placement onto the shared line endpoint at that corner.
fn rect_corner_index_at(x: f32, y: f32, w: f32, h: f32, u: f32, v: f32) -> u8 {
    let corners = [
        (x, y),
        (x + w, y),
        (x + w, y + h),
        (x, y + h),
    ];
    let mut best = 0u8;
    let mut best_d = f32::INFINITY;
    for (i, (cu, cv)) in corners.iter().enumerate() {
        let d = (cu - u).powi(2) + (cv - v).powi(2);
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
    }
    best
}

fn pane_status(pane: Pane, visible: bool) -> String {
    format!("{} {}", pane.label(), if visible { "shown" } else { "hidden" })
}

fn curve_mode_status(curve_mode: bool) -> String {
    format!("Line curve mode: {}", if curve_mode { "on" } else { "off" })
}

fn tangent_constraint_status(tangent_constraint: bool) -> String {
    format!(
        "Tangent constraint: {}",
        if tangent_constraint { "on" } else { "off" }
    )
}

/// Computes updated bezier handles for the shared vertex `v` between a chained line-tool
/// segment and the previous committed line it starts from (#73). `prev_far` is the previous
/// line's own far endpoint (its start); `prev_bezier_baseline` is that line's `bezier` value
/// before any of this segment's live preview touched it; `b` is this segment's far endpoint
/// (live mouse while drawing, or the actual commit point). Returns the previous line's
/// updated `bezier` and this segment's own `bezier`.
///
/// When `curve_mode` is off, neither side is touched (the previous line's baseline is
/// returned unchanged and this segment stays straight). When `curve_mode` is on and
/// `tangent_constraint` is on, both sides are smoothed via [`smooth_joint_bezier`] — the
/// previous line's far-from-`v` handle is preserved from its baseline (or freshly computed if
/// it wasn't already curved) and only its near-`v` handle changes. When `tangent_constraint`
/// is off, the previous line is left untouched and this segment gets independent "corner"
/// handles instead of mirrored ones.
pub(crate) fn chained_curve_handles(
    prev_far: (f32, f32),
    prev_bezier_baseline: Option<[(f32, f32); 2]>,
    v: (f32, f32),
    b: (f32, f32),
    curve_mode: bool,
    tangent_constraint: bool,
) -> (Option<[(f32, f32); 2]>, Option<[(f32, f32); 2]>) {
    if !curve_mode {
        return (prev_bezier_baseline, None);
    }
    if tangent_constraint {
        let ([h1_far, h1_near], [h2_near, h2_far]) = smooth_joint_bezier(prev_far, v, b);
        let prev0 = prev_bezier_baseline.map(|bez| bez[0]).unwrap_or(h1_far);
        (Some([prev0, h1_near]), Some([h2_near, h2_far]))
    } else {
        let near = independent_corner_handle(v, b);
        let far = independent_corner_handle(b, v);
        (prev_bezier_baseline, Some([near, far]))
    }
}

/// Whether the two lines meeting at `point` currently have mirrored, tangent-continuous
/// handles (within a small epsilon of what [`smooth_joint_bezier`] would produce) — used by
/// the `T` shortcut on a selection to decide which way to toggle (#73).
fn vertex_is_tangent_continuous(doc: &Document, sketch: SketchId, point: ConstraintPoint) -> bool {
    let Some([(line1, end1), (line2, end2)]) =
        vertex_drag::incident_two_lines(doc, sketch, point)
    else {
        return false;
    };
    let (Some(l1), Some(l2)) = (doc.lines.get(line1), doc.lines.get(line2)) else {
        return false;
    };
    let (Some(b1), Some(b2)) = (l1.bezier, l2.bezier) else {
        return false;
    };
    let (v, a) = match end1 {
        LineEnd::Start => ((l1.x0, l1.y0), (l1.x1, l1.y1)),
        LineEnd::End => ((l1.x1, l1.y1), (l1.x0, l1.y0)),
    };
    let b = match end2 {
        LineEnd::Start => (l2.x1, l2.y1),
        LineEnd::End => (l2.x0, l2.y0),
    };
    let ([_, h1_near], [h2_near, _]) = smooth_joint_bezier(a, v, b);
    let actual_h1_near = match end1 {
        LineEnd::Start => b1[0],
        LineEnd::End => b1[1],
    };
    let actual_h2_near = match end2 {
        LineEnd::Start => b2[0],
        LineEnd::End => b2[1],
    };
    const EPS: f32 = 1e-2;
    (actual_h1_near.0 - h1_near.0).abs() < EPS
        && (actual_h1_near.1 - h1_near.1).abs() < EPS
        && (actual_h2_near.0 - h2_near.0).abs() < EPS
        && (actual_h2_near.1 - h2_near.1).abs() < EPS
}

fn draw_mode_status(tool: &str, construction: bool) -> String {
    format!(
        "{tool} draw mode: {}",
        if construction {
            "construction"
        } else {
            "substantial"
        }
    )
}

fn distance_target_status_label(target: DistanceTarget) -> String {
    match target {
        DistanceTarget::LineLength(i) => format!("line {i}"),
        DistanceTarget::CircleDiameter(i) => format!("circle {i} diameter"),
        DistanceTarget::LineLineDistance { .. } => "parallel line spacing".to_string(),
        DistanceTarget::PointPointDistance { .. } => "point distance".to_string(),
        DistanceTarget::PointLineDistance { .. } => "point-line distance".to_string(),
    }
}

fn dimension_target_status_label(target: DimensionTarget) -> String {
    match target {
        DimensionTarget::Distance(distance) => distance_target_status_label(distance),
        DimensionTarget::Angle { .. } => "angle".to_string(),
    }
}

/// Shared validation for creating/editing a boolean operation. `editing` is the op being
/// edited (its own outputs are then off-limits as inputs, as is anything downstream of it).
fn validate_boolean_inputs(
    doc: &Document,
    kind: crate::model::BooleanOpKind,
    a: &[usize],
    b: &[usize],
    editing: Option<usize>,
) -> Result<(), String> {
    use crate::model::BooleanOpKind;
    if a.is_empty() {
        return Err(match kind {
            BooleanOpKind::Combine => "Pick at least two bodies to combine".to_string(),
            _ => "Pick at least one body for side A".to_string(),
        });
    }
    if kind == BooleanOpKind::Combine {
        if a.len() < 2 {
            return Err("Pick at least two bodies to combine".to_string());
        }
        if !b.is_empty() {
            return Err("Combine uses a single picker; side B must be empty".to_string());
        }
    } else if b.is_empty() {
        return Err(format!("{} needs at least one body on side B", kind.label()));
    }
    // While editing, the op's own current inputs are shadow bodies — re-picking them is
    // the normal case, not a conflict.
    let editing_inputs: Vec<usize> = editing
        .and_then(|e| doc.boolean_ops.get(e))
        .map(|o| o.a.iter().chain(o.b.iter()).copied().collect())
        .unwrap_or_default();
    for &bi in a.iter().chain(b.iter()) {
        let Some(body) = doc.bodies.get(bi).filter(|body| !body.deleted) else {
            return Err(format!("Body {bi} not found"));
        };
        if body.shadow && !editing_inputs.contains(&bi) {
            return Err(format!("Body {bi} is already consumed by another operation"));
        }
        if let crate::model::BodySource::Boolean { op, .. } = body.source {
            // An op may consume outputs of *earlier* ops only, so the dependency graph
            // stays acyclic (edit included).
            if editing.is_some_and(|e| op >= e) {
                return Err("Cannot use this operation's own (or a later) result as an input".to_string());
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for &bi in a.iter().chain(b.iter()) {
        if !seen.insert(bi) {
            return Err(format!("Body {bi} is picked twice"));
        }
    }
    Ok(())
}

/// Shared validation for creating/editing a move operation.
fn validate_move_inputs(
    doc: &Document,
    targets: &[usize],
    editing: Option<usize>,
) -> Result<(), String> {
    if targets.is_empty() {
        return Err("Pick at least one body to move".to_string());
    }
    let editing_inputs: Vec<usize> = editing
        .and_then(|e| doc.move_ops.get(e))
        .map(|o| o.targets.clone())
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for &bi in targets {
        let Some(body) = doc.bodies.get(bi).filter(|body| !body.deleted) else {
            return Err(format!("Body {bi} not found"));
        };
        if body.shadow && !editing_inputs.contains(&bi) {
            return Err(format!("Body {bi} is already consumed by another operation"));
        }
        if let crate::model::BodySource::Moved { op, .. } = body.source {
            if editing.is_some_and(|e| op >= e) {
                return Err(
                    "Cannot use this operation's own (or a later) result as an input".to_string(),
                );
            }
        }
        if !seen.insert(bi) {
            return Err(format!("Body {bi} is picked twice"));
        }
    }
    Ok(())
}

/// Shared validation for creating/editing a repeat operation.
fn validate_repeat_inputs(
    doc: &Document,
    targets: &[usize],
    plane_targets: &[usize],
) -> Result<(), String> {
    if targets.is_empty() && plane_targets.is_empty() {
        return Err("Pick at least one body or plane to repeat".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for &bi in targets {
        let Some(body) = doc.bodies.get(bi).filter(|body| !body.deleted) else {
            return Err(format!("Body {bi} not found"));
        };
        if body.shadow {
            return Err(format!("Body {bi} is consumed by another operation"));
        }
        if matches!(body.source, crate::model::BodySource::Repeated { .. }) {
            return Err("Cannot repeat a repeat output; edit the repeat instead".to_string());
        }
        if !seen.insert(bi) {
            return Err(format!("Body {bi} is picked twice"));
        }
    }
    let mut seen_planes = std::collections::HashSet::new();
    for &pi in plane_targets {
        let Some(plane) = doc.construction_planes.get(pi).filter(|p| !p.deleted) else {
            return Err(format!("Plane {pi} not found"));
        };
        if plane.repeat_instance.is_some() {
            return Err("Cannot repeat a repeat instance; edit the repeat instead".to_string());
        }
        if !seen_planes.insert(pi) {
            return Err(format!("Plane {pi} is picked twice"));
        }
    }
    Ok(())
}

/// Apply any inline `name = expr` parameter definitions in a tool's value-input text fields
/// before they are evaluated or stored (#201). Empty fields are skipped; the first parse
/// error is returned so the caller can restore its in-progress state. Every tool that reads a
/// typed value should route its text through this (or [`try_commit_inline_parameter_definition`]
/// directly) so expressions *and* variable definitions work everywhere by default.
pub(crate) fn commit_inline_parameter_defs<'a>(
    doc: &mut Document,
    texts: impl IntoIterator<Item = &'a mut String>,
) -> Result<(), String> {
    for text in texts {
        if !text.trim().is_empty() {
            try_commit_inline_parameter_definition(doc, text)?;
        }
    }
    Ok(())
}

/// Shared validation for creating/editing a slice operation. `editing` is the op being
/// edited (its own inputs are shadow bodies, so re-picking them is allowed).
fn validate_slice_inputs(
    doc: &Document,
    targets: &[usize],
    cutters: &[FaceId],
    editing: Option<usize>,
) -> Result<(), String> {
    if targets.is_empty() {
        return Err("Pick at least one body to slice".to_string());
    }
    if cutters.is_empty() {
        return Err("Pick at least one cutting plane or face".to_string());
    }
    let editing_inputs: Vec<usize> = editing
        .and_then(|e| doc.slice_ops.get(e))
        .map(|o| o.targets.clone())
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for &bi in targets {
        let Some(body) = doc.bodies.get(bi).filter(|body| !body.deleted) else {
            return Err(format!("Body {bi} not found"));
        };
        if body.shadow && !editing_inputs.contains(&bi) {
            return Err(format!("Body {bi} is already consumed by another operation"));
        }
        if let crate::model::BodySource::Sliced { op, .. } = body.source {
            if editing.is_some_and(|e| op >= e) {
                return Err(
                    "Cannot use this operation's own (or a later) result as an input".to_string(),
                );
            }
        }
        if !seen.insert(bi) {
            return Err(format!("Body {bi} is picked twice"));
        }
    }
    // Cutters must be planar faces the kernel can build a plane from (construction planes
    // or planar body faces).
    for cutter in cutters {
        if crate::face::sketch_frame(doc, cutter.clone()).is_none() {
            return Err("A cutter is not a valid planar face".to_string());
        }
    }
    Ok(())
}

fn element_label(element: SceneElement) -> String {
    match element {
        SceneElement::ConstructionPlane(i) => format!("Construction plane {i}"),
        SceneElement::Sketch(i) => format!("Sketch {i}"),
        SceneElement::Line(i) => format!("Line {i}"),
        SceneElement::Circle(i) => format!("Circle {i}"),
        SceneElement::Constraint(i) => format!("Constraint {i}"),
        SceneElement::Point(_) => "Point".to_string(),
        SceneElement::Extrusion(i) => format!("Extrusion {i}"),
        SceneElement::Body(i) => format!("Body {i}"),
        SceneElement::FaceEdge(_) => "Face edge".to_string(),
        SceneElement::BodyEdge { .. } => "Body edge".to_string(),
        SceneElement::BodyVertex { .. } => "Body vertex".to_string(),
        SceneElement::Image(i) => format!("Image {i}"),
        SceneElement::BooleanOp(i) => format!("Boolean operation {i}"),
        SceneElement::MoveOp(i) => format!("Move operation {i}"),
        SceneElement::RepeatOp(i) => format!("Repeat operation {i}"),
        SceneElement::SliceOp(i) => format!("Slice operation {i}"),
        SceneElement::Revolution(i) => format!("Revolve operation {i}"),
        SceneElement::Origin => "Origin".to_string(),
    }
}

fn require_construction_targets_editable(
    health: &DocumentHealth,
    selection: &SceneSelection,
) -> Result<(), String> {
    for element in construction_targets_from_selection(selection) {
        require_element_editable(health, element)?;
    }
    Ok(())
}

/// Result of dispatching an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionResult {
    Ok,
    /// Action needs a file path from a dialog (GUI-only).
    NeedsDialog,
    Err(String),
}

impl AppState {
    /// Whether committed sketch dimensions can be edited or repositioned.
    pub fn can_edit_sketch_dimensions(&self) -> bool {
        self.sketch_session.is_some()
            && self.creating_rect.is_none()
            && self.creating_line.is_none()
            && self.creating_circle.is_none()
    }

    /// Start editing a dimension on the current selection, if applicable.
    pub fn try_begin_dimension_from_selection(&mut self) -> bool {
        let Some(session) = self.sketch_session else {
            return false;
        };
        let Some(target) =
            dimension_edit_from_selection(&self.doc, session.sketch, &self.scene_selection)
        else {
            return false;
        };
        if let DimensionTarget::Angle {
            line_a,
            line_b,
            rotation_sign,
        } = &target
        {
            if crate::constraints::find_angle_constraint(&self.doc, line_a.clone(), line_b.clone())
                .is_none()
            {
                self.placing_angle_dimension = Some(PlacingAngleDimension {
                    line_a: line_a.clone(),
                    line_b: line_b.clone(),
                    rotation_sign: *rotation_sign,
                    arc_offset: None,
                });
                self.status =
                    "Move the mouse to choose the angle, then click to place".to_string();
                return true;
            }
        }
        self.start_committed_dimension_edit(target);
        true
    }

    fn start_committed_dimension_edit(&mut self, target: DimensionTarget) {
        if self.sketch_session.is_none()
            || require_dimension_target_editable(&self.document_health, &self.doc, target.clone())
                .is_err()
        {
            return;
        }
        let edit_target = if let Some(id) = find_dimension_constraint(&self.doc, target.clone()) {
            DimEditTarget::Constraint(id)
        } else {
            DimEditTarget::New(target.clone())
        };
        let sketch = self.sketch_session.map(|s| s.sketch).unwrap_or_default();
        let text = match &edit_target {
            DimEditTarget::Constraint(id) => committed_dim_expression(&self.doc, *id)
                .unwrap_or_else(|| default_dimension_expression(&self.doc, sketch, target.clone())),
            DimEditTarget::New(_) => default_dimension_expression(&self.doc, sketch, target.clone()),
        };
        let kind_label = match target {
            DimensionTarget::Distance(_) => "length",
            DimensionTarget::Angle { .. } => "angle",
        };
        self.editing_committed_dim = Some(EditingCommittedDim {
            target: edit_target,
            text,
            pending_focus: true,
            dim_offset: None,
        });
        self.status = format!(
            "Dimension {} • type {kind_label} • Enter commit • Esc cancel",
            dimension_target_status_label(target)
        );
    }

    /// Active or pending construction draw mode while the rectangle tool is selected.
    pub fn rect_draw_construction_mode(&self) -> Option<bool> {
        if self.tool != Tool::Rectangle {
            return None;
        }
        Some(
            self.creating_rect
                .as_ref()
                .map(|cr| cr.construction)
                .unwrap_or(self.draw_construction),
        )
    }

    /// Active or pending construction draw mode while the line tool is selected.
    pub fn line_draw_construction_mode(&self) -> Option<bool> {
        if self.tool != Tool::Line {
            return None;
        }
        Some(
            self.creating_line
                .as_ref()
                .map(|cl| cl.construction)
                .unwrap_or(self.draw_construction),
        )
    }

    /// Active or pending curve-mode (`B`) toggle while the line tool is selected (#73).
    pub fn line_curve_mode(&self) -> Option<bool> {
        if self.tool != Tool::Line {
            return None;
        }
        Some(
            self.creating_line
                .as_ref()
                .map(|cl| cl.curve_mode)
                .unwrap_or(self.draw_curve_mode),
        )
    }

    /// Active or pending tangent-constraint (`T`) toggle while the line tool is selected (#73).
    pub fn line_tangent_constraint(&self) -> Option<bool> {
        if self.tool != Tool::Line {
            return None;
        }
        Some(
            self.creating_line
                .as_ref()
                .map(|cl| cl.tangent_constraint)
                .unwrap_or(self.draw_tangent_constraint),
        )
    }

    /// Active or pending construction draw mode while the circle tool is selected.
    pub fn circle_draw_construction_mode(&self) -> Option<bool> {
        if self.tool != Tool::Circle {
            return None;
        }
        Some(
            self.creating_circle
                .as_ref()
                .map(|cc| cc.construction)
                .unwrap_or(self.draw_construction),
        )
    }

    pub fn apply(&mut self, action: Action) -> ActionResult {
        // Undo-group bookkeeping (#105): the OUTERMOST apply of a user-level action
        // records how much it grew `shape_order`; that growth becomes one undo group,
        // so `UndoLast` reverts the whole gesture (a rectangle's 4 lines + constraints
        // undo as a single step). Nested apply() calls — arms delegating to other
        // actions — stay inside the outer group. Reconciliation keeps the sizes
        // summing to `shape_order.len()` across legacy documents, out-of-band edits,
        // and net-zero mutations (e.g. a chamfer replacing a constraint entry with a
        // bridge-line entry), degrading gracefully to single-entry groups.
        let outermost = self.undo_group_depth == 0;
        let before = self.doc.shape_order.len();
        // Checkpoint undo (#194): snapshot the document before a mutating user action, so
        // undo restores the exact prior state instead of replaying per-entry reversals.
        // Cloning is skipped for the actions that never mutate the document (and for
        // undo/redo themselves); if a snapshotted action turns out to leave the document
        // unchanged, the snapshot is discarded, so no empty undo steps accumulate.
        let snapshot = (outermost && action.snapshots_for_undo()).then(|| self.doc.clone());
        let resets_history = outermost && action.resets_undo_history();
        if outermost {
            self.reconcile_undo_groups_to(before);
        }
        self.undo_group_depth += 1;
        let result = self.apply_inner(action);
        self.undo_group_depth = self.undo_group_depth.saturating_sub(1);
        if outermost {
            let after = self.doc.shape_order.len();
            if after > before {
                self.doc.undo_groups.push(after - before);
            } else {
                // Shrunk or replaced (UndoLast consumed its own group; New/Open
                // swapped the document): re-establish the sum invariant.
                self.reconcile_undo_groups_to(after);
            }
        }
        if resets_history {
            self.undo_stack.clear();
            self.redo_stack.clear();
        } else if let Some(snap) = snapshot {
            if self.doc != snap {
                self.undo_stack.push(snap);
                if self.undo_stack.len() > MAX_UNDO_SNAPSHOTS {
                    self.undo_stack.remove(0);
                }
                self.redo_stack.clear();
            }
        }
        result
    }

    /// Make `undo_groups` sum to exactly `len` (#105): excess is trimmed from the
    /// newest groups; any shortfall (legacy files, out-of-band `shape_order` pushes)
    /// is padded as single-entry groups, which is precisely the pre-#105 per-entry
    /// undo behavior for that content.
    fn reconcile_undo_groups_to(&mut self, len: usize) {
        let mut sum: usize = self.doc.undo_groups.iter().sum();
        while sum > len {
            let Some(last) = self.doc.undo_groups.last_mut() else { break };
            let excess = sum - len;
            if *last <= excess {
                sum -= *last;
                self.doc.undo_groups.pop();
            } else {
                *last -= excess;
                sum = len;
            }
        }
        while sum < len {
            self.doc.undo_groups.push(1);
            sum += 1;
        }
    }

    /// Fix up session state after `UndoLast`/`RedoLast` swaps the whole document (#194):
    /// drop selection and any in-progress dimension edit that referenced the old document,
    /// close a sketch session whose sketch no longer exists, and recompute health.
    fn after_history_restore(&mut self) {
        self.editing_committed_dim = None;
        self.placing_angle_dimension = None;
        self.scene_selection.clear();
        if let Some(session) = self.sketch_session {
            let alive = self
                .doc
                .sketches
                .get(session.sketch)
                .is_some_and(|s| !s.deleted);
            if !alive {
                self.sketch_session = None;
            }
        }
        self.refresh_document_health();
    }

    fn apply_inner(&mut self, action: Action) -> ActionResult {
        let logged_action = self.command_log.is_some().then(|| action.clone());
        if let Some(log) = &self.command_log {
            log.borrow_mut().before_apply(&action, &self.doc, &self.cam);
        }
        let result = match action {
            Action::NewDocument => {
                self.doc = Document::default();
                self.path = None;
                self.sketch_session = None;
                self.cam.set_view_up(None);
                self.creating_rect = None;
                self.creating_line = None;
                self.creating_circle = None;
                self.creating_plane = None;
                self.element_visibility = ElementVisibility::default();
                self.scene_selection.clear();
                self.tool = Tool::Select;
                self.document_health = DocumentHealth::default();
                self.status = "New document".to_string();
                ActionResult::Ok
            }
            Action::Open { path } => match crate::storage::open(&path) {
                Ok(mut doc) => {
                    if let Err(e) = recompute_document_geometry(&mut doc) {
                        self.status = format!("Open failed: {e}");
                        return ActionResult::Err(e);
                    }
                    let n_lines = doc.lines.len();
                    self.doc = doc;
                    self.sketch_session = None;
                    self.cam.set_view_up(None);
                    self.refresh_document_health();
                    self.path = Some(path.clone());
                    self.status = format!("Opened {} ({} line(s))", path, n_lines);
                    ActionResult::Ok
                }
                Err(e) => {
                    self.status = format!("Open failed: {e}");
                    ActionResult::Err(e)
                }
            },
            Action::Save { path } => {
                let target = path.or_else(|| self.path.clone());
                match target {
                    Some(p) => self.write_to(&p),
                    None => ActionResult::NeedsDialog,
                }
            }
            Action::ExportStl { path, body } => {
                let (name, mesh) = match &body {
                    Some(name) => {
                        match self.doc.bodies.iter().position(|b| {
                            !b.deleted && b.name.as_deref() == Some(name.as_str())
                        }) {
                            Some(bi) => {
                                (name.clone(), crate::extrude::body_solid_mesh(&self.doc, bi))
                            }
                            None => {
                                self.status = format!("Export failed: no body named '{name}'");
                                return ActionResult::Err(self.status.clone());
                            }
                        }
                    }
                    None => (
                        "bearcad".to_string(),
                        Some(crate::extrude::document_solid_mesh(&self.doc)),
                    ),
                };
                self.write_stl_file(&path, &name, mesh)
            }
            Action::ExportDrawingSvg { drawing, path } => {
                let Some(svg) = crate::drawing::drawing_to_svg(&self.doc, drawing) else {
                    self.status = format!("Export failed: no drawing {drawing}");
                    return ActionResult::Err(self.status.clone());
                };
                match std::fs::write(&path, svg) {
                    Ok(()) => {
                        self.status = format!("Exported drawing {drawing} to {path}");
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = format!("Export failed: {e}");
                        ActionResult::Err(self.status.clone())
                    }
                }
            }
            Action::ExportDrawingPdf { drawing, path } => {
                let Some(pdf) = crate::drawing::drawing_to_pdf(&self.doc, drawing) else {
                    self.status = format!("Export failed: no drawing {drawing}");
                    return ActionResult::Err(self.status.clone());
                };
                match std::fs::write(&path, pdf) {
                    Ok(()) => {
                        self.status = format!("Exported drawing {drawing} to {path}");
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = format!("Export failed: {e}");
                        ActionResult::Err(self.status.clone())
                    }
                }
            }
            Action::ExportStlBody { path, body } => {
                let Some(b) = self.doc.bodies.get(body).filter(|b| !b.deleted) else {
                    self.status = format!("Export failed: no body {body}");
                    return ActionResult::Err(self.status.clone());
                };
                let name = b
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("body-{body}"));
                let mesh = crate::extrude::body_solid_mesh(&self.doc, body);
                self.write_stl_file(&path, &name, mesh)
            }
            Action::ExportStep { path, body } => match &body {
                Some(name) => {
                    match self
                        .doc
                        .bodies
                        .iter()
                        .position(|b| !b.deleted && b.name.as_deref() == Some(name.as_str()))
                    {
                        Some(bi) => {
                            let name = name.clone();
                            self.write_step_body_file(&path, &name, bi)
                        }
                        None => {
                            self.status = format!("Export failed: no body named '{name}'");
                            ActionResult::Err(self.status.clone())
                        }
                    }
                }
                // Whole-document export: when the document holds exactly one live body,
                // route it through the per-body path so kernel builds write real BREP
                // (curved surfaces survive the round-trip, #106). Multi-body documents
                // keep the hand-rolled faceted concatenation (OCCT export is per single
                // body — see `write_step_body_file`).
                None => {
                    let mut live = self
                        .doc
                        .bodies
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| !b.deleted);
                    match (live.next(), live.next()) {
                        (Some((bi, b)), None) => {
                            let name = b
                                .name
                                .clone()
                                .unwrap_or_else(|| format!("body-{bi}"));
                            self.write_step_body_file(&path, &name, bi)
                        }
                        _ => {
                            let mesh = Some(crate::extrude::document_solid_mesh(&self.doc));
                            self.write_step_file(&path, "bearcad", mesh)
                        }
                    }
                }
            },
            Action::ExportStepBody { path, body } => {
                let Some(b) = self.doc.bodies.get(body).filter(|b| !b.deleted) else {
                    self.status = format!("Export failed: no body {body}");
                    return ActionResult::Err(self.status.clone());
                };
                let name = b
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("body-{body}"));
                self.write_step_body_file(&path, &name, body)
            }
            Action::CalibrateImage { image, a, b, length } => {
                let Some(img) = self
                    .doc
                    .tracing_images
                    .get(image)
                    .filter(|img| !img.deleted)
                else {
                    return ActionResult::Err(format!("Image {image} not found"));
                };
                let span = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                if span < 1e-6 {
                    return ActionResult::Err("Calibration line has zero length".to_string());
                }
                if length <= 0.0 {
                    return ActionResult::Err("Calibration length must be positive".to_string());
                }
                let factor = length / span;
                // Reference segment in image-UV (of the pre-scale quad), kept for re-editing.
                let (ox, oy) = img.origin;
                let (w, h) = (img.width_mm.max(1e-6), img.height_mm.max(1e-6));
                let calibration = crate::model::ImageCalibration {
                    u0: (a.0 - ox) / w,
                    v0: (a.1 - oy) / h,
                    u1: (b.0 - ox) / w,
                    v1: (b.1 - oy) / h,
                    length_mm: length,
                };
                // Uniform rescale about the segment midpoint, so the calibrated feature
                // stays where the user drew the line.
                let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                let img = &mut self.doc.tracing_images[image];
                img.origin = (
                    mid.0 + (img.origin.0 - mid.0) * factor,
                    mid.1 + (img.origin.1 - mid.1) * factor,
                );
                // If the image has already been moved, rescale its pristine base too so a later
                // move-op edit still recomputes to the calibrated position (#217).
                if let Some((bx, by)) = img.base_origin {
                    img.base_origin =
                        Some((mid.0 + (bx - mid.0) * factor, mid.1 + (by - mid.1) * factor));
                }
                img.width_mm *= factor;
                img.height_mm *= factor;
                img.calibration = Some(calibration);
                self.creating_calibration = None;
                self.status = format!(
                    "Calibrated image: {} (x{factor:.3})",
                    crate::value::format_length_display(length)
                );
                ActionResult::Ok
            }
            Action::BeginImageCalibration { image } => {
                if self
                    .doc
                    .tracing_images
                    .get(image)
                    .filter(|img| !img.deleted)
                    .is_none()
                {
                    return ActionResult::Err(format!("Image {image} not found"));
                }
                // Calibration point-placing takes over viewport clicks, so make sure no
                // drawing tool is armed underneath it.
                self.tool = Tool::Select;
                self.creating_calibration = Some(CreatingCalibration {
                    image,
                    points: Vec::new(),
                });
                self.status =
                    "Calibrate: click two points on the image over a feature of known size"
                        .to_string();
                ActionResult::Ok
            }
            Action::AddCalibrationPoint { x, y } => {
                let Some(cal) = self.creating_calibration.as_mut() else {
                    return ActionResult::Err("No calibration in progress".to_string());
                };
                if cal.points.len() >= 2 {
                    return ActionResult::Err("Both calibration points are placed".to_string());
                }
                cal.points.push((x, y));
                self.status = match cal.points.len() {
                    1 => "Calibrate: click the second point".to_string(),
                    _ => "Calibrate: type the real length of the marked span".to_string(),
                };
                ActionResult::Ok
            }
            Action::ImportImage { path, plane } => {
                let bytes = match std::fs::read(&path) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = format!("Import failed: {e}");
                        return ActionResult::Err(self.status.clone());
                    }
                };
                let dims = match image::load_from_memory(&bytes) {
                    Ok(img) => (img.width() as f32, img.height() as f32),
                    Err(e) => {
                        self.status = format!("Import failed: not a readable image ({e})");
                        return ActionResult::Err(self.status.clone());
                    }
                };
                let plane = plane.unwrap_or(0);
                if !self
                    .doc
                    .construction_planes
                    .get(plane)
                    .is_some_and(|p| !p.deleted)
                {
                    self.status = format!("Import failed: construction plane {plane} not found");
                    return ActionResult::Err(self.status.clone());
                }
                let source_name = std::path::Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "image".to_string());
                self.doc.tracing_images.push(crate::model::TracingImage {
                    bytes,
                    source_name: source_name.clone(),
                    plane,
                    // Centered on the plane origin at 1 px = 1 mm.
                    origin: (-dims.0 / 2.0, -dims.1 / 2.0),
                    width_mm: dims.0,
                    height_mm: dims.1,
                    name: None,
                    deleted: false,
                    calibration: None,
                    base_origin: None,
                });
                self.doc.shape_order.push(crate::model::ShapeKind::Image);
                self.refresh_document_health();
                self.status = format!(
                    "Imported image {source_name} ({} x {} px)",
                    dims.0 as u32, dims.1 as u32
                );
                ActionResult::Ok
            }
            Action::ImportStl { path } => {
                let bytes = match std::fs::read(&path) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = format!("Import failed: {e}");
                        return ActionResult::Err(self.status.clone());
                    }
                };
                match crate::stl::parse_stl(&bytes) {
                    Ok(tris) => self.import_mesh_body(
                        &path,
                        tris.into_iter().map(|t| t.vertices).collect(),
                    ),
                    Err(e) => {
                        self.status = format!("Import failed: {e}");
                        ActionResult::Err(self.status.clone())
                    }
                }
            }
            Action::ImportStep { path } => {
                // In `occt` builds, read real BREP (curved surfaces included) via
                // STEPControl_Reader and tessellate it (#71). Falls back to the hand-rolled
                // faceted-subset parser when the kernel isn't compiled in or can't read the
                // file (e.g. missing/empty/not-a-solid).
                #[cfg(feature = "occt")]
                {
                    if let Some(shape) = crate::kernel::Shape::read_step(std::path::Path::new(&path))
                    {
                        let tris = shape.tessellate(crate::extrude::OCCT_DEFLECTION as f64);
                        if !tris.is_empty() {
                            return self.import_mesh_body(&path, tris);
                        }
                    }
                }
                let text = match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(e) => {
                        self.status = format!("Import failed: {e}");
                        return ActionResult::Err(self.status.clone());
                    }
                };
                match crate::step::parse_step_mesh(&text) {
                    Ok(triangles) => self.import_mesh_body(&path, triangles),
                    Err(e) => {
                        self.status = format!("Import failed: {e}");
                        ActionResult::Err(self.status.clone())
                    }
                }
            }
            Action::Clear => {
                self.doc = Document::default();
                self.sketch_session = None;
                self.cam.set_view_up(None);
                self.creating_rect = None;
                self.creating_line = None;
                self.creating_circle = None;
                self.element_visibility = ElementVisibility::default();
                self.document_health = DocumentHealth::default();
                self.status = "Cleared".to_string();
                ActionResult::Ok
            }
            Action::UndoLast => {
                // Checkpoint undo (#194): restore the snapshot taken before the last mutating
                // action. Provably correct — it reinstates the exact prior document — which is
                // why it fixes the fillet-undo-deletes-the-line class of bug that the old
                // per-entry reversal machine was prone to.
                if let Some(prev) = self.undo_stack.pop() {
                    let current = std::mem::replace(&mut self.doc, prev);
                    self.redo_stack.push(current);
                    self.after_history_restore();
                    self.status = "Undid last action".to_string();
                } else {
                    self.status = "Nothing to undo".to_string();
                }
                ActionResult::Ok
            }
            Action::RedoLast => {
                // Redo (#193): re-apply the most recently undone document snapshot.
                if let Some(next) = self.redo_stack.pop() {
                    let current = std::mem::replace(&mut self.doc, next);
                    self.undo_stack.push(current);
                    self.after_history_restore();
                    self.status = "Redid last action".to_string();
                } else {
                    self.status = "Nothing to redo".to_string();
                }
                ActionResult::Ok
            }
            Action::ToggleFpsMode => {
                if let Some(exited) = self.fps.take() {
                    self.fps_memory = Some(exited);
                    self.status = "Left FPS mode".to_string();
                } else {
                    let player =
                        crate::fps::FpsController::enter(&self.cam, self.fps_memory.as_ref());
                    player.apply_to_camera(&mut self.cam);
                    self.fps = Some(player);
                    self.status = "FPS mode — WASD walk, mouse look, Space jump \
                                   (double-tap to fly), 1-9 tools, wheel cycles, Esc exits"
                        .to_string();
                }
                ActionResult::Ok
            }
            Action::SetTool(tool) => {
                if self.creating_rect.is_some() && tool != Tool::Rectangle {
                    self.creating_rect = None;
                }
                if self.creating_line.is_some() && tool != Tool::Line {
                    self.discard_creating_line();
                }
                if self.creating_circle.is_some() && tool != Tool::Circle {
                    self.creating_circle = None;
                }
                if self.creating_plane.is_some() && tool != Tool::ConstructionPlane {
                    self.creating_plane = None;
                }
                if self.creating_extrusion.is_some() && tool != Tool::Extrude {
                    self.creating_extrusion = None;
                }
                if self.creating_vertex_treatment.is_some()
                    && !matches!(tool, Tool::Chamfer | Tool::Fillet)
                {
                    self.creating_vertex_treatment = None;
                }
                if self.creating_edge_treatment.is_some()
                    && !matches!(tool, Tool::Chamfer | Tool::Fillet)
                {
                    self.creating_edge_treatment = None;
                }
                if self.creating_loft.is_some() && tool != Tool::Loft {
                    self.creating_loft = None;
                }
                if self.creating_boolean.is_some() && tool != Tool::Combine {
                    self.creating_boolean = None;
                }
                if tool == Tool::Combine && self.creating_boolean.is_none() {
                    self.creating_boolean = Some(CreatingBoolean::default());
                }
                if self.creating_move.is_some() && tool != Tool::Move {
                    self.creating_move = None;
                }
                if tool == Tool::Move && self.creating_move.is_none() {
                    self.creating_move = Some(CreatingMove::default());
                }
                if self.creating_repeat.is_some() && tool != Tool::Repeat {
                    self.creating_repeat = None;
                }
                if tool == Tool::Repeat && self.creating_repeat.is_none() {
                    self.creating_repeat = Some(CreatingRepeat::default());
                }
                if self.creating_slice.is_some() && tool != Tool::Slice {
                    self.creating_slice = None;
                }
                if tool == Tool::Slice && self.creating_slice.is_none() {
                    self.creating_slice = Some(CreatingSlice::default());
                }
                if self.creating_revolve.is_some() && tool != Tool::Revolve {
                    self.creating_revolve = None;
                }
                if self.creating_calibration.is_some() && tool != Tool::Select {
                    self.creating_calibration = None;
                }
                // #157/#166: switching to Chamfer/Fillet with body edges already selected
                // preloads them (filtered to treatable edges) so the gizmo shows right away.
                if matches!(tool, Tool::Chamfer | Tool::Fillet)
                    && self.sketch_session.is_none()
                    && self.creating_edge_treatment.is_none()
                {
                    let edges =
                        crate::extrude::treatable_edges_in_selection(&self.doc, &self.scene_selection);
                    if !edges.is_empty() {
                        self.creating_edge_treatment = Some(CreatingEdgeTreatment {
                            edges,
                            kind: if tool == Tool::Chamfer {
                                VertexTreatmentKind::Chamfer
                            } else {
                                VertexTreatmentKind::Fillet
                            },
                            amount_live: DEFAULT_VERTEX_TREATMENT_AMOUNT,
                            text: crate::value::format_length_display(
                                DEFAULT_VERTEX_TREATMENT_AMOUNT,
                            ),
                            user_edited: false,
                            pending_focus: true,
                        });
                    }
                }
                // Extruding/lofting act on the 3D model, not sketch geometry: leave
                // sketch editing when either tool is picked from inside a sketch.
                if matches!(tool, Tool::Extrude | Tool::Loft | Tool::Revolve)
                    && self.sketch_session.is_some()
                {
                    self.exit_sketch_session();
                }
                // Switching to Loft with profiles already selected preloads them as
                // sections (mirrors the Chamfer/Fillet preload above).
                if tool == Tool::Loft && self.creating_loft.is_none() {
                    let sections = crate::extrude::loft_sections_from_selection(
                        &self.doc,
                        &self.scene_selection,
                    );
                    self.creating_loft = Some(CreatingLoft { sections });
                }
                if !matches!(tool, Tool::Select | Tool::Dimension | Tool::Constraint) {
                    self.editing_committed_dim = None;
                }
                if tool != Tool::Dimension {
                    self.placing_angle_dimension = None;
                }
                self.tool = tool;
                self.status = match tool {
                    Tool::Select => {
                        "Select tool — Delete/Backspace removes selection".to_string()
                    }
                    Tool::Sketch => "Sketch tool — click a face".to_string(),
                    Tool::Rectangle if self.sketch_session.is_some() => {
                        "Rectangle tool".to_string()
                    }
                    Tool::Rectangle => "Rectangle tool — click a face".to_string(),
                    Tool::Line if self.sketch_session.is_some() => "Line tool".to_string(),
                    Tool::Line => "Line tool — click a face".to_string(),
                    Tool::Circle if self.sketch_session.is_some() => "Circle tool".to_string(),
                    Tool::Circle => "Circle tool — click a face".to_string(),
                    Tool::Dimension if self.sketch_session.is_some() => {
                        "Dimension tool — select geometry, then D, or click a segment".to_string()
                    }
                    Tool::Dimension => "Dimension tool — open a sketch first".to_string(),
                    Tool::Constraint if self.sketch_session.is_some() => {
                        "Constraint tool — select geometry, then pick a constraint".to_string()
                    }
                    Tool::Constraint => "Constraint tool — open a sketch first".to_string(),
                    Tool::ConstructionPlane => "Construction plane tool".to_string(),
                    Tool::Extrude => {
                        "Extrude tool — click coplanar faces, then set a distance".to_string()
                    }
                    Tool::Chamfer if self.sketch_session.is_some() => {
                        "Chamfer tool — click a sketch vertex".to_string()
                    }
                    Tool::Chamfer => "Chamfer tool — click a body edge".to_string(),
                    Tool::Fillet if self.sketch_session.is_some() => {
                        "Fillet tool — click a sketch vertex".to_string()
                    }
                    Tool::Fillet => "Fillet tool — click a body edge".to_string(),
                    Tool::Loft => {
                        "Loft tool — click two or more closed profiles".to_string()
                    }
                    Tool::Revolve => {
                        "Revolve tool — click profile faces, then an axis line".to_string()
                    }
                    Tool::Combine => {
                        "Combine tool — click bodies to pick them, choose the operation, Enter commits"
                            .to_string()
                    }
                    Tool::Move => {
                        "Move tool — click bodies to pick them, set the offset/rotation, Enter commits"
                            .to_string()
                    }
                    Tool::Repeat => {
                        "Repeat tool — click bodies, pick an axis and spacing, Enter commits"
                            .to_string()
                    }
                    Tool::Slice => {
                        "Slice tool — pick bodies, then cutting planes/faces, Enter commits"
                            .to_string()
                    }
                };
                if tool == Tool::Dimension {
                    self.try_begin_dimension_from_selection();
                }
                ActionResult::Ok
            }
            Action::CancelOperation => {
                self.line_start_snap = None;
                self.line_end_snap = None;
                self.rect_origin_snap = None;
                self.rect_opposite_snap = None;
                self.circle_center_snap = None;
                self.extension_anchors.clear();
                self.normal_inference_anchor = None;
                if self.editing_committed_dim.take().is_some()
                    || self.placing_angle_dimension.take().is_some()
                {
                    self.status = "Cancelled".to_string();
                } else if self.creating_extrusion.take().is_some() {
                    self.status = "Cancelled extrusion".to_string();
                } else if self.creating_loft.take().is_some() {
                    self.status = "Cancelled loft".to_string();
                } else if self.creating_revolve.take().is_some() {
                    self.status = "Cancelled revolve".to_string();
                } else if self.creating_calibration.take().is_some() {
                    self.status = "Cancelled calibration".to_string();
                } else if self.creating_rect.take().is_some()
                    || self.discard_creating_line()
                    || self.creating_circle.take().is_some()
                    || self.creating_plane.take().is_some()
                    || self.creating_vertex_treatment.take().is_some()
                {
                    self.status = "Cancelled".to_string();
                } else if self.sketch_session.is_some() {
                    if self.tool == Tool::Select {
                        self.exit_sketch_session();
                        self.status = "Exited sketch".to_string();
                    } else {
                        self.creating_rect = None;
                        self.discard_creating_line();
                        self.creating_circle = None;
                        self.tool = Tool::Select;
                        self.status =
                            "Select tool — Delete/Backspace removes selection".to_string();
                    }
                } else if self.tool != Tool::Select {
                    self.tool = Tool::Select;
                    self.status =
                        "Select tool — Delete/Backspace removes selection".to_string();
                }
                ActionResult::Ok
            }
            Action::BeginSketch { face, viewport } => {
                if sketch_frame(&self.doc, face.clone()).is_none() {
                    return ActionResult::Err(format!("Unknown face {:?}", face));
                }
                let sketch = self.doc.add_sketch(face);
                self.enter_sketch(sketch, viewport, None)
            }
            Action::OpenSketch { sketch, viewport } => {
                if self.doc.sketches.get(sketch).is_none() {
                    return ActionResult::Err(format!("Unknown sketch {sketch}"));
                }
                self.enter_sketch(sketch, viewport, Some(SKETCH_EDIT_FRAME_PADDING_PX))
            }
            Action::ExitSketch => {
                if self.sketch_session.is_none() {
                    return ActionResult::Err("Not in sketch mode".to_string());
                }
                self.exit_sketch_session();
                self.status = "Sketch saved".to_string();
                ActionResult::Ok
            }
            Action::CommitRectangle => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let Some(frame) = sketch_geometry_frame(&self.doc, session.sketch) else {
                    return ActionResult::Err("Sketch no longer exists".to_string());
                };
                let Some(mut cr) = self.creating_rect.take() else {
                    return ActionResult::Err("No rectangle in progress".to_string());
                };
                for i in 0..2 {
                    if cr.user_edited[i] {
                        if let Err(e) =
                            try_commit_inline_parameter_definition(&mut self.doc, &mut cr.texts[i])
                        {
                            self.creating_rect = Some(cr);
                            self.status = e.clone();
                            return ActionResult::Err(e);
                        }
                    }
                }
                let (ou, ov) = world_to_local(&frame, cr.origin);
                let end = cr.end_point(&frame, &self.doc);
                let (eu, ev) = world_to_local(&frame, end);
                let x = ou.min(eu);
                let y = ov.min(ev);
                let w = (eu - ou).abs();
                let h = (ev - ov).abs();
                if w > 0.5 && h > 0.5 {
                    let construction_edges = if cr.construction { [true; 4] } else { [false; 4] };
                    // Snapshot for rollback if a typed width/height constraint fails to apply.
                    let lines_before = self.doc.lines.len();
                    let constraints_before = self.doc.constraints.len();
                    let shape_order_before = self.doc.shape_order.len();
                    // A rectangle is now four plain lines (bottom, right, top, left) forming a
                    // closed loop with Horizontal/Vertical/Coincident constraints (#66 polygon).
                    let lines = crate::construction::add_line_rectangle(
                        &mut self.doc,
                        session.sketch,
                        x,
                        y,
                        w,
                        h,
                        construction_edges,
                    );
                    // Corners are shared line endpoints: corner `i` is `lines[i]`'s start.
                    let origin_corner = rect_corner_index_at(x, y, w, h, ou, ov);
                    let opposite_corner = rect_corner_index_at(x, y, w, h, eu, ev);
                    let mut constraint_err = None;
                    // Width drives the bottom edge (lines[0]); height the right edge (lines[1]).
                    if cr.user_edited[0] {
                        if let Err(e) = add_distance_constraint(
                            &mut self.doc,
                            session.sketch,
                            DistanceTarget::LineLength(lines[0]),
                            cr.texts[0].clone(),
                        ) {
                            constraint_err = Some(e);
                        }
                    }
                    if constraint_err.is_none() && cr.user_edited[1] {
                        if let Err(e) = add_distance_constraint(
                            &mut self.doc,
                            session.sketch,
                            DistanceTarget::LineLength(lines[1]),
                            cr.texts[1].clone(),
                        ) {
                            constraint_err = Some(e);
                        }
                    }
                    if let Some(e) = constraint_err {
                        self.doc.constraints.truncate(constraints_before);
                        self.doc.lines.truncate(lines_before);
                        self.doc.shape_order.truncate(shape_order_before);
                        self.rect_origin_snap = None;
                        self.rect_opposite_snap = None;
                        self.creating_rect = Some(cr);
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                    // Pin corners that were left on a snap target.
                    if let Some(target) = self.rect_origin_snap.take() {
                        let _ = self.add_snap_constraint(
                            session.sketch,
                            ConstraintPoint::LineEndpoint {
                                line: lines[origin_corner as usize],
                                end: LineEnd::Start,
                            },
                            target,
                        );
                    }
                    if let Some(target) = self.rect_opposite_snap.take() {
                        let _ = self.add_snap_constraint(
                            session.sketch,
                            ConstraintPoint::LineEndpoint {
                                line: lines[opposite_corner as usize],
                                end: LineEnd::Start,
                            },
                            target,
                        );
                    }
                    let unit = crate::model::effective_length_unit(&self.doc, session.sketch);
                    self.status = format!(
                        "Added rectangle ({} × {})",
                        crate::value::format_length_display_in(w, unit),
                        crate::value::format_length_display_in(h, unit)
                    );
                    ActionResult::Ok
                } else {
                    self.rect_origin_snap = None;
                    self.rect_opposite_snap = None;
                    self.creating_rect = Some(cr);
                    self.status = "Rectangle too small".to_string();
                    ActionResult::Err("Rectangle too small".to_string())
                }
            }
            Action::SetRectDimension { axis, value } => {
                // A committed rectangle's width/height are now ordinary line-length dimensions,
                // edited through the line-dimension path; this action only drives the width/height
                // fields while the rectangle is still being drawn.
                let Some(cr) = &mut self.creating_rect else {
                    return ActionResult::Err("No rectangle in progress".to_string());
                };
                let idx = axis.index();
                cr.texts[idx] = value;
                cr.user_edited[idx] = true;
                ActionResult::Ok
            }
            Action::FocusRectDimension { axis } => {
                let Some(cr) = &mut self.creating_rect else {
                    return ActionResult::Err("No rectangle in progress".to_string());
                };
                cr.focused = axis.index();
                cr.pending_focus = true;
                ActionResult::Ok
            }
            Action::CommitLine => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let Some(frame) = sketch_geometry_frame(&self.doc, session.sketch) else {
                    return ActionResult::Err("Sketch no longer exists".to_string());
                };
                let Some(mut cl) = self.creating_line.take() else {
                    return ActionResult::Err("No line in progress".to_string());
                };
                if cl.user_edited {
                    if let Err(e) =
                        try_commit_inline_parameter_definition(&mut self.doc, &mut cl.text)
                    {
                        self.creating_line = Some(cl);
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let (u0, v0) = world_to_local(&frame, cl.origin);
                let end = cl.end_point(&frame, &self.doc);
                let (u1, v1) = world_to_local(&frame, end);
                let mut line = Line::from_local_endpoints(session.sketch, u0, v0, u1, v1);
                line.construction = cl.construction;
                if line.length() > 0.5 {
                    // #73: while curve-mode is on, retroactively smooth (or corner-ize) the
                    // joint with the previous chained segment, and give this segment matching
                    // handles. No-op (both stay as they were / `None`) when curve-mode is off.
                    if let Some(prev_idx) = cl.chained_from {
                        if let Some(prev_far) =
                            self.doc.lines.get(prev_idx).map(|l| (l.x0, l.y0))
                        {
                            let (prev_bezier, line_bezier) = chained_curve_handles(
                                prev_far,
                                cl.chained_from_bezier,
                                (u0, v0),
                                (u1, v1),
                                cl.curve_mode,
                                cl.tangent_constraint,
                            );
                            if let Some(prev) = self.doc.lines.get_mut(prev_idx) {
                                prev.bezier = prev_bezier;
                            }
                            line.bezier = line_bezier;
                        }
                    }
                    self.doc.lines.push(line);
                    self.doc.shape_order.push(ShapeKind::Line);
                    let line_index = self.doc.lines.len() - 1;
                    if cl.user_edited {
                        if let Err(e) = add_distance_constraint(
                            &mut self.doc,
                            session.sketch,
                            DistanceTarget::LineLength(line_index),
                            cl.text.clone(),
                        ) {
                            self.doc.lines.pop();
                            self.doc.shape_order.pop();
                            self.creating_line = Some(cl);
                            self.status = e.clone();
                            return ActionResult::Err(e);
                        }
                    }
                    // If the segment's end latched onto an existing vertex (or the origin),
                    // the polyline is closing/joining, so we stop chaining (#20).
                    let end_on_vertex = matches!(
                        self.line_end_snap,
                        Some(crate::snapping::SnapTarget::Vertex(_))
                            | Some(crate::snapping::SnapTarget::Origin)
                    );
                    // Pin endpoints that were left on a snap target.
                    if let Some(target) = self.line_start_snap.take() {
                        let _ = self.add_snap_constraint(
                            session.sketch,
                            ConstraintPoint::LineEndpoint {
                                line: line_index,
                                end: LineEnd::Start,
                            },
                            target,
                        );
                    }
                    if let Some(target) = self.line_end_snap.take() {
                        let _ = self.add_snap_constraint(
                            session.sketch,
                            ConstraintPoint::LineEndpoint {
                                line: line_index,
                                end: LineEnd::End,
                            },
                            target,
                        );
                    }
                    let len = self.doc.lines.last().unwrap().length();
                    let len_label = crate::value::format_length_display_in(
                        len,
                        crate::model::effective_length_unit(&self.doc, session.sketch),
                    );
                    // Chain into the next segment: start a new line at this endpoint so polygons
                    // can be drawn with successive clicks. The new start snaps to the just-placed
                    // endpoint (coincident on commit), keeping the polyline connected. Skip this
                    // when we closed onto an existing vertex (#20).
                    if self.tool == Tool::Line && !end_on_vertex {
                        self.line_start_snap = Some(crate::snapping::SnapTarget::Vertex(
                            ConstraintPoint::LineEndpoint {
                                line: line_index,
                                end: LineEnd::End,
                            },
                        ));
                        self.line_end_snap = None;
                        self.creating_line = Some(CreatingLine {
                            origin: end,
                            text: String::new(),
                            last_mouse: end,
                            user_edited: false,
                            pending_focus: true,
                            construction: cl.construction,
                            curve_mode: self.draw_curve_mode,
                            tangent_constraint: self.draw_tangent_constraint,
                            chained_from: Some(line_index),
                            chained_from_bezier: self.doc.lines[line_index].bezier,
                        });
                        self.status = format!(
                            "Added line ({len_label}) • click for next point • Esc to finish"
                        );
                    } else {
                        self.status = format!("Added line ({len_label})");
                    }
                    ActionResult::Ok
                } else {
                    self.creating_line = Some(cl);
                    self.line_start_snap = None;
                    self.line_end_snap = None;
                    self.status = "Line too short".to_string();
                    ActionResult::Err("Line too short".to_string())
                }
            }
            Action::SetLineLength { value } => {
                if let Some(edit) = &mut self.editing_committed_dim {
                    let matches = match &edit.target {
                        DimEditTarget::Constraint(id) => constraint_is_line_length(&self.doc, *id),
                        DimEditTarget::New(DimensionTarget::Distance(DistanceTarget::LineLength(_))) => {
                            true
                        }
                        DimEditTarget::New(_) => false,
                    };
                    if matches {
                        edit.text = value;
                        return ActionResult::Ok;
                    }
                }
                let Some(cl) = &mut self.creating_line else {
                    return ActionResult::Err("No line in progress".to_string());
                };
                cl.text = value;
                cl.user_edited = true;
                ActionResult::Ok
            }
            Action::SetDimLabelOffset { target, offset } => {
                if let Err(e) = require_constraint_editable(&self.document_health, &self.doc, target)
                {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let offset = if constraint_is_circle_diameter(&self.doc, target) {
                    crate::dimensions::effective_circle_diameter_label_offset(Some(offset))
                } else if constraint_is_angle(&self.doc, target) {
                    crate::dimensions::effective_arc_dim_offset(Some(offset))
                } else {
                    crate::dimensions::effective_dim_offset(Some(offset))
                };
                match set_constraint_dim_offset(&mut self.doc, target, offset) {
                    Ok(()) => ActionResult::Ok,
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::SetConstraintAngleValue {
                constraint_id,
                angle_rad,
            } => {
                if let Err(e) =
                    require_constraint_editable(&self.document_health, &self.doc, constraint_id)
                {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                match crate::constraints::set_constraint_angle_value(
                    &mut self.doc,
                    constraint_id,
                    angle_rad,
                ) {
                    Ok(()) => ActionResult::Ok,
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::FocusLineLength => {
                if let Some(edit) = &mut self.editing_committed_dim {
                    let matches = match &edit.target {
                        DimEditTarget::Constraint(id) => constraint_is_line_length(&self.doc, *id),
                        DimEditTarget::New(DimensionTarget::Distance(DistanceTarget::LineLength(_))) => {
                            true
                        }
                        DimEditTarget::New(_) => false,
                    };
                    if matches {
                        edit.pending_focus = true;
                        return ActionResult::Ok;
                    }
                }
                let Some(cl) = &mut self.creating_line else {
                    return ActionResult::Err("No line in progress".to_string());
                };
                cl.pending_focus = true;
                ActionResult::Ok
            }
            Action::CommitCircle => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let Some(frame) = sketch_geometry_frame(&self.doc, session.sketch) else {
                    return ActionResult::Err("Sketch no longer exists".to_string());
                };
                let Some(mut cc) = self.creating_circle.take() else {
                    return ActionResult::Err("No circle in progress".to_string());
                };
                if cc.user_edited {
                    if let Err(e) =
                        try_commit_inline_parameter_definition(&mut self.doc, &mut cc.text)
                    {
                        self.creating_circle = Some(cc);
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let (cu, cv) = world_to_local(&frame, cc.origin);
                let r = cc.radius(&frame, &self.doc);
                let angle = cc.diameter_dim_angle(&frame);
                let mut circle =
                    Circle::from_local_center_radius(session.sketch, cu, cv, r, angle);
                circle.construction = cc.construction;
                if circle.r > 0.25 {
                    self.doc.circles.push(circle);
                    self.doc.shape_order.push(ShapeKind::Circle);
                    let circle_index = self.doc.circles.len() - 1;
                    if cc.user_edited {
                        if let Err(e) = add_distance_constraint(
                            &mut self.doc,
                            session.sketch,
                            DistanceTarget::CircleDiameter(circle_index),
                            cc.text.clone(),
                        ) {
                            self.doc.circles.pop();
                            self.doc.shape_order.pop();
                            self.circle_center_snap = None;
                            self.creating_circle = Some(cc);
                            self.status = e.clone();
                            return ActionResult::Err(e);
                        }
                    }
                    // Pin the center if it was left on a snap target.
                    if let Some(target) = self.circle_center_snap.take() {
                        let _ = self.add_snap_constraint(
                            session.sketch,
                            ConstraintPoint::CircleCenter(circle_index),
                            target,
                        );
                    }
                    let diameter = self.doc.circles.last().unwrap().diameter();
                    self.status = format!(
                        "Added circle ({})",
                        crate::value::format_diameter_display_in(
                            diameter,
                            crate::model::effective_length_unit(&self.doc, session.sketch)
                        )
                    );
                    ActionResult::Ok
                } else {
                    self.circle_center_snap = None;
                    self.creating_circle = Some(cc);
                    self.status = "Circle too small".to_string();
                    ActionResult::Err("Circle too small".to_string())
                }
            }
            Action::SetCircleDiameter { value } => {
                if let Some(edit) = &mut self.editing_committed_dim {
                    let matches = match &edit.target {
                        DimEditTarget::Constraint(id) => {
                            constraint_is_circle_diameter(&self.doc, *id)
                        }
                        DimEditTarget::New(DimensionTarget::Distance(
                            DistanceTarget::CircleDiameter(_),
                        )) => true,
                        DimEditTarget::New(_) => false,
                    };
                    if matches {
                        edit.text = value;
                        return ActionResult::Ok;
                    }
                }
                let Some(cc) = &mut self.creating_circle else {
                    return ActionResult::Err("No circle in progress".to_string());
                };
                cc.text = value;
                cc.user_edited = true;
                ActionResult::Ok
            }
            Action::FocusCircleDiameter => {
                if let Some(edit) = &mut self.editing_committed_dim {
                    let matches = match &edit.target {
                        DimEditTarget::Constraint(id) => {
                            constraint_is_circle_diameter(&self.doc, *id)
                        }
                        DimEditTarget::New(DimensionTarget::Distance(
                            DistanceTarget::CircleDiameter(_),
                        )) => true,
                        DimEditTarget::New(_) => false,
                    };
                    if matches {
                        edit.pending_focus = true;
                        return ActionResult::Ok;
                    }
                }
                let Some(cc) = &mut self.creating_circle else {
                    return ActionResult::Err("No circle in progress".to_string());
                };
                cc.pending_focus = true;
                ActionResult::Ok
            }
            Action::BeginEditCommittedDim { target } => {
                if !self.can_edit_sketch_dimensions() {
                    return ActionResult::Err(
                        "Open a sketch and finish the current draw operation to edit dimensions"
                            .to_string(),
                    );
                }
                if let Err(e) = require_constraint_editable(&self.document_health, &self.doc, target)
                {
                    return ActionResult::Err(e);
                }
                let Some(text) = committed_dim_expression(&self.doc, target) else {
                    return ActionResult::Err("Dimension is not editable".to_string());
                };
                self.editing_committed_dim = Some(EditingCommittedDim {
                    target: DimEditTarget::Constraint(target),
                    text,
                    pending_focus: true,
                    dim_offset: None,
                });
                self.status = "Edit dimension • Enter to commit • Esc to cancel".to_string();
                ActionResult::Ok
            }
            Action::BeginDimensionEdit { target } => {
                let Some(_session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                if self.tool != Tool::Dimension {
                    return ActionResult::Err("Dimension tool required".to_string());
                }
                self.start_committed_dimension_edit(target);
                ActionResult::Ok
            }
            Action::CommitCommittedDim => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let Some(edit) = self.editing_committed_dim.take() else {
                    return ActionResult::Err("No committed dimension being edited".to_string());
                };
                let frozen = match &edit.target {
                    DimEditTarget::Constraint(id) => {
                        require_constraint_editable(&self.document_health, &self.doc, *id)
                    }
                    DimEditTarget::New(target) => {
                        require_dimension_target_editable(&self.document_health, &self.doc, target.clone())
                    }
                };
                if let Err(e) = frozen {
                    self.editing_committed_dim = Some(edit);
                    return ActionResult::Err(e);
                }
                let target = edit.target.clone();
                let mut text = edit.text.clone();
                if let Err(e) = try_commit_inline_parameter_definition(&mut self.doc, &mut text) {
                    self.editing_committed_dim = Some(edit);
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                match apply_committed_dim_expression(
                    &mut self.doc,
                    session.sketch,
                    target.clone(),
                    &text,
                ) {
                    Ok(()) => {
                        // Persist the arc radius chosen while placing a new angle dimension
                        // so the committed arc keeps the size the preview showed (#188).
                        if let (Some(offset), DimEditTarget::New(DimensionTarget::Angle {
                            line_a,
                            line_b,
                            ..
                        })) = (edit.dim_offset, &target)
                        {
                            if let Some(id) = crate::constraints::find_angle_constraint(
                                &self.doc,
                                line_a.clone(),
                                line_b.clone(),
                            ) {
                                if let Some(c) = self.doc.constraints.get_mut(id) {
                                    c.dim_offset = Some(offset);
                                }
                            }
                        }
                        self.refresh_document_health();
                        self.status = "Updated dimension".to_string();
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.editing_committed_dim = Some(edit);
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::BeginConstructionPlane { reference, parent } => {
                self.creating_plane = Some(CreatingConstructionPlane {
                    edit_index: None,
                    reference,
                    parent,
                    offset_text: String::new(),
                    angle_text: String::new(),
                    focused: PlaneDim::Offset,
                    offset_live: 0.0,
                    axis_angle_deg: 0.0,
                    user_edited_offset: false,
                    user_edited_angle: false,
                    pending_focus: true,
                    axis_gizmo_drag: None,
                });
                self.tool = Tool::ConstructionPlane;
                self.status = "Set offset • type to lock • Tab cycle dims • click/Enter commit • Esc cancel"
                    .to_string();
                ActionResult::Ok
            }
            Action::BeginEditConstructionPlane { index } => {
                let Some(plane) = self.doc.construction_planes.get(index) else {
                    return ActionResult::Err(format!("Unknown construction plane {index}"));
                };
                let reference = reference_from_definition(&plane.definition);
                let (offset_live, axis_angle_deg) = match &reference {
                    PlaneReference::Face { .. } => (plane.definition.offset_mm, 0.0),
                    PlaneReference::Axis { .. } => {
                        (plane.definition.offset_mm, plane.definition.angle_deg)
                    }
                };
                self.creating_plane = Some(CreatingConstructionPlane {
                    edit_index: Some(index),
                    reference,
                    parent: plane.parent,
                    offset_text: format!("{offset_live:.1}"),
                    angle_text: format!("{axis_angle_deg:.0}"),
                    focused: PlaneDim::Offset,
                    offset_live,
                    axis_angle_deg,
                    user_edited_offset: false,
                    user_edited_angle: false,
                    pending_focus: true,
                    axis_gizmo_drag: None,
                });
                self.tool = Tool::ConstructionPlane;
                self.status = format!(
                    "Edit construction plane {index} • type to lock offset{} • Tab cycle dims • click/Enter commit • Esc cancel",
                    if plane.definition.is_axis() { "/angle" } else { "" }
                );
                ActionResult::Ok
            }
            Action::CommitConstructionPlane => {
                let Some(mut cp) = self.creating_plane.take() else {
                    return ActionResult::Err("No construction plane in progress".to_string());
                };
                if cp.user_edited_offset {
                    if let Err(e) =
                        try_commit_inline_parameter_definition(&mut self.doc, &mut cp.offset_text)
                    {
                        self.creating_plane = Some(cp);
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                if cp.user_edited_angle {
                    if let Err(e) =
                        try_commit_inline_parameter_definition(&mut self.doc, &mut cp.angle_text)
                    {
                        self.creating_plane = Some(cp);
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let definition = cp.resolved_definition();
                let live_offset = definition.offset_mm;
                if let Some(index) = cp.edit_index {
                    // Snapshot all planes before the edit so Undo can revert it (the edit
                    // also moves descendant planes, so snapshot the whole list).
                    let previous_planes = self.doc.construction_planes.clone();
                    match apply_construction_plane_edit(
                        &mut self.doc,
                        index,
                        &definition,
                        cp.parent,
                    ) {
                        Ok(()) => {
                            self.construction_plane_edit_undo.push(previous_planes);
                            self.doc.shape_order.push(ShapeKind::ConstructionPlaneEdit);
                            self.status = format!(
                                "Updated construction plane {index} ({} from {})",
                                crate::value::format_length_display_in(
                                    live_offset,
                                    self.doc.default_length_unit
                                ),
                                cp.reference.label()
                            );
                            ActionResult::Ok
                        }
                        Err(message) => {
                            self.creating_plane = Some(cp);
                            self.status = message.clone();
                            ActionResult::Err(message)
                        }
                    }
                } else {
                    self.add_construction_plane(definition, cp.parent)
                }
            }
            Action::AddConstructionPlane { from, offset_mm } => {
                let Some(reference_plane) = self.doc.construction_planes.get(from) else {
                    return ActionResult::Err(format!("Unknown construction plane {from}"));
                };
                let anchor = crate::model::PlaneAnchor::Face {
                    origin: reference_plane.origin,
                    normal: reference_plane.normal,
                    label: "Construction plane".to_string(),
                };
                let definition = crate::model::PlaneDefinition {
                    anchor,
                    offset_mm,
                    angle_deg: 0.0,
                };
                self.add_construction_plane(definition, ConstructionPlaneParent::Root)
            }
            Action::SetPlaneOffset { value } => {
                let Some(cp) = &mut self.creating_plane else {
                    return ActionResult::Err("No construction plane in progress".to_string());
                };
                cp.offset_text = value.clone();
                cp.user_edited_offset = true;
                if let Some(v) = crate::value::eval_length_mm(&value) {
                    cp.offset_live = v;
                }
                ActionResult::Ok
            }
            Action::SetPlaneAngle { value } => {
                let Some(cp) = &mut self.creating_plane else {
                    return ActionResult::Err("No construction plane in progress".to_string());
                };
                cp.angle_text = value.clone();
                cp.user_edited_angle = true;
                if let Ok(v) = value.trim().parse::<f32>() {
                    cp.axis_angle_deg = v.rem_euclid(360.0);
                }
                ActionResult::Ok
            }
            Action::FocusPlaneDim { dim } => {
                let Some(cp) = &mut self.creating_plane else {
                    return ActionResult::Err("No construction plane in progress".to_string());
                };
                cp.focused = dim;
                cp.pending_focus = true;
                ActionResult::Ok
            }
            Action::OrbitCamera { delta } => {
                self.cam.orbit(egui::vec2(delta.0, delta.1));
                ActionResult::Ok
            }
            Action::PanCamera {
                delta,
                viewport_height,
            } => {
                self.cam.pan(egui::vec2(delta.0, delta.1), viewport_height);
                ActionResult::Ok
            }
            Action::ZoomCamera {
                scroll,
                focal,
                viewport,
            } => {
                self.cam.zoom(scroll, focal, viewport);
                ActionResult::Ok
            }
            Action::SetStandardView(view) => {
                self.cam.start_view_transition(view, VIEW_TRANSITION_DURATION);
                self.status = format!("View: {:?}", view);
                ActionResult::Ok
            }
            Action::SetViewEdge(edge) => {
                self.cam.start_view_transition_to_direction(
                    view_cube::edge_view_direction(edge),
                    VIEW_TRANSITION_DURATION,
                );
                self.status = format!("View edge: {:?}", edge);
                ActionResult::Ok
            }
            Action::SetViewCorner(corner) => {
                self.cam.start_view_transition_to_direction(
                    view_cube::corner_view_direction(corner),
                    VIEW_TRANSITION_DURATION,
                );
                self.status = format!("View corner: {:?}", corner);
                ActionResult::Ok
            }
            Action::ViewHome => {
                self.cam.start_home_transition(VIEW_TRANSITION_DURATION);
                self.status = "View: home".to_string();
                ActionResult::Ok
            }
            Action::SetHomeView => {
                self.cam.set_home_from_current();
                self.status = "Home view set".to_string();
                ActionResult::Ok
            }
            Action::SetProjectionMode(mode) => {
                self.cam.set_projection_mode(mode);
                self.status = format!("Projection: {:?}", mode);
                ActionResult::Ok
            }
            Action::ToggleProjectionMode => {
                self.cam.toggle_projection_mode();
                self.status = format!("Projection: {:?}", self.cam.projection_mode());
                ActionResult::Ok
            }
            Action::ProjectSelection => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Open a sketch to project into".to_string());
                };
                let sources =
                    crate::projection::projection_sources_from_selection(&self.doc, &self.scene_selection);
                if sources.is_empty() {
                    return ActionResult::Err(
                        "Select body edges (or a body) to project".to_string(),
                    );
                }
                let mut created = 0usize;
                for source in sources {
                    let Some((wa, wb)) =
                        crate::projection::resolve_projection_source(&self.doc, source)
                    else {
                        continue;
                    };
                    let (Some(a), Some(b)) = (
                        crate::projection::project_world_point_into_sketch(&self.doc, session.sketch, wa),
                        crate::projection::project_world_point_into_sketch(&self.doc, session.sketch, wb),
                    ) else {
                        continue;
                    };
                    // Degenerate after projection (source edge parallel to the plane
                    // normal): skip rather than create a zero-length line.
                    if ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt() < 1e-3 {
                        continue;
                    }
                    let mut line = crate::model::Line::from_local_endpoints(
                        session.sketch,
                        a.0,
                        a.1,
                        b.0,
                        b.1,
                    );
                    line.construction = true;
                    line.projection = Some(source);
                    self.doc.lines.push(line);
                    self.doc.shape_order.push(crate::model::ShapeKind::Line);
                    created += 1;
                }
                if created == 0 {
                    return ActionResult::Err(
                        "Nothing projectable (edges vanish edge-on to the sketch plane)".to_string(),
                    );
                }
                self.status = format!("Projected {created} edge(s) into the sketch");
                self.refresh_document_health();
                ActionResult::Ok
            }
            Action::ZoomToFit => {
                let bounds = crate::extrude::selection_world_bounds(&self.doc, &self.scene_selection)
                    .or_else(|| crate::extrude::document_world_bounds(&self.doc));
                match bounds {
                    Some((min, max)) => {
                        self.cam.frame_bounds_instant(min, max, self.viewport_aspect);
                        self.status = if self.scene_selection.is_empty() {
                            "Zoomed to fit".to_string()
                        } else {
                            "Zoomed to selection".to_string()
                        };
                        ActionResult::Ok
                    }
                    None => ActionResult::Err("Nothing to zoom to".to_string()),
                }
            }
            Action::SetGroundDisplay(mode) => {
                self.cam.set_ground_display(mode);
                self.status = match mode {
                    crate::camera::GroundDisplay::Grid => "Ground: grid".to_string(),
                    crate::camera::GroundDisplay::Solid => "Ground: solid".to_string(),
                };
                ActionResult::Ok
            }
            Action::SetShadingMode(mode) => {
                self.cam.set_shading_mode(mode);
                self.status = format!("Shading: {:?}", mode);
                ActionResult::Ok
            }
            Action::SetElementsViewMode { mode } => {
                self.hierarchy_view_mode = mode;
                self.status = format!("Elements view: {}", mode.script_name());
                ActionResult::Ok
            }
            Action::AddParameter { name, expression } => {
                match add_parameter(&mut self.doc, name.clone(), expression.clone()) {
                    Ok(_) => {
                        self.status = format!("Added parameter {name}");
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::CreateParameterFromLineLength { line_index, name } => {
                match add_computed_parameter_from_line_length(&mut self.doc, line_index, name.clone())
                {
                    Ok(index) => {
                        let param_name = self.doc.parameters[index].name.clone();
                        self.refresh_document_health();
                        self.status = format!("Added parameter {param_name} from line length");
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::CommitParameterName { index, name } => {
                if let Err(e) = require_parameter_editable(&self.document_health, index) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                match set_parameter_name(&mut self.doc, index, name.clone()) {
                    Ok(()) => {
                        self.refresh_document_health();
                        self.status = format!("Renamed parameter to {name}");
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::CommitParameterExpression { index, expression } => {
                if let Err(e) = require_parameter_editable(&self.document_health, index) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if let Some(param) = self.doc.parameters.get(index) {
                    if let Err(e) = require_parameter_value_editable(param) {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                match set_parameter_expression(&mut self.doc, index, expression.clone()) {
                    Ok(()) => {
                        let _ = recompute_document_geometry(&mut self.doc);
                        self.refresh_document_health();
                        self.status = "Updated parameter".to_string();
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::DeleteParameter { index } => {
                match delete_parameter(&mut self.doc, index) {
                    Ok(()) => {
                        let _ = recompute_document_geometry(&mut self.doc);
                        self.refresh_document_health();
                        self.status = "Deleted parameter".to_string();
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::DeleteSelection => {
                if self.scene_selection.is_empty() {
                    self.status = "Nothing selected".to_string();
                    return ActionResult::Ok;
                }
                let targets = delete_targets_from_selection(&self.scene_selection);
                let count = tombstone_elements(&mut self.doc, &targets);
                if let Some(session) = self.sketch_session {
                    if !crate::document_lifecycle::sketch_alive(&self.doc, session.sketch) {
                        self.exit_sketch_session();
                    }
                }
                self.scene_selection.clear();
                let _ = recompute_document_geometry(&mut self.doc);
                self.refresh_document_health();
                let mut status = format!("Deleted {count} element(s)");
                let invalid = self
                    .document_health
                    .elements
                    .values()
                    .filter(|s| **s == crate::document_health::HealthStatus::Invalid)
                    .count()
                    + self
                        .document_health
                        .parameters
                        .values()
                        .filter(|s| **s == crate::document_health::HealthStatus::Invalid)
                        .count();
                let unstable = self
                    .document_health
                    .elements
                    .values()
                    .filter(|s| **s == crate::document_health::HealthStatus::Unstable)
                    .count();
                if invalid > 0 || unstable > 0 {
                    status.push_str(&format!(" — {invalid} invalid, {unstable} unstable"));
                }
                self.status = status;
                ActionResult::Ok
            }
            Action::SetCommandPaletteOpen { open } => {
                if open {
                    self.command_palette.open_palette();
                    self.status = "Command palette".to_string();
                } else {
                    self.command_palette.close_palette();
                }
                ActionResult::Ok
            }
            Action::ToggleCommandPalette => {
                if self.command_palette.open {
                    self.command_palette.close_palette();
                } else {
                    self.command_palette.open_palette();
                    self.status = "Command palette".to_string();
                }
                ActionResult::Ok
            }
            Action::SetPaneVisible { pane, visible } => {
                self.panes.set(pane, visible);
                self.status = pane_status(pane, visible);
                ActionResult::Ok
            }
            Action::TogglePane(pane) => {
                self.panes.toggle(pane);
                self.status = pane_status(pane, self.panes.is_visible(pane));
                ActionResult::Ok
            }
            Action::DragVertex { point, u, v } => {
                let Some(sketch) = self.sketch_session.map(|s| s.sketch) else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let element = vertex_drag::scene_element_for_point(point.clone());
                if let Err(e) = require_element_editable(&self.document_health, element) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if !vertex_drag::can_drag_point(&self.doc, sketch, point.clone()) {
                    return ActionResult::Err("Vertex is fully constrained".to_string());
                }
                match vertex_drag::drag_point(&mut self.doc, sketch, point, u, v) {
                    Ok(()) => ActionResult::Ok,
                    Err(e) => ActionResult::Err(e),
                }
            }
            Action::BeginLineDrag {
                target,
                anchor_u,
                anchor_v,
            } => {
                let Some(sketch) = self.sketch_session.map(|s| s.sketch) else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let element = vertex_drag::scene_element_for_line(target.clone());
                if let Err(e) = require_element_editable(&self.document_health, element) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if !vertex_drag::can_drag_line(&self.doc, sketch, target.clone()) {
                    return ActionResult::Err("Line is fully constrained".to_string());
                }
                match vertex_drag::begin_line_drag_session(
                    &self.doc,
                    sketch,
                    target,
                    (anchor_u, anchor_v),
                ) {
                    Ok(session) => {
                        self.line_drag_session = Some(session);
                        ActionResult::Ok
                    }
                    Err(e) => ActionResult::Err(e),
                }
            }
            Action::DragLine { u, v } => {
                let Some(sketch) = self.sketch_session.map(|s| s.sketch) else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let Some(session) = self.line_drag_session.clone() else {
                    return ActionResult::Err("No line drag in progress".to_string());
                };
                let element = vertex_drag::scene_element_for_line(session.target.clone());
                if let Err(e) = require_element_editable(&self.document_health, element) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                match vertex_drag::drag_line(&mut self.doc, sketch, &session, (u, v)) {
                    Ok(()) => ActionResult::Ok,
                    Err(e) => ActionResult::Err(e),
                }
            }
            Action::EndLineDrag => {
                self.line_drag_session = None;
                ActionResult::Ok
            }
            Action::SetBezierHandle { line, near_start, u, v } => {
                if let Err(e) =
                    require_element_editable(&self.document_health, SceneElement::Line(line))
                {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let Some(l) = self.doc.lines.get_mut(line) else {
                    return ActionResult::Err("Line no longer exists".to_string());
                };
                let Some(handles) = l.bezier.as_mut() else {
                    return ActionResult::Err("Line is not curved".to_string());
                };
                handles[if near_start { 0 } else { 1 }] = (u, v);
                ActionResult::Ok
            }
            Action::ConvertVertexToBezier { point } => {
                let Some(sketch) = crate::construction::point_sketch(&self.doc, point.clone()) else {
                    return ActionResult::Err("Vertex no longer exists".to_string());
                };
                let Some(corner) = vertex_drag::treatment_corner(&self.doc, sketch, point) else {
                    return ActionResult::Err(
                        "Vertex must join exactly two lines to become a curve".to_string(),
                    );
                };
                let vertex_drag::VertexTreatmentCorner { line1, end1, line2, end2, v, a, b } = corner;
                for &li in &[line1, line2] {
                    if let Err(e) =
                        require_element_editable(&self.document_health, SceneElement::Line(li))
                    {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let ([h1_far, h1_near], [h2_near, h2_far]) =
                    crate::model::smooth_joint_bezier(a, v, b);
                if let Some(l1) = self.doc.lines.get_mut(line1) {
                    l1.bezier = Some(match end1 {
                        LineEnd::Start => [h1_near, h1_far],
                        LineEnd::End => [h1_far, h1_near],
                    });
                }
                if let Some(l2) = self.doc.lines.get_mut(line2) {
                    l2.bezier = Some(match end2 {
                        LineEnd::Start => [h2_near, h2_far],
                        LineEnd::End => [h2_far, h2_near],
                    });
                }
                self.status = "Converted to curve".to_string();
                ActionResult::Ok
            }
            Action::StraightenLine { line } => {
                if let Err(e) =
                    require_element_editable(&self.document_health, SceneElement::Line(line))
                {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let Some(l) = self.doc.lines.get_mut(line) else {
                    return ActionResult::Err("Line no longer exists".to_string());
                };
                if !l.is_curved() {
                    return ActionResult::Err("Line is already straight".to_string());
                }
                l.bezier = None;
                self.status = "Straightened line".to_string();
                ActionResult::Ok
            }
            Action::CommitVertexTreatment { point, kind, amount } => {
                if !(amount > 0.0) {
                    let e = "Amount must be positive".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let Some(sketch) = crate::construction::point_sketch(&self.doc, point.clone()) else {
                    return ActionResult::Err("Vertex no longer exists".to_string());
                };
                let Some(corner) = vertex_drag::treatment_corner(&self.doc, sketch, point) else {
                    return ActionResult::Err(
                        "Vertex must join exactly two lines to chamfer/fillet".to_string(),
                    );
                };
                let vertex_drag::VertexTreatmentCorner { line1, end1, line2, end2, v, a, b } = corner;
                for &li in &[line1, line2] {
                    if let Err(e) =
                        require_element_editable(&self.document_health, SceneElement::Line(li))
                    {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let Some(geom) = vertex_treatment_geometry(v, a, b, kind, amount) else {
                    let e = "Cannot treat this vertex: corner is degenerate".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                };

                let old_len1 = self.doc.lines.get(line1).map(|l| l.length()).unwrap_or(0.0);
                let old_len2 = self.doc.lines.get(line2).map(|l| l.length()).unwrap_or(0.0);
                if let Some(l1) = self.doc.lines.get_mut(line1) {
                    match end1 {
                        LineEnd::Start => (l1.x0, l1.y0) = geom.p1,
                        LineEnd::End => (l1.x1, l1.y1) = geom.p1,
                    }
                }
                if let Some(l2) = self.doc.lines.get_mut(line2) {
                    match end2 {
                        LineEnd::Start => (l2.x0, l2.y0) = geom.p2,
                        LineEnd::End => (l2.x1, l2.y1) = geom.p2,
                    }
                }
                // A trimmed line's LENGTH dimension still demands the pre-trim length; the
                // next solve would drag the endpoint back and mangle the treated corner
                // (the bug showed as a bracket bend collapsing one step later). Re-target
                // the dims to the trimmed length — numeric expressions get the new number,
                // parameter expressions stay symbolic as `(expr) - trim` so they keep
                // following their parameters.
                for (li, old_len) in [(line1, old_len1), (line2, old_len2)] {
                    let new_len = self.doc.lines.get(li).map(|l| l.length()).unwrap_or(0.0);
                    let trim = old_len - new_len;
                    if trim <= 1e-4 {
                        continue;
                    }
                    if let Some(constraint) = self.doc.constraints.iter_mut().find(|c| {
                        !c.deleted
                            && c.sketch == sketch
                            && matches!(
                                &c.kind,
                                ConstraintKind::Distance {
                                    target: DistanceTarget::LineLength(target_line),
                                } if *target_line == li
                            )
                    }) {
                        let expr = constraint.expression.trim();
                        constraint.expression = if expr.parse::<f32>().is_ok() {
                            format!("{:.3}", new_len)
                        } else {
                            format!("({expr}) - {trim:.3}")
                        };
                    }
                }

                // The two treated endpoints are no longer coincident — a new line now bridges
                // them — so drop the constraint directly between them. Other constraints that
                // may have referenced the old vertex position are intentionally left alone
                // (documented limitation, see SPEC §3.1).
                let p_a = ConstraintPoint::LineEndpoint { line: line1, end: end1 };
                let p_b = ConstraintPoint::LineEndpoint { line: line2, end: end2 };
                if let Some(idx) = self.doc.constraints.iter().position(|c| {
                    !c.deleted
                        && c.sketch == sketch
                        && matches!(
                            &c.kind,
                            ConstraintKind::Coincident { a, b }
                                if (*a == ConstraintEntity::Point(p_a.clone()) && *b == ConstraintEntity::Point(p_b.clone()))
                                    || (*a == ConstraintEntity::Point(p_b.clone()) && *b == ConstraintEntity::Point(p_a.clone()))
                        )
                }) {
                    // Mark deleted directly rather than via `tombstone_elements`: the
                    // tombstone path also removes the constraint's shape_order entry, which
                    // shrinks this action's net growth and would leave the undo group
                    // covering only part of the gesture (the bridge line would survive the
                    // first UndoLast).
                    self.doc.constraints[idx].deleted = true;
                }

                let mut bridge =
                    Line::from_local_endpoints(sketch, geom.p1.0, geom.p1.1, geom.p2.0, geom.p2.1);
                bridge.bezier = geom.bezier;
                // Nest the bridging line under the lower-index trimmed line in the Elements
                // pane (#76): a chamfer/fillet corner is shared by two lines, so there's no
                // single unambiguous "the" parent — `line1` (from `treatment_corner`'s
                // `incident_two_lines`-derived ordering) is the deterministic, documented
                // scope call. See `hierarchy::build_sketch_entry`.
                bridge.chamfer_fillet_parent = Some(line1);
                self.doc.lines.push(bridge);
                self.doc.shape_order.push(ShapeKind::Line);
                // Tie the bridge to the trimmed endpoints with Coincident constraints, so
                // the treated profile stays a *closed loop* in the constraint graph — loop
                // detection (closed_line_loops) walks Coincident chains, so without these a
                // chamfered/filleted polygon silently stopped being a fillable, extrudable
                // face, and the solver could pull the bridge away from its corner.
                let bridge_line = self.doc.lines.len() - 1;
                for (bridge_end, trimmed) in [
                    (LineEnd::Start, p_a.clone()),
                    (LineEnd::End, p_b.clone()),
                ] {
                    self.doc.constraints.push(crate::model::Constraint {
                        sketch,
                        kind: ConstraintKind::Coincident {
                            a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                                line: bridge_line,
                                end: bridge_end,
                            }),
                            b: ConstraintEntity::Point(trimmed),
                        },
                        expression: String::new(),
                        dim_offset: None,
                        name: None,
                        deleted: false,
                    });
                    self.doc.shape_order.push(ShapeKind::Constraint);
                }

                self.refresh_document_health();
                self.status = match kind {
                    VertexTreatmentKind::Chamfer => "Added chamfer".to_string(),
                    VertexTreatmentKind::Fillet => "Added fillet".to_string(),
                };
                ActionResult::Ok
            }
            Action::CommitEdgeTreatments { edges, kind, amount } => {
                if edges.is_empty() {
                    return ActionResult::Err("No edges to treat".to_string());
                }
                let mut applied = 0usize;
                let mut first_error: Option<String> = None;
                for (extrusion, edge) in edges {
                    match self.apply(Action::CommitEdgeTreatment { extrusion, edge, kind, amount }) {
                        ActionResult::Ok | ActionResult::NeedsDialog => applied += 1,
                        ActionResult::Err(e) => {
                            if first_error.is_none() {
                                first_error = Some(e);
                            }
                        }
                    }
                }
                match (applied, first_error) {
                    (0, Some(e)) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                    (n, Some(e)) => {
                        self.status = format!("Treated {n} edge(s); skipped some: {e}");
                        ActionResult::Ok
                    }
                    (n, None) => {
                        let noun = match kind {
                            VertexTreatmentKind::Chamfer => "Chamfered",
                            VertexTreatmentKind::Fillet => "Filleted",
                        };
                        self.status = format!("{noun} {n} edge(s)");
                        ActionResult::Ok
                    }
                }
            }
            Action::CommitEdgeTreatment { extrusion, edge, kind, amount } => {
                if !(amount > 0.0) {
                    let e = "Amount must be positive".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if let Err(e) = require_element_editable(
                    &self.document_health,
                    SceneElement::Extrusion(extrusion),
                ) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if !crate::extrude::extrusion_edge_exists(&self.doc, extrusion, edge) {
                    let e = "Edge no longer exists or isn't chamfer/fillet-able (vertical and \
                        side/cap edges of Rect/Polygon-profiled extrusions, or the cap rims \
                        of Circle-profiled ones, are supported)"
                        .to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let n = self
                    .doc
                    .extrusions
                    .get(extrusion)
                    .and_then(|ext| ext.faces.get(edge.face()))
                    .map(crate::extrude::side_face_count)
                    .unwrap_or(0);
                let existing = &self.doc.extrusions[extrusion].edge_treatments;
                if crate::extrude::edge_treatment_conflicts(existing, edge, n) {
                    let e = "Cannot treat this edge: it shares a corner with another treated \
                        edge (blending 3+ bevels at a shared corner isn't supported)"
                        .to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if !crate::extrude::edge_treatment_would_bevel(&self.doc, extrusion, edge, kind, amount)
                {
                    let e = "Cannot treat this edge: corner is degenerate".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let treatment = EdgeTreatment { edge, kind, amount };
                let Some(updated) =
                    crate::extrude::extrusion_with_edge_treatment(&self.doc, extrusion, treatment)
                else {
                    return ActionResult::Err("Extrusion no longer exists".to_string());
                };
                // #103: kernel feasibility trial. If the kernel builds this extrusion today
                // but can't build it with the new treatment (an impossible fillet radius /
                // chamfer distance), storing it would silently knock the whole body onto the
                // additive-only mesh fallback — deleting its cut holes from the render — so
                // reject at commit instead. Runs only here (the final commit path shared by
                // the gizmo, the amount input, and scripting), never per-frame: the live drag
                // preview is a separate ghost mesh that doesn't go through this action. In a
                // no-kernel build there's nothing to consult; the mesh-bevel clamp stands.
                #[cfg(feature = "occt")]
                if !crate::extrude::occt_edge_treatments_feasible(&self.doc, extrusion, &updated) {
                    let (noun, param) = match kind {
                        VertexTreatmentKind::Chamfer => ("chamfer", "distance"),
                        VertexTreatmentKind::Fillet => ("fillet", "radius"),
                    };
                    let e = format!(
                        "{noun} of {amount:.1} mm doesn't fit this edge (kernel can't build \
                         it) — try a smaller {param}"
                    );
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                // #168: snapshot the prior treatment list so this in-place edit undoes.
                self.edge_treatment_undo.push((
                    extrusion,
                    self.doc.extrusions[extrusion].edge_treatments.clone(),
                ));
                self.doc.shape_order.push(crate::model::ShapeKind::EdgeTreatmentEdit);
                self.doc.extrusions[extrusion] = updated;
                self.refresh_document_health();
                self.status = match kind {
                    VertexTreatmentKind::Chamfer => format!("Chamfered edge ({amount:.1} mm)"),
                    VertexTreatmentKind::Fillet => format!("Filleted edge ({amount:.1} mm)"),
                };
                ActionResult::Ok
            }
            Action::CreateRectangle {
                x,
                y,
                width,
                height,
            } => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                if width <= 0.0 || height <= 0.0 {
                    return ActionResult::Err(
                        "Rectangle needs positive width and height".to_string(),
                    );
                }
                let lines_before = self.doc.lines.len();
                let constraints_before = self.doc.constraints.len();
                let shape_order_before = self.doc.shape_order.len();
                // A rectangle is four plain lines forming a closed loop (#66 polygon); width
                // drives the bottom edge, height the right edge, as length dimensions.
                let lines = crate::construction::add_line_rectangle(
                    &mut self.doc,
                    session.sketch,
                    x,
                    y,
                    width,
                    height,
                    [false; 4],
                );
                let mut add_dim = |line: usize, value: f32| {
                    add_distance_constraint(
                        &mut self.doc,
                        session.sketch,
                        DistanceTarget::LineLength(line),
                        value.to_string(),
                    )
                };
                if let Err(e) = add_dim(lines[0], width)
                    .and_then(|_| add_dim(lines[1], height))
                {
                    self.doc.constraints.truncate(constraints_before);
                    self.doc.lines.truncate(lines_before);
                    self.doc.shape_order.truncate(shape_order_before);
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                self.refresh_document_health();
                let unit = crate::model::effective_length_unit(&self.doc, session.sketch);
                self.status = format!(
                    "Added rectangle ({} × {})",
                    crate::value::format_length_display_in(width, unit),
                    crate::value::format_length_display_in(height, unit)
                );
                ActionResult::Ok
            }
            Action::CreateCircle { cx, cy, r } => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                if r <= 0.0 {
                    return ActionResult::Err("Circle needs a positive radius".to_string());
                }
                let circle = Circle::from_local_center_radius(session.sketch, cx, cy, r, 0.0);
                self.doc.circles.push(circle);
                self.doc.shape_order.push(ShapeKind::Circle);
                let circle_index = self.doc.circles.len() - 1;
                if let Err(e) = add_distance_constraint(
                    &mut self.doc,
                    session.sketch,
                    DistanceTarget::CircleDiameter(circle_index),
                    (r * 2.0).to_string(),
                ) {
                    while self.doc.shape_order.last() == Some(&ShapeKind::Constraint) {
                        self.doc.shape_order.pop();
                        self.doc.constraints.pop();
                    }
                    self.doc.circles.pop();
                    self.doc.shape_order.pop();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                self.refresh_document_health();
                self.status = format!(
                    "Added circle ({})",
                    crate::value::format_diameter_display_in(
                        r * 2.0,
                        crate::model::effective_length_unit(&self.doc, session.sketch)
                    )
                );
                ActionResult::Ok
            }
            Action::CreateLineSegment { x0, y0, x1, y1, bezier, dimension } => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                let mut line = Line::from_local_endpoints(session.sketch, x0, y0, x1, y1);
                let length = line.length();
                if length <= 0.5 {
                    return ActionResult::Err("Line is too short".to_string());
                }
                line.construction = self.draw_construction;
                line.bezier = bezier;
                self.doc.lines.push(line);
                self.doc.shape_order.push(ShapeKind::Line);
                let line_index = self.doc.lines.len() - 1;
                if let Some(expression) = dimension {
                    if let Err(e) = add_distance_constraint(
                        &mut self.doc,
                        session.sketch,
                        DistanceTarget::LineLength(line_index),
                        expression,
                    ) {
                        self.doc.lines.pop();
                        self.doc.shape_order.pop();
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                self.refresh_document_health();
                self.status = format!(
                    "Added line ({})",
                    crate::value::format_length_display_in(
                        length,
                        crate::model::effective_length_unit(&self.doc, session.sketch)
                    )
                );
                ActionResult::Ok
            }
            Action::CreateExtrusion {
                sketch,
                faces,
                distance,
                body,
                target,
            } => {
                if faces.is_empty() {
                    return ActionResult::Err("Extrusion needs at least one face".to_string());
                }
                // #104: a zero-distance extrusion would be an invisible dead entity; reject it
                // like the interactive tool ([`Action::CommitExtrusion`]) does. With a snap
                // target the effective distance derives from the target instead (#114).
                if target.is_none() && distance.abs() < 1e-3 {
                    let e = "Extrusion distance must be non-zero".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if let Some(t) = &target {
                    if let Err(e) = validate_extrude_target(&self.doc, t) {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                // #112: every face must resolve to a real profile — for a `Polygon`, all line
                // indices must be live lines forming a closed loop; for a `Circle`, the circle
                // must exist; `Boolean` operands are checked recursively and must reduce to a
                // single loop. `face_profile_world` is the same oracle the mesher uses, so
                // anything it rejects here would have produced no geometry.
                for face in &faces {
                    if crate::extrude::face_profile_world(&self.doc, face).is_none() {
                        let e =
                            "Extrude face does not exist or is not a closed loop".to_string();
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let candidate = extrude_merge_candidate(&self.doc, sketch);
                // An explicit merge/cut must have a body to attach to (#178): the sketch has to
                // sit on a body's face. Silently degrading to a standalone new body produced no
                // holes and raised nothing, hiding the mistake — so it's a hard error instead.
                let body_mode = match (body, candidate) {
                    (ExtrudeBodyChoice::New, _) => ExtrudeBodyMode::NewBody,
                    (ExtrudeBodyChoice::Merge, Some(bi)) => ExtrudeBodyMode::MergeInto(bi),
                    (ExtrudeBodyChoice::Cut, Some(bi)) => ExtrudeBodyMode::Cut(bi),
                    (ExtrudeBodyChoice::Merge, None) => {
                        let e = "Cannot merge: the sketch is not on a body face".to_string();
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                    (ExtrudeBodyChoice::Cut, None) => {
                        let e = "Cannot cut: the sketch is not on a body face".to_string();
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                };
                self.doc.extrusions.push(Extrusion {
                    sketch,
                    faces,
                    distance,
                    target,
                    expression: String::new(),
                    name: None,
                    deleted: false,
                    edge_treatments: Vec::new(),
                });
                self.doc.shape_order.push(ShapeKind::Extrusion);
                let extrusion_index = self.doc.extrusions.len() - 1;
                self.attach_new_extrusion_to_body(extrusion_index, body_mode);
                self.refresh_document_health();
                self.status = format!(
                    "Added extrusion ({})",
                    crate::value::format_length_display_in(
                        distance,
                        crate::model::effective_length_unit(&self.doc, sketch)
                    )
                );
                ActionResult::Ok
            }
            Action::UpdateExtrusion {
                extrusion,
                distance,
                target,
            } => {
                if distance.is_none() && target.is_none() {
                    return ActionResult::Err(
                        "Extrusion update needs a distance or a target".to_string(),
                    );
                }
                if let Some(d) = distance {
                    if target.is_none() && d.abs() < 1e-3 {
                        let e = "Extrusion distance must be non-zero".to_string();
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                if let Some(t) = &target {
                    if let Err(e) = validate_extrude_target(&self.doc, t) {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let Some(ext) = self
                    .doc
                    .extrusions
                    .get_mut(extrusion)
                    .filter(|e| !e.deleted)
                else {
                    return ActionResult::Err(format!("No extrusion {extrusion}"));
                };
                if let Some(d) = distance {
                    ext.distance = d;
                    ext.expression = String::new();
                    // A plain typed distance is a blind extrude: it replaces any snap
                    // target unless a new one is set in the same update.
                    if target.is_none() {
                        ext.target = None;
                    }
                }
                if target.is_some() {
                    ext.target = target;
                }
                self.refresh_document_health();
                self.status = format!("Updated extrusion {extrusion}");
                ActionResult::Ok
            }
            Action::ToggleExtrudeFace { face } => {
                let Some(sketch) = extrude_face_sketch(&self.doc, &face) else {
                    return ActionResult::Err("Face not found".to_string());
                };
                match &mut self.creating_extrusion {
                    Some(ce) if ce.sketch == sketch => {
                        if let Some(pos) = ce.faces.iter().position(|f| *f == face) {
                            ce.faces.remove(pos);
                        } else {
                            ce.faces.push(face);
                        }
                    }
                    // A face on a different plane starts a fresh extrusion.
                    _ => {
                        let merge_candidate = extrude_merge_candidate(&self.doc, sketch);
                        self.creating_extrusion = Some(CreatingExtrusion {
                            sketch,
                            faces: vec![face],
                            distance: DEFAULT_EXTRUDE_DISTANCE,
                            text: crate::value::format_length_display_in(
                                DEFAULT_EXTRUDE_DISTANCE,
                                crate::model::effective_length_unit(&self.doc, sketch),
                            ),
                            user_edited: false,
                            pending_focus: true,
                            target: None,
                            edit_index: None,
                            body_mode: merge_candidate
                                .map(ExtrudeBodyMode::MergeInto)
                                .unwrap_or(ExtrudeBodyMode::NewBody),
                            merge_candidate,
                        });
                    }
                }
                ActionResult::Ok
            }
            Action::ExtrudeBodyFace { face_id } => {
                let face = match create_implicit_extrude_sketch(&mut self.doc, face_id) {
                    Ok(face) => face,
                    Err(e) => {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                };
                let Some(sketch) = extrude_face_sketch(&self.doc, &face) else {
                    return ActionResult::Err("Face not found".to_string());
                };
                // A body face always starts a fresh single-face extrusion, never grouped
                // with whatever else was in progress (#122).
                let merge_candidate = extrude_merge_candidate(&self.doc, sketch);
                self.creating_extrusion = Some(CreatingExtrusion {
                    sketch,
                    faces: vec![face],
                    distance: DEFAULT_EXTRUDE_DISTANCE,
                    text: crate::value::format_length_display_in(
                        DEFAULT_EXTRUDE_DISTANCE,
                        crate::model::effective_length_unit(&self.doc, sketch),
                    ),
                    user_edited: false,
                    pending_focus: true,
                    target: None,
                    edit_index: None,
                    body_mode: merge_candidate
                        .map(ExtrudeBodyMode::MergeInto)
                        .unwrap_or(ExtrudeBodyMode::NewBody),
                    merge_candidate,
                });
                ActionResult::Ok
            }
            Action::CreateBodyFaceExtrusion { face_id, distance, target, body } => {
                // Build the implicit sketch on the body face, then create the extrusion in
                // one gesture — the scripted equivalent of clicking the face and pulling it.
                let face = match create_implicit_extrude_sketch(&mut self.doc, face_id) {
                    Ok(face) => face,
                    Err(e) => {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                };
                let Some(sketch) = extrude_face_sketch(&self.doc, &face) else {
                    return ActionResult::Err("Body face sketch not found".to_string());
                };
                self.apply(Action::CreateExtrusion {
                    sketch,
                    faces: vec![face],
                    distance,
                    body,
                    target,
                })
            }
            Action::SetExtrudeDistance { distance } => {
                if let Some(ce) = &mut self.creating_extrusion {
                    ce.distance = distance;
                    if !ce.user_edited {
                        ce.text = crate::value::format_length_display_in(
                            distance.abs(),
                            crate::model::effective_length_unit(&self.doc, ce.sketch),
                        );
                    }
                    // #141: the sketch sits on a face of `merge_candidate`, whose body lies on
                    // the negative-normal side. Extruding backward (negative distance) drives
                    // the profile into that body, so auto-switch to a cut; pulling forward
                    // again reverts to adding. A cut needs the kernel (see the pane's `occt`
                    // gate), so a non-`occt` build stays additive. Leaves an explicit `NewBody`
                    // choice untouched on forward drags — only the cut toggle is automatic.
                    if let Some(bi) = ce.merge_candidate {
                        if distance < 0.0 && cfg!(feature = "occt") {
                            ce.body_mode = ExtrudeBodyMode::Cut(bi);
                        } else if ce.body_mode == ExtrudeBodyMode::Cut(bi) {
                            ce.body_mode = ExtrudeBodyMode::MergeInto(bi);
                        }
                    }
                }
                ActionResult::Ok
            }
            Action::SetExtrudeTarget { target } => {
                if let Some(ce) = &mut self.creating_extrusion {
                    let has_target = target.is_some();
                    ce.target = target;
                    // Typing a distance again clears the object constraint.
                    if has_target {
                        ce.user_edited = false;
                    }
                }
                ActionResult::Ok
            }
            Action::SetExtrudeBodyMode { mode } => {
                let Some(ce) = &mut self.creating_extrusion else {
                    return ActionResult::Err("No extrusion in progress".to_string());
                };
                // Only the precomputed candidate (or plain NewBody) is a valid choice — an
                // arbitrary body index could point at an unrelated or deleted body.
                let allowed = match mode {
                    ExtrudeBodyMode::NewBody => true,
                    ExtrudeBodyMode::MergeInto(bi) | ExtrudeBodyMode::Cut(bi) => {
                        ce.merge_candidate == Some(bi)
                    }
                };
                if !allowed {
                    return ActionResult::Err("Not a valid body for this extrusion".to_string());
                }
                ce.body_mode = mode;
                ActionResult::Ok
            }
            Action::EditExtrusion { index } => {
                let Some(extrusion) = self.doc.extrusions.get(index) else {
                    return ActionResult::Err("Extrusion not found".to_string());
                };
                if extrusion.deleted {
                    return ActionResult::Err("Extrusion was deleted".to_string());
                }
                let merge_candidate = crate::model::body_index_for_extrusion(&self.doc, index);
                // Preserve the extrusion's current role: an extrusion already subtracted from
                // its body opens in Cut mode (#35), not MergeInto — otherwise re-committing
                // without touching the choice would silently re-fuse it.
                let is_cut = merge_candidate.is_some_and(|bi| {
                    self.doc.bodies[bi].source.cut_extrusion_indices().contains(&index)
                });
                let body_mode = match merge_candidate {
                    Some(bi) if is_cut => ExtrudeBodyMode::Cut(bi),
                    Some(bi) => ExtrudeBodyMode::MergeInto(bi),
                    None => ExtrudeBodyMode::NewBody,
                };
                self.creating_extrusion = Some(CreatingExtrusion {
                    sketch: extrusion.sketch,
                    faces: extrusion.faces.clone(),
                    distance: extrusion.distance,
                    text: crate::value::format_length_display_in(
                        extrusion.distance.abs(),
                        crate::model::effective_length_unit(&self.doc, extrusion.sketch),
                    ),
                    user_edited: false,
                    pending_focus: true,
                    target: extrusion.target.clone(),
                    edit_index: Some(index),
                    body_mode,
                    merge_candidate,
                });
                self.tool = Tool::Extrude;
                self.status = format!("Editing extrusion {index}");
                ActionResult::Ok
            }
            Action::CommitExtrusion => {
                let mut ce = match self.creating_extrusion.take() {
                    Some(ce) => ce,
                    None => return ActionResult::Err("No extrusion in progress".to_string()),
                };
                if ce.faces.is_empty() {
                    self.creating_extrusion = Some(ce);
                    return ActionResult::Err("Select at least one face".to_string());
                }
                // A typed `name = expr` defines a parameter and drives the depth from it, same
                // as the sketch dimension inputs (#196).
                if ce.user_edited {
                    if let Err(e) =
                        try_commit_inline_parameter_definition(&mut self.doc, &mut ce.text)
                    {
                        self.creating_extrusion = Some(ce);
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let distance = ce.evaluated_distance(&self.doc);
                if distance.abs() < 1e-3 {
                    self.creating_extrusion = Some(ce);
                    return ActionResult::Err("Extrusion distance must be non-zero".to_string());
                }
                // #112 defense-in-depth: interactively toggled faces come from picking real
                // geometry, but an edit session's stored faces could have gone stale (e.g.
                // their lines deleted since); reject rather than commit a dead extrusion.
                if ce
                    .faces
                    .iter()
                    .any(|f| crate::extrude::face_profile_world(&self.doc, f).is_none())
                {
                    self.creating_extrusion = Some(ce);
                    return ActionResult::Err(
                        "Extrude face does not exist or is not a closed loop".to_string(),
                    );
                }
                if let Some(idx) = ce.edit_index {
                    if let Some(extrusion) = self.doc.extrusions.get_mut(idx) {
                        extrusion.faces = ce.faces.clone();
                        extrusion.distance = distance;
                        extrusion.target = ce.target;
                    }
                    self.apply_extrude_body_mode(idx, ce.body_mode);
                    self.status = format!(
                        "Updated extrusion ({})",
                        crate::value::format_length_display_in(
                            distance,
                            crate::model::effective_length_unit(&self.doc, ce.sketch)
                        )
                    );
                } else {
                    let unit = crate::model::effective_length_unit(&self.doc, ce.sketch);
                    self.doc.extrusions.push(Extrusion {
                        sketch: ce.sketch,
                        faces: ce.faces.clone(),
                        distance,
                        target: ce.target,
                        expression: String::new(),
                        name: None,
                        deleted: false,
                        edge_treatments: Vec::new(),
                    });
                    self.doc.shape_order.push(ShapeKind::Extrusion);
                    let ei = self.doc.extrusions.len() - 1;
                    self.attach_new_extrusion_to_body(ei, ce.body_mode);
                    self.status = format!(
                        "Added extrusion ({})",
                        crate::value::format_length_display_in(distance, unit)
                    );
                }
                self.refresh_document_health();
                ActionResult::Ok
            }
            Action::ToggleLoftSection { section } => {
                if crate::extrude::face_profile_world(&self.doc, &section.face).is_none() {
                    return ActionResult::Err(
                        "Loft section is not a closed profile".to_string(),
                    );
                }
                let cl = self.creating_loft.get_or_insert_with(CreatingLoft::default);
                if let Some(pos) = cl.sections.iter().position(|sec| *sec == section) {
                    cl.sections.remove(pos);
                } else {
                    cl.sections.push(section);
                }
                self.status = format!("Loft: {} section(s)", cl.sections.len());
                ActionResult::Ok
            }
            Action::CommitLoft => {
                let Some(cl) = self.creating_loft.take() else {
                    return ActionResult::Err("No loft in progress".to_string());
                };
                if cl.sections.len() < 2 {
                    self.creating_loft = Some(cl);
                    return ActionResult::Err(
                        "Pick at least two cross sections to loft".to_string(),
                    );
                }
                let loft = crate::model::Loft {
                    sections: crate::extrude::order_loft_sections(&self.doc, cl.sections.clone()),
                    name: None,
                    deleted: false,
                };
                if crate::extrude::loft_mesh(&self.doc, &loft).is_none() {
                    self.creating_loft = Some(cl);
                    return ActionResult::Err(
                        "Loft sections must be closed profiles".to_string(),
                    );
                }
                let count = loft.sections.len();
                self.doc.lofts.push(loft);
                self.doc.bodies.push(crate::model::Body {
                    source: crate::model::BodySource::Loft(self.doc.lofts.len() - 1),
                    name: None,
                    deleted: false,
                    shadow: false,
                });
                // One shape-order marker for the pair; undo pops the body with the loft.
                self.doc.shape_order.push(ShapeKind::Loft);
                self.tool = Tool::Select;
                self.status = format!("Added loft ({count} sections)");
                self.refresh_document_health();
                ActionResult::Ok
            }
            Action::CreateDrawing { name } => {
                self.doc.drawings.push(crate::model::Drawing {
                    name: name.and_then(|n| {
                        let t = n.trim().to_string();
                        (!t.is_empty()).then_some(t)
                    }),
                    views: Vec::new(),
                    deleted: false,
                });
                let index = self.doc.drawings.len() - 1;
                self.editing_drawing = Some(index);
                self.status = format!("Added drawing {index}");
                ActionResult::Ok
            }
            Action::AddDrawingView {
                drawing,
                body,
                orientation,
            } => {
                if self.doc.bodies.get(body).is_none_or(|b| b.deleted) {
                    return ActionResult::Err(format!("No body {body}"));
                }
                let Some(d) = self.doc.drawings.get_mut(drawing).filter(|d| !d.deleted) else {
                    return ActionResult::Err(format!("No drawing {drawing}"));
                };
                d.views.push(crate::model::DrawingView {
                    body,
                    orientation,
                    dimensioned_edges: Vec::new(),
                    angle_dims: Vec::new(),
                });
                self.status = format!(
                    "Added {} view of body {body} to drawing {drawing}",
                    orientation.label()
                );
                ActionResult::Ok
            }
            Action::RemoveDrawingView { drawing, view } => {
                let Some(d) = self.doc.drawings.get_mut(drawing).filter(|d| !d.deleted) else {
                    return ActionResult::Err(format!("No drawing {drawing}"));
                };
                if view >= d.views.len() {
                    return ActionResult::Err(format!("No view {view} in drawing {drawing}"));
                }
                d.views.remove(view);
                self.status = format!("Removed view {view} from drawing {drawing}");
                ActionResult::Ok
            }
            Action::ToggleDrawingDimension {
                drawing,
                view,
                a,
                b,
            } => {
                // Order-normalize so the key is independent of which endpoint was clicked.
                let key = if a <= b { (a, b) } else { (b, a) };
                let Some(v) = self
                    .doc
                    .drawings
                    .get_mut(drawing)
                    .filter(|d| !d.deleted)
                    .and_then(|d| d.views.get_mut(view))
                else {
                    return ActionResult::Err(format!("No view {view} in drawing {drawing}"));
                };
                if let Some(pos) = v.dimensioned_edges.iter().position(|e| *e == key) {
                    v.dimensioned_edges.remove(pos);
                    self.status = "Hid edge dimension".to_string();
                } else {
                    v.dimensioned_edges.push(key);
                    self.status = "Showed edge dimension".to_string();
                }
                ActionResult::Ok
            }
            Action::ToggleDrawingAngle {
                drawing,
                view,
                edge1,
                edge2,
            } => {
                if edge1 == edge2 {
                    return ActionResult::Err("An angle needs two different edges".to_string());
                }
                // Order-normalize the pair so (e1, e2) == (e2, e1).
                let key = if edge1 <= edge2 {
                    (edge1, edge2)
                } else {
                    (edge2, edge1)
                };
                let Some(v) = self
                    .doc
                    .drawings
                    .get_mut(drawing)
                    .filter(|d| !d.deleted)
                    .and_then(|d| d.views.get_mut(view))
                else {
                    return ActionResult::Err(format!("No view {view} in drawing {drawing}"));
                };
                if let Some(pos) = v.angle_dims.iter().position(|e| *e == key) {
                    v.angle_dims.remove(pos);
                    self.status = "Hid angle dimension".to_string();
                } else {
                    v.angle_dims.push(key);
                    self.status = "Showed angle dimension".to_string();
                }
                ActionResult::Ok
            }
            Action::EditDrawing { drawing } => {
                if let Some(di) = drawing {
                    if self.doc.drawings.get(di).is_none_or(|d| d.deleted) {
                        return ActionResult::Err(format!("No drawing {di}"));
                    }
                }
                self.editing_drawing = drawing;
                ActionResult::Ok
            }
            Action::CommitRepeat => {
                let Some(mut cr) = self.creating_repeat.take() else {
                    return ActionResult::Err("No repeat in progress".to_string());
                };
                if let Err(e) = commit_inline_parameter_defs(
                    &mut self.doc,
                    [&mut cr.count, &mut cr.spacing, &mut cr.length],
                ) {
                    self.status = e.clone();
                    self.creating_repeat = Some(cr);
                    return ActionResult::Err(e);
                }
                let result = match cr.editing {
                    Some(op) => self.apply(Action::EditRepeatOperation {
                        op,
                        targets: cr.targets.clone(),
                        plane_targets: cr.plane_targets.clone(),
                        axis: cr.axis,
                        mode: cr.mode,
                        count: cr.count.clone(),
                        spacing: cr.spacing.clone(),
                        length: cr.length.clone(),
                    }),
                    None => self.apply(Action::CreateRepeatOperation {
                        targets: cr.targets.clone(),
                        plane_targets: cr.plane_targets.clone(),
                        axis: cr.axis,
                        mode: cr.mode,
                        count: cr.count.clone(),
                        spacing: cr.spacing.clone(),
                        length: cr.length.clone(),
                    }),
                };
                if matches!(result, ActionResult::Err(_)) {
                    self.creating_repeat = Some(cr);
                } else {
                    self.creating_repeat = Some(CreatingRepeat::default());
                }
                result
            }
            Action::CreateRepeatOperation { targets, plane_targets, axis, mode, count, spacing, length } => {
                if let Err(e) = validate_repeat_inputs(&self.doc, &targets, &plane_targets) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let op_index = self.doc.repeat_ops.len();
                self.doc.repeat_ops.push(crate::model::RepeatOperation {
                    targets: targets.clone(),
                    plane_targets: plane_targets.clone(),
                    axis,
                    mode,
                    count,
                    spacing,
                    length,
                    length_target: None,
                    outputs: Vec::new(),
                    plane_outputs: Vec::new(),
                    name: None,
                    deleted: false,
                });
                let Some(offsets) =
                    crate::extrude::repeat_offsets(&self.doc, &self.doc.repeat_ops[op_index])
                else {
                    self.doc.repeat_ops.pop();
                    let e =
                        "Repeat doesn't evaluate (check count/spacing/length and the axis)"
                            .to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                };
                if offsets.is_empty() {
                    self.doc.repeat_ops.pop();
                    let e = "Repeat produces no extra instances".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                self.doc.shape_order.push(ShapeKind::RepeatOperation);
                let mut outputs = Vec::new();
                for instance in 1..=offsets.len() {
                    for (ti, _) in targets.iter().enumerate() {
                        outputs.push(self.doc.bodies.len());
                        self.doc.bodies.push(crate::model::Body {
                            source: crate::model::BodySource::Repeated {
                                op: op_index,
                                target: ti,
                                instance,
                            },
                            name: None,
                            deleted: false,
                            shadow: false,
                        });
                        self.doc.shape_order.push(ShapeKind::Body);
                    }
                }
                self.doc.repeat_ops[op_index].outputs = outputs;
                // Generated plane instances (#221), same instance-major-then-target layout.
                let mut plane_outputs = Vec::new();
                for instance in 1..=offsets.len() {
                    for (ti, &src) in plane_targets.iter().enumerate() {
                        let mut plane = self.doc.construction_planes[src].clone();
                        plane.repeat_instance = Some(crate::model::RepeatPlaneInstance {
                            op: op_index,
                            target: ti,
                            instance,
                        });
                        // Instances group under the repeat op, not under the source's parent.
                        plane.parent = crate::model::ConstructionPlaneParent::Root;
                        plane.name = None;
                        plane_outputs.push(self.doc.construction_planes.len());
                        self.doc.construction_planes.push(plane);
                        self.doc.shape_order.push(ShapeKind::ConstructionPlane);
                    }
                }
                self.doc.repeat_ops[op_index].plane_outputs = plane_outputs;
                recompute_repeated_planes(&mut self.doc);
                self.refresh_document_health();
                self.status = format!(
                    "Repeated {} × {} instances",
                    repeat_input_status(targets.len(), plane_targets.len()),
                    offsets.len() + 1
                );
                ActionResult::Ok
            }
            Action::EditRepeatOperation { op, targets, plane_targets, axis, mode, count, spacing, length } => {
                if self.doc.repeat_ops.get(op).filter(|o| !o.deleted).is_none() {
                    let e = format!("Repeat operation {op} not found");
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if let Err(e) = validate_repeat_inputs(&self.doc, &targets, &plane_targets) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let old = self.doc.repeat_ops[op].clone();
                {
                    let entry = &mut self.doc.repeat_ops[op];
                    entry.targets = targets.clone();
                    entry.plane_targets = plane_targets.clone();
                    entry.axis = axis;
                    entry.mode = mode;
                    entry.count = count;
                    entry.spacing = spacing;
                    entry.length = length;
                }
                let Some(offsets) =
                    crate::extrude::repeat_offsets(&self.doc, &self.doc.repeat_ops[op])
                else {
                    self.doc.repeat_ops[op] = old;
                    let e =
                        "Repeat doesn't evaluate (check count/spacing/length and the axis)"
                            .to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                };
                // Resize body outputs to instance-count × targets (tombstone extras, grow new).
                let want = offsets.len() * targets.len();
                let have = self.doc.repeat_ops[op].outputs.len();
                if want > have {
                    let mut outputs = self.doc.repeat_ops[op].outputs.clone();
                    for slot in have..want {
                        let instance = slot / targets.len() + 1;
                        let ti = slot % targets.len();
                        outputs.push(self.doc.bodies.len());
                        self.doc.bodies.push(crate::model::Body {
                            source: crate::model::BodySource::Repeated {
                                op,
                                target: ti,
                                instance,
                            },
                            name: None,
                            deleted: false,
                            shadow: false,
                        });
                        self.doc.shape_order.push(ShapeKind::Body);
                        self.doc.undo_groups.push(1);
                    }
                    self.doc.repeat_ops[op].outputs = outputs;
                } else if want < have {
                    let extras = self.doc.repeat_ops[op].outputs.split_off(want);
                    for out in extras {
                        if let Some(body) = self.doc.bodies.get_mut(out) {
                            body.deleted = true;
                        }
                    }
                }
                // Re-point surviving outputs at the (possibly reordered) target list.
                let outputs = self.doc.repeat_ops[op].outputs.clone();
                for (slot, &out) in outputs.iter().enumerate() {
                    let instance = slot / targets.len() + 1;
                    let ti = slot % targets.len();
                    if let Some(body) = self.doc.bodies.get_mut(out) {
                        body.source = crate::model::BodySource::Repeated {
                            op,
                            target: ti,
                            instance,
                        };
                    }
                }
                // Same resize/re-point for the generated plane instances (#221).
                let want_p = offsets.len() * plane_targets.len();
                let have_p = self.doc.repeat_ops[op].plane_outputs.len();
                if want_p > have_p {
                    let mut plane_outputs = self.doc.repeat_ops[op].plane_outputs.clone();
                    for slot in have_p..want_p {
                        let instance = slot / plane_targets.len() + 1;
                        let ti = slot % plane_targets.len();
                        let src = plane_targets[ti];
                        let mut plane = self.doc.construction_planes[src].clone();
                        plane.repeat_instance = Some(crate::model::RepeatPlaneInstance {
                            op,
                            target: ti,
                            instance,
                        });
                        plane.parent = crate::model::ConstructionPlaneParent::Root;
                        plane.name = None;
                        plane_outputs.push(self.doc.construction_planes.len());
                        self.doc.construction_planes.push(plane);
                        self.doc.shape_order.push(ShapeKind::ConstructionPlane);
                        self.doc.undo_groups.push(1);
                    }
                    self.doc.repeat_ops[op].plane_outputs = plane_outputs;
                } else if want_p < have_p {
                    let extras = self.doc.repeat_ops[op].plane_outputs.split_off(want_p);
                    for out in extras {
                        if let Some(plane) = self.doc.construction_planes.get_mut(out) {
                            plane.deleted = true;
                        }
                    }
                }
                let plane_outputs = self.doc.repeat_ops[op].plane_outputs.clone();
                for (slot, &out) in plane_outputs.iter().enumerate() {
                    let instance = slot / plane_targets.len() + 1;
                    let ti = slot % plane_targets.len();
                    if let Some(plane) = self.doc.construction_planes.get_mut(out) {
                        plane.repeat_instance = Some(crate::model::RepeatPlaneInstance {
                            op,
                            target: ti,
                            instance,
                        });
                    }
                }
                recompute_repeated_planes(&mut self.doc);
                self.refresh_document_health();
                self.status = "Edited repeat".to_string();
                ActionResult::Ok
            }
            Action::CommitMove => {
                let Some(mut cm) = self.creating_move.take() else {
                    return ActionResult::Err("No move in progress".to_string());
                };
                if let Err(e) = commit_inline_parameter_defs(
                    &mut self.doc,
                    [&mut cm.tx, &mut cm.ty, &mut cm.tz, &mut cm.angle],
                ) {
                    self.status = e.clone();
                    self.creating_move = Some(cm);
                    return ActionResult::Err(e);
                }
                let result = match cm.editing {
                    Some(op) => self.apply(Action::EditMoveOperation {
                        op,
                        targets: cm.targets.clone(),
                        plane_targets: cm.plane_targets.clone(),
                        image_targets: cm.image_targets.clone(),
                        tx: cm.tx.clone(),
                        ty: cm.ty.clone(),
                        tz: cm.tz.clone(),
                        axis: cm.axis,
                        angle: cm.angle.clone(),
                    }),
                    None => {
                        // Coalesce (#217): re-moving the same element folds into its existing
                        // move op (when the combined transform is representable) instead of
                        // stacking a new tiny op.
                        match coalescible_move_op(&self.doc, &cm)
                            .and_then(|op| compose_move_values(&self.doc, &self.doc.move_ops[op], &cm).map(|v| (op, v)))
                        {
                            Some((op, (tx, ty, tz, axis, angle))) => {
                                self.apply(Action::EditMoveOperation {
                                    op,
                                    targets: self.doc.move_ops[op].targets.clone(),
                                    plane_targets: self.doc.move_ops[op].plane_targets.clone(),
                                    image_targets: self.doc.move_ops[op].image_targets.clone(),
                                    tx,
                                    ty,
                                    tz,
                                    axis,
                                    angle,
                                })
                            }
                            None => self.apply(Action::CreateMoveOperation {
                                targets: cm.targets.clone(),
                                plane_targets: cm.plane_targets.clone(),
                                image_targets: cm.image_targets.clone(),
                                tx: cm.tx.clone(),
                                ty: cm.ty.clone(),
                                tz: cm.tz.clone(),
                                axis: cm.axis,
                                angle: cm.angle.clone(),
                            }),
                        }
                    }
                };
                if matches!(result, ActionResult::Err(_)) {
                    self.creating_move = Some(cm);
                } else {
                    self.creating_move = Some(CreatingMove::default());
                }
                result
            }
            Action::CreateMoveOperation { targets, plane_targets, image_targets, tx, ty, tz, axis, angle } => {
                if targets.is_empty() && plane_targets.is_empty() && image_targets.is_empty() {
                    let e = "Pick at least one body, plane, or image to move".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if !targets.is_empty() {
                    if let Err(e) = validate_move_inputs(&self.doc, &targets, None) {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let op_index = self.doc.move_ops.len();
                self.doc.move_ops.push(crate::model::MoveOperation {
                    targets: targets.clone(),
                    plane_targets: plane_targets.clone(),
                    image_targets: image_targets.clone(),
                    tx,
                    ty,
                    tz,
                    axis,
                    angle,
                    outputs: Vec::new(),
                    name: None,
                    deleted: false,
                });
                if crate::extrude::move_op_transform(&self.doc, &self.doc.move_ops[op_index])
                    .is_none()
                {
                    self.doc.move_ops.pop();
                    let e = "Move amounts don't evaluate (check expressions and axis)".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                self.doc.shape_order.push(ShapeKind::MoveOperation);
                let mut outputs = Vec::with_capacity(targets.len());
                for (ordinal, _) in targets.iter().enumerate() {
                    outputs.push(self.doc.bodies.len());
                    self.doc.bodies.push(crate::model::Body {
                        source: crate::model::BodySource::Moved {
                            op: op_index,
                            target: ordinal,
                        },
                        name: None,
                        deleted: false,
                        shadow: false,
                    });
                    self.doc.shape_order.push(ShapeKind::Body);
                }
                self.doc.move_ops[op_index].outputs = outputs;
                for &input in &targets {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = true;
                    }
                }
                recompute_moved_planes(&mut self.doc);
                recompute_moved_images(&mut self.doc);
                recompute_repeated_planes(&mut self.doc);
                self.refresh_document_health();
                self.status = move_status(targets.len(), plane_targets.len(), image_targets.len());
                ActionResult::Ok
            }
            Action::EditMoveOperation { op, targets, plane_targets, image_targets, tx, ty, tz, axis, angle } => {
                if self.doc.move_ops.get(op).filter(|o| !o.deleted).is_none() {
                    let e = format!("Move operation {op} not found");
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if !targets.is_empty() {
                    if let Err(e) = validate_move_inputs(&self.doc, &targets, Some(op)) {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                let old = self.doc.move_ops[op].clone();
                for &input in &old.targets {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = false;
                    }
                }
                {
                    let entry = &mut self.doc.move_ops[op];
                    entry.targets = targets.clone();
                    entry.plane_targets = plane_targets.clone();
                    entry.image_targets = image_targets.clone();
                    entry.tx = tx;
                    entry.ty = ty;
                    entry.tz = tz;
                    entry.axis = axis;
                    entry.angle = angle;
                }
                if crate::extrude::move_op_transform(&self.doc, &self.doc.move_ops[op]).is_none() {
                    self.doc.move_ops[op] = old.clone();
                    for &input in &old.targets {
                        if let Some(body) = self.doc.bodies.get_mut(input) {
                            body.shadow = true;
                        }
                    }
                    let e = "Move amounts don't evaluate (check expressions and axis)".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                // The edit may change how many outputs the op *should* have; keep the
                // committed output count (extra targets reuse the last output's slot is
                // not possible for moves — instead cap targets to outputs, or grow).
                for &input in &targets {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = true;
                    }
                }
                for &input in old.targets.iter() {
                    if crate::model::body_shadowed_by_other_ops(&self.doc, input, None, Some(op), None) {
                        if let Some(body) = self.doc.bodies.get_mut(input) {
                            body.shadow = true;
                        }
                    }
                }
                // Grow/shrink outputs to match the new target list (tombstoning removed
                // ones keeps later body indices stable).
                let have = self.doc.move_ops[op].outputs.len();
                if targets.len() > have {
                    let mut outputs = self.doc.move_ops[op].outputs.clone();
                    for ordinal in have..targets.len() {
                        outputs.push(self.doc.bodies.len());
                        self.doc.bodies.push(crate::model::Body {
                            source: crate::model::BodySource::Moved { op, target: ordinal },
                            name: None,
                            deleted: false,
                            shadow: false,
                        });
                        self.doc.shape_order.push(ShapeKind::Body);
                        self.doc.undo_groups.push(1);
                    }
                    self.doc.move_ops[op].outputs = outputs;
                } else if targets.len() < have {
                    let outputs = self.doc.move_ops[op].outputs.split_off(targets.len());
                    for out in outputs {
                        if let Some(body) = self.doc.bodies.get_mut(out) {
                            body.deleted = true;
                        }
                    }
                }
                recompute_moved_planes(&mut self.doc);
                recompute_moved_images(&mut self.doc);
                recompute_repeated_planes(&mut self.doc);
                self.refresh_document_health();
                self.status = "Edited move".to_string();
                ActionResult::Ok
            }
            Action::CommitBoolean => {
                let Some(cb) = self.creating_boolean.take() else {
                    return ActionResult::Err("No boolean operation in progress".to_string());
                };
                let result = match cb.editing {
                    Some(op) => self.apply(Action::EditBooleanOperation {
                        op,
                        kind: cb.kind,
                        a: cb.a.clone(),
                        b: cb.b.clone(),
                        keep_b: cb.keep_b,
                    }),
                    None => self.apply(Action::CreateBooleanOperation {
                        kind: cb.kind,
                        a: cb.a.clone(),
                        b: cb.b.clone(),
                        keep_b: cb.keep_b,
                    }),
                };
                if matches!(result, ActionResult::Err(_)) {
                    self.creating_boolean = Some(cb);
                } else {
                    self.creating_boolean = Some(CreatingBoolean::default());
                }
                result
            }
            Action::CreateBooleanOperation { kind, a, b, keep_b } => {
                if let Err(e) = validate_boolean_inputs(&self.doc, kind, &a, &b, None) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let op_index = self.doc.boolean_ops.len();
                self.doc.boolean_ops.push(crate::model::BooleanOperation {
                    kind,
                    a: a.clone(),
                    b: b.clone(),
                    keep_b,
                    outputs: Vec::new(),
                    name: None,
                    deleted: false,
                });
                // Output body count = solids the kernel finds in the result right now
                // (at least one). A later rebuild that produces more solids folds the
                // extras into the last output body; fewer leaves trailing outputs empty.
                let solids = crate::extrude::boolean_result_solid_count(&self.doc, op_index)
                    .unwrap_or(1)
                    .max(1);
                self.doc.shape_order.push(ShapeKind::BooleanOperation);
                let mut outputs = Vec::with_capacity(solids);
                for ordinal in 0..solids {
                    outputs.push(self.doc.bodies.len());
                    self.doc.bodies.push(crate::model::Body {
                        source: crate::model::BodySource::Boolean {
                            op: op_index,
                            solid: ordinal,
                        },
                        name: None,
                        deleted: false,
                        shadow: false,
                    });
                    self.doc.shape_order.push(ShapeKind::Body);
                }
                self.doc.boolean_ops[op_index].outputs = outputs;
                for &input in a.iter() {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = true;
                    }
                }
                if !keep_b {
                    for &input in b.iter() {
                        if let Some(body) = self.doc.bodies.get_mut(input) {
                            body.shadow = true;
                        }
                    }
                }
                self.refresh_document_health();
                self.status = format!(
                    "{}: {} new body(ies) from {} input(s)",
                    kind.label(),
                    solids,
                    a.len() + b.len()
                );
                ActionResult::Ok
            }
            Action::EditBooleanOperation { op, kind, a, b, keep_b } => {
                if self
                    .doc
                    .boolean_ops
                    .get(op)
                    .filter(|o| !o.deleted)
                    .is_none()
                {
                    let e = format!("Boolean operation {op} not found");
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if let Err(e) = validate_boolean_inputs(&self.doc, kind, &a, &b, Some(op)) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                // Release the old inputs from shadow, re-point the op, shadow the new ones.
                let old = self.doc.boolean_ops[op].clone();
                for &input in old.a.iter().chain(old.b.iter()) {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = false;
                    }
                }
                {
                    let entry = &mut self.doc.boolean_ops[op];
                    entry.kind = kind;
                    entry.a = a.clone();
                    entry.b = b.clone();
                    entry.keep_b = keep_b;
                }
                for &input in a.iter() {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = true;
                    }
                }
                if !keep_b {
                    for &input in b.iter() {
                        if let Some(body) = self.doc.bodies.get_mut(input) {
                            body.shadow = true;
                        }
                    }
                }
                // Inputs shadowed by *other* live ops must stay shadows even if this op
                // just dropped them.
                let ops = self.doc.boolean_ops.clone();
                for (oi, o) in ops.iter().enumerate() {
                    if o.deleted || oi == op {
                        continue;
                    }
                    for &input in o.a.iter().chain((!o.keep_b).then_some(&o.b).into_iter().flatten()) {
                        if let Some(body) = self.doc.bodies.get_mut(input) {
                            body.shadow = true;
                        }
                    }
                }
                self.refresh_document_health();
                self.status = format!("Edited {}", kind.label());
                ActionResult::Ok
            }
            Action::CommitSlice => {
                let Some(cs) = self.creating_slice.take() else {
                    return ActionResult::Err("No slice operation in progress".to_string());
                };
                let result = match cs.editing {
                    Some(op) => self.apply(Action::EditSliceOperation {
                        op,
                        targets: cs.targets.clone(),
                        cutters: cs.cutters.clone(),
                        extend_infinite: cs.extend_infinite,
                    }),
                    None => self.apply(Action::CreateSliceOperation {
                        targets: cs.targets.clone(),
                        cutters: cs.cutters.clone(),
                        extend_infinite: cs.extend_infinite,
                    }),
                };
                if matches!(result, ActionResult::Err(_)) {
                    self.creating_slice = Some(cs);
                } else {
                    self.creating_slice = Some(CreatingSlice::default());
                }
                result
            }
            Action::CreateSliceOperation { targets, cutters, extend_infinite } => {
                if let Err(e) = validate_slice_inputs(&self.doc, &targets, &cutters, None) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let op_index = self.doc.slice_ops.len();
                self.doc.slice_ops.push(crate::model::SliceOperation {
                    targets: targets.clone(),
                    cutters: cutters.clone(),
                    extend_infinite,
                    outputs: Vec::new(),
                    name: None,
                    deleted: false,
                });
                self.doc.shape_order.push(ShapeKind::SliceOperation);
                // Fragment count per target = solids the kernel finds after cutting (at
                // least one). A target the cutters miss stays whole (one fragment).
                let mut outputs = Vec::new();
                for target in 0..targets.len() {
                    let pieces = crate::extrude::slice_piece_count(&self.doc, op_index, target)
                        .unwrap_or(1)
                        .max(1);
                    for piece in 0..pieces {
                        outputs.push(self.doc.bodies.len());
                        self.doc.bodies.push(crate::model::Body {
                            source: crate::model::BodySource::Sliced { op: op_index, target, piece },
                            name: None,
                            deleted: false,
                            shadow: false,
                        });
                        self.doc.shape_order.push(ShapeKind::Body);
                    }
                }
                let fragments = outputs.len();
                self.doc.slice_ops[op_index].outputs = outputs;
                for &input in targets.iter() {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = true;
                    }
                }
                self.refresh_document_health();
                self.status = format!(
                    "Slice: {} fragment(s) from {} body(ies)",
                    fragments,
                    targets.len()
                );
                ActionResult::Ok
            }
            Action::EditSliceOperation { op, targets, cutters, extend_infinite } => {
                if self.doc.slice_ops.get(op).filter(|o| !o.deleted).is_none() {
                    let e = format!("Slice operation {op} not found");
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                if let Err(e) = validate_slice_inputs(&self.doc, &targets, &cutters, Some(op)) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                // Release old inputs from shadow, re-point the op, shadow the new ones.
                let old_targets = self.doc.slice_ops[op].targets.clone();
                for &input in old_targets.iter() {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = false;
                    }
                }
                {
                    let entry = &mut self.doc.slice_ops[op];
                    entry.targets = targets.clone();
                    entry.cutters = cutters.clone();
                    entry.extend_infinite = extend_infinite;
                }
                for &input in targets.iter() {
                    if let Some(body) = self.doc.bodies.get_mut(input) {
                        body.shadow = true;
                    }
                }
                // Inputs shadowed by other live ops must stay shadows even if this op just
                // dropped them.
                for &input in old_targets.iter() {
                    if crate::model::body_shadowed_by_other_ops(&self.doc, input, None, None, Some(op)) {
                        if let Some(body) = self.doc.bodies.get_mut(input) {
                            body.shadow = true;
                        }
                    }
                }
                // Recompute the target-major fragment list and reconcile the output bodies:
                // reuse existing slots, push new ones for a surplus, tombstone the shortfall.
                let desired: Vec<(usize, usize)> = (0..targets.len())
                    .flat_map(|target| {
                        let pieces = crate::extrude::slice_piece_count(&self.doc, op, target)
                            .unwrap_or(1)
                            .max(1);
                        (0..pieces).map(move |piece| (target, piece))
                    })
                    .collect();
                let existing = self.doc.slice_ops[op].outputs.clone();
                let mut outputs = Vec::with_capacity(desired.len());
                for (i, &(target, piece)) in desired.iter().enumerate() {
                    let source = crate::model::BodySource::Sliced { op, target, piece };
                    if let Some(&bi) = existing.get(i) {
                        if let Some(body) = self.doc.bodies.get_mut(bi) {
                            body.source = source;
                            body.deleted = false;
                        }
                        outputs.push(bi);
                    } else {
                        outputs.push(self.doc.bodies.len());
                        self.doc.bodies.push(crate::model::Body {
                            source,
                            name: None,
                            deleted: false,
                            shadow: false,
                        });
                        self.doc.shape_order.push(ShapeKind::Body);
                        self.doc.undo_groups.push(1);
                    }
                }
                for &bi in existing.iter().skip(desired.len()) {
                    if let Some(body) = self.doc.bodies.get_mut(bi) {
                        body.deleted = true;
                    }
                }
                self.doc.slice_ops[op].outputs = outputs;
                self.refresh_document_health();
                self.status = "Edited slice".to_string();
                ActionResult::Ok
            }
            Action::CommitRevolve => {
                let Some(mut cr) = self.creating_revolve.take() else {
                    return ActionResult::Err("No revolve in progress".to_string());
                };
                if let Err(e) = commit_inline_parameter_defs(&mut self.doc, [&mut cr.text]) {
                    self.status = e.clone();
                    self.creating_revolve = Some(cr);
                    return ActionResult::Err(e);
                }
                if cr.faces.is_empty() {
                    self.creating_revolve = Some(cr);
                    let e = "Pick at least one profile face to revolve".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let (Some(axis), Some(sketch)) = (cr.axis, cr.sketch) else {
                    let e = "Pick a line (or global axis) to revolve around".to_string();
                    self.creating_revolve = Some(cr);
                    self.status = e.clone();
                    return ActionResult::Err(e);
                };
                let angle = cr.evaluated_angle_deg(&self.doc);
                if !(angle > 0.0) {
                    self.creating_revolve = Some(cr);
                    let e = "Revolve angle must be positive".to_string();
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                let mode = match self.resolve_revolve_mode(
                    sketch,
                    &cr.faces,
                    axis,
                    angle,
                    cr.symmetric,
                    cr.body_choice,
                    &cr.cut_bodies,
                ) {
                    Ok(mode) => mode,
                    Err(e) => {
                        self.creating_revolve = Some(cr);
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                };
                let result = match cr.editing {
                    Some(op) => self.edit_revolution(
                        op,
                        sketch,
                        cr.faces.clone(),
                        axis,
                        angle,
                        cr.symmetric,
                        mode,
                    ),
                    None => self.create_revolution(
                        sketch,
                        cr.faces.clone(),
                        axis,
                        angle,
                        cr.symmetric,
                        mode,
                    ),
                };
                if matches!(result, ActionResult::Err(_)) {
                    self.creating_revolve = Some(cr);
                }
                result
            }
            Action::CreateRevolution {
                sketch,
                faces,
                axis,
                angle_deg,
                symmetric,
                body,
                bodies,
            } => {
                let mode = match self.resolve_revolve_mode(
                    sketch, &faces, axis, angle_deg, symmetric, body, &bodies,
                ) {
                    Ok(mode) => mode,
                    Err(e) => {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                };
                self.create_revolution(sketch, faces, axis, angle_deg, symmetric, mode)
            }
            Action::SetSnapping(enabled) => {
                self.snapping_enabled = enabled;
                self.active_snap = None;
                self.status = if enabled {
                    "Snapping on".to_string()
                } else {
                    "Snapping off".to_string()
                };
                ActionResult::Ok
            }
            Action::ApplySnapConstraint { point, target } => {
                let Some(sketch) = self.sketch_session.map(|s| s.sketch) else {
                    return ActionResult::Err("Not in sketch mode".to_string());
                };
                match self.add_snap_constraint(sketch, point, target) {
                    Ok(()) => ActionResult::Ok,
                    Err(e) => ActionResult::Err(e),
                }
            }
            Action::ClickSceneElement { element, additive } => {
                // A body clicked while a body-gathering tool is active feeds that tool's set
                // (#218) — the Elements pane picks bodies regardless of viewport sub-element
                // picking — rather than touching the persistent selection. A line clicked while
                // the Move tool is active sets its rotation axis (#216), like a viewport pick.
                let consumed_by_tool = match &element {
                    SceneElement::Body(bi) => toggle_body_in_active_tool(self, *bi),
                    SceneElement::Line(li) if self.tool == Tool::Move => {
                        self.creating_move
                            .get_or_insert_with(CreatingMove::default)
                            .axis = Some(crate::model::RevolveAxis::Line(*li));
                        true
                    }
                    // A construction plane clicked with the Move tool joins its plane set (#217).
                    SceneElement::ConstructionPlane(pi) if self.tool == Tool::Move => {
                        let set = &mut self
                            .creating_move
                            .get_or_insert_with(CreatingMove::default)
                            .plane_targets;
                        if let Some(pos) = set.iter().position(|p| p == pi) {
                            set.remove(pos);
                        } else {
                            set.push(*pi);
                        }
                        true
                    }
                    // A construction plane clicked with the Repeat tool joins its plane set (#221).
                    SceneElement::ConstructionPlane(pi) if self.tool == Tool::Repeat => {
                        let set = &mut self
                            .creating_repeat
                            .get_or_insert_with(CreatingRepeat::default)
                            .plane_targets;
                        if let Some(pos) = set.iter().position(|p| p == pi) {
                            set.remove(pos);
                        } else {
                            set.push(*pi);
                        }
                        true
                    }
                    // A tracing image clicked with the Move tool joins its image set (#217).
                    SceneElement::Image(ii) if self.tool == Tool::Move => {
                        let set = &mut self
                            .creating_move
                            .get_or_insert_with(CreatingMove::default)
                            .image_targets;
                        if let Some(pos) = set.iter().position(|i| i == ii) {
                            set.remove(pos);
                        } else {
                            set.push(*ii);
                        }
                        true
                    }
                    _ => false,
                };
                if !consumed_by_tool {
                    click_scene_selection(&mut self.scene_selection, element, additive);
                    if let Some((health_status, reason)) =
                        selection_frozen_summary(&self.document_health, &self.scene_selection)
                    {
                        self.status = format!(
                            "{} — {}",
                            health_status_label(health_status),
                            reason
                        );
                    }
                }
                ActionResult::Ok
            }
            Action::ClearSceneSelection => {
                self.scene_selection.clear();
                ActionResult::Ok
            }
            Action::SetShapeConstruction {
                element,
                construction,
            } => {
                if let Err(e) = require_element_editable(&self.document_health, element.clone()) {
                    return ActionResult::Err(e);
                }
                match set_edge_construction(&mut self.doc, element.clone(), construction) {
                Ok(()) => {
                    self.status = format!(
                        "{} {}",
                        element_label(element),
                        if construction {
                            "marked construction"
                        } else {
                            "marked solid"
                        }
                    );
                    ActionResult::Ok
                }
                Err(e) => ActionResult::Err(e),
                }
            }
            Action::ApplyConstruction { construction } => {
                if let Some(cr) = &mut self.creating_rect {
                    cr.construction = construction;
                    self.draw_construction = construction;
                    self.status = draw_mode_status("Rectangle", construction);
                    return ActionResult::Ok;
                }
                if let Some(cl) = &mut self.creating_line {
                    cl.construction = construction;
                    self.draw_construction = construction;
                    self.status = draw_mode_status("Line", construction);
                    return ActionResult::Ok;
                }
                if let Some(cc) = &mut self.creating_circle {
                    cc.construction = construction;
                    self.draw_construction = construction;
                    self.status = draw_mode_status("Circle", construction);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Rectangle {
                    self.draw_construction = construction;
                    self.status = draw_mode_status("Rectangle", construction);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Line {
                    self.draw_construction = construction;
                    self.status = draw_mode_status("Line", construction);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Circle {
                    self.draw_construction = construction;
                    self.status = draw_mode_status("Circle", construction);
                    return ActionResult::Ok;
                }
                if let Err(e) =
                    require_construction_targets_editable(&self.document_health, &self.scene_selection)
                {
                    return ActionResult::Err(e);
                }
                let targets = construction_targets_from_selection(&self.scene_selection);
                match set_construction_for_targets(&mut self.doc, &targets, construction) {
                    Ok(count) if count > 0 => {
                        self.status = format!(
                            "{count} item(s) marked {}",
                            if construction {
                                "construction"
                            } else {
                                "substantial"
                            }
                        );
                        ActionResult::Ok
                    }
                    Ok(_) => ActionResult::Err("No constructable geometry selected".to_string()),
                    Err(e) => ActionResult::Err(e),
                }
            }
            Action::ToggleConstruction => {
                if let Some(cr) = &mut self.creating_rect {
                    cr.construction = !cr.construction;
                    self.draw_construction = cr.construction;
                    self.status = draw_mode_status("Rectangle", cr.construction);
                    return ActionResult::Ok;
                }
                if let Some(cl) = &mut self.creating_line {
                    cl.construction = !cl.construction;
                    self.draw_construction = cl.construction;
                    self.status = draw_mode_status("Line", cl.construction);
                    return ActionResult::Ok;
                }
                if let Some(cc) = &mut self.creating_circle {
                    cc.construction = !cc.construction;
                    self.draw_construction = cc.construction;
                    self.status = draw_mode_status("Circle", cc.construction);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Rectangle {
                    self.draw_construction = !self.draw_construction;
                    self.status = draw_mode_status("Rectangle", self.draw_construction);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Line {
                    self.draw_construction = !self.draw_construction;
                    self.status = draw_mode_status("Line", self.draw_construction);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Circle {
                    self.draw_construction = !self.draw_construction;
                    self.status = draw_mode_status("Circle", self.draw_construction);
                    return ActionResult::Ok;
                }
                if let Err(e) =
                    require_construction_targets_editable(&self.document_health, &self.scene_selection)
                {
                    return ActionResult::Err(e);
                }
                let targets = construction_targets_from_selection(&self.scene_selection);
                match toggle_construction_for_targets(&mut self.doc, &targets) {
                    Ok(count) if count > 0 => {
                        self.status = format!("Toggled construction on {count} item(s)");
                        ActionResult::Ok
                    }
                    Ok(_) => ActionResult::Err("No constructable geometry selected".to_string()),
                    Err(e) => ActionResult::Err(e),
                }
            }
            Action::ApplyCurveMode { curve_mode } => {
                if let Some(cl) = &mut self.creating_line {
                    cl.curve_mode = curve_mode;
                    self.draw_curve_mode = curve_mode;
                    self.status = curve_mode_status(curve_mode);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Line {
                    self.draw_curve_mode = curve_mode;
                    self.status = curve_mode_status(curve_mode);
                    return ActionResult::Ok;
                }
                ActionResult::Err("Select the line tool to set curve mode".to_string())
            }
            Action::ToggleCurveMode => {
                if let Some(cl) = &mut self.creating_line {
                    cl.curve_mode = !cl.curve_mode;
                    self.draw_curve_mode = cl.curve_mode;
                    self.status = curve_mode_status(cl.curve_mode);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Line {
                    self.draw_curve_mode = !self.draw_curve_mode;
                    self.status = curve_mode_status(self.draw_curve_mode);
                    return ActionResult::Ok;
                }
                match self.toggle_curve_at_selected_vertices() {
                    Ok(status) => {
                        self.status = status;
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::ApplyTangentConstraint { tangent_constraint } => {
                if let Some(cl) = &mut self.creating_line {
                    cl.tangent_constraint = tangent_constraint;
                    self.draw_tangent_constraint = tangent_constraint;
                    self.status = tangent_constraint_status(tangent_constraint);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Line {
                    self.draw_tangent_constraint = tangent_constraint;
                    self.status = tangent_constraint_status(tangent_constraint);
                    return ActionResult::Ok;
                }
                ActionResult::Err("Select the line tool to set the tangent constraint".to_string())
            }
            Action::ToggleTangentConstraint => {
                if let Some(cl) = &mut self.creating_line {
                    cl.tangent_constraint = !cl.tangent_constraint;
                    self.draw_tangent_constraint = cl.tangent_constraint;
                    self.status = tangent_constraint_status(cl.tangent_constraint);
                    return ActionResult::Ok;
                }
                if self.tool == Tool::Line {
                    self.draw_tangent_constraint = !self.draw_tangent_constraint;
                    self.status = tangent_constraint_status(self.draw_tangent_constraint);
                    return ActionResult::Ok;
                }
                match self.toggle_tangent_at_selected_vertices() {
                    Ok(status) => {
                        self.status = status;
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::SetVertexTangent { point, continuous } => {
                let Some(sketch) = crate::construction::point_sketch(&self.doc, point.clone()) else {
                    return ActionResult::Err("Vertex no longer exists".to_string());
                };
                let Some([(line1, _), (line2, _)]) =
                    vertex_drag::incident_two_lines(&self.doc, sketch, point.clone())
                else {
                    return ActionResult::Err(
                        "Vertex must join exactly two lines to set tangency".to_string(),
                    );
                };
                for &li in &[line1, line2] {
                    if let Err(e) =
                        require_element_editable(&self.document_health, SceneElement::Line(li))
                    {
                        self.status = e.clone();
                        return ActionResult::Err(e);
                    }
                }
                if continuous {
                    return self.apply(Action::ConvertVertexToBezier { point });
                }
                let Some([(line1, end1), (line2, end2)]) =
                    vertex_drag::incident_two_lines(&self.doc, sketch, point.clone())
                else {
                    return ActionResult::Err(
                        "Vertex must join exactly two lines to set tangency".to_string(),
                    );
                };
                for (line, end) in [(line1, end1), (line2, end2)] {
                    let Some(l) = self.doc.lines.get(line) else { continue };
                    let (v, far) = match end {
                        LineEnd::Start => ((l.x0, l.y0), (l.x1, l.y1)),
                        LineEnd::End => ((l.x1, l.y1), (l.x0, l.y0)),
                    };
                    let near_handle = independent_corner_handle(v, far);
                    let far_handle = l
                        .bezier
                        .map(|b| match end {
                            LineEnd::Start => b[1],
                            LineEnd::End => b[0],
                        })
                        .unwrap_or_else(|| independent_corner_handle(far, v));
                    if let Some(l) = self.doc.lines.get_mut(line) {
                        l.bezier = Some(match end {
                            LineEnd::Start => [near_handle, far_handle],
                            LineEnd::End => [far_handle, near_handle],
                        });
                    }
                }
                self.status = "Made handles independent".to_string();
                ActionResult::Ok
            }
            Action::SetElementVisible { element, visible } => {
                self.element_visibility.set_visible(element.clone(), visible);
                self.status = format!(
                    "{} {}",
                    element_label(element),
                    if visible { "shown" } else { "hidden" }
                );
                ActionResult::Ok
            }
            Action::ToggleElementVisibility(element) => {
                let visible = self.element_visibility.toggle(element.clone());
                self.status = format!(
                    "{} {}",
                    element_label(element),
                    if visible { "shown" } else { "hidden" }
                );
                ActionResult::Ok
            }
            Action::CommitElementName { element, name } => {
                if let Err(e) = require_element_editable(&self.document_health, element.clone()) {
                    self.status = e.clone();
                    return ActionResult::Err(e);
                }
                match set_element_name(&mut self.doc, element.clone(), name) {
                    Ok(()) => {
                        let label = element_name(&self.doc, element.clone())
                            .map(str::to_string)
                            .unwrap_or_else(|| element_label(element));
                        self.status = format!("Renamed to {label}");
                        ActionResult::Ok
                    }
                    Err(e) => {
                        self.status = e.clone();
                        ActionResult::Err(e)
                    }
                }
            }
            Action::AddGeometricConstraint(kind) => {
                let Some(session) = self.sketch_session else {
                    return ActionResult::Err("Open a sketch to add constraints".to_string());
                };
                for element in self.scene_selection.iter() {
                    if let Err(e) = require_element_editable(&self.document_health, element) {
                        return ActionResult::Err(e);
                    }
                }
                match crate::geometric_constraints::add_geometric_constraint_from_selection(
                    &mut self.doc,
                    session.sketch,
                    kind,
                    &self.scene_selection,
                ) {
                    Ok(index) => {
                        self.refresh_document_health();
                        self.status =
                            format!("Added {} constraint {index}", kind.label());
                        ActionResult::Ok
                    }
                    Err(e) => ActionResult::Err(e),
                }
            }
            Action::ApplyConstraintShortcut(key) => {
                let rows = crate::geometric_constraints::constraint_pane_rows(&self.scene_selection);
                let Some(kind) =
                    crate::geometric_constraints::enabled_constraint_for_key(&rows, key)
                else {
                    return ActionResult::Err(format!(
                        "Constraint shortcut '{}' is not active",
                        key.to_ascii_uppercase()
                    ));
                };
                self.apply(Action::AddGeometricConstraint(kind))
            }
            Action::FocusElementName => {
                let Some(element) = single_nameable_from_selection(&self.scene_selection) else {
                    return ActionResult::Err("Select a single element to rename".to_string());
                };
                self.panes.set(Pane::Context, true);
                self.context_pane.focus_name_field = true;
                self.context_pane.synced_element = Some(element.clone());
                self.context_pane.name_draft =
                    element_name(&self.doc, element).unwrap_or_default().to_string();
                self.status = "Rename element".to_string();
                ActionResult::Ok
            }
            Action::SetDocumentUnits { length, angle } => {
                self.doc.default_length_unit = length;
                self.doc.default_angle_unit = angle;
                self.status = format!(
                    "Default units set to {} / {}",
                    length.label(),
                    angle.label()
                );
                ActionResult::Ok
            }
            Action::SetSketchUnits { sketch, length, angle } => {
                let Some(s) = self.doc.sketches.get_mut(sketch) else {
                    return ActionResult::Err(format!("Sketch {sketch} not found"));
                };
                s.length_unit = length;
                s.angle_unit = angle;
                self.status = "Sketch units updated".to_string();
                ActionResult::Ok
            }
        };
        if matches!(result, ActionResult::Ok) {
            if let (Some(log), Some(action)) = (&self.command_log, logged_action) {
                log.borrow_mut().after_apply(action, &self.doc);
            }
        }
        // #103 part 2: the document just mutated (refresh_document_health ran) and some
        // cut-bearing body is rendering the additive-only kernel fallback — override the
        // arm's success status with the warning so the silent-wrong-geometry state is
        // visible the moment it's entered, and stays visible across further edits.
        if std::mem::take(&mut self.kernel_fallback_warning_pending)
            && matches!(result, ActionResult::Ok)
        {
            if let Some(warning) = &self.kernel_fallback_warning {
                self.status = warning.clone();
            }
        }
        result
    }

    /// Add the constraint implied by leaving a snapped point on its target (deduped).
    /// Drops the in-progress line, reverting any live curve-mode preview it applied to the
    /// previous chained segment's `bezier` field back to that segment's pre-preview baseline
    /// (#73). Use this instead of a bare `self.creating_line = None` whenever a line draw is
    /// abandoned without going through [`Action::CommitLine`] (which finalizes the mutation
    /// with real values instead). Returns whether a line was in progress.
    fn discard_creating_line(&mut self) -> bool {
        let Some(cl) = self.creating_line.take() else {
            return false;
        };
        if let Some(prev_idx) = cl.chained_from {
            if let Some(prev) = self.doc.lines.get_mut(prev_idx) {
                prev.bezier = cl.chained_from_bezier;
            }
        }
        true
    }

    /// Distinct selected sketch vertices (deduped by coincident group) that are
    /// `LineEndpoint`s — the set the retroactive `B`/`T` shortcuts operate on (#73).
    fn selected_vertex_points(&self) -> Vec<ConstraintPoint> {
        let mut seen: Vec<ConstraintPoint> = Vec::new();
        let mut reps: Vec<ConstraintPoint> = Vec::new();
        for element in self.scene_selection.iter() {
            let SceneElement::Point(point) = element else {
                continue;
            };
            if !matches!(point, ConstraintPoint::LineEndpoint { .. }) || seen.contains(&point) {
                continue;
            }
            if let Some(sketch) = crate::construction::point_sketch(&self.doc, point.clone()) {
                seen.extend(vertex_drag::coincident_group(&self.doc, sketch, point.clone()));
            } else {
                seen.push(point.clone());
            }
            reps.push(point);
        }
        reps
    }

    /// `B` shortcut on a Select-tool vertex selection (#73): straightens both incident lines
    /// if either is already curved, else curves them smoothly (matching
    /// [`Action::ConvertVertexToBezier`]). Vertices that don't join exactly two plain lines
    /// are silently skipped.
    fn toggle_curve_at_selected_vertices(&mut self) -> Result<String, String> {
        let vertices = self.selected_vertex_points();
        if vertices.is_empty() {
            return Err("Select a sketch vertex to toggle its curve".to_string());
        }
        let mut toggled = 0usize;
        for point in vertices {
            let Some(sketch) = crate::construction::point_sketch(&self.doc, point.clone()) else {
                continue;
            };
            let Some([(line1, _), (line2, _)]) =
                vertex_drag::incident_two_lines(&self.doc, sketch, point.clone())
            else {
                continue;
            };
            let curved = self.doc.lines.get(line1).is_some_and(Line::is_curved)
                || self.doc.lines.get(line2).is_some_and(Line::is_curved);
            let ok = if curved {
                let r1 = self.apply(Action::StraightenLine { line: line1 });
                let r2 = self.apply(Action::StraightenLine { line: line2 });
                matches!(r1, ActionResult::Ok) || matches!(r2, ActionResult::Ok)
            } else {
                matches!(
                    self.apply(Action::ConvertVertexToBezier { point }),
                    ActionResult::Ok
                )
            };
            if ok {
                toggled += 1;
            }
        }
        if toggled == 0 {
            Err("Selected vertex doesn't join exactly two lines".to_string())
        } else {
            Ok(format!("Toggled curve at {toggled} vertex(es)"))
        }
    }

    /// `T` shortcut on a Select-tool vertex selection (#73): re-smooths (mirrors) each
    /// selected vertex's handles if it isn't already tangent-continuous, else breaks the
    /// mirroring into independent "corner" handles (see [`Action::SetVertexTangent`]).
    /// Vertices that don't join exactly two plain lines are silently skipped.
    fn toggle_tangent_at_selected_vertices(&mut self) -> Result<String, String> {
        let vertices = self.selected_vertex_points();
        if vertices.is_empty() {
            return Err("Select a sketch vertex to toggle its tangent constraint".to_string());
        }
        let mut toggled = 0usize;
        for point in vertices {
            let Some(sketch) = crate::construction::point_sketch(&self.doc, point.clone()) else {
                continue;
            };
            if vertex_drag::incident_two_lines(&self.doc, sketch, point.clone()).is_none() {
                continue;
            }
            let continuous = !vertex_is_tangent_continuous(&self.doc, sketch, point.clone());
            if matches!(
                self.apply(Action::SetVertexTangent { point, continuous }),
                ActionResult::Ok
            ) {
                toggled += 1;
            }
        }
        if toggled == 0 {
            Err("Selected vertex doesn't join exactly two lines".to_string())
        } else {
            Ok(format!("Toggled tangent constraint at {toggled} vertex(es)"))
        }
    }

    fn add_snap_constraint(
        &mut self,
        sketch: SketchId,
        point: ConstraintPoint,
        target: crate::snapping::SnapTarget,
    ) -> Result<(), String> {
        if let crate::snapping::SnapTarget::NormalAtMidpoint(anchor_line) = target.clone() {
            return self.add_normal_at_midpoint_constraint(sketch, point, anchor_line);
        }
        if crate::snapping::snap_constraint_already_present(&self.doc, point.clone(), target.clone()) {
            return Ok(());
        }
        let kind = crate::snapping::snap_constraint_kind(point, target);
        self.doc.constraints.push(crate::model::Constraint {
            sketch,
            kind,
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        self.doc
            .shape_order
            .push(crate::model::ShapeKind::Constraint);
        let new_index = self.doc.constraints.len() - 1;
        crate::constraints::remove_subsumed_point_on_line(&mut self.doc, sketch, new_index);
        crate::constraints::solve_document_constraints(&mut self.doc)?;
        self.refresh_document_health();
        Ok(())
    }

    /// Commit a [`crate::snapping::SnapTarget::NormalAtMidpoint`] snap (#41). There is no single
    /// existing constraint that pins a point to "the infinite line normal to `anchor_line`
    /// through its midpoint", so this invents a construction line to carry it: a fresh
    /// (dashed, non-solid) [`Line`] running from `anchor_line`'s midpoint out toward `point`'s
    /// placed location, pinned there with a `Midpoint` constraint, held perpendicular to
    /// `anchor_line` with a `Perpendicular` constraint, and finally `point` is pinned onto that
    /// new line's infinite carrier with a `Coincident` point-on-line constraint (mirroring how
    /// `OnLine`/`OnLineExtension` pin a point to an existing line's carrier).
    fn add_normal_at_midpoint_constraint(
        &mut self,
        sketch: SketchId,
        point: ConstraintPoint,
        anchor_line: ConstraintLine,
    ) -> Result<(), String> {
        let ((x0, y0), (x1, y1)) =
            crate::geometric_constraints::line_uv_endpoints(&self.doc, sketch, anchor_line.clone())?;
        let (mx, my) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let anchor_len = dx.hypot(dy);
        let (ex, ey) = crate::geometric_constraints::point_uv(&self.doc, sketch, point.clone())
            .ok()
            .filter(|&(ex, ey)| (ex - mx).hypot(ey - my) > 1e-6)
            .unwrap_or_else(|| {
                // Degenerate: the placed point resolved exactly onto the midpoint. Fall back to
                // a small nonzero length along the perpendicular so the line isn't zero-length.
                let fallback_len = if anchor_len > 1e-6 { anchor_len } else { 1.0 };
                let perp_len = anchor_len.max(1e-6);
                (mx - dy / perp_len * fallback_len, my + dx / perp_len * fallback_len)
            });
        let mut line = Line::from_local_endpoints(sketch, mx, my, ex, ey);
        line.construction = true;
        self.doc.lines.push(line);
        self.doc.shape_order.push(ShapeKind::Line);
        let new_line_index = self.doc.lines.len() - 1;
        let new_line = ConstraintLine::Line(new_line_index);

        let push_constraint = |doc: &mut Document, kind: ConstraintKind| {
            doc.constraints.push(crate::model::Constraint {
                sketch,
                kind,
                expression: String::new(),
                dim_offset: None,
                name: None,
                deleted: false,
            });
            doc.shape_order.push(ShapeKind::Constraint);
        };
        push_constraint(
            &mut self.doc,
            ConstraintKind::Midpoint {
                point: ConstraintPoint::LineEndpoint {
                    line: new_line_index,
                    end: LineEnd::Start,
                },
                line: anchor_line.clone(),
            },
        );
        push_constraint(
            &mut self.doc,
            ConstraintKind::Perpendicular {
                line_a: new_line.clone(),
                line_b: anchor_line,
            },
        );
        push_constraint(
            &mut self.doc,
            ConstraintKind::Coincident {
                a: ConstraintEntity::Point(point),
                b: ConstraintEntity::Line(new_line),
            },
        );

        crate::constraints::solve_document_constraints(&mut self.doc)?;
        self.refresh_document_health();
        Ok(())
    }

    fn exit_sketch_session(&mut self) {
        self.active_snap = None;
        self.extension_anchors.clear();
        self.normal_inference_anchor = None;
        self.sketch_session = None;
        self.sketch_reframe_pending = false;
        self.creating_rect = None;
        self.discard_creating_line();
        self.editing_committed_dim = None;
        self.placing_angle_dimension = None;
        // Return to the pre-sketch camera pose; the transition restores world-orbit mode on
        // completion (its `view_up` is `None`). Fall back to a plain mode-leave if unknown.
        if let Some(pose) = self.pre_sketch_pose.take() {
            self.cam.start_transition_to_view(pose, VIEW_TRANSITION_DURATION);
        } else {
            self.cam.leave_sketch_mode();
        }
        self.tool = Tool::Select;
    }

    fn sketch_zoom_distance(
        &self,
        sketch: SketchId,
        viewport: egui::Rect,
        frame_padding_px: f32,
    ) -> Option<f32> {
        let frame_target = sketch_camera_target(&self.doc, sketch)?;
        let bounds = frame_target.zoom?;
        let face = self.doc.sketch_face(sketch)?;
        let frame = sketch_frame(&self.doc, face)?;
        let view_direction = self.cam.visible_face_view_direction(
            frame_target.target,
            frame_target.face_normal,
        );
        let current_look = (frame_target.target - self.cam.eye()).normalize_or_zero();
        let sketch_up = sketch_view_up(
            view_direction,
            &frame,
            current_look,
            self.cam.view_up_hint(),
        );
        let corners = bounds.world_corners(&frame);
        Some(self.cam.distance_to_fit_corners_with_up(
            frame_target.target,
            view_direction,
            sketch_up,
            &corners,
            frame_padding_px,
            viewport,
        ))
    }

    /// Apply deferred sketch framing once the viewport rect is available.
    pub fn apply_pending_sketch_reframe(&mut self, viewport: egui::Rect) {
        if !self.sketch_reframe_pending {
            return;
        }
        let Some(sketch) = self.sketch_session.map(|session| session.sketch) else {
            self.sketch_reframe_pending = false;
            return;
        };
        if let Some(zoom_distance) =
            self.sketch_zoom_distance(sketch, viewport, SKETCH_EDIT_FRAME_PADDING_PX)
        {
            self.cam.set_transition_zoom(zoom_distance);
        }
        self.sketch_reframe_pending = false;
    }

    /// Push a brand-new construction plane (never edits an existing one) and select it.
    /// Shared by the interactive commit flow (`Action::CommitConstructionPlane` with no
    /// `edit_index`) and the declarative `Action::AddConstructionPlane` (#116).
    fn add_construction_plane(
        &mut self,
        definition: crate::model::PlaneDefinition,
        parent: ConstructionPlaneParent,
    ) -> ActionResult {
        let live_offset = definition.offset_mm;
        let label = reference_from_definition(&definition).label().to_string();
        let plane = plane_from_definition(&definition, parent);
        self.doc.construction_planes.push(plane);
        self.doc.shape_order.push(ShapeKind::ConstructionPlane);
        let index = self.doc.construction_planes.len() - 1;
        self.scene_selection.clear();
        click_scene_selection(
            &mut self.scene_selection,
            SceneElement::ConstructionPlane(index),
            false,
        );
        self.status = format!(
            "Added construction plane ({} from {})",
            crate::value::format_length_display_in(live_offset, self.doc.default_length_unit),
            label
        );
        ActionResult::Ok
    }

    fn enter_sketch(
        &mut self,
        sketch: SketchId,
        viewport: Option<egui::Rect>,
        frame_padding_px: Option<f32>,
    ) -> ActionResult {
        self.sketch_reframe_pending = false;
        // Remember where the camera was so exiting can return to it. Only capture on the
        // first entry, so switching between sketches still returns to the pre-sketch pose.
        if self.sketch_session.is_none() {
            self.pre_sketch_pose = Some(self.cam.capture_view());
        }
        if let Some(frame_target) = sketch_camera_target(&self.doc, sketch) {
            let face = self.doc.sketch_face(sketch).unwrap();
            let frame = sketch_frame(&self.doc, face).unwrap();
            let view_direction = self.cam.visible_face_view_direction(
                frame_target.target,
                frame_target.face_normal,
            );
            let current_look = (frame_target.target - self.cam.eye()).normalize_or_zero();
            let sketch_up = sketch_view_up(
                view_direction,
                &frame,
                current_look,
                self.cam.view_up_hint(),
            );
            let zoom_distance = frame_target.zoom.and_then(|_| {
                let vp = viewport?;
                let padding = frame_padding_px?;
                self.sketch_zoom_distance(sketch, vp, padding)
            });
            if frame_target.zoom.is_some() && viewport.is_none() {
                self.sketch_reframe_pending = true;
            }
            self.cam.start_sketch_view_transition(
                frame_target.target,
                frame_target.face_normal,
                zoom_distance,
                VIEW_TRANSITION_DURATION,
                sketch_up,
            );
        }
        self.sketch_session = Some(SketchSession { sketch });
        self.creating_rect = None;
        self.discard_creating_line();
        if !self.tool.is_sketch_edit_tool() {
            self.tool = Tool::Select;
        }
        let name = sketch_label(&self.doc, sketch);
        self.status = match self.tool {
            Tool::Rectangle => format!("{name} — click to set corner"),
            Tool::Line => format!("{name} — click to set start"),
            _ => format!("{name} — pick line or rectangle"),
        };
        ActionResult::Ok
    }

    fn write_to(&mut self, path: &str) -> ActionResult {
        match crate::storage::save(path, &self.doc) {
            Ok(()) => {
                self.path = Some(path.to_string());
                self.status = format!(
                    "Saved {} line(s) to {}",
                    self.doc.lines.len(),
                    path
                );
                ActionResult::Ok
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                ActionResult::Err(e)
            }
        }
    }
}

/// Two index lists as equal sets (order-independent), for coalescing detection.
fn same_move_set(a: &[usize], b: &[usize]) -> bool {
    a.len() == b.len() && {
        let mut a: Vec<usize> = a.to_vec();
        let mut b: Vec<usize> = b.to_vec();
        a.sort_unstable();
        b.sort_unstable();
        a == b
    }
}

/// The existing Move op the in-progress move is re-moving, if any (#217): the same construction
/// planes moved again, or bodies whose targets are exactly an op's moved outputs. Consecutive
/// moves on the same element fold into one op instead of stacking tiny ops.
fn coalescible_move_op(doc: &Document, cm: &CreatingMove) -> Option<usize> {
    if cm.editing.is_some() {
        return None;
    }
    doc.move_ops.iter().position(|op| {
        if op.deleted {
            return false;
        }
        // Coalesce only a pure re-move of the exact same single kind of element set, so the
        // folded op keeps one clean target list (mixed-kind moves stack a fresh op).
        let planes_again = !cm.plane_targets.is_empty()
            && cm.targets.is_empty()
            && cm.image_targets.is_empty()
            && op.targets.is_empty()
            && op.image_targets.is_empty()
            && same_move_set(&op.plane_targets, &cm.plane_targets);
        let bodies_again = !cm.targets.is_empty()
            && cm.plane_targets.is_empty()
            && cm.image_targets.is_empty()
            && op.plane_targets.is_empty()
            && op.image_targets.is_empty()
            && same_move_set(&op.outputs, &cm.targets);
        let images_again = !cm.image_targets.is_empty()
            && cm.targets.is_empty()
            && cm.plane_targets.is_empty()
            && op.targets.is_empty()
            && op.plane_targets.is_empty()
            && same_move_set(&op.image_targets, &cm.image_targets);
        planes_again || bodies_again || images_again
    })
}

/// Whether a move's translation is non-zero.
fn move_translates(doc: &Document, tx: &str, ty: &str, tz: &str) -> bool {
    let v = |e: &str| crate::value::eval_length_mm_in_doc(e, doc).unwrap_or(0.0);
    v(tx).abs() > 1e-9 || v(ty).abs() > 1e-9 || v(tz).abs() > 1e-9
}

/// Whether a move's rotation is non-zero.
fn move_rotates(doc: &Document, axis: Option<crate::model::RevolveAxis>, angle: &str) -> bool {
    axis.is_some()
        && !angle.trim().is_empty()
        && crate::value::eval_angle_rad_in_doc(angle, doc).unwrap_or(0.0).abs() > 1e-9
}

/// Compose an existing move op with a new one into a single set of move values (#217), or `None`
/// if the composition isn't representable as translation + single-axis rotation (differing axes,
/// or a translation-plus-rotation mix). Values are combined numerically (coalescing bakes them).
fn compose_move_values(
    doc: &Document,
    op: &crate::model::MoveOperation,
    cm: &CreatingMove,
) -> Option<(String, String, String, Option<crate::model::RevolveAxis>, String)> {
    let len = |e: &str| crate::value::eval_length_mm_in_doc(e, doc).unwrap_or(0.0);
    let ang = |e: &str| crate::value::eval_angle_rad_in_doc(e, doc).unwrap_or(0.0);
    let op_rot = move_rotates(doc, op.axis, &op.angle);
    let cm_rot = move_rotates(doc, cm.axis, &cm.angle);
    let op_tr = move_translates(doc, &op.tx, &op.ty, &op.tz);
    let cm_tr = move_translates(doc, &cm.tx, &cm.ty, &cm.tz);
    // Pure translation on both → add the translations.
    if !op_rot && !cm_rot {
        return Some((
            format!("{}mm", len(&op.tx) + len(&cm.tx)),
            format!("{}mm", len(&op.ty) + len(&cm.ty)),
            format!("{}mm", len(&op.tz) + len(&cm.tz)),
            None,
            String::new(),
        ));
    }
    // Pure rotation on both about the same axis → add the angles.
    if !op_tr && !cm_tr && op.axis == cm.axis {
        let combined = (ang(&op.angle) + ang(&cm.angle)).to_degrees();
        return Some((
            String::new(),
            String::new(),
            String::new(),
            op.axis,
            format!("{combined}"),
        ));
    }
    None
}

/// Recompute the frames of construction planes moved by a Move op (#217): each targeted plane's
/// frame is its base definition composed with every (non-deleted) Move op targeting it, in op
/// order. Everything anchored to the plane (sketches, tracing images) then follows, since that
/// geometry is stored plane-local and projected through the plane frame on the fly. Only planes
/// that are actually a move target are touched, so untargeted planes are never disturbed.
pub fn recompute_moved_planes(doc: &mut crate::model::Document) {
    use std::collections::BTreeSet;
    let targeted: BTreeSet<usize> = doc
        .move_ops
        .iter()
        .filter(|o| !o.deleted)
        .flat_map(|o| o.plane_targets.iter().copied())
        .collect();
    if targeted.is_empty() {
        return;
    }
    // Precompute each op's world transform (immutable borrow, before mutating planes).
    let transforms: Vec<Option<glam::Mat4>> = doc
        .move_ops
        .iter()
        .map(|op| {
            (!op.deleted && !op.plane_targets.is_empty())
                .then(|| crate::extrude::move_op_transform(doc, op))
                .flatten()
        })
        .collect();
    let mut updates: Vec<(usize, glam::Vec3, glam::Vec3, glam::Vec3, glam::Vec3)> = Vec::new();
    for &i in &targeted {
        let Some(plane) = doc.construction_planes.get(i).filter(|p| !p.deleted) else {
            continue;
        };
        let base = plane_from_definition(&plane.definition, plane.parent);
        let (mut o, mut n, mut u, mut v) = (base.origin, base.normal, base.u_axis, base.v_axis);
        for (op_idx, op) in doc.move_ops.iter().enumerate() {
            if op.deleted || !op.plane_targets.contains(&i) {
                continue;
            }
            if let Some(m) = transforms[op_idx] {
                o = m.transform_point3(o);
                n = m.transform_vector3(n).normalize_or_zero();
                u = m.transform_vector3(u).normalize_or_zero();
                v = m.transform_vector3(v).normalize_or_zero();
            }
        }
        updates.push((i, o, n, u, v));
    }
    for (i, o, n, u, v) in updates {
        let p = &mut doc.construction_planes[i];
        p.origin = o;
        p.normal = n;
        p.u_axis = u;
        p.v_axis = v;
    }
}

/// Recompute the plane-local origins of tracing images moved by a Move op (#217). Each targeted
/// image's `origin` is its pristine `base_origin` projected onto the (already-recomputed) host
/// plane frame and pushed through every non-deleted Move op targeting the image, in op order.
/// Because the base corner is projected through the *current* plane frame, an image on a plane
/// that also moved follows the plane and then takes its own move on top. Untargeted images that
/// were previously moved are restored to their authored base and cleared; others are untouched.
///
/// Must run *after* [`recompute_moved_planes`], so the host plane frame is up to date.
pub fn recompute_moved_images(doc: &mut crate::model::Document) {
    use std::collections::BTreeSet;
    let targeted: BTreeSet<usize> = doc
        .move_ops
        .iter()
        .filter(|o| !o.deleted)
        .flat_map(|o| o.image_targets.iter().copied())
        .collect();
    // Precompute each op's world transform (immutable borrow, before mutating images).
    let transforms: Vec<Option<glam::Mat4>> = doc
        .move_ops
        .iter()
        .map(|op| {
            (!op.deleted && !op.image_targets.is_empty())
                .then(|| crate::extrude::move_op_transform(doc, op))
                .flatten()
        })
        .collect();
    // (image index, new origin, new base_origin).
    let mut updates: Vec<(usize, (f32, f32), Option<(f32, f32)>)> = Vec::new();
    for (i, img) in doc.tracing_images.iter().enumerate() {
        if img.deleted {
            continue;
        }
        if targeted.contains(&i) {
            // Lock in the pristine base the first time the image is targeted (its `origin`
            // has no move applied yet at that moment), then always recompute from it.
            let base = img.base_origin.unwrap_or(img.origin);
            let Some(frame) =
                crate::face::sketch_frame(doc, crate::model::FaceId::ConstructionPlane(img.plane))
            else {
                continue;
            };
            let mut world = frame.origin + frame.u_axis * base.0 + frame.v_axis * base.1;
            for (op_idx, op) in doc.move_ops.iter().enumerate() {
                if op.deleted || !op.image_targets.contains(&i) {
                    continue;
                }
                if let Some(m) = transforms[op_idx] {
                    world = m.transform_point3(world);
                }
            }
            let d = world - frame.origin;
            updates.push((i, (d.dot(frame.u_axis), d.dot(frame.v_axis)), Some(base)));
        } else if let Some(base) = img.base_origin {
            // No longer moved by any op → snap back to the authored position and forget the base.
            updates.push((i, base, None));
        }
    }
    for (i, origin, base) in updates {
        let img = &mut doc.tracing_images[i];
        img.origin = origin;
        img.base_origin = base;
    }
}

/// Recompute the cached frames of construction planes generated as Repeat-op instances (#221).
/// Each such plane copies its source plane's *current* frame (origin/normal/u/v) and translates
/// the origin by the instance's along-axis offset, so the copies step along the axis and follow
/// the source plane if it moves.
///
/// Must run *after* [`recompute_moved_planes`], so a moved source plane's frame is up to date
/// before its instances copy it.
pub fn recompute_repeated_planes(doc: &mut crate::model::Document) {
    // Precompute each op's axis direction and instance offsets (immutable borrow before mutating).
    let op_data: Vec<Option<(glam::Vec3, Vec<f32>)>> = doc
        .repeat_ops
        .iter()
        .map(|op| {
            if op.deleted || op.plane_targets.is_empty() {
                return None;
            }
            let (_, dir) = crate::extrude::axis_world(doc, op.axis)?;
            let offsets = crate::extrude::repeat_offsets(doc, op)?;
            Some((dir, offsets))
        })
        .collect();
    let mut updates: Vec<(usize, glam::Vec3, glam::Vec3, glam::Vec3, glam::Vec3)> = Vec::new();
    for (pi, plane) in doc.construction_planes.iter().enumerate() {
        if plane.deleted {
            continue;
        }
        let Some(inst) = plane.repeat_instance else {
            continue;
        };
        let Some((dir, offsets)) = op_data.get(inst.op).and_then(|d| d.as_ref()) else {
            continue;
        };
        let op = &doc.repeat_ops[inst.op];
        let Some(&src) = op.plane_targets.get(inst.target) else {
            continue;
        };
        let Some(offset) = inst
            .instance
            .checked_sub(1)
            .and_then(|i| offsets.get(i))
            .copied()
        else {
            continue;
        };
        let Some(source) = doc.construction_planes.get(src).filter(|p| !p.deleted) else {
            continue;
        };
        let o = source.origin + *dir * offset;
        updates.push((pi, o, source.normal, source.u_axis, source.v_axis));
    }
    for (i, o, n, u, v) in updates {
        let p = &mut doc.construction_planes[i];
        p.origin = o;
        p.normal = n;
        p.u_axis = u;
        p.v_axis = v;
    }
}

/// The "N body(ies)[, M plane(s)]" fragment for a Repeat op's status line (#221).
fn repeat_input_status(bodies: usize, planes: usize) -> String {
    let mut parts = Vec::new();
    if bodies > 0 {
        parts.push(format!("{bodies} body(ies)"));
    }
    if planes > 0 {
        parts.push(format!("{planes} plane(s)"));
    }
    parts.join(", ")
}

/// Status line after a Move commits, summarizing however many bodies/planes/images it touched.
fn move_status(bodies: usize, planes: usize, images: usize) -> String {
    let mut parts = Vec::new();
    if bodies > 0 {
        parts.push(format!("{bodies} body(ies)"));
    }
    if planes > 0 {
        parts.push(format!("{planes} plane(s)"));
    }
    if images > 0 {
        parts.push(format!("{images} image(s)"));
    }
    format!("Moved {}", parts.join(", "))
}

/// Toggle a body into the active body-gathering tool's set (#218): the Elements pane (and any
/// body selection) feeds bodies into whatever tool is collecting them — Move/Repeat/Slice
/// targets, Combine's active side, or a Revolve Cut's bodies — regardless of the viewport's
/// sub-element picking. Returns whether a tool consumed the click (so it shouldn't also change
/// the persistent selection). Shadow/deleted bodies aren't usable targets.
pub fn toggle_body_in_active_tool(state: &mut AppState, bi: usize) -> bool {
    if state.doc.bodies.get(bi).is_none_or(|b| b.deleted || b.shadow) {
        return false;
    }
    fn toggle(set: &mut Vec<usize>, bi: usize) {
        if let Some(pos) = set.iter().position(|b| *b == bi) {
            set.remove(pos);
        } else {
            set.push(bi);
        }
    }
    match state.tool {
        Tool::Move => {
            toggle(&mut state.creating_move.get_or_insert_with(CreatingMove::default).targets, bi);
            true
        }
        Tool::Repeat => {
            toggle(&mut state.creating_repeat.get_or_insert_with(CreatingRepeat::default).targets, bi);
            true
        }
        Tool::Slice => {
            toggle(&mut state.creating_slice.get_or_insert_with(CreatingSlice::default).targets, bi);
            true
        }
        Tool::Combine => {
            let cb = state.creating_boolean.get_or_insert_with(CreatingBoolean::default);
            if cb.picking_b {
                toggle(&mut cb.b, bi);
            } else {
                toggle(&mut cb.a, bi);
            }
            true
        }
        Tool::Revolve => {
            let cr = state.creating_revolve.get_or_insert_with(CreatingRevolve::default);
            if cr.body_choice == RevolveBodyChoice::Cut {
                toggle(&mut cr.cut_bodies, bi);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// A viewport gizmo currently available given the tool/creation state (#214). Each gizmo is a
/// drag that writes one scalar; scripting can enumerate them (`bearcad.gizmos()`) and drive
/// their value (`set_gizmo`/`drag_gizmo`) so gizmo tools are automatable/testable headlessly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoInfo {
    /// `"push_pull"`, `"rotate"`, or `"offset"`.
    pub kind: &'static str,
    /// Stable script handle, e.g. `"extrude"`.
    pub name: &'static str,
    /// The gizmo's live scalar (mm for push/pull and offset, radians for rotate).
    pub value: f32,
}

/// The script handle for a chamfer/fillet amount gizmo, named for the actual treatment kind.
fn treatment_gizmo_name(kind: crate::model::VertexTreatmentKind) -> &'static str {
    match kind {
        crate::model::VertexTreatmentKind::Chamfer => "chamfer",
        crate::model::VertexTreatmentKind::Fillet => "fillet",
    }
}

/// The gizmos the current state exposes, with their live values (#214). Push/pull and offset
/// values are millimetres; rotate values are radians. At most one of each is active at a time.
pub fn available_gizmos(state: &AppState) -> Vec<GizmoInfo> {
    let mut gizmos = Vec::new();
    if let Some(ce) = &state.creating_extrusion {
        gizmos.push(GizmoInfo { kind: "push_pull", name: "extrude", value: ce.distance });
    }
    // 2D (sketch vertex) and 3D (body edge) chamfer/fillet share a name by kind; only one runs.
    if let Some(cvt) = &state.creating_vertex_treatment {
        gizmos.push(GizmoInfo {
            kind: "push_pull",
            name: treatment_gizmo_name(cvt.kind),
            value: cvt.amount_live,
        });
    }
    if let Some(cet) = &state.creating_edge_treatment {
        gizmos.push(GizmoInfo {
            kind: "push_pull",
            name: treatment_gizmo_name(cet.kind),
            value: cet.amount_live,
        });
    }
    if let Some(cr) = &state.creating_revolve {
        gizmos.push(GizmoInfo { kind: "rotate", name: "revolve", value: cr.angle_live.to_radians() });
    }
    if let Some(cp) = &state.creating_plane {
        gizmos.push(GizmoInfo { kind: "offset", name: "offset", value: cp.offset_live });
    }
    // Move tool (#185): the translation components (mm) and — when a rotation axis is set — the
    // rotation angle (radians). These are the values the Move drag gizmos control; exposing them
    // makes the Move tool scriptable/testable ahead of the viewport handles.
    if let Some(cm) = &state.creating_move {
        let mm = |s: &str| crate::value::eval_length_mm_in_doc(s, &state.doc).unwrap_or(0.0);
        gizmos.push(GizmoInfo { kind: "offset", name: "move_x", value: mm(&cm.tx) });
        gizmos.push(GizmoInfo { kind: "offset", name: "move_y", value: mm(&cm.ty) });
        gizmos.push(GizmoInfo { kind: "offset", name: "move_z", value: mm(&cm.tz) });
        if cm.axis.is_some() {
            let rad = crate::value::eval_angle_rad_in_doc(&cm.angle, &state.doc).unwrap_or(0.0);
            gizmos.push(GizmoInfo { kind: "rotate", name: "move_angle", value: rad });
        }
    }
    gizmos
}

/// The current scalar of a named gizmo, if that gizmo is available (#214).
pub fn gizmo_value(state: &AppState, name: &str) -> Option<f32> {
    available_gizmos(state)
        .into_iter()
        .find(|g| g.name == name)
        .map(|g| g.value)
}

/// Drive a named gizmo's scalar to `value` the way a drag would (#214), returning whether that
/// gizmo was available. Push/pull and offset take millimetres; rotate takes radians. The change
/// is authoritative — it clears any typed override so the value takes effect on commit.
pub fn set_gizmo(state: &mut AppState, name: &str, value: f32) -> bool {
    match name {
        "extrude" => {
            if state.creating_extrusion.is_some() {
                state.apply(Action::SetExtrudeDistance { distance: value });
                true
            } else {
                false
            }
        }
        "chamfer" | "fillet" => {
            let unit = state.doc.default_length_unit;
            let amount = value.max(0.0);
            let text = crate::value::format_length_display_in(amount, unit);
            if let Some(cvt) = state.creating_vertex_treatment.as_mut() {
                if treatment_gizmo_name(cvt.kind) == name {
                    cvt.amount_live = amount;
                    cvt.user_edited = false;
                    cvt.text = text.clone();
                    return true;
                }
            }
            if let Some(cet) = state.creating_edge_treatment.as_mut() {
                if treatment_gizmo_name(cet.kind) == name {
                    cet.amount_live = amount;
                    cet.user_edited = false;
                    cet.text = text;
                    return true;
                }
            }
            false
        }
        "revolve" => {
            if let Some(cr) = state.creating_revolve.as_mut() {
                cr.angle_live = value.to_degrees().clamp(1.0, 360.0);
                cr.user_edited = false;
                cr.text = format!("{:.0}", cr.angle_live);
                true
            } else {
                false
            }
        }
        "offset" => {
            if state.creating_plane.is_some() {
                // Force millimetres so the value matches the mm-valued `offset` gizmo reading.
                state.apply(Action::SetPlaneOffset { value: format!("{value}mm") });
                true
            } else {
                false
            }
        }
        "move_x" | "move_y" | "move_z" => {
            if let Some(cm) = state.creating_move.as_mut() {
                let text = format!("{value}mm");
                match name {
                    "move_x" => cm.tx = text,
                    "move_y" => cm.ty = text,
                    _ => cm.tz = text,
                }
                true
            } else {
                false
            }
        }
        "move_angle" => {
            if let Some(cm) = state.creating_move.as_mut() {
                cm.angle = format!("{}", value.to_degrees());
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #218: while a body-gathering tool is active, clicking a body (Elements pane / `select`)
    /// toggles it into the tool's set instead of the persistent selection — you can pick bodies
    /// from the pane regardless of the viewport's sub-element picking.
    #[test]
    fn body_click_feeds_the_active_tool_set() {
        use crate::model::{ExtrudeFace, FaceId};
        let mut state = AppState::default();
        state.apply(Action::BeginSketch { face: FaceId::ConstructionPlane(0), viewport: None });
        let sketch = state.sketch_session.unwrap().sketch;
        let lines =
            crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(lines.to_vec())],
            distance: 5.0,
            body: ExtrudeBodyChoice::New,
            target: None,
        });
        assert_eq!(state.doc.bodies.len(), 1);

        state.apply(Action::SetTool(Tool::Move));
        // A body click lands in the Move set, not the persistent selection.
        state.apply(Action::ClickSceneElement { element: SceneElement::Body(0), additive: false });
        assert_eq!(state.creating_move.as_ref().unwrap().targets, vec![0]);
        assert!(state.scene_selection.is_empty(), "tool click must not touch the selection");
        // Clicking again toggles it back out.
        state.apply(Action::ClickSceneElement { element: SceneElement::Body(0), additive: false });
        assert!(state.creating_move.as_ref().unwrap().targets.is_empty());

        // A line clicked with the Move tool sets the rotation axis (#216).
        state.apply(Action::ClickSceneElement { element: SceneElement::Line(0), additive: false });
        assert_eq!(
            state.creating_move.as_ref().unwrap().axis,
            Some(crate::model::RevolveAxis::Line(0))
        );

        // With the Select tool, a body click is an ordinary selection.
        state.apply(Action::SetTool(Tool::Select));
        state.apply(Action::ClickSceneElement { element: SceneElement::Body(0), additive: false });
        assert!(state.scene_selection.is_selected(SceneElement::Body(0)));
    }

    /// #217: a Move op can target a construction plane, transforming its frame in place; the
    /// plane's origin shifts, and anything anchored to it follows (it's plane-relative).
    #[test]
    fn moving_a_construction_plane_shifts_its_frame() {
        let mut state = AppState::default();
        let base = state.doc.construction_planes[0].origin;
        let result = state.apply(Action::CreateMoveOperation {
            targets: vec![],
            plane_targets: vec![0],
            image_targets: vec![],
            tx: String::new(),
            ty: String::new(),
            tz: "40mm".to_string(),
            axis: None,
            angle: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok), "{}", state.status);
        let moved = state.doc.construction_planes[0].origin;
        assert!(
            (moved.z - base.z - 40.0).abs() < 1e-3,
            "plane should move +40 in z (base {} -> {})",
            base.z,
            moved.z
        );
        // Editing the op back to zero returns the plane home.
        let op = state.doc.move_ops.len() - 1;
        state.apply(Action::EditMoveOperation {
            op,
            targets: vec![],
            plane_targets: vec![0],
            image_targets: vec![],
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
        });
        assert!((state.doc.construction_planes[0].origin.z - base.z).abs() < 1e-3);
    }

    /// #217: consecutive moves on the same plane coalesce into one op (translations add) instead
    /// of stacking tiny ops.
    #[test]
    fn consecutive_plane_moves_coalesce() {
        let mut state = AppState::default();
        let base = state.doc.construction_planes[0].origin.z;
        let move_plane = |state: &mut AppState, tz: &str| {
            state.creating_move = Some(CreatingMove {
                plane_targets: vec![0],
                tz: tz.to_string(),
                ..Default::default()
            });
            assert!(matches!(state.apply(Action::CommitMove), ActionResult::Ok), "{}", state.status);
        };
        move_plane(&mut state, "40mm");
        assert_eq!(
            state.doc.move_ops.iter().filter(|o| !o.deleted).count(),
            1,
            "first move creates one op"
        );
        move_plane(&mut state, "10mm");
        assert_eq!(
            state.doc.move_ops.iter().filter(|o| !o.deleted).count(),
            1,
            "second move on the same plane coalesces into the first op"
        );
        assert!(
            (state.doc.construction_planes[0].origin.z - base - 50.0).abs() < 1e-3,
            "translations add up to +50"
        );
    }

    /// #217: a Move op can target a tracing image, translating its plane-local origin in place
    /// on its host plane. In-plane translation shifts the origin; editing the op back to zero and
    /// then dropping the image from the op returns it to its authored base.
    #[test]
    fn moving_a_tracing_image_shifts_its_plane_local_origin() {
        let mut state = AppState::default();
        state.doc.tracing_images.push(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: 0,
            origin: (0.0, 0.0),
            width_mm: 100.0,
            height_mm: 60.0,
            name: None,
            deleted: false,
            calibration: None,
            base_origin: None,
        });
        // Plane 0 is the XY ground (u = X, v = Y), so a +25 world-X move lands +25 in the
        // image's plane-local x, and a world-Z move (out of plane) doesn't touch the origin.
        let result = state.apply(Action::CreateMoveOperation {
            targets: vec![],
            plane_targets: vec![],
            image_targets: vec![0],
            tx: "25mm".to_string(),
            ty: String::new(),
            tz: "7mm".to_string(),
            axis: None,
            angle: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok), "{}", state.status);
        let img = &state.doc.tracing_images[0];
        assert!(
            (img.origin.0 - 25.0).abs() < 1e-3 && img.origin.1.abs() < 1e-3,
            "origin should shift +25 in plane x only, got {:?}",
            img.origin
        );
        assert_eq!(img.base_origin, Some((0.0, 0.0)), "authored base is locked in");

        let op = state.doc.move_ops.len() - 1;
        // Editing the op back to zero returns the image home (still targeted, base kept).
        state.apply(Action::EditMoveOperation {
            op,
            targets: vec![],
            plane_targets: vec![],
            image_targets: vec![0],
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
        });
        assert!(state.doc.tracing_images[0].origin.0.abs() < 1e-3);

        // Dropping the image from the op restores its authored base and forgets it.
        state.apply(Action::EditMoveOperation {
            op,
            targets: vec![],
            plane_targets: vec![0],
            image_targets: vec![],
            tx: "25mm".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
        });
        let img = &state.doc.tracing_images[0];
        assert_eq!(img.origin, (0.0, 0.0), "image snaps back to authored base");
        assert_eq!(img.base_origin, None, "base is forgotten once untargeted");
    }

    /// #217: consecutive moves on the same tracing image coalesce into one op (translations add).
    #[test]
    fn consecutive_image_moves_coalesce() {
        let mut state = AppState::default();
        state.doc.tracing_images.push(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: 0,
            origin: (0.0, 0.0),
            width_mm: 100.0,
            height_mm: 60.0,
            name: None,
            deleted: false,
            calibration: None,
            base_origin: None,
        });
        let move_image = |state: &mut AppState, tx: &str| {
            state.creating_move = Some(CreatingMove {
                image_targets: vec![0],
                tx: tx.to_string(),
                ..Default::default()
            });
            assert!(matches!(state.apply(Action::CommitMove), ActionResult::Ok), "{}", state.status);
        };
        move_image(&mut state, "25mm");
        move_image(&mut state, "5mm");
        assert_eq!(
            state.doc.move_ops.iter().filter(|o| !o.deleted).count(),
            1,
            "second move on the same image coalesces into the first op"
        );
        assert!(
            (state.doc.tracing_images[0].origin.0 - 30.0).abs() < 1e-3,
            "translations add up to +30, got {:?}",
            state.doc.tracing_images[0].origin
        );
    }

    /// #214: the extrude tool's in-progress push/pull depth is exposed as a gizmo and driven by
    /// `set_gizmo_action`, so scripting can enumerate and move it.
    #[test]
    fn gizmos_expose_and_drive_the_extrude_push_pull_depth() {
        use crate::model::{ExtrudeFace, FaceId};
        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let sketch = state.sketch_session.unwrap().sketch;
        let lines =
            crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        state.apply(Action::ExitSketch);
        // No gizmo before an extrusion is in progress.
        assert!(available_gizmos(&state).is_empty());

        state.apply(Action::ToggleExtrudeFace {
            face: ExtrudeFace::Polygon(lines.to_vec()),
        });
        let gizmos = available_gizmos(&state);
        assert_eq!(gizmos.len(), 1);
        assert_eq!(gizmos[0].name, "extrude");
        assert_eq!(gizmos[0].kind, "push_pull");
        assert_eq!(
            gizmo_value(&state, "extrude"),
            Some(state.creating_extrusion.as_ref().unwrap().distance)
        );

        // Absolute set and relative nudge drive the live depth.
        assert!(set_gizmo(&mut state, "extrude", 15.0));
        assert_eq!(state.creating_extrusion.as_ref().unwrap().distance, 15.0);
        let cur = gizmo_value(&state, "extrude").unwrap();
        assert!(set_gizmo(&mut state, "extrude", cur + 5.0));
        assert_eq!(state.creating_extrusion.as_ref().unwrap().distance, 20.0);

        assert!(!set_gizmo(&mut state, "nonesuch", 1.0));
    }

    /// #214: the revolve angle (rotate, radians) and construction-plane offset (mm) gizmos are
    /// enumerated and driven the same way.
    #[test]
    fn gizmos_cover_revolve_angle_and_plane_offset() {
        use std::f32::consts::PI;

        let mut state = AppState::default();
        state.creating_revolve = Some(CreatingRevolve {
            angle_live: 90.0,
            ..CreatingRevolve::default()
        });
        let revolve = available_gizmos(&state)
            .into_iter()
            .find(|g| g.name == "revolve")
            .expect("revolve gizmo");
        assert_eq!(revolve.kind, "rotate");
        assert!((revolve.value - 90f32.to_radians()).abs() < 1e-5);
        assert!(set_gizmo(&mut state, "revolve", PI)); // half turn
        assert!((state.creating_revolve.as_ref().unwrap().angle_live - 180.0).abs() < 1e-3);

        let mut state = AppState::default();
        state.apply(Action::BeginConstructionPlane {
            reference: PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: glam::Vec3::Z,
                label: "p".to_string(),
            },
            parent: ConstructionPlaneParent::Root,
        });
        assert!(state.creating_plane.is_some());
        assert!(set_gizmo(&mut state, "offset", 12.0));
        assert!((state.creating_plane.as_ref().unwrap().offset_live - 12.0).abs() < 1e-3);
        assert!((gizmo_value(&state, "offset").unwrap() - 12.0).abs() < 1e-3);
    }

    /// #185: the Move tool's translation (mm) and rotation (radians, only with an axis) are
    /// exposed and driven through the gizmo registry.
    #[test]
    fn gizmos_cover_move_translation_and_rotation() {
        use std::f32::consts::PI;
        let mut state = AppState::default();
        state.creating_move = Some(CreatingMove {
            targets: vec![],
            plane_targets: vec![],
            image_targets: vec![],
            tx: "5mm".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
            editing: None,
        });
        let names: Vec<&str> = available_gizmos(&state).iter().map(|g| g.name).collect();
        assert!(names.contains(&"move_x") && names.contains(&"move_y") && names.contains(&"move_z"));
        assert!(!names.contains(&"move_angle"), "no rotation gizmo without an axis");
        assert!((gizmo_value(&state, "move_x").unwrap() - 5.0).abs() < 1e-4);
        assert!(set_gizmo(&mut state, "move_y", 8.0));
        assert!((gizmo_value(&state, "move_y").unwrap() - 8.0).abs() < 1e-4);

        // A rotation axis makes the angle gizmo appear; it round-trips in radians.
        state.creating_move.as_mut().unwrap().axis = Some(crate::model::RevolveAxis::Z);
        assert!(set_gizmo(&mut state, "move_angle", PI));
        assert!((gizmo_value(&state, "move_angle").unwrap() - PI).abs() < 1e-3);
    }
    use crate::face::SketchFrame;

    fn xy_frame() -> SketchFrame {
        SketchFrame {
            origin: Vec3::ZERO,
            u_axis: Vec3::X,
            v_axis: Vec3::Y,
            normal: Vec3::Z,
        }
    }

    /// Dominant screen direction of a world axis from the origin (egui y-down).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ScreenAxisDir {
        Left,
        Right,
        Up,
        Down,
    }

    fn axis_screen_dir(
        cam: &crate::camera::Camera,
        viewport: egui::Rect,
        world_axis: Vec3,
    ) -> Option<ScreenAxisDir> {
        let vp = cam.view_proj(viewport);
        let o = cam.project(Vec3::ZERO, viewport, &vp)?;
        let p = cam.project(world_axis * 100.0, viewport, &vp)?;
        let d = p - o;
        if d.length() < 1.0 {
            return None;
        }
        if d.x.abs() >= d.y.abs() {
            Some(if d.x > 0.0 {
                ScreenAxisDir::Right
            } else {
                ScreenAxisDir::Left
            })
        } else if d.y > 0.0 {
            Some(ScreenAxisDir::Down)
        } else {
            Some(ScreenAxisDir::Up)
        }
    }

    fn axis_layout(
        cam: &crate::camera::Camera,
        viewport: egui::Rect,
    ) -> Option<(ScreenAxisDir, ScreenAxisDir)> {
        Some((
            axis_screen_dir(cam, viewport, Vec3::X)?,
            axis_screen_dir(cam, viewport, Vec3::Y)?,
        ))
    }

    fn begin_default_sketch(state: &mut AppState) -> SketchId {
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        state.sketch_session.unwrap().sketch
    }

    #[test]
    fn extrude_tool_toggles_closed_line_loop_polygon_face() {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, Line, LineEnd};

        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 5.0, 8.0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 5.0, 8.0, 0.0, 0.0));
        state.doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line, ShapeKind::Line]);
        let coincident = |a, b| Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(a),
                b: ConstraintEntity::Point(b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        };
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        state.doc.constraints.push(coincident(point(0, LineEnd::End), point(1, LineEnd::Start)));
        state.doc.constraints.push(coincident(point(1, LineEnd::End), point(2, LineEnd::Start)));
        state.doc.constraints.push(coincident(point(2, LineEnd::End), point(0, LineEnd::Start)));
        state.refresh_document_health();

        let loops = crate::polygon::closed_line_loops(&state.doc, sketch);
        assert_eq!(loops.len(), 1);
        let face = ExtrudeFace::Polygon(loops[0].clone());

        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace { face: face.clone() });
        assert_eq!(state.creating_extrusion.as_ref().unwrap().faces, vec![face]);
        state.apply(Action::SetExtrudeDistance { distance: 6.0 });
        state.apply(Action::CommitExtrusion);

        assert_eq!(state.doc.extrusions.len(), 1);
        assert_eq!(state.doc.bodies.len(), 1);
    }

    #[test]
    fn extrude_distance_input_defines_an_inline_parameter() {
        // Typing `name = expr` as the extrude depth defines a parameter and drives the
        // extrusion from it, like the sketch dimension inputs (#196).
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        let rect =
            crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        state.refresh_document_health();
        let face = ExtrudeFace::Polygon(rect.to_vec());
        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace { face });
        {
            let ce = state.creating_extrusion.as_mut().unwrap();
            ce.text = "depth = 20".to_string();
            ce.user_edited = true;
        }
        assert!(
            matches!(state.apply(Action::CommitExtrusion), ActionResult::Ok),
            "commit failed: {}",
            state.status
        );
        assert!(
            state.doc.parameters.iter().any(|p| !p.deleted && p.name == "depth"),
            "a `depth` parameter should have been defined"
        );
        assert!(
            (state.doc.extrusions[0].distance.abs() - 20.0).abs() < 1e-3,
            "distance={}",
            state.doc.extrusions[0].distance
        );
    }

    #[test]
    fn import_stl_adds_a_body_from_ascii_stl() {
        let mut state = AppState::default();
        let stl = "solid tri\n  facet normal 0 0 1\n    outer loop\n      vertex 0 0 0\n      vertex 1 0 0\n      vertex 0 1 0\n    endloop\n  endfacet\nendsolid tri\n";
        let path = std::env::temp_dir().join(format!("bearcad_import_{}.stl", std::process::id()));
        std::fs::write(&path, stl).unwrap();
        let path_str = path.to_string_lossy().to_string();

        let result = state.apply(Action::ImportStl { path: path_str.clone() });
        assert_eq!(result, ActionResult::Ok, "status: {}", state.status);
        assert_eq!(state.doc.imported_meshes.len(), 1);
        assert_eq!(state.doc.imported_meshes[0].triangles.len(), 1);
        assert_eq!(state.doc.bodies.len(), 1);
        assert_eq!(state.doc.bodies[0].source, crate::model::BodySource::Imported(0));
        assert_eq!(
            state.doc.bodies[0].name.as_deref(),
            path.file_stem().unwrap().to_str()
        );

        let mesh = crate::extrude::body_solid_mesh(&state.doc, 0).expect("imported mesh");
        assert_eq!(mesh.triangles.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn import_stl_reports_error_for_missing_file() {
        let mut state = AppState::default();
        let result = state.apply(Action::ImportStl {
            path: "/nonexistent/path/does-not-exist.stl".to_string(),
        });
        assert!(matches!(result, ActionResult::Err(_)));
        assert!(state.doc.imported_meshes.is_empty());
        assert!(state.doc.bodies.is_empty());
    }

    #[test]
    fn undo_after_import_stl_removes_the_body() {
        let mut state = AppState::default();
        let stl = "solid tri\n  facet normal 0 0 1\n    outer loop\n      vertex 0 0 0\n      vertex 1 0 0\n      vertex 0 1 0\n    endloop\n  endfacet\nendsolid tri\n";
        let path = std::env::temp_dir().join(format!("bearcad_import_undo_{}.stl", std::process::id()));
        std::fs::write(&path, stl).unwrap();
        state.apply(Action::ImportStl { path: path.to_string_lossy().to_string() });
        assert_eq!(state.doc.bodies.len(), 1);

        state.apply(Action::UndoLast);
        assert!(state.doc.bodies.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn import_step_rejects_a_file_that_is_not_step() {
        let mut state = AppState::default();
        let path = std::env::temp_dir().join(format!("bearcad_bad_import_{}.step", std::process::id()));
        std::fs::write(&path, "not a step file").unwrap();
        let result = state.apply(Action::ImportStep { path: path.to_string_lossy().to_string() });
        assert!(matches!(result, ActionResult::Err(_)));
        assert!(state.doc.bodies.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn picking_extrude_tool_from_within_a_sketch_exits_sketch_editing() {
        let mut state = AppState::default();
        let _sketch = begin_default_sketch(&mut state);
        assert!(state.sketch_session.is_some());
        // Extruding acts on the 3D model, so entering the tool leaves sketch editing.
        state.apply(Action::SetTool(Tool::Extrude));
        assert_eq!(state.tool, Tool::Extrude);
        assert!(state.sketch_session.is_none());
    }

    #[test]
    fn apply_snap_constraint_adds_coincident_dedups_and_solves() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 10.3, 0.2, 20.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.shape_order.push(ShapeKind::Line);
        state.refresh_document_health();

        let moved = ConstraintPoint::LineEndpoint {
            line: 1,
            end: LineEnd::Start,
        };
        let anchor = ConstraintPoint::LineEndpoint {
            line: 0,
            end: LineEnd::End,
        };
        let target = crate::snapping::SnapTarget::Vertex(anchor.clone());

        let before = state.doc.constraints.len();
        state.apply(Action::ApplySnapConstraint {
            point: moved.clone(),
            target: target.clone(),
        });
        assert_eq!(state.doc.constraints.len(), before + 1);

        let a = crate::geometric_constraints::point_uv(&state.doc, sketch, anchor.clone()).unwrap();
        let m = crate::geometric_constraints::point_uv(&state.doc, sketch, moved.clone()).unwrap();
        assert!(
            (a.0 - m.0).abs() < 1e-2 && (a.1 - m.1).abs() < 1e-2,
            "snapped endpoints should coincide: {a:?} vs {m:?}"
        );

        // Applying the same snap again must not add a duplicate constraint.
        state.apply(Action::ApplySnapConstraint {
            point: moved,
            target,
        });
        assert_eq!(state.doc.constraints.len(), before + 1);
    }

    #[test]
    fn apply_normal_at_midpoint_snap_invents_construction_line_and_constraints() {
        // #41: touching a line's midpoint, moving away, then leaving a point on the guide
        // perpendicular to it should invent a construction line + Midpoint/Perpendicular/
        // Coincident constraints, rather than requiring a new constraint primitive.
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        // A second line whose endpoint is the point being placed, positioned on the
        // perpendicular through the anchor's midpoint (5, 0) — i.e. u=5.
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 5.0, 4.0, 20.0, 4.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.refresh_document_health();

        let anchor = ConstraintLine::Line(0);
        let point = ConstraintPoint::LineEndpoint {
            line: 1,
            end: LineEnd::Start,
        };
        let target = crate::snapping::SnapTarget::NormalAtMidpoint(anchor.clone());

        let lines_before = state.doc.lines.len();
        let constraints_before = state.doc.constraints.len();
        let result = state.apply(Action::ApplySnapConstraint { point: point.clone(), target });
        assert_eq!(result, ActionResult::Ok);

        // A new construction line was invented.
        assert_eq!(state.doc.lines.len(), lines_before + 1);
        let new_line_index = state.doc.lines.len() - 1;
        assert!(state.doc.lines[new_line_index].construction);

        // Three new constraints were added: Midpoint, Perpendicular, and a point-on-line
        // Coincident pinning the placed point to the new line's carrier.
        assert_eq!(state.doc.constraints.len(), constraints_before + 3);
        let new_line = ConstraintLine::Line(new_line_index);
        let kinds: Vec<_> = state.doc.constraints[constraints_before..]
            .iter()
            .map(|c| c.kind.clone())
            .collect();
        assert!(kinds.iter().any(|k| matches!(
            k,
            ConstraintKind::Midpoint { line, .. } if *line == anchor
        )));
        assert!(kinds.iter().any(|k| matches!(
            k,
            ConstraintKind::Perpendicular { line_a, line_b }
                if *line_a == new_line && *line_b == anchor
        )));
        assert!(kinds.iter().any(|k| matches!(
            k,
            ConstraintKind::Coincident {
                a: ConstraintEntity::Point(p),
                b: ConstraintEntity::Line(l),
            } if *p == point && *l == new_line
        )));
    }

    #[test]
    fn set_snapping_toggles_flag() {
        let mut state = AppState::default();
        assert!(state.snapping_enabled);
        state.apply(Action::SetSnapping(false));
        assert!(!state.snapping_enabled);
        state.apply(Action::SetSnapping(true));
        assert!(state.snapping_enabled);
    }

    #[test]
    fn set_document_units_updates_defaults() {
        let mut state = AppState::default();
        assert_eq!(state.doc.default_length_unit, LengthUnit::Mm);
        assert_eq!(state.doc.default_angle_unit, AngleUnit::Deg);
        let result = state.apply(Action::SetDocumentUnits {
            length: LengthUnit::In,
            angle: AngleUnit::Rad,
        });
        assert_eq!(result, ActionResult::Ok);
        assert_eq!(state.doc.default_length_unit, LengthUnit::In);
        assert_eq!(state.doc.default_angle_unit, AngleUnit::Rad);
    }

    #[test]
    fn set_sketch_units_overrides_and_clears() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        let result = state.apply(Action::SetSketchUnits {
            sketch,
            length: Some(LengthUnit::Cm),
            angle: Some(AngleUnit::Rad),
        });
        assert_eq!(result, ActionResult::Ok);
        assert_eq!(state.doc.sketches[sketch].length_unit, Some(LengthUnit::Cm));
        assert_eq!(state.doc.sketches[sketch].angle_unit, Some(AngleUnit::Rad));

        // `None` clears the override back to inheriting the document default.
        state.apply(Action::SetSketchUnits {
            sketch,
            length: None,
            angle: None,
        });
        assert_eq!(state.doc.sketches[sketch].length_unit, None);
        assert_eq!(state.doc.sketches[sketch].angle_unit, None);
    }

    #[test]
    fn set_sketch_units_errors_for_missing_sketch() {
        let mut state = AppState::default();
        let result = state.apply(Action::SetSketchUnits {
            sketch: 42,
            length: Some(LengthUnit::Mm),
            angle: None,
        });
        assert_eq!(
            result,
            ActionResult::Err("Sketch 42 not found".to_string())
        );
    }

    #[test]
    fn set_tool_line_without_sketch_session() {
        let mut state = AppState::default();
        let result = state.apply(Action::SetTool(Tool::Line));
        assert_eq!(result, ActionResult::Ok);
        assert_eq!(state.tool, Tool::Line);
        assert!(state.sketch_session.is_none());
    }

    #[test]
    fn begin_sketch_preserves_rectangle_tool() {
        let mut state = AppState::default();
        state.apply(Action::SetTool(Tool::Rectangle));
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        assert_eq!(state.tool, Tool::Rectangle);
        assert!(state.sketch_session.is_some());
    }

    #[test]
    fn begin_sketch_from_sketch_tool_resets_to_select() {
        let mut state = AppState::default();
        state.apply(Action::SetTool(Tool::Sketch));
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        assert_eq!(state.tool, Tool::Select);
    }

    #[test]
    fn set_tool_construction_plane() {
        let mut state = AppState::default();
        state.apply(Action::SetTool(Tool::ConstructionPlane));
        assert_eq!(state.tool, Tool::ConstructionPlane);
    }

    /// Revolve (SPEC §3.5): committing a face + axis creates the revolution and its body
    /// under one marker; undo removes both. Cut mode requires picked bodies.
    #[test]
    fn commit_revolve_creates_body_and_undo_removes_both() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        let lines = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            10.0,
            0.0,
            10.0,
            10.0,
            [false; 4],
        );
        state.creating_revolve = Some(CreatingRevolve {
            sketch: Some(sketch),
            faces: vec![crate::model::ExtrudeFace::Polygon(lines.to_vec())],
            axis: Some(crate::model::RevolveAxis::Y),
            ..CreatingRevolve::default()
        });
        assert!(matches!(state.apply(Action::CommitRevolve), ActionResult::Ok));
        assert_eq!(state.doc.revolutions.len(), 1);
        assert!((state.doc.revolutions[0].angle_deg - 360.0).abs() < 1e-3);
        assert_eq!(
            state.doc.bodies.last().map(|b| b.source.clone()),
            Some(crate::model::BodySource::Revolve(0))
        );
        assert_eq!(state.doc.shape_order.last(), Some(&ShapeKind::Revolution));
        assert!(state.creating_revolve.is_none());
        assert!(crate::extrude::body_solid_mesh(&state.doc, state.doc.bodies.len() - 1).is_some());

        state.apply(Action::UndoLast);
        assert!(state.doc.revolutions.is_empty());
        assert!(state.doc.bodies.is_empty());
    }

    /// Editing a committed revolution (#211) re-points it in place: the angle changes, no new
    /// revolution or body is created, and its NewBody output body is preserved.
    #[test]
    fn edit_revolve_updates_params_in_place() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        let lines = crate::construction::add_line_rectangle(
            &mut state.doc, sketch, 10.0, 0.0, 10.0, 10.0, [false; 4],
        );
        let faces = vec![crate::model::ExtrudeFace::Polygon(lines.to_vec())];
        state.creating_revolve = Some(CreatingRevolve {
            sketch: Some(sketch),
            faces: faces.clone(),
            axis: Some(crate::model::RevolveAxis::Y),
            ..CreatingRevolve::default()
        });
        assert!(matches!(state.apply(Action::CommitRevolve), ActionResult::Ok));
        assert_eq!(state.doc.revolutions.len(), 1);
        let body_count = state.doc.bodies.len();

        // Re-open the revolution for editing with a new (180°) sweep.
        state.creating_revolve = Some(CreatingRevolve {
            sketch: Some(sketch),
            faces,
            axis: Some(crate::model::RevolveAxis::Y),
            text: "180".to_string(),
            user_edited: true,
            editing: Some(0),
            ..CreatingRevolve::default()
        });
        assert!(matches!(state.apply(Action::CommitRevolve), ActionResult::Ok));
        // Same revolution and body count; only the angle changed.
        assert_eq!(state.doc.revolutions.len(), 1);
        assert_eq!(state.doc.bodies.len(), body_count);
        assert!((state.doc.revolutions[0].angle_deg - 180.0).abs() < 1e-3);
        assert!(state.creating_revolve.is_none());
    }

    /// Cut mode without picked bodies is rejected and the in-progress state survives.
    #[test]
    fn commit_revolve_cut_requires_bodies() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        let lines = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            10.0,
            0.0,
            10.0,
            10.0,
            [false; 4],
        );
        state.creating_revolve = Some(CreatingRevolve {
            sketch: Some(sketch),
            faces: vec![crate::model::ExtrudeFace::Polygon(lines.to_vec())],
            axis: Some(crate::model::RevolveAxis::Y),
            body_choice: RevolveBodyChoice::Cut,
            ..CreatingRevolve::default()
        });
        assert!(matches!(
            state.apply(Action::CommitRevolve),
            ActionResult::Err(_)
        ));
        assert!(state.doc.revolutions.is_empty());
        assert!(state.creating_revolve.is_some());
    }

    /// A fillet/chamfer trim must re-target the trimmed lines' LENGTH dimensions:
    /// with stale dims, the next full solve dragged the endpoints back and mangled
    /// the treated corner (the Quickstart bracket's bend collapsed one step later).
    #[test]
    fn vertex_treatment_retargets_length_dims_so_resolve_keeps_the_trim() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        // Two lines meeting at (10, 0), each with a numeric length dim (like typed input).
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        for (li, len) in [(0usize, "10"), (1, "10")] {
            state.doc.constraints.push(crate::model::Constraint {
                sketch,
                kind: ConstraintKind::Distance {
                    target: DistanceTarget::LineLength(li),
                },
                expression: len.to_string(),
                dim_offset: None,
                name: None,
                deleted: false,
            });
        }
        state.doc.constraints.push(crate::model::Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 0,
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        let result = state.apply(Action::CommitVertexTreatment {
            point: ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::End },
            kind: VertexTreatmentKind::Fillet,
            amount: 3.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        // Trimmed: line 0 now ends at x = 7.
        assert!((state.doc.lines[0].x1 - 7.0).abs() < 1e-3);
        // The length dims followed the trim.
        let dim0 = state
            .doc
            .constraints
            .iter()
            .find(|c| {
                !c.deleted
                    && matches!(
                        &c.kind,
                        ConstraintKind::Distance { target: DistanceTarget::LineLength(0) }
                    )
            })
            .expect("length dim");
        assert!(
            (dim0.expression.parse::<f32>().unwrap() - 7.0).abs() < 1e-2,
            "dim should be ~7, got {}",
            dim0.expression
        );
        // A full re-solve keeps the trim instead of restoring the old length.
        crate::constraints::solve_document_constraints(&mut state.doc).unwrap();
        assert!(
            (state.doc.lines[0].x1 - 7.0).abs() < 1e-2,
            "trim must survive a re-solve, got x1 = {}",
            state.doc.lines[0].x1
        );
    }

    /// Loft (SPEC §3.5): toggling two circle sections and committing creates a loft plus
    /// its body under one undo marker; undo removes both together.
    #[test]
    fn commit_loft_creates_body_and_undo_removes_both() {
        let mut state = AppState::default();
        let bottom = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state
            .doc
            .circles
            .push(crate::model::Circle::from_local_center_radius(bottom, 0.0, 0.0, 5.0, 0.0));
        state.doc.construction_planes.push(plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Face {
                    origin: Vec3::ZERO,
                    normal: Vec3::Z,
                    label: "Ground".to_string(),
                },
                10.0,
                0.0,
            ),
            ConstructionPlaneParent::Root,
        ));
        let top = state.doc.add_sketch(FaceId::ConstructionPlane(1));
        state
            .doc
            .circles
            .push(crate::model::Circle::from_local_center_radius(top, 0.0, 0.0, 2.0, 0.0));

        for (sketch, ci) in [(bottom, 0), (top, 1)] {
            let result = state.apply(Action::ToggleLoftSection {
                section: crate::model::LoftSection {
                    sketch,
                    face: crate::model::ExtrudeFace::Circle(ci),
                },
            });
            assert!(matches!(result, ActionResult::Ok));
        }
        assert!(matches!(state.apply(Action::CommitLoft), ActionResult::Ok));
        assert_eq!(state.doc.lofts.len(), 1);
        assert_eq!(state.doc.lofts[0].sections.len(), 2);
        assert_eq!(
            state.doc.bodies.last().map(|b| b.source.clone()),
            Some(crate::model::BodySource::Loft(0))
        );
        assert_eq!(state.doc.shape_order.last(), Some(&ShapeKind::Loft));
        assert!(state.creating_loft.is_none());
        assert!(crate::extrude::body_solid_mesh(&state.doc, state.doc.bodies.len() - 1).is_some());

        state.apply(Action::UndoLast);
        assert!(state.doc.lofts.is_empty());
        assert!(state.doc.bodies.is_empty());
        assert!(!state.doc.shape_order.contains(&ShapeKind::Loft));
    }

    /// A loft needs at least two sections; a single-section commit is rejected and the
    /// in-progress state survives for the user to keep picking.
    #[test]
    fn commit_loft_rejects_fewer_than_two_sections() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state
            .doc
            .circles
            .push(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        state.apply(Action::ToggleLoftSection {
            section: crate::model::LoftSection {
                sketch,
                face: crate::model::ExtrudeFace::Circle(0),
            },
        });
        assert!(matches!(state.apply(Action::CommitLoft), ActionResult::Err(_)));
        assert!(state.doc.lofts.is_empty());
        assert_eq!(state.creating_loft.as_ref().map(|cl| cl.sections.len()), Some(1));
    }

    #[test]
    fn edit_construction_plane_updates_offset_and_descendants() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.construction_planes.push(plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Face {
                    origin: Vec3::ZERO,
                    normal: Vec3::Z,
                    label: "Ground".to_string(),
                },
                5.0,
                0.0,
            ),
            ConstructionPlaneParent::Sketch(sketch),
        ));
        let child_before = state.doc.construction_planes[1].origin.z;

        state.apply(Action::BeginEditConstructionPlane { index: 0 });
        state.apply(Action::SetPlaneOffset {
            value: "30".to_string(),
        });
        state.apply(Action::CommitConstructionPlane);

        assert!((state.doc.construction_planes[0].origin.z - 30.0).abs() < 1e-3);
        assert!((state.doc.construction_planes[1].origin.z - child_before - 30.0).abs() < 1e-3);
        assert!(state.creating_plane.is_none());
    }

    #[test]
    fn commit_construction_plane_adds_to_document_not_export_list() {
        let mut state = AppState::default();
        state.apply(Action::BeginConstructionPlane {
            reference: PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: Vec3::Z,
                label: "Ground".to_string(),
            },
            parent: ConstructionPlaneParent::Root,
        });
        let mut cp = state.creating_plane.take().unwrap();
        cp.offset_text = "20".to_string();
        cp.user_edited_offset = true;
        state.creating_plane = Some(cp);
        state.apply(Action::CommitConstructionPlane);
        assert_eq!(state.doc.construction_planes.len(), 2);
        assert!((state.doc.construction_planes[1].origin.z - 20.0).abs() < 1e-3);
        assert!(state
            .scene_selection
            .is_selected(SceneElement::ConstructionPlane(1)));
    }

    #[test]
    fn commit_construction_plane_replaces_stale_selection() {
        let mut state = AppState::default();
        state.apply(Action::BeginConstructionPlane {
            reference: PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: glam::Vec3::Z,
                label: "Ground".to_string(),
            },
            parent: ConstructionPlaneParent::Root,
        });
        state.scene_selection.clear();
        click_scene_selection(
            &mut state.scene_selection,
            SceneElement::ConstructionPlane(0),
            false,
        );
        let mut cp = state.creating_plane.take().unwrap();
        cp.offset_live = 12.0;
        state.creating_plane = Some(cp);
        state.apply(Action::CommitConstructionPlane);
        assert!(!state
            .scene_selection
            .is_selected(SceneElement::ConstructionPlane(0)));
        assert!(state
            .scene_selection
            .is_selected(SceneElement::ConstructionPlane(1)));
    }

    #[test]
    fn live_dims_use_offset_live_not_mouse() {
        let mut state = AppState::default();
        state.apply(Action::BeginConstructionPlane {
            reference: PlaneReference::Axis {
                origin: Vec3::ZERO,
                direction: Vec3::X,
                label: "Line".to_string(),
            },
            parent: ConstructionPlaneParent::Root,
        });
        let cp = state.creating_plane.as_mut().unwrap();
        cp.offset_live = 12.0;
        cp.axis_angle_deg = 45.0;
        assert_eq!(cp.live_dims(), (12.0, 45.0));
    }

    #[test]
    fn undo_construction_plane() {
        let mut state = AppState::default();
        state.apply(Action::BeginConstructionPlane {
            reference: PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: Vec3::Z,
                label: "Ground".to_string(),
            },
            parent: ConstructionPlaneParent::Root,
        });
        let mut cp = state.creating_plane.take().unwrap();
        cp.offset_text = "5".to_string();
        cp.user_edited_offset = true;
        state.creating_plane = Some(cp);
        state.apply(Action::CommitConstructionPlane);
        state.apply(Action::UndoLast);
        assert_eq!(state.doc.construction_planes.len(), 1);
    }

    #[test]
    fn undo_construction_plane_edit_restores_previous() {
        let mut state = AppState::default();
        state.apply(Action::BeginConstructionPlane {
            reference: PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: Vec3::Z,
                label: "Ground".to_string(),
            },
            parent: ConstructionPlaneParent::Root,
        });
        let mut cp = state.creating_plane.take().unwrap();
        cp.offset_text = "5".to_string();
        cp.user_edited_offset = true;
        state.creating_plane = Some(cp);
        state.apply(Action::CommitConstructionPlane);
        assert_eq!(state.doc.construction_planes.len(), 2);
        assert!((state.doc.construction_planes[1].origin.z - 5.0).abs() < 1e-3);

        // Edit the plane to a new offset.
        state.apply(Action::BeginEditConstructionPlane { index: 1 });
        state.apply(Action::SetPlaneOffset {
            value: "30".to_string(),
        });
        state.apply(Action::CommitConstructionPlane);
        assert!((state.doc.construction_planes[1].origin.z - 30.0).abs() < 1e-3);

        // Undo should revert the edit, not delete the plane.
        state.apply(Action::UndoLast);
        assert_eq!(state.doc.construction_planes.len(), 2);
        assert!(
            (state.doc.construction_planes[1].origin.z - 5.0).abs() < 1e-3,
            "expected undo to restore offset 5, got {}",
            state.doc.construction_planes[1].origin.z
        );
    }

    #[test]
    fn undo_construction_plane_edit_restores_descendants() {
        let mut state = AppState::default();
        // A sketch on the default plane (0) and a child plane defined relative to it,
        // so editing plane 0 moves the child (index 1).
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.construction_planes.push(plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Face {
                    origin: Vec3::ZERO,
                    normal: Vec3::Z,
                    label: "Ground".to_string(),
                },
                5.0,
                0.0,
            ),
            ConstructionPlaneParent::Sketch(sketch),
        ));
        state.doc.shape_order.push(ShapeKind::ConstructionPlane);
        let child_before = state.doc.construction_planes[1].origin.z;

        state.apply(Action::BeginEditConstructionPlane { index: 0 });
        state.apply(Action::SetPlaneOffset {
            value: "30".to_string(),
        });
        state.apply(Action::CommitConstructionPlane);
        assert!((state.doc.construction_planes[1].origin.z - child_before).abs() > 1e-3);

        state.apply(Action::UndoLast);
        assert!(
            (state.doc.construction_planes[1].origin.z - child_before).abs() < 1e-3,
            "expected descendant restored to {child_before}, got {}",
            state.doc.construction_planes[1].origin.z
        );
    }

    fn recording_state() -> AppState {
        let mut state = AppState::default();
        state.command_log = Some(std::cell::RefCell::new(
            crate::command_log::CommandLog::new_recording(false),
        ));
        state
    }

    #[test]
    fn interactive_rect_line_circle_commits_are_logged_for_replay() {
        let mut state = recording_state();
        begin_default_sketch(&mut state);
        state.creating_rect = Some(CreatingRect {
            origin: Vec3::new(0.0, 0.0, 0.0),
            texts: ["".to_string(), "".to_string()],
            focused: 0,
            last_mouse: Vec3::new(10.0, 5.0, 0.0),
            user_edited: [false, false],
            pending_focus: false,
            construction: false,
        });
        state.apply(Action::CommitRectangle);
        state.creating_line = Some(CreatingLine {
            origin: Vec3::new(0.0, 0.0, 0.0),
            text: String::new(),
            last_mouse: Vec3::new(20.0, 0.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::CommitLine);
        state.creating_circle = Some(CreatingCircle {
            origin: Vec3::new(0.0, 0.0, 0.0),
            text: String::new(),
            last_mouse: Vec3::new(8.0, 0.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
        });
        state.apply(Action::CommitCircle);

        let lua: Vec<String> = state
            .command_log
            .as_ref()
            .unwrap()
            .borrow()
            .session_lua_script("ts")
            .lines()
            .filter(|l| l.starts_with("bearcad."))
            .map(|l| l.to_string())
            .collect();
        assert!(lua.iter().any(|l| l.starts_with("bearcad.rect")), "{lua:?}");
        assert!(lua.iter().any(|l| l.starts_with("bearcad.line")), "{lua:?}");
        assert!(lua.iter().any(|l| l.starts_with("bearcad.circle")), "{lua:?}");
    }

    /// #136: a line endpoint snapping onto an existing vertex while drawing (closing a
    /// polyline loop) adds a `Coincident` constraint as a side effect of `CommitLine` — that
    /// must show up in the replay log too, not just the raw `bearcad.line{}` call, or the
    /// exported script silently drops the loop closure on replay.
    #[test]
    fn snap_closed_line_constraint_is_logged_for_replay() {
        let mut state = recording_state();
        let sketch = begin_default_sketch(&mut state);
        state
            .doc
            .lines
            .push(crate::model::Line::from_local_endpoints(sketch, 10.0, 0.0, 20.0, 0.0));
        state.doc.shape_order.push(crate::model::ShapeKind::Line);
        state.tool = Tool::Line;
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: "10".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.line_end_snap = Some(crate::snapping::SnapTarget::Vertex(
            ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::Start },
        ));
        state.apply(Action::CommitLine);
        assert!(
            state
                .doc
                .constraints
                .iter()
                .any(|c| !c.deleted && matches!(c.kind, crate::model::ConstraintKind::Coincident { .. })),
            "commit should have added the snap coincident constraint"
        );

        let lua: Vec<String> = state
            .command_log
            .as_ref()
            .unwrap()
            .borrow()
            .session_lua_script("ts")
            .lines()
            .filter(|l| l.starts_with("bearcad."))
            .map(|l| l.to_string())
            .collect();
        assert!(lua.iter().any(|l| l.starts_with("bearcad.select")), "{lua:?}");
        assert!(
            lua.iter().any(|l| l.contains("add_geometric_constraint(\"coincident\")")),
            "{lua:?}"
        );
    }

    #[test]
    fn create_parameter_from_line_length_action() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 0.0, 15.0, 0.0,
        ));
        state.doc.shape_order.push(crate::model::ShapeKind::Line);
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(0),
            additive: false,
        });
        state.apply(Action::CreateParameterFromLineLength {
            line_index: 0,
            name: None,
        });
        assert_eq!(state.doc.parameters.len(), 1);
        assert_eq!(state.doc.parameters[0].name, "line0_length");
        assert_eq!(state.doc.parameters[0].expression, "15.0 mm");
        assert!(crate::parameters::parameter_value_is_readonly(
            &state.doc.parameters[0]
        ));
    }

    #[test]
    fn apply_constraint_shortcut_a_adds_parallel() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.tool = Tool::Constraint;
        state.doc.lines.push(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 0.0, 10.0, 0.0,
        ));
        state.doc.lines.push(crate::model::Line::from_local_endpoints(
            sketch, 0.0, 5.0, 2.0, 8.0,
        ));
        state.doc.shape_order.push(crate::model::ShapeKind::Line);
        state.doc.shape_order.push(crate::model::ShapeKind::Line);
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(0),
            additive: false,
        });
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(1),
            additive: true,
        });
        state.apply(Action::ApplyConstraintShortcut('A'));
        assert_eq!(state.doc.constraints.len(), 1);
        assert!(matches!(
            state.doc.constraints[0].kind,
            crate::model::ConstraintKind::Parallel { .. }
        ));
    }

    #[test]
    fn rect_end_point_uses_parameter_reference() {
        let mut doc = Document::default();
        add_parameter(&mut doc, "A".to_string(), "10mm".to_string()).unwrap();
        let cr = CreatingRect {
            origin: Vec3::ZERO,
            texts: ["A".to_string(), "".to_string()],
            focused: 0,
            last_mouse: Vec3::new(100.0, 4.0, 0.0),
            user_edited: [true, false],
            pending_focus: false,
            construction: false,
        };
        let frame = xy_frame();
        let end = cr.end_point(&frame, &doc);
        assert!((end.x - 10.0).abs() < 1e-3);
        // Height is unconstrained, so it follows the mouse.
        assert!((end.y - 4.0).abs() < 1e-3);
    }

    #[test]
    fn begin_edit_committed_dim_blocked_while_drawing_rectangle() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.creating_rect = Some(CreatingRect {
            origin: Vec3::ZERO,
            texts: ["".to_string(), "".to_string()],
            focused: 0,
            last_mouse: Vec3::new(10.0, 5.0, 0.0),
            user_edited: [false, false],
            pending_focus: false,
            construction: false,
        });
        let result = state.apply(Action::BeginEditCommittedDim { target: 0 });
        assert!(matches!(result, ActionResult::Err(_)));
        assert!(state.editing_committed_dim.is_none());
    }

    #[test]
    fn commit_line_adds_to_document() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: "10".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::CommitLine);
        assert_eq!(state.doc.lines.len(), 1);
        assert!((state.doc.lines[0].length() - 10.0).abs() < 1e-4);
        assert_eq!(state.doc.constraints.len(), 1);
        assert!(state.doc.lines[0].length_locked);
        assert!(state.creating_line.is_none());
    }

    #[test]
    fn commit_line_without_curve_mode_stays_straight() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: String::new(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::CommitLine);
        assert_eq!(state.doc.lines.len(), 1);
        assert!(!state.doc.lines[0].is_curved());
    }

    #[test]
    fn commit_line_curve_mode_smooths_the_shared_vertex_with_the_previous_segment() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.tool = Tool::Line;
        state.draw_curve_mode = true;
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: String::new(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: true,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::CommitLine);
        assert_eq!(state.doc.lines.len(), 1);
        // The first segment of a fresh chain has nothing to smooth against yet.
        assert!(!state.doc.lines[0].is_curved());
        // Chaining should have carried curve-mode into the new segment.
        let cl = state
            .creating_line
            .as_ref()
            .expect("should chain into a new segment");
        assert!(cl.curve_mode);
        assert_eq!(cl.chained_from, Some(0));

        state.creating_line.as_mut().unwrap().last_mouse = Vec3::new(20.0, 5.0, 0.0);
        state.apply(Action::CommitLine);
        assert_eq!(state.doc.lines.len(), 2);
        // The shared vertex (10,0) is now smoothed retroactively on both sides.
        assert!(state.doc.lines[0].is_curved());
        assert!(state.doc.lines[1].is_curved());
        let h0_far = state.doc.lines[0].bezier.unwrap()[0];
        // Line 0 runs along +x from (0,0) to (10,0): its far (from-vertex) handle sits a third
        // of the way along that chord, independent of where the next point ended up.
        assert!((h0_far.0 - 10.0 / 3.0).abs() < 1e-3);
        assert!(h0_far.1.abs() < 1e-3);
    }

    #[test]
    fn commit_line_curve_mode_without_tangent_constraint_gives_independent_handles() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.tool = Tool::Line;
        state.draw_curve_mode = true;
        state.draw_tangent_constraint = false;
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: String::new(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: true,
            tangent_constraint: false,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::CommitLine);
        state.creating_line.as_mut().unwrap().last_mouse = Vec3::new(20.0, 5.0, 0.0);
        state.apply(Action::CommitLine);
        // The previous segment is left completely untouched (tangent constraint is off).
        assert!(!state.doc.lines[0].is_curved());
        // But the new segment still gets its own independent "corner" handles.
        assert!(state.doc.lines[1].is_curved());
    }

    #[test]
    fn cancel_operation_reverts_the_previous_lines_live_curve_preview() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        // Simulate a live curve-mode preview frame having bent the previous line's end handle.
        state.doc.lines[0].bezier = Some([(3.0, 0.0), (9.0, 2.0)]);
        state.creating_line = Some(CreatingLine {
            origin: Vec3::new(10.0, 0.0, 0.0),
            text: String::new(),
            last_mouse: Vec3::new(20.0, 5.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: true,
            tangent_constraint: true,
            chained_from: Some(0),
            chained_from_bezier: None,
        });
        state.apply(Action::CancelOperation);
        assert!(state.creating_line.is_none());
        // Reverted to the pre-preview baseline (straight, since `chained_from_bezier` was `None`).
        assert!(!state.doc.lines[0].is_curved());
    }

    #[test]
    fn set_bezier_handle_moves_the_control_point() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines[0].bezier = Some([(3.0, 0.0), (7.0, 0.0)]);
        let result = state.apply(Action::SetBezierHandle {
            line: 0,
            near_start: true,
            u: 3.0,
            v: 5.0,
        });
        assert!(matches!(result, ActionResult::Ok));
        assert_eq!(state.doc.lines[0].bezier, Some([(3.0, 5.0), (7.0, 0.0)]));
    }

    #[test]
    fn set_bezier_handle_errors_on_a_straight_line() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let result = state.apply(Action::SetBezierHandle {
            line: 0,
            near_start: true,
            u: 3.0,
            v: 5.0,
        });
        assert!(matches!(result, ActionResult::Err(_)));
    }

    #[test]
    fn convert_vertex_to_bezier_smooths_two_coincident_lines() {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, Line, LineEnd};

        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 20.0, 0.0));
        state.doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        state.doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 0,
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        let point = ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::End };
        let result = state.apply(Action::ConvertVertexToBezier { point });
        assert!(matches!(result, ActionResult::Ok));
        assert!(state.doc.lines[0].is_curved());
        assert!(state.doc.lines[1].is_curved());
    }

    #[test]
    fn convert_vertex_to_bezier_rejects_an_endpoint_with_only_one_line() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let point = ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::Start };
        let result = state.apply(Action::ConvertVertexToBezier { point });
        assert!(matches!(result, ActionResult::Err(_)));
        assert!(!state.doc.lines[0].is_curved());
    }

    #[test]
    fn straighten_line_clears_bezier() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines[0].bezier = Some([(3.0, 1.0), (7.0, 1.0)]);
        let result = state.apply(Action::StraightenLine { line: 0 });
        assert!(matches!(result, ActionResult::Ok));
        assert!(!state.doc.lines[0].is_curved());
    }

    /// A 90-degree corner: line 0 from (0,0) to (10,0), line 1 from (10,0) to (10,10),
    /// coincident at the shared vertex (10,0). Returns `(sketch, point)` for that vertex.
    fn two_coincident_lines_at_a_right_angle(state: &mut AppState) -> (SketchId, ConstraintPoint) {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, LineEnd};

        let sketch = begin_default_sketch(state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        state.doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        state.doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 0,
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        state.doc.shape_order.push(ShapeKind::Constraint);
        let point = ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::End };
        (sketch, point)
    }

    /// #194: undoing a fillet restores the two original lines instead of deleting one. The
    /// old per-entry undo mis-reversed the truncate-and-bridge gesture; checkpoint undo
    /// reinstates the exact pre-fillet document.
    #[test]
    fn undo_after_fillet_restores_the_original_lines() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        assert_eq!(state.doc.lines.len(), 2);
        let l0 = state.doc.lines[0].clone();
        let l1 = state.doc.lines[1].clone();

        let result = state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Fillet,
            amount: 3.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(state.doc.lines.len(), 3, "the fillet adds a bridge line");

        state.apply(Action::UndoLast);
        assert_eq!(state.doc.lines.len(), 2, "undo removes the bridge line");
        assert_eq!(state.doc.lines[0], l0, "line 0 restored to its pre-fillet geometry");
        assert_eq!(state.doc.lines[1], l1, "line 1 restored to its pre-fillet geometry");
    }

    /// #193: redo re-applies an undone action, and a fresh action clears the redo stack.
    #[test]
    fn redo_reapplies_an_undone_action() {
        let mut state = AppState::default();
        state.apply(Action::AddParameter {
            name: "w".to_string(),
            expression: "10".to_string(),
        });
        assert_eq!(state.doc.parameters.len(), 1);

        state.apply(Action::UndoLast);
        assert!(state.doc.parameters.is_empty(), "undo removed the parameter");

        state.apply(Action::RedoLast);
        assert_eq!(state.doc.parameters.len(), 1, "redo restored the parameter");

        // A new mutating action after an undo clears the redo stack.
        state.apply(Action::UndoLast);
        state.apply(Action::AddParameter {
            name: "h".to_string(),
            expression: "20".to_string(),
        });
        state.apply(Action::RedoLast);
        assert_eq!(
            state.doc.parameters.len(),
            1,
            "redo is a no-op once a new action has cleared the redo stack"
        );
        assert_eq!(state.doc.parameters[0].name, "h");
    }

    #[test]
    fn set_vertex_tangent_continuous_smooths_both_lines() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        let result = state.apply(Action::SetVertexTangent { point, continuous: true });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert!(state.doc.lines[0].is_curved());
        assert!(state.doc.lines[1].is_curved());
    }

    #[test]
    fn set_vertex_tangent_independent_gives_each_line_its_own_corner_handle() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        let result = state.apply(Action::SetVertexTangent { point, continuous: false });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert!(state.doc.lines[0].is_curved());
        assert!(state.doc.lines[1].is_curved());
        let h0_near = state.doc.lines[0].bezier.unwrap()[1];
        let h1_near = state.doc.lines[1].bezier.unwrap()[0];
        // Line 0 runs along +x from (0,0) to (10,0): its near-vertex handle sits a third of the
        // way back from (10,0) toward (0,0), independent of line 1's own direction.
        assert!((h0_near.0 - (10.0 - 10.0 / 3.0)).abs() < 1e-3);
        assert!(h0_near.1.abs() < 1e-3);
        // Line 1 runs along +y from (10,0) to (10,10): its near-vertex handle sits a third of
        // the way toward (10,10).
        assert!((h1_near.0 - 10.0).abs() < 1e-3);
        assert!((h1_near.1 - 10.0 / 3.0).abs() < 1e-3);
    }

    #[test]
    fn set_vertex_tangent_rejects_a_vertex_with_only_one_line() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let point = ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::Start };
        let result = state.apply(Action::SetVertexTangent { point, continuous: true });
        assert!(matches!(result, ActionResult::Err(_)));
    }

    #[test]
    fn toggle_curve_mode_on_selected_vertex_curves_then_straightens() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        crate::selection::click_scene_selection(
            &mut state.scene_selection,
            SceneElement::Point(point),
            false,
        );
        assert!(matches!(state.apply(Action::ToggleCurveMode), ActionResult::Ok));
        assert!(state.doc.lines[0].is_curved());
        assert!(state.doc.lines[1].is_curved());
        assert!(matches!(state.apply(Action::ToggleCurveMode), ActionResult::Ok));
        assert!(!state.doc.lines[0].is_curved());
        assert!(!state.doc.lines[1].is_curved());
    }

    #[test]
    fn toggle_tangent_constraint_on_selected_vertex_curves_then_breaks_mirroring() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        crate::selection::click_scene_selection(
            &mut state.scene_selection,
            SceneElement::Point(point),
            false,
        );
        // Starts straight (no bezier at all), so `T` first makes it tangent-continuous.
        assert!(matches!(state.apply(Action::ToggleTangentConstraint), ActionResult::Ok));
        let h0 = state.doc.lines[0].bezier.expect("should be curved now");
        // Toggling again should break the mirroring into independent corner handles.
        assert!(matches!(state.apply(Action::ToggleTangentConstraint), ActionResult::Ok));
        let h0_after = state.doc.lines[0].bezier.expect("should still be curved");
        assert_ne!(h0, h0_after);
    }

    #[test]
    fn toggle_curve_mode_while_drawing_a_line_flips_creating_line_and_persists() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.tool = Tool::Line;
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: String::new(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        assert!(!state.draw_curve_mode);
        assert!(matches!(state.apply(Action::ToggleCurveMode), ActionResult::Ok));
        assert!(state.creating_line.as_ref().unwrap().curve_mode);
        assert!(state.draw_curve_mode);
    }

    #[test]
    fn toggle_curve_mode_persists_for_the_line_tool_when_not_drawing() {
        let mut state = AppState::default();
        state.tool = Tool::Line;
        assert!(!state.draw_curve_mode);
        assert!(matches!(state.apply(Action::ToggleCurveMode), ActionResult::Ok));
        assert!(state.draw_curve_mode);
        assert_eq!(state.line_curve_mode(), Some(true));
    }

    #[test]
    fn toggle_tangent_constraint_persists_for_the_line_tool_when_not_drawing() {
        let mut state = AppState::default();
        state.tool = Tool::Line;
        assert!(state.draw_tangent_constraint);
        assert!(matches!(state.apply(Action::ToggleTangentConstraint), ActionResult::Ok));
        assert!(!state.draw_tangent_constraint);
        assert_eq!(state.line_tangent_constraint(), Some(false));
    }

    #[test]
    fn commit_vertex_treatment_chamfer_truncates_and_bridges_with_a_straight_line() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        let result = state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        // Line 0's End truncated back from (10,0) toward (0,0) by 3mm.
        assert!((state.doc.lines[0].x1 - 7.0).abs() < 1e-3);
        assert!(state.doc.lines[0].y1.abs() < 1e-3);
        // Line 1's Start truncated back from (10,0) toward (10,10) by 3mm.
        assert!((state.doc.lines[1].x0 - 10.0).abs() < 1e-3);
        assert!((state.doc.lines[1].y0 - 3.0).abs() < 1e-3);
        // A new straight bridging line was appended, tied into the loop by two
        // Coincident constraints (so the treated profile stays a closed loop).
        assert_eq!(state.doc.lines.len(), 3);
        assert!(!state.doc.lines[2].is_curved());
        assert_eq!(
            &state.doc.shape_order[state.doc.shape_order.len() - 3..],
            &[ShapeKind::Line, ShapeKind::Constraint, ShapeKind::Constraint]
        );
        // Nests under line 0 (the lower-index trimmed line, #76).
        assert_eq!(state.doc.lines[2].chamfer_fillet_parent, Some(0));
    }

    #[test]
    fn commit_vertex_treatment_fillet_bridges_with_a_curved_line() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        let result = state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Fillet,
            amount: 3.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(state.doc.lines.len(), 3);
        assert!(state.doc.lines[2].is_curved());
    }

    #[test]
    fn commit_vertex_treatment_removes_the_treated_coincident_constraint() {
        let mut state = AppState::default();
        let (sketch, point) = two_coincident_lines_at_a_right_angle(&mut state);
        assert!(state
            .doc
            .constraints
            .iter()
            .any(|c| !c.deleted && c.sketch == sketch));
        state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
        });
        // The old vertex's own Coincident is tombstoned; what's live is exactly the
        // two new constraints tying the bridge line (index 2) into the loop.
        let live: Vec<_> = state
            .doc
            .constraints
            .iter()
            .filter(|c| !c.deleted && c.sketch == sketch)
            .collect();
        assert_eq!(live.len(), 2);
        assert!(live.iter().all(|c| matches!(
            &c.kind,
            ConstraintKind::Coincident { a, b }
                if [a, b].iter().any(|e| matches!(
                    e,
                    ConstraintEntity::Point(ConstraintPoint::LineEndpoint { line: 2, .. })
                ))
        )));
    }

    #[test]
    fn commit_vertex_treatment_rejects_a_vertex_with_only_one_line() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let point = ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::Start };
        let result = state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
        });
        assert!(matches!(result, ActionResult::Err(_)));
        assert_eq!(state.doc.lines.len(), 1);
    }

    #[test]
    fn commit_vertex_treatment_rejects_a_degenerate_corner() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        // Two collinear lines meeting at (10,0): not a real corner.
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 20.0, 0.0));
        state.doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        state.doc.constraints.push(crate::model::Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 0,
                    end: LineEnd::End,
                }),
                b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                }),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        let point = ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::End };
        let result = state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
        });
        assert!(matches!(result, ActionResult::Err(_)));
        assert_eq!(state.doc.lines.len(), 2);
    }

    #[test]
    fn commit_vertex_treatment_rejects_non_positive_amount() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        let result = state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Fillet,
            amount: 0.0,
        });
        assert!(matches!(result, ActionResult::Err(_)));
        assert_eq!(state.doc.lines.len(), 2);
    }

    #[test]
    fn undo_after_commit_vertex_treatment_removes_the_bridging_line() {
        let mut state = AppState::default();
        let (_, point) = two_coincident_lines_at_a_right_angle(&mut state);
        state.apply(Action::CommitVertexTreatment {
            point,
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
        });
        assert_eq!(state.doc.lines.len(), 3);
        let result = state.apply(Action::UndoLast);
        assert!(matches!(result, ActionResult::Ok));
        // Undo is best-effort here: it pops the treatment's whole undo group (the
        // bridging line plus its two loop-closing constraints), but doesn't restore
        // the two truncated lines' original endpoints (documented limitation).
        assert_eq!(state.doc.lines.len(), 2);
        assert!(state
            .doc
            .constraints
            .iter()
            .filter(|c| !c.deleted)
            .all(|c| !matches!(
                &c.kind,
                ConstraintKind::Coincident { a, b }
                    if [a, b].iter().any(|e| matches!(
                        e,
                        ConstraintEntity::Point(ConstraintPoint::LineEndpoint { line: 2, .. })
                    ))
            )));
    }

    fn box_extrusion_state() -> AppState {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        let rect_lines = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            0.0,
            0.0,
            10.0,
            10.0,
            [false; 4],
        );
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 5.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
        });
        state
    }

    /// Two separate extruded box bodies side by side (overlapping when `overlap`).
    fn two_box_state(overlap: bool) -> AppState {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        let a = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            0.0,
            0.0,
            10.0,
            10.0,
            [false; 4],
        );
        let x1 = if overlap { 5.0 } else { 20.0 };
        let b = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            x1,
            0.0,
            x1 + 10.0,
            10.0,
            [false; 4],
        );
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(a.to_vec())],
            distance: 5.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
        });
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(b.to_vec())],
            distance: 5.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
        });
        assert_eq!(state.doc.bodies.len(), 2);
        state
    }

    /// Combine tool: committing shadows the inputs, creates the operation element and its
    /// output bodies, and one undo restores everything.
    #[test]
    fn boolean_combine_creates_outputs_and_shadows_inputs() {
        let mut state = two_box_state(true);
        let result = state.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Combine,
            a: vec![0, 1],
            b: Vec::new(),
            keep_b: false,
        });
        assert!(matches!(result, ActionResult::Ok));
        assert_eq!(state.doc.boolean_ops.len(), 1);
        let op = &state.doc.boolean_ops[0];
        assert!(!op.outputs.is_empty());
        for &out in &op.outputs {
            assert!(matches!(
                state.doc.bodies[out].source,
                crate::model::BodySource::Boolean { op: 0, .. }
            ));
            assert!(!state.doc.bodies[out].shadow);
        }
        assert!(state.doc.bodies[0].shadow, "input A should be a shadow body");
        assert!(state.doc.bodies[1].shadow, "input B should be a shadow body");

        state.apply(Action::UndoLast);
        assert!(state.doc.boolean_ops.is_empty());
        assert_eq!(state.doc.bodies.len(), 2);
        assert!(!state.doc.bodies[0].shadow);
        assert!(!state.doc.bodies[1].shadow);
    }

    /// Cut with keep-B leaves the B-side inputs as real bodies.
    #[test]
    fn boolean_cut_keep_b_leaves_b_real() {
        let mut state = two_box_state(true);
        let result = state.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![0],
            b: vec![1],
            keep_b: true,
        });
        assert!(matches!(result, ActionResult::Ok));
        assert!(state.doc.bodies[0].shadow);
        assert!(!state.doc.bodies[1].shadow, "keep_b must leave B real");
    }

    /// Validation: combine needs two inputs; consumed (shadow) inputs are rejected; the
    /// two-sided ops need a B side.
    #[test]
    fn boolean_validation_rejects_bad_inputs() {
        let mut state = two_box_state(true);
        assert!(matches!(
            state.apply(Action::CreateBooleanOperation {
                kind: crate::model::BooleanOpKind::Combine,
                a: vec![0],
                b: Vec::new(),
                keep_b: false,
            }),
            ActionResult::Err(_)
        ));
        assert!(matches!(
            state.apply(Action::CreateBooleanOperation {
                kind: crate::model::BooleanOpKind::Cut,
                a: vec![0],
                b: Vec::new(),
                keep_b: false,
            }),
            ActionResult::Err(_)
        ));
        // Consume both, then try to reuse a shadow input.
        state.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Combine,
            a: vec![0, 1],
            b: Vec::new(),
            keep_b: false,
        });
        let out = state.doc.boolean_ops[0].outputs[0];
        assert!(matches!(
            state.apply(Action::CreateBooleanOperation {
                kind: crate::model::BooleanOpKind::Cut,
                a: vec![out],
                b: vec![0],
                keep_b: false,
            }),
            ActionResult::Err(_)
        ));
    }

    /// Editing re-points the inputs and adjusts shadow flags.
    #[test]
    fn boolean_edit_repoints_inputs() {
        let mut state = two_box_state(true);
        state.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![0],
            b: vec![1],
            keep_b: false,
        });
        assert!(state.doc.bodies[1].shadow);
        let result = state.apply(Action::EditBooleanOperation {
            op: 0,
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![0],
            b: vec![1],
            keep_b: true,
        });
        assert!(matches!(result, ActionResult::Ok));
        assert!(state.doc.bodies[0].shadow);
        assert!(!state.doc.bodies[1].shadow, "edit to keep_b must release B");
        assert_eq!(state.doc.boolean_ops[0].keep_b, true);
    }

    /// Deleting the operation element tombstones its outputs and releases its inputs.
    #[test]
    fn boolean_delete_releases_inputs() {
        let mut state = two_box_state(true);
        state.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Intersect,
            a: vec![0],
            b: vec![1],
            keep_b: false,
        });
        let out = state.doc.boolean_ops[0].outputs[0];
        assert!(crate::document_lifecycle::tombstone_element(
            &mut state.doc,
            SceneElement::BooleanOp(0),
        ));
        assert!(state.doc.boolean_ops[0].deleted);
        assert!(state.doc.bodies[out].deleted);
        assert!(!state.doc.bodies[0].shadow);
        assert!(!state.doc.bodies[1].shadow);
    }

    /// Move tool: committing shadows the inputs, creates the operation + one moved output
    /// per input, geometry lands offset, and one undo restores everything.
    #[test]
    fn move_commit_creates_outputs_and_shadows_inputs() {
        let mut state = two_box_state(false);
        let result = state.apply(Action::CreateMoveOperation {
            targets: vec![0, 1],
            plane_targets: vec![],
            image_targets: vec![],
            tx: "25".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok));
        assert_eq!(state.doc.move_ops.len(), 1);
        let op = state.doc.move_ops[0].clone();
        assert_eq!(op.outputs.len(), 2);
        assert!(state.doc.bodies[0].shadow);
        assert!(state.doc.bodies[1].shadow);
        // The moved copy sits 25mm along +X of its source.
        let src = crate::extrude::body_solid_mesh(&state.doc, 0).unwrap();
        let out = crate::extrude::body_solid_mesh(&state.doc, op.outputs[0]).unwrap();
        let min_x = |m: &crate::extrude::SolidMesh| {
            m.triangles
                .iter()
                .flat_map(|t| t.iter())
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min)
        };
        assert!((min_x(&out) - min_x(&src) - 25.0).abs() < 1e-3);

        state.apply(Action::UndoLast);
        assert!(state.doc.move_ops.is_empty());
        assert_eq!(state.doc.bodies.len(), 2);
        assert!(!state.doc.bodies[0].shadow);
    }

    /// Editing a move re-points targets and grows/shrinks outputs to match.
    #[test]
    fn move_edit_repoints_and_resizes_outputs() {
        let mut state = two_box_state(false);
        state.apply(Action::CreateMoveOperation {
            targets: vec![0],
            plane_targets: vec![],
            image_targets: vec![],
            tx: "5".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
        });
        assert_eq!(state.doc.move_ops[0].outputs.len(), 1);
        let result = state.apply(Action::EditMoveOperation {
            op: 0,
            targets: vec![0, 1],
            plane_targets: vec![],
            image_targets: vec![],
            tx: "5".to_string(),
            ty: "2".to_string(),
            tz: String::new(),
            axis: Some(crate::model::RevolveAxis::Z),
            angle: "45".to_string(),
        });
        assert!(matches!(result, ActionResult::Ok));
        assert_eq!(state.doc.move_ops[0].outputs.len(), 2);
        assert!(state.doc.bodies[1].shadow);
    }

    /// A move driven by a parameter follows parameter edits at rebuild time.
    #[test]
    fn move_expression_is_parametric() {
        let mut state = two_box_state(false);
        state.apply(Action::AddParameter {
            name: "gap".to_string(),
            expression: "30".to_string(),
        });
        let result = state.apply(Action::CreateMoveOperation {
            targets: vec![0],
            plane_targets: vec![],
            image_targets: vec![],
            tx: "gap".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok));
        let op = state.doc.move_ops[0].clone();
        let m = crate::extrude::move_op_transform(&state.doc, &op).unwrap();
        assert!((m.w_axis.x - 30.0).abs() < 1e-4);
    }

    /// Repeat tool: count × gap produces N-1 output bodies at increasing offsets; undo
    /// removes everything; the original stays real (never shadowed).
    #[test]
    fn repeat_count_gap_creates_offset_instances() {
        let mut state = two_box_state(false);
        let result = state.apply(Action::CreateRepeatOperation {
            targets: vec![0],
            plane_targets: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "5".to_string(),
            length: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok));
        let op = state.doc.repeat_ops[0].clone();
        assert_eq!(op.outputs.len(), 2, "3 instances = original + 2 outputs");
        assert!(!state.doc.bodies[0].shadow, "repeat originals stay real");
        // Box extent along X is 10, so step = 15: instance offsets 15 and 30.
        let min_x = |bi: usize| {
            crate::extrude::body_solid_mesh(&state.doc, bi)
                .unwrap()
                .triangles
                .iter()
                .flat_map(|t| t.iter())
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min)
        };
        let base = min_x(0);
        assert!((min_x(op.outputs[0]) - base - 15.0).abs() < 1e-3);
        assert!((min_x(op.outputs[1]) - base - 30.0).abs() < 1e-3);

        state.apply(Action::UndoLast);
        assert!(state.doc.repeat_ops.is_empty());
        assert_eq!(state.doc.bodies.len(), 2);
    }

    /// The stud-spacing mode lands the last instance at the end of L with pitch <= D.
    #[test]
    fn repeat_fill_max_pitch_lands_on_the_end() {
        let mut state = two_box_state(false);
        // Box extent 10 along X; wall length 100 -> span 90; max pitch 40 -> 4 instances
        // at even pitch 30.
        let result = state.apply(Action::CreateRepeatOperation {
            targets: vec![0],
            plane_targets: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::FillMaxPitch,
            count: String::new(),
            spacing: "40".to_string(),
            length: "100".to_string(),
        });
        assert!(matches!(result, ActionResult::Ok));
        let op = state.doc.repeat_ops[0].clone();
        assert_eq!(op.outputs.len(), 3);
        let offsets = crate::extrude::repeat_offsets(&state.doc, &op).unwrap();
        assert_eq!(offsets.len(), 3);
        assert!((offsets[2] - 90.0).abs() < 1e-3, "last instance starts at L - extent");
        assert!(offsets[0] <= 40.0 + 1e-3, "pitch stays within the max");
    }

    /// Editing a repeat re-evaluates the instance count and resizes the outputs.
    #[test]
    fn repeat_edit_resizes_outputs() {
        let mut state = two_box_state(false);
        state.apply(Action::CreateRepeatOperation {
            targets: vec![0],
            plane_targets: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "2".to_string(),
            spacing: "5".to_string(),
            length: String::new(),
        });
        assert_eq!(state.doc.repeat_ops[0].outputs.len(), 1);
        let result = state.apply(Action::EditRepeatOperation {
            op: 0,
            targets: vec![0],
            plane_targets: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "5".to_string(),
            spacing: "5".to_string(),
            length: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok));
        assert_eq!(state.doc.repeat_ops[0].outputs.len(), 4);
    }

    /// A parameter-driven count follows parameter edits.
    #[test]
    fn repeat_count_is_parametric() {
        let mut state = two_box_state(false);
        state.apply(Action::AddParameter {
            name: "n".to_string(),
            expression: "4".to_string(),
        });
        state.apply(Action::CreateRepeatOperation {
            targets: vec![0],
            plane_targets: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "n".to_string(),
            spacing: "5".to_string(),
            length: String::new(),
        });
        let op = state.doc.repeat_ops[0].clone();
        assert_eq!(op.outputs.len(), 3);
        let offsets = crate::extrude::repeat_offsets(&state.doc, &op).unwrap();
        assert_eq!(offsets.len(), 3);
    }

    /// #221: repeating a construction plane ×3 along X emits two offset instance planes at the
    /// gap and twice the gap, each copying the source frame and grouped under the op.
    #[test]
    fn repeat_copies_a_construction_plane_along_the_axis() {
        let mut state = two_box_state(false);
        // The default document ships one construction plane (the XY ground) at the origin.
        let base = state.doc.construction_planes[0].origin;
        let normal = state.doc.construction_planes[0].normal;
        let result = state.apply(Action::CreateRepeatOperation {
            targets: Vec::new(),
            plane_targets: vec![0],
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok));
        let op = state.doc.repeat_ops[0].clone();
        assert!(op.outputs.is_empty(), "a plane-only repeat makes no body copies");
        assert_eq!(op.plane_outputs.len(), 2, "3 instances = original + 2 copies");
        // Planes are zero-thickness, so the step is exactly the gap.
        let p1 = state.doc.construction_planes[op.plane_outputs[0]].origin;
        let p2 = state.doc.construction_planes[op.plane_outputs[1]].origin;
        assert!((p1.x - base.x - 10.0).abs() < 1e-3);
        assert!((p2.x - base.x - 20.0).abs() < 1e-3);
        // Copies carry the back-reference and inherit the source orientation.
        let inst = state.doc.construction_planes[op.plane_outputs[0]]
            .repeat_instance
            .expect("instance carries a back-reference");
        assert_eq!((inst.op, inst.target, inst.instance), (0, 0, 1));
        assert!((state.doc.construction_planes[op.plane_outputs[0]].normal - normal).length() < 1e-4);

        // Undo removes the op and its instance planes.
        state.apply(Action::UndoLast);
        assert!(state.doc.repeat_ops.is_empty());
        assert_eq!(state.doc.construction_planes.len(), 1);
    }

    /// #221: a plane instance follows its source plane when the source is moved (the instance's
    /// frame is derived from the source's current frame at recompute).
    #[test]
    fn repeat_plane_instance_follows_a_moved_source() {
        let mut state = two_box_state(false);
        state.apply(Action::CreateRepeatOperation {
            targets: Vec::new(),
            plane_targets: vec![0],
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "2".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
        });
        let inst_idx = state.doc.repeat_ops[0].plane_outputs[0];
        assert!((state.doc.construction_planes[inst_idx].origin.x - 10.0).abs() < 1e-3);
        // Move the source plane +5 along X; the instance should sit at 5 + 10 = 15.
        let result = state.apply(Action::CreateMoveOperation {
            targets: Vec::new(),
            plane_targets: vec![0],
            image_targets: Vec::new(),
            tx: "5".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok));
        assert!((state.doc.construction_planes[0].origin.x - 5.0).abs() < 1e-3);
        assert!(
            (state.doc.construction_planes[inst_idx].origin.x - 15.0).abs() < 1e-3,
            "instance follows the moved source and keeps its own offset on top"
        );
    }

    /// #221: editing a plane-only repeat's count resizes the instance planes and re-points them.
    #[test]
    fn repeat_edit_resizes_plane_instances() {
        let mut state = two_box_state(false);
        state.apply(Action::CreateRepeatOperation {
            targets: Vec::new(),
            plane_targets: vec![0],
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "2".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
        });
        assert_eq!(state.doc.repeat_ops[0].plane_outputs.len(), 1);
        let result = state.apply(Action::EditRepeatOperation {
            op: 0,
            targets: Vec::new(),
            plane_targets: vec![0],
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "4".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
        });
        assert!(matches!(result, ActionResult::Ok));
        let op = state.doc.repeat_ops[0].clone();
        assert_eq!(op.plane_outputs.len(), 3, "4 instances = original + 3 copies");
        assert!((state.doc.construction_planes[op.plane_outputs[2]].origin.x - 30.0).abs() < 1e-3);
    }

    /// #189: selecting a point and the origin, then applying Coincident, pins the point to the
    /// origin — the constraint-tool flow the user asked for (not just snapping).
    #[test]
    fn selecting_a_point_and_the_origin_constrains_coincident() {
        use crate::geometric_constraints::GeometricConstraintType;
        use crate::model::{ConstraintPoint, LineEnd};

        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 5.0, 5.0, 12.0, 8.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.refresh_document_health();

        state.apply(Action::ClickSceneElement {
            element: SceneElement::Point(ConstraintPoint::LineEndpoint { line: 0, end: LineEnd::Start }),
            additive: false,
        });
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Origin,
            additive: true,
        });
        let result = state.apply(Action::AddGeometricConstraint(GeometricConstraintType::Coincident));
        assert!(matches!(result, ActionResult::Ok), "{}", state.status);
        assert!(
            state.doc.lines[0].x0.abs() < 1e-3 && state.doc.lines[0].y0.abs() < 1e-3,
            "start should be pinned to the origin, got ({}, {})",
            state.doc.lines[0].x0,
            state.doc.lines[0].y0
        );
    }

    /// #198: with a circle drawn on a body face, selecting the circle's center and one of the
    /// face's own edges resolves to a point-to-line distance dimension — the thing the user
    /// couldn't do. Exercises the selection→constraint logic end to end (the viewport pick and
    /// this share `dimension_edit_from_selection`).
    #[test]
    fn circle_center_and_face_edge_resolve_to_a_point_line_distance() {
        use crate::model::{ConstraintLine, ConstraintPoint, DimensionTarget, DistanceTarget};

        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        let rect = crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 20.0, 20.0, [false; 4]);
        let profile = ExtrudeFace::Polygon(rect.to_vec());
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![profile.clone()],
            distance: 10.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
        });
        let cap = FaceId::ExtrudeCap { extrusion: 0, profile, top: true };
        state.apply(Action::BeginSketch { face: cap.clone(), viewport: None });
        let cap_sketch = state.sketch_session.unwrap().sketch;
        state
            .doc
            .circles
            .push(crate::model::Circle::from_local_center_radius(cap_sketch, 5.0, 5.0, 3.0, 0.0));
        let ci = state.doc.circles.len() - 1;
        state.doc.shape_order.push(ShapeKind::Circle);
        state.refresh_document_health();

        state.apply(Action::ClickSceneElement {
            element: SceneElement::Point(ConstraintPoint::CircleCenter(ci)),
            additive: false,
        });
        state.apply(Action::ClickSceneElement {
            element: SceneElement::FaceEdge(ConstraintLine::FaceEdge { face: cap, index: 0 }),
            additive: true,
        });

        let target = crate::constraints::dimension_edit_from_selection(
            &state.doc,
            cap_sketch,
            &state.scene_selection,
        );
        assert!(
            matches!(
                target,
                Some(DimensionTarget::Distance(DistanceTarget::PointLineDistance { .. }))
            ),
            "circle center + face edge should dimension as a point-line distance, got {target:?}"
        );

        // And the viewport pick actually resolves the circle center: aim a top-down camera at
        // the cap, project the center, and click there.
        let center_world = crate::face::circle_world_center(&state.doc, &state.doc.circles[ci])
            .expect("circle center world position");
        let mut cam = crate::camera::Camera::default();
        let (yaw, pitch) = crate::camera::StandardView::Top.yaw_pitch();
        cam.yaw = yaw;
        cam.pitch = pitch;
        cam.target = center_world;
        cam.distance = 120.0;
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        let center_px = project(center_world).expect("center projects into view");
        let picked = crate::construction::nearest_sketch_point_in_sketch(
            center_px,
            &project,
            &state.doc,
            cap_sketch,
        );
        assert!(
            matches!(
                picked,
                Some((crate::model::ConstraintPoint::CircleCenter(c), _)) if c == ci
            ),
            "clicking the circle's center on a body-face sketch should pick it, got {picked:?}"
        );

        // #199: the face's own edges are pickable in the same sketch — click an edge midpoint.
        let loop_ = crate::extrude::face_boundary_loop_world(
            &state.doc,
            &FaceId::ExtrudeCap { extrusion: 0, profile: ExtrudeFace::Polygon(rect.to_vec()), top: true },
        )
        .expect("cap boundary loop");
        let edge_mid = (loop_[0] + loop_[1]) * 0.5;
        let edge_px = project(edge_mid).expect("edge midpoint projects into view");
        let edge_pick =
            crate::construction::nearest_sketch_line_in_sketch(edge_px, &project, &state.doc, cap_sketch);
        assert!(
            matches!(
                edge_pick,
                Some((crate::model::ConstraintLine::FaceEdge { index: 0, .. }, _))
            ),
            "clicking a face edge on a body-face sketch should pick it, got {edge_pick:?}"
        );
    }

    /// #201: committing a repeat whose count is typed as `name = expr` defines the parameter
    /// and stores the bare name, so the repeat stays parameter-driven.
    #[test]
    fn repeat_input_defines_inline_parameter() {
        let mut state = two_box_state(false);
        state.apply(Action::SetTool(Tool::Repeat));
        {
            let cr = state.creating_repeat.as_mut().unwrap();
            cr.targets = vec![0];
            cr.count = "n = 4".to_string();
            cr.spacing = "10".to_string();
        }
        assert!(matches!(state.apply(Action::CommitRepeat), ActionResult::Ok), "{}", state.status);
        assert!(state.doc.parameters.iter().any(|p| !p.deleted && p.name == "n"));
        assert_eq!(state.doc.repeat_ops[0].count, "n", "the stored count should be the bare name");
        assert_eq!(state.doc.repeat_ops[0].outputs.len(), 3, "n = 4 → 3 extra instances");
    }

    /// #201: a move offset typed as `name = expr` defines the parameter and stays parametric.
    #[test]
    fn move_input_defines_inline_parameter() {
        let mut state = two_box_state(false);
        state.apply(Action::SetTool(Tool::Move));
        {
            let cm = state.creating_move.as_mut().unwrap();
            cm.targets = vec![0];
            cm.tx = "dx = 25mm".to_string();
        }
        assert!(matches!(state.apply(Action::CommitMove), ActionResult::Ok), "{}", state.status);
        assert!(state.doc.parameters.iter().any(|p| !p.deleted && p.name == "dx"));
        assert_eq!(state.doc.move_ops[0].tx, "dx", "the stored offset should be the bare name");
    }

    /// Adds an XY-parallel construction plane at `z` and returns its `FaceId`.
    fn add_offset_xy_plane(state: &mut AppState, z: f32) -> FaceId {
        let plane = plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Face {
                    origin: Vec3::ZERO,
                    normal: Vec3::Z,
                    label: "Ground".to_string(),
                },
                z,
                0.0,
            ),
            ConstructionPlaneParent::Root,
        );
        state.doc.construction_planes.push(plane);
        FaceId::ConstructionPlane(state.doc.construction_planes.len() - 1)
    }

    /// Slice: a plane through the middle of a box splits it into two fragment bodies, the
    /// input becomes a shadow body, and one undo restores everything.
    #[cfg(feature = "occt")]
    #[test]
    fn slice_splits_a_box_into_two_fragments() {
        let mut state = two_box_state(false);
        // Box 0 spans z 0..5; a plane at z=2.5 halves it.
        let cutter = add_offset_xy_plane(&mut state, 2.5);
        let bodies_before = state.doc.bodies.len();
        let result = state.apply(Action::CreateSliceOperation {
            targets: vec![0],
            cutters: vec![cutter],
            extend_infinite: true,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(state.doc.slice_ops.len(), 1);
        let op = &state.doc.slice_ops[0];
        assert_eq!(op.outputs.len(), 2, "a mid-plane cut yields two fragments");
        for &out in &op.outputs {
            assert!(matches!(
                state.doc.bodies[out].source,
                crate::model::BodySource::Sliced { op: 0, .. }
            ));
            assert!(!state.doc.bodies[out].shadow);
            assert!(
                crate::extrude::body_solid_mesh(&state.doc, out).is_some(),
                "fragment {out} should have a kernel mesh"
            );
        }
        assert!(state.doc.bodies[0].shadow, "the sliced input becomes a shadow body");
        assert_eq!(state.doc.bodies.len(), bodies_before + 2);

        state.apply(Action::UndoLast);
        assert!(state.doc.slice_ops.is_empty());
        assert_eq!(state.doc.bodies.len(), bodies_before);
        assert!(!state.doc.bodies[0].shadow, "undo restores the input body");
    }

    /// A cutter that misses the body leaves it whole (one fragment).
    #[cfg(feature = "occt")]
    #[test]
    fn slice_with_a_missing_cutter_keeps_the_body_whole() {
        let mut state = two_box_state(false);
        // Box 0 spans z 0..5; a plane at z=20 is entirely above it.
        let cutter = add_offset_xy_plane(&mut state, 20.0);
        state.apply(Action::CreateSliceOperation {
            targets: vec![0],
            cutters: vec![cutter],
            extend_infinite: true,
        });
        assert_eq!(state.doc.slice_ops[0].outputs.len(), 1);
    }

    /// Editing a slice re-points its cutters and resizes the fragment list.
    #[cfg(feature = "occt")]
    #[test]
    fn slice_edit_resizes_outputs() {
        let mut state = two_box_state(false);
        let miss = add_offset_xy_plane(&mut state, 20.0);
        let hit = add_offset_xy_plane(&mut state, 2.5);
        state.apply(Action::CreateSliceOperation {
            targets: vec![0],
            cutters: vec![miss],
            extend_infinite: true,
        });
        assert_eq!(state.doc.slice_ops[0].outputs.len(), 1);
        let result = state.apply(Action::EditSliceOperation {
            op: 0,
            targets: vec![0],
            cutters: vec![hit],
            extend_infinite: true,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(
            state.doc.slice_ops[0].outputs.len(),
            2,
            "the mid-plane cutter now yields two fragments"
        );
        assert!(state.doc.bodies[0].shadow);
    }

    /// Slice rejects an empty cutter set and a shadow input.
    #[test]
    fn slice_validation_rejects_no_cutters_and_shadow_inputs() {
        let mut state = two_box_state(false);
        let no_cutters = state.apply(Action::CreateSliceOperation {
            targets: vec![0],
            cutters: Vec::new(),
            extend_infinite: true,
        });
        assert!(matches!(no_cutters, ActionResult::Err(_)));
        // Shadow body 0 via a move op, then it can't be sliced.
        state.doc.bodies[0].shadow = true;
        let cutter = add_offset_xy_plane(&mut state, 2.5);
        let shadowed = state.apply(Action::CreateSliceOperation {
            targets: vec![0],
            cutters: vec![cutter],
            extend_infinite: true,
        });
        assert!(matches!(shadowed, ActionResult::Err(_)));
    }

    /// Combining two *disjoint* boxes keeps them as one operation with (kernel builds) two
    /// output solids — and the outputs render as real meshes.
    #[cfg(feature = "occt")]
    #[test]
    fn boolean_combine_disjoint_produces_two_outputs() {
        let mut state = two_box_state(false);
        state.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Combine,
            a: vec![0, 1],
            b: Vec::new(),
            keep_b: false,
        });
        let op = &state.doc.boolean_ops[0];
        assert_eq!(op.outputs.len(), 2, "disjoint union should split into two bodies");
        for &out in &op.outputs {
            assert!(
                crate::extrude::body_solid_mesh(&state.doc, out).is_some(),
                "output body {out} should have a mesh"
            );
        }
    }

    /// Cutting an overlapping box out of another produces a real, smaller solid.
    #[cfg(feature = "occt")]
    #[test]
    fn boolean_cut_produces_kernel_mesh() {
        let mut state = two_box_state(true);
        state.apply(Action::CreateBooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![0],
            b: vec![1],
            keep_b: false,
        });
        let op = &state.doc.boolean_ops[0];
        assert_eq!(op.outputs.len(), 1);
        let mesh = crate::extrude::body_solid_mesh(&state.doc, op.outputs[0]);
        assert!(mesh.is_some_and(|m| !m.triangles.is_empty()));
    }

    /// #122: pushing/pulling a bare side wall (no separate sketch) creates an implicit
    /// sketch on that exact face and starts extruding from it.
    #[test]
    fn extrude_body_face_pushes_a_box_side_wall_directly() {
        let mut state = box_extrusion_state();
        let profile = state.doc.extrusions[0].faces[0].clone();
        let sketches_before = state.doc.sketches.len();
        let face_id = FaceId::ExtrudeSide {
            extrusion: 0,
            profile,
            edge: 0,
        };

        let result = state.apply(Action::ExtrudeBodyFace { face_id });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(
            state.doc.sketches.len(),
            sketches_before + 1,
            "should create exactly one implicit sketch"
        );
        let ce = state
            .creating_extrusion
            .as_ref()
            .expect("should start a fresh in-progress extrusion");
        assert_eq!(ce.faces.len(), 1);
        assert!(matches!(ce.faces[0], ExtrudeFace::Polygon(_)));

        state.apply(Action::CommitExtrusion);
        assert_eq!(state.doc.extrusions.len(), 2, "should commit as a second extrusion");
        // Sketching on an existing body's face merges into that body by default (#32) — the
        // push extends the original box rather than creating a separate one, so the merged
        // mesh's bounds are the union of the original box (y: 0..10) and the new slab
        // pushed out from the y=0 wall by the default 10mm (y: -10..0).
        let merged = crate::extrude::body_solid_mesh(&state.doc, 0).unwrap();
        let (min, max) = merged.bounds().unwrap();
        assert!((max.x - min.x - 10.0).abs() < 1e-3, "box width, got {min:?}..{max:?}");
        assert!((max.z - min.z - 5.0).abs() < 1e-3, "box height, got {min:?}..{max:?}");
        assert!((max.y - min.y - 20.0).abs() < 1e-3, "box + push, got {min:?}..{max:?}");
        assert!(min.y < -9.0, "push should extend past the original box, got {min:?}");
    }

    /// #122: a circular cap gets a real `Circle` in the implicit sketch, not a tessellated
    /// polygon approximation.
    #[test]
    fn extrude_body_face_on_a_circular_cap_creates_a_real_circle() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.circles.push(crate::model::Circle::from_local_center_radius(
            sketch, 0.0, 0.0, 6.0, 0.0,
        ));
        let profile = ExtrudeFace::Circle(0);
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![profile.clone()],
            distance: 4.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
        });
        let circles_before = state.doc.circles.len();
        let face_id = FaceId::ExtrudeCap {
            extrusion: 0,
            profile,
            top: true,
        };

        let result = state.apply(Action::ExtrudeBodyFace { face_id });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(state.doc.circles.len(), circles_before + 1);
        let new_circle = state.doc.circles.last().unwrap();
        assert!((new_circle.r - 6.0).abs() < 1e-3, "should mirror the source radius exactly");
        let ce = state.creating_extrusion.as_ref().unwrap();
        assert!(matches!(ce.faces[0], ExtrudeFace::Circle(_)));
    }

    /// #141: dragging a body-face extrusion backward (negative distance, into the body it sits
    /// on) auto-switches it to a cut; pulling forward again reverts to adding.
    #[test]
    #[cfg(feature = "occt")]
    fn extruding_backward_into_body_auto_switches_to_cut() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        let rect_lines =
            crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        let profile = ExtrudeFace::Polygon(rect_lines.to_vec());
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![profile.clone()],
            distance: 5.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
        });
        state.apply(Action::ExtrudeBodyFace {
            face_id: FaceId::ExtrudeCap { extrusion: 0, profile, top: true },
        });
        let bi = state.creating_extrusion.as_ref().unwrap().merge_candidate.unwrap();
        // Default forward drag adds to the body.
        assert_eq!(
            state.creating_extrusion.as_ref().unwrap().body_mode,
            ExtrudeBodyMode::MergeInto(bi)
        );
        // Backward drag → cut.
        state.apply(Action::SetExtrudeDistance { distance: -3.0 });
        assert_eq!(
            state.creating_extrusion.as_ref().unwrap().body_mode,
            ExtrudeBodyMode::Cut(bi)
        );
        // Forward again → back to adding.
        state.apply(Action::SetExtrudeDistance { distance: 3.0 });
        assert_eq!(
            state.creating_extrusion.as_ref().unwrap().body_mode,
            ExtrudeBodyMode::MergeInto(bi)
        );
    }

    /// #122: only a real body face (cap/side) can be extruded this way — anything else is a
    /// clear error, not a silent no-op.
    #[test]
    fn extrude_body_face_rejects_a_construction_plane() {
        let mut state = AppState::default();
        let result = state.apply(Action::ExtrudeBodyFace {
            face_id: FaceId::ConstructionPlane(0),
        });
        assert!(matches!(result, ActionResult::Err(_)), "{result:?}");
    }

    /// #140: pressing Y with a body edge selected projects it into the open sketch as an
    /// associative construction-style line, and editing the source geometry re-resolves it.
    #[test]
    fn project_selection_creates_associative_line() {
        use crate::hierarchy::{quantize_body_point, SceneElement};

        let mut state = box_extrusion_state();
        state.apply(Action::ExitSketch);
        // Select a top-cap edge of the 10x10x5 box.
        let treatable = crate::extrude::treatable_edges(&state.doc);
        let (_, _, a, b) = treatable
            .iter()
            .find(|(_, edge, _, _)| {
                matches!(edge, crate::model::ExtrusionEdgeRef::Cap { top: true, .. })
            })
            .expect("box has top-cap edges")
            .clone();
        state.apply(Action::ClickSceneElement {
            element: SceneElement::BodyEdge {
                body: 0,
                a: quantize_body_point(a),
                b: quantize_body_point(b),
            },
            additive: false,
        });

        // Open a sketch on the ground plane and project.
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let lines_before = state.doc.lines.len();
        let result = state.apply(Action::ProjectSelection);
        assert!(matches!(result, ActionResult::Ok), "{result:?}: {}", state.status);
        assert_eq!(state.doc.lines.len(), lines_before + 1);
        let line = state.doc.lines.last().unwrap().clone();
        assert!(line.construction, "projections render construction-style");
        assert!(line.projection.is_some(), "projection keeps its source link");
        // The top edge at z=5 projects straight down onto the ground plane: local coords
        // equal the source edge's x/y.
        let got = [(line.x0, line.y0), (line.x1, line.y1)];
        for world in [a, b] {
            assert!(
                got.iter().any(|p| {
                    (p.0 - world.x).abs() < 1e-3 && (p.1 - world.y).abs() < 1e-3
                }),
                "some projected endpoint should match source {world:?}, got {got:?}"
            );
        }

        // Associativity: re-resolving after a source change follows the edge. The cap edge
        // is keyed by its endpoints; a geometry recompute re-projects it.
        let li = state.doc.lines.len() - 1;
        state.doc.lines[li].x0 = 999.0; // knock it out of place
        crate::parameters::recompute_document_geometry(&mut state.doc).unwrap();
        let x0 = state.doc.lines[li].x0;
        assert!(
            (x0 - a.x).abs() < 1e-3 || (x0 - b.x).abs() < 1e-3,
            "refresh must snap the projected line back to its source, got {x0}"
        );
    }

    /// #171: calibrating with a reference segment rescales the image uniformly about the
    /// segment midpoint and stores the calibration for re-editing.
    /// Guided calibration (#163): Begin → two viewport points → CalibrateImage commit;
    /// the in-progress state gates each step and clears on commit and on cancel.
    #[test]
    fn guided_calibration_flow_places_points_then_commits() {
        let mut state = AppState::default();
        state.doc.tracing_images.push(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "grid".to_string(),
            plane: 0,
            origin: (-50.0, -30.0),
            width_mm: 100.0,
            height_mm: 60.0,
            name: None,
            deleted: false,
            calibration: None,
            base_origin: None,
        });
        // Out-of-range image is rejected; a point without a session is rejected.
        assert!(matches!(
            state.apply(Action::BeginImageCalibration { image: 3 }),
            ActionResult::Err(_)
        ));
        assert!(matches!(
            state.apply(Action::AddCalibrationPoint { x: 0.0, y: 0.0 }),
            ActionResult::Err(_)
        ));

        state.tool = Tool::Line;
        assert!(matches!(
            state.apply(Action::BeginImageCalibration { image: 0 }),
            ActionResult::Ok
        ));
        // Point placement takes over clicks, so the tool falls back to Select.
        assert_eq!(state.tool, Tool::Select);
        state.apply(Action::AddCalibrationPoint { x: -20.0, y: 0.0 });
        state.apply(Action::AddCalibrationPoint { x: 20.0, y: 0.0 });
        let cal = state.creating_calibration.as_ref().expect("still in progress");
        assert_eq!(cal.points, vec![(-20.0, 0.0), (20.0, 0.0)]);
        // A third point is refused.
        assert!(matches!(
            state.apply(Action::AddCalibrationPoint { x: 1.0, y: 1.0 }),
            ActionResult::Err(_)
        ));

        // Committing rescales and ends the session.
        let result = state.apply(Action::CalibrateImage {
            image: 0,
            a: (-20.0, 0.0),
            b: (20.0, 0.0),
            length: 80.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert!(state.creating_calibration.is_none());
        assert!((state.doc.tracing_images[0].width_mm - 200.0).abs() < 1e-3);

        // Esc cancels a fresh session.
        state.apply(Action::BeginImageCalibration { image: 0 });
        state.apply(Action::CancelOperation);
        assert!(state.creating_calibration.is_none());
    }

    #[test]
    fn calibrate_image_rescales_about_the_reference_segment() {
        let mut state = AppState::default();
        state.doc.tracing_images.push(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "grid".to_string(),
            plane: 0,
            origin: (-50.0, -30.0),
            width_mm: 100.0,
            height_mm: 60.0,
            name: None,
            deleted: false,
            calibration: None,
            base_origin: None,
        });
        // A feature spanning 40 mm on screen is declared to really be 80 mm → 2x.
        let result = state.apply(Action::CalibrateImage {
            image: 0,
            a: (-20.0, 0.0),
            b: (20.0, 0.0),
            length: 80.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        let img = &state.doc.tracing_images[0];
        assert!((img.width_mm - 200.0).abs() < 1e-3);
        assert!((img.height_mm - 120.0).abs() < 1e-3);
        // Scaled about the segment midpoint (0, 0): origin doubles away from it.
        assert!((img.origin.0 + 100.0).abs() < 1e-3 && (img.origin.1 + 60.0).abs() < 1e-3);
        let cal = img.calibration.expect("calibration stored");
        assert!((cal.length_mm - 80.0).abs() < 1e-3);
        // UV of the reference points on the pre-scale quad: x -20 → u 0.3, x 20 → u 0.7.
        assert!((cal.u0 - 0.3).abs() < 1e-3 && (cal.u1 - 0.7).abs() < 1e-3);

        // Degenerate inputs error.
        let r = state.apply(Action::CalibrateImage { image: 0, a: (0.0, 0.0), b: (0.0, 0.0), length: 10.0 });
        assert!(matches!(r, ActionResult::Err(_)));
        let r = state.apply(Action::CalibrateImage { image: 5, a: (0.0, 0.0), b: (1.0, 0.0), length: 10.0 });
        assert!(matches!(r, ActionResult::Err(_)));
    }

    /// #169: importing a PNG creates a tracing image on the plane, seeded 1 px = 1 mm and
    /// centered; Undo removes it; a missing/garbage file errors cleanly.
    #[test]
    fn import_image_creates_tracing_image_and_undoes() {
        // A tiny 4x2 PNG written via the `image` crate.
        let dir = std::env::temp_dir().join("bearcad_test_import_image");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("swatch.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();

        let mut state = AppState::default();
        let result = state.apply(Action::ImportImage {
            path: path.to_string_lossy().to_string(),
            plane: None,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}: {}", state.status);
        assert_eq!(state.doc.tracing_images.len(), 1);
        let img = &state.doc.tracing_images[0];
        assert_eq!(img.source_name, "swatch");
        assert_eq!(img.plane, 0);
        assert_eq!((img.width_mm, img.height_mm), (4.0, 2.0));
        assert_eq!(img.origin, (-2.0, -1.0), "centered on the plane origin");

        // JSON round trip: bytes survive the base64 codec.
        let json = serde_json::to_string(&state.doc).unwrap();
        assert!(json.contains("\"bytes\""));
        let doc2: crate::model::Document = serde_json::from_str(&json).unwrap();
        assert_eq!(doc2.tracing_images[0].bytes, state.doc.tracing_images[0].bytes);

        state.apply(Action::UndoLast);
        assert!(state.doc.tracing_images.is_empty(), "undo removes the import");

        let result = state.apply(Action::ImportImage {
            path: dir.join("missing.png").to_string_lossy().to_string(),
            plane: None,
        });
        assert!(matches!(result, ActionResult::Err(_)));
    }

    /// #164: Zoom to Fit frames the selection when one exists (camera target lands on the
    /// selected body's center), else the whole document's non-construction geometry.
    #[test]
    fn zoom_to_fit_frames_selection_then_falls_back_to_document() {
        let mut state = box_extrusion_state();
        state.apply(Action::ExitSketch);
        // Select the body (its box spans 0..10 in x/y, 0..5 in z → center (5, 5, 2.5)).
        state.apply(Action::ClickSceneElement {
            element: crate::hierarchy::SceneElement::Body(0),
            additive: false,
        });
        let result = state.apply(Action::ZoomToFit);
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        let target = state.cam.target;
        assert!(
            (target - glam::Vec3::new(5.0, 5.0, 2.5)).length() < 0.5,
            "camera should center on the selected body, got {target:?}"
        );

        // Nothing selected: still zooms (whole document).
        state.apply(Action::ClearSceneSelection);
        state.cam.target = glam::Vec3::new(999.0, 999.0, 999.0);
        let result = state.apply(Action::ZoomToFit);
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert!(
            (state.cam.target - glam::Vec3::new(5.0, 5.0, 2.5)).length() < 0.6,
            "empty selection frames the whole document, got {:?}",
            state.cam.target
        );
    }

    /// #166: one plural commit treats every edge in the set with the shared amount, as a
    /// single undo group (one Undo removes them all).
    #[test]
    fn commit_edge_treatments_applies_the_whole_set_in_one_undo_group() {
        let mut state = box_extrusion_state();
        let edges = vec![
            (0, crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 }),
            (0, crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 2 }),
        ];
        let result = state.apply(Action::CommitEdgeTreatments {
            edges: edges.clone(),
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        let treated: Vec<_> = state.doc.extrusions[0]
            .edge_treatments
            .iter()
            .map(|t| t.edge)
            .collect();
        assert_eq!(treated.len(), 2, "both edges treated: {treated:?}");
        for (_, edge) in &edges {
            assert!(treated.contains(edge), "missing {edge:?} in {treated:?}");
        }
        assert!(state.status.contains("2 edge"), "status: {}", state.status);

        // #168: the plural commit is one undo group — a single Undo reverts both
        // treatments while leaving the extrusion itself intact.
        state.apply(Action::UndoLast);
        assert_eq!(
            state.doc.extrusions[0].edge_treatments.len(),
            0,
            "one undo must remove the whole treated set"
        );
        assert!(!state.doc.extrusions[0].deleted, "the extrusion must survive the undo");
    }

    /// #168: undoing a single committed chamfer removes just that treatment (and restores
    /// any prior treatment list), never the extrusion.
    #[test]
    fn undo_reverts_a_single_edge_treatment() {
        let mut state = box_extrusion_state();
        let edge = crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge,
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 1);

        // Re-treating the same edge replaces it; undo restores the *previous* treatment.
        state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge,
            kind: VertexTreatmentKind::Chamfer,
            amount: 3.0,
        });
        assert!((state.doc.extrusions[0].edge_treatments[0].amount - 3.0).abs() < 1e-4);
        state.apply(Action::UndoLast);
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 1);
        assert!(
            (state.doc.extrusions[0].edge_treatments[0].amount - 2.0).abs() < 1e-4,
            "undo restores the prior treatment, got {}",
            state.doc.extrusions[0].edge_treatments[0].amount
        );

        state.apply(Action::UndoLast);
        assert!(state.doc.extrusions[0].edge_treatments.is_empty());
        assert!(!state.doc.extrusions[0].deleted, "the extrusion must survive");
    }

    /// #157/#166: switching to the Chamfer tool with treatable body edges already selected
    /// preloads them into the in-progress treatment so the gizmo shows immediately.
    #[test]
    fn switching_to_chamfer_preloads_selected_edges() {
        use crate::hierarchy::{quantize_body_point, SceneElement};

        let mut state = box_extrusion_state();
        state.apply(Action::ExitSketch);
        let treatable = crate::extrude::treatable_edges(&state.doc);
        let (_, _, a, b) = treatable[0].clone();
        state.apply(Action::ClickSceneElement {
            element: SceneElement::BodyEdge {
                body: 0,
                a: quantize_body_point(a),
                b: quantize_body_point(b),
            },
            additive: false,
        });
        state.apply(Action::SetTool(Tool::Chamfer));
        let cet = state
            .creating_edge_treatment
            .as_ref()
            .expect("selection should preload the treatment");
        assert_eq!(cet.edges.len(), 1);
        assert_eq!(cet.kind, VertexTreatmentKind::Chamfer);

        // Without a selection, no preload happens.
        let mut state = box_extrusion_state();
        state.apply(Action::ExitSketch);
        state.apply(Action::SetTool(Tool::Chamfer));
        assert!(state.creating_edge_treatment.is_none());
    }

    #[test]
    fn commit_edge_treatment_chamfers_a_vertical_edge() {
        let mut state = box_extrusion_state();
        let edge = crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        let untreated_tris = crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[0])
            .unwrap()
            .triangles
            .len();
        let result = state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge,
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 1);
        assert_eq!(state.doc.extrusions[0].edge_treatments[0].edge, edge);
        let treated_tris = crate::extrude::extrusion_mesh(&state.doc, &state.doc.extrusions[0])
            .unwrap()
            .triangles
            .len();
        assert_ne!(untreated_tris, treated_tris, "mesh should visibly change");
    }

    #[test]
    fn commit_edge_treatment_fillets_a_cap_edge() {
        let mut state = box_extrusion_state();
        let edge = crate::model::ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: true };
        let result = state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge,
            kind: VertexTreatmentKind::Fillet,
            amount: 1.5,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert_eq!(state.doc.extrusions[0].edge_treatments[0].kind, VertexTreatmentKind::Fillet);
    }

    #[test]
    fn commit_edge_treatment_re_editing_the_same_edge_replaces_rather_than_stacks() {
        let mut state = box_extrusion_state();
        let edge = crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge,
            kind: VertexTreatmentKind::Chamfer,
            amount: 1.0,
        });
        state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge,
            kind: VertexTreatmentKind::Fillet,
            amount: 2.5,
        });
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 1);
        assert_eq!(state.doc.extrusions[0].edge_treatments[0].kind, VertexTreatmentKind::Fillet);
        assert_eq!(state.doc.extrusions[0].edge_treatments[0].amount, 2.5);
    }

    #[test]
    fn commit_edge_treatment_rejects_a_conflicting_shared_vertex() {
        let mut state = box_extrusion_state();
        state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        // Cap edge 0 (base) touches profile vertices 0 and 1, sharing vertex 1 with the
        // vertical edge already treated above — a vertex miter, out of scope (SPEC §3.4).
        let result = state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge: crate::model::ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        assert!(matches!(result, ActionResult::Err(_)));
        assert_eq!(state.doc.extrusions[0].edge_treatments.len(), 1);
    }

    #[test]
    fn commit_edge_treatment_rejects_nonpositive_amount_and_out_of_range_edge() {
        let mut state = box_extrusion_state();
        let bad_amount = state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 0.0,
        });
        assert!(matches!(bad_amount, ActionResult::Err(_)));

        let out_of_range = state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 99 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        assert!(matches!(out_of_range, ActionResult::Err(_)));
        assert!(state.doc.extrusions[0].edge_treatments.is_empty());
    }

    /// #103: an edge treatment the OCCT kernel can't actually build (e.g. a fillet radius
    /// far larger than the solid) must be rejected at commit with an actionable error, not
    /// stored — storing it silently knocked the whole body onto the additive-only mesh
    /// fallback, deleting its cut holes from the render.
    #[cfg(feature = "occt")]
    #[test]
    fn commit_edge_treatment_rejects_a_kernel_infeasible_amount() {
        let mut state = box_extrusion_state();
        let result = state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Fillet,
            amount: 500.0,
        });
        assert!(matches!(result, ActionResult::Err(_)), "{result:?}");
        assert!(
            state.doc.extrusions[0].edge_treatments.is_empty(),
            "infeasible treatment must not be stored"
        );
        assert!(
            state.status.contains("doesn't fit") && state.status.contains("radius"),
            "status should explain the rejection: {}",
            state.status
        );
    }

    /// #103 part 2: a document that *already* contains a kernel-infeasible treatment on a
    /// cut-bearing body (created before the commit-time trial existed) renders the additive
    /// fallback — the status bar must warn that the cuts are not shown, both right after
    /// loading the document and after any later document mutation.
    #[cfg(feature = "occt")]
    #[test]
    fn kernel_fallback_on_a_cut_bearing_body_warns_on_open_and_mutation() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        let outer = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            0.0,
            0.0,
            10.0,
            10.0,
            [false; 4],
        );
        let inner = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            3.0,
            3.0,
            4.0,
            4.0,
            [false; 4],
        );
        for face in [
            ExtrudeFace::Polygon(outer.to_vec()),
            ExtrudeFace::Polygon(inner.to_vec()),
        ] {
            state.doc.extrusions.push(Extrusion {
                sketch,
                faces: vec![face],
                distance: 5.0,
                target: None,
                expression: String::new(),
                name: None,
                deleted: false,
                edge_treatments: Vec::new(),
            });
            state.doc.shape_order.push(ShapeKind::Extrusion);
        }
        state.doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid { add: vec![0], cut: vec![1] },
            name: None,
            deleted: false,
            shadow: false,
        });
        state.doc.shape_order.push(ShapeKind::Body);
        assert!(
            crate::extrude::occt_body_shape(&state.doc, 0).is_some(),
            "sanity: the untreated cut body builds in the kernel"
        );
        // Bypass commit validation: splice the impossible fillet straight into the document.
        state.doc.extrusions[0].edge_treatments.push(EdgeTreatment {
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Fillet,
            amount: 500.0,
        });
        // The fallback still renders something (additive-only)...
        assert!(crate::extrude::body_solid_mesh(&state.doc, 0).is_some());

        // ...and reopening the document surfaces the warning in the status bar.
        let path = std::env::temp_dir().join("bearcad_103_cut_fallback_warning.bearcad");
        let path = path.to_string_lossy().to_string();
        crate::storage::save(&path, &state.doc).unwrap();
        let mut reopened = AppState::default();
        let result = reopened.apply(Action::Open { path: path.clone() });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert!(
            reopened.status.contains("cuts are not shown"),
            "open should warn: {}",
            reopened.status
        );

        // Any later document mutation re-asserts the warning while the state persists. (A
        // valid chamfer on a far edge commits fine: the kernel trial only rejects when the
        // *base* shape builds, and this document's base is already kernel-infeasible.)
        let result = reopened.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 2 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 1.0,
        });
        assert!(matches!(result, ActionResult::Ok), "{result:?}");
        assert!(
            reopened.status.contains("cuts are not shown"),
            "mutation should re-warn: {}",
            reopened.status
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn commit_edge_treatment_rejects_a_circle_profile_edge() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state
            .doc
            .circles
            .push(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Circle);
        state.apply(Action::CreateExtrusion {
            sketch,
            faces: vec![ExtrudeFace::Circle(0)],
            distance: 6.0,
            body: crate::actions::ExtrudeBodyChoice::New,
            target: None,
        });
        let result = state.apply(Action::CommitEdgeTreatment {
            extrusion: 0,
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 1.0,
        });
        assert!(matches!(result, ActionResult::Err(_)));
    }

    /// Signed volume of a closed mesh via the divergence theorem (mirrors #77's
    /// `mesh_signed_volume` in src/extrude.rs) — an independent check that the committed
    /// extrusion's geometry matches the expected intersection-region volume.
    fn mesh_signed_volume(mesh: &crate::extrude::SolidMesh) -> f32 {
        mesh.triangles.iter().map(|[a, b, c]| a.dot(b.cross(*c)) / 6.0).sum()
    }

    /// End-to-end test for #16/#62: an overlapping rect+circle sketch, resolving a click
    /// inside their intersection to `ExtrudeFace::Boolean { Intersection, .. }`, committing the
    /// extrusion, and checking the resulting mesh's volume against the independently-computed
    /// intersection area × height (divergence-theorem check).
    #[test]
    fn boolean_intersection_face_toggles_and_extrudes_with_sane_volume() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        // Rect covers the right half-plane (corners (0,-20)-(20,20) => x=0,y=-20,w=20,h=40);
        // circle radius 5 at the origin. Their intersection is a right half-disk, area pi*r^2/2.
        let rect_lines =
            crate::construction::add_line_rectangle(&mut state.doc, sketch, 0.0, -20.0, 20.0, 40.0, [false; 4]);
        state.doc.circles.push(crate::model::Circle::from_local_center_radius(
            sketch, 0.0, 0.0, 5.0, 0.0,
        ));
        state.doc.shape_order.push(ShapeKind::Circle);
        state.refresh_document_health();

        let rect_face = ExtrudeFace::Polygon(rect_lines.to_vec());
        let circle_face = ExtrudeFace::Circle(0);
        let partner = crate::extrude::overlapping_partner(&state.doc, sketch, &rect_face);
        assert_eq!(
            partner,
            Some(circle_face.clone()),
            "rect/circle should be the unique overlapping pair"
        );

        // A click at (2, 0) lands inside both loops, so it should resolve to their Intersection.
        let resolved = crate::extrude::resolve_boolean_click(
            &state.doc,
            sketch,
            &rect_face,
            &circle_face,
            (2.0, 0.0),
        );
        let expected_face = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Intersection,
            a: Box::new(rect_face.clone()),
            b: Box::new(circle_face.clone()),
        };
        assert_eq!(resolved, Some(expected_face.clone()));

        state.apply(Action::SetTool(Tool::Extrude));
        state.apply(Action::ToggleExtrudeFace { face: resolved.unwrap() });
        assert_eq!(
            state.creating_extrusion.as_ref().unwrap().faces,
            vec![expected_face]
        );
        state.apply(Action::SetExtrudeDistance { distance: 4.0 });
        state.apply(Action::CommitExtrusion);

        assert_eq!(state.doc.extrusions.len(), 1);
        assert_eq!(state.doc.bodies.len(), 1);
        let mesh = crate::extrude::body_solid_mesh(&state.doc, 0).expect("mesh");
        assert!(!mesh.triangles.is_empty());

        let expected_area = std::f32::consts::PI * 25.0 / 2.0;
        let expected_volume = expected_area * 4.0;
        let volume = mesh_signed_volume(&mesh).abs();
        // The circle is only a 48-gon approximation, so allow a couple percent slack.
        assert!(
            (volume - expected_volume).abs() < expected_volume * 0.02,
            "volume {volume} !~= {expected_volume}"
        );
    }

    #[test]
    fn line_tool_chains_into_next_segment() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.tool = Tool::Line;
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: "10".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::CommitLine);

        // The segment was committed, and a fresh segment is already started at its endpoint.
        assert_eq!(state.doc.lines.len(), 1);
        let cl = state
            .creating_line
            .as_ref()
            .expect("a new segment should be chained from the endpoint");
        let frame = sketch_geometry_frame(&state.doc, sketch).unwrap();
        let (ou, ov) = world_to_local(&frame, cl.origin);
        assert!((ou - 10.0).abs() < 1e-3 && ov.abs() < 1e-3, "new origin at endpoint");
        // The new start snaps to the previous endpoint so the polyline stays connected.
        assert!(matches!(
            state.line_start_snap,
            Some(crate::snapping::SnapTarget::Vertex(ConstraintPoint::LineEndpoint {
                line: 0,
                end: LineEnd::End
            }))
        ));

        // Committing the chained segment connects the two lines (coincident constraint).
        state.creating_line.as_mut().unwrap().last_mouse = Vec3::new(10.0, 10.0, 0.0);
        state.creating_line.as_mut().unwrap().text.clear();
        state.creating_line.as_mut().unwrap().user_edited = false;
        state.apply(Action::CommitLine);
        assert_eq!(state.doc.lines.len(), 2);
        assert!(state
            .doc
            .constraints
            .iter()
            .any(|c| !c.deleted && matches!(c.kind, crate::model::ConstraintKind::Coincident { .. })));
    }

    #[test]
    fn line_tool_stops_chaining_when_closing_on_a_vertex() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.tool = Tool::Line;
        // An existing line whose start vertex sits at (10, 0).
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 10.0, 0.0, 20.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);

        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: "10".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        // The end latched onto the existing vertex at (10, 0).
        state.line_end_snap = Some(crate::snapping::SnapTarget::Vertex(
            ConstraintPoint::LineEndpoint {
                line: 0,
                end: LineEnd::Start,
            },
        ));
        state.apply(Action::CommitLine);

        assert_eq!(state.doc.lines.len(), 2);
        assert!(
            state.creating_line.is_none(),
            "closing onto a vertex finishes the polyline"
        );
    }

    #[test]
    fn commit_circle_adds_to_document_with_diameter_constraint() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.creating_circle = Some(CreatingCircle {
            origin: Vec3::ZERO,
            text: "20".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
        });
        state.apply(Action::CommitCircle);
        assert_eq!(state.doc.circles.len(), 1);
        assert!((state.doc.circles[0].diameter() - 20.0).abs() < 1e-4);
        assert_eq!(state.doc.constraints.len(), 1);
        assert!(state.doc.circles[0].diameter_locked);
        assert!(state.creating_circle.is_none());
    }

    /// #138: typing `name=value` into a dimension text input (here a circle's diameter) creates
    /// the variable and drives the dimension by it. `dia=30` makes a parameter `dia`=30 and a
    /// diameter constraint whose expression is `dia`.
    #[test]
    fn commit_circle_with_inline_variable_creates_parameter() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.creating_circle = Some(CreatingCircle {
            origin: Vec3::ZERO,
            text: "dia=30".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
        });
        state.apply(Action::CommitCircle);
        assert_eq!(state.doc.circles.len(), 1);
        assert!((state.doc.circles[0].diameter() - 30.0).abs() < 1e-4);
        let param = state
            .doc
            .parameters
            .iter()
            .find(|p| !p.deleted && p.name == "dia")
            .expect("variable dia created");
        assert_eq!(param.expression, "30");
        assert_eq!(
            state.doc.constraints.iter().find(|c| !c.deleted).unwrap().expression,
            "dia"
        );
    }

    /// #147 / SPEC §5.1.1: `dia=30` when `dia` already exists **redefines** the parameter and
    /// commits the circle, instead of failing with a duplicate-name error.
    #[test]
    fn commit_circle_with_inline_variable_redefines_existing_parameter() {
        let mut state = AppState::default();
        crate::parameters::add_parameter(&mut state.doc, "dia".to_string(), "20mm".to_string())
            .unwrap();
        begin_default_sketch(&mut state);
        state.creating_circle = Some(CreatingCircle {
            origin: Vec3::ZERO,
            text: "dia=30".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
        });
        state.apply(Action::CommitCircle);
        assert_eq!(state.doc.circles.len(), 1, "commit must not fail: {}", state.status);
        assert!((state.doc.circles[0].diameter() - 30.0).abs() < 1e-4);
        let dia_params: Vec<_> = state
            .doc
            .parameters
            .iter()
            .filter(|p| !p.deleted && p.name == "dia")
            .collect();
        assert_eq!(dia_params.len(), 1, "still exactly one 'dia' parameter");
        assert_eq!(dia_params[0].expression, "30", "existing parameter redefined");
    }

    #[test]
    fn dimension_tool_begins_edit_when_line_selected() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 8.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(0),
            additive: false,
        });
        state.apply(Action::SetTool(Tool::Dimension));
        assert!(state.editing_committed_dim.is_some());
        assert_eq!(
            state.editing_committed_dim.as_ref().unwrap().target,
            DimEditTarget::New(DimensionTarget::Distance(DistanceTarget::LineLength(0)))
        );
    }

    #[test]
    fn angle_gizmo_constraint_only_while_editing_committed_angle() {
        use crate::model::{ConstraintLine, DimensionTarget};

        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 0.0, 10.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.shape_order.push(ShapeKind::Line);
        let rotation_sign = crate::constraints::angle_constraint_natural_sign(
            &state.doc,
            ConstraintLine::Line(0),
            ConstraintLine::Line(1),
        )
        .unwrap();
        crate::constraints::add_angle_constraint_with_sign(
            &mut state.doc,
            sketch,
            ConstraintLine::Line(0),
            ConstraintLine::Line(1),
            rotation_sign,
            "90deg".to_string(),
        )
        .unwrap();
        assert_eq!(
            angle_gizmo_constraint_for_edit(state.editing_committed_dim.as_ref(), &state.doc),
            None
        );
        state.editing_committed_dim = Some(EditingCommittedDim {
            target: DimEditTarget::Constraint(0),
            text: "90deg".to_string(),
            pending_focus: true,
            dim_offset: None,
        });
        assert_eq!(
            angle_gizmo_constraint_for_edit(state.editing_committed_dim.as_ref(), &state.doc),
            Some(0)
        );
        state.editing_committed_dim = Some(EditingCommittedDim {
            target: DimEditTarget::New(DimensionTarget::Angle {
                line_a: ConstraintLine::Line(0),
                line_b: ConstraintLine::Line(1),
                rotation_sign: 1,
            }),
            text: "45deg".to_string(),
            pending_focus: true,
            dim_offset: None,
        });
        assert_eq!(
            angle_gizmo_constraint_for_edit(state.editing_committed_dim.as_ref(), &state.doc),
            None
        );
    }

    #[test]
    fn dimension_tool_begins_angle_edit_for_two_non_parallel_lines() {
        use crate::model::{ConstraintLine, DimensionTarget};

        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 0.0, 10.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.shape_order.push(ShapeKind::Line);
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(0),
            additive: false,
        });
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(1),
            additive: true,
        });
        state.apply(Action::SetTool(Tool::Dimension));
        // A brand-new angle dimension enters a placement phase (mouse picks the quadrant)
        // rather than jumping straight to editing the value (#40).
        assert!(state.editing_committed_dim.is_none());
        assert_eq!(
            state.placing_angle_dimension,
            Some(PlacingAngleDimension {
                line_a: ConstraintLine::Line(0),
                line_b: ConstraintLine::Line(1),
                rotation_sign: 1,
                arc_offset: None,
            })
        );

        let target = DimensionTarget::Angle {
            line_a: ConstraintLine::Line(0),
            line_b: ConstraintLine::Line(1),
            rotation_sign: 1,
        };
        state.placing_angle_dimension = None;
        state.apply(Action::BeginDimensionEdit { target: target.clone() });
        assert_eq!(
            state.editing_committed_dim.as_ref().unwrap().target,
            DimEditTarget::New(target)
        );
    }

    #[test]
    fn committing_a_placed_angle_dim_persists_its_arc_radius() {
        use crate::model::{ConstraintLine, DimensionTarget};

        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state
            .doc
            .lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 10.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.shape_order.push(ShapeKind::Line);
        state.tool = Tool::Dimension;

        let target = DimensionTarget::Angle {
            line_a: ConstraintLine::Line(0),
            line_b: ConstraintLine::Line(1),
            rotation_sign: 1,
        };
        state.apply(Action::BeginDimensionEdit { target });
        // The viewport carries the previewed arc radius onto the edit before commit.
        let edit = state.editing_committed_dim.as_mut().unwrap();
        edit.dim_offset = Some(52.0);
        edit.text = "45deg".to_string();
        assert!(matches!(state.apply(Action::CommitCommittedDim), ActionResult::Ok));

        let id = crate::constraints::find_angle_constraint(
            &state.doc,
            ConstraintLine::Line(0),
            ConstraintLine::Line(1),
        )
        .expect("angle constraint created");
        assert_eq!(
            state.doc.constraints[id].dim_offset,
            Some(52.0),
            "the placed arc radius should be persisted on the constraint"
        );
    }

    #[test]
    fn dimension_tool_adds_distance_constraint_to_line() {
        let mut state = AppState::default();
        let sketch = begin_default_sketch(&mut state);
        state.doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 8.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.apply(Action::SetTool(Tool::Dimension));
        state.apply(Action::BeginDimensionEdit {
            target: DimensionTarget::Distance(DistanceTarget::LineLength(0)),
        });
        state.apply(Action::SetLineLength {
            value: "12mm".to_string(),
        });
        state.apply(Action::CommitCommittedDim);
        assert_eq!(state.doc.constraints.len(), 1);
        assert!((state.doc.lines[0].length() - 12.0).abs() < 1e-3);
    }

    #[test]
    fn rect_end_point_evaluates_unit_expression() {
        let cr = CreatingRect {
            origin: Vec3::ZERO,
            texts: ["2in".to_string(), "5mm / 2".to_string()],
            focused: 0,
            last_mouse: Vec3::new(100.0, 100.0, 0.0),
            user_edited: [true, true],
            pending_focus: false,
            construction: false,
        };
        let frame = xy_frame();
        let doc = Document::default();
        let end = cr.end_point(&frame, &doc);
        assert!((end.x - 50.8).abs() < 1e-3);
        assert!((end.y - 2.5).abs() < 1e-3);
    }

    #[test]
    fn line_end_point_evaluates_mixed_expression() {
        let cl = CreatingLine {
            origin: Vec3::ZERO,
            text: "2in + 5mm / 2".to_string(),
            last_mouse: Vec3::new(10.0, 10.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        };
        let frame = xy_frame();
        let doc = Document::default();
        let end = cl.end_point(&frame, &doc);
        let (u0, v0) = world_to_local(&frame, cl.origin);
        let (u1, v1) = world_to_local(&frame, end);
        let line = Line::from_local_endpoints(0, u0, v0, u1, v1);
        assert!((line.length() - 53.3).abs() < 1e-2);
    }

    #[test]
    fn set_plane_offset_evaluates_expression() {
        let mut state = AppState::default();
        state.apply(Action::BeginConstructionPlane {
            reference: PlaneReference::Face {
                origin: Vec3::ZERO,
                normal: Vec3::Z,
                label: "Ground".to_string(),
            },
            parent: ConstructionPlaneParent::Root,
        });
        state.apply(Action::SetPlaneOffset {
            value: "1in + 2mm".to_string(),
        });
        let cp = state.creating_plane.as_ref().unwrap();
        assert!((cp.offset_live - 27.4).abs() < 1e-3);
        assert_eq!(cp.offset_text, "1in + 2mm");
    }

    #[test]
    fn line_end_point_uses_locked_length() {
        let cl = CreatingLine {
            origin: Vec3::new(1.0, 2.0, 0.0),
            text: "5".to_string(),
            last_mouse: Vec3::new(4.0, 6.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        };
        let frame = xy_frame();
        let doc = Document::default();
        let end = cl.end_point(&frame, &doc);
        let (u0, v0) = world_to_local(&frame, cl.origin);
        let (u1, v1) = world_to_local(&frame, end);
        let line = Line::from_local_endpoints(0, u0, v0, u1, v1);
        assert!((line.length() - 5.0).abs() < 1e-4);
    }

    #[test]
    fn line_end_point_defaults_along_x_when_no_direction() {
        let cl = CreatingLine {
            origin: Vec3::ZERO,
            text: "7".to_string(),
            last_mouse: Vec3::ZERO,
            user_edited: true,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        };
        let frame = xy_frame();
        let doc = Document::default();
        let end = cl.end_point(&frame, &doc);
        assert!((end.x - 7.0).abs() < 1e-4);
        assert!(end.y.abs() < 1e-4);
    }

    #[test]
    fn escape_on_line_tool_in_sketch_switches_to_select() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.apply(Action::SetTool(Tool::Line));
        state.apply(Action::CancelOperation);
        assert!(state.sketch_session.is_some());
        assert_eq!(state.tool, Tool::Select);
    }

    #[test]
    fn escape_on_select_tool_in_sketch_exits_sketch() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        assert_eq!(state.tool, Tool::Select);
        state.apply(Action::CancelOperation);
        assert!(state.sketch_session.is_none());
        assert_eq!(state.tool, Tool::Select);
    }

    #[test]
    fn escape_while_drawing_rectangle_cancels_without_exiting_sketch() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.apply(Action::SetTool(Tool::Rectangle));
        state.creating_rect = Some(CreatingRect {
            origin: Vec3::ZERO,
            texts: ["".to_string(), "".to_string()],
            focused: 0,
            last_mouse: Vec3::new(10.0, 5.0, 0.0),
            user_edited: [false, false],
            pending_focus: false,
            construction: false,
        });
        state.apply(Action::CancelOperation);
        assert!(state.sketch_session.is_some());
        assert_eq!(state.tool, Tool::Rectangle);
        assert!(state.creating_rect.is_none());
    }

    #[test]
    fn exit_sketch_restores_world_orbit_mode() {
        let mut state = AppState::default();
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        while state.cam.tick_transition(0.05) {}
        state.cam.orbit_trackball(egui::vec2(10.0, 6.0));
        state.apply(Action::ExitSketch);
        assert!(state.sketch_session.is_none());
        // Exit animates back to the pre-sketch pose; world-orbit mode is restored once the
        // return transition completes.
        while state.cam.tick_transition(0.05) {}
        assert!(!state.cam.has_custom_view_up());
        assert!(!state.cam.has_orbit_trackball_state());
    }

    #[test]
    fn exit_sketch_clears_session() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.apply(Action::ExitSketch);
        assert!(state.sketch_session.is_none());
        assert_eq!(state.tool, Tool::Select);
    }

    #[test]
    fn exit_sketch_returns_to_pre_sketch_view() {
        let mut state = AppState::default();
        let viewport =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let samples = [
            Vec3::ZERO,
            Vec3::new(40.0, 20.0, 0.0),
            Vec3::new(-30.0, 50.0, 10.0),
        ];

        // Capture the camera before entering the sketch.
        let vp_before = state.cam.view_proj(viewport);
        let screens_before: Vec<_> = samples
            .iter()
            .map(|p| state.cam.project(*p, viewport, &vp_before).unwrap())
            .collect();

        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        while state.cam.tick_transition(0.05) {}
        // Entering must actually have moved the camera (otherwise this proves nothing).
        let vp_sketch = state.cam.view_proj(viewport);
        let moved = samples.iter().zip(&screens_before).any(|(p, before)| {
            (state.cam.project(*p, viewport, &vp_sketch).unwrap() - *before).length() > 1.0
        });
        assert!(moved, "entering the sketch should reframe the camera");

        state.apply(Action::ExitSketch);
        while state.cam.tick_transition(0.05) {}

        let vp_after = state.cam.view_proj(viewport);
        for (p, before) in samples.iter().zip(screens_before) {
            let after = state.cam.project(*p, viewport, &vp_after).unwrap();
            assert!(
                (before - after).length() < 0.5,
                "exiting sketch should return to the pre-sketch view: {before:?} -> {after:?} for {p:?}"
            );
        }
    }

    #[test]
    fn begin_ground_plane_sketch_does_not_spin_yaw() {
        // The ground plane is a near-vertical (top-down) view, where yaw is just roll. Entry
        // must keep the current yaw rather than swinging it to zero (which looks like a spin).
        let mut state = AppState::default();
        let yaw_before = state.cam.yaw;
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        while state.cam.tick_transition(0.05) {}
        assert!(
            (state.cam.yaw - yaw_before).abs() < 0.02,
            "ground-plane sketch entry should not change yaw: {yaw_before} -> {}",
            state.cam.yaw
        );
    }

    #[test]
    fn begin_sketch_keeps_yaw_pitch_when_already_face_on() {
        use crate::camera::StandardView;

        let mut state = AppState::default();
        let (yaw, pitch) = StandardView::Top.yaw_pitch();
        state.cam.yaw = yaw;
        state.cam.pitch = pitch;
        state.cam.set_view_up(Some(Vec3::Y));
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        while state.cam.tick_transition(0.05) {}
        assert!((state.cam.yaw - yaw).abs() < 0.02);
        assert!((state.cam.pitch - pitch).abs() < 0.02);
    }

    #[test]
    fn begin_sketch_from_isometric_puts_u_axis_right_v_axis_up() {
        // #187: entering a sketch orients the plane's u-axis screen-right and v-axis
        // screen-up, so a Horizontal constraint (line along u) reads horizontal and a
        // Vertical constraint (along v) reads vertical — regardless of the prior roll.
        let viewport =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mut state = AppState::default();
        let start = axis_layout(&state.cam, viewport).expect("startup axes visible");
        assert_eq!(
            start,
            (ScreenAxisDir::Left, ScreenAxisDir::Right),
            "isometric startup should show red left and green right"
        );

        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        while state.cam.tick_transition(0.05) {}

        let end = axis_layout(&state.cam, viewport).expect("sketch axes visible");
        assert_eq!(
            end,
            (ScreenAxisDir::Right, ScreenAxisDir::Up),
            "sketch entry should put the u-axis (red/X) right and v-axis (green/Y) up"
        );

        let frame = sketch_frame(&state.doc, FaceId::ConstructionPlane(0)).unwrap();
        let vp = state.cam.view_proj(viewport);
        let base = state.cam.project(frame.origin, viewport, &vp).unwrap();
        let u = state
            .cam
            .project(frame.origin + frame.u_axis * 10.0, viewport, &vp)
            .unwrap();
        let v = state
            .cam
            .project(frame.origin + frame.v_axis * 10.0, viewport, &vp)
            .unwrap();
        assert!(u.x > base.x + 5.0, "u should point right on screen");
        assert!(v.y < base.y - 5.0, "v should point up on screen (egui y-down)");
    }

    #[test]
    fn begin_sketch_from_top_view_aligns_v_axis_up() {
        use crate::camera::StandardView;

        let mut state = AppState::default();
        let (yaw, pitch) = StandardView::Top.yaw_pitch();
        state.cam.yaw = yaw;
        state.cam.pitch = pitch;
        state.cam.set_view_up(Some(Vec3::Y));
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: None,
        });
        let frame = sketch_frame(&state.doc, FaceId::ConstructionPlane(0)).unwrap();
        while state.cam.tick_transition(0.05) {}
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let vp = state.cam.view_proj(viewport);
        let base = state
            .cam
            .project(frame.origin, viewport, &vp)
            .expect("origin visible");
        let above = state
            .cam
            .project(frame.origin + frame.v_axis * 10.0, viewport, &vp)
            .expect("v offset visible");
        assert!(above.y < base.y, "plane v-axis should point up on screen");
    }

    #[test]
    fn begin_sketch_frames_camera_to_face_normal() {
        let mut state = AppState::default();
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 40.0), egui::vec2(800.0, 600.0));
        let distance_before = state.cam.distance;
        state.apply(Action::BeginSketch {
            face: FaceId::ConstructionPlane(0),
            viewport: Some(viewport),
        });
        assert!(state.cam.is_transitioning());
        assert!(state.sketch_session.is_some());
        while state.cam.tick_transition(0.05) {}
        assert!((state.cam.distance - distance_before).abs() < 0.5);
        let view = (state.cam.eye() - state.cam.target).normalize();
        assert!(view.z > 0.95, "empty plane should look along face normal");
    }

    #[test]
    fn begin_sketch_creates_new_sketch_each_time() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        let second = begin_default_sketch(&mut state);
        assert_eq!(second, 1);
        assert_eq!(state.doc.sketches.len(), 2);
        assert_eq!(
            state.doc.sketches[0].face,
            FaceId::ConstructionPlane(0)
        );
        assert_eq!(
            state.doc.sketches[1].face,
            FaceId::ConstructionPlane(0)
        );
    }

    #[test]
    fn begin_sketch_on_circle_face_hosts_child_sketch() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.circles.push(Circle::from_local_center_radius(
            sketch, 0.0, 0.0, 20.0, 0.0,
        ));
        state.doc.shape_order.push(ShapeKind::Circle);
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        assert!(matches!(
            state.apply(Action::BeginSketch {
                face: FaceId::Circle(0),
                viewport: Some(viewport),
            }),
            ActionResult::Ok
        ));
        assert_eq!(state.doc.sketches.len(), 2);
        assert_eq!(state.doc.sketches[1].face, FaceId::Circle(0));
        assert!(state.sketch_session.is_some());
    }

    #[test]
    fn tree_pane_visible_by_default() {
        let state = AppState::default();
        assert!(state.panes.is_visible(Pane::Hierarchy));
        assert_eq!(Pane::Hierarchy.label(), "Elements");
    }

    #[test]
    fn context_pane_visible_by_default() {
        let state = AppState::default();
        assert!(state.panes.is_visible(Pane::Context));
        assert_eq!(Pane::Context.label(), "Context");
    }

    #[test]
    fn delete_selection_tombstones_selected_geometry() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::default());
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(0),
            additive: false,
        });
        state.apply(Action::DeleteSelection);
        assert!(state.doc.lines[0].deleted);
        assert!(state.scene_selection.is_empty());
        assert_eq!(
            state.document_health.element_status(SceneElement::Line(0)),
            crate::document_health::HealthStatus::Healthy
        );
    }

    #[test]
    fn delete_selection_status_reports_invalid_and_unstable_counts() {
        use crate::model::{Constraint, ConstraintKind, ConstraintLine};

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(0),
                line_b: ConstraintLine::Line(1),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(0),
            additive: false,
        });
        state.apply(Action::DeleteSelection);
        assert!(state.status.contains("1 invalid"));
        assert!(state.status.contains("1 unstable"));
    }

    #[test]
    fn frozen_unstable_line_blocks_rename_and_vertex_drag() {
        use crate::document_lifecycle::tombstone_element;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine, LineEnd};

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        state.doc.shape_order.push(ShapeKind::Line);
        state.doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(0),
                line_b: ConstraintLine::Line(1),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        tombstone_element(&mut state.doc, SceneElement::Line(0));
        state.refresh_document_health();
        state.apply(Action::OpenSketch {
            sketch,
            viewport: None,
        });

        assert_eq!(
            state.apply(Action::CommitElementName {
                element: SceneElement::Line(1),
                name: "Partner".to_string(),
            }),
            ActionResult::Err("unstable: Lost parallel/perpendicular partner".to_string())
        );
        assert_eq!(
            state.apply(Action::DragVertex {
                point: ConstraintPoint::LineEndpoint {
                    line: 1,
                    end: LineEnd::Start,
                },
                u: 1.0,
                v: 5.0,
            }),
            ActionResult::Err("unstable: Lost parallel/perpendicular partner".to_string())
        );
    }

    #[test]
    fn undo_last_refreshes_document_health() {
        let mut state = AppState::default();
        // Checkpoint the empty document, then force an invalid-parameter state in place.
        state.undo_stack.push(state.doc.clone());
        state.doc.parameters.push(crate::model::Parameter {
            name: "bad".to_string(),
            expression: "1mm / 0".to_string(),
            deleted: false,
            source: None,
        });
        state.doc.shape_order.push(ShapeKind::Parameter);
        state.refresh_document_health();
        assert_eq!(
            state.document_health.parameter_status(0),
            crate::document_health::HealthStatus::Invalid
        );

        // Undo restores the checkpoint and recomputes health from the restored document.
        state.apply(Action::UndoLast);
        assert!(state.doc.parameters.is_empty());
        assert!(state.document_health.parameters.is_empty());
    }

    #[test]
    fn open_tombstoned_document_recomputes_health() {
        use crate::document_lifecycle::tombstone_element;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine};

        let dir = std::env::temp_dir();
        let path = dir.join("bearcad_open_health.bearcad");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(0),
                line_b: ConstraintLine::Line(1),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        doc.shape_order.push(ShapeKind::Constraint);
        tombstone_element(&mut doc, SceneElement::Line(0));
        crate::storage::save(&path, &doc).unwrap();

        let loaded = crate::storage::open(&path).unwrap();
        assert!(loaded.lines[0].deleted);
        assert!(!loaded.lines[1].deleted);
        let health_after_load = crate::document_health::recompute_document_health(&loaded);
        assert_eq!(
            health_after_load.element_status(SceneElement::Constraint(0)),
            crate::document_health::HealthStatus::Invalid
        );
        assert_eq!(
            health_after_load.element_status(SceneElement::Line(1)),
            crate::document_health::HealthStatus::Unstable
        );

        let mut state = AppState::default();
        state.apply(Action::Open { path: path.clone() });
        assert_eq!(
            state.document_health.element_status(SceneElement::Constraint(0)),
            crate::document_health::HealthStatus::Invalid
        );
        assert_eq!(
            state.document_health.element_status(SceneElement::Line(1)),
            crate::document_health::HealthStatus::Unstable
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn toggle_construction_rectangle_tool_before_drawing() {
        let mut state = AppState::default();
        state.apply(Action::SetTool(Tool::Rectangle));
        assert_eq!(state.rect_draw_construction_mode(), Some(false));
        assert_eq!(state.apply(Action::ToggleConstruction), ActionResult::Ok);
        assert_eq!(state.rect_draw_construction_mode(), Some(true));
        assert!(state.creating_rect.is_none());
    }

    #[test]
    fn apply_construction_line_tool_before_drawing() {
        let mut state = AppState::default();
        state.apply(Action::SetTool(Tool::Line));
        assert_eq!(
            state.apply(Action::ApplyConstruction { construction: true }),
            ActionResult::Ok
        );
        assert_eq!(state.line_draw_construction_mode(), Some(true));
        assert!(state.creating_line.is_none());
    }

    #[test]
    fn draw_construction_mode_persists_across_rectangle_and_line_tools() {
        let mut state = AppState::default();
        state.apply(Action::SetTool(Tool::Rectangle));
        state.apply(Action::ToggleConstruction);
        assert!(state.draw_construction);
        state.apply(Action::SetTool(Tool::Line));
        assert_eq!(state.line_draw_construction_mode(), Some(true));
        state.apply(Action::SetTool(Tool::Rectangle));
        assert_eq!(state.rect_draw_construction_mode(), Some(true));
    }

    #[test]
    fn toggle_construction_while_drawing_rectangle() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.creating_rect = Some(CreatingRect {
            origin: Vec3::ZERO,
            texts: ["".to_string(), "".to_string()],
            focused: 0,
            last_mouse: Vec3::new(10.0, 5.0, 0.0),
            user_edited: [false, false],
            pending_focus: false,
            construction: false,
        });
        assert_eq!(state.apply(Action::ToggleConstruction), ActionResult::Ok);
        assert!(state.creating_rect.as_ref().unwrap().construction);
    }

    #[test]
    fn commit_line_with_construction_draw_mode() {
        let mut state = AppState::default();
        begin_default_sketch(&mut state);
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: "10".to_string(),
            last_mouse: Vec3::new(10.0, 0.0, 0.0),
            user_edited: true,
            pending_focus: false,
            construction: true,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::CommitLine);
        assert!(state.doc.lines[0].construction);
    }

    #[test]
    fn toggle_element_visibility() {
        let mut state = AppState::default();
        state.apply(Action::ToggleElementVisibility(SceneElement::Sketch(0)));
        assert!(!state.element_visibility.is_visible(SceneElement::Sketch(0)));
    }

    #[test]
    fn focus_line_length_sets_pending_focus() {
        let mut state = AppState::default();
        state.creating_line = Some(CreatingLine {
            origin: Vec3::ZERO,
            text: String::new(),
            last_mouse: Vec3::ZERO,
            user_edited: false,
            pending_focus: false,
            construction: false,
            curve_mode: false,
            tangent_constraint: true,
            chained_from: None,
            chained_from_bezier: None,
        });
        state.apply(Action::FocusLineLength);
        assert!(state.creating_line.as_ref().unwrap().pending_focus);
    }

    #[test]
    fn view_cube_pane_visible_by_default() {
        let state = AppState::default();
        assert!(state.panes.is_visible(Pane::ViewCube));
    }

    #[test]
    fn toggle_pane_hides_then_shows() {
        let mut state = AppState::default();
        state.apply(Action::TogglePane(Pane::ViewCube));
        assert!(!state.panes.is_visible(Pane::ViewCube));
        state.apply(Action::TogglePane(Pane::ViewCube));
        assert!(state.panes.is_visible(Pane::ViewCube));
    }

    #[test]
    fn toggle_command_palette_opens_and_closes() {
        let mut state = AppState::default();
        assert!(!state.command_palette.open);
        state.apply(Action::ToggleCommandPalette);
        assert!(state.command_palette.open);
        state.apply(Action::SetCommandPaletteOpen { open: false });
        assert!(!state.command_palette.open);
    }

    #[test]
    fn set_pane_visible_is_explicit() {
        let mut state = AppState::default();
        state.apply(Action::SetPaneVisible {
            pane: Pane::ViewCube,
            visible: false,
        });
        assert!(!state.panes.is_visible(Pane::ViewCube));
        // Setting the same value again is idempotent.
        state.apply(Action::SetPaneVisible {
            pane: Pane::ViewCube,
            visible: false,
        });
        assert!(!state.panes.is_visible(Pane::ViewCube));
    }

    #[test]
    fn set_home_view_action_stores_current_camera_pose() {
        let mut state = AppState::default();
        state.cam.target = Vec3::new(5.0, -3.0, 2.0);
        state.cam.yaw = 0.9;
        state.cam.pitch = 0.4;
        state.cam.distance = 180.0;
        state.apply(Action::SetHomeView);
        let home = state.cam.home_view();
        assert!((home.target.x - 5.0).abs() < 1e-4);
        assert!((home.yaw - 0.9).abs() < 1e-4);
        assert_eq!(state.status, "Home view set");
    }

    #[test]
    fn orbit_changes_camera() {
        let mut state = AppState::default();
        let yaw = state.cam.yaw;
        state.apply(Action::OrbitCamera { delta: (10.0, 5.0) });
        assert_ne!(state.cam.yaw, yaw);
    }

    #[test]
    fn commit_element_name_updates_document() {
        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 1.0, 0.0));
        assert_eq!(
            state.apply(Action::CommitElementName {
                element: SceneElement::Line(0),
                name: "Guide".to_string(),
            }),
            ActionResult::Ok
        );
        assert_eq!(state.doc.lines[0].name.as_deref(), Some("Guide"));
    }

    #[test]
    fn focus_element_name_requires_single_selection() {
        let mut state = AppState::default();
        assert_eq!(
            state.apply(Action::FocusElementName),
            ActionResult::Err("Select a single element to rename".to_string())
        );
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        state.doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 1.0, 0.0));
        state.apply(Action::ClickSceneElement {
            element: SceneElement::Line(0),
            additive: false,
        });
        assert_eq!(state.apply(Action::FocusElementName), ActionResult::Ok);
        assert!(state.context_pane.focus_name_field);
    }
}