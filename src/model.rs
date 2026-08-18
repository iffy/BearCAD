//! In-memory document model.
//!
//! This is the very first slice of BearCAD (see SPEC.md): a document is a flat list
//! of rectangles and lines on a single 2D sketch. As the action-DAG, components,
//! and the OCCT kernel come online this will grow, but the persistence boundary
//! (`storage.rs`) is kept narrow so the file format can evolve underneath it.

use crate::value::{AngleUnit, LengthUnit};
use serde::{Deserialize, Serialize};

/// A sketchable face that lines and rectangles can be drawn on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceId {
    Circle(CircleKey),
    /// A closed loop of plain `Line`s, identified by its ordered line indices (#66).
    Polygon(Vec<LineKey>),
    ConstructionPlane(ConstructionPlaneKey),
    /// A planar cap face of an extruded body: one profile face of an extrusion,
    /// at either the base (`top = false`) or offset (`top = true`) end.
    ExtrudeCap {
        extrusion: ExtrusionKey,
        profile: ExtrudeFace,
        top: bool,
    },
    /// A planar side wall of an extruded body: the quad swept by one `edge` of a
    /// polygonal profile (rectangles only; circular profiles have no flat sides).
    ExtrudeSide {
        extrusion: ExtrusionKey,
        profile: ExtrudeFace,
        edge: u8,
    },
    /// A flat side of a partial (< 360°) revolve (#621): one profile face rotated to the
    /// sweep's start (`end = false`) or end (`end = true`) angle. Full sweeps have none.
    RevolveCap {
        revolution: RevolutionKey,
        profile: ExtrudeFace,
        end: bool,
    },
    /// The flat washer/annular-sector face a revolve sweeps from one polygon-profile
    /// `edge` whose endpoints share an axis coordinate (#621) — e.g. the flat ends of a
    /// revolved ring. Edges not perpendicular to the axis sweep curved surfaces instead.
    RevolveSide {
        revolution: RevolutionKey,
        profile: ExtrudeFace,
        edge: u8,
    },
    /// A flat face of an imported unit (#725): the instance plus the face's analytic
    /// identity **in the unit's own document**. Resolved against the instance's rebuilt
    /// embedded document and placed by its transform, so a sketch hosted here follows the
    /// unit through override and placement changes.
    UnitFace {
        instance: UnitInstanceKey,
        face: Box<FaceId>,
    },
    /// A flat face of a primitive shape (#1103): a cuboid's six faces or a cylinder's two
    /// caps. A sphere has no flat faces. A sketch hosted here follows the primitive
    /// through edits to its frame and dimensions, exactly like one on an extrusion cap.
    PrimitiveFace {
        primitive: PrimitiveKey,
        face: PrimitiveFace,
    },
    /// A face of a **repeated** body instance (#1116): the source analytic face
    /// (typically an [`ExtrudeCap`](FaceId::ExtrudeCap)/[`ExtrudeSide`](FaceId::ExtrudeSide)
    /// of the input extrusion) placed by that instance's repeat offset. A repeated body
    /// is the same kind of body as any other — its faces are sketchable and
    /// extrude/revolve-able just like the original's.
    RepeatedFace {
        face: Box<FaceId>,
        #[serde(rename = "repeat_op")]
        op: RepeatOpKey,
        #[serde(default)]
        instance: usize,
    },
    /// A planar face of a body's **live solid mesh** (#1173): keyed like
    /// [`crate::hierarchy::SceneElement::BodyFace`] (quantized centroid + normal). Hosts a
    /// sketch on surfaces that have no analytic identity — a shell's inner wall, a boolean
    /// face, an imported flat — so the face under the cursor wins over a parallel outer face
    /// buried behind it along the pick ray.
    BodyMeshFace {
        body: BodyKey,
        centroid: [i32; 3],
        normal: [i32; 3],
    },
}

/// Which flat face of a primitive shape a [`FaceId::PrimitiveFace`] names (#1103).
/// A cuboid has six (bottom, top, four sides); a cylinder has two caps; a sphere has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveFace {
    CuboidBottom,
    CuboidTop,
    /// One of the four side walls, indexed by the base corner it spans (0..3).
    CuboidSide { edge: u8 },
    CylinderBottom,
    CylinderTop,
}

impl Default for FaceId {
    /// A face naming nothing. There is no "plane 0" to fall back on now that planes are
    /// keyed (#1055), and an empty polygon is already the crate's stand-in for a face with
    /// no geometry.
    fn default() -> Self {
        FaceId::Polygon(Vec::new())
    }
}

impl FaceId {
    /// Whether this face is a datum plane rather than real geometry (#844): a click that
    /// could mean either means the geometry.
    pub fn is_construction_plane(&self) -> bool {
        matches!(self, FaceId::ConstructionPlane(_))
    }

    /// A face from a script's `(kind, index)` pair. A circle is named by its **ordinal**
    /// among the live ones (#1055), so resolving one needs the document.
    pub fn from_script(doc: &Document, kind: &str, index: usize) -> Option<Self> {
        match kind.to_ascii_lowercase().as_str() {
            "circle" => Some(FaceId::Circle(doc.circles.keys().nth(index)?)),
            "plane" | "construction_plane" | "constructionplane" => {
                Some(FaceId::ConstructionPlane(doc.construction_planes.keys().nth(index)?))
            }
            _ => None,
        }
    }

    /// The extrusion index that owns this face, for the two body-face variants (#26/#27's
    /// `FaceVertex`/`FaceEdge` dependency tracking piggybacks on this: a sketch on a body face,
    /// or a constraint referencing that face's own boundary, both depend on the extrusion that
    /// produced it — same relationship `hierarchy::face_element` already tracks for sketches).
    pub fn extrusion_index(&self) -> Option<ExtrusionKey> {
        match self {
            FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
                Some(*extrusion)
            }
            FaceId::Circle(_)
            | FaceId::Polygon(_)
            | FaceId::ConstructionPlane(_)
            | FaceId::RevolveCap { .. }
            | FaceId::RevolveSide { .. }
            | FaceId::UnitFace { .. }
            | FaceId::PrimitiveFace { .. }
            | FaceId::RepeatedFace { .. }
            | FaceId::BodyMeshFace { .. } => None,
        }
    }

    /// The revolution that owns this face — the [`FaceId::extrusion_index`]
    /// analogue for sketches hosted on a revolve's flat sides (#621).
    pub fn revolution_key(&self) -> Option<RevolutionKey> {
        match self {
            FaceId::RevolveCap { revolution, .. } | FaceId::RevolveSide { revolution, .. } => {
                Some(*revolution)
            }
            FaceId::RepeatedFace { face, .. } => face.revolution_key(),
            _ => None,
        }
    }
}

/// Index into [`Document::sketches`].
pub type SketchId = crate::arena::Key<Sketch>;

/// Geometry that drives a read-only parameter value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterSource {
    LineLength(LineKey),
    /// Distance between two points (#432), measured in world space (2D or 3D).
    PointDistance(ConstraintPoint, ConstraintPoint),
    /// Distance between two parallel lines (#432).
    LineDistance(LineKey, LineKey),
    /// Angle between two non-parallel lines in the same sketch (#432), stored in degrees.
    LineAngle(LineKey, LineKey),
    /// Length of a **body's feature edge** (#647), keyed the way
    /// [`crate::hierarchy::SceneElement::BodyEdge`] is: the body plus the edge's quantized
    /// world endpoints. Re-resolved against the body's live mesh, so it reads the current
    /// length; if a rebuild moves the edge off that key, the parameter reads as unavailable
    /// (the same way a deleted line's does).
    BodyEdgeLength {
        body: BodyKey,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// Distance between two **body mesh corners** (#647), keyed like
    /// [`crate::hierarchy::SceneElement::BodyVertex`]. The two corners may sit on different
    /// bodies.
    BodyVertexDistance {
        body_a: BodyKey,
        a: [i32; 3],
        body_b: BodyKey,
        b: [i32; 3],
    },
    /// Length of an **imported unit's feature edge** (#724), stored analytically: the
    /// instance plus the `(face, edge ordinal)` in the unit's own document. Unlike the
    /// quantized [`Self::BodyEdgeLength`] key, this re-resolves after the instance's
    /// parameter overrides change — the boundary loop is recomputed from the rebuilt
    /// embedded document, so the dimension follows the unit.
    UnitEdgeLength {
        instance: UnitInstanceKey,
        face: FaceId,
        edge: usize,
    },
}

/// A named length or angle parameter (expression stored verbatim, evaluated on demand).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub expression: String,
    /// Primary parameters (#727) are a file's front door: the knobs someone importing
    /// this document is expected to change; secondary ones are internals. Advisory only —
    /// nothing is blocked either way. Existing documents load secondary (`default`); a
    /// newly created parameter is primary when its expression is a plain self-contained
    /// value and secondary when it references anything — computed once at creation
    /// (see `new_parameter_primary_default`), never recomputed on edit. UI toggles the
    /// inverse via the gear-options **Private** checkbox (#1176/#1180): checked = this
    /// flag false; plain-value new params start with Private unchecked.
    #[serde(default)]
    pub primary: bool,
    /// Optional minimum bound as an expression (#1176). Same unit kind as the default
    /// value. Empty / missing means unbounded below. Importers' overrides must respect it.
    #[serde(default)]
    pub minimum: Option<String>,
    /// Optional maximum bound as an expression (#1176). Same unit kind as the default
    /// value. Empty / missing means unbounded above. With both min and max set, the
    /// import UI offers a slider.
    #[serde(default)]
    pub maximum: Option<String>,
    /// Optional step as an expression (#1176): override values snap to the closest
    /// `minimum + n·step` (or `n·step` when no minimum). Empty / missing means free.
    #[serde(default)]
    pub step: Option<String>,
    /// When set, [`expression`] is synced from geometry and the value is read-only.
    #[serde(default)]
    pub source: Option<ParameterSource>,
}

/// How anything names a parameter by identity rather than by name (#1055).
pub type ParameterKey = crate::arena::Key<Parameter>;

/// A 2D sketch hosted on a face. A single face may host multiple independent sketches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sketch {
    pub face: FaceId,
    /// User-visible label in the Elements pane; empty uses the default.
    #[serde(default)]
    pub name: Option<String>,
    /// Default length unit override for this sketch; `None` inherits [`Document::default_length_unit`] (#52).
    #[serde(default)]
    pub length_unit: Option<LengthUnit>,
    /// Default angle unit override for this sketch; `None` inherits [`Document::default_angle_unit`] (#52).
    #[serde(default)]
    pub angle_unit: Option<AngleUnit>,
}

/// A line segment in face-local coordinates (millimetres, per SPEC §5.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub sketch: SketchId,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    /// Length was explicitly typed by the user (show dimension in sketch edit mode).
    #[serde(default)]
    pub length_locked: bool,
    /// User-placed offset from the measured segment to the length dimension line (px).
    #[serde(default)]
    pub length_dim_offset: Option<f32>,
    /// Expression text when [`length_locked`] is set.
    #[serde(default)]
    pub length_expr: Option<String>,
    /// Reference geometry (dashed, construction color); not solid model geometry.
    #[serde(default)]
    pub construction: bool,
    /// Consumed by a 2D in-sketch slice (#224): the original is kept (for editing/undo) but no
    /// longer participates in solid geometry — its split fragments do. Excluded from face/profile
    /// detection wherever [`construction`](Self::construction) is.
    #[serde(default)]
    pub shadow: bool,
    /// User-visible label in the Elements pane; empty uses the default.
    #[serde(default)]
    pub name: Option<String>,
    /// Cubic-bezier tangent handles in face-local coords: `[near (x0,y0), near (x1,y1)]`.
    /// `None` means a straight segment (the common case).
    #[serde(default)]
    pub bezier: Option<[(f32, f32); 2]>,
    /// Set when this line is the bridging line created by a chamfer/fillet vertex treatment
    /// (#37/#38): the trimmed line it nests under in the Elements pane (see
    /// [`crate::hierarchy`], #76). `None` for an ordinary line.
    #[serde(default)]
    pub chamfer_fillet_parent: Option<LineKey>,
    /// Set when this line is an **associative projection** of external 3D geometry into its
    /// sketch (#140): each geometry recompute re-resolves the source and rewrites the
    /// endpoints (see `crate::projection`). Projected lines render solid cyan (#1186;
    /// distinct from dashed construction), are fixed (not draggable), and otherwise behave
    /// like construction geometry.
    #[serde(default)]
    pub projection: Option<ProjectionSource>,
}

/// A line's identity (#1055): stable across deletions of other lines.
pub type LineKey = crate::arena::Key<Line>;

/// Source geometry an associative projection tracks (#140). Body mesh edges are identified
/// by their quantized endpoints (the same geometry-keyed identity 3D selection uses, #156):
/// there is no stable topological name for mesh edges, so if a rebuild moves the source the
/// projection keeps its last resolved shape (a static fallback) rather than dangling.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProjectionSource {
    BodyEdge {
        body: BodyKey,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// A boundary edge of an imported unit's analytic face (#725): unlike the quantized
    /// [`Self::BodyEdge`] key, this re-resolves against the instance's rebuilt embedded
    /// document, so the projection follows the unit's parameter overrides.
    UnitEdge {
        instance: UnitInstanceKey,
        face: FaceId,
        edge: usize,
    },
    /// A construction plane (#983): the projected line runs along the two planes'
    /// intersection, spanning the source plane's drawn extent. Identified by index — stable,
    /// unlike a mesh edge — so the reference follows the plane through moves and resizes.
    Plane { plane: ConstructionPlaneKey },
}

/// Number of straight sub-segments used to approximate a curved [`Line`] for rendering,
/// hit-testing, and extrusion tessellation (mirrors [`CIRCLE_SEGMENTS`]-style faceting).
pub const BEZIER_SEGMENTS: usize = 24;

impl Line {
    pub fn from_local_endpoints(
        sketch: SketchId,
        u0: f32,
        v0: f32,
        u1: f32,
        v1: f32,
    ) -> Self {
        Self {
            sketch,
            x0: u0,
            y0: v0,
            x1: u1,
            y1: v1,
            length_locked: false,
            length_dim_offset: None,
            length_expr: None,
            construction: false,
            shadow: false,
            name: None,
            bezier: None,
            chamfer_fillet_parent: None,
            projection: None,
        }
    }

    /// Straight-line distance between the two endpoints. For a curved line this is the
    /// chord, which is what a length dimension constrains (the sketch solver moves
    /// endpoints, not bezier handles).
    pub fn chord_length(&self) -> f32 {
        let du = self.x1 - self.x0;
        let dv = self.y1 - self.y0;
        (du * du + dv * dv).sqrt()
    }

    /// True length of the segment: the chord for straight lines, the bezier arc length
    /// for curved ones. Arc length sums the [`BEZIER_SEGMENTS`] tessellation from
    /// [`Self::sample_local`] so labels, introspection, and the rendered/extruded mesh
    /// all agree on the same discretization.
    pub fn length(&self) -> f32 {
        if !self.is_curved() {
            return self.chord_length();
        }
        self.sample_local(BEZIER_SEGMENTS)
            .windows(2)
            .map(|w| {
                let du = w[1].0 - w[0].0;
                let dv = w[1].1 - w[0].1;
                (du * du + dv * dv).sqrt()
            })
            .sum()
    }

    pub fn is_curved(&self) -> bool {
        self.bezier.is_some()
    }

    /// Sample this segment as a polyline in local coords (`segments + 1` points).
    /// Straight lines just return the two endpoints regardless of `segments`.
    pub fn sample_local(&self, segments: usize) -> Vec<(f32, f32)> {
        let p0 = (self.x0, self.y0);
        let p1 = (self.x1, self.y1);
        match self.bezier {
            None => vec![p0, p1],
            Some([c0, c1]) => (0..=segments)
                .map(|i| cubic_bezier_point(p0, c0, c1, p1, i as f32 / segments as f32))
                .collect(),
        }
    }
}

fn cubic_bezier_point(p0: (f32, f32), c0: (f32, f32), c1: (f32, f32), p1: (f32, f32), t: f32) -> (f32, f32) {
    let mt = 1.0 - t;
    let a = mt * mt * mt;
    let b = 3.0 * mt * mt * t;
    let c = 3.0 * mt * t * t;
    let d = t * t * t;
    (
        a * p0.0 + b * c0.0 + c * c1.0 + d * p1.0,
        a * p0.1 + b * c0.1 + c * c1.1 + d * p1.1,
    )
}

/// Smooths the joint at a shared vertex `v` between two lines (right-click "convert to bezier
/// curve"), given each line's other endpoint `a`/`b`. The tangent through `v` runs along the
/// `a`→`b` chord (Catmull-Rom style), so the curve stays visually smooth across the joint; each
/// line's far handle (away from `v`) sits a third of the way toward `v`, keeping that end
/// nearly straight since only the joint itself is being rounded.
///
/// Returns `([handle_near_a, handle_near_v], [handle_near_v, handle_near_b])` for the first and
/// second line respectively.
pub fn smooth_joint_bezier(
    a: (f32, f32),
    v: (f32, f32),
    b: (f32, f32),
) -> ([(f32, f32); 2], [(f32, f32); 2]) {
    let tx = b.0 - a.0;
    let ty = b.1 - a.1;
    let tlen = (tx * tx + ty * ty).sqrt();
    let unit = if tlen > 1e-6 { (tx / tlen, ty / tlen) } else { (0.0, 0.0) };

    let dist_av = ((v.0 - a.0).powi(2) + (v.1 - a.1).powi(2)).sqrt();
    let dist_vb = ((b.0 - v.0).powi(2) + (b.1 - v.1).powi(2)).sqrt();

    let h1_far = (a.0 + (v.0 - a.0) / 3.0, a.1 + (v.1 - a.1) / 3.0);
    let h1_near = (v.0 - unit.0 * dist_av / 3.0, v.1 - unit.1 * dist_av / 3.0);
    let h2_near = (v.0 + unit.0 * dist_vb / 3.0, v.1 + unit.1 * dist_vb / 3.0);
    let h2_far = (b.0 + (v.0 - b.0) / 3.0, b.1 + (v.1 - b.1) / 3.0);

    ([h1_far, h1_near], [h2_near, h2_far])
}

/// Default "corner point" tangent handle a third of the way from `from` toward `to`. Used
/// for a curve-mode segment's own handle when the tangent-constraint toggle is off: each
/// side of a vertex gets this independent, un-mirrored handle instead of one derived from
/// [`smooth_joint_bezier`] (#73).
pub fn independent_corner_handle(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    (from.0 + (to.0 - from.0) / 3.0, from.1 + (to.1 - from.1) / 3.0)
}

/// Whether a sketch-vertex treatment truncates the two adjoining lines and bridges them with a
/// straight cut (chamfer) or a rounded single-cubic-bezier arc (fillet). See SPEC §3.1, #37/#38.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VertexTreatmentKind {
    Chamfer,
    Fillet,
}

/// Truncated endpoints (and, for a fillet, bridging-line tangent-handle bezier control points)
/// produced by [`vertex_treatment_geometry`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VertexTreatmentGeometry {
    /// New endpoint for the line whose far point was `a` (truncated back from the vertex).
    pub p1: (f32, f32),
    /// New endpoint for the line whose far point was `b` (truncated back from the vertex).
    pub p2: (f32, f32),
    /// `Some` for a fillet (bridging line curves); `None` for a chamfer (bridging line is
    /// straight).
    pub bezier: Option<[(f32, f32); 2]>,
}

/// Interior angle (radians, within ~1° of 0° or 180°) treated as a degenerate corner: the two
/// edges are (nearly) parallel or anti-parallel, so there's no real corner to chamfer/fillet.
const VERTEX_TREATMENT_DEGENERATE_EPS: f32 = 0.0175; // ~1 degree

/// Computes the truncated endpoints (and bridging-line geometry) for a chamfer or fillet applied
/// at a sketch vertex `v` shared by two lines whose other ("far") endpoints are `a` and `b`, in
/// face-local/sketch-local UV coordinates (same convention as [`smooth_joint_bezier`]).
///
/// `amount` is the chamfer distance (straight tangent length back from `v`) or the fillet radius,
/// depending on `kind`. Returns `None` when `amount` isn't positive, either adjacent edge is
/// degenerate (zero length), or the corner itself is degenerate (interior angle within ~1° of 0°
/// or 180° — the edges are parallel/anti-parallel, so there's no real corner to round or cut).
///
/// The tangent length back from `v` is clamped so it never cuts back past either adjacent edge's
/// own far endpoint; for a fillet, the effective radius (and its arc) are recomputed from the
/// clamped tangent length so the arc stays geometrically consistent with where the truncated
/// endpoints actually land, rather than the originally requested radius.
pub fn vertex_treatment_geometry(
    v: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
    kind: VertexTreatmentKind,
    amount: f32,
) -> Option<VertexTreatmentGeometry> {
    if !(amount > 0.0) {
        return None;
    }
    let dist_va = ((a.0 - v.0).powi(2) + (a.1 - v.1).powi(2)).sqrt();
    let dist_vb = ((b.0 - v.0).powi(2) + (b.1 - v.1).powi(2)).sqrt();
    if dist_va < 1e-6 || dist_vb < 1e-6 {
        return None;
    }
    let dir_a = ((a.0 - v.0) / dist_va, (a.1 - v.1) / dist_va);
    let dir_b = ((b.0 - v.0) / dist_vb, (b.1 - v.1) / dist_vb);
    let cos_alpha = (dir_a.0 * dir_b.0 + dir_a.1 * dir_b.1).clamp(-1.0, 1.0);
    let alpha = cos_alpha.acos();
    if !(VERTEX_TREATMENT_DEGENERATE_EPS
        ..=std::f32::consts::PI - VERTEX_TREATMENT_DEGENERATE_EPS)
        .contains(&alpha)
    {
        return None;
    }

    let raw_t = match kind {
        VertexTreatmentKind::Chamfer => amount,
        VertexTreatmentKind::Fillet => amount / (alpha / 2.0).tan(),
    };
    let max_t = (dist_va * 0.95).min(dist_vb * 0.95);
    let t = raw_t.min(max_t);

    let p1 = (v.0 + dir_a.0 * t, v.1 + dir_a.1 * t);
    let p2 = (v.0 + dir_b.0 * t, v.1 + dir_b.1 * t);

    let bezier = match kind {
        VertexTreatmentKind::Chamfer => None,
        VertexTreatmentKind::Fillet => {
            // Recompute the effective radius from the (possibly clamped) tangent length so the
            // arc stays consistent with where p1/p2 actually landed.
            let radius = t * (alpha / 2.0).tan();
            let theta = std::f32::consts::PI - alpha;
            let k = radius * (4.0 / 3.0) * (theta / 4.0).tan();
            let h0 = (p1.0 - dir_a.0 * k, p1.1 - dir_a.1 * k);
            let h1 = (p2.0 - dir_b.0 * k, p2.1 - dir_b.1 * k);
            Some([h0, h1])
        }
    };

    Some(VertexTreatmentGeometry { p1, p2, bezier })
}

/// Re-fit the bezier handles of fillet-bridge arcs after a solve moved their endpoints.
///
/// A vertex fillet's arc is a single cubic bezier whose handles were computed for the corner
/// geometry *at creation time* ([`vertex_treatment_geometry`]). The sketch solver moves line
/// endpoints only, so when constraints reshape the profile (say a parameter-driven bend angle
/// changes), the arc's endpoints follow the trimmed lines but its handles stay stale — the
/// bend folds over itself and any extrusion built from the loop self-intersects. This re-fit
/// recomputes each fillet arc as the circular arc tangent to its two neighbouring lines at
/// the arc's current endpoints (the trims stay where the dimensions hold them, so the
/// effective radius follows the new corner angle).
pub fn refit_fillet_arc_handles(doc: &mut Document, sketch: SketchId) {
    const EPS: f32 = 1e-3;
    let arcs: Vec<crate::model::LineKey> = doc
        .lines
        .iter()
        .filter(|(_, l)| {
            l.sketch == sketch && l.chamfer_fillet_parent.is_some() && l.is_curved()
        })
        .map(|(i, _)| i)
        .collect();
    for arc in arcs {
        let (p0, p1) = {
            let l = &doc.lines[arc];
            ((l.x0, l.y0), (l.x1, l.y1))
        };
        // The straight tangent direction at an arc endpoint: along the neighbouring line,
        // pointing from its far end toward the shared endpoint (i.e. toward the trimmed-away
        // virtual corner beyond the arc).
        let tangent_at = |doc: &Document, p: (f32, f32)| -> Option<(f32, f32)> {
            for (j, l) in doc.lines.iter() {
                if j == arc || l.sketch != sketch || l.construction || l.is_curved()
                {
                    continue;
                }
                let (near, far) = if (l.x1 - p.0).abs() < EPS && (l.y1 - p.1).abs() < EPS {
                    ((l.x1, l.y1), (l.x0, l.y0))
                } else if (l.x0 - p.0).abs() < EPS && (l.y0 - p.1).abs() < EPS {
                    ((l.x0, l.y0), (l.x1, l.y1))
                } else {
                    continue;
                };
                let d = (near.0 - far.0, near.1 - far.1);
                let len = (d.0 * d.0 + d.1 * d.1).sqrt();
                if len > 1e-6 {
                    return Some((d.0 / len, d.1 / len));
                }
            }
            None
        };
        let (Some(u0), Some(u1)) = (tangent_at(doc, p0), tangent_at(doc, p1)) else {
            continue;
        };
        // Virtual corner: p0 + s*u0 == p1 + t*u1.
        let det = u0.0 * (-u1.1) - u0.1 * (-u1.0);
        if det.abs() < 1e-6 {
            continue;
        }
        let (rx, ry) = (p1.0 - p0.0, p1.1 - p0.1);
        let s = (rx * (-u1.1) - ry * (-u1.0)) / det;
        let v = (p0.0 + u0.0 * s, p0.1 + u0.1 * s);
        let to0 = (p0.0 - v.0, p0.1 - v.1);
        let to1 = (p1.0 - v.0, p1.1 - v.1);
        let (l0, l1) = (to0.0.hypot(to0.1), to1.0.hypot(to1.1));
        if l0 < 1e-6 || l1 < 1e-6 {
            continue;
        }
        let dir_a = (to0.0 / l0, to0.1 / l0);
        let dir_b = (to1.0 / l1, to1.1 / l1);
        let cos_alpha = (dir_a.0 * dir_b.0 + dir_a.1 * dir_b.1).clamp(-1.0, 1.0);
        let alpha = cos_alpha.acos();
        if !(VERTEX_TREATMENT_DEGENERATE_EPS
            ..=std::f32::consts::PI - VERTEX_TREATMENT_DEGENERATE_EPS)
            .contains(&alpha)
        {
            continue;
        }
        // Same handle-length formula as vertex_treatment_geometry, with the tangent length
        // averaged (asymmetric trims can't host an exactly tangent circle; the average keeps
        // the arc smooth and inside the corner).
        let t_avg = (l0 + l1) * 0.5;
        let radius = t_avg * (alpha / 2.0).tan();
        let theta = std::f32::consts::PI - alpha;
        let k = radius * (4.0 / 3.0) * (theta / 4.0).tan();
        let h0 = (p0.0 - dir_a.0 * k, p0.1 - dir_a.1 * k);
        let h1 = (p1.0 - dir_b.0 * k, p1.1 - dir_b.1 * k);
        doc.lines[arc].bezier = Some([h0, h1]);
    }
}

/// Which analytic edge family of an extrusion-sourced solid an [`EdgeTreatment`] targets
/// (#77): a 3D edge chamfer/fillet is a mesh-bevel approximation limited to the two edge
/// kinds that have a clean analytic definition for a `Rect`/`Polygon` profile — see
/// `crate::extrude::side_quad_world`/`cap_polygon_world`. A `Circle` profile has neither (its
/// side is curved, with no discrete side walls — `side_face_count` is 0), so it's out of
/// scope; so are STL/STEP-imported bodies (no analytic profile at all). See SPEC §3.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtrusionEdgeRef {
    /// The vertical edge shared by side walls `edge` and `edge + 1` (mod the profile's vertex
    /// count) of `face` (an index into [`Extrusion::faces`]) — i.e. the edge at profile vertex
    /// `(edge + 1) % n`, running the full height from base to top cap.
    Vertical { face: usize, edge: usize },
    /// The edge where side wall `edge` of `face` meets a cap: the base cap when `top` is
    /// `false`, the top cap when `true` (also a `cap_polygon_world` boundary edge).
    Cap { face: usize, edge: usize, top: bool },
}

impl ExtrusionEdgeRef {
    /// The face index this edge belongs to (an index into [`Extrusion::faces`]).
    pub fn face(self) -> usize {
        match self {
            ExtrusionEdgeRef::Vertical { face, .. } => face,
            ExtrusionEdgeRef::Cap { face, .. } => face,
        }
    }
}

/// The solid an analytic chamfer/fillet edge belongs to (#1329): an extrusion, or a
/// Shape-tool primitive. A cuboid has the same 12-edge topology as a rectangular
/// extrusion; a cylinder's two rims are `Cap { edge: 0, top }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreatableSolid {
    Extrusion(ExtrusionKey),
    Primitive(PrimitiveKey),
}

impl TreatableSolid {
    pub fn as_extrusion(self) -> Option<ExtrusionKey> {
        match self {
            Self::Extrusion(k) => Some(k),
            Self::Primitive(_) => None,
        }
    }
}

/// A parametric chamfer/fillet bevel applied to one analytic edge of an [`Extrusion`]'s solid
/// (#77): a mesh-bevel approximation, not a true BREP fillet (no tangent-continuous curved
/// surface, no vertex-miter blending) — see SPEC §3.4. Re-evaluated from the document every
/// frame by `crate::extrude::extrusion_mesh`, like everything else in this app; nothing here
/// is a baked/one-time mesh edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeTreatment {
    pub edge: ExtrusionEdgeRef,
    pub kind: VertexTreatmentKind,
    /// Chamfer distance or fillet radius (mm); must be positive to have any effect.
    pub amount: f32,
}

/// A circle in face-local coordinates (millimetres, per SPEC §5.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Circle {
    pub sketch: SketchId,
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    /// Diameter was explicitly typed by the user (show dimension in sketch edit mode).
    #[serde(default)]
    pub diameter_locked: bool,
    /// User-placed outward offset of the diameter label from the dimension line (px).
    #[serde(default)]
    pub diameter_dim_offset: Option<f32>,
    /// Expression text when [`diameter_locked`] is set.
    #[serde(default)]
    pub diameter_expr: Option<String>,
    /// Angle (radians) of the diameter dimension line in local (u, v) coords.
    #[serde(default)]
    pub diameter_dim_angle: f32,
    /// Reference geometry (dashed, construction color); not solid model geometry.
    #[serde(default)]
    pub construction: bool,
    /// Consumed by a 2D in-sketch slice (#224); see [`Line::shadow`].
    #[serde(default)]
    pub shadow: bool,
    /// User-visible label in the Elements pane; empty uses the default.
    #[serde(default)]
    pub name: Option<String>,
}

/// A circle's identity (#1055): stable across deletions of other circles.
pub type CircleKey = crate::arena::Key<Circle>;

impl Circle {
    pub fn from_local_center_radius(
        sketch: SketchId,
        cx: f32,
        cy: f32,
        r: f32,
        diameter_dim_angle: f32,
    ) -> Self {
        Self {
            sketch,
            cx,
            cy,
            r,
            diameter_locked: false,
            diameter_dim_offset: None,
            diameter_expr: None,
            diameter_dim_angle,
            construction: false,
            shadow: false,
            name: None,
        }
    }

    pub fn diameter(&self) -> f32 {
        self.r * 2.0
    }
}

/// Reference geometry a construction plane was built from (for later editing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PlaneAnchor {
    Face {
        origin: glam::Vec3,
        normal: glam::Vec3,
        label: String,
    },
    Axis {
        origin: glam::Vec3,
        direction: glam::Vec3,
        label: String,
    },
}

/// Editable offset/angle parameters that define a construction plane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaneDefinition {
    pub anchor: PlaneAnchor,
    pub offset_mm: f32,
    pub angle_deg: f32,
}

impl PlaneDefinition {
    pub fn is_axis(&self) -> bool {
        matches!(self.anchor, PlaneAnchor::Axis { .. })
    }
}

/// Where a construction plane sits in the scene hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ConstructionPlaneParent {
    /// Datum plane (default XY, ground, global axes, etc.).
    #[default]
    Root,
    /// Derived from geometry in a sketch.
    Sketch(SketchId),
}

/// A construction plane in world space (millimetres).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConstructionPlane {
    pub origin: glam::Vec3,
    pub normal: glam::Vec3,
    pub u_axis: glam::Vec3,
    pub v_axis: glam::Vec3,
    pub parent: ConstructionPlaneParent,
    pub definition: PlaneDefinition,
    /// Set when this plane is a generated instance of a Repeat op (#221): a copy of a source
    /// plane offset along the op's axis. Its cached frame is derived at recompute (see
    /// [`RepeatPlaneInstance`]); the `definition` is a copy of the source's and is not used for
    /// the instance's placement.
    #[serde(default)]
    pub repeat_instance: Option<RepeatPlaneInstance>,
    /// User-visible label in the Elements pane; empty uses the default.
    pub name: Option<String>,
    /// How far the drawn plane reaches in its own u/v axes (#833). Defaults to the old
    /// symmetric ±`PLANE_DISPLAY_HALF` square, so documents saved before extents existed
    /// look exactly as they did; the planes a new document starts with instead sit in one
    /// quadrant, and any plane can be resized by dragging its corner handles.
    #[serde(default)]
    pub extent: PlaneExtent,
}

/// A construction plane's identity (#1055): stable across deletions of other planes.
pub type ConstructionPlaneKey = crate::arena::Key<ConstructionPlane>;

/// A construction plane's drawn rectangle, in its own u/v millimetres relative to its origin
/// (#833). `u_min`/`v_min` are the low corner, `u_max`/`v_max` the high one — the two the
/// resize handles sit on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaneExtent {
    pub u_min: f32,
    pub u_max: f32,
    pub v_min: f32,
    pub v_max: f32,
}

impl Default for PlaneExtent {
    fn default() -> Self {
        let half = crate::construction::PLANE_DISPLAY_HALF;
        Self { u_min: -half, u_max: half, v_min: -half, v_max: half }
    }
}

impl PlaneExtent {
    /// A square `size` across sitting in the +u/+v quadrant, held `gap` clear of the plane's
    /// origin in both directions — what the three planes a new document opens with use, so
    /// they don't meet in a solid corner at the world origin (#838).
    pub fn quadrant(size: f32, gap: f32) -> Self {
        Self { u_min: gap, u_max: gap + size, v_min: gap, v_max: gap + size }
    }

    /// Keep the rectangle non-degenerate however the handles are dragged: at least
    /// `MIN_PLANE_EXTENT_MM` across in each direction, with min below max.
    pub fn normalized(self) -> Self {
        let (u_min, u_max) = ordered_span(self.u_min, self.u_max);
        let (v_min, v_max) = ordered_span(self.v_min, self.v_max);
        Self { u_min, u_max, v_min, v_max }
    }
}

/// Smallest side a construction plane can be dragged down to (mm).
pub const MIN_PLANE_EXTENT_MM: f32 = 5.0;

fn ordered_span(a: f32, b: f32) -> (f32, f32) {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    if hi - lo >= MIN_PLANE_EXTENT_MM {
        (lo, hi)
    } else {
        let mid = (lo + hi) * 0.5;
        (mid - MIN_PLANE_EXTENT_MM * 0.5, mid + MIN_PLANE_EXTENT_MM * 0.5)
    }
}

/// Which end of a line segment a constraint point refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnd {
    Start,
    End,
}

/// A point-like sketch entity for coincident and other constraints.
///
/// One of a text box's nine anchor points (#356): the four corners, four edge midpoints, and the
/// centre. Used to pin a sketch text to a sketch point (`SketchText::pin`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    #[default]
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl TextAnchor {
    /// The scripting name for this anchor (`bearcad.select{ anchor = ... }`), matching
    /// `lua_script::parse_text_anchor`'s canonical spellings.
    pub fn lua_name(self) -> &'static str {
        match self {
            TextAnchor::TopLeft => "top_left",
            TextAnchor::TopCenter => "top_center",
            TextAnchor::TopRight => "top_right",
            TextAnchor::MiddleLeft => "middle_left",
            TextAnchor::Center => "center",
            TextAnchor::MiddleRight => "middle_right",
            TextAnchor::BottomLeft => "bottom_left",
            TextAnchor::BottomCenter => "bottom_center",
            TextAnchor::BottomRight => "bottom_right",
        }
    }

    /// All nine anchors, in reading order (top-left → bottom-right).
    pub const ALL: [TextAnchor; 9] = [
        TextAnchor::TopLeft,
        TextAnchor::TopCenter,
        TextAnchor::TopRight,
        TextAnchor::MiddleLeft,
        TextAnchor::Center,
        TextAnchor::MiddleRight,
        TextAnchor::BottomLeft,
        TextAnchor::BottomCenter,
        TextAnchor::BottomRight,
    ];

    /// `(fx, fy)` fractions across the text's bounding box: x 0=left/0.5=centre/1=right, y
    /// 0=bottom/0.5=middle/1=top (baseline space, y up).
    pub fn fractions(self) -> (f32, f32) {
        let x = match self {
            TextAnchor::TopLeft | TextAnchor::MiddleLeft | TextAnchor::BottomLeft => 0.0,
            TextAnchor::TopCenter | TextAnchor::Center | TextAnchor::BottomCenter => 0.5,
            TextAnchor::TopRight | TextAnchor::MiddleRight | TextAnchor::BottomRight => 1.0,
        };
        let y = match self {
            TextAnchor::TopLeft | TextAnchor::TopCenter | TextAnchor::TopRight => 1.0,
            TextAnchor::MiddleLeft | TextAnchor::Center | TextAnchor::MiddleRight => 0.5,
            TextAnchor::BottomLeft | TextAnchor::BottomCenter | TextAnchor::BottomRight => 0.0,
        };
        (x, y)
    }
}

/// Not `Copy`: [`FaceVertex`](Self::FaceVertex) embeds a [`FaceId`], which is not `Copy`
/// (its `Polygon`/extrusion-profile variants own a `Vec<usize>`). Callers that used to rely on
/// implicit copies now need an explicit `.clone()`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintPoint {
    LineEndpoint { line: LineKey, end: LineEnd },
    CircleCenter(CircleKey),
    /// The sketch origin (local UV `(0, 0)`). Fixed, like [`ConstraintEntity::Origin`]:
    /// on a circular cap this *is* the extruded circle's centre, so a dimension from
    /// here to a hole locates it (#1436).
    Origin,
    /// A corner of an extrusion-backed face's own boundary loop (#26/#27): index into
    /// [`crate::extrude::face_boundary_loop_world`]'s ordered vertex list. Scoped to
    /// `FaceId::ExtrudeCap`/`FaceId::ExtrudeSide`; other face kinds never resolve. Fixed by
    /// the body's geometry, not draggable — mirrors [`ConstraintEntity::Origin`].
    FaceVertex { face: FaceId, index: usize },
    /// One of a sketch text's nine anchor points (#408): the bounding-box corners, edge
    /// midpoints, or centre. Solving moves the text's `origin` (the whole text translates
    /// rigidly); its rotation and size never change from constraints.
    TextAnchor { text: SketchTextKey, anchor: TextAnchor },
    /// One of a tracing image's two calibration reference points (#425). Solving moves the
    /// image's `origin` (the whole image translates rigidly); its scale never changes from
    /// constraints. Only valid in sketches hosted on the image's plane.
    ImageCalibrationPoint { image: TracingImageKey, index: usize },
}

/// A calibration reference point's host-plane-local position (#425).
pub fn image_calibration_point_uv(img: &TracingImage, index: usize) -> Option<(f32, f32)> {
    let cal = img.calibration.as_ref()?;
    let (ox, oy) = img.origin;
    let (w, h) = (img.width_mm.max(1e-6), img.height_mm.max(1e-6));
    match index {
        0 => Some((ox + cal.u0 * w, oy + cal.v0 * h)),
        1 => Some((ox + cal.u1 * w, oy + cal.v1 * h)),
        _ => None,
    }
}

/// A line-like sketch entity for parallel, perpendicular, and orientation constraints.
///
/// Not `Copy` — see [`ConstraintPoint`]'s doc comment.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintLine {
    Line(LineKey),
    /// An edge of an extrusion-backed face's own boundary loop (#26/#27): runs from
    /// `boundary_loop[index]` to `boundary_loop[(index + 1) % boundary_loop.len()]`. Same
    /// scope and fixed-geometry treatment as [`ConstraintPoint::FaceVertex`].
    FaceEdge { face: FaceId, index: usize },
    /// One of the sketch's own axes through the origin (#189): the X axis (local U, the line
    /// `v = 0`) or the Y axis (local V, the line `u = 0`). A fixed reference line — a point
    /// constrains *onto* it (point-on-line coincidence), pinning that coordinate to 0. Same
    /// "no owning sketch, fixed geometry" treatment as [`ConstraintLine::FaceEdge`].
    OriginAxis(SketchAxis),
}

/// One of a sketch's in-plane origin axes (#189): X is the local U direction, Y the local V.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SketchAxis {
    X,
    Y,
}

/// +1 or -1 disambiguation for constraints with two valid solutions.
pub type ConstraintSign = i8;

pub fn default_constraint_sign() -> ConstraintSign {
    1
}

/// Geometry a distance constraint applies to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceTarget {
    LineLength(LineKey),
    CircleDiameter(CircleKey),
    /// Spacing between parallel lines. `side` is the sign of the movable line's
    /// perpendicular offset from the reference line (+1 = positive perpendicular side).
    LineLineDistance {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
        #[serde(default = "default_constraint_sign")]
        side: ConstraintSign,
    },
    /// Distance between two points. `anchor` stays fixed; `mover` is placed
    /// `dir_u`/`dir_v` away from the anchor.
    PointPointDistance {
        anchor: ConstraintPoint,
        mover: ConstraintPoint,
        dir_u: f32,
        dir_v: f32,
    },
    /// Perpendicular distance from a point to a line. `side` is the sign of the
    /// point's offset from the line (+1 = positive perpendicular side).
    PointLineDistance {
        point: ConstraintPoint,
        line: ConstraintLine,
        #[serde(default = "default_constraint_sign")]
        side: ConstraintSign,
    },
}

/// Target for the dimension tool (distance or angle).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionTarget {
    Distance(DistanceTarget),
    Angle {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
        #[serde(default = "default_constraint_sign")]
        rotation_sign: ConstraintSign,
    },
}

/// Kind of sketch constraint.
///
/// Horizontal/Vertical were removed (#577/#580) in favour of constraining a line **parallel to a
/// sketch axis**. Documents that still contain the legacy `horizontal`/`vertical` tags load via
/// [`ConstraintKindWire`], which maps them to `Parallel` against the X/Y origin axis; new documents
/// never write those tags.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(from = "ConstraintKindWire")]
pub enum ConstraintKind {
    Distance { target: DistanceTarget },
    Parallel {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
    },
    Perpendicular {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
    },
    /// Two edges constrained to have equal length. See #47.
    Equal {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
    },
    Coincident {
        a: ConstraintEntity,
        b: ConstraintEntity,
    },
    Midpoint {
        point: ConstraintPoint,
        line: ConstraintLine,
    },
    Angle {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
        /// +1: movable line rotates counterclockwise from reference; -1: clockwise.
        #[serde(default = "default_constraint_sign")]
        rotation_sign: ConstraintSign,
    },
    /// Tangent-continuous curve joint (#473): the two curved line ends meeting at a
    /// vertex keep their handles mirrored — moving one handle rotates the partner onto
    /// the opposite ray. Maintained by the app's handle editing (not a solver equation).
    Tangent {
        a: ConstraintPoint,
        b: ConstraintPoint,
    },
}

/// Deserialize-only mirror of [`ConstraintKind`] that still understands the legacy `horizontal`/
/// `vertical` tags (#577/#580). Old documents load by mapping Horizontal → parallel to the sketch
/// X axis and Vertical → parallel to the Y axis; every other kind passes through unchanged.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConstraintKindWire {
    Distance { target: DistanceTarget },
    Parallel { line_a: ConstraintLine, line_b: ConstraintLine },
    Perpendicular { line_a: ConstraintLine, line_b: ConstraintLine },
    Equal { line_a: ConstraintLine, line_b: ConstraintLine },
    Coincident { a: ConstraintEntity, b: ConstraintEntity },
    Midpoint { point: ConstraintPoint, line: ConstraintLine },
    Horizontal { line: ConstraintLine },
    Vertical { line: ConstraintLine },
    Angle {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
        #[serde(default = "default_constraint_sign")]
        rotation_sign: ConstraintSign,
    },
    Tangent { a: ConstraintPoint, b: ConstraintPoint },
}

impl From<ConstraintKindWire> for ConstraintKind {
    fn from(w: ConstraintKindWire) -> Self {
        use ConstraintKindWire as W;
        match w {
            W::Distance { target } => ConstraintKind::Distance { target },
            W::Parallel { line_a, line_b } => ConstraintKind::Parallel { line_a, line_b },
            W::Perpendicular { line_a, line_b } => ConstraintKind::Perpendicular { line_a, line_b },
            W::Equal { line_a, line_b } => ConstraintKind::Equal { line_a, line_b },
            W::Coincident { a, b } => ConstraintKind::Coincident { a, b },
            W::Midpoint { point, line } => ConstraintKind::Midpoint { point, line },
            // Legacy Horizontal/Vertical → parallel to the X/Y sketch axis (#577/#580).
            W::Horizontal { line } => ConstraintKind::Parallel {
                line_a: line,
                line_b: ConstraintLine::OriginAxis(SketchAxis::X),
            },
            W::Vertical { line } => ConstraintKind::Parallel {
                line_a: line,
                line_b: ConstraintLine::OriginAxis(SketchAxis::Y),
            },
            W::Angle { line_a, line_b, rotation_sign } => {
                ConstraintKind::Angle { line_a, line_b, rotation_sign }
            }
            W::Tangent { a, b } => ConstraintKind::Tangent { a, b },
        }
    }
}

/// Point or line reference for coincident constraints.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintEntity {
    Point(ConstraintPoint),
    Line(ConstraintLine),
    /// A circle's perimeter (point-on-circle when paired with a point).
    Circle(CircleKey),
    /// The sketch origin (local UV `(0, 0)`); a fixed point for snapping.
    Origin,
}

/// A sketch constraint (distance is the first supported kind).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub sketch: SketchId,
    pub kind: ConstraintKind,
    pub expression: String,
    /// User-placed offset from the measured segment to the dimension line (px).
    #[serde(default)]
    pub dim_offset: Option<f32>,
    /// User-visible label in the Elements pane; empty uses the default.
    #[serde(default)]
    pub name: Option<String>,
}

/// A constraint's identity (#1055): stable across deletions of other constraints.
pub type ConstraintKey = crate::arena::Key<Constraint>;

/// A boolean combination of two coplanar sketch faces (#16/#62): the atomic regions a user
/// can toggle when two shapes overlap (their shared intersection, or one minus the other).
/// No `Union` variant is needed — unioning two shapes is already achievable by toggling both
/// of their whole-shape `ExtrudeFace`s into the same extrusion (pre-existing multi-face
/// selection), see SPEC.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    Intersection,
    /// `a` minus `b`.
    Difference,
}

/// A closed sketch profile (face) included in an extrusion.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeFace {
    Circle(CircleKey),
    /// A closed loop of plain `Line`s, identified by its ordered line indices (#66).
    Polygon(Vec<LineKey>),
    /// A boolean-combined region of two other faces (#16/#62), computed on demand via
    /// [`crate::polygon_boolean::polygon_boolean`] rather than stored as its own geometry.
    /// Recursive (`a`/`b` can themselves be `Boolean`) so the data model stays general, even
    /// though the interactive picker (see `src/face.rs`/`src/main.rs`) only ever constructs
    /// depth-1 combinations of two raw (`Rect`/`Circle`/`Polygon`) shapes.
    Boolean {
        op: BooleanOp,
        a: Box<ExtrudeFace>,
        b: Box<ExtrudeFace>,
    },
    /// One glyph region of a sketch text (#285): `text` indexes `Document::sketch_texts`, `glyph`
    /// indexes the grouped glyph regions (`text::group_glyphs`) — an outer loop plus its counters
    /// (holes). Extruding a whole text toggles one of these per glyph into `Extrusion::faces`.
    TextGlyph { text: SketchTextKey, glyph: usize },
    /// One region of a **hosted sketch's plane** (#993): the sketch's own lines together with the
    /// boundary of the face it is drawn on divide that face into regions, and this names the one
    /// containing `(seed_u, seed_v)` — a point in sketch-local coordinates.
    ///
    /// Named by a seed rather than by its boundary because the boundary is *derived*: it runs
    /// partly along lines the sketch owns and partly along the host face's own outline, which has
    /// no line indices to point at. The region is recomputed from the live sketch every time
    /// (`polygon::sketch_plane_regions`), so it follows edits the way every other profile does;
    /// if the cuts change enough that no region contains the seed any more, the profile simply
    /// stops resolving, which is what `document_health` already reports for a face gone missing.
    ///
    /// The seed is in **thousandths** of a sketch unit, so a profile stays `Eq`/`Hash` — which
    /// the pickers and the extrude face set rely on to tell one profile from another.
    SketchRegion {
        sketch: SketchId,
        seed_u: i32,
        seed_v: i32,
    },
}

impl ExtrudeFace {
    /// The sketchable face this profile corresponds to. For `Boolean`, there's no `FaceId` of
    /// its own (it's not a stored shape) — this recurses into `a` since `a` and `b` always
    /// share the same underlying sketch plane, so `a`'s frame (axes/normal) is equally valid;
    /// only its in-plane origin differs, which callers of `face_id()` don't rely on.
    pub fn face_id(&self) -> FaceId {
        match self {
            ExtrudeFace::Circle(i) => FaceId::Circle(*i),
            ExtrudeFace::Polygon(lines) => FaceId::Polygon(lines.clone()),
            ExtrudeFace::Boolean { a, .. } => a.face_id(),
            // A text glyph has no stored sketch shape; callers that need its plane go through
            // `extrude_face_sketch(doc, ..)` (which resolves the text's sketch) rather than a
            // FaceId, so this placeholder is never used to look up geometry.
            ExtrudeFace::TextGlyph { .. } => FaceId::Polygon(Vec::new()),
            // Nor does a plane region (#993) — like `Boolean`, it is computed rather than stored.
            // Its plane is the sketch's, which `extrude_face_sketch` resolves.
            ExtrudeFace::SketchRegion { .. } => FaceId::Polygon(Vec::new()),
        }
    }
}

/// An object an extrusion is constrained to reach (its extended plane), instead of a fixed
/// distance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeTarget {
    /// Up to the plane through a vertex (perpendicular to the extrusion normal).
    Vertex(ConstraintPoint),
    /// Up to the extended plane of a face.
    Face(ExtrudeFace),
    /// Up to a construction plane.
    Plane(ConstructionPlaneKey),
    /// Up to the extended plane of a 3D body face — another (or the same) extrusion's cap
    /// or side wall (#126), not a flat sketch profile. Always `FaceId::ExtrudeCap` or
    /// `FaceId::ExtrudeSide`; other `FaceId` kinds don't reach this variant (they already
    /// have their own — `Face`/`Plane` above).
    BodyFace(FaceId),
    /// Up to a **repeated instance's** face (#452): the source extrusion face's plane
    /// translated along the repeat axis by instance `instance`'s offset. Parametric — the
    /// snap follows when the repeat's spacing or the source body changes.
    RepeatedFace {
        face: FaceId,
        op: RepeatOpKey,
        instance: usize,
    },
}

/// How the extrude taper amount is measured (#1243): a length added per side of the end
/// face, or a draft angle of each side wall against the extrusion normal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeTaperMode {
    /// End face grows/shrinks by this many millimetres along each dimension (per side).
    #[default]
    Distance,
    /// Side wall angle from the normal, in degrees; [−90°, 89°]. Positive flares out.
    Angle,
}

impl ExtrudeTaperMode {
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "distance" | "length" => Some(Self::Distance),
            "angle" | "deg" | "degrees" => Some(Self::Angle),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Distance => "distance",
            Self::Angle => "angle",
        }
    }
}

/// An extrusion of one or more coplanar sketch faces into a 3D solid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extrusion {
    /// The sketch whose plane the faces lie on (gives the extrusion normal).
    pub sketch: SketchId,
    /// Faces included in this extrusion (toggled on/off while editing).
    pub faces: Vec<ExtrudeFace>,
    /// Signed extrusion distance along the plane normal (mm); negative goes the other way.
    /// When `target` is set this is the cached/last value; the effective distance is derived.
    /// When [`Self::symmetric`] is true, this is the *total* height (half each side).
    pub distance: f32,
    /// When set, the depth is constrained to reach this object's extended plane.
    #[serde(default)]
    pub target: Option<ExtrudeTarget>,
    /// Optional expression driving `distance` (empty = free/gizmo-driven, no constraint).
    #[serde(default)]
    pub expression: String,
    /// Extrude half the distance to each side of the sketch plane (#504). Ignored when
    /// `target` is set (depth is plane-to-plane).
    #[serde(default)]
    pub symmetric: bool,
    /// End-face size change vs the start face (#1243). Units depend on [`Self::taper_mode`]:
    /// length (mm, per side) or angle (degrees). Zero = prism (no taper).
    #[serde(default)]
    pub taper: f32,
    /// Whether [`Self::taper`] is a length or a draft angle (#1243).
    #[serde(default)]
    pub taper_mode: ExtrudeTaperMode,
    /// Optional expression driving `taper` (empty = free number).
    #[serde(default)]
    pub taper_expression: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Parametric 3D edge chamfer/fillet bevels applied to this extrusion's own analytic
    /// side/cap edges (#77) — see [`EdgeTreatment`].
    #[serde(default)]
    pub edge_treatments: Vec<EdgeTreatment>,
}

/// An extrusion's identity (#1055): stable across deletions of other extrusions.
pub type ExtrusionKey = crate::arena::Key<Extrusion>;

/// The feature(s) that produced a solid body.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodySource {
    Extrusion(ExtrusionKey),
    Extrusions(Vec<ExtrusionKey>),
    /// A mesh body brought in via STL import (#70); keys `Document::imported_meshes`
    /// rather than depending on a sketch-based feature.
    Imported(ImportedMeshKey),
    /// A lofted solid; indexes `Document::lofts`.
    Loft(LoftKey),
    /// A revolved solid (#revolve); keys `Document::revolutions`.
    Revolve(RevolutionKey),
    /// A primitive solid (#909); keys `Document::primitives`.
    Primitive(PrimitiveKey),
    /// A swept solid (the Sweep tool, #sweep); keys `Document::sweeps`.
    Sweep(SweepKey),
    /// One repeated instance of one input of a linear repeat (Repeat tool): `op` indexes
    /// `Document::repeat_ops`; `target` is the input's position in the op's target list;
    /// `instance` counts from 1 (the original body is instance 0).
    Repeated {
        #[serde(rename = "repeat_op")]
        op: RepeatOpKey,
        #[serde(default)]
        target: usize,
        #[serde(default)]
        instance: usize,
        /// Extrusions fused onto this instance after the repeat (#1345).
        #[serde(default)]
        add: Vec<ExtrusionKey>,
        /// Extrusions cut from this instance after the repeat (#1345).
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// The moved copy of one input of a move operation (Move tool): `op` indexes
    /// `Document::move_ops`, `target` is the position within that operation's input list.
    Moved {
        #[serde(rename = "move_op")]
        op: MoveOpKey,
        #[serde(default)]
        target: usize,
        /// Extrusions fused onto the moved solid after the move (#1345).
        #[serde(default)]
        add: Vec<ExtrusionKey>,
        /// Extrusions cut from the moved solid after the move (#1345).
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// The reflected copy of one input of a mirror operation (Mirror tool, #523): `op`
    /// indexes `Document::mirror_ops`, `target` is the input's position within that
    /// operation's target list. Unlike Move, the original input body is **kept** (not
    /// shadowed) — a mirror adds the reflection alongside the source.
    Mirrored {
        #[serde(rename = "mirror_op")]
        op: MirrorOpKey,
        #[serde(default)]
        target: usize,
        /// Extrusions fused onto the reflection after the mirror (#1345).
        #[serde(default)]
        add: Vec<ExtrusionKey>,
        /// Extrusions cut from the reflection after the mirror (#1345).
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// One output solid of a boolean operation (Combine tool): `op` indexes
    /// `Document::boolean_ops`, `solid` is the ordinal of this body's solid within the
    /// operation's result (a cut or difference can split into several pieces). The last
    /// output body absorbs any extra solids a parametric rebuild produces, so the pane's
    /// element list stays stable while geometry changes.
    Boolean {
        op: BooleanOpKey,
        #[serde(default)]
        solid: usize,
        /// Extrusions fused onto the combined solid after the boolean (#1338).
        #[serde(default)]
        add: Vec<ExtrusionKey>,
        /// Extrusions cut from the combined solid after the boolean (#1338).
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// One piece of a slice operation (Slice tool, #181): `op` indexes
    /// `Document::slice_ops`, `target` is the sliced input body's position in the op's
    /// target list, and `piece` is the ordinal of this fragment within that target's cut
    /// result. The input body becomes a shadow body; each fragment is its own `Body`.
    Sliced {
        #[serde(rename = "slice_op")]
        op: SliceOpKey,
        #[serde(default)]
        target: usize,
        #[serde(default)]
        piece: usize,
        /// Extrusions fused onto this fragment after the slice (#1345).
        #[serde(default)]
        add: Vec<ExtrusionKey>,
        /// Extrusions cut from this fragment after the slice (#1345).
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// The hollowed output of one input of a shell operation (Shell tool, #1156): `op`
    /// indexes `Document::shell_ops`, `target` is the input's position in the op's target
    /// list. The input body becomes a shadow body; this output carries the hollow solid.
    /// Extrusions fused onto / cut from the hollow after shelling live in `add`/`cut`
    /// (#1168) — same role as [`BodySource::Solid`]'s lists on a primitive base.
    Shelled {
        #[serde(rename = "shell_op")]
        op: ShellOpKey,
        #[serde(default)]
        target: usize,
        /// Extrusions fused onto the hollow solid after shelling (#1168).
        #[serde(default)]
        add: Vec<ExtrusionKey>,
        /// Extrusions cut from the hollow solid after shelling (#1168).
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// The chamfered/filleted output of one input of an edge-treatment operation (#531): `op`
    /// indexes `Document::edge_treatment_ops`, `target` is the input's position within that
    /// operation's target list. The input body becomes a shadow body; this output carries the
    /// bevel.
    EdgeTreated {
        #[serde(rename = "edge_treatment_op")]
        op: EdgeTreatmentOpKey,
        #[serde(default)]
        target: usize,
        /// Extrusions fused onto the beveled solid after the treatment (#1345).
        #[serde(default)]
        add: Vec<ExtrusionKey>,
        /// Extrusions cut from the beveled solid after the treatment (#1345).
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// Additive extrusions with one or more extrusions **subtracted** (cut) from them (#35).
    /// Purely-additive bodies stay in the `Extrusion`/`Extrusions` forms; a body only takes
    /// this shape once it has a cut. `cut` is `#[serde(default)]` so any future add-only
    /// `Solid` serialization stays readable (existing saved files never carry a cut list —
    /// they load as `Extrusion`/`Extrusions` unchanged).
    ///
    /// `base` is an optional primitive solid the extrusions are fused onto (#1104): a Shape-
    /// tool body that an extrusion is "added to" lives here, so its solid is the primitive
    /// united with the `add` extrusions and minus the `cut` extrusions. `#[serde(default)]`
    /// keeps old files loading as the base-less `Solid` they were.
    Solid {
        #[serde(default)]
        base: Option<PrimitiveKey>,
        add: Vec<ExtrusionKey>,
        #[serde(default)]
        cut: Vec<ExtrusionKey>,
    },
    /// The materialized geometry of one imported-unit instance (#724): indexes
    /// `Document::unit_instances`. Derived data kept in sync by `units::sync_unit_bodies`
    /// (one live body per live instance); its mesh is the instance's evaluated, placed
    /// unit geometry. Having units be real bodies is what makes them snappable and
    /// referenceable exactly like the document's own geometry — Move's point pickers,
    /// body-edge dimensions (#647), face pickers, and export all just see a body. It has
    /// no Elements-pane row of its own: the instance row (#723) stands for it.
    UnitInstance(UnitInstanceKey),
    /// A unit instance's geometry with extrusions **cut** out of it in the importing
    /// document (#726): the read-only unit is the input; this body is the importing
    /// document's own result. The unit's materialized body shadows while consumed, the
    /// way a boolean input does — but is never mutated.
    UnitCut { instance: UnitInstanceKey, cut: Vec<ExtrusionKey> },
}

impl BodySource {
    pub fn single(extrusion: ExtrusionKey) -> Self {
        Self::Extrusion(extrusion)
    }

    /// The input body this operation output was produced from, if any.
    /// Move / mirror / repeat / slice / shell / fillet all name one; a boolean
    /// has several and is not a single predecessor.
    pub fn input_body(&self, doc: &Document) -> Option<BodyKey> {
        match self {
            Self::Moved { op, target, .. } => doc.move_ops.get(*op)?.targets.get(*target).copied(),
            Self::Mirrored { op, target, .. } => {
                doc.mirror_ops.get(*op)?.targets.get(*target).copied()
            }
            Self::Repeated { op, target, .. } => {
                doc.repeat_ops.get(*op)?.targets.get(*target).copied()
            }
            Self::Sliced { op, target, .. } => {
                doc.slice_ops.get(*op)?.targets.get(*target).copied()
            }
            Self::Shelled { op, target, .. } => {
                doc.shell_ops.get(*op)?.targets.get(*target).copied()
            }
            Self::EdgeTreated { op, target, .. } => {
                doc.edge_treatment_ops.get(*op)?.targets.get(*target).copied()
            }
            _ => None,
        }
    }

    /// Extrusions **added** to (fused into) the body.
    pub fn extrusion_indices(&self) -> &[ExtrusionKey] {
        match self {
            Self::Extrusion(index) => std::slice::from_ref(index),
            Self::Extrusions(indices) => indices.as_slice(),
            Self::Solid { add, .. }
            | Self::Shelled { add, .. }
            | Self::Boolean { add, .. }
            | Self::Moved { add, .. }
            | Self::Mirrored { add, .. }
            | Self::Repeated { add, .. }
            | Self::Sliced { add, .. }
            | Self::EdgeTreated { add, .. } => add.as_slice(),
            Self::Loft(_)
            | Self::Revolve(_)
            | Self::Primitive(_)
            | Self::Sweep(_)
            | Self::UnitInstance(_)
            | Self::UnitCut { .. } => &[],
            Self::Imported(_) => &[],
        }
    }

    /// Extrusions **subtracted** (cut) from the body (#35). Empty for every non-`Solid`
    /// form except a unit cut (#726), a shelled body with post-shell cuts (#1168), a
    /// combined body with post-boolean cuts (#1338), and the other op-produced sources
    /// that keep post-op add/cut lists (#1345).
    pub fn cut_extrusion_indices(&self) -> &[ExtrusionKey] {
        match self {
            Self::Solid { cut, .. }
            | Self::Shelled { cut, .. }
            | Self::Boolean { cut, .. }
            | Self::Moved { cut, .. }
            | Self::Mirrored { cut, .. }
            | Self::Repeated { cut, .. }
            | Self::Sliced { cut, .. }
            | Self::EdgeTreated { cut, .. } => cut.as_slice(),
            Self::UnitCut { cut, .. } => cut.as_slice(),
            Self::Extrusion(_)
            | Self::Extrusions(_)
            | Self::Imported(_)
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Primitive(_)
            | Self::Sweep(_)
            | Self::UnitInstance(_) => &[],
        }
    }

    pub fn imported_mesh_key(&self) -> Option<ImportedMeshKey> {
        match self {
            Self::Imported(key) => Some(*key),
            Self::Extrusion(_)
            | Self::Extrusions(_)
            | Self::Solid { .. }
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Primitive(_)
            | Self::Sweep(_)
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Mirrored { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. }
            | Self::Shelled { .. }
            | Self::EdgeTreated { .. }
            | Self::UnitInstance(_)
            | Self::UnitCut { .. } => None,
        }
    }

    /// Whether the body is built from `extrusion` in any role (added or cut).
    pub fn owns_extrusion(&self, extrusion: ExtrusionKey) -> bool {
        self.extrusion_indices().contains(&extrusion)
            || self.cut_extrusion_indices().contains(&extrusion)
    }

    pub fn append_extrusion(&mut self, extrusion: ExtrusionKey) {
        match self {
            Self::Extrusion(existing) => {
                *self = Self::Extrusions(vec![*existing, extrusion]);
            }
            Self::Extrusions(indices) => indices.push(extrusion),
            Self::Solid { add, .. }
            | Self::Shelled { add, .. }
            | Self::Boolean { add, .. }
            | Self::Moved { add, .. }
            | Self::Mirrored { add, .. }
            | Self::Repeated { add, .. }
            | Self::Sliced { add, .. }
            | Self::EdgeTreated { add, .. } => add.push(extrusion),
            // A primitive base takes its first added extrusion by becoming a `Solid` whose
            // base is that primitive (#1104); further adds push onto the list.
            Self::Primitive(pi) => {
                *self = Self::Solid {
                    base: Some(*pi),
                    add: vec![extrusion],
                    cut: Vec::new(),
                };
            }
            // An imported mesh body has no extrusion to merge into; unreachable in practice
            // since merge candidates only ever come from extrusion-backed bodies.
            Self::Imported(_)
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Sweep(_)
            | Self::UnitInstance(_)
            | Self::UnitCut { .. } => {}
        }
    }

    /// Register `extrusion` as a **cut** (subtraction) of this body (#35), moving the source
    /// into the `Solid` form if it wasn't already.
    pub fn append_cut_extrusion(&mut self, extrusion: ExtrusionKey) {
        match self {
            Self::Extrusion(existing) => {
                *self = Self::Solid {
                    base: None,
                    add: vec![*existing],
                    cut: vec![extrusion],
                };
            }
            Self::Extrusions(indices) => {
                *self = Self::Solid {
                    base: None,
                    add: std::mem::take(indices),
                    cut: vec![extrusion],
                };
            }
            Self::Solid { cut, .. }
            | Self::Shelled { cut, .. }
            | Self::Boolean { cut, .. }
            | Self::Moved { cut, .. }
            | Self::Mirrored { cut, .. }
            | Self::Repeated { cut, .. }
            | Self::Sliced { cut, .. }
            | Self::EdgeTreated { cut, .. } => cut.push(extrusion),
            // A primitive base takes its first cut by becoming a `Solid` whose base is that
            // primitive (#1104); the cut list starts with this extrusion.
            Self::Primitive(pi) => {
                *self = Self::Solid {
                    base: Some(*pi),
                    add: Vec::new(),
                    cut: vec![extrusion],
                };
            }
            // A unit-cut body takes further cuts (#726).
            Self::UnitCut { cut, .. } => cut.push(extrusion),
            // An imported mesh body has no solid feature to cut; unreachable in practice.
            Self::Imported(_) | Self::Loft(_) | Self::Revolve(_) | Self::Sweep(_) | Self::UnitInstance(_) => {
            }
        }
    }

    /// Remove `extrusion` from this source in whatever role it plays (e.g. undoing a merge or
    /// a cut). Collapses back to the simplest form once the cut list is empty (and to the
    /// single-extrusion form when one added index remains). No-op if `extrusion` isn't owned.
    /// Undo never removes a body's last/only *added* extrusion this way — that path deletes
    /// the whole body instead.
    pub fn remove_extrusion(&mut self, extrusion: ExtrusionKey) {
        match self {
            Self::Extrusions(indices) => {
                indices.retain(|&ei| ei != extrusion);
                if let [only] = indices.as_slice() {
                    *self = Self::Extrusion(*only);
                }
            }
            Self::Solid { base, add, cut } => {
                add.retain(|&ei| ei != extrusion);
                cut.retain(|&ei| ei != extrusion);
                if cut.is_empty() {
                    if let Some(p) = *base {
                        // No remaining extrusions: the body is the shape again (#1104).
                        if add.is_empty() {
                            *self = Self::Primitive(p);
                        }
                        // Otherwise stay `Solid { base, add, cut: [] }` — the primitive
                        // with its fused additions.
                    } else {
                        *self = match add.as_slice() {
                            [only] => Self::Extrusion(*only),
                            _ => Self::Extrusions(std::mem::take(add)),
                        };
                    }
                }
            }
            // Shelled keeps its form; empty add/cut is the pure hollow (#1168).
            Self::Shelled { add, cut, .. } => {
                add.retain(|&ei| ei != extrusion);
                cut.retain(|&ei| ei != extrusion);
            }
            // Combined body keeps its form; empty add/cut is the pure boolean (#1338).
            Self::Boolean { add, cut, .. } => {
                add.retain(|&ei| ei != extrusion);
                cut.retain(|&ei| ei != extrusion);
            }
            // Other op-produced bodies keep their form; empty add/cut is the pure op (#1345).
            Self::Moved { add, cut, .. }
            | Self::Mirrored { add, cut, .. }
            | Self::Repeated { add, cut, .. }
            | Self::Sliced { add, cut, .. }
            | Self::EdgeTreated { add, cut, .. } => {
                add.retain(|&ei| ei != extrusion);
                cut.retain(|&ei| ei != extrusion);
            }
            // A unit cut keeps its form with an empty list (#726): it then reads as the
            // intact unit; the sync pass re-shadows accordingly.
            Self::UnitCut { cut, .. } => {
                cut.retain(|&ei| ei != extrusion);
            }
            Self::Extrusion(_)
            | Self::Imported(_)
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Primitive(_)
            | Self::Sweep(_)
            | Self::UnitInstance(_) => {}
        }
    }

    /// The primitive this body is built on, if any (#1104): either a pure shape body or a
    /// `Solid` whose `base` is that shape (after an add-to-body / cut into a Shape-tool solid).
    pub fn primitive_base(&self) -> Option<PrimitiveKey> {
        match self {
            Self::Primitive(p) => Some(*p),
            Self::Solid { base: Some(p), .. } => Some(*p),
            _ => None,
        }
    }

    /// The extrusion that *produced* this body as its fused output (#1106/#1107): the last
    /// added extrusion, or — when the body is cuts-only on a primitive base — the last cut.
    /// Used so the combined body nests under that extrusion in the graph, not under the Shape.
    pub fn producing_extrusion(&self) -> Option<ExtrusionKey> {
        match self {
            Self::Extrusion(e) => Some(*e),
            Self::Extrusions(indices) => indices.last().copied(),
            Self::Solid { add, cut, .. } | Self::Shelled { add, cut, .. } => {
                add.last().copied().or_else(|| cut.last().copied())
            }
            // In-place cuts stay under the op; only a fused add re-parents (#1338/#1345).
            Self::Boolean { add, .. }
            | Self::Moved { add, .. }
            | Self::Mirrored { add, .. }
            | Self::Repeated { add, .. }
            | Self::Sliced { add, .. }
            | Self::EdgeTreated { add, .. } => add.last().copied(),
            _ => None,
        }
    }

    /// Source that would remain if `extrusion` were peeled off as the body's producing
    /// extrusion (#1106). `None` if `extrusion` is not this body's producer, or if peeling
    /// would leave nothing (a lone new-body extrusion).
    pub fn predecessor_source(&self, extrusion: ExtrusionKey) -> Option<BodySource> {
        if self.producing_extrusion() != Some(extrusion) {
            return None;
        }
        match self {
            Self::Extrusion(_) => None,
            Self::Extrusions(indices) => {
                let mut left: Vec<ExtrusionKey> =
                    indices.iter().copied().filter(|&e| e != extrusion).collect();
                match left.len() {
                    0 => None,
                    1 => Some(Self::Extrusion(left.pop().unwrap())),
                    _ => Some(Self::Extrusions(left)),
                }
            }
            Self::Solid { base, add, cut } => {
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                let cut: Vec<ExtrusionKey> =
                    cut.iter().copied().filter(|&e| e != extrusion).collect();
                // Producer was an add (preferred) or a cut when add was empty.
                if add.is_empty() && cut.is_empty() {
                    return base.map(Self::Primitive);
                }
                if cut.is_empty() {
                    if let Some(p) = *base {
                        return Some(Self::Solid {
                            base: Some(p),
                            add,
                            cut: Vec::new(),
                        });
                    }
                    return match add.as_slice() {
                        [only] => Some(Self::Extrusion(*only)),
                        _ => Some(Self::Extrusions(add)),
                    };
                }
                Some(Self::Solid {
                    base: *base,
                    add,
                    cut,
                })
            }
            // Peel a fused add; the boolean output remains when add/cut are empty (#1338).
            Self::Boolean { op, solid, add, cut } => {
                if add.last().copied() != Some(extrusion) {
                    return None;
                }
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                Some(Self::Boolean {
                    op: *op,
                    solid: *solid,
                    add,
                    cut: cut.clone(),
                })
            }
            // Peel the last fused extrusion; pure hollow when add and cut are empty (#1168).
            Self::Shelled { op, target, add, cut } => {
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                let cut: Vec<ExtrusionKey> =
                    cut.iter().copied().filter(|&e| e != extrusion).collect();
                Some(Self::Shelled {
                    op: *op,
                    target: *target,
                    add,
                    cut,
                })
            }
            // Peel a fused add; the op output remains when add/cut are empty (#1345).
            Self::Moved { op, target, add, cut } => {
                if add.last().copied() != Some(extrusion) {
                    return None;
                }
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                Some(Self::Moved {
                    op: *op,
                    target: *target,
                    add,
                    cut: cut.clone(),
                })
            }
            Self::Mirrored { op, target, add, cut } => {
                if add.last().copied() != Some(extrusion) {
                    return None;
                }
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                Some(Self::Mirrored {
                    op: *op,
                    target: *target,
                    add,
                    cut: cut.clone(),
                })
            }
            Self::Repeated {
                op,
                target,
                instance,
                add,
                cut,
            } => {
                if add.last().copied() != Some(extrusion) {
                    return None;
                }
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                Some(Self::Repeated {
                    op: *op,
                    target: *target,
                    instance: *instance,
                    add,
                    cut: cut.clone(),
                })
            }
            Self::Sliced {
                op,
                target,
                piece,
                add,
                cut,
            } => {
                if add.last().copied() != Some(extrusion) {
                    return None;
                }
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                Some(Self::Sliced {
                    op: *op,
                    target: *target,
                    piece: *piece,
                    add,
                    cut: cut.clone(),
                })
            }
            Self::EdgeTreated { op, target, add, cut } => {
                if add.last().copied() != Some(extrusion) {
                    return None;
                }
                let add: Vec<ExtrusionKey> =
                    add.iter().copied().filter(|&e| e != extrusion).collect();
                Some(Self::EdgeTreated {
                    op: *op,
                    target: *target,
                    add,
                    cut: cut.clone(),
                })
            }
            _ => None,
        }
    }
}

/// Whether any live operation (boolean or move) other than the excluded ones consumes
/// `body` on a side that shadows it — used when deleting/editing an operation to decide
/// whether an input body stays a shadow. Also true when a fuse-merge/cut extrusion still
/// consumes the body as its host (#1106).
pub fn body_shadowed_by_other_ops(
    doc: &Document,
    body: BodyKey,
    skip_boolean: Option<crate::model::BooleanOpKey>,
    skip_move: Option<MoveOpKey>,
    skip_slice: Option<SliceOpKey>,
    skip_edge_treatment: Option<EdgeTreatmentOpKey>,
) -> bool {
    body_shadowed_by_other_ops_ex(
        doc,
        body,
        skip_boolean,
        skip_move,
        skip_slice,
        skip_edge_treatment,
        None,
    )
}

/// Like [`body_shadowed_by_other_ops`], also skipping a shell op being edited (#1156).
pub fn body_shadowed_by_other_ops_ex(
    doc: &Document,
    body: BodyKey,
    skip_boolean: Option<crate::model::BooleanOpKey>,
    skip_move: Option<MoveOpKey>,
    skip_slice: Option<SliceOpKey>,
    skip_edge_treatment: Option<EdgeTreatmentOpKey>,
    skip_shell: Option<ShellOpKey>,
) -> bool {
    doc.boolean_ops.iter().any(|(oi, o)| {
        skip_boolean != Some(oi) && (o.a.contains(&body) || (!o.keep_b && o.b.contains(&body)))
    }) || doc.move_ops.iter().any(|(oi, o)| {
        // Paste Linked (#1236) keeps its inputs live — it does not consume them.
        skip_move != Some(oi) && !o.keep_inputs && o.targets.contains(&body)
    }) || doc.slice_ops.iter().any(|(oi, o)| {
        skip_slice != Some(oi) && o.targets.contains(&body)
    }) || doc.edge_treatment_ops.iter().any(|(oi, o)| {
        skip_edge_treatment != Some(oi) && o.targets.contains(&body)
    }) || doc.shell_ops.iter().any(|(oi, o)| {
        skip_shell != Some(oi) && o.targets.contains(&body)
    }) || body_is_fuse_host(doc, body)
}

/// The body whose source includes `extrusion` (added or cut), if any.
/// Prefers a live (non-shadow) body when several own the same extrusion after fuse-merge
/// (#1106/#1107) — intermediate shadow solids keep older add lists that still name it.
fn source_derives_from_extrusion(
    doc: &Document,
    source: &BodySource,
    extrusion: ExtrusionKey,
) -> bool {
    source.owns_extrusion(extrusion)
        || source
            .input_body(doc)
            .and_then(|bi| doc.bodies.get(bi))
            .is_some_and(|b| source_derives_from_extrusion(doc, &b.source, extrusion))
}

/// A live body that *is* this add extrusion alone — not a cut, not an add fused onto
/// other geometry. Repeating that extrusion copies the body (#1475) instead of fusing
/// disjoint instances into one compound.
pub fn standalone_body_for_add_extrusion(doc: &Document, extrusion: ExtrusionKey) -> Option<BodyKey> {
    if doc
        .bodies
        .values()
        .any(|b| b.source.cut_extrusion_indices().contains(&extrusion))
    {
        return None;
    }
    let mut found = None;
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        if body.source.extrusion_indices() != [extrusion] {
            continue;
        }
        if body.source.primitive_base().is_some() {
            return None;
        }
        if found.is_some() {
            return None;
        }
        found = Some(bi);
    }
    found
}

pub fn body_index_for_extrusion(doc: &Document, extrusion: ExtrusionKey) -> Option<BodyKey> {
    let matches: Vec<(BodyKey, bool)> = doc
        .bodies
        .iter()
        .filter(|(_, b)| source_derives_from_extrusion(doc, &b.source, extrusion))
        .map(|(k, b)| (k, b.shadow))
        .collect();
    let live: Vec<BodyKey> = matches
        .iter()
        .filter(|(_, shadow)| !shadow)
        .map(|(k, _)| *k)
        .collect();
    if let Some(&direct) = live.iter().find(|&&k| {
        doc.bodies
            .get(k)
            .is_some_and(|b| b.source.owns_extrusion(extrusion))
    }) {
        return Some(direct);
    }
    if let Some(&k) = live.first() {
        return Some(k);
    }
    matches.first().map(|(k, _)| *k)
}

/// The body that was shadowed when `body` was created by a merge/cut extrude (#1106/#1107),
/// if any: a shadow body whose source matches this body's source with its producing
/// extrusion peeled off.
pub fn fuse_host_of(doc: &Document, body: BodyKey) -> Option<BodyKey> {
    let src = &doc.bodies.get(body)?.source;
    let producing = src.producing_extrusion()?;
    let pred = src.predecessor_source(producing)?;
    doc.bodies.iter().find_map(|(k, b)| {
        (k != body && b.source == pred).then_some(k)
    })
}

/// Whether any live body is a fuse-merge/cut result of `host` (#1106).
pub fn body_is_fuse_host(doc: &Document, host: BodyKey) -> bool {
    doc.bodies
        .keys()
        .any(|k| k != host && fuse_host_of(doc, k) == Some(host))
}

/// The body a face belongs to (#926), when it has one: a cap/side wall belongs to its
/// extrusion's body, a revolve's flat side to the revolution's. Sketch profiles and
/// construction planes belong to no body.
pub fn body_index_for_face(doc: &Document, face: &FaceId) -> Option<BodyKey> {
    match face {
        FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
            body_index_for_extrusion(doc, *extrusion)
        }
        FaceId::RevolveCap { revolution, .. } | FaceId::RevolveSide { revolution, .. } => {
            body_index_for_revolution(doc, *revolution)
        }
        FaceId::UnitFace { .. } | FaceId::ConstructionPlane(_) | FaceId::Circle(_)
        | FaceId::Polygon(_) => None,
        FaceId::PrimitiveFace { primitive, .. } => body_index_for_primitive(doc, *primitive),
        // A repeated face lives on the repeat output body for that instance (#1116).
        FaceId::RepeatedFace { op, instance, .. } => {
            let rep = doc.repeat_ops.get(*op)?;
            // outputs[0] is instance 1, etc. (instance 0 is the source target body).
            let idx = instance.checked_sub(1)?;
            rep.outputs.get(idx).copied()
        }
        FaceId::BodyMeshFace { body, .. } => Some(*body),
    }
}

/// Whether `face` is a face of **this specific** body's geometry (#1172).
///
/// Unlike [`body_index_for_face`], this does **not** prefer a live shell/fuse output over a
/// shadowed input. Shell open-face matching must ask "does this face belong to target body
/// B?" — after a shell commits, `body_index_for_face` returns the shelled output for
/// primitive faces (live preference), which would drop every open face on the input.
pub fn face_belongs_to_body(doc: &Document, face: &FaceId, body: BodyKey) -> bool {
    let Some(b) = doc.bodies.get(body) else {
        return false;
    };
    match face {
        FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
            b.source.owns_extrusion(*extrusion)
                || b.source
                    .input_body(doc)
                    .is_some_and(|input| face_belongs_to_body(doc, face, input))
        }
        FaceId::RevolveCap { revolution, .. } | FaceId::RevolveSide { revolution, .. } => {
            matches!(b.source, BodySource::Revolve(r) if r == *revolution)
        }
        FaceId::PrimitiveFace { primitive, .. } => match &b.source {
            // Pure primitive or solid-with-that-base: the face lives here.
            BodySource::Primitive(p) => *p == *primitive,
            BodySource::Solid {
                base: Some(p), ..
            } => *p == *primitive,
            // Move / shell / fillet / … : primitive faces still name the original
            // shape — walk the input chain. A body is a body (#1463).
            _ => b
                .source
                .input_body(doc)
                .is_some_and(|input| face_belongs_to_body(doc, face, input)),
        },
        FaceId::RepeatedFace { .. } => body_index_for_face(doc, face) == Some(body),
        FaceId::BodyMeshFace { body: b, .. } => *b == body,
        FaceId::UnitFace { .. }
        | FaceId::ConstructionPlane(_)
        | FaceId::Circle(_)
        | FaceId::Polygon(_) => false,
    }
}

/// Whether this body source ultimately derives from Shape-tool primitive `primitive`
/// (#1168/#1463): pure primitive, solid-with-that-base, or any operation output
/// (move, shell, fillet, …) of such. A body is a body.
fn source_derives_from_primitive(
    doc: &Document,
    source: &BodySource,
    primitive: PrimitiveKey,
) -> bool {
    match source {
        BodySource::Primitive(p) => *p == primitive,
        BodySource::Solid {
            base: Some(p), ..
        } => *p == primitive,
        _ => source
            .input_body(doc)
            .and_then(|bi| doc.bodies.get(bi))
            .is_some_and(|b| source_derives_from_primitive(doc, &b.source, primitive)),
    }
}

/// The body built on primitive shape `primitive` (#909/#1104): a pure
/// [`BodySource::Primitive`] body, a [`BodySource::Solid`] whose `base` is that shape,
/// or a live operation output of either (shell #1168, move #1463, fillet, …).
/// Prefers a live body when fuse-merge / shell / move left the pure primitive as a
/// shadow; among several live matches, the original primitive/solid itself (a
/// mirror keeps the source live and adds a copy), else the fuse-chain leaf.
pub fn body_index_for_primitive(doc: &Document, primitive: PrimitiveKey) -> Option<BodyKey> {
    let matches: Vec<(BodyKey, bool)> = doc
        .bodies
        .iter()
        .filter(|(_, b)| source_derives_from_primitive(doc, &b.source, primitive))
        .map(|(k, b)| (k, b.shadow))
        .collect();
    let live: Vec<BodyKey> = matches
        .iter()
        .filter(|(_, shadow)| !shadow)
        .map(|(k, _)| *k)
        .collect();
    if let Some(&direct) = live.iter().find(|&&k| {
        match doc.bodies.get(k).map(|b| &b.source) {
            Some(BodySource::Primitive(p)) => *p == primitive,
            Some(BodySource::Solid {
                base: Some(p), ..
            }) => *p == primitive,
            _ => false,
        }
    }) {
        return Some(direct);
    }
    // Prefer the fuse-chain leaf: a live body that is not the fuse host of another match.
    if let Some(&leaf) = live.iter().find(|&&k| {
        !live
            .iter()
            .any(|&other| other != k && fuse_host_of(doc, other) == Some(k))
    }) {
        return Some(leaf);
    }
    if let Some(&k) = live.first() {
        return Some(k);
    }
    matches.first().map(|(k, _)| *k)
}

/// Body index whose source is `revolution` (#621) — the revolve analogue of
/// [`body_index_for_extrusion`].
pub fn body_index_for_revolution(doc: &Document, revolution: RevolutionKey) -> Option<BodyKey> {
    doc.bodies.iter().find_map(|(key, body)| {
        matches!(body.source, BodySource::Revolve(r) if r == revolution).then_some(key)
    })
}

/// A solid body produced by a feature; it depends on its source feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub source: BodySource,
    #[serde(default)]
    pub name: Option<String>,
    /// The material this body is made of (#834), a key into [`Document::materials`]. `None`
    /// is the document's default material — the look every body has always had.
    #[serde(default)]
    pub material: Option<MaterialKey>,
    /// A shadow body — a consumed operation input, or any body the user has marked as
    /// shadow (#1218). Still listed in the Elements pane (dimmed, its own icon) but
    /// hidden in the viewport except while hovered or selected there (renders ghosted),
    /// and omitted from whole-document / component export.
    #[serde(default)]
    pub shadow: bool,
}

/// How everything names a body (#1055): every operation's inputs and outputs, every
/// mesh-keyed sub-element (edge, vertex, face, cylinder, axis), every joint member, and
/// every pick. Deleting a body used to renumber all of them.
pub type BodyKey = crate::arena::Key<Body>;

/// How a body names its material (#1055): a key, so removing a material never renames
/// another one's colour onto a body that was never made of it.
pub type MaterialKey = crate::arena::Key<Material>;

/// A material a body can be made of (#834): a name and the colour it renders in. Documents
/// start with none — a body with no material renders in the default body colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// Rendered colour, sRGB.
    pub color: [u8; 3],
}

impl Material {
    /// The materials every document starts with (#925/#927/#928): the whole palette is in
    /// the picker from the first frame, so choosing what a body is made of never means
    /// making a material first.
    ///
    /// **Unobtainium** comes first and is what a new body is made of — its colour is the
    /// grey-blue every body rendered in before materials existed, so nothing looks
    /// different until you pick something else. The rest walk hues that **contrast with
    /// their neighbours** (#927): blue, green, red, yellow, purple, orange, cyan, pink,
    /// grey — so two materials made one after the other never look alike. Every entry is
    /// light enough (Rec. 709 Y > 0.35) that a shaded solid still reads as its own colour
    /// where the lighting falls away.
    pub const DEFAULTS: [(&'static str, [u8; 3]); 10] = [
        ("Unobtainium", [150, 168, 196]),
        ("Blue", [0x3d, 0x8e, 0xf0]),
        ("Green", [0x57, 0xc4, 0x6a]),
        ("Red", [0xe8, 0x61, 0x5c]),
        ("Yellow", [0xe8, 0xc9, 0x4a]),
        ("Purple", [0xa9, 0x7f, 0xe0]),
        ("Orange", [0xef, 0x94, 0x40]),
        ("Cyan", [0x4f, 0xd0, 0xd6]),
        ("Pink", [0xf0, 0x7a, 0xc0]),
        ("Grey", [0xc9, 0xce, 0xd8]),
    ];

    /// The colours a **new** material walks through, so consecutive ones look different
    /// without anyone picking colours (#834/#927): the defaults' palette, minus
    /// Unobtainium (which is the starting material, not a choice in the rotation).
    pub const NEW_COLORS: [[u8; 3]; 9] = [
        Self::DEFAULTS[1].1,
        Self::DEFAULTS[2].1,
        Self::DEFAULTS[3].1,
        Self::DEFAULTS[4].1,
        Self::DEFAULTS[5].1,
        Self::DEFAULTS[6].1,
        Self::DEFAULTS[7].1,
        Self::DEFAULTS[8].1,
        Self::DEFAULTS[9].1,
    ];

    /// The materials a fresh document is seeded with (#928).
    pub fn defaults() -> crate::arena::Arena<Material> {
        Self::DEFAULTS
            .iter()
            .map(|(name, color)| Material { name: (*name).to_string(), color: *color })
            .collect()
    }
}

/// The key a body gets from slot `n` alone (#1055) — **tests only**. A body inserted into a
/// document that has never had one removed lands in slot `n` at generation 0, so this is that
/// body's real key. It is not a substitute for asking the document: after any removal the slot
/// is reused at a higher generation and this stops matching, which is the point of the type.
#[cfg(test)]
pub fn body_key_for_slot(n: usize) -> BodyKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a boolean operation (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn boolean_op_key_for_slot(n: usize) -> BooleanOpKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a move operation (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn move_op_key_for_slot(n: usize) -> MoveOpKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a mirror operation (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn mirror_op_key_for_slot(n: usize) -> MirrorOpKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a linear repeat (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn repeat_op_key_for_slot(n: usize) -> RepeatOpKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a slice operation (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn slice_op_key_for_slot(n: usize) -> SliceOpKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a shell operation (#1156) — tests only, same caveat.
#[cfg(test)]
pub fn shell_op_key_for_slot(n: usize) -> ShellOpKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for an edge-treatment operation (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn edge_treatment_op_key_for_slot(n: usize) -> EdgeTreatmentOpKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a joint (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn joint_key_for_slot(n: usize) -> JointKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// Drop every datum plane but the XY ground (#1055) — tests only. Several tests index
/// planes by hand and only ever want the one they sketch on.
#[cfg(test)]
pub fn retain_ground_plane_only(doc: &mut Document) {
    // Built fresh rather than removed one by one, so the freed slots keep generation 0 and
    // the next plane a test adds lands at the slot its assertions expect.
    let Some(ground) = doc.ground_plane().and_then(|k| doc.construction_planes.get(k).cloned())
    else {
        return;
    };
    doc.construction_planes = crate::arena::Arena::new();
    doc.construction_planes.insert(ground);
}

/// The same for a line (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn line_key_for_slot(n: usize) -> LineKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a construction plane (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn plane_key_for_slot(n: usize) -> ConstructionPlaneKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a circle (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn circle_key_for_slot(n: usize) -> CircleKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a sketch (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn sketch_key_for_slot(n: usize) -> SketchId {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a constraint (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn constraint_key_for_slot(n: usize) -> ConstraintKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a sketch text (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn sketch_text_key_for_slot(n: usize) -> SketchTextKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for an extrusion (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn extrusion_key_for_slot(n: usize) -> ExtrusionKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a Shape-tool primitive (#1329) — tests only, same caveat.
#[cfg(test)]
pub fn primitive_key_for_slot(n: usize) -> PrimitiveKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for an imported unit (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn unit_key_for_slot(n: usize) -> UnitKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a drawing (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn drawing_key_for_slot(n: usize) -> DrawingKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a drawing annotation (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn annotation_key_for_slot(n: usize) -> AnnotationKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a component (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn component_key_for_slot(n: usize) -> ComponentKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for a unit instance (#1055) — tests only, same caveat.
#[cfg(test)]
pub fn unit_instance_key_for_slot(n: usize) -> UnitInstanceKey {
    crate::arena::Key::from_bits((n as u64) << 32)
}

/// The same for any in-sketch operation (#1055) — tests only, same caveat. Generic because
/// the five sketch ops all take the same shape.
#[cfg(test)]
pub fn sketch_op_key_for_slot<T>(n: usize) -> crate::arena::Key<T> {
    crate::arena::Key::from_bits((n as u64) << 32)
}

impl Document {
    /// The XY ground plane (#1055): every document opens with it as its first datum plane,
    /// and it is what a script or a default means by "plane 0".
    pub fn ground_plane(&self) -> Option<ConstructionPlaneKey> {
        self.construction_planes.keys().next()
    }

    /// The body at ordinal `n` among the live ones (#1055) — what a script names, and how
    /// to say "the first body" now that a position is not an identity.
    pub fn body_at(&self, n: usize) -> Option<BodyKey> {
        self.bodies.keys().nth(n)
    }

    /// What a body with no material of its own is made of (#924): the document's first
    /// material, which a fresh document seeds as **Unobtainium**. Older files (and any body
    /// whose material was cleared) fall back to it rather than to a colour with no entry
    /// behind it. `None` only when a document has no materials at all.
    pub fn default_material(&self) -> Option<MaterialKey> {
        self.materials.keys().next()
    }
}

#[cfg(test)]
mod material_tests {
    use super::Material;

    /// #925/#927/#928: the seeded palette leads with Unobtainium — the colour every body
    /// rendered in before materials existed — and then walks contrasting hues, so two
    /// materials made one after the other never look alike.
    #[test]
    fn default_materials_lead_with_unobtainium_and_contrast() {
        let arena = Material::defaults();
        assert_eq!(arena.len(), Material::DEFAULTS.len());
        let defaults: Vec<&Material> = arena.values().collect();
        assert_eq!(defaults[0].name, "Unobtainium");
        assert_eq!(defaults[0].color, [150, 168, 196], "the old default body colour");
        assert_eq!(
            defaults.iter().skip(1).map(|m| m.name.as_str()).collect::<Vec<_>>(),
            vec!["Blue", "Green", "Red", "Yellow", "Purple", "Orange", "Cyan", "Pink", "Grey"]
        );
        // Neighbours differ strongly: at least 120 of summed channel distance apart.
        for pair in defaults.windows(2) {
            let d: i32 = (0..3)
                .map(|c| (i32::from(pair[0].color[c]) - i32::from(pair[1].color[c])).abs())
                .sum();
            assert!(
                d >= 120,
                "{} and {} are too close ({d})",
                pair[0].name,
                pair[1].name
            );
        }
        // The rotation a new material walks is the palette minus Unobtainium.
        assert_eq!(Material::NEW_COLORS[0], defaults[1].color);
    }

    /// Every entry is light enough to read as its own color on the dark viewport once a
    /// solid is shaded — the reason this scheme was picked over a data-viz palette meant
    /// for thin marks on white.
    #[test]
    fn new_material_colors_are_all_light() {
        for color in Material::NEW_COLORS {
            // Rec. 709 relative luminance, on the sRGB values as authored.
            let [r, g, b] = color.map(|c| f32::from(c) / 255.0);
            let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            assert!(y > 0.35, "{color:?} is too dark for a shaded body (Y = {y:.2})");
        }
    }

    /// Consecutive materials never repeat a color until the palette runs out.
    #[test]
    fn new_material_colors_are_distinct() {
        let mut seen = Vec::new();
        for color in Material::NEW_COLORS {
            assert!(!seen.contains(&color), "{color:?} appears twice");
            seen.push(color);
        }
    }
}

/// Stable handle to a [`Loft`] (#1055). Replaces the positional index, so removing one loft
/// cannot renumber another.
pub type LoftKey = crate::arena::Key<Loft>;

/// A loft: a solid blended through two or more cross-section profiles on (usually)
/// different planes. Parametric like everything else — the mesh is rebuilt from the live
/// section profiles on every geometry recompute, so editing a section reshapes the loft.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Loft {
    /// The cross sections, in blend order (sorted along the loft's principal direction at
    /// commit time). Each names a closed profile the same way `Extrusion::faces` does.
    pub sections: Vec<LoftSection>,
    /// How the solid lands (#479): its own body (the default — pre-#479 files load as
    /// this), fused into existing bodies, or subtracted from them.
    #[serde(default)]
    pub mode: LoftMode,
    #[serde(default)]
    pub name: Option<String>,
}

/// How a lofted solid lands in the document (#479), mirroring [`SweepMode`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoftMode {
    #[default]
    NewBody,
    AddTo(Vec<BodyKey>),
    Cut(Vec<BodyKey>),
}

/// One loft cross section: a closed profile (`ExtrudeFace`) plus the sketch it lives in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoftSection {
    pub sketch: SketchId,
    pub face: ExtrudeFace,
}

/// A straight reference axis: a line in a sketch (plain, construction, or projected — any
/// line works), a **feature edge of a solid body** (#643), or one of the origin's global axes.
/// Used as a [`Revolution`]'s sweep axis, a move's rotation axis, and a [`RepeatOperation`]'s
/// direction.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevolveAxis {
    Line(LineKey),
    /// One feature edge of a body's solid mesh, by its world-space endpoints (#643) — the same
    /// identity [`crate::construction::PickTargetKind::BodyEdge`] carries. Only the direction
    /// `a → b` matters to a linear repeat; a revolve/rotation also uses `a` as the pivot.
    BodyEdge {
        body: BodyKey,
        a: glam::Vec3,
        b: glam::Vec3,
    },
    X,
    Y,
    Z,
}

/// How a revolved solid lands in the document (#revolve): its own body, fused into
/// existing bodies, or subtracted from existing bodies (the cut list is user-picked).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevolveMode {
    NewBody,
    AddTo(Vec<BodyKey>),
    Cut(Vec<BodyKey>),
}

/// Which primitive a [`Primitive`] is (#909): the shapes the Create Shape tool places
/// directly in 3D, without a sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimitiveKind {
    Cuboid,
    Cylinder,
    Sphere,
}

impl PrimitiveKind {
    /// Every shape, in the order the tool cycles them (#909).
    #[allow(dead_code)]
    pub const ALL: [PrimitiveKind; 3] = [
        PrimitiveKind::Cuboid,
        PrimitiveKind::Cylinder,
        PrimitiveKind::Sphere,
    ];

    /// The script/serialization name.
    pub fn script_name(self) -> &'static str {
        match self {
            PrimitiveKind::Cuboid => "cuboid",
            PrimitiveKind::Cylinder => "cylinder",
            PrimitiveKind::Sphere => "sphere",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "cuboid" | "box" | "cube" => Some(PrimitiveKind::Cuboid),
            "cylinder" => Some(PrimitiveKind::Cylinder),
            "sphere" | "ball" => Some(PrimitiveKind::Sphere),
            _ => None,
        }
    }

    /// The next shape in the tool's cycle (#909).
    #[allow(dead_code)]
    pub fn next(self) -> Self {
        match self {
            PrimitiveKind::Cuboid => PrimitiveKind::Cylinder,
            PrimitiveKind::Cylinder => PrimitiveKind::Sphere,
            PrimitiveKind::Sphere => PrimitiveKind::Cuboid,
        }
    }
}

/// A primitive solid placed straight into 3D (#909) — no sketch, no profile. It sits **on**
/// its anchor plane (the ground, a body face, or a construction plane) and grows along that
/// plane's normal: a cuboid from the centre of its base rectangle, a cylinder from the centre
/// of its base circle, a sphere from the point it rests on. Every dimension is an expression,
/// so a shape is as parametric as anything else.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Primitive {
    pub kind: PrimitiveKind,
    /// Where the shape sits, in world mm.
    pub origin: [f32; 3],
    /// The anchor plane's normal — the direction the shape grows along.
    pub normal: [f32; 3],
    /// The anchor plane's first in-plane direction: a cuboid's width runs along it, its
    /// depth along `normal × u_axis`.
    pub u_axis: [f32; 3],
    /// Cuboid only: the extent along `u_axis`.
    #[serde(default)]
    pub width: String,
    /// Cuboid only: the extent across it.
    #[serde(default)]
    pub depth: String,
    /// Cuboid/cylinder: the extent along the normal.
    #[serde(default)]
    pub height: String,
    /// Cylinder/sphere.
    #[serde(default)]
    pub radius: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a primitive shape (#1055), like [`RevolutionKey`].
pub type PrimitiveKey = crate::arena::Key<Primitive>;

impl Primitive {
    /// A shape of `kind` on the ground at the world origin, with no dimensions yet.
    pub fn new(kind: PrimitiveKind) -> Self {
        Self {
            kind,
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            u_axis: [1.0, 0.0, 0.0],
            width: String::new(),
            depth: String::new(),
            height: String::new(),
            radius: String::new(),
            name: None,
        }
    }
}

/// A revolved solid: one or more coplanar closed profiles swept around an axis. Parametric
/// like everything else — the solid is rebuilt from the live profiles on every recompute.
/// A non-zero [`Self::pitch_mm`] makes the sweep helical (springs, #1242): the profile
/// advances that far along the axis per full 360° turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revolution {
    pub sketch: SketchId,
    /// Closed profiles to sweep, same shape as [`Extrusion::faces`].
    pub faces: Vec<ExtrudeFace>,
    pub axis: RevolveAxis,
    /// Sweep angle in degrees (default 360 = a full solid of revolution). Signed and
    /// multi-turn are allowed (#1242): negative reverses the turn, and values past ±360
    /// keep winding when [`Self::pitch_mm`] is non-zero.
    pub angle_deg: f32,
    /// Axial travel per full 360° revolution in millimetres (#1242). Zero is a pure revolve;
    /// non-zero makes a helix so successive coils of a spring sit this far apart
    /// start-to-start. Signed: negative advances the other way along the axis.
    #[serde(default)]
    pub pitch_mm: f32,
    /// Sweep `angle_deg/2` to each side of the profile plane instead of one way.
    #[serde(default)]
    pub symmetric: bool,
    /// How the solid lands (new body / fuse into bodies / cut bodies).
    pub mode: RevolveMode,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a revolve (#1055): the bodies it produced, its Elements-pane row,
/// its scene element, and its component membership.
pub type RevolutionKey = crate::arena::Key<Revolution>;

/// How a swept solid lands in the document (#sweep): its own body, fused into
/// existing bodies, or subtracted from existing bodies (the cut list is user-picked).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepMode {
    NewBody,
    AddTo(Vec<BodyKey>),
    Cut(Vec<BodyKey>),
}

/// A swept solid (the Sweep tool, #sweep): one or more coplanar closed
/// profiles swept along a path of sketch lines (straight or bezier) that intersects the
/// profile plane. Parametric like everything else — the solid is rebuilt from the live
/// profiles and path on every recompute.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sweep {
    pub sketch: SketchId,
    /// Closed profiles to sweep, same shape as [`Extrusion::faces`].
    pub faces: Vec<ExtrudeFace>,
    /// Path segments as `Document::lines` indices; chained tip-to-tail on evaluation
    /// (pick order doesn't matter).
    pub path: Vec<LineKey>,
    /// How the solid lands (new body / fuse into bodies / cut bodies).
    pub mode: SweepMode,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a sweep (#1055), like [`RevolutionKey`].
pub type SweepKey = crate::arena::Key<Sweep>;

/// Which set algebra a boolean operation (Combine tool) applies to its input bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOpKind {
    /// Union of all `a` inputs into one solid.
    Combine,
    /// `a` minus `b`.
    Cut,
    /// Only what's common to `a` and `b`.
    Intersect,
    /// Symmetric difference: everything *not* common to `a` and `b`.
    Difference,
}

#[allow(dead_code)] // wired up by the Combine tool below in this feature
impl BooleanOpKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Combine => "Combine",
            Self::Cut => "Cut",
            Self::Intersect => "Intersect",
            Self::Difference => "Difference",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "combine" | "union" | "fuse" | "merge" => Some(Self::Combine),
            "cut" | "subtract" => Some(Self::Cut),
            "intersect" | "intersection" | "common" => Some(Self::Intersect),
            "difference" | "xor" | "symmetric_difference" => Some(Self::Difference),
            _ => None,
        }
    }
}

/// A boolean operation between whole bodies (the Combine tool). Its inputs become
/// **shadow** bodies (unless `keep_b`), its outputs are fresh [`Body`] elements with
/// [`BodySource::Boolean`] sources, and the operation itself is an editable element in
/// the pane: outputs depend on the operation, the operation depends on every input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BooleanOperation {
    pub kind: BooleanOpKind,
    /// Input bodies on the A side (the only side for `Combine`).
    pub a: Vec<BodyKey>,
    /// Input bodies on the B side (cut/intersect/difference).
    #[serde(default)]
    pub b: Vec<BodyKey>,
    /// Keep the B-side inputs as real bodies after the operation instead of shadowing them.
    #[serde(default)]
    pub keep_b: bool,
    /// Output bodies, in solid-ordinal order.
    #[serde(default)]
    pub outputs: Vec<BodyKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a boolean operation (#1055).
pub type BooleanOpKey = crate::arena::Key<BooleanOperation>;

/// How a [`MoveOperation`]'s translation is specified (#648), the Move pane's Move mode
/// dropdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveTranslateMode {
    /// Snap a picked **source point** on the moving bodies onto a picked **target point** on
    /// the stationary geometry (#650). The Move tool's default.
    #[default]
    PointSnap,
    /// Put a picked **face** on the moving part onto a picked face on a fixed one (#1076),
    /// point to point, with a side to land on and a turn about the target's normal. The
    /// Joint tool's default — a mate is nearly always a face going onto a face.
    FaceSnap,
    /// Type or drag X/Y/Z amounts and X/Y/Z turns outright (#648/#1076) — no reference to any
    /// other object.
    Free,
    /// The part is already where it belongs, so the move is the identity (#1076). Offered by
    /// the **Joint tool only**: a joint often just needs saying "don't move anything", and
    /// without this the user has to fight the tool to say it. It takes no picks and no
    /// values, which is also how the pane says so — by offering nothing.
    InPlace,
}

impl MoveTranslateMode {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "snap" | "points" | "point_snap" => Some(Self::PointSnap),
            "face" | "faces" | "face_snap" => Some(Self::FaceSnap),
            "free" | "components" | "xyz" => Some(Self::Free),
            "in_place" | "place" | "none" => Some(Self::InPlace),
            _ => None,
        }
    }

    /// The name scripts and the status line use.
    pub fn name(self) -> &'static str {
        match self {
            Self::PointSnap => "point_snap",
            Self::FaceSnap => "face_snap",
            Self::Free => "free",
            Self::InPlace => "in_place",
        }
    }

    /// The pane's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::PointSnap => "Point Snap",
            Self::FaceSnap => "Face Snap",
            Self::Free => "Free",
            Self::InPlace => "In place",
        }
    }

    /// The modes a tool offers, in pane order (#1076). Only the Joint tool offers
    /// [`InPlace`](Self::InPlace) — a Move that moves nothing is a no-op, not a mode.
    pub fn for_tool(joint: bool) -> Vec<Self> {
        let mut modes = vec![Self::PointSnap, Self::FaceSnap, Self::Free];
        if joint {
            modes.push(Self::InPlace);
        }
        modes
    }

    /// Whether this mode derives its placement from picked geometry rather than from typed
    /// amounts — the two snapping modes.
    pub fn is_snap(self) -> bool {
        matches!(self, Self::PointSnap | Self::FaceSnap)
    }
}

/// A point a Move snaps from or onto (#649/#650): a corner, the midpoint of a feature edge,
/// the middle of a planar face, or the **world origin** (#946). The body-derived variants are
/// keyed exactly like [`crate::hierarchy::SceneElement::BodyVertex`]/`BodyEdge` — the body plus
/// quantized world points — and resolved against the body's live mesh, so they follow the
/// geometry and simply stop resolving if a rebuild takes them away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovePointRef {
    Vertex {
        body: BodyKey,
        p: [i32; 3],
    },
    EdgeMidpoint {
        body: BodyKey,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// A point **along** a body's feature edge (#670) — where the end-point-B constraint
    /// sphere crosses it. It isn't a mesh vertex, so unlike the other two it can't be
    /// re-found by matching; it keeps its own quantized world position and simply stops
    /// resolving if the body goes away.
    OnEdge {
        body: BodyKey,
        p: [i32; 3],
    },
    /// A point lying **within** one of a body's planar faces (#738/#1074) — what a Face Snap
    /// mate puts onto another face. The face is keyed by quantized centroid+normal exactly
    /// like [`crate::hierarchy::SceneElement::BodyFace`] and resolved against the live mesh's
    /// coplanar-triangle groups, so it follows the geometry.
    ///
    /// The point itself is held as an offset from the face's centre **in the face's own
    /// plane**, not as a world position: a rebuild that moves or resizes the face carries the
    /// point with it, which is the whole reason a mate can survive a parameter change. The
    /// face **centre** (#738, the only kind of face point there used to be) is simply
    /// `uv == [0, 0]`.
    OnFace {
        body: BodyKey,
        centroid: [i32; 3],
        normal: [i32; 3],
        /// Offset from the face centre along the face frame's u and v axes
        /// ([`crate::construction::plane_basis`]), quantized like every other body point so
        /// the variant stays `Eq`/`Hash`.
        #[serde(default)]
        uv: [i32; 2],
    },
    /// The **world origin** (#946): a fixed stationary point every document has, so a body can
    /// be snapped onto (0, 0, 0) or turned about it without a body having a corner there.
    Origin,
}

/// The face a mating point sits on as a [`MateRef`] (#1079) — how the joint pane shows the
/// two faces its placement named. `None` for a point that isn't on one.
pub fn move_point_host_mate_ref(point: &MovePointRef) -> Option<MateRef> {
    let MovePointRef::OnFace { body, centroid, normal, .. } = point else {
        return None;
    };
    Some(MateRef::Face { body: *body, centroid: *centroid, normal: *normal })
}

impl MoveOperation {
    /// Whether this move's translation actually comes from its two snap points (#650). A Snap
    /// move that hasn't got both points yet — or one with no bodies at all, like a plane or
    /// image move — still reads its `tx`/`ty`/`tz` expressions, so the tool stays usable while
    /// the points are being picked and gizmo drags keep working.
    pub fn has_snap_translation(&self) -> bool {
        // Both snapping modes (#1076) derive their translation from the A pair; Face Snap
        // simply picks its points on faces.
        self.translate_mode.is_snap()
            && self.start_point_a.is_some()
            && self.end_point_a.is_some()
    }

    /// Whether the optional B pair is complete, so the move rotates as well as translates
    /// (#669). Only meaningful alongside a resolved A pair — B turns *about* end point A.
    pub fn has_snap_rotation(&self) -> bool {
        self.has_snap_translation()
            && self.start_point_b.is_some()
            && self.end_point_b.is_some()
    }

    /// Whether the optional C pair is complete, so the move's last degree of freedom is
    /// pinned too. Only meaningful alongside a resolved B pair — C spins *about* the
    /// `endA → endB` axis that B leaves free.
    pub fn has_snap_roll(&self) -> bool {
        self.has_snap_rotation()
            && self.start_point_c.is_some()
            && self.end_point_c.is_some()
    }

    /// Whether the third pair is set as an **angle** instead (#1078). The point form wins
    /// when both are set: a picked point says where the part should face, a number only says
    /// how far to turn it, so the more specific answer is the one to honour.
    pub fn has_snap_roll_angle(&self) -> bool {
        self.has_snap_rotation()
            && self.end_point_c.is_none()
            && !self.roll_angle.trim().is_empty()
    }
}

impl MovePointRef {
    /// The body this point lives on — what tells a *moving* point from a stationary one.
    /// `None` for the document-level [`Self::Origin`] (#946), which no body owns.
    pub fn body(&self) -> Option<BodyKey> {
        match self {
            MovePointRef::Vertex { body, .. }
            | MovePointRef::EdgeMidpoint { body, .. }
            | MovePointRef::OnEdge { body, .. }
            | MovePointRef::OnFace { body, .. } => Some(*body),
            MovePointRef::Origin => None,
        }
    }
}

/// A move operation (Move tool, #176/#183): a rigid **translation** applied to whole bodies.
/// By default inputs become **shadow** bodies; each input gets a moved output body
/// (`BodySource::Moved`), and the operation itself is an editable pane element. The
/// translation components are expressions, so moves are parameter-driven like dimensions.
///
/// With [`keep_inputs`](Self::keep_inputs) set (Paste Linked, #1236), inputs stay live and
/// the outputs remain dependent copies that update when the originals change.
///
/// `Default` is the empty move — no targets, no picks, no amounts — which places nothing.
/// That is what a joint holds until its mate is set (#1079).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MoveOperation {
    /// Input bodies, one output per entry (same order).
    pub targets: Vec<BodyKey>,
    /// When true, input bodies stay live (not shadowed) — Paste Linked (#1236). Outputs still
    /// update when the sources change. Default false: ordinary Move consumes its inputs.
    #[serde(default)]
    pub keep_inputs: bool,
    /// How the translation is specified (#648).
    #[serde(default)]
    pub translate_mode: MoveTranslateMode,
    /// The point on the moving bodies that a snap translation moves **from** (#649/#668).
    #[serde(default)]
    pub start_point_a: Option<MovePointRef>,
    /// The point on the stationary geometry that a snap translation moves start point A
    /// **onto** (#650/#668). With both set, the translation is `end - start` and the `tx`/
    /// `ty`/`tz` expressions are ignored.
    #[serde(default)]
    pub end_point_a: Option<MovePointRef>,
    /// The optional second pair (#669): another point on the **moving** bodies and where it
    /// should end up. The A pair fixes the translation; B then fixes the **rotation** about
    /// end point A that brings start B as near end B as it can. Both must be set to rotate.
    #[serde(default)]
    pub start_point_b: Option<MovePointRef>,
    #[serde(default)]
    pub end_point_b: Option<MovePointRef>,
    /// The optional third pair: a point on the **moving** bodies and where it should end up.
    /// A fixes the translation and B the rotation, but that still leaves the bodies free to
    /// spin about the `endA → endB` axis; C fixes that last turn, so the placement is fully
    /// determined. Both must be set for it to apply.
    #[serde(default)]
    pub start_point_c: Option<MovePointRef>,
    #[serde(default)]
    pub end_point_c: Option<MovePointRef>,
    /// Construction planes moved by this op (#217): transformed in place at recompute, so
    /// sketches/images anchored to them follow. No output bodies — the plane itself moves.
    #[serde(default)]
    pub plane_targets: Vec<ConstructionPlaneKey>,
    /// Tracing images moved by this op (#217): their plane-local origin is transformed in
    /// place at recompute (projected onto the host plane), like a plane. No output bodies.
    #[serde(default)]
    pub image_targets: Vec<TracingImageKey>,
    /// Unit instances moved by this op (#735): like a plane, the instance itself moves —
    /// its placement transform composes with this op at evaluation, no output bodies.
    #[serde(default)]
    pub instance_targets: Vec<UnitInstanceKey>,
    /// Translation components (mm expressions; empty = 0).
    #[serde(default)]
    pub tx: String,
    #[serde(default)]
    pub ty: String,
    #[serde(default)]
    pub tz: String,
    /// Point Snap's third pair set as an **angle** instead of a target point (#1078): the
    /// spin about the `endA → endB` axis, a degree expression. Used when no end point C is
    /// picked — the point form wins when both are set, since a picked point says exactly
    /// where the part should face and a number only says how far to turn it.
    #[serde(default)]
    pub roll_angle: String,
    /// Face Snap (#1077): which side of the target face to land on. The default puts the two
    /// normals **opposed**, so the surfaces touch — outsides of objects go together; flipped,
    /// they point the same way and the part sits behind the face.
    #[serde(default)]
    pub face_flip: bool,
    /// Face Snap (#1077): the turn of the moving face about the target face's normal, through
    /// the mate point. A degree expression; empty = 0.
    #[serde(default)]
    pub face_spin: String,
    /// Face Snap (#1079): a gap held along the target face's normal, an mm expression. Empty
    /// is flush — the surfaces touching, which is the usual answer; a joint that needs a
    /// washer's worth of clearance says so here.
    #[serde(default)]
    pub face_offset: String,
    /// Free-mode turns about the world X/Y/Z axes (#1076), degree expressions; empty = 0.
    /// Applied about the moving bodies' own centre, in X-then-Y-then-Z order, before the
    /// translation — so typing a turn spins the part where it stands rather than swinging it
    /// around the world origin. Ignored by every other mode, which get their rotation from
    /// the geometry they snap to.
    #[serde(default)]
    pub rx: String,
    #[serde(default)]
    pub ry: String,
    #[serde(default)]
    pub rz: String,
    /// Output bodies, matching `targets` order.
    #[serde(default)]
    pub outputs: Vec<BodyKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a move operation (#1055).
pub type MoveOpKey = crate::arena::Key<MoveOperation>;

/// What a joint holds on each side (#891): a whole body, everything in a component, or a
/// placed unit instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointRef {
    Body(BodyKey),
    Component(ComponentKey),
    UnitInstance(UnitInstanceKey),
}

/// How a joint lets its driven side move relative to its base (#891).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    /// No freedom: the driven side is held to the base. The only kind that accepts more
    /// than two members — a rigid group (#900) is a rigid joint with a longer member list.
    #[default]
    Rigid,
    /// Slide along the frame's primary axis.
    Slider,
    /// Turn about the frame's primary axis.
    Revolute,
    /// Slide and turn about the same axis, independently.
    Cylindrical,
    /// Slide across the frame's plane (both secondary directions) and spin about its
    /// primary axis.
    Planar,
    /// Turn about all three frame axes; no translation.
    Ball,
    /// Slide along the primary axis while turning about the secondary one.
    PinSlot,
    /// Turn about the primary axis with the slide coupled to it by a lead.
    Screw {
        /// Travel per full turn, a mm expression.
        lead: String,
    },
}

/// One side of a mate pick (#1014/#1015): the geometry a part is placed by. Every variant is
/// a body-local or world-fixed key resolved against the live model, like [`MovePointRef`], so
/// a mate survives a rebuild and simply stops resolving when its geometry goes away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MateRef {
    /// A planar face of a body's mesh, keyed by quantized centroid + normal exactly like
    /// [`crate::hierarchy::SceneElement::BodyFace`].
    Face {
        body: BodyKey,
        centroid: [i32; 3],
        normal: [i32; 3],
    },
    /// A datum plane (#1018) — what the first part of an assembly is grounded against.
    Plane(ConstructionPlaneKey),
    /// A straight edge of a body's mesh, keyed like [`crate::hierarchy::SceneElement::BodyEdge`].
    Edge {
        body: BodyKey,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// One of the world axes (#952/#1018).
    Axis(crate::construction::GlobalAxis),
    /// A cylindrical surface's centre line (#1013): a hole's or a shaft's axis, which is what
    /// "line these two up" usually means. Keyed by the fitted axis, re-found on the live mesh.
    HoleAxis {
        body: BodyKey,
        origin: [i32; 3],
        dir: [i32; 3],
    },
    /// A point: a corner, an edge midpoint, a face's middle, or the world origin.
    Point(MovePointRef),
}

impl MateRef {
    /// The body this reference lives on. `None` for the world-fixed ones (a datum plane, a
    /// world axis, the origin), which no body owns and no joint pose carries.
    pub fn body(&self) -> Option<BodyKey> {
        match self {
            MateRef::Face { body, .. }
            | MateRef::Edge { body, .. }
            | MateRef::HoleAxis { body, .. } => Some(*body),
            MateRef::Point(p) => p.body(),
            MateRef::Plane(_) | MateRef::Axis(_) => None,
        }
    }
}

/// How a joint's freedoms are oriented (#1079): the frame its slide and turn act in.
///
/// This used to be derived from the mate at solve time — the primary axis *was* the mating
/// normal, and there was no way to say otherwise. Now that a joint's placement is an ordinary
/// move (#1068), three of the four move modes name no plane at all, so the frame is stored
/// state instead: a mate may **populate** it — Face Snap fills in the fixed face's normal and
/// an in-plane direction — but every field stays the user's to change, and a move that names
/// no plane simply leaves it unset to be filled in by hand.
///
/// `None` throughout is a joint with no frame yet: it holds its parts where they are, which is
/// what a Rigid joint does anyway and is the honest answer for the others until an axis is
/// given.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JointFrame {
    /// Where the freedoms act from — the frame's origin.
    #[serde(default)]
    pub origin: Option<MovePointRef>,
    /// What a Slider slides along and a Revolute turns about. A face or datum plane
    /// contributes its **normal**, an edge or world axis its direction, a hole its centre
    /// line; a point has no direction and is not accepted here.
    #[serde(default)]
    pub primary: Option<MateRef>,
    /// The second axis, which fixes the frame's roll: what a Planar joint's V runs along and
    /// what a Ball's second turn is about. Squared against `primary`, so it only has to be
    /// roughly right.
    #[serde(default)]
    pub secondary: Option<MateRef>,
}

impl JointFrame {
    /// Nothing set: the joint has no frame and holds its parts where they are.
    pub fn is_empty(&self) -> bool {
        self.origin.is_none() && self.primary.is_none() && self.secondary.is_none()
    }
}

/// Where a joint's travel stops (#896). Every field is optional — an empty expression and
/// no target leaves that end open.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JointLimits {
    /// Slide minimum/maximum, mm expressions.
    #[serde(default)]
    pub slide_min: String,
    #[serde(default)]
    pub slide_max: String,
    /// Slide stops as geometry: the travel ends where the driven side meets the target's
    /// extended plane (the "extrude to object" idea, [`ExtrudeTarget`]). Wins over the
    /// expression on the same end when both are set.
    #[serde(default)]
    pub slide_min_target: Option<ExtrudeTarget>,
    #[serde(default)]
    pub slide_max_target: Option<ExtrudeTarget>,
    /// Turn minimum/maximum, signed degree expressions to either side of zero — a hinge
    /// that opens 110° one way and not at all the other is `turn_min = "0"`,
    /// `turn_max = "110"`.
    #[serde(default)]
    pub turn_min: String,
    #[serde(default)]
    pub turn_max: String,
}

/// A joint (#891): a kinematic relationship between parts — bodies, components, or unit
/// instances. A joint changes where things *are*, never their shape: at recompute the
/// driven members are transformed **in place**, the way a Move's plane targets are — two
/// (or more) inputs, no output bodies. `members[base]` is held; the rest are driven.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Joint {
    /// The joined parts. Exactly two for every kind but [`JointKind::Rigid`], which
    /// accepts more (#900).
    pub members: Vec<JointRef>,
    /// Index into `members` of the base (held) side. Defaults to the first picked.
    #[serde(default)]
    pub base: usize,
    #[serde(default)]
    pub kind: JointKind,
    /// Where the parts start out (#1021/#1079): an ordinary **move**, in any of its modes.
    /// A joint mate *is* a move — optionally a no-op, when the part is already where it
    /// belongs — so it is one, rather than a second thing shaped like one. A placement only:
    /// the kind's freedoms act on top of it, oriented by [`Joint::frame`]. An empty move
    /// places nothing, so joining parts already in place moves nothing.
    #[serde(default)]
    pub placement: MoveOperation,
    /// How the freedoms are oriented (#1079). Seeded from the mate when it names a plane,
    /// and editable either way.
    #[serde(default)]
    pub frame: JointFrame,
    /// The joint's current value along each freedom, as expressions so a pose is
    /// parametric like a dimension. What each slot means depends on `kind`:
    /// slider/cylindrical/pin-slot/planar read `position` as mm of slide (planar's u),
    /// revolute/screw as degrees of turn; `position2` is cylindrical's/pin-slot's degrees
    /// and planar's v; `position3` is planar's spin. Ball reads all three as degrees
    /// about the primary/secondary/tertiary axes. Empty = 0.
    #[serde(default)]
    pub position: String,
    #[serde(default)]
    pub position2: String,
    #[serde(default)]
    pub position3: String,
    /// The set/default pose to revert to (#898), same slots as `position`. Captured from
    /// wherever the parts were when the joint was made.
    #[serde(default)]
    pub rest: String,
    #[serde(default)]
    pub rest2: String,
    #[serde(default)]
    pub rest3: String,
    #[serde(default)]
    pub limits: JointLimits,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a joint (#1055).
pub type JointKey = crate::arena::Key<Joint>;

impl JointKind {
    #[allow(dead_code)] // consumed by the Joint tool + scripting (#894/#901)
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "rigid" => Some(Self::Rigid),
            "slider" => Some(Self::Slider),
            "revolute" => Some(Self::Revolute),
            "cylindrical" => Some(Self::Cylindrical),
            "planar" => Some(Self::Planar),
            "ball" => Some(Self::Ball),
            "pin_slot" | "pinslot" | "pin-slot" => Some(Self::PinSlot),
            "screw" => Some(Self::Screw { lead: String::new() }),
            _ => None,
        }
    }

    /// The next kind in the Joint tool's cycle (#921), in the order the dropdown lists
    /// them. A screw keeps no lead — it's a fresh pick each time round.
    pub fn next(&self) -> Self {
        match self {
            Self::Rigid => Self::Slider,
            Self::Slider => Self::Revolute,
            Self::Revolute => Self::Cylindrical,
            Self::Cylindrical => Self::Planar,
            Self::Planar => Self::Ball,
            Self::Ball => Self::PinSlot,
            Self::PinSlot => Self::Screw { lead: String::new() },
            Self::Screw { .. } => Self::Rigid,
        }
    }

    /// The kind's script/display name (the inverse of [`JointKind::from_name`]).
    #[allow(dead_code)] // consumed by the Joint tool + scripting (#894/#901)
    pub fn name(&self) -> &'static str {
        match self {
            Self::Rigid => "rigid",
            Self::Slider => "slider",
            Self::Revolute => "revolute",
            Self::Cylindrical => "cylindrical",
            Self::Planar => "planar",
            Self::Ball => "ball",
            Self::PinSlot => "pin_slot",
            Self::Screw { .. } => "screw",
        }
    }
}

impl Joint {
    /// The base (held) member, if the joint has any members at all.
    #[allow(dead_code)] // consumed by the kinematics pass (#893)
    pub fn base_member(&self) -> Option<JointRef> {
        self.members.get(self.base).or_else(|| self.members.first()).copied()
    }

    /// The members the joint moves: everyone but the base.
    #[allow(dead_code)] // consumed by the kinematics pass (#893)
    pub fn driven_members(&self) -> impl Iterator<Item = JointRef> + '_ {
        let base = if self.base < self.members.len() { self.base } else { 0 };
        self.members
            .iter()
            .enumerate()
            .filter(move |(i, _)| *i != base)
            .map(|(_, m)| *m)
    }
}

/// How a [`MirrorOperation`]'s reflections land (#639), the Mirror pane's **Output** row —
/// the same New body / Join / Cut choice the Revolve tool offers, but each reflection combines
/// with **its own source body** (there's nothing else to pick): the half-model → whole-model
/// case for `Join`, and a mirrored pocket for `Cut`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorMode {
    /// Each reflection is its own body and the originals stay (the pre-#639 behavior).
    #[default]
    NewBody,
    /// Each output is its source fused with the reflection; the source is consumed (shadowed).
    Join,
    /// Each output is its source with the reflection subtracted; the source is consumed.
    Cut,
}

impl MirrorMode {
    /// Whether this mode consumes its input body into the output (shadowing it), the way Move
    /// and the edge treatments do. Driven by the mirror operation's [`crate::opsigs::Operation`]
    /// `HOST_EFFECT` (Join/Cut shadow; New body does not).
    pub fn consumes_input(self) -> bool {
        matches!(
            crate::opsigs::mirror_host_effect(self),
            crate::opsigs::HostBodyEffect::ShadowHostAndProduce
        )
    }
}

/// A mirror operation (Mirror tool, #523): reflects each input body across a mirror plane,
/// producing one output body per input. In the default `NewBody` mode the reflection is a
/// body of its own and the originals stay; `Join`/`Cut` fuse or subtract it against its own
/// source instead (#639). The mirror plane is a `FaceId` — a construction plane or a planar
/// body face.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MirrorOperation {
    /// The mirror plane: a construction plane or a planar body face.
    pub plane: FaceId,
    /// Input bodies, one reflected output per entry (same order).
    pub targets: Vec<BodyKey>,
    /// How each reflection lands (#639).
    #[serde(default)]
    pub mode: MirrorMode,
    /// Output bodies, matching `targets` order.
    #[serde(default)]
    pub outputs: Vec<BodyKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a mirror operation (#1055).
pub type MirrorOpKey = crate::arena::Key<MirrorOperation>;

/// How a linear repeat spaces its instances (Repeat tool, #182). `gap` measures between
/// an instance's end and the next one's start; `pitch` measures start-to-start; `fit`
/// modes squeeze N instances into a length L.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    /// N instances, clear gap D between them (end-to-start).
    CountGap,
    /// N instances spread evenly so the last *ends* at length D.
    CountFitEnds,
    /// N instances spread evenly so the last *starts* at length D (start-to-start span).
    CountFitCenters,
    /// Fill length L with as many instances as fit, clear gap D between them.
    FillGap,
    /// Fill length L with as many instances as fit at start-to-start pitch D.
    FillPitch,
    /// Fill length L ending with an instance at the end, pitch at most D (stud spacing:
    /// never farther apart than D on center, squeezed evenly to land the last one).
    FillMaxPitch,
    /// N instances at start-to-start **pitch** D (#257): like [`Self::CountGap`] but D is the
    /// pitch (item length + gap) rather than the clear gap, so the offset-vs-gap toggle works
    /// with a count.
    CountPitch,
    /// Fill a start-to-start **span** L with clear gap D between instances (#257) — the span
    /// variant of [`Self::FillGap`], which fills an end-to-end length.
    FillGapSpan,
    /// Fill a start-to-start **span** L at pitch D (#257) — the span variant of [`Self::FillPitch`].
    FillPitchSpan,
}

impl RepeatMode {
    /// Human-readable mode name. Retained for diagnostics/scripting though the count/gap/distance
    /// UI (#257) no longer surfaces raw modes.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::CountGap => "Count × gap",
            Self::CountFitEnds => "Count fit (to end)",
            Self::CountFitCenters => "Count fit (start-to-start)",
            Self::FillGap => "Fill length, gap",
            Self::FillPitch => "Fill length, pitch",
            Self::FillMaxPitch => "Fill length, max pitch",
            Self::CountPitch => "Count × pitch",
            Self::FillGapSpan => "Fill span, gap",
            Self::FillPitchSpan => "Fill span, pitch",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "count_gap" | "count" => Some(Self::CountGap),
            "count_fit_ends" | "fit" => Some(Self::CountFitEnds),
            "count_fit_centers" | "fit_centers" => Some(Self::CountFitCenters),
            "fill_gap" => Some(Self::FillGap),
            "fill_pitch" => Some(Self::FillPitch),
            "fill_max_pitch" | "max_pitch" => Some(Self::FillMaxPitch),
            "count_pitch" => Some(Self::CountPitch),
            "fill_gap_span" => Some(Self::FillGapSpan),
            "fill_pitch_span" => Some(Self::FillPitchSpan),
            _ => None,
        }
    }

    /// Whether the mode uses the count `n` (vs deriving it from the length).
    #[allow(dead_code)]
    pub fn uses_count(self) -> bool {
        matches!(
            self,
            Self::CountGap | Self::CountFitEnds | Self::CountFitCenters | Self::CountPitch
        )
    }

    /// Whether the mode uses the fill length `length`.
    #[allow(dead_code)]
    pub fn uses_length(self) -> bool {
        !matches!(self, Self::CountGap | Self::CountPitch)
    }

    /// The `RepeatMode` for the count/gap/distance UI (#257) given which variable is **computed**
    /// (the other two are user-set) and the two toggles: `gap_is_offset` (the gap field is a
    /// start-to-start pitch rather than a clear gap) and `distance_is_end` (distance is measured
    /// to the end of the last item rather than to its start). The UI's count/gap/distance fields
    /// map straight onto the mode's count/spacing/length inputs.
    pub fn from_repeat_ui(computed: RepeatVar, gap_is_offset: bool, distance_is_end: bool) -> Self {
        match computed {
            // count + gap given → distance computed.
            RepeatVar::Distance => {
                if gap_is_offset {
                    Self::CountPitch
                } else {
                    Self::CountGap
                }
            }
            // count + distance given → gap computed.
            RepeatVar::Gap => {
                if distance_is_end {
                    Self::CountFitEnds
                } else {
                    Self::CountFitCenters
                }
            }
            // gap + distance given → count computed.
            RepeatVar::Count => match (gap_is_offset, distance_is_end) {
                (false, true) => Self::FillGap,
                (false, false) => Self::FillGapSpan,
                (true, true) => Self::FillPitch,
                (true, false) => Self::FillPitchSpan,
            },
        }
    }

    /// The count/gap/distance UI state `(computed, gap_is_offset, distance_is_end)` for a stored
    /// mode (#257) — the inverse of [`Self::from_repeat_ui`], used when re-opening a committed
    /// repeat for editing. The legacy `FillMaxPitch` maps to the nearest UI (count-computed,
    /// offset) since the new UI can't otherwise express it.
    pub fn to_repeat_ui(self) -> (RepeatVar, bool, bool) {
        match self {
            Self::CountGap => (RepeatVar::Distance, false, true),
            Self::CountPitch => (RepeatVar::Distance, true, true),
            Self::CountFitEnds => (RepeatVar::Gap, false, true),
            Self::CountFitCenters => (RepeatVar::Gap, false, false),
            Self::FillGap => (RepeatVar::Count, false, true),
            Self::FillGapSpan => (RepeatVar::Count, false, false),
            Self::FillPitch => (RepeatVar::Count, true, true),
            Self::FillPitchSpan => (RepeatVar::Count, true, false),
            Self::FillMaxPitch => (RepeatVar::Count, true, true),
        }
    }
}

impl RepeatVar {
    /// The MRU array (`[set, set, computed]`) placing `self` as the computed variable (#257).
    pub fn as_mru(self) -> [RepeatVar; 3] {
        let others: Vec<RepeatVar> = [RepeatVar::Count, RepeatVar::Gap, RepeatVar::Distance]
            .into_iter()
            .filter(|&v| v != self)
            .collect();
        [others[0], others[1], self]
    }
}

/// One of the Repeat tool's three interlinked variables (#257): the user sets two and the third
/// is computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatVar {
    Count,
    Gap,
    Distance,
}

/// A linear repeat (Repeat tool, #182): copies of whole bodies spaced along an axis. The
/// original stays as instance 0; each further instance of each target gets an output body
/// (`BodySource::Repeated`). Count/spacing/length are expressions, so repeats rebuild
/// parametrically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepeatOperation {
    pub targets: Vec<BodyKey>,
    /// Source construction plane indices to repeat as offset copies (#221). Separate from
    /// `targets` (bodies) because a plane instance is a generated [`ConstructionPlane`] carrying
    /// a [`RepeatPlaneInstance`], not a [`BodySource::Repeated`] body.
    #[serde(default)]
    pub plane_targets: Vec<ConstructionPlaneKey>,
    /// Cut / add-to-body **extrusion** indices whose *effect* is replayed at each offset
    /// (#220): a cut punches N holes; an add fused onto another body grows N bumps. A
    /// standalone add extrusion (one that *is* a body) is promoted to [`targets`] instead
    /// (#1475), so disjoint copies are separate bodies.
    #[serde(default)]
    pub extrusion_targets: Vec<ExtrusionKey>,
    /// Source sketch indices to repeat as offset copies (#226). Each copy rides a generated
    /// construction plane parallel to the source's, translated along the axis, so its entities
    /// keep their plane-local coords and step by the offset in world. Restricted to
    /// construction-plane-hosted sketches.
    #[serde(default)]
    pub sketch_targets: Vec<SketchId>,
    /// Generated host-plane indices for the sketch copies (#226), instance-major then target.
    #[serde(default)]
    pub sketch_plane_outputs: Vec<ConstructionPlaneKey>,
    /// Generated copy-sketch indices (#226), instance-major then target. Each copy's lines and
    /// circles are found by sketch membership (not tracked separately).
    #[serde(default)]
    pub sketch_outputs: Vec<SketchId>,
    pub axis: RevolveAxis,
    /// A **circle** used as the path (#840): the copies ride around its circumference,
    /// keeping their orientation. When set it wins over `axis`.
    #[serde(default)]
    pub path_circle: Option<CircleKey>,
    /// Repeat **around** the path instead of along it (#839): copies turn about the axis
    /// rather than sliding along it, and `spacing`/`length` are read as angles (degrees).
    #[serde(default)]
    pub around_axis: bool,
    /// Run the pattern the **other way** along the path (#989). A path has two directions and
    /// nothing about picking a line, edge or axis says which one you meant — the direction
    /// falls out of how the geometry happens to be stored, so half the time the copies march
    /// off the wrong way and there was no way to say so. Reverses the slide along a straight
    /// axis, the sense of the turn when `around_axis`, and which end a curved path is followed
    /// from.
    #[serde(default)]
    pub flip: bool,
    pub mode: RepeatMode,
    /// Instance count expression (count modes).
    #[serde(default)]
    pub count: String,
    /// Gap/pitch expression `D`.
    #[serde(default)]
    pub spacing: String,
    /// Fill length expression `L` (fill and fit modes).
    #[serde(default)]
    pub length: String,
    /// When set, the fill length `L` is derived from the along-axis distance to this target's
    /// extended plane (like an extrusion's "up to face" #126), so `L` follows the face if it
    /// moves — overriding the `length` expression (#186).
    #[serde(default)]
    pub length_target: Option<ExtrudeTarget>,
    /// Output bodies: instance-major, then target (instance 1 of each target, then
    /// instance 2 of each target, …).
    #[serde(default)]
    pub outputs: Vec<BodyKey>,
    /// Generated construction-plane instance indices for [`plane_targets`] (#221), laid out
    /// instance-major then target, exactly like [`outputs`]. Each entry is a
    /// [`ConstructionPlane`] whose `repeat_instance` points back here.
    #[serde(default)]
    pub plane_outputs: Vec<ConstructionPlaneKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a linear repeat (#1055).
pub type RepeatOpKey = crate::arena::Key<RepeatOperation>;

/// Back-reference stamped on a generated construction plane that is one instance of a
/// [`RepeatOperation`]'s plane repeat (#221). The instance's frame is derived at recompute from
/// the source plane's *current* frame offset along the op's axis, so it follows the source if the
/// source plane itself moves — the same "cache derived from another element" pattern moved images
/// use (#217).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepeatPlaneInstance {
    /// The repeat that generated this plane.
    pub op: RepeatOpKey,
    /// Index into the op's [`RepeatOperation::plane_targets`].
    pub target: usize,
    /// 1-based instance number; the along-axis offset is `repeat_offsets(op)[instance - 1]`.
    pub instance: usize,
}

/// One cutter of a 3D [`SliceOperation`] (#181, #1126): a planar face/plane, or a sketch
/// line that laser-cuts through the body along the line's path.
///
/// Stored untagged so existing documents keep bare [`FaceId`] cutter JSON; a line cutter
/// is the object `{"line": <key>}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SliceCutter {
    /// Laser path: a sketch line (straight or curved) on a face. The cut extrudes the path
    /// through the body along the sketch normal. With [`SliceOperation::extend_infinite`]
    /// the path expands past its endpoints so a partial line still severs the solid.
    Line { line: LineKey },
    /// Planar cutter: construction plane or planar body face (#181).
    Face(FaceId),
}

impl From<FaceId> for SliceCutter {
    fn from(face: FaceId) -> Self {
        Self::Face(face)
    }
}

/// A slice operation (Slice tool, #181/#1126/#1151): cuts whole bodies with planar cutters
/// (construction planes or planar body faces) and/or sketch lines used as laser-style
/// path cutters, splitting each target into the fragments that fall on either side. Each
/// input body becomes a **shadow** body; every fragment is a fresh [`Body`] with a
/// [`BodySource::Sliced`] source, and the operation itself is an editable pane element —
/// fragments depend on the operation, the operation depends on every target, cutter, and
/// (for laser lines) the **sketch** that owns those lines. Sketch edits re-evaluate the
/// cut parametrically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SliceOperation {
    /// Input bodies (the A side); each is sliced independently.
    pub targets: Vec<BodyKey>,
    /// Cutters (the B side): planar faces/planes and/or sketch lines (#1126).
    #[serde(default)]
    pub cutters: Vec<SliceCutter>,
    /// When set, each planar cutter divides the whole target (its plane extends infinitely)
    /// and each line cutter expands past its endpoints. When clear, a face carves only its
    /// own footprint and a line only severs material within its span.
    #[serde(default)]
    pub extend_infinite: bool,
    /// Output bodies: target-major, then piece (all fragments of target 0, then
    /// target 1, …). The last fragment of each target absorbs any extra solids a rebuild
    /// produces, so the pane's element list stays stable while geometry changes.
    #[serde(default)]
    pub outputs: Vec<BodyKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a slice operation (#1055).
pub type SliceOpKey = crate::arena::Key<SliceOperation>;

/// A shell operation (Shell tool, #1156): hollows whole bodies to a wall thickness,
/// optionally removing picked open faces. Each input body becomes a **shadow** body; each
/// hollow result is a fresh [`Body`] with a [`BodySource::Shelled`] source. Open faces are
/// analytic body faces ([`FaceId`]) that belong to one of the targets; adjacent open faces
/// also remove the wall between them (OCCT `MakeThickSolid`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShellOperation {
    /// Input bodies; each is shelled independently.
    pub targets: Vec<BodyKey>,
    /// Faces to open (remove), leaving the wall thickness around them.
    #[serde(default)]
    pub open_faces: Vec<FaceId>,
    /// Wall thickness expression (mm when evaluated). Must be positive.
    #[serde(default = "default_shell_thickness")]
    pub thickness: String,
    /// Output bodies matching `targets` order.
    #[serde(default)]
    pub outputs: Vec<BodyKey>,
    #[serde(default)]
    pub name: Option<String>,
}

fn default_shell_thickness() -> String {
    "1".to_string()
}

/// How anything names a shell operation (#1156).
pub type ShellOpKey = crate::arena::Key<ShellOperation>;

/// One edge treated by an [`EdgeTreatmentOperation`] (#531): the stable, parametric edge
/// identity is a [`TreatableSolid`] plus an [`ExtrusionEdgeRef`] (a topological face/edge
/// address that re-resolves to live world coordinates on every rebuild), **not** a coordinate
/// snapshot — so a chamfer/fillet follows its edge when a parameter reshapes the body.
/// `target` says which of the op's input bodies the edge lives on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreatedEdge {
    /// Index into the owning op's `targets` list (which input body this edge belongs to).
    pub target: usize,
    /// Extrusion or Shape-tool primitive the analytic edge belongs to (#1329). Flattened so
    /// existing files keep the `extrusion` field; primitives write `primitive` instead.
    #[serde(flatten)]
    pub solid: TreatableSolid,
    pub edge: ExtrusionEdgeRef,
}

/// A 3D edge chamfer/fillet as a first-class operation (#531): its inputs are the bodies whose
/// edges are beveled plus the edges themselves; on commit each input body is turned into a
/// **shadow** body and a new output body (`BodySource::EdgeTreated`) carries the modification.
/// Modeled on [`MoveOperation`] — one shadowed input and one output per `targets` entry — so it
/// participates in the graph, the timeline, rollback, and undo like every other body operation.
/// The bevel itself reuses the extrusion mesh/kernel machinery: the output's shape is the input
/// body built with these treatments spliced onto its extrusions (see
/// `crate::extrude::occt_edge_treated_output_shape`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeTreatmentOperation {
    /// Input bodies — each is shadowed and gets one chamfered/filleted output.
    pub targets: Vec<BodyKey>,
    /// Edges to treat, each tagged with the `targets` entry it lives on.
    pub edges: Vec<TreatedEdge>,
    pub kind: VertexTreatmentKind,
    /// Chamfer distance / fillet radius (mm); must be positive to have any effect.
    pub amount: f32,
    /// Output bodies, matching `targets` order.
    #[serde(default)]
    pub outputs: Vec<BodyKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names an edge-treatment operation (#1055).
pub type EdgeTreatmentOpKey = crate::arena::Key<EdgeTreatmentOperation>;

/// A 2D in-sketch linear repeat (#222): duplicates selected sketch entities along an in-plane
/// direction as generated entities in the *same* sketch, grouped under the operation. The
/// sketch-space analogue of the 3D body [`RepeatOperation`] — operands and results are sketch
/// lines/circles offset in plane-local `(u, v)` coordinates, reusing the same spacing-mode math
/// ([`crate::extrude::spacing_offsets`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchRepeatOperation {
    /// The sketch the operands live in; every copy lands in the same sketch.
    pub sketch: SketchId,
    /// Source line indices to duplicate.
    #[serde(default)]
    pub line_targets: Vec<LineKey>,
    /// Source circle indices to duplicate.
    #[serde(default)]
    pub circle_targets: Vec<CircleKey>,
    /// Repeat direction in plane-local coords (normalized at recompute; the step is taken along
    /// this unit vector).
    pub dir_u: f32,
    pub dir_v: f32,
    pub mode: RepeatMode,
    #[serde(default)]
    pub count: String,
    #[serde(default)]
    pub spacing: String,
    #[serde(default)]
    pub length: String,
    /// Generated line-copy indices, instance-major then target (instance 1 of each target, then
    /// instance 2 of each target, …) — the same layout [`RepeatOperation::outputs`] uses.
    #[serde(default)]
    pub line_outputs: Vec<LineKey>,
    /// Generated circle-copy indices, instance-major then target.
    #[serde(default)]
    pub circle_outputs: Vec<CircleKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a SketchRepeatOperation (#1055).
pub type SketchRepeatOpKey = crate::arena::Key<SketchRepeatOperation>;

/// A 2D in-sketch offset: parallel copies of the picked lines (mitered where they
/// chain end-to-end) and concentric copies of the picked circles, at a signed
/// distance. Outputs are separate `Line`/`Circle` entries grouped under the op in
/// the Elements pane and regenerated whenever the sources or the distance change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchOffsetOperation {
    /// The sketch the sources live in; outputs land in the same sketch.
    pub sketch: SketchId,
    /// Source line indices to offset.
    #[serde(default)]
    pub line_targets: Vec<LineKey>,
    /// Source circle indices to offset.
    #[serde(default)]
    pub circle_targets: Vec<CircleKey>,
    /// Signed offset distance expression (mm): positive grows a closed loop/circle,
    /// negative shrinks (or flips an open chain's side).
    #[serde(default)]
    pub distance: String,
    /// Emit the offset copies as construction geometry.
    #[serde(default)]
    pub construction: bool,
    /// Generated line indices, aligned with `line_targets`.
    #[serde(default)]
    pub line_outputs: Vec<LineKey>,
    /// Generated circle indices, aligned with `circle_targets`.
    #[serde(default)]
    pub circle_outputs: Vec<CircleKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a SketchOffsetOperation (#1055).
pub type SketchOffsetOpKey = crate::arena::Key<SketchOffsetOperation>;

/// A 2D in-sketch mirror (Mirror tool inside a sketch, #523): reflects the picked lines and
/// circles across a mirror line, emitting the reflections as separate `Line`/`Circle` entries
/// grouped under the op and regenerated whenever the sources or the mirror line change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchMirrorOperation {
    /// The sketch the sources live in; outputs land in the same sketch.
    pub sketch: SketchId,
    /// The mirror line: a straight sketch line whose infinite extension is the mirror axis.
    pub line: LineKey,
    /// Source line indices to reflect.
    #[serde(default)]
    pub line_targets: Vec<LineKey>,
    /// Source circle indices to reflect.
    #[serde(default)]
    pub circle_targets: Vec<CircleKey>,
    /// Generated line indices, aligned with `line_targets`.
    #[serde(default)]
    pub line_outputs: Vec<LineKey>,
    /// Generated circle indices, aligned with `circle_targets`.
    #[serde(default)]
    pub circle_outputs: Vec<CircleKey>,
    /// Generated coincidence-constraint indices reflecting the sources' shared corners onto the
    /// outputs (#547), so a mirrored polygon's reflected edges join into a fillable face.
    /// Tombstoned and regenerated on every rebuild, like the output geometry.
    #[serde(default)]
    pub constraint_outputs: Vec<ConstraintKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a SketchMirrorOperation (#1055).
pub type SketchMirrorOpKey = crate::arena::Key<SketchMirrorOperation>;

/// A 2D in-sketch slice (#224): splits the target sketch **lines** at their interior crossings
/// with the cutter lines, shadowing each split original and emitting its fragments as new lines
/// in the same sketch, grouped under the operation. The sketch-space analogue of the 3D
/// [`SliceOperation`] — shadowed originals behave like shadow bodies (kept for editing, excluded
/// from face detection). Curve and face targets are a tracked follow-up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchSliceOperation {
    /// The sketch the operands live in.
    pub sketch: SketchId,
    /// Target line indices (the A side); each is split where the cutters cross it.
    #[serde(default)]
    pub line_targets: Vec<LineKey>,
    /// Cutter line indices (the B side); interior crossings with these divide each target.
    #[serde(default)]
    pub cutter_lines: Vec<LineKey>,
    /// Target circle indices (#237); each is split into arcs where the cutters cross it. The arcs
    /// are emitted as curved (bezier) fragment lines, the source circle is shadowed.
    #[serde(default)]
    pub circle_targets: Vec<CircleKey>,
    /// Target **face** loops (#238): each entry is the line indices of a closed sketch face to
    /// slice. The cutter is expected to cross the loop's boundary at two points; the two crossed
    /// boundary edges are split, a cut **chord** is emitted between the crossings, and coincidence
    /// constraints are generated so the loop resolves into two faces (see `rebuild_sketch_slice`).
    #[serde(default)]
    pub face_targets: Vec<Vec<LineKey>>,
    /// Generated fragment-line indices, target-major (all fragments of target 0, then target 1…).
    /// Both split lines *and* split-circle arcs land here (arcs are bezier `Line`s); face-slice
    /// boundary fragments and cut chords land here too.
    #[serde(default)]
    pub line_outputs: Vec<LineKey>,
    /// Generated coincidence-constraint indices (#238) that stitch a face slice's fragments into
    /// two loops. Tombstoned and regenerated on every rebuild, like `line_outputs`.
    #[serde(default)]
    pub constraint_outputs: Vec<ConstraintKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a SketchSliceOperation (#1055).
pub type SketchSliceOpKey = crate::arena::Key<SketchSliceOperation>;

/// One treated corner owned by a [`SketchVertexTreatmentOperation`] (#538): the two edges that
/// meet at a sketch vertex, addressed by their position in the op's `line_targets` and which end
/// of each edge sits at the vertex, plus the chamfer/fillet kind and a parametric amount.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchVertexTreatmentCorner {
    /// Index INTO the op's `line_targets` of the first edge, and which end meets the vertex.
    pub a: usize,
    pub a_end: LineEnd,
    pub b: usize,
    pub b_end: LineEnd,
    pub kind: VertexTreatmentKind,
    /// Chamfer distance / fillet radius expression (mm), parametric.
    pub amount: String,
}

/// A 2D in-sketch chamfer/fillet as a parametric operation (#538): the source edges are
/// shadowed and kept solving (so their dimensions stay, referencing the virtual sharp
/// corner); the rebuild reads their solved endpoints and regenerates one trimmed copy per
/// source edge plus one bridge per corner, stitched into a closed loop. One op owns a
/// connected treated region (many corners), like the 3D [`EdgeTreatmentOperation`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchVertexTreatmentOperation {
    pub sketch: SketchId,
    /// Source edge indices (shadowed), deduped; corners reference these by position.
    #[serde(default)]
    pub line_targets: Vec<LineKey>,
    #[serde(default)]
    pub corners: Vec<SketchVertexTreatmentCorner>,
    /// Generated trimmed copies, index-aligned with `line_targets` (output i is the trimmed
    /// copy of source line_targets[i]). Regenerated each rebuild; reuse slots when possible.
    #[serde(default)]
    pub line_outputs: Vec<LineKey>,
    /// Generated bridge lines, index-aligned with `corners`.
    #[serde(default)]
    pub bridge_outputs: Vec<LineKey>,
    /// Generated stitch coincidence constraints; removed and regenerated each rebuild.
    #[serde(default)]
    pub constraint_outputs: Vec<ConstraintKey>,
    #[serde(default)]
    pub name: Option<String>,
}

/// How anything names a SketchVertexTreatmentOperation (#1055).
pub type SketchVertexTreatmentOpKey = crate::arena::Key<SketchVertexTreatmentOperation>;

/// A text element placed in a sketch (#282). The glyph outlines are **baked** at create/edit time
/// into `contours` (sketch-local mm, laid out from a baseline at the local origin, *before* the
/// element's `origin`/`rotation` transform) and the source font is embedded (`font_bytes`, base64
/// in JSON) so the text renders identically on a machine that lacks the font — like a PDF. The
/// outlines are what render and extrude; the string/font/size are kept so it can be re-baked when
/// edited. Contours include both outer loops and counters (holes); callers separate them by
/// winding/containment (`text::contour_signed_area`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchText {
    pub sketch: SketchId,
    pub text: String,
    pub font_family: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    /// Evaluated font size in mm; `size_expr` is the source (may reference parameters).
    pub size: f32,
    #[serde(default)]
    pub size_expr: String,
    /// Baseline start in sketch-local coords, before rotation.
    pub origin: (f32, f32),
    /// Rotation about `origin`, radians (also settable with the Move tool, #282d).
    #[serde(default)]
    pub rotation: f32,
    /// Optional wrap width (mm); when set, text wraps to this width and grows downward.
    #[serde(default)]
    pub wrap_width: Option<f32>,
    /// Text-on-curve groundwork (#286): the sketch line/curve the baseline follows. `None` means
    /// a straight baseline from `origin` (rotated by `rotation`), which is all baking supports
    /// today — [`crate::text::outline_text`] advances a pen along a straight baseline. Curve
    /// support later means resolving this into a baseline provider (arc-length position +
    /// tangent per pen offset) at bake time; the stored model won't need to change shape.
    #[serde(default)]
    pub baseline_line: Option<usize>,
    /// Baked glyph contours (sketch-local mm, baseline-relative, pre-transform).
    #[serde(default)]
    pub contours: Vec<Vec<(f32, f32)>>,
    /// Embedded source font bytes (base64 in JSON) for reproducible rendering.
    #[serde(default, with = "font_bytes_base64")]
    pub font_bytes: Vec<u8>,
    /// Legacy position pin (#356, removed by #408): retained only so old documents
    /// deserialize; converted to a `Coincident` constraint on load and never written back.
    #[serde(default, skip_serializing)]
    pub pin: Option<(ConstraintPoint, TextAnchor)>,
    #[serde(default)]
    pub name: Option<String>,
}

/// A sketch text's identity (#1055): stable across deletions of other texts.
pub type SketchTextKey = crate::arena::Key<SketchText>;

/// Serde codec storing [`SketchText::font_bytes`] as base64 (same rationale as the tracing-image
/// codec — raw byte arrays would bloat the JSON 4x).
mod font_bytes_base64 {
    use base64::Engine as _;

    pub fn serialize<S: serde::Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text: String = serde::Deserialize::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// How anything names a tracing image (#1055): a key, so deleting one never slides another
/// image into the plane, calibration point or move target the deleted one held.
pub type TracingImageKey = crate::arena::Key<TracingImage>;

/// A reference image imported for tracing (#163/#169), hosted on a construction plane.
/// The encoded file bytes are embedded (base64 in the saved JSON) so documents stay
/// self-contained, like imported meshes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TracingImage {
    /// Original encoded file bytes (PNG or JPEG).
    #[serde(with = "tracing_image_bytes")]
    pub bytes: Vec<u8>,
    /// Source file name (without extension), used as the default display name.
    pub source_name: String,
    /// Host construction plane index; the image lies in that plane.
    pub plane: ConstructionPlaneKey,
    /// Image lower-left corner in plane-local mm.
    pub origin: (f32, f32),
    /// Authored lower-left before any Move op (#217). `None` = no move applied, so `origin`
    /// itself is the base. Set when the image first becomes a move target, so editing a move
    /// op recomputes `origin` from a pristine base — the same base/cache split construction
    /// planes have between `definition` and their cached frame.
    #[serde(default)]
    pub base_origin: Option<(f32, f32)>,
    /// Displayed size in mm. Import seeds 1 px = 1 mm; calibration (#171) rescales.
    pub width_mm: f32,
    pub height_mm: f32,
    #[serde(default)]
    pub name: Option<String>,
    /// Last applied scale calibration (#171), kept for re-editing: the reference segment in
    /// image-UV space (0..1 across the displayed quad) and the real length it was assigned.
    #[serde(default)]
    pub calibration: Option<ImageCalibration>,
}

/// A tracing image's scale calibration (#171).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageCalibration {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub length_mm: f32,
}

/// Serde codec storing [`TracingImage::bytes`] as base64 (JSON documents would otherwise
/// encode each byte as a number — 4x the size).
mod tracing_image_bytes {
    use base64::Engine as _;

    pub fn serialize<S: serde::Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text: String = serde::Deserialize::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// A solid mesh brought in via file import (STL, #70), stored as-is (no scaling/centering)
/// in the document's coordinate space. Backs a `Body` via `BodySource::Imported`.
///
/// When the import was a real STEP BREP (#1029), `step_bytes` keeps the file so booleans
/// and other kernel ops can re-read the solid. Pure mesh imports (STL, faceted-only STEP)
/// leave it empty — they stay triangle-only.
/// How a body names the mesh it was imported from (#1055).
pub type ImportedMeshKey = crate::arena::Key<ImportedMesh>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ImportedMesh {
    pub triangles: Vec<[glam::Vec3; 3]>,
    /// Source file name (without extension), used as the default body name.
    pub source_name: String,
    /// Original STEP content when the import came from a BREP file (#1029).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_bytes: Option<Vec<u8>>,
}

/// Which sketch primitive was created, in chronological order (for undo).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Sketch,
    Line,
    Circle,
    Parameter,
    Constraint,
    ConstructionPlane,
    Extrusion,
    Body,
    /// A tracing image import (#169).
    Image,
    /// A loft feature (its body is a separate `Body` entry).
    Loft,
    Revolution,
    /// A primitive shape (#909); its body is a separate `Body` entry.
    Primitive,
    /// A sweep (its body is a separate `Body` entry).
    Sweep,
    /// A boolean operation between bodies (its output bodies are separate `Body` entries).
    BooleanOperation,
    /// A move operation on bodies (its output bodies are separate `Body` entries).
    MoveOperation,
    /// A mirror operation on bodies (#523): its reflected output bodies are separate
    /// `Body` entries; the originals are kept.
    MirrorOperation,
    /// A linear repeat on bodies (its output bodies are separate `Body` entries).
    RepeatOperation,
    /// A slice operation on bodies (its fragment bodies are separate `Body` entries).
    SliceOperation,
    /// A shell operation on bodies (#1156): its hollowed output bodies are separate
    /// `Body` entries; the originals become shadow bodies.
    ShellOperation,
    /// An edge chamfer/fillet operation on bodies (#531): its beveled output bodies are
    /// separate `Body` entries; the originals become shadow bodies.
    EdgeTreatmentOperation,
    /// A 2D in-sketch linear repeat (#222): its duplicated lines/circles are separate
    /// `Line`/`Circle` entries.
    SketchRepeatOperation,
    /// A 2D in-sketch slice (#224): its fragment lines are separate `Line` entries.
    SketchSliceOperation,
    /// A 2D in-sketch offset: its parallel lines/circles are separate entries.
    SketchOffsetOperation,
    /// A 2D in-sketch mirror (#523): its reflected lines/circles are separate entries.
    SketchMirrorOperation,
    /// A 2D in-sketch chamfer/fillet (#538): its trimmed copies + bridge lines are separate
    /// `Line` entries; the source edges are shadowed and kept.
    SketchVertexTreatmentOperation,
    /// A sketch text element (#282): baked glyph outlines + embedded font.
    SketchText,
    /// A joint between parts (#891): a kinematic relationship, no output bodies.
    Joint,
    /// An in-place edit of an existing construction plane (undo restores the prior planes).
    /// Transient: never persisted (storage rebuilds `shape_order` from created shapes only).
    ConstructionPlaneEdit,
    /// An in-place 3D chamfer/fillet commit (#168): undo restores the extrusion's prior
    /// `edge_treatments` list from the snapshot stack. Transient, like
    /// [`ShapeKind::ConstructionPlaneEdit`].
    EdgeTreatmentEdit,
}

/// A diagonal "edge" view (#339): looking square at one of the cube's twelve edges — the view you
/// get by clicking an edge on the navigation bear. Each edge sits between two orthographic faces;
/// its basis is derived from theirs (see [`DrawingOrientation::view_axes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeView {
    FrontRight,
    BackRight,
    BackLeft,
    FrontLeft,
    FrontTop,
    RightTop,
    BackTop,
    LeftTop,
    FrontBottom,
    RightBottom,
    BackBottom,
    LeftBottom,
}

impl EdgeView {
    pub const ALL: &'static [EdgeView] = &[
        EdgeView::FrontRight,
        EdgeView::BackRight,
        EdgeView::BackLeft,
        EdgeView::FrontLeft,
        EdgeView::FrontTop,
        EdgeView::RightTop,
        EdgeView::BackTop,
        EdgeView::LeftTop,
        EdgeView::FrontBottom,
        EdgeView::RightBottom,
        EdgeView::BackBottom,
        EdgeView::LeftBottom,
    ];

    /// The two orthographic faces this edge lies between; its view basis is their average.
    pub fn faces(self) -> (DrawingOrientation, DrawingOrientation) {
        use DrawingOrientation as O;
        match self {
            EdgeView::FrontRight => (O::Front, O::Right),
            EdgeView::BackRight => (O::Back, O::Right),
            EdgeView::BackLeft => (O::Back, O::Left),
            EdgeView::FrontLeft => (O::Front, O::Left),
            EdgeView::FrontTop => (O::Front, O::Top),
            EdgeView::RightTop => (O::Right, O::Top),
            EdgeView::BackTop => (O::Back, O::Top),
            EdgeView::LeftTop => (O::Left, O::Top),
            EdgeView::FrontBottom => (O::Front, O::Bottom),
            EdgeView::RightBottom => (O::Right, O::Bottom),
            EdgeView::BackBottom => (O::Back, O::Bottom),
            EdgeView::LeftBottom => (O::Left, O::Bottom),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EdgeView::FrontRight => "Front-Right",
            EdgeView::BackRight => "Back-Right",
            EdgeView::BackLeft => "Back-Left",
            EdgeView::FrontLeft => "Front-Left",
            EdgeView::FrontTop => "Front-Top",
            EdgeView::RightTop => "Right-Top",
            EdgeView::BackTop => "Back-Top",
            EdgeView::LeftTop => "Left-Top",
            EdgeView::FrontBottom => "Front-Bottom",
            EdgeView::RightBottom => "Right-Bottom",
            EdgeView::BackBottom => "Back-Bottom",
            EdgeView::LeftBottom => "Left-Bottom",
        }
    }

    /// Script/name spelling, e.g. `"front-right"`.
    pub fn name(self) -> &'static str {
        match self {
            EdgeView::FrontRight => "front-right",
            EdgeView::BackRight => "back-right",
            EdgeView::BackLeft => "back-left",
            EdgeView::FrontLeft => "front-left",
            EdgeView::FrontTop => "front-top",
            EdgeView::RightTop => "right-top",
            EdgeView::BackTop => "back-top",
            EdgeView::LeftTop => "left-top",
            EdgeView::FrontBottom => "front-bottom",
            EdgeView::RightBottom => "right-bottom",
            EdgeView::BackBottom => "back-bottom",
            EdgeView::LeftBottom => "left-bottom",
        }
    }
}

/// A three-quarter "corner" view (#344): looking square at one of the cube's eight corners — the
/// view you get by clicking a corner on the navigation bear. Each corner meets three faces; its
/// basis is the average of theirs (see [`DrawingOrientation::view_axes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CornerView {
    FrontLeftBottom,
    FrontRightBottom,
    BackRightBottom,
    BackLeftBottom,
    FrontLeftTop,
    FrontRightTop,
    BackRightTop,
    BackLeftTop,
}

impl CornerView {
    pub const ALL: &'static [CornerView] = &[
        CornerView::FrontLeftBottom,
        CornerView::FrontRightBottom,
        CornerView::BackRightBottom,
        CornerView::BackLeftBottom,
        CornerView::FrontLeftTop,
        CornerView::FrontRightTop,
        CornerView::BackRightTop,
        CornerView::BackLeftTop,
    ];

    /// The three orthographic faces this corner meets; its view basis is their average.
    pub fn faces(self) -> (DrawingOrientation, DrawingOrientation, DrawingOrientation) {
        use DrawingOrientation as O;
        match self {
            CornerView::FrontLeftBottom => (O::Front, O::Left, O::Bottom),
            CornerView::FrontRightBottom => (O::Front, O::Right, O::Bottom),
            CornerView::BackRightBottom => (O::Back, O::Right, O::Bottom),
            CornerView::BackLeftBottom => (O::Back, O::Left, O::Bottom),
            CornerView::FrontLeftTop => (O::Front, O::Left, O::Top),
            CornerView::FrontRightTop => (O::Front, O::Right, O::Top),
            CornerView::BackRightTop => (O::Back, O::Right, O::Top),
            CornerView::BackLeftTop => (O::Back, O::Left, O::Top),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CornerView::FrontLeftBottom => "Front-Left-Bottom",
            CornerView::FrontRightBottom => "Front-Right-Bottom",
            CornerView::BackRightBottom => "Back-Right-Bottom",
            CornerView::BackLeftBottom => "Back-Left-Bottom",
            CornerView::FrontLeftTop => "Front-Left-Top",
            CornerView::FrontRightTop => "Front-Right-Top",
            CornerView::BackRightTop => "Back-Right-Top",
            CornerView::BackLeftTop => "Back-Left-Top",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            CornerView::FrontLeftBottom => "front-left-bottom",
            CornerView::FrontRightBottom => "front-right-bottom",
            CornerView::BackRightBottom => "back-right-bottom",
            CornerView::BackLeftBottom => "back-left-bottom",
            CornerView::FrontLeftTop => "front-left-top",
            CornerView::FrontRightTop => "front-right-top",
            CornerView::BackRightTop => "back-right-top",
            CornerView::BackLeftTop => "back-left-top",
        }
    }
}

/// The orientation a body is projected from in a technical drawing view (#180). The six
/// orthographic "straight-on" directions, an isometric three-quarter view, the twelve diagonal
/// edge views (#339), the eight corner three-quarter views (#344), plus a free angle (#345).
/// (No `Eq`/`Hash`: the `Free` basis holds floats.)
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum DrawingOrientation {
    #[default]
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Isometric,
    /// A diagonal edge view (#339).
    Edge(EdgeView),
    /// A corner three-quarter view (#344).
    Corner(CornerView),
    /// A free (arbitrary) viewing angle (#345): the projection's `(right, up)` basis is stored
    /// directly, set by spinning the orientation widget rather than picking a preset.
    Free { right: [f32; 3], up: [f32; 3] },
}

impl DrawingOrientation {
    pub const ALL: &'static [DrawingOrientation] = &[
        DrawingOrientation::Front,
        DrawingOrientation::Back,
        DrawingOrientation::Left,
        DrawingOrientation::Right,
        DrawingOrientation::Top,
        DrawingOrientation::Bottom,
        DrawingOrientation::Isometric,
        DrawingOrientation::Edge(EdgeView::FrontRight),
        DrawingOrientation::Edge(EdgeView::BackRight),
        DrawingOrientation::Edge(EdgeView::BackLeft),
        DrawingOrientation::Edge(EdgeView::FrontLeft),
        DrawingOrientation::Edge(EdgeView::FrontTop),
        DrawingOrientation::Edge(EdgeView::RightTop),
        DrawingOrientation::Edge(EdgeView::BackTop),
        DrawingOrientation::Edge(EdgeView::LeftTop),
        DrawingOrientation::Edge(EdgeView::FrontBottom),
        DrawingOrientation::Edge(EdgeView::RightBottom),
        DrawingOrientation::Edge(EdgeView::BackBottom),
        DrawingOrientation::Edge(EdgeView::LeftBottom),
        DrawingOrientation::Corner(CornerView::FrontLeftBottom),
        DrawingOrientation::Corner(CornerView::FrontRightBottom),
        DrawingOrientation::Corner(CornerView::BackRightBottom),
        DrawingOrientation::Corner(CornerView::BackLeftBottom),
        DrawingOrientation::Corner(CornerView::FrontLeftTop),
        DrawingOrientation::Corner(CornerView::FrontRightTop),
        DrawingOrientation::Corner(CornerView::BackRightTop),
        DrawingOrientation::Corner(CornerView::BackLeftTop),
    ];

    pub fn label(self) -> &'static str {
        match self {
            DrawingOrientation::Front => "Front",
            DrawingOrientation::Back => "Back",
            DrawingOrientation::Left => "Left",
            DrawingOrientation::Right => "Right",
            DrawingOrientation::Top => "Top",
            DrawingOrientation::Bottom => "Bottom",
            DrawingOrientation::Edge(e) => e.label(),
            DrawingOrientation::Corner(c) => c.label(),
            DrawingOrientation::Isometric => "Isometric",
            DrawingOrientation::Free { .. } => "Free angle",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "front" => Some(DrawingOrientation::Front),
            "back" | "rear" => Some(DrawingOrientation::Back),
            "left" => Some(DrawingOrientation::Left),
            "right" => Some(DrawingOrientation::Right),
            "top" => Some(DrawingOrientation::Top),
            "bottom" => Some(DrawingOrientation::Bottom),
            "isometric" | "iso" | "diagonal" => Some(DrawingOrientation::Isometric),
            other => EdgeView::ALL
                .iter()
                .find(|e| e.name() == other)
                .map(|e| DrawingOrientation::Edge(*e))
                .or_else(|| {
                    CornerView::ALL
                        .iter()
                        .find(|c| c.name() == other)
                        .map(|c| DrawingOrientation::Corner(*c))
                }),
        }
    }
}

/// One view on a technical [`Drawing`] (#180): one or more bodies (or a sketch) projected in a
/// fixed orientation. Multi-body projections hold an assembly in one card (#1190/#1191).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingView {
    /// Bodies this view projects when `sketch` is `None`. Empty for sketch-only views.
    /// One body is the usual case; several come from shift-clicking more bodies onto the same
    /// projection (#1191) or adding a whole component (#1190).
    #[serde(default)]
    pub bodies: Vec<BodyKey>,
    /// When `Some`, this view projects a **sketch** rather than bodies (#278). Kept as an
    /// optional field (rather than replacing `bodies` with an enum) so older saved drawings —
    /// which only ever had a body — deserialize with a custom path if needed.
    #[serde(default)]
    pub sketch: Option<SketchId>,
    #[serde(default)]
    pub orientation: DrawingOrientation,
    /// Placement centre on the page, as a fraction of the page (0..1) from the top-left (#274).
    /// Defaults to the page centre; the pane cascades new placements so they don't fully stack.
    #[serde(default = "default_view_pos")]
    pub pos_x: f32,
    #[serde(default = "default_view_pos")]
    pub pos_y: f32,
    /// Card size as a fraction of the page (#1207). Width and height are independent; aligned
    /// views share the matching axis (Above/Below share `size_x`, Left/Right share `size_y`).
    /// Defaults to 0.42 — the historical fixed card size.
    #[serde(default = "default_view_size")]
    pub size_x: f32,
    #[serde(default = "default_view_size")]
    pub size_y: f32,
    /// Body edges whose length dimension is shown, keyed by their quantized world endpoints
    /// (order-normalized, smaller endpoint first) — a geometry identity that survives
    /// rebuilds, like [`crate::hierarchy::SceneElement::BodyEdge`]. A new view starts with
    /// every edge dimensioned (#299); edge clicks toggle them off from there.
    #[serde(default)]
    pub dimensioned_edges: Vec<([i32; 3], [i32; 3])>,
    /// Pairs of edges whose **angle** is shown (#180), each edge a quantized-endpoint key like
    /// `dimensioned_edges`; the pair itself is order-normalized.
    #[serde(default)]
    pub angle_dims: Vec<(([i32; 3], [i32; 3]), ([i32; 3], [i32; 3]))>,
    /// Per-edge dimension-label offset overrides (#294), keyed by the same edge key as
    /// `dimensioned_edges`. The value is the label's signed offset (mm, in projected view
    /// space) along the edge's outward perpendicular from the geometry centroid — a positive
    /// value pushes the label further out. Absent → the auto-placed default distance. A drag
    /// writes an override here; it survives rebuilds because the key is geometry-based.
    #[serde(default)]
    pub dimension_offsets: Vec<(([i32; 3], [i32; 3]), f32)>,
    /// Detected circles (holes, cylinders) whose **diameter** dimension is shown, keyed by the
    /// circle's quantized world centre (#342). Like `dimensioned_edges`, a new view starts empty
    /// and "Show all dimensions" populates it; "Hide all" clears it, so a circle's Ø dimension is
    /// toggleable rather than always drawn.
    #[serde(default)]
    pub dimensioned_circles: Vec<[i32; 3]>,
    /// Per-circle Ø-label offset overrides (#397), keyed like `dimensioned_circles`. For a
    /// face-on circle the offset slides the label off the diameter line along its
    /// perpendicular (up/down for the default horizontal line); for an edge-on circle it
    /// pushes the linear dimension further out, like `dimension_offsets`. Projected mm.
    #[serde(default)]
    pub circle_dim_offsets: Vec<([i32; 3], f32)>,
    /// Print scale as `"page:model"` text, e.g. `"1:20"` (#300). Always stored validated
    /// (see [`parse_drawing_scale`]); `None` auto-fits the projection to its card.
    #[serde(default)]
    pub scale: Option<String>,
    /// How the projection is drawn (#301): hidden lines removed, full wireframe, or shaded.
    #[serde(default)]
    pub style: DrawingViewStyle,
    /// Aligned child projection (#296): the index of the parent view this one derives from,
    /// and the direction it was placed relative to it. While set, the child stays lined up
    /// with the parent along their shared axis (the child only slides along the other axis),
    /// and it inherits the parent's scale.
    #[serde(default)]
    pub aligned_parent: Option<usize>,
    #[serde(default)]
    pub aligned_dir: Option<AlignDir>,
    /// Draw dashed projection lines from the base view to this aligned child (#377/#1206): two
    /// lightweight lines connecting the facing body edges at each outer silhouette extreme
    /// across the gap. Only meaningful while `aligned_parent` is set.
    #[serde(default)]
    pub align_lines: bool,
    /// Hide the view's caption label on the page and in exports (#372).
    #[serde(default)]
    pub label_hidden: bool,
    /// Where the caption label sits within the view's card (#372).
    #[serde(default)]
    pub label_pos: DrawingLabelPos,
    /// Custom caption template (#372): `None` uses the automatic
    /// "Source — Orientation (scale)" text. Like any label it may embed `{expression}`
    /// interpolation fields (#338), resolved against the document's parameters.
    #[serde(default)]
    pub label_text: Option<String>,
}

impl DrawingView {
    /// A body (or multi-body) projection with default card placement and no dimensions shown.
    pub fn from_bodies(bodies: Vec<BodyKey>, orientation: DrawingOrientation) -> Self {
        Self {
            bodies,
            sketch: None,
            orientation,
            dimensioned_edges: Vec::new(),
            angle_dims: Vec::new(),
            dimension_offsets: Vec::new(),
            dimensioned_circles: Vec::new(),
            circle_dim_offsets: Vec::new(),
            aligned_parent: None,
            aligned_dir: None,
            align_lines: false,
            label_hidden: false,
            label_pos: DrawingLabelPos::default(),
            label_text: None,
            pos_x: default_view_pos(),
            pos_y: default_view_pos(),
            size_x: default_view_size(),
            size_y: default_view_size(),
            scale: None,
            style: DrawingViewStyle::default(),
        }
    }

    /// A sketch projection (#278).
    pub fn from_sketch(sketch: SketchId, orientation: DrawingOrientation) -> Self {
        Self {
            bodies: Vec::new(),
            sketch: Some(sketch),
            orientation,
            dimensioned_edges: Vec::new(),
            angle_dims: Vec::new(),
            dimension_offsets: Vec::new(),
            dimensioned_circles: Vec::new(),
            circle_dim_offsets: Vec::new(),
            aligned_parent: None,
            aligned_dir: None,
            align_lines: false,
            label_hidden: false,
            label_pos: DrawingLabelPos::default(),
            label_text: None,
            pos_x: default_view_pos(),
            pos_y: default_view_pos(),
            size_x: default_view_size(),
            size_y: default_view_size(),
            scale: None,
            style: DrawingViewStyle::default(),
        }
    }
}

/// Where a drawing view's caption label sits within its card (#372).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawingLabelPos {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl DrawingLabelPos {
    /// All positions in grid order: the top row, then the bottom row.
    pub const ALL: [DrawingLabelPos; 6] = [
        DrawingLabelPos::TopLeft,
        DrawingLabelPos::TopCenter,
        DrawingLabelPos::TopRight,
        DrawingLabelPos::BottomLeft,
        DrawingLabelPos::BottomCenter,
        DrawingLabelPos::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopCenter => "Top center",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomCenter => "Bottom center",
            Self::BottomRight => "Bottom right",
        }
    }

    /// Stable scripting name (`bearcad.drawing_view_label{ pos = … }`).
    pub fn name(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopCenter => "top-center",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomCenter => "bottom-center",
            Self::BottomRight => "bottom-right",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|p| p.name() == name)
    }
}

/// Where an aligned child projection sits relative to its parent (#296). The name is the
/// screen direction the mouse moved to create it (which also names the resulting view for a
/// Front parent: down → Bottom, up → Top, right → Right, left → Left).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignDir {
    Below,
    Above,
    Right,
    Left,
}

impl AlignDir {
    /// The child is aligned **vertically** with its parent (shares the horizontal `pos_x`)
    /// when placed above/below; **horizontally** (shares `pos_y`) when placed left/right.
    pub fn shares_pos_x(self) -> bool {
        matches!(self, AlignDir::Below | AlignDir::Above)
    }
}

/// How a drawing view renders its body (#301).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrawingViewStyle {
    /// Only the edges visible from the view direction (hidden lines removed).
    Visible,
    /// Every feature edge, including back edges.
    #[default]
    Wireframe,
    /// Grey-shaded faces with the visible edges on top.
    Shaded,
}

impl DrawingViewStyle {
    pub const ALL: [DrawingViewStyle; 3] = [
        DrawingViewStyle::Visible,
        DrawingViewStyle::Wireframe,
        DrawingViewStyle::Shaded,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Visible => "Visible edges",
            Self::Wireframe => "Wireframe",
            Self::Shaded => "Shaded",
        }
    }
}

/// Parse a drawing-view scale like `"1:20"` or `"2:3"` into page-mm per model-mm (#300):
/// `a:b` means `a` page units represent `b` model units, so the factor is `a / b`. `None`
/// for anything that isn't two positive numbers around a colon.
pub fn parse_drawing_scale(text: &str) -> Option<f32> {
    let (a, b) = text.trim().split_once(':')?;
    let a: f32 = a.trim().parse().ok()?;
    let b: f32 = b.trim().parse().ok()?;
    (a > 0.0 && b > 0.0 && a.is_finite() && b.is_finite()).then(|| a / b)
}

/// A quantized body-edge key: a pair of quantized world endpoints, order-normalized so the
/// two endpoints compare equal regardless of which was clicked first (#180).
pub type DrawingEdgeKey = ([i32; 3], [i32; 3]);

/// Order-normalize an edge's two quantized endpoints (smaller first).
pub fn normalized_edge_key(a: [i32; 3], b: [i32; 3]) -> DrawingEdgeKey {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// A technical drawing (#180): a black-on-white sheet showing one or more body views for
/// print/PDF output. It references bodies but produces no solid geometry of its own, so it
/// lives outside the shape/undo-group DAG (undo is snapshot-based, #194).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub views: Vec<DrawingView>,
    /// Page width and height in millimetres (#273). Default: landscape US Letter (11 x 8.5 in).
    #[serde(default = "default_page_width_mm")]
    pub page_width_mm: f32,
    #[serde(default = "default_page_height_mm")]
    pub page_height_mm: f32,
    /// Uniform page margin in millimetres (#273). Default: 0.5 in.
    #[serde(default = "default_page_margin_mm")]
    pub margin_mm: f32,
    /// Free text annotations placed on the page (#312): notes, titles, callouts.
    #[serde(default)]
    pub annotations: crate::arena::Arena<DrawingAnnotation>,
}

/// A drawing's identity (#1055): stable across deletions of other drawings.
pub type DrawingKey = crate::arena::Key<Drawing>;

/// A free text annotation on a drawing page (#312). Positions and sizes are page-relative
/// fractions so they stay put across page-size changes and render identically at any zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingAnnotation {
    pub text: String,
    /// Top-left of the text block, as a fraction of the page (0..1) from the top-left.
    pub pos_x: f32,
    pub pos_y: f32,
    /// Font size as a fraction of the page height (so it scales with the sheet). ~0.025 default.
    #[serde(default = "default_annotation_size")]
    pub size_frac: f32,
    /// Optional wrap width as a fraction of page width; `None` is a single growing line (#312).
    #[serde(default)]
    pub wrap_frac: Option<f32>,
}

/// An annotation's identity (#1055): stable across deletions of other annotations on the page.
pub type AnnotationKey = crate::arena::Key<DrawingAnnotation>;

fn default_annotation_size() -> f32 {
    0.025
}

fn default_page_width_mm() -> f32 {
    11.0 * 25.4
}
fn default_page_height_mm() -> f32 {
    8.5 * 25.4
}
fn default_page_margin_mm() -> f32 {
    0.5 * 25.4
}
fn default_view_pos() -> f32 {
    0.5
}
/// Default card size fraction — historically the fixed 0.42 of the page (#1207).
pub fn default_view_size() -> f32 {
    0.42
}
/// Smallest allowed card size fraction (keeps a corner grip grabbable).
pub const MIN_VIEW_SIZE_FRAC: f32 = 0.05;

impl Default for Drawing {
    fn default() -> Self {
        Self {
            name: None,
            views: Vec::new(),
            page_width_mm: default_page_width_mm(),
            page_height_mm: default_page_height_mm(),
            margin_mm: default_page_margin_mm(),
            annotations: crate::arena::Arena::new(),
        }
    }
}

/// Where an imported unit's source document lives (#719).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitSource {
    /// A path relative to the importing document's own file.
    RelativePath(String),
    /// A path under the app's library directory (#720).
    Library(String),
}

/// Whether an imported unit follows its source file (#719).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkMode {
    /// The embedded copy is frozen: updates to the source file are not seen.
    #[default]
    Static,
    /// The embedded copy syncs from the source file when it changes (#732).
    Dynamic,
}

/// An imported BearCAD document (#719): one embedded copy of the source, shared by every
/// [`UnitInstance`] that places it. The importing document is self-contained — it opens
/// and rebuilds with the source file absent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedUnit {
    pub source: UnitSource,
    #[serde(default)]
    pub link: LinkMode,
    /// The embedded copy of the source document (recursive — it may hold units of its
    /// own, capped at [`MAX_UNIT_DEPTH`]).
    pub document: Document,
    /// The source file's modification time (**nanoseconds** since the Unix epoch — save
    /// bursts land inside one second) when the copy was last synced; the cheap first
    /// staleness check.
    #[serde(default)]
    pub source_mtime: Option<i64>,
    /// [`content_hash`] of the source file's bytes when the copy was last synced; the
    /// authoritative staleness check (mtimes lie across copies and checkouts).
    #[serde(default)]
    pub source_hash: Option<u64>,
}

/// An imported unit's identity (#1055): stable across deletions of other units.
pub type UnitKey = crate::arena::Key<ImportedUnit>;

/// One placement of an [`ImportedUnit`] (#719). Ten instances of A cost one embedded copy
/// of A plus ten of these.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitInstance {
    /// The embedded copy this places.
    pub unit: UnitKey,
    /// The instance name used in qualified expression references (`name.param`, #729).
    #[serde(default)]
    pub name: Option<String>,
    /// `(parameter name, expression)`, only where this instance differs from the unit's
    /// own value.
    #[serde(default)]
    pub parameter_overrides: Vec<(String, String)>,
    /// Where the instance sits in this document's world space.
    #[serde(default)]
    pub placement: UnitPlacement,
}

/// A unit instance's identity (#1055): stable across deletions of other instances.
pub type UnitInstanceKey = crate::arena::Key<UnitInstance>;

/// A [`UnitInstance`]'s placement (#719): a rotation about an axis through the unit's
/// origin, then a translation. Identity by default. Every field is `#[serde(default)]`,
/// so a future **scale** (#735: instances will be scalable) slots in without a format
/// change.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UnitPlacement {
    /// Translation components (mm expressions; empty = 0), like the Move tool's.
    #[serde(default)]
    pub tx: String,
    #[serde(default)]
    pub ty: String,
    #[serde(default)]
    pub tz: String,
    /// Rotation axis direction (need not be normalized); zero = no rotation.
    #[serde(default)]
    pub axis: [f32; 3],
    /// Rotation angle about `axis` (degree expression; empty = 0).
    #[serde(default)]
    pub angle: String,
}

/// Stable content hash for unit staleness checks (#719): FNV-1a 64 over the file bytes.
/// Not cryptographic — it only answers "did the source change since we copied it".
#[allow(dead_code)] // consumed by the import command (#721) and sync (#732)
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// Hard cap on unit nesting (#719): loading or importing a document nested deeper than
/// this fails with a clear error instead of recursing unboundedly.
pub const MAX_UNIT_DEPTH: usize = 8;

/// Reject over-deep nesting and import cycles among a document's units (#719), matched on
/// resolved source path. `own_path` is the document's file path when known (native open);
/// relative sources then resolve against its directory, so "A imports B imports A" is
/// caught however the two files spell the paths. Without it (web, in-memory bytes),
/// sources still resolve lexically relative to each other, catching structural cycles.
pub fn validate_units(doc: &Document, own_path: Option<&std::path::Path>) -> Result<(), String> {
    use std::path::{Component, Path, PathBuf};

    /// Lexical normalization only — the source file may legitimately be absent, so no
    /// filesystem access: fold `.`, pop `..` where a normal component precedes it.
    fn normalize(path: &Path) -> String {
        let mut out = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    let can_pop =
                        matches!(out.components().next_back(), Some(Component::Normal(_)));
                    if !can_pop || !out.pop() {
                        out.push("..");
                    }
                }
                other => out.push(other.as_os_str()),
            }
        }
        out.to_string_lossy().into_owned()
    }

    /// `prefix` namespaces the key ("" = resolved against a real file path, "rel:" =
    /// unanchored relative context, "lib:" = under the library directory); `dir` is the
    /// directory nested relative sources resolve against.
    fn walk(
        doc: &Document,
        prefix: &str,
        dir: &Path,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_UNIT_DEPTH {
            return Err(format!(
                "imported units nest deeper than {MAX_UNIT_DEPTH} levels"
            ));
        }
        for unit in doc.units.values() {
            let (key, child_prefix, child_dir) = match &unit.source {
                UnitSource::RelativePath(p) => {
                    let resolved = dir.join(p);
                    let child_dir = resolved.parent().map(PathBuf::from).unwrap_or_default();
                    (
                        format!("{prefix}{}", normalize(&resolved)),
                        prefix.to_string(),
                        child_dir,
                    )
                }
                UnitSource::Library(p) => {
                    let path = Path::new(p);
                    let child_dir = path.parent().map(PathBuf::from).unwrap_or_default();
                    (format!("lib:{}", normalize(path)), "lib:".to_string(), child_dir)
                }
            };
            if stack.contains(&key) {
                return Err(format!(
                    "import cycle: '{key}' is imported by a document it imports"
                ));
            }
            stack.push(key);
            walk(&unit.document, &child_prefix, &child_dir, stack, depth + 1)?;
            stack.pop();
        }
        Ok(())
    }

    let mut stack = Vec::new();
    let (prefix, dir) = match own_path {
        Some(path) => {
            stack.push(normalize(path));
            ("", path.parent().map(PathBuf::from).unwrap_or_default())
        }
        None => ("rel:", PathBuf::new()),
    };
    walk(doc, prefix, &dir, &mut stack, 0)
}

/// The whole document: sketches, sketch primitives, constraints, and construction planes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub parameters: crate::arena::Arena<Parameter>,
    pub sketches: crate::arena::Arena<Sketch>,
    pub lines: crate::arena::Arena<Line>,
    pub circles: crate::arena::Arena<Circle>,
    pub constraints: crate::arena::Arena<Constraint>,
    pub construction_planes: crate::arena::Arena<ConstructionPlane>,
    #[serde(default)]
    pub extrusions: crate::arena::Arena<Extrusion>,
    #[serde(default)]
    pub bodies: crate::arena::Arena<Body>,
    /// Materials bodies can be made of (#834). A body with no material renders in the
    /// document's default body colour.
    #[serde(default)]
    pub materials: crate::arena::Arena<Material>,
    #[serde(default)]
    pub imported_meshes: crate::arena::Arena<ImportedMesh>,
    /// Reference images imported for tracing (#163/#169).
    #[serde(default)]
    pub tracing_images: crate::arena::Arena<TracingImage>,
    /// Loft features (solids blended through cross sections).
    #[serde(default)]
    pub lofts: crate::arena::Arena<Loft>,
    /// Revolved solids (#revolve).
    #[serde(default)]
    pub revolutions: crate::arena::Arena<Revolution>,
    /// Primitive solids placed straight into 3D (#909): cuboids, cylinders, spheres.
    #[serde(default)]
    pub primitives: crate::arena::Arena<Primitive>,
    /// Swept solids (the Sweep tool, #sweep).
    #[serde(default)]
    pub sweeps: crate::arena::Arena<Sweep>,
    /// Boolean operations between bodies (the Combine tool).
    #[serde(default)]
    pub boolean_ops: crate::arena::Arena<BooleanOperation>,
    /// Move operations on bodies (the Move tool, #176/#183).
    #[serde(default)]
    pub move_ops: crate::arena::Arena<MoveOperation>,
    /// Mirror operations on bodies (the Mirror tool, #523).
    #[serde(default)]
    pub mirror_ops: crate::arena::Arena<MirrorOperation>,
    /// Linear repeats on bodies (the Repeat tool, #182).
    #[serde(default)]
    pub repeat_ops: crate::arena::Arena<RepeatOperation>,
    /// Slice operations on bodies (the Slice tool, #181).
    #[serde(default)]
    pub slice_ops: crate::arena::Arena<SliceOperation>,
    /// Shell operations on bodies (the Shell tool, #1156): each shadows its input bodies
    /// and produces hollowed output bodies.
    #[serde(default)]
    pub shell_ops: crate::arena::Arena<ShellOperation>,
    /// Edge chamfer/fillet operations on bodies (#531): each shadows its input bodies and
    /// produces beveled output bodies.
    #[serde(default)]
    pub edge_treatment_ops: crate::arena::Arena<EdgeTreatmentOperation>,
    /// 2D in-sketch linear repeats (#222): duplicated sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_repeat_ops: crate::arena::Arena<SketchRepeatOperation>,
    /// 2D in-sketch slices (#224): split sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_slice_ops: crate::arena::Arena<SketchSliceOperation>,
    /// 2D in-sketch offsets: parallel sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_offset_ops: crate::arena::Arena<SketchOffsetOperation>,
    /// 2D in-sketch mirrors (#523): reflected sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_mirror_ops: crate::arena::Arena<SketchMirrorOperation>,
    /// 2D in-sketch chamfer/fillet operations (#538): shadowed source edges plus regenerated
    /// trimmed copies + bridge lines, grouped under an op.
    #[serde(default)]
    pub sketch_vertex_treatment_ops: crate::arena::Arena<SketchVertexTreatmentOperation>,
    /// Sketch text elements (#282): baked glyph outlines + embedded font, per sketch.
    #[serde(default)]
    pub sketch_texts: crate::arena::Arena<SketchText>,
    /// Technical drawings (#180): black-on-white projected sheets of bodies for print/PDF.
    #[serde(default)]
    pub drawings: crate::arena::Arena<Drawing>,
    /// Joints between parts (#891): kinematic relationships resolved in place at
    /// recompute — no output bodies.
    #[serde(default)]
    pub joints: crate::arena::Arena<Joint>,
    pub shape_order: Vec<ShapeKind>,
    /// Undo-group sizes (#105): entry k is how many [`shape_order`](Self::shape_order)
    /// entries the k-th user-level action created, maintained by `AppState::apply` under
    /// the invariant `undo_groups.iter().sum() == shape_order.len()` (drift from legacy
    /// files or out-of-band edits is reconciled into single-entry groups). **Undo last**
    /// pops one whole group, so a gesture that creates many entries (a rectangle = 4
    /// lines + their constraints) undoes as a single step.
    #[serde(default)]
    pub undo_groups: Vec<usize>,
    /// Document-wide default length unit (context pane, nothing selected; #52).
    ///
    /// Drives dimension-label and Elements-pane display formatting via
    /// [`effective_length_unit`] (#85); bare-number expression parsing is unaffected and
    /// still defaults to mm.
    #[serde(default)]
    pub default_length_unit: LengthUnit,
    /// Document-wide default angle unit (context pane, nothing selected; #52). Same scope
    /// caveat as [`default_length_unit`](Document::default_length_unit).
    #[serde(default)]
    pub default_angle_unit: AngleUnit,
    /// Components (#423): named groups of top-level elements, nestable. The document itself
    /// acts as the root component (its defaults are the top of the unit-inheritance chain).
    #[serde(default)]
    pub components: crate::arena::Arena<Component>,
    /// Component membership (#423): which component each assigned top-level element belongs
    /// to, as `(member, component index)`. Elements without an entry sit directly under the
    /// document root. Tombstoned elements may leave stale entries; lookups go through live
    /// elements only.
    #[serde(default)]
    pub component_members: Vec<(ComponentMember, ComponentKey)>,
    /// Imported units (#719): one embedded copy per imported source document, placed by
    /// [`unit_instances`](Self::unit_instances).
    #[serde(default)]
    pub units: crate::arena::Arena<ImportedUnit>,
    /// Placements of imported units (#719), each with its own name, parameter overrides,
    /// and placement transform.
    #[serde(default)]
    pub unit_instances: crate::arena::Arena<UnitInstance>,
    /// Geometry generation for mesh caches (#1027). Bumped when geometry-affecting state
    /// changes. Not serialized — loads start at 0, first mutation bumps to 1. An integer
    /// compare is the cache key instead of serializing the whole document to JSON.
    #[serde(skip)]
    pub mesh_rev: u64,
}

impl Document {
    /// Bump [`mesh_rev`](Self::mesh_rev) so every mesh cache keyed on it misses (#1027).
    ///
    /// The value is taken from a process-wide counter, not `self.mesh_rev + 1`, so two
    /// documents that have each been edited the same number of times never share a key —
    /// thread-local mesh caches would otherwise hand the first document's triangles to the
    /// second.
    pub fn bump_mesh_rev(&mut self) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        self.mesh_rev = NEXT.fetch_add(1, Ordering::Relaxed);
    }
}

/// A component (#423): a named, nestable group of top-level elements in the Elements pane.
/// Purely organizational — grouping never changes geometry. Carries optional unit overrides
/// that its contents inherit (falling back through parent components to the document).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Component {
    #[serde(default)]
    pub name: Option<String>,
    /// Parent component; `None` = directly under the document root.
    #[serde(default)]
    pub parent: Option<ComponentKey>,
    /// Length-unit override; `None` inherits from the parent chain, then the document.
    #[serde(default)]
    pub length_unit: Option<LengthUnit>,
    /// Angle-unit override; `None` inherits like `length_unit`.
    #[serde(default)]
    pub angle_unit: Option<AngleUnit>,
}

/// A component's identity (#1055): stable across deletions of other components.
pub type ComponentKey = crate::arena::Key<Component>;

/// A top-level element a component can hold (#423) — one of the Elements pane's root rows.
/// Nested elements (sketches on a plane, bodies under an op) follow their root.
///
/// Each variant carries the identity its own collection uses, so the kind and the reference
/// are one value that cannot disagree (#1055). A `Vec`-backed collection is still named by
/// its index; an arena-backed one by its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentMember {
    ConstructionPlane(ConstructionPlaneKey),
    Extrusion(ExtrusionKey),
    Body(BodyKey),
    Loft(LoftKey),
    BooleanOp(BooleanOpKey),
    MoveOp(MoveOpKey),
    MirrorOp(MirrorOpKey),
    RepeatOp(RepeatOpKey),
    SliceOp(SliceOpKey),
    ShellOp(ShellOpKey),
    EdgeTreatmentOp(EdgeTreatmentOpKey),
    Revolution(RevolutionKey),
    Sweep(SweepKey),
    Drawing(DrawingKey),
}

impl Document {
    /// The component an assigned top-level element belongs to, if any (#423).
    pub fn component_of(&self, member: ComponentMember) -> Option<ComponentKey> {
        self.component_members
            .iter()
            .find(|(m, _)| *m == member)
            .map(|(_, c)| *c)
            .filter(|&c| self.components.contains(c))
    }

    /// Assign (or with `None`, unassign) a top-level element to a component (#423).
    pub fn set_component_member(
        &mut self,
        member: ComponentMember,
        component: Option<ComponentKey>,
    ) {
        self.component_members.retain(|(m, _)| *m != member);
        if let Some(c) = component {
            self.component_members.push((member, c));
        }
    }

    /// Walk a component's parent chain (self first). Cycles are cut off defensively.
    pub fn component_chain(&self, component: ComponentKey) -> Vec<ComponentKey> {
        let mut chain = Vec::new();
        let mut cur = Some(component);
        while let Some(c) = cur {
            if chain.contains(&c) || !self.components.contains(c) {
                break;
            }
            chain.push(c);
            cur = self.components[c].parent;
        }
        chain
    }
}

/// Effective length unit for a component (#423): its own override, else the nearest
/// ancestor's, else the document default.
pub fn effective_component_length_unit(doc: &Document, component: ComponentKey) -> LengthUnit {
    doc.component_chain(component)
        .into_iter()
        .find_map(|c| doc.components[c].length_unit)
        .unwrap_or(doc.default_length_unit)
}

/// Effective angle unit for a component (#423), like [`effective_component_length_unit`].
pub fn effective_component_angle_unit(doc: &Document, component: ComponentKey) -> AngleUnit {
    doc.component_chain(component)
        .into_iter()
        .find_map(|c| doc.components[c].angle_unit)
        .unwrap_or(doc.default_angle_unit)
}

/// The component a sketch's geometry belongs to (#423): resolved through the sketch's host
/// face — a construction plane's own assignment (or, for a face-anchored plane, the host
/// sketch's component), or the owning extrusion's assignment for a body-face sketch.
pub fn sketch_component(doc: &Document, sketch: SketchId) -> Option<ComponentKey> {
    fn plane_component(doc: &Document, plane: ConstructionPlaneKey, depth: u8) -> Option<ComponentKey> {
        if depth > 8 {
            return None;
        }
        if let Some(c) = doc.component_of(ComponentMember::ConstructionPlane(plane)) {
            return Some(c);
        }
        match doc.construction_planes.get(plane)?.parent {
            ConstructionPlaneParent::Root => None,
            ConstructionPlaneParent::Sketch(s) => sketch_component_inner(doc, s, depth + 1),
        }
    }
    fn sketch_component_inner(
        doc: &Document,
        sketch: SketchId,
        depth: u8,
    ) -> Option<ComponentKey> {
        if depth > 8 {
            return None;
        }
        match doc.sketch_face(sketch)? {
            FaceId::ConstructionPlane(p) => plane_component(doc, p, depth + 1),
            FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
                doc.component_of(ComponentMember::Extrusion(extrusion)).or_else(|| {
                    doc.extrusions
                        .get(extrusion)
                        .and_then(|e| sketch_component_inner(doc, e.sketch, depth + 1))
                })
            }
            _ => None,
        }
    }
    sketch_component_inner(doc, sketch, 0)
}

impl Default for Document {
    fn default() -> Self {
        Self {
            parameters: crate::arena::Arena::new(),
            sketches: crate::arena::Arena::new(),
            lines: crate::arena::Arena::new(),
            circles: crate::arena::Arena::new(),
            constraints: crate::arena::Arena::new(),
            construction_planes: crate::face::default_datum_planes(),
            extrusions: crate::arena::Arena::new(),
            bodies: crate::arena::Arena::new(),
            materials: Material::defaults(),
            imported_meshes: crate::arena::Arena::new(),
            tracing_images: crate::arena::Arena::new(),
            lofts: crate::arena::Arena::new(),
            revolutions: crate::arena::Arena::new(),
            primitives: crate::arena::Arena::new(),
            sweeps: crate::arena::Arena::new(),
            boolean_ops: crate::arena::Arena::new(),
            move_ops: crate::arena::Arena::new(),
            mirror_ops: crate::arena::Arena::new(),
            repeat_ops: crate::arena::Arena::new(),
            slice_ops: crate::arena::Arena::new(),
            shell_ops: crate::arena::Arena::new(),
            edge_treatment_ops: crate::arena::Arena::new(),
            sketch_repeat_ops: crate::arena::Arena::new(),
            sketch_offset_ops: crate::arena::Arena::new(),
            sketch_mirror_ops: crate::arena::Arena::new(),
            sketch_vertex_treatment_ops: crate::arena::Arena::new(),
            sketch_slice_ops: crate::arena::Arena::new(),
            sketch_texts: crate::arena::Arena::new(),
            drawings: crate::arena::Arena::new(),
            joints: crate::arena::Arena::new(),
            shape_order: Vec::new(),
            undo_groups: Vec::new(),
            default_length_unit: LengthUnit::default(),
            default_angle_unit: AngleUnit::default(),
            components: crate::arena::Arena::new(),
            component_members: Vec::new(),
            units: crate::arena::Arena::new(),
            unit_instances: crate::arena::Arena::new(),
            mesh_rev: 0,
        }
    }
}

impl Document {
    pub fn sketch_face(&self, sketch: SketchId) -> Option<FaceId> {
        self.sketches.get(sketch).map(|s| s.face.clone())
    }

    pub fn sketches_on_face(&self, face: FaceId) -> impl Iterator<Item = SketchId> + '_ {
        self.sketches
            .iter()
            .filter_map(move |(i, s)| (s.face == face).then_some(i))
    }

    #[allow(dead_code)] // query helper; now exercised only by tests since undo went snapshot-based (#194)
    pub fn sketch_has_geometry(&self, sketch: SketchId) -> bool {
        self.lines.values().any(|l| l.sketch == sketch)
            || self.circles.values().any(|c| c.sketch == sketch)
    }

    #[allow(dead_code)] // query helper; now exercised only by tests
    pub fn has_children(&self, face: &FaceId) -> bool {
        self.sketches.values().any(|s| &s.face == face)
    }

    pub fn add_sketch(&mut self, face: FaceId) -> SketchId {
        let id = self.sketches.insert(Sketch {
            face,
            name: None,
            length_unit: None,
            angle_unit: None,
        });
        self.shape_order.push(ShapeKind::Sketch);
        id
    }
}

/// Effective default length unit for `sketch`: its own override, or the document default if
/// unset or the sketch doesn't exist (#52).
pub fn effective_length_unit(doc: &Document, sketch: SketchId) -> LengthUnit {
    doc.sketches
        .get(sketch)
        .and_then(|s| s.length_unit)
        .or_else(|| {
            // Component units (#423): a sketch with no override inherits its component chain.
            sketch_component(doc, sketch).and_then(|c| {
                doc.component_chain(c)
                    .into_iter()
                    .find_map(|c| doc.components[c].length_unit)
            })
        })
        .unwrap_or(doc.default_length_unit)
}

/// Effective default angle unit for `sketch`: its own override, or the document default if
/// unset or the sketch doesn't exist (#52).
pub fn effective_angle_unit(doc: &Document, sketch: SketchId) -> AngleUnit {
    doc.sketches
        .get(sketch)
        .and_then(|s| s.angle_unit)
        .or_else(|| {
            sketch_component(doc, sketch).and_then(|c| {
                doc.component_chain(c)
                    .into_iter()
                    .find_map(|c| doc.components[c].angle_unit)
            })
        })
        .unwrap_or(doc.default_angle_unit)
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;

    /// #921: repeated J walks the joint kinds in the dropdown's order and comes back round.
    #[test]
    fn joint_kind_cycles_through_every_kind() {
        let mut kind = JointKind::Rigid;
        let mut seen = vec![kind.name()];
        for _ in 0..7 {
            kind = kind.next();
            seen.push(kind.name());
        }
        assert_eq!(
            seen,
            vec![
                "rigid",
                "slider",
                "revolute",
                "cylindrical",
                "planar",
                "ball",
                "pin_slot",
                "screw"
            ]
        );
        assert_eq!(kind.next().name(), "rigid", "and round again");
    }
    use super::*;

    /// #1056: membership names an element with the identity that element's own collection
    /// uses, so a key's generation travels with it. Recreating a loft reuses the freed slot,
    /// and the entry the old loft left behind must not claim the new one.
    #[test]
    fn a_membership_entry_does_not_survive_onto_whatever_reuses_the_slot() {
        let mut doc = Document::default();
        let comp = doc.components.insert(Component {
            name: None,
            parent: None,
            length_unit: None,
            angle_unit: None,
        });
        let loft = || Loft { sections: Vec::new(), mode: LoftMode::NewBody, name: None };
        let old = doc.lofts.insert(loft());
        doc.set_component_member(ComponentMember::Loft(old), Some(comp));
        assert_eq!(doc.component_of(ComponentMember::Loft(old)), Some(comp));

        doc.lofts.remove(old);
        let new = doc.lofts.insert(loft());
        assert_eq!(old.index(), new.index(), "the slot was reused");
        assert_eq!(
            doc.component_of(ComponentMember::Loft(new)),
            None,
            "the new loft sits at the document root, not in the old one's component"
        );
    }

    /// #833: documents saved before planes had an extent load with the old symmetric
    /// ±50mm square, so they look exactly as they did.
    #[test]
    fn a_plane_saved_without_an_extent_keeps_the_old_centred_square() {
        let plane = crate::face::default_xy_plane();
        let mut json: serde_json::Value = serde_json::to_value(&plane).unwrap();
        json.as_object_mut().unwrap().remove("extent").expect("planes serialize their extent");
        let loaded: ConstructionPlane = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.extent, PlaneExtent::default());
        assert_eq!(loaded.extent.u_max, crate::construction::PLANE_DISPLAY_HALF);
        assert_eq!(loaded.extent.u_min, -crate::construction::PLANE_DISPLAY_HALF);
    }

    #[test]
    fn legacy_horizontal_vertical_constraints_migrate_to_axis_parallel() {
        // #577/#580: old documents storing `horizontal`/`vertical` constraint tags load by mapping
        // them to Parallel against the X/Y sketch axis.
        let horizontal: ConstraintKind =
            serde_json::from_str(r#"{"horizontal":{"line":{"line":[3,0]}}}"#).unwrap();
        assert_eq!(
            horizontal,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(lkey(3)),
                line_b: ConstraintLine::OriginAxis(SketchAxis::X),
            }
        );
        let vertical: ConstraintKind =
            serde_json::from_str(r#"{"vertical":{"line":{"line":[7,0]}}}"#).unwrap();
        assert_eq!(
            vertical,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(lkey(7)),
                line_b: ConstraintLine::OriginAxis(SketchAxis::Y),
            }
        );
        // A normal constraint still round-trips unchanged.
        let parallel = ConstraintKind::Parallel {
            line_a: ConstraintLine::Line(lkey(0)),
            line_b: ConstraintLine::Line(lkey(1)),
        };
        let json = serde_json::to_string(&parallel).unwrap();
        assert!(!json.contains("horizontal") && !json.contains("vertical"));
        assert_eq!(serde_json::from_str::<ConstraintKind>(&json).unwrap(), parallel);
    }

    /// #1076: four move modes, and only the Joint tool offers **In place** — a Move that moves
    /// nothing is a no-op, not a mode. Both snapping modes derive their translation from the
    /// A pair; Face Snap just picks its points on faces.
    #[test]
    fn move_modes_and_which_tool_offers_them() {
        use MoveTranslateMode as M;
        assert_eq!(M::default(), M::PointSnap, "the Move tool's default");
        assert_eq!(
            M::for_tool(false),
            vec![M::PointSnap, M::FaceSnap, M::Free],
            "no In place outside the Joint tool"
        );
        assert_eq!(M::for_tool(true).last(), Some(&M::InPlace));
        assert!(M::PointSnap.is_snap() && M::FaceSnap.is_snap());
        assert!(!M::Free.is_snap() && !M::InPlace.is_snap());
        // Every mode round-trips through its scripted name, and the old spellings still read.
        for m in M::for_tool(true) {
            assert_eq!(M::from_name(m.name()), Some(m), "{m:?}");
        }
        assert_eq!(M::from_name("snap"), Some(M::PointSnap));
        assert_eq!(M::from_name("xyz"), Some(M::Free));
        assert_eq!(M::from_name("nope"), None);
    }

    /// #257: the count/gap/distance UI mapping round-trips through `RepeatMode`, and each toggle
    /// combination picks the right mode.
    #[test]
    fn repeat_ui_mode_mapping_round_trips() {
        for computed in [RepeatVar::Count, RepeatVar::Gap, RepeatVar::Distance] {
            for gap_off in [false, true] {
                for dist_end in [false, true] {
                    let mode = RepeatMode::from_repeat_ui(computed, gap_off, dist_end);
                    let (c2, g2, d2) = mode.to_repeat_ui();
                    assert_eq!(c2, computed, "computed var round-trips");
                    // The toggles round-trip on the axes the computed variable actually uses.
                    match computed {
                        RepeatVar::Distance => assert_eq!(g2, gap_off),
                        RepeatVar::Gap => assert_eq!(d2, dist_end),
                        RepeatVar::Count => {
                            assert_eq!((g2, d2), (gap_off, dist_end));
                        }
                    }
                }
            }
        }
        // Spot-check specific modes.
        assert_eq!(RepeatMode::from_repeat_ui(RepeatVar::Distance, false, true), RepeatMode::CountGap);
        assert_eq!(RepeatMode::from_repeat_ui(RepeatVar::Distance, true, true), RepeatMode::CountPitch);
        assert_eq!(RepeatMode::from_repeat_ui(RepeatVar::Count, false, false), RepeatMode::FillGapSpan);
    }

    #[test]
    fn line_length_from_endpoints() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let line = Line::from_local_endpoints(sketch, 0.0, 0.0, 3.0, 4.0);
        assert!((line.length() - 5.0).abs() < 1e-4);
    }

    #[test]
    fn straight_line_samples_to_just_its_two_endpoints() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let line = Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        assert_eq!(line.sample_local(BEZIER_SEGMENTS), vec![(0.0, 0.0), (10.0, 0.0)]);
        assert!(!line.is_curved());
    }

    #[test]
    fn curved_line_samples_pass_through_both_endpoints() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let mut line = Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        line.bezier = Some([(3.0, 4.0), (7.0, 4.0)]);
        let pts = line.sample_local(BEZIER_SEGMENTS);
        assert_eq!(pts.len(), BEZIER_SEGMENTS + 1);
        assert_eq!(pts[0], (0.0, 0.0));
        assert_eq!(*pts.last().unwrap(), (10.0, 0.0));
        // Bulges away from the straight chord partway through.
        assert!(pts[BEZIER_SEGMENTS / 2].1 > 1.0);
        assert!(line.is_curved());
    }

    #[test]
    fn straight_line_arc_length_equals_chord_exactly() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let line = Line::from_local_endpoints(sketch, 0.0, 0.0, 3.0, 4.0);
        assert_eq!(line.length(), line.chord_length());
    }

    #[test]
    fn curved_line_length_is_the_arc_not_the_chord() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let mut line = Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        // Extreme handles far off the chord (todoer #111): a 10 mm chord with a huge bulge.
        line.bezier = Some([(200.0, 300.0), (-190.0, 300.0)]);
        assert!((line.chord_length() - 10.0).abs() < 1e-4);
        assert!(
            line.length() > line.chord_length() * 10.0,
            "arc {} should dwarf the 10 mm chord",
            line.length()
        );
    }

    #[test]
    fn kappa_quarter_circle_arc_length_matches_analytic_value() {
        // The standard cubic-bezier circle approximation: start (r, 0), end (0, r),
        // handles at (r, r*kappa) and (r*kappa, r). Its arc length must match (pi/2)*r
        // to within ~0.1% at BEZIER_SEGMENTS resolution.
        const KAPPA: f32 = 0.552_284_7;
        let r = 10.0_f32;
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let mut line = Line::from_local_endpoints(sketch, r, 0.0, 0.0, r);
        line.bezier = Some([(r, r * KAPPA), (r * KAPPA, r)]);
        let expected = std::f32::consts::FRAC_PI_2 * r;
        let arc = line.length();
        let rel_err = (arc - expected).abs() / expected;
        assert!(rel_err < 1e-3, "arc {arc} vs {expected}: relative error {rel_err}");
        assert!(arc > line.chord_length());
    }

    #[test]
    fn degenerate_bezier_with_handles_on_endpoints_has_arc_equal_to_chord() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let mut line = Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        line.bezier = Some([(0.0, 0.0), (10.0, 0.0)]);
        assert!(line.is_curved());
        assert!((line.length() - line.chord_length()).abs() < 1e-4);
    }

    #[test]
    fn smooth_joint_bezier_keeps_both_handles_on_the_a_to_b_tangent() {
        let a = (0.0, 0.0);
        let v = (10.0, 0.0);
        let b = (20.0, 0.0);
        let ([h1_far, h1_near], [h2_near, h2_far]) = smooth_joint_bezier(a, v, b);
        // Collinear a-v-b: every handle should stay on the same horizontal line.
        for (_, y) in [h1_far, h1_near, h2_near, h2_far] {
            assert!(y.abs() < 1e-4);
        }
        // Handles near the joint sit strictly between the far endpoints and v.
        assert!(h1_near.0 > a.0 && h1_near.0 < v.0);
        assert!(h2_near.0 > v.0 && h2_near.0 < b.0);
    }

    #[test]
    fn independent_corner_handle_sits_a_third_of_the_way_toward_the_target() {
        let h = independent_corner_handle((0.0, 0.0), (9.0, 6.0));
        assert!((h.0 - 3.0).abs() < 1e-4);
        assert!((h.1 - 2.0).abs() < 1e-4);
    }

    #[test]
    fn vertex_treatment_chamfer_on_a_right_angle_corner_is_symmetric() {
        let v = (0.0, 0.0);
        let a = (10.0, 0.0);
        let b = (0.0, 10.0);
        let geom =
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Chamfer, 3.0).unwrap();
        assert!((geom.p1.0 - 3.0).abs() < 1e-4 && geom.p1.1.abs() < 1e-4);
        assert!((geom.p2.1 - 3.0).abs() < 1e-4 && geom.p2.0.abs() < 1e-4);
        assert_eq!(geom.bezier, None);
    }

    #[test]
    fn vertex_treatment_fillet_on_a_right_angle_corner_stays_radius_from_center() {
        let v = (0.0, 0.0);
        let a = (10.0, 0.0);
        let b = (0.0, 10.0);
        let radius = 3.0;
        let geom =
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Fillet, radius).unwrap();
        // Tangent length for a 90 degree corner equals the radius (tan(45deg) == 1).
        assert!((geom.p1.0 - radius).abs() < 1e-4 && geom.p1.1.abs() < 1e-4);
        assert!((geom.p2.1 - radius).abs() < 1e-4 && geom.p2.0.abs() < 1e-4);
        let bezier = geom.bezier.expect("fillet should curve the bridging line");

        // The arc center sits on the inward bisector, equidistant (by `radius`) from both p1/p2.
        let center = (3.0, 3.0);
        let mut line =
            Line::from_local_endpoints(sketch_key_for_slot(0), geom.p1.0, geom.p1.1, geom.p2.0, geom.p2.1);
        line.bezier = Some(bezier);
        for (x, y) in line.sample_local(BEZIER_SEGMENTS) {
            let dist = ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt();
            assert!(
                (dist - radius).abs() < radius * 0.02,
                "sampled point ({x}, {y}) at distance {dist} from center, expected ~{radius}"
            );
        }
    }

    #[test]
    fn vertex_treatment_fillet_on_a_45_degree_corner_stays_radius_from_center() {
        // A shallower corner: far points at 90 degrees apart around a 45 degree wedge.
        let v = (0.0, 0.0);
        let a = (10.0, 0.0);
        let b = (10.0 * (std::f32::consts::FRAC_PI_4).cos(), 10.0 * (std::f32::consts::FRAC_PI_4).sin());
        let radius = 2.0;
        let geom =
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Fillet, radius).unwrap();
        let bezier = geom.bezier.unwrap();
        let alpha = std::f32::consts::FRAC_PI_4;
        let bisector_len = radius / (alpha / 2.0).sin();
        let bisector_angle = alpha / 2.0;
        let center = (
            bisector_len * bisector_angle.cos(),
            bisector_len * bisector_angle.sin(),
        );
        let mut line =
            Line::from_local_endpoints(sketch_key_for_slot(0), geom.p1.0, geom.p1.1, geom.p2.0, geom.p2.1);
        line.bezier = Some(bezier);
        for (x, y) in line.sample_local(BEZIER_SEGMENTS) {
            let dist = ((x - center.0).powi(2) + (y - center.1).powi(2)).sqrt();
            assert!(
                (dist - radius).abs() < radius * 0.05,
                "sampled point ({x}, {y}) at distance {dist} from center, expected ~{radius}"
            );
        }
    }

    #[test]
    fn vertex_treatment_clamps_tangent_length_to_the_shorter_edge() {
        // Both edges only 2mm long; a 10mm chamfer distance must clamp back to ~1.9mm (0.95x).
        let v = (0.0, 0.0);
        let a = (2.0, 0.0);
        let b = (0.0, 2.0);
        let geom =
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Chamfer, 10.0).unwrap();
        assert!((geom.p1.0 - 1.9).abs() < 1e-4);
        assert!((geom.p2.1 - 1.9).abs() < 1e-4);
    }

    #[test]
    fn vertex_treatment_rejects_a_degenerate_straight_corner() {
        let v = (0.0, 0.0);
        // a and b both lie along +X from v: the "corner" is actually a straight continuation.
        let a = (10.0, 0.0);
        let b = (20.0, 0.0);
        assert_eq!(
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Chamfer, 3.0),
            None
        );
        assert_eq!(
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Fillet, 3.0),
            None
        );
    }

    #[test]
    fn vertex_treatment_rejects_a_degenerate_folded_back_corner() {
        let v = (0.0, 0.0);
        // a and b point in opposite directions from v: a 180 degree fold, not a real corner.
        let a = (10.0, 0.0);
        let b = (-10.0, 0.0);
        assert_eq!(
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Chamfer, 3.0),
            None
        );
    }

    #[test]
    fn vertex_treatment_rejects_non_positive_amount() {
        let v = (0.0, 0.0);
        let a = (10.0, 0.0);
        let b = (0.0, 10.0);
        assert_eq!(
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Chamfer, 0.0),
            None
        );
        assert_eq!(
            vertex_treatment_geometry(v, a, b, VertexTreatmentKind::Fillet, -1.0),
            None
        );
    }

    #[test]
    fn face_id_from_script_parses_circle() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        for i in 0..3 {
            doc.circles.insert(Circle::from_local_center_radius(
                sketch, 0.0, 0.0, i as f32 + 1.0, 0.0,
            ));
        }
        let third = doc.circles.keys().nth(2).unwrap();
        assert_eq!(FaceId::from_script(&doc, "circle", 2), Some(FaceId::Circle(third)));
    }

    #[test]
    fn multiple_sketches_on_one_face() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let s1 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        assert_ne!(s0, s1);
        let on_plane: Vec<_> = doc.sketches_on_face(FaceId::ConstructionPlane(pkey(0))).collect();
        assert_eq!(on_plane, vec![s0, s1]);
    }

    #[test]
    fn sketch_has_geometry_detects_primitives() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        assert!(!doc.sketch_has_geometry(sketch));
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 1.0, 1.0));
        assert!(doc.sketch_has_geometry(sketch));
    }

    #[test]
    fn circle_diameter_is_twice_radius() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let circle = Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0);
        assert!((circle.diameter() - 10.0).abs() < 1e-4);
    }

    #[test]
    fn default_document_units_are_mm_and_deg() {
        let doc = Document::default();
        assert_eq!(doc.default_length_unit, LengthUnit::Mm);
        assert_eq!(doc.default_angle_unit, AngleUnit::Deg);
    }

    #[test]
    fn new_sketch_inherits_document_units_by_default() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        assert_eq!(doc.sketches[sketch].length_unit, None);
        assert_eq!(doc.sketches[sketch].angle_unit, None);
        assert_eq!(effective_length_unit(&doc, sketch), LengthUnit::Mm);
        assert_eq!(effective_angle_unit(&doc, sketch), AngleUnit::Deg);
    }

    #[test]
    fn effective_units_follow_document_default_change() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.default_length_unit = LengthUnit::In;
        doc.default_angle_unit = AngleUnit::Rad;
        assert_eq!(effective_length_unit(&doc, sketch), LengthUnit::In);
        assert_eq!(effective_angle_unit(&doc, sketch), AngleUnit::Rad);
    }

    #[test]
    fn sketch_override_takes_precedence_over_document_default() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.sketches[sketch].length_unit = Some(LengthUnit::Cm);
        doc.sketches[sketch].angle_unit = Some(AngleUnit::Rad);
        assert_eq!(effective_length_unit(&doc, sketch), LengthUnit::Cm);
        assert_eq!(effective_angle_unit(&doc, sketch), AngleUnit::Rad);
        // Document default is unaffected by the sketch's override.
        assert_eq!(doc.default_length_unit, LengthUnit::Mm);
    }

    #[test]
    fn effective_units_for_missing_sketch_fall_back_to_document_default() {
        let doc = Document::default();
        let missing = sketch_key_for_slot(99);
        assert_eq!(effective_length_unit(&doc, missing), LengthUnit::Mm);
        assert_eq!(effective_angle_unit(&doc, missing), AngleUnit::Deg);
    }

    /// #1168: a shell output of a cuboid is the live body for that primitive's faces —
    /// not the shadow primitive input. Append/cut on Shelled record the fused extrusions.
    #[test]
    fn shelled_body_owns_primitive_faces_and_takes_fused_extrusions() {
        let pi = PrimitiveKey::from_bits(1);
        let ei = ExtrusionKey::from_bits(2);
        let mut doc = Document::default();
        let input = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: true,
        });
        let op = doc.shell_ops.insert(ShellOperation {
            targets: vec![input],
            open_faces: Vec::new(),
            thickness: "1".to_string(),
            outputs: Vec::new(),
            name: None,
        });
        let out = doc.bodies.insert(Body {
            source: BodySource::Shelled {
                op,
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.shell_ops[op].outputs = vec![out];

        assert_eq!(
            body_index_for_primitive(&doc, pi),
            Some(out),
            "live shelled body owns the cuboid's faces, not the shadow input"
        );

        // #1172: shell open-face matching must still attribute the face to the *input*
        // target, even though body_index_for_face prefers the live shell output.
        let top = FaceId::PrimitiveFace {
            primitive: pi,
            face: PrimitiveFace::CuboidTop,
        };
        assert!(
            face_belongs_to_body(&doc, &top, input),
            "primitive open face still belongs to the shell input body"
        );
        assert_eq!(
            body_index_for_face(&doc, &top),
            Some(out),
            "live preference for picking still points at the shell output"
        );

        let mut src = BodySource::Shelled {
            op,
            target: 0,
            add: Vec::new(),
            cut: Vec::new(),
        };
        src.append_extrusion(ei);
        assert_eq!(src.extrusion_indices(), [ei]);
        assert!(src.cut_extrusion_indices().is_empty());
        assert_eq!(src.producing_extrusion(), Some(ei));
        assert_eq!(
            src.predecessor_source(ei),
            Some(BodySource::Shelled {
                op,
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            })
        );
        let mut cut_src = BodySource::Shelled {
            op,
            target: 0,
            add: Vec::new(),
            cut: Vec::new(),
        };
        cut_src.append_cut_extrusion(ei);
        assert_eq!(cut_src.cut_extrusion_indices(), [ei]);
        assert!(cut_src.extrusion_indices().is_empty());
    }

    /// #1463: a Move of a cuboid is the live body for that primitive's faces —
    /// not the shadow primitive input. A body is a body.
    #[test]
    fn moved_body_owns_primitive_faces() {
        let pi = PrimitiveKey::from_bits(1);
        let mut doc = Document::default();
        let input = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: true,
        });
        let op = doc.move_ops.insert(MoveOperation {
            targets: vec![input],
            translate_mode: MoveTranslateMode::Free,
            tx: "80".into(),
            ..MoveOperation::default()
        });
        let out = doc.bodies.insert(Body {
            source: BodySource::Moved {
                op,
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.move_ops[op].outputs = vec![out];

        assert_eq!(
            body_index_for_primitive(&doc, pi),
            Some(out),
            "live moved body owns the cuboid's faces, not the shadow input"
        );

        let top = FaceId::PrimitiveFace {
            primitive: pi,
            face: PrimitiveFace::CuboidTop,
        };
        assert!(
            face_belongs_to_body(&doc, &top, input),
            "primitive face still belongs to the move input body"
        );
        assert!(
            face_belongs_to_body(&doc, &top, out),
            "and to the moved body — a body is a body"
        );
        assert_eq!(
            body_index_for_face(&doc, &top),
            Some(out),
            "live preference for picking still points at the moved output"
        );
    }

    /// #1338: a cut into a Combine result must record the extrusion on that boolean body
    /// instead of silently no-op'ing (which left an orphan extrusion and blocked further cuts).
    #[test]
    fn boolean_body_source_records_a_cut_extrusion() {
        let op = boolean_op_key_for_slot(0);
        let ei = extrusion_key_for_slot(1);
        let mut src = BodySource::Boolean {
            op,
            solid: 0,
            add: Vec::new(),
            cut: Vec::new(),
        };
        src.append_cut_extrusion(ei);
        assert_eq!(
            src.cut_extrusion_indices(),
            [ei],
            "cut into a combined body must stick to that body"
        );
        assert!(src.extrusion_indices().is_empty());
    }

    /// #1345: a cut into a Move/Slice/Mirror/Repeat/fillet result must record the
    /// extrusion on that live body instead of silently no-op'ing (same class as #1338).
    #[test]
    fn op_produced_body_sources_record_a_cut_extrusion() {
        let ei = extrusion_key_for_slot(1);
        let cases = [
            BodySource::Moved {
                op: move_op_key_for_slot(0),
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            BodySource::Sliced {
                op: slice_op_key_for_slot(0),
                target: 0,
                piece: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            BodySource::Mirrored {
                op: mirror_op_key_for_slot(0),
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            BodySource::Repeated {
                op: repeat_op_key_for_slot(0),
                target: 0,
                instance: 1,
                add: Vec::new(),
                cut: Vec::new(),
            },
            BodySource::EdgeTreated {
                op: edge_treatment_op_key_for_slot(0),
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
        ];
        for mut src in cases {
            let label = format!("{src:?}");
            src.append_cut_extrusion(ei);
            assert_eq!(
                src.cut_extrusion_indices(),
                [ei],
                "cut into {label} must stick to that body"
            );
            assert!(
                src.extrusion_indices().is_empty(),
                "cut into {label} must not land as an add"
            );
        }
    }

    /// #1104: adding/cutting an extrusion on a Shape-tool body turns Primitive into Solid
    /// with that primitive as base; removing the last addition restores Primitive.
    #[test]
    fn primitive_body_source_grows_into_solid_with_base() {
        let pi = PrimitiveKey::from_bits(1);
        let ei = ExtrusionKey::from_bits(2);
        let ej = ExtrusionKey::from_bits(3);
        let mut src = BodySource::Primitive(pi);
        src.append_extrusion(ei);
        assert_eq!(
            src,
            BodySource::Solid {
                base: Some(pi),
                add: vec![ei],
                cut: vec![],
            }
        );
        src.append_extrusion(ej);
        assert_eq!(src.extrusion_indices(), [ei, ej]);
        src.remove_extrusion(ej);
        src.remove_extrusion(ei);
        assert_eq!(src, BodySource::Primitive(pi));

        let mut cut_src = BodySource::Primitive(pi);
        cut_src.append_cut_extrusion(ei);
        assert_eq!(
            cut_src,
            BodySource::Solid {
                base: Some(pi),
                add: vec![],
                cut: vec![ei],
            }
        );
        assert_eq!(body_index_for_primitive(
            &{
                let mut doc = Document::default();
                doc.bodies.insert(Body {
                    source: BodySource::Solid {
                        base: Some(pi),
                        add: vec![ei],
                        cut: vec![],
                    },
                    material: None,
                    name: None,
                    shadow: false,
                });
                // primitive_base lookup doesn't need the primitive arena entry.
                doc
            },
            pi
        ).is_some(), true);
    }
}
/// Scale for [`ExtrudeFace::SketchRegion`]'s seed point: thousandths of a sketch unit, which
/// keeps a profile `Eq`/`Hash` while staying far finer than any region it has to tell apart.
pub const SKETCH_REGION_SEED_SCALE: f32 = 1000.0;

/// Quantize a sketch-local point into a [`ExtrudeFace::SketchRegion`] seed.
pub fn sketch_region_seed(u: f32, v: f32) -> (i32, i32) {
    (
        (u * SKETCH_REGION_SEED_SCALE).round() as i32,
        (v * SKETCH_REGION_SEED_SCALE).round() as i32,
    )
}

/// The sketch-local point a [`ExtrudeFace::SketchRegion`] seed stands for.
pub fn sketch_region_seed_point(seed_u: i32, seed_v: i32) -> (f32, f32) {
    (
        seed_u as f32 / SKETCH_REGION_SEED_SCALE,
        seed_v as f32 / SKETCH_REGION_SEED_SCALE,
    )
}
