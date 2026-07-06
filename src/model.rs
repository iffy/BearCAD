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
            FaceId::Circle(_) | FaceId::Polygon(_) | FaceId::ConstructionPlane(_) => {
                None
            }
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
    if alpha < VERTEX_TREATMENT_DEGENERATE_EPS
        || alpha > std::f32::consts::PI - VERTEX_TREATMENT_DEGENERATE_EPS
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
        if alpha < VERTEX_TREATMENT_DEGENERATE_EPS
            || alpha > std::f32::consts::PI - VERTEX_TREATMENT_DEGENERATE_EPS
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    Horizontal { line: ConstraintLine },
    Vertical { line: ConstraintLine },
    Angle {
        line_a: ConstraintLine,
        line_b: ConstraintLine,
        /// +1: movable line rotates counterclockwise from reference; -1: clockwise.
        #[serde(default = "default_constraint_sign")]
        rotation_sign: ConstraintSign,
    },
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
    pub distance: f32,
    /// When set, the depth is constrained to reach this object's extended plane.
    #[serde(default)]
    pub target: Option<ExtrudeTarget>,
    /// Optional expression driving `distance` (empty = free/gizmo-driven, no constraint).
    #[serde(default)]
    pub expression: String,
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
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. } => &[],
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
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. } => &[],
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
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. } => None,
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
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. } => {}
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
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. } => {}
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
            | Self::Boolean { .. }
            | Self::Moved { .. }
            | Self::Repeated { .. }
            | Self::Sliced { .. } => {}
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
) -> bool {
    doc.boolean_ops.iter().enumerate().any(|(oi, o)| {
        skip_boolean != Some(oi)
            && !o.deleted
            && (o.a.contains(&body) || (!o.keep_b && o.b.contains(&body)))
    }) || doc.move_ops.iter().enumerate().any(|(oi, o)| {
        skip_move != Some(oi) && !o.deleted && o.targets.contains(&body)
    }) || doc.slice_ops.iter().enumerate().any(|(oi, o)| {
        skip_slice != Some(oi) && !o.deleted && o.targets.contains(&body)
    })
}

/// Body index whose source includes `extrusion` (added or cut), if any.
pub fn body_index_for_extrusion(doc: &Document, extrusion: usize) -> Option<usize> {
    doc.bodies.iter().position(|body| {
        !body.deleted && body.source.owns_extrusion(extrusion)
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
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

/// One loft cross section: a closed profile (`ExtrudeFace`) plus the sketch it lives in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoftSection {
    pub sketch: SketchId,
    pub face: ExtrudeFace,
}

/// The axis a [`Revolution`] sweeps around: a line in the profile's own sketch (plain,
/// construction, or projected — any line works), or one of the origin's global axes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevolveAxis {
    Line(usize),
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

/// A move operation (Move tool, #176/#183): rigid translation and/or rotation applied to
/// whole bodies. Inputs become **shadow** bodies; each input gets a moved output body
/// (`BodySource::Moved`), and the operation itself is an editable pane element. The
/// translation components and angle are expressions, so moves are parameter-driven like
/// dimensions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveOperation {
    /// Input body indices, one output per entry (same order).
    pub targets: Vec<usize>,
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
}

impl RepeatMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::CountGap => "Count × gap",
            Self::CountFitEnds => "Count fit (to end)",
            Self::CountFitCenters => "Count fit (start-to-start)",
            Self::FillGap => "Fill length, gap",
            Self::FillPitch => "Fill length, pitch",
            Self::FillMaxPitch => "Fill length, max pitch",
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
            _ => None,
        }
    }

    /// Whether the mode uses the count `n` (vs deriving it from the length).
    pub fn uses_count(self) -> bool {
        matches!(self, Self::CountGap | Self::CountFitEnds | Self::CountFitCenters)
    }

    /// Whether the mode uses the fill length `length`.
    pub fn uses_length(self) -> bool {
        !matches!(self, Self::CountGap)
    }
}

/// A linear repeat (Repeat tool, #182): copies of whole bodies spaced along an axis. The
/// original stays as instance 0; each further instance of each target gets an output body
/// (`BodySource::Repeated`). Count/spacing/length are expressions, so repeats rebuild
/// parametrically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepeatOperation {
    pub targets: Vec<usize>,
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
    /// Output body indices: instance-major, then target (instance 1 of each target, then
    /// instance 2 of each target, …).
    #[serde(default)]
    pub outputs: Vec<usize>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deleted: bool,
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
    /// A boolean operation between bodies (its output bodies are separate `Body` entries).
    BooleanOperation,
    /// A move operation on bodies (its output bodies are separate `Body` entries).
    MoveOperation,
    /// A linear repeat on bodies (its output bodies are separate `Body` entries).
    RepeatOperation,
    /// A slice operation on bodies (its fragment bodies are separate `Body` entries).
    SliceOperation,
    /// An in-place edit of an existing construction plane (undo restores the prior planes).
    /// Transient: never persisted (storage rebuilds `shape_order` from created shapes only).
    ConstructionPlaneEdit,
    /// An in-place 3D chamfer/fillet commit (#168): undo restores the extrusion's prior
    /// `edge_treatments` list from the snapshot stack. Transient, like
    /// [`ShapeKind::ConstructionPlaneEdit`].
    EdgeTreatmentEdit,
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
    /// Boolean operations between bodies (the Combine tool).
    #[serde(default)]
    pub boolean_ops: Vec<BooleanOperation>,
    /// Move operations on bodies (the Move tool, #176/#183).
    #[serde(default)]
    pub move_ops: Vec<MoveOperation>,
    /// Linear repeats on bodies (the Repeat tool, #182).
    #[serde(default)]
    pub repeat_ops: Vec<RepeatOperation>,
    /// Slice operations on bodies (the Slice tool, #181).
    #[serde(default)]
    pub slice_ops: Vec<SliceOperation>,
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
            boolean_ops: Vec::new(),
            move_ops: Vec::new(),
            repeat_ops: Vec::new(),
            slice_ops: Vec::new(),
            shape_order: Vec::new(),
            undo_groups: Vec::new(),
            default_length_unit: LengthUnit::default(),
            default_angle_unit: AngleUnit::default(),
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
        .unwrap_or(doc.default_length_unit)
}

/// Effective default angle unit for `sketch`: its own override, or the document default if
/// unset or the sketch doesn't exist (#52).
pub fn effective_angle_unit(doc: &Document, sketch: SketchId) -> AngleUnit {
    doc.sketches
        .get(sketch)
        .and_then(|s| s.angle_unit)
        .unwrap_or(doc.default_angle_unit)
}

#[cfg(test)]
mod tests {
    use super::*;

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