//! In-memory document model.
//!
//! This is the very first slice of BearCAD (see SPEC.md): a document is a flat list
//! of rectangles and lines on a single 2D sketch. As the action-DAG, components,
//! and the OCCT kernel come online this will grow, but the persistence boundary
//! (`storage.rs`) is kept narrow so the file format can evolve underneath it.

use crate::face::default_xy_plane;
use crate::value::{AngleUnit, LengthUnit};
use serde::{Deserialize, Serialize};

/// A sketchable face that lines and rectangles can be drawn on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceId {
    Circle(usize),
    /// A closed loop of plain `Line`s, identified by its ordered line indices (#66).
    Polygon(Vec<usize>),
    ConstructionPlane(usize),
    /// A planar cap face of an extruded body: one profile face of an extrusion,
    /// at either the base (`top = false`) or offset (`top = true`) end.
    ExtrudeCap {
        extrusion: usize,
        profile: ExtrudeFace,
        top: bool,
    },
    /// A planar side wall of an extruded body: the quad swept by one `edge` of a
    /// polygonal profile (rectangles only; circular profiles have no flat sides).
    ExtrudeSide {
        extrusion: usize,
        profile: ExtrudeFace,
        edge: u8,
    },
    /// A flat side of a partial (< 360°) revolve (#621): one profile face rotated to the
    /// sweep's start (`end = false`) or end (`end = true`) angle. Full sweeps have none.
    RevolveCap {
        revolution: usize,
        profile: ExtrudeFace,
        end: bool,
    },
    /// The flat washer/annular-sector face a revolve sweeps from one polygon-profile
    /// `edge` whose endpoints share an axis coordinate (#621) — e.g. the flat ends of a
    /// revolved ring. Edges not perpendicular to the axis sweep curved surfaces instead.
    RevolveSide {
        revolution: usize,
        profile: ExtrudeFace,
        edge: u8,
    },
}

impl Default for FaceId {
    fn default() -> Self {
        FaceId::ConstructionPlane(0)
    }
}

impl FaceId {
    pub fn from_script(kind: &str, index: usize) -> Option<Self> {
        match kind.to_ascii_lowercase().as_str() {
            "circle" => Some(FaceId::Circle(index)),
            "plane" | "construction_plane" | "constructionplane" => {
                Some(FaceId::ConstructionPlane(index))
            }
            _ => None,
        }
    }

    /// The extrusion index that owns this face, for the two body-face variants (#26/#27's
    /// `FaceVertex`/`FaceEdge` dependency tracking piggybacks on this: a sketch on a body face,
    /// or a constraint referencing that face's own boundary, both depend on the extrusion that
    /// produced it — same relationship `hierarchy::face_element` already tracks for sketches).
    pub fn extrusion_index(&self) -> Option<usize> {
        match self {
            FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
                Some(*extrusion)
            }
            FaceId::Circle(_)
            | FaceId::Polygon(_)
            | FaceId::ConstructionPlane(_)
            | FaceId::RevolveCap { .. }
            | FaceId::RevolveSide { .. } => None,
        }
    }

    /// The revolution index that owns this face — the [`FaceId::extrusion_index`]
    /// analogue for sketches hosted on a revolve's flat sides (#621).
    pub fn revolution_index(&self) -> Option<usize> {
        match self {
            FaceId::RevolveCap { revolution, .. } | FaceId::RevolveSide { revolution, .. } => {
                Some(*revolution)
            }
            _ => None,
        }
    }
}

/// Index into [`Document::sketches`].
pub type SketchId = usize;

/// Geometry that drives a read-only parameter value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterSource {
    LineLength(usize),
    /// Distance between two points (#432), measured in world space (2D or 3D).
    PointDistance(ConstraintPoint, ConstraintPoint),
    /// Distance between two parallel lines (#432).
    LineDistance(usize, usize),
    /// Angle between two non-parallel lines in the same sketch (#432), stored in degrees.
    LineAngle(usize, usize),
    /// Length of a **body's feature edge** (#647), keyed the way
    /// [`crate::hierarchy::SceneElement::BodyEdge`] is: the body plus the edge's quantized
    /// world endpoints. Re-resolved against the body's live mesh, so it reads the current
    /// length; if a rebuild moves the edge off that key, the parameter reads as unavailable
    /// (the same way a deleted line's does).
    BodyEdgeLength {
        body: usize,
        a: [i32; 3],
        b: [i32; 3],
    },
    /// Distance between two **body mesh corners** (#647), keyed like
    /// [`crate::hierarchy::SceneElement::BodyVertex`]. The two corners may sit on different
    /// bodies.
    BodyVertexDistance {
        body_a: usize,
        a: [i32; 3],
        body_b: usize,
        b: [i32; 3],
    },
}

/// A named length or angle parameter (expression stored verbatim, evaluated on demand).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub expression: String,
    #[serde(default)]
    pub deleted: bool,
    /// When set, [`expression`] is synced from geometry and the value is read-only.
    #[serde(default)]
    pub source: Option<ParameterSource>,
}

/// A 2D sketch hosted on a face. A single face may host multiple independent sketches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sketch {
    pub face: FaceId,
    /// User-visible label in the Elements pane; empty uses the default.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
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
    #[serde(default)]
    pub deleted: bool,
    /// Cubic-bezier tangent handles in face-local coords: `[near (x0,y0), near (x1,y1)]`.
    /// `None` means a straight segment (the common case).
    #[serde(default)]
    pub bezier: Option<[(f32, f32); 2]>,
    /// Set when this line is the bridging line created by a chamfer/fillet vertex treatment
    /// (#37/#38): the index of the (lower-index) trimmed line it nests under in the Elements
    /// pane (see [`crate::hierarchy`], #76). `None` for an ordinary line.
    #[serde(default)]
    pub chamfer_fillet_parent: Option<usize>,
    /// Set when this line is an **associative projection** of external 3D geometry into its
    /// sketch (#140): each geometry recompute re-resolves the source and rewrites the
    /// endpoints (see `crate::projection`). Projected lines render dashed in their own color
    /// (distinct from construction), are fixed (not draggable), and otherwise behave like
    /// construction geometry.
    #[serde(default)]
    pub projection: Option<ProjectionSource>,
}

/// Source geometry an associative projection tracks (#140). Body mesh edges are identified
/// by their quantized endpoints (the same geometry-keyed identity 3D selection uses, #156):
/// there is no stable topological name for mesh edges, so if a rebuild moves the source the
/// projection keeps its last resolved shape (a static fallback) rather than dangling.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProjectionSource {
    BodyEdge {
        body: usize,
        a: [i32; 3],
        b: [i32; 3],
    },
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
            deleted: false,
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
    let arcs: Vec<usize> = doc
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            !l.deleted && l.sketch == sketch && l.chamfer_fillet_parent.is_some() && l.is_curved()
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
            for (j, l) in doc.lines.iter().enumerate() {
                if j == arc || l.deleted || l.sketch != sketch || l.construction || l.is_curved()
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
    #[serde(default)]
    pub deleted: bool,
}

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
            deleted: false,
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
    #[serde(default)]
    pub deleted: bool,
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
    LineEndpoint { line: usize, end: LineEnd },
    CircleCenter(usize),
    /// A corner of an extrusion-backed face's own boundary loop (#26/#27): index into
    /// [`crate::extrude::face_boundary_loop_world`]'s ordered vertex list. Scoped to
    /// `FaceId::ExtrudeCap`/`FaceId::ExtrudeSide`; other face kinds never resolve. Fixed by
    /// the body's geometry, not draggable — mirrors [`ConstraintEntity::Origin`].
    FaceVertex { face: FaceId, index: usize },
    /// One of a sketch text's nine anchor points (#408): the bounding-box corners, edge
    /// midpoints, or centre. Solving moves the text's `origin` (the whole text translates
    /// rigidly); its rotation and size never change from constraints.
    TextAnchor { text: usize, anchor: TextAnchor },
    /// One of a tracing image's two calibration reference points (#425). Solving moves the
    /// image's `origin` (the whole image translates rigidly); its scale never changes from
    /// constraints. Only valid in sketches hosted on the image's plane.
    ImageCalibrationPoint { image: usize, index: usize },
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
    Line(usize),
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
    LineLength(usize),
    CircleDiameter(usize),
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
    Circle(usize),
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
    #[serde(default)]
    pub deleted: bool,
}

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
    Circle(usize),
    /// A closed loop of plain `Line`s, identified by its ordered line indices (#66).
    Polygon(Vec<usize>),
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
    TextGlyph { text: usize, glyph: usize },
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
    Plane(usize),
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
        op: usize,
        instance: usize,
    },
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
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    /// Parametric 3D edge chamfer/fillet bevels applied to this extrusion's own analytic
    /// side/cap edges (#77) — see [`EdgeTreatment`].
    #[serde(default)]
    pub edge_treatments: Vec<EdgeTreatment>,
}

/// The feature(s) that produced a solid body.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodySource {
    Extrusion(usize),
    Extrusions(Vec<usize>),
    /// A mesh body brought in via STL import (#70); indexes `Document::imported_meshes`
    /// rather than depending on a sketch-based feature.
    Imported(usize),
    /// A lofted solid; indexes `Document::lofts`.
    Loft(usize),
    /// A revolved solid (#revolve); indexes `Document::revolutions`.
    Revolve(usize),
    /// A swept solid (the Sweep tool, #sweep); indexes `Document::sweeps`.
    Sweep(usize),
    /// One repeated instance of one input of a linear repeat (Repeat tool): `op` indexes
    /// `Document::repeat_ops`; `target` is the input's position in the op's target list;
    /// `instance` counts from 1 (the original body is instance 0).
    Repeated {
        #[serde(rename = "repeat_op")]
        op: usize,
        #[serde(default)]
        target: usize,
        #[serde(default)]
        instance: usize,
    },
    /// The moved copy of one input of a move operation (Move tool): `op` indexes
    /// `Document::move_ops`, `target` is the position within that operation's input list.
    Moved {
        #[serde(rename = "move_op")]
        op: usize,
        #[serde(default)]
        target: usize,
    },
    /// The reflected copy of one input of a mirror operation (Mirror tool, #523): `op`
    /// indexes `Document::mirror_ops`, `target` is the input's position within that
    /// operation's target list. Unlike Move, the original input body is **kept** (not
    /// shadowed) — a mirror adds the reflection alongside the source.
    Mirrored {
        #[serde(rename = "mirror_op")]
        op: usize,
        #[serde(default)]
        target: usize,
    },
    /// One output solid of a boolean operation (Combine tool): `op` indexes
    /// `Document::boolean_ops`, `solid` is the ordinal of this body's solid within the
    /// operation's result (a cut or difference can split into several pieces). The last
    /// output body absorbs any extra solids a parametric rebuild produces, so the pane's
    /// element list stays stable while geometry changes.
    Boolean {
        op: usize,
        #[serde(default)]
        solid: usize,
    },
    /// One piece of a slice operation (Slice tool, #181): `op` indexes
    /// `Document::slice_ops`, `target` is the sliced input body's position in the op's
    /// target list, and `piece` is the ordinal of this fragment within that target's cut
    /// result. The input body becomes a shadow body; each fragment is its own `Body`.
    Sliced {
        #[serde(rename = "slice_op")]
        op: usize,
        #[serde(default)]
        target: usize,
        #[serde(default)]
        piece: usize,
    },
    /// The chamfered/filleted output of one input of an edge-treatment operation (#531): `op`
    /// indexes `Document::edge_treatment_ops`, `target` is the input's position within that
    /// operation's target list. The input body becomes a shadow body; this output carries the
    /// bevel.
    EdgeTreated {
        #[serde(rename = "edge_treatment_op")]
        op: usize,
        #[serde(default)]
        target: usize,
    },
    /// Additive extrusions with one or more extrusions **subtracted** (cut) from them (#35).
    /// Purely-additive bodies stay in the `Extrusion`/`Extrusions` forms; a body only takes
    /// this shape once it has a cut. `cut` is `#[serde(default)]` so any future add-only
    /// `Solid` serialization stays readable (existing saved files never carry a cut list —
    /// they load as `Extrusion`/`Extrusions` unchanged).
    Solid {
        add: Vec<usize>,
        #[serde(default)]
        cut: Vec<usize>,
    },
}

impl BodySource {
    pub fn single(extrusion: usize) -> Self {
        Self::Extrusion(extrusion)
    }

    /// Extrusions **added** to (fused into) the body.
    pub fn extrusion_indices(&self) -> &[usize] {
        match self {
            Self::Extrusion(index) => std::slice::from_ref(index),
            Self::Extrusions(indices) => indices.as_slice(),
            Self::Solid { add, .. } => add.as_slice(),
            Self::Loft(_)
            | Self::Revolve(_)
            | Self::Sweep(_)
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Mirrored { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. }
            | Self::EdgeTreated { .. } => &[],
            Self::Imported(_) => &[],
        }
    }

    /// Extrusions **subtracted** (cut) from the body (#35). Empty for every non-`Solid` form.
    pub fn cut_extrusion_indices(&self) -> &[usize] {
        match self {
            Self::Solid { cut, .. } => cut.as_slice(),
            Self::Extrusion(_)
            | Self::Extrusions(_)
            | Self::Imported(_)
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Sweep(_)
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Mirrored { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. }
            | Self::EdgeTreated { .. } => &[],
        }
    }

    pub fn imported_mesh_index(&self) -> Option<usize> {
        match self {
            Self::Imported(index) => Some(*index),
            Self::Extrusion(_)
            | Self::Extrusions(_)
            | Self::Solid { .. }
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Sweep(_)
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Mirrored { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. }
            | Self::EdgeTreated { .. } => None,
        }
    }

    /// Whether the body is built from `extrusion` in any role (added or cut).
    pub fn owns_extrusion(&self, extrusion: usize) -> bool {
        self.extrusion_indices().contains(&extrusion)
            || self.cut_extrusion_indices().contains(&extrusion)
    }

    pub fn append_extrusion(&mut self, extrusion: usize) {
        match self {
            Self::Extrusion(existing) => {
                *self = Self::Extrusions(vec![*existing, extrusion]);
            }
            Self::Extrusions(indices) => indices.push(extrusion),
            Self::Solid { add, .. } => add.push(extrusion),
            // An imported mesh body has no extrusion to merge into; unreachable in practice
            // since merge candidates only ever come from extrusion-backed bodies.
            Self::Imported(_)
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Sweep(_)
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Mirrored { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. }
            | Self::EdgeTreated { .. } => {}
        }
    }

    /// Register `extrusion` as a **cut** (subtraction) of this body (#35), moving the source
    /// into the `Solid` form if it wasn't already.
    pub fn append_cut_extrusion(&mut self, extrusion: usize) {
        match self {
            Self::Extrusion(existing) => {
                *self = Self::Solid {
                    add: vec![*existing],
                    cut: vec![extrusion],
                };
            }
            Self::Extrusions(indices) => {
                *self = Self::Solid {
                    add: std::mem::take(indices),
                    cut: vec![extrusion],
                };
            }
            Self::Solid { cut, .. } => cut.push(extrusion),
            // An imported mesh body has no solid feature to cut; unreachable in practice.
            Self::Imported(_)
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Sweep(_)
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Mirrored { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. }
            | Self::EdgeTreated { .. } => {}
        }
    }

    /// Remove `extrusion` from this source in whatever role it plays (e.g. undoing a merge or
    /// a cut). Collapses back to the simplest form once the cut list is empty (and to the
    /// single-extrusion form when one added index remains). No-op if `extrusion` isn't owned.
    /// Undo never removes a body's last/only *added* extrusion this way — that path tombstones
    /// the whole body instead.
    pub fn remove_extrusion(&mut self, extrusion: usize) {
        match self {
            Self::Extrusions(indices) => {
                indices.retain(|&ei| ei != extrusion);
                if let [only] = indices.as_slice() {
                    *self = Self::Extrusion(*only);
                }
            }
            Self::Solid { add, cut } => {
                add.retain(|&ei| ei != extrusion);
                cut.retain(|&ei| ei != extrusion);
                if cut.is_empty() {
                    *self = match add.as_slice() {
                        [only] => Self::Extrusion(*only),
                        _ => Self::Extrusions(std::mem::take(add)),
                    };
                }
            }
            Self::Extrusion(_)
            | Self::Imported(_)
            | Self::Loft(_)
            | Self::Revolve(_)
            | Self::Sweep(_)
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Mirrored { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. }
            | Self::EdgeTreated { .. } => {}
        }
    }
}

/// Whether any live operation (boolean or move) other than the excluded ones consumes
/// `body` on a side that shadows it — used when deleting/editing an operation to decide
/// whether an input body stays a shadow.
pub fn body_shadowed_by_other_ops(
    doc: &Document,
    body: usize,
    skip_boolean: Option<usize>,
    skip_move: Option<usize>,
    skip_slice: Option<usize>,
    skip_edge_treatment: Option<usize>,
) -> bool {
    doc.boolean_ops.iter().enumerate().any(|(oi, o)| {
        skip_boolean != Some(oi)
            && !o.deleted
            && (o.a.contains(&body) || (!o.keep_b && o.b.contains(&body)))
    }) || doc.move_ops.iter().enumerate().any(|(oi, o)| {
        skip_move != Some(oi) && !o.deleted && o.targets.contains(&body)
    }) || doc.slice_ops.iter().enumerate().any(|(oi, o)| {
        skip_slice != Some(oi) && !o.deleted && o.targets.contains(&body)
    }) || doc.edge_treatment_ops.iter().enumerate().any(|(oi, o)| {
        skip_edge_treatment != Some(oi) && !o.deleted && o.targets.contains(&body)
    })
}

/// Body index whose source includes `extrusion` (added or cut), if any.
pub fn body_index_for_extrusion(doc: &Document, extrusion: usize) -> Option<usize> {
    doc.bodies.iter().position(|body| {
        !body.deleted && body.source.owns_extrusion(extrusion)
    })
}

/// Body index whose source is `revolution` (#621) — the revolve analogue of
/// [`body_index_for_extrusion`].
pub fn body_index_for_revolution(doc: &Document, revolution: usize) -> Option<usize> {
    doc.bodies.iter().position(|body| {
        !body.deleted && matches!(body.source, BodySource::Revolve(r) if r == revolution)
    })
}

/// A solid body produced by a feature; it depends on its source feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub source: BodySource,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    /// A consumed boolean-operation input (Combine tool): still listed in the Elements
    /// pane (dimmed, its own icon) but hidden in the viewport except while hovered or
    /// selected there, where it renders ghosted.
    #[serde(default)]
    pub shadow: bool,
}

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
    #[serde(default)]
    pub deleted: bool,
}

/// How a lofted solid lands in the document (#479), mirroring [`SweepMode`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoftMode {
    #[default]
    NewBody,
    AddTo(Vec<usize>),
    Cut(Vec<usize>),
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
    Line(usize),
    /// One feature edge of a body's solid mesh, by its world-space endpoints (#643) — the same
    /// identity [`crate::construction::PickTargetKind::BodyEdge`] carries. Only the direction
    /// `a → b` matters to a linear repeat; a revolve/rotation also uses `a` as the pivot.
    BodyEdge {
        body: usize,
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
    AddTo(Vec<usize>),
    Cut(Vec<usize>),
}

/// A revolved solid: one or more coplanar closed profiles swept around an axis. Parametric
/// like everything else — the solid is rebuilt from the live profiles on every recompute.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revolution {
    pub sketch: SketchId,
    /// Closed profiles to sweep, same shape as [`Extrusion::faces`].
    pub faces: Vec<ExtrudeFace>,
    pub axis: RevolveAxis,
    /// Sweep angle in degrees (default 360 = a full solid of revolution).
    pub angle_deg: f32,
    /// Sweep `angle_deg/2` to each side of the profile plane instead of one way.
    #[serde(default)]
    pub symmetric: bool,
    /// How the solid lands (new body / fuse into bodies / cut bodies).
    pub mode: RevolveMode,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// How a swept solid lands in the document (#sweep): its own body, fused into
/// existing bodies, or subtracted from existing bodies (the cut list is user-picked).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepMode {
    NewBody,
    AddTo(Vec<usize>),
    Cut(Vec<usize>),
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
    pub path: Vec<usize>,
    /// How the solid lands (new body / fuse into bodies / cut bodies).
    pub mode: SweepMode,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

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
    pub a: Vec<usize>,
    /// Input bodies on the B side (cut/intersect/difference).
    #[serde(default)]
    pub b: Vec<usize>,
    /// Keep the B-side inputs as real bodies after the operation instead of shadowing them.
    #[serde(default)]
    pub keep_b: bool,
    /// Output body indices, in solid-ordinal order.
    #[serde(default)]
    pub outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// How a [`MoveOperation`]'s translation is specified (#648), the Move pane's Translate
/// dropdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveTranslateMode {
    /// Snap a picked **source point** on the moving bodies onto a picked **target point** on
    /// the stationary geometry (#650). The default.
    #[default]
    Snap,
    /// Type or drag X/Y/Z components outright (#648) — the pre-#648 behavior.
    Free,
}

/// A point on a body's mesh that a Move snaps from or onto (#649/#650): either a corner or
/// the midpoint of a feature edge. Keyed exactly like [`crate::hierarchy::SceneElement::
/// BodyVertex`]/`BodyEdge` — the body plus quantized world points — and resolved against the
/// body's live mesh, so it follows the geometry and simply stops resolving if a rebuild takes
/// it away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovePointRef {
    Vertex {
        body: usize,
        p: [i32; 3],
    },
    EdgeMidpoint {
        body: usize,
        a: [i32; 3],
        b: [i32; 3],
    },
}

impl MoveOperation {
    /// Whether this move's translation actually comes from its two snap points (#650). A Snap
    /// move that hasn't got both points yet — or one with no bodies at all, like a plane or
    /// image move — still reads its `tx`/`ty`/`tz` expressions, so the tool stays usable while
    /// the points are being picked and gizmo drags keep working.
    pub fn has_snap_translation(&self) -> bool {
        self.translate_mode == MoveTranslateMode::Snap
            && self.source_point.is_some()
            && self.target_point.is_some()
    }
}

impl MovePointRef {
    /// The body this point lives on — what tells a *moving* point from a stationary one.
    pub fn body(&self) -> usize {
        match self {
            MovePointRef::Vertex { body, .. } | MovePointRef::EdgeMidpoint { body, .. } => *body,
        }
    }
}

/// A move operation (Move tool, #176/#183): rigid translation and/or rotation applied to
/// whole bodies. Inputs become **shadow** bodies; each input gets a moved output body
/// (`BodySource::Moved`), and the operation itself is an editable pane element. The
/// translation components and angle are expressions, so moves are parameter-driven like
/// dimensions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveOperation {
    /// Input body indices, one output per entry (same order).
    pub targets: Vec<usize>,
    /// How the translation is specified (#648).
    #[serde(default)]
    pub translate_mode: MoveTranslateMode,
    /// The point on the moving bodies that a snap translation moves **from** (#649).
    #[serde(default)]
    pub source_point: Option<MovePointRef>,
    /// The point on the stationary geometry that a snap translation moves the source point
    /// **onto** (#650). With both set, the translation is `target - source` and the `tx`/`ty`/
    /// `tz` expressions are ignored.
    #[serde(default)]
    pub target_point: Option<MovePointRef>,
    /// Construction planes moved by this op (#217): transformed in place at recompute, so
    /// sketches/images anchored to them follow. No output bodies — the plane itself moves.
    #[serde(default)]
    pub plane_targets: Vec<usize>,
    /// Tracing images moved by this op (#217): their plane-local origin is transformed in
    /// place at recompute (projected onto the host plane), like a plane. No output bodies.
    #[serde(default)]
    pub image_targets: Vec<usize>,
    /// Translation components (mm expressions; empty = 0).
    #[serde(default)]
    pub tx: String,
    #[serde(default)]
    pub ty: String,
    #[serde(default)]
    pub tz: String,
    /// Rotation axis; `None` = no rotation.
    #[serde(default)]
    pub axis: Option<RevolveAxis>,
    /// Rotation angle (angle expression; empty = 0).
    #[serde(default)]
    pub angle: String,
    /// Output body indices, matching `targets` order.
    #[serde(default)]
    pub outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
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
    /// and the edge treatments do.
    pub fn consumes_input(self) -> bool {
        !matches!(self, MirrorMode::NewBody)
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
    /// Input body indices, one reflected output per entry (same order).
    pub targets: Vec<usize>,
    /// How each reflection lands (#639).
    #[serde(default)]
    pub mode: MirrorMode,
    /// Output body indices, matching `targets` order.
    #[serde(default)]
    pub outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

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
    pub targets: Vec<usize>,
    /// Source construction plane indices to repeat as offset copies (#221). Separate from
    /// `targets` (bodies) because a plane instance is a generated [`ConstructionPlane`] carrying
    /// a [`RepeatPlaneInstance`], not a [`BodySource::Repeated`] body.
    #[serde(default)]
    pub plane_targets: Vec<usize>,
    /// Cut **extrusion** indices whose *effect* is replayed at each offset (#220): the cutting
    /// tool is subtracted from its body again at every instance position (punching N holes),
    /// rather than copying a solid. No output bodies — the extra cuts fold into the target body's
    /// shape at build time (`occt_body_shape_from_indices`).
    #[serde(default)]
    pub extrusion_targets: Vec<usize>,
    /// Source sketch indices to repeat as offset copies (#226). Each copy rides a generated
    /// construction plane parallel to the source's, translated along the axis, so its entities
    /// keep their plane-local coords and step by the offset in world. Restricted to
    /// construction-plane-hosted sketches.
    #[serde(default)]
    pub sketch_targets: Vec<usize>,
    /// Generated host-plane indices for the sketch copies (#226), instance-major then target.
    #[serde(default)]
    pub sketch_plane_outputs: Vec<usize>,
    /// Generated copy-sketch indices (#226), instance-major then target. Each copy's lines and
    /// circles are found by sketch membership (not tracked separately).
    #[serde(default)]
    pub sketch_outputs: Vec<usize>,
    pub axis: RevolveAxis,
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
    /// Output body indices: instance-major, then target (instance 1 of each target, then
    /// instance 2 of each target, …).
    #[serde(default)]
    pub outputs: Vec<usize>,
    /// Generated construction-plane instance indices for [`plane_targets`] (#221), laid out
    /// instance-major then target, exactly like [`outputs`]. Each entry is a
    /// [`ConstructionPlane`] whose `repeat_instance` points back here.
    #[serde(default)]
    pub plane_outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// Back-reference stamped on a generated construction plane that is one instance of a
/// [`RepeatOperation`]'s plane repeat (#221). The instance's frame is derived at recompute from
/// the source plane's *current* frame offset along the op's axis, so it follows the source if the
/// source plane itself moves — the same "cache derived from another element" pattern moved images
/// use (#217).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepeatPlaneInstance {
    /// Index into [`Document::repeat_ops`].
    pub op: usize,
    /// Index into the op's [`RepeatOperation::plane_targets`].
    pub target: usize,
    /// 1-based instance number; the along-axis offset is `repeat_offsets(op)[instance - 1]`.
    pub instance: usize,
}

/// A slice operation (Slice tool, #181): cuts whole bodies with one or more planar
/// cutters (construction planes or planar body faces), splitting each target into the
/// fragments that fall on either side. Each input body becomes a **shadow** body; every
/// fragment is a fresh [`Body`] with a [`BodySource::Sliced`] source, and the operation
/// itself is an editable pane element — fragments depend on the operation, the operation
/// depends on every target and cutter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SliceOperation {
    /// Input body indices (the A side); each is sliced independently.
    pub targets: Vec<usize>,
    /// Planar cutters (the B side): construction planes and/or planar body faces.
    #[serde(default)]
    pub cutters: Vec<FaceId>,
    /// When set, each cutter divides the whole target (its plane extends infinitely).
    /// When clear, a cutter only separates material within its own face footprint.
    #[serde(default)]
    pub extend_infinite: bool,
    /// Output body indices: target-major, then piece (all fragments of target 0, then
    /// target 1, …). The last fragment of each target absorbs any extra solids a rebuild
    /// produces, so the pane's element list stays stable while geometry changes.
    #[serde(default)]
    pub outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// One edge treated by an [`EdgeTreatmentOperation`] (#531): the stable, parametric edge
/// identity is the extrusion-relative [`ExtrusionEdgeRef`] (a topological face/edge address
/// that re-resolves to live world coordinates on every rebuild), **not** a coordinate snapshot
/// — so a chamfer/fillet follows its edge when a parameter reshapes the body. `target` says
/// which of the op's input bodies the edge lives on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreatedEdge {
    /// Index into the owning op's `targets` list (which input body this edge belongs to).
    pub target: usize,
    /// The extrusion whose analytic edge is treated.
    pub extrusion: usize,
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
    /// Input body indices — each is shadowed and gets one chamfered/filleted output.
    pub targets: Vec<usize>,
    /// Edges to treat, each tagged with the `targets` entry it lives on.
    pub edges: Vec<TreatedEdge>,
    pub kind: VertexTreatmentKind,
    /// Chamfer distance / fillet radius (mm); must be positive to have any effect.
    pub amount: f32,
    /// Output body indices, matching `targets` order.
    #[serde(default)]
    pub outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

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
    pub line_targets: Vec<usize>,
    /// Source circle indices to duplicate.
    #[serde(default)]
    pub circle_targets: Vec<usize>,
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
    pub line_outputs: Vec<usize>,
    /// Generated circle-copy indices, instance-major then target.
    #[serde(default)]
    pub circle_outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

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
    pub line_targets: Vec<usize>,
    /// Source circle indices to offset.
    #[serde(default)]
    pub circle_targets: Vec<usize>,
    /// Signed offset distance expression (mm): positive grows a closed loop/circle,
    /// negative shrinks (or flips an open chain's side).
    #[serde(default)]
    pub distance: String,
    /// Emit the offset copies as construction geometry.
    #[serde(default)]
    pub construction: bool,
    /// Generated line indices, aligned with `line_targets`.
    #[serde(default)]
    pub line_outputs: Vec<usize>,
    /// Generated circle indices, aligned with `circle_targets`.
    #[serde(default)]
    pub circle_outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// A 2D in-sketch mirror (Mirror tool inside a sketch, #523): reflects the picked lines and
/// circles across a mirror line, emitting the reflections as separate `Line`/`Circle` entries
/// grouped under the op and regenerated whenever the sources or the mirror line change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SketchMirrorOperation {
    /// The sketch the sources live in; outputs land in the same sketch.
    pub sketch: SketchId,
    /// The mirror line: a straight sketch line whose infinite extension is the mirror axis.
    pub line: usize,
    /// Source line indices to reflect.
    #[serde(default)]
    pub line_targets: Vec<usize>,
    /// Source circle indices to reflect.
    #[serde(default)]
    pub circle_targets: Vec<usize>,
    /// Generated line indices, aligned with `line_targets`.
    #[serde(default)]
    pub line_outputs: Vec<usize>,
    /// Generated circle indices, aligned with `circle_targets`.
    #[serde(default)]
    pub circle_outputs: Vec<usize>,
    /// Generated coincidence-constraint indices reflecting the sources' shared corners onto the
    /// outputs (#547), so a mirrored polygon's reflected edges join into a fillable face.
    /// Tombstoned and regenerated on every rebuild, like the output geometry.
    #[serde(default)]
    pub constraint_outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

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
    pub line_targets: Vec<usize>,
    /// Cutter line indices (the B side); interior crossings with these divide each target.
    #[serde(default)]
    pub cutter_lines: Vec<usize>,
    /// Target circle indices (#237); each is split into arcs where the cutters cross it. The arcs
    /// are emitted as curved (bezier) fragment lines, the source circle is shadowed.
    #[serde(default)]
    pub circle_targets: Vec<usize>,
    /// Target **face** loops (#238): each entry is the line indices of a closed sketch face to
    /// slice. The cutter is expected to cross the loop's boundary at two points; the two crossed
    /// boundary edges are split, a cut **chord** is emitted between the crossings, and coincidence
    /// constraints are generated so the loop resolves into two faces (see `rebuild_sketch_slice`).
    #[serde(default)]
    pub face_targets: Vec<Vec<usize>>,
    /// Generated fragment-line indices, target-major (all fragments of target 0, then target 1…).
    /// Both split lines *and* split-circle arcs land here (arcs are bezier `Line`s); face-slice
    /// boundary fragments and cut chords land here too.
    #[serde(default)]
    pub line_outputs: Vec<usize>,
    /// Generated coincidence-constraint indices (#238) that stitch a face slice's fragments into
    /// two loops. Tombstoned and regenerated on every rebuild, like `line_outputs`.
    #[serde(default)]
    pub constraint_outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

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
    pub line_targets: Vec<usize>,
    #[serde(default)]
    pub corners: Vec<SketchVertexTreatmentCorner>,
    /// Generated trimmed copies, index-aligned with `line_targets` (output i is the trimmed
    /// copy of source line_targets[i]). Regenerated each rebuild; reuse slots when possible.
    #[serde(default)]
    pub line_outputs: Vec<usize>,
    /// Generated bridge lines, index-aligned with `corners`.
    #[serde(default)]
    pub bridge_outputs: Vec<usize>,
    /// Generated stitch coincidence constraints; tombstoned+regenerated each rebuild.
    #[serde(default)]
    pub constraint_outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

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
    #[serde(default)]
    pub deleted: bool,
}

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
    pub plane: usize,
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
    #[serde(default)]
    pub deleted: bool,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportedMesh {
    pub triangles: Vec<[glam::Vec3; 3]>,
    /// Source file name (without extension), used as the default body name.
    pub source_name: String,
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

/// One view on a technical [`Drawing`] (#180): a body projected in a fixed orientation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingView {
    /// Index into [`Document::bodies`] (the source when `sketch` is `None`).
    pub body: usize,
    /// When `Some`, this view projects a **sketch** rather than a body (#278). Kept as an
    /// optional field (rather than replacing `body` with an enum) so older saved drawings —
    /// which only ever had `body` — deserialize unchanged.
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
    /// Draw dashed projection lines from the base view to this aligned child (#377): two
    /// lightweight lines connecting the outer silhouette extremes of the two views across
    /// the gap between them. Only meaningful while `aligned_parent` is set.
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
    #[serde(default)]
    pub deleted: bool,
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
    pub annotations: Vec<DrawingAnnotation>,
}

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
    #[serde(default)]
    pub deleted: bool,
}

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

impl Default for Drawing {
    fn default() -> Self {
        Self {
            name: None,
            views: Vec::new(),
            deleted: false,
            page_width_mm: default_page_width_mm(),
            page_height_mm: default_page_height_mm(),
            margin_mm: default_page_margin_mm(),
            annotations: Vec::new(),
        }
    }
}

/// The whole document: sketches, sketch primitives, constraints, and construction planes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub parameters: Vec<Parameter>,
    pub sketches: Vec<Sketch>,
    pub lines: Vec<Line>,
    pub circles: Vec<Circle>,
    pub constraints: Vec<Constraint>,
    pub construction_planes: Vec<ConstructionPlane>,
    #[serde(default)]
    pub extrusions: Vec<Extrusion>,
    #[serde(default)]
    pub bodies: Vec<Body>,
    #[serde(default)]
    pub imported_meshes: Vec<ImportedMesh>,
    /// Reference images imported for tracing (#163/#169).
    #[serde(default)]
    pub tracing_images: Vec<TracingImage>,
    /// Loft features (solids blended through cross sections).
    #[serde(default)]
    pub lofts: Vec<Loft>,
    /// Revolved solids (#revolve).
    #[serde(default)]
    pub revolutions: Vec<Revolution>,
    /// Swept solids (the Sweep tool, #sweep).
    #[serde(default)]
    pub sweeps: Vec<Sweep>,
    /// Boolean operations between bodies (the Combine tool).
    #[serde(default)]
    pub boolean_ops: Vec<BooleanOperation>,
    /// Move operations on bodies (the Move tool, #176/#183).
    #[serde(default)]
    pub move_ops: Vec<MoveOperation>,
    /// Mirror operations on bodies (the Mirror tool, #523).
    #[serde(default)]
    pub mirror_ops: Vec<MirrorOperation>,
    /// Linear repeats on bodies (the Repeat tool, #182).
    #[serde(default)]
    pub repeat_ops: Vec<RepeatOperation>,
    /// Slice operations on bodies (the Slice tool, #181).
    #[serde(default)]
    pub slice_ops: Vec<SliceOperation>,
    /// Edge chamfer/fillet operations on bodies (#531): each shadows its input bodies and
    /// produces beveled output bodies.
    #[serde(default)]
    pub edge_treatment_ops: Vec<EdgeTreatmentOperation>,
    /// 2D in-sketch linear repeats (#222): duplicated sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_repeat_ops: Vec<SketchRepeatOperation>,
    /// 2D in-sketch slices (#224): split sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_slice_ops: Vec<SketchSliceOperation>,
    /// 2D in-sketch offsets: parallel sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_offset_ops: Vec<SketchOffsetOperation>,
    /// 2D in-sketch mirrors (#523): reflected sketch entities grouped under an op.
    #[serde(default)]
    pub sketch_mirror_ops: Vec<SketchMirrorOperation>,
    /// 2D in-sketch chamfer/fillet operations (#538): shadowed source edges plus regenerated
    /// trimmed copies + bridge lines, grouped under an op.
    #[serde(default)]
    pub sketch_vertex_treatment_ops: Vec<SketchVertexTreatmentOperation>,
    /// Sketch text elements (#282): baked glyph outlines + embedded font, per sketch.
    #[serde(default)]
    pub sketch_texts: Vec<SketchText>,
    /// Technical drawings (#180): black-on-white projected sheets of bodies for print/PDF.
    #[serde(default)]
    pub drawings: Vec<Drawing>,
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
    pub components: Vec<Component>,
    /// Component membership (#423): which component each assigned top-level element belongs
    /// to, as `(member kind, element index, component index)`. Elements without an entry sit
    /// directly under the document root. Tombstoned elements may leave stale entries; lookups
    /// go through live elements only.
    #[serde(default)]
    pub component_members: Vec<(ComponentMember, usize, usize)>,
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
    pub parent: Option<usize>,
    /// Length-unit override; `None` inherits from the parent chain, then the document.
    #[serde(default)]
    pub length_unit: Option<LengthUnit>,
    /// Angle-unit override; `None` inherits like `length_unit`.
    #[serde(default)]
    pub angle_unit: Option<AngleUnit>,
    #[serde(default)]
    pub deleted: bool,
}

/// The kinds of top-level element a component can hold (#423) — the Elements pane's root
/// rows. Nested elements (sketches on a plane, bodies under an op) follow their root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentMember {
    ConstructionPlane,
    Extrusion,
    Body,
    Loft,
    BooleanOp,
    MoveOp,
    MirrorOp,
    RepeatOp,
    SliceOp,
    EdgeTreatmentOp,
    Revolution,
    Sweep,
    Drawing,
}

impl Document {
    /// The component an assigned top-level element belongs to, if any (#423).
    pub fn component_of(&self, kind: ComponentMember, index: usize) -> Option<usize> {
        self.component_members
            .iter()
            .find(|(k, i, _)| *k == kind && *i == index)
            .map(|(_, _, c)| *c)
            .filter(|&c| self.components.get(c).is_some_and(|comp| !comp.deleted))
    }

    /// Assign (or with `None`, unassign) a top-level element to a component (#423).
    pub fn set_component_member(
        &mut self,
        kind: ComponentMember,
        index: usize,
        component: Option<usize>,
    ) {
        self.component_members
            .retain(|(k, i, _)| !(*k == kind && *i == index));
        if let Some(c) = component {
            self.component_members.push((kind, index, c));
        }
    }

    /// Walk a component's parent chain (self first). Cycles are cut off defensively.
    pub fn component_chain(&self, component: usize) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut cur = Some(component);
        while let Some(c) = cur {
            if chain.contains(&c) || self.components.get(c).is_none_or(|comp| comp.deleted) {
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
pub fn effective_component_length_unit(doc: &Document, component: usize) -> LengthUnit {
    doc.component_chain(component)
        .into_iter()
        .find_map(|c| doc.components[c].length_unit)
        .unwrap_or(doc.default_length_unit)
}

/// Effective angle unit for a component (#423), like [`effective_component_length_unit`].
pub fn effective_component_angle_unit(doc: &Document, component: usize) -> AngleUnit {
    doc.component_chain(component)
        .into_iter()
        .find_map(|c| doc.components[c].angle_unit)
        .unwrap_or(doc.default_angle_unit)
}

/// The component a sketch's geometry belongs to (#423): resolved through the sketch's host
/// face — a construction plane's own assignment (or, for a face-anchored plane, the host
/// sketch's component), or the owning extrusion's assignment for a body-face sketch.
pub fn sketch_component(doc: &Document, sketch: SketchId) -> Option<usize> {
    fn plane_component(doc: &Document, plane: usize, depth: u8) -> Option<usize> {
        if depth > 8 {
            return None;
        }
        if let Some(c) = doc.component_of(ComponentMember::ConstructionPlane, plane) {
            return Some(c);
        }
        match doc.construction_planes.get(plane)?.parent {
            ConstructionPlaneParent::Root => None,
            ConstructionPlaneParent::Sketch(s) => sketch_component_inner(doc, s, depth + 1),
        }
    }
    fn sketch_component_inner(doc: &Document, sketch: SketchId, depth: u8) -> Option<usize> {
        if depth > 8 {
            return None;
        }
        match doc.sketch_face(sketch)? {
            FaceId::ConstructionPlane(p) => plane_component(doc, p, depth + 1),
            FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
                doc.component_of(ComponentMember::Extrusion, extrusion).or_else(|| {
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
            parameters: Vec::new(),
            sketches: Vec::new(),
            lines: Vec::new(),
            circles: Vec::new(),
            constraints: Vec::new(),
            construction_planes: vec![default_xy_plane()],
            extrusions: Vec::new(),
            bodies: Vec::new(),
            imported_meshes: Vec::new(),
            tracing_images: Vec::new(),
            lofts: Vec::new(),
            revolutions: Vec::new(),
            sweeps: Vec::new(),
            boolean_ops: Vec::new(),
            move_ops: Vec::new(),
            mirror_ops: Vec::new(),
            repeat_ops: Vec::new(),
            slice_ops: Vec::new(),
            edge_treatment_ops: Vec::new(),
            sketch_repeat_ops: Vec::new(),
            sketch_offset_ops: Vec::new(),
            sketch_mirror_ops: Vec::new(),
            sketch_vertex_treatment_ops: Vec::new(),
            sketch_slice_ops: Vec::new(),
            sketch_texts: Vec::new(),
            drawings: Vec::new(),
            shape_order: Vec::new(),
            undo_groups: Vec::new(),
            default_length_unit: LengthUnit::default(),
            default_angle_unit: AngleUnit::default(),
            components: Vec::new(),
            component_members: Vec::new(),
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
            .enumerate()
            .filter_map(move |(i, s)| (s.face == face).then_some(i))
    }

    #[allow(dead_code)] // query helper; now exercised only by tests since undo went snapshot-based (#194)
    pub fn sketch_has_geometry(&self, sketch: SketchId) -> bool {
        self.lines.iter().any(|l| l.sketch == sketch)
            || self.circles.iter().any(|c| c.sketch == sketch)
    }

    #[allow(dead_code)] // query helper; now exercised only by tests
    pub fn has_children(&self, face: &FaceId) -> bool {
        self.sketches.iter().any(|s| &s.face == face)
    }

    pub fn add_sketch(&mut self, face: FaceId) -> SketchId {
        let id = self.sketches.len();
        self.sketches.push(Sketch {
            face,
            name: None,
            deleted: false,
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
    use super::*;

    #[test]
    fn legacy_horizontal_vertical_constraints_migrate_to_axis_parallel() {
        // #577/#580: old documents storing `horizontal`/`vertical` constraint tags load by mapping
        // them to Parallel against the X/Y sketch axis.
        let horizontal: ConstraintKind =
            serde_json::from_str(r#"{"horizontal":{"line":{"line":3}}}"#).unwrap();
        assert_eq!(
            horizontal,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(3),
                line_b: ConstraintLine::OriginAxis(SketchAxis::X),
            }
        );
        let vertical: ConstraintKind =
            serde_json::from_str(r#"{"vertical":{"line":{"line":7}}}"#).unwrap();
        assert_eq!(
            vertical,
            ConstraintKind::Parallel {
                line_a: ConstraintLine::Line(7),
                line_b: ConstraintLine::OriginAxis(SketchAxis::Y),
            }
        );
        // A normal constraint still round-trips unchanged.
        let parallel = ConstraintKind::Parallel {
            line_a: ConstraintLine::Line(0),
            line_b: ConstraintLine::Line(1),
        };
        let json = serde_json::to_string(&parallel).unwrap();
        assert!(!json.contains("horizontal") && !json.contains("vertical"));
        assert_eq!(serde_json::from_str::<ConstraintKind>(&json).unwrap(), parallel);
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let line = Line::from_local_endpoints(sketch, 0.0, 0.0, 3.0, 4.0);
        assert!((line.length() - 5.0).abs() < 1e-4);
    }

    #[test]
    fn straight_line_samples_to_just_its_two_endpoints() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let line = Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0);
        assert_eq!(line.sample_local(BEZIER_SEGMENTS), vec![(0.0, 0.0), (10.0, 0.0)]);
        assert!(!line.is_curved());
    }

    #[test]
    fn curved_line_samples_pass_through_both_endpoints() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let line = Line::from_local_endpoints(sketch, 0.0, 0.0, 3.0, 4.0);
        assert_eq!(line.length(), line.chord_length());
    }

    #[test]
    fn curved_line_length_is_the_arc_not_the_chord() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
            Line::from_local_endpoints(0, geom.p1.0, geom.p1.1, geom.p2.0, geom.p2.1);
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
            Line::from_local_endpoints(0, geom.p1.0, geom.p1.1, geom.p2.0, geom.p2.1);
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
        assert_eq!(FaceId::from_script("circle", 2), Some(FaceId::Circle(2)));
    }

    #[test]
    fn multiple_sketches_on_one_face() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        let s1 = doc.add_sketch(FaceId::ConstructionPlane(0));
        assert_ne!(s0, s1);
        let on_plane: Vec<_> = doc.sketches_on_face(FaceId::ConstructionPlane(0)).collect();
        assert_eq!(on_plane, vec![0, 1]);
    }

    #[test]
    fn sketch_has_geometry_detects_primitives() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        assert!(!doc.sketch_has_geometry(sketch));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 1.0, 1.0));
        assert!(doc.sketch_has_geometry(sketch));
    }

    #[test]
    fn circle_diameter_is_twice_radius() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        assert_eq!(doc.sketches[sketch].length_unit, None);
        assert_eq!(doc.sketches[sketch].angle_unit, None);
        assert_eq!(effective_length_unit(&doc, sketch), LengthUnit::Mm);
        assert_eq!(effective_angle_unit(&doc, sketch), AngleUnit::Deg);
    }

    #[test]
    fn effective_units_follow_document_default_change() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.default_length_unit = LengthUnit::In;
        doc.default_angle_unit = AngleUnit::Rad;
        assert_eq!(effective_length_unit(&doc, sketch), LengthUnit::In);
        assert_eq!(effective_angle_unit(&doc, sketch), AngleUnit::Rad);
    }

    #[test]
    fn sketch_override_takes_precedence_over_document_default() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
        assert_eq!(effective_length_unit(&doc, 99), LengthUnit::Mm);
        assert_eq!(effective_angle_unit(&doc, 99), AngleUnit::Deg);
    }
}