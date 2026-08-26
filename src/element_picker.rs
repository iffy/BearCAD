//! A reusable element-picker control (#213): the single, consistent way every tool gathers
//! the scene elements it operates on.
//!
//! Historically each tool grew its own bespoke selection state (`creating_boolean.a`,
//! `creating_loft.sections`, the Chamfer/Fillet edge set, the constraint tool reading
//! `scene_selection` directly, …) with subtly different click, limit, and highlight rules.
//! [`ElementPicker`] replaces all of that with one configurable control:
//!
//! - it accepts a configurable **subset of element kinds** (planes, lines, bodies, operations,
//!   …), and can further restrict the [`OperationKind`]s it will take;
//! - it enforces a **pick limit** (a whole number, or [`PickLimit::Infinite`]);
//! - it renders like a focusable combo-box input with a generic empty state (the count plus
//!   the pickable kinds' icons, #388), a collapsed
//!   `N ⟨icon⟩` summary per kind, and an expandable popup with a remove button per row (the
//!   rendering lives in the context pane; this module is the state + rules it drives);
//! - it carries a **selected-highlight color** that defaults to the theme's selection color but
//!   can be overridden per picker (e.g. the Slice tool paints its cutters red).
//!
//! This module is deliberately free of egui widget code so the pick/limit/filter rules can be
//! unit-tested in isolation; only the small [`Color32`]/[`IconId`] value types are borrowed.

#![allow(dead_code)]

use crate::hierarchy::SceneElement;
use crate::icons::IconId;
use crate::model::Document;
use eframe::egui::{self, Color32};

/// A user-facing category of selectable scene element. Every [`SceneElement`] maps to exactly
/// one kind (see [`ElementKind::of`]); a picker accepts a configurable subset of kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementKind {
    /// A construction plane (or, for now, a tracing image sitting on one).
    Plane,
    Sketch,
    /// A straight sketch segment.
    Line,
    /// One of the world axes (#952) — pickable wherever a straight reference is wanted (a
    /// Repeat path, a Revolve axis), and distinct from a sketch [`Line`](ElementKind::Line) so a
    /// picker can take one without the other.
    Axis,
    Circle,
    /// A point: a sketch/constraint point, a body corner, or the origin.
    Vertex,
    /// An edge of a body/face boundary (as opposed to a free sketch [`Line`](ElementKind::Line)).
    Edge,
    /// A **cylindrical** surface of a solid body (#1013): a hole's wall, a boss, a shaft. Its
    /// own kind rather than a [`Face`](ElementKind::Face), because it is no plane — a picker
    /// that wants something to sit a part flush on must not be offered one.
    Cylinder,
    /// A flat face of a solid body (#555/#566), distinct from the whole [`Body`](ElementKind::Body):
    /// a picker can accept planes-or-faces without also taking whole bodies. This is the **mesh**
    /// face — a group of coplanar triangles, quantized from the body's geometry.
    Face,
    /// The same surface as an *analytic* face (#957): a sketch profile, a body's extrude cap or
    /// side wall, a revolve's flat face — named by the geometry that generated it rather than by
    /// the triangles it renders as. A body's cap reaches the cursor **both** ways, and the two
    /// are different elements, so they are different kinds: the tools that build from a face
    /// (Extrude, Revolve, Sweep, Loft) want the analytic one, and only a picker taking
    /// everything takes both.
    Profile,
    Constraint,
    /// A projected view on a drawing page (#967).
    Projection,
    /// A cross-section view (#1671): a saved way of looking at the model.
    View,
    /// A text note on a drawing page (#967).
    Annotation,
    /// A dimension shown on a drawing page (#967).
    Dimension,
    /// A solid body.
    Body,
    Image,
    /// A joint between parts (#952) — its own kind, not a history operation, so a picker can
    /// take joints without swallowing every extrude and boolean too.
    Joint,
    /// A component (#952) — likewise its own kind rather than an operation.
    Component,
    /// A history operation (extrude, boolean, move, repeat, slice, revolve). Restrict which ones
    /// with [`ElementFilter::operations`].
    Operation,
}

impl ElementKind {
    /// Kinds in the canonical order used for filter membership and the collapsed summary, so a
    /// picker accepting several kinds always renders them in the same, stable order.
    ///
    /// **Every** kind must appear here: [`ElementFilter::kinds`] builds its accepted set by
    /// walking this list, so a kind left out is one no picker can accept and no summary can
    /// count (which is exactly what happened to `Image`). `every_kind_is_in_the_canonical_order`
    /// guards that.
    pub const ORDER: [ElementKind; 20] = [
        ElementKind::Plane,
        ElementKind::Image,
        ElementKind::Sketch,
        ElementKind::Line,
        ElementKind::Circle,
        ElementKind::Axis,
        ElementKind::Vertex,
        ElementKind::Edge,
        ElementKind::Face,
        ElementKind::Cylinder,
        ElementKind::Profile,
        ElementKind::Constraint,
        ElementKind::Projection,
        ElementKind::View,
        ElementKind::Annotation,
        ElementKind::Dimension,
        ElementKind::Body,
        ElementKind::Component,
        ElementKind::Joint,
        ElementKind::Operation,
    ];

    /// The kind an element belongs to. Total: every [`SceneElement`] has exactly one kind.
    pub fn of(element: &SceneElement) -> ElementKind {
        match element {
            SceneElement::ConstructionPlane(_) => ElementKind::Plane,
            SceneElement::Image(_) => ElementKind::Image,
            SceneElement::Sketch(_) => ElementKind::Sketch,
            SceneElement::Line(_) => ElementKind::Line,
            SceneElement::Circle(_) => ElementKind::Circle,
            SceneElement::Point(_) | SceneElement::BodyVertex { .. } | SceneElement::Origin => {
                ElementKind::Vertex
            }
            SceneElement::GlobalAxis(_) => ElementKind::Axis,
            // An analytic face (#952/#957). `from_face_id` has already peeled off the
            // construction-plane case, so anything left here really is a profile.
            SceneElement::SketchFace(_) => ElementKind::Profile,
            // A Move/Joint snap point (#952) is a point, whatever geometry it sits on.
            SceneElement::MovePoint(_) => ElementKind::Vertex,
            // An extrusion's analytic edge (#952) is an edge, like the mesh edge it draws as.
            SceneElement::ExtrusionEdge { .. } | SceneElement::PrimitiveEdge { .. } => {
                ElementKind::Edge
            }
            // A repeat instance's face (#955) is an analytic one — it is the source face's
            // plane, translated, not any mesh in the document.
            SceneElement::RepeatedFace { .. } => ElementKind::Profile,

            SceneElement::FaceEdge(_) | SceneElement::BodyEdge { .. } => ElementKind::Edge,
            SceneElement::Constraint(_) => ElementKind::Constraint,
            // A drawing's three item types keep their own kinds (#363/#967), so a picker can
            // say "projections only" — which is exactly what the Aligned-view tool's base
            // view wants — and each row keeps the icon the Elements pane gives it.
            SceneElement::DrawingElement { element, .. } => {
                use crate::context::DrawingElementRef as D;
                match element {
                    D::Projection(_) => ElementKind::Projection,
                    D::Text(_) => ElementKind::Annotation,
                    D::Dimension { .. } | D::PointDim { .. } => ElementKind::Dimension,
                }
            }
            // A flat body face (#555/#566) is its own kind, so a "planes or faces" picker can
            // accept it without also accepting whole bodies.
            SceneElement::BodyFace { .. } => ElementKind::Face,
            // A round wall is its own kind (#1013); its centre line is a straight reference,
            // exactly like a world axis.
            SceneElement::BodyCylinder { .. } => ElementKind::Cylinder,
            SceneElement::BodyAxis { .. } => ElementKind::Axis,
            SceneElement::Body(_) => ElementKind::Body,
            SceneElement::Component(_) => ElementKind::Component,
            SceneElement::Joint(_) => ElementKind::Joint,
            SceneElement::UnitInstance(_) => ElementKind::Body,
            SceneElement::Extrusion(_)
            | SceneElement::BooleanOp(_)
            | SceneElement::MoveOp(_)
            | SceneElement::MirrorOp(_)
            | SceneElement::RepeatOp(_)
            | SceneElement::SketchRepeatOp(_)
            | SceneElement::SketchOffsetOp(_)
            | SceneElement::SketchMirrorOp(_)
            | SceneElement::SketchVertexTreatmentOp(_)
            | SceneElement::SketchSliceOp(_)
            | SceneElement::SketchText(_)
            | SceneElement::SliceOp(_)
            | SceneElement::ShellOp(_)
            | SceneElement::EdgeTreatmentOp(_)
            | SceneElement::Revolution(_)
            | SceneElement::Shape(_)
            | SceneElement::SweepOp(_)
            | SceneElement::Loft(_) => ElementKind::Operation,
            SceneElement::Drawing(_) => ElementKind::Projection,
            SceneElement::CrossSection(_) => ElementKind::View,
            SceneElement::SectionPlane { .. } => ElementKind::Plane,
        }
    }

    /// A representative icon for a collapsed summary chip of this kind.
    pub fn icon(self) -> IconId {
        match self {
            ElementKind::Plane => IconId::Plane,
            ElementKind::Image => IconId::Image,
            ElementKind::Sketch => IconId::Sketch,
            ElementKind::Line => IconId::Line,
            ElementKind::Circle => IconId::Circle,
            ElementKind::Axis => IconId::Line,
            // No dedicated point glyph; the coincident icon reads as "a point".
            ElementKind::Vertex => IconId::Coincident,
            ElementKind::Edge => IconId::Line,
            ElementKind::Face | ElementKind::Profile | ElementKind::Cylinder => IconId::Face,
            ElementKind::Constraint => IconId::Constraint,
            ElementKind::Projection => IconId::Projection,
            ElementKind::View => IconId::CrossSection,
            ElementKind::Annotation => IconId::Text,
            ElementKind::Dimension => IconId::Dimension,
            ElementKind::Body => IconId::Body,
            ElementKind::Component => IconId::Component,
            ElementKind::Joint => IconId::Joint,
            ElementKind::Operation => IconId::Gear,
        }
    }

    /// A short human label for hints and tooltips.
    pub fn label(self) -> &'static str {
        match self {
            ElementKind::Plane => "plane",
            ElementKind::Image => "image",
            ElementKind::Sketch => "sketch",
            ElementKind::Line => "line",
            ElementKind::Circle => "circle",
            ElementKind::Axis => "axis",
            ElementKind::Vertex => "vertex",
            ElementKind::Edge => "edge",
            ElementKind::Face => "face",
            ElementKind::Cylinder => "cylinder",
            ElementKind::Profile => "profile",
            ElementKind::Constraint => "constraint",
            ElementKind::Projection => "projection",
            ElementKind::View => "view",
            ElementKind::Annotation => "annotation",
            ElementKind::Dimension => "dimension",
            ElementKind::Body => "body",
            ElementKind::Component => "component",
            ElementKind::Joint => "joint",
            ElementKind::Operation => "operation",
        }
    }
}

/// Toggle a value in a picked set: in if absent, out if present — what a click on an element
/// does to whichever picker takes it.
///
/// One definition (#970). Three copies of this existed: a free function in `main.rs`, a nested
/// one inside `toggle_body_in_active_tool`, and a third in the pane's click routing.
pub fn toggle_picked<T: PartialEq>(set: &mut Vec<T>, value: T) {
    match set.iter().position(|v| *v == value) {
        Some(i) => {
            set.remove(i);
        }
        None => set.push(value),
    }
}

/// The global pick-priority bands (#959): when several things crowd the cursor, the one in the
/// lower band wins, and a tie inside a band goes to whichever is nearest in pixels.
///
/// The sharpest thing first — a corner beats an edge running through it, which beats the face
/// they lie on, which beats a construction plane behind it, which beats a whole body. Kinds that
/// should compete on *distance* rather than on kind share a band: a sketch line and a body edge
/// under the same pixel both read as "the thing you're pointing at", so neither outranks the
/// other.
///
/// A picker may override this with [`ElementPicker::with_priority`]. The match is total, so no
/// pick can fall off the end of the ranking.
pub fn default_pick_band(kind: ElementKind) -> usize {
    match kind {
        ElementKind::Vertex => 0,
        ElementKind::Edge | ElementKind::Line | ElementKind::Circle | ElementKind::Axis => 1,
        ElementKind::Constraint => 2,
        // A drawing's items don't share a viewport with anything else, so their band only has
        // to order them among themselves: a dimension over a note over the view beneath both.
        ElementKind::Dimension => 2,
        ElementKind::Annotation => 3,
        ElementKind::Projection => 5,
        ElementKind::View => 5,
        ElementKind::Face | ElementKind::Profile | ElementKind::Cylinder => 3,
        ElementKind::Plane | ElementKind::Image => 4,
        ElementKind::Sketch => 5,
        ElementKind::Body => 6,
        ElementKind::Component | ElementKind::Joint | ElementKind::Operation => 7,
    }
}

/// A history-operation sub-kind, so a picker can accept e.g. only bodies produced by a boolean
/// while rejecting move/repeat operations (the user's "limit it to selecting only certain
/// operations").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperationKind {
    Extrude,
    Boolean,
    Move,
    Mirror,
    Repeat,
    Slice,
    Shell,
    EdgeTreatment,
    Revolution,
    Sweep,
    Loft,
    Shape,
    SketchText,
    SketchRepeat,
    SketchOffset,
    SketchMirror,
    SketchVertexTreatment,
    SketchSlice,
}

impl OperationKind {
    /// The operation sub-kind of an element, or `None` if the element is not an operation.
    pub fn of(element: &SceneElement) -> Option<OperationKind> {
        Some(match element {
            SceneElement::Extrusion(_) => OperationKind::Extrude,
            SceneElement::BooleanOp(_) => OperationKind::Boolean,
            SceneElement::MoveOp(_) => OperationKind::Move,
            SceneElement::MirrorOp(_) => OperationKind::Mirror,
            SceneElement::RepeatOp(_) => OperationKind::Repeat,
            SceneElement::SliceOp(_) => OperationKind::Slice,
            SceneElement::ShellOp(_) => OperationKind::Shell,
            SceneElement::EdgeTreatmentOp(_) => OperationKind::EdgeTreatment,
            SceneElement::Revolution(_) => OperationKind::Revolution,
            SceneElement::SweepOp(_) => OperationKind::Sweep,
            SceneElement::Loft(_) => OperationKind::Loft,
            SceneElement::Shape(_) => OperationKind::Shape,
            SceneElement::SketchText(_) => OperationKind::SketchText,
            SceneElement::SketchRepeatOp(_) => OperationKind::SketchRepeat,
            SceneElement::SketchOffsetOp(_) => OperationKind::SketchOffset,
            SceneElement::SketchMirrorOp(_) => OperationKind::SketchMirror,
            SceneElement::SketchVertexTreatmentOp(_) => OperationKind::SketchVertexTreatment,
            SceneElement::SketchSliceOp(_) => OperationKind::SketchSlice,
            _ => return None,
        })
    }
}

/// An instance-level restriction on what a picker will take (#953), beyond its element kinds —
/// the design's "restrict selection to particular elements/components/bodies". A picker's rules
/// **all** have to pass.
///
/// Data-carrying rather than a boxed closure because [`ElementPicker`] lives inside
/// `ContextPaneContent`, which is `Clone + Debug + PartialEq` and diffed every frame. A closure
/// breaks all three; a plain `fn` pointer keeps them but can't capture the state these rules
/// need (which bodies are moving, what a sibling picker already holds).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickRule {
    /// Only geometry belonging to this sketch (#742): while a sketch is open, Select and
    /// Constraint touch only its own geometry.
    InSketch(crate::model::SketchId),
    /// What the Projection tool can source for this sketch (#983): **outside** geometry a
    /// projection resolves — a body, its edges/corners, a construction plane that actually
    /// crosses the sketch plane — plus a line already projected *into* this sketch, picked
    /// to un-project it. Never the sketch's own drawn geometry.
    ProjectableInto(crate::model::SketchId),
    /// Only bodies that exist, aren't deleted, and aren't shadow (already consumed by another
    /// operation). Non-body elements pass — combine with a body-only kind filter.
    LiveBody,
    /// Only geometry sitting on one of these bodies — the Move tool's start points, which must
    /// land on a **moving** body (#649).
    OnBodies(Vec<crate::model::BodyKey>),
    /// Only geometry **not** sitting on one of these bodies — the Move tool's end points, which
    /// land on stationary geometry (#650). Geometry belonging to no body at all (the origin, a
    /// world axis) counts as stationary.
OffBodies(Vec<crate::model::BodyKey>),
    /// A 2D Move point (#1601): an image box point slides in its plane, so the Move tool's
    /// point pickers answer it as well as a body corner. `moving` picks the side the way the
    /// body list Gates a body — a start point lands on a **moving** image (or, while none is
    /// picked yet, any image — the click declares the mover), an end point on a stationary
    /// one or the world origin.
    MovePoint2D(bool, Vec<crate::model::BodyKey>, Vec<crate::model::TracingImageKey>),
    /// Only straight references: a sketch line with no bezier, a body edge, or a world axis.
    /// The Revolve axis and Repeat path pickers. Whether a circle is on the menu is the
    /// filter's kind list to say — Repeat takes one as a path (#840), Revolve does not.
    Straight,
    /// Only construction geometry (`true`) or only real geometry (`false`).
    Construction(bool),
    /// Excludes what a sibling picker already holds — Combine's B side against its A side.
    NotIn(Vec<SceneElement>),
    /// Only points lying **on** this face (#1075) — the second half of a Face Snap pick.
    /// The face is whatever the picker's first stage took, so this rule is injected by the
    /// picker rather than configured by the caller.
    OnFace(Box<SceneElement>),
    /// A 2D mirror line (#1538): a straight sketch line, a sketch origin axis, or a world
    /// axis. Body edges and other edges stay out — those are 3D references.
    MirrorLine,
}

/// Whether `element` is a point lying on `face` (#1075) — the filter Face Snap's second pick
/// runs on. "On the face" is geometric, not structural: any point that resolves to a world
/// position in the face's plane and inside its outline counts, so a corner, an edge midpoint,
/// or a point already recorded as being on that face all pass, while a corner on the far side
/// of the body does not.
fn point_lies_on_face(doc: &Document, face: &SceneElement, element: &SceneElement) -> bool {
    let SceneElement::BodyFace { body, centroid, normal } = face else {
        return false;
    };
    let Some(world) = point_world_position(doc, element) else {
        return false;
    };
    crate::extrude::body_face_triangles(doc, *body, *centroid, *normal)
        .is_some_and(|tris| crate::extrude::face_group_contains(&tris, world))
}

/// The world position of an element that *is* a point — a picked snap point, or a body corner
/// picked directly. `None` for anything with extent, which is never a point on a face.
fn point_world_position(doc: &Document, element: &SceneElement) -> Option<glam::Vec3> {
    match element {
        SceneElement::MovePoint(point) => crate::extrude::move_point_world(doc, point),
        SceneElement::BodyVertex { body, p } => {
            crate::parameters::body_vertex_world_position(doc, *body, *p)
        }
        _ => None,
    }
}

impl PickRule {
    /// Whether this rule lets the element through.
    pub fn allows(&self, doc: &Document, element: &SceneElement) -> bool {
        match self {
            PickRule::OnFace(face) => point_lies_on_face(doc, face, element),
            PickRule::InSketch(sketch) => element_in_sketch(doc, *sketch, element),
            PickRule::ProjectableInto(sketch) => match element {
                // A projected line of this sketch is re-picked to un-project it; every other
                // sketch line — this sketch's own geometry, or another sketch's, which no
                // projection source can track — is refused.
                SceneElement::Line(li) => doc.lines.get(*li).is_some_and(|l| {
                    l.sketch == *sketch && l.projection.is_some()
                }),
                SceneElement::Body(_)
                | SceneElement::BodyEdge { .. }
                | SceneElement::BodyVertex { .. } => true,
                // Only a plane the projection can actually resolve — one that crosses the
                // sketch plane — so the fan never offers a parallel plane a click would refuse.
                SceneElement::ConstructionPlane(plane) => {
                    crate::projection::plane_sketch_intersection(doc, *sketch, *plane).is_some()
                }
                _ => false,
            },
            PickRule::LiveBody => match element {
                SceneElement::Body(index) => {
                    doc.bodies.get(*index).is_some_and(|b| !b.shadow)
                }
                _ => true,
            },
            PickRule::OnBodies(bodies) => {
                element_body(doc, element).is_some_and(|b| bodies.contains(&b))
            }
            PickRule::OffBodies(bodies) => {
                element_body(doc, element).is_none_or(|b| !bodies.contains(&b))
            }
            PickRule::MovePoint2D(moving, bodies, images) => match element {
                SceneElement::Point(crate::model::ConstraintPoint::ImageAnchor { image, .. }) => {
                    if *moving {
                        images.is_empty() || images.contains(image)
                    } else {
                        !images.contains(image)
                    }
                }
                SceneElement::Origin => !*moving,
                _ => element_body(doc, element).is_some_and(|b| {
                    if *moving { bodies.contains(&b) } else { !bodies.contains(&b) }
                }),
            },
            PickRule::Straight => match element {
                SceneElement::Line(index) => {
                    doc.lines.get(*index).is_some_and(|l| l.bezier.is_none())
                }
                // Sketch-local origin axes have no `RevolveAxis` mapping — a 3D axis picker
                // must not hover them as if a click would take them.
                SceneElement::FaceEdge(crate::model::ConstraintLine::OriginAxis(_)) => false,
                _ => true,
            },
            PickRule::Construction(want) => match element {
                SceneElement::Line(index) => {
                    doc.lines.get(*index).is_some_and(|l| l.construction == *want)
                }
                SceneElement::Circle(index) => doc
                    .circles
                    .get(*index)
                    .is_some_and(|c| c.construction == *want),
                _ => true,
            },
            PickRule::NotIn(excluded) => !excluded.contains(element),
            PickRule::MirrorLine => match element {
                SceneElement::Line(index) => {
                    doc.lines.get(*index).is_some_and(|l| l.bezier.is_none())
                }
                SceneElement::FaceEdge(crate::model::ConstraintLine::OriginAxis(_)) => true,
                SceneElement::GlobalAxis(_) => true,
                _ => false,
            },
        }
    }
}

/// The body an element sits on, for [`PickRule::OnBodies`]/[`OffBodies`](PickRule::OffBodies).
/// `None` for anything that belongs to no body — the origin, a world axis, sketch geometry.
fn element_body(doc: &Document, element: &SceneElement) -> Option<crate::model::BodyKey> {
    match element {
        SceneElement::Body(index) => Some(*index),
        SceneElement::BodyEdge { body, .. }
        | SceneElement::BodyVertex { body, .. }
        | SceneElement::BodyFace { body, .. } => Some(*body),
        SceneElement::MovePoint(point) => point.body(),
        // Every face that has a body, not just extrusion and revolve ones (#1726): a
        // primitive's face belongs to the primitive's body, a repeated face to its instance,
        // a mesh face to the body it was read off.
        SceneElement::SketchFace(face) => crate::model::body_index_for_face(doc, face),
        SceneElement::ExtrusionEdge { extrusion, .. } => {
            crate::model::body_index_for_extrusion(doc, *extrusion)
        }
        SceneElement::PrimitiveEdge { primitive, .. } => {
            crate::model::body_index_for_primitive(doc, *primitive)
        }
        _ => None,
    }
}

/// Whether a selection-family pick belongs to the open sketch (#742): while a sketch is
/// being edited, Select and Constraint touch only that sketch's own geometry — its lines,
/// circles, points, and text, the origin and its axes, and the sketched-on face's own
/// edges and corners. Outside bodies and other sketches stay untouchable until the
/// Project tool references them in.
pub fn element_in_sketch(
    doc: &Document,
    sketch: crate::model::SketchId,
    element: &SceneElement,
) -> bool {
    let line_in = |li: crate::model::LineKey| doc.lines.get(li).is_some_and(|l| l.sketch == sketch);
    let circle_in = |ci: crate::model::CircleKey| doc.circles.get(ci).is_some_and(|c| c.sketch == sketch);
    let text_in = |ti: crate::model::SketchTextKey| doc.sketch_texts.get(ti).is_some_and(|t| t.sketch == sketch);
    let host_face = doc.sketch_face(sketch);
    let host_body = host_face.as_ref().and_then(|face| {
        face.extrusion_index()
            .and_then(|e| crate::model::body_index_for_extrusion(doc, e))
            .or_else(|| {
                face.revolution_key()
                    .and_then(|r| crate::model::body_index_for_revolution(doc, r))
            })
    });
    let body_in_sketch = |body: &crate::model::BodyKey| host_body.is_some_and(|hb| hb == *body);
    let constraint_line_in = |cl: &crate::model::ConstraintLine| match cl {
        crate::model::ConstraintLine::Line(li) => line_in(*li),
        crate::model::ConstraintLine::FaceEdge { face, .. } => Some(face) == host_face.as_ref(),
        crate::model::ConstraintLine::OriginAxis(_) => true,
        crate::model::ConstraintLine::ImageEdge { .. } => true,
    };
    match element {
        SceneElement::Line(li) => line_in(*li),
        SceneElement::Circle(ci) => circle_in(*ci),
        SceneElement::SketchText(ti) => text_in(*ti),
        SceneElement::Point(point) => match point {
            crate::model::ConstraintPoint::LineEndpoint { line, .. } => line_in(*line),
            crate::model::ConstraintPoint::CircleCenter(ci) => circle_in(*ci),
            crate::model::ConstraintPoint::FaceVertex { face, .. } => Some(face) == host_face.as_ref(),
            crate::model::ConstraintPoint::TextAnchor { text, .. } => text_in(*text),
            // Gated to the host plane at creation; nothing sketch-foreign resolves here.
            crate::model::ConstraintPoint::ImageCalibrationPoint { .. }
            | crate::model::ConstraintPoint::ImageAnchor { .. } => true,
            crate::model::ConstraintPoint::Origin => true,
        },
        SceneElement::FaceEdge(cl) => constraint_line_in(cl),
        SceneElement::BodyEdge { body, .. }
        | SceneElement::BodyFace { body, .. }
        | SceneElement::BodyVertex { body, .. } => body_in_sketch(body),
        SceneElement::Origin => true,
        SceneElement::GlobalAxis(axis) => {
            crate::face::world_dir_in_sketch_plane(doc, sketch, axis.direction())
        }
        SceneElement::Constraint(ci) => doc.constraints.get(*ci).is_some_and(|c| c.sketch == sketch),
        _ => false,
    }
}

/// The maximal tangent-continuous run of sketch lines through `li` (#984), sorted ascending
/// and always containing `li` itself. Two lines chain where **exactly two** line-ends meet at
/// a shared endpoint and their away-directions are nearly opposite — the same 30° rule (and
/// the same [`crate::gpu_viewport::chain_by_tangency`] union-find) the solid-mesh feature-edge
/// chains use (#626) — so a straight line that breaks into a tangent curve and exits again as
/// a tangent line reads as one line-curve-line run. Corners and junctions of 3+ ends break the
/// chain. Only the line's own sketch participates; deleted and shadow lines don't.
pub fn sketch_line_tangent_chain(doc: &Document, li: crate::model::LineKey) -> Vec<crate::model::LineKey> {
    use glam::Vec3;
    let Some(line) = doc.lines.get(li).filter(|l| !l.shadow) else {
        return vec![li];
    };
    let lines: Vec<crate::model::LineKey> = doc
        .lines
        .iter()
        .filter(|(_, l)| !l.shadow && l.sketch == line.sketch)
        .map(|(i, _)| i)
        .collect();
    // 0.001 sketch-unit precision, like the solid-mesh vertex key.
    let quantize = |x: f32, y: f32| ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64, 0);
    // Direction leaving an endpoint into the line: along the bezier handle for a curved line
    // (that is its tangent there), along the chord for a straight one — falling back to the
    // chord when a degenerate handle sits on its own endpoint.
    let away = |l: &crate::model::Line, from_start: bool| -> Vec3 {
        let (px, py, handle) = match (from_start, l.bezier) {
            (true, h) => (l.x0, l.y0, h.map(|b| b[0])),
            (false, h) => (l.x1, l.y1, h.map(|b| b[1])),
        };
        let (tx, ty) = handle
            .filter(|(hx, hy)| (hx - px).abs() > 1e-6 || (hy - py).abs() > 1e-6)
            .unwrap_or(if from_start { (l.x1, l.y1) } else { (l.x0, l.y0) });
        Vec3::new(tx - px, ty - py, 0.0).normalize_or_zero()
    };
    let ends: Vec<[((i64, i64, i64), Vec3); 2]> = lines
        .iter()
        .map(|&i| {
            let l = &doc.lines[i];
            [
                (quantize(l.x0, l.y0), away(l, true)),
                (quantize(l.x1, l.y1), away(l, false)),
            ]
        })
        .collect();
    for chain in crate::gpu_viewport::chain_by_tangency(&ends) {
        let mut members: Vec<crate::model::LineKey> = chain.into_iter().map(|i| lines[i]).collect();
        if members.contains(&li) {
            members.sort_unstable();
            return members;
        }
    }
    vec![li]
}

/// What a pick of `element` should actually put into `picker` (#960).
///
/// Normally just the element. But when the picker takes **edges** and not **faces**, clicking a
/// face means "all of that face's edges" — otherwise a face click is a dead click, since the
/// picker refuses it and nothing says why. The same rule covers a sketch profile when the
/// picker wants lines: its boundary lines are what you meant.
///
/// And a sketch line means its whole tangent-continuous run (#984) when `chain` is set —
/// clicking any segment of a line-curve-line run picks the run as one unit. `chain` is false
/// when the user holds **Control**, which picks only the edge under the cursor. A single-slot
/// picker never chains, for the same reason it never takes a face's edges (#955): the run has
/// nowhere to go.
///
/// Empty when the pick has nothing to offer this picker.
pub fn expand_pick(
    doc: &Document,
    picker: &ElementPicker,
    element: &SceneElement,
    chain: bool,
) -> Vec<SceneElement> {
    if picker.accepts(doc, element) {
        if chain && !picker.limit().is_single() {
            if let SceneElement::Line(li) = element {
                return sketch_line_tangent_chain(doc, *li)
                    .into_iter()
                    .map(SceneElement::Line)
                    .filter(|e| picker.accepts(doc, e))
                    .collect();
            }
        }
        return vec![element.clone()];
    }
    // A picker that takes whole **bodies** takes a click anywhere on one (#218): its faces,
    // edges and corners all resolve to the body they belong to. Checked before the face
    // expansion, so a body picker gets the body rather than the face's edges.
    if picker.active_filter().accepts_kind(ElementKind::Body) {
        if let Some(body) = element_body(doc, element) {
            let whole = SceneElement::Body(body);
            if picker.accepts(doc, &whole) {
                return vec![whole];
            }
        }
    }
    // Otherwise expand a face — either representation of one (#957) — and only into kinds this
    // picker takes but can't reach directly.
    if !matches!(
        ElementKind::of(element),
        ElementKind::Face | ElementKind::Profile
    ) {
        return Vec::new();
    }
    // Not into a single-pick input, though: "all of this face's edges" has nowhere to go in a
    // picker with one slot, and offering it means a Revolve axis or a Repeat path lights up
    // every edge of the face under the cursor instead of the one edge it can take (#955).
    if picker.limit() == PickLimit::Finite(1) {
        return Vec::new();
    }
    face_boundary_elements(doc, element)
        .into_iter()
        .filter(|e| picker.accepts(doc, e))
        .collect()
}

/// The elements bounding a face: a mesh face's feature edges, or an analytic face's own
/// boundary geometry (a profile's lines, a circle profile's circle).
fn face_boundary_elements(doc: &Document, face: &SceneElement) -> Vec<SceneElement> {
    match face {
        SceneElement::BodyFace { body, centroid, normal } => {
            let Some(tris) = crate::extrude::body_face_triangles(doc, *body, *centroid, *normal)
            else {
                return Vec::new();
            };
            crate::construction::coplanar_face_boundary(&tris)
                .into_iter()
                .map(|(a, b)| {
                    // Canonically ordered, like every other body-edge key.
                    let (qa, qb) = (
                        crate::hierarchy::quantize_body_point(a),
                        crate::hierarchy::quantize_body_point(b),
                    );
                    let (qa, qb) = if qa <= qb { (qa, qb) } else { (qb, qa) };
                    SceneElement::BodyEdge { body: *body, a: qa, b: qb }
                })
                .collect()
        }
        SceneElement::SketchFace(face) => match face {
            crate::model::FaceId::Polygon(lines) => {
                lines.iter().map(|&li| SceneElement::Line(li)).collect()
            }
            crate::model::FaceId::Circle(ci) => vec![SceneElement::Circle(*ci)],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The second half of a [staged picker](ElementPicker::face_then_point) (#1075).
#[derive(Clone, Debug, PartialEq)]
pub struct PickStage {
    /// What the second pick accepts, before the first pick scopes it.
    filter: ElementFilter,
    /// How the first pick scopes it.
    scope: PickScope,
}

/// How a staged picker's first pick narrows its second (#1075).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickScope {
    /// Only points lying on the face picked first.
    OnPickedFace,
}

/// Which elements a picker will accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementFilter {
    /// Accept every kind and every operation. The Select tool's "select everything" picker.
    everything: bool,
    /// Accepted kinds (ignored when `everything`). Ordered per [`ElementKind::ORDER`].
    kinds: Vec<ElementKind>,
    /// When `Some`, an [`ElementKind::Operation`] element is accepted only if its
    /// [`OperationKind`] is listed. `None` accepts every operation (subject to `kinds`).
    operations: Option<Vec<OperationKind>>,
    /// Instance-level restrictions (#953), all of which must pass. Applied **after** the kind
    /// check, and applied even by an `everything` filter — the Select tool has none, but a
    /// picker that takes any kind within one sketch is a perfectly good configuration.
    rules: Vec<PickRule>,
}

impl ElementFilter {
    /// Accept anything selectable — used by the Select tool.
    pub fn everything() -> ElementFilter {
        ElementFilter {
            everything: true,
            kinds: Vec::new(),
            operations: None,
            rules: Vec::new(),
        }
    }

    /// Accept exactly the given kinds (deduplicated, canonically ordered).
    pub fn kinds(kinds: &[ElementKind]) -> ElementFilter {
        let mut ordered = Vec::new();
        for &k in ElementKind::ORDER.iter() {
            if kinds.contains(&k) {
                ordered.push(k);
            }
        }
        // Image shares the Plane row in ORDER; accept it explicitly when Plane is requested so a
        // "planes" picker also takes tracing images sitting on a plane.
        ElementFilter {
            everything: false,
            kinds: ordered,
            operations: None,
            rules: Vec::new(),
        }
    }

    /// A single-kind filter (the common case, e.g. "bodies only").
    pub fn kind(kind: ElementKind) -> ElementFilter {
        ElementFilter::kinds(&[kind])
    }

    /// Add an instance-level restriction (#953). Chainable; every rule added must pass.
    pub fn rule(mut self, rule: PickRule) -> ElementFilter {
        self.rules.push(rule);
        self
    }

    /// This filter's instance-level restrictions.
    pub fn rules(&self) -> &[PickRule] {
        &self.rules
    }

    /// Restrict accepted operations to the given sub-kinds. Implies [`ElementKind::Operation`].
    pub fn operations(mut self, ops: &[OperationKind]) -> ElementFilter {
        if !self.everything && !self.kinds.contains(&ElementKind::Operation) {
            self.kinds.push(ElementKind::Operation);
        }
        self.operations = Some(ops.to_vec());
        self
    }

    /// Whether a whole kind is (potentially) acceptable — drives hover styling of every element
    /// of that category while the picker is focused.
    /// The icons of the kinds this filter accepts, in canonical order, for the picker's
    /// generic empty state (#388). An accept-everything filter returns none — a bare count
    /// reads better than every icon at once.
    pub fn pickable_icons(&self) -> Vec<IconId> {
        if self.everything {
            return Vec::new();
        }
        let mut icons = Vec::new();
        for kind in &self.kinds {
            let icon = kind.icon();
            if !icons.contains(&icon) {
                icons.push(icon);
            }
        }
        icons
    }

    /// The kinds this filter accepts, in canonical order — for reporting a picker's
    /// configuration (#968). An accept-everything filter reports every kind.
    pub fn accepted_kinds(&self) -> Vec<ElementKind> {
        if self.everything {
            return ElementKind::ORDER.to_vec();
        }
        self.kinds.clone()
    }

    pub fn accepts_kind(&self, kind: ElementKind) -> bool {
        if self.everything {
            return true;
        }
        // Image is accepted wherever Plane is (see `kinds`).
        self.kinds.contains(&kind) || (kind == ElementKind::Image && self.kinds.contains(&ElementKind::Plane))
    }

    /// Whether a specific element is acceptable: its kind, then the operation restriction, then
    /// every instance-level [`PickRule`] (#953).
    pub fn accepts(&self, doc: &Document, element: &SceneElement) -> bool {
        if !self.rules.iter().all(|rule| rule.allows(doc, element)) {
            return false;
        }
        if self.everything {
            return true;
        }
        let kind = ElementKind::of(element);
        if !self.accepts_kind(kind) {
            return false;
        }
        if kind == ElementKind::Operation {
            if let Some(allowed) = &self.operations {
                return OperationKind::of(element).is_some_and(|op| allowed.contains(&op));
            }
        }
        true
    }
}

/// How many elements a picker will hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickLimit {
    /// At most `n` elements. `Finite(1)` is single-select: a new pick replaces the current one.
    Finite(usize),
    /// No cap.
    Infinite,
}

impl PickLimit {
    /// Whether one more element could be added when `current` are already picked.
    pub fn has_room(self, current: usize) -> bool {
        match self {
            PickLimit::Finite(n) => current < n,
            PickLimit::Infinite => true,
        }
    }

    /// Single-select pickers replace rather than reject on a new pick.
    pub fn is_single(self) -> bool {
        matches!(self, PickLimit::Finite(1))
    }
}

/// What happened when an element was offered to a picker via [`ElementPicker::pick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickOutcome {
    /// Newly added to the set.
    Added,
    /// Already present, so the click toggled it off.
    Removed,
    /// The picker replaced its single held element (a `Finite(1)` picker).
    Replaced,
    /// Rejected: wrong kind/operation for this picker's filter.
    NotAccepted,
    /// Rejected: the set is already at its (multi-element) limit.
    Full,
}

/// A configurable, focusable element-selection control. Holds both the configuration (filter,
/// limit, highlight color) and the live picked set + focus state.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementPicker {
    filter: ElementFilter,
    /// A second stage (#1075): once the first element is picked, this filter — scoped by
    /// that pick — is what the picker accepts. `None` is the ordinary flat picker.
    stage: Option<PickStage>,
    limit: PickLimit,
    /// Overrides the theme selection color for this picker's highlights (e.g. Slice cutters red).
    selected_color: Option<Color32>,
    /// Kinds this picker prefers among a crowd at the cursor, most-wanted first (#959).
    /// Empty means the global [`DEFAULT_PICK_PRIORITY`].
    priority: Vec<ElementKind>,

    /// Picked elements in click order (stable for the popup rows and remove-by-index).
    picked: Vec<SceneElement>,
    focused: bool,
}

impl ElementPicker {
    /// A picker with the given filter and limit, unfocused, empty, default highlight color.
    pub fn new(filter: ElementFilter, limit: PickLimit) -> ElementPicker {
        ElementPicker {
            filter,
            stage: None,
            limit,
            selected_color: None,
            priority: Vec::new(),
            picked: Vec::new(),
            focused: false,
        }
    }

    /// A picker that takes **two elements in sequence** (#1075), the second scoped by the
    /// first: a face, then a point on that face.
    ///
    /// The alternative was a bespoke two-click interaction, which would have had to
    /// reimplement the Selection Exploder — the exploder asks the picker what it accepts, so
    /// putting the sequence inside the picker gets the crowd fan-out of the currently-valid
    /// set for nothing.
    pub fn face_then_point(face: ElementFilter, point: ElementFilter) -> ElementPicker {
        let mut picker = ElementPicker::new(face, PickLimit::Finite(2));
        picker.stage = Some(PickStage { filter: point, scope: PickScope::OnPickedFace });
        picker
    }

    /// The scene selection's picker (#966): accepts everything, unbounded, focused.
    ///
    /// It used to carry a `sticky_focus` flag meaning "never blurs". That is the focus model's
    /// job, not a property of the control: the selection picker is the Select tool's **only**
    /// picker, so it is focused whenever no other one is — which is what "exactly one picker
    /// has focus" already says. What made the flag look necessary was that it also stood in for
    /// "this picker is the selection, not a tool's gathered set"; `PickerTarget::Selection`
    /// says that directly.
    pub fn select_everything() -> ElementPicker {
        let mut picker = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        picker.focused = true;
        picker
    }

    // ---- builders -------------------------------------------------------------------------

    pub fn with_selected_color(mut self, color: Color32) -> ElementPicker {
        self.selected_color = Some(color);
        self
    }

    /// This picker with one more instance-level [`PickRule`] on its filter (#953) — how an
    /// already-configured picker (the selection's, say) is scoped to the open sketch.
    pub fn with_rule(mut self, rule: PickRule) -> ElementPicker {
        self.filter = self.filter.rule(rule);
        self
    }

    /// Override the global pick priority (#959) for this picker: the listed kinds win over
    /// everything else, in the order given. Kinds left out keep their relative
    /// [`DEFAULT_PICK_PRIORITY`] order, behind every listed kind — so an override only has to
    /// name what it wants promoted ("faces over edges"), not restate the whole list.
    pub fn with_priority(mut self, kinds: &[ElementKind]) -> ElementPicker {
        self.priority = kinds.to_vec();
        self
    }

    /// How strongly this picker wants `kind` among a crowd at the cursor — lower wins. Ties
    /// (kinds in the same band) are broken by pixel distance by the caller.
    pub fn rank(&self, kind: ElementKind) -> usize {
        match self.priority.iter().position(|k| *k == kind) {
            Some(i) => i,
            // Behind everything the override named, in the default order.
            None => self.priority.len() + default_pick_band(kind),
        }
    }

    // ---- configuration accessors ----------------------------------------------------------

    pub fn filter(&self) -> &ElementFilter {
        &self.filter
    }

    /// The filter in force **right now** (#1075). For a flat picker that is always the base
    /// filter; for a staged one it becomes the second stage's, scoped by the first pick, as
    /// soon as there is a first pick. Everything that asks what this picker takes — the
    /// exploder, the hover, the click — goes through [`accepts`](Self::accepts), so the
    /// selectable set changes as the picker is used without any of them knowing.
    pub fn active_filter(&self) -> ElementFilter {
        match (&self.stage, self.picked.first()) {
            (Some(stage), Some(first)) => match stage.scope {
                PickScope::OnPickedFace => {
                    stage.filter.clone().rule(PickRule::OnFace(Box::new(first.clone())))
                }
            },
            _ => self.filter.clone(),
        }
    }

    /// Whether this picker takes its two elements in sequence (#1075).
    pub fn is_staged(&self) -> bool {
        self.stage.is_some()
    }

    pub fn limit(&self) -> PickLimit {
        self.limit
    }

    /// The highlight color for this picker's selected elements, resolving the per-picker override
    /// against the caller-supplied theme default.
    pub fn selected_color(&self, default: Color32) -> Color32 {
        self.selected_color.unwrap_or(default)
    }

    /// Whether this element is one this picker would accept (delegates to the filter).
    pub fn accepts(&self, doc: &Document, element: &SceneElement) -> bool {
        self.active_filter().accepts(doc, element)
    }

    // ---- focus ----------------------------------------------------------------------------

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    // ---- picked set -----------------------------------------------------------------------

    pub fn picked(&self) -> &[SceneElement] {
        &self.picked
    }

    pub fn iter(&self) -> impl Iterator<Item = &SceneElement> {
        self.picked.iter()
    }

    pub fn len(&self) -> usize {
        self.picked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.picked.is_empty()
    }

    pub fn contains(&self, element: &SceneElement) -> bool {
        self.picked.contains(element)
    }

    /// Whether the set is at its limit (a `Finite` limit that's reached; never for `Infinite`).
    pub fn is_full(&self) -> bool {
        !self.limit.has_room(self.picked.len())
    }

    /// Offer an element to the picker. Toggles off if already present; otherwise adds it when the
    /// filter accepts it and there is room, replacing the sole element for a single-select picker.
    pub fn pick(&mut self, doc: &Document, element: SceneElement) -> PickOutcome {
        if let Some(pos) = self.picked.iter().position(|e| e == &element) {
            // `remove_index` is what drops a staged picker's second element along with its
            // first (#1075).
            self.remove_index(pos);
            return PickOutcome::Removed;
        }
        // A staged picker that is full still takes a **new face**: starting the sequence over
        // is the only thing a second face could mean (#1075).
        if self.stage.is_some() && self.is_full() && self.filter.accepts(doc, &element) {
            self.picked.clear();
            self.picked.push(element);
            return PickOutcome::Replaced;
        }
        if !self.accepts(doc, &element) {
            return PickOutcome::NotAccepted;
        }
        if self.is_full() {
            if self.limit.is_single() {
                self.picked.clear();
                self.picked.push(element);
                return PickOutcome::Replaced;
            }
            return PickOutcome::Full;
        }
        self.picked.push(element);
        PickOutcome::Added
    }

    /// Remove a specific element if present; returns whether it was there.
    pub fn remove(&mut self, element: &SceneElement) -> bool {
        match self.picked.iter().position(|e| e == element) {
            Some(pos) => {
                self.remove_index(pos);
                true
            }
            None => false,
        }
    }

    /// Remove the element at a popup-row index (the popup builds rows from [`picked`]).
    ///
    /// On a staged picker (#1075) removing the first element removes the second too: the
    /// second was only ever "a point on *that* face".
    pub fn remove_index(&mut self, index: usize) -> Option<SceneElement> {
        if index >= self.picked.len() {
            return None;
        }
        let gone = self.picked.remove(index);
        if self.stage.is_some() && index == 0 {
            self.picked.clear();
        }
        Some(gone)
    }

    pub fn clear(&mut self) {
        self.picked.clear();
    }

    /// Add an element with no filter or limit check, keeping pick order.
    ///
    /// The unchecked door, for the two callers that have already decided: the scene selection
    /// (#966), whose picker takes everything anyway, and folding a tool's in-progress set into
    /// what the viewport highlights. Everything a *user* picks goes through
    /// [`pick`](Self::pick) or [`set_picked`](Self::set_picked), which do check.
    pub fn push(&mut self, element: SceneElement) {
        self.picked.push(element);
    }

    /// Drop picked elements that no longer satisfy `keep` — e.g. removed by a delete.
    pub fn retain(&mut self, mut keep: impl FnMut(&SceneElement) -> bool) {
        self.picked.retain(|e| keep(e));
    }

    /// Replace the whole picked set (e.g. re-syncing an edit session from a committed operation).
    /// Elements the filter rejects are dropped, and the limit is honored.
    pub fn set_picked(
        &mut self,
        doc: &Document,
        elements: impl IntoIterator<Item = SceneElement>,
    ) {
        self.picked.clear();
        for element in elements {
            if self.is_full() {
                break;
            }
            if self.filter.accepts(doc, &element) && !self.picked.contains(&element) {
                self.picked.push(element);
            }
        }
    }

    /// The collapsed summary: one `(icon, count)` chip per present kind, in canonical kind order.
    /// This is what the un-expanded control shows, e.g. `2 ⟨line⟩  1 ⟨body⟩`.
    pub fn summary(&self) -> Vec<(IconId, usize)> {
        let mut chips = Vec::new();
        for &kind in ElementKind::ORDER.iter() {
            let count = self
                .picked
                .iter()
                .filter(|e| ElementKind::of(e) == kind)
                .count();
            if count > 0 {
                chips.push((kind.icon(), count));
            }
        }
        chips
    }
}

/// A user interaction with the picker widget in a frame, applied by the caller against the
/// owning [`ElementPicker`] (the widget borrows the picker immutably so the caller keeps
/// control of tool-specific side effects of a removal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// The user clicked the input; it should take focus (and peers should blur).
    Focus,
    /// Remove the picked element at this popup-row index.
    Remove(usize),
    /// Clear the whole set.
    Clear,
}

const ROW_ICON_SIZE: f32 = 14.0;
/// Space reserved on the right of the combo strip for the painted dropdown caret.
const CARET_RESERVE: f32 = 14.0;
/// Inset from the strip's right edge to the caret triangle centre.
const CARET_INSET: f32 = 11.0;

fn row_icon(ui: &mut egui::Ui, icon: IconId) {
    ui.add(
        egui::Image::new(crate::icons::sized_texture(ui.ctx(), icon))
            .fit_to_exact_size(egui::vec2(ROW_ICON_SIZE, ROW_ICON_SIZE)),
    );
}

/// The shared combo-box rendering (#213) behind both the [`ElementPicker`] widget and the
/// label-only [`show_labeled`] path: a focusable input strip with a `N ⟨icon⟩` collapsed
/// summary and an expandable popup of `⟨icon⟩ label ✕` rows. Fully data-driven so any tool's
/// picked set renders identically.
fn render_combo(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    focused: bool,
    single: bool,
    empty_icons: &[IconId],
    summary: &[(IconId, usize)],
    rows: &[(IconId, String)],
) -> Option<PickerEvent> {
    let mut event = None;
    let ring = if focused {
        egui::Stroke::new(2.0, crate::theme::FOCUS_ACCENT)
    } else {
        egui::Stroke::new(1.0, crate::theme::INPUT_BORDER)
    };

    let frame = egui::Frame::NONE
        .fill(crate::theme::INPUT_BG)
        .stroke(ring)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .corner_radius(egui::CornerRadius::same(3));

    // The whole framed strip is one click target that toggles the popup.
    let inner = frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_width(ui.available_width().max(120.0));
            if rows.is_empty() {
                // Generic empty state (#388): the count ("0", or "0/1" for single-select)
                // plus dimmed icons of what this picker can take.
                let empty_count = if single { "0/1" } else { "0" };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(empty_count)
                            .color(Color32::from_gray(130))
                            .strong(),
                    )
                    .selectable(false),
                );
                for &icon in empty_icons {
                    ui.add(
                        egui::Image::new(crate::icons::sized_texture(ui.ctx(), icon))
                            .fit_to_exact_size(egui::vec2(ROW_ICON_SIZE, ROW_ICON_SIZE))
                            .tint(Color32::from_gray(120)),
                    );
                }
            } else {
                for &(icon, count) in summary {
                    // A single-select picker reads "1/1" (#388); the rest just count.
                    let text = if single { format!("{count}/1") } else { count.to_string() };
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).strong())
                            .selectable(false),
                    );
                    row_icon(ui, icon);
                    ui.add_space(4.0);
                }
            }
            // Room for the painted caret so summary chips don't run under it.
            ui.add_space(CARET_RESERVE);
        });
    });

    // Dropdown caret painted on the strip — not a nested right_to_left allocate.
    // Nested RTL inside horizontal thrashing auto-ids between egui multipass frames
    // (todoer #1169 / emilk/egui#8343); the caret is decoration only.
    {
        let strip = inner.response.rect;
        let c = egui::pos2(strip.right() - CARET_INSET, strip.center().y);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(c.x - 3.0, c.y - 2.0),
                egui::pos2(c.x + 3.0, c.y - 2.0),
                egui::pos2(c.x, c.y + 2.5),
            ],
            Color32::from_gray(150),
            egui::Stroke::NONE,
        ));
    }

    // One interactable over the whole strip (click to focus + toggle popup).
    let response = ui
        .interact(inner.response.rect, ui.make_persistent_id(&id_source), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        event = Some(PickerEvent::Focus);
    }

    egui::Popup::from_toggle_button_response(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(180.0);
            if rows.is_empty() {
                ui.label(egui::RichText::new("Nothing picked yet").weak().italics());
                return;
            }
            for (i, (icon, label)) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    // A muted-red ✕ icon (#256), soft enough not to jar against the dark theme.
                    let remove = ui.add(
                        egui::Button::new(
                            egui::Image::new(crate::icons::sized_texture(
                                ui.ctx(),
                                crate::icons::IconId::Close,
                            ))
                            .tint(egui::Color32::from_rgb(0xC9, 0x6F, 0x66)),
                        )
                        .frame(false),
                    );
                    if remove.on_hover_text("Remove").clicked() {
                        event = Some(PickerEvent::Remove(i));
                    }
                    row_icon(ui, *icon);
                    ui.label(label);
                });
            }
            if rows.len() > 1 {
                ui.separator();
                if ui.small_button("Clear all").clicked() {
                    event = Some(PickerEvent::Clear);
                }
            }
        });

    event
}

/// Render an [`ElementPicker`] as a focusable, combo-box-style input in `ui`.
///
/// Collapsed, it looks like a text input: the "no selection" placeholder when empty, otherwise
/// a `N ⟨icon⟩` chip per present kind. A focused picker draws an accent ring. Clicking opens a
/// popup listing each picked element (icon + label + ✕ remove) with a Clear-all footer.
pub fn show(
    ui: &mut egui::Ui,
    picker: &ElementPicker,
    doc: &Document,
    id_source: impl std::hash::Hash + std::fmt::Debug,
) -> Option<PickerEvent> {
    let rows: Vec<(IconId, String)> = picker
        .picked()
        .iter()
        .map(|element| {
            (
                ElementKind::of(element).icon(),
                crate::names::scene_element_label(doc, element),
            )
        })
        .collect();
    render_combo(
        ui,
        id_source,
        picker.is_focused(),
        picker.limit().is_single(),
        &picker.active_filter().pickable_icons(),
        &picker.summary(),
        &rows,
    )
}

/// Render a label picker (#213/#363) whose rows carry their own icons, for non-[`SceneElement`]
/// sets with mixed item types (e.g. the drawing Select tool's projections/text/dimensions). The
/// collapsed summary counts rows per icon in first-seen order.
pub fn show_rows(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    focused: bool,
    pickable: &[IconId],
    single: bool,
    rows: &[(IconId, String)],
) -> Option<PickerEvent> {
    let mut summary: Vec<(IconId, usize)> = Vec::new();
    for (icon, _) in rows {
        if let Some(entry) = summary.iter_mut().find(|(i, _)| i == icon) {
            entry.1 += 1;
        } else {
            summary.push((*icon, 1));
        }
    }
    render_combo(ui, id_source, focused, single, pickable, &summary, rows)
}

/// Render a label-only picker (#213) with the same combo-box look as [`show`], for tool sets
/// whose items are not [`SceneElement`]s (Chamfer/Fillet edges, Loft sections, Slice cutters,
/// …). All rows share one `icon`; `labels` are the popup rows in order.
pub fn show_labeled(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    focused: bool,
    single: bool,
    icon: IconId,
    labels: &[String],
) -> Option<PickerEvent> {
    let summary = if labels.is_empty() {
        Vec::new()
    } else {
        vec![(icon, labels.len())]
    };
    let rows: Vec<(IconId, String)> = labels.iter().map(|l| (icon, l.clone())).collect();
    render_combo(ui, id_source, focused, single, &[icon], &summary, &rows)
}

/// Apply a widget [`PickerEvent`] to a picker's own state. Focus is handled by the caller (it
/// also needs to blur peer pickers), so `Focus` is a no-op here and returns `false`; `Remove`
/// and `Clear` mutate the set and return `true` so the caller can react (e.g. re-preview).
pub fn apply_event(picker: &mut ElementPicker, event: PickerEvent) -> bool {
    match event {
        PickerEvent::Focus => false,
        PickerEvent::Remove(i) => picker.remove_index(i).is_some(),
        PickerEvent::Clear => {
            let had = !picker.is_empty();
            picker.clear();
            had
        }
    }
}

#[cfg(test)]
mod tests {
    /// #1726: a primitive's face belongs to that primitive's body, so Shell's `OnBodies`
    /// rule keeps it. It used to resolve only extrusion and revolve faces, so shelling a
    /// plain cuboid showed an empty Open-faces picker with two faces actually picked.
    #[test]
    fn a_primitive_face_belongs_to_its_body() {
        use crate::model::{Body, BodySource, FaceId, Primitive, PrimitiveFace, PrimitiveKind};
        let mut doc = Document::default();
        let mut cuboid = Primitive::new(PrimitiveKind::Cuboid);
        cuboid.width = "40".to_string();
        cuboid.depth = "40".to_string();
        cuboid.height = "20".to_string();
        let prim = doc.primitives.insert(cuboid);
        let body = doc.bodies.insert(Body {
            source: BodySource::Primitive(prim),
            material: None,
            name: None,
            shadow: false,
        });
        let top = SceneElement::from_face_id(FaceId::PrimitiveFace {
            primitive: prim,
            face: PrimitiveFace::CuboidTop,
        });
        assert_eq!(element_body(&doc, &top), Some(body));

        let mut picker = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Face, ElementKind::Profile])
                .rule(PickRule::OnBodies(vec![body])),
            PickLimit::Infinite,
        );
        picker.set_picked(&doc, [top]);
        assert_eq!(picker.picked().len(), 1, "the open face must show in the picker");
    }

    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::sketch_key_for_slot as skey;
    use crate::model::constraint_key_for_slot as nkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::component_key_for_slot as ckey;
    use crate::model::body_key_for_slot as bkey;
    use crate::model::joint_key_for_slot as jkey;
    use crate::model::slice_op_key_for_slot as slckey;
    use crate::model::move_op_key_for_slot as mopkey;
    use crate::model::boolean_op_key_for_slot as bopkey;
    use super::*;

    fn body(i: usize) -> SceneElement {
        SceneElement::Body(bkey(i))
    }
    fn line(i: usize) -> SceneElement {
        SceneElement::Line(lkey(i))
    }

    #[test]
    fn kind_of_covers_operations_and_geometry() {
        assert_eq!(ElementKind::of(&SceneElement::Body(bkey(0))), ElementKind::Body);
        assert_eq!(ElementKind::of(&SceneElement::Line(lkey(0))), ElementKind::Line);
        assert_eq!(ElementKind::of(&SceneElement::Origin), ElementKind::Vertex);
        assert_eq!(
            ElementKind::of(&SceneElement::BooleanOp(bopkey(0))),
            ElementKind::Operation
        );
        assert_eq!(
            ElementKind::of(&SceneElement::ConstructionPlane(pkey(0))),
            ElementKind::Plane
        );
    }

    fn body_face(body: usize) -> SceneElement {
        SceneElement::BodyFace {
            body: bkey(body),
            centroid: [0, 0, 0],
            normal: [0, 0, 1],
        }
    }

    /// A 10x10x5 box body, for the staged-picker tests below — they need real face triangles.
    fn box_doc() -> Document {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let lines = crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
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
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        doc
    }

    /// The box's top cap, as a pickable face element.
    fn top_cap(doc: &Document) -> SceneElement {
        let solid = crate::extrude::body_solid_mesh(doc, bkey(0)).expect("box mesh");
        let tris = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid)
            .into_iter()
            .find(|t| {
                (t[0][1] - t[0][0]).cross(t[0][2] - t[0][0]).normalize_or_zero().z > 0.9
            })
            .expect("the top cap");
        let q = crate::hierarchy::quantize_body_point;
        SceneElement::BodyFace {
            body: bkey(0),
            centroid: q(crate::extrude::face_group_center(&tris)),
            normal: q((tris[0][1] - tris[0][0]).cross(tris[0][2] - tris[0][0]).normalize_or_zero()),
        }
    }

    fn point_on(face: &SceneElement, uv: [i32; 2]) -> SceneElement {
        let SceneElement::BodyFace { body, centroid, normal } = face else {
            panic!("not a face");
        };
        SceneElement::MovePoint(crate::model::MovePointRef::OnFace {
            body: *body,
            centroid: *centroid,
            normal: *normal,
            uv,
        })
    }

    /// #1075: a staged picker takes a face, and only *then* takes points — and only points on
    /// that face. The selectable set changes as the picker is used, which no flat picker does.
    #[test]
    fn a_staged_picker_only_takes_points_once_it_has_a_face() {
        let doc = box_doc();
        let cap = top_cap(&doc);
        let mut picker = ElementPicker::face_then_point(
            ElementFilter::kind(ElementKind::Face),
            ElementFilter::kind(ElementKind::Vertex),
        );
        assert!(picker.is_staged());

        // Before the face, a point is not on offer at all.
        assert!(!picker.accepts(&doc, &point_on(&cap, [0, 0])));
        assert!(picker.accepts(&doc, &cap));

        assert_eq!(picker.pick(&doc, cap.clone()), PickOutcome::Added);
        // After the face, the face itself is no longer what this picker wants — points are.
        assert!(picker.accepts(&doc, &point_on(&cap, [0, 0])));
        assert_eq!(picker.pick(&doc, point_on(&cap, [200, 200])), PickOutcome::Added);
        assert_eq!(picker.picked().len(), 2);
    }

    /// #1075: "points on that face" is geometric — a point beyond the face's outline, or on
    /// another face of the same body, is not on offer.
    #[test]
    fn a_staged_pickers_second_stage_refuses_points_off_the_face() {
        let doc = box_doc();
        let cap = top_cap(&doc);
        let mut picker = ElementPicker::face_then_point(
            ElementFilter::kind(ElementKind::Face),
            ElementFilter::kind(ElementKind::Vertex),
        );
        picker.pick(&doc, cap.clone());

        // Inside the 10x10 cap (centre-relative), fine.
        assert!(picker.accepts(&doc, &point_on(&cap, [400, 400])));
        // A metre off the side of it, not.
        assert!(!picker.accepts(&doc, &point_on(&cap, [100_000, 0])));
        // A corner of the *bottom* cap is a real point on the body, but not on this face.
        let bottom = SceneElement::MovePoint(crate::model::MovePointRef::Vertex {
            body: bkey(0),
            p: crate::hierarchy::quantize_body_point(glam::Vec3::ZERO),
        });
        assert!(!picker.accepts(&doc, &bottom));
    }

    /// #1075: the whole reason the sequence lives inside the picker — `expand_pick`, which is
    /// what the Selection Exploder fans a crowd out through, follows the stage without knowing
    /// anything about it. A face click that would have meant "the whole body" in stage 0 means
    /// nothing in stage 1, because the picker no longer takes bodies.
    #[test]
    fn a_staged_picker_changes_what_a_crowd_expands_into() {
        let doc = box_doc();
        let cap = top_cap(&doc);
        let mut picker = ElementPicker::face_then_point(
            ElementFilter::kind(ElementKind::Face),
            ElementFilter::kind(ElementKind::Vertex),
        );
        // Stage 0 takes the face itself, and a point on it is not on offer.
        assert_eq!(expand_pick(&doc, &picker, &cap, false), vec![cap.clone()]);
        let p = point_on(&cap, [100, 100]);
        assert!(expand_pick(&doc, &picker, &p, false).is_empty());

        picker.pick(&doc, cap.clone());
        // Stage 1 has swapped them over: the face is spent, the point is what it wants.
        assert!(expand_pick(&doc, &picker, &cap, false).is_empty());
        assert_eq!(expand_pick(&doc, &picker, &p, false), vec![p]);
    }

    /// #1075: the second pick is only ever "a point on *that* face", so dropping the face
    /// drops the point with it — and picking a different face starts the sequence over.
    #[test]
    fn dropping_a_staged_pickers_face_drops_its_point() {
        let doc = box_doc();
        let cap = top_cap(&doc);
        let mut picker = ElementPicker::face_then_point(
            ElementFilter::kind(ElementKind::Face),
            ElementFilter::kind(ElementKind::Vertex),
        );
        picker.pick(&doc, cap.clone());
        picker.pick(&doc, point_on(&cap, [200, 200]));

        // A second face replaces the pair rather than being refused as "full".
        let side = {
            let solid = crate::extrude::body_solid_mesh(&doc, bkey(0)).expect("box mesh");
            let tris = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid)
                .into_iter()
                .find(|t| {
                    (t[0][1] - t[0][0]).cross(t[0][2] - t[0][0]).normalize_or_zero().z.abs() < 0.1
                })
                .expect("a side wall");
            let q = crate::hierarchy::quantize_body_point;
            SceneElement::BodyFace {
                body: bkey(0),
                centroid: q(crate::extrude::face_group_center(&tris)),
                normal: q((tris[0][1] - tris[0][0]).cross(tris[0][2] - tris[0][0]).normalize_or_zero()),
            }
        };
        assert_eq!(picker.pick(&doc, side.clone()), PickOutcome::Replaced);
        assert_eq!(picker.picked(), std::slice::from_ref(&side));

        // And removing the face row takes the point row with it.
        picker.pick(&doc, point_on(&side, [0, 0]));
        assert_eq!(picker.picked().len(), 2);
        picker.remove_index(0);
        assert!(picker.picked().is_empty(), "{:?}", picker.picked());
    }

    #[test]
    fn body_face_is_its_own_face_kind() {
        // #566: a flat body face is `Face`, not `Body`, so a planes-or-faces picker can take it
        // without also swallowing whole bodies.
        assert_eq!(ElementKind::of(&body_face(3)), ElementKind::Face);
        assert_eq!(ElementKind::of(&SceneElement::Body(bkey(3))), ElementKind::Body);
    }

    #[test]
    fn plane_or_face_filter_takes_planes_and_faces_not_bodies() {
        // The Mirror tool's plane picker (#566): construction planes and flat faces, never a
        // whole body.
        let f = ElementFilter::kinds(&[ElementKind::Plane, ElementKind::Face]);
        assert!(f.accepts(&Document::default(), &SceneElement::ConstructionPlane(pkey(0))));
        assert!(f.accepts(&Document::default(), &body_face(0)));
        assert!(!f.accepts(&Document::default(), &SceneElement::Body(bkey(0))));
    }

    #[test]
    fn everything_filter_accepts_all() {
        let f = ElementFilter::everything();
        assert!(f.accepts(&Document::default(), &body(0)));
        assert!(f.accepts(&Document::default(), &SceneElement::Origin));
        assert!(f.accepts(&Document::default(), &SceneElement::MoveOp(mopkey(3))));
        assert!(f.accepts_kind(ElementKind::Constraint));
    }

    #[test]
    fn kind_filter_rejects_other_kinds() {
        let f = ElementFilter::kind(ElementKind::Body);
        assert!(f.accepts(&Document::default(), &body(0)));
        assert!(!f.accepts(&Document::default(), &line(0)));
        assert!(!f.accepts_kind(ElementKind::Line));
    }

    #[test]
    fn plane_filter_also_accepts_images() {
        let f = ElementFilter::kind(ElementKind::Plane);
        assert!(f.accepts(&Document::default(), &SceneElement::ConstructionPlane(pkey(0))));
        let image = SceneElement::Image(crate::arena::Key::from_bits(0));
        assert!(f.accepts(&Document::default(), &image));
    }

    #[test]
    fn an_images_only_filter_takes_images() {
        // Image was missing from `ORDER`, so `kinds()` dropped it and an images-only picker
        // accepted nothing at all.
        let f = ElementFilter::kind(ElementKind::Image);
        let image = SceneElement::Image(crate::arena::Key::from_bits(0));
        assert!(f.accepts(&Document::default(), &image));
        assert!(!f.accepts(&Document::default(), &SceneElement::ConstructionPlane(pkey(0))));
    }

    #[test]
    fn a_picked_image_shows_in_the_summary() {
        // Same root cause: `summary()` walks `ORDER`, so a picked image counted as nothing.
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        p.pick(&Document::default(), SceneElement::Image(crate::arena::Key::from_bits(0)));
        assert_eq!(p.summary().len(), 1, "one chip for the picked image");
        assert_eq!(p.summary()[0].1, 1);
    }

    #[test]
    fn global_axes_are_their_own_kind() {
        // The world axes are pickable (a Repeat path, a Revolve axis) but had no scene element,
        // so they could never live in a picker.
        use crate::construction::GlobalAxis;
        assert_eq!(
            ElementKind::of(&SceneElement::GlobalAxis(GlobalAxis::Z)),
            ElementKind::Axis
        );
        let f = ElementFilter::kinds(&[ElementKind::Axis, ElementKind::Line]);
        assert!(f.accepts(&Document::default(), &SceneElement::GlobalAxis(GlobalAxis::X)));
        assert!(f.accepts(&Document::default(), &line(0)));
        assert!(!f.accepts(&Document::default(), &body(0)));
        // An axis-only picker refuses sketch lines.
        let axes = ElementFilter::kind(ElementKind::Axis);
        assert!(axes.accepts(&Document::default(), &SceneElement::GlobalAxis(GlobalAxis::Y)));
        assert!(!axes.accepts(&Document::default(), &line(0)));
    }

    #[test]
    fn joints_and_components_are_not_lumped_in_with_operations() {
        // The design lists joints and components as target types of their own; both used to
        // report as `Operation`, so an operations picker swallowed them.
        assert_eq!(ElementKind::of(&SceneElement::Joint(jkey(0))), ElementKind::Joint);
        assert_eq!(
            ElementKind::of(&SceneElement::Component(ckey(0))),
            ElementKind::Component
        );
        let ops = ElementFilter::kind(ElementKind::Operation);
        assert!(ops.accepts(&Document::default(), &SceneElement::BooleanOp(bopkey(0))));
        assert!(!ops.accepts(&Document::default(), &SceneElement::Joint(jkey(0))));
        assert!(!ops.accepts(&Document::default(), &SceneElement::Component(ckey(0))));
    }

    #[test]
    fn an_analytic_face_is_a_profile_a_picker_can_hold() {
        // #952: Extrude profiles, Revolve/Sweep profiles and Slice cutters all carry a `FaceId`
        // — the *analytic* face, a different identity from the quantized mesh `BodyFace` — and
        // had no scene element, so those inputs kept bespoke `Vec<FaceId>` state.
        let profile = SceneElement::from_face_id(crate::model::FaceId::Circle(rkey(3)));
        assert_eq!(
            profile,
            SceneElement::SketchFace(crate::model::FaceId::Circle(rkey(3)))
        );
        // #957: and it is a *different kind* from the mesh face over the same surface, so a
        // picker can say which of the two representations it wants — which is what stops the
        // Exploder fanning one face twice.
        assert_eq!(ElementKind::of(&profile), ElementKind::Profile);
        let profiles = ElementFilter::kind(ElementKind::Profile);
        assert!(profiles.accepts(&Document::default(), &profile));
        assert!(!profiles.accepts(&Document::default(), &body(0)));
        let mesh = SceneElement::BodyFace {
            body: bkey(0),
            centroid: [0, 0, 0],
            normal: [0, 0, 1],
        };
        assert!(!profiles.accepts(&Document::default(), &mesh));
        assert!(!ElementFilter::kind(ElementKind::Face).accepts(&Document::default(), &profile));
    }

    #[test]
    fn a_face_id_naming_a_construction_plane_is_that_plane() {
        // One identity per thing: a `FaceId::ConstructionPlane` and the plane's own element are
        // the same plane, so a picker holding both would double-count it.
        assert_eq!(
            SceneElement::from_face_id(crate::model::FaceId::ConstructionPlane(pkey(2))),
            SceneElement::ConstructionPlane(pkey(2))
        );
        assert_eq!(
            ElementKind::of(&SceneElement::from_face_id(
                crate::model::FaceId::ConstructionPlane(pkey(2))
            )),
            ElementKind::Plane
        );
    }

    #[test]
    fn a_move_point_is_a_vertex_a_picker_can_hold() {
        // #952: the Move and Joint tools each have six point pickers, all label-only because a
        // `MovePointRef` — an edge midpoint, a face middle — had no scene element.
        use crate::model::MovePointRef;
        let midpoint = SceneElement::from_move_point(MovePointRef::EdgeMidpoint {
            body: bkey(1),
            a: [0; 3],
            b: [100, 0, 0],
        });
        assert_eq!(ElementKind::of(&midpoint), ElementKind::Vertex);
        assert!(ElementFilter::kind(ElementKind::Vertex).accepts(&Document::default(), &midpoint));
        assert!(!ElementFilter::kind(ElementKind::Edge).accepts(&Document::default(), &midpoint));
    }

    #[test]
    fn move_points_that_name_something_else_normalize_to_it() {
        // One identity per thing, as for faces: a move point on a body corner is that corner,
        // and the origin move point is the origin.
        use crate::model::MovePointRef;
        assert_eq!(
            SceneElement::from_move_point(MovePointRef::Vertex { body: bkey(2), p: [1, 2, 3] }),
            SceneElement::BodyVertex { body: bkey(2), p: [1, 2, 3] }
        );
        assert_eq!(
            SceneElement::from_move_point(MovePointRef::Origin),
            SceneElement::Origin
        );
        // Round-trips: what a picker holds converts back to what the geometry code wants.
        for point in [
            MovePointRef::Vertex { body: bkey(2), p: [1, 2, 3] },
            MovePointRef::Origin,
            MovePointRef::EdgeMidpoint { body: bkey(0), a: [0; 3], b: [5; 3] },
            MovePointRef::OnEdge { body: bkey(0), p: [7; 3] },
            MovePointRef::OnFace {
                body: bkey(4),
                centroid: [1; 3],
                normal: [0, 0, 100],
                uv: [0, 0],
            },
        ] {
            assert_eq!(
                SceneElement::from_move_point(point).as_move_point(),
                Some(point),
                "{point:?} should survive the round trip"
            );
        }
    }

    #[test]
    fn an_extrusion_edge_is_an_edge_a_picker_can_hold() {
        // #952: the 3D Chamfer/Fillet set is `Vec<(crate::model::ExtrusionKey, ExtrusionEdgeRef)>` — the analytic
        // edge, not the quantized mesh `BodyEdge` — so it had no element and kept its own state
        // behind the legacy row-list picker.
        let edge = SceneElement::ExtrusionEdge {
            extrusion: xkey(2),
            edge: crate::model::ExtrusionEdgeRef::Vertical { face: 0, edge: 1 },
        };
        assert_eq!(ElementKind::of(&edge), ElementKind::Edge);
        assert!(ElementFilter::kind(ElementKind::Edge).accepts(&Document::default(), &edge));
        assert!(!ElementFilter::kind(ElementKind::Body).accepts(&Document::default(), &edge));
    }

    #[test]
    fn a_loft_section_needs_no_element_of_its_own() {
        // A loft section is a closed profile plus its sketch, and the profile is a `FaceId` —
        // so the analytic face element already names it; the sketch follows from the face.
        let section = crate::model::ExtrudeFace::Circle(rkey(4));
        let element = SceneElement::from_face_id(crate::model::FaceId::Circle(rkey(4)));
        assert_eq!(
            crate::extrude::extrude_face_scene_element(&section),
            element
        );
        assert_eq!(ElementKind::of(&element), ElementKind::Profile);
    }

    /// A document with two solid bodies, a straight line and a curved one, so the rules have
    /// something real to judge.
    fn doc_with_two_bodies() -> Document {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0, [false; 4]);
        // A curved line, so `Straight` has something to reject.
        let curved = doc.lines.insert(crate::model::Line {
            bezier: Some([(1.0, 1.0), (2.0, 2.0)]),
            ..doc.lines[lkey(0)].clone()
        });
        assert!(doc.lines[curved].bezier.is_some());
        for _ in 0..2 {
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
                material: None,
                name: None,
                shadow: false,
            });
        }
        doc
    }

    #[test]
    fn a_live_body_rule_rejects_deleted_and_consumed_bodies() {
        // The `!deleted && !shadow` gate that `toggle_body_in_active_tool` and every `SetTool`
        // seeding block re-checks by hand.
        let mut doc = doc_with_two_bodies();
        doc.bodies.values_mut().nth(1).unwrap().shadow = true;
        let f = ElementFilter::kind(ElementKind::Body).rule(PickRule::LiveBody);
        assert!(f.accepts(&doc, &body(0)));
        assert!(!f.accepts(&doc, &body(1)), "a consumed body is not pickable");
        assert!(!f.accepts(&doc, &body(9)), "a body that isn't there is not pickable");
    }

    #[test]
    fn on_and_off_bodies_split_the_moving_from_the_stationary() {
        // The Move tool's rule: start points land on a *moving* body, end points on a
        // stationary one (#649/#650).
        let doc = doc_with_two_bodies();
        let moving = vec![bkey(0)];
        let on_moving = ElementFilter::kind(ElementKind::Vertex)
            .rule(PickRule::OnBodies(moving.clone()));
        let off_moving =
            ElementFilter::kind(ElementKind::Vertex).rule(PickRule::OffBodies(moving));
        let corner_of_0 = SceneElement::BodyVertex { body: bkey(0), p: [0; 3] };
        let corner_of_1 = SceneElement::BodyVertex { body: bkey(1), p: [0; 3] };
        assert!(on_moving.accepts(&doc, &corner_of_0));
        assert!(!on_moving.accepts(&doc, &corner_of_1));
        assert!(!off_moving.accepts(&doc, &corner_of_0));
        assert!(off_moving.accepts(&doc, &corner_of_1));
        // The origin belongs to no body, so it is stationary but never "on" a moving one.
        assert!(off_moving.accepts(&doc, &SceneElement::Origin));
        assert!(!on_moving.accepts(&doc, &SceneElement::Origin));
    }

    /// #1601: the 2D point-snap rule answers an image box point by which image is moving,
    /// alongside the body corpus `OnBodies`/`OffBodies` already split.
    #[test]
    fn move_point_2d_admits_an_image_box_point_by_its_moving_side() {
        let mut doc = doc_with_two_bodies();
        let image = doc.tracing_images.insert(crate::model::TracingImage {
            bytes: Vec::new(),
            source_name: "trace".to_string(),
            plane: crate::model::plane_key_for_slot(0),
            origin: (0.0, 0.0),
            width_mm: 100.0,
            height_mm: 60.0,
            opacity: crate::model::DEFAULT_TRACING_IMAGE_OPACITY,
            name: None,
            calibration: None,
            base_origin: None,
            rotation: 0.0,
            base_rotation: None,
        });
        let anchor = SceneElement::Point(crate::model::ConstraintPoint::ImageAnchor {
            image,
            anchor: crate::model::TextAnchor::BottomLeft,
        });
        let start = ElementFilter::kind(ElementKind::Vertex)
            .rule(PickRule::MovePoint2D(true, vec![bkey(0)], vec![image]));
        let fixed = ElementFilter::kind(ElementKind::Vertex)
            .rule(PickRule::MovePoint2D(false, vec![bkey(0)], vec![image]));
        // A moving image answers the start pick; that same image is off-limits as a fixed point.
        assert!(start.accepts(&doc, &anchor));
        assert!(!fixed.accepts(&doc, &anchor));
        // The moving body still gates the body corner, and the origin stays stationary-only.
        let corner = SceneElement::BodyVertex { body: bkey(0), p: [0; 3] };
        assert!(start.accepts(&doc, &corner));
        assert!(!fixed.accepts(&doc, &corner));
        assert!(!start.accepts(&doc, &SceneElement::Origin));
        assert!(fixed.accepts(&doc, &SceneElement::Origin));
    }

    /// #1721 fallout: a sketch's origin axes draw through solids, so the occlusion gate no
    /// longer buries them — which put one at the top of the pick list wherever it crosses a
    /// body. `as_revolve_axis` has no mapping for it, so Repeat's Path picker must refuse it
    /// the way Revolve's Axis picker already does, or the click lands on nothing at all. Its
    /// circle path (#840) still has to get through.
    #[test]
    fn the_repeat_path_rule_refuses_a_sketch_origin_axis_but_keeps_a_circle() {
        let mut doc = doc_with_two_bodies();
        let circle = doc.circles.insert(crate::model::Circle::from_local_center_radius(
            doc.sketches.keys().next().expect("the helper made a sketch"),
            0.0,
            0.0,
            5.0,
            0.0,
        ));
        let f = crate::context::picker_filter(crate::context::PickerTarget::RepeatPath);
        assert!(
            !f.accepts(
                &doc,
                &SceneElement::FaceEdge(crate::model::ConstraintLine::OriginAxis(
                    crate::model::SketchAxis::X
                ))
            ),
            "a sketch origin axis is not a path Repeat can follow"
        );
        assert!(f.accepts(&doc, &line(0)), "a straight sketch line is");
        assert!(
            f.accepts(&doc, &SceneElement::Circle(circle)),
            "and so is a circle — the copies ride round it"
        );
        assert!(f.accepts(
            &doc,
            &SceneElement::BodyEdge { body: bkey(0), a: [0; 3], b: [1; 3] }
        ));
    }

    #[test]
    fn a_straight_rule_rejects_a_curved_line() {
        // The Revolve axis / Repeat path rule: a straight reference only.
        let doc = doc_with_two_bodies();
        let f = ElementFilter::kinds(&[ElementKind::Line, ElementKind::Axis, ElementKind::Edge])
            .rule(PickRule::Straight);
        assert!(f.accepts(&doc, &line(0)), "a plain sketch line is straight");
        let curved = doc.lines.len() - 1;
        assert!(!f.accepts(&doc, &line(curved)), "a bezier line is not");
        assert!(f.accepts(
            &doc,
            &SceneElement::GlobalAxis(crate::construction::GlobalAxis::Z)
        ));
        assert!(f.accepts(
            &doc,
            &SceneElement::BodyEdge { body: bkey(0), a: [0; 3], b: [1; 3] }
        ));
    }

    /// #983: the Projection tool's rule — outside sources (bodies, their edges/corners,
    /// planes that cross the sketch) plus this sketch's already-projected lines (picked to
    /// un-project them), never the sketch's own drawn geometry.
    #[test]
    fn projectable_into_takes_outside_sources_and_projected_lines_only() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut projected = crate::model::Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0);
        projected.projection = Some(crate::model::ProjectionSource::Plane {
            plane: doc.ground_plane().unwrap(),
        });
        doc.lines.insert(projected);

        let rule = PickRule::ProjectableInto(sketch);
        assert!(!rule.allows(&doc, &line(0)), "the sketch's own drawn line is refused");
        assert!(rule.allows(&doc, &line(1)), "a projected line is taken, to un-project it");
        assert!(rule.allows(&doc, &body(0)));
        assert!(rule.allows(
            &doc,
            &SceneElement::BodyEdge { body: bkey(0), a: [0; 3], b: [1; 3] }
        ));
        assert!(
            rule.allows(&doc, &SceneElement::ConstructionPlane(pkey(2))),
            "YZ crosses the ground sketch"
        );
        assert!(
            !rule.allows(&doc, &SceneElement::ConstructionPlane(pkey(0))),
            "the sketch's own plane is parallel — no line to project"
        );
        assert!(!rule.allows(&doc, &SceneElement::Origin));
    }

    #[test]
    fn a_not_in_rule_keeps_a_sibling_pickers_items_out() {
        // Combine's B side must not take what side A already holds.
        let doc = doc_with_two_bodies();
        let f = ElementFilter::kind(ElementKind::Body).rule(PickRule::NotIn(vec![body(0)]));
        assert!(!f.accepts(&doc, &body(0)));
        assert!(f.accepts(&doc, &body(1)));
    }

    #[test]
    fn rules_all_have_to_pass() {
        let mut doc = doc_with_two_bodies();
        doc.bodies.values_mut().nth(1).unwrap().shadow = true;
        let f = ElementFilter::kind(ElementKind::Body)
            .rule(PickRule::LiveBody)
            .rule(PickRule::NotIn(vec![body(0)]));
        // 0 is live but excluded; 1 is not excluded but consumed. Neither passes both.
        assert!(!f.accepts(&doc, &body(0)));
        assert!(!f.accepts(&doc, &body(1)));
    }

    #[test]
    fn a_rule_gates_picking_not_just_the_filter() {
        // The point of the rules: `pick` and `set_picked` honour them, so every path — viewport
        // click, pane click, tool handoff — gets the same answer.
        let mut doc = doc_with_two_bodies();
        doc.bodies.values_mut().nth(1).unwrap().shadow = true;
        let mut p = ElementPicker::new(
            ElementFilter::kind(ElementKind::Body).rule(PickRule::LiveBody),
            PickLimit::Infinite,
        );
        assert_eq!(p.pick(&doc, body(1)), PickOutcome::NotAccepted);
        assert_eq!(p.pick(&doc, body(0)), PickOutcome::Added);
        p.set_picked(&doc, [body(0), body(1)]);
        assert_eq!(p.picked(), &[body(0)], "set_picked drops what a rule rejects");
    }

    #[test]
    fn the_default_priority_puts_the_sharpest_thing_first() {
        // A corner beats an edge through it, which beats the face they lie on, which beats the
        // construction plane behind it, which beats a whole body. This is the ordering the pick
        // resolver hard-coded as a `u8` per candidate (#959).
        let band = default_pick_band;
        assert!(
            band(ElementKind::Vertex) < band(ElementKind::Edge),
            "a corner beats an edge through it"
        );
        assert!(
            band(ElementKind::Edge) < band(ElementKind::Face),
            "an edge beats the face it lies on"
        );
        assert!(
            band(ElementKind::Face) < band(ElementKind::Plane),
            "a face beats a construction plane behind it"
        );
        assert!(
            band(ElementKind::Plane) < band(ElementKind::Body),
            "a plane beats a whole body"
        );
        // The linear kinds share the edge band, so which of them wins is decided by pixel
        // distance rather than by kind.
        for kind in [ElementKind::Line, ElementKind::Circle, ElementKind::Axis] {
            assert_eq!(
                band(kind),
                band(ElementKind::Edge),
                "{kind:?} shares the edge band"
            );
        }
    }

    #[test]
    fn a_picker_can_override_the_priority() {
        // The design's example: a picker that wants faces over edges (#959).
        let default = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        assert!(
            default.rank(ElementKind::Vertex) < default.rank(ElementKind::Face),
            "the default prefers the corner"
        );
        let faces_first = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite)
            .with_priority(&[ElementKind::Face]);
        assert!(
            faces_first.rank(ElementKind::Face) < faces_first.rank(ElementKind::Vertex),
            "an override wins over the default"
        );
        // Kinds the override doesn't mention rank behind every kind it does, keeping the
        // default's relative order among themselves.
        assert!(
            faces_first.rank(ElementKind::Vertex) < faces_first.rank(ElementKind::Body),
            "unlisted kinds keep the default order behind the listed ones"
        );
    }

    #[test]
    fn a_face_click_expands_to_its_edges_when_the_picker_wants_edges() {
        // #960: an edges-only picker refuses a face outright, so clicking one was a dead click.
        // Now it means "all of that face's edges".
        let doc = doc_with_two_bodies();
        let edges_only =
            ElementPicker::new(ElementFilter::kind(ElementKind::Edge), PickLimit::Infinite);
        // A sketch profile's boundary is its lines; an edges picker takes none of them, so it
        // gets nothing rather than something wrong.
        let profile = SceneElement::from_face_id(crate::model::FaceId::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]));
        assert!(expand_pick(&doc, &edges_only, &profile, false).is_empty());

        // A lines picker, though, gets exactly the profile's lines.
        let lines_only =
            ElementPicker::new(ElementFilter::kind(ElementKind::Line), PickLimit::Infinite);
        assert_eq!(
            expand_pick(&doc, &lines_only, &profile, false),
            vec![
                SceneElement::Line(lkey(0)),
                SceneElement::Line(lkey(1)),
                SceneElement::Line(lkey(2)),
                SceneElement::Line(lkey(3))
            ]
        );
        // A circle profile is its circle.
        let circles = ElementPicker::new(
            ElementFilter::kinds(&[ElementKind::Circle]),
            PickLimit::Infinite,
        );
        let circle_face = SceneElement::from_face_id(crate::model::FaceId::Circle(rkey(2)));
        assert_eq!(
            expand_pick(&doc, &circles, &circle_face, false),
            vec![SceneElement::Circle(rkey(2))]
        );
    }

    /// A sketch holding the reported shape (#984): a straight line that breaks into a tangent
    /// curve and exits again as a tangent line, plus a fourth line meeting the run at a right
    /// angle. Lines 0-1-2 are the run; line 3 is the corner that must break it.
    ///
    /// ```text
    ///   (0,0) --0-- (10,0) ~~1~~ (20,10) --2-- (30,10)
    ///                                             |
    ///                                             3
    ///                                          (30,20)
    /// ```
    fn doc_with_a_tangent_run() -> Document {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let mut line = |x0: f32, y0: f32, x1: f32, y1: f32, bezier| {
            doc.lines.insert(crate::model::Line {
                sketch,
                x0,
                y0,
                x1,
                y1,
                bezier,
                ..crate::model::Line::from_local_endpoints(sketch, x0, y0, x1, y1)
            });
        };
        line(0.0, 0.0, 10.0, 0.0, None);
        // Handles continue each neighbour's direction: horizontal out of (10,0), and along
        // the +x of line 2 back into (20,10).
        line(10.0, 0.0, 20.0, 10.0, Some([(16.0, 0.0), (14.0, 10.0)]));
        line(20.0, 10.0, 30.0, 10.0, None);
        // The corner: straight up from the run's far end, a 90° turn.
        line(30.0, 10.0, 30.0, 20.0, None);
        doc
    }

    #[test]
    fn a_tangent_run_of_lines_chains_and_stops_at_the_corner() {
        // #984: hovering/clicking any segment of a line-curve-line run takes the whole run.
        let doc = doc_with_a_tangent_run();
        for start in [0usize, 1, 2] {
            assert_eq!(
                sketch_line_tangent_chain(&doc, lkey(start)),
                vec![lkey(0), lkey(1), lkey(2)],
                "line {start} should reach the whole run in both directions"
            );
        }
        // The 90° corner is a boundary: line 3 is its own run.
        assert_eq!(sketch_line_tangent_chain(&doc, lkey(3)), vec![lkey(3)]);
    }

    #[test]
    fn a_junction_of_three_lines_breaks_the_chain() {
        // Three ends at one vertex is a junction, not a smooth continuation — even when two of
        // them are perfectly tangent, since there's no telling which one continues the curve.
        let mut doc = doc_with_a_tangent_run();
        let sketch = doc.lines[lkey(0)].sketch;
        doc.lines
            .insert(crate::model::Line::from_local_endpoints(sketch, 10.0, 0.0, 10.0, -10.0));
        assert_eq!(sketch_line_tangent_chain(&doc, lkey(0)), vec![lkey(0)]);
        assert_eq!(sketch_line_tangent_chain(&doc, lkey(1)), vec![lkey(1), lkey(2)]);
    }

    #[test]
    fn a_deleted_line_is_not_part_of_a_run() {
        let mut doc = doc_with_a_tangent_run();
        doc.lines.remove(lkey(1));
        assert_eq!(sketch_line_tangent_chain(&doc, lkey(0)), vec![lkey(0)]);
        assert_eq!(sketch_line_tangent_chain(&doc, lkey(2)), vec![lkey(2)]);
    }

    #[test]
    fn a_lines_picker_takes_the_whole_run_unless_chaining_is_off() {
        // #984: the default is the run; Control (chain = false) is the single line.
        let doc = doc_with_a_tangent_run();
        let lines =
            ElementPicker::new(ElementFilter::kind(ElementKind::Line), PickLimit::Infinite);
        assert_eq!(
            expand_pick(&doc, &lines, &line(1), true),
            vec![line(0), line(1), line(2)]
        );
        assert_eq!(expand_pick(&doc, &lines, &line(1), false), vec![line(1)]);
    }

    #[test]
    fn a_single_slot_picker_never_takes_a_run() {
        // A one-slot input has nowhere to put a run — the same reason it never takes a face's
        // edges (#955). It gets the line under the cursor.
        let doc = doc_with_a_tangent_run();
        let one = ElementPicker::new(ElementFilter::kind(ElementKind::Line), PickLimit::Finite(1));
        assert_eq!(expand_pick(&doc, &one, &line(1), true), vec![line(1)]);
    }

    #[test]
    fn a_picker_that_refuses_part_of_a_run_takes_only_what_it_accepts() {
        // A rule-restricted picker still runs the chain, then keeps the members it can hold:
        // a `Straight` picker takes the run's two straight lines and drops its curve.
        let doc = doc_with_a_tangent_run();
        let straight = ElementPicker::new(
            ElementFilter::kind(ElementKind::Line).rule(PickRule::Straight),
            PickLimit::Infinite,
        );
        assert_eq!(
            expand_pick(&doc, &straight, &line(0), true),
            vec![line(0), line(2)]
        );
    }

    #[test]
    fn a_picker_that_takes_faces_gets_the_face_itself() {
        // No expansion when the picker can hold what was clicked.
        let doc = doc_with_two_bodies();
        let faces =
            ElementPicker::new(ElementFilter::kind(ElementKind::Face), PickLimit::Infinite);
        let face = body_face(0);
        assert_eq!(expand_pick(&doc, &faces, &face, false), vec![face.clone()]);
        // And nothing at all when the pick is simply wrong for the picker.
        let bodies =
            ElementPicker::new(ElementFilter::kind(ElementKind::Body), PickLimit::Infinite);
        assert!(expand_pick(&doc, &bodies, &line(0), false).is_empty());
    }

    #[test]
    fn every_kind_is_in_the_canonical_order() {
        // `ORDER` drives both `kinds()` membership and `summary()`, so a kind missing from it
        // is a kind no picker can accept and no summary can count.
        for element in [
            SceneElement::ConstructionPlane(pkey(0)),
            SceneElement::Image(crate::arena::Key::from_bits(0)),
            SceneElement::Sketch(skey(0)),
            SceneElement::Line(lkey(0)),
            SceneElement::Circle(rkey(0)),
            SceneElement::Origin,
            SceneElement::BodyEdge { body: bkey(0), a: [0; 3], b: [1; 3] },
            body_face(0),
            SceneElement::Constraint(nkey(0)),
            SceneElement::Body(bkey(0)),
            SceneElement::GlobalAxis(crate::construction::GlobalAxis::X),
            SceneElement::Joint(jkey(0)),
            SceneElement::Component(ckey(0)),
            SceneElement::BooleanOp(bopkey(0)),
        ] {
            let kind = ElementKind::of(&element);
            assert!(
                ElementKind::ORDER.contains(&kind),
                "{kind:?} (from {element:?}) is missing from ElementKind::ORDER"
            );
            assert!(
                ElementFilter::kind(kind).accepts(&Document::default(), &element),
                "a {kind:?}-only picker should accept {element:?}"
            );
        }
    }

    /// #1487: `OperationKind` must cover every scene element `ElementKind::of` calls
    /// an Operation, so a picker restricted with `.operations([...])` can name any of
    /// them (including "any operation except X").
    #[test]
    fn every_element_kind_operation_has_an_operation_kind() {
        use crate::model::{
            edge_treatment_op_key_for_slot as etkey, primitive_key_for_slot as primkey,
            sketch_op_key_for_slot as skop, sketch_text_key_for_slot as tkey,
        };
        let ops = [
            SceneElement::Extrusion(xkey(0)),
            SceneElement::BooleanOp(bopkey(0)),
            SceneElement::MoveOp(mopkey(0)),
            SceneElement::MirrorOp(crate::model::mirror_op_key_for_slot(0)),
            SceneElement::RepeatOp(crate::model::repeat_op_key_for_slot(0)),
            SceneElement::SketchRepeatOp(skop(0)),
            SceneElement::SketchOffsetOp(skop(0)),
            SceneElement::SketchMirrorOp(skop(0)),
            SceneElement::SketchVertexTreatmentOp(skop(0)),
            SceneElement::SketchSliceOp(skop(0)),
            SceneElement::SketchText(tkey(0)),
            SceneElement::SliceOp(slckey(0)),
            SceneElement::ShellOp(crate::model::shell_op_key_for_slot(0)),
            SceneElement::EdgeTreatmentOp(etkey(0)),
            SceneElement::Revolution(crate::arena::Key::from_bits(0)),
            SceneElement::Shape(primkey(0)),
            SceneElement::SweepOp(crate::arena::Key::from_bits(0)),
            SceneElement::Loft(crate::arena::Key::from_bits(0)),
        ];
        for element in ops {
            assert_eq!(
                ElementKind::of(&element),
                ElementKind::Operation,
                "{element:?} must stay an Operation so this walk stays the full set"
            );
            assert!(
                OperationKind::of(&element).is_some(),
                "{element:?} is an Operation but OperationKind::of is None"
            );
        }
    }

    #[test]
    fn operation_restriction_filters_by_sub_kind() {
        let f = ElementFilter::kinds(&[ElementKind::Body])
            .operations(&[OperationKind::Boolean, OperationKind::Slice]);
        assert!(f.accepts(&Document::default(), &SceneElement::BooleanOp(bopkey(0))));
        assert!(f.accepts(&Document::default(), &SceneElement::SliceOp(slckey(0))));
        assert!(!f.accepts(&Document::default(), &SceneElement::MoveOp(mopkey(0))));
        // Body still accepted alongside the operations.
        assert!(f.accepts(&Document::default(), &body(0)));
    }

    #[test]
    fn pick_toggles_and_respects_kind() {
        let mut p = ElementPicker::new(ElementFilter::kind(ElementKind::Body), PickLimit::Infinite);
        assert_eq!(p.pick(&Document::default(), body(0)), PickOutcome::Added);
        assert_eq!(p.pick(&Document::default(), line(0)), PickOutcome::NotAccepted);
        assert_eq!(p.pick(&Document::default(), body(1)), PickOutcome::Added);
        assert_eq!(p.len(), 2);
        assert_eq!(p.pick(&Document::default(), body(0)), PickOutcome::Removed);
        assert_eq!(p.len(), 1);
        assert!(p.contains(&body(1)));
    }

    #[test]
    fn finite_limit_blocks_when_full() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Finite(2));
        assert_eq!(p.pick(&Document::default(), body(0)), PickOutcome::Added);
        assert_eq!(p.pick(&Document::default(), body(1)), PickOutcome::Added);
        assert!(p.is_full());
        assert_eq!(p.pick(&Document::default(), body(2)), PickOutcome::Full);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn single_select_replaces() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Finite(1));
        assert_eq!(p.pick(&Document::default(), body(0)), PickOutcome::Added);
        assert_eq!(p.pick(&Document::default(), body(1)), PickOutcome::Replaced);
        assert_eq!(p.picked(), &[body(1)]);
    }

    #[test]
    fn the_selection_picker_takes_everything_and_starts_focused() {
        // #966: it used to carry a `sticky_focus` flag that made `set_focused(false)` a no-op.
        // That was the focus model leaking into the control — the selection picker is the
        // Select tool's only picker, so it is focused whenever no other one is, which is what
        // "exactly one picker has focus" already says.
        let p = ElementPicker::select_everything();
        assert!(p.is_focused());
        assert!(p.accepts(&Document::default(), &SceneElement::Sketch(skey(0))));
        assert!(p.accepts(&Document::default(), &SceneElement::Body(bkey(0))));
    }

    #[test]
    fn summary_groups_by_kind_in_canonical_order() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        p.pick(&Document::default(), body(0));
        p.pick(&Document::default(), line(0));
        p.pick(&Document::default(), line(1));
        // Canonical order puts lines before bodies.
        let summary = p.summary();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].1, 2, "two lines first");
        assert_eq!(summary[1].1, 1, "one body second");
    }

    #[test]
    fn sketch_mirror_line_filter_takes_axes() {
        use crate::construction::GlobalAxis;
        use crate::model::{ConstraintLine, FaceId, Line, SketchAxis};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let filter = crate::context::picker_filter(crate::context::PickerTarget::SketchMirrorLine)
            .rule(PickRule::InSketch(sketch));
        assert!(filter.accepts(&doc, &SceneElement::Line(lkey(0))));
        assert!(filter.accepts(
            &doc,
            &SceneElement::FaceEdge(ConstraintLine::OriginAxis(SketchAxis::X))
        ));
        assert!(filter.accepts(&doc, &SceneElement::GlobalAxis(GlobalAxis::X)));
        assert!(
            !filter.accepts(&doc, &SceneElement::GlobalAxis(GlobalAxis::Z)),
            "world Z is the ground-sketch normal, not a 2D mirror line"
        );
        assert!(!filter.accepts(&doc, &SceneElement::Circle(rkey(0))));
    }

    #[test]
    fn selected_color_override_wins() {
        let default = Color32::from_rgb(1, 2, 3);
        let red = Color32::from_rgb(200, 0, 0);
        let plain = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        assert_eq!(plain.selected_color(default), default);
        let cutters = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite)
            .with_selected_color(red);
        assert_eq!(cutters.selected_color(default), red);
    }

    #[test]
    fn apply_event_removes_and_clears() {
        let mut p = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);
        p.pick(&Document::default(), body(0));
        p.pick(&Document::default(), line(0));
        assert!(apply_event(&mut p, PickerEvent::Remove(0)));
        assert_eq!(p.picked(), &[line(0)]);
        assert!(!apply_event(&mut p, PickerEvent::Focus));
        assert!(apply_event(&mut p, PickerEvent::Clear));
        assert!(p.is_empty());
    }

    #[test]
    fn set_picked_drops_rejected_and_honors_limit() {
        let mut p = ElementPicker::new(ElementFilter::kind(ElementKind::Body), PickLimit::Finite(2));
        p.set_picked(&Document::default(), [body(0), line(0), body(1), body(2)]);
        assert_eq!(p.picked(), &[body(0), body(1)]);
    }

    /// #1169: the combo strip used nested `right_to_left` + `allocate_exact_size` for the
    /// dropdown caret, which thrashing auto-ids between egui multipass frames (and logged
    /// `Widget rect … changed id between passes`). Painting the caret without a widget —
    /// and without nested RTL — must still lay out empty and filled strips cleanly.
    #[test]
    fn combo_strip_survives_multipass_frames() {
        let ctx = egui::Context::default();
        ctx.options_mut(|o| {
            o.max_passes = std::num::NonZeroUsize::new(2).unwrap();
        });
        let doc = Document::default();
        let mut picker = ElementPicker::new(ElementFilter::everything(), PickLimit::Infinite);

        for frame in 0..6 {
            let _ = ctx.run_ui(Default::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    // First show of a Grid requests a multipass discard — exercises the
                    // same path that used to thrash the caret's auto-id (#1169).
                    if frame == 0 {
                        egui::Grid::new("picker_multipass_probe").show(ui, |ui| {
                            ui.label("probe");
                            ui.end_row();
                        });
                    }
                    let _ = show(ui, &picker, &doc, "picker_stability_a");
                    // A second stable-id picker so two strips sit stacked like the
                    // context pane's tool pickers.
                    let _ = show(ui, &picker, &doc, "picker_stability_b");
                });
            });
            if frame == 2 {
                picker.pick(&doc, body(0));
                picker.pick(&doc, line(0));
            }
        }
        assert_eq!(picker.picked().len(), 2);
    }
}
