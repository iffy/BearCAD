//! Elements pane: construction planes, sketches, and sketch geometry.

/// Side-panel title shown in the UI.
pub const PANE_TITLE: &str = "Elements";

/// Egui-memory key for the first sketch row drawn in Elements this frame (#1279).
fn elements_sketch_row_rect_id() -> egui::Id {
    egui::Id::new("elements_sketch_row_rect")
}

/// Where the Elements pane drew a sketch row this frame (#1279) — tutorial orb target.
pub fn elements_sketch_row_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<egui::Rect>(elements_sketch_row_rect_id()))
}

/// Egui-memory key for the first body row drawn in Elements this frame (#1647).
fn elements_body_row_rect_id() -> egui::Id {
    egui::Id::new("elements_body_row_rect")
}

/// Egui-memory key for the **newest** construction-plane row drawn in Elements this frame
/// (#1673). The newest one is the plane the user just made — the datum planes come first.
fn elements_plane_row_rect_id() -> egui::Id {
    egui::Id::new("elements_plane_row_rect")
}

/// Where the Elements pane drew the newest construction-plane row this frame (#1673) —
/// where the angled-plane walkthrough's orb points to reopen it.
pub fn elements_plane_row_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<egui::Rect>(elements_plane_row_rect_id()))
}

/// Where the Elements pane drew a body row this frame (#1647) — the Add-view tool takes its
/// click there, so the drawing walkthrough's orb points at it.
pub fn elements_body_row_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<egui::Rect>(elements_body_row_rect_id()))
}

/// Egui-memory key for the Graph view's per-row rects this frame (#1670).
fn elements_graph_row_rects_id() -> egui::Id {
    egui::Id::new("elements_graph_row_rects")
}

/// Where the Graph view drew each of its rows this frame (#1670), in screen points, for the
/// rows that were actually on screen. Scripts read these back through
/// `bearcad.ui.elements_graph()` so a test can click a row where it really is. Empty
/// whenever the Elements pane is showing some other view.
pub fn elements_graph_row_rects(ctx: &egui::Context) -> Vec<(HierarchyNode, egui::Rect)> {
    ctx.data(|d| d.get_temp::<Vec<(HierarchyNode, egui::Rect)>>(elements_graph_row_rects_id()))
        .unwrap_or_default()
}

fn elements_list_row_rects_id() -> egui::Id {
    egui::Id::new("elements_list_row_rects")
}

/// Where the List view drew each of its rows this frame (#1712), by label, in screen points.
/// Scripts read these through `bearcad.ui.elements_row_rect(label)` so a test can click a row
/// where it really is. Empty whenever the pane is showing the Graph.
pub fn elements_list_row_rect(ctx: &egui::Context, label: &str) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<Vec<(String, egui::Rect)>>(elements_list_row_rects_id()))
        .and_then(|rows| rows.into_iter().find(|(name, _)| name == label).map(|(_, r)| r))
}

fn set_elements_list_row_rects(ctx: &egui::Context, rows: Vec<(String, egui::Rect)>) {
    ctx.data_mut(|d| d.insert_temp(elements_list_row_rects_id(), rows));
}

fn set_elements_graph_row_rects(ctx: &egui::Context, rects: Vec<(HierarchyNode, egui::Rect)>) {
    ctx.data_mut(|d| d.insert_temp(elements_graph_row_rects_id(), rects));
}

use crate::actions::SketchSession;
use crate::icons::{
    icon_button, icon_for_constraint_kind, icon_for_visibility, selectable_icon_button,
    sized_texture, IconId, ICON_DISPLAY_SIZE,
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
use std::collections::{HashMap, HashSet};

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
    ConstructionPlane(crate::model::ConstructionPlaneKey),
    Sketch(SketchId),
    Line(crate::model::LineKey),
    Circle(crate::model::CircleKey),
    Constraint(crate::model::ConstraintKey),
    Extrusion(crate::model::ExtrusionKey),
    Body(crate::model::BodyKey),
    /// A tracing image (#163/#169).
    Image(crate::model::TracingImageKey),
    /// A boolean operation between bodies (Combine tool); its output bodies nest under it.
    BooleanOp(crate::model::BooleanOpKey),
    /// A move operation on bodies (Move tool); its output bodies nest under it.
    MoveOp(crate::model::MoveOpKey),
    /// A mirror operation on bodies (Mirror tool, #523); its reflected bodies nest under it.
    MirrorOp(crate::model::MirrorOpKey),
    /// A linear repeat on bodies (Repeat tool); its output bodies nest under it.
    RepeatOp(crate::model::RepeatOpKey),
    /// A 2D in-sketch linear repeat (#222/#228); its duplicated lines/circles nest under it.
    SketchRepeatOp(crate::model::SketchRepeatOpKey),
    SketchOffsetOp(crate::model::SketchOffsetOpKey),
    /// A 2D in-sketch mirror (#523/#1540); nests under its sketch, with reflected
    /// lines/circles under it.
    SketchMirrorOp(crate::model::SketchMirrorOpKey),
    /// A 2D in-sketch chamfer/fillet (#538); its trimmed copies + bridge lines nest under it.
    SketchVertexTreatmentOp(crate::model::SketchVertexTreatmentOpKey),
    /// A 2D in-sketch slice (#224/#229); its fragment lines nest under it.
    SketchSliceOp(crate::model::SketchSliceOpKey),
    /// A sketch text element (#282/#286); nests under its sketch like a line.
    SketchText(crate::model::SketchTextKey),
    /// A slice operation on bodies (Slice tool); its fragment bodies nest under it.
    SliceOp(crate::model::SliceOpKey),
    /// A shell operation on bodies (Shell tool, #1156); its hollowed output bodies nest under it.
    ShellOp(crate::model::ShellOpKey),
    /// An edge chamfer/fillet operation on bodies (#531); its beveled output bodies nest under
    /// it and its input bodies + treated edges feed it as graph inputs.
    EdgeTreatmentOp(crate::model::EdgeTreatmentOpKey),
    /// A revolved solid (Revolve tool); its output body nests under it (#211).
    Revolution(crate::model::RevolutionKey),
    /// A primitive shape (Create Shape tool, #909); its body nests under it.
    Shape(crate::model::PrimitiveKey),
    /// A sweep (Sweep tool); its output body nests under it.
    SweepOp(crate::model::SweepKey),
    /// A loft (Loft tool): its output body nests under it, and its cross-section sketches feed
    /// it as graph inputs (#252). Selectable via [`SceneElement::Loft`] (#1487).
    Loft(crate::model::LoftKey),
    /// Synthetic section under Document that holds every unassigned technical drawing
    /// (#1205). Present only when the document has at least one drawing that isn't filed
    /// into a component; collapsible in the List view so drawings sit together at the
    /// bottom instead of interspersing with bodies. Display-only (no [`SceneElement`]).
    Drawings,
    /// The collapsible **Views** section (#1671): where cross-section views live, the way
    /// drawings live under [`HierarchyNode::Drawings`].
    Views,
    /// A cross-section view (#1671).
    CrossSection(crate::model::CrossSectionKey),
    /// One cutting plane of a cross-section view: its own element, nested under the view
    /// in the Views section.
    SectionPlane {
        view: crate::model::CrossSectionKey,
        cut: usize,
    },
    /// A technical drawing (#180). A display-only leaf (no [`SceneElement`], like
    /// [`HierarchyNode::Document`]): it has its own icon and is right-clickable to edit
    /// (opening the drawing pane), but isn't a selectable/hideable scene element. Lives
    /// under [`HierarchyNode::Drawings`] (or a component, if filed there).
    Drawing(crate::model::DrawingKey),
    /// A 3D edge chamfer/fillet applied to an extrusion (#77); `index` is into that
    /// extrusion's `edge_treatments`. A display-only leaf (like [`HierarchyNode::Document`]
    /// it has no [`SceneElement`]): it nests under its extrusion and is right-clickable to
    /// edit its amount after the fact (#192), but isn't individually selectable/hideable.
    EdgeTreatment { extrusion: crate::model::ExtrusionKey, index: usize },
    /// A body/sketch **projection** placed on a technical drawing (#281): a display-only leaf
    /// nested under its [`HierarchyNode::Drawing`]. `view` indexes the drawing's `views`. It has
    /// no [`SceneElement`] (not selectable/hideable through the scene graph); its source
    /// body/sketch is a second input, surfaced once the element graph (#252) lands.
    DrawingProjection { drawing: crate::model::DrawingKey, view: usize },
    /// A component (#423): a named group row whose member roots nest beneath it; components
    /// nest inside each other via their `parent` link.
    Component(crate::model::ComponentKey),
    /// A text note on a drawing page (#333), nested under its [`HierarchyNode::Drawing`].
    /// `annotation` keys into the drawing's `annotations` (#1055). Like a projection it's a
    /// display-only leaf with no [`SceneElement`]; clicking it opens the drawing.
    DrawingAnnotation {
        drawing: crate::model::DrawingKey,
        annotation: crate::model::AnnotationKey,
    },
    /// An imported unit instance (#723): a selectable top-level row (its
    /// [`SceneElement::UnitInstance`] renames, hides, and deletes the instance). Its
    /// children ([`HierarchyNode::UnitChild`]) expand beneath it in the List view.
    UnitInstance(crate::model::UnitInstanceKey),
    /// One element inside an imported unit (#723): a **display-only, read-only** leaf (no
    /// [`SceneElement`]) shown when the instance row is expanded, so a user can look
    /// inside without being able to edit. `ordinal` indexes [`unit_child_rows`]'s output.
    UnitChild { instance: crate::model::UnitInstanceKey, ordinal: usize },
    /// A length dimension shown on a projection (#341), nested under its
    /// [`HierarchyNode::DrawingProjection`]. `a`/`b` are the dimensioned edge's quantized world
    /// endpoints. A display-only leaf; clicking it opens the drawing and selects the dimension.
    DrawingDimension { drawing: crate::model::DrawingKey, view: usize, a: [i32; 3], b: [i32; 3] },
    /// A free point-to-point dimension on a projection (#1645), nested under it like an edge
    /// dimension; `index` is its place in the view's `point_dims`.
    DrawingPointDim { drawing: crate::model::DrawingKey, view: usize, index: usize },
    /// A joint between parts (#891): a childless top-level row whose members feed it as
    /// graph inputs — a relationship, not a feature, so nothing nests under it.
    Joint(crate::model::JointKey),
}

/// Identifies an element whose visibility can be toggled.
///
/// Not `Copy` — see [`crate::model::ConstraintPoint`]'s doc comment: `Point` embeds a
/// `ConstraintPoint`, which embeds a `FaceId` for `FaceVertex` (#26/#27), and `FaceId` isn't
/// `Copy`. Callers that used to rely on implicit copies now need an explicit `.clone()`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SceneElement {
    /// A thing on a drawing page (#967): a projected view, a text note, or a shown dimension.
    /// The drawing workbench had its own parallel selection world because its items weren't
    /// scene elements; this is what lets its inputs be ordinary element pickers.
    DrawingElement {
        drawing: crate::model::DrawingKey,
        element: crate::context::DrawingElementRef,
    },
    ConstructionPlane(crate::model::ConstructionPlaneKey),
    Sketch(SketchId),
    Line(crate::model::LineKey),
    Circle(crate::model::CircleKey),
    Point(ConstraintPoint),
    Constraint(crate::model::ConstraintKey),
    Extrusion(crate::model::ExtrusionKey),
    Body(crate::model::BodyKey),
    /// A constraint-authoring line that isn't a sketch `Line`: a face's own edge
    /// (#26/#27), a sketch origin axis (#189), or a tracing-image displayed-quad
    /// edge (#1589). Mirrors `Point` wrapping the whole `ConstraintPoint` enum
    /// (the `Line` variant already has its own dedicated `SceneElement::Line`).
    FaceEdge(ConstraintLine),
    /// One feature edge of a body's solid mesh, selectable in 3D select mode (#156).
    /// Identified by its quantized world endpoints (see [`quantize_body_point`]) — a
    /// transient, geometry-keyed identity: if a rebuild moves the edge, the selection simply
    /// drops, which is acceptable for ephemeral (never persisted) selection state.
    BodyEdge {
        body: crate::model::BodyKey,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// A corner of a body's solid mesh, selectable in 3D select mode (#156); quantized like
    /// [`SceneElement::BodyEdge`].
    BodyVertex { body: crate::model::BodyKey, p: [i32; 3] },
    /// A projected edge on a drawing view (#1714): the page analogue of [`BodyEdge`].
    /// Keyed by its quantized world endpoints inside a view, plus the body it came from
    /// when the view is of bodies rather than a sketch.
    ProjectedEdge {
        drawing: crate::model::DrawingKey,
        view: usize,
        body: Option<crate::model::BodyKey>,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// A projected corner on a drawing view (#1714): the page analogue of [`BodyVertex`].
    ProjectedCorner {
        drawing: crate::model::DrawingKey,
        view: usize,
        body: Option<crate::model::BodyKey>,
        p: [i32; 3],
    },
    /// A planar face of a body's solid mesh, selectable in 3D select mode (#555/#557). A face
    /// has no stable index, so — like [`SceneElement::BodyEdge`] — its identity is its quantized
    /// geometry: the average of its triangle vertices (`centroid`) plus its `normal`, both
    /// quantized via [`quantize_body_point`]. Deterministic mesh → deterministic key, so two
    /// picks of the same face compare equal; a rebuild that moves the face simply drops the
    /// (ephemeral, never persisted) selection.
    BodyFace {
        body: crate::model::BodyKey,
        centroid: [i32; 3],
        normal: [i32; 3],
    },
    /// A **cylindrical** surface of a body's solid mesh (#1013): a hole's wall, a boss, a
    /// round shaft. Keyed by its fitted axis and radius — quantized like every other
    /// mesh-derived identity — so clicking a hole picks the hole, not one facet of it.
    BodyCylinder {
        body: crate::model::BodyKey,
        origin: [i32; 3],
        dir: [i32; 3],
        radius: i32,
    },
    /// A cylindrical surface's **centre line** (#1013): derived geometry with no owning
    /// entity, the way [`SceneElement::GlobalAxis`] gave the world axes an identity. This is
    /// what "put this hole on that shaft" and "slide along this bore" are actually about.
    BodyAxis {
        body: crate::model::BodyKey,
        origin: [i32; 3],
        dir: [i32; 3],
    },
    /// A tracing image (#163/#169).
    Image(crate::model::TracingImageKey),
    /// A boolean operation between bodies (Combine tool).
    BooleanOp(crate::model::BooleanOpKey),
    /// A move operation on bodies (Move tool).
    MoveOp(crate::model::MoveOpKey),
    /// A mirror operation on bodies (Mirror tool, #523).
    MirrorOp(crate::model::MirrorOpKey),
    /// A linear repeat on bodies (Repeat tool).
    RepeatOp(crate::model::RepeatOpKey),
    /// A 2D in-sketch linear repeat (#222/#228).
    SketchRepeatOp(crate::model::SketchRepeatOpKey),
    SketchOffsetOp(crate::model::SketchOffsetOpKey),
    /// A 2D in-sketch mirror (#523/#1540); nests under its sketch, with reflected
    /// lines/circles under it.
    SketchMirrorOp(crate::model::SketchMirrorOpKey),
    /// A 2D in-sketch chamfer/fillet (#538); its trimmed copies + bridge lines nest under it.
    SketchVertexTreatmentOp(crate::model::SketchVertexTreatmentOpKey),
    /// A 2D in-sketch slice (#224/#229).
    SketchSliceOp(crate::model::SketchSliceOpKey),
    /// A sketch text element (#282): selecting it selects the whole text.
    SketchText(crate::model::SketchTextKey),
    /// A slice operation on bodies (Slice tool).
    SliceOp(crate::model::SliceOpKey),
    /// A shell operation on bodies (Shell tool, #1156).
    ShellOp(crate::model::ShellOpKey),
    /// An edge chamfer/fillet operation on bodies (#531).
    EdgeTreatmentOp(crate::model::EdgeTreatmentOpKey),
    /// A revolved solid (Revolve tool, #211).
    Revolution(crate::model::RevolutionKey),
    /// A primitive shape placed straight into 3D (Create Shape tool, #909).
    Shape(crate::model::PrimitiveKey),
    /// A sweep (Sweep tool).
    SweepOp(crate::model::SweepKey),
    /// A loft (Loft tool, #1487).
    Loft(crate::model::LoftKey),
    /// A drawing page (#1525): named by `move_to_component{ kind = "drawing" }`.
    /// A cross-section view (#1671): selectable, renameable and deletable like a drawing.
    CrossSection(crate::model::CrossSectionKey),
    /// One cutting plane of a view: selectable, renameable, deletable, and hideable.
    SectionPlane {
        view: crate::model::CrossSectionKey,
        cut: usize,
    },
    Drawing(crate::model::DrawingKey),
    /// The origin, selectable in a sketch so a point can be constrained coincident to it from
    /// the constraint tool (#189). Fixed geometry with no owning entity, like `FaceEdge`.
    Origin,
    /// One of the world axes (#952). Fixed geometry with no owning entity, like `Origin`: the
    /// axes are pickable (a Repeat path, a Revolve axis) so they need an identity an element
    /// picker can hold. Not a row in the Elements pane — there is no `HierarchyNode` for it.
    GlobalAxis(crate::construction::GlobalAxis),
    /// An **analytic** face (#952): a sketch profile, a body cap/side wall, or a revolve's flat
    /// face — exactly what `face::pick_sketch_face` picks and what `PickTargetKind::SketchFace`
    /// carries. Distinct from [`SceneElement::BodyFace`], which is a *mesh* face keyed by its
    /// quantized centroid+normal: the analytic face is the parametric thing an Extrude profile,
    /// a Revolve/Sweep profile, or a Slice cutter is defined against.
    ///
    /// Build one with [`SceneElement::from_face_id`], never directly — a
    /// [`FaceId::ConstructionPlane`] normalizes to [`SceneElement::ConstructionPlane`] so a plane
    /// has one identity rather than two.
    SketchFace(FaceId),
    /// A point the Move and Joint tools snap from or onto (#952): an edge midpoint, a point
    /// along an edge, or a planar face's middle. Their six point pickers each hold one of
    /// these, and had no element to put in a picker before.
    ///
    /// Build one with [`SceneElement::from_move_point`], never directly — the corner and origin
    /// cases normalize to [`SceneElement::BodyVertex`] / [`SceneElement::Origin`], which already
    /// name those points.
    ///
    MovePoint(crate::model::MovePointRef),
    /// One **analytic** edge of an extrusion's solid (#952) — what the 3D Chamfer/Fillet tool
    /// treats. Distinct from [`SceneElement::BodyEdge`], the quantized *mesh* edge: this one is
    /// the parametric edge a committed `EdgeTreatment` is defined against.
    #[allow(dead_code)]
    ExtrusionEdge {
        extrusion: crate::model::ExtrusionKey,
        edge: crate::model::ExtrusionEdgeRef,
    },
    /// One analytic edge of a Shape-tool primitive (#1329) — the cuboid/cylinder analogue
    /// of [`SceneElement::ExtrusionEdge`].
    PrimitiveEdge {
        primitive: crate::model::PrimitiveKey,
        edge: crate::model::ExtrusionEdgeRef,
    },
    /// A **repeat instance's** face (#452/#955): the source face's plane translated along the
    /// repeat axis by that instance's offset. Parametric — it follows when the repeat's spacing
    /// or the source body changes — and it is not the source face, which is a different plane,
    /// so it needs an identity of its own. What an "extrude up to" / "distance to" / joint-stop
    /// picker holds when the user snapped to a copy rather than to the original.
    RepeatedFace {
        face: FaceId,
        op: crate::model::RepeatOpKey,
        instance: usize,
    },
    /// A component (#423): a named, nestable group of top-level elements. Hiding one hides
    /// everything inside it.
    Component(crate::model::ComponentKey),
    /// An imported unit instance (#723): one row per placement of an imported document.
    /// Selecting, renaming, hiding, and deleting act on the instance; its contents are
    /// read-only from the importing document.
    UnitInstance(crate::model::UnitInstanceKey),
    /// A joint between parts (#891): a kinematic relationship, selectable and deletable
    /// like any operation.
    Joint(crate::model::JointKey),
}

impl SceneElement {
    /// The element for an analytic face (#952), normalizing the one case where a `FaceId` names
    /// something that already has an element of its own: a construction plane. Without that a
    /// plane would have two identities, and a picker holding both would count it twice.
    pub fn from_face_id(face: FaceId) -> SceneElement {
        match face {
            FaceId::ConstructionPlane(index) => SceneElement::ConstructionPlane(index),
            other => SceneElement::SketchFace(other),
        }
    }

    /// The analytic face this element names, if any — the inverse of [`from_face_id`], so a
    /// picker's contents can be handed back to the geometry code as `FaceId`s.
    ///
    /// [`from_face_id`]: SceneElement::from_face_id
    #[allow(dead_code)]
    pub fn as_face_id(&self) -> Option<FaceId> {
        match self {
            SceneElement::SketchFace(face) => Some(face.clone()),
            SceneElement::ConstructionPlane(index) => Some(FaceId::ConstructionPlane(*index)),
            _ => None,
        }
    }

    /// The element for a Move/Joint snap point (#952), normalizing the two cases that name
    /// something with an element already: a body corner is that corner, and the origin point is
    /// the origin.
    pub fn from_move_point(point: crate::model::MovePointRef) -> SceneElement {
        use crate::model::MovePointRef;
        match point {
            MovePointRef::Vertex { body, p } => SceneElement::BodyVertex { body, p },
            MovePointRef::Origin => SceneElement::Origin,
            MovePointRef::ImageAnchor { image, anchor } => SceneElement::Point(
                crate::model::ConstraintPoint::ImageAnchor { image, anchor },
            ),
            other => SceneElement::MovePoint(other),
        }
    }

    /// The element a mate pick holds (#1014/#1015). Every [`crate::model::MateRef`] already
    /// names an element the pickers and the Elements pane know, so this is a straight
    /// re-spelling — which is what lets the mate rows be ordinary element pickers.
    pub fn from_mate_ref(r: &crate::model::MateRef) -> SceneElement {
        use crate::model::MateRef;
        match r {
            MateRef::Face { body, centroid, normal } => SceneElement::BodyFace {
                body: *body,
                centroid: *centroid,
                normal: *normal,
            },
            MateRef::Plane(i) => SceneElement::ConstructionPlane(*i),
            MateRef::Edge { body, a, b } => SceneElement::BodyEdge { body: *body, a: *a, b: *b },
            MateRef::Axis(a) => SceneElement::GlobalAxis(*a),
            MateRef::Point(p) => SceneElement::from_move_point(*p),
            MateRef::HoleAxis { body, origin, dir } => SceneElement::BodyAxis {
                body: *body,
                origin: *origin,
                dir: *dir,
            },
        }
    }

    /// The mate pick an element stands for, if a mate can hold it — the inverse of
    /// [`SceneElement::from_mate_ref`], and what turns a viewport click into a mate row.
    pub fn to_mate_ref(&self) -> Option<crate::model::MateRef> {
        use crate::model::{MateRef, MovePointRef};
        Some(match self {
            SceneElement::BodyFace { body, centroid, normal } => MateRef::Face {
                body: *body,
                centroid: *centroid,
                normal: *normal,
            },
            SceneElement::ConstructionPlane(i) => MateRef::Plane(*i),
            SceneElement::BodyEdge { body, a, b } => {
                MateRef::Edge { body: *body, a: *a, b: *b }
            }
            SceneElement::GlobalAxis(a) => MateRef::Axis(*a),
            SceneElement::BodyVertex { body, p } => {
                MateRef::Point(MovePointRef::Vertex { body: *body, p: *p })
            }
            SceneElement::Origin => MateRef::Point(MovePointRef::Origin),
            SceneElement::MovePoint(p) => MateRef::Point(*p),
            // A hole's centre line is what "line these up" usually means (#1013).
            SceneElement::BodyAxis { body, origin, dir } => MateRef::HoleAxis {
                body: *body,
                origin: *origin,
                dir: *dir,
            },
            _ => return None,
        })
    }

    /// The element for an "extrude up to" style target (#955): a vertex, a face, a plane, or a
    /// repeat instance's translated face. Every `ExtrudeTarget` maps to exactly one element.
    pub fn from_extrude_target(target: &crate::model::ExtrudeTarget) -> SceneElement {
        use crate::model::ExtrudeTarget;
        match target {
            ExtrudeTarget::Vertex(point) => SceneElement::Point(point.clone()),
            ExtrudeTarget::Face(face) => SceneElement::from_face_id(face.face_id()),
            ExtrudeTarget::Plane(index) => SceneElement::ConstructionPlane(*index),
            ExtrudeTarget::BodyFace(face) => SceneElement::from_face_id(face.clone()),
            ExtrudeTarget::RepeatedFace { face, op, instance } => SceneElement::RepeatedFace {
                face: face.clone(),
                op: *op,
                instance: *instance,
            },
        }
    }

    /// The element for a joint member (#955): a body, a component, or a unit instance — each
    /// of which already has one.
    pub fn from_joint_ref(member: crate::model::JointRef) -> SceneElement {
        use crate::model::JointRef;
        match member {
            JointRef::Body(index) => SceneElement::Body(index),
            JointRef::Component(index) => SceneElement::Component(index),
            JointRef::UnitInstance(index) => SceneElement::UnitInstance(index),
        }
    }

    /// The joint member this element stands for (#991) — the inverse of [`Self::from_joint_ref`].
    ///
    /// A unit's materialized body joins as its **instance**: the joint poses the placement, not
    /// the generated solid (#894). `None` for anything a joint can't hold.
    pub fn as_joint_ref(&self, doc: &Document) -> Option<crate::model::JointRef> {
        use crate::model::JointRef;
        match self {
            SceneElement::Body(index) => match doc.bodies.get(*index).map(|b| &b.source) {
                Some(crate::model::BodySource::UnitInstance(i)) => Some(JointRef::UnitInstance(*i)),
                _ => Some(JointRef::Body(*index)),
            },
            SceneElement::Component(index) => Some(JointRef::Component(*index)),
            SceneElement::UnitInstance(index) => Some(JointRef::UnitInstance(*index)),
            _ => None,
        }
    }

    /// The element for a straight reference axis (#952/#955): a sketch line, a body's feature
    /// edge, or one of the world axes. What the Revolve axis and Repeat path pickers hold.
    pub fn from_revolve_axis(axis: crate::model::RevolveAxis) -> SceneElement {
        use crate::model::RevolveAxis;
        match axis {
            RevolveAxis::Line(index) => SceneElement::Line(index),
            RevolveAxis::BodyEdge { body, a, b } => {
                let (qa, qb) = (quantize_body_point(a), quantize_body_point(b));
                // Canonically ordered, like every other body-edge key, so either traversal
                // direction of the same edge is one element.
                let (qa, qb) = if qa <= qb { (qa, qb) } else { (qb, qa) };
                SceneElement::BodyEdge { body, a: qa, b: qb }
            }
            RevolveAxis::X => SceneElement::GlobalAxis(crate::construction::GlobalAxis::X),
            RevolveAxis::Y => SceneElement::GlobalAxis(crate::construction::GlobalAxis::Y),
            RevolveAxis::Z => SceneElement::GlobalAxis(crate::construction::GlobalAxis::Z),
        }
    }

    pub fn from_sketch_mirror_axis(axis: crate::model::SketchMirrorAxis) -> SceneElement {
        use crate::construction::GlobalAxis;
        use crate::model::SketchMirrorAxis;
        match axis {
            SketchMirrorAxis::Line(index) => SceneElement::Line(index),
            SketchMirrorAxis::OriginAxis(axis) => {
                SceneElement::FaceEdge(crate::model::ConstraintLine::OriginAxis(axis))
            }
            SketchMirrorAxis::X => SceneElement::GlobalAxis(GlobalAxis::X),
            SketchMirrorAxis::Y => SceneElement::GlobalAxis(GlobalAxis::Y),
            SketchMirrorAxis::Z => SceneElement::GlobalAxis(GlobalAxis::Z),
        }
    }

    pub fn as_sketch_mirror_axis(&self) -> Option<crate::model::SketchMirrorAxis> {
        use crate::construction::GlobalAxis;
        use crate::model::SketchMirrorAxis;
        Some(match self {
            SceneElement::Line(index) => SketchMirrorAxis::Line(*index),
            SceneElement::FaceEdge(crate::model::ConstraintLine::OriginAxis(axis)) => {
                SketchMirrorAxis::OriginAxis(*axis)
            }
            SceneElement::GlobalAxis(GlobalAxis::X) => SketchMirrorAxis::X,
            SceneElement::GlobalAxis(GlobalAxis::Y) => SketchMirrorAxis::Y,
            SceneElement::GlobalAxis(GlobalAxis::Z) => SketchMirrorAxis::Z,
            _ => return None,
        })
    }

    /// The straight reference this element names, if any — the inverse of
    /// [`from_revolve_axis`](SceneElement::from_revolve_axis). A Revolve's axis and a Repeat's
    /// path are both this, so a pick into either picker converts here rather than in a
    /// per-tool `match` on the pick target (#970).
    pub fn as_revolve_axis(&self) -> Option<crate::model::RevolveAxis> {
        use crate::model::RevolveAxis;
        Some(match self {
            SceneElement::Line(index) => RevolveAxis::Line(*index),
            SceneElement::BodyEdge { body, a, b } => RevolveAxis::BodyEdge {
                body: *body,
                a: dequantize_body_point(*a),
                b: dequantize_body_point(*b),
            },
            // A hole's or a shaft's centre line is a straight reference like any other
            // (#1013): revolve about it, repeat along it, slide down it.
            SceneElement::BodyAxis { body, origin, dir } => {
                let (o, d) = (dequantize_body_point(*origin), dequantize_body_point(*dir));
                RevolveAxis::BodyEdge { body: *body, a: o - d, b: o + d }
            }
            SceneElement::GlobalAxis(crate::construction::GlobalAxis::X) => RevolveAxis::X,
            SceneElement::GlobalAxis(crate::construction::GlobalAxis::Y) => RevolveAxis::Y,
            SceneElement::GlobalAxis(crate::construction::GlobalAxis::Z) => RevolveAxis::Z,
            _ => return None,
        })
    }

    /// The snap point this element names, if any — the inverse of [`from_move_point`].
    ///
    /// [`from_move_point`]: SceneElement::from_move_point
    #[allow(dead_code)]
    pub fn as_move_point(&self) -> Option<crate::model::MovePointRef> {
        use crate::model::MovePointRef;
        match self {
            SceneElement::MovePoint(point) => Some(*point),
            SceneElement::BodyVertex { body, p } => {
                Some(MovePointRef::Vertex { body: *body, p: *p })
            }
            SceneElement::Origin => Some(MovePointRef::Origin),
            SceneElement::Point(crate::model::ConstraintPoint::ImageAnchor { image, anchor }) => {
                Some(MovePointRef::ImageAnchor {
                    image: *image,
                    anchor: *anchor,
                })
            }
            _ => None,
        }
    }
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
        // Display-only leaves with no independent selectable/hideable identity (#192/#180/#1205).
        HierarchyNode::Document
        | HierarchyNode::Drawings
        // The Views section groups cross-section views; the views themselves are elements
        // (#1671), so only the section header is display-only.
        | HierarchyNode::Views
        | HierarchyNode::EdgeTreatment { .. }
        | HierarchyNode::Drawing(_)
        | HierarchyNode::DrawingProjection { .. }
        | HierarchyNode::DrawingAnnotation { .. }
        | HierarchyNode::DrawingDimension { .. }
        | HierarchyNode::DrawingPointDim { .. }
        // A unit's contents are read-only from the importing document (#723): no scene
        // identity means no selection, visibility, deletion, or renaming can target them.
        | HierarchyNode::UnitChild { .. } => return None,
        HierarchyNode::ConstructionPlane(i) => SceneElement::ConstructionPlane(i),
        HierarchyNode::CrossSection(i) => SceneElement::CrossSection(i),
        HierarchyNode::SectionPlane { view, cut } => SceneElement::SectionPlane { view, cut },
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
        HierarchyNode::ShellOp(i) => SceneElement::ShellOp(i),
        HierarchyNode::EdgeTreatmentOp(i) => SceneElement::EdgeTreatmentOp(i),
        HierarchyNode::Revolution(i) => SceneElement::Revolution(i),
        HierarchyNode::Shape(i) => SceneElement::Shape(i),
        HierarchyNode::SweepOp(i) => SceneElement::SweepOp(i),
        HierarchyNode::Loft(i) => SceneElement::Loft(i),
        HierarchyNode::Component(i) => SceneElement::Component(i),
        HierarchyNode::UnitInstance(i) => SceneElement::UnitInstance(i),
        HierarchyNode::Joint(i) => SceneElement::Joint(i),
    })
}

/// The [`SceneElement`] for an operation whose editing is opened the **universal** way — a
/// double-click on the row or a right-click → "Edit" (#546 / #1486) — reloading it into its tool.
/// `None` for elements edited through their own dedicated entry (sketches, planes, extrusions,
/// 3D edge treatments, drawings) or that aren't operations at all.
pub fn node_editable_operation(node: HierarchyNode) -> Option<SceneElement> {
    match node {
        HierarchyNode::BooleanOp(i) => Some(SceneElement::BooleanOp(i)),
        HierarchyNode::MoveOp(i) => Some(SceneElement::MoveOp(i)),
        HierarchyNode::MirrorOp(i) => Some(SceneElement::MirrorOp(i)),
        HierarchyNode::RepeatOp(i) => Some(SceneElement::RepeatOp(i)),
        HierarchyNode::SliceOp(i) => Some(SceneElement::SliceOp(i)),
        HierarchyNode::ShellOp(i) => Some(SceneElement::ShellOp(i)),
        HierarchyNode::Revolution(i) => Some(SceneElement::Revolution(i)),
        HierarchyNode::Shape(i) => Some(SceneElement::Shape(i)),
        HierarchyNode::SweepOp(i) => Some(SceneElement::SweepOp(i)),
        HierarchyNode::Loft(i) => Some(SceneElement::Loft(i)),
        HierarchyNode::SketchMirrorOp(i) => Some(SceneElement::SketchMirrorOp(i)),
        HierarchyNode::SketchOffsetOp(i) => Some(SceneElement::SketchOffsetOp(i)),
        HierarchyNode::SketchRepeatOp(i) => Some(SceneElement::SketchRepeatOp(i)),
        HierarchyNode::SketchSliceOp(i) => Some(SceneElement::SketchSliceOp(i)),
        HierarchyNode::SketchVertexTreatmentOp(i) => Some(SceneElement::SketchVertexTreatmentOp(i)),
        HierarchyNode::Joint(i) => Some(SceneElement::Joint(i)),
        // A cross-section view opens the View workbench (#1671) — a double-click or the
        // row's Edit entry, the same universal path every operation uses.
        HierarchyNode::CrossSection(i) => Some(SceneElement::CrossSection(i)),
        // Double-click / Edit reopens the plane in the cutting-plane tool (#1755) — its
        // live offset/tilt draft — not just the parent view's workbench (#1767).
        HierarchyNode::SectionPlane { view, cut } => {
            Some(SceneElement::SectionPlane { view, cut })
        }
        _ => None,
    }
}

/// A rest-pose command from a joint row's context menu (#898).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointRestCommand {
    /// Capture the joint's current position as its rest pose.
    SetRest(crate::model::JointKey),
    /// Put the joint back to its rest pose.
    Revert(crate::model::JointKey),
    /// Put every joint back to its rest pose.
    RevertAll,
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

    /// Whether any element in `targets` is individually visible (own flag only).
    pub fn any_visible(&self, targets: &[SceneElement]) -> bool {
        targets.iter().any(|e| self.is_visible(e.clone()))
    }

    /// Hide every element in `extra` on top of the current toggles (#524): the rollback
    /// marker builds a render-only visibility that suppresses everything created after it.
    pub fn with_hidden(&self, extra: &HashSet<SceneElement>) -> Self {
        let mut merged = self.clone();
        merged.hidden.extend(extra.iter().cloned());
        merged
    }

    /// Whether `component` and all its ancestors are individually visible (#423).
    fn component_chain_visible(&self, doc: &Document, component: crate::model::ComponentKey) -> bool {
        doc.component_chain(component)
            .into_iter()
            .all(|c| self.is_visible(SceneElement::Component(c)))
    }

    /// What a construction plane inherits from its ancestors, ignoring its own hidden flag
    /// (#667) — the part sketches drawn on it still follow.
    fn plane_inherited_visible(&self, doc: &Document, index: crate::model::ConstructionPlaneKey) -> bool {
        let Some(plane) = doc.construction_planes.get(index) else {
            return true;
        };
        if let Some(c) = owning_component(doc, &SceneElement::ConstructionPlane(index)) {
            if !self.component_chain_visible(doc, c) {
                return false;
            }
        }
        match plane.parent {
            ConstructionPlaneParent::Root => true,
            ConstructionPlaneParent::Sketch(sketch) => {
                self.effective_visible(doc, SceneElement::Sketch(sketch))
            }
        }
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
            // A drawing's items are shown on the page, not hidden through the scene (#967/#1714).
            SceneElement::DrawingElement { .. }
            | SceneElement::ProjectedEdge { .. }
            | SceneElement::ProjectedCorner { .. } => true,
            // A cross-section view is a way of looking, not a thing in the scene (#1671).
            SceneElement::CrossSection(_) => true,
            SceneElement::SectionPlane { view, .. } => {
                self.effective_visible(doc, SceneElement::CrossSection(view))
            }
            // A unit instance's visibility is just its own toggle (#723).
            SceneElement::UnitInstance(_) => true,
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
            // A sketch follows the thing it's drawn on — except that hiding a **construction
            // plane** doesn't hide sketches on it (#667). Hiding a plane puts its display quad
            // away; the geometry sketched on it isn't part of the plane and stays. Only the
            // plane's *own* flag is skipped, not what it inherits — a plane anchored to a
            // hidden sketch is still gone, and so is anything sketched on it. A body face is
            // different again: hide the body and the face isn't there, so its sketches go too.
            // Shadow bodies count as gone too (#1219/#1221): a sketch on a consumed cuboid /
            // fuse solid must not steal picks from the live cut pieces that replaced it.
            SceneElement::Sketch(sketch) => doc.sketch_face(sketch).is_some_and(|face| {
                match face {
                    FaceId::ConstructionPlane(i) => self.plane_inherited_visible(doc, i),
                    other => {
                        if let Some(bi) = crate::model::body_index_for_face(doc, &other) {
                            if doc.bodies.get(bi).is_some_and(|b| b.shadow) {
                                return false;
                            }
                            return self.effective_visible(doc, SceneElement::Body(bi));
                        }
                        self.effective_visible(doc, face_element(other))
                    }
                }
            }),
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
            // A face's own edge tracks the feature that produced its face, same as
            // `FaceVertex` in `point_effective_visible` below.
            SceneElement::FaceEdge(line) => {
                let owner = match &line {
                    ConstraintLine::FaceEdge { face, .. } => face_owner_element(face),
                    ConstraintLine::Line(_) | ConstraintLine::OriginAxis(_) => None,
                    ConstraintLine::ImageEdge { image, .. } => Some(SceneElement::Image(*image)),
                };
                // With no owning feature there is nothing to inherit from: a face edge
                // whose face names no extrusion is visible on its own (#1055 — there is no
                // "impossible index" to stand in for one any more).
                owner.is_none_or(|owner| self.effective_visible(doc, owner))
            }
            // A body's own edge/vertex/face (#156/#555) is visible exactly when its body is.
            SceneElement::BodyEdge { body, .. }
            | SceneElement::BodyVertex { body, .. }
            | SceneElement::BodyFace { body, .. }
            | SceneElement::BodyCylinder { body, .. }
            | SceneElement::BodyAxis { body, .. } => {
                self.effective_visible(doc, SceneElement::Body(body))
            }
            // An analytic face (#952) has no row of its own; its owner's visibility governs
            // whether it can be seen at all, and that is enforced where the owner draws.
            SceneElement::SketchFace(_) => true,
            // An extrusion's analytic edge (#952) shows when its extrusion does.
            SceneElement::ExtrusionEdge { extrusion, .. } => {
                self.effective_visible(doc, SceneElement::Extrusion(extrusion))
            }
            SceneElement::PrimitiveEdge { primitive, .. } => {
                self.effective_visible(doc, SceneElement::Shape(primitive))
            }
            // A repeat instance's face shows when its repeat does (#955).
            SceneElement::RepeatedFace { op, .. } => {
                self.effective_visible(doc, SceneElement::RepeatOp(op))
            }
            // A snap point (#952) shows exactly when the body it sits on does.
            SceneElement::MovePoint(point) => match point.body() {
                Some(body) => self.effective_visible(doc, SceneElement::Body(body)),
                None => true,
            },
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
            SceneElement::ShellOp(_) => true,
            SceneElement::EdgeTreatmentOp(_) => true,
            SceneElement::Revolution(_) => true,
            SceneElement::Shape(_) => true,
            SceneElement::SweepOp(_) => true,
            SceneElement::Loft(_) => true,
            SceneElement::Drawing(_) => true,
            // A joint is a relationship, not geometry — its icon shows whenever its
            // parts do (#891).
            SceneElement::Joint(_) => true,
            // The origin and the world axes are always visible (#189/#952).
            SceneElement::Origin | SceneElement::GlobalAxis(_) => true,
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
        ConstraintPoint::Origin => true,
        // A face's own vertex tracks the feature that produced its face — same dependency
        // `face_element` gives a sketch placed on a body cap/side wall.
        // With no owning feature there is nothing to inherit from (#1055).
        ConstraintPoint::FaceVertex { face, .. } => face_owner_element(&face)
            .is_none_or(|owner| visibility.effective_visible(doc, owner)),
        ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.get(text).is_some_and(|entity| {
                visibility.effective_visible(doc, SceneElement::Sketch(entity.sketch))
            })
        }
        ConstraintPoint::ImageCalibrationPoint { image, .. }
        | ConstraintPoint::ImageAnchor { image, .. } => {
            visibility.effective_visible(doc, SceneElement::Image(image))
        }
    }
}

pub fn face_element(face: FaceId) -> SceneElement {
    match face {
        // A sketch on a unit's face depends on the instance (#725).
        FaceId::UnitFace { instance, .. } => SceneElement::UnitInstance(instance),
        FaceId::ConstructionPlane(i) => SceneElement::ConstructionPlane(i),
        FaceId::Circle(i) => SceneElement::Circle(i),
        // A polygon face is just a closed loop of existing lines (#66); its visibility
        // tracks its first constituent line.
        FaceId::Polygon(lines) => SceneElement::Line(lines[0]),
        // A sketch on a body cap or side wall depends on the extrusion that produced it.
        FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
            SceneElement::Extrusion(extrusion)
        }
        // A sketch on a revolve's flat side depends on that revolution (#621).
        FaceId::RevolveCap { revolution, .. } | FaceId::RevolveSide { revolution, .. } => {
            SceneElement::Revolution(revolution)
        }
        // A sketch on a primitive shape's face depends on that primitive (#1103).
        FaceId::PrimitiveFace { primitive, .. } => SceneElement::Shape(primitive),
        // A sketch on a repeated body face depends on that instance (#1116).
        FaceId::RepeatedFace { face, op, instance } => SceneElement::RepeatedFace {
            face: *face,
            op,
            instance,
        },
        // A sketch on a mesh face depends on the body that owns the mesh (#1173).
        FaceId::BodyMeshFace { body, .. } => SceneElement::Body(body),
    }
}

/// The feature element that produced a body face — the owning extrusion or, for a
/// partial revolve's flat side, the owning revolution (#621). `None` for sketch-profile
/// and construction-plane faces (they have no producing feature).
pub fn face_owner_element(face: &FaceId) -> Option<SceneElement> {
    face.extrusion_index()
        .map(SceneElement::Extrusion)
        .or_else(|| face.revolution_key().map(SceneElement::Revolution))
}

/// Which of the Elements pane's bottom sections are collapsed (#1205/#1671) — UI-only state
/// held by the app, never part of the document.
#[derive(Clone, Copy, Debug, Default)]
pub struct SectionCollapse {
    pub drawings: bool,
    pub views: bool,
}

impl SectionCollapse {
    /// Whether `node`'s section is collapsed; `false` for anything that isn't a section.
    fn collapsed(&self, node: HierarchyNode) -> bool {
        match node {
            HierarchyNode::Drawings => self.drawings,
            HierarchyNode::Views => self.views,
            _ => false,
        }
    }

    fn toggle(&mut self, node: HierarchyNode) {
        match node {
            HierarchyNode::Drawings => self.drawings = !self.drawings,
            HierarchyNode::Views => self.views = !self.views,
            _ => {}
        }
    }
}

/// Whether a node is one of the pane's bottom grouping sections (#1205/#1671).
fn is_section_node(node: HierarchyNode) -> bool {
    matches!(node, HierarchyNode::Drawings | HierarchyNode::Views)
}

/// Drop cutting-plane rows so they only appear while the View workbench is open (#1761).
pub(crate) fn prune_section_planes(tree: &mut [HierarchyEntry]) {
    for entry in tree {
        if matches!(entry.node, HierarchyNode::CrossSection(_)) {
            entry.children.clear();
        } else {
            prune_section_planes(&mut entry.children);
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

/// `(input, consumer)` dependency pairs for the Graph view (#266/#281): relationships beyond the
/// single tree parent — an operation's **input** elements feeding it, and a drawing projection's
/// **source**. These become the input edges of the eventual full element graph (#252). Both
/// endpoints are [`HierarchyNode`]s; the renderer skips any pair whose nodes aren't on screen.
pub fn graph_dependency_edges(doc: &Document) -> Vec<(HierarchyNode, HierarchyNode)> {
    let mut edges = Vec::new();

    // Boolean operations consume their side-A/side-B input bodies (now shadows) (#266).
    for (oi, op) in doc.boolean_ops.iter() {
        for &bi in op.a.iter().chain(op.b.iter()) {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::BooleanOp(oi)));
        }
    }
    // Move and Slice operations consume their input bodies too.
    for (oi, op) in doc.move_ops.iter() {
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::MoveOp(oi)));
        }
    }
    // A mirror consumes its input bodies (and its plane face's body, if any) — #523.
    for (oi, op) in doc.mirror_ops.iter() {
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::MirrorOp(oi)));
        }
    }
    for (oi, op) in doc.slice_ops.iter() {
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::SliceOp(oi)));
        }
    }
    // An edge-treatment op consumes the input bodies whose edges it bevels (#531). The treated
    // edges themselves have no persistent node, so the body carries the dependency.
    for (oi, op) in doc.edge_treatment_ops.iter() {
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::EdgeTreatmentOp(oi)));
        }
    }
    for (oi, op) in doc.shell_ops.iter() {
        for &bi in &op.targets {
            edges.push((HierarchyNode::Body(bi), HierarchyNode::ShellOp(oi)));
        }
    }

    // A loft is fed by its cross-section sketches (#252) — the user's canonical example: three
    // sketches feeding one loft that outputs a body.
    for (li, loft) in doc.lofts.iter() {
        let mut seen = std::collections::HashSet::new();
        for section in &loft.sections {
            if seen.insert(section.sketch) {
                edges.push((HierarchyNode::Sketch(section.sketch), HierarchyNode::Loft(li)));
            }
        }
    }

    // A repeat consumes its input bodies, source planes/sketches, and replayed cut
    // extrusions (#448): the original body is the repeat's parent, not a sibling.
    for (oi, op) in doc.repeat_ops.iter() {
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
    for (oi, op) in doc.move_ops.iter() {
        for &pi in &op.plane_targets {
            edges.push((HierarchyNode::ConstructionPlane(pi), HierarchyNode::MoveOp(oi)));
        }
        for &ii in &op.image_targets {
            edges.push((HierarchyNode::Image(ii), HierarchyNode::MoveOp(oi)));
        }
    }
    // A joint's members feed it (#891): two (or more) inputs, no outputs.
    for (ji, joint) in doc.joints.iter() {
        for member in &joint.members {
            let input = match *member {
                crate::model::JointRef::Body(bi) => HierarchyNode::Body(bi),
                crate::model::JointRef::Component(ci) => HierarchyNode::Component(ci),
                crate::model::JointRef::UnitInstance(ui) => HierarchyNode::UnitInstance(ui),
            };
            edges.push((input, HierarchyNode::Joint(ji)));
        }
    }
    // A slice's cutters feed it (#449/#1126/#1151): construction planes and sketch lines have
    // nodes; body faces don't. Laser-line cutters also take their **sketch** as an input so
    // the defining sketch shows as a dependency of the slice (and sketch edits cascade).
    for (oi, op) in doc.slice_ops.iter() {
        let mut seen_sketches = std::collections::HashSet::new();
        for cutter in &op.cutters {
            match cutter {
                crate::model::SliceCutter::Face(FaceId::ConstructionPlane(pi)) => {
                    edges.push((HierarchyNode::ConstructionPlane(*pi), HierarchyNode::SliceOp(oi)));
                }
                crate::model::SliceCutter::Line { line } => {
                    edges.push((HierarchyNode::Line(*line), HierarchyNode::SliceOp(oi)));
                    if let Some(line) = doc.lines.get(*line) {
                        if seen_sketches.insert(line.sketch) {
                            edges.push((
                                HierarchyNode::Sketch(line.sketch),
                                HierarchyNode::SliceOp(oi),
                            ));
                        }
                    }
                }
                crate::model::SliceCutter::Face(_) => {}
            }
        }
    }
    // A revolution is fed by its profile sketch, and by its axis line if any (#449).
    for (ri, rev) in doc.revolutions.iter() {
        edges.push((HierarchyNode::Sketch(rev.sketch), HierarchyNode::Revolution(ri)));
        if let crate::model::RevolveAxis::Line(li) = rev.axis {
            edges.push((HierarchyNode::Line(li), HierarchyNode::Revolution(ri)));
        }
    }
    // A sweep is fed by its profile sketch and every path line.
    for (fi, fp) in doc.sweeps.iter() {
        edges.push((HierarchyNode::Sketch(fp.sketch), HierarchyNode::SweepOp(fi)));
        for &li in &fp.path {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SweepOp(fi)));
        }
    }
    // In-sketch ops consume their source lines/circles (#449); the in-sketch slice also
    // its cutter lines.
    for (oi, op) in doc.sketch_repeat_ops.iter() {
        for &li in &op.line_targets {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchRepeatOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchRepeatOp(oi)));
        }
    }
    for (oi, op) in doc.sketch_offset_ops.iter() {
        for &li in &op.line_targets {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchOffsetOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchOffsetOp(oi)));
        }
    }
    // An in-sketch mirror consumes its mirror line and every source line/circle (#523).
    for (oi, op) in doc.sketch_mirror_ops.iter() {
        if let crate::model::SketchMirrorAxis::Line(li) = op.line {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchMirrorOp(oi)));
        }
        for &li in &op.line_targets {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchMirrorOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchMirrorOp(oi)));
        }
    }
    // An in-sketch chamfer/fillet consumes its (shadowed) source edges (#538).
    for (oi, op) in doc.sketch_vertex_treatment_ops.iter() {
        for &li in &op.line_targets {
            edges.push((
                HierarchyNode::Line(li),
                HierarchyNode::SketchVertexTreatmentOp(oi),
            ));
        }
    }
    for (oi, op) in doc.sketch_slice_ops.iter() {
        for &li in op.line_targets.iter().chain(op.cutter_lines.iter()) {
            edges.push((HierarchyNode::Line(li), HierarchyNode::SketchSliceOp(oi)));
        }
        for &ci in &op.circle_targets {
            edges.push((HierarchyNode::Circle(ci), HierarchyNode::SketchSliceOp(oi)));
        }
    }
    // A drawing projection depends on its source body/sketch (#281). Multi-body views
    // depend on every body (#1190/#1191).
    for (di, drawing) in doc.drawings.iter() {
        for (vi, view) in drawing.views.iter().enumerate() {
            if let Some(si) = view.sketch {
                edges.push((
                    HierarchyNode::Sketch(si),
                    HierarchyNode::DrawingProjection { drawing: di, view: vi },
                ));
            } else {
                for &bi in &view.bodies {
                    edges.push((
                        HierarchyNode::Body(bi),
                        HierarchyNode::DrawingProjection { drawing: di, view: vi },
                    ));
                }
            }
        }
    }
    // Fuse-merge/cut extrusions consume their host body (#1106/#1107): the prior body is a
    // shadow input and the combined solid nests under the extrusion as output.
    for (bi, body) in doc.bodies.iter() {
        if let Some(ei) = body.source.producing_extrusion() {
            if let Some(host) = crate::model::fuse_host_of(doc, bi) {
                edges.push((HierarchyNode::Body(host), HierarchyNode::Extrusion(ei)));
            }
        }
    }
    edges
}

/// Tree-parent edges the Graph view draws. A Document→element spoke is omitted when that
/// element already has any other input/parent (#1324) — e.g. a fillet fed by the body it
/// treats should not also hang off the document root.
pub fn graph_parent_edges(
    positions: &[GraphNodePosition],
    doc: &Document,
) -> Vec<(HierarchyNode, HierarchyNode)> {
    let has_other_input: HashSet<HierarchyNode> = graph_dependency_edges(doc)
        .into_iter()
        .map(|(_, consumer)| consumer)
        .collect();
    positions
        .iter()
        .filter_map(|p| {
            let parent = p.parent?;
            if parent == HierarchyNode::Document && has_other_input.contains(&p.node) {
                None
            } else {
                Some((parent, p.node))
            }
        })
        .collect()
}

/// Whether `node` is a shadow body or a consumed (shadowed) sketch edge.
fn is_shadow_hierarchy_node(doc: &Document, node: HierarchyNode) -> bool {
    match node {
        HierarchyNode::Body(bi) => doc.bodies.get(bi).is_some_and(|b| b.shadow),
        HierarchyNode::Line(li) => doc.lines.get(li).is_some_and(|l| l.shadow),
        HierarchyNode::Circle(ci) => doc.circles.get(ci).is_some_and(|c| c.shadow),
        _ => false,
    }
}

/// Walk `parent_of` from `node` to the nearest ancestor that is currently on screen.
fn visible_ancestor_of(
    node: HierarchyNode,
    parent_of: &HashMap<HierarchyNode, HierarchyNode>,
    present: &HashSet<HierarchyNode>,
) -> Option<HierarchyNode> {
    let mut current = node;
    let mut seen = HashSet::new();
    while let Some(&parent) = parent_of.get(&current) {
        if !seen.insert(parent) {
            break;
        }
        if present.contains(&parent) {
            return Some(parent);
        }
        current = parent;
    }
    None
}

/// Dashed skip-edges for hidden shadow dependencies (#1425): when a visible node is fed by a
/// shadow element that is not on screen, connect the shadow's visible parent to that consumer
/// — skipping the hidden shadow node. Endpoints are both on-screen [`HierarchyNode`]s.
pub fn graph_shadow_skip_edges(
    doc: &Document,
    present: &HashSet<HierarchyNode>,
) -> Vec<(HierarchyNode, HierarchyNode)> {
    let full = graph_node_positions(&build_hierarchy(doc, None));
    let parent_of: HashMap<HierarchyNode, HierarchyNode> = full
        .iter()
        .filter_map(|p| p.parent.map(|parent| (p.node, parent)))
        .collect();

    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    for (source, consumer) in graph_dependency_edges(doc) {
        if !present.contains(&consumer) || present.contains(&source) {
            continue;
        }
        if !is_shadow_hierarchy_node(doc, source) {
            continue;
        }
        if let Some(parent) = visible_ancestor_of(source, &parent_of, present) {
            if parent != consumer && seen.insert((parent, consumer)) {
                edges.push((parent, consumer));
            }
        }
    }
    edges
}

/// One line of the Graph view (#1670): a single node, with its dot sitting in `lane`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphLaneRow {
    pub node: HierarchyNode,
    pub lane: usize,
}

/// What a line between two rows means (#1670).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphLaneEdgeKind {
    /// The upper node contains or produced the lower one — the plain parent/child trunk.
    Parent,
    /// The upper node is an **input** the lower one is built from (#266/#281): a second
    /// relationship beyond the tree parent, drawn in its own accent.
    Dependency,
    /// A constraint's tie to the geometry it constrains: "related", not parent/child, so it
    /// gets no lane of its own and is drawn as a soft dashed leg.
    Related,
}

impl GraphLaneEdgeKind {
    /// Whether this line means "the upper node feeds the lower one" — everything but a
    /// constraint's sideways tie.
    pub fn is_input(self) -> bool {
        matches!(self, Self::Parent | Self::Dependency)
    }
}

/// A line drawn between two rows of the Graph view (#1670).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphLaneEdge {
    pub from: HierarchyNode,
    pub to: HierarchyNode,
    /// The lane the vertical run of this line occupies — the source's trunk. `Related` ties
    /// take the shortest route between the two dots instead and ignore this.
    pub lane: usize,
    pub kind: GraphLaneEdgeKind,
}

/// The Graph view's layout (#1670): one node per line, top to bottom, with the relationships
/// drawn as mostly-vertical lanes beside them — the way `gitk` draws commits.
#[derive(Clone, Debug, Default)]
pub struct GraphLaneLayout {
    /// Rows top to bottom; a node's row is its index here.
    pub rows: Vec<GraphLaneRow>,
    pub edges: Vec<GraphLaneEdge>,
    /// How many lanes the widest point of the graph needs.
    pub lane_count: usize,
}

impl GraphLaneLayout {
    pub fn row_of(&self, node: HierarchyNode) -> Option<usize> {
        self.rows.iter().position(|r| r.node == node)
    }

    /// The rightmost lane anything is drawn in at each row — the node's own dot, plus every
    /// line passing through that row (#1683). A row's label starts past this, so no line ever
    /// runs across a name.
    pub fn row_line_extents(&self) -> Vec<usize> {
        let row_of: HashMap<HierarchyNode, usize> = self
            .rows
            .iter()
            .enumerate()
            .map(|(row, r)| (r.node, row))
            .collect();
        let lane_of: HashMap<HierarchyNode, usize> =
            self.rows.iter().map(|r| (r.node, r.lane)).collect();
        let mut out: Vec<usize> = self.rows.iter().map(|r| r.lane).collect();
        let bump = |row: usize, lane: usize, out: &mut Vec<usize>| {
            if let Some(slot) = out.get_mut(row) {
                *slot = (*slot).max(lane);
            }
        };
        for edge in &self.edges {
            let (Some(&from), Some(&to)) = (row_of.get(&edge.from), row_of.get(&edge.to)) else {
                continue;
            };
            let (start, end) = (from.min(to), from.max(to));
            let from_lane = lane_of.get(&edge.from).copied().unwrap_or(0);
            let to_lane = lane_of.get(&edge.to).copied().unwrap_or(0);
            if edge.kind.is_input() {
                // Down the trunk, with a leg into each end row.
                bump(from, from_lane.max(edge.lane), &mut out);
                bump(to, to_lane.max(edge.lane), &mut out);
                for row in (start + 1)..end {
                    bump(row, edge.lane, &mut out);
                }
            } else {
                // A constraint's tie runs straight between the two dots, so its lane at each
                // row is the interpolated one (rounded outward).
                let span = (end - start).max(1) as f32;
                let (lo, hi) = (from_lane.min(to_lane), from_lane.max(to_lane));
                for row in start..=end {
                    let t = (row - start) as f32 / span;
                    let lane = from_lane as f32 + (to_lane as f32 - from_lane as f32) * t;
                    bump(row, (lane.ceil() as usize).clamp(lo, hi), &mut out);
                }
            }
        }
        out
    }
}

/// The line/circle/image node a constraint endpoint refers to, if it has one (#1670).
fn constraint_point_node(point: &ConstraintPoint) -> Option<HierarchyNode> {
    match *point {
        ConstraintPoint::LineEndpoint { line, .. } => Some(HierarchyNode::Line(line)),
        ConstraintPoint::CircleCenter(circle) => Some(HierarchyNode::Circle(circle)),
        ConstraintPoint::ImageAnchor { image, .. }
        | ConstraintPoint::ImageCalibrationPoint { image, .. } => Some(HierarchyNode::Image(image)),
        _ => None,
    }
}

fn constraint_line_node(line: &ConstraintLine) -> Option<HierarchyNode> {
    match *line {
        ConstraintLine::Line(li) => Some(HierarchyNode::Line(li)),
        ConstraintLine::ImageEdge { image, .. } => Some(HierarchyNode::Image(image)),
        ConstraintLine::FaceEdge { .. } | ConstraintLine::OriginAxis(_) => None,
    }
}

fn constraint_entity_node(entity: &ConstraintEntity) -> Option<HierarchyNode> {
    match entity {
        ConstraintEntity::Point(point) => constraint_point_node(point),
        ConstraintEntity::Line(line) => constraint_line_node(line),
        ConstraintEntity::Circle(circle) => Some(HierarchyNode::Circle(*circle)),
        ConstraintEntity::Origin => None,
    }
}

/// The nodes a constraint relates (#1670) — the geometry it holds together, in no particular
/// order. A constraint is nobody's child, so this is what it ties to in the Graph view.
pub fn constraint_related_nodes(kind: &ConstraintKind) -> Vec<HierarchyNode> {
    let mut out = Vec::new();
    let mut push = |node: Option<HierarchyNode>| {
        if let Some(node) = node {
            if !out.contains(&node) {
                out.push(node);
            }
        }
    };
    match kind {
        ConstraintKind::Distance { target } => match target {
            DistanceTarget::LineLength(li) => push(Some(HierarchyNode::Line(*li))),
            DistanceTarget::CircleDiameter(ci) => push(Some(HierarchyNode::Circle(*ci))),
            DistanceTarget::LineLineDistance { line_a, line_b, .. } => {
                push(constraint_line_node(line_a));
                push(constraint_line_node(line_b));
            }
            DistanceTarget::PointPointDistance { anchor, mover, .. } => {
                push(constraint_point_node(anchor));
                push(constraint_point_node(mover));
            }
            DistanceTarget::PointLineDistance { point, line, .. } => {
                push(constraint_point_node(point));
                push(constraint_line_node(line));
            }
        },
        ConstraintKind::Parallel { line_a, line_b }
        | ConstraintKind::Perpendicular { line_a, line_b }
        | ConstraintKind::Equal { line_a, line_b }
        | ConstraintKind::Angle { line_a, line_b, .. } => {
            push(constraint_line_node(line_a));
            push(constraint_line_node(line_b));
        }
        ConstraintKind::Coincident { a, b } => {
            push(constraint_entity_node(a));
            push(constraint_entity_node(b));
        }
        ConstraintKind::Midpoint { point, line } => {
            push(constraint_point_node(point));
            push(constraint_line_node(line));
        }
        ConstraintKind::Tangent { a, b } => {
            push(constraint_point_node(a));
            push(constraint_point_node(b));
        }
    }
    out
}

/// Lay the Elements graph out as one node per line (#1670).
///
/// Rows run top to bottom in a depth-first walk of the hierarchy, deferring any node whose
/// inputs haven't been emitted yet — so a node always sits below everything that feeds it.
/// Each node with consumers reserves **one** lane for all of them: its children string down
/// that single vertical run rather than fanning out sideways, and a lane is reused as soon as
/// its last consumer is passed, so a straight history chain never drifts right. When that
/// preferred lane is already carrying another trunk, the node packs **left** into a free
/// column instead of stepping further right (#1764).
pub fn graph_lane_layout(doc: &Document, tree: &[HierarchyEntry]) -> GraphLaneLayout {
    let positions = graph_node_positions(tree);
    if positions.is_empty() {
        return GraphLaneLayout::default();
    }
    let present: HashSet<HierarchyNode> = positions.iter().map(|p| p.node).collect();
    let tree_parent: HashMap<HierarchyNode, HierarchyNode> = positions
        .iter()
        .filter_map(|p| p.parent.map(|parent| (p.node, parent)))
        .collect();

    // Input edges: the tree's parent links (Document spokes already dropped for nodes with a
    // real input, #1324), the dependency inputs, and the hidden-shadow skips. A constraint's
    // sketch parent is deliberately not one — see `constraint_related_nodes`.
    let mut inputs: Vec<(HierarchyNode, HierarchyNode, GraphLaneEdgeKind)> = Vec::new();
    let mut seen = HashSet::new();
    let deps = graph_dependency_edges(doc)
        .into_iter()
        .filter(|(from, to)| present.contains(from) && present.contains(to))
        .chain(graph_shadow_skip_edges(doc, &present))
        .map(|(from, to)| (from, to, GraphLaneEdgeKind::Dependency));
    for (from, to, kind) in graph_parent_edges(&positions, doc)
        .into_iter()
        .map(|(from, to)| (from, to, GraphLaneEdgeKind::Parent))
        .chain(deps)
    {
        if from == to || matches!(to, HierarchyNode::Constraint(_)) {
            continue;
        }
        if seen.insert((from, to)) {
            inputs.push((from, to, kind));
        }
    }

    let order = graph_lane_order(tree, &inputs);
    let row_of: HashMap<HierarchyNode, usize> =
        order.iter().enumerate().map(|(row, node)| (*node, row)).collect();

    let mut consumers: HashMap<HierarchyNode, Vec<HierarchyNode>> = HashMap::new();
    let mut sources: HashMap<HierarchyNode, Vec<HierarchyNode>> = HashMap::new();
    for (from, to, _) in &inputs {
        consumers.entry(*from).or_default().push(*to);
        sources.entry(*to).or_default().push(*from);
    }
    let last_consumer_row: HashMap<HierarchyNode, usize> = consumers
        .iter()
        .filter_map(|(node, list)| {
            list.iter().filter_map(|c| row_of.get(c)).max().map(|row| (*node, *row))
        })
        .collect();

    // The rows of the model's own top-level elements — nothing feeds them and they sit under
    // nothing. The first lane is kept clear across those rows so a top-level element always
    // sits flush left and indentation keeps meaning "sits under something" (#1670). A
    // constraint is not one: it has no input, but it does hang inside its sketch.
    let root_rows: HashSet<usize> = order
        .iter()
        .enumerate()
        .filter(|(_, node)| !sources.contains_key(node) && !tree_parent.contains_key(node))
        .map(|(row, _)| row)
        .collect();
    let crosses_a_root = |from: usize, to: usize| (from..=to).any(|row| root_rows.contains(&row));

    // Lane state: the last row each lane's trunk still runs through.
    let mut busy_until: Vec<Option<usize>> = Vec::new();
    let free_at = |busy: &[Option<usize>], lane: usize, row: usize| {
        busy.get(lane).copied().flatten().is_none_or(|until| until < row)
    };
    // A node can sit on a trunk that *ends* at this row (it is the destination) but not on
    // one that continues through it. Through-traffic uses `free_at` (`until < row`).
    let can_sit_at = |busy: &[Option<usize>], lane: usize, row: usize| {
        busy.get(lane).copied().flatten().is_none_or(|until| until <= row)
    };
    // Prefer `min_lane` when it is free to sit on. If it is blocked, pack left into the
    // leftmost sit-able lane rather than only searching to the right (#1764).
    let first_sit = |busy: &mut Vec<Option<usize>>, row: usize, min_lane: usize| {
        let lane = if can_sit_at(busy, min_lane, row) {
            min_lane
        } else {
            let mut lane = 0;
            while !can_sit_at(busy, lane, row) {
                lane += 1;
            }
            lane
        };
        while busy.len() <= lane {
            busy.push(None);
        }
        lane
    };
    // A trunk lane must be free for the whole run *and*, in the first lane, must not run
    // across a top-level element's row.
    let first_free_trunk = |busy: &mut Vec<Option<usize>>, row: usize, last: usize, min_lane: usize| {
        let mut lane = min_lane;
        loop {
            let usable = free_at(busy, lane, row) && !(lane == 0 && crosses_a_root(row, last));
            if usable {
                break;
            }
            lane += 1;
        }
        while busy.len() <= lane {
            busy.push(None);
        }
        lane
    };

    // `trunk_of` is the lane a node's line runs down; `child_lane_of` is where its consumers'
    // dots sit — the same lane for a single consumer (a chain carries straight on), one to the
    // right for a fan, so the shared trunk runs *beside* the children instead of through their
    // icons (#1683).
    let mut trunk_of: HashMap<HierarchyNode, usize> = HashMap::new();
    // node -> (lane its consumers' dots sit in, whether that lane is a fan's own reservation
    // rather than the trunk itself — a fan's children have to check the lane is free).
    let mut child_lane_of: HashMap<HierarchyNode, (usize, bool)> = HashMap::new();
    let mut rows = Vec::with_capacity(order.len());
    for (row, node) in order.iter().enumerate() {
        // A node sits where its **nearest** input puts its consumers — the input just above it,
        // whose line is shortest. Riding a farther input's lane instead would make the near
        // line detour out of its lane and back, crossing whatever runs between (#1684). Ties
        // (two inputs on the same row) go to the leftmost lane. A node with no input at all (a
        // top-level element, a constraint) takes the leftmost free lane at or right of where
        // its tree parent puts its children. If that preferred lane is already carrying
        // through-traffic, pack left into a free column instead of drifting right (#1764).
        let nearest_input = sources.get(node).and_then(|list| {
            list.iter()
                .filter_map(|src| Some((*row_of.get(src)?, *child_lane_of.get(src)?)))
                .max_by_key(|(src_row, (lane, _))| (*src_row, std::cmp::Reverse(*lane)))
                .map(|(_, child_lane)| child_lane)
        });
        let lane = match nearest_input {
            // A chain arrives on its input's own trunk, which is by definition this node's.
            Some((lane, false)) => lane,
            Some((lane, true)) => first_sit(&mut busy_until, row, lane),
            None => {
                let min_lane = tree_parent
                    .get(node)
                    .and_then(|parent| child_lane_of.get(parent).map(|(lane, _)| *lane))
                    .unwrap_or(0);
                first_sit(&mut busy_until, row, min_lane)
            }
        };
        while busy_until.len() <= lane {
            busy_until.push(None);
        }
        rows.push(GraphLaneRow { node: *node, lane });

        // Reserve this node's one trunk for every consumer below — its own lane when that is
        // free from here down, otherwise the leftmost free lane right of it.
        if let Some(&last) = last_consumer_row.get(node) {
            let reuse_own_lane = free_at(&busy_until, lane, row + 1)
                && !(lane == 0 && crosses_a_root(row + 1, last));
            let trunk = if reuse_own_lane {
                lane
            } else {
                first_free_trunk(&mut busy_until, row + 1, last, lane)
            };
            let until = busy_until[trunk].map_or(last, |prev| prev.max(last));
            busy_until[trunk] = Some(until);
            trunk_of.insert(*node, trunk);
            let fan = consumers.get(node).is_some_and(|list| list.len() > 1);
            child_lane_of.insert(*node, if fan { (trunk + 1, true) } else { (trunk, false) });
        }
    }

    let lane_of: HashMap<HierarchyNode, usize> =
        rows.iter().map(|r| (r.node, r.lane)).collect();
    let mut edges: Vec<GraphLaneEdge> = inputs
        .iter()
        .map(|(from, to, kind)| GraphLaneEdge {
            from: *from,
            to: *to,
            lane: trunk_of.get(from).copied().unwrap_or(0),
            kind: *kind,
        })
        .collect();
    // Constraints tie sideways to what they constrain.
    for row in &rows {
        let HierarchyNode::Constraint(ci) = row.node else {
            continue;
        };
        let Some(constraint) = doc.constraints.get(ci) else {
            continue;
        };
        for related in constraint_related_nodes(&constraint.kind) {
            if lane_of.contains_key(&related) {
                edges.push(GraphLaneEdge {
                    from: related,
                    to: row.node,
                    lane: row.lane,
                    kind: GraphLaneEdgeKind::Related,
                });
            }
        }
    }

    let lane_count = rows.iter().map(|r| r.lane + 1).max().unwrap_or(0).max(busy_until.len());
    GraphLaneLayout { rows, edges, lane_count }
}

/// Row order for [`graph_lane_layout`]: a depth-first walk of the hierarchy that holds a node
/// back until every input it depends on has been emitted. Depth-first keeps a node's children
/// contiguous (so one lane serves them all); the input gate keeps consumers below their
/// inputs even when those live in another branch.
fn graph_lane_order(
    tree: &[HierarchyEntry],
    inputs: &[(HierarchyNode, HierarchyNode, GraphLaneEdgeKind)],
) -> Vec<HierarchyNode> {
    fn walk(entry: &HierarchyEntry, out: &mut Vec<HierarchyNode>) {
        out.push(entry.node);
        for child in &entry.children {
            walk(child, out);
        }
    }
    let mut preorder = Vec::new();
    for entry in tree {
        walk(entry, &mut preorder);
    }

    let mut sources: HashMap<HierarchyNode, Vec<HierarchyNode>> = HashMap::new();
    for (from, to, _) in inputs {
        sources.entry(*to).or_default().push(*from);
    }

    let mut emitted: HashSet<HierarchyNode> = HashSet::new();
    let mut order: Vec<HierarchyNode> = Vec::with_capacity(preorder.len());
    loop {
        let before = order.len();
        for node in &preorder {
            if emitted.contains(node) {
                continue;
            }
            let ready = sources
                .get(node)
                .is_none_or(|list| list.iter().all(|from| emitted.contains(from)));
            if ready {
                emitted.insert(*node);
                order.push(*node);
            }
        }
        if order.len() == before || order.len() == preorder.len() {
            break;
        }
    }
    // Defensive: a dependency cycle would strand nodes — emit them in tree order rather
    // than dropping them from the view.
    for node in preorder {
        if emitted.insert(node) {
            order.push(node);
        }
    }
    order
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

/// The element whose shared Elements-pane context menu should open for a viewport
/// right-click on already-selected geometry (#1224).
///
/// - The exact pick, when it is selected and has a hierarchy row.
/// - Otherwise a body (or extrusion) sub-pick opens the owner's menu when that owner
///   is selected — so right-clicking any face/edge/vertex of a selected body shows the
///   same menu as the body's Elements-pane row.
pub fn selected_context_menu_element(
    picked: &SceneElement,
    selection: &SceneSelection,
) -> Option<SceneElement> {
    if selection.is_selected(picked.clone()) && hierarchy_node_for_element(picked).is_some() {
        return Some(picked.clone());
    }
    if let Some(owner) = visibility_target_for_element(picked) {
        if owner != *picked
            && selection.is_selected(owner.clone())
            && hierarchy_node_for_element(&owner).is_some()
        {
            return Some(owner);
        }
    }
    None
}

/// The [`HierarchyNode`] for a [`SceneElement`] — the inverse of [`scene_element_for_node`]
/// for the kinds that appear in the element graph (#524/#531). `None` for sub-element
/// selections (points, edges, vertices) that aren't graph nodes.
pub fn hierarchy_node_for_element(element: &SceneElement) -> Option<HierarchyNode> {
    Some(match element {
        SceneElement::ConstructionPlane(i) => HierarchyNode::ConstructionPlane(*i),
        SceneElement::CrossSection(i) => HierarchyNode::CrossSection(*i),
        SceneElement::SectionPlane { view, cut } => {
            HierarchyNode::SectionPlane { view: *view, cut: *cut }
        }
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
        SceneElement::ShellOp(i) => HierarchyNode::ShellOp(*i),
        SceneElement::EdgeTreatmentOp(i) => HierarchyNode::EdgeTreatmentOp(*i),
        SceneElement::Revolution(i) => HierarchyNode::Revolution(*i),
        SceneElement::Shape(i) => HierarchyNode::Shape(*i),
        SceneElement::SweepOp(i) => HierarchyNode::SweepOp(*i),
        SceneElement::Loft(i) => HierarchyNode::Loft(*i),
        SceneElement::Drawing(i) => HierarchyNode::Drawing(*i),
        SceneElement::Component(i) => HierarchyNode::Component(*i),
        SceneElement::UnitInstance(i) => HierarchyNode::UnitInstance(*i),
        SceneElement::Joint(i) => HierarchyNode::Joint(*i),
        SceneElement::Point(_)
        | SceneElement::FaceEdge(_)
        | SceneElement::Origin
        | SceneElement::GlobalAxis(_)
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::ProjectedEdge { .. }
        | SceneElement::ProjectedCorner { .. }
        | SceneElement::BodyFace { .. }
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_)
        | SceneElement::ExtrusionEdge { .. }
        | SceneElement::PrimitiveEdge { .. }
        | SceneElement::RepeatedFace { .. }
        // A drawing's items have rows under their page, keyed by the page rather than by a
        // graph node of their own (#967).
        | SceneElement::DrawingElement { .. } => return None,
    })
}

/// The element whose visibility toggle a selection item should drive (#1152).
///
/// Hierarchy rows (bodies, sketches, lines, …) hide themselves. Body mesh parts
/// (faces/edges/vertices/axes) hide their body; extrusion analytic edges hide the extrusion.
/// Transient picks (origin, axes, points, drawing page items) have no hide target.
pub fn visibility_target_for_element(element: &SceneElement) -> Option<SceneElement> {
    match element {
        SceneElement::BodyEdge { body, .. }
        | SceneElement::BodyVertex { body, .. }
        | SceneElement::BodyFace { body, .. }
        | SceneElement::BodyCylinder { body, .. }
        | SceneElement::BodyAxis { body, .. } => Some(SceneElement::Body(*body)),
        SceneElement::ProjectedEdge { body: Some(body), .. }
        | SceneElement::ProjectedCorner { body: Some(body), .. } => Some(SceneElement::Body(*body)),
        SceneElement::ProjectedEdge { .. } | SceneElement::ProjectedCorner { .. } => None,
        SceneElement::ExtrusionEdge { extrusion, .. } => Some(SceneElement::Extrusion(*extrusion)),
        SceneElement::PrimitiveEdge { primitive, .. } => Some(SceneElement::Shape(*primitive)),
        SceneElement::FaceEdge(line) => match line {
            ConstraintLine::FaceEdge { face, .. } => face_owner_element(face),
            ConstraintLine::Line(i) => Some(SceneElement::Line(*i)),
            ConstraintLine::OriginAxis(_) => None,
            ConstraintLine::ImageEdge { image, .. } => Some(SceneElement::Image(*image)),
        },
        SceneElement::Point(_)
        | SceneElement::Origin
        | SceneElement::GlobalAxis(_)
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_)
        | SceneElement::RepeatedFace { .. }
        | SceneElement::DrawingElement { .. } => None,
        other if hierarchy_node_for_element(other).is_some() => Some(other.clone()),
        _ => None,
    }
}

/// Unique visibility targets for the current selection (#1152), ordered stably.
pub fn visibility_targets_from_selection(selection: &SceneSelection) -> Vec<SceneElement> {
    let mut targets: Vec<SceneElement> = selection
        .iter()
        .filter_map(|e| visibility_target_for_element(&e))
        .collect();
    targets.sort_by_key(|e| visibility_sort_key(e));
    targets.dedup();
    targets
}

fn visibility_sort_key(element: &SceneElement) -> (u8, u64) {
    match element {
        SceneElement::ConstructionPlane(i) => (0, i.index() as u64),
        SceneElement::Sketch(i) => (1, i.index() as u64),
        SceneElement::Line(i) => (2, i.index() as u64),
        SceneElement::Circle(i) => (3, i.index() as u64),
        SceneElement::Constraint(i) => (4, i.index() as u64),
        SceneElement::Extrusion(i) => (5, i.index() as u64),
        SceneElement::Body(i) => (6, i.index() as u64),
        SceneElement::Image(i) => (7, i.index() as u64),
        SceneElement::BooleanOp(i) => (8, i.index() as u64),
        SceneElement::MoveOp(i) => (9, i.index() as u64),
        SceneElement::MirrorOp(i) => (10, i.index() as u64),
        SceneElement::RepeatOp(i) => (11, i.index() as u64),
        SceneElement::SketchRepeatOp(i) => (12, i.index() as u64),
        SceneElement::SketchOffsetOp(i) => (13, i.index() as u64),
        SceneElement::SketchMirrorOp(i) => (14, i.index() as u64),
        SceneElement::SketchVertexTreatmentOp(i) => (15, i.index() as u64),
        SceneElement::SketchSliceOp(i) => (16, i.index() as u64),
        SceneElement::SketchText(i) => (17, i.index() as u64),
        SceneElement::SliceOp(i) => (18, i.index() as u64),
        SceneElement::ShellOp(i) => (18, i.index() as u64 + 100_000),  // distinct sort key
        SceneElement::EdgeTreatmentOp(i) => (19, i.index() as u64),
        SceneElement::Revolution(i) => (20, i.index() as u64),
        SceneElement::Shape(i) => (21, i.index() as u64),
        SceneElement::SweepOp(i) => (22, i.index() as u64),
        SceneElement::Loft(i) => (22, i.index() as u64 + 100_000),
        SceneElement::Drawing(i) => (22, i.index() as u64 + 200_000),
        SceneElement::Component(i) => (23, i.index() as u64),
        SceneElement::UnitInstance(i) => (24, i.index() as u64),
        SceneElement::Joint(i) => (25, i.index() as u64),
        _ => (255, 0),
    }
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
    for (i, plane) in doc.construction_planes.iter() {
        if !matches!(plane.parent, ConstructionPlaneParent::Root) {
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
        for (ii, image) in doc.tracing_images.iter() {
            if image.plane == i {
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
    for (i, extrusion) in doc.extrusions.iter() {
        if sketch_alive(doc, extrusion.sketch) {
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
    // Bodies produced by an extrusion (including cuts-only Solids, #1106) nest under that
    // extrusion via build_sketch_extrusions — skip them here even when `add` is empty.
    for (bi, body) in doc.bodies.iter() {
        if body.source.producing_extrusion().is_some() {
            continue;
        }
        if body.source.extrusion_indices().is_empty()
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
                    // A shape's pure primitive body nests under its Shape node (#909).
                    | crate::model::BodySource::Primitive(_)
                    // A unit's materialized body has no row of its own (#724): the
                    // instance row (#723) stands for it.
                    | crate::model::BodySource::UnitInstance(_)
            )
        {
            push_body_and_mesh_sketches(&mut roots, doc, bi, sketch_session);
        }
    }
    // Lofts (#252): the loft is an operation node with its output body nested beneath it (its
    // cross-section sketches feed it as graph inputs, see `graph_dependency_edges`). Previously
    // the loft body surfaced as a bare top-level element with no sign of what produced it.
    for (li, _loft) in doc.lofts.iter() {
        let mut children = Vec::new();
        for (bi, b) in doc.bodies.iter() {
            if matches!(b.source, crate::model::BodySource::Loft(l) if l == li) {
                push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
            }
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::Loft(li),
            children,
        });
    }
    // Boolean operations (Combine tool): the operation is an element of its own, with its
    // output bodies nested beneath it — outputs depend on the operation, the operation on
    // its (shadow) inputs.
    for (oi, op) in doc.boolean_ops.iter() {
        let mut children = Vec::new();
        for &bi in &op.outputs {
            push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::BooleanOp(oi),
            children,
        });
    }
    for (oi, op) in doc.move_ops.iter() {
        let mut children = Vec::new();
        for &bi in &op.outputs {
            push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::MoveOp(oi),
            children,
        });
    }
    for (oi, op) in doc.mirror_ops.iter() {
        let mut children = Vec::new();
        for &bi in &op.outputs {
            push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::MirrorOp(oi),
            children,
        });
    }
    for (oi, op) in doc.repeat_ops.iter() {
        let mut children = Vec::new();
        for &bi in &op.outputs {
            push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
        }
        // Generated construction-plane instances (#221) nest under the op too.
        children.extend(
            op.plane_outputs
                .iter()
                .filter(|&&pi| doc.construction_planes.contains(pi))
                .map(|&pi| HierarchyEntry {
                    node: HierarchyNode::ConstructionPlane(pi),
                    children: Vec::new(),
                }),
        );
        // Repeated-sketch host planes (#226/#231) nest under the op, each with its copy sketch.
        children.extend(
            op.sketch_plane_outputs
                .iter()
                .filter(|&&pi| doc.construction_planes.contains(pi))
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
    for (oi, op) in doc.sketch_repeat_ops.iter() {
        let mut children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .filter(|&&li| doc.lines.contains(li))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        children.extend(
            op.circle_outputs
                .iter()
                .filter(|&&ci| doc.circles.contains(ci))
                .map(|&ci| HierarchyEntry { node: HierarchyNode::Circle(ci), children: Vec::new() }),
        );
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchRepeatOp(oi),
            children,
        });
    }
    // 2D in-sketch offsets nest under the sketch they offset (#941, see `build_sketch_entry`);
    // only an offset whose sketch died falls back to a top-level orphan here so it stays
    // reachable.
    for (oi, op) in doc.sketch_offset_ops.iter() {
        if crate::document_lifecycle::sketch_alive(doc, op.sketch) {
            continue;
        }
        roots.push(build_sketch_offset_entry(doc, oi));
    }
    // 2D in-sketch mirrors nest under the sketch they reflect (#1540, see
    // `build_sketch_entry`); only a mirror whose sketch died falls back to a top-level
    // orphan here so it stays reachable.
    for (oi, op) in doc.sketch_mirror_ops.iter() {
        if crate::document_lifecycle::sketch_alive(doc, op.sketch) {
            continue;
        }
        roots.push(build_sketch_mirror_entry(doc, oi));
    }
    // 2D in-sketch chamfer/fillet (#538): the op with its trimmed copies + bridge lines nested
    // beneath it (the shadowed source edges stay listed under the sketch, dimmed).
    for (oi, op) in doc.sketch_vertex_treatment_ops.iter() {
        let children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .chain(op.bridge_outputs.iter())
            .filter(|&&li| doc.lines.contains(li))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchVertexTreatmentOp(oi),
            children,
        });
    }
    // 2D in-sketch slices (#224/#229): the op is its own element with its fragment lines nested
    // beneath it (the shadowed originals stay listed under the sketch, dimmed).
    for (oi, op) in doc.sketch_slice_ops.iter() {
        let children: Vec<HierarchyEntry> = op
            .line_outputs
            .iter()
            .filter(|&&li| doc.lines.contains(li))
            .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::SketchSliceOp(oi),
            children,
        });
    }
    // Slice operations (Slice tool): the operation is its own element, with its fragment
    // bodies nested beneath it.
    for (oi, op) in doc.slice_ops.iter() {
        let mut children = Vec::new();
        for &bi in &op.outputs {
            push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::SliceOp(oi),
            children,
        });
    }
    // Shell operations (#1156): hollowed output bodies nest under the op.
    for (oi, op) in doc.shell_ops.iter() {
        let mut children = Vec::new();
        for &bi in &op.outputs {
            push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::ShellOp(oi),
            children,
        });
    }
    // Edge chamfer/fillet operations (#531): the operation is its own element, with its beveled
    // output bodies nested beneath it (the shadowed input bodies stay listed, dimmed).
    for (oi, op) in doc.edge_treatment_ops.iter() {
        let mut children = Vec::new();
        for &bi in &op.outputs {
            push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::EdgeTreatmentOp(oi),
            children,
        });
    }
    // Revolved solids (Revolve tool, #211): the operation is its own element, with its output
    // body (linked by `BodySource::Revolve`) nested beneath it.
    for (oi, _rev) in doc.revolutions.iter() {
        let mut children = Vec::new();
        for (bi, b) in doc.bodies.iter() {
            if b.source == crate::model::BodySource::Revolve(oi) {
                push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
            }
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::Revolution(oi),
            children,
        });
    }
    // Primitive shapes (#909): the shape is its own top-level element with its **pure**
    // primitive body beneath it. After a fuse-merge/cut (#1106) that pure body is a shadow
    // and the combined solid nests under the extrusion that produced it — not under the
    // Shape. Sketches on the shape's faces (#1103/#1105) nest here too.
    for (oi, shape) in doc.primitives.iter() {
        let mut children = Vec::new();
        for (bi, b) in doc.bodies.iter() {
            if matches!(b.source, crate::model::BodySource::Primitive(p) if p == oi) {
                push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
            }
        }
        for face in crate::primitives::flat_faces(shape) {
            children.extend(build_face_sketches(
                doc,
                crate::model::FaceId::PrimitiveFace {
                    primitive: oi,
                    face,
                },
                sketch_session,
            ));
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::Shape(oi),
            children,
        });
    }
    // Sweeps nest under their profile sketch (#478, see `build_sketch_entry`); only a
    // sweep whose sketch died falls back to a top-level orphan here so it stays reachable.
    for (oi, fp) in doc.sweeps.iter() {
        if crate::document_lifecycle::sketch_alive(doc, fp.sketch) {
            continue;
        }
        let mut children = Vec::new();
        for (bi, b) in doc.bodies.iter() {
            if b.source == crate::model::BodySource::Sweep(oi) {
                push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
            }
        }
        roots.push(HierarchyEntry {
            node: HierarchyNode::SweepOp(oi),
            children,
        });
    }
    // Technical drawings (#180): top-level leaves (they reference bodies but aren't part of
    // the geometry DAG), each right-clickable to open its drawing pane.
    for (di, drawing) in doc.drawings.iter() {
        {
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
            for ai in drawing.annotations.keys() {
                children.push(HierarchyEntry {
                    node: HierarchyNode::DrawingAnnotation { drawing: di, annotation: ai },
                    children: Vec::new(),
                });
            }
            roots.push(HierarchyEntry {
                node: HierarchyNode::Drawing(di),
                children,
            });
        }
    }
    // Imported unit instances (#723): one top-level row each; the contents expand as
    // read-only leaves beneath it.
    for index in doc.unit_instances.keys().collect::<Vec<_>>() {
        let children = (0..unit_child_rows(doc, index).len())
            .map(|ordinal| HierarchyEntry {
                node: HierarchyNode::UnitChild { instance: index, ordinal },
                children: Vec::new(),
            })
            .collect();
        roots.push(HierarchyEntry {
            node: HierarchyNode::UnitInstance(index),
            children,
        });
    }
    // Joints (#891): childless top-level rows — their members feed them as graph inputs,
    // nothing nests beneath them.
    for (ji, _joint) in doc.joints.iter() {
        {
            roots.push(HierarchyEntry {
                node: HierarchyNode::Joint(ji),
                children: Vec::new(),
            });
        }
    }
    // Sketches on body mesh faces that no producing feature claimed (#1465) still
    // surface at the top level so they never disappear from the pane.
    {
        fn collect_sketches(entries: &[HierarchyEntry], out: &mut HashSet<SketchId>) {
            for e in entries {
                if let HierarchyNode::Sketch(s) = e.node {
                    out.insert(s);
                }
                collect_sketches(&e.children, out);
            }
        }
        let mut seen = HashSet::new();
        collect_sketches(&roots, &mut seen);
        for (si, _) in doc.sketches.iter() {
            if !seen.contains(&si) && sketch_alive(doc, si) {
                let entry = build_sketch_entry(doc, si, sketch_session);
                collect_sketches(std::slice::from_ref(&entry), &mut seen);
                roots.push(entry);
            }
        }
    }
    // Components (#423): move member roots under their component's entry, then nest
    // component entries by their parent links. Unassigned roots stay at the top level.
    let mut roots = group_roots_into_components(doc, roots);
    // Drawings belong at the bottom of the document under a collapsible section (#1205),
    // not interleaved with bodies/ops. Component-filed drawings stay under their component
    // (group_roots_into_components already moved them); only unassigned ones land here.
    let mut drawing_entries = Vec::new();
    let mut i = 0;
    while i < roots.len() {
        if matches!(roots[i].node, HierarchyNode::Drawing(_)) {
            drawing_entries.push(roots.remove(i));
        } else {
            i += 1;
        }
    }
    if !drawing_entries.is_empty() {
        // Stable kind-then-index order among drawings (same Ord as the flat list's tiebreak).
        drawing_entries.sort_by_key(|e| e.node);
        roots.push(HierarchyEntry {
            node: HierarchyNode::Drawings,
            children: drawing_entries,
        });
    }
    // Cross-section views get the same treatment under a Views section (#1671): ways of
    // looking at the model, grouped at the bottom rather than mixed in with it.
    if !doc.cross_sections.is_empty() {
        roots.push(HierarchyEntry {
            node: HierarchyNode::Views,
            children: doc
                .cross_sections
                .keys()
                .map(|key| HierarchyEntry {
                    node: HierarchyNode::CrossSection(key),
                    children: doc.cross_sections[key]
                        .cuts
                        .iter()
                        .enumerate()
                        .map(|(cut, _)| HierarchyEntry {
                            node: HierarchyNode::SectionPlane { view: key, cut },
                            children: Vec::new(),
                        })
                        .collect(),
                })
                .collect(),
        });
    }
    vec![HierarchyEntry {
        node: HierarchyNode::Document,
        children: roots,
    }]
}

/// Group top-level entries into their components' entries (#423). Components render even
/// when empty; a component whose parent chain is broken surfaces at the top level.
fn group_roots_into_components(doc: &Document, roots: Vec<HierarchyEntry>) -> Vec<HierarchyEntry> {
    if doc.components.is_empty() {
        return roots;
    }
    let member_of = |node: &HierarchyNode| -> Option<crate::model::ComponentKey> {
        doc.component_of(component_member_for_node(node)?)
    };
    // component -> its (initially childless) entry.
    let mut comp_children: HashMap<crate::model::ComponentKey, Vec<HierarchyEntry>> =
        HashMap::new();
    for ci in doc.components.keys() {
        comp_children.insert(ci, Vec::new());
    }
    // Extract assigned entries wherever they sit (#423): an assigned element that nests
    // inside another entry's subtree (an extrusion under its sketch's plane, a body under
    // an op) moves — with its own subtree — into the component's entry.
    fn extract_members(
        entries: &mut Vec<HierarchyEntry>,
        member_of: &impl Fn(&HierarchyNode) -> Option<crate::model::ComponentKey>,
        comp_children: &mut HashMap<crate::model::ComponentKey, Vec<HierarchyEntry>>,
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
    let keys: Vec<crate::model::ComponentKey> = comp_children.keys().copied().collect();
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
    let mut order: Vec<crate::model::ComponentKey> = comp_children.keys().copied().collect();
    order.sort_unstable();
    // Depth of each component (root = 0), cycles cut by component_chain.
    let depth = |c: crate::model::ComponentKey| doc.component_chain(c).len();
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
    /// In-sketch geometry: lines, circles, constraints, edge treatments, and sketch mirrors.
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
    /// What's **inside** an imported unit (#723). Off by default: the node graph shows an
    /// instance as one opaque row; turning this on lets its contents into the graph. The
    /// List view ignores it — there the instance row expands instead.
    pub unit_contents: bool,
    /// Cross-section **views** (#1671) and the section that groups them.
    pub views: bool,
    /// **Shadow bodies** — bodies consumed (shadowed) by an operation (#1109). Off by
    /// default: a shadow body is a kept-for-editing input, not a live result, so the pane
    /// hides it unless the user turns this on. Bodies are always leaves in the hierarchy
    /// tree, so hiding one never strands a child.
    pub shadow_bodies: bool,
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
            unit_contents: false,
            shadow_bodies: false,
            views: true,
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
            unit_contents: false,
            shadow_bodies: false,
            views: true,
        }
    }

    /// The toggles in display order: `(label, &mut enabled)` pairs the filter UI iterates.
    pub fn rows(&mut self) -> [(&'static str, &mut bool); 11] {
        [
            ("Planes", &mut self.planes),
            ("Sketches", &mut self.sketches),
            ("Sketch geometry", &mut self.sketch_geometry),
            ("Bodies", &mut self.bodies),
            ("Shadow bodies", &mut self.shadow_bodies),
            ("Operations", &mut self.operations),
            ("Images", &mut self.images),
            ("Drawings", &mut self.drawings),
            ("Drawing components", &mut self.drawing_components),
            ("Unit contents", &mut self.unit_contents),
            ("Views", &mut self.views),
        ]
    }

    /// Whether a node's type is currently shown. The synthetic Document root is always shown.
    fn shows(&self, node: HierarchyNode) -> bool {
        match node {
            HierarchyNode::Document => true,
            // The Drawings section exists only when drawings do (#1205); hide it with them.
            HierarchyNode::Drawings => self.drawings,
            // Same for the Views section and the cross-section views inside it (#1671).
            HierarchyNode::Views
            | HierarchyNode::CrossSection(_)
            | HierarchyNode::SectionPlane { .. } => self.views,
            HierarchyNode::Component(_) => true,
            HierarchyNode::ConstructionPlane(_) => self.planes,
            HierarchyNode::Sketch(_) => self.sketches,
            HierarchyNode::Line(_)
            | HierarchyNode::Circle(_)
            | HierarchyNode::Constraint(_)
            | HierarchyNode::SketchText(_)
            | HierarchyNode::EdgeTreatment { .. }
            | HierarchyNode::SketchMirrorOp(_) => self.sketch_geometry,
            HierarchyNode::Body(_) => self.bodies,
            HierarchyNode::Extrusion(_)
            | HierarchyNode::Shape(_)
            | HierarchyNode::BooleanOp(_)
            | HierarchyNode::MoveOp(_)
            | HierarchyNode::MirrorOp(_)
            | HierarchyNode::RepeatOp(_)
            | HierarchyNode::SketchRepeatOp(_)
            | HierarchyNode::SketchOffsetOp(_)
            | HierarchyNode::SketchVertexTreatmentOp(_)
            | HierarchyNode::SketchSliceOp(_)
            | HierarchyNode::SliceOp(_)
            | HierarchyNode::ShellOp(_)
            | HierarchyNode::EdgeTreatmentOp(_)
            | HierarchyNode::Revolution(_)
            | HierarchyNode::SweepOp(_)
            | HierarchyNode::Joint(_)
            | HierarchyNode::Loft(_) => self.operations,
            HierarchyNode::Image(_) => self.images,
            HierarchyNode::Drawing(_) => self.drawings,
            HierarchyNode::DrawingProjection { .. }
            | HierarchyNode::DrawingAnnotation { .. }
            | HierarchyNode::DrawingDimension { .. }
            | HierarchyNode::DrawingPointDim { .. } => {
                self.drawings && self.drawing_components
            }
            HierarchyNode::UnitInstance(_) => true,
            // The List view keeps a unit's children (they hide behind the row's collapse
            // instead); the Graph consults `unit_contents` separately (#723).
            HierarchyNode::UnitChild { .. } => true,
        }
    }
}

/// Drop every [`HierarchyNode::UnitChild`] from a tree (#723): the graph's default view
/// of a unit instance is one opaque node. Unlike [`filter_hierarchy`] there is no
/// promotion — a unit's contents never surface as loose nodes.
pub fn prune_unit_children(tree: &mut Vec<HierarchyEntry>) {
    tree.retain(|e| !matches!(e.node, HierarchyNode::UnitChild { .. }));
    for entry in tree {
        prune_unit_children(&mut entry.children);
    }
}

/// Drop every [`HierarchyNode::Body`] whose body is a shadow (#1109) — a consumed input
/// kept for editing, not a live result. Bodies are always leaves in the hierarchy tree
/// (`build_hierarchy` gives them no children; mesh-face sketches sit as siblings, #1465),
/// so dropping one never strands a child. The prune is recursive so a shadow nested under
/// any node (a Shape, an operation, a unit instance's read-only contents) is removed
/// wherever it sits.
pub fn prune_shadow_bodies(tree: &mut Vec<HierarchyEntry>, doc: &Document) {
    tree.retain(|e| {
        !matches!(
            e.node,
            HierarchyNode::Body(bi) if doc.bodies.get(bi).is_some_and(|b| b.shadow)
        )
    });
    for entry in tree {
        prune_shadow_bodies(&mut entry.children, doc);
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

/// The hierarchy the Graph view lays out: filtered, then pruned of the three things the graph
/// shows differently from the List. A unit instance is one opaque node there (#723) — its
/// contents stay out unless the "Unit contents" toggle lets them in, where the List instead
/// hides them behind the row's collapse — and shadow bodies follow their own toggle (#1109).
/// Shared with the script API so `bearcad.ui.elements_graph()` reads the same rows the pane
/// draws (#1670).
pub fn graph_view_tree(
    doc: &Document,
    sketch_session: Option<SketchSession>,
    filter: &ElementFilter,
) -> Vec<HierarchyEntry> {
    let mut tree = filter_hierarchy(&build_hierarchy(doc, sketch_session), filter);
    if !filter.unit_contents {
        prune_unit_children(&mut tree);
    }
    if !filter.shadow_bodies {
        prune_shadow_bodies(&mut tree, doc);
    }
    // The synthetic Document root is a List-view affordance — a place to drop things on and
    // a handle for the whole model. The graph shows relationships, and "everything hangs off
    // the document" is not one, so the model's own top-level elements are the roots here
    // (#1682).
    tree = tree
        .into_iter()
        .flat_map(|entry| {
            if entry.node == HierarchyNode::Document {
                entry.children
            } else {
                vec![entry]
            }
        })
        .collect();
    tree
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

/// The [`SceneElement`] a component member points at (#423/#1525).
pub fn component_member_element(member: crate::model::ComponentMember) -> Option<SceneElement> {
    use crate::model::ComponentMember as CM;
    Some(match member {
        CM::ConstructionPlane(i) => SceneElement::ConstructionPlane(i),
        CM::Extrusion(i) => SceneElement::Extrusion(i),
        CM::Body(i) => SceneElement::Body(i),
        CM::BooleanOp(i) => SceneElement::BooleanOp(i),
        CM::MoveOp(i) => SceneElement::MoveOp(i),
        CM::MirrorOp(i) => SceneElement::MirrorOp(i),
        CM::RepeatOp(i) => SceneElement::RepeatOp(i),
        CM::SliceOp(i) => SceneElement::SliceOp(i),
        CM::ShellOp(i) => SceneElement::ShellOp(i),
        CM::EdgeTreatmentOp(i) => SceneElement::EdgeTreatmentOp(i),
        CM::Revolution(i) => SceneElement::Revolution(i),
        CM::Sweep(i) => SceneElement::SweepOp(i),
        CM::Loft(i) => SceneElement::Loft(i),
        CM::Drawing(i) => SceneElement::Drawing(i),
    })
}

/// The component member a scene element stands for (#423) — the inverse of
/// [`component_member_element`], and what turns "move this into that component" into a
/// membership entry. `None` for anything that is not a top-level member: a shape (#909),
/// a nested element, or derived geometry.
pub fn component_member_for_element(
    element: &SceneElement,
) -> Option<crate::model::ComponentMember> {
    use crate::model::ComponentMember as CM;
    Some(match element {
        SceneElement::ConstructionPlane(i) => CM::ConstructionPlane(*i),
        SceneElement::Extrusion(i) => CM::Extrusion(*i),
        SceneElement::Body(i) => CM::Body(*i),
        SceneElement::BooleanOp(i) => CM::BooleanOp(*i),
        SceneElement::MoveOp(i) => CM::MoveOp(*i),
        SceneElement::MirrorOp(i) => CM::MirrorOp(*i),
        SceneElement::RepeatOp(i) => CM::RepeatOp(*i),
        SceneElement::SliceOp(i) => CM::SliceOp(*i),
        SceneElement::ShellOp(i) => CM::ShellOp(*i),
        SceneElement::EdgeTreatmentOp(i) => CM::EdgeTreatmentOp(*i),
        SceneElement::Revolution(i) => CM::Revolution(*i),
        SceneElement::SweepOp(i) => CM::Sweep(*i),
        SceneElement::Loft(i) => CM::Loft(*i),
        SceneElement::Drawing(i) => CM::Drawing(*i),
        _ => return None,
    })
}

/// The component member an Elements-pane row stands for (#423). Wider than
/// [`component_member_for_element`]: drawings are rows a user can file into a
/// component even though they have no scene element.
pub fn component_member_for_node(node: &HierarchyNode) -> Option<crate::model::ComponentMember> {
    use crate::model::ComponentMember as CM;
    Some(match node {
        HierarchyNode::ConstructionPlane(i) => CM::ConstructionPlane(*i),
        HierarchyNode::Extrusion(i) => CM::Extrusion(*i),
        HierarchyNode::Body(i) => CM::Body(*i),
        HierarchyNode::Loft(k) => CM::Loft(*k),
        HierarchyNode::BooleanOp(i) => CM::BooleanOp(*i),
        HierarchyNode::MoveOp(i) => CM::MoveOp(*i),
        HierarchyNode::MirrorOp(i) => CM::MirrorOp(*i),
        HierarchyNode::RepeatOp(i) => CM::RepeatOp(*i),
        HierarchyNode::SliceOp(i) => CM::SliceOp(*i),
        HierarchyNode::ShellOp(i) => CM::ShellOp(*i),
        HierarchyNode::EdgeTreatmentOp(i) => CM::EdgeTreatmentOp(*i),
        HierarchyNode::Revolution(i) => CM::Revolution(*i),
        HierarchyNode::SweepOp(i) => CM::Sweep(*i),
        HierarchyNode::Drawing(i) => CM::Drawing(*i),
        _ => return None,
    })
}

/// The component a scene element belongs to (#423): a direct membership for top-level
/// kinds, or the membership of the root it nests under (a body via its producing
/// operation/extrusion, an extrusion or image via its sketch's host plane).
pub fn owning_component(
    doc: &Document,
    element: &SceneElement,
) -> Option<crate::model::ComponentKey> {
    use crate::model::ComponentMember as CM;
    match element {
        SceneElement::Component(i) => doc.components.get(*i).and_then(|c| c.parent),
        SceneElement::ConstructionPlane(i) => {
            doc.component_of(CM::ConstructionPlane(*i)).or_else(|| {
                match doc.construction_planes.get(*i)?.parent {
                    ConstructionPlaneParent::Root => None,
                    ConstructionPlaneParent::Sketch(s) => crate::model::sketch_component(doc, s),
                }
            })
        }
        SceneElement::Sketch(s) => crate::model::sketch_component(doc, *s),
        SceneElement::Extrusion(i) => doc.component_of(CM::Extrusion(*i)).or_else(|| {
            doc.extrusions
                .get(*i)
                .and_then(|e| crate::model::sketch_component(doc, e.sketch))
        }),
        SceneElement::Body(i) => doc.component_of(CM::Body(*i)).or_else(|| {
            use crate::model::BodySource;
            match &doc.bodies.get(*i)?.source {
                BodySource::Extrusion(e) => {
                    owning_component(doc, &SceneElement::Extrusion(*e))
                }
                BodySource::Extrusions(es) => es
                    .iter()
                    .find_map(|e| owning_component(doc, &SceneElement::Extrusion(*e))),
                BodySource::Imported(_) => None,
                BodySource::Loft(l) => doc.component_of(CM::Loft(*l)),
                BodySource::Revolve(r) => doc.component_of(CM::Revolution(*r)),
                // A shape has no component membership of its own (#909); the body's does.
                BodySource::Primitive(_) => None,
                BodySource::Sweep(f) => doc.component_of(CM::Sweep(*f)),
                BodySource::Repeated { op, .. } => doc.component_of(CM::RepeatOp(*op)),
                BodySource::Moved { op, .. } => doc.component_of(CM::MoveOp(*op)),
                BodySource::Mirrored { op, .. } => doc.component_of(CM::MirrorOp(*op)),
                BodySource::Boolean { op, .. } => doc.component_of(CM::BooleanOp(*op)),
                BodySource::Sliced { op, .. } => doc.component_of(CM::SliceOp(*op)),
                BodySource::Shelled { op, .. } => doc.component_of(CM::ShellOp(*op)),
                BodySource::EdgeTreated { op, .. } => {
                    doc.component_of(CM::EdgeTreatmentOp(*op))
                }
                BodySource::Solid { .. } => None,
                BodySource::UnitInstance(_) | BodySource::UnitCut { .. } => None,
                BodySource::Fused { inner, .. } => match inner.as_ref() {
                    BodySource::Loft(l) => doc.component_of(CM::Loft(*l)),
                    BodySource::Revolve(r) => doc.component_of(CM::Revolution(*r)),
                    BodySource::Sweep(f) => doc.component_of(CM::Sweep(*f)),
                    _ => None,
                },
            }
        }),
        SceneElement::Image(i) => doc
            .tracing_images
            .get(*i)
            .and_then(|img| owning_component(doc, &SceneElement::ConstructionPlane(img.plane))),
        SceneElement::BooleanOp(i) => doc.component_of(CM::BooleanOp(*i)),
        SceneElement::MoveOp(i) => doc.component_of(CM::MoveOp(*i)),
        SceneElement::MirrorOp(i) => doc.component_of(CM::MirrorOp(*i)),
        SceneElement::RepeatOp(i) => doc.component_of(CM::RepeatOp(*i)),
        SceneElement::SliceOp(i) => doc.component_of(CM::SliceOp(*i)),
        SceneElement::ShellOp(i) => doc.component_of(CM::ShellOp(*i)),
        SceneElement::EdgeTreatmentOp(i) => doc.component_of(CM::EdgeTreatmentOp(*i)),
        SceneElement::Revolution(i) => doc.component_of(CM::Revolution(*i)),
        // A shape isn't a component member of its own (#909).
        SceneElement::Shape(_) => None,
        SceneElement::SweepOp(i) => doc.component_of(CM::Sweep(*i)),
        SceneElement::Loft(i) => doc.component_of(CM::Loft(*i)),
        SceneElement::Drawing(i) => doc.component_of(CM::Drawing(*i)),
        // In-sketch geometry cascades through its sketch's plane (handled by the sketch's
        // own effective-visibility recursion); everything else has no owning component.
        _ => None,
    }
}

fn parent_element(doc: &Document, element: SceneElement) -> Option<SceneElement> {
    match element {
        // A drawing item's parent is its page, which has no scene element of its own (#967).
        SceneElement::DrawingElement { .. } => None,
        // A cross-section view hangs off nothing (#1671).
        SceneElement::CrossSection(_) => None,
        SceneElement::SectionPlane { view, .. } => Some(SceneElement::CrossSection(view)),
        // A unit instance is always a top-level row (#723).
        SceneElement::UnitInstance(_) => None,
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
        // A body nests under the feature that produced it (#1106): a pure shape body under
        // its Shape; a fused combine/cut solid under the extrusion that produced it (last
        // add/cut), not under the Shape.
        SceneElement::Body(index) => doc.bodies.get(index).and_then(|body| {
            if let crate::model::BodySource::Primitive(p) = body.source {
                return Some(SceneElement::Shape(p));
            }
            body.source
                .producing_extrusion()
                .map(|ei| SceneElement::Extrusion(ei))
                .or_else(|| {
                    body.source
                        .extrusion_indices()
                        .first()
                        .map(|&ei| SceneElement::Extrusion(ei))
                })
        }),
        // A face's own edge isn't a hierarchy-pane node in its own right (it's a constraint
        // reference, not an independently listed element) — no parent to nest under.
        SceneElement::FaceEdge(_) | SceneElement::Origin | SceneElement::GlobalAxis(_) => None,
        // Body sub-elements (#156/#555) likewise aren't pane nodes of their own.
        SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::ProjectedEdge { .. }
        | SceneElement::ProjectedCorner { .. }
        | SceneElement::BodyFace { .. }
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_)
        | SceneElement::ExtrusionEdge { .. }
        | SceneElement::PrimitiveEdge { .. }
        | SceneElement::RepeatedFace { .. } => None,
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
        SceneElement::ShellOp(_) => None,
        SceneElement::EdgeTreatmentOp(_) => None,
        SceneElement::Revolution(_) | SceneElement::Shape(_) => None,
        SceneElement::SweepOp(_) => None,
        SceneElement::Loft(_) => None,
        SceneElement::Drawing(_) => None,
        // A joint is always a top-level row (#891).
        SceneElement::Joint(_) => None,
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
        ConstraintPoint::ImageCalibrationPoint { image, .. }
        | ConstraintPoint::ImageAnchor { image, .. } => Some(SceneElement::Image(image)),
        ConstraintPoint::Origin => Some(SceneElement::Origin),
        // A face's own vertex nests under the feature that produced its face.
        ConstraintPoint::FaceVertex { face, .. } => face_owner_element(&face),
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
        // Nothing hangs off a drawing item (#967).
        SceneElement::DrawingElement { .. } => {}
        SceneElement::CrossSection(view) => {
            if let Some(v) = doc.cross_sections.get(view) {
                for cut in 0..v.cuts.len() {
                    let child = SceneElement::SectionPlane { view, cut };
                    out.insert(child.clone());
                    collect_descendants(doc, child, out);
                }
            }
        }
        SceneElement::SectionPlane { .. } => {}
        // A unit's contents have no scene identity to collect (#723).
        SceneElement::UnitInstance(_) => {}
        SceneElement::Component(index) => {
            for (m, c) in doc.component_members.iter() {
                if *c != index {
                    continue;
                }
                if let Some(e) = component_member_element(*m) {
                    out.insert(e.clone());
                    collect_descendants(doc, e, out);
                }
            }
            for (ci, comp) in doc.components.iter() {
                if comp.parent == Some(index) {
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
            for (li, line) in doc.lines.iter() {
                if line.sketch == sketch {
                    out.insert(SceneElement::Line(li));
                }
            }
            for (ci, circle) in doc.circles.iter() {
                if circle.sketch == sketch {
                    out.insert(SceneElement::Circle(ci));
                }
            }
            for (ci, constraint) in doc.constraints.iter() {
                if constraint.sketch == sketch {
                    out.insert(SceneElement::Constraint(ci));
                }
            }
            for (ti, text) in doc.sketch_texts.iter() {
                if text.sketch == sketch {
                    out.insert(SceneElement::SketchText(ti));
                }
            }
            for (pi, plane) in doc.construction_planes.iter() {
                if matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
                    out.insert(SceneElement::ConstructionPlane(pi));
                    collect_descendants(doc, SceneElement::ConstructionPlane(pi), out);
                }
            }
            for (ei, extrusion) in doc.extrusions.iter() {
                if extrusion.sketch == sketch {
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
            for (bi, body) in doc.bodies.iter() {
                if body.source.producing_extrusion() == Some(index) {
                    out.insert(SceneElement::Body(bi));
                    collect_descendants(doc, SceneElement::Body(bi), out);
                    for si in sketches_on_body_mesh(doc, bi) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
            // Sketches placed on this extrusion's cap or side-wall faces.
            for (si, sketch) in doc.sketches.iter() {
                if matches!(sketch.face,
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
        | SceneElement::GlobalAxis(_)
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::ProjectedEdge { .. }
        | SceneElement::ProjectedCorner { .. }
        | SceneElement::BodyFace { .. }
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_)
        | SceneElement::ExtrusionEdge { .. }
        | SceneElement::PrimitiveEdge { .. }
        | SceneElement::RepeatedFace { .. }
        | SceneElement::SketchText(_)
        // A joint has no outputs — nothing descends from it (#891).
        | SceneElement::Joint(_)
        | SceneElement::Image(_) => {}
        SceneElement::BooleanOp(index) => {
            if let Some(op) = doc.boolean_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                    for si in sketches_on_body_mesh(doc, output) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
        }
        SceneElement::MoveOp(index) => {
            if let Some(op) = doc.move_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                    for si in sketches_on_body_mesh(doc, output) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
        }
        SceneElement::MirrorOp(index) => {
            if let Some(op) = doc.mirror_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                    for si in sketches_on_body_mesh(doc, output) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
        }
        SceneElement::RepeatOp(index) => {
            if let Some(op) = doc.repeat_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                    for si in sketches_on_body_mesh(doc, output) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
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
                    for si in sketches_on_body_mesh(doc, output) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
        }
        SceneElement::ShellOp(index) => {
            if let Some(op) = doc.shell_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                    for si in sketches_on_body_mesh(doc, output) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
        }
        SceneElement::EdgeTreatmentOp(index) => {
            if let Some(op) = doc.edge_treatment_ops.get(index) {
                for &output in &op.outputs {
                    out.insert(SceneElement::Body(output));
                    collect_descendants(doc, SceneElement::Body(output), out);
                    for si in sketches_on_body_mesh(doc, output) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
        }
        SceneElement::Revolution(index) => {
            // The revolved solid's output body is linked by `BodySource::Revolve`, not an
            // `outputs` list.
            for (bi, body) in doc.bodies.iter() {
                if body.source == crate::model::BodySource::Revolve(index) {
                    out.insert(SceneElement::Body(bi));
                    collect_descendants(doc, SceneElement::Body(bi), out);
                }
            }
        }
        SceneElement::Shape(index) => {
            // A shape's pure primitive body (#909/#1106) — the fused solid after a merge
            // lives under its extrusion, not here. Sketches on its faces also descend (#1103).
            for (bi, body) in doc.bodies.iter() {
                if matches!(body.source, crate::model::BodySource::Primitive(p) if p == index) {
                    out.insert(SceneElement::Body(bi));
                    collect_descendants(doc, SceneElement::Body(bi), out);
                    for si in sketches_on_body_mesh(doc, bi) {
                        out.insert(SceneElement::Sketch(si));
                        collect_descendants(doc, SceneElement::Sketch(si), out);
                    }
                }
            }
            if let Some(shape) = doc.primitives.get(index) {
                for face in crate::primitives::flat_faces(shape) {
                    let face_id = crate::model::FaceId::PrimitiveFace {
                        primitive: index,
                        face,
                    };
                    for sketch in doc.sketches_on_face(face_id) {
                        out.insert(SceneElement::Sketch(sketch));
                        collect_descendants(doc, SceneElement::Sketch(sketch), out);
                    }
                }
            }
        }
        SceneElement::SweepOp(index) => {
            // The swept solid's output body is linked by `BodySource::Sweep`.
            for (bi, body) in doc.bodies.iter() {
                if body.source == crate::model::BodySource::Sweep(index) {
                    out.insert(SceneElement::Body(bi));
                    collect_descendants(doc, SceneElement::Body(bi), out);
                }
            }
        }
        SceneElement::Loft(index) => {
            for (bi, body) in doc.bodies.iter() {
                if body.source == crate::model::BodySource::Loft(index) {
                    out.insert(SceneElement::Body(bi));
                    collect_descendants(doc, SceneElement::Body(bi), out);
                }
            }
        }
        SceneElement::Drawing(_) => {}
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
        (ConstraintLine::ImageEdge { image, .. }, SceneElement::Image(i)) => image == i,
        (ConstraintLine::ImageEdge { image, .. }, SceneElement::Point(ConstraintPoint::ImageAnchor { image: i, .. })) => {
            image == i
        }
        (ConstraintLine::ImageEdge { image, .. }, SceneElement::Point(ConstraintPoint::ImageCalibrationPoint { image: i, .. })) => {
            image == i
        }
        (ConstraintLine::ImageEdge { image: a, .. }, SceneElement::FaceEdge(ConstraintLine::ImageEdge { image: b, .. })) => {
            a == b
        }
        _ => false,
    }
}

fn constraint_point_touches_element(point: &ConstraintPoint, element: &SceneElement) -> bool {
    match (point, element) {
        (p, SceneElement::Point(q)) => p == q,
        (ConstraintPoint::LineEndpoint { line, .. }, SceneElement::Line(i)) => line == i,
        (ConstraintPoint::CircleCenter(c), SceneElement::Circle(i)) => c == i,
        (ConstraintPoint::Origin, SceneElement::Origin) => true,
        (
            ConstraintPoint::ImageCalibrationPoint { image, .. }
            | ConstraintPoint::ImageAnchor { image, .. },
            SceneElement::Image(i),
        ) => image == i,
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

fn constraints_for_element(
    doc: &Document,
    element: SceneElement,
) -> Vec<crate::model::ConstraintKey> {
    doc.constraints
        .iter()
        .filter_map(|(index, constraint)| {
            constraint_kind_touches_element(&constraint.kind, &element).then_some(index)
        })
        .collect()
}

/// Constraint indices that apply to the current selection (for Elements pane highlighting).
pub fn selection_related_constraints(
    doc: &Document,
    selection: &SceneSelection,
) -> HashSet<crate::model::ConstraintKey> {
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

/// The bodies an element **produced**, for showing what a history step made (#977).
///
/// Hovering an operation's row in the Elements pane has nothing of its own to light — an
/// operation isn't in the 3D view — so it lights its outputs instead. A component lights every
/// body under it, recursively, and a joint lights the parts it joins. Descendants, filtered to
/// live bodies: `collect_descendants` already knows every operation's outputs, so this doesn't
/// re-derive them per op kind.
pub fn produced_bodies(doc: &Document, element: &SceneElement) -> Vec<crate::model::BodyKey> {
    let mut out = HashSet::new();
    collect_descendants(doc, element.clone(), &mut out);
    let mut bodies: Vec<crate::model::BodyKey> = out
        .into_iter()
        .filter_map(|e| match e {
            SceneElement::Body(bi) => Some(bi),
            _ => None,
        })
        .filter(|bi| doc.bodies.get(*bi).is_some_and(|b| !b.shadow))
        .collect();
    // A joint has no descendants — what it "produces" is the parts it holds together (#891),
    // the same set hovering its badge in the viewport already glows (#899).
    if let SceneElement::Joint(index) = element {
        if let Some(joint) = doc.joints.get(*index) {
            for member in &joint.members {
                bodies.extend(crate::joints::member_bodies(doc, *member));
            }
        }
    }
    bodies.sort_unstable();
    bodies.dedup();
    bodies
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
/// Accent for rows whose dimension uses the hovered/focused variable — the same green the
/// 3D viewport uses for those elements (#633, `gpu_viewport::PARAMETER_HIGHLIGHT`).
const USES_VARIABLE_TEXT: Color32 = Color32::from_rgb(90, 220, 130);

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
    related_constraints: &HashSet<crate::model::ConstraintKey>,
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

/// A translucent wash of the pick-hover yellow over a hovered row the **armed picker can take**
/// (#965). The pane then says the same thing the viewport does: this is what the next click
/// feeds. A row the picker refuses gets egui's ordinary hover instead — it still selects (#963),
/// it just doesn't claim to be a pick.
fn paint_pick_affordance(
    ui: &egui::Ui,
    doc: &Document,
    armed: Option<&crate::element_picker::ElementPicker>,
    element: &SceneElement,
    hovered: bool,
    rect: egui::Rect,
) {
    if !hovered {
        return;
    }
    let takes = armed
        .is_some_and(|p| !crate::element_picker::expand_pick(doc, p, element, false).is_empty());
    if !takes {
        return;
    }
    ui.painter().rect_filled(
        rect,
        3.0,
        crate::construction::PICK_HOVER_RGBA.gamma_multiply(0.22),
    );
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

/// Paint the "active" marker (#429) inline as a small filled circle in the accent color,
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
        // Section header reuses the drawing icon (#1205).
        HierarchyNode::Drawings => IconId::Drawing,
        // The Views section and its views wear the cross-section glyph (#1671).
        HierarchyNode::Views | HierarchyNode::CrossSection(_) => IconId::CrossSection,
        HierarchyNode::SectionPlane { .. } => IconId::Plane,
        HierarchyNode::Component(_) => IconId::Component,
        HierarchyNode::ConstructionPlane(_) => IconId::Plane,
        HierarchyNode::Sketch(_) => IconId::Sketch,
        // #1193: projected sketch lines wear the Projection tool's icon so they read
        // differently from ordinary drawn lines in the Elements pane.
        HierarchyNode::Line(index) => {
            if doc.lines.get(index).is_some_and(|l| l.projection.is_some()) {
                IconId::Project
            } else {
                IconId::Line
            }
        }
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
        // Tracing images (#1549): the picture icon, distinct from the host plane.
        HierarchyNode::Image(_) => IconId::Image,
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
        HierarchyNode::ShellOp(_) => IconId::Shell,
        HierarchyNode::EdgeTreatmentOp(index) => {
            match doc.edge_treatment_ops.get(index).map(|o| o.kind) {
                Some(crate::model::VertexTreatmentKind::Fillet) => IconId::Fillet,
                _ => IconId::Chamfer,
            }
        }
        HierarchyNode::Revolution(_) => IconId::Revolve,
        // Each shape kind carries its own icon (#909).
        HierarchyNode::Shape(index) => match doc.primitives.get(index).map(|s| s.kind) {
            Some(crate::model::PrimitiveKind::Cylinder) => IconId::ShapeCylinder,
            Some(crate::model::PrimitiveKind::Sphere) => IconId::ShapeSphere,
            _ => IconId::ShapeCuboid,
        },
        HierarchyNode::SweepOp(_) => IconId::Sweep,
        // Each kind gets its own icon (#899).
        HierarchyNode::Joint(index) => doc
            .joints
            .get(index)
            .map(|j| crate::icons::icon_for_joint_kind(&j.kind))
            .unwrap_or(IconId::Joint),
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
        HierarchyNode::DrawingDimension { .. } | HierarchyNode::DrawingPointDim { .. } => {
            IconId::Dimension
        }
        // A placed unit is an assembly of parts, not the import action (#923).
        HierarchyNode::UnitInstance(_) => IconId::Assembly,
        HierarchyNode::UnitChild { instance, ordinal } => {
            return unit_child_rows(doc, instance)
                .get(ordinal)
                .map(|(icon, _)| *icon)
        }
    })
}

/// The rows a unit instance expands into (#723): the embedded document's live planes
/// (past the default ground plane), sketches, and bodies as `(icon, label)` pairs —
/// enough to look inside a part without exposing its full history. Read-only: these back
/// [`HierarchyNode::UnitChild`] display leaves with no [`SceneElement`].
pub fn unit_child_rows(doc: &Document, instance: crate::model::UnitInstanceKey) -> Vec<(IconId, String)> {
    let Some(inst) = doc.unit_instances.get(instance) else {
        return Vec::new();
    };
    let Some(unit) = doc.units.get(inst.unit) else {
        return Vec::new();
    };
    let inner = &unit.document;
    let mut rows = Vec::new();
    for (i, _plane) in inner.construction_planes.iter().skip(1) {
        rows.push((IconId::Plane, node_label(inner, HierarchyNode::ConstructionPlane(i))));
    }
    for (i, _sketch) in inner.sketches.iter() {
        rows.push((IconId::Sketch, node_label(inner, HierarchyNode::Sketch(i))));
    }
    for (i, body) in inner.bodies.iter() {
        // A unit's own materialized instance bodies show as nested-unit rows below.
        if !body.shadow
            && !matches!(body.source, crate::model::BodySource::UnitInstance(_))
        {
            rows.push((IconId::Body, node_label(inner, HierarchyNode::Body(i))));
        }
    }
    // Nested units (#735): a unit the unit itself imported reads as one opaque row —
    // its instance name — however deep the nesting goes (the cap is MAX_UNIT_DEPTH,
    // enforced at load/import, #719).
    for (i, _nested) in inner.unit_instances.iter() {
        rows.push((IconId::Import, node_label(inner, HierarchyNode::UnitInstance(i))));
    }
    rows
}

/// The [`EdgeTreatment`] a [`HierarchyNode::EdgeTreatment`] points at, if it still exists.
fn edge_treatment_at(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
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

/// Pointer state for one elements-pane row, folded across every part of the row that acts as a
/// click target (#964). A row is selectable by its **name** and by its **type icon** — both
/// report into this, so the row reacts whichever one the pointer landed on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowClick {
    pub clicked: bool,
    pub double_clicked: bool,
    pub hovered: bool,
}

impl RowClick {
    /// Read one click target's state off its response. `double_clicked` goes through
    /// [`row_primary_double_clicked`], which also catches a double-click egui attributed to the
    /// pointer rather than the widget.
    fn of(response: &egui::Response, ui: &egui::Ui) -> RowClick {
        RowClick {
            clicked: response.clicked(),
            double_clicked: row_primary_double_clicked(response, ui),
            hovered: response.hovered(),
        }
    }

    /// Fold another click target of the same row in.
    fn or(self, other: RowClick) -> RowClick {
        RowClick {
            clicked: self.clicked || other.clicked,
            double_clicked: self.double_clicked || other.double_clicked,
            hovered: self.hovered || other.hovered,
        }
    }
}

/// How an Elements row that can be *reopened* should react to pointer input this frame:
/// double-click edits, a plain click selects. Sketches, construction planes (#1691) and
/// every editable operation share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    None,
    Select { additive: bool },
    Edit,
}

pub fn row_click_action(double_clicked: bool, clicked: bool, additive: bool) -> RowAction {
    if double_clicked {
        RowAction::Edit
    } else if clicked {
        RowAction::Select { additive }
    } else {
        RowAction::None
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

/// Sketches hosted on a body's live mesh face (#1173 / #1465). Bodies stay leaves —
/// these nest as **siblings** of the host under the feature that produced it, so a
/// later merge-extrude that shadows the host does not drop them from the pane.
fn sketches_on_body_mesh(doc: &Document, body: crate::model::BodyKey) -> impl Iterator<Item = SketchId> + '_ {
    doc.sketches.iter().filter_map(move |(si, sk)| {
        match sk.face {
            FaceId::BodyMeshFace { body: b, .. } if b == body && sketch_alive(doc, si) => Some(si),
            _ => None,
        }
    })
}

fn build_body_mesh_sketches(
    doc: &Document,
    body: crate::model::BodyKey,
    sketch_session: Option<SketchSession>,
) -> Vec<HierarchyEntry> {
    sketches_on_body_mesh(doc, body)
        .map(|sketch| build_sketch_entry(doc, sketch, sketch_session))
        .collect()
}

fn push_body_and_mesh_sketches(
    children: &mut Vec<HierarchyEntry>,
    doc: &Document,
    body: crate::model::BodyKey,
    sketch_session: Option<SketchSession>,
) {
    if !doc.bodies.contains(body) {
        return;
    }
    children.push(HierarchyEntry {
        node: HierarchyNode::Body(body),
        children: Vec::new(),
    });
    children.extend(build_body_mesh_sketches(doc, body, sketch_session));
}

fn build_sketch_child_planes(
    doc: &Document,
    sketch: SketchId,
    sketch_session: Option<SketchSession>,
) -> Vec<HierarchyEntry> {
    let mut children = Vec::new();
    for (pi, plane) in doc.construction_planes.iter() {
        if !matches!(plane.parent, ConstructionPlaneParent::Sketch(s) if s == sketch) {
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
fn is_repeat_sketch_host_plane(doc: &Document, pi: crate::model::ConstructionPlaneKey) -> bool {
    doc.repeat_ops
        .values()
        .any(|op| op.sketch_plane_outputs.contains(&pi))
}

/// Whether a line is a fragment/copy generated by an in-sketch repeat (#222/#228) — those are
/// listed under their operation node, not under the sketch directly.
fn is_sketch_repeat_line_output(doc: &Document, li: crate::model::LineKey) -> bool {
    doc.sketch_repeat_ops
        .values()
        .any(|op| op.line_outputs.contains(&li))
        || doc
            .sketch_slice_ops
            .values()
            .any(|op| op.line_outputs.contains(&li))
        || doc
            .sketch_offset_ops
            .values()
            .any(|op| op.line_outputs.contains(&li))
        || doc
            .sketch_mirror_ops
            .values()
            .any(|op| op.line_outputs.contains(&li))
        || doc.sketch_vertex_treatment_ops.values().any(|op| {
            op.line_outputs.contains(&li) || op.bridge_outputs.contains(&li)
        })
}

fn is_sketch_repeat_circle_output(doc: &Document, ci: crate::model::CircleKey) -> bool {
    doc.sketch_repeat_ops
        .values()
        .any(|op| op.circle_outputs.contains(&ci))
        || doc
            .sketch_offset_ops
            .values()
            .any(|op| op.circle_outputs.contains(&ci))
        || doc
            .sketch_mirror_ops
            .values()
            .any(|op| op.circle_outputs.contains(&ci))
}

/// One in-sketch offset's row: the op with its parallel lines/circles nested beneath it
/// (they're excluded from the sketch's own listing, see `is_sketch_repeat_line_output`).
fn build_sketch_offset_entry(doc: &Document, oi: crate::model::SketchOffsetOpKey) -> HierarchyEntry {
    let op = &doc.sketch_offset_ops[oi];
    let mut children: Vec<HierarchyEntry> = op
        .line_outputs
        .iter()
        .filter(|&&li| doc.lines.contains(li))
        .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
        .collect();
    children.extend(
        op.circle_outputs
            .iter()
            .filter(|&&ci| doc.circles.contains(ci))
            .map(|&ci| HierarchyEntry { node: HierarchyNode::Circle(ci), children: Vec::new() }),
    );
    HierarchyEntry {
        node: HierarchyNode::SketchOffsetOp(oi),
        children,
    }
}

/// One in-sketch mirror's row: the op with its reflected lines/circles nested beneath it
/// (they're excluded from the sketch's own listing, see `is_sketch_repeat_line_output`).
fn build_sketch_mirror_entry(doc: &Document, oi: crate::model::SketchMirrorOpKey) -> HierarchyEntry {
    let op = &doc.sketch_mirror_ops[oi];
    let mut children: Vec<HierarchyEntry> = op
        .line_outputs
        .iter()
        .filter(|&&li| doc.lines.contains(li))
        .map(|&li| HierarchyEntry { node: HierarchyNode::Line(li), children: Vec::new() })
        .collect();
    children.extend(
        op.circle_outputs
            .iter()
            .filter(|&&ci| doc.circles.contains(ci))
            .map(|&ci| HierarchyEntry { node: HierarchyNode::Circle(ci), children: Vec::new() }),
    );
    HierarchyEntry {
        node: HierarchyNode::SketchMirrorOp(oi),
        children,
    }
}

fn build_sketch_entry(
    doc: &Document,
    sketch: SketchId,
    sketch_session: Option<SketchSession>,
) -> HierarchyEntry {
    let mut children = build_sketch_child_planes(doc, sketch, sketch_session);

    if sketch_session.is_some_and(|s| s.sketch == sketch) {
        for (li, line) in doc.lines.iter() {
            if line.sketch != sketch || is_sketch_repeat_line_output(doc, li) {
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
            // the parent is gone or otherwise not found — same graceful-orphan
            // handling as elsewhere in this file — fall back to a top-level sibling instead of
            // dropping the bridging line from the tree.
            if let Some(parent) = line.chamfer_fillet_parent {
                let alive_parent = doc
                    .lines
                    .get(parent)
                    .is_some_and(|p| p.sketch == sketch);
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
        for (ci, circle) in doc.circles.iter() {
            if circle.sketch != sketch || is_sketch_repeat_circle_output(doc, ci) {
                continue;
            }
            let nested = build_face_sketches(doc, FaceId::Circle(ci), sketch_session);
            children.push(HierarchyEntry {
                node: HierarchyNode::Circle(ci),
                children: nested,
            });
        }
        for (ci, constraint) in doc.constraints.iter() {
            if constraint.sketch != sketch {
                continue;
            }
            children.push(HierarchyEntry {
                node: HierarchyNode::Constraint(ci),
                children: vec![],
            });
        }
        for (ti, text) in doc.sketch_texts.iter() {
            if text.sketch != sketch {
                continue;
            }
            children.push(HierarchyEntry {
                node: HierarchyNode::SketchText(ti),
                children: vec![],
            });
        }
    } else {
        for (ci, circle) in doc.circles.iter() {
            if circle.sketch != sketch || is_sketch_repeat_circle_output(doc, ci) {
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

    // Offsets of this sketch's geometry nest under it (#941): the op belongs to the sketch,
    // so it reads as a sketch feature rather than a document-level sibling.
    for (oi, op) in doc.sketch_offset_ops.iter() {
        if op.sketch == sketch {
            children.push(build_sketch_offset_entry(doc, oi));
        }
    }
    // Mirrors of this sketch's geometry nest under it too (#1540): a sketch mirror is a
    // component of the sketch, so hiding sketch components hides it and its children.
    for (oi, op) in doc.sketch_mirror_ops.iter() {
        if op.sketch == sketch {
            children.push(build_sketch_mirror_entry(doc, oi));
        }
    }
    // Extrusions built from this sketch nest under it (each owns its Body).
    children.extend(build_sketch_extrusions(doc, sketch, sketch_session));
    // Sweeps whose profile faces live in this sketch nest under it too (#478), each
    // owning its output body — so the graph shows the sketch (the faces' proxy) as the
    // op's input rather than the document root.
    for (oi, fp) in doc.sweeps.iter() {
        if fp.sketch != sketch {
            continue;
        }
        let mut bodies = Vec::new();
        for (bi, b) in doc.bodies.iter() {
            if b.source == crate::model::BodySource::Sweep(oi) {
                push_body_and_mesh_sketches(&mut bodies, doc, bi, sketch_session);
            }
        }
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
        .filter(|(_, extrusion)| extrusion.sketch == sketch)
        .map(|(ei, _)| {
            // Bodies **produced** by this extrusion nest under it (#1106/#1107): the fused
            // combine/cut result is the extrusion's output. Intermediate shadow solids that
            // still list this extrusion further down their add list stay under their own
            // producing extrusion.
            let mut children = Vec::new();
            for (bi, body) in doc.bodies.iter() {
                if body.source.producing_extrusion() == Some(ei) {
                    push_body_and_mesh_sketches(&mut children, doc, bi, sketch_session);
                }
            }
            for (si, sk) in doc.sketches.iter() {
                if matches!(sk.face,
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
    filter: &mut ElementFilter,
    filter_expanded: &mut bool,
    on_edit_sketch: &mut impl FnMut(SketchId),
    on_edit_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_import_image_on_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_edit_extrusion: &mut impl FnMut(crate::model::ExtrusionKey),
    on_edit_edge_treatment: &mut impl FnMut(crate::model::ExtrusionKey, usize),
    on_edit_edge_treatment_op: &mut impl FnMut(crate::model::EdgeTreatmentOpKey),
    on_edit_operation: &mut impl FnMut(SceneElement),
    on_joint_rest: &mut impl FnMut(JointRestCommand),
    on_edit_drawing: &mut impl FnMut(crate::model::DrawingKey),
    on_select_drawing_element: &mut impl FnMut(HierarchyNode),
    on_hover_drawing_element: &mut impl FnMut(Option<HierarchyNode>),
    selected_drawing_leaf: Option<HierarchyNode>,
    on_rename_drawing: &mut impl FnMut(crate::model::DrawingKey, String),
    on_set_body_shadow: &mut impl FnMut(crate::model::BodyKey, bool),
    on_export_body: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_step: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_3mf: &mut impl FnMut(crate::model::BodyKey),
    on_export_component: &mut impl FnMut(crate::model::ComponentKey),
    on_export_component_step: &mut impl FnMut(crate::model::ComponentKey),
    on_export_component_3mf: &mut impl FnMut(crate::model::ComponentKey),
    on_toggle_visibility: &mut impl FnMut(SceneElement, bool),
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_hover_element: &mut impl FnMut(SceneElement),
    on_delete_element: &mut impl FnMut(SceneElement),
    on_clone_unit_instance: &mut impl FnMut(crate::model::UnitInstanceKey),
    clipboard_has_items: bool,
    clipboard_has_linkable: bool,
    on_copy: &mut impl FnMut(),
    on_paste: &mut impl FnMut(bool),
    // `active_drawing`: the open drawing (Drawing workbench) enabling the row "Add to
    // drawing" action (#274); `on_add_to_drawing` receives the body index.
    // `on_create_drawing_of_body`: Elements-pane body right-click → new drawing of that body (#1158).
    active_drawing: Option<crate::model::DrawingKey>,
    on_add_to_drawing: &mut impl FnMut(SceneElement),
    on_create_drawing_of_body: &mut impl FnMut(crate::model::BodyKey),
    highlight_elements: &HashSet<SceneElement>,
    // The armed element picker (#965), if any: a row it can take wears the pick affordance on
    // hover, so the pane says what the next click would feed. A row it refuses still hovers
    // and still selects (#963) — it just doesn't claim to be a pick.
    armed: Option<&crate::element_picker::ElementPicker>,
    rolled_back: &HashSet<SceneElement>,
    // The current timeline rollback marker (#524), if any, and a setter (None clears it).
    rollback_marker: Option<&RollbackMarker>,
    on_set_rollback: &mut impl FnMut(Option<RollbackMarker>),
    collapsed_components: &mut HashSet<crate::model::ComponentKey>,
    // Unit instances whose read-only contents are expanded in the List (#723); default
    // collapsed, so an instance reads as one row.
    expanded_units: &mut HashSet<crate::model::UnitInstanceKey>,
    // The collapsible Drawings section at the bottom of the List (#1205); default expanded.
    section_collapsed: &mut SectionCollapse,
    on_add_component: &mut impl FnMut(Option<crate::model::ComponentKey>),
    on_add_cross_section: &mut impl FnMut(),
    on_add_drawing: &mut impl FnMut(),
    // Cutting planes of views are Elements-pane children only in the View workbench (#1761).
    show_section_planes: bool,
    on_move_to_component: &mut impl FnMut(SceneElement, Option<crate::model::ComponentKey>),
    active_component: Option<crate::model::ComponentKey>,
    on_activate_component: &mut impl FnMut(Option<crate::model::ComponentKey>),
) {
    // The tutorial row anchors below are "first row of its kind, this frame" targets
    // (#1279/#1647/#1673), and `insert_temp` outlives the frame — so clearing them here is
    // what makes them per-frame. Without it the rect captured while the Modeling pane held a
    // longer tree stuck around on the Drawing workbench, parking the orb well below the row
    // it named (#1702/#1705).
    ui.ctx().data_mut(|d| {
        d.remove::<egui::Rect>(elements_sketch_row_rect_id());
        d.remove::<egui::Rect>(elements_body_row_rect_id());
        d.remove::<egui::Rect>(elements_plane_row_rect_id());
    });
    ui.horizontal(|ui| {
        ui.heading(PANE_TITLE);
        // Explicit id so nested RTL auto-ids stay stable across multipass (#1169 / egui#8343).
        ui.scope_builder(
            egui::UiBuilder::new()
                .layout(egui::Layout::right_to_left(egui::Align::Center))
                .id(ui.id().with("hierarchy_mode_toolbar")),
            |ui| {
                // The Tree view is retired (#252): a strict tree can't show an element with multiple
                // inputs (e.g. a body that's both an op output and another op's input), so only List
                // and the dependency-aware Graph remain. The enum variant stays for script
                // back-compat; a lingering `Tree` mode renders as List (see the match below).
                for (mode, icon, tooltip) in [
                    (HierarchyViewMode::Graph, IconId::ViewGraph, "Graph view"),
                    (HierarchyViewMode::List, IconId::ViewList, "List view"),
                ] {
                    let selected = *view_mode == mode
                        || (mode == HierarchyViewMode::List && *view_mode == HierarchyViewMode::Tree);
                    ui.push_id(icon.label(), |ui| {
                        if selectable_icon_button(ui, icon, selected, tooltip).clicked() {
                            *view_mode = mode;
                        }
                    });
                }
                // Add menu (#423): the + opens a popup with creatable containers.
                let add = ui
                    .push_id("hierarchy_add", |ui| {
                        selectable_icon_button(ui, IconId::Plus, false, "Add…")
                    })
                    .inner;
                egui::Popup::menu(&add).show(|ui| {
                    if ui.button("New component").clicked() {
                        on_add_component(None);
                        ui.close();
                    }
                    // A cross-section view (#1671): a way of looking at the model, kept with
                    // the other views at the bottom of the pane.
                    if ui.button("New drawing").clicked() {
                        on_add_drawing();
                        ui.close();
                    }
                    if ui.button("New cross section").clicked() {
                        on_add_cross_section();
                        ui.close();
                    }
                });
            },
        );
    });
    ui.separator();

    // Timeline rollback status (#524/#545): when rolled back, show the marker and a Done button
    // (#619) to roll forward. Setting a rollback point is done per-element from the row's
    // right-click "Rollback" submenu (#545), not a header button.
    if let Some(marker) = rollback_marker {
        ui.horizontal(|ui| {
            let noun = if marker.inclusive { "just before" } else { "" };
            let banner = format!(
                "⏮ Rolled back to {noun} {}",
                crate::names::scene_element_label(doc, &marker.element)
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&banner)
                        .color(crate::theme::FOCUS_ACCENT)
                        .size(11.5),
                )
                .truncate(),
            )
            .on_hover_text(&banner);
            if ui
                .small_button("Done")
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

    // Filter bottom panel FIRST (#1282 / egui panel ordering): nested panels must be
    // allocated before the remaining content. Showing it after the ScrollArea forced a
    // multipass re-layout that thrashing auto-ids on the whole Elements pane (full-height
    // rect flashing red with "Widget rect … changed id between passes").
    show_elements_filter(ui, filter, filter_expanded);

    match view_mode {
        // `Tree` is retired (#252); a lingering script-set Tree mode falls back to List.
        HierarchyViewMode::List | HierarchyViewMode::Tree => {
            // Row rects belong to the Graph view alone; don't leave last frame's behind (#1670).
            set_elements_graph_row_rects(ui.ctx(), Vec::new());
            let mut list_rows: Vec<(String, egui::Rect)> = Vec::new();
            let tree = filter_hierarchy(&build_hierarchy(doc, sketch_session), filter);
            let mut tree = if filter.shadow_bodies {
                tree
            } else {
                let mut t = tree;
                prune_shadow_bodies(&mut t, doc);
                t
            };
            if !show_section_planes {
                prune_section_planes(&mut tree);
            }
            let mut rows = component_list_rows(
                &tree,
                doc,
                collapsed_components,
                *section_collapsed,
            );
            // A collapsed unit instance is one row (#723): its read-only contents only
            // appear while the row's triangle has expanded it.
            rows.retain(|(node, _)| {
                !matches!(node, HierarchyNode::UnitChild { instance, .. }
                    if !expanded_units.contains(instance))
            });
            let elements: Vec<HierarchyNode> = rows.iter().map(|(n, _)| *n).collect();
            let style_selection = selection_styles_visible_list(&elements, selection);
            // Explicit salt: nested auto-ids under the list stay put across multipass (#1282).
            egui::ScrollArea::vertical()
                .id_salt("elements_list")
                .show(ui, |ui| {
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
                            active_drawing,
                            on_toggle_visibility,
                            on_click_element,
                            on_delete_element,
                            clipboard_has_items,
                            clipboard_has_linkable,
                            on_copy,
                            on_paste,
                            on_add_component,
                            on_move_to_component,
                            on_export_component,
                            on_export_component_step,
                            on_export_component_3mf,
                            on_add_to_drawing,
                        );
                        continue;
                    }
                    // Section headers (#1205/#1671): collapse triangle + label, no eye.
                    if is_section_node(node) {
                        show_section_row(ui, doc, node, base_depth, section_collapsed);
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
                    // A unit's contents indent one level under their instance row (#723).
                    let row_depth = row_depth
                        + usize::from(matches!(node, HierarchyNode::UnitChild { .. }));
                    // Where each List row landed (#1712), so a script can click a row where it
                    // really is — the Graph view already reports its rows this way (#1670).
                    let row_top = ui.cursor().top();
                    show_row(
                        ui,
                        doc,
                        node,
                        row_depth,
                        expanded_units,
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
                        on_joint_rest,
                        on_edit_drawing,
                        on_select_drawing_element,
                        on_hover_drawing_element,
                        selected_drawing_leaf,
                        on_rename_drawing,
                        on_set_body_shadow,
                        on_export_body,
                        on_export_body_step,
                        on_export_body_3mf,
                        on_set_rollback,
                        on_toggle_visibility,
                        on_click_element,
                        on_hover_element,
                        on_delete_element,
                        on_clone_unit_instance,
                        clipboard_has_items,
                        clipboard_has_linkable,
                        on_copy,
                        on_paste,
                        active_drawing,
                        on_add_to_drawing,
                        on_create_drawing_of_body,
                        highlight_elements,
                        armed,
                        rolled_back,
                        on_move_to_component,
                        active_component,
                        on_activate_component,
                    );
                    let row_rect = egui::Rect::from_x_y_ranges(
                        ui.min_rect().x_range(),
                        row_top..=ui.cursor().top(),
                    );
                    list_rows.push((node_label(doc, node), row_rect));
                }
                set_elements_list_row_rects(ui.ctx(), list_rows);
            });
        }
        HierarchyViewMode::Graph => {
            let mut tree = graph_view_tree(doc, sketch_session, filter);
            if !show_section_planes {
                prune_section_planes(&mut tree);
            }
            show_graph_view(
                ui,
                doc,
                &tree,
                selection,
                health,
                &context,
                &related_constraints,
                on_click_element,
                on_hover_element,
                on_delete_element,
                on_clone_unit_instance,
                clipboard_has_items,
                clipboard_has_linkable,
                on_copy,
                on_paste,
                highlight_elements,
                armed,
                rolled_back,
                active_drawing,
                on_edit_sketch,
                on_edit_plane,
                on_import_image_on_plane,
                on_edit_extrusion,
                on_edit_edge_treatment,
                on_edit_edge_treatment_op,
                on_edit_operation,
                on_joint_rest,
                on_add_to_drawing,
                on_create_drawing_of_body,
                on_set_body_shadow,
                on_export_body,
                on_export_body_step,
                on_export_body_3mf,
                on_move_to_component,
                on_set_rollback,
                on_edit_drawing,
                on_rename_drawing,
            );
        }
    }
}

/// Filter control (#275): a button at the pane's bottom that expands up into a set of
/// per-type show/hide toggles. Drawn as a nested bottom panel **before** the list/graph
/// content so egui's multipass layout keeps widget ids stable (#1282).
fn show_elements_filter(
    ui: &mut egui::Ui,
    filter: &mut ElementFilter,
    filter_expanded: &mut bool,
) {
    egui::Panel::bottom("elements_filter")
        .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(4, 3)))
        .show(ui, |ui| {
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
                        shadow_bodies,
                        operations,
                        images,
                        drawings,
                        drawing_components,
                        unit_contents,
                        views,
                    } = filter;
                    let groups: [(&str, &[I], &mut bool); 11] = [
                        ("Planes", &[I::Plane], planes),
                        ("Sketches", &[I::Sketch], sketches),
                        ("Sketch components", &[I::SketchComponents], sketch_geometry),
                        ("Bodies", &[I::Body], bodies),
                        ("Shadow bodies", &[I::ShadowBody], shadow_bodies),
                        ("Operations", &[I::Extrude, I::Revolve, I::Combine], operations),
                        ("Images", &[I::Image], images),
                        ("Drawings", &[I::Drawing], drawings),
                        ("Drawing components", &[I::DrawingComponents], drawing_components),
                        ("Unit contents", &[I::Import], unit_contents),
                        ("Views", &[I::CrossSection], views),
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

/// Accent stroke for graph-view lanes/rows among the selected node's ancestors and
/// descendants. Row styling has no direct line-drawing equivalent to reuse, so this is a
/// dedicated bold accent, distinct from the node fill colors (which do reuse
/// [`icon_tint_for_row_style`] for consistency with the List/Tree views).
const GRAPH_RELATED_EDGE: Color32 = Color32::from_rgb(120, 200, 255);
/// A dependency lane (an input, not a parent) in the graph view — e.g. a drawing projection to
/// its source body (#281). A warm accent so it reads apart from the neutral parent lanes.
const GRAPH_DEPENDENCY_EDGE: Color32 = Color32::from_rgb(224, 168, 96);

/// Soft dashed tie from a constraint to the geometry it constrains (#1670) — a "related"
/// link, not a parent/child one, so it reads quieter than either input stroke.
const GRAPH_RELATED_TIE: Color32 = Color32::from_rgb(150, 140, 190);

/// Row pitch, lane pitch, and glyph sizes of the Graph view's one-node-per-line layout (#1670).
const GRAPH_ROW_H: f32 = 24.0;
const GRAPH_LANE_W: f32 = 13.0;
const GRAPH_LEFT_PAD: f32 = 10.0;
const GRAPH_ICON_SIZE: f32 = 13.0;

/// Render the graph-node view (#1670): one node per line, top to bottom, with the
/// relationships drawn as mostly-vertical lanes beside them — the way `gitk` draws commits.
/// Rows come from [`graph_lane_layout`]: a node sits below everything that feeds it, a
/// parent's children string down one shared lane with short legs into each row, and a lane is
/// reused the moment its last consumer passes — packing left into a free column when the
/// preferred lane is already carrying a trunk (#1764) — so the graph never spills right
/// further than the branches actually need. Height scrolls vertically.
#[allow(clippy::too_many_arguments)]
fn show_graph_view(
    ui: &mut egui::Ui,
    doc: &Document,
    tree: &[HierarchyEntry],
    selection: &SceneSelection,
    health: &DocumentHealth,
    context: &HashSet<SceneElement>,
    related_constraints: &HashSet<crate::model::ConstraintKey>,
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_hover_element: &mut impl FnMut(SceneElement),
    on_delete_element: &mut impl FnMut(SceneElement),
    on_clone_unit_instance: &mut impl FnMut(crate::model::UnitInstanceKey),
    clipboard_has_items: bool,
    clipboard_has_linkable: bool,
    on_copy: &mut impl FnMut(),
    on_paste: &mut impl FnMut(bool),
    highlight_elements: &HashSet<SceneElement>,
    armed: Option<&crate::element_picker::ElementPicker>,
    rolled_back: &HashSet<SceneElement>,
    // The full row context menus work on graph nodes too (#623).
    active_drawing: Option<crate::model::DrawingKey>,
    on_edit_sketch: &mut impl FnMut(SketchId),
    on_edit_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_import_image_on_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_edit_extrusion: &mut impl FnMut(crate::model::ExtrusionKey),
    on_edit_edge_treatment: &mut impl FnMut(crate::model::ExtrusionKey, usize),
    on_edit_edge_treatment_op: &mut impl FnMut(crate::model::EdgeTreatmentOpKey),
    on_edit_operation: &mut impl FnMut(SceneElement),
    on_joint_rest: &mut impl FnMut(JointRestCommand),
    on_add_to_drawing: &mut impl FnMut(SceneElement),
    on_create_drawing_of_body: &mut impl FnMut(crate::model::BodyKey),
    on_set_body_shadow: &mut impl FnMut(crate::model::BodyKey, bool),
    on_export_body: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_step: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_3mf: &mut impl FnMut(crate::model::BodyKey),
    on_move_to_component: &mut impl FnMut(SceneElement, Option<crate::model::ComponentKey>),
    on_set_rollback: &mut impl FnMut(Option<RollbackMarker>),
    on_edit_drawing: &mut impl FnMut(crate::model::DrawingKey),
    on_rename_drawing: &mut impl FnMut(crate::model::DrawingKey, String),
) {
    let layout = graph_lane_layout(doc, tree);
    if layout.rows.is_empty() {
        return;
    }

    // Nodes matching the current selection, plus their tree ancestors/descendants (#34): the
    // set of related nodes whose lines/labels get the bold accent.
    let mut related_nodes: HashSet<HierarchyNode> = HashSet::new();
    for row in &layout.rows {
        if let Some(element) = scene_element_for_node(row.node) {
            if row_is_selected(&element, selection) {
                related_nodes.extend(graph_related_nodes(tree, row.node));
            }
        }
    }
    // Only dim unrelated nodes once something is actually selected — same convention as
    // `selection_styles_visible_list` uses for the List/Tree rows.
    let style_selection = !selection.is_empty();

    let row_of: HashMap<HierarchyNode, usize> = layout
        .rows
        .iter()
        .enumerate()
        .map(|(row, r)| (r.node, row))
        .collect();
    let lane_of: HashMap<HierarchyNode, usize> =
        layout.rows.iter().map(|r| (r.node, r.lane)).collect();

    let content_height = (layout.rows.len() as f32 + 1.0) * GRAPH_ROW_H;

    egui::ScrollArea::vertical()
        .id_salt("elements_graph")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width().max(GRAPH_LEFT_PAD * 2.0);
            let (rect, _response) = ui
                .allocate_exact_size(egui::vec2(width, content_height), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            let lane_x =
                |lane: usize| rect.left() + GRAPH_LEFT_PAD + lane as f32 * GRAPH_LANE_W;
            let row_y = |row: usize| rect.top() + (row as f32 + 0.5) * GRAPH_ROW_H;
            let dot_of = |node: &HierarchyNode| -> Option<egui::Pos2> {
                Some(egui::pos2(
                    lane_x(*lane_of.get(node)?),
                    row_y(*row_of.get(node)?),
                ))
            };
            let row_rect = |row: usize| {
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), row_y(row) - GRAPH_ROW_H * 0.5),
                    egui::pos2(rect.right(), row_y(row) + GRAPH_ROW_H * 0.5),
                )
            };

            let row_extents = layout.row_line_extents();

            // Where each on-screen row landed, for scripts driving the pane (#1670).
            let clip = ui.clip_rect();
            set_elements_graph_row_rects(
                ui.ctx(),
                layout
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(row, r)| (r.node, row_rect(row)))
                    .filter(|(_, rr)| clip.intersects(*rr))
                    .collect(),
            );

            // Interact first, so the row backgrounds paint underneath the lanes and labels.
            let responses: Vec<egui::Response> = layout
                .rows
                .iter()
                .enumerate()
                .map(|(row, r)| {
                    let id = ui.id().with(("hierarchy_graph_node", r.node));
                    ui.interact(row_rect(row), id, egui::Sense::click())
                })
                .collect();
            let row_fills: Vec<Option<Color32>> = layout
                .rows
                .iter()
                .enumerate()
                .map(|(row, r)| {
                    let selected = scene_element_for_node(r.node)
                        .is_some_and(|el| row_shows_selection(&el, selection, style_selection));
                    if selected {
                        Some(ui.visuals().selection.bg_fill.gamma_multiply(0.55))
                    } else if responses[row].hovered() {
                        Some(ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.35))
                    } else {
                        None
                    }
                })
                .collect();
            for (row, fill) in row_fills.iter().enumerate() {
                if let Some(fill) = fill {
                    painter.rect_filled(row_rect(row), 2.0, *fill);
                }
            }

            // Every relationship line, cut into segments up front: which one gives way at a
            // crossing is then a property of the pair, not of paint order (#1683).
            let mut solid: Vec<GraphLineSegment> = Vec::new();
            for edge in &layout.edges {
                let (Some(from), Some(to)) = (dot_of(&edge.from), dot_of(&edge.to)) else {
                    continue;
                };
                let highlighted =
                    related_nodes.contains(&edge.from) && related_nodes.contains(&edge.to);
                if !edge.kind.is_input() {
                    // A constraint ties sideways to what it constrains: the shortest dashed
                    // hop between the two dots, claiming no lane of its own (#1670). Dashes
                    // already read as "passing under", so these need no crossing gaps.
                    paint_dashed_line(&painter, from, to, GRAPH_RELATED_TIE, highlighted);
                    continue;
                }
                let x = lane_x(edge.lane);
                let mut points = vec![from];
                if (x - from.x).abs() > 0.5 {
                    points.push(egui::pos2(x, from.y + GRAPH_ROW_H * 0.5));
                }
                if (x - to.x).abs() > 0.5 {
                    points.push(egui::pos2(x, to.y - GRAPH_ROW_H * 0.5));
                }
                points.push(to);
                let color = if highlighted {
                    GRAPH_RELATED_EDGE
                } else if edge.kind == GraphLaneEdgeKind::Dependency {
                    GRAPH_DEPENDENCY_EDGE
                } else {
                    Color32::from_gray(110)
                };
                let stroke = egui::Stroke::new(if highlighted { 2.0 } else { 1.2 }, color);
                for pair in points.windows(2) {
                    solid.push(GraphLineSegment { a: pair[0], b: pair[1], stroke });
                }
            }
            paint_graph_lines(&painter, &solid);

            for (row, r) in layout.rows.iter().enumerate() {
                let node = r.node;
                let center = egui::pos2(lane_x(r.lane), row_y(row));
                let element = scene_element_for_node(node);
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
                let related = related_nodes.contains(&node);
                let tint = if selected {
                    Color32::WHITE
                } else if related {
                    GRAPH_RELATED_EDGE
                } else {
                    style.map(icon_tint_for_row_style).unwrap_or(Color32::from_gray(170))
                };

                let icon_rect =
                    egui::Rect::from_center_size(center, egui::Vec2::splat(GRAPH_ICON_SIZE));
                let response = &responses[row];
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    // Pane-hover → viewport highlight (#161).
                    if let Some(element) = element.clone() {
                        on_hover_element(element);
                    }
                }
                if let Some(element) = element {
                    paint_pick_affordance(ui, doc, armed, &element, response.hovered(), icon_rect);
                    // With a drawing open, the same rows the List view drags onto the page drag
                    // from here too (#1819): the drop places a projection at the pointer. Whole
                    // row, like the click. Re-sensed for drag so the payload arms; plain clicks
                    // still select as usual.
                    if active_drawing.is_some()
                        && matches!(
                            node,
                            HierarchyNode::Body(_)
                                | HierarchyNode::Sketch(_)
                                | HierarchyNode::Component(_)
                                | HierarchyNode::CrossSection(_)
                        )
                    {
                        response
                            .clone()
                            .interact(egui::Sense::drag())
                            .dnd_set_drag_payload(DrawingDragPayload(element.clone()));
                    }
                    // Double-click edits where the List/Tree rows do (#546/#1691/#1755), a
                    // plain click selects — the same dispatch, one node per graph row (#1770).
                    // The double-click goes through `row_primary_double_clicked`, which also
                    // catches one egui attributes to the pointer rather than the row.
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    let double_clicked = row_primary_double_clicked(response, ui);
                    match node {
                        HierarchyNode::Sketch(sketch) => {
                            if double_clicked {
                                on_edit_sketch(sketch);
                            } else if response.clicked() {
                                on_click_element(element.clone(), additive);
                            }
                        }
                        HierarchyNode::ConstructionPlane(index) => {
                            if double_clicked {
                                on_edit_plane(index);
                            } else if response.clicked() {
                                on_click_element(element.clone(), additive);
                            }
                        }
                        HierarchyNode::Extrusion(index) => {
                            if double_clicked {
                                on_edit_extrusion(index);
                            } else if response.clicked() {
                                on_click_element(element.clone(), additive);
                            }
                        }
                        HierarchyNode::EdgeTreatmentOp(index) => {
                            if double_clicked {
                                on_edit_edge_treatment_op(index);
                            } else if response.clicked() {
                                on_click_element(element.clone(), additive);
                            }
                        }
                        _ if node_editable_operation(node).is_some() => {
                            if double_clicked {
                                on_edit_operation(element.clone());
                            } else if response.clicked() {
                                on_click_element(element.clone(), additive);
                            }
                        }
                        _ => {
                            if response.clicked() {
                                on_click_element(element.clone(), additive);
                            }
                        }
                    }
                    // The same context menu the List/Tree row shows (#623).
                    response.context_menu(|ui| {
                        element_context_menu(
                            ui,
                            doc,
                            node,
                            &element,
                            active_drawing,
                            on_edit_sketch,
                            on_edit_plane,
                            on_import_image_on_plane,
                            on_edit_extrusion,
                            on_edit_edge_treatment_op,
                            on_edit_operation,
                            on_joint_rest,
                            on_add_to_drawing,
                            on_create_drawing_of_body,
                            on_set_body_shadow,
                            on_export_body,
                            on_export_body_step,
                            on_export_body_3mf,
                            on_move_to_component,
                            on_set_rollback,
                            on_delete_element,
                            on_clone_unit_instance,
                            clipboard_has_items,
                            clipboard_has_linkable,
                            crate::copy_paste::copyable_element(&element).is_some(),
                            on_copy,
                            on_paste,
                        );
                    });
                } else {
                    // Display-only leaves keep their row menus in the graph too (#623).
                    match node {
                        HierarchyNode::Drawing(index) => {
                            response.context_menu(|ui| {
                                drawing_context_menu(ui, doc, index, on_edit_drawing, on_rename_drawing);
                            });
                        }
                        HierarchyNode::EdgeTreatment { extrusion, index } => {
                            if let Some(treatment) = edge_treatment_at(doc, extrusion, index) {
                                let noun = match treatment.kind {
                                    crate::model::VertexTreatmentKind::Chamfer => "chamfer",
                                    crate::model::VertexTreatmentKind::Fillet => "fillet",
                                };
                                response.context_menu(|ui| {
                                    if ui.button(format!("Edit {noun}")).clicked() {
                                        on_edit_edge_treatment(extrusion, index);
                                        ui.close();
                                    }
                                });
                            }
                        }
                        _ => {}
                    }
                }

                // Each node draws as its element's icon (#152) — the same icon the List/Tree
                // rows use, tinted by selection/health state; only the synthetic Document
                // root (which has no icon) keeps the plain dot.
                // An icon is mostly transparent, so a lane running to the dot would show
                // straight through the glyph: give it the row's own background to sit on, and
                // the lines disappear behind it instead (#1683).
                let chip = icon_rect.expand(1.0);
                painter.rect_filled(chip, 3.0, ui.visuals().panel_fill);
                if let Some(fill) = row_fills[row] {
                    painter.rect_filled(chip, 3.0, fill);
                }
                if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                    crate::icons::paint_icon(&painter, ui.ctx(), icon, icon_rect, tint);
                } else {
                    painter.circle_filled(center, GRAPH_ICON_SIZE * 0.3, tint);
                }

                // The label follows its own dot, so a row's indent shows its place in the
                // graph, but never starts before the rightmost line drawn at this row — a
                // lane running past must not cross a name (#1683). It truncates at the pane
                // edge rather than widening the pane (#34).
                let label_x = (center.x + GRAPH_ICON_SIZE * 0.5 + 4.0)
                    .max(lane_x(row_extents.get(row).copied().unwrap_or(r.lane)) + 5.0);
                let label = node_label(doc, node);
                let truncated = truncate_label(&label, (rect.right() - 4.0 - label_x).max(20.0), &painter);
                painter.text(
                    egui::pos2(label_x, center.y),
                    egui::Align2::LEFT_CENTER,
                    truncated,
                    egui::FontId::default(),
                    if selected || related { Color32::WHITE } else { Color32::from_gray(200) },
                );
            }
        });
}

/// One straight run of a graph relationship line (#1683). Lines are split into these so a
/// crossing can be resolved by the pair — a vertical run keeps going, the line that meets it
/// at an angle breaks around it — rather than by whichever happened to paint last.
#[derive(Clone, Copy)]
struct GraphLineSegment {
    a: egui::Pos2,
    b: egui::Pos2,
    stroke: egui::Stroke,
}

impl GraphLineSegment {
    fn is_vertical(&self) -> bool {
        (self.a.x - self.b.x).abs() < 0.5
    }

    /// Where this segment crosses `other`, as a fraction along self, if they cross at all
    /// (endpoints touching — every line meeting a dot — never count).
    fn crossing(&self, other: &GraphLineSegment) -> Option<f32> {
        let r = self.b - self.a;
        let s = other.b - other.a;
        let denom = r.x * s.y - r.y * s.x;
        if denom.abs() < 1e-4 {
            return None;
        }
        let q = other.a - self.a;
        let t = (q.x * s.y - q.y * s.x) / denom;
        let u = (q.x * r.y - q.y * r.x) / denom;
        const EDGE: f32 = 0.02;
        (t > EDGE && t < 1.0 - EDGE && u > EDGE && u < 1.0 - EDGE).then_some(t)
    }
}

/// Paint the graph's relationship lines, breaking the giving-way line at each crossing
/// (#1683). Vertical runs — the lanes — always keep going; a line that meets one at an angle
/// breaks around it. Two angled lines that cross settle it the same way every frame: the one
/// that starts further left keeps going.
fn paint_graph_lines(painter: &egui::Painter, segments: &[GraphLineSegment]) {
    // Draw order: the ones that give way first, so the gap sits under a solid neighbour.
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by_key(|i| (segments[*i].is_vertical(), *i));
    for &i in &order {
        let segment = segments[i];
        let mut gaps: Vec<f32> = Vec::new();
        for (j, other) in segments.iter().enumerate() {
            if i == j || wins_the_crossing(&segment, other, i, j) {
                continue;
            }
            if let Some(t) = segment.crossing(other) {
                gaps.push(t);
            }
        }
        paint_segment_with_gaps(painter, &segment, &mut gaps);
    }
}

/// Whether `a` (index `i`) keeps going where it crosses `b` (index `j`).
fn wins_the_crossing(a: &GraphLineSegment, b: &GraphLineSegment, i: usize, j: usize) -> bool {
    match (a.is_vertical(), b.is_vertical()) {
        (true, false) => true,
        (false, true) => false,
        // Both angled (or both vertical, which cannot cross): the leftmost start wins, with
        // the index as a last tiebreak so the choice never flickers.
        _ => (a.a.x, a.a.y, i) < (b.a.x, b.a.y, j),
    }
}

/// Draw one segment, leaving a small gap at each crossing fraction in `gaps`.
fn paint_segment_with_gaps(painter: &egui::Painter, segment: &GraphLineSegment, gaps: &mut Vec<f32>) {
    let delta = segment.b - segment.a;
    let len = delta.length();
    if gaps.is_empty() || len < 1.0 {
        painter.line_segment([segment.a, segment.b], segment.stroke);
        return;
    }
    const GAP_PX: f32 = 3.0;
    let half = (GAP_PX / len).min(0.45);
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut cursor = 0.0f32;
    for gap in gaps.iter() {
        let start = gap - half;
        if start > cursor {
            painter.line_segment(
                [segment.a + delta * cursor, segment.a + delta * start],
                segment.stroke,
            );
        }
        cursor = cursor.max(gap + half);
    }
    if cursor < 1.0 {
        painter.line_segment([segment.a + delta * cursor, segment.b], segment.stroke);
    }
}

/// Dashes a straight line between two points — the graph view's constraint "related" tie
/// (#1670), which egui has no stroke style for.
fn paint_dashed_line(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    color: Color32,
    highlighted: bool,
) {
    let delta = to - from;
    let len = delta.length();
    if len < 1.0 {
        return;
    }
    let dir = delta / len;
    let (color, width) = if highlighted {
        (GRAPH_RELATED_EDGE, 2.0)
    } else {
        (color, 1.0)
    };
    let dash = 4.0;
    let mut t = 0.0;
    while t < len {
        let start = from + dir * t;
        let end = from + dir * (t + dash).min(len);
        painter.line_segment([start, end], egui::Stroke::new(width, color));
        t += dash * 2.0;
    }
}

/// Selectable row label that clips to the remaining pane width instead of stretching it (#1575).
fn clipped_selectable_label(
    ui: &mut egui::Ui,
    selected: bool,
    text: impl Into<egui::WidgetText>,
    full: &str,
) -> egui::Response {
    let available = ui.available_width();
    let too_long = ui.fonts_mut(|f| {
        f.layout_no_wrap(full.to_string(), egui::FontId::default(), Color32::WHITE)
            .size()
            .x
    }) > available;
    let resp = ui.add(egui::Button::selectable(selected, text).truncate());
    if too_long {
        resp.on_hover_text(full)
    } else {
        resp
    }
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

/// Flatten the tree into List-view rows with component nesting (#423) and a trailing
/// Drawings section (#1205): loose elements first (flat, topologically sorted, depth
/// `base`), then each component row with its contents indented one level, then the
/// Drawings section at the bottom. Collapsed components / the drawings section skip
/// their contents.
fn component_list_rows(
    tree: &[HierarchyEntry],
    doc: &Document,
    collapsed: &HashSet<crate::model::ComponentKey>,
    sections_collapsed: SectionCollapse,
) -> Vec<(HierarchyNode, usize)> {
    fn level(
        entries: &[HierarchyEntry],
        doc: &Document,
        collapsed: &HashSet<crate::model::ComponentKey>,
        sections_collapsed: SectionCollapse,
        base: usize,
        out: &mut Vec<(HierarchyNode, usize)>,
    ) {
        let (components, rest): (Vec<&HierarchyEntry>, Vec<&HierarchyEntry>) = entries
            .iter()
            .partition(|e| matches!(e.node, HierarchyNode::Component(_)));
        // The bottom sections are forced last so they never intersperse with model elements
        // (#1205/#1671).
        let (sections, loose): (Vec<&HierarchyEntry>, Vec<&HierarchyEntry>) = rest
            .into_iter()
            .partition(|e| is_section_node(e.node));
        let loose_owned: Vec<HierarchyEntry> = loose.into_iter().cloned().collect();
        for node in element_list_from_tree(&loose_owned, doc) {
            out.push((node, base));
        }
        for entry in components {
            let HierarchyNode::Component(ci) = entry.node else { unreachable!() };
            out.push((entry.node, base));
            if !collapsed.contains(&ci) {
                level(
                    &entry.children,
                    doc,
                    collapsed,
                    sections_collapsed,
                    base + 1,
                    out,
                );
            }
        }
        for entry in sections {
            out.push((entry.node, base));
            if !sections_collapsed.collapsed(entry.node) {
                if matches!(entry.node, HierarchyNode::Views) {
                    // Cutting planes nest under the view they belong to (#1754), not as
                    // siblings of it.
                    for child in &entry.children {
                        out.push((child.node, base + 1));
                        for plane in &child.children {
                            out.push((plane.node, base + 2));
                        }
                    }
                } else {
                    // Each drawing (and its projections/notes when the filter keeps them) sits
                    // one level under the section header.
                    for child in &entry.children {
                        for node in element_list_from_tree(std::slice::from_ref(child), doc) {
                            out.push((node, base + 1));
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    // The tree is the single synthetic Document root; its children are the real entries.
    for root in tree {
        level(
            &root.children,
            doc,
            collapsed,
            sections_collapsed,
            1,
            &mut out,
        );
    }
    out
}

/// Collapse/expand disclosure triangle for list rows.
///
/// `width` is the hit-target column width. Component/unit rows use a narrow column in front
/// of the eye; the Drawings section has no eye, so its column matches [`ICON_DISPLAY_SIZE`]
/// and the following type icon lines up with every other row (#1232).
fn collapse_triangle(ui: &mut egui::Ui, collapsed: bool, width: f32) -> egui::Response {
    let (tri_rect, tri_resp) =
        ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::click());
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
    tri_resp.on_hover_text(if collapsed { "Expand" } else { "Collapse" })
}

/// Narrow disclosure column when a row also has a visibility eye (components, units).
const DISCLOSURE_BEFORE_EYE_WIDTH: f32 = 12.0;

/// A collapsible bottom-section header in the List view — "Drawings" (#1205) or "Views"
/// (#1671): triangle + icon + label. Not selectable or hideable; it only groups its rows.
fn show_section_row(
    ui: &mut egui::Ui,
    doc: &Document,
    node: HierarchyNode,
    depth: usize,
    section_collapsed: &mut SectionCollapse,
) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 18.0);
        let collapsed = section_collapsed.collapsed(node);
        // Triangle replaces the eye column, so match eye width for type-icon alignment (#1232).
        if collapse_triangle(ui, collapsed, ICON_DISPLAY_SIZE).clicked() {
            section_collapsed.toggle(node);
        }
        if let Some(icon) = icon_for_hierarchy_node(doc, node) {
            ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
        }
        let label = node_label(doc, node);
        let _ = clipped_selectable_label(ui, false, RichText::new(&label).strong(), &label);
    });
}

/// One component row in the List view (#423): collapse triangle, eye toggle, icon, name;
/// click selects, right-click offers a nested component / delete; rows dropped on it move
/// into the component. With a drawing open, the row also drags onto the page and offers
/// **Add to drawing** (#1190).
#[allow(clippy::too_many_arguments)]
fn show_component_row(
    ui: &mut egui::Ui,
    doc: &Document,
    ci: crate::model::ComponentKey,
    depth: usize,
    visibility: &mut ElementVisibility,
    selection: &SceneSelection,
    health: &DocumentHealth,
    context: &HashSet<SceneElement>,
    related_constraints: &HashSet<crate::model::ConstraintKey>,
    style_selection: bool,
    highlight_elements: &HashSet<SceneElement>,
    rolled_back: &HashSet<SceneElement>,
    collapsed_components: &mut HashSet<crate::model::ComponentKey>,
    active_component: Option<crate::model::ComponentKey>,
    active_drawing: Option<crate::model::DrawingKey>,
    on_toggle_visibility: &mut impl FnMut(SceneElement, bool),
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_delete_element: &mut impl FnMut(SceneElement),
    clipboard_has_items: bool,
    clipboard_has_linkable: bool,
    on_copy: &mut impl FnMut(),
    on_paste: &mut impl FnMut(bool),
    on_add_component: &mut impl FnMut(Option<crate::model::ComponentKey>),
    on_move_to_component: &mut impl FnMut(SceneElement, Option<crate::model::ComponentKey>),
    on_export_component: &mut impl FnMut(crate::model::ComponentKey),
    on_export_component_step: &mut impl FnMut(crate::model::ComponentKey),
    on_export_component_3mf: &mut impl FnMut(crate::model::ComponentKey),
    on_add_to_drawing: &mut impl FnMut(SceneElement),
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
        if collapse_triangle(ui, collapsed, DISCLOSURE_BEFORE_EYE_WIDTH).clicked() {
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
        // The component icon selects the row like its name does (#964).
        let icon_response = ui
            .add(
                egui::Image::new(sized_texture(ui.ctx(), IconId::Component))
                    .tint(icon_tint_for_row_style(style)),
            )
            .interact(egui::Sense::click());
        let label = node_label(doc, HierarchyNode::Component(ci));
        // The active component (#429) — where new elements land — reads in the accent
        // color with a painted dot marker (#520).
        let text = if active_component == Some(ci) {
            active_marker_dot(ui);
            RichText::new(&label).color(crate::theme::FOCUS_ACCENT)
        } else {
            styled_label(&label, style)
        };
        let response = clipped_selectable_label(
            ui,
            row_shows_selection(&element, selection, style_selection),
            text,
            &label,
        );
        if RowClick::of(&response, ui)
            .or(RowClick::of(&icon_response, ui))
            .clicked
        {
            let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
            on_click_element(element.clone(), additive);
        }
        // With a drawing open, drag the component onto the page as a multi-body projection
        // (#1190). Otherwise rows re-parent into another component (#423).
        if active_drawing.is_some() {
            response
                .interact(egui::Sense::drag())
                .dnd_set_drag_payload(DrawingDragPayload(element.clone()));
            if let Some(icon_resp) = Some(&icon_response) {
                icon_resp
                    .clone()
                    .interact(egui::Sense::drag())
                    .dnd_set_drag_payload(DrawingDragPayload(element.clone()));
            }
        } else {
            response
                .interact(egui::Sense::drag())
                .dnd_set_drag_payload(ComponentDragPayload(element.clone()));
        }
        response.context_menu(|ui| {
            if ui.button("New component inside").clicked() {
                on_add_component(Some(ci));
                ui.close();
            }
            // In the Drawing workbench, project every body in the component as one view (#1190).
            if active_drawing.is_some() && ui.button("Add to drawing").clicked() {
                on_add_to_drawing(element.clone());
                ui.close();
            }
            // Export the component's bodies (#521): everything filed into it and its nested
            // components, as one STL/STEP file.
            if ui.button("Export STL…").clicked() {
                on_export_component(ci);
                ui.close();
            }
            if ui.button("Export 3MF…").clicked() {
                on_export_component_3mf(ci);
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
            // Copy / Paste (#1236).
            if ui.button("Copy").clicked() {
                on_copy();
                ui.close();
            }
            if clipboard_has_items && ui.button("Paste").clicked() {
                on_paste(false);
                ui.close();
            }
            if clipboard_has_linkable && ui.button("Paste Linked").clicked() {
                on_paste(true);
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
    // when a child covers the pointer. Disabled while a drawing is open (rows drag to the
    // page instead).
    let row_rect = row.response.rect;
    let dragging = active_drawing.is_none()
        && egui::DragAndDrop::has_payload_of_type::<ComponentDragPayload>(ui.ctx());
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
    expanded_units: &mut HashSet<crate::model::UnitInstanceKey>,
    visibility: &mut ElementVisibility,
    selection: &SceneSelection,
    health: &DocumentHealth,
    context: &HashSet<SceneElement>,
    related_constraints: &HashSet<crate::model::ConstraintKey>,
    style_selection: bool,
    on_edit_sketch: &mut impl FnMut(SketchId),
    on_edit_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_import_image_on_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_edit_extrusion: &mut impl FnMut(crate::model::ExtrusionKey),
    on_edit_edge_treatment: &mut impl FnMut(crate::model::ExtrusionKey, usize),
    on_edit_edge_treatment_op: &mut impl FnMut(crate::model::EdgeTreatmentOpKey),
    on_edit_operation: &mut impl FnMut(SceneElement),
    on_joint_rest: &mut impl FnMut(JointRestCommand),
    on_edit_drawing: &mut impl FnMut(crate::model::DrawingKey),
    on_select_drawing_element: &mut impl FnMut(HierarchyNode),
    on_hover_drawing_element: &mut impl FnMut(Option<HierarchyNode>),
    selected_drawing_leaf: Option<HierarchyNode>,
    on_rename_drawing: &mut impl FnMut(crate::model::DrawingKey, String),
    on_set_body_shadow: &mut impl FnMut(crate::model::BodyKey, bool),
    on_export_body: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_step: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_3mf: &mut impl FnMut(crate::model::BodyKey),
    on_set_rollback: &mut impl FnMut(Option<RollbackMarker>),
    on_toggle_visibility: &mut impl FnMut(SceneElement, bool),
    on_click_element: &mut impl FnMut(SceneElement, bool),
    on_hover_element: &mut impl FnMut(SceneElement),
    on_delete_element: &mut impl FnMut(SceneElement),
    on_clone_unit_instance: &mut impl FnMut(crate::model::UnitInstanceKey),
    clipboard_has_items: bool,
    clipboard_has_linkable: bool,
    on_copy: &mut impl FnMut(),
    on_paste: &mut impl FnMut(bool),
    active_drawing: Option<crate::model::DrawingKey>,
    on_add_to_drawing: &mut impl FnMut(SceneElement),
    on_create_drawing_of_body: &mut impl FnMut(crate::model::BodyKey),
    highlight_elements: &HashSet<SceneElement>,
    armed: Option<&crate::element_picker::ElementPicker>,
    rolled_back: &HashSet<SceneElement>,
    on_move_to_component: &mut impl FnMut(SceneElement, Option<crate::model::ComponentKey>),
    active_component: Option<crate::model::ComponentKey>,
    on_activate_component: &mut impl FnMut(Option<crate::model::ComponentKey>),
) {
    // The synthetic Document root has no SceneElement — it isn't selectable, hideable, or
    // otherwise dispatched through the scene graph — so it gets a minimal, always-shown row
    // and returns before any of the SceneElement-keyed lookups below. Every other row is
    // indented `depth` levels (List always passes 1, matching #87's original single level;
    // Tree passes the node's real depth in the nested hierarchy, #34).
    // A section header is drawn by `show_section_row` in the List path; Graph falls through
    // here with a plain label.
    if is_section_node(node) {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 18.0);
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let label = node_label(doc, node);
            let _ = clipped_selectable_label(
                ui,
                false,
                RichText::new(&label).strong(),
                &label,
            );
        });
        return;
    }
    if matches!(node, HierarchyNode::Document) {
        let row = ui.horizontal(|ui| {
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let active_root = active_component.is_none() && !doc.components.is_empty();
            if active_root {
                // With components present, mark where new elements land (#429): the
                // document root, unless a component is active. Painted dot, not a `●`
                // glyph, so it renders even when the font lacks that codepoint (#520).
                active_marker_dot(ui);
            }
            let label = node_label(doc, node);
            let text = if active_root {
                RichText::new(&label)
                    .color(crate::theme::FOCUS_ACCENT)
                    .strong()
            } else {
                RichText::new(&label).strong()
            };
            let resp = ui
                .add(egui::Label::new(text).truncate().sense(egui::Sense::click()))
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
            let label = node_label(doc, node);
            let response = clipped_selectable_label(ui, false, &label, &label);
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

    // A technical drawing (#180): a display-only leaf with no `SceneElement`. **Double**-click
    // the row (or right-click → "Edit drawing") to open the drawing pane — re-entering a
    // drawing is the same gesture as reopening a sketch (#1712), not a single click that
    // whisks you onto another workbench the moment you brush the row.
    if let HierarchyNode::Drawing(index) = node {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 18.0);
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(egui::Image::new(sized_texture(ui.ctx(), icon)));
            }
            let label = node_label(doc, node);
            let response = clipped_selectable_label(ui, false, &label, &label);
            if row_primary_double_clicked(&response, ui) {
                on_edit_drawing(index);
            }
            response
                .context_menu(|ui| drawing_context_menu(ui, doc, index, on_edit_drawing, on_rename_drawing));
        });
        return;
    }

    // An element inside an imported unit (#723): a display-only, read-only leaf. Clicking
    // explains itself instead of selecting — nothing in a unit can be edited from here.
    if let HierarchyNode::UnitChild { .. } = node {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 18.0);
            if let Some(icon) = icon_for_hierarchy_node(doc, node) {
                ui.add(
                    egui::Image::new(sized_texture(ui.ctx(), icon))
                        .tint(Color32::from_gray(140)),
                );
            }
            ui.add(
                egui::Label::new(
                    RichText::new(node_label(doc, node)).color(Color32::from_gray(150)),
                )
                .truncate(),
            )
            .on_hover_text("Part of an imported unit — read-only here; edit the source file");
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
            let label = node_label(doc, node);
            let resp = clipped_selectable_label(
                ui,
                selected_drawing_leaf == Some(node),
                &label,
                &label,
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
        // A unit instance row grows a collapse triangle (#723), like a component's: its
        // read-only contents expand beneath it in the List.
        if let HierarchyNode::UnitInstance(index) = node {
            let expanded = expanded_units.contains(&index);
            // Units keep the narrow disclosure in front of the eye; hover text differs.
            let tri = collapse_triangle(ui, !expanded, DISCLOSURE_BEFORE_EYE_WIDTH)
                .on_hover_text(if expanded {
                    "Collapse"
                } else {
                    "Look inside (read-only)"
                });
            if tri.clicked() {
                if expanded {
                    expanded_units.remove(&index);
                } else {
                    expanded_units.insert(index);
                }
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

        // The type icon selects the row just like its name does (#964), so it is sensed for
        // clicks rather than being inert decoration.
        let icon_response = icon_for_hierarchy_node(doc, node).map(|icon| {
            ui.add(
                egui::Image::new(sized_texture(ui.ctx(), icon))
                    .tint(icon_tint_for_row_style(style)),
            )
            .interact(egui::Sense::click())
        });

        let label = node_label(doc, node);
        let response = clipped_selectable_label(
            ui,
            row_shows_selection(&element, selection, style_selection),
            styled_label(&label, style),
            &label,
        );
        // A stale unit (#732): the embedded copy is behind the source file. An amber dot
        // says so; right-click → "Update from source file" picks the change up.
        if let HierarchyNode::UnitInstance(index) = node {
            let stale = doc
                .unit_instances
                .get(index)
                .is_some_and(|inst| health.stale_units.contains(&inst.unit));
            if stale {
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    dot.center(),
                    3.0,
                    crate::document_health::UNSTABLE_DISPLAY,
                );
                ui.allocate_rect(dot, egui::Sense::hover()).on_hover_text(
                    "The source file has changed — right-click → Update from source file",
                );
            }
        }
        // Both click targets of the row — its name and its type icon (#964) — feed one
        // pointer state, so a click, double-click, or hover on either drives the row.
        let row = RowClick::of(&response, ui).or(icon_response
            .as_ref()
            .map(|icon| RowClick::of(icon, ui))
            .unwrap_or_default());
        let row_rect = match icon_response.as_ref() {
            Some(icon) => icon.rect.union(response.rect),
            None => response.rect,
        };
        // Tutorial orb: first sketch row (#1279) / first body row (#1647) in Elements.
        let orb_row = match node {
            HierarchyNode::Sketch(_) => Some(elements_sketch_row_rect_id()),
            HierarchyNode::Body(_) => Some(elements_body_row_rect_id()),
            _ => None,
        };
        if let Some(id) = orb_row {
            ui.ctx().data_mut(|d| {
                if d.get_temp::<egui::Rect>(id).is_none() {
                    d.insert_temp(id, row_rect);
                }
            });
        }
        // The newest plane row (#1673): the one the user just built, not a datum.
        if matches!(node, HierarchyNode::ConstructionPlane(i)
            if Some(i) == doc.construction_planes.keys().last())
        {
            ui.ctx()
                .data_mut(|d| d.insert_temp(elements_plane_row_rect_id(), row_rect));
        }
        // Pane-hover → viewport highlight (#161): the 3D view shows what this row is.
        if row.hovered {
            on_hover_element(element.clone());
        }
        paint_pick_affordance(
            ui,
            doc,
            armed,
            &element,
            row.hovered,
            row_rect,
        );
        // With a drawing open, body, sketch, and component rows drag onto the page (#290/#1190):
        // the drop places a projection at the pointer. A cross-section view row drags too
        // (#1776): dropping it on a projection sections that view (and its aligned children).
        // Both the name label and the type icon are grab handles (#368). Re-sensed for drag so
        // the payload arms; plain clicks still select as usual.
        if active_drawing.is_some()
            && matches!(
                node,
                HierarchyNode::Body(_)
                    | HierarchyNode::Sketch(_)
                    | HierarchyNode::Component(_)
                    | HierarchyNode::CrossSection(_)
            )
        {
            response
                .interact(egui::Sense::drag())
                .dnd_set_drag_payload(DrawingDragPayload(element.clone()));
            if let Some(icon_resp) = icon_response.as_ref() {
                icon_resp
                    .clone()
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
            HierarchyNode::Document | HierarchyNode::Drawings | HierarchyNode::Views => {
                unreachable!("handled by the early return above")
            }
            HierarchyNode::Sketch(sketch) => {
                let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                match row_click_action(row.double_clicked, row.clicked, additive) {
                    RowAction::Edit => on_edit_sketch(sketch),
                    RowAction::Select { additive } => {
                        on_click_element(element.clone(), additive)
                    }
                    RowAction::None => {}
                }
            }
            // #1691: a plane reopens in the Plane tool the same way, so its offset and tilt
            // can be changed after the fact.
            HierarchyNode::ConstructionPlane(index) => {
                let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                match row_click_action(row.double_clicked, row.clicked, additive) {
                    RowAction::Edit => on_edit_plane(index),
                    RowAction::Select { additive } => {
                        on_click_element(element.clone(), additive)
                    }
                    RowAction::None => {}
                }
            }
            HierarchyNode::SectionPlane { view, cut } => {
                let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                match row_click_action(row.double_clicked, row.clicked, additive) {
                    RowAction::Edit => on_edit_operation(SceneElement::SectionPlane { view, cut }),
                    RowAction::Select { additive } => {
                        on_click_element(element.clone(), additive)
                    }
                    RowAction::None => {}
                }
            }
            HierarchyNode::Extrusion(index) => {
                if row.double_clicked {
                    on_edit_extrusion(index);
                } else if row.clicked {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
            HierarchyNode::EdgeTreatmentOp(index) => {
                if row.double_clicked {
                    on_edit_edge_treatment_op(index);
                } else if row.clicked {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
            // Handled by the early return above (no SceneElement).
            HierarchyNode::EdgeTreatment { .. } | HierarchyNode::Drawing(_) => unreachable!(),
            // Every other operation edits the universal way: double-click reopens it in its tool
            // (#546); a plain click selects it.
            node if node_editable_operation(node).is_some() => {
                if row.double_clicked {
                    on_edit_operation(element.clone());
                } else if row.clicked {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
            _ => {
                if row.clicked {
                    let additive = ui.input(|i| additive_click_modifiers(&i.modifiers));
                    on_click_element(element.clone(), additive);
                }
            }
        }
        // One context menu per element row: any node-specific actions, then a universal Delete
        // so any element can be deleted from the pane (#253). Shared with the Graph view (#623).
        response.context_menu(|ui| {
            element_context_menu(
                ui,
                doc,
                node,
                &element,
                active_drawing,
                on_edit_sketch,
                on_edit_plane,
                on_import_image_on_plane,
                on_edit_extrusion,
                on_edit_edge_treatment_op,
                on_edit_operation,
                on_joint_rest,
                on_add_to_drawing,
                on_create_drawing_of_body,
                on_set_body_shadow,
                on_export_body,
                on_export_body_step,
                on_export_body_3mf,
                on_move_to_component,
                on_set_rollback,
                on_delete_element,
                on_clone_unit_instance,
                clipboard_has_items,
                clipboard_has_linkable,
                crate::copy_paste::copyable_element(&element).is_some(),
                on_copy,
                on_paste,
            );
        });
    });
}

/// A technical drawing's context menu (#180/#255), shared by the List/Tree row and the
/// Graph view's node (#623): Edit drawing, plus the inline rename field (seeded from the
/// current name, held in egui temp memory while the menu is open).
fn drawing_context_menu(
    ui: &mut egui::Ui,
    doc: &Document,
    index: crate::model::DrawingKey,
    on_edit_drawing: &mut impl FnMut(crate::model::DrawingKey),
    on_rename_drawing: &mut impl FnMut(crate::model::DrawingKey, String),
) {
    if ui.button("Edit drawing").clicked() {
        on_edit_drawing(index);
        ui.close();
    }
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
}

/// The element context menu shared by the List/Tree rows, the Graph view's nodes (#623),
/// and a right-click on already-selected geometry in the 3D viewport (#1224):
/// node-specific actions (edit entries, drawing/export extras), Move-to-component (#423),
/// Rollback (#545), and the universal Delete (#253).
#[allow(clippy::too_many_arguments)]
pub(crate) fn element_context_menu(
    ui: &mut egui::Ui,
    doc: &Document,
    node: HierarchyNode,
    element: &SceneElement,
    active_drawing: Option<crate::model::DrawingKey>,
    on_edit_sketch: &mut impl FnMut(SketchId),
    on_edit_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_import_image_on_plane: &mut impl FnMut(crate::model::ConstructionPlaneKey),
    on_edit_extrusion: &mut impl FnMut(crate::model::ExtrusionKey),
    on_edit_edge_treatment_op: &mut impl FnMut(crate::model::EdgeTreatmentOpKey),
    on_edit_operation: &mut impl FnMut(SceneElement),
    on_joint_rest: &mut impl FnMut(JointRestCommand),
    on_add_to_drawing: &mut impl FnMut(SceneElement),
    on_create_drawing_of_body: &mut impl FnMut(crate::model::BodyKey),
    on_set_body_shadow: &mut impl FnMut(crate::model::BodyKey, bool),
    on_export_body: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_step: &mut impl FnMut(crate::model::BodyKey),
    on_export_body_3mf: &mut impl FnMut(crate::model::BodyKey),
    on_move_to_component: &mut impl FnMut(SceneElement, Option<crate::model::ComponentKey>),
    on_set_rollback: &mut impl FnMut(Option<RollbackMarker>),
    on_delete_element: &mut impl FnMut(SceneElement),
    // #1404: clone a unit instance (another instance of the same unit, same parameter overrides).
    on_clone_unit_instance: &mut impl FnMut(crate::model::UnitInstanceKey),
    // Copy / Paste / Paste Linked (#1236).
    clipboard_has_items: bool,
    clipboard_has_linkable: bool,
    element_is_copyable: bool,
    on_copy: &mut impl FnMut(),
    on_paste: &mut impl FnMut(bool),
) {
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
        HierarchyNode::SectionPlane { view, cut } => {
            if ui.button("Edit cutting plane").clicked() {
                on_edit_operation(SceneElement::SectionPlane { view, cut });
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
        HierarchyNode::CrossSection(view) => {
            // In the Drawing workbench, put the whole view on the open page (#1689).
            if active_drawing.is_some() && ui.button("Add to drawing").clicked() {
                on_add_to_drawing(SceneElement::CrossSection(view));
                ui.close();
            }
        }
        HierarchyNode::Body(index) => {
            // Immediately create a new drawing of this body (#1158) — same paths as CAD →
            // New Drawing + Add to drawing, without needing a drawing open first.
            if ui.button("Create drawing").clicked() {
                on_create_drawing_of_body(index);
                ui.close();
            }
            // In the Drawing workbench, add this body as a view of the open drawing (#274).
            if active_drawing.is_some() && ui.button("Add to drawing").clicked() {
                on_add_to_drawing(SceneElement::Body(index));
                ui.close();
            }
            // Manual shadow body (#1218): hide in the viewport and drop from export.
            let is_shadow = doc.bodies.get(index).is_some_and(|b| b.shadow);
            let shadow_label = if is_shadow {
                "Make live body"
            } else {
                "Make shadow body"
            };
            if ui.button(shadow_label).clicked() {
                on_set_body_shadow(index, !is_shadow);
                ui.close();
            }
            if ui.button("Export STL…").clicked() {
                on_export_body(index);
                ui.close();
            }
            if ui.button("Export 3MF…").clicked() {
                on_export_body_3mf(index);
                ui.close();
            }
            if ui.button("Export STEP…").clicked() {
                on_export_body_step(index);
                ui.close();
            }
        }
        // A unit instance (#732): update its unit's embedded copy from the source file
        // (every instance of the unit updates at once); routed through the universal
        // operation callback and dispatched in `begin_operation_edit`.
        HierarchyNode::UnitInstance(index) => {
            if ui.button("Update from source file").clicked() {
                on_edit_operation(SceneElement::UnitInstance(index));
                ui.close();
            }
            if ui
                .button(format!(
                    "Import another {}",
                    node_label(doc, HierarchyNode::UnitInstance(index))
                ))
                .clicked()
            {
                on_clone_unit_instance(index);
                ui.close();
            }
        }
        // A joint carries its rest pose (#898): capture it, go back to it, or send the
        // whole assembly home — alongside the universal Edit.
        HierarchyNode::Joint(index) => {
            if ui.button("Edit").clicked() {
                on_edit_operation(element.clone());
                ui.close();
            }
            if ui.button("Set rest to current position").clicked() {
                on_joint_rest(JointRestCommand::SetRest(index));
                ui.close();
            }
            if ui.button("Revert to rest position").clicked() {
                on_joint_rest(JointRestCommand::Revert(index));
                ui.close();
            }
            if ui.button("Revert all joints").clicked() {
                on_joint_rest(JointRestCommand::RevertAll);
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
    if component_member_node(node) && !doc.components.is_empty() {
        ui.menu_button("Move to", |ui| {
            if ui.button("Document").clicked() {
                on_move_to_component(element.clone(), None);
                ui.close();
            }
            for (ci, _c) in doc.components.iter() {
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
    if hierarchy_node_for_element(element).is_some() {
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
    // Copy / Paste / Paste Linked (#1236).
    if element_is_copyable && ui.button("Copy").clicked() {
        on_copy();
        ui.close();
    }
    if clipboard_has_items && ui.button("Paste").clicked() {
        on_paste(false);
        ui.close();
    }
    if clipboard_has_linkable && ui.button("Paste Linked").clicked() {
        on_paste(true);
        ui.close();
    }
    if ui.button("Delete").clicked() {
        on_delete_element(element.clone());
        ui.close();
    }
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
            | HierarchyNode::ShellOp(_)
            | HierarchyNode::Revolution(_)
            | HierarchyNode::SweepOp(_)
            | HierarchyNode::Loft(_)
    )
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::retain_ground_plane_only;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::sketch_key_for_slot as skey;
    use crate::model::constraint_key_for_slot as nkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::unit_key_for_slot as ukey;
    use crate::model::annotation_key_for_slot as akey;
    use crate::model::drawing_key_for_slot as dkey;
    use crate::model::component_key_for_slot as ckey;
    use crate::model::unit_instance_key_for_slot as uikey;
    use crate::model::body_key_for_slot as bkey;
    use crate::model::joint_key_for_slot as jkey;
    use crate::model::sketch_op_key_for_slot as skop;
    use crate::model::slice_op_key_for_slot as slckey;
    use crate::model::edge_treatment_op_key_for_slot as etkey;
    use crate::model::repeat_op_key_for_slot as repkey;
    use crate::model::mirror_op_key_for_slot as mirkey;
    use crate::model::move_op_key_for_slot as mopkey;
    use crate::model::boolean_op_key_for_slot as bopkey;
    use super::*;
    use crate::model::ShapeKind;

    /// #1193: projected sketch lines wear the Projection tool's icon in the Elements pane,
    /// not the plain line glyph.
    #[test]
    fn projected_line_uses_projection_icon_in_elements_pane() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let mut plain = crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        plain.construction = false;
        let plain_li = doc.lines.insert(plain);
        let mut projected = crate::model::Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0);
        projected.construction = true;
        projected.projection = Some(crate::model::ProjectionSource::Plane {
            plane: pkey(2),
        });
        let projected_li = doc.lines.insert(projected);

        assert_eq!(
            icon_for_hierarchy_node(&doc, HierarchyNode::Line(plain_li)),
            Some(IconId::Line),
            "drawn lines keep the line icon"
        );
        assert_eq!(
            icon_for_hierarchy_node(&doc, HierarchyNode::Line(projected_li)),
            Some(IconId::Project),
            "projected lines wear the Projection (projector) icon"
        );
    }

    /// #1549: tracing images wear the picture icon in the Elements pane, not the plane glyph.
    #[test]
    fn tracing_image_uses_image_icon_in_elements_pane() {
        let mut doc = Document::default();
        let image = doc.tracing_images.insert(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: pkey(0),
            origin: (0.0, 0.0),
            base_origin: None,
            width_mm: 100.0,
            height_mm: 60.0,
            opacity: crate::model::DEFAULT_TRACING_IMAGE_OPACITY,
            name: None,
            calibration: None,
            rotation: 0.0,
            base_rotation: None,
        });

        assert_eq!(
            icon_for_hierarchy_node(&doc, HierarchyNode::Image(image)),
            Some(IconId::Image),
            "tracing images wear the picture icon"
        );
        assert_eq!(
            icon_for_hierarchy_node(&doc, HierarchyNode::ConstructionPlane(pkey(0))),
            Some(IconId::Plane),
            "construction planes keep the plane icon"
        );
        assert_ne!(
            IconId::Image.svg_source(),
            IconId::Plane.svg_source(),
            "the picture icon is a distinct asset from the plane parallelogram"
        );
    }

    /// #1056: the two directions between a member and its scene element must agree. They used
    /// to be written out separately, and the "move this into that component" side had quietly
    /// dropped mirror ops and sweeps — you could see them grouped but not put them there.
    #[test]
    fn every_member_with_a_scene_element_can_be_moved_into_a_component() {
        use crate::model::ComponentMember as CM;
        let members = [
            CM::ConstructionPlane(pkey(1)),
            CM::Extrusion(xkey(1)),
            CM::Body(bkey(1)),
            CM::BooleanOp(bopkey(1)),
            CM::MoveOp(mopkey(1)),
            CM::MirrorOp(mirkey(1)),
            CM::RepeatOp(repkey(1)),
            CM::SliceOp(slckey(1)),
            CM::EdgeTreatmentOp(etkey(1)),
            CM::Revolution(crate::arena::Key::from_bits(1)),
            CM::Sweep(crate::arena::Key::from_bits(1)),
            CM::Loft(crate::arena::Key::from_bits(1)),
        ];
        for member in members {
            let element = component_member_element(member)
                .unwrap_or_else(|| panic!("{member:?} names a scene element"));
            assert_eq!(component_member_for_element(&element), Some(member));
        }
    }

    /// #1224: right-clicking already-selected geometry in the viewport opens the same
    /// context menu as that element's Elements-pane row. Body sub-picks count as the body
    /// when the body is selected; unselected picks never open the menu.
    /// #1630: the construction plane's context menu — the one a viewport right-click on a
    /// plane now opens — offers importing a tracing image onto that plane.
    #[test]
    fn construction_plane_context_menu_offers_importing_an_image() {
        let doc = Document::default();
        let index = doc.construction_planes.keys().next().expect("a datum plane");
        let element = SceneElement::ConstructionPlane(index);
        let mut imported_on = None;
        let mut texts: Vec<String> = Vec::new();
        let output = egui::Context::default().run_ui(Default::default(), |ui| {
            {
                element_context_menu(
                    ui,
                    &doc,
                    HierarchyNode::ConstructionPlane(index),
                    &element,
                    None,
                    &mut |_| {},
                    &mut |_| {},
                    &mut |plane| imported_on = Some(plane),
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_, _| {},
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_, _| {},
                    &mut |_| {},
                    &mut |_| {},
                    &mut |_| {},
                    false,
                    false,
                    false,
                    &mut || {},
                    &mut |_| {},
                );
            }
        });
        collect_shape_text(&output.shapes, &mut texts);
        assert!(
            texts.iter().any(|t| t == "Import image on this plane…"),
            "a plane's menu should offer importing an image onto it, got {texts:?}"
        );
        assert!(imported_on.is_none(), "nothing was clicked, so nothing imported");
    }

    /// Every string painted by a rendered frame, so a menu's items can be asserted on.
    fn collect_shape_text(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        for clipped in shapes {
            walk(&clipped.shape, out);
        }
    }

    #[test]
    fn selected_context_menu_element_matches_elements_pane_target() {
        let body = SceneElement::Body(bkey(0));
        let face = SceneElement::BodyFace {
            body: bkey(0),
            centroid: [0, 0, 0],
            normal: [0, 0, 1],
        };
        let edge = SceneElement::BodyEdge {
            body: bkey(0),
            a: [0, 0, 0],
            b: [1000, 0, 0],
        };
        let other_body = SceneElement::Body(bkey(1));
        let line = SceneElement::Line(lkey(0));
        let plane = SceneElement::ConstructionPlane(pkey(0));

        let mut sel = SceneSelection::default();
        // Nothing selected → no menu, even on a solid pick.
        assert_eq!(selected_context_menu_element(&body, &sel), None);
        assert_eq!(selected_context_menu_element(&face, &sel), None);

        sel.insert(body.clone());
        // Exact body pick while selected.
        assert_eq!(
            selected_context_menu_element(&body, &sel),
            Some(body.clone())
        );
        // Face/edge of the selected body open the body's menu.
        assert_eq!(
            selected_context_menu_element(&face, &sel),
            Some(body.clone())
        );
        assert_eq!(
            selected_context_menu_element(&edge, &sel),
            Some(body.clone())
        );
        // A different body under the cursor does not inherit the selection.
        assert_eq!(selected_context_menu_element(&other_body, &sel), None);
        // Sub-element only (no hierarchy row of its own) with the body unselected → none.
        sel.clear();
        sel.insert(face.clone());
        assert_eq!(
            selected_context_menu_element(&face, &sel),
            None,
            "a face has no Elements-pane row of its own"
        );

        // Hierarchy rows that are selected open for themselves.
        sel.clear();
        sel.insert(line.clone());
        assert_eq!(selected_context_menu_element(&line, &sel), Some(line));
        sel.clear();
        sel.insert(plane.clone());
        assert_eq!(selected_context_menu_element(&plane, &sel), Some(plane));
    }

    /// #977: an operation, a component and a joint have no shape of their own in the 3D view,
    /// so hovering their Elements-pane rows lights what they **made** instead. This is the
    /// mapping that finds it.
    #[test]
    fn produced_bodies_finds_what_a_row_made() {
        use crate::model::{Body, BodySource, Component, ComponentMember, JointKind, JointRef};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let lines =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 5.0, [false; 4]);
        doc.extrusions.insert(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(lines.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        let live = |source| Body {
            source,
            material: None,
            name: None,
            shadow: false,
        };
        doc.bodies.insert(live(BodySource::Extrusion(xkey(0)))); // body 0
        doc.bodies.insert(live(BodySource::Extrusion(xkey(0)))); // body 1
        // A consumed input, which is not an output of anything the user can point at.
        doc.bodies.insert(Body {
            shadow: true,
            ..live(BodySource::Extrusion(xkey(0)))
        });

        assert_eq!(
            produced_bodies(&doc, &SceneElement::Extrusion(xkey(0))),
            vec![bkey(0), bkey(1)],
            "an operation's outputs, and not the input it consumed"
        );

        // A component lights every body under it.
        doc.components.insert(Component {
            name: None,
            parent: None,
            length_unit: None,
            angle_unit: None,
        });
        doc.component_members.push((ComponentMember::Body(bkey(1)), ckey(0)));
        assert_eq!(produced_bodies(&doc, &SceneElement::Component(ckey(0))), vec![bkey(1)]);

        // A joint has no descendants at all — what it holds together is the answer.
        doc.joints.insert(crate::model::Joint {
            members: vec![JointRef::Body(bkey(0)), JointRef::Body(bkey(1))],
            base: 0,
            kind: JointKind::Rigid,
            placement: Default::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: crate::model::JointLimits::default(),
            name: None,
            frame: Default::default(),
        });
        assert_eq!(produced_bodies(&doc, &SceneElement::Joint(jkey(0))), vec![bkey(0), bkey(1)]);

        // A body isn't an operation; it has nothing downstream to stand in for it.
        assert!(produced_bodies(&doc, &SceneElement::Body(bkey(0))).is_empty());
    }

    #[test]
    fn a_row_reacts_when_either_of_its_click_targets_does() {
        // #964: the name label and the type icon are both click targets, so the row selects
        // whichever one the pointer landed on.
        let label = RowClick {
            clicked: true,
            ..RowClick::default()
        };
        let icon = RowClick {
            clicked: true,
            double_clicked: true,
            hovered: true,
        };
        assert!(RowClick::default().or(icon).clicked, "the icon alone selects");
        assert!(
            RowClick::default().or(icon).double_clicked,
            "the icon alone edits on a double click"
        );
        assert!(
            RowClick::default().or(icon).hovered,
            "hovering the icon highlights the row's element in the viewport"
        );
        assert!(label.or(RowClick::default()).clicked, "the label alone still selects");
        assert_eq!(
            RowClick::default().or(RowClick::default()),
            RowClick::default(),
            "an untouched row reports nothing"
        );
    }

    /// A document with one imported unit (a sketch + a body inside) and one instance (#723).
    fn doc_with_unit_instance() -> Document {
        let mut inner = Document::default();
        let sketch = inner.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        crate::construction::add_line_rectangle(&mut inner, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        inner.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: Some("Inner body".to_string()),
            shadow: false,
        });
        let mut doc = Document::default();
        doc.units.insert(crate::model::ImportedUnit {
            source: crate::model::UnitSource::RelativePath("a.bearcad".to_string()),
            link: crate::model::LinkMode::Static,
            document: inner,
            source_mtime: None,
            source_hash: None,
        });
        doc.unit_instances.insert(crate::model::UnitInstance {
            unit: ukey(0),
            name: Some("bracket".to_string()),
            parameter_overrides: Vec::new(),
            placement: crate::model::UnitPlacement::default(),
        });
        doc
    }

    /// #723: an instance is one top-level row; its children are display-only leaves with
    /// no scene identity — the single gate every selection/visibility/mutation dispatch
    /// goes through — so nothing inside a unit can be targeted by a mutating action.
    #[test]
    fn unit_instance_is_one_row_and_its_children_are_read_only() {
        let doc = doc_with_unit_instance();
        let tree = build_hierarchy(&doc, None);
        let instance_entry = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::UnitInstance(uikey(0)))
            .expect("the instance is a top-level row");
        assert!(!instance_entry.children.is_empty(), "the contents expand beneath it");
        for child in &instance_entry.children {
            assert!(
                matches!(child.node, HierarchyNode::UnitChild { .. }),
                "every child is a read-only unit-content leaf: {:?}",
                child.node
            );
            assert_eq!(
                scene_element_for_node(child.node),
                None,
                "no scene identity → unaddressable by selection or any mutating action"
            );
        }
        // The row itself is a real element: selectable, nameable, hideable.
        assert_eq!(
            scene_element_for_node(HierarchyNode::UnitInstance(uikey(0))),
            Some(SceneElement::UnitInstance(uikey(0)))
        );
        // The child labels come from the unit's own document.
        let labels: Vec<String> = unit_child_rows(&doc, uikey(0)).into_iter().map(|(_, l)| l).collect();
        assert!(labels.iter().any(|l| l == "Inner body"), "{labels:?}");
    }

    /// #723: the node graph hides a unit's contents by default — the "Unit contents"
    /// filter toggle (default off) lets them in; the prune never promotes them.
    #[test]
    fn graph_filter_hides_unit_children_by_default() {
        let doc = doc_with_unit_instance();
        let filter = ElementFilter::default();
        assert!(!filter.unit_contents, "unit contents are off by default");
        let mut tree = filter_hierarchy(&build_hierarchy(&doc, None), &filter);
        prune_unit_children(&mut tree);
        fn any_unit_child(entries: &[HierarchyEntry]) -> bool {
            entries.iter().any(|e| {
                matches!(e.node, HierarchyNode::UnitChild { .. }) || any_unit_child(&e.children)
            })
        }
        assert!(!any_unit_child(&tree), "pruned tree has no unit contents anywhere");
        // The instance row itself survives the prune.
        assert!(tree[0]
            .children
            .iter()
            .any(|e| e.node == HierarchyNode::UnitInstance(uikey(0))));
    }





    /// #448/#449: every operation's inputs appear as graph dependency edges — the
    /// repeat's input body was the reported gap.
    #[test]
    fn graph_dependency_edges_cover_operation_inputs() {
        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        doc.repeat_ops.insert(crate::model::RepeatOperation {
            targets: vec![bkey(0)],
            plane_targets: vec![pkey(0)],
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            axis: crate::model::RevolveAxis::X,
            path_circle: None,
            around_axis: false,
            flip: false,
            mode: crate::model::RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            name: None,
        });
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let rev = doc.revolutions.insert(crate::model::Revolution {
            sketch,
            faces: Vec::new(),
            axis: crate::model::RevolveAxis::Line(lkey(0)),
            angle_deg: 360.0,
            angle_expression: String::new(),
            angle_is_revolutions: false,
            pitch_mm: 0.0,
            pitch_expression: String::new(),
            gap_is_offset: true,
            symmetric: false,
            mode: crate::model::RevolveMode::NewBody,
            name: None,
        });
        let edges = graph_dependency_edges(&doc);
        assert!(edges.contains(&(HierarchyNode::Body(bkey(0)), HierarchyNode::RepeatOp(repkey(0)))));
        assert!(edges.contains(&(
            HierarchyNode::ConstructionPlane(pkey(0)),
            HierarchyNode::RepeatOp(repkey(0))
        )));
        assert!(edges.contains(&(HierarchyNode::Sketch(sketch), HierarchyNode::Revolution(rev))));
        assert!(edges.contains(&(HierarchyNode::Line(lkey(0)), HierarchyNode::Revolution(rev))));
    }

    /// #1324: a fillet already fed by its input body must not also draw a Document parent
    /// spoke — those extra root lines were the reported clutter.
    #[test]
    fn graph_omits_document_edge_when_an_element_has_other_inputs() {
        use crate::model::{
            Body, BodySource, EdgeTreatmentOperation, ExtrudeFace, Extrusion, TreatedEdge,
            VertexTreatmentKind,
        };
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let rect =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        let host = doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: true,
        });
        let op = doc.edge_treatment_ops.insert(EdgeTreatmentOperation {
            targets: vec![host],
            edges: vec![TreatedEdge {
                target: 0,
                solid: crate::model::TreatableSolid::Extrusion(xkey(0)),
                edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            }],
            kind: VertexTreatmentKind::Fillet,
            amount: 1.5,
            expression: String::new(),
            outputs: Vec::new(),
            name: None,
        });
        let output = doc.bodies.insert(Body {
            source: BodySource::EdgeTreated {
                op,
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.edge_treatment_ops[op].outputs = vec![output];

        let tree = build_hierarchy(&doc, None);
        let positions = graph_node_positions(&tree);
        let fillet = HierarchyNode::EdgeTreatmentOp(op);
        let parents = graph_parent_edges(&positions, &doc);
        assert!(
            !parents
                .iter()
                .any(|(p, c)| *p == HierarchyNode::Document && *c == fillet),
            "fillet has Body as an input, so it must not also speak to Document"
        );
        // World planes have no other inputs — they keep the Document spoke.
        assert!(
            parents.iter().any(|(p, c)| *p == HierarchyNode::Document
                && matches!(c, HierarchyNode::ConstructionPlane(_))),
            "a root plane still hangs off Document"
        );
        assert!(graph_dependency_edges(&doc).contains(&(HierarchyNode::Body(host), fillet)));
        // The tree still nests the fillet under Document (list/tree unchanged).
        assert_eq!(
            positions.iter().find(|p| p.node == fillet).and_then(|p| p.parent),
            Some(HierarchyNode::Document)
        );
    }

    /// #423: assigned roots nest under their component entry in the built hierarchy, and the
    /// List rows indent them one level (skipping contents when collapsed).
    #[test]
    fn components_group_roots_in_hierarchy_and_list() {
        use crate::model::ComponentMember as CM;
        let mut doc = Document::default();
        doc.components.insert(crate::model::Component {
            name: Some("Frame".to_string()),
            parent: None,
            length_unit: None,
            angle_unit: None,
        });
        let plane = doc.construction_planes.insert(crate::face::default_xy_plane());
        doc.set_component_member(CM::ConstructionPlane(plane), Some(ckey(0)));

        let tree = build_hierarchy(&doc, None);
        let root = &tree[0];
        let comp = root
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Component(ckey(0)))
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
        let rows = component_list_rows(&tree, &doc, &HashSet::new(), SectionCollapse::default());
        let comp_row = rows.iter().find(|(n, _)| *n == HierarchyNode::Component(ckey(0))).unwrap();
        assert_eq!(comp_row.1, 1);
        let plane_row = rows
            .iter()
            .find(|(n, _)| *n == HierarchyNode::ConstructionPlane(plane))
            .unwrap();
        assert_eq!(plane_row.1, 2, "component contents indent one level");
        let collapsed: HashSet<crate::model::ComponentKey> = [ckey(0)].into_iter().collect();
        let rows = component_list_rows(&tree, &doc, &collapsed, SectionCollapse::default());
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
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y]],
            source_name: "part".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let mut visibility = ElementVisibility::default();
        assert!(visibility.effective_visible(&doc, SceneElement::Body(bkey(0))));
        visibility.set_visible(SceneElement::Body(bkey(0)), false);
        assert!(!visibility.effective_visible(&doc, SceneElement::Body(bkey(0))));
    }

    /// #667: hiding a construction plane hides the plane itself, **not** the sketches drawn on
    /// it — you're putting the plane's display quad away, not the geometry. Sketches on a body
    /// face still follow that body, which genuinely stops existing when it's hidden.
    #[test]
    fn hiding_a_construction_plane_keeps_its_sketches_visible() {
        use crate::model::FaceId;
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));

        let mut visibility = ElementVisibility::default();
        assert!(visibility.effective_visible(&doc, SceneElement::Sketch(sketch)));
        visibility.set_visible(SceneElement::ConstructionPlane(pkey(0)), false);
        assert!(
            !visibility.effective_visible(&doc, SceneElement::ConstructionPlane(pkey(0))),
            "the plane itself goes away"
        );
        assert!(
            visibility.effective_visible(&doc, SceneElement::Sketch(sketch)),
            "its sketch stays"
        );
        assert!(
            visibility.effective_visible(&doc, SceneElement::Line(lkey(0))),
            "and so does the geometry in it"
        );
        // Hiding the sketch itself still works.
        visibility.set_visible(SceneElement::Sketch(sketch), false);
        assert!(!visibility.effective_visible(&doc, SceneElement::Sketch(sketch)));
        assert!(!visibility.effective_visible(&doc, SceneElement::Line(lkey(0))));
    }

    /// #667: only the plane's **own** hidden flag is skipped. A plane anchored to a sketch
    /// still disappears with that sketch, and so does anything drawn on it — otherwise hiding
    /// a sketch would stop hiding its descendants.
    #[test]
    fn a_plane_anchored_to_a_hidden_sketch_takes_its_sketches_with_it() {
        use crate::model::{ConstructionPlaneParent, FaceId};
        let mut doc = Document::default();
        // Just the ground plane: this test indexes planes by hand, so the other two datum
        // planes (#833) would only shift the numbers.
        retain_ground_plane_only(&mut doc);
        let base = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        // A plane anchored to that sketch, with a second sketch drawn on it.
        let mut plane = doc.construction_planes[pkey(0)].clone();
        plane.parent = ConstructionPlaneParent::Sketch(base);
        doc.construction_planes.insert(plane);
        let on_top = doc.add_sketch(FaceId::ConstructionPlane(pkey(1)));

        let mut visibility = ElementVisibility::default();
        assert!(visibility.effective_visible(&doc, SceneElement::Sketch(on_top)));

        // Hiding the anchored plane itself leaves the sketch on it alone (#667).
        visibility.set_visible(SceneElement::ConstructionPlane(pkey(1)), false);
        assert!(visibility.effective_visible(&doc, SceneElement::Sketch(on_top)));

        // Hiding the sketch the plane hangs off takes the whole chain with it.
        visibility.set_visible(SceneElement::Sketch(base), false);
        assert!(!visibility.effective_visible(&doc, SceneElement::ConstructionPlane(pkey(1))));
        assert!(!visibility.effective_visible(&doc, SceneElement::Sketch(on_top)));
    }

    /// #266: a boolean operation's shadow input bodies feed it as dependency edges in the graph.
    #[test]
    fn boolean_op_inputs_are_graph_dependencies() {
        let mut doc = Document::default();
        for _ in 0..3 {
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
                material: None,
                name: None,
                shadow: false,
            });
        }
        doc.boolean_ops.insert(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![bkey(0)],
            b: vec![bkey(1)],
            keep_b: false,
            outputs: vec![bkey(2)],
            name: None,
        });
        let edges = graph_dependency_edges(&doc);
        assert!(edges.contains(&(HierarchyNode::Body(bkey(0)), HierarchyNode::BooleanOp(bopkey(0)))));
        assert!(edges.contains(&(HierarchyNode::Body(bkey(1)), HierarchyNode::BooleanOp(bopkey(0)))));
        // The output body is a tree child, not a dependency input.
        assert!(!edges.contains(&(HierarchyNode::Body(bkey(2)), HierarchyNode::BooleanOp(bopkey(0)))));
    }

    /// Visible graph nodes after the pane's default filter — shadows pruned (#1109).
    fn default_graph_present(doc: &Document) -> (Vec<HierarchyEntry>, HashSet<HierarchyNode>) {
        let filter = ElementFilter::default();
        let mut tree = filter_hierarchy(&build_hierarchy(doc, None), &filter);
        prune_shadow_bodies(&mut tree, doc);
        if !filter.unit_contents {
            prune_unit_children(&mut tree);
        }
        let present = graph_node_positions(&tree).into_iter().map(|p| p.node).collect();
        (tree, present)
    }

    /// #1425: a Move that consumes a hidden shadow body draws a dashed skip-edge from the
    /// shadow's parent (the Shape) to the Move — the shadow itself is not on screen.
    #[test]
    fn hidden_shadow_dependency_skips_to_the_shadows_parent() {
        use crate::model::{Body, BodySource, MoveOperation, Primitive, PrimitiveKind};
        let mut doc = Document::default();
        let shape = doc.primitives.insert(Primitive::new(PrimitiveKind::Cuboid));
        let shadow = doc.bodies.insert(Body {
            source: BodySource::Primitive(shape),
            material: None,
            name: None,
            shadow: true,
        });
        let live = doc.bodies.insert(Body {
            source: BodySource::Moved {
                op: mopkey(0),
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.move_ops.insert(MoveOperation {
            targets: vec![shadow],
            outputs: vec![live],
            ..MoveOperation::default()
        });

        let (_tree, present) = default_graph_present(&doc);
        assert!(
            !present.contains(&HierarchyNode::Body(shadow)),
            "the consumed cuboid body is a hidden shadow"
        );
        assert!(present.contains(&HierarchyNode::Shape(shape)));
        assert!(present.contains(&HierarchyNode::MoveOp(mopkey(0))));

        let skips = graph_shadow_skip_edges(&doc, &present);
        assert!(
            skips.contains(&(HierarchyNode::Shape(shape), HierarchyNode::MoveOp(mopkey(0)))),
            "hidden shadow's parent (the cuboid) should dash to the move that consumed it: {skips:?}"
        );
        // The ordinary dependency still names the hidden body — the renderer skips that pair.
        assert!(graph_dependency_edges(&doc)
            .contains(&(HierarchyNode::Body(shadow), HierarchyNode::MoveOp(mopkey(0)))));

        // With shadows shown, the ordinary Body→Move dash is enough; no skip.
        let all_present: HashSet<HierarchyNode> = graph_node_positions(&build_hierarchy(&doc, None))
            .into_iter()
            .map(|p| p.node)
            .collect();
        assert!(all_present.contains(&HierarchyNode::Body(shadow)));
        assert!(
            graph_shadow_skip_edges(&doc, &all_present).is_empty(),
            "visible shadows use the ordinary dependency edge, not a skip"
        );
    }

    /// #1425: a fillet whose host body is a hidden shadow skips from that body's parent
    /// (the extrusion) to the fillet — otherwise the fillet floats off Document.
    #[test]
    fn hidden_shadow_host_skips_from_extrusion_to_fillet() {
        use crate::model::{
            Body, BodySource, EdgeTreatmentOperation, ExtrudeFace, Extrusion, TreatedEdge,
            VertexTreatmentKind,
        };
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let rect =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        let host = doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: true,
        });
        let op = doc.edge_treatment_ops.insert(EdgeTreatmentOperation {
            targets: vec![host],
            edges: vec![TreatedEdge {
                target: 0,
                solid: crate::model::TreatableSolid::Extrusion(xkey(0)),
                edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            }],
            kind: VertexTreatmentKind::Fillet,
            amount: 1.5,
            expression: String::new(),
            outputs: Vec::new(),
            name: None,
        });
        let output = doc.bodies.insert(Body {
            source: BodySource::EdgeTreated {
                op,
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.edge_treatment_ops[op].outputs = vec![output];

        let (_tree, present) = default_graph_present(&doc);
        assert!(!present.contains(&HierarchyNode::Body(host)));
        let skips = graph_shadow_skip_edges(&doc, &present);
        assert!(
            skips.contains(&(HierarchyNode::Extrusion(xkey(0)), HierarchyNode::EdgeTreatmentOp(op))),
            "fillet should dash to the extrusion that produced its hidden shadow host: {skips:?}"
        );
    }

    /// #1425: a hidden *live* (non-shadow) source does not invent a skip — only shadows do.
    #[test]
    fn hidden_live_body_does_not_skip() {
        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: false,
        });
        doc.boolean_ops.insert(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![bkey(0)],
            b: Vec::new(),
            keep_b: false,
            outputs: Vec::new(),
            name: None,
        });
        // Pretend the live input is off-screen (bodies filter off) while the op stays.
        let present: HashSet<HierarchyNode> =
            [HierarchyNode::Document, HierarchyNode::BooleanOp(bopkey(0))]
                .into_iter()
                .collect();
        assert!(
            graph_shadow_skip_edges(&doc, &present).is_empty(),
            "a hidden live body is not a shadow skip"
        );
    }

    /// #281: each placed drawing view is a "projection" child of its drawing node, labelled by
    /// its source body and orientation.
    #[test]
    fn drawing_views_nest_as_projections_under_the_drawing() {
        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: Some("Plate".to_string()),
            shadow: false,
        });
        doc.drawings.insert(crate::model::Drawing {
            views: vec![crate::model::DrawingView {
                cross_section: None,
                bodies: vec![bkey(0)],
                sketch: None,
                orientation: crate::model::DrawingOrientation::Front,
                dimensioned_edges: Vec::new(),
                angle_dims: Vec::new(),
                dimension_offsets: Vec::new(),
                dimensioned_circles: Vec::new(), dimensioned_curves: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(),
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
                size_x: crate::drawing::CELL_FRAC,
                size_y: crate::drawing::CELL_FRAC,
            }],
            ..Default::default()
        });

        let tree = build_hierarchy(&doc, None);
        // Document -> Drawings -> Drawing(0) -> DrawingProjection { drawing: dkey(0), view: 0 }.
        let drawings = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawings)
            .expect("Drawings section present");
        let drawing = drawings
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawing(dkey(0)))
            .expect("drawing node present");
        assert_eq!(
            drawing.children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![HierarchyNode::DrawingProjection { drawing: dkey(0), view: 0 }]
        );
        assert_eq!(
            node_label(&doc, HierarchyNode::DrawingProjection { drawing: dkey(0), view: 0 }),
            "Plate — Front"
        );
    }

    /// #341: a projection's shown dimensions appear as `DrawingDimension` children nested under it.
    #[test]
    fn drawing_dimensions_nest_under_their_projection() {
        let mut doc = Document::default();
        let a = crate::hierarchy::quantize_body_point(glam::Vec3::ZERO);
        let b = crate::hierarchy::quantize_body_point(glam::Vec3::new(40.0, 0.0, 0.0));
        doc.drawings.insert(crate::model::Drawing {
            views: vec![crate::model::DrawingView {
                cross_section: None,
                bodies: vec![bkey(0)],
                sketch: None,
                orientation: crate::model::DrawingOrientation::Front,
                dimensioned_edges: vec![(a, b)],
                angle_dims: Vec::new(),
                dimension_offsets: Vec::new(),
                dimensioned_circles: Vec::new(), dimensioned_curves: Vec::new(),
circle_dim_offsets: Vec::new(), point_dims: Vec::new(),
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
                size_x: crate::drawing::CELL_FRAC,
                size_y: crate::drawing::CELL_FRAC,
            }],
            ..Default::default()
        });
        let tree = build_hierarchy(&doc, None);
        let drawing = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawings)
            .expect("Drawings section")
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawing(dkey(0)))
            .expect("drawing node");
        let projection = drawing
            .children
            .iter()
            .find(|e| matches!(e.node, HierarchyNode::DrawingProjection { .. }))
            .expect("projection node");
        assert_eq!(
            projection.children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![HierarchyNode::DrawingDimension { drawing: dkey(0), view: 0, a, b }],
            "the dimension nests under its projection"
        );
    }

    /// #333: a drawing's text notes appear as `DrawingAnnotation` children under the drawing,
    /// after its projections, labelled by their text.
    #[test]
    fn drawing_annotations_show_as_hierarchy_children() {
        let mut doc = Document::default();
        doc.drawings.insert(crate::model::Drawing {
            annotations: crate::arena::Arena::from_iter([crate::model::DrawingAnnotation {
                text: "Scale 1:2".to_string(),
                pos_x: 0.05,
                pos_y: 0.05,
                size_frac: 0.03,
                wrap_frac: None,
            }]),
            ..Default::default()
        });
        let tree = build_hierarchy(&doc, None);
        let drawing = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawings)
            .expect("Drawings section")
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Drawing(dkey(0)))
            .expect("drawing node present");
        assert!(
            drawing
                .children
                .iter()
                .any(|c| c.node == HierarchyNode::DrawingAnnotation { drawing: dkey(0), annotation: akey(0) }),
            "the text note is a child of the drawing"
        );
        assert_eq!(
            node_label(&doc, HierarchyNode::DrawingAnnotation { drawing: dkey(0), annotation: akey(0) }),
            "Text: Scale 1:2"
        );
    }

    /// #1205: once a document has a drawing, unassigned drawings live under a collapsible
    /// Drawings section at the bottom of Document — never interleaved with bodies/ops.
    #[test]
    fn drawings_section_groups_drawings_at_document_bottom() {
        let mut doc = Document::default();
        // A body that would otherwise appear after a mid-list drawing under the old order.
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: Some("Part".to_string()),
            shadow: false,
        });
        doc.drawings.insert(crate::model::Drawing::default());
        doc.drawings.insert(crate::model::Drawing {
            name: Some("Sheet".to_string()),
            ..Default::default()
        });

        let tree = build_hierarchy(&doc, None);
        let root_children: Vec<HierarchyNode> =
            tree[0].children.iter().map(|c| c.node).collect();
        assert!(
            !root_children.iter().any(|n| matches!(n, HierarchyNode::Drawing(_))),
            "drawings are not loose Document children: {root_children:?}"
        );
        let drawings_idx = root_children
            .iter()
            .position(|n| *n == HierarchyNode::Drawings)
            .expect("Drawings section present");
        assert_eq!(
            drawings_idx,
            root_children.len() - 1,
            "Drawings section is the last Document child: {root_children:?}"
        );
        let section = &tree[0].children[drawings_idx];
        assert_eq!(
            section.children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![HierarchyNode::Drawing(dkey(0)), HierarchyNode::Drawing(dkey(1))],
            "both drawings nest under the section in index order"
        );
        assert_eq!(node_label(&doc, HierarchyNode::Drawings), "Drawings");

        // No drawings → no section.
        let empty = build_hierarchy(&Document::default(), None);
        assert!(
            !empty[0]
                .children
                .iter()
                .any(|c| c.node == HierarchyNode::Drawings),
            "empty document has no Drawings section"
        );

        // List: section last; drawings one level under it; collapse hides them.
        let rows = component_list_rows(&tree, &doc, &HashSet::new(), SectionCollapse::default());
        let section_pos = rows
            .iter()
            .position(|(n, _)| *n == HierarchyNode::Drawings)
            .expect("section row");
        assert!(
            section_pos == rows.len() - 1
                || rows[section_pos + 1..]
                    .iter()
                    .all(|(n, _)| matches!(
                        n,
                        HierarchyNode::Drawing(_)
                            | HierarchyNode::DrawingProjection { .. }
                            | HierarchyNode::DrawingAnnotation { .. }
                            | HierarchyNode::DrawingDimension { .. }
                    )),
            "nothing non-drawing follows the Drawings section: {rows:?}"
        );
        let body_pos = rows
            .iter()
            .position(|(n, _)| *n == HierarchyNode::Body(bkey(0)))
            .expect("body row");
        assert!(
            body_pos < section_pos,
            "body appears before the Drawings section: body={body_pos} section={section_pos}"
        );
        let d0 = rows
            .iter()
            .find(|(n, _)| *n == HierarchyNode::Drawing(dkey(0)))
            .expect("drawing 0 row");
        assert_eq!(d0.1, 2, "drawings indent under the section");
        let collapsed_rows = component_list_rows(&tree, &doc, &HashSet::new(), SectionCollapse { drawings: true, views: true });
        assert!(
            !collapsed_rows
                .iter()
                .any(|(n, _)| matches!(n, HierarchyNode::Drawing(_))),
            "collapsed section hides its drawings"
        );
        assert!(
            collapsed_rows
                .iter()
                .any(|(n, _)| *n == HierarchyNode::Drawings),
            "collapsed section still shows the header"
        );
    }

    /// #1282: Elements filter bottom panel is allocated *before* the list ScrollArea.
    /// The reverse order forced multipass re-layout that renumbered widget ids on the whole
    /// pane (full-height red flash: "Widget rect … changed id between passes"). List-row
    /// button ids under the explicit scroll salt must stay put across multipass frames.
    #[test]
    fn elements_filter_before_list_keeps_widget_ids_stable() {
        let ctx = egui::Context::default();
        ctx.options_mut(|o| {
            o.max_passes = std::num::NonZeroUsize::new(2).unwrap();
        });
        let mut ids = Vec::new();
        for _frame in 0..6 {
            let mut captured = None;
            let mut filter = ElementFilter::default();
            let mut expanded = false;
            let _ = ctx.run_ui(Default::default(), |ui| {
                egui::Panel::left("tree").default_size(220.0).show(ui, |ui| {
                    ui.scope_builder(
                        egui::UiBuilder::new().id(egui::Id::new(("pane_contents", "tree"))),
                        |ui| {
                            ui.heading("Elements");
                            ui.separator();
                            // Production order: filter panel first, then list content.
                            super::show_elements_filter(ui, &mut filter, &mut expanded);
                            // Always-present Grid (not frame-conditional) so multipass
                            // sizing still runs without changing the widget tree shape.
                            egui::Grid::new("elements_multipass_probe").show(ui, |ui| {
                                ui.label("probe");
                                ui.end_row();
                            });
                            egui::ScrollArea::vertical()
                                .id_salt("elements_list")
                                .show(ui, |ui| {
                                    captured = Some(ui.button("Sketch 0").id);
                                });
                        },
                    );
                });
            });
            ids.push(captured.expect("list button id"));
        }
        // Frame 0 may multipass-settle panel sizes; after that ids must not thrash.
        assert!(
            ids[1..].windows(2).all(|w| w[0] == w[1]),
            "list-row widget id must not renumber after settle: {ids:?}"
        );
    }

    /// Paint the Elements list into `ui` with no-op callbacks — layout tests only.
    fn paint_elements_list_for_test(
        ui: &mut egui::Ui,
        doc: &Document,
        sketch_session: Option<SketchSession>,
    ) {
        let mut visibility = ElementVisibility::default();
        let selection = SceneSelection::default();
        let health = crate::document_health::recompute_document_health(doc);
        let mut view_mode = HierarchyViewMode::List;
        let mut filter = ElementFilter::default();
        let mut filter_expanded = false;
        let mut collapsed_components = HashSet::new();
        let mut expanded_units = HashSet::new();
        let mut section_collapsed = SectionCollapse::default();
        let none: HashSet<SceneElement> = HashSet::new();
        show_pane(
            ui,
            doc,
            sketch_session,
            &mut visibility,
            &selection,
            &health,
            &mut view_mode,
            &mut filter,
            &mut filter_expanded,
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_, _| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            None,
            &mut |_, _| {},
            &mut |_, _| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_, _| {},
            &mut |_, _| {},
            &mut |_| {},
            &mut |_| {},
            &mut |_| {},
            false,
            false,
            &mut || {},
            &mut |_| {},
            None,
            &mut |_| {},
            &mut |_| {},
            &none,
            None,
            &none,
            None,
            &mut |_| {},
            &mut collapsed_components,
            &mut expanded_units,
            &mut section_collapsed,
            &mut |_| {},
            &mut || {},
            &mut || {},
            true,
            &mut |_, _| {},
            None,
            &mut |_| {},
        );
    }

    /// #1575: a long row label must clip inside the pane instead of stretching it.
    #[test]
    fn long_element_names_do_not_widen_the_elements_pane() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let line = doc
            .lines
            .insert(crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 510.0, 0.0));
        crate::names::set_element_name(
            &mut doc,
            SceneElement::Line(line),
            "this is an extremely long element name that must not stretch the elements pane".into(),
        )
        .unwrap();
        crate::constraints::add_distance_constraint(
            &mut doc,
            sketch,
            crate::model::DistanceTarget::LineLength(line),
            "510mm".to_string(),
        )
        .unwrap();

        const PANE: f32 = 220.0;
        let label = crate::names::node_label(&doc, HierarchyNode::Line(line));
        let ctx = egui::Context::default();
        let mut last_width = 0.0f32;
        let mut galley_width = 0.0f32;
        for _ in 0..4 {
            let _ = ctx.run_ui(Default::default(), |ui| {
                galley_width = ui.fonts_mut(|f| {
                    f.layout_no_wrap(label.clone(), egui::FontId::default(), Color32::WHITE)
                        .size()
                        .x
                });
                let resp = egui::Panel::left("tree")
                    .resizable(true)
                    .default_size(PANE)
                    .show(ui, |ui| {
                        paint_elements_list_for_test(
                            ui,
                            &doc,
                            Some(SketchSession { sketch }),
                        );
                    });
                last_width = resp.response.rect.width();
            });
        }
        assert!(
            galley_width > PANE,
            "fixture name should be wider than the default pane (galley {galley_width})"
        );
        assert!(
            last_width <= PANE + 1.0,
            "long names must clip, not widen the Elements pane (got {last_width}, default {PANE})"
        );
    }

    /// #1232: the Drawings section has no visibility eye; its collapse triangle claims the
    /// same column width as the eye so the type icon lines up with every other list row.
    #[test]
    fn drawings_section_type_icon_aligns_with_other_row_icons() {
        let ctx = egui::Context::default();
        let mut body_icon_x = f32::NAN;
        let mut drawings_icon_x = f32::NAN;
        let depth = 1usize;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            // Normal list row leading controls: eye, then type-icon placeholder.
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 18.0);
                let _ = icon_button(ui, icon_for_visibility(true), "Hide");
                let (icon_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ICON_DISPLAY_SIZE, ICON_DISPLAY_SIZE),
                    egui::Sense::hover(),
                );
                body_icon_x = icon_rect.min.x;
            });
            // Drawings section: disclosure (eye-width), then type-icon placeholder.
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 18.0);
                let _ = collapse_triangle(ui, false, ICON_DISPLAY_SIZE);
                let (icon_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ICON_DISPLAY_SIZE, ICON_DISPLAY_SIZE),
                    egui::Sense::hover(),
                );
                drawings_icon_x = icon_rect.min.x;
            });
        });
        assert!(
            (body_icon_x - drawings_icon_x).abs() < 0.5,
            "Drawings section type icon x={drawings_icon_x} should match body type icon x={body_icon_x}"
        );
        // Regression guard: a narrow (pre-fix) disclosure leaves the icon ~6px early.
        let mut narrow_icon_x = f32::NAN;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 18.0);
                let _ = collapse_triangle(ui, false, DISCLOSURE_BEFORE_EYE_WIDTH);
                let (icon_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ICON_DISPLAY_SIZE, ICON_DISPLAY_SIZE),
                    egui::Sense::hover(),
                );
                narrow_icon_x = icon_rect.min.x;
            });
        });
        assert!(
            drawings_icon_x > narrow_icon_x + 1.0,
            "eye-width disclosure must sit the type icon right of a narrow triangle \
             (drawings={drawings_icon_x}, narrow={narrow_icon_x})"
        );
    }

    /// #275: hiding a category prunes those nodes but promotes their kept children — so hiding
    /// "Operations" while keeping "Bodies" still shows the result body, just un-nested.
    #[test]
    fn filter_hierarchy_promotes_kept_children_of_hidden_nodes() {
        let tree = vec![HierarchyEntry {
            node: HierarchyNode::Document,
            children: vec![HierarchyEntry {
                node: HierarchyNode::BooleanOp(bopkey(0)),
                children: vec![HierarchyEntry {
                    node: HierarchyNode::Body(bkey(3)),
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
            vec![HierarchyNode::Body(bkey(3))]
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
        assert!(f.shows(HierarchyNode::Body(bkey(0))));
        assert!(f.shows(HierarchyNode::Sketch(skey(0))));
        assert!(f.shows(HierarchyNode::Document), "the root is always shown");
        assert!(f.shows(HierarchyNode::DrawingProjection { drawing: dkey(0), view: 0 }));
        assert!(f.shows(HierarchyNode::DrawingAnnotation { drawing: dkey(0), annotation: akey(0) }));
        assert!(!f.shows(HierarchyNode::ConstructionPlane(pkey(0))));
        assert!(!f.shows(HierarchyNode::Extrusion(xkey(0))));
    }

    /// #381: the Model workbench default keeps drawing rows but hides their **components**
    /// (projections, notes, dimensions) — page details are noise while modeling. The
    /// "Drawing components" toggle brings them back.
    #[test]
    fn model_workbench_default_hides_drawing_components() {
        let f = ElementFilter::default();
        assert!(f.shows(HierarchyNode::Drawing(dkey(0))), "the drawing row itself stays");
        assert!(!f.shows(HierarchyNode::DrawingProjection { drawing: dkey(0), view: 0 }));
        assert!(!f.shows(HierarchyNode::DrawingAnnotation { drawing: dkey(0), annotation: akey(0) }));
        assert!(!f.shows(HierarchyNode::DrawingDimension {
            drawing: dkey(0),
            view: 0,
            a: [0; 3],
            b: [0; 3]
        }));
        let f = ElementFilter { drawing_components: true, ..ElementFilter::default() };
        assert!(f.shows(HierarchyNode::DrawingProjection { drawing: dkey(0), view: 0 }));
    }

    #[test]
    fn default_document_hierarchy_has_single_document_root() {
        let doc = Document::default();
        let tree = build_hierarchy(&doc, None);
        assert_eq!(tree.len(), 1, "hierarchy should have exactly one root: {tree:?}");
        assert_eq!(tree[0].node, HierarchyNode::Document);
        // The default document's three datum planes (#833) nest under Document rather than
        // sitting as extra roots (#87).
        assert_eq!(
            tree[0].children.iter().map(|c| c.node).collect::<Vec<_>>(),
            vec![
                HierarchyNode::ConstructionPlane(pkey(0)),
                HierarchyNode::ConstructionPlane(pkey(1)),
                HierarchyNode::ConstructionPlane(pkey(2)),
            ]
        );

        let list = build_element_list(&doc, None);
        assert_eq!(
            list,
            vec![
                HierarchyNode::Document,
                HierarchyNode::ConstructionPlane(pkey(0)),
                HierarchyNode::ConstructionPlane(pkey(1)),
                HierarchyNode::ConstructionPlane(pkey(2)),
            ]
        );
    }

    #[test]
    fn root_level_items_nest_under_document_root() {
        use crate::document_lifecycle::delete_element;

        let mut doc = Document::default();
        // A second root-level construction plane (#87: root planes nest under Document,
        // not as separate roots).
        doc.construction_planes.insert(default_xy_plane());
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        // An orphaned extrusion: its sketch is gone, but the extrusion
        // itself is not cascaded away, so it must still surface — as a Document child, not
        // a top-level root.
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.extrusions.insert(crate::model::Extrusion {
            sketch,
            faces: Vec::new(),
            distance: 5.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        assert!(delete_element(&mut doc, SceneElement::Sketch(sketch)));
        assert!(!sketch_alive(&doc, sketch));

        // An orphaned body (STL import, no source extrusion, #70) also nests under Document.
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y]],
            source_name: "part".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        assert_eq!(tree.len(), 1, "hierarchy should have exactly one root: {tree:?}");
        assert_eq!(tree[0].node, HierarchyNode::Document);
        let children: Vec<HierarchyNode> = tree[0].children.iter().map(|c| c.node).collect();
        assert!(children.contains(&HierarchyNode::ConstructionPlane(pkey(0))));
        assert!(children.contains(&HierarchyNode::ConstructionPlane(pkey(1))));
        assert!(children.contains(&HierarchyNode::Extrusion(xkey(0))));
        assert!(children.contains(&HierarchyNode::Body(bkey(0))));
    }

    #[test]
    fn imported_mesh_body_surfaces_at_top_level() {
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Y]],
            source_name: "part".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: Some("part".to_string()),
            shadow: false,
        });
        doc.shape_order.push(ShapeKind::Body);

        let list = build_element_list(&doc, None);
        assert!(
            list.contains(&HierarchyNode::Body(bkey(0))),
            "imported body should be visible in the elements list, got {list:?}"
        );
        assert_eq!(parent_element(&doc, SceneElement::Body(bkey(0))), None);
    }

    #[test]
    fn construction_plane_ordering_is_deterministic_by_index() {
        let mut doc = Document::default();
        // Just the ground plane: this test indexes planes by hand, so the other two datum
        // planes (#833) would only shift the numbers.
        retain_ground_plane_only(&mut doc);
        // Independent planes (no input relationship) order by kind+index (#540), which is
        // stable across the randomized HashSet iteration order — never by creation time.
        // shape_order is populated only to prove it no longer influences pane ordering.
        doc.construction_planes.insert(default_xy_plane());
        doc.shape_order.push(ShapeKind::ConstructionPlane);
        doc.construction_planes.insert(default_xy_plane());
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        let expected = vec![
            HierarchyNode::Document,
            HierarchyNode::ConstructionPlane(pkey(0)),
            HierarchyNode::ConstructionPlane(pkey(1)),
            HierarchyNode::ConstructionPlane(pkey(2)),
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
            HierarchyNode::BooleanOp(bopkey(0)),
            HierarchyNode::Body(bkey(5)),
            HierarchyNode::Body(bkey(2)),
        ];
        let parent_of = HashMap::new();
        let mut input_sources = HashMap::new();
        // The boolean consumes both bodies, so it must come after them regardless of the
        // enum order (a Body sorts before a BooleanOp only because inputs come first here).
        input_sources.insert(
            HierarchyNode::BooleanOp(bopkey(0)),
            vec![HierarchyNode::Body(bkey(5)), HierarchyNode::Body(bkey(2))],
        );
        let out = topological_flat_sort(nodes, parent_of, input_sources);
        assert_eq!(
            out,
            vec![
                HierarchyNode::Body(bkey(2)), // input, lower index first
                HierarchyNode::Body(bkey(5)), // input
                HierarchyNode::BooleanOp(bopkey(0)), // consumer, after its inputs
            ]
        );
    }

    #[test]
    fn sketch_row_double_click_opens_for_edit_not_select() {
        assert_eq!(row_click_action(true, true, false), RowAction::Edit);
        assert_eq!(
            row_click_action(false, true, false),
            RowAction::Select { additive: false }
        );
        assert_eq!(row_click_action(false, false, false), RowAction::None);
    }

    #[test]
    fn open_sketch_from_elements_pane_action() {
        use crate::actions::{Action, AppState, SketchSession};

        let mut state = AppState::default();
        let sketch = state.doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
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
        // Just the ground plane: this test indexes planes by hand, so the other two datum
        // planes (#833) would only shift the numbers.
        retain_ground_plane_only(&mut doc);
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let s1 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        crate::construction::add_line_rectangle(&mut doc, s0, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.lines
            .insert(Line::from_local_endpoints(s1, 0.0, 0.0, 5.0, 0.0));
        doc
    }

    #[test]
    fn main_view_lists_planes_and_sketches_only() {
        let doc = doc_with_plane_sketches();
        let list = build_element_list(&doc, None);
        assert_eq!(list.len(), 4);
        assert_eq!(list[0], HierarchyNode::Document);
        assert_eq!(list[1], HierarchyNode::ConstructionPlane(pkey(0)));
        assert_eq!(list[2], HierarchyNode::Sketch(skey(0)));
        assert_eq!(list[3], HierarchyNode::Sketch(skey(1)));
    }

    #[test]
    fn sketch_view_lists_constraints_for_active_sketch() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        crate::constraints::add_distance_constraint(
            &mut doc,
            sketch,
            crate::model::DistanceTarget::LineLength(lkey(0)),
            "5mm".to_string(),
        )
        .unwrap();
        let list = build_element_list(&doc, Some(SketchSession { sketch }));
        assert!(list.contains(&HierarchyNode::Constraint(nkey(0))));
        assert!(!build_element_list(&doc, None).contains(&HierarchyNode::Constraint(nkey(0))));
    }

    #[test]
    fn nested_sketches_on_circle_face_follow_parent_order() {
        let mut doc = Document::default();
        // Just the ground plane: this test indexes planes by hand, so the other two datum
        // planes (#833) would only shift the numbers.
        retain_ground_plane_only(&mut doc);
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(s0, 0.0, 0.0, 20.0, 0.0));
        let s1 = doc.add_sketch(FaceId::Circle(rkey(0)));

        let list = build_element_list(&doc, None);
        assert_eq!(
            list,
            vec![
                HierarchyNode::Document,
                HierarchyNode::ConstructionPlane(pkey(0)),
                HierarchyNode::Sketch(skey(0)),
                HierarchyNode::Circle(rkey(0)),
                HierarchyNode::Sketch(skey(1)),
            ]
        );
        let _ = s1;
    }

    #[test]
    fn plane_from_sketch_geometry_lists_under_sketch() {
        let mut doc = Document::default();
        // Just the ground plane: this test indexes planes by hand, so the other two datum
        // planes (#833) would only shift the numbers.
        retain_ground_plane_only(&mut doc);
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
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
        doc.construction_planes.insert(derived);
        doc.shape_order.push(ShapeKind::ConstructionPlane);

        let list = build_element_list(&doc, None);
        assert_eq!(
            list,
            vec![
                HierarchyNode::Document,
                HierarchyNode::ConstructionPlane(pkey(0)),
                HierarchyNode::Sketch(skey(0)),
                HierarchyNode::ConstructionPlane(pkey(1)),
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

    /// #941: an in-sketch offset belongs to the sketch it offsets, so its row nests under
    /// that sketch (with its output lines under it) instead of standing as a root sibling.
    #[test]
    fn sketch_offset_op_nests_under_its_sketch() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        doc.sketch_offset_ops.insert(crate::model::SketchOffsetOperation {
            sketch,
            line_targets: vec![lkey(0)],
            circle_targets: Vec::new(),
            distance: "5".to_string(),
            construction: false,
            line_outputs: vec![lkey(1)],
            circle_outputs: Vec::new(),
            name: None,
        });
        doc.shape_order.push(ShapeKind::SketchOffsetOperation);

        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let roots = &tree[0].children;
        assert!(
            !roots.iter().any(|e| e.node == HierarchyNode::SketchOffsetOp(skop(0))),
            "the offset must not be a document-level root: {roots:?}"
        );
        let sketch_entry = find_entry(&tree, HierarchyNode::Sketch(sketch)).expect("sketch entry");
        let op = sketch_entry
            .children
            .iter()
            .find(|c| c.node == HierarchyNode::SketchOffsetOp(skop(0)))
            .expect("offset nests under its sketch");
        assert_eq!(op.children, vec![HierarchyEntry {
            node: HierarchyNode::Line(lkey(1)),
            children: vec![],
        }]);
    }

    /// #1540: an in-sketch mirror belongs to the sketch it reflects, so its row nests under
    /// that sketch (with its output lines under it) instead of standing as a root sibling.
    #[test]
    fn sketch_mirror_op_nests_under_its_sketch() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 0.0, 10.0));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, -10.0, 0.0));
        doc.shape_order
            .extend([ShapeKind::Line, ShapeKind::Line, ShapeKind::Line]);
        doc.sketch_mirror_ops
            .insert(crate::model::SketchMirrorOperation {
                sketch,
                line: lkey(1).into(),
                line_targets: vec![lkey(0)],
                circle_targets: Vec::new(),
                line_outputs: vec![lkey(2)],
                circle_outputs: Vec::new(),
                constraint_outputs: Vec::new(),
                name: None,
            });
        doc.shape_order.push(ShapeKind::SketchMirrorOperation);

        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let roots = &tree[0].children;
        assert!(
            !roots
                .iter()
                .any(|e| e.node == HierarchyNode::SketchMirrorOp(skop(0))),
            "the mirror must not be a document-level root: {roots:?}"
        );
        let sketch_entry = find_entry(&tree, HierarchyNode::Sketch(sketch)).expect("sketch entry");
        let op = sketch_entry
            .children
            .iter()
            .find(|c| c.node == HierarchyNode::SketchMirrorOp(skop(0)))
            .expect("mirror nests under its sketch");
        assert_eq!(
            op.children,
            vec![HierarchyEntry {
                node: HierarchyNode::Line(lkey(2)),
                children: vec![],
            }]
        );
        assert!(
            !sketch_entry
                .children
                .iter()
                .any(|c| c.node == HierarchyNode::Line(lkey(2))),
            "mirror output lines must not also list directly under the sketch"
        );
    }

    /// #1540: a mirror whose sketch is gone still surfaces somewhere, so it never becomes
    /// unreachable in the tree.
    #[test]
    fn sketch_mirror_op_surfaces_at_root_when_its_sketch_is_gone() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.sketches.remove(sketch);
        doc.sketch_mirror_ops
            .insert(crate::model::SketchMirrorOperation {
                sketch,
                line: lkey(0).into(),
                line_targets: Vec::new(),
                circle_targets: Vec::new(),
                line_outputs: Vec::new(),
                circle_outputs: Vec::new(),
                constraint_outputs: Vec::new(),
                name: None,
            });
        doc.shape_order.push(ShapeKind::SketchMirrorOperation);
        let tree = build_hierarchy(&doc, None);
        assert!(tree[0]
            .children
            .iter()
            .any(|e| e.node == HierarchyNode::SketchMirrorOp(skop(0))));
    }

    /// #1540: a sketch mirror is a sketch component, so the "Sketch components" filter
    /// hides the op and its reflected children. Hiding "Operations" does not.
    #[test]
    fn hiding_sketch_components_hides_sketch_mirror_and_its_children() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 0.0, 10.0));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, -10.0, 0.0));
        doc.shape_order
            .extend([ShapeKind::Line, ShapeKind::Line, ShapeKind::Line]);
        doc.sketch_mirror_ops
            .insert(crate::model::SketchMirrorOperation {
                sketch,
                line: lkey(1).into(),
                line_targets: vec![lkey(0)],
                circle_targets: Vec::new(),
                line_outputs: vec![lkey(2)],
                circle_outputs: Vec::new(),
                constraint_outputs: Vec::new(),
                name: None,
            });
        doc.shape_order.push(ShapeKind::SketchMirrorOperation);

        let mirror = HierarchyNode::SketchMirrorOp(skop(0));
        let output = HierarchyNode::Line(lkey(2));
        assert!(
            ElementFilter::default().shows(mirror),
            "default filter shows the sketch mirror"
        );
        assert!(
            !ElementFilter {
                sketch_geometry: false,
                ..ElementFilter::default()
            }
            .shows(mirror),
            "Sketch components off hides the sketch mirror"
        );
        assert!(
            ElementFilter {
                operations: false,
                ..ElementFilter::default()
            }
            .shows(mirror),
            "the mirror is not an Operations-filter node"
        );

        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let hidden = filter_hierarchy(
            &tree,
            &ElementFilter {
                sketch_geometry: false,
                ..ElementFilter::default()
            },
        );
        fn collect(entries: &[HierarchyEntry]) -> Vec<HierarchyNode> {
            let mut out = Vec::new();
            for e in entries {
                out.push(e.node);
                out.extend(collect(&e.children));
            }
            out
        }
        let nodes = collect(&hidden);
        assert!(
            !nodes.contains(&mirror),
            "filtered tree must drop the sketch mirror: {nodes:?}"
        );
        assert!(
            !nodes.contains(&output),
            "filtered tree must drop the mirror's children: {nodes:?}"
        );
        assert!(
            nodes.contains(&HierarchyNode::Sketch(sketch)),
            "the sketch itself stays when only sketch components are hidden"
        );
    }

    /// #941: an offset whose sketch is gone still surfaces somewhere, so it never becomes
    /// unreachable in the tree.
    #[test]
    fn sketch_offset_op_surfaces_at_root_when_its_sketch_is_gone() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.sketches.remove(sketch);
        doc.sketch_offset_ops.insert(crate::model::SketchOffsetOperation {
            sketch,
            line_targets: Vec::new(),
            circle_targets: Vec::new(),
            distance: "5".to_string(),
            construction: false,
            line_outputs: Vec::new(),
            circle_outputs: Vec::new(),
            name: None,
        });
        doc.shape_order.push(ShapeKind::SketchOffsetOperation);
        let tree = build_hierarchy(&doc, None);
        assert!(tree[0]
            .children
            .iter()
            .any(|e| e.node == HierarchyNode::SketchOffsetOp(skop(0))));
    }

    #[test]
    fn chamfer_fillet_bridge_line_nests_under_lower_index_trimmed_line() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        let mut bridge = Line::from_local_endpoints(sketch, 7.0, 0.0, 10.0, 3.0);
        bridge.chamfer_fillet_parent = Some(lkey(0));
        doc.lines.insert(bridge);
        doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line, ShapeKind::Line]);

        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let sketch_entry = find_entry(&tree, HierarchyNode::Sketch(sketch)).expect("sketch entry");
        // The bridge (line 2) is *not* a top-level sibling of the sketch's lines...
        assert!(!sketch_entry
            .children
            .iter()
            .any(|c| c.node == HierarchyNode::Line(lkey(2))));
        // ...it nests under line 0 (the lower-index trimmed line, #76).
        let line0_entry = sketch_entry
            .children
            .iter()
            .find(|c| c.node == HierarchyNode::Line(lkey(0)))
            .expect("line 0 entry");
        assert_eq!(line0_entry.children, vec![HierarchyEntry {
            node: HierarchyNode::Line(lkey(2)),
            children: vec![],
        }]);

        // The flat list keeps line 0 before its nested bridge, and still includes line 1.
        let list = build_element_list(&doc, Some(SketchSession { sketch }));
        let l0 = list.iter().position(|n| *n == HierarchyNode::Line(lkey(0))).unwrap();
        let l1 = list.iter().position(|n| *n == HierarchyNode::Line(lkey(1)));
        let l2 = list.iter().position(|n| *n == HierarchyNode::Line(lkey(2))).unwrap();
        assert!(l0 < l2, "parent line must come before the nested bridge");
        assert!(l1.is_some(), "the other trimmed line must still be listed");
    }

    #[test]
    fn chamfer_fillet_bridge_line_falls_back_to_top_level_when_parent_is_gone() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut bridge = Line::from_local_endpoints(sketch, 7.0, 0.0, 10.0, 3.0);
        // Points at a parent index that doesn't exist (e.g. the parent line was later removed
        // by undo) — must degrade gracefully to a top-level row, not panic or vanish.
        bridge.chamfer_fillet_parent = Some(lkey(99));
        doc.lines.insert(bridge);
        doc.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);

        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let sketch_entry = find_entry(&tree, HierarchyNode::Sketch(sketch)).expect("sketch entry");
        assert!(sketch_entry
            .children
            .iter()
            .any(|c| c.node == HierarchyNode::Line(lkey(1))));

        // Also degrades gracefully when the recorded parent line has been deleted.
        let mut doc2 = Document::default();
        let sketch2 = doc2.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let dead = doc2
            .lines
            .insert(Line::from_local_endpoints(sketch2, 0.0, 0.0, 10.0, 0.0));
        doc2.lines.remove(dead);
        let mut bridge2 = Line::from_local_endpoints(sketch2, 7.0, 0.0, 10.0, 3.0);
        bridge2.chamfer_fillet_parent = Some(dead);
        let bridge2 = doc2.lines.insert(bridge2);
        doc2.shape_order.extend([ShapeKind::Line, ShapeKind::Line]);
        let tree2 = build_hierarchy(&doc2, Some(SketchSession { sketch: sketch2 }));
        let sketch_entry2 =
            find_entry(&tree2, HierarchyNode::Sketch(sketch2)).expect("sketch entry");
        assert!(sketch_entry2
            .children
            .iter()
            .any(|c| c.node == HierarchyNode::Line(bridge2)));
    }

    #[test]
    fn row_style_faints_unrelated_rows_when_selection_active() {
        let mut doc = Document::default();
        let _s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Sketch(skey(0)),
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
                SceneElement::Sketch(skey(0)),
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
                SceneElement::ConstructionPlane(pkey(0)),
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
                SceneElement::Sketch(skey(1)),
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        crate::constraints::add_distance_constraint(
            &mut doc,
            sketch,
            DistanceTarget::LineLength(lkey(0)),
            "5mm".to_string(),
        )
        .unwrap();

        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Line(lkey(0)),
            false,
        );
        let context = selection_context_elements(&doc, &selection);
        let related = selection_related_constraints(&doc, &selection);
        assert!(context.contains(&SceneElement::Constraint(nkey(0))));
        assert!(related.contains(&nkey(0)));
    }

    #[test]
    fn row_style_highlights_related_constraint_when_line_selected() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        crate::constraints::add_distance_constraint(
            &mut doc,
            sketch,
            DistanceTarget::LineLength(lkey(0)),
            "5mm".to_string(),
        )
        .unwrap();

        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Line(lkey(0)),
            false,
        );
        let context = selection_context_elements(&doc, &selection);
        let related = selection_related_constraints(&doc, &selection);
        let list = build_element_list(&doc, Some(SketchSession { sketch }));
        let style_selection = selection_styles_visible_list(&list, &selection);
        let health = DocumentHealth::default();
        assert_eq!(
            row_style(
                SceneElement::Constraint(nkey(0)),
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
                SceneElement::Line(lkey(1)),
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
        use crate::document_lifecycle::delete_element;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine, Line, ShapeKind};

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let line_a = doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        let line_b = doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(line_a),
                line_b: ConstraintLine::Line(line_b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        delete_element(&mut doc, SceneElement::Line(line_a));
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
                SceneElement::Constraint(nkey(0)),
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
        use crate::document_lifecycle::delete_element;
        use crate::model::{Constraint, ConstraintKind, ConstraintLine, Line, ShapeKind};

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        doc.shape_order.push(ShapeKind::Line);
        doc.constraints.insert(Constraint {
            sketch,
            kind: ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(lkey(0)),
                line_b: ConstraintLine::Line(lkey(1)),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        });
        delete_element(&mut doc, SceneElement::Line(lkey(0)));
        let health = crate::document_health::recompute_document_health(&doc);

        let mut selection = SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::Constraint(nkey(0)),
            false,
        );
        assert!(row_shows_selection(
            &SceneElement::Constraint(nkey(0)),
            &selection,
            true
        ));
        assert_eq!(
            row_style(
                SceneElement::Constraint(nkey(0)),
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
        crate::selection::click_scene_selection(&mut selection, SceneElement::Line(lkey(1)), false);
        assert!(row_shows_selection(&SceneElement::Line(lkey(1)), &selection, true));
        assert_eq!(
            row_style(
                SceneElement::Line(lkey(1)),
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
        // Just the ground plane: this test indexes planes by hand, so the other two datum
        // planes (#833) would only shift the numbers.
        retain_ground_plane_only(&mut doc);
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.construction_planes.insert(plane_from_definition(
            &default_xy_plane().definition,
            ConstructionPlaneParent::Sketch(sketch),
        ));

        let mut vis = ElementVisibility::default();
        vis.set_visible(SceneElement::Sketch(sketch), false);
        assert!(!vis.effective_visible(&doc, SceneElement::ConstructionPlane(pkey(1))));
    }

    #[test]
    fn toggle_visibility_flips_state() {
        let mut vis = ElementVisibility::default();
        assert!(vis.is_visible(SceneElement::Sketch(skey(0))));
        assert!(!vis.toggle(SceneElement::Sketch(skey(0))));
        assert!(!vis.is_visible(SceneElement::Sketch(skey(0))));
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
        assert!(rb.contains(&SceneElement::Extrusion(xkey(0))), "extrusion depends on the sketch");
        assert!(rb.contains(&SceneElement::Body(bkey(0))), "body depends on the extrusion");
        assert!(!rb.contains(&SceneElement::Sketch(sketch)), "the marker itself stays");
        assert!(!rb.contains(&SceneElement::ConstructionPlane(pkey(0))), "ancestors stay active");

        // "Just before here" additionally hides the marker element itself.
        let rb_before = rolled_back_elements(&doc, &before(SceneElement::Sketch(sketch)));
        assert!(rb_before.contains(&SceneElement::Sketch(sketch)), "inclusive hides the marker");
        assert!(rb_before.contains(&SceneElement::Extrusion(xkey(0))), "and its descendants");

        // Rolling back to the body (a leaf nothing consumes) suppresses nothing — unless
        // inclusive, which hides just the body.
        assert!(rolled_back_elements(&doc, &here(SceneElement::Body(bkey(0)))).is_empty());
        let body_before = rolled_back_elements(&doc, &before(SceneElement::Body(bkey(0))));
        assert_eq!(body_before.len(), 1);
        assert!(body_before.contains(&SceneElement::Body(bkey(0))));
        // An unknown / non-graph marker suppresses nothing.
        assert!(rolled_back_elements(&doc, &here(SceneElement::Sketch(skey(99)))).is_empty());
        assert!(rolled_back_elements(&doc, &here(SceneElement::Origin)).is_empty());
    }

    fn doc_with_plane_sketch_rect_and_extrusion() -> (Document, SketchId) {
        use crate::model::{Body, BodySource, ExtrudeFace, Extrusion};

        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let rect_lines =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 5.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        (doc, sketch)
    }

    /// The lane a node's dot sits in, for the layout assertions below.
    fn lane_of(layout: &GraphLaneLayout, node: HierarchyNode) -> Option<usize> {
        layout.rows.iter().find(|r| r.node == node).map(|r| r.lane)
    }

    /// #1670: the Graph view reads one node per line — every node gets its own row, and an
    /// input always sits above the node it feeds.
    #[test]
    fn graph_lane_layout_puts_one_node_per_row() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let layout = graph_lane_layout(&doc, &tree);

        let nodes: Vec<HierarchyNode> = layout.rows.iter().map(|r| r.node).collect();
        let unique: HashSet<HierarchyNode> = nodes.iter().copied().collect();
        assert_eq!(nodes.len(), unique.len(), "one row per node");
        assert_eq!(
            unique,
            graph_node_positions(&tree).iter().map(|p| p.node).collect::<HashSet<_>>(),
            "every node on screen gets a row"
        );

        for edge in &layout.edges {
            if !edge.kind.is_input() {
                continue;
            }
            let from = layout.row_of(edge.from).expect("source row");
            let to = layout.row_of(edge.to).expect("consumer row");
            assert!(from < to, "{:?} feeds {:?}, so it must sit above it", edge.from, edge.to);
        }
    }

    /// #1670: a top-level element stays flush in the first lane — a trunk that would run
    /// across its row steps right instead of pushing the root over, so indentation keeps
    /// meaning "sits under something".
    #[test]
    fn graph_lane_layout_keeps_top_level_elements_in_the_first_lane() {
        let (mut doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        // A boolean op consuming the extruded body: it sits at the top level, *after* the
        // XZ/YZ datum planes, so its input lane has to cross their rows.
        doc.boolean_ops.insert(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![bkey(0)],
            b: Vec::new(),
            keep_b: false,
            outputs: Vec::new(),
            name: None,
        });
        let tree = graph_view_tree(&doc, Some(SketchSession { sketch }), &ElementFilter::default());
        let layout = graph_lane_layout(&doc, &tree);

        let planes: Vec<usize> = doc
            .construction_planes
            .keys()
            .filter_map(|pi| lane_of(&layout, HierarchyNode::ConstructionPlane(pi)))
            .collect();
        assert_eq!(planes.len(), 3, "all three datum planes are shown");
        assert!(
            planes.iter().all(|lane| *lane == 0),
            "datum planes are top level, so they stay in lane 0: {planes:?}"
        );
        let op = lane_of(&layout, HierarchyNode::BooleanOp(bopkey(0))).expect("the op has a row");
        assert!(op > 0, "the op is fed by the body, so it rides that input's lane");
    }

    /// Every drawn relationship line, in (lane, row) space — the same polyline the pane
    /// paints, so the layout invariants below are checked on what is actually on screen.
    fn lane_polylines(layout: &GraphLaneLayout) -> Vec<Vec<(f32, f32)>> {
        let row_of: HashMap<HierarchyNode, usize> = layout
            .rows
            .iter()
            .enumerate()
            .map(|(row, r)| (r.node, row))
            .collect();
        let lane_of: HashMap<HierarchyNode, usize> =
            layout.rows.iter().map(|r| (r.node, r.lane)).collect();
        let mut out = Vec::new();
        for edge in &layout.edges {
            let (Some(&from_row), Some(&to_row)) =
                (row_of.get(&edge.from), row_of.get(&edge.to))
            else {
                continue;
            };
            let from = (lane_of[&edge.from] as f32, from_row as f32);
            let to = (lane_of[&edge.to] as f32, to_row as f32);
            if !edge.kind.is_input() {
                out.push(vec![from, to]);
                continue;
            }
            let x = edge.lane as f32;
            let mut points = vec![from];
            if (x - from.0).abs() > 0.01 {
                points.push((x, from.1 + 0.5));
            }
            if (x - to.0).abs() > 0.01 {
                points.push((x, to.1 - 0.5));
            }
            points.push(to);
            out.push(points);
        }
        out
    }

    /// How many pairs of drawn lines cross away from their endpoints (#1684).
    fn lane_crossings(layout: &GraphLaneLayout) -> usize {
        let segments: Vec<((f32, f32), (f32, f32))> = lane_polylines(layout)
            .iter()
            .flat_map(|line| {
                line.windows(2).map(|pair| (pair[0], pair[1])).collect::<Vec<_>>()
            })
            .collect();
        let mut crossings = 0;
        for (i, (a0, a1)) in segments.iter().enumerate() {
            for (b0, b1) in segments.iter().skip(i + 1) {
                let r = (a1.0 - a0.0, a1.1 - a0.1);
                let s = (b1.0 - b0.0, b1.1 - b0.1);
                let denom = r.0 * s.1 - r.1 * s.0;
                if denom.abs() < 1e-6 {
                    continue;
                }
                let q = (b0.0 - a0.0, b0.1 - a0.1);
                let t = (q.0 * s.1 - q.1 * s.0) / denom;
                let u = (q.0 * r.1 - q.1 * r.0) / denom;
                const EDGE: f32 = 1e-3;
                if t > EDGE && t < 1.0 - EDGE && u > EDGE && u < 1.0 - EDGE {
                    crossings += 1;
                }
            }
        }
        crossings
    }

    /// A doc shaped like the one from #1684: a primitive body with two sketches on its faces,
    /// one of them cut back into the body — so an input line has to run past a whole subtree.
    fn doc_with_two_sketches_on_a_body() -> (Document, SketchId) {
        use crate::model::{Body, BodySource, ExtrudeFace, Extrusion};
        let (mut doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        // A second sketch on the extruded body's cap, extruded in turn.
        let cap = doc.add_sketch(FaceId::ExtrudeCap {
            extrusion: xkey(0),
            profile: ExtrudeFace::Polygon(doc.lines.keys().take(4).collect()),
            top: true,
        });
        let circle = doc.circles.insert(crate::model::Circle::from_local_center_radius(cap, 0.0, 0.0, 2.0, 0.0));
        doc.extrusions.insert(Extrusion {
            sketch: cap,
            faces: vec![ExtrudeFace::Circle(circle)],
            distance: 3.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(1)),
            material: None,
            name: None,
            shadow: false,
        });
        (doc, sketch)
    }

    /// #1684: the lines don't cross when the graph can be drawn without crossings, and
    /// #1683: no line runs through a row's icon — a shared trunk passes *beside* its children.
    #[test]
    fn graph_lane_layout_draws_without_crossings_or_lines_through_icons() {
        let (doc, sketch) = doc_with_two_sketches_on_a_body();
        let tree = graph_view_tree(&doc, Some(SketchSession { sketch }), &ElementFilter::default());
        let layout = graph_lane_layout(&doc, &tree);

        assert_eq!(lane_crossings(&layout), 0, "this graph draws without any crossing");

        let lane_of: HashMap<HierarchyNode, usize> =
            layout.rows.iter().map(|r| (r.node, r.lane)).collect();
        for edge in layout.edges.iter().filter(|e| e.kind.is_input()) {
            let (Some(from), Some(to)) = (layout.row_of(edge.from), layout.row_of(edge.to)) else {
                continue;
            };
            for (row, r) in layout.rows.iter().enumerate() {
                if row > from && row < to && r.lane == edge.lane {
                    panic!(
                        "the line {:?} -> {:?} runs straight through {:?}'s icon",
                        edge.from, edge.to, r.node
                    );
                }
            }
            let _ = &lane_of;
        }
    }

    /// #1683: a label starts past every line drawn at its row, so nothing runs across a name.
    #[test]
    fn graph_lane_row_extents_clear_the_lines_crossing_each_row() {
        let (mut doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        doc.boolean_ops.insert(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Cut,
            a: vec![bkey(0)],
            b: Vec::new(),
            keep_b: false,
            outputs: Vec::new(),
            name: None,
        });
        let tree = graph_view_tree(&doc, Some(SketchSession { sketch }), &ElementFilter::default());
        let layout = graph_lane_layout(&doc, &tree);
        let extents = layout.row_line_extents();

        assert_eq!(extents.len(), layout.rows.len());
        for (row, r) in layout.rows.iter().enumerate() {
            assert!(extents[row] >= r.lane, "a row always covers its own dot");
        }
        assert!(
            layout.rows.iter().enumerate().any(|(row, r)| extents[row] > r.lane),
            "a line passing a row pushes that row's label past it"
        );
        // The op's input runs down past the datum planes, so their labels clear that lane.
        let op_row = layout.row_of(HierarchyNode::BooleanOp(bopkey(0))).expect("op row");
        let body_row = layout.row_of(HierarchyNode::Body(bkey(0))).expect("body row");
        for row in (body_row + 1)..op_row {
            assert!(
                extents[row] > 0,
                "row {row} is crossed by the op's input lane, so its label must clear it"
            );
        }
    }

    /// #1671: cross-section views live in their own Views section at the bottom of the pane,
    /// the way drawings live under Drawings.
    #[test]
    fn cross_section_views_group_under_a_views_section() {
        let mut doc = Document::default();
        doc.cross_sections.insert(crate::model::CrossSection {
            name: Some("Front half".to_string()),
            cuts: Vec::new(),
        });
        let key = doc.cross_sections.keys().next().expect("the view");

        let tree = build_hierarchy(&doc, None);
        let root = tree.first().expect("the document root");
        let views = root
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Views)
            .expect("a Views section");
        assert_eq!(
            views.children.iter().map(|e| e.node).collect::<Vec<_>>(),
            vec![HierarchyNode::CrossSection(key)],
            "the view sits inside the section"
        );

        // A cutting plane is its own element nested under the view.
        doc.cross_sections[key].cuts.push(crate::model::CrossSectionCut::default());
        let tree = build_hierarchy(&doc, None);
        let views = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Views)
            .expect("a Views section");
        let view_row = views
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::CrossSection(key))
            .expect("the view");
        assert_eq!(
            view_row.children.iter().map(|e| e.node).collect::<Vec<_>>(),
            vec![HierarchyNode::SectionPlane { view: key, cut: 0 }],
            "the plane sits under its view"
        );
        assert_eq!(
            node_label(&doc, HierarchyNode::SectionPlane { view: key, cut: 0 }),
            "Cutting plane"
        );
        assert_eq!(
            scene_element_for_node(HierarchyNode::SectionPlane { view: key, cut: 0 }),
            Some(SceneElement::SectionPlane { view: key, cut: 0 }),
            "a cutting plane is selectable, deletable and renameable"
        );
        assert_eq!(node_label(&doc, HierarchyNode::Views), "Views");
        assert_eq!(node_label(&doc, HierarchyNode::CrossSection(key)), "Front half");
        assert_eq!(
            scene_element_for_node(HierarchyNode::CrossSection(key)),
            Some(SceneElement::CrossSection(key)),
            "a view is selectable, deletable and renameable like any element"
        );

        let rows = component_list_rows(
            &tree,
            &doc,
            &HashSet::new(),
            SectionCollapse::default(),
        );
        let plane_row = rows
            .iter()
            .find(|(n, _)| matches!(n, HierarchyNode::SectionPlane { .. }))
            .expect("the cutting plane is listed");
        let view_row = rows
            .iter()
            .find(|(n, _)| matches!(n, HierarchyNode::CrossSection(_)))
            .expect("the view is listed");
        assert!(
            plane_row.1 > view_row.1,
            "the cutting plane sits indented under its view, got plane depth {} view depth {}",
            plane_row.1,
            view_row.1
        );

        let mut hidden = tree.clone();
        prune_section_planes(&mut hidden);
        let views = hidden[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Views)
            .expect("Views");
        let view_row = views
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::CrossSection(key))
            .expect("the view");
        assert!(
            view_row.children.is_empty(),
            "modeling workbench hides cutting planes under the view"
        );

        // Without any view there is no empty section.
        let empty = build_hierarchy(&Document::default(), None);
        assert!(
            !empty[0].children.iter().any(|e| e.node == HierarchyNode::Views),
            "no views, no section"
        );
    }

    /// #1682: the Graph view drops the synthetic Document root — a model's own top-level
    /// elements are the roots there. The List view keeps it.
    #[test]
    fn graph_view_tree_has_no_document_row() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let session = Some(SketchSession { sketch });
        let tree = graph_view_tree(&doc, session, &ElementFilter::default());
        let layout = graph_lane_layout(&doc, &tree);

        assert!(
            !layout.rows.iter().any(|r| r.node == HierarchyNode::Document),
            "the graph has no Document row"
        );
        assert!(
            !layout
                .edges
                .iter()
                .any(|e| e.from == HierarchyNode::Document || e.to == HierarchyNode::Document),
            "and no lines to it"
        );
        assert!(
            layout
                .rows
                .iter()
                .any(|r| matches!(r.node, HierarchyNode::ConstructionPlane(_))),
            "the planes are roots of their own"
        );
        assert!(
            build_element_list(&doc, session).contains(&HierarchyNode::Document),
            "the List view still shows Document"
        );
    }

    /// #1670: siblings string down a single lane (short legs, not a fan to the right), and a
    /// one-consumer chain keeps its lane instead of stepping right at every step.
    #[test]
    fn graph_lane_layout_keeps_the_graph_narrow() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let tree = graph_view_tree(&doc, Some(SketchSession { sketch }), &ElementFilter::default());
        let layout = graph_lane_layout(&doc, &tree);

        let lanes: HashSet<usize> = doc
            .lines
            .keys()
            .map(|li| lane_of(&layout, HierarchyNode::Line(li)).expect("line lane"))
            .collect();
        assert_eq!(lanes.len(), 1, "the rectangle's four lines share one lane: {lanes:?}");

        assert_eq!(
            lane_of(&layout, HierarchyNode::Body(bkey(0))),
            lane_of(&layout, HierarchyNode::Extrusion(xkey(0))),
            "an only child continues its input's lane"
        );
        assert!(layout.lane_count <= 3, "lanes stay narrow, got {}", layout.lane_count);
    }

    /// #1670: a constraint relates to the geometry it constrains — it is nobody's child, so
    /// it hangs off those elements with "related" ties rather than a parent line.
    #[test]
    fn graph_lane_layout_ties_constraints_to_their_geometry() {
        let (doc, sketch) = doc_with_plane_sketch_rect_and_extrusion();
        let tree = build_hierarchy(&doc, Some(SketchSession { sketch }));
        let layout = graph_lane_layout(&doc, &tree);

        let ci = doc.constraints.keys().next().expect("the rectangle brings constraints");
        let node = HierarchyNode::Constraint(ci);
        let incoming: Vec<&GraphLaneEdge> = layout.edges.iter().filter(|e| e.to == node).collect();
        assert!(!incoming.is_empty(), "the constraint is tied to something");
        assert!(
            incoming.iter().all(|e| e.kind == GraphLaneEdgeKind::Related),
            "constraint ties are related, never parent/child"
        );
        assert!(
            incoming.iter().any(|e| matches!(e.from, HierarchyNode::Line(_))),
            "a coincident constraint ties to the lines it holds together"
        );
        assert!(
            !layout
                .edges
                .iter()
                .any(|e| e.kind.is_input() && matches!(e.to, HierarchyNode::Constraint(_))),
            "no sketch → constraint parent line"
        );
    }

    /// #1764: when a fan sibling's preferred lane is occupied by another child's outgoing
    /// trunk, pack left into a free lane rather than stepping further right. A shape with a
    /// shadow body plus a face sketch is the smallest case: the body's trunk occupies lane 1
    /// at the sketch's row, so the sketch should sit in lane 0 instead of drifting to lane 2.
    fn doc_with_shape_shadow_body_and_face_sketch() -> (Document, crate::model::SketchId) {
        use crate::model::{
            Body, BodySource, ExtrudeFace, Extrusion, FaceId, Primitive, PrimitiveFace,
            PrimitiveKind, Sketch,
        };
        let mut doc = Document::default();
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "40".to_string();
        shape.depth = "30".to_string();
        shape.height = "20".to_string();
        let pi = doc.primitives.insert(shape);
        doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: true,
        });
        let sketch = doc.sketches.insert(Sketch {
            face: FaceId::PrimitiveFace {
                primitive: pi,
                face: PrimitiveFace::CuboidTop,
            },
            name: None,
            length_unit: None,
            angle_unit: None,
        });
        let ei = doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(vec![])],
            distance: 10.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        doc.bodies.insert(Body {
            source: BodySource::Solid {
                base: Some(pi),
                add: vec![ei],
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        (doc, sketch)
    }

    fn graph_with_shadow_bodies(doc: &Document) -> GraphLaneLayout {
        let mut filter = ElementFilter::default();
        filter.shadow_bodies = true;
        filter.sketch_geometry = false;
        let tree = graph_view_tree(doc, None, &filter);
        graph_lane_layout(doc, &tree)
    }

    #[test]
    fn graph_lane_layout_packs_left_when_the_preferred_lane_is_busy() {
        let (doc, sketch) = doc_with_shape_shadow_body_and_face_sketch();
        let layout = graph_with_shadow_bodies(&doc);
        assert_eq!(
            lane_of(&layout, HierarchyNode::Sketch(sketch)),
            Some(0),
            "the face sketch packs into the unused first lane instead of stepping right of the body's trunk"
        );
        assert_eq!(lane_crossings(&layout), 0, "packing left must not introduce crossings");
    }

    /// #1764: the reported document — two cuboids, a cut, a face sketch, and a slice —
    /// wastes left columns under Sketch 1 / Slice 0 / Body 4 / Body 5. Pack so Sketch 1
    /// sits on the far left, Slice 0 moves left, and the slice bodies occupy earlier columns.
    #[test]
    fn graph_lane_layout_packs_the_1764_slice_graph_left() {
        let bytes = include_bytes!("../tests/fixtures/issue_1764.json");
        let doc = crate::storage::from_json_bytes(bytes).expect("load issue 1764");
        let layout = graph_with_shadow_bodies(&doc);

        let lane = |name: &str| {
            layout
                .rows
                .iter()
                .find(|r| node_label(&doc, r.node) == name)
                .map(|r| r.lane)
        };
        assert_eq!(lane("Sketch 1"), Some(0), "Sketch 1's icon sits in the far left column");
        assert_eq!(lane("Slice 0"), Some(0), "Slice 0 packs left with the sketch that feeds it");
        assert_eq!(lane("Body 4"), Some(1), "Body 4 sits in the second column");
        assert_eq!(lane("Body 5"), Some(1), "Body 5 shares that column with its sibling");
        assert!(
            layout.lane_count <= 2,
            "unused left columns are not wasted, got {} lanes",
            layout.lane_count
        );
        assert_eq!(lane_crossings(&layout), 0, "packing left must not introduce crossings");

        let lane_of: HashMap<HierarchyNode, usize> =
            layout.rows.iter().map(|r| (r.node, r.lane)).collect();
        for edge in layout.edges.iter().filter(|e| e.kind.is_input()) {
            let (Some(from), Some(to)) = (layout.row_of(edge.from), layout.row_of(edge.to)) else {
                continue;
            };
            for (row, r) in layout.rows.iter().enumerate() {
                if row > from && row < to && r.lane == edge.lane {
                    panic!(
                        "the line {:?} -> {:?} runs straight through {:?}'s icon",
                        edge.from, edge.to, r.node
                    );
                }
            }
            let _ = &lane_of;
        }
    }











    /// #252: a loft appears as an operation node with its output body nested beneath it, and its
    /// cross-section sketches feed it as graph dependency edges — the user's canonical example.
    #[test]
    fn loft_is_an_operation_with_body_output_and_sketch_inputs() {
        use crate::model::{Body, BodySource, ExtrudeFace, Loft, LoftSection};
        let mut doc = Document::default();
        let loft_key = doc.lofts.insert(Loft {
            sections: vec![
                LoftSection { sketch: skey(0), face: ExtrudeFace::Circle(rkey(0)) },
                LoftSection { sketch: skey(1), face: ExtrudeFace::Circle(rkey(1)) },
                LoftSection { sketch: skey(2), face: ExtrudeFace::Circle(rkey(2)) },
            ],
            mode: crate::model::LoftMode::NewBody,
            name: None,
        });
        doc.bodies.insert(Body {
            source: BodySource::Loft(loft_key),
            material: None,
            name: None,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        let loft = tree[0]
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Loft(loft_key))
            .expect("loft is a top-level operation, not a bare body");
        assert!(
            loft.children.iter().any(|c| c.node == HierarchyNode::Body(bkey(0))),
            "the loft body nests under the loft as its output"
        );
        // The three section sketches feed the loft as dependency inputs.
        let deps = graph_dependency_edges(&doc);
        for si in doc.sketches.keys() {
            assert!(
                deps.contains(&(HierarchyNode::Sketch(si), HierarchyNode::Loft(loft_key))),
                "sketch {si:?} feeds the loft"
            );
        }
        // #1487: a loft is a real scene element, same as a sweep — selectable, hideable,
        // renameable, deletable, and a component member.
        assert_eq!(
            scene_element_for_node(HierarchyNode::Loft(loft_key)),
            Some(SceneElement::Loft(loft_key))
        );
        assert_eq!(
            hierarchy_node_for_element(&SceneElement::Loft(loft_key)),
            Some(HierarchyNode::Loft(loft_key))
        );
        assert_eq!(
            node_editable_operation(HierarchyNode::Loft(loft_key)),
            Some(SceneElement::Loft(loft_key))
        );
        assert_eq!(
            component_member_element(crate::model::ComponentMember::Loft(loft_key)),
            Some(SceneElement::Loft(loft_key))
        );
        assert_eq!(
            component_member_for_element(&SceneElement::Loft(loft_key)),
            Some(crate::model::ComponentMember::Loft(loft_key))
        );
        assert_eq!(
            produced_bodies(&doc, &SceneElement::Loft(loft_key)),
            vec![bkey(0)]
        );
    }

    /// #sweep: the op node depends on its profile sketch and every path line, and
    /// its NewBody output body nests beneath it.
    #[test]
    fn sweep_appears_in_the_tree_and_feeds_from_its_inputs() {
        use crate::model::{Body, BodySource, SweepMode, Sweep, Line};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines.insert(Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, 10.0));
        let sweep = doc.sweeps.insert(Sweep {
            sketch,
            faces: Vec::new(),
            path: vec![lkey(0), lkey(1)],
            mode: SweepMode::NewBody,
            name: None,
        });
        doc.bodies.insert(Body {
            source: BodySource::Sweep(sweep),
            material: None,
            name: None,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        // The op nests under its profile sketch (#478), with the output body beneath it.
        fn find_op(
            entries: &[HierarchyEntry],
            sweep: crate::model::SweepKey,
        ) -> Option<&HierarchyEntry> {
            for e in entries {
                if e.node == HierarchyNode::SweepOp(sweep) {
                    return Some(e);
                }
                if let Some(found) = find_op(&e.children, sweep) {
                    return Some(found);
                }
            }
            None
        }
        let op = find_op(&tree, sweep).expect("the sweep op appears in the tree");
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
            sketch_entry.children.iter().any(|c| c.node == HierarchyNode::SweepOp(sweep)),
            "the sweep op nests under its profile sketch"
        );
        assert!(
            op.children.iter().any(|c| c.node == HierarchyNode::Body(bkey(0))),
            "the swept body nests under the sweep op as its output"
        );
        let deps = graph_dependency_edges(&doc);
        assert!(deps.contains(&(HierarchyNode::Sketch(sketch), HierarchyNode::SweepOp(sweep))));
        assert!(deps.contains(&(HierarchyNode::Line(lkey(0)), HierarchyNode::SweepOp(sweep))));
        assert!(deps.contains(&(HierarchyNode::Line(lkey(1)), HierarchyNode::SweepOp(sweep))));
    }

    /// #1151: a 3D slice that laser-cuts with sketch lines takes the defining sketch (and
    /// each line) as graph inputs — so Sketch 0 feeds Slice 0 in the Elements graph, not
    /// only the body being cut.
    #[test]
    fn slice_with_laser_lines_has_sketch_as_graph_input() {
        use crate::model::{Body, BodySource, Line, SliceCutter, SliceOperation};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let a = doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        let b = doc.lines.insert(Line::from_local_endpoints(sketch, 5.0, 0.0, 5.0, 10.0));
        doc.bodies.insert(Body {
            source: BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: true,
        });
        let op = doc.slice_ops.insert(SliceOperation {
            targets: vec![bkey(0)],
            cutters: vec![
                SliceCutter::Line { line: a },
                SliceCutter::Line { line: b },
            ],
            extend_infinite: true,
            outputs: vec![bkey(1), bkey(2)],
            name: None,
        });
        for piece in 0..2 {
            doc.bodies.insert(Body {
                source: BodySource::Sliced {
                    op,
                    target: 0,
                    piece,
                    add: Vec::new(),
                    cut: Vec::new(),
                },
                material: None,
                name: None,
                shadow: false,
            });
        }

        let deps = graph_dependency_edges(&doc);
        assert!(
            deps.contains(&(HierarchyNode::Sketch(sketch), HierarchyNode::SliceOp(op))),
            "the sketch that defined the laser path feeds the slice (#1151)"
        );
        assert!(deps.contains(&(HierarchyNode::Line(a), HierarchyNode::SliceOp(op))));
        assert!(deps.contains(&(HierarchyNode::Line(b), HierarchyNode::SliceOp(op))));
        assert!(deps.contains(&(HierarchyNode::Body(bkey(0)), HierarchyNode::SliceOp(op))));
        // One sketch edge, not one per line.
        let sketch_edges = deps
            .iter()
            .filter(|(s, c)| {
                *s == HierarchyNode::Sketch(sketch) && *c == HierarchyNode::SliceOp(op)
            })
            .count();
        assert_eq!(sketch_edges, 1, "dedupe sketch→slice edges across lines on the same sketch");
    }

    /// #909: a shape is a top-level element with its body nested under it, named by kind.
    #[test]
    fn shape_appears_in_the_tree_with_its_body() {
        use crate::model::{Body, BodySource, Primitive, PrimitiveKind};
        let mut doc = Document::default();
        let mut shape = Primitive::new(PrimitiveKind::Sphere);
        shape.radius = "6".to_string();
        let key = doc.primitives.insert(shape);
        doc.bodies.insert(Body {
            source: BodySource::Primitive(key),
            material: None,
            name: None,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        let root = &tree[0];
        let entry = root
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Shape(key))
            .expect("the shape is a top-level element");
        assert!(
            entry.children.iter().any(|c| c.node == HierarchyNode::Body(bkey(0))),
            "its body nests under it",
        );
        assert!(
            !root.children.iter().any(|e| e.node == HierarchyNode::Body(bkey(0))),
            "and isn't also a Document-level orphan",
        );
        assert_eq!(
            scene_element_for_node(HierarchyNode::Shape(key)),
            Some(SceneElement::Shape(key))
        );
        assert_eq!(
            crate::names::default_node_label(&doc, HierarchyNode::Shape(key)),
            "Sphere 0"
        );
    }

    /// #1104/#1105/#1106: after a fuse-merge onto a Shape-tool body, the pure cuboid body
    /// stays under the Shape (as a shadow), the sketch stays under the Shape, and the
    /// combined solid nests under the extrusion as its output.
    #[test]
    fn shape_with_merged_extrusion_keeps_body_and_face_sketch_in_tree() {
        use crate::model::{
            Body, BodySource, ExtrudeFace, Extrusion, FaceId, Primitive, PrimitiveFace,
            PrimitiveKind, Sketch,
        };
        let mut doc = Document::default();
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "40".to_string();
        shape.depth = "30".to_string();
        shape.height = "20".to_string();
        let pi = doc.primitives.insert(shape);
        let shadow_bi = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: true,
        });
        let live_bi = doc.bodies.insert(Body {
            source: BodySource::Solid {
                base: Some(pi),
                add: vec![xkey(0)],
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        let sketch = doc.sketches.insert(Sketch {
            face: FaceId::PrimitiveFace {
                primitive: pi,
                face: PrimitiveFace::CuboidTop,
            },
            name: None,
            length_unit: None,
            angle_unit: None,
        });
        let ei = doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(vec![])],
            distance: 10.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        // Keep body source extrusion key in sync with the real extrusion key.
        doc.bodies[live_bi].source = BodySource::Solid {
            base: Some(pi),
            add: vec![ei],
            cut: Vec::new(),
        };

        let tree = build_hierarchy(&doc, None);
        let root = &tree[0];
        let entry = root
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Shape(pi))
            .expect("shape stays top-level");
        assert!(
            entry
                .children
                .iter()
                .any(|c| c.node == HierarchyNode::Body(shadow_bi)),
            "pure cuboid body (shadow) nests under the shape, children: {:?}",
            entry.children.iter().map(|c| c.node).collect::<Vec<_>>()
        );
        assert!(
            entry
                .children
                .iter()
                .any(|c| c.node == HierarchyNode::Sketch(sketch)),
            "sketch on a primitive face nests under the shape"
        );
        assert_eq!(
            parent_element(&doc, SceneElement::Body(shadow_bi)),
            Some(SceneElement::Shape(pi))
        );
        assert_eq!(
            parent_element(&doc, SceneElement::Body(live_bi)),
            Some(SceneElement::Extrusion(ei))
        );
        let sketch_entry = entry
            .children
            .iter()
            .find(|c| c.node == HierarchyNode::Sketch(sketch))
            .expect("sketch under shape");
        let extrude_entry = sketch_entry
            .children
            .iter()
            .find(|c| c.node == HierarchyNode::Extrusion(ei))
            .expect("extrusion under sketch");
        assert!(
            extrude_entry
                .children
                .iter()
                .any(|c| c.node == HierarchyNode::Body(live_bi)),
            "combined body is extrusion output"
        );
    }

    #[test]
    fn revolution_appears_in_the_tree_with_its_body(){
        use crate::model::{Body, BodySource, Revolution, RevolveAxis, RevolveMode};
        let mut doc = Document::default();
        let rev_key = doc.revolutions.insert(Revolution {
            sketch: skey(0),
            faces: Vec::new(),
            axis: RevolveAxis::X,
            angle_deg: 360.0,
            angle_expression: String::new(),
            angle_is_revolutions: false,
            pitch_mm: 0.0,
            pitch_expression: String::new(),
            gap_is_offset: true,
            symmetric: false,
            mode: RevolveMode::NewBody,
            name: None,
        });
        doc.bodies.insert(Body {
            source: BodySource::Revolve(rev_key),
            material: None,
            name: None,
            shadow: false,
        });

        let tree = build_hierarchy(&doc, None);
        let root = &tree[0];
        let rev = root
            .children
            .iter()
            .find(|e| e.node == HierarchyNode::Revolution(rev_key))
            .expect("the revolution is a top-level element (#211)");
        assert!(
            rev.children.iter().any(|c| c.node == HierarchyNode::Body(bkey(0))),
            "the revolved body nests under the revolution",
        );
        // The body's *only* parent is the revolution (#305): it must not also surface as a
        // top-level orphan under Document.
        assert!(
            !root.children.iter().any(|e| e.node == HierarchyNode::Body(bkey(0))),
            "a revolved body is not a Document-level orphan",
        );
        // It maps to a selectable scene element.
        assert_eq!(
            scene_element_for_node(HierarchyNode::Revolution(rev_key)),
            Some(SceneElement::Revolution(rev_key))
        );
    }

    /// #1109: shadow bodies are filtered out of the Elements pane by default. The default
    /// filter carries the toggle (off), and `prune_shadow_bodies` drops shadow body rows
    /// wherever they sit in the tree while leaving live bodies untouched.
    #[test]
    fn pane_filters_out_shadow_bodies_by_default() {
        use crate::model::{Body, BodySource, Extrusion, ExtrudeFace, FaceId, Sketch};
        let mut doc = Document::default();
        // A sketch + extrusion so the bodies have a real producer to nest under.
        let sketch = doc.sketches.insert(Sketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            name: None,
            length_unit: None,
            angle_unit: None,
        });
        doc.extrusions.insert(Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Polygon(vec![])],
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
        let live = doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let shadow = doc.bodies.insert(Body {
            source: BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: true,
        });

        // The default filter has the toggle off — shadow bodies hidden.
        let filter = ElementFilter::default();
        assert!(!filter.shadow_bodies, "shadow bodies are off by default");

        // The pane builds its tree as: filter_hierarchy(...) then prune_shadow_bodies(...).
        let mut tree = filter_hierarchy(&build_hierarchy(&doc, None), &filter);
        prune_shadow_bodies(&mut tree, &doc);
        fn any_body(entries: &[HierarchyEntry], bi: crate::model::BodyKey) -> bool {
            entries.iter().any(|e| {
                e.node == HierarchyNode::Body(bi) || any_body(&e.children, bi)
            })
        }
        assert!(any_body(&tree, live), "the live body stays in the pane");
        assert!(
            !any_body(&tree, shadow),
            "the shadow body is filtered out by default"
        );

        // Opting in (the toggle) keeps the shadow body — the filter is reversible.
        let filter = ElementFilter {
            shadow_bodies: true,
            ..ElementFilter::default()
        };
        let tree = filter_hierarchy(&build_hierarchy(&doc, None), &filter);
        assert!(
            any_body(&tree, shadow),
            "turning the toggle on shows the shadow body"
        );

        // The unfiltered scripting list (`build_element_list`) still includes the shadow —
        // the prune is a pane presentation concern, not a change to the document's element API.
        let list = build_element_list(&doc, None);
        assert!(
            list.contains(&HierarchyNode::Body(shadow)),
            "the unfiltered element list still reports shadow bodies"
        );
    }

    /// #1109: `prune_shadow_bodies` drops shadow bodies nested under any parent (a Shape's
    /// pure primitive body after a fuse-merge) while keeping the shape and its other children.
    #[test]
    fn prune_shadow_bodies_drops_nested_shadow_keeps_siblings() {
        use crate::model::{Body, BodySource, Primitive, PrimitiveKind, Sketch, FaceId};
        let mut doc = Document::default();
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "1".to_string();
        let pi = doc.primitives.insert(shape);
        let shadow_bi = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: true,
        });
        // A face sketch nests under the shape alongside the shadow body.
        let sketch = doc.sketches.insert(Sketch {
            face: FaceId::PrimitiveFace {
                primitive: pi,
                face: crate::model::PrimitiveFace::CuboidTop,
            },
            name: None,
            length_unit: None,
            angle_unit: None,
        });

        let mut tree = build_hierarchy(&doc, None);
        prune_shadow_bodies(&mut tree, &doc);
        fn collect(entries: &[HierarchyEntry]) -> Vec<HierarchyNode> {
            let mut out = Vec::new();
            for e in entries {
                out.push(e.node);
                out.extend(collect(&e.children));
            }
            out
        }
        let nodes = collect(&tree);
        assert!(
            !nodes.contains(&HierarchyNode::Body(shadow_bi)),
            "the nested shadow body is pruned"
        );
        assert!(
            nodes.contains(&HierarchyNode::Shape(pi)),
            "the shape itself stays"
        );
        assert!(
            nodes.contains(&HierarchyNode::Sketch(sketch)),
            "a sibling sketch under the shape is untouched"
        );
    }

    /// How a [`SceneElement`] is re-opened after commit (#1486). Exhaustive so a new
    /// variant cannot skip the "Edit* action ⇒ Elements-pane row" contract.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ElementEditPath {
        /// Double-click / right-click → Edit via [`node_editable_operation`].
        Row { has_edit_action: bool },
        /// Own dedicated entry (sketch, plane, extrusion, 3D chamfer/fillet, text, drawing).
        Dedicated,
        /// Not an operation.
        None,
    }

    fn element_edit_path(element: &SceneElement) -> ElementEditPath {
        use ElementEditPath::*;
        match element {
            // A cross-section view opens the View workbench from its row, the universal way.
            SceneElement::CrossSection(_) | SceneElement::SectionPlane { .. } => {
                Row { has_edit_action: true }
            }
            SceneElement::BooleanOp(_)
            | SceneElement::MoveOp(_)
            | SceneElement::MirrorOp(_)
            | SceneElement::RepeatOp(_)
            | SceneElement::SketchRepeatOp(_)
            | SceneElement::SketchOffsetOp(_)
            | SceneElement::SketchMirrorOp(_)
            | SceneElement::SketchSliceOp(_)
            | SceneElement::SliceOp(_)
            | SceneElement::ShellOp(_)
            | SceneElement::Shape(_)
            | SceneElement::Joint(_) => Row {
                has_edit_action: true,
            },
            // Re-opened through the universal row, but commit is Create-with-`editing`
            // (revolve/sweep) or `CommitVertexTreatment` (2D chamfer/fillet) rather
            // than a distinct `Edit*` action.
            SceneElement::Revolution(_)
            | SceneElement::SweepOp(_)
            | SceneElement::Loft(_)
            | SceneElement::SketchVertexTreatmentOp(_) => Row {
                has_edit_action: false,
            },
            SceneElement::ConstructionPlane(_)
            | SceneElement::Sketch(_)
            | SceneElement::Extrusion(_)
            | SceneElement::EdgeTreatmentOp(_)
            | SceneElement::SketchText(_)
            | SceneElement::Drawing(_)
            | SceneElement::DrawingElement { .. } => Dedicated,
            SceneElement::Line(_)
            | SceneElement::Circle(_)
            | SceneElement::Point(_)
            | SceneElement::Constraint(_)
            | SceneElement::Body(_)
            | SceneElement::FaceEdge(_)
            | SceneElement::BodyEdge { .. }
            | SceneElement::BodyVertex { .. }
            | SceneElement::ProjectedEdge { .. }
            | SceneElement::ProjectedCorner { .. }
            | SceneElement::BodyFace { .. }
            | SceneElement::BodyCylinder { .. }
            | SceneElement::BodyAxis { .. }
            | SceneElement::Image(_)
            | SceneElement::Origin
            | SceneElement::GlobalAxis(_)
            | SceneElement::SketchFace(_)
            | SceneElement::MovePoint(_)
            | SceneElement::ExtrusionEdge { .. }
            | SceneElement::PrimitiveEdge { .. }
            | SceneElement::RepeatedFace { .. }
            | SceneElement::Component(_)
            | SceneElement::UnitInstance(_) => None,
        }
    }

    fn sample_every_scene_element() -> Vec<SceneElement> {
        use crate::construction::GlobalAxis;
        use crate::model::{
            annotation_key_for_slot as akey, component_key_for_slot as ckey,
            drawing_key_for_slot as dkey, extrusion_key_for_slot as xkey,
            primitive_key_for_slot as primkey, sketch_text_key_for_slot as tkey,
            unit_instance_key_for_slot as uikey, ConstraintLine, ConstraintPoint, ExtrusionEdgeRef,
            FaceId, LineEnd, MovePointRef,
        };
        let sweep = crate::arena::Key::from_bits(0);
        let revolution = crate::arena::Key::from_bits(0);
        vec![
            SceneElement::DrawingElement {
                drawing: dkey(0),
                element: crate::context::DrawingElementRef::Projection(0),
            },
            SceneElement::ConstructionPlane(pkey(0)),
            SceneElement::Sketch(skey(0)),
            SceneElement::Line(lkey(0)),
            SceneElement::Circle(rkey(0)),
            SceneElement::Point(ConstraintPoint::LineEndpoint {
                line: lkey(0),
                end: LineEnd::Start,
            }),
            SceneElement::Constraint(nkey(0)),
            SceneElement::Extrusion(xkey(0)),
            SceneElement::Body(bkey(0)),
            SceneElement::ProjectedEdge {
                drawing: dkey(0),
                view: 0,
                body: Some(bkey(0)),
                a: [0; 3],
                b: [1; 3],
            },
            SceneElement::ProjectedCorner {
                drawing: dkey(0),
                view: 0,
                body: Some(bkey(0)),
                p: [0; 3],
            },
            SceneElement::FaceEdge(ConstraintLine::Line(lkey(0))),
            SceneElement::BodyEdge {
                body: bkey(0),
                a: [0, 0, 0],
                b: [1, 0, 0],
            },
            SceneElement::BodyVertex {
                body: bkey(0),
                p: [0, 0, 0],
            },
            SceneElement::BodyFace {
                body: bkey(0),
                centroid: [0, 0, 0],
                normal: [0, 0, 1],
            },
            SceneElement::BodyCylinder {
                body: bkey(0),
                origin: [0, 0, 0],
                dir: [0, 0, 1],
                radius: 1,
            },
            SceneElement::BodyAxis {
                body: bkey(0),
                origin: [0, 0, 0],
                dir: [0, 0, 1],
            },
            SceneElement::Image(crate::arena::Key::from_bits(0)),
            SceneElement::BooleanOp(bopkey(0)),
            SceneElement::MoveOp(mopkey(0)),
            SceneElement::MirrorOp(mirkey(0)),
            SceneElement::RepeatOp(repkey(0)),
            SceneElement::SketchRepeatOp(skop(0)),
            SceneElement::SketchOffsetOp(skop(0)),
            SceneElement::SketchMirrorOp(skop(0)),
            SceneElement::SketchVertexTreatmentOp(skop(0)),
            SceneElement::SketchSliceOp(skop(0)),
            SceneElement::SketchText(tkey(0)),
            SceneElement::SliceOp(slckey(0)),
            SceneElement::ShellOp(crate::model::shell_op_key_for_slot(0)),
            SceneElement::EdgeTreatmentOp(etkey(0)),
            SceneElement::Revolution(revolution),
            SceneElement::Shape(primkey(0)),
            SceneElement::SweepOp(sweep),
            SceneElement::Loft(crate::arena::Key::from_bits(0)),
            SceneElement::Drawing(dkey(0)),
            SceneElement::Origin,
            SceneElement::GlobalAxis(GlobalAxis::X),
            SceneElement::SketchFace(FaceId::ConstructionPlane(pkey(0))),
            SceneElement::MovePoint(MovePointRef::Origin),
            SceneElement::ExtrusionEdge {
                extrusion: xkey(0),
                edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            },
            SceneElement::PrimitiveEdge {
                primitive: primkey(0),
                edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            },
            SceneElement::RepeatedFace {
                face: FaceId::ConstructionPlane(pkey(0)),
                op: repkey(0),
                instance: 0,
            },
            SceneElement::Component(ckey(0)),
            SceneElement::UnitInstance(uikey(0)),
            SceneElement::Joint(jkey(0)),
            SceneElement::CrossSection(crate::model::cross_section_key_for_slot(0)),
            SceneElement::SectionPlane {
                view: crate::model::cross_section_key_for_slot(0),
                cut: 0,
            },
            // Keep DrawingElementRef::Text represented so the walk stays a real sample
            // of every *operation* kind; the drawing page itself is Dedicated above.
            SceneElement::DrawingElement {
                drawing: dkey(0),
                element: crate::context::DrawingElementRef::Text(akey(0)),
            },
        ]
    }

    /// #1486: walking every [`SceneElement`] — an `Edit*` action on an operation
    /// implies a double-click / right-click Edit on its Elements-pane row.
    #[test]
    fn edit_action_implies_row_entry() {
        for element in sample_every_scene_element() {
            match element_edit_path(&element) {
                ElementEditPath::Row { has_edit_action } => {
                    let node = hierarchy_node_for_element(&element).unwrap_or_else(|| {
                        panic!("{element:?} is a row-editable operation but has no hierarchy node")
                    });
                    assert_eq!(
                        node_editable_operation(node),
                        Some(element.clone()),
                        "{element:?} should have a row entry (edit_action={has_edit_action})"
                    );
                }
                ElementEditPath::Dedicated | ElementEditPath::None => {
                    if let Some(node) = hierarchy_node_for_element(&element) {
                        assert!(
                            node_editable_operation(node).is_none(),
                            "{element:?} must not use the universal row-edit path"
                        );
                    }
                }
            }
        }
    }
}


/// An element's ordinal among the live ones of its kind (#1055/#1801), or `None` once it is
/// gone. Scripts address elements by this ordinal; a handle re-reads it on every call.
/// Historically `script_json::scene_element_selection_index`: the element's
/// index, or `None` for the point/edge selectors that name a sub-feature of another element
/// rather than a whole element (`Point`/`FaceEdge`).
pub fn element_live_index(
    doc: &crate::model::Document,
    element: &SceneElement,
) -> Option<usize> {
    match element {
        // An arena-backed element reports its **ordinal** among the live ones of its kind
        // (#1055) — the same integer `scene_element_from_kind` takes back.
        SceneElement::Image(key) => doc.tracing_images.keys().position(|k| k == *key),
        SceneElement::CrossSection(key) => doc.cross_sections.keys().position(|k| k == *key),
        SceneElement::SectionPlane { view, cut } => {
            crate::model::section_plane_ordinal(doc, *view, *cut)
        }
        SceneElement::Body(key) => doc.bodies.keys().position(|k| k == *key),
        SceneElement::BooleanOp(key) => doc.boolean_ops.keys().position(|k| k == *key),
        SceneElement::MoveOp(key) => doc.move_ops.keys().position(|k| k == *key),
        SceneElement::MirrorOp(key) => doc.mirror_ops.keys().position(|k| k == *key),
        SceneElement::RepeatOp(key) => doc.repeat_ops.keys().position(|k| k == *key),
        SceneElement::SliceOp(key) => doc.slice_ops.keys().position(|k| k == *key),
        SceneElement::ShellOp(key) => doc.shell_ops.keys().position(|k| k == *key),
        SceneElement::SketchRepeatOp(key) => {
            doc.sketch_repeat_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchOffsetOp(key) => {
            doc.sketch_offset_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchMirrorOp(key) => {
            doc.sketch_mirror_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchVertexTreatmentOp(key) => {
            doc.sketch_vertex_treatment_ops.keys().position(|k| k == *key)
        }
        SceneElement::SketchSliceOp(key) => {
            doc.sketch_slice_ops.keys().position(|k| k == *key)
        }
        SceneElement::EdgeTreatmentOp(key) => {
            doc.edge_treatment_ops.keys().position(|k| k == *key)
        }
        SceneElement::Revolution(key) => doc.revolutions.keys().position(|k| k == *key),
        SceneElement::SweepOp(key) => doc.sweeps.keys().position(|k| k == *key),
        SceneElement::Loft(key) => doc.lofts.keys().position(|k| k == *key),
        SceneElement::Drawing(key) => doc.drawings.keys().position(|k| k == *key),
        SceneElement::Shape(key) => doc.primitives.keys().position(|k| k == *key),
        // A body face (#555) names a sub-feature with no flat index, like Point/FaceEdge.
        SceneElement::Point(_)
        | SceneElement::FaceEdge(_)
        | SceneElement::BodyFace { .. }
        // A cylinder and its centre line are keyed by geometry, not by an index (#1013).
        | SceneElement::BodyCylinder { .. }
        | SceneElement::BodyAxis { .. }
        | SceneElement::SketchFace(_)
        | SceneElement::MovePoint(_) => None,
        SceneElement::ExtrusionEdge { extrusion, .. } => {
            doc.extrusions.keys().position(|k| k == *extrusion)
        }
        SceneElement::PrimitiveEdge { primitive, .. } => {
            doc.primitives.keys().position(|k| k == *primitive)
        }
        SceneElement::RepeatedFace { instance, .. } => Some(*instance),
        // A page item indexes by its place on the page; a dimension has no index of its own,
        // so it reports the view it is shown on (#967).
        SceneElement::DrawingElement { drawing, element } => {
            use crate::context::DrawingElementRef as D;
            Some(match element {
                D::Projection(i) => *i,
                D::Text(key) => doc
                    .drawings
                    .get(*drawing)
                    .and_then(|d| d.annotations.keys().position(|k| k == *key))?,
                D::Dimension { view, .. } | D::PointDim { view, .. } => *view,
            })
        }
        // X/Y/Z report as 0/1/2 (#952), matching `lua_script::element_index`.
        SceneElement::GlobalAxis(axis) => Some(match axis {
            crate::construction::GlobalAxis::X => 0,
            crate::construction::GlobalAxis::Y => 1,
            crate::construction::GlobalAxis::Z => 2,
        }),
        SceneElement::Line(key) => doc.lines.keys().position(|k| k == *key),
        SceneElement::ConstructionPlane(key) => {
            doc.construction_planes.keys().position(|k| k == *key)
        }
        SceneElement::Circle(key) => doc.circles.keys().position(|k| k == *key),
        SceneElement::Sketch(key) => doc.sketches.keys().position(|k| k == *key),
        SceneElement::Constraint(key) => doc.constraints.keys().position(|k| k == *key),
        SceneElement::SketchText(key) => doc.sketch_texts.keys().position(|k| k == *key),
        SceneElement::Extrusion(key) => doc.extrusions.keys().position(|k| k == *key),
        SceneElement::Component(key) => doc.components.keys().position(|k| k == *key),
        SceneElement::UnitInstance(key) => doc.unit_instances.keys().position(|k| k == *key),
        SceneElement::Joint(key) => doc.joints.keys().position(|k| k == *key),
        SceneElement::Origin
        | SceneElement::BodyEdge { .. }
        | SceneElement::BodyVertex { .. }
        | SceneElement::ProjectedEdge { .. }
        | SceneElement::ProjectedCorner { .. } => Some(0),
    }
}

/// The document-unique, never-reused ID of an element (#1801): its kind plus the arena key
/// (slot and generation) it holds, e.g. `"body#3v0"`. Removing an element bumps its slot's
/// generation, so an ID is handed out exactly once for the life of a document — a retired one
/// resolves to nothing rather than to whatever moved into that slot. `None` for the
/// sub-element references (a face, an edge, a point, an axis) that have no identity of their
/// own to hand out.
pub fn element_id(element: &SceneElement) -> Option<String> {
    let (kind, bits) = element_kind_and_key(element)?;
    Some(format!("{kind}#{}v{}", bits >> 32, bits as u32))
}

/// The element an [`element_id`] string names, if it is still in the document. A well-formed
/// ID for an element that has since been deleted reads as `None`, the same as a malformed one.
pub fn element_from_id(doc: &crate::model::Document, id: &str) -> Option<SceneElement> {
    let (kind, rest) = id.split_once('#')?;
    let (slot, generation) = rest.split_once('v')?;
    let bits = ((slot.parse::<u32>().ok()? as u64) << 32) | generation.parse::<u32>().ok()? as u64;
    let element = element_from_kind_and_key(kind, bits)?;
    element_live_index(doc, &element)?;
    Some(element)
}

/// An element's ID kind name and arena key. One match rather than two, so the spelling an ID
/// is written with and the spelling it parses back from cannot drift apart.
fn element_kind_and_key(element: &SceneElement) -> Option<(&'static str, u64)> {
    use SceneElement as E;
    Some(match element {
        E::ConstructionPlane(k) => ("plane", k.to_bits()),
        E::Sketch(k) => ("sketch", k.to_bits()),
        E::Line(k) => ("line", k.to_bits()),
        E::Circle(k) => ("circle", k.to_bits()),
        E::Constraint(k) => ("constraint", k.to_bits()),
        E::Extrusion(k) => ("extrusion", k.to_bits()),
        E::Body(k) => ("body", k.to_bits()),
        E::Image(k) => ("image", k.to_bits()),
        E::BooleanOp(k) => ("boolean_op", k.to_bits()),
        E::MoveOp(k) => ("move_op", k.to_bits()),
        E::MirrorOp(k) => ("mirror_op", k.to_bits()),
        E::RepeatOp(k) => ("repeat_op", k.to_bits()),
        E::SketchRepeatOp(k) => ("sketch_repeat_op", k.to_bits()),
        E::SketchOffsetOp(k) => ("sketch_offset_op", k.to_bits()),
        E::SketchMirrorOp(k) => ("sketch_mirror_op", k.to_bits()),
        E::SketchVertexTreatmentOp(k) => ("sketch_vertex_treatment_op", k.to_bits()),
        E::SketchSliceOp(k) => ("sketch_slice_op", k.to_bits()),
        E::SketchText(k) => ("sketch_text", k.to_bits()),
        E::SliceOp(k) => ("slice_op", k.to_bits()),
        E::ShellOp(k) => ("shell_op", k.to_bits()),
        E::EdgeTreatmentOp(k) => ("edge_treatment_op", k.to_bits()),
        E::Revolution(k) => ("revolution", k.to_bits()),
        E::Shape(k) => ("shape", k.to_bits()),
        E::SweepOp(k) => ("sweep", k.to_bits()),
        E::Loft(k) => ("loft", k.to_bits()),
        E::Drawing(k) => ("drawing", k.to_bits()),
        E::CrossSection(k) => ("cross_section", k.to_bits()),
        E::Component(k) => ("component", k.to_bits()),
        E::UnitInstance(k) => ("unit_instance", k.to_bits()),
        E::Joint(k) => ("joint", k.to_bits()),
        _ => return None,
    })
}

/// The inverse of [`element_kind_and_key`]. Says nothing about whether that element is still
/// in the document — [`element_from_id`] checks that.
fn element_from_kind_and_key(kind: &str, bits: u64) -> Option<SceneElement> {
    use crate::arena::Key;
    use SceneElement as E;
    Some(match kind {
        "plane" => E::ConstructionPlane(Key::from_bits(bits)),
        "sketch" => E::Sketch(Key::from_bits(bits)),
        "line" => E::Line(Key::from_bits(bits)),
        "circle" => E::Circle(Key::from_bits(bits)),
        "constraint" => E::Constraint(Key::from_bits(bits)),
        "extrusion" => E::Extrusion(Key::from_bits(bits)),
        "body" => E::Body(Key::from_bits(bits)),
        "image" => E::Image(Key::from_bits(bits)),
        "boolean_op" => E::BooleanOp(Key::from_bits(bits)),
        "move_op" => E::MoveOp(Key::from_bits(bits)),
        "mirror_op" => E::MirrorOp(Key::from_bits(bits)),
        "repeat_op" => E::RepeatOp(Key::from_bits(bits)),
        "sketch_repeat_op" => E::SketchRepeatOp(Key::from_bits(bits)),
        "sketch_offset_op" => E::SketchOffsetOp(Key::from_bits(bits)),
        "sketch_mirror_op" => E::SketchMirrorOp(Key::from_bits(bits)),
        "sketch_vertex_treatment_op" => E::SketchVertexTreatmentOp(Key::from_bits(bits)),
        "sketch_slice_op" => E::SketchSliceOp(Key::from_bits(bits)),
        "sketch_text" => E::SketchText(Key::from_bits(bits)),
        "slice_op" => E::SliceOp(Key::from_bits(bits)),
        "shell_op" => E::ShellOp(Key::from_bits(bits)),
        "edge_treatment_op" => E::EdgeTreatmentOp(Key::from_bits(bits)),
        "revolution" => E::Revolution(Key::from_bits(bits)),
        "shape" => E::Shape(Key::from_bits(bits)),
        "sweep" => E::SweepOp(Key::from_bits(bits)),
        "loft" => E::Loft(Key::from_bits(bits)),
        "drawing" => E::Drawing(Key::from_bits(bits)),
        "cross_section" => E::CrossSection(Key::from_bits(bits)),
        "component" => E::Component(Key::from_bits(bits)),
        "unit_instance" => E::UnitInstance(Key::from_bits(bits)),
        "joint" => E::Joint(Key::from_bits(bits)),
        _ => return None,
    })
}
