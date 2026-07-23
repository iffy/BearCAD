//! Elements pane: construction planes, sketches, and sketch geometry.

/// Side-panel title shown in the UI.
pub const PANE_TITLE: &str = "Elements";

use crate::actions::SketchSession;
use crate::icons::{
    icon_button, icon_for_constraint_kind, icon_for_visibility, selectable_icon_button,
    sized_texture, IconId,
};
use crate::document_health::{DocumentHealth, HealthStatus};
use crate::document_lifecycle::{element_alive, sketch_alive};
use crate::model::{
    ConstraintEntity, ConstraintKind, ConstraintLine, ConstraintPoint, ConstructionPlaneParent,
    DistanceTarget, Document, FaceId, SketchId,
};
use crate::names;
use crate::selection::{additive_click_modifiers, SceneSelection};
use eframe::egui::{self, Color32, RichText};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A node in the scene hierarchy.
///
/// The derived `Ord` (variant order, then index) is the flat list's tiebreak among nodes with
/// no input-dependency relationship (#540): a stable, kind-then-index ordering that never
/// depends on when an element was created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HierarchyNode {
    /// Synthetic singleton root shown at the top of the Elements pane; every other
    /// top-level node (root construction planes, orphaned extrusions, orphaned bodies)
    /// nests under it. It carries no index — there is exactly one per document — and has
    /// no corresponding [`SceneElement`]: it isn't individually selectable, hideable, or
    /// otherwise dispatched through the scene graph (see [`scene_element_for_node`]).
    Document,
    ConstructionPlane(usize),
    Sketch(SketchId),
    Line(usize),
    Circle(usize),
    Constraint(usize),
    Extrusion(usize),
    Body(usize),
    /// A tracing image (#163/#169).
    Image(usize),
    /// A boolean operation between bodies (Combine tool); its output bodies nest under it.
    BooleanOp(usize),
    /// A move operation on bodies (Move tool); its output bodies nest under it.
    MoveOp(usize),
    /// A mirror operation on bodies (Mirror tool, #523); its reflected bodies nest under it.
    MirrorOp(usize),
    /// A linear repeat on bodies (Repeat tool); its output bodies nest under it.
    RepeatOp(usize),
    /// A 2D in-sketch linear repeat (#222/#228); its duplicated lines/circles nest under it.
    SketchRepeatOp(usize),
    SketchOffsetOp(usize),
    /// A 2D in-sketch mirror (#523); its reflected lines/circles nest under it.
    SketchMirrorOp(usize),
    /// A 2D in-sketch chamfer/fillet (#538); its trimmed copies + bridge lines nest under it.
    SketchVertexTreatmentOp(usize),
    /// A 2D in-sketch slice (#224/#229); its fragment lines nest under it.
    SketchSliceOp(usize),
    /// A sketch text element (#282/#286); nests under its sketch like a line.
    SketchText(usize),
    /// A slice operation on bodies (Slice tool); its fragment bodies nest under it.
    SliceOp(usize),
    /// An edge chamfer/fillet operation on bodies (#531); its beveled output bodies nest under
    /// it and its input bodies + treated edges feed it as graph inputs.
    EdgeTreatmentOp(usize),
    /// A revolved solid (Revolve tool); its output body nests under it (#211).
    Revolution(usize),
    /// A sweep (Sweep tool); its output body nests under it.
    SweepOp(usize),
    /// A loft (Loft tool): its output body nests under it, and its cross-section sketches feed
    /// it as graph inputs (#252). Display-only for now (no `SceneElement`).
    Loft(usize),
    /// A technical drawing (#180). A display-only top-level leaf (no [`SceneElement`], like
    /// [`HierarchyNode::Document`]): it has its own icon and is right-clickable to edit
    /// (opening the drawing pane), but isn't a selectable/hideable scene element.
    Drawing(usize),
    /// A 3D edge chamfer/fillet applied to an extrusion (#77); `index` is into that
    /// extrusion's `edge_treatments`. A display-only leaf (like [`HierarchyNode::Document`]
    /// it has no [`SceneElement`]): it nests under its extrusion and is right-clickable to
    /// edit its amount after the fact (#192), but isn't individually selectable/hideable.
    EdgeTreatment { extrusion: usize, index: usize },
    /// A body/sketch **projection** placed on a technical drawing (#281): a display-only leaf
    /// nested under its [`HierarchyNode::Drawing`]. `view` indexes the drawing's `views`. It has
    /// no [`SceneElement`] (not selectable/hideable through the scene graph); its source
    /// body/sketch is a second input, surfaced once the element graph (#252) lands.
    DrawingProjection { drawing: usize, view: usize },
    /// A component (#423): a named group row whose member roots nest beneath it; components
    /// nest inside each other via their `parent` link.
    Component(usize),
    /// A text note on a drawing page (#333), nested under its [`HierarchyNode::Drawing`].
    /// `annotation` indexes the drawing's `annotations`. Like a projection it's a display-only
    /// leaf with no [`SceneElement`]; clicking it opens the drawing.
    DrawingAnnotation { drawing: usize, annotation: usize },
    /// A length dimension shown on a projection (#341), nested under its
    /// [`HierarchyNode::DrawingProjection`]. `a`/`b` are the dimensioned edge's quantized world
    /// endpoints. A display-only leaf; clicking it opens the drawing and selects the dimension.
    DrawingDimension { drawing: usize, view: usize, a: [i32; 3], b: [i32; 3] },
}

/// Identifies an element whose visibility can be toggled.
///
/// Not `Copy` — see [`crate::model::ConstraintPoint`]'s doc comment: `Point` embeds a
/// `ConstraintPoint`, which embeds a `FaceId` for `FaceVertex` (#26/#27), and `FaceId` isn't
/// `Copy`. Callers that used to rely on implicit copies now need an explicit `.clone()`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SceneElement {
    ConstructionPlane(usize),
    Sketch(SketchId),
    Line(usize),
    Circle(usize),
    Point(ConstraintPoint),
    Constraint(usize),
    Extrusion(usize),
    Body(usize),
    /// An edge of an extrusion-backed body face's own boundary loop (#26/#27), for
    /// constraint-authoring selection — mirrors `Point` wrapping the whole `ConstraintPoint`
    /// enum; only ever constructed with `ConstraintLine::FaceEdge` (the `Line`
    /// variant already has its own dedicated `SceneElement::Line`).
    FaceEdge(ConstraintLine),
    /// One feature edge of a body's solid mesh, selectable in 3D select mode (#156).
    /// Identified by its quantized world endpoints (see [`quantize_body_point`]) — a
    /// transient, geometry-keyed identity: if a rebuild moves the edge, the selection simply
    /// drops, which is acceptable for ephemeral (never persisted) selection state.
    BodyEdge {
        body: usize,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// A corner of a body's solid mesh, selectable in 3D select mode (#156); quantized like
    /// [`SceneElement::BodyEdge`].
    BodyVertex { body: usize, p: [i32; 3] },
    /// A planar face of a body's solid mesh, selectable in 3D select mode (#555/#557). A face
    /// has no stable index, so — like [`SceneElement::BodyEdge`] — its identity is its quantized
    /// geometry: the average of its triangle vertices (`centroid`) plus its `normal`, both
    /// quantized via [`quantize_body_point`]. Deterministic mesh → deterministic key, so two
    /// picks of the same face compare equal; a rebuild that moves the face simply drops the
    /// (ephemeral, never persisted) selection.
    BodyFace {
        body: usize,
        centroid: [i32; 3],
        normal: [i32; 3],
    },
    /// A tracing image (#163/#169).
    Image(usize),
    /// A boolean operation between bodies (Combine tool).
    BooleanOp(usize),
    /// A move operation on bodies (Move tool).
    MoveOp(usize),
    /// A mirror operation on bodies (Mirror tool, #523).
    MirrorOp(usize),
    /// A linear repeat on bodies (Repeat tool).
    RepeatOp(usize),
    /// A 2D in-sketch linear repeat (#222/#228).
    SketchRepeatOp(usize),
    SketchOffsetOp(usize),
    /// A 2D in-sketch mirror (#523); its reflected lines/circles nest under it.
    SketchMirrorOp(usize),
    /// A 2D in-sketch chamfer/fillet (#538); its trimmed copies + bridge lines nest under it.
    SketchVertexTreatmentOp(usize),
    /// A 2D in-sketch slice (#224/#229).
    SketchSliceOp(usize),
    /// A sketch text element (#282): selecting it selects the whole text.
    SketchText(usize),
    /// A slice operation on bodies (Slice tool).
    SliceOp(usize),
    /// An edge chamfer/fillet operation on bodies (#531).
    EdgeTreatmentOp(usize),
    /// A revolved solid (Revolve tool, #211).
    Revolution(usize),
    /// A sweep (Sweep tool).
    SweepOp(usize),
    /// The origin, selectable in a sketch so a point can be constrained coincident to it from
    /// the constraint tool (#189). Fixed geometry with no owning entity, like `FaceEdge`.
    Origin,
    /// A component (#423): a named, nestable group of top-level elements. Hiding one hides
    /// everything inside it.
    Component(usize),
}

/// Quantize a world position (mm) to the 0.01 mm grid used for body edge/vertex selection
/// identity (#156) — fine enough that distinct vertices never collide, coarse enough that
/// float noise across frames maps to the same key.
pub fn quantize_body_point(p: glam::Vec3) -> [i32; 3] {
    [
        (p.x * 100.0).round() as i32,
        (p.y * 100.0).round() as i32,
        (p.z * 100.0).round() as i32,
    ]
}

/// Invert [`quantize_body_point`] for rendering the selected edge/vertex highlight.
pub fn dequantize_body_point(p: [i32; 3]) -> glam::Vec3 {
    glam::Vec3::new(p[0] as f32 / 100.0, p[1] as f32 / 100.0, p[2] as f32 / 100.0)
}

/// The [`SceneElement`] a hierarchy node dispatches through for selection, visibility,
/// and health lookups — `None` for [`HierarchyNode::Document`], the synthetic root, which
/// has no independent selectable/hideable identity of its own.
pub fn scene_element_for_node(node: HierarchyNode) -> Option<SceneElement> {
    Some(match node {
        // Display-only leaves with no independent selectable/hideable identity (#192/#180).
        HierarchyNode::Document
        | HierarchyNode::EdgeTreatment { .. }
        | HierarchyNode::Drawing(_)
        | HierarchyNode::DrawingProjection { .. }
        | HierarchyNode::DrawingAnnotation { .. }
        | HierarchyNode::DrawingDimension { .. }
        | HierarchyNode::Loft(_) => return None,
        HierarchyNode::ConstructionPlane(i) => SceneElement::ConstructionPlane(i),
        HierarchyNode::Sketch(i) => SceneElement::Sketch(i),
        HierarchyNode::Line(i) => SceneElement::Line(i),
        HierarchyNode::Circle(i) => SceneElement::Circle(i),
        HierarchyNode::Constraint(i) => SceneElement::Constraint(i),
        HierarchyNode::Extrusion(i) => SceneElement::Extrusion(i),
        HierarchyNode::Body(i) => SceneElement::Body(i),
        HierarchyNode::Image(i) => SceneElement::Image(i),
        HierarchyNode::BooleanOp(i) => SceneElement::BooleanOp(i),
        HierarchyNode::MoveOp(i) => SceneElement::MoveOp(i),
        HierarchyNode::MirrorOp(i) => SceneElement::MirrorOp(i),
        HierarchyNode::RepeatOp(i) => SceneElement::RepeatOp(i),
        HierarchyNode::SketchRepeatOp(i) => SceneElement::SketchRepeatOp(i),
        HierarchyNode::SketchOffsetOp(i) => SceneElement::SketchOffsetOp(i),
        HierarchyNode::SketchMirrorOp(i) => SceneElement::SketchMirrorOp(i),
        HierarchyNode::SketchVertexTreatmentOp(i) => SceneElement::SketchVertexTreatmentOp(i),
        HierarchyNode::SketchSliceOp(i) => SceneElement::SketchSliceOp(i),
        HierarchyNode::SketchText(i) => SceneElement::SketchText(i),
        HierarchyNode::SliceOp(i) => SceneElement::SliceOp(i),
        HierarchyNode::EdgeTreatmentOp(i) => SceneElement::EdgeTreatmentOp(i),
        HierarchyNode::Revolution(i) => SceneElement::Revolution(i),
        HierarchyNode::SweepOp(i) => SceneElement::SweepOp(i),
        HierarchyNode::Component(i) => SceneElement::Component(i),
    })
}

/// The [`SceneElement`] for an operation whose editing is opened the **universal** way — a
/// double-click on the row or a right-click → "Edit" (#546) — reloading it into its tool. `None`
/// for elements edited through their own dedicated entry (sketches, planes, extrusions, edge
/// treatments, drawings) or that aren't operations at all.
pub fn node_editable_operation(node: HierarchyNode) -> Option<SceneElement> {
    match node {
        HierarchyNode::BooleanOp(i) => Some(SceneElement::BooleanOp(i)),
        HierarchyNode::MoveOp(i) => Some(SceneElement::MoveOp(i)),
        HierarchyNode::MirrorOp(i) => Some(SceneElement::MirrorOp(i)),
        HierarchyNode::RepeatOp(i) => Some(SceneElement::RepeatOp(i)),
        HierarchyNode::SliceOp(i) => Some(SceneElement::SliceOp(i)),
        HierarchyNode::Revolution(i) => Some(SceneElement::Revolution(i)),
        HierarchyNode::SweepOp(i) => Some(SceneElement::SweepOp(i)),
        HierarchyNode::SketchMirrorOp(i) => Some(SceneElement::SketchMirrorOp(i)),
        HierarchyNode::SketchOffsetOp(i) => Some(SceneElement::SketchOffsetOp(i)),
        _ => None,
    }
}

/// Drag-and-drop payload for dragging an Elements-pane row onto the open drawing page (#290):
/// the dragged body/sketch becomes a projection at the drop point.
#[derive(Clone, Debug)]
pub struct DrawingDragPayload(pub SceneElement);

/// Drag-and-drop payload for dragging an Elements-pane row onto a component row (#423):
/// the dragged element moves into that component.
#[derive(Clone, Debug)]
pub struct ComponentDragPayload(pub SceneElement);

/// User-toggled visibility for scene elements. Absent entries are visible.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElementVisibility {
    hidden: HashSet<SceneElement>,
}

impl ElementVisibility {
    pub fn is_visible(&self, element: SceneElement) -> bool {
        !self.hidden.contains(&element)
    }

    pub fn set_visible(&mut self, element: SceneElement, visible: bool) {
        if visible {
            self.hidden.remove(&element);
        } else {
            self.hidden.insert(element);
        }
    }

    pub fn toggle(&mut self, element: SceneElement) -> bool {
        let next = !self.is_visible(element.clone());
        self.set_visible(element, next);
        next
    }

    /// Hide every element in `extra` on top of the current toggles (#524): the rollback
    /// marker builds a render-only visibility that suppresses everything created after it.
    pub fn with_hidden(&self, extra: &HashSet<SceneElement>) -> Self {
        let mut merged = self.clone();
        merged.hidden.extend(extra.iter().cloned());
        merged
    }

    /// Whether `component` and all its ancestors are individually visible (#423).
    fn component_chain_visible(&self, doc: &Document, component: usize) -> bool {
        doc.component_chain(component)
            .into_iter()
            .all(|c| self.is_visible(SceneElement::Component(c)))
    }

    pub fn effective_visible(&self, doc: &Document, element: SceneElement) -> bool {
        if !self.is_visible(element.clone()) {
            return false;
        }
        // A hidden component hides everything inside it (#423): resolve the element's
        // owning component (directly, or through the root element it nests under) and
        // require the whole chain visible.
        if let Some(c) = owning_component(doc, &element) {
            if !self.component_chain_visible(doc, c) {
                return false;
            }
        }
        match element {
            SceneElement::Component(index) => doc
                .components
                .get(index)
                .and_then(|c| c.parent)
                .is_none_or(|p| self.effective_visible(doc, SceneElement::Component(p))),
            SceneElement::ConstructionPlane(index) => doc
                .construction_planes
                .get(index)
                .map(|plane| match plane.parent {
                    ConstructionPlaneParent::Root => true,
                    ConstructionPlaneParent::Sketch(sketch) => {
                        self.effective_visible(doc, SceneElement::Sketch(sketch))
                    }
                })
                .unwrap_or(true),
            SceneElement::Sketch(sketch) => doc
                .sketch_face(sketch)
                .is_some_and(|face| self.effective_visible(doc, face_element(face))),
            SceneElement::Line(index) => doc.lines.get(index).is_some_and(|line| {
                self.effective_visible(doc, SceneElement::Sketch(line.sketch))
            }),
            SceneElement::Circle(index) => doc.circles.get(index).is_some_and(|circle| {
                self.effective_visible(doc, SceneElement::Sketch(circle.sketch))
            }),
            SceneElement::Point(point) => point_effective_visible(self, doc, point),
            SceneElement::Constraint(index) => doc.constraints.get(index).is_some_and(|c| {
                self.effective_visible(doc, SceneElement::Sketch(c.sketch))
            }),
            SceneElement::Extrusion(index) => self.is_visible(SceneElement::Extrusion(index)),
            SceneElement::Body(index) => {
                self.is_visible(SceneElement::Body(index))
                    && doc.bodies.get(index).is_some_and(|body| {
                        // An imported body has no source extrusions — `any()` over the empty
                        // list must not read as "hidden" (it made STL/STEP bodies invisible
                        // to every effective-visibility consumer).
                        let extrusions = body.source.extrusion_indices();
                        extrusions.is_empty()
                            || extrusions.iter().any(|&ei| {
                                self.effective_visible(doc, SceneElement::Extrusion(ei))
                            })
                    })
            }
            // A face's own edge tracks the extrusion that produced its face, same as
            // `FaceVertex` in `point_effective_visible` below.
            SceneElement::FaceEdge(line) => {
                let extrusion = match &line {
                    ConstraintLine::FaceEdge { face, .. } => face.extrusion_index(),
                    ConstraintLine::Line(_) | ConstraintLine::OriginAxis(_) => None,
                };
                self.effective_visible(
                    doc,
                    SceneElement::Extrusion(extrusion.unwrap_or(usize::MAX)),
                )
            }
            // A body's own edge/vertex/face (#156/#555) is visible exactly when its body is.
            SceneElement::BodyEdge { body, .. }
            | SceneElement::BodyVertex { body, .. }
            | SceneElement::BodyFace { body, .. } => {
                self.effective_visible(doc, SceneElement::Body(body))
            }
            SceneElement::Image(index) => self.is_visible(SceneElement::Image(index)),
            // Boolean/move operations are pane-only elements with no viewport visibility
            // of their own (their outputs are ordinary bodies).
            SceneElement::BooleanOp(_) => true,
            SceneElement::MoveOp(_) => true,
            SceneElement::MirrorOp(_) => true,
            SceneElement::RepeatOp(_) => true,
            SceneElement::SketchRepeatOp(_) => true,
            SceneElement::SketchOffsetOp(_) => true,
            SceneElement::SketchMirrorOp(_) => true,
            SceneElement::SketchVertexTreatmentOp(_) => true,
            SceneElement::SketchSliceOp(_) => true,
            SceneElement::SketchText(index) => doc
                .sketch_texts
                .get(index)
                .is_some_and(|t| self.effective_visible(doc, SceneElement::Sketch(t.sketch))),
            SceneElement::SliceOp(_) => true,
            SceneElement::EdgeTreatmentOp(_) => true,
            SceneElement::Revolution(_) => true,
            SceneElement::SweepOp(_) => true,
            // The origin is always visible while sketching (#189).
            SceneElement::Origin => true,
        }
    }
}

fn point_effective_visible(
    visibility: &ElementVisibility,
    doc: &Document,
    point: ConstraintPoint,
) -> bool {
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => doc.lines.get(line).is_some_and(|entity| {
            visibility.effective_visible(doc, SceneElement::Sketch(entity.sketch))
        }),
        ConstraintPoint::CircleCenter(circle) => doc.circles.get(circle).is_some_and(|entity| {
            visibility.effective_visible(doc, SceneElement::Sketch(entity.sketch))
        }),
        // A face's own vertex tracks the extrusion that produced its face — same dependency
        // `face_element` gives a sketch placed on a body cap/side wall.
        ConstraintPoint::FaceVertex { face, .. } => visibility.effective_visible(
            doc,
            face.extrusion_index()
                .map(SceneElement::Extrusion)
                .unwrap_or(SceneElement::Extrusion(usize::MAX)),
        ),
        ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.get(text).is_some_and(|entity| {
                visibility.effective_visible(doc, SceneElement::Sketch(entity.sketch))
            })
        }
        ConstraintPoint::ImageCalibrationPoint { image, .. } => {
            visibility.effective_visible(doc, SceneElement::Image(image))
        }
    }
}

fn face_element(face: FaceId) -> SceneElement {
    match face {
        FaceId::ConstructionPlane(i) => SceneElement::ConstructionPlane(i),
        FaceId::Circle(i) => SceneElement::Circle(i),
        // A polygon face is just a closed loop of existing lines (#66); its visibility
        // tracks its first constituent line.
        FaceId::Polygon(lines) => SceneElement::Line(lines[0]),
        // A sketch on a body cap or side wall depends on the extrusion that produced it.
        FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
            SceneElement::Extrusion(extrusion)
        }
    }
}

/// A hierarchy entry with optional children (used to derive parent links).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HierarchyEntry {
    pub node: HierarchyNode,
    pub children: Vec<HierarchyEntry>,
}

/// Which layout the Elements pane renders its nodes in (#issue 34). This is an ephemeral UI
/// preference, not document data — it lives on `AppState` (alongside the other never-persisted
/// view state) so scripts can drive it via `bearcad.ui.elements_view` (#108), and is threaded
/// into [`show_pane`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HierarchyViewMode {
    /// Flat, topologically-sorted list (the pre-existing default view).
    #[default]
    List,
    /// The real nested tree, each level indented farther than its parent.
    Tree,
    /// A 2D node-link diagram: column = depth, row = position within that column.
    Graph,
}

impl HierarchyViewMode {
    /// Parse a script name (`bearcad.ui.elements_view("list"|"tree"|"graph")`, #108);
    /// mirrors `ShadingMode::from_name`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "list" => Some(Self::List),
            "tree" => Some(Self::Tree),
            "graph" => Some(Self::Graph),
            _ => None,
        }
    }

    pub fn script_name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Tree => "tree",
            Self::Graph => "graph",
        }
    }
}

/// One node's position in the graph-node view's deterministic column/row layout — pure data,
/// no `egui` types, so it's directly unit-testable. Column equals tree depth; row is the
/// node's sequential position within that column in tree-walk (pre-order, depth-first) order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphNodePosition {
    pub node: HierarchyNode,
    pub parent: Option<HierarchyNode>,
    pub depth: usize,
    pub column: usize,
    pub row: usize,
}

/// Compute the graph-node view's layout: depth-first walk of `tree`, assigning each node a
/// column (its depth) and a row (its sequential order within that column). Deterministic and
/// non-force-directed, per #34 — the whole graph is meant to fit horizontally by construction
/// (column count is bounded by tree depth), with height handled by vertical scrolling.
pub fn graph_node_positions(tree: &[HierarchyEntry]) -> Vec<GraphNodePosition> {
    fn walk(
        entry: &HierarchyEntry,
        depth: usize,
        parent: Option<HierarchyNode>,
        next_row_in_column: &mut HashMap<usize, usize>,
        out: &mut Vec<GraphNodePosition>,
    ) {
        // Components (#423) are drawn as areas encompassing their members, not as nodes:
        // pass through to the children at the same depth, keeping the outer parent.
        if matches!(entry.node, HierarchyNode::Component(_)) {
            for child in &entry.children {
                walk(child, depth, parent, next_row_in_column, out);
            }
            return;
        }
        let row = next_row_in_column.entry(depth).or_insert(0);
        let this_row = *row;
        *row += 1;
        out.push(GraphNodePosition {
            node: entry.node,
            parent,
            depth,
            column: depth,
            row: this_row,
        });
        for child in &entry.children {
            walk(child, depth + 1, Some(entry.node), next_row_in_column, out);
        }
    }

    let mut next_row_in_column = HashMap::new();
    let mut positions = Vec::new();
    for entry in tree {
        walk(entry, 0, None, &mut next_row_in_column, &mut positions);
    }
    positions
}

/// Per-component sets of graph nodes inside that component's subtree (#423), nested
/// components included in their ancestors' sets — the areas the Graph view shades.
pub fn component_node_sets(tree: &[HierarchyEntry]) -> Vec<(usize, HashSet<HierarchyNode>)> {
    fn collect_nodes(entry: &HierarchyEntry, out: &mut HashSet<HierarchyNode>) {
        if !matches!(entry.node, HierarchyNode::Component(_)) {
            out.insert(entry.node);
        }
        for child in &entry.children {
            collect_nodes(child, out);
        }
    }
    fn walk(entry: &HierarchyEntry, out: &mut Vec<(usize, HashSet<HierarchyNode>)>) {
        if let HierarchyNode::Component(ci) = entry.node {
            let mut nodes = HashSet::new();
            for child in &entry.children {
                collect_nodes(child, &mut nodes);
            }
            out.push((ci, nodes));
        }
        for child in &entry.children {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for entry in tree {
        walk(entry, &mut out);
    }
    out
}

/// `(input, consumer)` dependency pairs for the Graph view (#266/#281): relationships beyond the
/// single tree parent — an operation's **input** elements feeding it, and a drawing projection's
/// **source**. These become the input edges of the eventual full element graph (#252). Both
/// endpoints are [`HierarchyNode`]s; the renderer skips any pair whose nodes aren't on screen.
pub fn graph_dependency_edges(doc: &Document) -> Vec<(HierarchyNode, HierarchyNode)> {
    let mut edges = Vec::new();

    // Boolean operations consume their side-A/side-B input bodies (now shadows) (#266).
    for (oi, op) in doc.boolean_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &bi in op.a.iter().chain(op.b.iter()) {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::BooleanOp(oi)));
        }
    }
    // Move and Slice operations consume their input bodies too.
    for (oi, op) in doc.move_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::MoveOp(oi)));
        }
    }
    // A mirror consumes its input bodies (and its plane face's body, if any) — #523.
    for (oi, op) in doc.mirror_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::MirrorOp(oi)));
        }
    }
    for (oi, op) in doc.slice_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::SliceOp(oi)));
        }
    }
    // An edge-treatment op consumes the input bodies whose edges it bevels (#531). The treated
    // edges themselves have no persistent node, so the body carries the dependency.
    for (oi, op) in doc.edge_treatment_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::EdgeTreatmentOp(oi)));
        }
    }

    // A loft is fed by its cross-section sketches (#252) — the user's canonical example: three
    // sketches feeding one loft that outputs a body.
    for (li, loft) in doc.lofts.iter().enumerate() {
        if loft.deleted {
            continue;
        }
        let mut seen = std::collections::HashSet::new();
        for section in &loft.sections {
            if seen.insert(section.sketch) {
                edges.push((HierarchyNode::Sketch(section.sketch), HierarchyNode::Loft(li)));
            }
        }
    }

    // A repeat consumes its input bodies, source planes/sketches, and replayed cut
    // extrusions (#448): the original body is the repeat's parent, not a sibling.
    for (oi, op) in doc.repeat_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::RepeatOp(oi)));
        }
        for &pi in &op.plane_targets {
            edges.push((HierarchyNode::ConstructionPlane(pi), HierarchyNode::RepeatOp(oi)));
        }
        for &si in &op.sketch_targets {
            edges.push((HierarchyNode::Sketch(si), HierarchyNode::RepeatOp(oi)));
        }
        for &ei in &op.extrusion_targets {
            edges.push((HierarchyNode::Extrusion(ei), HierarchyNode::RepeatOp(oi)));
        }
    }
    // A move also consumes its planes and images, beyond the bodies covered above (#449).
    for (oi, op) in doc.move_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &pi in &op.plane_targets {
            edges.push((HierarchyNode::ConstructionPlane(pi), HierarchyNode::MoveOp(oi)));
        }
        for &ii in &op.image_targets {
            edges.push((HierarchyNode::Image(ii), HierarchyNode::MoveOp(oi)));
        }
    }
    // A slice's cutters feed it (#449): construction planes have a node; body faces don't.
    for (oi, op) in doc.slice_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for cutter in &op.cutters {
            if let FaceId::ConstructionPlane(pi) = cutter {
                edges.push((HierarchyNode::ConstructionPlane(*pi), HierarchyNode::SliceOp(oi)));
            }
        }
    }
    // A revolution is fed by its profile sketch, and by its axis line if any (#449).
    for (ri, rev) in doc.revolutions.iter().enumerate() {
        if rev.deleted {
            continue;
        }
        edges.push((HierarchyNode::Sketch(rev.sketch), HierarchyNode::Revolution(ri)));
        if let crate::model::RevolveAxis::Line(li) = rev.axis {
            edges.push((HierarchyNode::Line(li), HierarchyNode::Revolution(ri)));
        }
    }
    // A sweep is fed by its profile sketch and every path line.
    for (fi, fp) in doc.sweeps.iter().enumerate() {
        if fp.deleted {
            continue;
        }
        edges.push((HierarchyNode::Sketch(fp.sketch), HierarchyNode::SweepOp(fi)));
        for &li in &fp.path {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SweepOp(fi)));
        }
    }
    // In-sketch ops consume their source lines/circles (#449); the in-sketch slice also
    // its cutter lines.
    for (oi, op) in doc.sketch_repeat_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &li in &op.line_targets {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchRepeatOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchRepeatOp(oi)));
        }
    }
    for (oi, op) in doc.sketch_offset_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &li in &op.line_targets {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchOffsetOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchOffsetOp(oi)));
        }
    }
    // An in-sketch mirror consumes its mirror line and every source line/circle (#523).
    for (oi, op) in doc.sketch_mirror_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        edges.push((HierarchyNode::Line(op.line), HierarchyNode::SketchMirrorOp(oi)));
        for &li in &op.line_targets {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchMirrorOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchMirrorOp(oi)));
        }
    }
    // An in-sketch chamfer/fillet consumes its (shadowed) source edges (#538).
    for (oi, op) in doc.sketch_vertex_treatment_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &li in &op.line_targets {
            edges.push((
                HierarchyNode::Line(li),
                HierarchyNode::SketchVertexTreatmentOp(oi),
            ));
        }
    }
    for (oi, op) in doc.sketch_slice_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        for &li in op.line_targets.iter().chain(op.cutter_lines.iter()) {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchSliceOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchSliceOp(oi)));
        }
    }
    // A drawing projection depends on its source body/sketch (#281).
    for (di, drawing) in doc.drawings.iter().enumerate() {
        if drawing.deleted {
            continue;
        }
        for (vi, view) in drawing.views.iter().enumerate() {
            let source = match view.sketch {
                Some(si) => HierarchyNode::Sketch(si),
                None => HierarchyNode::Body(view.body),
            };
            edges.push((source, HierarchyNode::DrawingProjection { drawing: di, view: vi }));
        }
    }
    edges
}

/// The dot radius, label gap, and minimum breathing room used by [`declutter_label_bands`] and
/// mirrored by the graph render (`show_graph_view`). Kept here so the physics-free declutter is
/// unit-testable without pulling in `egui`.
const GRAPH_NODE_RADIUS_PX: f32 = 9.0;
const GRAPH_LABEL_GAP_PX: f32 = 4.0;
const GRAPH_LABEL_CLEAR_PX: f32 = 6.0;

/// Guarantee that no two graph-node labels overlap (#248). The force sim positions the dots
/// nicely but its labels — drawn rightward from each dot — can still land on a neighbour's dot
/// or text. Different depth bands sit `LAYER_HEIGHT` apart vertically (far beyond a line of
/// text), so only same-depth labels can collide; within each band this spreads the nodes just
/// enough horizontally to clear every label, preserving their left-to-right order and the
/// band's centre so the layout stays stable frame to frame. Returns an x override per node
/// (in the same local space as `sim_x`).
fn declutter_label_bands(
    positions: &[GraphNodePosition],
    sim_x: &HashMap<HierarchyNode, f32>,
    label_widths: &HashMap<HierarchyNode, f32>,
    available_width: f32,
) -> HashMap<HierarchyNode, (f32, usize)> {
    let x_of = |n: &HierarchyNode| sim_x.get(n).copied().unwrap_or(0.0);
    let w_of = |n: &HierarchyNode| label_widths.get(n).copied().unwrap_or(0.0);
    // A node's full right extent (dot + gap + label + clearance).
    let extent = |n: &HierarchyNode| {
        2.0 * GRAPH_NODE_RADIUS_PX + GRAPH_LABEL_GAP_PX + w_of(n) + GRAPH_LABEL_CLEAR_PX
    };

    let mut by_depth: BTreeMap<usize, Vec<HierarchyNode>> = BTreeMap::new();
    for p in positions {
        by_depth.entry(p.depth).or_default().push(p.node);
    }

    let usable_right = (available_width - GRAPH_MARGIN).max(GRAPH_MARGIN + 1.0);
    let usable = (usable_right - GRAPH_MARGIN).max(1.0);

    let mut out = HashMap::new();
    for (_, mut band) in by_depth {
        // Order by simulated x (identity as a deterministic tiebreak) and hold that order.
        band.sort_by(|a, b| {
            x_of(a)
                .partial_cmp(&x_of(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(b))
        });
        let n = band.len();
        if n == 0 {
            continue;
        }
        let total_extent: f32 = band.iter().map(&extent).sum();
        if total_extent <= usable {
            // Fits in one row: keep the organic sweep, recentred on the band's centroid so no
            // single label overflows the pane (#248).
            let mut new_x = vec![0.0f32; n];
            new_x[0] = x_of(&band[0]);
            for i in 1..n {
                let min_next = new_x[i - 1] + extent(&band[i - 1]);
                new_x[i] = x_of(&band[i]).max(min_next);
            }
            let sim_mean = band.iter().map(x_of).sum::<f32>() / n as f32;
            let new_mean = new_x.iter().sum::<f32>() / n as f32;
            let shift = sim_mean - new_mean;
            let placed: Vec<f32> = new_x.iter().map(|x| x + shift).collect();
            // Translate the whole band (never per-node clamp, which would collapse spacing) so it
            // sits within [margin, usable_right]; since it fits (total_extent ≤ usable) this always
            // succeeds without overlap.
            let left = placed.iter().copied().fold(f32::MAX, f32::min);
            let right = placed
                .iter()
                .zip(&band)
                .map(|(x, node)| x + extent(node))
                .fold(f32::MIN, f32::max);
            let mut adjust = 0.0;
            if left < GRAPH_MARGIN {
                adjust = GRAPH_MARGIN - left;
            }
            if right + adjust > usable_right {
                adjust -= right + adjust - usable_right;
            }
            for (node, x) in band.iter().zip(placed) {
                out.insert(*node, (x + adjust, 0usize));
            }
        } else {
            // Too wide for the pane: pack left→right and wrap into stacked sub-rows so the band
            // grows *taller* instead of overflowing the width (#350).
            let mut cursor = GRAPH_MARGIN;
            let mut row = 0usize;
            for node in &band {
                let ext = extent(node);
                if cursor > GRAPH_MARGIN && cursor + ext > usable_right {
                    row += 1;
                    cursor = GRAPH_MARGIN;
                }
                out.insert(*node, (cursor, row));
                cursor += ext;
            }
        }
    }
    out
}

/// Find `node`'s entry anywhere in `tree` (not just at the root — e.g. a sketch nests under
/// its construction plane).
pub(crate) fn find_hierarchy_entry(
    tree: &[HierarchyEntry],
    node: HierarchyNode,
) -> Option<&HierarchyEntry> {
    for entry in tree {
        if entry.node == node {
            return Some(entry);
        }
        if let Some(found) = find_hierarchy_entry(&entry.children, node) {
            return Some(found);
        }
    }
    None
}

fn collect_entry_descendants(entry: &HierarchyEntry, out: &mut HashSet<HierarchyNode>) {
    for child in &entry.children {
        out.insert(child.node);
        collect_entry_descendants(child, out);
    }
}

/// The graph-node view's highlight set for a selected node: the node itself, all its
/// ancestors (walked via the parent links from [`graph_node_positions`]), and all its
/// descendants (walked via `tree`'s own nested `children`, no `SceneElement` lookups needed —
/// the tree structure already gives parent/child relationships directly).
pub fn graph_related_nodes(tree: &[HierarchyEntry], selected: HierarchyNode) -> HashSet<HierarchyNode> {
    let positions = graph_node_positions(tree);
    let parent_of: HashMap<HierarchyNode, HierarchyNode> = positions
        .iter()
        .filter_map(|p| p.parent.map(|parent| (p.node, parent)))
        .collect();

    let mut related = HashSet::new();
    related.insert(selected);

    let mut current = selected;
    while let Some(&parent) = parent_of.get(&current) {
        related.insert(parent);
        current = parent;
    }

    if let Some(entry) = find_hierarchy_entry(tree, selected) {
        collect_entry_descendants(entry, &mut related);
    }

    related
}

/// Persistent physics state for the Graph view's force-directed layout (#94). Held on `App`
/// (never persisted to disk — a purely ephemeral view state, like [`HierarchyViewMode`]) and
/// threaded into [`show_graph_view`], so node positions/velocities carry across frames and the
/// simulation can animate ("bounce around") until it settles. Coordinates are layout-local:
/// x is contained to the pane width, y flows top-to-bottom by tree depth.
#[derive(Default)]
pub struct GraphLayout {
    nodes: HashMap<HierarchyNode, GraphNodeState>,
    /// Persistent per-node drag offsets (#451): the user can grab any node and move it;
    /// the offset adds to the computed layout position so physics/declutter still run.
    drag_offsets: HashMap<HierarchyNode, egui::Vec2>,
}

/// One node's live physics state in [`GraphLayout`]: current position and velocity.
#[derive(Clone, Copy, Debug)]
struct GraphNodeState {
    pos: egui::Vec2,
    vel: egui::Vec2,
}

/// Vertical spacing between successive tree depths — the "somewhat vertical" target the
/// layering force pulls each node toward (parents above children, flow top-to-bottom, #94).
const LAYER_HEIGHT: f32 = 64.0;
/// Horizontal inset kept clear at each side of the pane; x is soft-restored and hard-clamped
/// into `[MARGIN, width - MARGIN]` so the graph never exceeds the pane width (#34).
const GRAPH_MARGIN: f32 = 18.0;

/// Deterministic horizontal seed for a freshly-inserted node, derived purely from the node's
/// identity (no `rand`, so layout is reproducible across runs and in tests). Spreads new nodes
/// across the pane width so the simulation starts un-coincident.
fn seed_x(node: HierarchyNode, width: f32) -> f32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    node.hash(&mut hasher);
    let frac = (hasher.finish() % 10_000) as f32 / 10_000.0;
    let lo = GRAPH_MARGIN;
    let hi = (width - GRAPH_MARGIN).max(GRAPH_MARGIN + 1.0);
    lo + frac * (hi - lo)
}

/// Advance the force-directed layout one integration step (semi-implicit Euler) and return the
/// total kinetic energy (Σ‖vel‖²) — a pure, `egui`-painting-free function so the physics is
/// directly unit-testable (settle, containment, vertical ordering, determinism). `edges` are
/// `(child, parent)` pairs; `depth_of` gives each node's tree depth; `width` is the pane width
/// the x coordinate is contained to.
///
/// Forces: a vertical layering spring pulling `y` toward `depth * LAYER_HEIGHT`; pairwise
/// inverse-square repulsion (min-distance/max-force capped) spreading siblings sideways;
/// parent↔child edge springs toward a rest length; a soft horizontal-containment restoring
/// force; and per-step velocity damping so it settles rather than oscillating forever.
fn step_graph_layout(
    nodes: &mut HashMap<HierarchyNode, GraphNodeState>,
    edges: &[(HierarchyNode, HierarchyNode)],
    depth_of: &HashMap<HierarchyNode, usize>,
    width: f32,
    dt: f32,
) -> f32 {
    const LAYER_STIFFNESS: f32 = 10.0;
    // #151: repulsion must win against the edge springs at dot-diameter range or siblings
    // pile up on their parent's x. The spring is deliberately weak *and capped* (it only
    // keeps children loosely near their parent), its rest length sits above LAYER_HEIGHT so
    // a child directly below its parent is in compression (pushed to fan out sideways), and
    // the repulsion constant is sized to hold ~26 px spacing against the capped spring.
    const REPULSION: f32 = 30_000.0;
    const MIN_DIST: f32 = 6.0;
    const MAX_REPULSION_FORCE: f32 = 300.0;
    const EDGE_SPRING_K: f32 = 0.6;
    const EDGE_SPRING_MAX_FORCE: f32 = 40.0;
    const EDGE_REST_LENGTH: f32 = 84.0;
    const CONTAIN_STIFFNESS: f32 = 14.0;
    const DAMPING: f32 = 0.86;

    // Iterate a sorted key list (not the HashMap's arbitrary order) so force accumulation is
    // order-independent and thus bit-for-bit deterministic across runs.
    let mut keys: Vec<HierarchyNode> = nodes.keys().copied().collect();
    keys.sort();
    let index_of: HashMap<HierarchyNode, usize> =
        keys.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let mut forces = vec![egui::Vec2::ZERO; keys.len()];

    // Vertical layering spring: pull y toward the node's depth band.
    for (i, node) in keys.iter().enumerate() {
        let target_y = *depth_of.get(node).unwrap_or(&0) as f32 * LAYER_HEIGHT;
        let y = nodes[node].pos.y;
        forces[i].y += LAYER_STIFFNESS * (target_y - y);
    }

    // Pairwise repulsion (inverse-square, min-distance and max-force capped).
    for a in 0..keys.len() {
        for b in (a + 1)..keys.len() {
            let pa = nodes[&keys[a]].pos;
            let pb = nodes[&keys[b]].pos;
            let mut delta = pa - pb;
            let mut dist = delta.length();
            if dist < MIN_DIST {
                // Coincident (or nearly): shove apart along a deterministic axis to avoid a
                // divide-by-zero / NaN blowup.
                if dist < 1e-4 {
                    delta = egui::vec2(1.0, 0.0);
                    dist = 1.0;
                }
                dist = dist.max(MIN_DIST);
            }
            let dir = delta / dist;
            let mag = (REPULSION / (dist * dist)).min(MAX_REPULSION_FORCE);
            let f = dir * mag;
            forces[a] += f;
            forces[b] -= f;
        }
    }

    // Edge springs (parent↔child attraction toward a rest length).
    for (child, parent) in edges {
        let (Some(&ci), Some(&pi)) = (index_of.get(child), index_of.get(parent)) else {
            continue;
        };
        let delta = nodes[child].pos - nodes[parent].pos;
        let dist = delta.length().max(MIN_DIST);
        let dir = delta / dist;
        let mag = (-EDGE_SPRING_K * (dist - EDGE_REST_LENGTH))
            .clamp(-EDGE_SPRING_MAX_FORCE, EDGE_SPRING_MAX_FORCE);
        let f = dir * mag;
        forces[ci] += f;
        forces[pi] -= f;
    }

    // Horizontal soft-containment restoring force.
    let lo = GRAPH_MARGIN;
    let hi = (width - GRAPH_MARGIN).max(lo);
    for (i, node) in keys.iter().enumerate() {
        let x = nodes[node].pos.x;
        if x < lo {
            forces[i].x += CONTAIN_STIFFNESS * (lo - x);
        } else if x > hi {
            forces[i].x += CONTAIN_STIFFNESS * (hi - x);
        }
    }

    // Integrate: vel += force*dt; vel *= damping; pos += vel*dt; then hard-clamp x.
    let mut kinetic = 0.0;
    for (i, node) in keys.iter().enumerate() {
        let state = nodes.get_mut(node).expect("key came from this map");
        let mut force = forces[i];
        if !force.x.is_finite() {
            force.x = 0.0;
        }
        if !force.y.is_finite() {
            force.y = 0.0;
        }
        state.vel += force * dt;
        state.vel *= DAMPING;
        // Speed cap: in an overcrowded pane the stiff close-range forces would otherwise
        // orbit the equilibrium forever instead of settling into it.
        const MAX_SPEED: f32 = 200.0;
        if state.vel.length() > MAX_SPEED {
            state.vel = state.vel.normalized() * MAX_SPEED;
        }
        state.pos += state.vel * dt;
        // The hard x-clamp is a wall collision: also kill the velocity component pushing
        // into the wall, or a crowded row pins nodes at the margin with an ever-pumping
        // velocity (sustained kinetic energy) and the layout never registers as settled.
        let clamped_x = state.pos.x.clamp(lo, hi);
        if clamped_x != state.pos.x {
            state.pos.x = clamped_x;
            state.vel.x = 0.0;
        }
        if !state.pos.x.is_finite() {
            state.pos.x = lo;
        }
        if !state.pos.y.is_finite() {
            state.pos.y = 0.0;
        }
        kinetic += state.vel.length_sq();
    }
    kinetic
}

impl GraphLayout {
    /// Sync the live node set to `positions` (seed newly-appeared nodes deterministically,
    /// drop departed ones), then advance the simulation `substeps` times, returning the final
    /// kinetic energy for settle detection.
    fn sync_and_step(
        &mut self,
        positions: &[GraphNodePosition],
        width: f32,
        substeps: u32,
        dt: f32,
        run_physics: bool,
    ) -> f32 {
        let present: HashSet<HierarchyNode> = positions.iter().map(|p| p.node).collect();
        self.nodes.retain(|node, _| present.contains(node));
        let depth_of: HashMap<HierarchyNode, usize> =
            positions.iter().map(|p| (p.node, p.depth)).collect();
        for p in positions {
            self.nodes.entry(p.node).or_insert_with(|| GraphNodeState {
                pos: egui::vec2(seed_x(p.node, width), p.depth as f32 * LAYER_HEIGHT),
                vel: egui::Vec2::ZERO,
            });
        }
        // Force layout off (#525): keep nodes synced (new ones seeded, gone ones dropped) but
        // freeze them in place — no stepping — so a busy graph holds still.
        if !run_physics {
            return 0.0;
        }
        let edges: Vec<(HierarchyNode, HierarchyNode)> = positions
            .iter()
            .filter_map(|p| p.parent.map(|parent| (p.node, parent)))
            .collect();
        let mut kinetic = 0.0;
        for _ in 0..substeps.max(1) {
            kinetic = step_graph_layout(&mut self.nodes, &edges, &depth_of, width, dt);
        }
        kinetic
    }

    /// The user's drag offset for a node (#451), zero if never dragged.
    fn drag_offset(&self, node: HierarchyNode) -> egui::Vec2 {
        self.drag_offsets.get(&node).copied().unwrap_or(egui::Vec2::ZERO)
    }

    /// Accumulate a drag delta onto a node's offset (#451).
    fn add_drag_offset(&mut self, node: HierarchyNode, delta: egui::Vec2) {
        *self.drag_offsets.entry(node).or_insert(egui::Vec2::ZERO) += delta;
    }

    fn pos_of(&self, node: HierarchyNode) -> Option<egui::Vec2> {
        self.nodes.get(&node).map(|s| s.pos)
    }
}

/// The [`HierarchyNode`] for a [`SceneElement`] — the inverse of [`scene_element_for_node`]
/// for the kinds that appear in the element graph (#524/#531). `None` for sub-element
/// selections (points, edges, vertices) that aren't graph nodes.
pub fn hierarchy_node_for_element(element: &SceneElement) -> Option<HierarchyNode> {
    Some(match element {
        SceneElement::ConstructionPlane(i) => HierarchyNode::ConstructionPlane(*i),
        SceneElement::Sketch(i) => HierarchyNode::Sketch(*i),
        SceneElement::Line(i) => HierarchyNode::Line(*i),
        SceneElement::Circle(i) => HierarchyNode::Circle(*i),
        SceneElement::Constraint(i) => HierarchyNode::Constraint(*i),
        SceneElement::Extrusion(i) => HierarchyNode::Extrusion(*i),
        SceneElement::Body(i) => HierarchyNode::Body(*i),
        SceneElement::Image(i) => HierarchyNode::Image(*i),
        SceneElement::BooleanOp(i) => HierarchyNode::BooleanOp(*i),
        SceneElement::MoveOp(i) => HierarchyNode::MoveOp(*i),
        SceneElement::MirrorOp(i) => HierarchyNode::MirrorOp(*i),
        SceneElement::RepeatOp(i) => HierarchyNode::RepeatOp(*i),
        SceneElement::SketchRepeatOp(i) => HierarchyNode::SketchRepeatOp(*i),
        SceneElement::SketchOffsetOp(i) => HierarchyNode::SketchOffsetOp(*i),
        SceneElement::SketchMirrorOp(i) => HierarchyNode::SketchMirrorOp(*i),
        SceneElement::SketchVertexTreatmentOp(i) => HierarchyNode::SketchVertexTreatmentOp(*i),
        SceneElement::SketchSliceOp(i) => HierarchyNode::SketchSliceOp(*i),
        SceneElement::SketchText(i) => HierarchyNode::SketchText(*i),
        SceneElement::SliceOp(i) => HierarchyNode::SliceOp(*i),
        SceneElement::EdgeTreatmentOp(i) => HierarchyNode::EdgeTreatmentOp(*i),
        SceneElement::Revolution(i) => HierarchyNode::Revolution(*i),
        SceneElement::SweepOp(i) => HierarchyNode::SweepOp(*i),
        SceneElement::Component(i) => HierarchyNode::Component(*i),
        SceneElement::Point(_)
        | SceneElement::FaceEdge(_)
        | SceneElement::Origin
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::BodyFace { .. } => return None,
    })
}

/// A timeline rollback point (#524/#545): the element to roll back to, plus whether the
/// rollback is **inclusive** — "rollback to just before here" hides the element itself along
/// with its descendants, whereas the default "rollback to here" keeps the element and hides
/// only what depends on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackMarker {
    pub element: SceneElement,
    pub inclusive: bool,
}

/// Every element suppressed by a rollback marker (#524/#531/#545): the marker element's
/// descendants in the element graph — found by walking forward from it along both the nesting
/// tree (an op to its output bodies, a sketch to its geometry, …) and the dashed dependency
/// edges (an input feeding a consuming operation) — plus the marker element itself when the
/// marker is **inclusive** ("just before here"). Unlike a creation-order cutoff, this hides only
/// what genuinely derives from the marker — two independent branches don't affect each other.
pub fn rolled_back_elements(doc: &Document, marker: &RollbackMarker) -> HashSet<SceneElement> {
    let Some(marker_node) = hierarchy_node_for_element(&marker.element) else {
        return HashSet::new();
    };
    let tree = build_hierarchy(doc, None);
    // Forward dependency adjacency: source node -> the operations that consume it.
    let mut consumers: HashMap<HierarchyNode, Vec<HierarchyNode>> = HashMap::new();
    for (source, consumer) in graph_dependency_edges(doc) {
        consumers.entry(source).or_default().push(consumer);
    }

    let mut result: HashSet<HierarchyNode> = HashSet::new();
    let mut seen: HashSet<HierarchyNode> = HashSet::from([marker_node]);
    let mut stack = vec![marker_node];
    while let Some(node) = stack.pop() {
        // Nesting descendants (e.g. an op's output bodies, a sketch's geometry).
        if let Some(entry) = find_hierarchy_entry(&tree, node) {
            let mut kids = HashSet::new();
            collect_entry_descendants(entry, &mut kids);
            for k in kids {
                if seen.insert(k) {
                    result.insert(k);
                    stack.push(k);
                }
            }
        }
        // Operations that consume this node.
        if let Some(cs) = consumers.get(&node) {
            for &c in cs {
                if seen.insert(c) {
                    result.insert(c);
                    stack.push(c);
                }
            }
        }
    }
    let mut elements: HashSet<SceneElement> =
        result.iter().filter_map(|&n| scene_element_for_node(n)).collect();
    // "Rollback to just before here" also hides the marker element itself.
    if marker.inclusive {
        elements.insert(marker.element.clone());
    }
    elements
}

/// Build the hierarchy tree for the current view context.
///
/// Returns a single-element vec: the synthetic [`HierarchyNode::Document`] root, with every
/// former top-level item (root construction planes, orphaned extrusions, orphaned bodies)
/// nested as its children (#87).
pub fn build_hierarchy(
    doc: &Document,
    sketch_session: Option<SketchSession>,
) -> Vec<HierarchyEntry> {
    let mut roots = Vec::new();
    for (i, plane) in doc.construction_planes.iter().enumerate() {
        if plane.deleted || !matches!(plane.parent, ConstructionPlaneParent::Root) {
            continue;
        }
        // Repeat-op plane instances (#221) and repeated-sketch host planes (#226/#231) are
        // grouped under their operation, not at the top level.
        if plane.repeat_instance.is_some() || is_repeat_sketch_host_plane(doc, i) {
            continue;
        }
        let face = FaceId::ConstructionPlane(i);
        let mut children = build_face_sketches(doc, face, sketch_session);
        // Tracing images (#169) nest under their host plane.
        for (ii, image) in doc.tracing_images.iter().enumerate() {
            if !image.deleted && image.plane == i {
                children.push(HierarchyEntry {
                    node: HierarchyNode::Image(ii),
                    children: Vec::new(),
                });
            }
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::ConstructionPlane(i),
            children,
        });
    }
    // Extrusions nest under the sketch they were built from (see
    // build_sketch_entry). Any extrusion whose sketch is no longer reachable is
    // surfaced at the top level so it never disappears from the tree.
    for (i, extrusion) in doc.extrusions.iter().enumerate() {
        if extrusion.deleted || sketch_alive(doc, extrusion.sketch) {
            continue;
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::Extrusion(i),
            children: build_sketch_extrusions(doc, extrusion.sketch, sketch_session)
                .into_iter()
                .find(|e| e.node == HierarchyNode::Extrusion(i))
                .map(|e| e.children)
                .unwrap_or_default(),
        });
    }
    // Bodies with no source extrusion (e.g. STL imports, #70) have no sketch/feature to nest
    // under, so they surface at the top level, same as an orphaned extrusion above.
    for (bi, body) in doc.bodies.iter().enumerate() {
        if !body.deleted
            && body.source.extrusion_indices().is_empty()
            && !matches!(
                body.source,
                crate::model::BodySource::Boolean { .. }
                    | crate::model::BodySource::Moved { .. }
                    | crate::model::BodySource::Repeated { .. }
                    | crate::model::BodySource::Sliced { .. }
                    // A beveled body nests under its edge-treatment op (#531), not the root.
                    | crate::model::BodySource::EdgeTreated { .. }
                    | crate::model::BodySource::Loft(_)
                    // A revolved body nests under its Revolution node (#305), not the root.
                    | crate::model::BodySource::Revolve(_)
                    // A swept body nests under its Sweep node, not the root.
                    | crate::model::BodySource::Sweep(_)
            )
        {
            roots.push(HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            });
        }
    }
    // Lofts (#252): the loft is an operation node with its output body nested beneath it (its
    // cross-section sketches feed it as graph inputs, see `graph_dependency_edges`). Previously
    // the loft body surfaced as a bare top-level element with no sign of what produced it.
    for (li, loft) in doc.lofts.iter().enumerate() {
        if loft.deleted {
            continue;
        }
        let children = doc
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.deleted && matches!(b.source, crate::model::BodySource::Loft(l) if l == li))
            .map(|(bi, _)| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::Loft(li),
            children,
        });
    }
    // Boolean operations (Combine tool): the operation is an element of its own, with its
    // output bodies nested beneath it — outputs depend on the operation, the operation on
    // its (shadow) inputs.
    for (oi, op) in doc.boolean_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let children = op
            .outputs
            .iter()
            .filter(|&&bi| doc.bodies.get(bi).is_some_and(|b| !b.deleted))
            .map(|&bi| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::BooleanOp(oi),
            children,
        });
    }
    for (oi, op) in doc.move_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let children = op
            .outputs
            .iter()
            .filter(|&&bi| doc.bodies.get(bi).is_some_and(|b| !b.deleted))
            .map(|&bi| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::MoveOp(oi),
            children,
        });
    }
    for (oi, op) in doc.mirror_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let children = op
            .outputs
            .iter()
            .filter(|&&bi| doc.bodies.get(bi).is_some_and(|b| !b.deleted))
            .map(|&bi| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::MirrorOp(oi),
            children,
        });
    }
    for (oi, op) in doc.repeat_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let mut children: Vec<HierarchyEntry> = op
            .outputs
            .iter()
            .filter(|&&bi| doc.bodies.get(bi).is_some_and(|b| !b.deleted))
            .map(|&bi| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        // Generated construction-plane instances (#221) nest under the op too.
        children.extend(
            op.plane_outputs
                .iter()
                .filter(|&&pi| doc.construction_planes.get(pi).is_some_and(|p| !p.deleted))
                .map(|&pi| HierarchyEntry {
                    node: HierarchyNode::ConstructionPlane(pi),
                    children: Vec::new(),
                }),
        );
        // Repeated-sketch host planes (#226/#231) nest under the op, each with its copy sketch.
        children.extend(
            op.sketch_plane_outputs
                .iter()
                .filter(|&&pi| doc.construction_planes.get(pi).is_some_and(|p| !p.deleted))
                .map(|&pi| HierarchyEntry {
                    node: HierarchyNode::ConstructionPlane(pi),
                    children: build_face_sketches(doc, FaceId::ConstructionPlane(pi), sketch_session),
                }),
        );
        roots.push(HierarchyEntry {
            node: HierarchyNode::RepeatOp(oi),
            children,
        });
    }
    // 2D in-sketch repeats (#222/#228): the op is its own element with its duplicated
    // lines/circles nested beneath it (they're excluded from the sketch's own listing).
    for (oi, op) in doc.sketch_repeat_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let mut children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .filter(|&&li| doc.lines.get(li).is_some_and(|l| !l.deleted))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        children.extend(
            op.circle_outputs
                .iter()
                .filter(|&&ci| doc.circles.get(ci).is_some_and(|c| !c.deleted))
                .map(|&ci| HierarchyEntry { node: HierarchyNode::Circle(ci), children: Vec::new() }),
        );
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchRepeatOp(oi),
            children,
        });
    }
    // 2D in-sketch offsets: the op is its own element with its parallel lines/circles
    // nested beneath it (they're excluded from the sketch's own listing).
    for (oi, op) in doc.sketch_offset_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let mut children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .filter(|&&li| doc.lines.get(li).is_some_and(|l| !l.deleted))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        children.extend(
            op.circle_outputs
                .iter()
                .filter(|&&ci| doc.circles.get(ci).is_some_and(|c| !c.deleted))
                .map(|&ci| HierarchyEntry { node: HierarchyNode::Circle(ci), children: Vec::new() }),
        );
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchOffsetOp(oi),
            children,
        });
    }
    // 2D in-sketch mirrors (#523): the op with its reflected lines/circles nested beneath.
    for (oi, op) in doc.sketch_mirror_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let mut children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .filter(|&&li| doc.lines.get(li).is_some_and(|l| !l.deleted))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        children.extend(
            op.circle_outputs
                .iter()
                .filter(|&&ci| doc.circles.get(ci).is_some_and(|c| !c.deleted))
                .map(|&ci| HierarchyEntry { node: HierarchyNode::Circle(ci), children: Vec::new() }),
        );
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchMirrorOp(oi),
            children,
        });
    }
    // 2D in-sketch chamfer/fillet (#538): the op with its trimmed copies + bridge lines nested
    // beneath it (the shadowed source edges stay listed under the sketch, dimmed).
    for (oi, op) in doc.sketch_vertex_treatment_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .chain(op.bridge_outputs.iter())
            .filter(|&&li| doc.lines.get(li).is_some_and(|l| !l.deleted))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchVertexTreatmentOp(oi),
            children,
        });
    }
    // 2D in-sketch slices (#224/#229): the op is its own element with its fragment lines nested
    // beneath it (the shadowed originals stay listed under the sketch, dimmed).
    for (oi, op) in doc.sketch_slice_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .filter(|&&li| doc.lines.get(li).is_some_and(|l| !l.deleted))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchSliceOp(oi),
            children,
        });
    }
    // Slice operations (Slice tool): the operation is its own element, with its fragment
    // bodies nested beneath it.
    for (oi, op) in doc.slice_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let children = op
            .outputs
            .iter()
            .filter(|&&bi| doc.bodies.get(bi).is_some_and(|b| !b.deleted))
            .map(|&bi| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::SliceOp(oi),
            children,
        });
    }
    // Edge chamfer/fillet operations (#531): the operation is its own element, with its beveled
    // output bodies nested beneath it (the shadowed input bodies stay listed, dimmed).
    for (oi, op) in doc.edge_treatment_ops.iter().enumerate() {
        if op.deleted {
            continue;
        }
        let children = op
            .outputs
            .iter()
            .filter(|&&bi| doc.bodies.get(bi).is_some_and(|b| !b.deleted))
            .map(|&bi| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::EdgeTreatmentOp(oi),
            children,
        });
    }
    // Revolved solids (Revolve tool, #211): the operation is its own element, with its output
    // body (linked by `BodySource::Revolve`) nested beneath it.
    for (oi, rev) in doc.revolutions.iter().enumerate() {
        if rev.deleted {
            continue;
        }
        let children = doc
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.deleted && b.source == crate::model::BodySource::Revolve(oi))
            .map(|(bi, _)| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::Revolution(oi),
            children,
        });
    }
    // Sweeps nest under their profile sketch (#478, see `build_sketch_entry`); only a
    // sweep whose sketch died falls back to a top-level orphan here so it stays reachable.
    for (oi, fp) in doc.sweeps.iter().enumerate() {
        if fp.deleted || crate::document_lifecycle::sketch_alive(doc, fp.sketch) {
            continue;
        }
        let children = doc
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.deleted && b.source == crate::model::BodySource::Sweep(oi))
            .map(|(bi, _)| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::SweepOp(oi),
            children,
        });
    }
    // Technical drawings (#180): top-level leaves (they reference bodies but aren't part of
    // the geometry DAG), each right-clickable to open its drawing pane.
    for (di, drawing) in doc.drawings.iter().enumerate() {
        if !drawing.deleted {
            // Each placed view is a "projection" child of the drawing (#281), with its shown
            // dimensions nested under it (#341); each text note is a "text" child (#333).
            let mut children: Vec<HierarchyEntry> = drawing
                .views
                .iter()
                .enumerate()
                .map(|(vi, view)| HierarchyEntry {
                    node: HierarchyNode::DrawingProjection { drawing: di, view: vi },
                    children: view
                        .dimensioned_edges
                        .iter()
                        .map(|(a, b)| HierarchyEntry {
                            node: HierarchyNode::DrawingDimension {
                                drawing: di,
                                view: vi,
                                a: *a,
                                b: *b,
                            },
                            children: Vec::new(),
                        })
                        .collect(),
                })
                .collect();
            for (ai, ann) in drawing.annotations.iter().enumerate() {
                if !ann.deleted {
                    children.push(HierarchyEntry {
                        node: HierarchyNode::DrawingAnnotation { drawing: di, annotation: ai },
                        children: Vec::new(),
                    });
                }
            }
            roots.push(HierarchyEntry {
                node: HierarchyNode::Drawing(di),
                children,
            });
        }
    }
    // Components (#423): move member roots under their component's entry, then nest
    // component entries by their parent links. Unassigned roots stay at the top level.
    let roots = group_roots_into_components(doc, roots);
    vec![HierarchyEntry {
        node: HierarchyNode::Document,
        children: roots,
    }]
}

/// Group top-level entries into their components' entries (#423). Components render even
/// when empty; a component whose parent chain is broken surfaces at the top level.
fn group_roots_into_components(doc: &Document, roots: Vec<HierarchyEntry>) -> Vec<HierarchyEntry> {
    use crate::model::ComponentMember as CM;
    if doc.components.iter().all(|c| c.deleted) {
        return roots;
    }
    let member_of = |node: &HierarchyNode| -> Option<usize> {
        let (kind, index) = match node {
            HierarchyNode::ConstructionPlane(i) => (CM::ConstructionPlane, *i),
            HierarchyNode::Extrusion(i) => (CM::Extrusion, *i),
            HierarchyNode::Body(i) => (CM::Body, *i),
            HierarchyNode::Loft(i) => (CM::Loft, *i),
            HierarchyNode::BooleanOp(i) => (CM::BooleanOp, *i),
            HierarchyNode::MoveOp(i) => (CM::MoveOp, *i),
            HierarchyNode::MirrorOp(i) => (CM::MirrorOp, *i),
            HierarchyNode::RepeatOp(i) => (CM::RepeatOp, *i),
            HierarchyNode::SliceOp(i) => (CM::SliceOp, *i),
            HierarchyNode::Revolution(i) => (CM::Revolution, *i),
            HierarchyNode::SweepOp(i) => (CM::Sweep, *i),
            HierarchyNode::Drawing(i) => (CM::Drawing, *i),
            _ => return None,
        };
        doc.component_of(kind, index)
    };
    // component index -> its (initially childless) entry.
    let mut comp_children: HashMap<usize, Vec<HierarchyEntry>> = HashMap::new();
    for (ci, c) in doc.components.iter().enumerate() {
        if !c.deleted {
            comp_children.insert(ci, Vec::new());
        }
    }
    // Extract assigned entries wherever they sit (#423): an assigned element that nests
    // inside another entry's subtree (an extrusion under its sketch's plane, a body under
    // an op) moves — with its own subtree — into the component's entry.
    fn extract_members(
        entries: &mut Vec<HierarchyEntry>,
        member_of: &impl Fn(&HierarchyNode) -> Option<usize>,
        comp_children: &mut HashMap<usize, Vec<HierarchyEntry>>,
    ) {
        let mut i = 0;
        while i < entries.len() {
            match member_of(&entries[i].node) {
                Some(c) if comp_children.contains_key(&c) => {
                    let e = entries.remove(i);
                    comp_children.get_mut(&c).unwrap().push(e);
                }
                _ => {
                    extract_members(&mut entries[i].children, member_of, comp_children);
                    i += 1;
                }
            }
        }
    }
    let mut top = roots;
    extract_members(&mut top, &member_of, &mut comp_children);
    // Assigned entries may themselves contain nested assigned entries; extract within the
    // component buckets too (one pass per bucket is enough for direct nesting).
    let keys: Vec<usize> = comp_children.keys().copied().collect();
    for c in keys {
        let mut bucket = comp_children.remove(&c).unwrap();
        extract_members(&mut bucket, &member_of, &mut comp_children);
        match comp_children.entry(c) {
            std::collections::hash_map::Entry::Occupied(mut e) => e.get_mut().extend(bucket),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(bucket);
            }
        }
    }
    // Attach child components to their parents, deepest-first so nested chains assemble.
    // Order components by index; children append after member elements.
    let mut order: Vec<usize> = comp_children.keys().copied().collect();
    order.sort_unstable();
    // Depth of each component (root = 0), cycles cut by component_chain.
    let depth = |c: usize| doc.component_chain(c).len();
    order.sort_by_key(|&c| std::cmp::Reverse(depth(c)));
    for c in order {
        let children = comp_children.remove(&c).unwrap();
        let entry = HierarchyEntry {
            node: HierarchyNode::Component(c),
            children,
        };
        let parent = doc.components[c]
            .parent
            .filter(|p| comp_children.contains_key(p));
        match parent {
            Some(p) => comp_children.get_mut(&p).unwrap().push(entry),
            None => top.push(entry),
        }
    }
    top
}

/// Flat element list: parents always above descendants; newer elements after older ones when possible.
/// The unfiltered element list (used by tests and the scripting element API). The pane's List
/// view builds a [`filter_hierarchy`]-pruned tree and flattens it with [`element_list_from_tree`].
#[cfg_attr(not(test), allow(dead_code))]
pub fn build_element_list(
    doc: &Document,
    sketch_session: Option<SketchSession>,
) -> Vec<HierarchyNode> {
    let tree = build_hierarchy(doc, sketch_session);
    element_list_from_tree(&tree, doc)
}

/// Flatten an already-built (and possibly [`filter_hierarchy`]-pruned) tree into the List
/// view's node list. Ordering depends only on the element graph — the nesting tree plus the
/// dependency edges (inputs) — never on when elements were created (#540); `shape_order`
/// stays purely an undo/redo concern.
fn element_list_from_tree(tree: &[HierarchyEntry], doc: &Document) -> Vec<HierarchyNode> {
    let mut nodes = Vec::new();
    let mut parent_of = HashMap::new();
    for entry in tree {
        collect_with_parents(entry, None, &mut nodes, &mut parent_of);
    }
    // Input dependencies: each consumer must follow every input it's built from.
    let mut input_sources: HashMap<HierarchyNode, Vec<HierarchyNode>> = HashMap::new();
    for (source, consumer) in graph_dependency_edges(doc) {
        input_sources.entry(consumer).or_default().push(source);
    }
    topological_flat_sort(nodes, parent_of, input_sources)
}

/// User-facing element-type toggles for the Elements-pane filter (#275). Absent categories are
/// hidden; the default shows everything. The Drawing workbench narrows it to sketches + bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementFilter {
    pub planes: bool,
    pub sketches: bool,
    /// In-sketch geometry: lines, circles, constraints, and edge treatments.
    pub sketch_geometry: bool,
    pub bodies: bool,
    /// History operations: extrude, boolean, move, repeat, slice, revolve, and in-sketch ops.
    pub operations: bool,
    pub images: bool,
    pub drawings: bool,
    /// A drawing's **components** — its projections, text notes, and dimensions — separately
    /// from the drawing rows themselves (#381): page details are noise while modeling, so
    /// the Model workbench hides them by default (the Drawing workbench shows them).
    pub drawing_components: bool,
}

impl Default for ElementFilter {
    fn default() -> Self {
        Self {
            planes: true,
            sketches: true,
            sketch_geometry: true,
            bodies: true,
            operations: true,
            images: true,
            drawings: true,
            drawing_components: false,
        }
    }
}

impl ElementFilter {
    /// The Drawing workbench default: the sources you can add views from (sketches and bodies)
    /// plus the drawings themselves — so the open drawing's projections and text notes show in the
    /// Elements pane (#254/#275/#333).
    pub fn for_drawing_workbench() -> Self {
        Self {
            planes: false,
            sketches: true,
            sketch_geometry: false,
            bodies: true,
            operations: false,
            images: false,
            drawings: true,
            drawing_components: true,
        }
    }

    /// The toggles in display order: `(label, &mut enabled)` pairs the filter UI iterates.
    pub fn rows(&mut self) -> [(&'static str, &mut bool); 8] {
        [
            ("Planes", &mut self.planes),
            ("Sketches", &mut self.sketches),
            ("Sketch geometry", &mut self.sketch_geometry),
            ("Bodies", &mut self.bodies),
            ("Operations", &mut self.operations),
            ("Images", &mut self.images),
            ("Drawings", &mut self.drawings),
            ("Drawing components", &mut self.drawing_components),
        ]
    }

    /// Whether a node's type is currently shown. The synthetic Document root is always shown.
    fn shows(&self, node: HierarchyNode) -> bool {
        match node {
            HierarchyNode::Document => true,
            HierarchyNode::Component(_) => true,
            HierarchyNode::ConstructionPlane(_) => self.planes,
            HierarchyNode::Sketch(_) => self.sketches,
            HierarchyNode::Line(_)
            | HierarchyNode::Circle(_)
            | HierarchyNode::Constraint(_)
            | HierarchyNode::SketchText(_)
            | HierarchyNode::EdgeTreatment { .. } => self.sketch_geometry,
            HierarchyNode::Body(_) => self.bodies,
            HierarchyNode::Extrusion(_)
            | HierarchyNode::BooleanOp(_)
            | HierarchyNode::MoveOp(_)
            | HierarchyNode::MirrorOp(_)
            | HierarchyNode::RepeatOp(_)
            | HierarchyNode::SketchRepeatOp(_)
            | HierarchyNode::SketchOffsetOp(_)
            | HierarchyNode::SketchMirrorOp(_)
            | HierarchyNode::SketchVertexTreatmentOp(_)
            | HierarchyNode::SketchSliceOp(_)
            | HierarchyNode::SliceOp(_)
            | HierarchyNode::EdgeTreatmentOp(_)
            | HierarchyNode::Revolution(_)
            | HierarchyNode::SweepOp(_)
            | HierarchyNode::Loft(_) => self.operations,
            HierarchyNode::Image(_) => self.images,
            HierarchyNode::Drawing(_) => self.drawings,
            HierarchyNode::DrawingProjection { .. }
            | HierarchyNode::DrawingAnnotation { .. }
            | HierarchyNode::DrawingDimension { .. } => {
                self.drawings && self.drawing_components
            }
        }
    }
}

/// Prune a hierarchy tree to the enabled [`ElementFilter`] categories (#275). A hidden node is
/// dropped but its (recursively filtered) children are **promoted** to its parent — so hiding
/// "Operations" while keeping "Bodies" still shows the result bodies, just un-nested.
pub fn filter_hierarchy(tree: &[HierarchyEntry], filter: &ElementFilter) -> Vec<HierarchyEntry> {
    let mut out = Vec::new();
    for entry in tree {
        let children = filter_hierarchy(&entry.children, filter);
        if filter.shows(entry.node) {
            out.push(HierarchyEntry {
                node: entry.node,
                children,
            });
        } else {
            out.extend(children);
        }
    }
    out
}

fn collect_with_parents(
    entry: &HierarchyEntry,
    parent: Option<HierarchyNode>,
    nodes: &mut Vec<HierarchyNode>,
    parent_of: &mut HashMap<HierarchyNode, HierarchyNode>,
) {
    if let Some(parent) = parent {
        parent_of.insert(entry.node, parent);
    }
    nodes.push(entry.node);
    for child in &entry.children {
        collect_with_parents(child, Some(entry.node), nodes, parent_of);
    }
}

/// Flatten the element graph into a stable, **input-driven** order (#540): a node is emitted
/// only once its tree parent and every input it depends on have been emitted, so consumers
/// always follow their inputs. Among nodes with no such relationship the tiebreak is the node
/// itself (kind then index, via `HierarchyNode`'s derived `Ord`) — deterministic and
/// independent of creation time. `shape_order` is intentionally not consulted.
fn topological_flat_sort(
    nodes: Vec<HierarchyNode>,
    parent_of: HashMap<HierarchyNode, HierarchyNode>,
    input_sources: HashMap<HierarchyNode, Vec<HierarchyNode>>,
) -> Vec<HierarchyNode> {
    let mut remaining: HashSet<HierarchyNode> = nodes.into_iter().collect();
    let mut result = Vec::new();
    while !remaining.is_empty() {
        let mut ready: Vec<HierarchyNode> = remaining
            .iter()
            .filter(|node| {
                let parent_ready = parent_of
                    .get(node)
                    .map(|parent| !remaining.contains(parent))
                    .unwrap_or(true);
                let inputs_ready = input_sources
                    .get(node)
                    .map(|sources| sources.iter().all(|s| !remaining.contains(s)))
                    .unwrap_or(true);
                parent_ready && inputs_ready
            })
            .copied()
            .collect();
        // Defensive: a dependency cycle (never expected in a valid graph) would leave nothing
        // ready — release everything left, ordered deterministically, rather than looping.
        if ready.is_empty() {
            ready = remaining.iter().copied().collect();
        }
        ready.sort();
        for node in ready {
            remaining.remove(&node);
            result.push(node);
        }
    }
    result
}

/// The [`SceneElement`] a component member reference points at (#423). Drawings and lofts
/// are display-only (no scene element).
pub fn component_member_element(
    kind: crate::model::ComponentMember,
    index: usize,
) -> Option<SceneElement> {
    use crate::model::ComponentMember as CM;
    Some(match kind {
        CM::ConstructionPlane => SceneElement::ConstructionPlane(index),
        CM::Extrusion => SceneElement::Extrusion(index),
        CM::Body => SceneElement::Body(index),
        CM::BooleanOp => SceneElement::BooleanOp(index),
        CM::MoveOp => SceneElement::MoveOp(index),
        CM::MirrorOp => SceneElement::MirrorOp(index),
        CM::RepeatOp => SceneElement::RepeatOp(index),
        CM::SliceOp => SceneElement::SliceOp(index),
        CM::EdgeTreatmentOp => SceneElement::EdgeTreatmentOp(index),
        CM::Revolution => SceneElement::Revolution(index),
        CM::Sweep => SceneElement::SweepOp(index),
        CM::Loft | CM::Drawing => return None,
    })
}

/// The component a scene element belongs to (#423): a direct membership for top-level
/// kinds, or the membership of the root it nests under (a body via its producing
/// operation/extrusion, an extrusion or image via its sketch's host plane).
pub fn owning_component(doc: &Document, element: &SceneElement) -> Option<usize> {
    use crate::model::ComponentMember as CM;
    match element {
        SceneElement::Component(i) => doc.components.get(*i).and_then(|c| c.parent),
        SceneElement::ConstructionPlane(i) => {
            doc.component_of(CM::ConstructionPlane, *i).or_else(|| {
                match doc.construction_planes.get(*i)?.parent {
                    ConstructionPlaneParent::Root => None,
                    ConstructionPlaneParent::Sketch(s) => crate::model::sketch_component(doc, s),
                }
            })
        }
        SceneElement::Sketch(s) => crate::model::sketch_component(doc, *s),
        SceneElement::Extrusion(i) => doc.component_of(CM::Extrusion, *i).or_else(|| {
            doc.extrusions
                .get(*i)
                .and_then(|e| crate::model::sketch_component(doc, e.sketch))
        }),
        SceneElement::Body(i) => doc.component_of(CM::Body, *i).or_else(|| {
            use crate::model::BodySource;
            match &doc.bodies.get(*i)?.source {
                BodySource::Extrusion(e) => {
                    owning_component(doc, &SceneElement::Extrusion(*e))
                }
                BodySource::Extrusions(es) => es
                    .iter()
                    .find_map(|e| owning_component(doc, &SceneElement::Extrusion(*e))),
                BodySource::Imported(_) => None,
                BodySource::Loft(l) => doc.component_of(CM::Loft, *l),
                BodySource::Revolve(r) => doc.component_of(CM::Revolution, *r),
                BodySource::Sweep(f) => doc.component_of(CM::Sweep, *f),
                BodySource::Repeated { op, .. } => doc.component_of(CM::RepeatOp, *op),
                BodySource::Moved { op, .. } => doc.component_of(CM::MoveOp, *op),
                BodySource::Mirrored { op, .. } => doc.component_of(CM::MirrorOp, *op),
                BodySource::Boolean { op, .. } => doc.component_of(CM::BooleanOp, *op),
                BodySource::Sliced { op, .. } => doc.component_of(CM::SliceOp, *op),
                BodySource::EdgeTreated { op, .. } => {
                    doc.component_of(CM::EdgeTreatmentOp, *op)
                }
                BodySource::Solid { .. } => None,
            }
        }),
        SceneElement::Image(i) => doc
            .tracing_images
            .get(*i)
            .and_then(|img| owning_component(doc, &SceneElement::ConstructionPlane(img.plane))),
        SceneElement::BooleanOp(i) => doc.component_of(CM::BooleanOp, *i),
        SceneElement::MoveOp(i) => doc.component_of(CM::MoveOp, *i),
        SceneElement::MirrorOp(i) => doc.component_of(CM::MirrorOp, *i),
        SceneElement::RepeatOp(i) => doc.component_of(CM::RepeatOp, *i),
        SceneElement::SliceOp(i) => doc.component_of(CM::SliceOp, *i),
        SceneElement::EdgeTreatmentOp(i) => doc.component_of(CM::EdgeTreatmentOp, *i),
        SceneElement::Revolution(i) => doc.component_of(CM::Revolution, *i),
        SceneElement::SweepOp(i) => doc.component_of(CM::Sweep, *i),
        // In-sketch geometry cascades through its sketch's plane (handled by the sketch's
        // own effective-visibility recursion); everything else has no owning component.
        _ => None,
    }
}

fn parent_element(doc: &Document, element: SceneElement) -> Option<SceneElement> {
    match element {
        SceneElement::Component(index) => doc
            .components
            .get(index)
            .and_then(|c| c.parent)
            .map(SceneElement::Component),
        SceneElement::ConstructionPlane(index) => doc.construction_planes.get(index).and_then(
            |plane| match plane.parent {
                ConstructionPlaneParent::Root => None,
                ConstructionPlaneParent::Sketch(sketch) => Some(SceneElement::Sketch(sketch)),
            },
        ),
        SceneElement::Sketch(sketch) => doc
            .sketch_face(sketch)
            .map(face_element),
        SceneElement::Line(index) => doc
            .lines
            .get(index)
            .map(|line| SceneElement::Sketch(line.sketch)),
        SceneElement::Circle(index) => doc
            .circles
            .get(index)
            .map(|circle| SceneElement::Sketch(circle.sketch)),
        SceneElement::Constraint(index) => doc
            .constraints
            .get(index)
            .map(|c| SceneElement::Sketch(c.sketch)),
        SceneElement::Point(point) => point_parent_element(doc, point),
        // An extrusion depends on (and nests under) the sketch it was built from.
        SceneElement::Extrusion(index) => doc
            .extrusions
            .get(index)
            .map(|extrusion| SceneElement::Sketch(extrusion.sketch)),
        // A body depends on (and nests under) the feature that produced it; a merged body
        // nests under its first (originating) extrusion.
        SceneElement::Body(index) => doc.bodies.get(index).and_then(|body| {
            body.source
                .extrusion_indices()
                .first()
                .map(|&ei| SceneElement::Extrusion(ei))
        }),
        // A face's own edge isn't a hierarchy-pane node in its own right (it's a constraint
        // reference, not an independently listed element) — no parent to nest under.
        SceneElement::FaceEdge(_) | SceneElement::Origin => None,
        // Body sub-elements (#156/#555) likewise aren't pane nodes of their own.
        SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::BodyFace { .. } => None,
        // A tracing image nests under its host construction plane (#169).
        SceneElement::Image(index) => doc
            .tracing_images
            .get(index)
            .map(|img| SceneElement::ConstructionPlane(img.plane)),
        SceneElement::BooleanOp(_) => None,
        SceneElement::MoveOp(_) => None,
        SceneElement::MirrorOp(_) => None,
        SceneElement::RepeatOp(_) => None,
        SceneElement::SketchRepeatOp(_) => None,
        SceneElement::SketchOffsetOp(_) => None,
        SceneElement::SketchMirrorOp(_) => None,
        SceneElement::SketchVertexTreatmentOp(_) => None,
        SceneElement::SketchSliceOp(_) => None,
        // A sketch text nests under the sketch it lives in (#282).
        SceneElement::SketchText(index) => doc
            .sketch_texts
            .get(index)
            .map(|t| SceneElement::Sketch(t.sketch)),
        SceneElement::SliceOp(_) => None,
        SceneElement::EdgeTreatmentOp(_) => None,
        SceneElement::Revolution(_) => None,
        SceneElement::SweepOp(_) => None,
    }
}

fn point_parent_element(doc: &Document, point: ConstraintPoint) -> Option<SceneElement> {
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => doc
            .lines
            .get(line)
            .map(|_| SceneElement::Line(line)),
        ConstraintPoint::CircleCenter(circle) => Some(SceneElement::Circle(circle)),
        ConstraintPoint::TextAnchor { text, .. } => Some(SceneElement::SketchText(text)),
        ConstraintPoint::ImageCalibrationPoint { image, .. } => Some(SceneElement::Image(image)),
        // A face's own vertex nests under the extrusion that produced its face.
        ConstraintPoint::FaceVertex { face, .. } => {
            face.extrusion_index().map(SceneElement::Extrusion)
        }
    }
}

fn collect_ancestors(doc: &Document, element: SceneElement, out: &mut HashSet<SceneElement>) {
    let mut current = element;
    while let Some(parent) = parent_element(doc, current) {
        out.insert(parent.clone());
        current = parent;
    }
}

fn collect_descendants(doc: &Document, element: SceneElement, out: &mut HashSet<SceneElement>) {
    match element {
        SceneElement::Component(index) => {
            for (k, i, c) in doc.component_members.iter() {
                if *c != index {
                    continue;
                }
                if let Some(e) = component_member_element(*k, *i) {
                    out.insert(e.clone());
                    collect_descendants(doc, e, out);
                }
            }
            for (ci, comp) in doc.components.iter().enumerate() {
                if !comp.deleted && comp.parent == Some(index) {
                    out.insert(SceneElement::Component(ci));
                    collect_descendants(doc, SceneElement::Component(ci), out);
                }
            }
        }
        SceneElement::ConstructionPlane(index) => {
            let face = FaceId::ConstructionPlane(index);
            for sketch in doc.sketches_on_face(face) {
                out.insert(SceneElement::Sketch(sketch));
                collect_descendants(doc, SceneElement::Sketch(sketch), out);
            }
        }
        SceneElement::Sketch(sketch) => {
            for (li, line) in doc.lines.iter().enumerate() {
                if line.sketch == sketch {
                    out.insert(SceneElement::Line(li));
                }
            }
            for (ci, circle) in doc.circles.iter().enumerate() {
                if circle.sketch == sketch {
                    out.insert(SceneElement::Circle(ci));
                }
            }
            for (ci, constraint) in doc.constraints.iter().enumerate() {
                if constraint.sketch == sketch {
                    out.insert(SceneElement::Constraint(ci));
                }
            }
            for (ti, text) in doc.sketch_texts.iter().enumerate() {
                if !text.deleted && text.sketch == sketch {
                    out.insert(SceneElement::SketchText(ti));
                }
            }
            for (pi, plane) in doc.construction_planes.iter().enumerate() {
                if matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
                    out.insert(SceneElement::ConstructionPlane(pi));
                    collect_descendants(doc, SceneElement::ConstructionPlane(pi), out);
                }
            }
            for (ei, extrusion) in doc.extrusions.iter().enumerate() {
                if !extrusion.deleted && extrusion.sketch == sketch {
                    out.insert(SceneElement::Extrusion(ei));
                    collect_descendants(doc, SceneElement::Extrusion(ei), out);
                }
            }
        }
        SceneElement::Circle(index) => {
            for sketch in doc.sketches_on_face(FaceId::Circle(index)) {
                out.insert(SceneElement::Sketch(sketch));
                collect_descendants(doc, SceneElement::Sketch(sketch), out);
            }
        }
        SceneElement::Extrusion(index) => {
            for (bi, body) in doc.bodies.iter().enumerate() {
                if !body.deleted && body.source.owns_extrusion(index) {
                    out.insert(SceneElement::Body(bi));
                }
            }
            // Sketches placed on this extrusion's cap or side-wall faces.
            for (si, sketch) in doc.sketches.iter().enumerate() {
                if !sketch.deleted
                    && matches!(sketch.face,
                        FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. }
                        if extrusion == index)
                {
                    out.insert(SceneElement::Sketch(si));
                    collect_descendants(doc, SceneElement::Sketch(si), out);
                }
            }
        }
        SceneElement::Line(_)
        | SceneElement::Constraint(_)
        | SceneElement::Point(_)
        | SceneElement::Body(_)
        | SceneElement::FaceEdge(_)
        | SceneElement::Origin
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::BodyFace { .. }
        | SceneElement::SketchText(_)
        | SceneElement::Image(_) => {}
        SceneElement::BooleanOp(index) => {
            if let Some(op) = doc.boolean_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                }
            }
        }
        SceneElement::MoveOp(index) => {
            if let Some(op) = doc.move_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                }
            }
        }
        SceneElement::MirrorOp(index) => {
            if let Some(op) = doc.mirror_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                }
            }
        }
        SceneElement::RepeatOp(index) => {
            if let Some(op) = doc.repeat_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                }
                for &output in &op.plane_outputs {
                    out.insert(SceneElement::ConstructionPlane(output));
                }
            }
        }
        SceneElement::SketchRepeatOp(index) => {
            if let Some(op) = doc.sketch_repeat_ops.get(index) {
                for &output in &op.line_outputs {
                    out.insert(SceneElement::Line(output));
                }
                for &output in &op.circle_outputs {
                    out.insert(SceneElement::Circle(output));
                }
            }
        }
        SceneElement::SketchOffsetOp(index) => {
            if let Some(op) = doc.sketch_offset_ops.get(index) {
                for &output in &op.line_outputs {
                    out.insert(SceneElement::Line(output));
                }
                for &output in &op.circle_outputs {
                    out.insert(SceneElement::Circle(output));
                }
            }
        }
        SceneElement::SketchMirrorOp(index) => {
            if let Some(op) = doc.sketch_mirror_ops.get(index) {
                for &output in &op.line_outputs {
                    out.insert(SceneElement::Line(output));
                }
                for &output in &op.circle_outputs {
                    out.insert(SceneElement::Circle(output));
                }
            }
        }
        SceneElement::SketchVertexTreatmentOp(index) => {
            if let Some(op) = doc.sketch_vertex_treatment_ops.get(index) {
                for &output in op.line_outputs.iter().chain(op.bridge_outputs.iter()) {
                    out.insert(SceneElement::Line(output));
                }
            }
        }
        SceneElement::SketchSliceOp(index) => {
            if let Some(op) = doc.sketch_slice_ops.get(index) {
                for &output in &op.line_outputs {
                    out.insert(SceneElement::Line(output));
                }
            }
        }
        SceneElement::SliceOp(index) => {
            if let Some(op) = doc.slice_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                }
            }
        }
        SceneElement::EdgeTreatmentOp(index) => {
            if let Some(op) = doc.edge_treatment_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                }
            }
        }
        SceneElement::Revolution(index) => {
            // The revolved solid's output body is linked by `BodySource::Revolve`, not an
            // `outputs` list.
            for (bi, body) in doc.bodies.iter().enumerate() {
                if !body.deleted && body.source == crate::model::BodySource::Revolve(index) {
                    out.insert(SceneElement::Body(bi));
                    collect_descendants(doc, SceneElement::Body(bi), out);
                }
            }
        }
        SceneElement::SweepOp(index) => {
            // The swept solid's output body is linked by `BodySource::Sweep`.
            for (bi, body) in doc.bodies.iter().enumerate() {
                if !body.deleted && body.source == crate::model::BodySource::Sweep(index) {
                    out.insert(SceneElement::Body(bi));
                    collect_descendants(doc, SceneElement::Body(bi), out);
                }
            }
        }
    }
}

fn selection_anchor(element: &SceneElement) -> SceneElement {
    element.clone()
}

fn distance_target_touches_element(target: &DistanceTarget, element: &SceneElement) -> bool {
    match (target, element) {
        (DistanceTarget::LineLength(i), SceneElement::Line(j)) => i == j,
        (DistanceTarget::CircleDiameter(c), SceneElement::Circle(i)) => c == i,
        (DistanceTarget::LineLineDistance {
            line_a,
            line_b,
            side: _,
        }, element) => {
            constraint_line_touches_element(line_a, element)
                || constraint_line_touches_element(line_b, element)
        }
        (DistanceTarget::PointPointDistance { anchor, mover, .. }, element) => {
            constraint_point_touches_element(anchor, element)
                || constraint_point_touches_element(mover, element)
        }
        (DistanceTarget::PointLineDistance { point, line, .. }, element) => {
            constraint_point_touches_element(point, element)
                || constraint_line_touches_element(line, element)
        }
        _ => false,
    }
}

fn constraint_line_touches_element(line: &ConstraintLine, element: &SceneElement) -> bool {
    match (line, element) {
        (ConstraintLine::Line(i), SceneElement::Line(j)) => i == j,
        (
            ConstraintLine::Line(i),
            SceneElement::Point(ConstraintPoint::LineEndpoint { line, .. }),
        ) => i == line,
        (ConstraintLine::FaceEdge { face, index }, SceneElement::Point(ConstraintPoint::FaceVertex {
            face: f,
            index: i,
        })) => face == f && (*index == *i || (*index + 1) == *i),
        (ConstraintLine::FaceEdge { .. }, _) => false,
        _ => false,
    }
}

fn constraint_point_touches_element(point: &ConstraintPoint, element: &SceneElement) -> bool {
    match (point, element) {
        (p, SceneElement::Point(q)) => p == q,
        (ConstraintPoint::LineEndpoint { line, .. }, SceneElement::Line(i)) => line == i,
        (ConstraintPoint::CircleCenter(c), SceneElement::Circle(i)) => c == i,
        _ => false,
    }
}

fn constraint_entity_touches_element(entity: &ConstraintEntity, element: &SceneElement) -> bool {
    match entity {
        ConstraintEntity::Point(point) => constraint_point_touches_element(point, element),
        ConstraintEntity::Line(line) => constraint_line_touches_element(line, element),
        ConstraintEntity::Circle(circle) => *element == SceneElement::Circle(*circle),
        ConstraintEntity::Origin => false,
    }
}

fn constraint_kind_touches_element(kind: &ConstraintKind, element: &SceneElement) -> bool {
    match kind {
        ConstraintKind::Distance { target } => distance_target_touches_element(target, element),
        ConstraintKind::Parallel { line_a, line_b }
        | ConstraintKind::Perpendicular { line_a, line_b }
        | ConstraintKind::Equal { line_a, line_b } => {
            constraint_line_touches_element(line_a, element)
                || constraint_line_touches_element(line_b, element)
        }
        ConstraintKind::Coincident { a, b } => {
            constraint_entity_touches_element(a, element)
                || constraint_entity_touches_element(b, element)
        }
        ConstraintKind::Midpoint { point, line } => {
            constraint_point_touches_element(point, element)
                || constraint_line_touches_element(line, element)
        }
        ConstraintKind::Angle {
            line_a,
            line_b,
            rotation_sign: _,
        } => {
            constraint_line_touches_element(line_a, element)
                || constraint_line_touches_element(line_b, element)
        }
        ConstraintKind::Tangent { a, b } => {
            constraint_point_touches_element(a, element)
                || constraint_point_touches_element(b, element)
        }
    }
}

fn constraints_for_element(doc: &Document, element: SceneElement) -> Vec<usize> {
    doc.constraints
        .iter()
        .enumerate()
        .filter_map(|(index, constraint)| {
            constraint_kind_touches_element(&constraint.kind, &element).then_some(index)
        })
        .collect()
}

/// Constraint indices that apply to the current selection (for Elements pane highlighting).
pub fn selection_related_constraints(
    doc: &Document,
    selection: &SceneSelection,
) -> HashSet<usize> {
    let mut related = HashSet::new();
    for element in selection.iter() {
        let anchor = selection_anchor(&element);
        let anchor_differs = anchor != element;
        related.extend(constraints_for_element(doc, anchor));
        if anchor_differs {
            related.extend(constraints_for_element(doc, element));
        }
    }
    related
}

/// Selected elements plus their ancestors, descendants, and related constraints.
pub fn selection_context_elements(
    doc: &Document,
    selection: &SceneSelection,
) -> HashSet<SceneElement> {
    let mut context = HashSet::new();
    for element in selection.iter() {
        let anchor = selection_anchor(&element);
        context.insert(anchor.clone());
        collect_ancestors(doc, anchor.clone(), &mut context);
        collect_descendants(doc, anchor, &mut context);
    }
    for index in selection_related_constraints(doc, selection) {
        context.insert(SceneElement::Constraint(index));
    }
    context
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowStyle {
    Selected,
    RelatedConstraint,
    UsesVariable,
    Invalid,
    Unstable,
    InContext,
    Normal,
    Faint,
}

/// Accent for constraint rows tied to the current selection.
const RELATED_CONSTRAINT_TEXT: Color32 = Color32::from_rgb(255, 205, 88);
const INVALID_TEXT: Color32 = Color32::from_rgb(220, 80, 80);
const UNSTABLE_TEXT: Color32 = Color32::from_rgb(255, 180, 60);
/// Accent for rows whose dimension uses the focused variable.
const USES_VARIABLE_TEXT: Color32 = Color32::from_rgb(120, 215, 230);

fn row_is_selected(element: &SceneElement, selection: &SceneSelection) -> bool {
    selection.is_selected(element.clone())
}

/// Only dim the list when a selected element is actually shown in it.
fn selection_styles_visible_list(elements: &[HierarchyNode], selection: &SceneSelection) -> bool {
    if selection.is_empty() {
        return false;
    }
    let list_elements: HashSet<SceneElement> = elements
        .iter()
        .filter_map(|node| scene_element_for_node(*node))
        .collect();
    selection.iter().any(|element| {
        let anchor = selection_anchor(&element);
        list_elements.contains(&anchor)
    })
}

#[allow(clippy::too_many_arguments)]
fn row_style(
    element: SceneElement,
    selection: &SceneSelection,
    context: &HashSet<SceneElement>,
    related_constraints: &HashSet<usize>,
    style_selection: bool,
    health: &DocumentHealth,
    highlight_elements: &HashSet<SceneElement>,
    rolled_back: &HashSet<SceneElement>,
) -> RowStyle {
    // Timeline rollback (#524): elements created after the marker are inert, so fade them —
    // above everything else, so a rolled-back invalid/selected row still reads as inert.
    if rolled_back.contains(&element) {
        return RowStyle::Faint;
    }
    // Health tints the label/icon (red/amber). Selection highlight is applied separately
    // via `row_is_selected` so an invalid/unstable row can still show as selected (#511).
    match health.element_status(element.clone()) {
        HealthStatus::Invalid => return RowStyle::Invalid,
        HealthStatus::Unstable => return RowStyle::Unstable,
        HealthStatus::Healthy => {}
    }
    // A focused variable highlights the elements that use it, dimming the rest.
    if !highlight_elements.is_empty() {
        return if highlight_elements.contains(&element) {
            RowStyle::UsesVariable
        } else {
            RowStyle::Faint
        };
    }
    if !style_selection {
        return RowStyle::Normal;
    }
    if row_is_selected(&element, selection) {
        RowStyle::Selected
    } else if matches!(&element, SceneElement::Constraint(index) if related_constraints.contains(index)) {
        RowStyle::RelatedConstraint
    } else if context.contains(&element) {
        RowStyle::InContext
    } else {
        RowStyle::Faint
    }
}

/// Whether the row should paint the egui selected background — independent of health tint (#511).
fn row_shows_selection(
    element: &SceneElement,
    selection: &SceneSelection,
    style_selection: bool,
) -> bool {
    style_selection && row_is_selected(element, selection)
}

fn styled_label(label: &str, style: RowStyle) -> RichText {
    match style {
        RowStyle::Selected | RowStyle::InContext | RowStyle::Normal => RichText::new(label),
        RowStyle::RelatedConstraint => RichText::new(label).color(RELATED_CONSTRAINT_TEXT),
        RowStyle::UsesVariable => RichText::new(label).color(USES_VARIABLE_TEXT),
        RowStyle::Invalid => RichText::new(label).color(INVALID_TEXT),
        RowStyle::Unstable => RichText::new(label).color(UNSTABLE_TEXT),
        RowStyle::Faint => RichText::new(label).color(Color32::from_gray(120)),
    }
}

/// Paint the "active" marker (#429) inline as a small filled circle in the accent colour,
/// drawn by hand rather than as a `●` glyph: the default font lacks that codepoint, so the
/// glyph rendered as a tofu box before the active component/root name (#520). Allocates
/// roughly the footprint the `● ` prefix took so the label lines up as before.
fn active_marker_dot(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 14.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 3.0, crate::theme::FOCUS_ACCENT);
}

fn icon_tint_for_row_style(style: RowStyle) -> Color32 {
    match style {
        RowStyle::Selected | RowStyle::InContext | RowStyle::Normal => Color32::WHITE,
        RowStyle::RelatedConstraint => RELATED_CONSTRAINT_TEXT,
        RowStyle::UsesVariable => USES_VARIABLE_TEXT,
        RowStyle::Invalid => INVALID_TEXT,
        RowStyle::Unstable => UNSTABLE_TEXT,
        RowStyle::Faint => Color32::from_gray(120),
    }
}

/// Icon for a hierarchy row, or `None` when no existing icon fits (the synthetic Document
/// root — nothing in [`IconId`] represents "the whole document", so it renders without one).
fn icon_for_hierarchy_node(doc: &Document, node: HierarchyNode) -> Option<IconId> {
    Some(match node {
        HierarchyNode::Document => return None,
        HierarchyNode::Component(_) => IconId::Component,
        HierarchyNode::ConstructionPlane(_) => IconId::Plane,
        HierarchyNode::Sketch(_) => IconId::Sketch,
        HierarchyNode::Line(_) => IconId::Line,
        HierarchyNode::Circle(_) => IconId::Circle,
        HierarchyNode::Constraint(index) => doc
            .constraints
            .get(index)
            .map(|constraint| icon_for_constraint_kind(&constraint.kind))
            .unwrap_or(IconId::Constraint),
        HierarchyNode::Extrusion(_) => IconId::Extrude,
        HierarchyNode::Body(index) => {
            if doc.bodies.get(index).is_some_and(|b| b.shadow) {
                IconId::ShadowBody
            } else {
                IconId::Body
            }
        }
        // No dedicated image icon yet; the plane icon reads as "flat thing on a plane".
        HierarchyNode::Image(_) => IconId::Plane,
        HierarchyNode::BooleanOp(_) => IconId::Combine,
        HierarchyNode::MoveOp(_) => IconId::Move,
        HierarchyNode::MirrorOp(_) => IconId::Mirror,
        HierarchyNode::RepeatOp(_) => IconId::Repeat,
        HierarchyNode::SketchRepeatOp(_) => IconId::Repeat,
        HierarchyNode::SketchOffsetOp(_) => IconId::Offset,
        HierarchyNode::SketchMirrorOp(_) => IconId::Mirror,
        HierarchyNode::SketchVertexTreatmentOp(index) => {
            match doc
                .sketch_vertex_treatment_ops
                .get(index)
                .and_then(|o| o.corners.first())
                .map(|c| c.kind)
            {
                Some(crate::model::VertexTreatmentKind::Fillet) => IconId::Fillet,
                _ => IconId::Chamfer,
            }
        }
        HierarchyNode::SketchSliceOp(_) => IconId::Slice,
        HierarchyNode::SketchText(_) => IconId::Text,
        HierarchyNode::SliceOp(_) => IconId::Slice,
        HierarchyNode::EdgeTreatmentOp(index) => {
            match doc.edge_treatment_ops.get(index).map(|o| o.kind) {
                Some(crate::model::VertexTreatmentKind::Fillet) => IconId::Fillet,
                _ => IconId::Chamfer,
            }
        }
        HierarchyNode::Revolution(_) => IconId::Revolve,
        HierarchyNode::SweepOp(_) => IconId::Sweep,
        HierarchyNode::Loft(_) => IconId::Loft,
        HierarchyNode::EdgeTreatment { extrusion, index } => {
            match edge_treatment_at(doc, extrusion, index).map(|t| t.kind) {
                Some(crate::model::VertexTreatmentKind::Chamfer) => IconId::Chamfer,
                _ => IconId::Fillet,
            }
        }
        HierarchyNode::Drawing(_) => IconId::Drawing,
        HierarchyNode::DrawingProjection { .. } => IconId::Projection,
        HierarchyNode::DrawingAnnotation { .. } => IconId::Text,
        HierarchyNode::DrawingDimension { .. } => IconId::Dimension,
    })
}

/// The [`EdgeTreatment`] a [`HierarchyNode::EdgeTreatment`] points at, if it still exists.
fn edge_treatment_at(
    doc: &Document,
    extrusion: usize,
    index: usize,
) -> Option<&crate::model::EdgeTreatment> {
    doc.extrusions
        .get(extrusion)
        .and_then(|ext| ext.edge_treatments.get(index))
}

/// Primary double-click on a row label (fallback when [`egui::Response::double_clicked`] misses).
fn row_primary_double_clicked(response: &egui::Response, ui: &egui::Ui) -> bool {
    if response.double_clicked() {
        return true;
    }
    let pointer_double = ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
    if !pointer_double {
        return false;
    }
    let pos = response
        .interact_pointer_pos()
        .or_else(|| ui.input(|i| i.pointer.interact_pos()));
    pos.is_some_and(|pos| response.rect.contains(pos))
}

/// How a sketch row should react to pointer input this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchRowAction {
    None,
    Select { additive: bool },
    Edit,
}

pub fn sketch_row_action(double_clicked: bool, clicked: bool, additive: bool) -> SketchRowAction {
    if double_clicked {
        SketchRowAction::Edit
    } else if clicked {
        SketchRowAction::Select { additive }
    } else {
        SketchRowAction::None
    }
}

fn build_face_sketches(
    doc: &Document,
    face: FaceId,
    sketch_session: Option<SketchSession>,
) -> Vec<HierarchyEntry> {
    doc.sketches_on_face(face)
        .filter(|sketch| sketch_alive(doc, *sketch))
        .map(|sketch| build_sketch_entry(doc, sketch, sketch_session))
        .collect()
}

fn build_sketch_child_planes(
    doc: &Document,
    sketch: SketchId,
    sketch_session: Option<SketchSession>,
) -> Vec<HierarchyEntry> {
    let mut children = Vec::new();
    for (pi, plane) in doc.construction_planes.iter().enumerate() {
        if plane.deleted || !matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
            continue;
        }
        let face = FaceId::ConstructionPlane(pi);
        children.push(HierarchyEntry {
            node: HierarchyNode::ConstructionPlane(pi),
            children: build_face_sketches(doc, face, sketch_session),
        });
    }
    children
}

/// Whether a construction plane is a generated host for a repeated-sketch copy (#226/#231) —
/// those group under their repeat operation, not at the top level.
fn is_repeat_sketch_host_plane(doc: &Document, pi: usize) -> bool {
    doc.repeat_ops
        .iter()
        .any(|op| !op.deleted && op.sketch_plane_outputs.contains(&pi))
}

/// Whether a line is a fragment/copy generated by an in-sketch repeat (#222/#228) — those are
/// listed under their operation node, not under the sketch directly.
fn is_sketch_repeat_line_output(doc: &Document, li: usize) -> bool {
    doc.sketch_repeat_ops
        .iter()
        .any(|op| !op.deleted && op.line_outputs.contains(&li))
        || doc
            .sketch_slice_ops
            .iter()
            .any(|op| !op.deleted && op.line_outputs.contains(&li))
        || doc
            .sketch_offset_ops
            .iter()
            .any(|op| !op.deleted && op.line_outputs.contains(&li))
        || doc.sketch_vertex_treatment_ops.iter().any(|op| {
            !op.deleted && (op.line_outputs.contains(&li) || op.bridge_outputs.contains(&li))
        })
}

fn is_sketch_repeat_circle_output(doc: &Document, ci: usize) -> bool {
    doc.sketch_repeat_ops
        .iter()
        .any(|op| !op.deleted && op.circle_outputs.contains(&ci))
        || doc
            .sketch_offset_ops
            .iter()
            .any(|op| !op.deleted && op.circle_outputs.contains(&ci))
}

fn build_sketch_entry(
    doc: &Document,
    sketch: SketchId,
    sketch_session: Option<SketchSession>,
) -> HierarchyEntry {
    let mut children = build_sketch_child_planes(doc, sketch, sketch_session);

    if sketch_session.is_some_and(|s| s.sketch == sketch) {
        for (li, line) in doc.lines.iter().enumerate() {
            if line.deleted || line.sketch != sketch || is_sketch_repeat_line_output(doc, li) {
                continue;
            }
            let entry = HierarchyEntry {
                node: HierarchyNode::Line(li),
                children: vec![],
            };
            // A chamfer/fillet bridging line (#76) nests under the (lower-index) trimmed line
            // it came from, rather than sitting as an ordinary sibling. Since `chamfer_fillet_
            // parent` is always a lower line index, and `doc.lines` is iterated in index order,
            // the parent's entry is always already in `children` by the time we get here. If
            // the parent is gone (tombstoned) or otherwise not found — same graceful-orphan
            // handling as elsewhere in this file — fall back to a top-level sibling instead of
            // dropping the bridging line from the tree.
            if let Some(parent) = line.chamfer_fillet_parent {
                let alive_parent = doc
                    .lines
                    .get(parent)
                    .is_some_and(|p| !p.deleted && p.sketch == sketch);
                if alive_parent {
                    if let Some(parent_entry) = children
                        .iter_mut()
                        .find(|e| e.node == HierarchyNode::Line(parent))
                    {
                        parent_entry.children.push(entry);
                        continue;
                    }
                }
            }
            children.push(entry);
        }
        for (ci, circle) in doc.circles.iter().enumerate() {
            if circle.deleted || circle.sketch != sketch || is_sketch_repeat_circle_output(doc, ci) {
                continue;
            }
            let nested = build_face_sketches(doc, FaceId::Circle(ci), sketch_session);
            children.push(HierarchyEntry {
                node: HierarchyNode::Circle(ci),
                children: nested,
            });
        }
        for (ci, constraint) in doc.constraints.iter().enumerate() {
            if constraint.deleted || constraint.sketch != sketch {
                continue;
            }
            children.push(HierarchyEntry {
                node: HierarchyNode::Constraint(ci),
                children: vec![],
            });
        }
        for (ti, text) in doc.sketch_texts.iter().enumerate() {
            if text.deleted || text.sketch != sketch {
                continue;
            }
            children.push(HierarchyEntry {
                node: HierarchyNode::SketchText(ti),
                children: vec![],
            });
        }
    } else {
        for (ci, circle) in doc.circles.iter().enumerate() {
            if circle.deleted || circle.sketch != sketch || is_sketch_repeat_circle_output(doc, ci) {
                continue;
            }
            let nested = build_face_sketches(doc, FaceId::Circle(ci), sketch_session);
            if !nested.is_empty() {
                children.push(HierarchyEntry {
                    node: HierarchyNode::Circle(ci),
                    children: nested,
                });
            }
        }
    }

    // Extrusions built from this sketch nest under it (each owns its Body).
    children.extend(build_sketch_extrusions(doc, sketch, sketch_session));
    // Sweeps whose profile faces live in this sketch nest under it too (#478), each
    // owning its output body — so the graph shows the sketch (the faces' proxy) as the
    // op's input rather than the document root.
    for (oi, fp) in doc.sweeps.iter().enumerate() {
        if fp.deleted || fp.sketch != sketch {
            continue;
        }
        let bodies = doc
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.deleted && b.source == crate::model::BodySource::Sweep(oi))
            .map(|(bi, _)| HierarchyEntry {
                node: HierarchyNode::Body(bi),
                children: Vec::new(),
            })
            .collect();
        children.push(HierarchyEntry {
            node: HierarchyNode::SweepOp(oi),
            children: bodies,
        });
    }

    HierarchyEntry {
        node: HierarchyNode::Sketch(sketch),
        children,
    }
}

/// Hierarchy entries for the extrusions produced from `sketch`, each owning the
/// body it created and any sketches placed on its cap faces.
fn build_sketch_extrusions(
    doc: &Document,
    sketch: SketchId,
    sketch_session: Option<SketchSession>,
) -> Vec<HierarchyEntry> {
    doc.extrusions
        .iter()
        .enumerate()
        .filter(|(_, extrusion)| !extrusion.deleted && extrusion.sketch == sketch)
        .map(|(ei, _)| {
            let mut children: Vec<HierarchyEntry> = doc
                .bodies
                .iter()
                .enumerate()
                .filter(|(_, body)| !body.deleted && body.source.owns_extrusion(ei))
                .map(|(bi, _)| HierarchyEntry {
                    node: HierarchyNode::Body(bi),
                    children: Vec::new(),
                })
                .collect();
            for (si, sk) in doc.sketches.iter().enumerate() {
                if !sk.deleted
                    && matches!(sk.face,
                        FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. }
                        if extrusion == ei)
                {
                    children.push(build_sketch_entry(doc, si, sketch_session));
                }
            }
            // Edge chamfers/fillets applied to this extrusion (#192) show as leaves under it,
            // right-clickable to edit their amount.
            for ti in 0..doc.extrusions[ei].edge_treatments.len() {
                children.push(HierarchyEntry {
                    node: HierarchyNode::EdgeTreatment {
                        extrusion: ei,
                        index: ti,
                    },
                    children: Vec::new(),
                });
            }
            HierarchyEntry {
                node: HierarchyNode::Extrusion(ei),
                children,
            }
        })
        .collect()
}

pub fn node_label(doc: &Document, node: HierarchyNode) -> String {
    names::node_label(doc, node)
}

/// Draw the elements list in a side panel.
#[allow(clippy::too_many_arguments)]
pub fn show_pane(
    ui: &mut egui::Ui,
    doc: &Document,
    sketch_session: Option<SketchSession>,
    visibility: &mut ElementVisibility,
    selection: &SceneSelection,
    health: &DocumentHealth,
    view_mode: &mut HierarchyViewMode,
    graph_layout: &mut GraphLayout,
    graph_force: &mut bool,
    filter: &mut ElementFilter,
    filter_expanded: &mut bool,
    on_edit_sketch: &mut impl FnMut(SketchId),
    on_edit_plane: &mut impl FnMut(usize),
    on_import_image_on_plane: &mut impl FnMut(usize),
    on_edit_extrusion: &mut impl FnMut(usize),
    on_edit_edge_treatment: &mut impl FnMut(usize, usize),
    on_edit_edge_treatment_op: &mut impl FnMut(usize),
    on_edit_operation: &mut impl FnMut(SceneElement),
    on_edit_drawing: &mut impl FnMut(usize),
    on_select_drawing_element: &mut impl FnMut(HierarchyNode),
    on_hover_drawing_element: &mut impl FnMut(Option<HierarchyNode>),
    selected_drawing_leaf: Option<HierarchyNode>,
    on_rename_drawing: &mut impl FnMut(usize, String),
    on_export_body: &mut impl FnMut(usize),
    on_export_body_step: &mut impl FnMut(usize),
    on_export_component: &mut impl FnMut(usize),
    on_export_component_step: &mut impl FnMut(usize),
    on_toggle_visibility: &mut impl FnMut(SceneElement, bool),
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_hover_element: &mut impl FnMut(SceneElement),
    on_delete_element: &mut impl FnMut(SceneElement),
    // `active_drawing`: the open drawing (Drawing workbench) enabling the row "Add to
    // drawing" action (#274); `on_add_to_drawing` receives the body index.
    active_drawing: Option<usize>,
    on_add_to_drawing: &mut impl FnMut(SceneElement),
    highlight_elements: &HashSet<SceneElement>,
    rolled_back: &HashSet<SceneElement>,
    // The current timeline rollback marker (#524), if any, and a setter (None clears it).
    rollback_marker: Option<&RollbackMarker>,
    on_set_rollback: &mut impl FnMut(Option<RollbackMarker>),
    collapsed_components: &mut HashSet<usize>,
    on_add_component: &mut impl FnMut(Option<usize>),
    on_move_to_component: &mut impl FnMut(SceneElement, Option<usize>),
    active_component: Option<usize>,
    on_activate_component: &mut impl FnMut(Option<usize>),
) {
    ui.horizontal(|ui| {
        ui.heading(PANE_TITLE);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The Tree view is retired (#252): a strict tree can't show an element with multiple
            // inputs (e.g. a body that's both an op output and another op's input), so only List
            // and the dependency-aware Graph remain. The enum variant stays for script
            // back-compat; a lingering `Tree` mode renders as List (see the match below).
            for (mode, icon, tooltip) in [
                (HierarchyViewMode::Graph, IconId::ViewGraph, "Graph-node view"),
                (HierarchyViewMode::List, IconId::ViewList, "List view"),
            ] {
                let selected =
                    *view_mode == mode || (mode == HierarchyViewMode::List && *view_mode == HierarchyViewMode::Tree);
                if selectable_icon_button(ui, icon, selected, tooltip).clicked() {
                    *view_mode = mode;
                }
            }
            // Force-layout toggle (#525): only meaningful in the Graph view. When on, nodes
            // repel and space themselves; when off, the layout freezes so a busy graph holds
            // still to read and drag.
            if *view_mode == HierarchyViewMode::Graph {
                if selectable_icon_button(
                    ui,
                    IconId::GraphForce,
                    *graph_force,
                    "Force layout — auto-space nodes",
                )
                .clicked()
                {
                    *graph_force = !*graph_force;
                }
            }
            // Add menu (#423): the + opens a popup with creatable containers.
            let add = selectable_icon_button(ui, IconId::Plus, false, "Add…");
            egui::Popup::menu(&add).show(|ui| {
                if ui.button("New component").clicked() {
                    on_add_component(None);
                    ui.close();
                }
            });
        });
    });
    ui.separator();

    // Timeline rollback status (#524/#545): when rolled back, show the marker and a Clear button
    // to roll forward. Setting a rollback point is done per-element from the row's right-click
    // "Rollback" submenu (#545), not a header button.
    if let Some(marker) = rollback_marker {
        ui.horizontal(|ui| {
            let noun = if marker.inclusive { "just before" } else { "" };
            ui.label(
                egui::RichText::new(format!(
                    "⏮ Rolled back to {noun} {}",
                    crate::names::scene_element_label(doc, &marker.element)
                ))
                .color(crate::theme::FOCUS_ACCENT)
                .size(11.5),
            );
            if ui
                .small_button("Clear")
                .on_hover_text("Roll forward — re-enable everything after this point")
                .clicked()
            {
                on_set_rollback(None);
            }
        });
        ui.separator();
    }

    let context = selection_context_elements(doc, selection);
    let related_constraints = selection_related_constraints(doc, selection);

    // Drag feedback (#430): while an Elements-pane row is being dragged toward a
    // component, a floating name tag follows the cursor and the cursor shows grabbing.
    if let Some(payload) = egui::DragAndDrop::payload::<ComponentDragPayload>(ui.ctx()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            let painter = ui
                .ctx()
                .layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("component_drag_tag")));
            let label = crate::names::scene_element_label(doc, &payload.0);
            let galley = painter.layout_no_wrap(
                label,
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
            let rect = egui::Rect::from_min_size(
                pos + egui::vec2(12.0, 8.0),
                galley.size() + egui::vec2(10.0, 6.0),
            );
            painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(40, 60, 90, 230));
            painter.galley(rect.min + egui::vec2(5.0, 3.0), galley, Color32::WHITE);
        }
    }

    match view_mode {
        // `Tree` is retired (#252); a lingering script-set Tree mode falls back to List.
        HierarchyViewMode::List | HierarchyViewMode::Tree => {
            let tree = filter_hierarchy(&build_hierarchy(doc, sketch_session), filter);
            let rows = component_list_rows(&tree, doc, collapsed_components);
            let elements: Vec<HierarchyNode> = rows.iter().map(|(n, _)| *n).collect();
            let style_selection = selection_styles_visible_list(&elements, selection);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (node, base_depth) in rows {
                    // Component rows render inline (#423): triangle, eye, icon, name; they
                    // collapse their contents and accept row drops.
                    if let HierarchyNode::Component(ci) = node {
                        show_component_row(
                            ui,
                            doc,
                            ci,
                            base_depth,
                            visibility,
                            selection,
                            health,
                            &context,
                            &related_constraints,
                            style_selection,
                            highlight_elements,
                            rolled_back,
                            collapsed_components,
                            active_component,
                            on_toggle_visibility,
                            on_click_element,
                            on_delete_element,
                            on_add_component,
                            on_move_to_component,
                            on_export_component,
                            on_export_component_step,
                        );
                        continue;
                    }
                    // When editing a sketch, indent that sketch's own components one level so they
                    // read as belonging to it (#244).
                    let row_depth = base_depth
                        + match (sketch_session, node) {
                            (Some(s), HierarchyNode::Line(i))
                                if doc.lines.get(i).is_some_and(|l| l.sketch == s.sketch) =>
                            {
                                1
                            }
                            (Some(s), HierarchyNode::Circle(i))
                                if doc.circles.get(i).is_some_and(|c| c.sketch == s.sketch) =>
                            {
                                1
                            }
                            (Some(s), HierarchyNode::Constraint(i))
                                if doc.constraints.get(i).is_some_and(|c| c.sketch == s.sketch) =>
                            {
                                1
                            }
                            _ => 0,
                        };
                    show_row(
                        ui,
                        doc,
                        node,
                        row_depth,
                        visibility,
                        selection,
                        health,
                        &context,
                        &related_constraints,
                        style_selection,
                        on_edit_sketch,
                        on_edit_plane,
                        on_import_image_on_plane,
                        on_edit_extrusion,
                        on_edit_edge_treatment,
                        on_edit_edge_treatment_op,
                        on_edit_operation,
                        on_edit_drawing,
                        on_select_drawing_element,
                        on_hover_drawing_element,
                        selected_drawing_leaf,
                        on_rename_drawing,
                        on_export_body,
                        on_export_body_step,
                        on_set_rollback,
                        on_toggle_visibility,
                        on_click_element,
                        on_hover_element,
                        on_delete_element,
                        active_drawing,
                        on_add_to_drawing,
                        highlight_elements,
                        rolled_back,
                        on_move_to_component,
                        active_component,
                        on_activate_component,
                    );
                }
            });
        }
        HierarchyViewMode::Graph => {
            let tree = filter_hierarchy(&build_hierarchy(doc, sketch_session), filter);
            show_graph_view(
                ui,
                doc,
                &tree,
                graph_layout,
                *graph_force,
                selection,
                health,
                &context,
                &related_constraints,
                on_click_element,
                on_hover_element,
                on_delete_element,
                highlight_elements,
                rolled_back,
            );
        }
    }

    // Filter control (#275): a button at the pane's bottom that expands up into a set of
    // per-type show/hide toggles.
    egui::TopBottomPanel::bottom("elements_filter")
        .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(4, 3)))
        .show_inside(ui, |ui| {
            if *filter_expanded {
                let all_on = filter.rows().iter().all(|(_, e)| **e);
                // Icon-group toggles (#382): each category is a toggleable button showing the
                // icons of the element types it covers (hover for the name). Laid out like text
                // — flowing left-to-right and wrapping to the next line (#526) — so every button
                // stays visible in a narrow pane instead of a tall column crowding the list.
                {
                    use crate::icons::IconId as I;
                    let ElementFilter {
                        planes,
                        sketches,
                        sketch_geometry,
                        bodies,
                        operations,
                        images,
                        drawings,
                        drawing_components,
                    } = filter;
                    let groups: [(&str, &[I], &mut bool); 8] = [
                        ("Planes", &[I::Plane], planes),
                        ("Sketches", &[I::Sketch], sketches),
                        ("Sketch components", &[I::SketchComponents], sketch_geometry),
                        ("Bodies", &[I::Body], bodies),
                        ("Operations", &[I::Extrude, I::Revolve, I::Combine], operations),
                        ("Images", &[I::Image], images),
                        ("Drawings", &[I::Drawing], drawings),
                        ("Drawing components", &[I::DrawingComponents], drawing_components),
                    ];
                    ui.horizontal_wrapped(|ui| {
                        for (label, icons, enabled) in groups {
                            if crate::icons::selectable_icon_group(ui, icons, *enabled, label)
                                .clicked()
                            {
                                *enabled = !*enabled;
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.small_button(if all_on { "Hide all" } else { "Show all" }).clicked() {
                        let target = !all_on;
                        for (_, enabled) in filter.rows() {
                            *enabled = target;
                        }
                    }
                    if ui.small_button("Done").clicked() {
                        *filter_expanded = false;
                    }
                });
            } else {
                let hidden = filter.rows().iter().filter(|(_, e)| !**e).count();
                let label = if hidden == 0 {
                    "Filter".to_string()
                } else {
                    format!("Filter ({hidden} hidden)")
                };
                let button = egui::Button::image_and_text(
                    crate::icons::sized_texture(ui.ctx(), crate::icons::IconId::Filter),
                    label,
                );
                if ui.add(button).on_hover_text("Show/hide element types").clicked() {
                    *filter_expanded = true;
                }
            }
        });
}

/// Accent stroke for graph-view edges/nodes among the selected node's ancestors and
/// descendants. Row styling has no direct line-drawing equivalent to reuse, so this is a
/// dedicated bold accent, distinct from the node fill colors (which do reuse
/// [`icon_tint_for_row_style`] for consistency with the List/Tree views).
const GRAPH_RELATED_EDGE: Color32 = Color32::from_rgb(120, 200, 255);
/// Dashed dependency edge (input, not parent) in the graph view — e.g. a drawing projection to
/// its source body (#281). A warm accent so it reads apart from the neutral parent edges.
const GRAPH_DEPENDENCY_EDGE: Color32 = Color32::from_rgb(224, 168, 96);

/// Render the graph-node view: a force-directed node-link diagram (#94). Nodes are pulled into
/// depth-ordered horizontal layers (so the graph flows top-to-bottom, "somewhat vertical"),
/// repelled from one another, and joined by parent↔child springs; the simulation animates each
/// frame ("bounce around") until its kinetic energy decays below a threshold, then settles and
/// stops requesting repaints. x is contained to the pane width; height scrolls vertically (#34).
#[allow(clippy::too_many_arguments)]
/// Whether a node is present in the current graph positions (#423).
fn present_in(positions: &[GraphNodePosition], node: &HierarchyNode) -> bool {
    positions.iter().any(|p| p.node == *node)
}

/// A smooth convex outline around `pts`, padded by `pad` px (#423): each point is expanded
/// into a small circle of sample points and the convex hull of the expansion is returned,
/// which rounds the corners without a curve primitive.
fn rounded_hull(pts: &[egui::Pos2], pad: f32) -> Vec<egui::Pos2> {
    let mut cloud: Vec<egui::Pos2> = Vec::with_capacity(pts.len() * 8);
    for p in pts {
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::TAU / 8.0;
            cloud.push(*p + egui::vec2(a.cos(), a.sin()) * pad);
        }
    }
    convex_hull(&mut cloud)
}

/// Andrew's monotone chain convex hull.
fn convex_hull(points: &mut Vec<egui::Pos2>) -> Vec<egui::Pos2> {
    points.sort_by(|a, b| {
        a.x.partial_cmp(&b.x)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
    });
    points.dedup_by(|a, b| (*a - *b).length_sq() < 1e-6);
    if points.len() < 3 {
        return points.clone();
    }
    let cross = |o: egui::Pos2, a: egui::Pos2, b: egui::Pos2| {
        (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
    };
    let mut lower: Vec<egui::Pos2> = Vec::new();
    for &p in points.iter() {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<egui::Pos2> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[allow(clippy::too_many_arguments)]
fn show_graph_view(
    ui: &mut egui::Ui,
    doc: &Document,
    tree: &[HierarchyEntry],
    graph_layout: &mut GraphLayout,
    force_enabled: bool,
    selection: &SceneSelection,
    health: &DocumentHealth,
    context: &HashSet<SceneElement>,
    related_constraints: &HashSet<usize>,
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_hover_element: &mut impl FnMut(SceneElement),
    on_delete_element: &mut impl FnMut(SceneElement),
    highlight_elements: &HashSet<SceneElement>,
    rolled_back: &HashSet<SceneElement>,
) {
    let positions = graph_node_positions(tree);
    if positions.is_empty() {
        return;
    }

    const NODE_RADIUS: f32 = 9.0;
    const TOP_PADDING: f32 = 24.0;
    const BOTTOM_PADDING: f32 = 24.0;
    // Per-frame integration: a handful of small substeps keeps the sim stable while settling
    // within a second or so of wall-clock animation.
    const SUBSTEPS: u32 = 6;
    const DT: f32 = 0.16;
    // Below this total kinetic energy the layout is considered settled; stop animating so an
    // idle pane doesn't busy-repaint.
    const SETTLE_KE: f32 = 0.05;

    // Nodes matching the current selection, plus their tree ancestors/descendants (#34): the
    // set of related nodes whose edges/fills get the bold accent.
    let mut related_nodes: HashSet<HierarchyNode> = HashSet::new();
    for position in &positions {
        if let Some(element) = scene_element_for_node(position.node) {
            if row_is_selected(&element, selection) {
                related_nodes.extend(graph_related_nodes(tree, position.node));
            }
        }
    }
    // Only dim unrelated nodes once something is actually selected — same convention as
    // `selection_styles_visible_list` uses for the List/Tree rows.
    let style_selection = !selection.is_empty();

    let available_width = ui.available_width().max(2.0 * GRAPH_MARGIN + 1.0);

    // Advance the physics, then keep animating until it settles. With the force layout off
    // (#525) the nodes stay synced but frozen, so no repaint is scheduled.
    let kinetic =
        graph_layout.sync_and_step(&positions, available_width, SUBSTEPS, DT, force_enabled);
    if kinetic > SETTLE_KE {
        ui.ctx().request_repaint();
    }

    // Measure each node's label, then spread each depth band horizontally so no two labels
    // overlap (#248). Widths are capped at the pane width so one very long label can't demand
    // absurd spacing; the render truncates to the pane edge regardless.
    let label_widths: HashMap<HierarchyNode, f32> = positions
        .iter()
        .map(|p| {
            let label = node_label(doc, p.node);
            let w = ui.fonts(|f| {
                f.layout_no_wrap(label, egui::FontId::default(), Color32::WHITE)
                    .size()
                    .x
            });
            (p.node, w.min(available_width))
        })
        .collect();
    let sim_x: HashMap<HierarchyNode, f32> = positions
        .iter()
        .filter_map(|p| graph_layout.pos_of(p.node).map(|v| (p.node, v.x)))
        .collect();
    // Each node gets an x within the pane and a sub-row within its depth band; wide bands wrap
    // into stacked sub-rows so the graph fits the pane width and grows taller instead (#350).
    let display = declutter_label_bands(&positions, &sim_x, &label_widths, available_width);

    // Vertical layout: bands stack top-to-bottom, each as tall as its wrapped sub-row count, so
    // the whole graph fits the pane width (no horizontal scroll) and only grows downward (#350).
    const ROW_H: f32 = 46.0;
    let depth_of: HashMap<HierarchyNode, usize> =
        positions.iter().map(|p| (p.node, p.depth)).collect();
    let row_of = |n: &HierarchyNode| display.get(n).map(|(_, r)| *r).unwrap_or(0);
    let mut band_rows: BTreeMap<usize, usize> = BTreeMap::new();
    for p in &positions {
        let rows = row_of(&p.node) + 1;
        band_rows
            .entry(p.depth)
            .and_modify(|m| *m = (*m).max(rows))
            .or_insert(rows);
    }
    let mut base_y: HashMap<usize, f32> = HashMap::new();
    let mut acc = 0.0f32;
    for (&depth, &rows) in &band_rows {
        base_y.insert(depth, acc);
        acc += rows as f32 * ROW_H;
    }
    let node_y = move |n: &HierarchyNode| -> f32 {
        let base = depth_of.get(n).and_then(|d| base_y.get(d)).copied().unwrap_or(0.0);
        base + row_of(n) as f32 * ROW_H
    };

    let content_width = available_width;
    let content_height = acc + TOP_PADDING + BOTTOM_PADDING + NODE_RADIUS;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (rect, _response) =
                ui.allocate_exact_size(egui::vec2(content_width, content_height), egui::Sense::hover());
            let painter = ui.painter_at(rect);

            let drag_offsets: HashMap<HierarchyNode, egui::Vec2> = positions
                .iter()
                .map(|p| (p.node, graph_layout.drag_offset(p.node)))
                .collect();
            let pos_of = |node: HierarchyNode| -> egui::Pos2 {
                let x = display.get(&node).map(|(x, _)| *x).unwrap_or(GRAPH_MARGIN);
                let offset = drag_offsets.get(&node).copied().unwrap_or(egui::Vec2::ZERO);
                egui::pos2(rect.left() + x, rect.top() + TOP_PADDING + node_y(&node)) + offset
            };
            let mut dragged_delta: Option<(HierarchyNode, egui::Vec2)> = None;

            // Component areas first, beneath everything (#423): each component shades a
            // smooth convex region encompassing its member nodes. Outer components paint
            // before nested ones so the nesting reads as layered tints.
            let mut comp_sets = component_node_sets(tree);
            comp_sets.sort_by_key(|(ci, _)| doc.component_chain(*ci).len());
            for (ci, nodes) in &comp_sets {
                let pts: Vec<egui::Pos2> =
                    nodes.iter().filter(|n| present_in(&positions, n)).map(|n| pos_of(*n)).collect();
                if pts.is_empty() {
                    continue;
                }
                let hull = rounded_hull(&pts, 18.0);
                if hull.len() >= 3 {
                    painter.add(egui::Shape::convex_polygon(
                        hull.clone(),
                        Color32::from_rgba_unmultiplied(140, 160, 200, 18),
                        egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(150, 170, 210, 60)),
                    ));
                    // Label the area at its top edge.
                    let top = hull
                        .iter()
                        .copied()
                        .min_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(egui::Pos2::ZERO);
                    painter.text(
                        top + egui::vec2(0.0, -2.0),
                        egui::Align2::CENTER_BOTTOM,
                        node_label(doc, HierarchyNode::Component(*ci)),
                        egui::FontId::proportional(11.0),
                        Color32::from_rgba_unmultiplied(170, 190, 230, 160),
                    );
                }
            }

            // Edges first, so node dots paint over the line endpoints.
            for position in &positions {
                let Some(parent) = position.parent else { continue };
                let highlighted =
                    related_nodes.contains(&position.node) && related_nodes.contains(&parent);
                let stroke = if highlighted {
                    egui::Stroke::new(2.5, GRAPH_RELATED_EDGE)
                } else {
                    egui::Stroke::new(1.0, Color32::from_gray(110))
                };
                painter.line_segment([pos_of(parent), pos_of(position.node)], stroke);
            }

            // Dependency edges (input → consumer): relationships beyond the single tree parent —
            // a drawing projection to its source (#281), and a boolean operation to its shadow
            // input bodies (#266). Drawn dashed in an accent colour so they read apart from the
            // neutral parent edges. (A step toward the full element graph, #252.)
            let present: std::collections::HashSet<HierarchyNode> =
                positions.iter().map(|p| p.node).collect();
            for (source, consumer) in graph_dependency_edges(doc) {
                if !present.contains(&source) || !present.contains(&consumer) {
                    continue;
                }
                let (a, b) = (pos_of(source), pos_of(consumer));
                // Manual dashes so it's visually distinct without a dashed-line primitive.
                let delta = b - a;
                let len = delta.length();
                if len < 1.0 {
                    continue;
                }
                let dir = delta / len;
                let dash = 5.0;
                let mut t = 0.0;
                while t < len {
                    let s = a + dir * t;
                    let e = a + dir * (t + dash).min(len);
                    painter.line_segment([s, e], egui::Stroke::new(1.2, GRAPH_DEPENDENCY_EDGE));
                    t += dash * 2.0;
                }
            }

            for position in &positions {
                let center = pos_of(position.node);
                let element = scene_element_for_node(position.node);
                let style = element.clone().map(|el| {
                    row_style(
                        el.clone(),
                        selection,
                        context,
                        related_constraints,
                        style_selection,
                        health,
                        highlight_elements,
                        rolled_back,
                    )
                });
                // Selection fills white even when health tints the icon red/amber (#511).
                let selected = element
                    .as_ref()
                    .is_some_and(|el| row_shows_selection(el, selection, style_selection));
                let related = related_nodes.contains(&position.node);
                let fill = if selected {
                    Color32::WHITE
                } else if related {
                    GRAPH_RELATED_EDGE
                } else {
                    style.map(icon_tint_for_row_style).unwrap_or(Color32::from_gray(170))
                };

                let node_rect =
                    egui::Rect::from_center_size(center, egui::Vec2::splat(NODE_RADIUS * 2.0));
                let id = ui.id().with(("hierarchy_graph_node", position.node));
                let response = ui.interact(node_rect, id, egui::Sense::click_and_drag());
                // Nodes are draggable (#451): the offset persists on top of the layout.
                if response.dragged() {
                    dragged_delta = Some((position.node, response.drag_delta()));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    // Pane-hover → viewport highlight (#161).
                    if let Some(element) = element.clone() {
                        on_hover_element(element);
                    }
                }
                if let Some(element) = element {
                    let response = response.on_hover_text(node_label(doc, position.node));
                    if response.clicked() {
                        let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                        on_click_element(element.clone(), additive);
                    }
                    response.context_menu(|ui| {
                        if ui.button("Delete").clicked() {
                            on_delete_element(element.clone());
                            ui.close();
                        }
                    });
                }

                // Each node draws as its element's icon (#152) — the same icon the List/Tree
                // rows use, tinted by selection/health state; only the synthetic Document
                // root (which has no icon) keeps the plain dot.
                if let Some(icon) = icon_for_hierarchy_node(doc, position.node) {
                    crate::icons::paint_icon(&painter, ui.ctx(), icon, node_rect, fill);
                } else {
                    painter.circle_filled(center, NODE_RADIUS, fill);
                    painter.circle_stroke(
                        center,
                        NODE_RADIUS,
                        egui::Stroke::new(1.0, Color32::from_gray(30)),
                    );
                }

                let label = node_label(doc, position.node);
                // Keep the label inside the pane's right edge (#34).
                let max_label_width =
                    (rect.right() - (center.x + NODE_RADIUS + 4.0) - 4.0).max(20.0);
                let truncated = truncate_label(&label, max_label_width, &painter);
                painter.text(
                    center + egui::vec2(NODE_RADIUS + 4.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    truncated,
                    egui::FontId::default(),
                    if selected || related { Color32::WHITE } else { Color32::from_gray(200) },
                );
            }
            if let Some((node, delta)) = dragged_delta {
                graph_layout.add_drag_offset(node, delta);
                ui.ctx().request_repaint();
            }
        });
}

/// Truncate `label` (with an ellipsis) so it fits within `max_width` pixels at the default
/// font — graph-node labels must stay inside their column (#34).
fn truncate_label(label: &str, max_width: f32, painter: &egui::Painter) -> String {
    let font_id = egui::FontId::default();
    let galley_width =
        |s: &str| -> f32 { painter.layout_no_wrap(s.to_string(), font_id.clone(), Color32::WHITE).size().x };
    if galley_width(label) <= max_width {
        return label.to_string();
    }
    let mut truncated = String::new();
    for ch in label.chars() {
        let candidate = format!("{truncated}{ch}…");
        if galley_width(&candidate) > max_width {
            break;
        }
        truncated.push(ch);
    }
    format!("{truncated}…")
}

/// Flatten the tree into List-view rows with component nesting (#423): loose elements
/// first (flat, topologically sorted, depth `base`), then each component row with its
/// contents indented one level; collapsed components skip their contents.
fn component_list_rows(
    tree: &[HierarchyEntry],
    doc: &Document,
    collapsed: &HashSet<usize>,
) -> Vec<(HierarchyNode, usize)> {
    fn level(
        entries: &[HierarchyEntry],
        doc: &Document,
        collapsed: &HashSet<usize>,
        base: usize,
        out: &mut Vec<(HierarchyNode, usize)>,
    ) {
        let (components, loose): (Vec<&HierarchyEntry>, Vec<&HierarchyEntry>) = entries
            .iter()
            .partition(|e| matches!(e.node, HierarchyNode::Component(_)));
        let loose_owned: Vec<HierarchyEntry> = loose.into_iter().cloned().collect();
        for node in element_list_from_tree(&loose_owned, doc) {
            out.push((node, base));
        }
        for entry in components {
            let HierarchyNode::Component(ci) = entry.node else { unreachable!() };
            out.push((entry.node, base));
            if !collapsed.contains(&ci) {
                level(&entry.children, doc, collapsed, base + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    // The tree is the single synthetic Document root; its children are the real entries.
    for root in tree {
        level(&root.children, doc, collapsed, 1, &mut out);
    }
    out
}

/// One component row in the List view (#423): collapse triangle, eye toggle, icon, name;
/// click selects, right-click offers a nested component / delete; rows dropped on it move
/// into the component.
#[allow(clippy::too_many_arguments)]
fn show_component_row(
    ui: &mut egui::Ui,
    doc: &Document,
    ci: usize,
    depth: usize,
    visibility: &mut ElementVisibility,
    selection: &SceneSelection,
    health: &DocumentHealth,
    context: &HashSet<SceneElement>,
    related_constraints: &HashSet<usize>,
    style_selection: bool,
    highlight_elements: &HashSet<SceneElement>,
    rolled_back: &HashSet<SceneElement>,
    collapsed_components: &mut HashSet<usize>,
    active_component: Option<usize>,
    on_toggle_visibility: &mut impl FnMut(SceneElement, bool),
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_delete_element: &mut impl FnMut(SceneElement),
    on_add_component: &mut impl FnMut(Option<usize>),
    on_move_to_component: &mut impl FnMut(SceneElement, Option<usize>),
    on_export_component: &mut impl FnMut(usize),
    on_export_component_step: &mut impl FnMut(usize),
) {
    let element = SceneElement::Component(ci);
    let visible = visibility.effective_visible(doc, element.clone());
    let style = row_style(
        element.clone(),
        selection,
        context,
        related_constraints,
        style_selection,
        health,
        highlight_elements,
        rolled_back,
    );
    let row = ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 18.0);
        let collapsed = collapsed_components.contains(&ci);
        let (tri_rect, tri_resp) =
            ui.allocate_exact_size(egui::vec2(12.0, 14.0), egui::Sense::click());
        let c = tri_rect.center();
        let r = 4.0;
        let pts = if collapsed {
            vec![
                egui::pos2(c.x - r * 0.5, c.y - r),
                egui::pos2(c.x + r, c.y),
                egui::pos2(c.x - r * 0.5, c.y + r),
            ]
        } else {
            vec![
                egui::pos2(c.x - r, c.y - r * 0.5),
                egui::pos2(c.x + r, c.y - r * 0.5),
                egui::pos2(c.x, c.y + r),
            ]
        };
        ui.painter().add(egui::Shape::convex_polygon(
            pts,
            Color32::from_gray(170),
            egui::Stroke::NONE,
        ));
        if tri_resp
            .on_hover_text(if collapsed { "Expand" } else { "Collapse" })
            .clicked()
        {
            if collapsed {
                collapsed_components.remove(&ci);
            } else {
                collapsed_components.insert(ci);
            }
        }
        if icon_button(
            ui,
            icon_for_visibility(visible),
            if visible { "Hide" } else { "Show" },
        )
        .clicked()
        {
            let next = visibility.toggle(element.clone());
            on_toggle_visibility(element.clone(), next);
        }
        ui.add(
            egui::Image::new(sized_texture(ui.ctx(), IconId::Component))
                .tint(icon_tint_for_row_style(style)),
        );
        let label = node_label(doc, HierarchyNode::Component(ci));
        // The active component (#429) — where new elements land — reads in the accent
        // colour with a painted dot marker (#520).
        let text = if active_component == Some(ci) {
            active_marker_dot(ui);
            RichText::new(label).color(crate::theme::FOCUS_ACCENT)
        } else {
            styled_label(&label, style)
        };
        let response = ui.selectable_label(
            row_shows_selection(&element, selection, style_selection),
            text,
        );
        if response.clicked() {
            let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
            on_click_element(element.clone(), additive);
        }
        // Component rows drag too, to re-parent into another component (#423).
        response
            .interact(egui::Sense::drag())
            .dnd_set_drag_payload(ComponentDragPayload(element.clone()));
        response.context_menu(|ui| {
            if ui.button("New component inside").clicked() {
                on_add_component(Some(ci));
                ui.close();
            }
            // Export the component's bodies (#521): everything filed into it and its nested
            // components, as one STL/STEP file.
            if ui.button("Export STL…").clicked() {
                on_export_component(ci);
                ui.close();
            }
            if ui.button("Export STEP…").clicked() {
                on_export_component_step(ci);
                ui.close();
            }
            if ui.button("Move to document root").clicked() {
                on_move_to_component(element.clone(), None);
                ui.close();
            }
            if ui.button("Delete").clicked() {
                on_delete_element(element.clone());
                ui.close();
            }
        });
    });
    // Whole-row drop target (#430): rect-based so releasing over any child widget (the
    // name label, the icon) still lands the drop — `Response::dnd_release_payload` misses
    // when a child covers the pointer.
    let row_rect = row.response.rect;
    let dragging =
        egui::DragAndDrop::has_payload_of_type::<ComponentDragPayload>(ui.ctx());
    if dragging && ui.rect_contains_pointer(row_rect) {
        ui.painter().rect_stroke(
            row_rect,
            2.0,
            egui::Stroke::new(1.5, crate::theme::FOCUS_ACCENT),
            egui::StrokeKind::Inside,
        );
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(payload) =
                egui::DragAndDrop::take_payload::<ComponentDragPayload>(ui.ctx())
            {
                if payload.0 != element {
                    on_move_to_component(payload.0.clone(), Some(ci));
                }
            }
        }
    }
}

fn show_row(
    ui: &mut egui::Ui,
    doc: &Document,
    node: HierarchyNode,
    depth: usize,
    visibility: &mut ElementVisibility,
    selection: &SceneSelection,
    health: &DocumentHealth,
    context: &HashSet<SceneElement>,
    related_constraints: &HashSet<usize>,
    style_selection: bool,
    on_edit_sketch: &mut impl FnMut(SketchId),
    on_edit_plane: &mut impl FnMut(usize),
    on_import_image_on_plane: &mut impl FnMut(usize),
    on_edit_extrusion: &mut impl FnMut(usize),
    on_edit_edge_treatment: &mut impl FnMut(usize, usize),
    on_edit_edge_treatment_op: &mut impl FnMut(usize),
    on_edit_operation: &mut impl FnMut(SceneElement),
    on_edit_drawing: &mut impl FnMut(usize),
    on_select_drawing_element: &mut impl FnMut(HierarchyNode),
    on_hover_drawing_element: &mut impl FnMut(Option<HierarchyNode>),
    selected_drawing_leaf: Option<HierarchyNode>,
    on_rename_drawing: &mut impl FnMut(usize, String),
    on_export_body: &mut impl FnMut(usize),
    on_export_body_step: &mut impl FnMut(usize),
    on_set_rollback: &mut impl FnMut(Option<RollbackMarker>),
    on_toggle_visibility: &mut impl FnMut(SceneElement, bool),
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_hover_element: &mut impl FnMut(SceneElement),
    on_delete_element: &mut impl FnMut(SceneElement),
    active_drawing: Option<usize>,
    on_add_to_drawing: &mut impl FnMut(SceneElement),
    highlight_elements: &HashSet<SceneElement>,
    rolled_back: &HashSet<SceneElement>,
    on_move_to_component: &mut impl FnMut(SceneElement, Option<usize>),
    active_component: Option<usize>,
    on_activate_component: &mut impl FnMut(Option<usize>),
) {
    // The synthetic Document root has no SceneElement — it isn't selectable, hideable, or
    // otherwise dispatched through the scene graph — so it gets a minimal, always-shown row
    // and returns before any of the SceneElement-keyed lookups below. Every other row is
    // indented `depth` levels (List always passes 1, matching #87's original single level;
    // Tree passes the node's real depth in the nested hierarchy, #34).
    if matches!(node, HierarchyNode::Document) {
        let row = ui.horizontal(|ui| {
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let active_root =
                active_component.is_none() && doc.components.iter().any(|c| !c.deleted);
            if active_root {
                // With components present, mark where new elements land (#429): the
                // document root, unless a component is active. Painted dot, not a `●`
                // glyph, so it renders even when the font lacks that codepoint (#520).
                active_marker_dot(ui);
            }
            let text = if active_root {
                RichText::new(node_label(doc, node))
                    .color(crate::theme::FOCUS_ACCENT)
                    .strong()
            } else {
                RichText::new(node_label(doc, node)).strong()
            };
            let resp = ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .on_hover_text("Click to make new elements land at the document root");
            if resp.clicked() {
                on_activate_component(None);
            }
        });
        // Dropping a dragged row on the Document root moves it out of any component
        // (#423/#430): rect-based, like the component rows.
        let row_rect = row.response.rect;
        if egui::DragAndDrop::has_payload_of_type::<ComponentDragPayload>(ui.ctx())
            && ui.rect_contains_pointer(row_rect)
        {
            ui.painter().rect_stroke(
                row_rect,
                2.0,
                egui::Stroke::new(1.5, crate::theme::FOCUS_ACCENT),
                egui::StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(payload) =
                    egui::DragAndDrop::take_payload::<ComponentDragPayload>(ui.ctx())
                {
                    on_move_to_component(payload.0.clone(), None);
                }
            }
        }
        return;
    }

    // An edge chamfer/fillet (#192): a display-only leaf with no `SceneElement`. Editing is done
    // by bringing back its push/pull gizmo + amount input (#259) — either double-click the row or
    // right-click → "Edit"; it doesn't participate in selection/visibility.
    if let HierarchyNode::EdgeTreatment { extrusion, index } = node {
        let Some(treatment) = edge_treatment_at(doc, extrusion, index) else {
            return;
        };
        let noun = match treatment.kind {
            crate::model::VertexTreatmentKind::Chamfer => "chamfer",
            crate::model::VertexTreatmentKind::Fillet => "fillet",
        };
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 18.0);
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let response = ui.selectable_label(false, node_label(doc, node));
            if response.double_clicked() {
                on_edit_edge_treatment(extrusion, index);
            }
            response.context_menu(|ui| {
                if ui.button(format!("Edit {noun}")).clicked() {
                    on_edit_edge_treatment(extrusion, index);
                    ui.close();
                }
            });
        });
        return;
    }

    // A technical drawing (#180): a display-only leaf with no `SceneElement`. Clicking the row
    // or its right-click "Edit drawing" opens the drawing pane.
    if let HierarchyNode::Drawing(index) = node {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 18.0);
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let response = ui.selectable_label(false, node_label(doc, node));
            if response.clicked() {
                on_edit_drawing(index);
            }
            response.context_menu(|ui| {
                if ui.button("Edit drawing").clicked() {
                    on_edit_drawing(index);
                    ui.close();
                }
                // Rename (#255): an inline field seeded from the current name, held in egui temp
                // memory while the menu is open.
                ui.separator();
                ui.label("Rename");
                let id = ui.make_persistent_id(("rename_drawing", index));
                let current = doc
                    .drawings
                    .get(index)
                    .and_then(|d| d.name.clone())
                    .unwrap_or_default();
                let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or(current);
                let resp = ui.text_edit_singleline(&mut text);
                let commit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.data_mut(|d| d.insert_temp(id, text.clone()));
                if commit || ui.button("Apply name").clicked() {
                    on_rename_drawing(index, text);
                    ui.data_mut(|d| d.remove::<String>(id));
                    ui.close();
                }
            });
        });
        return;
    }

    // A loft operation (#252): a display-only row (no SceneElement yet); its output body nests
    // beneath it and its sketch inputs show as graph edges.
    if matches!(node, HierarchyNode::Loft(_)) {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 18.0);
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let _ = ui.selectable_label(false, node_label(doc, node));
        });
        return;
    }

    // A drawing projection (#281), text note (#333), or dimension (#341): a display-only leaf.
    // Clicking opens the drawing and selects that element (like clicking a sketch's child), so
    // its editor opens and it highlights on the page.
    if let HierarchyNode::DrawingProjection { drawing, .. }
    | HierarchyNode::DrawingAnnotation { drawing, .. }
    | HierarchyNode::DrawingDimension { drawing, .. } = node
    {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 18.0);
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let resp = ui.selectable_label(
                selected_drawing_leaf == Some(node),
                node_label(doc, node),
            );
            if resp.clicked() {
                on_edit_drawing(drawing);
                on_select_drawing_element(node);
            }
            if resp.hovered() {
                on_hover_drawing_element(Some(node));
            }
        });
        return;
    }

    let element = scene_element_for_node(node)
        .expect("non-Document HierarchyNode always maps to a SceneElement");
    if !element_alive(doc, element.clone()) {
        return;
    }
    let visible = visibility.effective_visible(doc, element.clone());
    let style = row_style(
        element.clone(),
        selection,
        context,
        related_constraints,
        style_selection,
        health,
        highlight_elements,
        rolled_back,
    );

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 18.0);
        if icon_button(
            ui,
            icon_for_visibility(visible),
            if visible { "Hide" } else { "Show" },
        )
        .clicked()
        {
            let next = visibility.toggle(element.clone());
            on_toggle_visibility(element.clone(), next);
        }

        let icon_response = icon_for_hierarchy_node(doc, node).map(|icon| {
            ui.add(
                egui::Image::new(sized_texture(ui.ctx(), icon))
                    .tint(icon_tint_for_row_style(style)),
            )
        });

        let label = node_label(doc, node);
        let response = ui.selectable_label(
            row_shows_selection(&element, selection, style_selection),
            styled_label(&label, style),
        );
        // Pane-hover → viewport highlight (#161): the 3D view shows what this row is.
        if response.hovered() {
            on_hover_element(element.clone());
        }
        // With a drawing open, body and sketch rows drag onto the page (#290): the drop
        // places a projection at the pointer. Both the name label and the type icon are
        // grab handles (#368). Re-sensed for drag so the payload arms; plain clicks still
        // select as usual.
        if active_drawing.is_some()
            && matches!(node, HierarchyNode::Body(_) | HierarchyNode::Sketch(_))
        {
            response
                .interact(egui::Sense::drag())
                .dnd_set_drag_payload(DrawingDragPayload(element.clone()));
            if let Some(icon_resp) = icon_response {
                icon_resp
                    .interact(egui::Sense::drag())
                    .dnd_set_drag_payload(DrawingDragPayload(element.clone()));
            }
        }
        // Top-level rows drag onto component rows to move into them (#423).
        if component_member_node(node) && active_drawing.is_none() {
            response
                .interact(egui::Sense::drag())
                .dnd_set_drag_payload(ComponentDragPayload(element.clone()));
        }
        // Clicks: double-click edits (where applicable), single-click selects.
        match node {
            HierarchyNode::Document => unreachable!("handled by the early return above"),
            HierarchyNode::Sketch(sketch) => {
                let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                match sketch_row_action(
                    row_primary_double_clicked(&response, ui),
                    response.clicked(),
                    additive,
                ) {
                    SketchRowAction::Edit => on_edit_sketch(sketch),
                    SketchRowAction::Select { additive } => {
                        on_click_element(element.clone(), additive)
                    }
                    SketchRowAction::None => {}
                }
            }
            HierarchyNode::Extrusion(index) => {
                if row_primary_double_clicked(&response, ui) {
                    on_edit_extrusion(index);
                } else if response.clicked() {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
            HierarchyNode::EdgeTreatmentOp(index) => {
                if row_primary_double_clicked(&response, ui) {
                    on_edit_edge_treatment_op(index);
                } else if response.clicked() {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
            // Handled by the early return above (no SceneElement).
            HierarchyNode::EdgeTreatment { .. } | HierarchyNode::Drawing(_) => unreachable!(),
            // Every other operation edits the universal way: double-click reopens it in its tool
            // (#546); a plain click selects it.
            node if node_editable_operation(node).is_some() => {
                if row_primary_double_clicked(&response, ui) {
                    on_edit_operation(element.clone());
                } else if response.clicked() {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
            _ => {
                if response.clicked() {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
        }
        // One context menu per element row: any node-specific actions, then a universal Delete
        // so any element can be deleted from the pane (#253).
        response.context_menu(|ui| {
            match node {
                HierarchyNode::Sketch(sketch) => {
                    if ui.button("Edit sketch").clicked() {
                        on_edit_sketch(sketch);
                        ui.close();
                    }
                    // In the Drawing workbench, add this sketch as a projection (#278).
                    if active_drawing.is_some() && ui.button("Add to drawing").clicked() {
                        on_add_to_drawing(SceneElement::Sketch(sketch));
                        ui.close();
                    }
                }
                HierarchyNode::ConstructionPlane(index) => {
                    if ui.button("Edit plane").clicked() {
                        on_edit_plane(index);
                        ui.close();
                    }
                    if ui.button("Import image on this plane…").clicked() {
                        on_import_image_on_plane(index);
                        ui.close();
                    }
                }
                HierarchyNode::Extrusion(index) => {
                    if ui.button("Edit extrusion").clicked() {
                        on_edit_extrusion(index);
                        ui.close();
                    }
                }
                HierarchyNode::EdgeTreatmentOp(index) => {
                    let noun = match doc.edge_treatment_ops.get(index).map(|o| o.kind) {
                        Some(crate::model::VertexTreatmentKind::Fillet) => "fillet",
                        _ => "chamfer",
                    };
                    if ui.button(format!("Edit {noun}")).clicked() {
                        on_edit_edge_treatment_op(index);
                        ui.close();
                    }
                }
                HierarchyNode::Body(index) => {
                    // In the Drawing workbench, add this body as a view of the open drawing (#274).
                    if active_drawing.is_some() && ui.button("Add to drawing").clicked() {
                        on_add_to_drawing(SceneElement::Body(index));
                        ui.close();
                    }
                    if ui.button("Export STL…").clicked() {
                        on_export_body(index);
                        ui.close();
                    }
                    if ui.button("Export STEP…").clicked() {
                        on_export_body_step(index);
                        ui.close();
                    }
                }
                // Every other operation edits the universal way: right-click → "Edit" (#546).
                node if node_editable_operation(node).is_some() => {
                    if ui.button("Edit").clicked() {
                        on_edit_operation(element.clone());
                        ui.close();
                    }
                }
                _ => {}
            }
            // Move to component (#423): every top-level row can be filed into a component
            // (or back to the document root) from its context menu; dragging works too.
            if component_member_node(node)
                && doc.components.iter().any(|c| !c.deleted)
            {
                ui.menu_button("Move to", |ui| {
                    if ui.button("Document").clicked() {
                        on_move_to_component(element.clone(), None);
                        ui.close();
                    }
                    for (ci, c) in doc.components.iter().enumerate() {
                        if c.deleted {
                            continue;
                        }
                        if ui
                            .button(node_label(doc, HierarchyNode::Component(ci)))
                            .clicked()
                        {
                            on_move_to_component(element.clone(), Some(ci));
                            ui.close();
                        }
                    }
                });
            }
            // Timeline rollback (#545): roll the model back relative to this element — to just
            // after it (keeping it, hiding its dependents) or to just before it (hiding it too).
            // Only elements that are graph nodes can be rollback points.
            if hierarchy_node_for_element(&element).is_some() {
                ui.menu_button("Rollback", |ui| {
                    if ui.button("Rollback to here").clicked() {
                        on_set_rollback(Some(RollbackMarker {
                            element: element.clone(),
                            inclusive: false,
                        }));
                        ui.close();
                    }
                    if ui.button("Rollback to just before here").clicked() {
                        on_set_rollback(Some(RollbackMarker {
                            element: element.clone(),
                            inclusive: true,
                        }));
                        ui.close();
                    }
                });
            }
            if ui.button("Delete").clicked() {
                on_delete_element(element.clone());
                ui.close();
            }
        });
    });
}

/// Whether a hierarchy node is a top-level kind a component can hold (#423).
fn component_member_node(node: HierarchyNode) -> bool {
    matches!(
        node,
        HierarchyNode::ConstructionPlane(_)
            | HierarchyNode::Extrusion(_)
            | HierarchyNode::Body(_)
            | HierarchyNode::BooleanOp(_)
            | HierarchyNode::MoveOp(_)
            | HierarchyNode::RepeatOp(_)
            | HierarchyNode::SliceOp(_)
            | HierarchyNode::Revolution(_)
            | HierarchyNode::SweepOp(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ShapeKind;

    /// #448/#449: every operation's inputs appear as graph dependency edges — the
    /// repeat's input body was the reported gap.
    #[test]
    fn graph_dependency_edges_cover_operation_inputs() {
        let mut doc = Document::default();
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        doc.repeat_ops.push(crate::model::RepeatOperation {
            targets: vec![0],
            plane_targets: vec![0],
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            mode: crate::model::RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            name: None,
            deleted: false,
        });
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.revolutions.push(crate::model::Revolution {
            sketch,
            faces: Vec::new(),
            axis: crate::model::RevolveAxis::Line(0),
            angle_deg: 360.0,
            symmetric: false,
            mode: crate::model::RevolveMode::NewBody,
            name: None,
            deleted: false,
        });
        let edges = graph_dependency_edges(&doc);
        assert!(edges.contains(&(HierarchyNode::Body(0), HierarchyNode::RepeatOp(0))));
        assert!(edges.contains(&(
            HierarchyNode::ConstructionPlane(0),
            HierarchyNode::RepeatOp(0)
        )));
        assert!(edges.contains(&(HierarchyNode::Sketch(sketch), HierarchyNode::Revolution(0))));
        assert!(edges.contains(&(HierarchyNode::Line(0), HierarchyNode::Revolution(0))));
    }

    /// #423: assigned roots nest under their component entry in the built hierarchy, and the
    /// List rows indent them one level (skipping contents when collapsed).
    #[test]
    fn components_group_roots_in_hierarchy_and_list() {
        use crate::model::ComponentMember as CM;
        let mut doc = Document::default();
        doc.components.push(crate::model::Component {
            name: Some("Frame".to_string()),
            parent: None,
            length_unit: None,
            angle_unit: None,
            deleted: false,
        });
        let plane = doc.construction_planes.len();
        doc.construction_planes.push(crate::face::default_xy_plane());
        doc.set_component_member(CM::ConstructionPlane, plane, Some(0));

        let tree = build_hierarchy(&doc, None);
        let root = &tree[0];
        let comp = root
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Component(0))
            .expect("component entry present");
        assert!(
            comp.children.iter().any(|e| e.node == HierarchyNode::ConstructionPlane(plane)),
            "assigned plane nests under the component"
        );
        assert!(
            !root.children.iter().any(|e| e.node == HierarchyNode::ConstructionPlane(plane)),
            "assigned plane no longer sits at the top level"
        );

        // List rows: the component at depth 1, its plane at depth 2; collapsing hides it.
        let rows = component_list_rows(&tree, &doc, &HashSet::new());
        let comp_row = rows.iter().find(|(n, _)| *n == HierarchyNode::Component(0)).unwrap();
        assert_eq!(comp_row.1, 1);
        let plane_row = rows
            .iter()
            .find(|(n, _)| *n == HierarchyNode::ConstructionPlane(plane))
            .unwrap();
        assert_eq!(plane_row.1, 2, "component contents indent one level");
        let collapsed: HashSet<usize> = [0].into_iter().collect();
        let rows = component_list_rows(&tree, &doc, &collapsed);
        assert!(
            !rows.iter().any(|(n, _)| *n == HierarchyNode::ConstructionPlane(plane)),
            "collapsed component hides its contents"
        );
    }
    use crate::construction::{definition_from_reference, plane_from_definition};
    use crate::face::default_xy_plane;
    use crate::construction::PlaneReference;
    use crate::model::{ConstructionPlaneParent, Line};

    /// An imported (STL/STEP) body has no source extrusions; it must still be effectively
    /// visible by default — `any()` over the empty extrusion list used to read as hidden,
    /// making imported bodies invisible to every effective-visibility consumer.
    #[test]
    fn imported_body_is_effectively_visible_by_default() {
        let mut doc = Document::default();
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles: vec![[glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y]],
            source_name: "part".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let mut visibility = ElementVisibility::default();
        assert!(visibility.effective_visible(&doc, SceneElement::Body(0)));
        visibility.set_visible(SceneElement::Body(0), false);
        assert!(!visibility.effective_visible(&doc, SceneElement::Body(0)));
    }

    /// #266: a boolean operation's shadow input bodies feed it as dependency edges in the graph.
    #[test]
    fn boolean_op_inputs_are_graph_dependencies() {
        let mut doc = Document::default();
        for _ in 0..3 {
            doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Imported(0),
                name: None,
                deleted: false,
                shadow: false,
            });
        }
        doc.boolean_ops.push(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![0],
            b: vec![1],
            keep_b: false,
            outputs: vec![2],
            name: None,
            deleted: false,
        });
        let edges = graph_dependency_edges(&doc);
        assert!(edges.contains(&(HierarchyNode::Body(0), HierarchyNode::BooleanOp(0))));
        assert!(edges.contains(&(HierarchyNode::Body(1), HierarchyNode::BooleanOp(0))));
        // The output body is a tree child, not a dependency input.
        assert!(!edges.contains(&(HierarchyNode::Body(2), HierarchyNode::BooleanOp(0))));
    }

    /// #281: each placed drawing view is a "projection" child of its drawing node, labelled by
    /// its source body and orientation.
    #[test]
    fn drawing_views_nest_as_projections_under_the_drawing() {
        let mut doc = Document::default();
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: Some("Plate".to_string()),
            deleted: false,
            shadow: false,
        });
        doc.drawings.push(crate::model::Drawing {
            views: vec![crate::model::DrawingView {
                body: 0,
                sketch: None,
                orientation: crate::model::DrawingOrientation::Front,
                dimensioned_edges: Vec::new(),
                angle_dims: Vec::new(),
                dimension_offsets: Vec::new(),
                dimensioned_circles: Vec::new(),
circle_dim_offsets: Vec::new(),
                aligned_parent: None,
                aligned_dir: None,
                scale: None,
                style: Default::default(),
                align_lines: false,
label_hidden: false,
                label_pos: Default::default(),
                label_text: None,
                pos_x: 0.5,
                pos_y: 0.5,
            }],
            ..Default::default()
        });

        let tree = build_hierarchy(&doc, None);
        // Document -> Drawing(0) -> DrawingProjection { drawing: 0, view: 0 }.
        let drawing = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawing(0))
            .expect("drawing node present");
        assert_eq!(
            drawing.children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![HierarchyNode::DrawingProjection { drawing: 0, view: 0 }]
        );
        assert_eq!(
            node_label(&doc, HierarchyNode::DrawingProjection { drawing: 0, view: 0 }),
            "Plate — Front"
        );
    }

    /// #341: a projection's shown dimensions appear as `DrawingDimension` children nested under it.
    #[test]
    fn drawing_dimensions_nest_under_their_projection() {
        let mut doc = Document::default();
        let a = crate::hierarchy::quantize_body_point(glam::Vec3::ZERO);
        let b = crate::hierarchy::quantize_body_point(glam::Vec3::new(40.0, 0.0, 0.0));
        doc.drawings.push(crate::model::Drawing {
            views: vec![crate::model::DrawingView {
                body: 0,
                sketch: None,
                orientation: crate::model::DrawingOrientation::Front,
                dimensioned_edges: vec![(a, b)],
                angle_dims: Vec::new(),
                dimension_offsets: Vec::new(),
                dimensioned_circles: Vec::new(),
circle_dim_offsets: Vec::new(),
                aligned_parent: None,
                aligned_dir: None,
                scale: None,
                style: Default::default(),
                align_lines: false,
label_hidden: false,
                label_pos: Default::default(),
                label_text: None,
                pos_x: 0.5,
                pos_y: 0.5,
            }],
            ..Default::default()
        });
        let tree = build_hierarchy(&doc, None);
        let drawing = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawing(0))
            .expect("drawing node");
        let projection = drawing
            .children
            .iter()
            .find(|e| matches!(e.node, HierarchyNode::DrawingProjection { .. }))
            .expect("projection node");
        assert_eq!(
            projection.children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![HierarchyNode::DrawingDimension { drawing: 0, view: 0, a, b }],
            "the dimension nests under its projection"
        );
    }

    /// #333: a drawing's text notes appear as `DrawingAnnotation` children under the drawing,
    /// after its projections, labelled by their text.
    #[test]
    fn drawing_annotations_show_as_hierarchy_children() {
        let mut doc = Document::default();
        doc.drawings.push(crate::model::Drawing {
            annotations: vec![crate::model::DrawingAnnotation {
                text: "Scale 1:2".to_string(),
                pos_x: 0.05,
                pos_y: 0.05,
                size_frac: 0.03,
                wrap_frac: None,
                deleted: false,
            }],
            ..Default::default()
        });
        let tree = build_hierarchy(&doc, None);
        let drawing = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawing(0))
            .expect("drawing node present");
        assert!(
            drawing
                .children
                .iter()
                .any(|c| c.node == HierarchyNode::DrawingAnnotation { drawing: 0, annotation: 0 }),
            "the text note is a child of the drawing"
        );
        assert_eq!(
            node_label(&doc, HierarchyNode::DrawingAnnotation { drawing: 0, annotation: 0 }),
            "Text: Scale 1:2"
        );
    }

    /// #275: hiding a category prunes those nodes but promotes their kept children — so hiding
    /// "Operations" while keeping "Bodies" still shows the result body, just un-nested.
    #[test]
    fn filter_hierarchy_promotes_kept_children_of_hidden_nodes() {
        let tree = vec![HierarchyEntry {
            node: HierarchyNode::Document,
            children: vec![HierarchyEntry {
                node: HierarchyNode::BooleanOp(0),
                children: vec![HierarchyEntry {
                    node: HierarchyNode::Body(3),
                    children: Vec::new(),
                }],
            }],
        }];
        let filter = ElementFilter {
            operations: false,
            ..ElementFilter::default()
        };
        let out = filter_hierarchy(&tree, &filter);
        // Document kept; the hidden BooleanOp collapses, promoting Body(3) directly under it.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].node, HierarchyNode::Document);
        assert_eq!(
            out[0].children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![HierarchyNode::Body(3)]
        );

        // Hiding Bodies too removes it entirely.
        let filter = ElementFilter {
            operations: false,
            bodies: false,
            ..ElementFilter::default()
        };
        let out = filter_hierarchy(&tree, &filter);
        assert!(out[0].children.is_empty(), "no kept descendants remain");
    }

    /// #275/#333: the Drawing workbench filter shows the sources (sketches and bodies) plus the
    /// drawings themselves, so the open drawing's projections and text notes appear in the pane.
    #[test]
    fn drawing_workbench_filter_shows_sources_and_drawings() {
        let f = ElementFilter::for_drawing_workbench();
        assert!(f.sketches && f.bodies && f.drawings);
        assert!(!f.planes && !f.operations && !f.sketch_geometry && !f.images);
        assert!(f.shows(HierarchyNode::Body(0)));
        assert!(f.shows(HierarchyNode::Sketch(0)));
        assert!(f.shows(HierarchyNode::Document), "the root is always shown");
        assert!(f.shows(HierarchyNode::DrawingProjection { drawing: 0, view: 0 }));
        assert!(f.shows(HierarchyNode::DrawingAnnotation { drawing: 0, annotation: 0 }));
        assert!(!f.shows(HierarchyNode::ConstructionPlane(0)));
        assert!(!f.shows(HierarchyNode::Extrusion(0)));
    }

    /// #381: the Model workbench default keeps drawing rows but hides their **components**
    /// (projections, notes, dimensions) — page details are noise while modeling. The
    /// "Drawing components" toggle brings them back.
    #[test]
    fn model_workbench_default_hides_drawing_components() {
        let f = ElementFilter::default();
        assert!(f.shows(HierarchyNode::Drawing(0)), "the drawing row itself stays");
        assert!(!f.shows(HierarchyNode::DrawingProjection { drawing: 0, view: 0 }));
        assert!(!f.shows(HierarchyNode::DrawingAnnotation { drawing: 0, annotation: 0 }));
        assert!(!f.shows(HierarchyNode::DrawingDimension {
            drawing: 0,
            view: 0,
            a: [0; 3],
            b: [0; 3]
        }));
        let f = ElementFilter { drawing_components: true, ..ElementFilter::default() };
        assert!(f.shows(HierarchyNode::DrawingProjection { drawing: 0, view: 0 }));
    }

    #[test]
    fn default_document_hierarchy_has_single_document_root() {
        let doc = Document::default();
        let tree = build_hierarchy(&doc, None);
        assert_eq!(tree.len(), 1, "hierarchy should have exactly one root: {tree:?}");
        assert_eq!(tree[0].node, HierarchyNode::Document);
        // The default document's lone construction plane nests under Document rather than
        // sitting as a second root (#87).
        assert_eq!(
            tree[0].children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![HierarchyNode::ConstructionPlane(0)]
        );

        let list = build_element_list(&doc, None);
        assert_eq!(
            list,
            vec![HierarchyNode::Document, HierarchyNode::ConstructionPlane(0)]
        );
    }

    #[test]
    fn root_level_items_nest_under_document_root() {
        use crate::document_lifecycle::tombstone_element;

        let mut doc = Document::default();
        // A second root-level construction plane (#87: root planes nest under Document,
        // not as separate roots).
        doc.construction_planes.push(default_xy_plane());
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        // An orphaned extrusion: its sketch is tombstoned (unreachable), but the extrusion
        // itself is not cascaded away, so it must still surface — as a Document child, not
        // a top-level root.
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.extrusions.push(crate::model::Extrusion {
            sketch,
            faces: Vec::new(),
            distance: 5.0,
            target: None,
            expression: String::new(),
            name: None,
            deleted: false,
            symmetric: false,
            edge_treatments: Vec::new(),
        });
        assert!(tombstone_element(&mut doc, SceneElement::Sketch(sketch)));
        assert!(!sketch_alive(&doc, sketch));

        // An orphaned body (STL import, no source extrusion, #70) also nests under Document.
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles: vec![[glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y]],
            source_name: "part".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        assert_eq!(tree.len(), 1, "hierarchy should have exactly one root: {tree:?}");
        assert_eq!(tree[0].node, HierarchyNode::Document);
        let children: Vec<HierarchyNode> = tree[0].children.iter().map(|c| c.node).collect();
        assert!(children.contains(&HierarchyNode::ConstructionPlane(0)));
        assert!(children.contains(&HierarchyNode::ConstructionPlane(1)));
        assert!(children.contains(&HierarchyNode::Extrusion(0)));
        assert!(children.contains(&HierarchyNode::Body(0)));
    }

    #[test]
    fn imported_mesh_body_surfaces_at_top_level() {
        let mut doc = Document::default();
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles: vec![[glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y]],
            source_name: "part".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: Some("part".to_string()),
            deleted: false,
            shadow: false,
        });
        doc.shape_order.push(ShapeKind::Body);

        let list = build_element_list(&doc, None);
        assert!(
            list.contains(&HierarchyNode::Body(0)),
            "imported body should be visible in the elements list, got {list:?}"
        );
        assert_eq!(parent_element(&doc, SceneElement::Body(0)), None);
    }

    #[test]
    fn construction_plane_ordering_is_deterministic_by_index() {
        let mut doc = Document::default();
        // Independent planes (no input relationship) order by kind+index (#540), which is
        // stable across the randomized HashSet iteration order — never by creation time.
        // shape_order is populated only to prove it no longer influences pane ordering.
        doc.construction_planes.push(default_xy_plane());
        doc.shape_order.push(ShapeKind::ConstructionPlane);
        doc.construction_planes.push(default_xy_plane());
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        let expected = vec![
            HierarchyNode::Document,
            HierarchyNode::ConstructionPlane(0),
            HierarchyNode::ConstructionPlane(1),
            HierarchyNode::ConstructionPlane(2),
        ];
        // Repeat: HashSet iteration order is randomized per run, so a non-deterministic
        // sort would eventually disagree.
        for _ in 0..50 {
            assert_eq!(build_element_list(&doc, None), expected);
        }
    }

    /// #540: the flat list orders purely by the element graph — a consumer follows every input
    /// it's built from, and independent nodes tiebreak by kind+index — never by creation time
    /// (`shape_order` is not consulted here at all).
    #[test]
    fn flat_sort_orders_by_inputs_then_kind_index() {
        let nodes = vec![
            HierarchyNode::BooleanOp(0),
            HierarchyNode::Body(5),
            HierarchyNode::Body(2),
        ];
        let parent_of = HashMap::new();
        let mut input_sources = HashMap::new();
        // The boolean consumes both bodies, so it must come after them regardless of the
        // enum order (a Body sorts before a BooleanOp only because inputs come first here).
        input_sources.insert(
            HierarchyNode::BooleanOp(0),
            vec![HierarchyNode::Body(5), HierarchyNode::Body(2)],
        );
        let out = topological_flat_sort(nodes, parent_of, input_sources);
        assert_eq!(
            out,
            vec![
                HierarchyNode::Body(2), // input, lower index first
                HierarchyNode::Body(5), // input
                HierarchyNode::BooleanOp(0), // consumer, after its inputs
            ]
        );
    }

    #[test]
    fn sketch_row_double_click_opens_for_edit_not_select() {
        assert_eq!(
            sketch_row_action(true, true, false),
            SketchRowAction::Edit
        );
        assert_eq!(
            sketch_row_action(false, true, false),
            SketchRowAction::Select { additive: false }
        );
        assert_eq!(sketch_row_action(false, false, false), SketchRowAction::None);
    }

    #[test]
    fn open_sketch_from_elements_pane_action() {
        use crate::actions::{Action, AppState, SketchSession};

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(0));
        assert!(state.sketch_session.is_none());
        assert_eq!(
            state.apply(Action::OpenSketch {
                sketch,
                viewport: None,
            }),
            crate::actions::ActionResult::Ok
        );
        assert_eq!(state.sketch_session, Some(SketchSession { sketch }));
    }

    fn doc_with_plane_sketches() -> Document {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        let s1 = doc.add_sketch(FaceId::ConstructionPlane(0));
        crate::construction::add_line_rectangle(&mut doc, s0, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.lines
            .push(Line::from_local_endpoints(s1, 0.0, 0.0, 5.0, 0.0));
        doc
    }

    #[test]
    fn main_view_lists_planes_and_sketches_only() {
        let doc = doc_with_plane_sketches();
        let list = build_element_list(&doc, None);
        assert_eq!(list.len(), 4);
        assert_eq!(list[0], HierarchyNode::Document);
        assert_eq!(list[1], HierarchyNode::ConstructionPlane(0));
        assert_eq!(list[2], HierarchyNode::Sketch(0));
        assert_eq!(list[3], HierarchyNode::Sketch(1));
    }

    #[test]
    fn sketch_view_lists_constraints_for_active_sketch() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        crate::constraints::add_distance_constraint(
            &mut doc,
            sketch,
            crate::model::DistanceTarget::LineLength(0),
            "5mm".to_string(),
        )
        .unwrap();
        let list = build_element_list(&doc, Some(SketchSession { sketch }));
        assert!(list.contains(&HierarchyNode::Constraint(0)));
        assert!(!build_element_list(&doc, None).contains(&HierarchyNode::Constraint(0)));
    }

    #[test]
    fn nested_sketches_on_circle_face_follow_parent_order() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles
            .push(crate::model::Circle::from_local_center_radius(s0, 0.0, 0.0, 20.0, 0.0));
        let s1 = doc.add_sketch(FaceId::Circle(0));

        let list = build_element_list(&doc, None);
        assert_eq!(
            list,
            vec![
                HierarchyNode::Document,
                HierarchyNode::ConstructionPlane(0),
                HierarchyNode::Sketch(0),
                HierarchyNode::Circle(0),
                HierarchyNode::Sketch(1),
            ]
        );
        let _ = s1;
    }

    #[test]
    fn plane_from_sketch_geometry_lists_under_sketch() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let derived = plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Face {
                    origin: glam::Vec3::ZERO,
                    normal: glam::Vec3::Z,
                    label: "Ground".to_string(),
                },
                5.0,
                0.0,
            ),
            ConstructionPlaneParent::Sketch(sketch),
        );
        doc.construction_planes.push(derived);
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        let list = build_element_list(&doc, None);
        assert_eq!(
            list,
            vec![
                HierarchyNode::Document,
                HierarchyNode::ConstructionPlane(0),
                HierarchyNode::Sketch(0),
                HierarchyNode::ConstructionPlane(1),
            ]
        );
    }

    /// Recursively finds `node`'s entry anywhere in the tree (entries aren't just roots — e.g.
    /// a sketch nests under its construction-plane root).
    fn find_entry(entries: &[HierarchyEntry], node: HierarchyNode) -> Option<&HierarchyEntry> {
        for entry in entries {
            if entry.node == node {
                return Some(entry);
            }
            if let Some(found) = find_entry(&entry.children, node) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn chamfer_fillet_bridge_line_nests_under_lower_index_trimmed_line() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        let mut bridge = Line::from_local_endpoints(sketch, 7.0, 0.0, 10.0, 3.0);
        bridge.chamfer_fillet_parent = Some(0);
        doc.lines.push(bridge);
        doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line, ShapeKind::Line]);

        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let sketch_entry = find_entry(&tree, HierarchyNode::Sketch(sketch)).expect("sketch entry");
        // The bridge (line 2) is *not* a top-level sibling of the sketch's lines...
        assert!(!sketch_entry
            .children
            .iter()
            .any(|c| c.node == HierarchyNode::Line(2)));
        // ...it nests under line 0 (the lower-index trimmed line, #76).
        let line0_entry = sketch_entry
            .children
            .iter()
            .find(|c| c.node == HierarchyNode::Line(0))
            .expect("line 0 entry");
        assert_eq!(line0_entry.children, vec![HierarchyEntry {
            node: HierarchyNode::Line(2),
            children: vec![],
        }]);

        // The flat list keeps line 0 before its nested bridge, and still includes line 1.
        let list = build_element_list(&doc, Some(SketchSession { sketch }));
        let l0 = list.iter().position(|n| *n == HierarchyNode::Line(0)).unwrap();
        let l1 = list.iter().position(|n| *n == HierarchyNode::Line(1));
        let l2 = list.iter().position(|n| *n == HierarchyNode::Line(2)).unwrap();
        assert!(l0 < l2, "parent line must come before the nested bridge");
        assert!(l1.is_some(), "the other trimmed line must still be listed");
    }

    #[test]
    fn chamfer_fillet_bridge_line_falls_back_to_top_level_when_parent_is_gone() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut bridge = Line::from_local_endpoints(sketch, 7.0, 0.0, 10.0, 3.0);
        // Points at a parent index that doesn't exist (e.g. the parent line was later removed
        // by undo) — must degrade gracefully to a top-level row, not panic or vanish.
        bridge.chamfer_fillet_parent = Some(99);
        doc.lines.push(bridge);
        doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);

        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let sketch_entry = find_entry(&tree, HierarchyNode::Sketch(sketch)).expect("sketch entry");
        assert!(sketch_entry
            .children
            .iter()
            .any(|c| c.node == HierarchyNode::Line(1)));

        // Also degrades gracefully when the recorded parent line exists but is tombstoned.
        let mut doc2 = Document::default();
        let sketch2 = doc2.add_sketch(FaceId::ConstructionPlane(0));
        doc2.lines
            .push(Line::from_local_endpoints(sketch2, 0.0, 0.0, 10.0, 0.0));
        doc2.lines[0].deleted = true;
        let mut bridge2 = Line::from_local_endpoints(sketch2, 7.0, 0.0, 10.0, 3.0);
        bridge2.chamfer_fillet_parent = Some(0);
        doc2.lines.push(bridge2);
        doc2.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        let tree2 = build_hierarchy(&doc2, Some(SketchSession { sketch: sketch2 }));
        let sketch_entry2 =
            find_entry(&tree2, HierarchyNode::Sketch(sketch2)).expect("sketch entry");
        assert!(sketch_entry2
            .children
            .iter()
            .any(|c| c.node == HierarchyNode::Line(1)));
    }

    #[test]
    fn row_style_faints_unrelated_rows_when_selection_active() {
        let mut doc = Document::default();
        let _s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.add_sketch(FaceId::ConstructionPlane(0));
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Sketch(0),
            false,
        );
        let context = selection_context_elements(&doc, &selection);
        let related_constraints = selection_related_constraints(&doc, &selection);
        let list = build_element_list(&doc, None);
        let style_selection = selection_styles_visible_list(&list, &selection);
        assert!(style_selection);
        let health = DocumentHealth::default();
        assert_eq!(
            row_style(
                SceneElement::Sketch(0),
                &selection,
                &context,
                &related_constraints,
                style_selection,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::Selected
        );
        assert_eq!(
            row_style(
                SceneElement::ConstructionPlane(0),
                &selection,
                &context,
                &related_constraints,
                style_selection,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::InContext
        );
        assert_eq!(
            row_style(
                SceneElement::Sketch(1),
                &selection,
                &context,
                &related_constraints,
                style_selection,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::Faint
        );
    }

    #[test]
    fn selection_context_includes_constraints_for_selected_line() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        crate::constraints::add_distance_constraint(
            &mut doc,
            sketch,
            DistanceTarget::LineLength(0),
            "5mm".to_string(),
        )
        .unwrap();

        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Line(0),
            false,
        );
        let context = selection_context_elements(&doc, &selection);
        let related = selection_related_constraints(&doc, &selection);
        assert!(context.contains(&SceneElement::Constraint(0)));
        assert!(related.contains(&0));
    }

    #[test]
    fn row_style_highlights_related_constraint_when_line_selected() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        crate::constraints::add_distance_constraint(
            &mut doc,
            sketch,
            DistanceTarget::LineLength(0),
            "5mm".to_string(),
        )
        .unwrap();

        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Line(0),
            false,
        );
        let context = selection_context_elements(&doc, &selection);
        let related = selection_related_constraints(&doc, &selection);
        let list = build_element_list(&doc, Some(SketchSession { sketch }));
        let style_selection = selection_styles_visible_list(&list, &selection);
        let health = DocumentHealth::default();
        assert_eq!(
            row_style(
                SceneElement::Constraint(0),
                &selection,
                &context,
                &related,
                style_selection,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::RelatedConstraint
        );
        assert_eq!(
            row_style(
                SceneElement::Line(1),
                &selection,
                &context,
                &related,
                style_selection,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::Faint
        );
    }

    #[test]
    fn row_style_prefers_invalid_and_unstable_over_selection() {
        use crate::document_lifecycle::tombstone_element;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine, Line, ShapeKind};

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        let line_a = 0;
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        let line_b = 1;
        doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(line_a),
                line_b: ConstraintLine::Line(line_b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        tombstone_element(&mut doc, SceneElement::Line(line_a));
        let health = crate::document_health::recompute_document_health(&doc);
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Line(line_b),
            false,
        );
        let context = selection_context_elements(&doc, &selection);
        let related = selection_related_constraints(&doc, &selection);
        assert_eq!(
            row_style(
                SceneElement::Constraint(0),
                &selection,
                &context,
                &related,
                true,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::Invalid
        );
        assert_eq!(
            row_style(
                SceneElement::Line(line_b),
                &selection,
                &context,
                &related,
                true,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::Unstable
        );
    }

    /// #511: an invalid/unstable row still paints as selected when picked in the pane.
    #[test]
    fn invalid_and_unstable_rows_still_show_selection_highlight() {
        use crate::document_lifecycle::tombstone_element;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine, Line, ShapeKind};

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
        tombstone_element(&mut doc, SceneElement::Line(0));
        let health = crate::document_health::recompute_document_health(&doc);

        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Constraint(0),
            false,
        );
        assert!(row_shows_selection(
            &SceneElement::Constraint(0),
            &selection,
            true
        ));
        assert_eq!(
            row_style(
                SceneElement::Constraint(0),
                &selection,
                &HashSet::new(),
                &HashSet::new(),
                true,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::Invalid,
            "health tint stays invalid while selected"
        );

        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(&mut selection, SceneElement::Line(1), false);
        assert!(row_shows_selection(&SceneElement::Line(1), &selection, true));
        assert_eq!(
            row_style(
                SceneElement::Line(1),
                &selection,
                &HashSet::new(),
                &HashSet::new(),
                true,
                &health,
                &HashSet::new(),
                &HashSet::new(),
            ),
            RowStyle::Unstable
        );
    }

    #[test]
    fn hiding_sketch_hides_derived_construction_plane() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.construction_planes.push(plane_from_definition(
            &default_xy_plane().definition,
            ConstructionPlaneParent::Sketch(sketch),
        ));

        let mut vis = ElementVisibility::default();
        vis.set_visible(SceneElement::Sketch(sketch), false);
        assert!(!vis.effective_visible(&doc, SceneElement::ConstructionPlane(1)));
    }

    #[test]
    fn toggle_visibility_flips_state() {
        let mut vis = ElementVisibility::default();
        assert!(vis.is_visible(SceneElement::Sketch(0)));
        assert!(!vis.toggle(SceneElement::Sketch(0)));
        assert!(!vis.is_visible(SceneElement::Sketch(0)));
    }

    #[test]
    fn pane_title_is_elements() {
        assert_eq!(PANE_TITLE, "Elements");
    }

    #[test]
    fn hierarchy_view_mode_defaults_to_list() {
        assert_eq!(HierarchyViewMode::default(), HierarchyViewMode::List);
    }

    /// Drive the force layout to rest and return the final state, using the same fixture as the
    /// static-layout tests (plane → sketch → rect + extrusion → body).
    /// #524/#531/#545: "rollback to here" suppresses the marker's **graph descendants** — what
    /// nests under it and what depends on it — not everything created after it in time; and not
    /// the marker itself, unless the rollback is **inclusive** ("just before here").
    #[test]
    fn rollback_suppresses_graph_descendants() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let here = |el: SceneElement| RollbackMarker { element: el, inclusive: false };
        let before = |el: SceneElement| RollbackMarker { element: el, inclusive: true };
        // Rolling back to the sketch hides everything built from it: its rect lines, the
        // extrusion, and the body — but not the sketch itself or its host plane.
        let rb = rolled_back_elements(&doc, &here(SceneElement::Sketch(sketch)));
        assert!(rb.contains(&SceneElement::Extrusion(0)), "extrusion depends on the sketch");
        assert!(rb.contains(&SceneElement::Body(0)), "body depends on the extrusion");
        assert!(!rb.contains(&SceneElement::Sketch(sketch)), "the marker itself stays");
        assert!(!rb.contains(&SceneElement::ConstructionPlane(0)), "ancestors stay active");

        // "Just before here" additionally hides the marker element itself.
        let rb_before = rolled_back_elements(&doc, &before(SceneElement::Sketch(sketch)));
        assert!(rb_before.contains(&SceneElement::Sketch(sketch)), "inclusive hides the marker");
        assert!(rb_before.contains(&SceneElement::Extrusion(0)), "and its descendants");

        // Rolling back to the body (a leaf nothing consumes) suppresses nothing — unless
        // inclusive, which hides just the body.
        assert!(rolled_back_elements(&doc, &here(SceneElement::Body(0))).is_empty());
        let body_before = rolled_back_elements(&doc, &before(SceneElement::Body(0)));
        assert_eq!(body_before.len(), 1);
        assert!(body_before.contains(&SceneElement::Body(0)));
        // An unknown / non-graph marker suppresses nothing.
        assert!(rolled_back_elements(&doc, &here(SceneElement::Sketch(99))).is_empty());
        assert!(rolled_back_elements(&doc, &here(SceneElement::Origin)).is_empty());
    }

    fn doc_with_plane_sketch_rect_and_extrusion() -> (Document, SketchId) {
        use crate::model::{Body, BodySource, ExtrudeFace, Extrusion};

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let rect_lines =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.extrusions.push(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            name: None,
            deleted: false,
            symmetric: false,
            edge_treatments: Vec::new(),
        });
        doc.bodies.push(Body {
            source: BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        (doc, sketch)
    }

    fn settle_graph_layout(width: f32, steps: u32) -> (GraphLayout, Vec<GraphNodePosition>) {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let positions = graph_node_positions(&tree);
        let mut layout = GraphLayout::default();
        // First call seeds all nodes; subsequent calls just step.
        for _ in 0..steps {
            layout.sync_and_step(&positions, width, 1, 0.16, true);
        }
        (layout, positions)
    }

    #[test]
    fn force_layout_settles_stays_contained_and_flows_top_to_bottom() {
        let width = 300.0;
        let (layout, positions) = settle_graph_layout(width, 4000);

        // Kinetic energy has decayed toward zero — the sim settled rather than oscillating.
        let depth_of: HashMap<HierarchyNode, usize> =
            positions.iter().map(|p| (p.node, p.depth)).collect();
        let edges: Vec<(HierarchyNode, HierarchyNode)> = positions
            .iter()
            .filter_map(|p| p.parent.map(|parent| (p.node, parent)))
            .collect();
        let mut nodes = layout.nodes.clone();
        let ke = step_graph_layout(&mut nodes, &edges, &depth_of, width, 0.16);
        assert!(ke < 1e-2, "layout should settle, residual KE = {ke}");

        for p in &positions {
            let pos = layout.pos_of(p.node).expect("node has a settled position");
            assert!(pos.x.is_finite() && pos.y.is_finite(), "finite pos for {:?}: {pos:?}", p.node);
            assert!(
                (0.0..=width).contains(&pos.x),
                "x contained to pane for {:?}: {}",
                p.node,
                pos.x
            );
        }

        // Vertical-layering invariant: every parent settles strictly above (smaller y than)
        // each of its children.
        for p in &positions {
            let Some(parent) = p.parent else { continue };
            let child_y = layout.pos_of(p.node).unwrap().y;
            let parent_y = layout.pos_of(parent).unwrap().y;
            assert!(
                parent_y < child_y,
                "parent {parent:?} (y={parent_y}) must sit above child {:?} (y={child_y})",
                p.node
            );
        }
    }

    /// #151: repulsion must actually separate the dots — with a sketch's dozen-plus children
    /// sharing one depth band, no two settled nodes may overlap (dot diameter is 18 px).
    #[test]
    fn force_layout_keeps_nodes_from_overlapping() {
        let width = 420.0;
        let (layout, positions) = settle_graph_layout(width, 4000);
        for (i, a) in positions.iter().enumerate() {
            for b in positions.iter().skip(i + 1) {
                let pa = layout.pos_of(a.node).unwrap();
                let pb = layout.pos_of(b.node).unwrap();
                let dist = (pa - pb).length();
                assert!(
                    dist >= 19.0,
                    "nodes {:?} and {:?} overlap: {dist:.1} px apart",
                    a.node,
                    b.node
                );
            }
        }
    }

    /// #248: with real (wide) labels, no two nodes' label boxes may overlap after settling —
    /// so drawn words never land on top of one another.
    #[test]
    fn declutter_spreads_bands_so_no_two_labels_overlap() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let positions = graph_node_positions(&tree);

        // Settle the physics, then feed its x's plus chunky uniform labels through the declutter.
        let mut layout = GraphLayout::default();
        for _ in 0..2000 {
            layout.sync_and_step(&positions, 300.0, 1, 0.16, true);
        }
        let sim_x: HashMap<HierarchyNode, f32> = positions
            .iter()
            .map(|p| (p.node, layout.pos_of(p.node).unwrap().x))
            .collect();
        let label_widths: HashMap<HierarchyNode, f32> =
            positions.iter().map(|p| (p.node, 60.0)).collect();
        // A very wide pane so nothing wraps — same-band nodes must be cleared horizontally.
        let display = declutter_label_bands(&positions, &sim_x, &label_widths, 100_000.0);

        // Every node keeps a placement, and no two label boxes (dot + rightward text) overlap
        // when they're in the same band *and* the same sub-row.
        const R: f32 = 9.0;
        const GAP: f32 = 4.0;
        for (i, a) in positions.iter().enumerate() {
            assert!(display.contains_key(&a.node), "no placement for {:?}", a.node);
            for b in positions.iter().skip(i + 1) {
                if a.depth != b.depth {
                    continue;
                }
                let (xa, ra) = display[&a.node];
                let (xb, rb) = display[&b.node];
                if ra != rb {
                    continue;
                }
                let overlap = (xa + R + GAP + label_widths[&a.node])
                    .min(xb + R + GAP + label_widths[&b.node])
                    - (xa - R).max(xb - R);
                assert!(
                    overlap <= 0.0,
                    "labels of {:?} and {:?} overlap by {overlap:.1}px",
                    a.node,
                    b.node
                );
            }
        }
    }

    /// #350: a band too wide for the pane wraps into stacked sub-rows (grows taller) instead of
    /// overflowing the width.
    #[test]
    fn declutter_wraps_wide_bands_into_rows() {
        let nodes: Vec<HierarchyNode> = (0..8).map(HierarchyNode::Constraint).collect();
        let positions: Vec<GraphNodePosition> = nodes
            .iter()
            .enumerate()
            .map(|(i, &node)| GraphNodePosition { node, parent: None, depth: 1, column: 1, row: i })
            .collect();
        let sim_x: HashMap<HierarchyNode, f32> =
            nodes.iter().enumerate().map(|(i, &n)| (n, i as f32 * 20.0)).collect();
        let label_widths: HashMap<HierarchyNode, f32> = nodes.iter().map(|&n| (n, 60.0)).collect();
        // A narrow pane: 8 nodes × ~88px each can't fit ~200px, so they wrap onto several rows.
        let display = declutter_label_bands(&positions, &sim_x, &label_widths, 200.0);
        let max_row = nodes.iter().map(|n| display[n].1).max().unwrap();
        assert!(max_row >= 1, "a wide band should wrap onto more than one row");
        // Every placement stays within the pane width.
        for n in &nodes {
            assert!(display[n].0 >= GRAPH_MARGIN - 0.1 && display[n].0 <= 200.0, "x within pane");
        }
    }

    /// Declutter preserves each band's left-to-right order and leaves an already-spread band
    /// untouched (it only pushes nodes apart, never together).
    #[test]
    fn declutter_preserves_order_and_leaves_spread_bands_be() {
        let nodes = [
            HierarchyNode::Constraint(0),
            HierarchyNode::Constraint(1),
            HierarchyNode::Constraint(2),
        ];
        let positions: Vec<GraphNodePosition> = nodes
            .iter()
            .enumerate()
            .map(|(i, &node)| GraphNodePosition {
                node,
                parent: None,
                depth: 1,
                column: 1,
                row: i,
            })
            .collect();
        // Already 200 px apart — far beyond any label — so declutter must not move them (a wide
        // pane so nothing wraps; positions kept ≥ the left margin so no clamp bites).
        let sim_x: HashMap<HierarchyNode, f32> = nodes
            .iter()
            .enumerate()
            .map(|(i, &n)| (n, GRAPH_MARGIN + i as f32 * 200.0))
            .collect();
        let label_widths: HashMap<HierarchyNode, f32> = nodes.iter().map(|&n| (n, 40.0)).collect();
        let out = declutter_label_bands(&positions, &sim_x, &label_widths, 100_000.0);
        for &n in &nodes {
            assert_eq!(out[&n].1, 0, "spread band stays on one row for {n:?}");
            assert!((out[&n].0 - sim_x[&n]).abs() < 1e-3, "spread band moved for {n:?}");
        }
    }

    #[test]
    #[ignore]
    fn force_layout_probe() {
        let (layout, _positions) = settle_graph_layout(300.0, 4000);
        let mut states: Vec<_> = layout.nodes.iter().map(|(n, s)| (*n, *s)).collect();
        states.sort_by(|a, b| b.1.vel.length_sq().partial_cmp(&a.1.vel.length_sq()).unwrap());
        for (n, s) in states.iter().take(10) {
            println!("{:?} pos=({:.1},{:.1}) vel=({:.2},{:.2})", n, s.pos.x, s.pos.y, s.vel.x, s.vel.y);
        }
    }

    #[test]
    fn force_layout_is_deterministic() {
        let width = 320.0;
        let (a, positions) = settle_graph_layout(width, 1500);
        let (b, _) = settle_graph_layout(width, 1500);
        for p in &positions {
            let pa = a.pos_of(p.node).unwrap();
            let pb = b.pos_of(p.node).unwrap();
            assert!(
                (pa.x - pb.x).abs() < 1e-4 && (pa.y - pb.y).abs() < 1e-4,
                "same seed must give same settled position for {:?}: {pa:?} vs {pb:?}",
                p.node
            );
        }
    }

    /// #525: with the force layout off, `sync_and_step` still seeds new nodes but does not
    /// move them — positions are frozen and it reports zero kinetic energy.
    #[test]
    fn force_layout_off_freezes_positions() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let positions = graph_node_positions(&tree);
        let mut layout = GraphLayout::default();

        // First pass seeds every node even with physics off.
        let ke = layout.sync_and_step(&positions, 300.0, 6, 0.16, false);
        assert_eq!(ke, 0.0, "no physics runs, so no kinetic energy");
        assert_eq!(layout.nodes.len(), positions.len(), "all nodes are seeded");
        let before: Vec<egui::Vec2> =
            positions.iter().map(|p| layout.pos_of(p.node).unwrap()).collect();

        // Further frozen steps never move a node.
        for _ in 0..20 {
            layout.sync_and_step(&positions, 300.0, 6, 0.16, false);
        }
        for (p, &b) in positions.iter().zip(before.iter()) {
            let now = layout.pos_of(p.node).unwrap();
            assert!(
                (now - b).length() < 1e-6,
                "frozen node {:?} moved: {b:?} -> {now:?}",
                p.node
            );
        }
    }

    #[test]
    fn force_layout_syncs_added_and_removed_nodes() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let positions = graph_node_positions(&tree);
        let mut layout = GraphLayout::default();
        layout.sync_and_step(&positions, 300.0, 1, 0.16, true);
        assert_eq!(layout.nodes.len(), positions.len());

        // A smaller node set (just the Document root) drops the departed nodes.
        let root_only = vec![GraphNodePosition {
            node: HierarchyNode::Document,
            parent: None,
            depth: 0,
            column: 0,
            row: 0,
        }];
        layout.sync_and_step(&root_only, 300.0, 1, 0.16, true);
        assert_eq!(layout.nodes.len(), 1);
        assert!(layout.pos_of(HierarchyNode::Document).is_some());
    }

    /// #252: a loft appears as an operation node with its output body nested beneath it, and its
    /// cross-section sketches feed it as graph dependency edges — the user's canonical example.
    #[test]
    fn loft_is_an_operation_with_body_output_and_sketch_inputs() {
        use crate::model::{Body, BodySource, ExtrudeFace, Loft, LoftSection};
        let mut doc = Document::default();
        doc.lofts.push(Loft {
            sections: vec![
                LoftSection { sketch: 0, face: ExtrudeFace::Circle(0) },
                LoftSection { sketch: 1, face: ExtrudeFace::Circle(1) },
                LoftSection { sketch: 2, face: ExtrudeFace::Circle(2) },
            ],
            mode: crate::model::LoftMode::NewBody,
            name: None,
            deleted: false,
        });
        doc.bodies.push(Body {
            source: BodySource::Loft(0),
            name: None,
            deleted: false,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        let loft = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Loft(0))
            .expect("loft is a top-level operation, not a bare body");
        assert!(
            loft.children.iter().any(|c| c.node == HierarchyNode::Body(0)),
            "the loft body nests under the loft as its output"
        );
        // The three section sketches feed the loft as dependency inputs.
        let deps = graph_dependency_edges(&doc);
        for si in 0..3 {
            assert!(
                deps.contains(&(HierarchyNode::Sketch(si), HierarchyNode::Loft(0))),
                "sketch {si} feeds the loft"
            );
        }
    }

    /// #sweep: the op node depends on its profile sketch and every path line, and
    /// its NewBody output body nests beneath it.
    #[test]
    fn sweep_appears_in_the_tree_and_feeds_from_its_inputs() {
        use crate::model::{Body, BodySource, SweepMode, Sweep, Line};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        doc.sweeps.push(Sweep {
            sketch,
            faces: Vec::new(),
            path: vec![0, 1],
            mode: SweepMode::NewBody,
            name: None,
            deleted: false,
        });
        doc.bodies.push(Body {
            source: BodySource::Sweep(0),
            name: None,
            deleted: false,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        // The op nests under its profile sketch (#478), with the output body beneath it.
        fn find_op(entries: &[HierarchyEntry]) -> Option<&HierarchyEntry> {
            for e in entries {
                if e.node == HierarchyNode::SweepOp(0) {
                    return Some(e);
                }
                if let Some(found) = find_op(&e.children) {
                    return Some(found);
                }
            }
            None
        }
        let op = find_op(&tree).expect("the sweep op appears in the tree");
        let sketch_entry = {
            fn find_sketch(entries: &[HierarchyEntry], sketch: SketchId) -> Option<&HierarchyEntry> {
                for e in entries {
                    if e.node == HierarchyNode::Sketch(sketch) {
                        return Some(e);
                    }
                    if let Some(found) = find_sketch(&e.children, sketch) {
                        return Some(found);
                    }
                }
                None
            }
            find_sketch(&tree, sketch).expect("profile sketch in the tree")
        };
        assert!(
            sketch_entry.children.iter().any(|c| c.node == HierarchyNode::SweepOp(0)),
            "the sweep op nests under its profile sketch"
        );
        assert!(
            op.children.iter().any(|c| c.node == HierarchyNode::Body(0)),
            "the swept body nests under the sweep op as its output"
        );
        let deps = graph_dependency_edges(&doc);
        assert!(deps.contains(&(HierarchyNode::Sketch(sketch), HierarchyNode::SweepOp(0))));
        assert!(deps.contains(&(HierarchyNode::Line(0), HierarchyNode::SweepOp(0))));
        assert!(deps.contains(&(HierarchyNode::Line(1), HierarchyNode::SweepOp(0))));
    }

    #[test]
    fn revolution_appears_in_the_tree_with_its_body(){
        use crate::model::{Body, BodySource, Revolution, RevolveAxis, RevolveMode};
        let mut doc = Document::default();
        doc.revolutions.push(Revolution {
            sketch: 0,
            faces: Vec::new(),
            axis: RevolveAxis::X,
            angle_deg: 360.0,
            symmetric: false,
            mode: RevolveMode::NewBody,
            name: None,
            deleted: false,
        });
        doc.bodies.push(Body {
            source: BodySource::Revolve(0),
            name: None,
            deleted: false,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        let root = &tree[0];
        let rev = root
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Revolution(0))
            .expect("the revolution is a top-level element (#211)");
        assert!(
            rev.children.iter().any(|c| c.node == HierarchyNode::Body(0)),
            "the revolved body nests under the revolution",
        );
        // The body's *only* parent is the revolution (#305): it must not also surface as a
        // top-level orphan under Document.
        assert!(
            !root.children.iter().any(|e| e.node == HierarchyNode::Body(0)),
            "a revolved body is not a Document-level orphan",
        );
        // It maps to a selectable scene element.
        assert_eq!(
            scene_element_for_node(HierarchyNode::Revolution(0)),
            Some(SceneElement::Revolution(0))
        );
    }
}

