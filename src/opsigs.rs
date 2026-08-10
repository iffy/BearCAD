//! Compile-time operation signatures.
//!
//! Each **operation variant** is its own zero-sized type implementing [`Operation`]:
//! inputs, outputs, shadows, and host-body effect are associated constants. The
//! commit path for body attachment reads those constants (e.g.
//! [`ExtrudeMerge::HOST_EFFECT`]), so the table `opsigs` prints cannot disagree
//! with what the code does without a compile or unit-test failure.
//!
//! `bearcad opsigs` / `cargo opsigs` walks [`ALL_OPERATIONS`] and prints markdown + HTML.

use crate::actions::{ExtrudeBodyMode, Tool};
use crate::model::MirrorMode;

// ── Element vocabulary ──────────────────────────────────────────────────────

/// Element kinds on an operation boundary (inputs, outputs, shadows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementType {
    Body,
    Face,
    Sketch,
    Line,
    Circle,
    ConstructionPlane,
    Constraint,
    SketchText,
    Shape,
    Axis,
    Point,
    DrawingView,
    Drawing,
    Joint,
    Path,
    Image,
}

impl ElementType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Face => "face",
            Self::Sketch => "sketch",
            Self::Line => "line",
            Self::Circle => "circle",
            Self::ConstructionPlane => "construction plane",
            Self::Constraint => "constraint",
            Self::SketchText => "text",
            Self::Shape => "shape",
            Self::Axis => "axis",
            Self::Point => "point",
            Self::DrawingView => "drawing view",
            Self::Drawing => "drawing",
            Self::Joint => "joint",
            Self::Path => "path",
            Self::Image => "image",
        }
    }
}

/// What a body-producing op does to host bodies it targets on commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostBodyEffect {
    /// No host body, or hosts left live; create a standalone result.
    None,
    /// Shadow each host and produce a new result body (merge, Move, Combine, …).
    ShadowHostAndProduce,
    /// Mutate the host body in place (extrude cut, revolve add/cut into host, …).
    MutateHost,
}

/// Whether this variant runs in a sketch (2D) or on solid/model geometry (3D).
/// Tools with both modes (chamfer, fillet, mirror, select, …) contribute a row to each table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpSpace {
    /// Sketch plane / drawing sheet geometry.
    TwoD,
    /// 3D solid modeling.
    ThreeD,
}


// ── The shape every operation variant implements ────────────────────────────

/// Compile-time contract for one operation **variant** (e.g. extrude-merge ≠ extrude-cut).
///
/// Implement on a unit struct. Constants are the source of truth for I/O and for
/// body-attachment behaviour used at commit time.
pub trait Operation: 'static {
    /// Toolbar tool this variant belongs to.
    const TOOL: Tool;
    /// Distinguishes modes of the same tool; empty when the tool has one mode only.
    const VARIANT: &'static str;
    const INPUTS: &'static [ElementType];
    const OUTPUTS: &'static [ElementType];
    /// Host elements that become shadows on commit (subset of inputs).
    const SHADOWS: &'static [ElementType];
    const HOST_EFFECT: HostBodyEffect;
    /// Sketch (2D) or solid (3D) table this variant belongs in.
    const SPACE: OpSpace;
}

pub fn tool_label(tool: Tool) -> &'static str {
    match tool {
        Tool::Select => "Select",
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
        Tool::Offset => "Offset",
        Tool::Project => "Projection",
        Tool::Loft => "Loft",
        Tool::Revolve => "Revolve",
        Tool::Shape => "Shape",
        Tool::Sweep => "Sweep",
        Tool::Combine => "Combine",
        Tool::Move => "Move",
        Tool::Mirror => "Mirror",
        Tool::Repeat => "Repeat",
        Tool::Slice => "Slice",
        Tool::Shell => "Shell",
        Tool::Joint => "Joint",
        Tool::Text => "Text",
        Tool::DrawingAdd => "Drawing projection",
        Tool::DrawingAlign => "Drawing align",
    }
}

// ── Type-erased view for listing / printing (built only from Operation impls) ─

/// Erased snapshot of an [`Operation`]'s constants — for tables and CLI only.
#[derive(Clone, Copy, Debug)]
pub struct OpSig {
    pub tool: Tool,
    pub variant: &'static str,
    pub inputs: &'static [ElementType],
    pub outputs: &'static [ElementType],
    pub shadows: &'static [ElementType],
    pub host_effect: HostBodyEffect,
    pub space: OpSpace,
}

impl OpSig {
    pub const fn from_op<O: Operation>() -> Self {
        Self {
            tool: O::TOOL,
            variant: O::VARIANT,
            inputs: O::INPUTS,
            outputs: O::OUTPUTS,
            shadows: O::SHADOWS,
            host_effect: O::HOST_EFFECT,
            space: O::SPACE,
        }
    }

    pub fn tool_name(self) -> &'static str {
        tool_label(self.tool)
    }
}

/// Lift one `Operation` implementor into the list used by `opsigs`.
const fn sig<O: Operation>() -> OpSig {
    OpSig::from_op::<O>()
}

// ── Extrude variants (one type each) ────────────────────────────────────────

/// Extrude → new body per profile group.
pub struct ExtrudeNewBody;
impl Operation for ExtrudeNewBody {
    const TOOL: Tool = Tool::Extrude;
    const VARIANT: &'static str = "new body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

/// Extrude → one body holding every profile (`body = "join"`).
pub struct ExtrudeJoinProfiles;
impl Operation for ExtrudeJoinProfiles {
    const TOOL: Tool = Tool::Extrude;
    const VARIANT: &'static str = "join profiles";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

/// Extrude merge into a host body: shadows host, produces combined solid (#1106).
pub struct ExtrudeMerge;
impl Operation for ExtrudeMerge {
    const TOOL: Tool = Tool::Extrude;
    const VARIANT: &'static str = "merge into body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

/// Extrude cut: mutates host in place (#35).
pub struct ExtrudeCut;
impl Operation for ExtrudeCut {
    const TOOL: Tool = Tool::Extrude;
    const VARIANT: &'static str = "cut body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::MutateHost;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

/// Body-attachment effect for a live [`ExtrudeBodyMode`] — **only** source of truth
/// for whether commit shadows, mutates, or creates a standalone body.
pub fn extrude_host_effect(mode: ExtrudeBodyMode) -> HostBodyEffect {
    match mode {
        ExtrudeBodyMode::NewBody => ExtrudeNewBody::HOST_EFFECT,
        ExtrudeBodyMode::JoinNew => ExtrudeJoinProfiles::HOST_EFFECT,
        ExtrudeBodyMode::MergeInto(_) => ExtrudeMerge::HOST_EFFECT,
        ExtrudeBodyMode::Cut(_) => ExtrudeCut::HOST_EFFECT,
    }
}

// ── Mirror variants ─────────────────────────────────────────────────────────

/// In-sketch mirror: reflect lines/circles across a sketch line (#528).
pub struct MirrorSketch;
impl Operation for MirrorSketch {
    const TOOL: Tool = Tool::Mirror;
    const VARIANT: &'static str = "sketch";
    const INPUTS: &'static [ElementType] = &[ElementType::Line, ElementType::Circle];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Line, ElementType::Circle];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct MirrorNewBody;
impl Operation for MirrorNewBody {
    const TOOL: Tool = Tool::Mirror;
    const VARIANT: &'static str = "new body";
    const INPUTS: &'static [ElementType] = &[ElementType::Body, ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct MirrorJoin;
impl Operation for MirrorJoin {
    const TOOL: Tool = Tool::Mirror;
    const VARIANT: &'static str = "join";
    const INPUTS: &'static [ElementType] = &[ElementType::Body, ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct MirrorCut;
impl Operation for MirrorCut {
    const TOOL: Tool = Tool::Mirror;
    const VARIANT: &'static str = "cut";
    const INPUTS: &'static [ElementType] = &[ElementType::Body, ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub fn mirror_host_effect(mode: MirrorMode) -> HostBodyEffect {
    match mode {
        MirrorMode::NewBody => MirrorNewBody::HOST_EFFECT,
        MirrorMode::Join => MirrorJoin::HOST_EFFECT,
        MirrorMode::Cut => MirrorCut::HOST_EFFECT,
    }
}

// ── Other tools (one or more variant types each) ────────────────────────────

pub struct Select2d;
impl Operation for Select2d {
    const TOOL: Tool = Tool::Select;
    const VARIANT: &'static str = "sketch";
    const INPUTS: &'static [ElementType] = &[];
    const OUTPUTS: &'static [ElementType] = &[];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct Select3d;
impl Operation for Select3d {
    const TOOL: Tool = Tool::Select;
    const VARIANT: &'static str = "model";
    const INPUTS: &'static [ElementType] = &[];
    const OUTPUTS: &'static [ElementType] = &[];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Rectangle;
impl Operation for Rectangle {
    const TOOL: Tool = Tool::Rectangle;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Line];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct Line;
impl Operation for Line {
    const TOOL: Tool = Tool::Line;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Line];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct Circle;
impl Operation for Circle {
    const TOOL: Tool = Tool::Circle;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Circle];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct ConstructionPlane;
impl Operation for ConstructionPlane {
    const TOOL: Tool = Tool::ConstructionPlane;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Face, ElementType::Axis, ElementType::Point];
    const OUTPUTS: &'static [ElementType] = &[ElementType::ConstructionPlane];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Sketch;
impl Operation for Sketch {
    const TOOL: Tool = Tool::Sketch;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Sketch];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Dimension;
impl Operation for Dimension {
    const TOOL: Tool = Tool::Dimension;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Line, ElementType::Circle, ElementType::Point];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Constraint];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct Constraint;
impl Operation for Constraint {
    const TOOL: Tool = Tool::Constraint;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Line, ElementType::Circle, ElementType::Point];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Constraint];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct ChamferSketch;
impl Operation for ChamferSketch {
    const TOOL: Tool = Tool::Chamfer;
    const VARIANT: &'static str = "sketch";
    const INPUTS: &'static [ElementType] = &[ElementType::Line, ElementType::Point];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Line];
    const SHADOWS: &'static [ElementType] = &[ElementType::Line];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct ChamferBodyEdges;
impl Operation for ChamferBodyEdges {
    const TOOL: Tool = Tool::Chamfer;
    const VARIANT: &'static str = "body edges";
    const INPUTS: &'static [ElementType] = &[ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct FilletSketch;
impl Operation for FilletSketch {
    const TOOL: Tool = Tool::Fillet;
    const VARIANT: &'static str = "sketch";
    const INPUTS: &'static [ElementType] = &[ElementType::Line, ElementType::Point];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Line];
    const SHADOWS: &'static [ElementType] = &[ElementType::Line];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct FilletBodyEdges;
impl Operation for FilletBodyEdges {
    const TOOL: Tool = Tool::Fillet;
    const VARIANT: &'static str = "body edges";
    const INPUTS: &'static [ElementType] = &[ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Offset;
impl Operation for Offset {
    const TOOL: Tool = Tool::Offset;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Line, ElementType::Circle, ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Line, ElementType::Circle];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct Project;
impl Operation for Project {
    const TOOL: Tool = Tool::Project;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[
        ElementType::Body,
        ElementType::Line,
        ElementType::ConstructionPlane,
    ];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Line];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct LoftNewBody;
impl Operation for LoftNewBody {
    const TOOL: Tool = Tool::Loft;
    const VARIANT: &'static str = "new body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct LoftAddToBody;
impl Operation for LoftAddToBody {
    const TOOL: Tool = Tool::Loft;
    const VARIANT: &'static str = "add to body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::MutateHost;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct LoftCutBody;
impl Operation for LoftCutBody {
    const TOOL: Tool = Tool::Loft;
    const VARIANT: &'static str = "cut body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::MutateHost;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct RevolveNewBody;
impl Operation for RevolveNewBody {
    const TOOL: Tool = Tool::Revolve;
    const VARIANT: &'static str = "new body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face, ElementType::Axis];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct RevolveAddToBody;
impl Operation for RevolveAddToBody {
    const TOOL: Tool = Tool::Revolve;
    const VARIANT: &'static str = "add to body";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Face, ElementType::Axis, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::MutateHost;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct RevolveCutBody;
impl Operation for RevolveCutBody {
    const TOOL: Tool = Tool::Revolve;
    const VARIANT: &'static str = "cut body";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Face, ElementType::Axis, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::MutateHost;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Shape;
impl Operation for Shape {
    const TOOL: Tool = Tool::Shape;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Shape, ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct SweepNewBody;
impl Operation for SweepNewBody {
    const TOOL: Tool = Tool::Sweep;
    const VARIANT: &'static str = "new body";
    const INPUTS: &'static [ElementType] = &[ElementType::Face, ElementType::Path];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct SweepAddToBody;
impl Operation for SweepAddToBody {
    const TOOL: Tool = Tool::Sweep;
    const VARIANT: &'static str = "add to body";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Face, ElementType::Path, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::MutateHost;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct SweepCutBody;
impl Operation for SweepCutBody {
    const TOOL: Tool = Tool::Sweep;
    const VARIANT: &'static str = "cut body";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Face, ElementType::Path, ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::MutateHost;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct CombineUnion;
impl Operation for CombineUnion {
    const TOOL: Tool = Tool::Combine;
    const VARIANT: &'static str = "union";
    const INPUTS: &'static [ElementType] = &[ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct CombineCut;
impl Operation for CombineCut {
    const TOOL: Tool = Tool::Combine;
    const VARIANT: &'static str = "cut";
    const INPUTS: &'static [ElementType] = &[ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct CombineIntersect;
impl Operation for CombineIntersect {
    const TOOL: Tool = Tool::Combine;
    const VARIANT: &'static str = "intersect";
    const INPUTS: &'static [ElementType] = &[ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct CombineDifference;
impl Operation for CombineDifference {
    const TOOL: Tool = Tool::Combine;
    const VARIANT: &'static str = "difference";
    const INPUTS: &'static [ElementType] = &[ElementType::Body];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Move;
impl Operation for Move {
    const TOOL: Tool = Tool::Move;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[
        ElementType::Body,
        ElementType::ConstructionPlane,
        ElementType::Image,
        ElementType::Point,
    ];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Repeat;
impl Operation for Repeat {
    const TOOL: Tool = Tool::Repeat;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[
        ElementType::Body,
        ElementType::Axis,
        ElementType::ConstructionPlane,
        ElementType::Sketch,
    ];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Slice;
impl Operation for Slice {
    const TOOL: Tool = Tool::Slice;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[
        ElementType::Body,
        ElementType::Face,
        ElementType::ConstructionPlane,
        // Laser-path cutters (#1126): the sketch lines, and the sketch that owns them (#1151).
        ElementType::Line,
        ElementType::Sketch,
    ];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Shell;
impl Operation for Shell {
    const TOOL: Tool = Tool::Shell;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[ElementType::Body, ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Body];
    const SHADOWS: &'static [ElementType] = &[ElementType::Body];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::ShadowHostAndProduce;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Joint;
impl Operation for Joint {
    const TOOL: Tool = Tool::Joint;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Body, ElementType::Point, ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::Joint];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::ThreeD;
}

pub struct Text;
impl Operation for Text {
    const TOOL: Tool = Tool::Text;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[ElementType::Face];
    const OUTPUTS: &'static [ElementType] = &[ElementType::SketchText];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct DrawingAdd;
impl Operation for DrawingAdd {
    const TOOL: Tool = Tool::DrawingAdd;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] =
        &[ElementType::Body, ElementType::Sketch, ElementType::Drawing];
    const OUTPUTS: &'static [ElementType] = &[ElementType::DrawingView];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

pub struct DrawingAlign;
impl Operation for DrawingAlign {
    const TOOL: Tool = Tool::DrawingAlign;
    const VARIANT: &'static str = "";
    const INPUTS: &'static [ElementType] = &[ElementType::DrawingView];
    const OUTPUTS: &'static [ElementType] = &[ElementType::DrawingView];
    const SHADOWS: &'static [ElementType] = &[];
    const HOST_EFFECT: HostBodyEffect = HostBodyEffect::None;
    const SPACE: OpSpace = OpSpace::TwoD;
}

// ── Gather list for opsigs (explicit; adding a type without listing it is a miss) ─

/// Every operation variant, for `opsigs` and coverage tests.
///
/// New `impl Operation` types must be added here — the
/// [`every_tool_has_an_operation`] test fails if a [`Tool`] is left out.
pub static ALL_OPERATIONS: &[OpSig] = &[
    sig::<Select2d>(),
    sig::<Select3d>(),
    sig::<Rectangle>(),
    sig::<Line>(),
    sig::<Circle>(),
    sig::<ConstructionPlane>(),
    sig::<Sketch>(),
    sig::<Dimension>(),
    sig::<Constraint>(),
    sig::<ExtrudeNewBody>(),
    sig::<ExtrudeJoinProfiles>(),
    sig::<ExtrudeMerge>(),
    sig::<ExtrudeCut>(),
    sig::<ChamferSketch>(),
    sig::<ChamferBodyEdges>(),
    sig::<FilletSketch>(),
    sig::<FilletBodyEdges>(),
    sig::<Offset>(),
    sig::<Project>(),
    sig::<LoftNewBody>(),
    sig::<LoftAddToBody>(),
    sig::<LoftCutBody>(),
    sig::<RevolveNewBody>(),
    sig::<RevolveAddToBody>(),
    sig::<RevolveCutBody>(),
    sig::<Shape>(),
    sig::<SweepNewBody>(),
    sig::<SweepAddToBody>(),
    sig::<SweepCutBody>(),
    sig::<CombineUnion>(),
    sig::<CombineCut>(),
    sig::<CombineIntersect>(),
    sig::<CombineDifference>(),
    sig::<Move>(),
    sig::<MirrorSketch>(),
    sig::<MirrorNewBody>(),
    sig::<MirrorJoin>(),
    sig::<MirrorCut>(),
    sig::<Repeat>(),
    sig::<Slice>(),
    sig::<Shell>(),
    sig::<Joint>(),
    sig::<Text>(),
    sig::<DrawingAdd>(),
    sig::<DrawingAlign>(),
];

// ── Render / CLI ────────────────────────────────────────────────────────────

fn format_types(types: &[ElementType]) -> String {
    if types.is_empty() {
        return "—".to_string();
    }
    types
        .iter()
        .map(|t| t.label())
        .collect::<Vec<_>>()
        .join(", ")
}

fn host_effect_label(effect: HostBodyEffect) -> &'static str {
    match effect {
        HostBodyEffect::None => "—",
        HostBodyEffect::ShadowHostAndProduce => "shadow host → new body",
        HostBodyEffect::MutateHost => "mutate host",
    }
}

fn render_table_md(ops: &[&OpSig]) -> String {
    let mut out = String::from(
        "| Tool | Variant | Inputs | Outputs | Shadows | Host effect |\n\
         | --- | --- | --- | --- | --- | --- |\n",
    );
    for sig in ops {
        let variant = if sig.variant.is_empty() {
            "—"
        } else {
            sig.variant
        };
        let shadows = if sig.shadows.is_empty() {
            "—".to_string()
        } else {
            format_types(sig.shadows)
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            sig.tool_name(),
            variant,
            format_types(sig.inputs),
            format_types(sig.outputs),
            shadows,
            host_effect_label(sig.host_effect),
        ));
    }
    out
}

fn ops_in_space(space: OpSpace) -> Vec<&'static OpSig> {
    ALL_OPERATIONS
        .iter()
        .filter(|s| s.space == space)
        .collect()
}

/// Markdown: one table for 2D (sketch) ops, one for 3D (solid) ops.
pub fn render_markdown() -> String {
    let mut out = String::from("# BearCAD operation signatures\n\n");
    out.push_str("## 2D (sketch)\n\n");
    out.push_str(&render_table_md(&ops_in_space(OpSpace::TwoD)));
    out.push_str("\n## 3D (solid)\n\n");
    out.push_str(&render_table_md(&ops_in_space(OpSpace::ThreeD)));
    out
}

fn render_table_html(ops: &[&OpSig]) -> String {
    let mut out = String::from(
        "<table>\n<thead><tr>\
         <th>Tool</th><th>Variant</th><th>Inputs</th><th>Outputs</th>\
         <th>Shadows</th><th>Host effect</th>\
         </tr></thead>\n<tbody>\n",
    );
    for sig in ops {
        let variant = if sig.variant.is_empty() {
            "—"
        } else {
            sig.variant
        };
        let shadows = if sig.shadows.is_empty() {
            "—".to_string()
        } else {
            format_types(sig.shadows)
        };
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            html_escape(sig.tool_name()),
            html_escape(variant),
            html_escape(&format_types(sig.inputs)),
            html_escape(&format_types(sig.outputs)),
            html_escape(&shadows),
            html_escape(host_effect_label(sig.host_effect)),
        ));
    }
    out.push_str("</tbody>\n</table>\n");
    out
}

/// HTML document with separate 2D and 3D tables.
pub fn render_html() -> String {
    let mut out = String::from(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>BearCAD operation signatures</title>\
         <style>\
         body{font-family:system-ui,sans-serif;margin:2rem;}\
         table{border-collapse:collapse;width:100%;margin-bottom:2rem;}\
         th,td{border:1px solid #ccc;padding:0.4rem 0.6rem;text-align:left;}\
         th{background:#f4f4f4;}\
         tr:nth-child(even){background:#fafafa;}\
         </style></head><body>\n\
         <h1>BearCAD operation signatures</h1>\n",
    );
    out.push_str("<h2>2D (sketch)</h2>\n");
    out.push_str(&render_table_html(&ops_in_space(OpSpace::TwoD)));
    out.push_str("<h2>3D (solid)</h2>\n");
    out.push_str(&render_table_html(&ops_in_space(OpSpace::ThreeD)));
    out.push_str("</body></html>\n");
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Print signatures (`bearcad opsigs` / `cargo opsigs`). Markdown by default;
/// pass `html: true` (`--html`) for an HTML document instead.
pub fn run_cli(html: bool) {
    if html {
        print!("{}", render_html());
    } else {
        print!("{}", render_markdown());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Tool;
    use std::collections::HashSet;

    #[test]
    fn every_tool_has_an_operation() {
        let covered: HashSet<Tool> = ALL_OPERATIONS.iter().map(|s| s.tool).collect();
        for tool in Tool::ALL {
            assert!(
                covered.contains(&tool),
                "{tool:?} has no Operation type listed in ALL_OPERATIONS"
            );
        }
    }

    #[test]
    fn extrude_variants_are_distinct_types_with_correct_effects() {
        assert_eq!(ExtrudeNewBody::HOST_EFFECT, HostBodyEffect::None);
        assert_eq!(ExtrudeJoinProfiles::HOST_EFFECT, HostBodyEffect::None);
        assert_eq!(
            ExtrudeMerge::HOST_EFFECT,
            HostBodyEffect::ShadowHostAndProduce
        );
        assert_eq!(ExtrudeCut::HOST_EFFECT, HostBodyEffect::MutateHost);
        assert!(ExtrudeMerge::INPUTS.contains(&ElementType::Body));
        assert!(ExtrudeMerge::SHADOWS.contains(&ElementType::Body));
        assert!(ExtrudeCut::SHADOWS.is_empty());
    }

    #[test]
    fn extrude_body_mode_effect_is_the_operation_constant() {
        let bi = crate::model::BodyKey::from_bits(0);
        assert_eq!(
            extrude_host_effect(ExtrudeBodyMode::NewBody),
            ExtrudeNewBody::HOST_EFFECT
        );
        assert_eq!(
            extrude_host_effect(ExtrudeBodyMode::JoinNew),
            ExtrudeJoinProfiles::HOST_EFFECT
        );
        assert_eq!(
            extrude_host_effect(ExtrudeBodyMode::MergeInto(bi)),
            ExtrudeMerge::HOST_EFFECT
        );
        assert_eq!(
            extrude_host_effect(ExtrudeBodyMode::Cut(bi)),
            ExtrudeCut::HOST_EFFECT
        );
    }

    #[test]
    fn mirror_consumes_input_matches_operation_host_effect() {
        for mode in [MirrorMode::NewBody, MirrorMode::Join, MirrorMode::Cut] {
            let effect = mirror_host_effect(mode);
            let consumes = matches!(effect, HostBodyEffect::ShadowHostAndProduce);
            assert_eq!(mode.consumes_input(), consumes, "{mode:?}");
        }
    }

    #[test]
    fn markdown_lists_extrude_merge_and_cut() {
        let md = render_markdown();
        assert!(md.contains("## 2D (sketch)"));
        assert!(md.contains("## 3D (solid)"));
        assert!(md.contains("merge into body"));
        assert!(md.contains("cut body"));
        assert!(md.contains("Extrude"));
        // Dual-space tools show up in both tables.
        assert!(md.find("Chamfer") != md.rfind("Chamfer"));
    }

    #[test]
    fn two_d_and_three_d_partition_all_ops() {
        let two = ops_in_space(OpSpace::TwoD);
        let three = ops_in_space(OpSpace::ThreeD);
        assert!(!two.is_empty());
        assert!(!three.is_empty());
        assert_eq!(two.len() + three.len(), ALL_OPERATIONS.len());
        assert!(two.iter().all(|s| s.space == OpSpace::TwoD));
        assert!(three.iter().all(|s| s.space == OpSpace::ThreeD));
    }

    #[test]
    fn shadow_effect_lists_shadows() {
        for sig in ALL_OPERATIONS {
            if sig.host_effect == HostBodyEffect::ShadowHostAndProduce {
                assert!(
                    !sig.shadows.is_empty(),
                    "{}: ShadowHostAndProduce requires SHADOWS",
                    sig.tool_name()
                );
            }
        }
    }
}
