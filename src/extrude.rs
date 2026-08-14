//! Extrusions: turning coplanar sketch faces into 3D solid meshes.
//!
//! Stage 1 builds the data-driven solid geometry (a prism/cylinder per face) from an
//! [`Extrusion`]. Rendering and the interactive tool layer build on top of this.
// The mesh API is exercised by tests and consumed by the (next-stage) GPU renderer.
#![allow(dead_code)]

use crate::face::{local_to_world, sketch_frame, sketch_geometry_frame, SketchFrame};
use crate::geometric_constraints::point_uv;
use crate::model::{
    vertex_treatment_geometry, Document, EdgeTreatment, ExtrudeFace, ExtrudeTarget, ExtrudeTaperMode,
    Extrusion, ExtrusionEdgeRef, FaceId, TreatableSolid, VertexTreatmentKind,
};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// Number of segments used to facet a circular profile.
pub const CIRCLE_SEGMENTS: usize = 48;

/// A triangle solid mesh in world space (3 positions per triangle).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SolidMesh {
    pub triangles: Vec<[Vec3; 3]>,
}

impl SolidMesh {
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Axis-aligned bounds of all triangle vertices, if any.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut iter = self.triangles.iter().flat_map(|t| t.iter());
        let first = *iter.next()?;
        let mut min = first;
        let mut max = first;
        for p in iter {
            min = min.min(*p);
            max = max.max(*p);
        }
        Some((min, max))
    }
}

/// Above this angle between two adjacent triangles, the edge between them is a **crease** —
/// a real edge of the model — and their normals are not averaged across it (#1037). Below it
/// the edge is an artefact of tessellating a curved surface and gets smoothed away.
///
/// A 64-segment cylinder bends ~5.6° per facet and a chamfer meets its faces at 45° or more,
/// so 30° smooths every curved wall while leaving box corners, chamfers, and extrusion caps
/// crisp.
pub const CREASE_ANGLE_DEG: f32 = 30.0;

/// Per-vertex normals for smooth shading (#1037): three normals per triangle, parallel to
/// `mesh.triangles`.
///
/// For each corner, the normals of every triangle meeting at that position are averaged —
/// but only those within [`CREASE_ANGLE_DEG`] of the corner's own triangle, so smoothing
/// never rounds a real edge. The contributions are the raw cross products, which weights
/// each triangle by its area and keeps a sliver from swinging the result.
///
/// Deriving normals this way rather than reading them off OCCT's `Poly_Triangulation` means
/// analytic primitive meshes, the hand-rolled fallbacks, and kernel output all get the same
/// treatment through one code path. At the 0.05 mm deflection this app tessellates to, the
/// difference from true surface normals is not visible.
pub fn smooth_normals(mesh: &SolidMesh) -> Vec<[Vec3; 3]> {
    // Weld corners that coincide to within a micrometre. Adjacent kernel faces share exact
    // node positions, but the analytic meshers close their seams by recomputation, so an
    // exact bit compare would leave a visible stripe down every cylinder.
    let key = |p: Vec3| {
        (
            (p.x * 1000.0).round() as i64,
            (p.y * 1000.0).round() as i64,
            (p.z * 1000.0).round() as i64,
        )
    };
    // Un-normalized face normals: direction plus twice the area, which is exactly the
    // weighting the averaging wants.
    let face_normals: Vec<Vec3> = mesh
        .triangles
        .iter()
        .map(|[a, b, c]| (*b - *a).cross(*c - *a))
        .collect();
    let mut at_position: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (fi, tri) in mesh.triangles.iter().enumerate() {
        for corner in tri {
            at_position.entry(key(*corner)).or_default().push(fi);
        }
    }
    let crease_cos = CREASE_ANGLE_DEG.to_radians().cos();
    mesh.triangles
        .iter()
        .enumerate()
        .map(|(fi, tri)| {
            let own = face_normals[fi].normalize_or_zero();
            let mut corners = [own; 3];
            for (ci, corner) in tri.iter().enumerate() {
                let Some(sharing) = at_position.get(&key(*corner)) else {
                    continue;
                };
                let mut sum = Vec3::ZERO;
                for &other in sharing {
                    let n = face_normals[other];
                    // Two-sided: a neighbour wound the other way describes the same
                    // surface, so compare (and accumulate) it flipped rather than
                    // discarding it as a crease.
                    let unit = n.normalize_or_zero();
                    let aligned = if unit.dot(own) < 0.0 { -n } else { n };
                    if aligned.normalize_or_zero().dot(own) >= crease_cos {
                        sum += aligned;
                    }
                }
                let smoothed = sum.normalize_or_zero();
                corners[ci] = if smoothed.length_squared() > 0.0 {
                    smoothed
                } else {
                    own
                };
            }
            corners
        })
        .collect()
}

/// Signed volume of a closed mesh via the divergence theorem
/// (`sum(dot(a, cross(b, c))) / 6`). Negative when the winding is inward; callers that want
/// a physical volume take the absolute value. Used by the treatment tests as an independent
/// sanity check and by `bearcad.body_stats` (#107).
pub(crate) fn mesh_signed_volume(mesh: &SolidMesh) -> f32 {
    mesh.triangles
        .iter()
        .map(|[a, b, c]| a.dot(b.cross(*c)) / 6.0)
        .sum()
}

/// Whether `mesh` is a closed (watertight) manifold: every undirected edge is shared by exactly two
/// triangles (#582). Vertices are snapped to a micrometre grid so a shared edge compares equal
/// across independent floating-point paths. An open shell — e.g. a lofted extrusion that came back
/// without its end caps — has boundary edges used by a single triangle, so this returns false.
pub(crate) fn mesh_is_watertight(mesh: &SolidMesh) -> bool {
    use std::collections::HashMap;
    let key = |p: Vec3| {
        (
            (p.x * 1000.0).round() as i64,
            (p.y * 1000.0).round() as i64,
            (p.z * 1000.0).round() as i64,
        )
    };
    let mut edge_count: HashMap<((i64, i64, i64), (i64, i64, i64)), u32> = HashMap::new();
    for tri in &mesh.triangles {
        for i in 0..3 {
            let a = key(tri[i]);
            let b = key(tri[(i + 1) % 3]);
            if a == b {
                return false; // degenerate zero-length edge
            }
            let e = if a <= b { (a, b) } else { (b, a) };
            *edge_count.entry(e).or_insert(0) += 1;
        }
    }
    !edge_count.is_empty() && edge_count.values().all(|&c| c == 2)
}

/// World-space bounding box of everything visible in the document (#108's
/// `bearcad.ui.zoom_fit()`): every non-deleted body's solid mesh, plus every non-deleted
/// line/circle's world-space extent on its sketch plane (curved lines use their sampled
/// polyline; circles use the plane-local bounding square of the perimeter). Construction
/// planes are not included — an empty document returns `None`.
pub(crate) fn document_world_bounds(doc: &Document) -> Option<(Vec3, Vec3)> {
    let mut bounds: Option<(Vec3, Vec3)> = None;
    let mut extend = |p: Vec3| {
        bounds = Some(match bounds {
            Some((min, max)) => (min.min(p), max.max(p)),
            None => (p, p),
        });
    };
    for (i, body) in doc.bodies.iter() {
        // Shadow bodies (#1218) are ghost/faded preview copies consumed by an operation; they
        // are not real geometry, so zoom-to-fit (#1381) must not try to fit them in.
        if body.shadow {
            continue;
        }
        if let Some((min, max)) = body_solid_mesh(doc, i).and_then(|m| m.bounds()) {
            extend(min);
            extend(max);
        }
    }
    // Construction geometry is scaffolding, not "the model" — zoom-to-fit (#164) frames
    // only real geometry.
    for line in doc.lines.values().filter(|l| !l.construction) {
        if let Some(frame) = sketch_geometry_frame(doc, line.sketch) {
            for (u, v) in line.sample_local(crate::model::BEZIER_SEGMENTS) {
                extend(local_to_world(&frame, u, v));
            }
        }
    }
    for circle in doc.circles.values().filter(|c| !c.construction) {
        if let Some(frame) = sketch_geometry_frame(doc, circle.sketch) {
            for (du, dv) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                extend(local_to_world(
                    &frame,
                    circle.cx + du * circle.r,
                    circle.cy + dv * circle.r,
                ));
            }
        }
    }
    bounds
}

/// Build the solid mesh for an extrusion, or `None` if it has no faces or zero distance.
pub fn extrusion_mesh(doc: &Document, extrusion: &Extrusion) -> Option<SolidMesh> {
    let distance = effective_distance(doc, extrusion);
    if extrusion.faces.is_empty() || distance.abs() < 1e-4 {
        return None;
    }
    // First real switch onto the OCCT kernel (#86): a plain single-profile
    // extrusion becomes a genuine BREP prism, tessellated by OCCT. Falls through
    // to the hand-rolled mesher for everything it doesn't yet cover (slanted
    // targets, edge chamfers/fillets, multi-face bodies) so behavior is preserved.
    if let Some(mesh) = occt_extrusion_mesh(doc, extrusion, distance) {
        // OCCT's lofted slanted extrusions can silently come back as an **open shell** — the side
        // wall without its end caps, i.e. a pipe instead of a closed solid (#582). When the kernel
        // mesh isn't watertight, prefer the hand-rolled mesher, which caps both ends, as long as it
        // produces a closed solid; otherwise keep the kernel mesh as the best available.
        if mesh_is_watertight(&mesh) {
            return Some(mesh);
        }
        if let Some(fallback) = extrusion_mesh_tessellated(doc, extrusion, distance) {
            if mesh_is_watertight(&fallback) {
                return Some(fallback);
            }
        }
        return Some(mesh);
    }
    extrusion_mesh_tessellated(doc, extrusion, distance)
}

/// The hand-rolled (non-kernel) mesher for an extrusion — caps, walls, hole-aware regions,
/// polygon-vertex bevels. The kernel path falls back here; the live text preview (#386) uses
/// it directly because it's orders of magnitude faster than per-glyph kernel booleans.
fn extrusion_mesh_tessellated(
    doc: &Document,
    extrusion: &Extrusion,
    distance: f32,
) -> Option<SolidMesh> {
    // #1268: symmetric + taper must keep the sketch-plane size at mid. A single base→top loft
    // with both ends equally offset is a prism of the *tapered* size. Split into two
    // non-symmetric half-extrusions (sketch→+d/2 and sketch→−d/2) and merge the triangles.
    if let Some(mesh) = extrusion_mesh_tessellated_symmetric_taper(doc, extrusion, distance) {
        return Some(mesh);
    }
    let mut mesh = SolidMesh::default();
    for (face_index, face) in extrusion.faces.iter().enumerate() {
        if let Some((profile, top, _normal)) = extrusion_profile_rings(doc, extrusion, face, distance)
        {
            // A face with holes (annulus, #268) has no edge treatments in the fallback path;
            // build it as a hollow region (hole-aware caps + inner walls). Holes follow taper
            // the same way the outer does (#1243).
            let hole_rings = extrusion_hole_rings(doc, extrusion, face, distance);
            if !hole_rings.is_empty() {
                let holes: Vec<Vec<Vec3>> = hole_rings.iter().map(|(b, _)| b.clone()).collect();
                let holes_top: Vec<Vec<Vec3>> = hole_rings.iter().map(|(_, t)| t.clone()).collect();
                extrude_region(&profile, &top, &holes, &holes_top, &mut mesh.triangles);
                continue;
            }
            let treatments: Vec<&EdgeTreatment> = extrusion
                .edge_treatments
                .iter()
                // Circle cap rims (#177) are kernel-only; the hand-rolled bevel builder is
                // polygon-vertex-based, so the fallback renders the rim untreated.
                .filter(|t| {
                    t.edge.face() == face_index
                        && t.amount > 0.0
                        && !is_circle_cap_rim(face, t.edge)
                })
                .collect();
            if treatments.is_empty() {
                extrude_profile(&profile, &top, &mut mesh.triangles);
            } else {
                extrude_profile_with_treatments(&profile, &top, &treatments, &mut mesh.triangles);
            }
        }
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// #1268: hand-rolled mesh for a symmetric tapered extrude — two half-extrusions merged.
/// Returns `None` when this path does not apply (caller uses the normal mesher).
fn extrusion_mesh_tessellated_symmetric_taper(
    doc: &Document,
    extrusion: &Extrusion,
    distance: f32,
) -> Option<SolidMesh> {
    if !(extrusion.symmetric && extrusion.target.is_none() && extrusion.taper.abs() > 1e-12) {
        return None;
    }
    let half = distance.abs() * 0.5;
    if half < 1e-4 {
        return None;
    }
    let sign = if distance < 0.0 { -1.0 } else { 1.0 };
    let mut half_ext = extrusion.clone();
    half_ext.symmetric = false;
    let mut mesh = SolidMesh::default();
    for d in [half * sign, -half * sign] {
        let Some(part) = extrusion_mesh_tessellated(doc, &half_ext, d) else {
            continue;
        };
        mesh.triangles.extend(part.triangles);
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// OCCT BREP solid for the extrusions the kernel currently handles (#86/#77): a
/// single profile face extruded by a pure translation (prism) or to a slanted
/// target (ruled loft), with any 3D edge chamfer/fillet edge treatments applied as
/// *real* `BRepFilletAPI` fillets/chamfers on the built solid (#77). `None` for
/// anything else — a multi-face extrusion, a degenerate profile, or any edge
/// treatment the kernel can't place (see [`edge_ref_world_endpoints`]) — so callers
/// fall back to the hand-rolled mesher and we never ship broken geometry.
fn occt_extrusion_shape(
    doc: &Document,
    extrusion: &Extrusion,
    distance: f32,
) -> Option<crate::kernel::Shape> {
    occt_extrusion_shape_overshoot(doc, extrusion, distance, 0.0)
}

/// [`occt_extrusion_shape`] with an optional `overshoot` (mm) that extends the built solid by
/// that amount past *both* ends along the extrusion direction. Used to build **cut tools**:
/// when a cut's cap lands exactly on a body face (e.g. an extrude-to-face cut that spans the
/// body), a flush boolean leaves a coincident zero-thickness seam face — the wall renders
/// capped even though the material is gone (#200). Overshooting the tool moves both caps
/// clear of the body faces so the walls open cleanly; the extra length is outside the body,
/// so it changes nothing else.
/// BREP solid for a single extrude face, extruded by this extrusion's distance/target (#268).
/// A `Boolean` face is built the *right way*: extrude each operand into its own solid and apply
/// the same boolean to the solids — so a `Difference` of two concentric circles becomes a true
/// **tube** (outer cylinder minus inner cylinder, exact walls and single circular rims), and any
/// annulus/face-with-hole falls out for free. Leaf faces (circle/polygon) build a true cylinder
/// (circle, pure translation) or a prism/ruled loft as before. `overshoot` extends both ends
/// (cut tools) and is threaded to every operand so a cut passes fully through.
fn occt_face_solid(
    doc: &Document,
    extrusion: &Extrusion,
    face: &ExtrudeFace,
    distance: f32,
    overshoot: f32,
) -> Option<crate::kernel::Shape> {
    // #1268: symmetric + taper — mid plane must stay the sketch size. Fuse two
    // non-symmetric half-extrusions (sketch→+d/2 and sketch→−d/2), each tapering from
    // the profile at mid to the free end. A single loft with both ends equally offset
    // would be a prism of the *tapered* size (wrong mid).
    if extrusion.symmetric && extrusion.target.is_none() && extrusion.taper.abs() > 1e-12 {
        let half = distance.abs() * 0.5;
        if half < 1e-4 {
            return None;
        }
        let sign = if distance < 0.0 { -1.0 } else { 1.0 };
        let mut half_ext = extrusion.clone();
        half_ext.symmetric = false;
        let upper = occt_face_solid(doc, &half_ext, face, half * sign, overshoot)?;
        let lower = occt_face_solid(doc, &half_ext, face, -half * sign, overshoot)?;
        return upper.boolean(&lower, crate::kernel::BoolOp::Fuse);
    }
    if let ExtrudeFace::Boolean { op, a, b } = face {
        let sa = occt_face_solid(doc, extrusion, a, distance, overshoot)?;
        let sb = occt_face_solid(doc, extrusion, b, distance, overshoot)?;
        let boolop = match op {
            crate::model::BooleanOp::Difference => crate::kernel::BoolOp::Cut,
            crate::model::BooleanOp::Intersection => crate::kernel::BoolOp::Common,
        };
        return sa.boolean(&sb, boolop);
    }
    let (mut profile, mut top, _normal) =
        extrusion_profile_rings(doc, extrusion, face, distance)?;
    // Extend both ends by `overshoot` along the extrusion direction (cut tools only).
    if overshoot > 1e-6 {
        let u = (top[0] - profile[0]).normalize_or_zero();
        profile = profile.iter().map(|p| *p - u * overshoot).collect();
        top = top.iter().map(|t| *t + u * overshoot).collect();
    }
    // A pure translation is a single prism (simplest/most robust); a slanted target (per-vertex
    // top offset, e.g. extrude-to-an-angled-face) is a ruled loft between the bottom and top loops.
    let dir = top[0] - profile[0];
    let is_translation = profile
        .iter()
        .zip(&top)
        .all(|(p, t)| (*t - *p - dir).length() <= 1e-4);
    // A circle profile extruded by pure translation builds as a *true* cylinder (#177): real
    // cylindrical wall, single circular rim edges — treatable and with exact volume, unlike a
    // prism over the sampled 48-gon. Slanted targets still loft the sampled profile.
    let mut shape = if is_translation && matches!(face, ExtrudeFace::Circle(_)) {
        let center = profile.iter().copied().sum::<Vec3>() / profile.len() as f32;
        let radius = (profile[0] - center).length() as f64;
        let height = dir.length() as f64;
        let axis = dir.normalize_or_zero();
        crate::kernel::Shape::cylinder(center, axis, radius, height)
    } else if is_translation {
        crate::kernel::Shape::prism(&profile, dir)
    } else {
        crate::kernel::Shape::loft(&profile, &top)
    }?;
    // A leaf face with holes (a text glyph's counters, #285): subtract each hole's prism/loft so
    // the glyph extrudes hollow. Boolean faces get their holes via the recursion above instead.
    // Holes follow taper the same way the outer does (#1243).
    for (mut hole, mut htop) in extrusion_hole_rings(doc, extrusion, face, distance) {
        if hole.len() < 3 {
            continue;
        }
        if overshoot > 1e-6 {
            let u = (htop[0] - hole[0]).normalize_or_zero();
            hole = hole.iter().map(|p| *p - u * overshoot).collect();
            htop = htop.iter().map(|t| *t + u * overshoot).collect();
        }
        let hdir = htop[0] - hole[0];
        let is_translation = hole
            .iter()
            .zip(&htop)
            .all(|(p, t)| (*t - *p - hdir).length() <= 1e-4);
        let hole_solid = if is_translation {
            crate::kernel::Shape::prism(&hole, hdir)
        } else {
            crate::kernel::Shape::loft(&hole, &htop)
        };
        let Some(hole_solid) = hole_solid else {
            continue;
        };
        shape = shape.boolean(&hole_solid, crate::kernel::BoolOp::Cut)?;
    }
    Some(shape)
}

fn occt_extrusion_shape_overshoot(
    doc: &Document,
    extrusion: &Extrusion,
    distance: f32,
    overshoot: f32,
) -> Option<crate::kernel::Shape> {
    // One solid per face, fused. A single-face extrusion (the common case) skips the
    // boolean; a multi-face one (several coplanar profiles extruded together) fuses into
    // one solid so it cuts/merges correctly — a multi-face *cut* used to return `None`
    // here, silently dropping every hole of the cut via the mesh fallback.
    let mut fused: Option<crate::kernel::Shape> = None;
    for face in &extrusion.faces {
        let shape = occt_face_solid(doc, extrusion, face, distance, overshoot)?;
        fused = Some(match fused {
            None => shape,
            Some(acc) => acc.boolean(&shape, crate::kernel::BoolOp::Fuse)?,
        });
    }
    let base_shape = fused?;

    // Real BREP edge fillets/chamfers (#77). Split the active treatments into fillet
    // and chamfer groups (each applied in one batched kernel call), matching each
    // edge to the built solid by its analytic world-space endpoints. Any missing edge
    // or kernel error returns `None` -> the whole extrusion falls back to the mesher.
    let mut fillet_edges: Vec<(Vec3, Vec3)> = Vec::new();
    let mut fillet_radii: Vec<f32> = Vec::new();
    let mut chamfer_edges: Vec<(Vec3, Vec3)> = Vec::new();
    let mut chamfer_dists: Vec<f32> = Vec::new();
    for t in &extrusion.edge_treatments {
        if t.amount <= 0.0 {
            continue;
        }
        let endpoints = edge_ref_world_endpoints(doc, extrusion, &t.edge)?;
        match t.kind {
            VertexTreatmentKind::Fillet => {
                fillet_edges.push(endpoints);
                fillet_radii.push(t.amount);
            }
            VertexTreatmentKind::Chamfer => {
                chamfer_edges.push(endpoints);
                chamfer_dists.push(t.amount);
            }
        }
    }
    if fillet_edges.is_empty() && chamfer_edges.is_empty() {
        return Some(base_shape);
    }
    let mut shape = base_shape;
    if !fillet_edges.is_empty() {
        shape = shape.fillet(&fillet_edges, &fillet_radii)?;
    }
    if !chamfer_edges.is_empty() {
        shape = shape.chamfer(&chamfer_edges, &chamfer_dists)?;
    }
    Some(shape)
}

/// World-space endpoints of one analytic extrusion edge (#77), derived from the very
/// same analytic geometry [`treatable_edges`] and the hand-rolled mesh-bevel builder
/// use — so the OCCT edge-matching in [`occt_extrusion_shape`] keys off the identical
/// coordinates the picking/preview code does. A `Vertical` edge runs from a bottom
/// profile vertex to the corresponding top vertex; a `Cap` edge is the boundary
/// between consecutive vertices of the chosen (base/top) ring. `None` if the face is
/// missing/degenerate or the edge index is out of range for its profile loop.
fn edge_ref_world_endpoints(
    doc: &Document,
    extrusion: &Extrusion,
    edge: &ExtrusionEdgeRef,
) -> Option<(Vec3, Vec3)> {
    let face = extrusion.faces.get(edge.face())?;
    let distance = effective_distance(doc, extrusion);
    let (base, top, _normal) = extrusion_profile_rings(doc, extrusion, face, distance)?;
    // A circle cap rim (#177) is one closed edge: request it as two diametrically opposite
    // points on the rim — the kernel matcher's closed-edge pass matches by curve hits.
    if is_circle_cap_rim(face, *edge) {
        let ExtrusionEdgeRef::Cap { top: is_top, .. } = edge else {
            return None;
        };
        let m = base.len();
        if m < 4 {
            return None;
        }
        let ring = if *is_top { &top } else { &base };
        return Some((ring[0], ring[m / 2]));
    }
    let n = base.len();
    if n < 3 {
        return None;
    }
    match *edge {
        ExtrusionEdgeRef::Vertical { edge, .. } => {
            if edge >= n {
                return None;
            }
            let v = (edge + 1) % n;
            Some((base[v], top[v]))
        }
        ExtrusionEdgeRef::Cap { edge, top: is_top, .. } => {
            if edge >= n {
                return None;
            }
            let e2 = (edge + 1) % n;
            if is_top {
                Some((top[edge], top[e2]))
            } else {
                Some((base[edge], base[e2]))
            }
        }
    }
}

/// OCCT-backed mesh for a single extrusion (see [`occt_extrusion_shape`]).
fn occt_extrusion_mesh(doc: &Document, extrusion: &Extrusion, distance: f32) -> Option<SolidMesh> {
    let shape = occt_extrusion_shape(doc, extrusion, distance)?;
    let tris = shape.tessellate(OCCT_DEFLECTION as f64);
    (!tris.is_empty()).then_some(SolidMesh { triangles: tris })
}

/// OCCT solid fusing every kernel-representable extrusion in `indices` into one real unioned
/// shape. `None` if any listed extrusion isn't kernel-representable; the outer `Option`-of-
/// -`Option` collapses to `Some(None)` when the list contributes no geometry at all (all
/// deleted/degenerate).
fn occt_fused_extrusions(
    doc: &Document,
    indices: &[crate::model::ExtrusionKey],
) -> Option<Option<crate::kernel::Shape>> {
    use crate::kernel::BoolOp;
    let mut fused: Option<crate::kernel::Shape> = None;
    for &ei in indices {
        let extrusion = doc.extrusions.get(ei)?;
        let distance = effective_distance(doc, extrusion);
        if extrusion.faces.is_empty() || distance.abs() < 1e-4 {
            continue;
        }
        let shape = occt_extrusion_shape(doc, extrusion, distance)?;
        // Placements this add contributes: the base, plus one per repeat-op replay offset (#220
        // add-replay) — an add extrusion targeted by a repeat op is fused again at each instance,
        // growing N bumps instead of one.
        let mut placements: Vec<glam::Mat4> = vec![glam::Mat4::IDENTITY];
        for op in doc.repeat_ops.values() {
            if !op.extrusion_targets.contains(&ei) {
                continue;
            }
            if let Some(offsets) = repeat_offsets(doc, op) {
                for off in offsets {
                    if let Some(m) = repeat_offset_transform(doc, op, off) {
                        placements.push(m);
                    }
                }
            }
        }
        for m in placements {
            let piece = shape.transformed(&mat4_to_rows_3x4(&m))?;
            fused = Some(match fused.take() {
                None => piece,
                Some(acc) => acc.boolean(&piece, BoolOp::Fuse)?,
            });
        }
    }
    Some(fused)
}

/// OCCT-backed mesh for a whole body whose every extrusion the kernel can
/// represent: the per-extrusion prisms are **fused** into one real unioned solid
/// (#86), then any **cut** extrusions are subtracted from that solid (#35) — so
/// overlapping add-to-body extrusions merge into a single watertight shape and cuts
/// carve real holes, instead of concatenated triangle soup with internal walls.
/// `None` if any add/cut extrusion isn't kernel-representable, so [`body_solid_mesh`]
/// falls back to the hand-rolled per-extrusion concatenation.
fn occt_body_mesh(
    doc: &Document,
    add_indices: &[crate::model::ExtrusionKey],
    cut_indices: &[crate::model::ExtrusionKey],
) -> Option<SolidMesh> {
    let solid = occt_body_shape_from_indices(doc, add_indices, cut_indices)?;
    let tris = solid.tessellate(OCCT_DEFLECTION as f64);
    (!tris.is_empty()).then_some(SolidMesh { triangles: tris })
}

/// Build the fused/cut OCCT solid for the extrusions in `add_indices`/`cut_indices` — the
/// real BREP shape *before* tessellation (see [`occt_body_mesh`]). `None` if any add/cut
/// extrusion isn't kernel-representable, or the adds contribute no geometry at all.
fn occt_body_shape_from_indices(
    doc: &Document,
    add_indices: &[crate::model::ExtrusionKey],
    cut_indices: &[crate::model::ExtrusionKey],
) -> Option<crate::kernel::Shape> {
    let solid = occt_fused_extrusions(doc, add_indices)??;
    occt_subtract_cut_extrusions(doc, solid, cut_indices)
}

/// Fuse a primitive base with the additive extrusions, then subtract the cuts (#1104):
/// the `Solid { base: Some(..), add, cut }` body's solid. The primitive is the starting
/// solid; each additive extrusion is unioned onto it; each cut extrusion is subtracted.
fn occt_solid_with_primitive_base(
    doc: &Document,
    base: crate::model::PrimitiveKey,
    add_indices: &[crate::model::ExtrusionKey],
    cut_indices: &[crate::model::ExtrusionKey],
) -> Option<crate::kernel::Shape> {
    let primitive = doc.primitives.get(base)?;
    let solid = crate::primitives::kernel_shape(doc, primitive)?;
    occt_fuse_then_cut_extrusions(doc, solid, add_indices, cut_indices)
}

/// Fuse each additive extrusion onto `solid`, then subtract the cuts (#1104/#1168).
/// Shared by primitive-base solids and post-shell extrusions on a hollow body.
fn occt_fuse_then_cut_extrusions(
    doc: &Document,
    mut solid: crate::kernel::Shape,
    add_indices: &[crate::model::ExtrusionKey],
    cut_indices: &[crate::model::ExtrusionKey],
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    for &ei in add_indices {
        let extrusion = doc.extrusions.get(ei)?;
        let distance = effective_distance(doc, extrusion);
        if extrusion.faces.is_empty() || distance.abs() < 1e-4 {
            continue;
        }
        let added = occt_extrusion_shape_for_host(doc, extrusion, distance)?;
        solid = solid.boolean(&added, BoolOp::Fuse)?;
    }
    occt_subtract_cut_extrusions(doc, solid, cut_indices)
}

/// Subtract each cut extrusion's solid from `solid` (#35/#726). A cut that isn't
/// kernel-representable aborts to the fallback (returns `None`); a cut contributing no
/// geometry is a no-op. Shared by extrusion-backed bodies and unit-cut bodies.
fn occt_subtract_cut_extrusions(
    doc: &Document,
    mut solid: crate::kernel::Shape,
    cut_indices: &[crate::model::ExtrusionKey],
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    for &ei in cut_indices {
        let extrusion = doc.extrusions.get(ei)?;
        let distance = effective_distance(doc, extrusion);
        if extrusion.faces.is_empty() || distance.abs() < 1e-4 {
            continue;
        }
        // Circle-cap rim treatments on a *cut* extrusion are countersinks (#177): they carve
        // into the resulting body's hole rim, not into the cutting tool (beveling the tool
        // would leave a lip — the inverse). Build the tool without them, subtract, then
        // apply them to the body: the hole's rim edge lies exactly on the tool's rim circle,
        // so the same closed-edge matching finds it.
        let mut tool = extrusion.clone();
        let mut rim_fillets: (Vec<(Vec3, Vec3)>, Vec<f32>) = (Vec::new(), Vec::new());
        let mut rim_chamfers: (Vec<(Vec3, Vec3)>, Vec<f32>) = (Vec::new(), Vec::new());
        tool.edge_treatments.retain(|t| {
            let is_rim = extrusion
                .faces
                .get(t.edge.face())
                .is_some_and(|f| is_circle_cap_rim(f, t.edge));
            if is_rim && t.amount > 0.0 {
                if let Some(endpoints) = edge_ref_world_endpoints(doc, extrusion, &t.edge) {
                    match t.kind {
                        VertexTreatmentKind::Fillet => {
                            rim_fillets.0.push(endpoints);
                            rim_fillets.1.push(t.amount);
                        }
                        VertexTreatmentKind::Chamfer => {
                            rim_chamfers.0.push(endpoints);
                            rim_chamfers.1.push(t.amount);
                        }
                    }
                }
                false
            } else {
                true
            }
        });
        let cut = occt_extrusion_shape_overshoot(doc, &tool, distance, CUT_TOOL_OVERSHOOT)?;
        let cut = match extrusion_host_unpose(doc, extrusion) {
            Some(inv) => cut.transformed(&mat4_to_rows_3x4(&inv))?,
            None => cut,
        };
        solid = solid.boolean(&cut, BoolOp::Cut)?;
        // Repeat-operation replay (#220): any non-deleted repeat op that targets this cut
        // extrusion subtracts the same tool again at each instance offset along its axis —
        // punching N holes rather than copying a solid.
        for op in doc.repeat_ops.values() {
            if !op.extrusion_targets.contains(&ei) {
                continue;
            }
            if let Some(offsets) = repeat_offsets(doc, op) {
                for off in offsets {
                    let Some(m) = repeat_offset_transform(doc, op, off) else {
                        continue;
                    };
                    let moved = cut.transformed(&mat4_to_rows_3x4(&m))?;
                    solid = solid.boolean(&moved, BoolOp::Cut)?;
                }
            }
        }
        if !rim_fillets.0.is_empty() {
            solid = solid.fillet(&rim_fillets.0, &rim_fillets.1)?;
        }
        if !rim_chamfers.0.is_empty() {
            solid = solid.chamfer(&rim_chamfers.0, &rim_chamfers.1)?;
        }
    }
    Some(solid)
}

/// A unit instance's kernel solid (#726): every live body of the rebuilt embedded
/// document built through the kernel, fused, and placed by the instance's transform.
/// `None` when any inner body isn't kernel-representable — callers fall back to the
/// mesh path (where a cut then can't apply and the fallback warning surfaces, like any
/// non-kernel body).
pub fn occt_unit_instance_shape(doc: &Document, instance: crate::model::UnitInstanceKey) -> Option<crate::kernel::Shape> {
    let eval = crate::units::evaluate_instance(doc, instance)?;
    let inner = &eval.document;
    let mut solid: Option<crate::kernel::Shape> = None;
    for bi in inner.bodies.keys().collect::<Vec<_>>() {
                let shape = occt_body_shape(inner, bi)?;
        solid = Some(match solid {
            Some(fused) => fused.boolean(&shape, crate::kernel::BoolOp::Fuse)?,
            None => shape,
        });
    }
    let m = crate::units::instance_transform(doc, instance);
    solid?.transformed(&mat4_to_rows_3x4(&m))
}

/// Re-read a STEP import's BREP from the bytes kept on the mesh (#1029). `None` when the
/// import was triangle-only (STL, faceted parser) or the kernel can't parse the bytes.
fn imported_step_shape(
    doc: &Document,
    mesh: crate::model::ImportedMeshKey,
) -> Option<crate::kernel::Shape> {
    let bytes = doc.imported_meshes.get(mesh)?.step_bytes.as_ref()?;
    if bytes.is_empty() {
        return None;
    }
    #[cfg(target_arch = "wasm32")]
    {
        return crate::kernel::Shape::read_step_bytes(bytes);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Native STEP reader is path-based; stage the bytes somewhere short-lived.
        let path = std::env::temp_dir().join(format!(
            "bearcad-import-shape-{}-{}.step",
            std::process::id(),
            mesh.index()
        ));
        std::fs::write(&path, bytes).ok()?;
        let shape = crate::kernel::Shape::read_step(&path);
        let _ = std::fs::remove_file(&path);
        shape
    }
}

/// The body's real OCCT BREP solid (adds fused, cuts subtracted), *before* tessellation —
/// used by STEP export (#65) to write genuine BREP rather than tessellated triangles. `None`
/// for a deleted/missing body, a pure triangle import (STL / faceted-only STEP), or a body
/// whose geometry isn't fully kernel-representable (the caller then falls back to the mesh
/// path). A STEP import that kept its original bytes (#1029) re-reads them here so booleans
/// and other kernel ops still see a solid.
///
/// Memoized per (document fingerprint, body): a cut preview used to rebuild the whole
/// shell/slice/combine history on every frame (#1337).
pub fn occt_body_shape(doc: &Document, body_index: crate::model::BodyKey) -> Option<crate::kernel::Shape> {
    let fingerprint = document_mesh_fingerprint(doc);
    // `mesh_rev` is the cheap idle-frame key; trial clones keep it and rewrite
    // geometry in place, so a structural extra tag must agree too (#1337).
    let extra = if doc.mesh_rev == 0 {
        0
    } else {
        structural_mesh_fingerprint(doc)
    };
    let cached = BODY_SHAPE_CACHE.with(|cache| {
        let cache = cache.try_borrow().ok()?;
        if cache.0 != fingerprint || cache.1 != extra {
            return None;
        }
        match cache.2.get(&body_index)? {
            // Clone failed → miss, rebuild rather than pretend the body is gone.
            Some(shape) => shape.try_clone().map(Some),
            None => Some(None),
        }
    });
    if let Some(hit) = cached {
        return hit;
    }
    let shape = occt_body_shape_uncached(doc, body_index);
    BODY_SHAPE_CACHE.with(|cache| {
        let Ok(mut cache) = cache.try_borrow_mut() else {
            return;
        };
        if cache.0 != fingerprint || cache.1 != extra {
            cache.0 = fingerprint;
            cache.1 = extra;
            cache.2.clear();
        }
        let stored = match &shape {
            Some(s) => match s.try_clone() {
                Some(clone) => Some(clone),
                None => return,
            },
            None => None,
        };
        cache.2.insert(body_index, stored);
    });
    shape
}

fn occt_body_shape_uncached(
    doc: &Document,
    body_index: crate::model::BodyKey,
) -> Option<crate::kernel::Shape> {
    let body = doc.bodies.get(body_index)?;
    if let Some(mi) = body.source.imported_mesh_key() {
        return imported_step_shape(doc, mi);
    }
    let mut solid = match body.source {
        crate::model::BodySource::Primitive(pi) => {
            crate::primitives::kernel_shape(doc, doc.primitives.get(pi)?)?
        }
        crate::model::BodySource::Revolve(ri) => {
            occt_revolution_shape(doc, doc.revolutions.get(ri)?)?
        }
        crate::model::BodySource::Sweep(fi) => {
            occt_sweep_shape(doc, doc.sweeps.get(fi)?)?
        }
        crate::model::BodySource::Loft(li) => {
            occt_loft_shape(doc, doc.lofts.get(li)?)?
        }
        crate::model::BodySource::Boolean {
            op,
            solid,
            ref add,
            ref cut,
        } => {
            let base = occt_boolean_output_shape(doc, op, solid)?;
            return occt_fuse_then_cut_extrusions(doc, base, add, cut);
        }
        crate::model::BodySource::Moved {
            op,
            target,
            ref add,
            ref cut,
        } => {
            let base = occt_moved_output_shape(doc, op, target)?;
            return occt_fuse_then_cut_extrusions(doc, base, add, cut);
        }
        crate::model::BodySource::Mirrored {
            op,
            target,
            ref add,
            ref cut,
        } => {
            let base = occt_mirrored_output_shape(doc, op, target)?;
            return occt_fuse_then_cut_extrusions(doc, base, add, cut);
        }
        crate::model::BodySource::Repeated {
            op,
            target,
            instance,
            ref add,
            ref cut,
        } => {
            let base = occt_repeated_output_shape(doc, op, target, instance)?;
            return occt_fuse_then_cut_extrusions(doc, base, add, cut);
        }
        crate::model::BodySource::Sliced {
            op,
            target,
            piece,
            ref add,
            ref cut,
        } => {
            let base = occt_sliced_output_shape(doc, op, target, piece)?;
            return occt_fuse_then_cut_extrusions(doc, base, add, cut);
        }
        crate::model::BodySource::Shelled {
            op,
            target,
            ref add,
            ref cut,
        } => {
            // Hollow first, then fuse/cut any post-shell extrusions (#1168).
            let hollow = occt_shelled_output_shape(doc, op, target)?;
            return occt_fuse_then_cut_extrusions(doc, hollow, add, cut);
        }
        crate::model::BodySource::EdgeTreated {
            op,
            target,
            ref add,
            ref cut,
        } => {
            let base = occt_edge_treated_output_shape(doc, op, target)?;
            return occt_fuse_then_cut_extrusions(doc, base, add, cut);
        }
        // A unit instance's fused, placed kernel solid (#726).
        crate::model::BodySource::UnitInstance(instance) => {
            occt_unit_instance_shape(doc, instance)?
        }
        // A unit with extrusions cut out of it in the importing document (#726).
        crate::model::BodySource::UnitCut { instance, ref cut } => {
            let solid = occt_unit_instance_shape(doc, instance)?;
            occt_subtract_cut_extrusions(doc, solid, cut)?
        }
        // A primitive base with extrusions added/cut (#1104): fuse the primitive with the
        // additive extrusions, then subtract the cuts.
        crate::model::BodySource::Solid { base: Some(base), ref add, ref cut } => {
            occt_solid_with_primitive_base(doc, base, add, cut)?
        }
        _ => occt_body_shape_from_indices(
            doc,
            body.source.extrusion_indices(),
            body.source.cut_extrusion_indices(),
        )?,
    };
    // Revolutions that fuse into / cut this body (#revolve).
    for (ri, is_cut) in revolutions_targeting(doc, body_index) {
        let rev = &doc.revolutions[ri];
        let shape = occt_revolution_shape(doc, rev)?;
        let op = if is_cut {
            crate::kernel::BoolOp::Cut
        } else {
            crate::kernel::BoolOp::Fuse
        };
        solid = solid.boolean(&shape, op)?;
    }
    // Sweeps that fuse into / cut this body (#sweep).
    for (fi, is_cut) in sweeps_targeting(doc, body_index) {
        let fp = &doc.sweeps[fi];
        let shape = occt_sweep_shape(doc, fp)?;
        let op = if is_cut {
            crate::kernel::BoolOp::Cut
        } else {
            crate::kernel::BoolOp::Fuse
        };
        solid = solid.boolean(&shape, op)?;
    }
    // Lofts that fuse into / cut this body (#479).
    for (li, is_cut) in lofts_targeting(doc, body_index) {
        let loft = &doc.lofts[li];
        let shape = occt_loft_shape(doc, loft)?;
        let op = if is_cut {
            crate::kernel::BoolOp::Cut
        } else {
            crate::kernel::BoolOp::Fuse
        };
        solid = solid.boolean(&shape, op)?;
    }
    Some(solid)
}

/// World-space rigid transform of one move operation (Move tool): rotation about the
/// op's axis (through its world origin) then translation. Expressions evaluate against
/// document parameters, so moves rebuild parametrically. `None` when the axis line died
/// or an expression doesn't evaluate.
/// The world position of a [`crate::model::MovePointRef`] (#649/#650), resolved against the
/// body's live mesh. `None` once the mesh no longer has that corner/edge; the world origin
/// (#946) always resolves.
pub fn move_point_world(doc: &Document, point: &crate::model::MovePointRef) -> Option<Vec3> {
    match point {
        crate::model::MovePointRef::Vertex { body, p } => {
            crate::parameters::body_vertex_world_position(doc, *body, *p)
        }
        crate::model::MovePointRef::EdgeMidpoint { body, a, b } => {
            let (p0, p1) = crate::parameters::body_edge_world_segment(doc, *body, *a, *b)?;
            Some((p0 + p1) * 0.5)
        }
        // A point along an edge (#670) is its own position; it only needs its body alive.
        crate::model::MovePointRef::OnEdge { body, p } => {
            doc.bodies.get(*body)?;
            Some(crate::hierarchy::dequantize_body_point(*p))
        }
        // A point on a face (#738/#1074): re-find the coplanar group by its key, then step
        // out from its live centre along the live face's own in-plane axes — so a rebuild
        // that moves or resizes the face takes the point with it. Uncached mesher for the
        // same borrow reason as the vertex arm.
        crate::model::MovePointRef::OnFace { body, centroid, normal, uv } => {
            doc.bodies.get(*body)?;
            let solid = body_solid_mesh_uncached_pub(doc, *body)?;
            let tris = face_group_matching(&solid, *centroid, *normal)?;
            Some(face_uv_world(&tris, *uv))
        }
        // The world origin (#946) is fixed and always resolves — no body to outlive.
        crate::model::MovePointRef::Origin => Some(Vec3::ZERO),
    }
}

/// A move's translation vector (#648/#650): in `Snap` mode the offset that lands the source
/// start point A on end point A, otherwise the `tx`/`ty`/`tz` expressions. A snap with either
/// point missing or unresolvable contributes no translation, so the op stays valid while the
/// user is still picking.
pub fn move_op_translation(doc: &Document, op: &crate::model::MoveOperation) -> Option<Vec3> {
    if op.has_snap_translation() {
        let (source, target) = (op.start_point_a.as_ref()?, op.end_point_a.as_ref()?);
        // Points that no longer resolve contribute nothing rather than killing the op — the
        // same forgiveness a repeat's dead length target gets.
        if let (Some(from), Some(to)) = (move_point_world(doc, source), move_point_world(doc, target))
        {
            return Some(to - from);
        }
        return Some(Vec3::ZERO);
    }
    let eval_len = |expr: &str| -> Option<f32> {
        if expr.trim().is_empty() {
            return Some(0.0);
        }
        crate::value::eval_length_mm_in_doc(expr, doc)
    };
    Some(Vec3::new(
        eval_len(&op.tx)?,
        eval_len(&op.ty)?,
        eval_len(&op.tz)?,
    ))
}

/// The radius end point B is confined to (#669): the distance from start A to start B. The
/// rotation about end point A can only swing start B around a sphere of that radius, so any
/// end B off it is unreachable. `None` until both start points are picked and resolve.
pub fn snap_rotation_radius(
    doc: &Document,
    start_a: Option<&crate::model::MovePointRef>,
    start_b: Option<&crate::model::MovePointRef>,
) -> Option<f32> {
    let a = move_point_world(doc, start_a?)?;
    let b = move_point_world(doc, start_b?)?;
    Some((b - a).length())
}

/// Whether a candidate end point B is reachable (#669): it must sit on the constraint sphere
/// centred on end point A with [`snap_rotation_radius`], within a tolerance that forgives the
/// 0.01 mm quantisation the picked points carry.
pub fn snap_rotation_reachable(
    doc: &Document,
    cm_start_a: Option<&crate::model::MovePointRef>,
    cm_start_b: Option<&crate::model::MovePointRef>,
    end_a: Option<&crate::model::MovePointRef>,
    candidate: Vec3,
) -> bool {
    let (Some(radius), Some(pivot)) = (
        snap_rotation_radius(doc, cm_start_a, cm_start_b),
        end_a.and_then(|p| move_point_world(doc, p)),
    ) else {
        return false;
    };
    ((candidate - pivot).length() - radius).abs() <= SNAP_ROTATION_TOLERANCE_MM
}

/// How far off the constraint sphere an end-point-B pick may sit and still count (#669).
pub const SNAP_ROTATION_TOLERANCE_MM: f32 = 0.05;

/// Every point where a body's feature edges cross the end-point-B constraint sphere (#670):
/// the reachable landing spots for start point B, offered as candidates while that picker is
/// armed. Bodies being moved are skipped — start B has to land on something that stays put.
///
/// Each edge is a segment; the crossings are the roots of `|p(t) - centre|² = r²` for
/// `p(t) = a + t(b - a)`, `t ∈ [0, 1]`.
pub fn snap_rotation_candidates(
    doc: &Document,
    moving: &[crate::model::BodyKey],
    centre: Vec3,
    radius: f32,
) -> Vec<(crate::model::BodyKey, Vec3)> {
    let mut out: Vec<(crate::model::BodyKey, Vec3)> = Vec::new();
    if !(radius.is_finite() && radius > 1e-4) {
        return out;
    }
    for (bi, body) in doc.bodies.iter() {
        if body.shadow || moving.contains(&bi) {
            continue;
        }
        let Some(solid) = body_solid_mesh(doc, bi) else { continue };
        for (a, b) in crate::gpu_viewport::solid_mesh_unique_edges(&solid) {
            let d = b - a;
            let f = a - centre;
            let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - radius * radius);
            if qa < 1e-12 {
                continue;
            }
            let disc = qb * qb - 4.0 * qa * qc;
            if disc < 0.0 {
                continue;
            }
            let root = disc.sqrt();
            for t in [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)] {
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let p = a + d * t;
                // Two edges meeting at a crossing would offer the same spot twice.
                if !out.iter().any(|(_, q)| (*q - p).length() < 1e-3) {
                    out.push((bi, p));
                }
            }
        }
    }
    out
}

/// Below this angle snap the End-point-B candidate dots would be a cloud — a sphere carries
/// roughly `(180/step) × (360/step)` of them, so even 15° is hundreds — and the tool shows the
/// constraint **sphere** itself instead, reading the angle off the cursor (#920/#950).
pub const ANGLE_SNAP_SPHERE_DEG: f32 = 30.0;
/// The same threshold for End point C (#920), which rides a **circle**: one ring of
/// `360/step` dots stays readable far finer than a sphereful does.
pub const ANGLE_SNAP_CIRCLE_DEG: f32 = 5.0;

/// Where a ray meets the End-point-B constraint sphere (#920): the near hit if the ray
/// crosses it, otherwise the point on the sphere nearest the ray, so the pick still lands
/// somewhere sensible when the cursor slips off the silhouette.
pub fn ray_sphere_point(origin: Vec3, dir: Vec3, centre: Vec3, radius: f32) -> Option<Vec3> {
    if !(radius.is_finite() && radius > 1e-4) {
        return None;
    }
    let dir = dir.normalize_or_zero();
    if dir.length_squared() < 0.5 {
        return None;
    }
    let f = origin - centre;
    let b = 2.0 * f.dot(dir);
    let c = f.dot(f) - radius * radius;
    let disc = b * b - 4.0 * c;
    if disc >= 0.0 {
        let root = disc.sqrt();
        for t in [(-b - root) / 2.0, (-b + root) / 2.0] {
            if t > 0.0 {
                return Some(origin + dir * t);
            }
        }
    }
    // A miss: take the closest approach and push it out onto the sphere.
    let t = (-f).dot(dir).max(0.0);
    let closest = origin + dir * t;
    let out = (closest - centre).normalize_or_zero();
    (out.length_squared() > 0.5).then(|| centre + out * radius)
}

/// Snap a direction to the angle grid (#920): its azimuth and elevation each rounded to the
/// nearest multiple of `step_deg`. A step of zero leaves the direction alone.
pub fn snap_direction_to_angle(dir: Vec3, step_deg: f32) -> Vec3 {
    let dir = dir.normalize_or_zero();
    if dir.length_squared() < 0.5 || !(step_deg > 0.01) {
        return dir;
    }
    let step = step_deg.to_radians();
    let round = |a: f32| (a / step).round() * step;
    let elevation = round(dir.z.clamp(-1.0, 1.0).asin());
    let flat = Vec3::new(dir.x, dir.y, 0.0);
    let azimuth = if flat.length() > 1e-4 {
        round(dir.y.atan2(dir.x))
    } else {
        0.0
    };
    let (sin_e, cos_e) = elevation.sin_cos();
    let (sin_a, cos_a) = azimuth.sin_cos();
    Vec3::new(cos_e * cos_a, cos_e * sin_a, sin_e)
}

/// The sweeps that reach a hovered End-point-B candidate (#919): two arcs about the pivot —
/// the **azimuth** turned in the ground plane from +X, and the **elevation** lifted out of it
/// — each returned as a polyline with the angle it stands for, in degrees. A candidate
/// straight up or down has no azimuth of its own, so only the elevation comes back.
pub fn move_direction_sweeps(pivot: Vec3, target: Vec3) -> Vec<(Vec<Vec3>, f32)> {
    let offset = target - pivot;
    let radius = offset.length();
    if radius < 1e-4 {
        return Vec::new();
    }
    let dir = offset / radius;
    let flat = Vec3::new(dir.x, dir.y, 0.0);
    let mut out = Vec::new();
    // Azimuth: +X round to the direction's bearing, drawn in the ground plane.
    if flat.length() > 1e-3 {
        let azimuth = dir.y.atan2(dir.x);
        out.push((
            arc_points(pivot, Vec3::X, Vec3::Z, radius * 0.55, azimuth, 32),
            azimuth.to_degrees(),
        ));
        // Elevation: the flattened bearing up (or down) to the direction itself.
        let bearing = flat.normalize();
        let elevation = dir.z.clamp(-1.0, 1.0).asin();
        if elevation.abs() > 1e-3 {
            let axis = bearing.cross(Vec3::Z).normalize_or_zero();
            out.push((
                arc_points(pivot, bearing, -axis, radius * 0.75, elevation, 32),
                elevation.to_degrees(),
            ));
        }
    } else {
        // Straight up or down: one arc from +X to the pole.
        let elevation = if dir.z > 0.0 {
            std::f32::consts::FRAC_PI_2
        } else {
            -std::f32::consts::FRAC_PI_2
        };
        out.push((
            arc_points(pivot, Vec3::X, -Vec3::Y, radius * 0.75, elevation, 32),
            elevation.to_degrees(),
        ));
    }
    out
}

/// A polyline arc: `steps + 1` points from `origin + from * radius`, swept `angle` radians
/// about `axis` through `origin`.
fn arc_points(origin: Vec3, from: Vec3, axis: Vec3, radius: f32, angle: f32, steps: usize) -> Vec<Vec3> {
    let from = from.normalize_or_zero();
    let axis = axis.normalize_or_zero();
    if from.length_squared() < 0.5 || axis.length_squared() < 0.5 {
        return Vec::new();
    }
    (0..=steps)
        .map(|i| {
            let t = angle * i as f32 / steps as f32;
            origin + glam::Quat::from_axis_angle(axis, t) * (from * radius)
        })
        .collect()
}

impl SpinCircle {
    /// The sweep from the no-spin position round to `target` (#919): the arc and its angle
    /// in degrees, signed about the circle's axis.
    pub fn sweep_to(&self, target: Vec3) -> Option<(Vec<Vec3>, f32)> {
        let v = target - self.center;
        let flat = v - self.axis * v.dot(self.axis);
        if flat.length() < 1e-4 {
            return None;
        }
        let dir = flat.normalize();
        let cross = self.reference.cross(dir).dot(self.axis);
        let angle = cross.atan2(self.reference.dot(dir));
        Some((
            arc_points(self.center, self.reference, self.axis, self.radius * 0.8, angle, 48),
            angle.to_degrees(),
        ))
    }
}

/// Rotation candidates by **angle** (#918): directions on the end-point-B constraint sphere
/// every `step_deg` degrees about the world axes, as world points at `radius` from `centre`.
///
/// 90° gives the six axis directions; 45° gives 26 (two poles plus three rings of eight).
/// A step of zero (or one that doesn't divide the sphere sensibly) yields nothing — the
/// caller then falls back to the geometry-derived spots alone.
pub fn snap_angle_sphere_candidates(centre: Vec3, radius: f32, step_deg: f32) -> Vec<Vec3> {
    let mut out = Vec::new();
    if !(radius.is_finite() && radius > 1e-4) || !(step_deg > 0.5) {
        return out;
    }
    let step = step_deg.to_radians();
    let polar_steps = (std::f32::consts::PI / step).round().max(1.0) as i32;
    let azimuth_steps = (std::f32::consts::TAU / step).round().max(1.0) as i32;
    for i in 0..=polar_steps {
        let phi = std::f32::consts::PI * i as f32 / polar_steps as f32;
        let (sin_phi, cos_phi) = phi.sin_cos();
        // The poles are one point each, whatever the azimuth.
        let ring = if sin_phi.abs() < 1e-4 { 1 } else { azimuth_steps };
        for j in 0..ring {
            let theta = std::f32::consts::TAU * j as f32 / azimuth_steps as f32;
            let (sin_t, cos_t) = theta.sin_cos();
            let dir = Vec3::new(sin_phi * cos_t, sin_phi * sin_t, cos_phi);
            let p = centre + dir * radius;
            if !out.iter().any(|q: &Vec3| (*q - p).length() < 1e-3) {
                out.push(p);
            }
        }
    }
    out
}

/// The same idea on the end-point-C circle (#918): every `step_deg` around it, starting at
/// `reference` (the no-extra-spin direction).
pub fn snap_angle_circle_candidates(
    centre: Vec3,
    axis: Vec3,
    reference: Vec3,
    radius: f32,
    step_deg: f32,
) -> Vec<Vec3> {
    let mut out = Vec::new();
    if !(radius.is_finite() && radius > 1e-4) || !(step_deg > 0.5) {
        return out;
    }
    let steps = (360.0 / step_deg).round().max(1.0) as i32;
    for i in 0..steps {
        let angle = std::f32::consts::TAU * i as f32 / steps as f32;
        let p = centre + glam::Quat::from_axis_angle(axis, angle) * (reference * radius);
        if !out.iter().any(|q: &Vec3| (*q - p).length() < 1e-3) {
            out.push(p);
        }
    }
    out
}

/// The circle end point C rides (#914/#918): its centre on the `end A → end B` axis, the
/// axis itself, the no-extra-spin direction, and the radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpinCircle {
    pub center: Vec3,
    pub axis: Vec3,
    /// Unit direction of the zero-spin position, perpendicular to `axis`.
    pub reference: Vec3,
    pub radius: f32,
}

impl SpinCircle {
    /// The spots on it, `step_deg` apart (#918); the first is the zero-spin position.
    pub fn spots(&self, step_deg: f32) -> Vec<Vec3> {
        snap_angle_circle_candidates(
            self.center,
            self.axis,
            self.reference,
            self.radius,
            step_deg,
        )
    }
}

/// Where end point C can land (#914): with end points A and B fixed, the part can still
/// spin about the A→B axis, so start point C sweeps a **circle** — not a sphere. This
/// returns four spots a quarter turn apart on that circle, together with the circle's
/// centre (which the viewport draws a guide to each spot from).
///
/// The first spot is the **no extra spin** position: the start-side geometry carried over
/// by the minimal rotation that takes the start axis onto the end axis. The others follow
/// at 90°, 180° and 270° about the end axis.
pub fn snap_spin_candidates(
    doc: &Document,
    start_a: Option<&crate::model::MovePointRef>,
    start_b: Option<&crate::model::MovePointRef>,
    start_c: Option<&crate::model::MovePointRef>,
    end_a: Option<&crate::model::MovePointRef>,
    end_b: Option<&crate::model::MovePointRef>,
) -> Option<SpinCircle> {
    let sa = move_point_world(doc, start_a?)?;
    let sb = move_point_world(doc, start_b?)?;
    let sc = move_point_world(doc, start_c?)?;
    let ea = move_point_world(doc, end_a?)?;
    let eb = move_point_world(doc, end_b?)?;
    let start_dir = (sb - sa).normalize_or_zero();
    let end_dir = (eb - ea).normalize_or_zero();
    if start_dir.length_squared() < 0.5 || end_dir.length_squared() < 0.5 {
        return None;
    }
    // Split start C into its along-axis and perpendicular parts about the start axis.
    let v = sc - sa;
    let axial = v.dot(start_dir);
    let perp = v - start_dir * axial;
    let radius = perp.length();
    if radius < 1e-4 {
        // C sits on the axis: spinning moves it nowhere, so there's nothing to offer.
        return None;
    }
    let center = ea + end_dir * axial;
    // The zero-spin reference: the perpendicular carried over by the minimal rotation
    // between the two axes.
    let carry = glam::Quat::from_rotation_arc(start_dir, end_dir);
    let reference = (carry * perp).normalize_or_zero();
    let reference = if reference.length_squared() < 0.5 {
        end_dir.any_orthonormal_vector()
    } else {
        // Re-orthogonalize against float drift so all four sit exactly on the circle.
        (reference - end_dir * reference.dot(end_dir)).normalize_or_zero()
    };
    Some(SpinCircle {
        center,
        axis: end_dir,
        reference,
        radius,
    })
}

/// Reachable landing spots in **mid-air** (#745): every stationary body's feature edge
/// whose line passes through end point A (the sphere's centre), extended straight out to
/// where it crosses the constraint sphere — so start point B can land along an edge's
/// direction even beyond the edge's own extent. The viewport draws a dashed guide from
/// the pivot to each of these.
pub fn snap_rotation_axis_candidates(
    doc: &Document,
    moving: &[crate::model::BodyKey],
    centre: Vec3,
    radius: f32,
) -> Vec<(crate::model::BodyKey, Vec3)> {
    let mut out: Vec<(crate::model::BodyKey, Vec3)> = Vec::new();
    if !(radius.is_finite() && radius > 1e-4) {
        return out;
    }
    for (bi, body) in doc.bodies.iter() {
        if body.shadow || moving.contains(&bi) {
            continue;
        }
        let Some(solid) = body_solid_mesh(doc, bi) else { continue };
        for (a, b) in crate::gpu_viewport::solid_mesh_unique_edges(&solid) {
            let dir = (b - a).normalize_or_zero();
            if dir.length_squared() < 0.5 {
                continue;
            }
            // The edge's line must pass through the pivot (within the pick quantisation).
            let off = a - centre;
            if (off - dir * off.dot(dir)).length() > SNAP_ROTATION_TOLERANCE_MM {
                continue;
            }
            for p in [centre + dir * radius, centre - dir * radius] {
                if !out.iter().any(|(_, q)| (*q - p).length() < 1e-3) {
                    out.push((bi, p));
                }
            }
        }
    }
    out
}

/// The rotation the optional B pair asks for (#669): after the A translation has landed start
/// point A on end point A, turn the bodies **about end point A** so that the moved start point
/// B points at end point B.
///
/// The turn is the shortest one taking the direction `endA → movedStartB` to
/// `endA → endB` — a single rotation about their common perpendicular. Only the *direction*
/// matters: end point B is constrained to the sphere of radius `|startA - startB|` about end
/// point A (#669), so a valid pick already sits at the right distance and the rotation alone
/// lands start B on it.
pub fn move_snap_rotation(
    doc: &Document,
    op: &crate::model::MoveOperation,
) -> Option<glam::Mat3> {
    let (axis, angle) = move_snap_rotation_axis_angle(doc, op)?;
    if angle.abs() < 1e-9 {
        return Some(glam::Mat3::IDENTITY);
    }
    Some(glam::Mat3::from_axis_angle(axis, angle))
}

/// The unit axis and angle behind [`move_snap_rotation`], exposed so the preview can sweep
/// the **arc** the rotation drags start B along — the actual road the point travels about
/// end point A. An aligned pair reports a zero angle (with an arbitrary valid axis).
pub fn move_snap_rotation_axis_angle(
    doc: &Document,
    op: &crate::model::MoveOperation,
) -> Option<(Vec3, f32)> {
    if !op.has_snap_rotation() {
        return None;
    }
    let translation = move_op_translation(doc, op)?;
    let pivot = move_point_world(doc, op.end_point_a.as_ref()?)?;
    // Start B rides along with the translation before it turns.
    let moved_start_b = move_point_world(doc, op.start_point_b.as_ref()?)? + translation;
    let target_b = move_point_world(doc, op.end_point_b.as_ref()?)?;
    let from = (moved_start_b - pivot).normalize_or_zero();
    let to = (target_b - pivot).normalize_or_zero();
    if from.length_squared() < 0.5 || to.length_squared() < 0.5 {
        return None;
    }
    let dot = from.dot(to).clamp(-1.0, 1.0);
    // Already aligned: no turn. Exactly opposed: any perpendicular axis is a half turn, so
    // pick a stable one rather than leaving the cross product degenerate.
    if dot > 1.0 - 1e-9 {
        return Some((from.any_orthonormal_vector(), 0.0));
    }
    let axis = if dot < -1.0 + 1e-9 {
        from.any_orthonormal_vector()
    } else {
        from.cross(to).normalize_or_zero()
    };
    if axis.length_squared() < 0.5 {
        return Some((from.any_orthonormal_vector(), 0.0));
    }
    Some((axis, dot.acos()))
}

/// The axis the bodies would turn about if `target` were taken as end point B (#949) — the
/// same rotation [`move_snap_rotation_axis_angle`] derives, but for a candidate spot rather
/// than a picked end B, so each candidate dot can be coloured by the axis it turns about.
/// `None` when the pair can't be resolved or the turn is degenerate (no rotation at all).
pub fn snap_rotation_axis_toward(
    doc: &Document,
    start_a: Option<&crate::model::MovePointRef>,
    start_b: Option<&crate::model::MovePointRef>,
    end_a: Option<&crate::model::MovePointRef>,
    target: Vec3,
) -> Option<Vec3> {
    let sa = move_point_world(doc, start_a?)?;
    let ea = move_point_world(doc, end_a?)?;
    // Start B rides along with the A pair's translation before it turns.
    let moved_start_b = move_point_world(doc, start_b?)? + (ea - sa);
    let from = (moved_start_b - ea).normalize_or_zero();
    let to = (target - ea).normalize_or_zero();
    if from.length_squared() < 0.5 || to.length_squared() < 0.5 {
        return None;
    }
    let axis = from.cross(to).normalize_or_zero();
    (axis.length_squared() > 0.5).then_some(axis)
}

/// The spin the optional C pair asks for: B lines the bodies up along `endA → endB` but
/// leaves them free to turn about that line, and C is what pins it. The angle is the one
/// about that axis that brings the already-translated, already-rotated start point C as near
/// end point C as the axis allows.
///
/// Only C's *direction about the axis* is used — the component along the axis, and the
/// distance out from it, are B's and A's to decide, and a pick can't change them. So any end
/// point C gives a well-defined answer; there is no reachable/unreachable to refuse, unlike
/// end point B's constraint sphere.
pub fn move_snap_roll_axis_angle(
    doc: &Document,
    op: &crate::model::MoveOperation,
) -> Option<(Vec3, f32)> {
    if !op.has_snap_roll() && !op.has_snap_roll_angle() {
        return None;
    }
    let translation = move_op_translation(doc, op)?;
    let pivot = move_point_world(doc, op.end_point_a.as_ref()?)?;
    let target_b = move_point_world(doc, op.end_point_b.as_ref()?)?;
    let axis = (target_b - pivot).normalize_or_zero();
    if axis.length_squared() < 0.5 {
        return None;
    }
    // The third pair set as an angle (#1078): the spin is simply that many degrees about the
    // axis, with no third point to measure from — which is the whole point of offering it.
    if op.has_snap_roll_angle() {
        return Some((axis, crate::value::eval_angle_rad_in_doc(&op.roll_angle, doc)?));
    }
    // Start C rides the translation and then B's turn before it spins.
    let rot_b = move_snap_rotation(doc, op)?;
    let start_c = move_point_world(doc, op.start_point_c.as_ref()?)? + translation;
    let moved_start_c = pivot + rot_b * (start_c - pivot);
    let target_c = move_point_world(doc, op.end_point_c.as_ref()?)?;
    // Only what's perpendicular to the axis can turn; flatten both onto that plane.
    let flatten = |v: Vec3| v - axis * v.dot(axis);
    let from = flatten(moved_start_c - pivot).normalize_or_zero();
    let to = flatten(target_c - pivot).normalize_or_zero();
    // A point on the axis itself has no direction about it to line up — no spin to derive.
    if from.length_squared() < 0.5 || to.length_squared() < 0.5 {
        return None;
    }
    // Signed about the axis, so the spin turns the short way round in the right direction.
    let angle = from.cross(to).dot(axis).atan2(from.dot(to));
    Some((axis, angle))
}

/// The rotation matrix behind [`move_snap_roll_axis_angle`].
pub fn move_snap_roll(doc: &Document, op: &crate::model::MoveOperation) -> Option<glam::Mat3> {
    let (axis, angle) = move_snap_roll_axis_angle(doc, op)?;
    if angle.abs() < 1e-9 {
        return Some(glam::Mat3::IDENTITY);
    }
    Some(glam::Mat3::from_axis_angle(axis, angle))
}

pub fn move_op_transform(doc: &Document, op: &crate::model::MoveOperation) -> Option<glam::Mat4> {
    // In place (#1076) is the identity by definition — the part is already where it belongs.
    if op.translate_mode == crate::model::MoveTranslateMode::InPlace {
        return Some(glam::Mat4::IDENTITY);
    }
    let translation = glam::Mat4::from_translation(move_op_translation(doc, op)?);
    // Free mode's typed turns (#1076) spin the part about its own centre, before the
    // translation carries it away — typing "45° about Z" means "turn it where it stands",
    // not "swing it around the world origin".
    if op.translate_mode == crate::model::MoveTranslateMode::Free {
        let Some(free) = move_op_free_rotation(doc, op) else {
            return Some(translation);
        };
        let pivot = move_targets_center(doc, op)?;
        return Some(
            translation
                * glam::Mat4::from_translation(pivot)
                * glam::Mat4::from_mat3(free)
                * glam::Mat4::from_translation(-pivot),
        );
    }
    // Face Snap (#1077) turns the moving face to meet the target face, then spins it about
    // the target's normal, both about the mate point.
    if op.translate_mode == crate::model::MoveTranslateMode::FaceSnap {
        let Some(rot) = move_face_snap_rotation(doc, op) else {
            return Some(translation);
        };
        let pivot = move_point_world(doc, op.end_point_a.as_ref()?)?;
        // A gap held along the target face's normal (#1079), after the turn — so it is
        // clearance off the surface, whichever way round the part ended up.
        let gap = if op.face_offset.trim().is_empty() {
            glam::Mat4::IDENTITY
        } else {
            let n = move_point_face_normal(doc, op.end_point_a.as_ref()?)?;
            glam::Mat4::from_translation(n * crate::value::eval_length_mm_in_doc(&op.face_offset, doc)?)
        };
        return Some(
            gap * glam::Mat4::from_translation(pivot)
                * glam::Mat4::from_mat3(rot)
                * glam::Mat4::from_translation(-pivot)
                * translation,
        );
    }
    // The B pair adds a rotation about end point A, applied after the translation (#669).
    let Some(rot) = move_snap_rotation(doc, op) else {
        return Some(translation);
    };
    let pivot = move_point_world(doc, op.end_point_a.as_ref()?)?;
    // The C pair spins about the endA → endB axis B left free, applied after B's turn.
    let roll = move_snap_roll(doc, op).unwrap_or(glam::Mat3::IDENTITY);
    Some(
        glam::Mat4::from_translation(pivot)
            * glam::Mat4::from_mat3(roll * rot)
            * glam::Mat4::from_translation(-pivot)
            * translation,
    )
}

/// The outward world normal of the face a [`crate::model::MovePointRef::OnFace`] sits on
/// (#1077). `None` for any other kind of point — Face Snap needs faces on both sides.
pub fn move_point_face_normal(
    doc: &Document,
    point: &crate::model::MovePointRef,
) -> Option<Vec3> {
    let crate::model::MovePointRef::OnFace { body, centroid, normal, .. } = point else {
        return None;
    };
    // The **un-posed** mesh, like `move_point_world`'s own face arm: a joint resolves its
    // placement from inside the pose pass, so reaching for the posed mesh here would be a
    // re-entrant borrow — and would resolve the key against geometry the joint itself moved.
    let solid = body_solid_mesh_uncached_pub(doc, *body)?;
    let tris = face_group_matching(&solid, *centroid, *normal)?;
    let (u, v) = face_group_basis(&tris);
    Some(u.cross(v).normalize_or_zero())
}

/// Face Snap's rotation (#1077): turn the moving face's normal to meet the target face's,
/// then spin about the target's normal by `face_spin`.
///
/// The default opposes the two normals, so the surfaces touch — outsides of objects go
/// together, which is what "put this face on that face" nearly always means. `face_flip`
/// points them the same way instead, putting the part behind the face.
///
/// `None` until both points are picked, both resolve, and both sit on faces.
pub fn move_face_snap_rotation(
    doc: &Document,
    op: &crate::model::MoveOperation,
) -> Option<glam::Mat3> {
    if op.translate_mode != crate::model::MoveTranslateMode::FaceSnap {
        return None;
    }
    let from = move_point_face_normal(doc, op.start_point_a.as_ref()?)?;
    let to = move_point_face_normal(doc, op.end_point_a.as_ref()?)?;
    if from.length_squared() < 0.5 || to.length_squared() < 0.5 {
        return None;
    }
    let want = if op.face_flip { to } else { -to };
    let align = glam::Mat3::from_quat(glam::Quat::from_rotation_arc(from, want));
    let spin = if op.face_spin.trim().is_empty() {
        0.0
    } else {
        crate::value::eval_angle_rad_in_doc(&op.face_spin, doc)?
    };
    if spin.abs() < 1e-9 {
        return Some(align);
    }
    Some(glam::Mat3::from_axis_angle(to, spin) * align)
}

/// Free mode's typed turns as one rotation (#1076): X, then Y, then Z, about the world axes.
/// `None` when nothing is typed (so the caller can skip the pivot work) or an expression
/// doesn't evaluate.
fn move_op_free_rotation(
    doc: &Document,
    op: &crate::model::MoveOperation,
) -> Option<glam::Mat3> {
    let eval = |expr: &str| -> Option<f32> {
        if expr.trim().is_empty() {
            return Some(0.0);
        }
        crate::value::eval_angle_rad_in_doc(expr, doc)
    };
    let (x, y, z) = (eval(&op.rx)?, eval(&op.ry)?, eval(&op.rz)?);
    if x.abs() < 1e-9 && y.abs() < 1e-9 && z.abs() < 1e-9 {
        return None;
    }
    Some(
        glam::Mat3::from_quat(
            glam::Quat::from_rotation_z(z)
                * glam::Quat::from_rotation_y(y)
                * glam::Quat::from_rotation_x(x),
        ),
    )
}

/// Free mode's typed turns as one unit quaternion (#1414): X, then Y, then Z about the world
/// axes — the same rotation [`move_op_free_rotation`] builds, exposed so the rotation gizmos
/// can rotate their base references along with the preview. `None` when a turn expression
/// doesn't evaluate (an all-zero turn still yields the identity).
pub fn move_op_free_rotation_quat(
    doc: &Document,
    rx: &str,
    ry: &str,
    rz: &str,
) -> Option<glam::Quat> {
    let eval = |expr: &str| -> Option<f32> {
        if expr.trim().is_empty() {
            return Some(0.0);
        }
        crate::value::eval_angle_rad_in_doc(expr, doc)
    };
    let (x, y, z) = (eval(rx)?, eval(ry)?, eval(rz)?);
    Some(
        glam::Quat::from_rotation_z(z)
            * glam::Quat::from_rotation_y(y)
            * glam::Quat::from_rotation_x(x),
    )
}

/// The centre of what a move operation moves (#1076) — the pivot Free mode's typed turns act
/// about. `None` when nothing it moves has world extent.
fn move_targets_center(doc: &Document, op: &crate::model::MoveOperation) -> Option<Vec3> {
    free_move_targets_bounds(doc, &op.targets, &op.plane_targets).map(|(lo, hi)| (lo + hi) * 0.5)
}

/// Axis-aligned bounds of Free-move targets (#1233): body solid meshes plus plane origins.
/// `None` when nothing contributes world extent.
pub fn free_move_targets_bounds(
    doc: &Document,
    bodies: &[crate::model::BodyKey],
    planes: &[crate::model::ConstructionPlaneKey],
) -> Option<(Vec3, Vec3)> {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    let mut any = false;
    for &body in bodies {
        // Un-posed, for the same reason `move_point_face_normal` is.
        if let Some((a, b)) = body_solid_mesh_uncached_pub(doc, body).and_then(|m| m.bounds()) {
            lo = lo.min(a);
            hi = hi.max(b);
            any = true;
        }
    }
    for &plane in planes {
        if let Some(p) = doc.construction_planes.get(plane) {
            lo = lo.min(p.origin);
            hi = hi.max(p.origin);
            any = true;
        }
    }
    any.then_some((lo, hi))
}

/// One Free-mode translation handle (#1233): sits on a face of the selection's tight AABB.
///
/// `axis` is 0/1/2 for world X/Y/Z (the component it drives). `origin` is the face centre;
/// `outward` is the face normal pointing out of the cuboid. Dragging projects onto the
/// positive world axis for that component so both faces of a pair update the same way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeMoveTranslationHandle {
    pub axis: usize,
    pub origin: Vec3,
    pub outward: Vec3,
}

/// Translation handles on all six faces of the tight AABB around the Free-move selection
/// (#1233). Order: +X, −X, +Y, −Y, +Z, −Z.
pub fn free_move_translation_handles(min: Vec3, max: Vec3) -> [FreeMoveTranslationHandle; 6] {
    let c = (min + max) * 0.5;
    [
        FreeMoveTranslationHandle {
            axis: 0,
            origin: Vec3::new(max.x, c.y, c.z),
            outward: Vec3::X,
        },
        FreeMoveTranslationHandle {
            axis: 0,
            origin: Vec3::new(min.x, c.y, c.z),
            outward: -Vec3::X,
        },
        FreeMoveTranslationHandle {
            axis: 1,
            origin: Vec3::new(c.x, max.y, c.z),
            outward: Vec3::Y,
        },
        FreeMoveTranslationHandle {
            axis: 1,
            origin: Vec3::new(c.x, min.y, c.z),
            outward: -Vec3::Y,
        },
        FreeMoveTranslationHandle {
            axis: 2,
            origin: Vec3::new(c.x, c.y, max.z),
            outward: Vec3::Z,
        },
        FreeMoveTranslationHandle {
            axis: 2,
            origin: Vec3::new(c.x, c.y, min.z),
            outward: -Vec3::Z,
        },
    ]
}

/// Free-mode rotation rings (#1234): centre (AABB centre) and a radius that sits just outside
/// the tight bounding cuboid so the three axis rings clear the selection.
pub fn free_move_rotation_ring(min: Vec3, max: Vec3) -> (Vec3, f32) {
    let c = (min + max) * 0.5;
    let half = (max - min) * 0.5;
    // Half-diagonal keeps the ring outside every corner; floor so a degenerate box still
    // offers a grab.
    let radius = half.length().max(5.0) * 1.1;
    (c, radius)
}

/// World-axis unit vector for Free-move axis index 0/1/2.
pub fn free_move_axis_dir(axis: usize) -> Vec3 {
    match axis {
        0 => Vec3::X,
        1 => Vec3::Y,
        _ => Vec3::Z,
    }
}

/// Deterministic base reference each Free-move rotation ring's handle floats on (#1413).
/// The three rings' handle starting positions spread around the body so they never overlap:
/// the X ring (turning about X, its ring spanning the YZ plane) starts on +Y, the Y ring on
/// +Z, and the Z ring on +X.
pub fn free_move_rotation_base_dir(axis: usize) -> Vec3 {
    match axis {
        0 => Vec3::Y,
        1 => Vec3::Z,
        _ => Vec3::X,
    }
}

/// The three Free-move rotation-gizmo handle world positions (#1413/#1414): one per ring at a
/// deterministic, non-overlapping base reference (see [`free_move_rotation_base_dir`]) rotated
/// by the live composed Free turn, so every handle follows the moving body's preview when any
/// single ring is rotated. `(min, max)` are the targets' resting bounds, `translation` the live
/// Free translation; `None` when a typed turn doesn't evaluate.
pub fn free_move_rotation_handles(
    doc: &Document,
    min: Vec3,
    max: Vec3,
    translation: Vec3,
    rx: &str,
    ry: &str,
    rz: &str,
) -> Option<[Vec3; 3]> {
    let (center, radius) = free_move_rotation_ring(min + translation, max + translation);
    let q = move_op_free_rotation_quat(doc, rx, ry, rz)?;
    let mut out = [Vec3::ZERO; 3];
    for axis in 0..3 {
        out[axis] = center + q * free_move_rotation_base_dir(axis) * radius;
    }
    Some(out)
}

/// Gizmo script names for Free-move translation axes.
pub fn free_move_translation_gizmo_name(axis: usize) -> &'static str {
    match axis {
        0 => "move_x",
        1 => "move_y",
        _ => "move_z",
    }
}

/// Gizmo script names for Free-move rotation axes (#1234).
pub fn free_move_rotation_gizmo_name(axis: usize) -> &'static str {
    match axis {
        0 => "move_rx",
        1 => "move_ry",
        _ => "move_rz",
    }
}

/// Resolve a rotation/revolve axis to world origin + unit direction.
pub fn axis_world(doc: &Document, axis: crate::model::RevolveAxis) -> Option<(Vec3, Vec3)> {
    match axis {
        crate::model::RevolveAxis::Line(li) => {
            let line = doc.lines.get(li)?;
            if !crate::document_lifecycle::line_alive(doc, li) {
                return None;
            }
            let (a, b) = crate::face::line_world_endpoints(doc, line)?;
            let dir = (b - a).normalize_or_zero();
            (dir.length_squared() > 1e-8).then_some((a, dir))
        }
        // A body edge (#643) keeps its world endpoints; it resolves as long as the body it
        // was picked on is still around.
        crate::model::RevolveAxis::BodyEdge { body, a, b } => {
            let alive = doc.bodies.contains(body);
            if !alive {
                return None;
            }
            let dir = (b - a).normalize_or_zero();
            (dir.length_squared() > 1e-8).then_some((a, dir))
        }
        crate::model::RevolveAxis::X => Some((Vec3::ZERO, Vec3::X)),
        crate::model::RevolveAxis::Y => Some((Vec3::ZERO, Vec3::Y)),
        crate::model::RevolveAxis::Z => Some((Vec3::ZERO, Vec3::Z)),
    }
}

/// Row-major 3x4 (rotation + translation) of a glam column-major Mat4, the layout both
/// OCCT's `gp_Trsf::SetValues` and the kernel transform entry point take.
fn mat4_to_rows_3x4(m: &glam::Mat4) -> [f64; 12] {
    let c = m.to_cols_array_2d();
    [
        c[0][0] as f64, c[1][0] as f64, c[2][0] as f64, c[3][0] as f64,
        c[0][1] as f64, c[1][1] as f64, c[2][1] as f64, c[3][1] as f64,
        c[0][2] as f64, c[1][2] as f64, c[2][2] as f64, c[3][2] as f64,
    ]
}

/// The BREP solid of one move-operation output: the input body's shape, transformed.
fn occt_moved_output_shape(
    doc: &Document,
    op_index: crate::model::MoveOpKey,
    target: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.move_ops.get(op_index)?;
    let &input = op.targets.get(target)?;
    if op.outputs.contains(&input) {
        return None;
    }
    let shape = occt_body_shape(doc, input)?;
    let m = move_op_transform(doc, op)?;
    shape.transformed(&mat4_to_rows_3x4(&m))
}

/// Follow the edge-treatment chain from `body` to its live tip (#1323): each op that consumes
/// the current body yields its matching output, until nothing further treats it. Sequential
/// fillets on the same solid stack this way instead of forking sibling bodies.
pub fn live_edge_treated_body(
    doc: &Document,
    body: crate::model::BodyKey,
) -> crate::model::BodyKey {
    let mut current = body;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current) {
        let next = doc.edge_treatment_ops.values().find_map(|op| {
            op.targets
                .iter()
                .position(|&t| t == current)
                .and_then(|i| op.outputs.get(i).copied())
        });
        match next {
            Some(out) if out != current => current = out,
            _ => break,
        }
    }
    current
}

/// The live body that currently carries `extrusion` after any chain of edge-treatment ops
/// (#1323). Falls back to the extrusion's own body when nothing has treated it yet.
pub fn live_body_for_treated_extrusion(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
) -> Option<crate::model::BodyKey> {
    Some(live_edge_treated_body(
        doc,
        crate::model::body_index_for_extrusion(doc, extrusion)?,
    ))
}

/// The live body that currently carries `solid` after any chain of edge-treatment ops
/// (#1323/#1329).
pub fn live_body_for_treatable_solid(
    doc: &Document,
    solid: TreatableSolid,
) -> Option<crate::model::BodyKey> {
    match solid {
        TreatableSolid::Extrusion(e) => live_body_for_treated_extrusion(doc, e),
        TreatableSolid::Primitive(p) => Some(live_edge_treated_body(
            doc,
            crate::model::body_index_for_primitive(doc, p)?,
        )),
    }
}

/// Treatments already applied along the EdgeTreated chain that produced `body`, oldest first,
/// restricted to `extrusion` (#1322). A second-fillet preview splices these onto the ghost so
/// it is based on the already-filleted solid, not the original sharp box.
pub fn edge_treatments_leading_to(
    doc: &Document,
    body: crate::model::BodyKey,
    extrusion: crate::model::ExtrusionKey,
) -> Vec<EdgeTreatment> {
    let mut out = Vec::new();
    let mut current = body;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current) {
        let Some(b) = doc.bodies.get(current) else {
            break;
        };
        let crate::model::BodySource::EdgeTreated { op, target, .. } = b.source else {
            break;
        };
        let Some(operation) = doc.edge_treatment_ops.get(op) else {
            break;
        };
        for te in operation
            .edges
            .iter()
            .filter(|e| e.target == target && e.solid == TreatableSolid::Extrusion(extrusion))
        {
            out.push(EdgeTreatment {
                edge: te.edge,
                kind: operation.kind,
                amount: operation.amount,
            });
        }
        let Some(&input) = operation.targets.get(target) else {
            break;
        };
        current = input;
    }
    out.reverse();
    out
}

/// True when `body` is an EdgeTreated output whose input chain traces back to `extrusion`
/// (#1322). The live fillet preview hides these descendants so only the ghost (original +
/// every committed treatment + the in-progress one) is drawn.
pub fn body_is_edge_treated_from_extrusion(
    doc: &Document,
    body: crate::model::BodyKey,
    extrusion: crate::model::ExtrusionKey,
) -> bool {
    let mut current = body;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current) {
        let Some(b) = doc.bodies.get(current) else {
            return false;
        };
        match b.source {
            crate::model::BodySource::EdgeTreated { op, target, .. } => {
                let Some(operation) = doc.edge_treatment_ops.get(op) else {
                    return false;
                };
                let Some(&input) = operation.targets.get(target) else {
                    return false;
                };
                current = input;
            }
            _ => return b.source.owns_extrusion(extrusion),
        }
    }
    false
}

/// A clone of `doc` with the edge-treatment op's treatments spliced onto the target input
/// body's extrusions, together with that input body's index (#531). Building or meshing the
/// input body in this clone then reuses the whole extrusion chamfer/fillet machinery — so an
/// edge-treatment op's output is exactly its input body, beveled. `None` when the op or target
/// is gone, or an output body was fed back as its own input.
fn edge_treated_input_doc(
    doc: &Document,
    op_index: crate::model::EdgeTreatmentOpKey,
    target: usize,
) -> Option<(Document, crate::model::BodyKey)> {
    let op = doc.edge_treatment_ops.get(op_index)?;
    let &input = op.targets.get(target)?;
    if op.outputs.contains(&input) {
        return None;
    }
    let mut clone = doc.clone();
    for te in op.edges.iter().filter(|e| e.target == target) {
        if let TreatableSolid::Extrusion(ei) = te.solid {
            if let Some(ext) = clone.extrusions.get_mut(ei) {
                ext.edge_treatments.push(crate::model::EdgeTreatment {
                    edge: te.edge,
                    kind: op.kind,
                    amount: op.amount,
                });
            }
        }
    }
    Some((clone, input))
}

/// The BREP solid of one edge-treatment output (#531): the input body's shape built with the
/// op's chamfer/fillet edges spliced onto its extrusions, then any primitive-hosted edges
/// applied as a kernel post-process (#1329).
fn occt_edge_treated_output_shape(
    doc: &Document,
    op_index: crate::model::EdgeTreatmentOpKey,
    target: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.edge_treatment_ops.get(op_index)?;
    let (clone, input) = edge_treated_input_doc(doc, op_index, target)?;
    let mut shape = occt_body_shape(&clone, input)?;
    let mut fillet_edges: Vec<(Vec3, Vec3)> = Vec::new();
    let mut fillet_radii: Vec<f32> = Vec::new();
    let mut chamfer_edges: Vec<(Vec3, Vec3)> = Vec::new();
    let mut chamfer_dists: Vec<f32> = Vec::new();
    for te in op.edges.iter().filter(|e| e.target == target) {
        let TreatableSolid::Primitive(pi) = te.solid else {
            continue;
        };
        if op.amount <= 0.0 {
            continue;
        }
        let Some(endpoints) = primitive_edge_kernel_endpoints(doc, pi, te.edge) else {
            return None;
        };
        match op.kind {
            VertexTreatmentKind::Fillet => {
                fillet_edges.push(endpoints);
                fillet_radii.push(op.amount);
            }
            VertexTreatmentKind::Chamfer => {
                chamfer_edges.push(endpoints);
                chamfer_dists.push(op.amount);
            }
        }
    }
    if !fillet_edges.is_empty() {
        shape = shape.fillet(&fillet_edges, &fillet_radii)?;
    }
    if !chamfer_edges.is_empty() {
        shape = shape.chamfer(&chamfer_edges, &chamfer_dists)?;
    }
    Some(shape)
}

/// World-space reflection (a `Mat4` with determinant −1) across a mirror operation's plane
/// (Mirror tool, #523). `None` when the plane face died or isn't planar. The reflection of a
/// point `x` across the plane through `o` with unit normal `n` is `x - 2((x-o)·n) n`.
pub fn mirror_op_transform(doc: &Document, op: &crate::model::MirrorOperation) -> Option<glam::Mat4> {
    let frame = crate::face::sketch_frame(doc, op.plane.clone())?;
    let n = frame.normal.normalize_or_zero();
    if n.length_squared() < 1e-8 {
        return None;
    }
    let o = frame.origin;
    // Householder reflection: R = I - 2 n nᵀ (columns are n·n[j]).
    let r = glam::Mat3::IDENTITY - 2.0 * glam::Mat3::from_cols(n * n.x, n * n.y, n * n.z);
    // Affine reflection about the plane through `o`: x' = R(x - o) + o = R x + (o - R o).
    Some(glam::Mat4::from_translation(o - r * o) * glam::Mat4::from_mat3(r))
}

/// The BREP solid of one mirror-operation output: the input body's shape, reflected across
/// the op's plane. In the default `NewBody` mode that reflection *is* the output and the input
/// body is kept, so — unlike Move — the output never shadows its source. `Join`/`Cut` (#639)
/// instead fuse or subtract the reflection against the source, and the source is shadowed.
fn occt_mirrored_output_shape(
    doc: &Document,
    op_index: crate::model::MirrorOpKey,
    target: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.mirror_ops.get(op_index)?;
    let &input = op.targets.get(target)?;
    if op.outputs.contains(&input) {
        return None;
    }
    let shape = occt_body_shape(doc, input)?;
    let m = mirror_op_transform(doc, op)?;
    let reflected = shape.transformed(&mat4_to_rows_3x4(&m))?;
    match op.mode {
        crate::model::MirrorMode::NewBody => Some(reflected),
        crate::model::MirrorMode::Join => shape.boolean(&reflected, crate::kernel::BoolOp::Fuse),
        crate::model::MirrorMode::Cut => shape.boolean(&reflected, crate::kernel::BoolOp::Cut),
    }
}

/// The axis-aligned offsets (mm along the axis direction) of a repeat's instances 1..N-1
/// — instance 0 is the original at offset 0. `None` when an expression doesn't evaluate,
/// the axis died, or the configuration is degenerate. Instance counts are clamped to a
/// sane ceiling so a bad expression can't wedge the app.
/// Upper bound on how many instances any linear repeat (3D body #182 or 2D in-sketch #222)
/// will generate, guarding against a runaway fill length / tiny pitch.
pub const MAX_REPEAT_INSTANCES: usize = 512;

/// The along-direction offsets of a linear repeat's extra instances (instance 1..n-1; instance 0
/// is the original at offset 0), given the spacing `mode`, the operands' `extent` along the
/// direction, and the already-evaluated `count` / `gap` / `length` inputs each mode needs
/// (`None` when the relevant expression didn't evaluate). This is the pure spacing-mode math
/// shared by the 3D body repeat ([`repeat_offsets`]) and the 2D in-sketch repeat (#222); it has
/// no notion of what is being repeated. Returns `None` when the configuration can't produce a
/// valid step, and an empty `Vec` for count-fit modes with `count < 2` (just the original).
pub fn spacing_offsets(
    mode: crate::model::RepeatMode,
    extent: f32,
    count: Option<usize>,
    gap: Option<f32>,
    length: Option<f32>,
) -> Option<Vec<f32>> {
    use crate::model::RepeatMode;
    let offsets = |n: usize, step: f32| -> Option<Vec<f32>> {
        (n >= 1 && step.is_finite() && step > 1e-6).then(|| (1..n).map(|i| step * i as f32).collect())
    };
    match mode {
        RepeatMode::CountGap => {
            let n = count?;
            let gap = gap?;
            offsets(n, extent + gap)
        }
        RepeatMode::CountFitEnds => {
            let n = count?;
            if n < 2 {
                return Some(Vec::new());
            }
            let total = length?;
            offsets(n, (total - extent) / (n as f32 - 1.0))
        }
        RepeatMode::CountFitCenters => {
            let n = count?;
            if n < 2 {
                return Some(Vec::new());
            }
            let span = length?;
            offsets(n, span / (n as f32 - 1.0))
        }
        RepeatMode::FillGap => {
            let l = length?;
            let gap = gap?;
            let step = extent + gap;
            if step <= 1e-6 {
                return None;
            }
            let n = (((l - extent) / step).floor() as isize + 1).max(1) as usize;
            offsets(n.min(MAX_REPEAT_INSTANCES), step)
        }
        RepeatMode::FillPitch => {
            let l = length?;
            let pitch = gap?;
            if pitch <= 1e-6 {
                return None;
            }
            let n = (((l - extent) / pitch).floor() as isize + 1).max(1) as usize;
            offsets(n.min(MAX_REPEAT_INSTANCES), pitch)
        }
        RepeatMode::FillMaxPitch => {
            // Stud spacing: last instance lands exactly at the end of L, pitch <= D.
            let l = length?;
            let max_pitch = gap?;
            if max_pitch <= 1e-6 {
                return None;
            }
            let span = (l - extent).max(0.0);
            if span <= 1e-6 {
                return Some(Vec::new());
            }
            let n = ((span / max_pitch).ceil() as usize + 1).min(MAX_REPEAT_INSTANCES);
            offsets(n, span / (n as f32 - 1.0))
        }
        RepeatMode::CountPitch => {
            // N instances at start-to-start pitch `gap` (#257).
            let n = count?;
            offsets(n, gap?)
        }
        RepeatMode::FillGapSpan => {
            // Fill a start-to-start span `length` with clear gap `gap` (step = extent + gap).
            let span = length?;
            let step = extent + gap?;
            if step <= 1e-6 {
                return None;
            }
            let n = ((span / step).floor() as isize + 1).max(1) as usize;
            offsets(n.min(MAX_REPEAT_INSTANCES), step)
        }
        RepeatMode::FillPitchSpan => {
            // Fill a start-to-start span `length` at pitch `gap`.
            let span = length?;
            let pitch = gap?;
            if pitch <= 1e-6 {
                return None;
            }
            let n = ((span / pitch).floor() as isize + 1).max(1) as usize;
            offsets(n.min(MAX_REPEAT_INSTANCES), pitch)
        }
    }
}

/// The plane-local along-direction offsets of a 2D in-sketch repeat's copies (#222), i.e. the
/// same `spacing_offsets` result but with the operands' extent measured in sketch `(u, v)` space:
/// each targeted line endpoint and circle rim is projected onto the (normalized) repeat direction.
/// Returns `None` if the direction is degenerate, nothing is targeted, or the config doesn't
/// evaluate.
/// How far the repeated entities themselves reach along the repeat direction (mm) — the `L`
/// the gap/distance maths measures from, and what the pane's computed readout needs (#835).
pub fn sketch_repeat_extent(
    doc: &Document,
    op: &crate::model::SketchRepeatOperation,
) -> Option<f32> {
    let len = (op.dir_u * op.dir_u + op.dir_v * op.dir_v).sqrt();
    if len <= 1e-6 {
        return None;
    }
    let (du, dv) = (op.dir_u / len, op.dir_v / len);
    if op.line_targets.is_empty() && op.circle_targets.is_empty() {
        return None;
    }
    let mut min_p = f32::INFINITY;
    let mut max_p = f32::NEG_INFINITY;
    let mut extend = |p: f32| {
        min_p = min_p.min(p);
        max_p = max_p.max(p);
    };
    for &li in &op.line_targets {
        let l = doc.lines.get(li)?;
        extend(l.x0 * du + l.y0 * dv);
        extend(l.x1 * du + l.y1 * dv);
    }
    for &ci in &op.circle_targets {
        let c = doc.circles.get(ci)?;
        let center = c.cx * du + c.cy * dv;
        extend(center - c.r);
        extend(center + c.r);
    }
    if !min_p.is_finite() || !max_p.is_finite() {
        return None;
    }
    Some((max_p - min_p).max(0.0))
}

pub fn sketch_repeat_offsets(
    doc: &Document,
    op: &crate::model::SketchRepeatOperation,
) -> Option<Vec<f32>> {
    let extent = sketch_repeat_extent(doc, op)?;
    let eval = |expr: &str| -> Option<f32> {
        (!expr.trim().is_empty())
            .then(|| crate::value::eval_length_mm_in_doc(expr, doc))
            .flatten()
    };
    let count = || -> Option<usize> {
        let n = crate::value::eval_count_in_doc(&op.count, doc)?;
        (n >= 1.0).then_some((n.round() as usize).min(MAX_REPEAT_INSTANCES))
    };
    spacing_offsets(op.mode, extent, count(), eval(&op.spacing), eval(&op.length))
}

/// Every body strictly **downstream** of `seeds` (#260): bodies produced by an operation that
/// consumes a seed body, transitively. Used to fade the descendants of an operation being edited.
pub fn descendant_bodies(doc: &Document, seeds: &[crate::model::BodyKey]) -> std::collections::HashSet<crate::model::BodyKey> {
    use std::collections::{HashSet, VecDeque};
    let mut result = HashSet::new();
    let mut queue: VecDeque<crate::model::BodyKey> = seeds.iter().copied().collect();
    let mut visited: HashSet<crate::model::BodyKey> = seeds.iter().copied().collect();
    while let Some(bi) = queue.pop_front() {
        let mut outs: Vec<crate::model::BodyKey> = Vec::new();
        for op in doc.boolean_ops.values() {
            if op.a.contains(&bi) || op.b.contains(&bi) {
                outs.extend(op.outputs.iter().copied());
            }
        }
        for op in doc.move_ops.values() {
            if op.targets.contains(&bi) {
                outs.extend(op.outputs.iter().copied());
            }
        }
        for op in doc.repeat_ops.values() {
            if op.targets.contains(&bi) {
                outs.extend(op.outputs.iter().copied());
            }
        }
        for op in doc.slice_ops.values() {
            if op.targets.contains(&bi) {
                outs.extend(op.outputs.iter().copied());
            }
        }
        for out in outs {
            if visited.insert(out) {
                result.insert(out);
                queue.push_back(out);
            }
        }
    }
    result
}

/// The repeat targets' combined **extent** along the axis (the item length `L`) — used by the
/// count/gap/distance UI (#257) to convert between a clear gap and a start-to-start pitch, and
/// to derive the computed variable's value. Point-like targets (planes/sketches) have extent 0.
/// Where a repeat's **distance gizmo** hangs (#644): the point on the targets' start plane
/// (their minimum along the axis) at their centroid in the other two directions, plus the
/// axis's unit direction. Distances are measured from that plane, so the handle sits exactly
/// at `anchor + dir * distance`. `None` without a resolvable axis or any meshed target.
pub fn repeat_gizmo_anchor(
    doc: &Document,
    targets: &[crate::model::BodyKey],
    axis: crate::model::RevolveAxis,
) -> Option<(Vec3, Vec3)> {
    let (_, dir) = axis_world(doc, axis)?;
    let mut sum = Vec3::ZERO;
    let mut n = 0u32;
    let mut min_p = f32::INFINITY;
    for &bi in targets {
        let mesh = body_solid_mesh(doc, bi)?;
        for p in mesh.triangles.iter().flatten() {
            sum += *p;
            n += 1;
            min_p = min_p.min(p.dot(dir));
        }
    }
    if n == 0 || !min_p.is_finite() {
        return None;
    }
    let centroid = sum / n as f32;
    Some((centroid - dir * (centroid.dot(dir) - min_p), dir))
}

pub fn repeat_extent(doc: &Document, op: &crate::model::RepeatOperation) -> Option<f32> {
    let (_, dir) = axis_world(doc, op.axis)?;
    let mut min_p = f32::INFINITY;
    let mut max_p = f32::NEG_INFINITY;
    for &bi in &op.targets {
        let mesh = body_solid_mesh_uncached(doc, bi)?;
        for tri in &mesh.triangles {
            for p in tri {
                let d = p.dot(dir);
                min_p = min_p.min(d);
                max_p = max_p.max(d);
            }
        }
    }
    if !min_p.is_finite() || !max_p.is_finite() {
        return Some(0.0);
    }
    Some((max_p - min_p).max(0.0))
}

/// The world polyline of a **curved** repeat path (#840): a bezier sketch line sampled along
/// its length. `None` for anything straight — those repeat along their direction as before.
pub fn repeat_path_polyline(doc: &Document, axis: crate::model::RevolveAxis) -> Option<Vec<Vec3>> {
    repeat_path_polyline_of(doc, axis, None)
}

/// The world polyline of a repeat's path (#840): the picked **circle**'s circumference when
/// there is one, else a curved line's samples. `None` for a straight path, which repeats
/// along its direction instead.
pub fn repeat_path_polyline_of(
    doc: &Document,
    axis: crate::model::RevolveAxis,
    path_circle: Option<crate::model::CircleKey>,
) -> Option<Vec<Vec3>> {
    if let Some(ci) = path_circle {
        let circle = doc.circles.get(ci)?;
        let frame = crate::face::sketch_geometry_frame(doc, circle.sketch)?;
        // Closed: the last point repeats the first, so a pattern can run the whole way round.
        const N: usize = 96;
        let points: Vec<Vec3> = (0..=N)
            .map(|i| {
                let t = i as f32 / N as f32 * std::f32::consts::TAU;
                crate::face::local_to_world(
                    &frame,
                    circle.cx + circle.r * t.cos(),
                    circle.cy + circle.r * t.sin(),
                )
            })
            .collect();
        return (circle.r > 1e-6).then_some(points);
    }
    let crate::model::RevolveAxis::Line(li) = axis else {
        return None;
    };
    let line = doc.lines.get(li).filter(|_| crate::document_lifecycle::line_alive(doc, li))?;
    if !line.is_curved() {
        return None;
    }
    let points = crate::face::line_world_polyline(doc, line)?;
    (points.len() >= 2).then_some(points)
}

/// The point `distance` along a polyline from its start, walking segment by segment. Past the
/// end it keeps going along the last segment's direction, so a pattern longer than its path
/// runs off the end in a straight line rather than piling up at the tip.
fn point_along_polyline(points: &[Vec3], distance: f32) -> Option<Vec3> {
    let mut left = distance;
    for pair in points.windows(2) {
        let seg = pair[1] - pair[0];
        let len = seg.length();
        if len <= 1e-9 {
            continue;
        }
        if left <= len {
            return Some(pair[0] + seg / len * left);
        }
        left -= len;
    }
    let last = points.last()?;
    let seg = *last - *points.get(points.len().checked_sub(2)?)?;
    Some(*last + seg.normalize_or_zero() * left)
}

/// The world transform one repeat instance applies to its source (#839): a slide along the
/// axis, or — when the op repeats **around** the path — a turn about it. `instance` counts
/// from 1; instance 0 is the original.
pub fn repeat_instance_transform(
    doc: &Document,
    op: &crate::model::RepeatOperation,
    instance: usize,
) -> Option<glam::Mat4> {
    let step = *repeat_offsets(doc, op)?.get(instance.checked_sub(1)?)?;
    repeat_offset_transform(doc, op, step)
}

/// The transform for one step of `op` — `step` millimetres along a straight path, degrees
/// about it when turning (#839), or arc length along a curved one (#840).
pub fn repeat_offset_transform(
    doc: &Document,
    op: &crate::model::RepeatOperation,
    step: f32,
) -> Option<glam::Mat4> {
    // A curved path carries the copies along it: each one is offset by the vector from the
    // path's start to the point that far along it, so the pattern follows the bend. Flipped
    // (#989), it is followed from the other end — reversing the polyline rather than stepping
    // backwards off the start, so the copies stay on the path.
    if let Some(mut points) = repeat_path_polyline_of(doc, op.axis, op.path_circle) {
        if op.flip {
            points.reverse();
        }
        let start = *points.first()?;
        return Some(glam::Mat4::from_translation(
            point_along_polyline(&points, step)? - start,
        ));
    }
    let (origin, dir) = axis_world(doc, op.axis)?;
    // Negating the direction reverses a slide, and — turning about it — the sense of the turn
    // (#989). This is the one place a step becomes a transform, so it is the only place the
    // flip has to be applied: every preview, ghost and output goes through here.
    let dir = if op.flip { -dir } else { dir };
    Some(repeat_step_transform(origin, dir, op.around_axis, step))
}

/// One repeat step as a transform: `step` millimetres along `dir`, or `step` **degrees**
/// about the axis through `origin` when `around` (#839).
pub fn repeat_step_transform(origin: Vec3, dir: Vec3, around: bool, step: f32) -> glam::Mat4 {
    if around {
        glam::Mat4::from_translation(origin)
            * glam::Mat4::from_axis_angle(dir.normalize_or_zero(), step.to_radians())
            * glam::Mat4::from_translation(-origin)
    } else {
        glam::Mat4::from_translation(dir * step)
    }
}

/// The angles (degrees) of a rotational repeat's copies (#839): the same count/gap/span maths
/// the linear one uses, with the items treated as points on the circle.
fn repeat_angles(doc: &Document, op: &crate::model::RepeatOperation) -> Option<Vec<f32>> {
    let angle = |expr: &str| -> Option<f32> {
        (!expr.trim().is_empty())
            .then(|| crate::value::eval_angle_rad_in_doc(expr, doc).map(f32::to_degrees))
            .flatten()
    };
    let count = repeat_count(doc, op);
    spacing_offsets(op.mode, 0.0, count, angle(&op.spacing), angle(&op.length))
}

/// The op's instance count expression, evaluated and clamped.
fn repeat_count(doc: &Document, op: &crate::model::RepeatOperation) -> Option<usize> {
    let n = crate::value::eval_count_in_doc(&op.count, doc)?;
    (n >= 1.0).then_some((n.round() as usize).min(MAX_REPEAT_INSTANCES))
}

pub fn repeat_offsets(doc: &Document, op: &crate::model::RepeatOperation) -> Option<Vec<f32>> {
    // Turning about the axis measures in degrees, and the items have no angular extent of
    // their own to space around (#839). A curved path is only ever followed, never turned
    // about, so it can't be in this mode (#840).
    if op.around_axis && repeat_path_polyline_of(doc, op.axis, op.path_circle).is_none() {
        return repeat_angles(doc, op);
    }
    // Along a curved path (#840) the copies step by arc length; there's no single direction
    // to measure the items' own extent along, so they space centre-to-centre like planes do.
    if repeat_path_polyline_of(doc, op.axis, op.path_circle).is_some() {
        let eval = |expr: &str| -> Option<f32> {
            (!expr.trim().is_empty())
                .then(|| crate::value::eval_length_mm_in_doc(expr, doc))
                .flatten()
        };
        return spacing_offsets(
            op.mode,
            0.0,
            repeat_count(doc, op),
            eval(&op.spacing),
            eval(&op.length),
        );
    }
    let (_, dir) = axis_world(doc, op.axis)?;
    // The targets' combined extent along the axis (end-to-start measurements need it).
    let mut min_p = f32::INFINITY;
    let mut max_p = f32::NEG_INFINITY;
    for &bi in &op.targets {
        // Uncached: this runs inside the mesh cache's borrow when a repeat output's own
        // mesh is being built.
        let mesh = body_solid_mesh_uncached(doc, bi)?;
        for tri in &mesh.triangles {
            for p in tri {
                let d = p.dot(dir);
                min_p = min_p.min(d);
                max_p = max_p.max(d);
            }
        }
    }
    if !min_p.is_finite() || !max_p.is_finite() {
        // No body extent. Plane targets (#221), replayed cut extrusions (#220), and repeated
        // sketches (#226) have no along-axis extent of their own — treat as a point pattern
        // spaced purely by the gap/pitch (instances step center-to-center).
        if op.plane_targets.is_empty()
            && op.extrusion_targets.is_empty()
            && op.sketch_targets.is_empty()
        {
            return None;
        }
        min_p = 0.0;
        max_p = 0.0;
    }
    let extent = (max_p - min_p).max(0.0);
    let eval = |expr: &str| -> Option<f32> {
        if expr.trim().is_empty() {
            return None;
        }
        crate::value::eval_length_mm_in_doc(expr, doc)
    };
    // Fill length `L`: a face/plane target derives it from the along-axis distance to that
    // target's extended plane (so it follows the face, #186), overriding the expression.
    let length = || -> Option<f32> {
        if let Some(target) = &op.length_target {
            // Measure from the pattern's start (instance 0's near end) along the axis.
            let start = dir * min_p;
            if let Some(d) = target_distance(doc, start, dir, target) {
                return Some(d.abs());
            }
        }
        eval(&op.length)
    };
    let count = || -> Option<usize> {
        let n = crate::value::eval_count_in_doc(&op.count, doc)?;
        (n >= 1.0).then_some((n.round() as usize).min(MAX_REPEAT_INSTANCES))
    };
    // Fill modes never read `count`, and count modes never read `length`, but evaluating both
    // eagerly is side-effect-free and lets the shared spacing math stay input-only.
    spacing_offsets(op.mode, extent, count(), eval(&op.spacing), length())
}

/// The BREP solid of one repeat output: the input body's shape translated to its instance
/// offset along the axis.
fn occt_repeated_output_shape(
    doc: &Document,
    op_index: crate::model::RepeatOpKey,
    target: usize,
    instance: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.repeat_ops.get(op_index)?;
    let &input = op.targets.get(target)?;
    let m = repeat_instance_transform(doc, op, instance)?;
    let shape = occt_body_shape(doc, input)?;
    shape.transformed(&mat4_to_rows_3x4(&m))
}

/// The half-space cutting solid for one slice cutter: a large prism built on the cutter's
/// plane, occupying the `+normal` side. With `extend_infinite` the profile is a big square
/// covering the target; otherwise it's the cutter face's own boundary (a planar body face),
/// so the cut only reaches material within that footprint. Construction planes have no
/// finite boundary and always cut as infinite planes.
fn occt_slice_face_halfspace(
    doc: &Document,
    cutter: &FaceId,
    extend_infinite: bool,
    target: crate::model::BodyKey,
) -> Option<crate::kernel::Shape> {
    let frame = sketch_frame(doc, cutter.clone())?;
    let n = frame.normal.normalize_or_zero();
    if n == Vec3::ZERO {
        return None;
    }
    let (min, max) = body_solid_mesh_uncached(doc, target)?.bounds()?;
    let reach = (max - min).length().max(1.0) * 4.0;
    let finite = if extend_infinite {
        None
    } else {
        face_boundary_loop_world(doc, cutter).filter(|loop_world| loop_world.len() >= 3)
    };
    let profile = match finite {
        Some(loop_world) => loop_world,
        None => {
            // A big square in the plane, centered on the target's centroid projected onto
            // the cutter plane, sized to overhang the whole body.
            let u = frame.u_axis.normalize_or_zero();
            let v = frame.v_axis.normalize_or_zero();
            let centroid = (min + max) * 0.5;
            let center = centroid - n * (centroid - frame.origin).dot(n);
            let half = reach;
            vec![
                center - u * half - v * half,
                center + u * half - v * half,
                center + u * half + v * half,
                center - u * half + v * half,
            ]
        }
    };
    crate::kernel::Shape::prism(&profile, n * reach)
}

/// One effective slice cutter after chaining endpoint-connected laser lines into continuous
/// paths (#1142). A zigzag of three lines is one laser path (two fragments), not three
/// successive half-space cuts.
#[derive(Clone, Debug)]
enum EffectiveSliceCutter {
    Face(FaceId),
    /// Ordered sketch lines forming one continuous laser path (single line or chained).
    LinePath(Vec<crate::model::LineKey>),
}

const LASER_ENDPOINT_EPS: f32 = 1e-3;

/// Group endpoint-connected line cutters into continuous laser paths; face cutters stay
/// as-is. Order follows first appearance in `cutters`. At a branch (endpoint shared by
/// three or more lines) each edge is its own cutter so a Y of lines still cuts along
/// every leg.
fn effective_slice_cutters(
    doc: &Document,
    cutters: &[crate::model::SliceCutter],
) -> Vec<EffectiveSliceCutter> {
    let near = |a: Vec3, b: Vec3| (a - b).length_squared() < LASER_ENDPOINT_EPS * LASER_ENDPOINT_EPS;

    // Line cutters with world endpoints.
    let mut ends: Vec<(crate::model::LineKey, Vec3, Vec3)> = Vec::new();
    for c in cutters {
        if let crate::model::SliceCutter::Line { line } = c {
            if let Some(l) = doc.lines.get(*line) {
                if let Some(poly) = crate::face::line_world_polyline(doc, l) {
                    if poly.len() >= 2 {
                        ends.push((*line, poly[0], *poly.last().unwrap()));
                    }
                }
            }
        }
    }
    // Adjacency: for each line, the other lines that touch each endpoint.
    let touchers = |key: crate::model::LineKey, at: Vec3| -> Vec<crate::model::LineKey> {
        ends.iter()
            .filter(|(o, o0, o1)| *o != key && (near(at, *o0) || near(at, *o1)))
            .map(|(o, _, _)| *o)
            .collect()
    };
    // An endpoint is a branch when three or more lines meet there (this line + ≥2 others).
    let is_branch_end = |key: crate::model::LineKey, at: Vec3| touchers(key, at).len() >= 2;

    let mut used = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in cutters {
        match c {
            crate::model::SliceCutter::Face(face) => {
                out.push(EffectiveSliceCutter::Face(face.clone()));
            }
            crate::model::SliceCutter::Line { line } => {
                if !used.insert(*line) {
                    continue;
                }
                let Some(&(_, a, b)) = ends.iter().find(|(k, _, _)| *k == *line) else {
                    out.push(EffectiveSliceCutter::LinePath(vec![*line]));
                    continue;
                };
                // Branched at either end → this edge cuts alone.
                if is_branch_end(*line, a) || is_branch_end(*line, b) {
                    out.push(EffectiveSliceCutter::LinePath(vec![*line]));
                    continue;
                }
                // Walk a simple chain. Prefer a free end as the start so we cover the whole path.
                let start_at_a = touchers(*line, a).is_empty();
                // Build the ordered line list by walking from the seed.
                let mut path = vec![*line];
                // Direction 1: from the "forward" tip.
                let mut tip = if start_at_a { b } else { a };
                loop {
                    let nexts: Vec<_> = touchers(path[path.len() - 1], tip)
                        .into_iter()
                        .filter(|o| !used.contains(o))
                        .collect();
                    // Stop at branches or dead ends; simple path has exactly one unused neighbour.
                    if nexts.len() != 1 {
                        break;
                    }
                    let next = nexts[0];
                    let Some(&(_, n0, n1)) = ends.iter().find(|(k, _, _)| *k == next) else {
                        break;
                    };
                    // Don't continue through a branch vertex.
                    if is_branch_end(next, n0) || is_branch_end(next, n1) {
                        break;
                    }
                    used.insert(next);
                    path.push(next);
                    tip = if near(tip, n0) { n1 } else { n0 };
                }
                // Direction 2: from the other end of the seed (covers mid-chain seeds).
                let mut tip2 = if start_at_a { a } else { b };
                let mut front = Vec::new();
                loop {
                    let seed = if front.is_empty() {
                        path[0]
                    } else {
                        front[front.len() - 1]
                    };
                    let nexts: Vec<_> = touchers(seed, tip2)
                        .into_iter()
                        .filter(|o| !used.contains(o))
                        .collect();
                    if nexts.len() != 1 {
                        break;
                    }
                    let next = nexts[0];
                    let Some(&(_, n0, n1)) = ends.iter().find(|(k, _, _)| *k == next) else {
                        break;
                    };
                    if is_branch_end(next, n0) || is_branch_end(next, n1) {
                        break;
                    }
                    used.insert(next);
                    front.push(next);
                    tip2 = if near(tip2, n0) { n1 } else { n0 };
                }
                front.reverse();
                front.append(&mut path);
                out.push(EffectiveSliceCutter::LinePath(front));
            }
        }
    }
    out
}

/// How many laser-line cutters touch a world point (within endpoint epsilon). Degree 1
/// means a free end of the path graph — the only places Infinite cut may extend (#1145).
fn laser_endpoint_degree(
    doc: &Document,
    all_laser_lines: &[crate::model::LineKey],
    at: Vec3,
) -> usize {
    let eps2 = LASER_ENDPOINT_EPS * LASER_ENDPOINT_EPS;
    all_laser_lines
        .iter()
        .filter(|&&key| {
            let Some(line) = doc.lines.get(key) else {
                return false;
            };
            let Some(poly) = crate::face::line_world_polyline(doc, line) else {
                return false;
            };
            if poly.len() < 2 {
                return false;
            }
            (poly[0] - at).length_squared() < eps2
                || (poly.last().unwrap() - at).length_squared() < eps2
        })
        .count()
}

/// World polyline for a continuous laser path of one or more sketch lines, optionally
/// extended past its **free** ends along the end tangents (#1126/#1142/#1145).
///
/// `all_laser_lines` is the full set of line cutters in the op (not just this chain) so
/// free-end degree can be judged against the whole path graph. When `extend_to` is set
/// (axis-aligned box), only degree-1 free ends are pushed outward along the end tangents
/// **to that box's boundary** — never overshot and then clamped, which would drag ends
/// sideways along the AABB faces (#1147).
fn laser_path_world_polyline(
    doc: &Document,
    lines: &[crate::model::LineKey],
    extend_to: Option<(Vec3, Vec3)>,
    all_laser_lines: &[crate::model::LineKey],
) -> Option<(Vec<Vec3>, Vec3)> {
    if lines.is_empty() {
        return None;
    }
    // Collect each line's world polyline, then stitch. Orient the first segment so its
    // *end* attaches to the second — cutter picker order is not geometric order (#1146).
    let mut segments: Vec<Vec<Vec3>> = Vec::with_capacity(lines.len());
    let mut normal = Vec3::ZERO;
    for (i, &key) in lines.iter().enumerate() {
        let line = doc.lines.get(key)?;
        let frame = sketch_geometry_frame(doc, line.sketch)?;
        if i == 0 {
            normal = frame.normal.normalize_or_zero();
        }
        let poly = crate::face::line_world_polyline(doc, line)?;
        if poly.len() < 2 {
            return None;
        }
        segments.push(poly);
    }
    if normal == Vec3::ZERO {
        return None;
    }

    let mut path = segments.remove(0);
    if let Some(second) = segments.first() {
        let s0 = second[0];
        let s1 = *second.last().unwrap();
        let dist_end = (*path.last().unwrap() - s0)
            .length_squared()
            .min((*path.last().unwrap() - s1).length_squared());
        let dist_start = (path[0] - s0)
            .length_squared()
            .min((path[0] - s1).length_squared());
        if dist_start < dist_end {
            path.reverse();
        }
    }
    for mut poly in segments {
        let tip = *path.last().unwrap();
        let d_start = (poly[0] - tip).length_squared();
        let d_end = (poly.last().unwrap() - tip).length_squared();
        if d_end < d_start {
            poly.reverse();
        }
        // Drop the shared endpoint.
        if (poly[0] - tip).length_squared() < LASER_ENDPOINT_EPS * LASER_ENDPOINT_EPS {
            path.extend(poly.into_iter().skip(1));
        } else {
            path.extend(poly);
        }
    }
    path.dedup_by(|a, b| (*a - *b).length_squared() < 1e-12);
    if path.len() < 2 {
        return None;
    }
    if let Some((bmin, bmax)) = extend_to {
        // Snapshot free ends before moving them; degree is against the un-extended path.
        let start = path[0];
        let end = *path.last().unwrap();
        let start_free = laser_endpoint_degree(doc, all_laser_lines, start) <= 1;
        let end_free = laser_endpoint_degree(doc, all_laser_lines, end) <= 1;
        let d0 = (path[1] - path[0]).normalize_or_zero();
        let dn = (*path.last().unwrap() - path[path.len() - 2]).normalize_or_zero();
        // Ray-cast free ends to the box face along the end tangent (#1147). If the end is
        // already on the boundary and the tangent points out, the exit parameter is ~0 and
        // the vertex is left alone — a top-to-bottom path stays put.
        if start_free && d0 != Vec3::ZERO {
            let dir = -d0;
            if let Some((_t_enter, t_exit)) = line_aabb_interval(path[0], dir, bmin, bmax) {
                if t_exit > 1e-4 {
                    path[0] += dir * t_exit;
                }
            }
        }
        if end_free && dn != Vec3::ZERO {
            if let Some((_t_enter, t_exit)) = line_aabb_interval(*path.last().unwrap(), dn, bmin, bmax)
            {
                if t_exit > 1e-4 {
                    let last = path.len() - 1;
                    path[last] += dn * t_exit;
                }
            }
        }
    }
    Some((path, normal))
}

/// Slab intersection of the infinite line `origin + t * dir` with an AABB. Returns
/// `(t_enter, t_exit)` when the line hits the box; `dir` need not be unit.
fn line_aabb_interval(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<(f32, f32)> {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;
    for i in 0..3 {
        let o = origin[i];
        let d = dir[i];
        let lo = min[i];
        let hi = max[i];
        if d.abs() < 1e-12 {
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let mut t1 = (lo - o) / d;
        let mut t2 = (hi - o) / d;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }
    Some((tmin, tmax))
}

/// Project `p` into the plane frame `(origin, u, v)` → 2D.
fn laser_plane_to_2d(p: Vec3, origin: Vec3, u: Vec3, v: Vec3) -> [f32; 2] {
    let d = p - origin;
    [d.dot(u), d.dot(v)]
}

fn laser_plane_from_2d(p: [f32; 2], origin: Vec3, u: Vec3, v: Vec3) -> Vec3 {
    origin + u * p[0] + v * p[1]
}

/// Side index of a point on the boundary of an axis-aligned square, plus perimeter
/// parameter in `[0, 4)` (each side contributes 1). Used to walk the box boundary.
fn square_boundary_param(p: [f32; 2], lo: f32, hi: f32) -> Option<f32> {
    let eps = ((hi - lo).abs() * 1e-5).max(1e-5);
    let (x, y) = (p[0], p[1]);
    // Sides in CCW order starting at bottom (y=lo, x lo→hi): 0 bottom, 1 right, 2 top, 3 left.
    if (y - lo).abs() <= eps && x >= lo - eps && x <= hi + eps {
        return Some(0.0 + ((x - lo) / (hi - lo)).clamp(0.0, 1.0));
    }
    if (x - hi).abs() <= eps && y >= lo - eps && y <= hi + eps {
        return Some(1.0 + ((y - lo) / (hi - lo)).clamp(0.0, 1.0));
    }
    if (y - hi).abs() <= eps && x >= lo - eps && x <= hi + eps {
        return Some(2.0 + ((hi - x) / (hi - lo)).clamp(0.0, 1.0));
    }
    if (x - lo).abs() <= eps && y >= lo - eps && y <= hi + eps {
        return Some(3.0 + ((hi - y) / (hi - lo)).clamp(0.0, 1.0));
    }
    None
}

fn square_boundary_point(t: f32, lo: f32, hi: f32) -> [f32; 2] {
    let t = t.rem_euclid(4.0);
    let s = hi - lo;
    if t < 1.0 {
        [lo + s * t, lo]
    } else if t < 2.0 {
        [hi, lo + s * (t - 1.0)]
    } else if t < 3.0 {
        [hi - s * (t - 2.0), hi]
    } else {
        [lo, hi - s * (t - 3.0)]
    }
}

/// Walk the square boundary from `from_t` to `to_t` in the given direction (CCW if
/// `ccw`, else CW), emitting corner vertices and the end point (not the start).
fn square_boundary_walk(from_t: f32, to_t: f32, lo: f32, hi: f32, ccw: bool) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    let from_t = from_t.rem_euclid(4.0);
    let to_t = to_t.rem_euclid(4.0);
    if (from_t - to_t).abs() < 1e-6 {
        return out;
    }
    // Progress in the chosen direction until we pass `to_t`.
    let mut t = from_t;
    // Next corner in the walk direction.
    let next_corner = |t: f32, ccw: bool| -> f32 {
        if ccw {
            (t.floor() + 1.0).rem_euclid(4.0)
        } else {
            // Previous integer boundary, careful when t is already on a corner.
            let f = if (t - t.floor()).abs() < 1e-8 {
                (t - 1.0).rem_euclid(4.0)
            } else {
                t.floor().rem_euclid(4.0)
            };
            f
        }
    };
    let crosses = |a: f32, b: f32, target: f32, ccw: bool| -> bool {
        // Does the open segment a→b (direction ccw/cw on the circle R/4) contain target?
        if ccw {
            let span = (b - a).rem_euclid(4.0);
            let d = (target - a).rem_euclid(4.0);
            d > 1e-8 && d <= span + 1e-8
        } else {
            let span = (a - b).rem_euclid(4.0);
            let d = (a - target).rem_euclid(4.0);
            d > 1e-8 && d <= span + 1e-8
        }
    };
    for _ in 0..8 {
        let corner = next_corner(t, ccw);
        // If target is before the next corner, emit target and stop.
        if crosses(t, corner, to_t, ccw) || (corner - to_t).abs() < 1e-6 {
            out.push(square_boundary_point(to_t, lo, hi));
            break;
        }
        out.push(square_boundary_point(corner, lo, hi));
        t = corner;
        // Landed exactly on target corner.
        if (t - to_t).abs() < 1e-6 {
            break;
        }
    }
    out
}

/// Closed 2D polygon: the part of the square lying to the **left** of `path` (when the
/// path is traversed in order). `path` must start and end on the square boundary.
/// This is the correct half-region for a zigzag — thick left-strips cover both sides of
/// a sharp path and pull the cut away from the drawn line (#1146/#1148).
fn square_left_of_path(path: &[[f32; 2]], lo: f32, hi: f32) -> Option<Vec<[f32; 2]>> {
    if path.len() < 2 {
        return None;
    }
    let start = path[0];
    let end = *path.last().unwrap();
    let t_start = square_boundary_param(start, lo, hi)?;
    let t_end = square_boundary_param(end, lo, hi)?;
    // End tangent → left direction in 2D (CCW of tangent).
    let prev = path[path.len() - 2];
    let tx = end[0] - prev[0];
    let ty = end[1] - prev[1];
    let len = (tx * tx + ty * ty).sqrt();
    if len < 1e-12 {
        return None;
    }
    let (tx, ty) = (tx / len, ty / len);
    let left = [-ty, tx];
    // Sample a point just along the boundary from `end` in each direction; pick the
    // walk whose first step goes into the left half.
    let dparam = (1e-3 / (hi - lo).max(1e-6)).clamp(1e-4, 0.25);
    let p_ccw = square_boundary_point((t_end + dparam).rem_euclid(4.0), lo, hi);
    let p_cw = square_boundary_point((t_end - dparam).rem_euclid(4.0), lo, hi);
    let prefer_ccw = {
        let d_ccw = [p_ccw[0] - end[0], p_ccw[1] - end[1]];
        let d_cw = [p_cw[0] - end[0], p_cw[1] - end[1]];
        d_ccw[0] * left[0] + d_ccw[1] * left[1] >= d_cw[0] * left[0] + d_cw[1] * left[1]
    };
    let mut poly: Vec<[f32; 2]> = path.to_vec();
    poly.extend(square_boundary_walk(t_end, t_start, lo, hi, prefer_ccw));
    // Drop a trailing duplicate of start if the walk closed exactly.
    if poly.len() >= 2 {
        let a = poly[0];
        let b = *poly.last().unwrap();
        if (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5 {
            poly.pop();
        }
    }
    (poly.len() >= 3).then_some(poly)
}

/// Half-space solid on the left of a planar laser path (#1126/#1146/#1148): the path
/// splits a working square in the face plane that covers the target body; the left
/// region is extruded through ±n. Zigzags keep the drawn path as the exact cut surface
/// (thick left-strips used to envelope past the line — #1148).
///
/// When `extend_infinite` is false, free ends that already lie on (or nearly on) the
/// working square stay put; interior free ends are not forced to the boundary, so a
/// finite cut only spans the drawn path.
fn laser_left_region_solid(
    path: &[Vec3],
    n: Vec3,
    body_min: Vec3,
    body_max: Vec3,
    reach: f32,
    extend_infinite: bool,
) -> Option<crate::kernel::Shape> {
    let n = n.normalize_or_zero();
    if n == Vec3::ZERO || path.len() < 2 || reach <= 0.0 {
        return None;
    }
    // Orthonormal in-plane axes. Prefer a stable u from the first segment.
    let mut u = (path[1] - path[0]).normalize_or_zero();
    u = (u - n * u.dot(n)).normalize_or_zero();
    if u == Vec3::ZERO {
        u = n.any_orthonormal_vector();
    }
    let v = n.cross(u).normalize_or_zero();
    if v == Vec3::ZERO {
        return None;
    }
    // Origin at the body centroid projected onto the path's plane so the working
    // square is centered on the solid, not on a free path end.
    let centroid = (body_min + body_max) * 0.5;
    let origin = centroid - n * (centroid - path[0]).dot(n);

    // 2D bounds: body AABB corners + path points, padded, then expanded to a square
    // so boundary walking stays simple.
    let corners = [
        Vec3::new(body_min.x, body_min.y, body_min.z),
        Vec3::new(body_max.x, body_min.y, body_min.z),
        Vec3::new(body_min.x, body_max.y, body_min.z),
        Vec3::new(body_max.x, body_max.y, body_min.z),
        Vec3::new(body_min.x, body_min.y, body_max.z),
        Vec3::new(body_max.x, body_min.y, body_max.z),
        Vec3::new(body_min.x, body_max.y, body_max.z),
        Vec3::new(body_max.x, body_max.y, body_max.z),
    ];
    let mut min_u = f32::INFINITY;
    let mut max_u = f32::NEG_INFINITY;
    let mut min_v = f32::INFINITY;
    let mut max_v = f32::NEG_INFINITY;
    for p in corners.iter().chain(path.iter()) {
        let q = laser_plane_to_2d(*p, origin, u, v);
        min_u = min_u.min(q[0]);
        max_u = max_u.max(q[0]);
        min_v = min_v.min(q[1]);
        max_v = max_v.max(q[1]);
    }
    let pad = ((max_u - min_u).max(max_v - min_v) * 0.05).max(1.0);
    min_u -= pad;
    max_u += pad;
    min_v -= pad;
    max_v += pad;
    // Expand to a square centered on the 2D bounds.
    let cu = 0.5 * (min_u + max_u);
    let cv = 0.5 * (min_v + max_v);
    let half = 0.5 * (max_u - min_u).max(max_v - min_v);
    // square_left_of_path expects a square [lo,hi]²; re-origin so the working square is
    // centered at the UV centroid of the body+path bounds.
    let origin2 = origin + u * cu + v * cv;
    let lo = -half;
    let hi = half;

    let mut path2: Vec<[f32; 2]> = path
        .iter()
        .map(|p| laser_plane_to_2d(*p, origin2, u, v))
        .collect();

    // Ray-extend a free end in 2D to the square boundary along its outward tangent.
    let extend_end_2d = |path2: &mut Vec<[f32; 2]>, at_start: bool| {
        let npts = path2.len();
        let (i, j) = if at_start { (0, 1) } else { (npts - 1, npts - 2) };
        if square_boundary_param(path2[i], lo, hi).is_some() {
            return;
        }
        let dir = [
            path2[i][0] - path2[j][0],
            path2[i][1] - path2[j][1],
        ];
        let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
        if len < 1e-12 {
            return;
        }
        let dir = [dir[0] / len, dir[1] / len];
        let mut t_hit = f32::INFINITY;
        for (d, p0) in [(dir[0], path2[i][0]), (dir[1], path2[i][1])] {
            if d.abs() < 1e-12 {
                continue;
            }
            for bound in [lo, hi] {
                let t = (bound - p0) / d;
                if t > 1e-5 {
                    let q = [path2[i][0] + dir[0] * t, path2[i][1] + dir[1] * t];
                    if q[0] >= lo - 1e-4
                        && q[0] <= hi + 1e-4
                        && q[1] >= lo - 1e-4
                        && q[1] <= hi + 1e-4
                    {
                        t_hit = t_hit.min(t);
                    }
                }
            }
        }
        if t_hit.is_finite() {
            path2[i][0] += dir[0] * t_hit;
            path2[i][1] += dir[1] * t_hit;
        }
    };

    // Snapshot the un-extended ends for finite end-cap slabs, then always extend to
    // the square so box-split has a boundary-to-boundary path. A path that already
    // spans the body is unchanged inside the solid either way (#1147). For finite, the
    // end-cap slab below restricts the cut to the drawn span (#docs: "within its span").
    let start_world = path[0];
    let end_world = *path.last().unwrap();
    let d0_world = (path[1] - path[0]).normalize_or_zero();
    let dn_world = (*path.last().unwrap() - path[path.len() - 2]).normalize_or_zero();

    extend_end_2d(&mut path2, true);
    extend_end_2d(&mut path2, false);
    // Snap any residual floating error onto the boundary.
    let snap = |p: &mut [f32; 2]| {
        if square_boundary_param(*p, lo, hi).is_some() {
            return;
        }
        p[0] = p[0].clamp(lo, hi);
        p[1] = p[1].clamp(lo, hi);
        if square_boundary_param(*p, lo, hi).is_none() {
            let dx = (p[0] - lo).min(hi - p[0]);
            let dy = (p[1] - lo).min(hi - p[1]);
            if dx <= dy {
                p[0] = if p[0] - lo <= hi - p[0] { lo } else { hi };
            } else {
                p[1] = if p[1] - lo <= hi - p[1] { lo } else { hi };
            }
        }
    };
    let last = path2.len() - 1;
    snap(&mut path2[0]);
    snap(&mut path2[last]);

    let poly2 = square_left_of_path(&path2, lo, hi)?;
    // OCCT MakeFace from a wire is unreliable on concave polygons (a zigzag left-region
    // is concave). Ear-clip into triangles and fuse convex prisms (#1148).
    let mut solid = laser_fuse_triangulated_prism(&poly2, origin2, u, v, n, reach)?;

    if !extend_infinite && d0_world != Vec3::ZERO && dn_world != Vec3::ZERO {
        // Finite + short path: keep only the slab between the free-end planes of the
        // *drawn* path so a mid-face line does not sever past its span. When free ends
        // already lie on the body boundary (top-to-bottom cut), the extension outside
        // the solid is a no-op and end-caps must not clip — zigzag tangents would
        // otherwise shave material off the left region (#1147).
        let on_body_boundary = |p: Vec3| -> bool {
            let eps = pad.max(0.5);
            (p.x - body_min.x).abs() <= eps
                || (p.x - body_max.x).abs() <= eps
                || (p.y - body_min.y).abs() <= eps
                || (p.y - body_max.y).abs() <= eps
                || (p.z - body_min.z).abs() <= eps
                || (p.z - body_max.z).abs() <= eps
        };
        let short_path = !on_body_boundary(start_world) || !on_body_boundary(end_world);
        if short_path {
            use crate::kernel::BoolOp;
            // Half-space solid covering points with (x - origin) · dir >= 0.
            let halfspace = |origin: Vec3, dir: Vec3| -> Option<crate::kernel::Shape> {
                let dir = dir.normalize_or_zero();
                if dir == Vec3::ZERO {
                    return None;
                }
                let au = dir.any_orthonormal_vector();
                let av = dir.cross(au).normalize_or_zero();
                let h = reach;
                let profile = vec![
                    origin - au * h - av * h,
                    origin + au * h - av * h,
                    origin + au * h + av * h,
                    origin - au * h + av * h,
                ];
                crate::kernel::Shape::prism(&profile, dir * h)
            };
            if let Some(s0) = halfspace(start_world, d0_world) {
                solid = solid.boolean(&s0, BoolOp::Common).unwrap_or(solid);
            }
            if let Some(s1) = halfspace(end_world, -dn_world) {
                solid = solid.boolean(&s1, BoolOp::Common).unwrap_or(solid);
            }
        }
    }
    Some(solid)
}

/// Ear-clip a simple 2D polygon and fuse the triangular prisms into one solid.
fn laser_fuse_triangulated_prism(
    poly2: &[[f32; 2]],
    origin: Vec3,
    u: Vec3,
    v: Vec3,
    n: Vec3,
    reach: f32,
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    if poly2.len() < 3 {
        return None;
    }
    let tris = ear_clip_2d(poly2)?;
    let mut acc: Option<crate::kernel::Shape> = None;
    for [a, b, c] in tris {
        let profile = [
            laser_plane_from_2d(a, origin, u, v) - n * reach,
            laser_plane_from_2d(b, origin, u, v) - n * reach,
            laser_plane_from_2d(c, origin, u, v) - n * reach,
        ];
        let Some(prism) = crate::kernel::Shape::prism(&profile, n * (2.0 * reach)) else {
            continue;
        };
        acc = Some(match acc.take() {
            None => prism,
            Some(sum) => sum.boolean(&prism, BoolOp::Fuse).unwrap_or(sum),
        });
    }
    acc
}

/// Ear-clip a simple polygon (no holes) into triangles. Vertices CCW or CW; both work.
fn ear_clip_2d(poly: &[[f32; 2]]) -> Option<Vec<[[f32; 2]; 3]>> {
    let n = poly.len();
    if n < 3 {
        return None;
    }
    // Signed area to learn winding.
    let mut area = 0.0_f32;
    for i in 0..n {
        let j = (i + 1) % n;
        area += poly[i][0] * poly[j][1] - poly[j][0] * poly[i][1];
    }
    if area.abs() < 1e-12 {
        return None;
    }
    let ccw = area > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut tris = Vec::with_capacity(n - 2);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let i0 = idx[(i + m - 1) % m];
            let i1 = idx[i];
            let i2 = idx[(i + 1) % m];
            let a = poly[i0];
            let b = poly[i1];
            let c = poly[i2];
            // Reflex check: cross at b should match polygon winding for a convex ear.
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if (ccw && cross <= 1e-12) || (!ccw && cross >= -1e-12) {
                continue; // not a convex ear
            }
            // No other vertex inside triangle abc.
            let inside = |p: [f32; 2]| -> bool {
                let sign = if ccw { 1.0 } else { -1.0 };
                let c1 = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                let c2 = (c[0] - b[0]) * (p[1] - b[1]) - (c[1] - b[1]) * (p[0] - b[0]);
                let c3 = (a[0] - c[0]) * (p[1] - c[1]) - (a[1] - c[1]) * (p[0] - c[0]);
                c1 * sign >= -1e-8 && c2 * sign >= -1e-8 && c3 * sign >= -1e-8
            };
            let mut blocked = false;
            for &j in &idx {
                if j == i0 || j == i1 || j == i2 {
                    continue;
                }
                if inside(poly[j]) {
                    blocked = true;
                    break;
                }
            }
            if blocked {
                continue;
            }
            tris.push([a, b, c]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    if idx.len() == 3 {
        tris.push([poly[idx[0]], poly[idx[1]], poly[idx[2]]]);
    }
    (!tris.is_empty()).then_some(tris)
}

fn occt_slice_line_path_halfspace(
    doc: &Document,
    lines: &[crate::model::LineKey],
    extend_infinite: bool,
    target: crate::model::BodyKey,
    all_laser_lines: &[crate::model::LineKey],
) -> Option<crate::kernel::Shape> {
    let (min, max) = body_solid_mesh_uncached(doc, target)?.bounds()?;
    let reach = (max - min).length().max(1.0) * 4.0;
    // Free ends extend to a box that overhangs the body; ray-cast keeps the path
    // geometrically faithful (#1147). For finite cuts the path is used as drawn —
    // when it already spans the solid, infinite and finite match inside the body.
    let pad = (max - min).length().max(1.0) * 0.5;
    let extend_box = extend_infinite.then_some((min - Vec3::splat(pad), max + Vec3::splat(pad)));
    let (path, n) = laser_path_world_polyline(doc, lines, extend_box, all_laser_lines)?;

    // Straight line → a true plane half-space (same quality as a plane cutter).
    let dir = (*path.last().unwrap() - path[0]).normalize_or_zero();
    let straight = path.len() == 2
        || path.iter().all(|p| {
            let to = *p - path[0];
            to.length_squared() < 1e-12 || to.normalize_or_zero().cross(dir).length() < 1e-4
        });
    if straight && dir != Vec3::ZERO {
        let plane_n = dir.cross(n).normalize_or_zero();
        if plane_n == Vec3::ZERO {
            return None;
        }
        // Profile lives in the cutting plane (normal = plane_n); axes along the path and
        // the sketch normal (laser direction through the body).
        let u = dir;
        let v = n;
        let centroid = (min + max) * 0.5;
        let mid = (path[0] + *path.last().unwrap()) * 0.5;
        // Cutting plane through the laser path (#1151). Project the body centroid onto
        // that plane so the oversized profile still covers the solid — do **not** project
        // the path onto a plane through the centroid (that forced every straight laser
        // cut through the body centre, ignoring sketch edits).
        let center = centroid - plane_n * (centroid - mid).dot(plane_n);
        let half_u = if extend_infinite {
            reach
        } else {
            (*path.last().unwrap() - path[0]).length() * 0.5 + 1e-3
        };
        let half_v = reach;
        let profile = vec![
            center - u * half_u - v * half_v,
            center + u * half_u - v * half_v,
            center + u * half_u + v * half_v,
            center - u * half_u + v * half_v,
        ];
        return crate::kernel::Shape::prism(&profile, plane_n * reach);
    }

    // Polyline / zigzag (#1146/#1148): left region of the body-covering square split by
    // the path — cut surface is exactly the ruled extrusion of the drawn line.
    laser_left_region_solid(&path, n, min, max, reach, extend_infinite)
}

/// Dispatch an effective slice cutter to its half-space solid (#181 planar / #1126 laser).
fn occt_slice_effective_halfspace(
    doc: &Document,
    cutter: &EffectiveSliceCutter,
    extend_infinite: bool,
    target: crate::model::BodyKey,
    all_laser_lines: &[crate::model::LineKey],
) -> Option<crate::kernel::Shape> {
    match cutter {
        EffectiveSliceCutter::Face(face) => {
            occt_slice_face_halfspace(doc, face, extend_infinite, target)
        }
        EffectiveSliceCutter::LinePath(lines) => {
            occt_slice_line_path_halfspace(doc, lines, extend_infinite, target, all_laser_lines)
        }
    }
}

/// The ordered fragments one slice target splits into: start from the input body's solid(s)
/// and, for each effective cutter (endpoint-connected laser lines chained into one path,
/// #1142), replace every current piece with its two sides of the cutter's half-space,
/// dropping empty results. Deterministic order (common side before cut side, in cutter
/// order) keeps output-body mapping stable across edits.
fn occt_slice_pieces(doc: &Document, op_index: crate::model::SliceOpKey, target_pos: usize) -> Option<Vec<crate::kernel::Shape>> {
    use crate::kernel::BoolOp;
    const MIN_PIECE_VOLUME: f64 = 1e-6;
    let op = doc.slice_ops.get(op_index)?;
    let &input = op.targets.get(target_pos)?;
    // Inputs must precede this op's outputs; the guard breaks any accidental self-reference.
    if op.outputs.contains(&input) {
        return None;
    }
    let base = occt_body_shape(doc, input)?;
    let mut pieces: Vec<crate::kernel::Shape> = base.solids();
    if pieces.is_empty() {
        pieces = vec![base];
    }
    let all_laser_lines: Vec<crate::model::LineKey> = op
        .cutters
        .iter()
        .filter_map(|c| match c {
            crate::model::SliceCutter::Line { line } => Some(*line),
            _ => None,
        })
        .collect();
    let effective = effective_slice_cutters(doc, &op.cutters);
    for cutter in &effective {
        let Some(hs) =
            occt_slice_effective_halfspace(doc, cutter, op.extend_infinite, input, &all_laser_lines)
        else {
            continue;
        };
        let mut next = Vec::new();
        for piece in &pieces {
            for op_code in [BoolOp::Common, BoolOp::Cut] {
                if let Some(side) = piece.boolean(&hs, op_code) {
                    for solid in side.solids() {
                        if solid.volume().map(|v| v.abs() > MIN_PIECE_VOLUME).unwrap_or(false) {
                            next.push(solid);
                        }
                    }
                }
            }
        }
        if !next.is_empty() {
            pieces = next;
        }
    }
    Some(pieces)
}

/// Preview meshes of the laser cutting surfaces for in-progress slice cutters (#1142/#1144):
/// each continuous laser path becomes a **prismatic** ruled strip extruded along the face
/// normal — front and back edges are parallel translates of the path (#1217), not a
/// per-point AABB clip that shears into a trapezoid/triangle when the face is tilted.
/// Laterally, free ends with `extend_infinite` ray-cast to the body AABB along the end
/// tangent only (#1145/#1147). Depth along `n` is one shared interval for the whole path
/// so the laser line on the cut face stays parallel with the cut line on the back.
pub fn slice_laser_preview_meshes(
    doc: &Document,
    cutters: &[crate::model::SliceCutter],
    extend_infinite: bool,
    targets: &[crate::model::BodyKey],
) -> Vec<SolidMesh> {
    if cutters.is_empty() || targets.is_empty() {
        return Vec::new();
    }
    // Bounds over every target so the strip clears the largest body.
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for &bi in targets {
        if let Some(mesh) = body_solid_mesh(doc, bi) {
            if let Some((bmin, bmax)) = mesh.bounds() {
                min = min.min(bmin);
                max = max.max(bmax);
                any = true;
            }
        }
    }
    if !any {
        return Vec::new();
    }
    // Tiny pad so the surface is still visible right on the boundary faces.
    let pad = 1e-3;
    let bmin = min - Vec3::splat(pad);
    let bmax = max + Vec3::splat(pad);
    // Free ends extend only to the body AABB (ray-cast along the tangent) — a path that
    // already spans the solid is unchanged, so infinite and finite previews match (#1147).
    let extend = extend_infinite.then_some((bmin, bmax));
    let all_laser_lines: Vec<crate::model::LineKey> = cutters
        .iter()
        .filter_map(|c| match c {
            crate::model::SliceCutter::Line { line } => Some(*line),
            _ => None,
        })
        .collect();
    let mut meshes = Vec::new();
    for cutter in effective_slice_cutters(doc, cutters) {
        let EffectiveSliceCutter::LinePath(lines) = cutter else {
            continue;
        };
        let Some((path, n)) =
            laser_path_world_polyline(doc, &lines, extend, &all_laser_lines)
        else {
            continue;
        };
        if path.len() < 2 || n == Vec3::ZERO {
            continue;
        }
        // One shared depth along n for every path point (#1217). Per-point AABB
        // intervals shear the strip when the face normal is not axis-aligned (front
        // and back stop being parallel) and free ends that land on an AABB corner
        // collapse a ruling to a point — the goofy triangle. Pick the thickest
        // non-degenerate hit among path samples so the slab still covers the body.
        let mut best: Option<(f32, f32)> = None;
        let mut best_len = 0.0_f32;
        for &p in &path {
            let origin = if p.x >= bmin.x
                && p.x <= bmax.x
                && p.y >= bmin.y
                && p.y <= bmax.y
                && p.z >= bmin.z
                && p.z <= bmax.z
            {
                p
            } else {
                Vec3::new(
                    p.x.clamp(bmin.x, bmax.x),
                    p.y.clamp(bmin.y, bmax.y),
                    p.z.clamp(bmin.z, bmax.z),
                )
            };
            if let Some((t0, t1)) = line_aabb_interval(origin, n, bmin, bmax) {
                let len = (t1 - t0).abs();
                if len > best_len {
                    best_len = len;
                    best = Some((t0, t1));
                }
            }
        }
        let Some((t0, t1)) = best else {
            continue;
        };
        if best_len < 1e-6 {
            continue;
        }
        // Prismatic strip: same (t0, t1) at every path point. Path points stay where
        // free-end extension left them (#1147) — not axis-clamped per vertex.
        let mut clipped: Vec<(Vec3, Vec3)> = Vec::with_capacity(path.len());
        for &p in &path {
            clipped.push((p + n * t0, p + n * t1));
        }
        let mut triangles = Vec::with_capacity((clipped.len() - 1) * 2);
        for w in clipped.windows(2) {
            let (a0, a1) = w[0];
            let (b0, b1) = w[1];
            if (a0 - b0).length_squared() < 1e-12 && (a1 - b1).length_squared() < 1e-12 {
                continue;
            }
            triangles.push([a0, b0, b1]);
            triangles.push([a0, b1, a1]);
        }
        if !triangles.is_empty() {
            meshes.push(SolidMesh { triangles });
        }
    }
    meshes
}

/// Max distance from the laser path (ruled through the body) to the nearest point on
/// any fragment mesh of one slice target — a proxy for "does the cut follow the path?"
/// (#1148). Samples the path and a few offsets along the face normal into the solid.
pub fn slice_laser_cut_path_max_deviation(
    doc: &Document,
    op_index: crate::model::SliceOpKey,
    target_pos: usize,
) -> Option<f32> {
    let op = doc.slice_ops.get(op_index)?;
    let &input = op.targets.get(target_pos)?;
    let all_laser_lines: Vec<crate::model::LineKey> = op
        .cutters
        .iter()
        .filter_map(|c| match c {
            crate::model::SliceCutter::Line { line } => Some(*line),
            _ => None,
        })
        .collect();
    if all_laser_lines.is_empty() {
        return None;
    }
    let (bmin, bmax) = body_solid_mesh(doc, input)?.bounds()?;
    // Unextended path — the drawn line is the ground truth for "follows the path".
    let mut paths: Vec<(Vec<Vec3>, Vec3)> = Vec::new();
    for cutter in effective_slice_cutters(doc, &op.cutters) {
        let EffectiveSliceCutter::LinePath(lines) = cutter else {
            continue;
        };
        if let Some(pn) = laser_path_world_polyline(doc, &lines, None, &all_laser_lines) {
            paths.push(pn);
        }
    }
    if paths.is_empty() {
        return None;
    }
    // Fragment meshes for this target.
    let mut frag_meshes: Vec<SolidMesh> = Vec::new();
    for &out in &op.outputs {
        if let crate::model::BodySource::Sliced {
            op: o,
            target: t,
            ..
        } = doc.bodies.get(out)?.source
        {
            if o == op_index && t == target_pos {
                frag_meshes.push(body_solid_mesh(doc, out)?);
            }
        }
    }
    if frag_meshes.is_empty() {
        return None;
    }
    let _ = input;

    let point_to_mesh = |p: Vec3, mesh: &SolidMesh| -> f32 {
        let mut best = f32::INFINITY;
        for tri in &mesh.triangles {
            // Distance to triangle via point-to-segment on edges + plane if inside.
            for i in 0..3 {
                let a = tri[i];
                let b = tri[(i + 1) % 3];
                let ab = b - a;
                let t = if ab.length_squared() < 1e-12 {
                    0.0
                } else {
                    ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                };
                best = best.min((a + ab * t - p).length());
            }
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
            let nn = n.length_squared();
            if nn > 1e-12 {
                let n = n / nn.sqrt();
                let dist_plane = (p - tri[0]).dot(n).abs();
                // Barycentric inside check (planar).
                let a = tri[0];
                let b = tri[1];
                let c = tri[2];
                let v0 = c - a;
                let v1 = b - a;
                let v2 = p - n * (p - a).dot(n) - a;
                let dot00 = v0.dot(v0);
                let dot01 = v0.dot(v1);
                let dot02 = v0.dot(v2);
                let dot11 = v1.dot(v1);
                let dot12 = v1.dot(v2);
                let inv = 1.0 / (dot00 * dot11 - dot01 * dot01);
                let u = (dot11 * dot02 - dot01 * dot12) * inv;
                let v = (dot00 * dot12 - dot01 * dot02) * inv;
                if u >= -1e-3 && v >= -1e-3 && u + v <= 1.0 + 1e-3 {
                    best = best.min(dist_plane);
                }
            }
        }
        best
    };

    let mut max_dev = 0.0_f32;
    for (path, n) in &paths {
        let n = n.normalize_or_zero();
        if n == Vec3::ZERO || path.len() < 2 {
            continue;
        }
        // Sample each segment (including vertices) and a mid-depth point along ±n
        // inside the body — the cut surface is the ruled extrusion of the path.
        for w in path.windows(2) {
            for s in [0.0, 0.5, 1.0] {
                let p = w[0] + (w[1] - w[0]) * s;
                // Probe along the laser into the body AABB.
                let Some((t0, t1)) = line_aabb_interval(p, n, bmin, bmax) else {
                    continue;
                };
                let mid_t = 0.5 * (t0 + t1);
                let sample = p + n * mid_t;
                let mut best = f32::INFINITY;
                for mesh in &frag_meshes {
                    best = best.min(point_to_mesh(sample, mesh));
                }
                max_dev = max_dev.max(best);
            }
        }
    }
    Some(max_dev)
}

/// The BREP solid of one slice fragment: piece `piece` of target `target`. The target's
/// *last* fragment absorbs any extra solids a rebuild produced (fused into one shape), so
/// the body list stays stable while geometry changes underneath — same contract as boolean
/// outputs.
fn occt_sliced_output_shape(
    doc: &Document,
    op_index: crate::model::SliceOpKey,
    target: usize,
    piece: usize,
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    let mut pieces = occt_slice_pieces(doc, op_index, target)?;
    if pieces.is_empty() {
        return None;
    }
    // The stable fragment count for this target is how many output bodies it owns.
    let owned = slice_target_body_count(doc, op_index, target);
    let last = owned.saturating_sub(1);
    if piece > last || piece >= pieces.len() && piece != last {
        return None;
    }
    if piece == last && pieces.len() > owned {
        let mut extra = pieces.drain(last..).collect::<Vec<_>>().into_iter();
        let mut sum = extra.next()?;
        for s in extra {
            sum = sum.boolean(&s, BoolOp::Fuse)?;
        }
        return Some(sum);
    }
    if piece < pieces.len() {
        Some(pieces.swap_remove(piece))
    } else {
        None
    }
}

/// How many (live) output bodies a slice target currently owns — the authoritative,
/// stable fragment count, recovered from the `BodySource::Sliced` sources.
fn slice_target_body_count(doc: &Document, op_index: crate::model::SliceOpKey, target: usize) -> usize {
    doc.bodies
        .iter()
        .filter(|(_, b)| {
            matches!(
                    b.source,
                    crate::model::BodySource::Sliced { op, target: t, .. }
                        if op == op_index && t == target
                )
        })
        .count()
}

/// Number of fragments a slice target currently produces (commit-time output sizing).
pub fn slice_piece_count(doc: &Document, op_index: crate::model::SliceOpKey, target: usize) -> Option<usize> {
    Some(occt_slice_pieces(doc, op_index, target)?.len())
}

/// The BREP solid of one shell-operation output (#1156): the input body hollowed to the
/// op's wall thickness with the listed open faces removed.
fn occt_shelled_output_shape(
    doc: &Document,
    op_index: crate::model::ShellOpKey,
    target: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.shell_ops.get(op_index)?;
    let &input = op.targets.get(target)?;
    if op.outputs.contains(&input) {
        return None;
    }
    let shape = occt_body_shape(doc, input)?;
    let thickness = crate::value::eval_length_mm_in_doc(&op.thickness, doc)?;
    if !(thickness > 0.0) {
        return None;
    }
    // Open faces belonging to this target: convert FaceId → (point, outward normal).
    // Match against the *input* body via source ownership, not body_index_for_face —
    // after the shell exists that prefers the live shelled output and would drop every
    // open face on a primitive target (#1172).
    let mut open: Vec<(glam::Vec3, glam::Vec3)> = Vec::new();
    for face in &op.open_faces {
        if !crate::model::face_belongs_to_body(doc, face, input) {
            continue;
        }
        let Some(frame) = crate::face::sketch_frame(doc, face.clone()) else {
            continue;
        };
        let point = face_boundary_loop_world(doc, face)
            .map(|pts| pts.iter().copied().sum::<glam::Vec3>() / pts.len().max(1) as f32)
            .unwrap_or(frame.origin);
        let normal = frame.normal.normalize_or_zero();
        if normal.length_squared() < 1e-12 {
            continue;
        }
        open.push((point, normal));
    }
    shape.shell(&open, thickness)
}

/// Preview meshes for an in-progress shell (#1156): the would-be hollowed solids for each
/// target (falls back to the solid input mesh when the kernel can't build).
pub fn preview_shell_meshes(
    doc: &Document,
    targets: &[crate::model::BodyKey],
    open_faces: &[crate::model::FaceId],
    thickness: f32,
) -> Option<Vec<SolidMesh>> {
    if !(thickness > 0.0) || targets.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(targets.len());
    for &bi in targets {
        let Some(shape) = occt_body_shape(doc, bi) else {
            // Fall back to the un-shelled solid mesh so the preview still shows something.
            out.push(body_solid_mesh(doc, bi)?);
            continue;
        };
        let mut open: Vec<(glam::Vec3, glam::Vec3)> = Vec::new();
        for face in open_faces {
            // Same ownership check as occt_shelled_output_shape (#1172).
            if !crate::model::face_belongs_to_body(doc, face, bi) {
                continue;
            }
            let Some(frame) = crate::face::sketch_frame(doc, face.clone()) else {
                continue;
            };
            let point = face_boundary_loop_world(doc, face)
                .map(|pts| pts.iter().copied().sum::<glam::Vec3>() / pts.len().max(1) as f32)
                .unwrap_or(frame.origin);
            let normal = frame.normal.normalize_or_zero();
            if normal.length_squared() < 1e-12 {
                continue;
            }
            open.push((point, normal));
        }
        if let Some(shelled) = shape.shell(&open, thickness) {
            let tris = shelled.tessellate(0.2);
            if !tris.is_empty() {
                out.push(SolidMesh { triangles: tris });
                continue;
            }
        }
        out.push(body_solid_mesh(doc, bi)?);
    }
    Some(out)
}


/// The whole (possibly multi-solid) OCCT result of one boolean operation: A-side bodies
/// fused, then combined with the fused B side per the operation's algebra. Difference
/// (symmetric) is (A∪B) − (A∩B). `None` when any input body isn't kernel-representable.
fn occt_boolean_result_shape(
    doc: &Document,
    op_index: crate::model::BooleanOpKey,
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    let op = doc.boolean_ops.get(op_index)?;
    let fuse_all = |list: &[crate::model::BodyKey]| -> Option<crate::kernel::Shape> {
        let mut acc: Option<crate::kernel::Shape> = None;
        for &bi in list {
            // Inputs must precede this op's outputs; the index guard breaks any accidental
            // self-reference cycle (an output can never be its own op's input).
            if op.outputs.contains(&bi) {
                return None;
            }
            let shape = occt_body_shape(doc, bi)?;
            acc = Some(match acc {
                None => shape,
                Some(sum) => sum.boolean(&shape, BoolOp::Fuse)?,
            });
        }
        acc
    };
    let a = fuse_all(&op.a)?;
    match op.kind {
        crate::model::BooleanOpKind::Combine => Some(a),
        crate::model::BooleanOpKind::Cut => {
            let b = fuse_all(&op.b)?;
            a.boolean(&b, BoolOp::Cut)
        }
        crate::model::BooleanOpKind::Intersect => {
            let b = fuse_all(&op.b)?;
            a.boolean(&b, BoolOp::Common)
        }
        crate::model::BooleanOpKind::Difference => {
            let b = fuse_all(&op.b)?;
            let union = a.boolean(&b, BoolOp::Fuse)?;
            let common = a.boolean(&b, BoolOp::Common)?;
            union.boolean(&common, BoolOp::Cut)
        }
    }
}

/// The BREP solid of one boolean output body: solid `ordinal` of the operation's split
/// result. The op's *last* output absorbs any extra solids a rebuild produced (fused into
/// one shape), so the body list stays stable while geometry changes underneath.
fn occt_boolean_output_shape(
    doc: &Document,
    op_index: crate::model::BooleanOpKey,
    ordinal: usize,
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    let op = doc.boolean_ops.get(op_index)?;
    let result = occt_boolean_result_shape(doc, op_index)?;
    let mut solids = result.solids();
    if solids.is_empty() {
        return None;
    }
    let last = op.outputs.len().saturating_sub(1);
    if ordinal > last || ordinal >= solids.len() && ordinal != last {
        return None;
    }
    if ordinal == last && solids.len() > op.outputs.len() {
        let mut acc = solids.drain(last..).collect::<Vec<_>>().into_iter();
        let mut sum = acc.next()?;
        for extra in acc {
            sum = sum.boolean(&extra, BoolOp::Fuse)?;
        }
        return Some(sum);
    }
    if ordinal < solids.len() {
        Some(solids.swap_remove(ordinal))
    } else {
        None
    }
}

/// Kernel solids of a boolean, tessellated — for off-thread precompute so the UI does not
/// freeze while a heavy cut/fuse runs (#1031). `op` must already be in `doc.boolean_ops`.
/// Empty when the boolean built but produced no solid (disjoint intersect, a cut that
/// consumes the target). `None` when the kernel cannot represent an input.
pub fn boolean_result_meshes(
    doc: &Document,
    op_index: crate::model::BooleanOpKey,
) -> Option<Vec<SolidMesh>> {
    let result = occt_boolean_result_shape(doc, op_index)?;
    let solids = result.solids();
    let meshes: Vec<SolidMesh> = solids
        .into_iter()
        .map(|s| SolidMesh {
            triangles: s.tessellate(OCCT_DEFLECTION as f64),
        })
        .filter(|m| !m.triangles.is_empty())
        .collect();
    Some(meshes)
}

fn boolean_empty_error(kind: crate::model::BooleanOpKind) -> String {
    format!("{} result is empty", kind.label())
}

/// Probe a would-be boolean without committing it: clone is the caller's, the op is pushed
/// temporarily, and the kernel result is tessellated. Used by the background combine job
/// (#1031). An empty result (no solid) is an error so the caller never inserts a phantom.
pub fn precompute_boolean(
    doc: &Document,
    kind: crate::model::BooleanOpKind,
    a: &[crate::model::BodyKey],
    b: &[crate::model::BodyKey],
    keep_b: bool,
) -> Result<Vec<SolidMesh>, String> {
    let mut probe = doc.clone();
    let op_index = probe.boolean_ops.insert(crate::model::BooleanOperation {
        kind,
        a: a.to_vec(),
        b: b.to_vec(),
        keep_b,
        outputs: Vec::new(),
        name: None,
    });
    let meshes = boolean_result_meshes(&probe, op_index)
        .ok_or_else(|| "Boolean failed — one of the bodies may not be kernel-representable".to_string())?;
    if meshes.is_empty() {
        return Err(boolean_empty_error(kind));
    }
    Ok(meshes)
}

thread_local! {
    /// The Combine tool's live result preview (#1033), keyed by (document, picked sides).
    /// One entry: there is at most one preview at a time, and the picks only change on a
    /// click — so every frame between clicks is free rather than another kernel boolean.
    static PREVIEW_BOOLEAN_CACHE: std::cell::RefCell<Option<((u64, u64), Option<Vec<SolidMesh>>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Live preview of what the Combine tool would produce from the bodies picked so far
/// (#1033) — the same solids a commit builds, so the hole a cut takes out is visible
/// before committing it. `None` until each side the operation needs is populated, or when
/// the kernel can't build the result. Cached per (document, kind, sides).
pub fn preview_boolean_meshes(
    doc: &Document,
    kind: crate::model::BooleanOpKind,
    a: &[crate::model::BodyKey],
    b: &[crate::model::BodyKey],
) -> Option<Vec<SolidMesh>> {
    use std::hash::{Hash, Hasher};
    // Combine unions one picked set; the two-sided operations need both sides.
    let ready = match kind {
        crate::model::BooleanOpKind::Combine => a.len() >= 2,
        _ => !a.is_empty() && !b.is_empty(),
    };
    if !ready {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind as u8).hash(&mut h);
    a.hash(&mut h);
    b.hash(&mut h);
    let key = (document_mesh_fingerprint(doc), h.finish());
    PREVIEW_BOOLEAN_CACHE.with(|cache| {
        if let Some((cached_key, meshes)) = cache.borrow().as_ref() {
            if *cached_key == key {
                return meshes.clone();
            }
        }
        // `keep_b` only decides whether the B inputs survive as their own bodies; it
        // doesn't change the result solids, so the preview doesn't need it.
        let meshes = precompute_boolean(doc, kind, a, b, false)
            .ok()
            .filter(|ms| ms.iter().any(|m| !m.triangles.is_empty()));
        *cache.borrow_mut() = Some((key, meshes.clone()));
        meshes
    })
}

/// Seed the per-thread mesh cache with a precomputed body mesh so the first paint after a
/// background boolean does not re-run the kernel (#1031).
pub fn warm_body_mesh_cache(doc: &Document, body_index: crate::model::BodyKey, mesh: SolidMesh) {
    let fingerprint = document_mesh_fingerprint(doc);
    BODY_MESH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        cache.1.insert(body_index, Some(mesh.clone()));
    });
    record_committed_mesh(doc, body_index, &mesh);
}


/// Commit-time kernel feasibility trial for a 3D edge treatment (#103). `candidate` is the
/// would-be extrusion (built by [`extrusion_with_edge_treatment`], the treatment already
/// spliced in); `extrusion` indexes its current, committed source in `doc`.
///
/// Returns `false` only when the kernel builds the *current* extrusion fine but can't build
/// the candidate — i.e. the new treatment itself is what breaks it (an impossible fillet
/// radius / chamfer distance), which would silently knock the whole body onto the additive-
/// only mesh fallback and delete its cut holes from the render. Returns `true` whenever the
/// kernel has no say: the current extrusion isn't kernel-representable anyway (the mesh-bevel
/// fallback governs before *and* after, nothing to validate against), or it's missing/
/// degenerate (other commit checks own those rejections).
pub fn occt_edge_treatments_feasible(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    candidate: &Extrusion,
) -> bool {
    let Some(base) = doc.extrusions.get(extrusion) else {
        return true;
    };
    let distance = effective_distance(doc, base);
    if base.faces.is_empty() || distance.abs() < 1e-4 {
        return true;
    }
    // When the extrusion belongs to a body, trial the *body* build — that's where the
    // treatment actually lands (a cut extrusion's rim chamfer is applied to the body as a
    // countersink after subtraction, #177 — the standalone tool never carries it).
    if let Some(bi) = doc
        .bodies
        .iter()
        .find_map(|(k, b)| b.source.owns_extrusion(extrusion).then_some(k))
    {
        if occt_body_shape(doc, bi).is_none() {
            return true;
        }
        let mut clone = doc.clone();
        clone.extrusions[extrusion] = candidate.clone();
        return occt_body_shape(&clone, bi).is_some();
    }
    if occt_extrusion_shape(doc, base, distance).is_none() {
        return true;
    }
    occt_extrusion_shape(doc, candidate, distance).is_some()
}

/// #103 part 2: a status-bar warning when some body would render *wrong* geometry — it has
/// cut extrusions, the kernel is compiled in, but [`occt_body_shape`] can't build it (e.g. a
/// pre-existing kernel-infeasible edge treatment), so [`body_solid_mesh`] falls back to the
/// hand-rolled additive-only mesher and the cuts silently vanish from the render. `None` when
/// every cut-bearing body builds (or there are none). Recomputed by
/// [`crate::actions::AppState::refresh_document_health`] at every document mutation point
/// (and on open), never per-frame.
pub fn kernel_fallback_cut_warning(doc: &Document) -> Option<String> {
    for (i, body) in doc.bodies.iter() {
        let cut_by_revolve = revolutions_targeting(doc, i).iter().any(|(_, cut)| *cut);
        let cut_by_sweep = sweeps_targeting(doc, i).iter().any(|(_, cut)| *cut);
        let cut_by_loft = lofts_targeting(doc, i).iter().any(|(_, cut)| *cut);
        if body.source.imported_mesh_key().is_some()
            || (body.source.cut_extrusion_indices().is_empty()
                && !cut_by_revolve
                && !cut_by_sweep
                && !cut_by_loft)
        {
            continue;
        }
        if occt_body_shape(doc, i).is_none() {
            let label = body
                .name
                .clone()
                .unwrap_or_else(|| format!("body {}", i.index()));
            return Some(format!(
                "Warning: {label} couldn't be built by the kernel — cuts are not shown \
                 (falling back to approximate geometry)"
            ));
        }
    }
    None
}

/// Linear tessellation deflection (mm) for OCCT meshing (#86). Flat prism faces
/// triangulate exactly regardless; this only bounds the chord error on curved
/// faces once those go through the kernel.
pub const OCCT_DEFLECTION: f32 = 0.05;

/// Live cut/boolean preview tessellation (#1337). Coarser than [`OCCT_DEFLECTION`]
/// so dragging a cylinder through a shelled/sliced body stays interactive; commit
/// still uses the fine deflection.
pub const OCCT_PREVIEW_DEFLECTION: f32 = 0.4;

/// How far a cut tool overshoots each end past its nominal extent so its caps never sit
/// exactly on a body face (which would leave a coincident seam face; #200). Small enough to
/// be geometrically irrelevant, large enough to clear float noise at typical mm scale.
const CUT_TOOL_OVERSHOOT: f32 = 0.05;

/// World-space axis (origin, unit direction) a [`crate::model::Revolution`] sweeps around.
pub fn revolve_axis_world(
    doc: &Document,
    rev: &crate::model::Revolution,
) -> Option<(Vec3, Vec3)> {
    axis_world(doc, rev.axis)
}

/// Effective sweep angle in degrees for building the solid.
///
/// Pure revolve (`pitch_mm ≈ 0`): magnitude is clamped to (0.1, 360] — multi-turn without
/// pitch collapses onto one full turn. Helical revolve keeps the signed multi-turn angle
/// so springs can wind more than once (#1242). Zero is nudged to a tiny positive so the
/// mesh still builds something pickable.
pub fn revolve_effective_angle(rev: &crate::model::Revolution) -> f32 {
    let a = rev.angle_deg;
    if rev.pitch_mm.abs() < 1e-9 {
        let mag = a.abs().clamp(0.1, 360.0);
        if a < 0.0 {
            -mag
        } else {
            mag
        }
    } else if a.abs() < 0.1 {
        if a < 0.0 {
            -0.1
        } else {
            0.1
        }
    } else {
        a
    }
}

/// Whether the revolve has distinct start/end profile caps (not closed on itself).
/// A pure full turn closes; a helix always has ends (#1242).
pub fn revolve_has_caps(rev: &crate::model::Revolution) -> bool {
    rev.pitch_mm.abs() > 1e-6 || revolve_effective_angle(rev).abs() < 359.99
}

/// Axial span of the revolve profiles along the axis (max − min of points projected onto
/// the axis direction). Used to convert Gap ↔ Offset for helical pitch (#1242).
pub fn revolve_profile_axial_extent(
    doc: &Document,
    rev: &crate::model::Revolution,
) -> Option<f32> {
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let dir = dir.normalize_or_zero();
    if dir.length_squared() < 1e-8 {
        return None;
    }
    let mut min_t = f32::INFINITY;
    let mut max_t = f32::NEG_INFINITY;
    for face in &rev.faces {
        let (pts, _) = face_profile_world(doc, face)?;
        for p in pts {
            let t = (p - origin).dot(dir);
            min_t = min_t.min(t);
            max_t = max_t.max(t);
        }
    }
    if !min_t.is_finite() {
        return None;
    }
    Some((max_t - min_t).max(0.0))
}

/// Helical transform: rotate `p` around `axis` (through `origin`) by `angle_deg` and
/// translate along the axis by `pitch_mm * angle_deg / 360` (#1242).
fn helical_point(origin: Vec3, dir: Vec3, pitch_mm: f32, angle_deg: f32, p: Vec3) -> Vec3 {
    let q = glam::Quat::from_axis_angle(dir, angle_deg.to_radians());
    let axial = pitch_mm * (angle_deg / 360.0);
    origin + q * (p - origin) + dir * axial
}

/// World polygon of a partial revolve's flat start/end side (#621): the profile rotated
/// to the sweep's start (`end = false`) or end (`end = true`) angle, plus the outward
/// face normal (the sweep-tangent direction at the cap, pointing out of the solid).
/// `None` for a full 360° sweep — that closes on itself and has no flat sides.
pub fn revolve_cap_polygon_world(
    doc: &Document,
    revolution: crate::model::RevolutionKey,
    profile: &crate::model::ExtrudeFace,
    end: bool,
) -> Option<(Vec<Vec3>, Vec3)> {
    let rev = doc.revolutions.get(revolution)?;
    if !rev.faces.contains(profile) {
        return None;
    }
    if !revolve_has_caps(rev) {
        return None;
    }
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let angle = revolve_effective_angle(rev);
    let start = if rev.symmetric { -angle / 2.0 } else { 0.0 };
    let cap_angle = if end { start + angle } else { start };
    let (pts, _) = face_profile_world(doc, profile)?;
    if pts.len() < 3 {
        return None;
    }
    let poly: Vec<Vec3> = pts
        .iter()
        .map(|p| helical_point(origin, dir, rev.pitch_mm, cap_angle, *p))
        .collect();
    // Sweep tangent at the rotated centroid (direction of increasing angle, right-hand
    // rule about `dir`): the end cap faces along it, the start cap opposite. With pitch
    // the path is helical, so the outward also has a small axial component; for picking
    // the circumferential tangent is still the useful normal of a flat-ish cap.
    let centroid = poly.iter().copied().sum::<Vec3>() / poly.len() as f32;
    let radial = centroid - (origin + dir * (centroid - origin).dot(dir));
    let tangent = dir.cross(radial).normalize_or_zero();
    if tangent.length_squared() < 1e-8 {
        return None;
    }
    let outward = if end { tangent } else { -tangent };
    Some((poly, outward))
}

/// How many flat side-face candidates a revolve profile can sweep (#621): one per polygon
/// edge (each validated by [`revolve_side_geom`] — only edges perpendicular to the axis
/// sweep flat faces); circles and boolean profiles sweep no flat sides (mirrors
/// [`side_face_count`]'s documented limitation).
pub fn revolve_side_count(profile: &ExtrudeFace) -> usize {
    match profile {
        ExtrudeFace::Polygon(lines) => lines.len(),
        _ => 0,
    }
}

/// The flat washer/annular-sector face swept by one polygon-profile `edge` of a revolve
/// (#621), when that edge's endpoints share an axis coordinate — the sweep then stays in
/// the perpendicular plane there (e.g. the flat ends of a revolved ring). Returns the
/// boundary polygon (world), the face's sketch frame — normal pointing out of the solid
/// (away from the profile along the axis), origin on the axis — and a point guaranteed to
/// lie **on** the face (the unrotated edge's midpoint; a full washer's boundary centroid
/// sits in its hole, #625). `None` for edges that sweep curved surfaces.
pub fn revolve_side_geom(
    doc: &Document,
    revolution: crate::model::RevolutionKey,
    profile: &ExtrudeFace,
    edge: usize,
) -> Option<(Vec<Vec3>, SketchFrame, Vec3)> {
    let rev = doc.revolutions.get(revolution)?;
    if !rev.faces.contains(profile) {
        return None;
    }
    // Helical sides are not flat — only pure revolves expose flat washer faces (#1242).
    if rev.pitch_mm.abs() > 1e-6 {
        return None;
    }
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let (pts, _) = face_profile_world(doc, profile)?;
    let n = pts.len();
    if n < 3 || edge >= n {
        return None;
    }
    let (a, b) = (pts[edge], pts[(edge + 1) % n]);
    let ta = (a - origin).dot(dir);
    let tb = (b - origin).dot(dir);
    if (ta - tb).abs() > 1e-3 {
        return None;
    }
    let center = origin + dir * ((ta + tb) * 0.5);
    let (ra, rb) = ((a - center).length(), (b - center).length());
    if ra.max(rb) < 1e-4 {
        return None;
    }
    // Outward normal: away from the rest of the profile along the axis.
    let tc = pts.iter().map(|p| (*p - origin).dot(dir)).sum::<f32>() / n as f32;
    let normal = if tc > ta { -dir } else { dir };
    let u_axis = ((if ra >= rb { a } else { b }) - center).normalize_or_zero();
    if u_axis.length_squared() < 1e-8 {
        return None;
    }
    let v_axis = normal.cross(u_axis).normalize_or_zero();
    let frame = SketchFrame {
        origin: center,
        u_axis,
        v_axis,
        normal,
    };
    let angle = revolve_effective_angle(rev);
    let start = if rev.symmetric { -angle / 2.0 } else { 0.0 };
    let full = !revolve_has_caps(rev);
    let steps = (((CIRCLE_SEGMENTS as f32) * angle.abs() / 360.0).ceil() as usize).max(8);
    let arc = |p: Vec3, reverse: bool| -> Vec<Vec3> {
        (0..=steps)
            .map(|i| {
                let i = if reverse { steps - i } else { i };
                let rad = (start + angle * i as f32 / steps as f32).to_radians();
                center + glam::Quat::from_axis_angle(dir, rad) * (p - center)
            })
            .collect()
    };
    // Boundary: the two endpoints' sweep arcs (a forward, b back) close into a loop for
    // partial sweeps. A full sweep's washer is approximated by its outer rim for
    // pick/highlight purposes — the same hole-blind simplification extrusion caps use.
    let boundary = if full {
        arc(if ra >= rb { a } else { b }, false)
    } else {
        let mut poly = arc(a, false);
        poly.extend(arc(b, true));
        poly
    };
    Some((boundary, frame, (a + b) * 0.5))
}

/// Inner/outer radii of a full-sweep revolve side's washer (#625): `Some` only when the
/// swept region is a complete annulus (sweep ≥ 360°), which a boundary line loop can't
/// represent — the rim polygon would fill the hole — so callers mirror it into a sketch
/// as real circles instead. Radii are measured from the axis in the face's plane.
pub fn revolve_side_annulus(
    doc: &Document,
    revolution: crate::model::RevolutionKey,
    profile: &ExtrudeFace,
    edge: usize,
) -> Option<(f32, f32)> {
    let rev = doc.revolutions.get(revolution)?;
    // Full pure washer only — a helix has no closed annulus (#1242).
    if revolve_has_caps(rev) {
        return None;
    }
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let (pts, _) = face_profile_world(doc, profile)?;
    let n = pts.len();
    if edge >= n {
        return None;
    }
    let (a, b) = (pts[edge], pts[(edge + 1) % n]);
    let ta = (a - origin).dot(dir);
    let center = origin + dir * ta;
    let (ra, rb) = ((a - center).length(), (b - center).length());
    Some((ra.min(rb), ra.max(rb)))
}

/// Angular steps for the hand-rolled revolve lathe mesh (#1242/#1249) — live ghost
/// preview and no-kernel fallback only. The committed viewport uses the kernel's
/// smooth helix BREP tessellation instead.
///
/// Pure revolve keeps the usual [`CIRCLE_SEGMENTS`] density for one turn. Helical
/// multi-turn springs scale at a quarter of that per turn with a hard cap so the
/// ghost stays light while dragging angle/pitch.
pub fn revolve_mesh_steps(angle_deg: f32, pitch_mm: f32) -> usize {
    if pitch_mm.abs() > 1e-6 {
        const PER_TURN: f32 = 24.0;
        const MAX_STEPS: usize = 256;
        let steps = ((PER_TURN * angle_deg.abs() / 360.0).ceil() as usize).max(8);
        steps.min(MAX_STEPS)
    } else {
        (((CIRCLE_SEGMENTS as f32) * angle_deg.abs() / 360.0).ceil() as usize).max(8)
    }
}

/// Hand-rolled lathe mesh for a revolution (the no-kernel fallback and the live ghost
/// preview): each profile is swept around the axis in angular steps, walls stitched
/// between consecutive rotated rings, with the start/end profile faces capped for a
/// partial sweep (a full 360-degree pure revolve closes on itself and needs no caps).
/// Non-zero pitch advances each ring along the axis for a helical spring coil (#1242).
pub fn revolve_mesh(doc: &Document, rev: &crate::model::Revolution) -> Option<SolidMesh> {
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let angle = revolve_effective_angle(rev);
    let full = !revolve_has_caps(rev);
    let start = if rev.symmetric { -angle / 2.0 } else { 0.0 };
    let pitch = rev.pitch_mm;
    let mut mesh = SolidMesh::default();
    for face in &rev.faces {
        let (profile, _normal) = face_profile_world(doc, face)?;
        if profile.len() < 3 {
            return None;
        }
        let steps = revolve_mesh_steps(angle, pitch);
        let rings: Vec<Vec<Vec3>> = (0..=steps)
            .map(|i| {
                let a = start + angle * i as f32 / steps as f32;
                profile
                    .iter()
                    .map(|p| helical_point(origin, dir, pitch, a, *p))
                    .collect()
            })
            .collect();
        // Orientation reference: the *rotated profile centroid* at each sweep step — a
        // point locally inside the solid, which stays correct for washer-like profiles
        // that don't contain the axis (a single on-axis reference flips the inner wall).
        let centroid = profile.iter().copied().sum::<Vec3>() / profile.len() as f32;
        let centroids: Vec<Vec3> = (0..=steps)
            .map(|i| {
                let a = start + angle * i as f32 / steps as f32;
                helical_point(origin, dir, pitch, a, centroid)
            })
            .collect();
        let n = profile.len();
        for (i, w) in rings.windows(2).enumerate() {
            let (ra, rb) = (&w[0], &w[1]);
            let interior = (centroids[i] + centroids[i + 1]) * 0.5;
            for k in 0..n {
                let k1 = (k + 1) % n;
                push_oriented(&mut mesh.triangles, [ra[k], ra[k1], rb[k1]], interior);
                push_oriented(&mut mesh.triangles, [ra[k], rb[k1], rb[k]], interior);
            }
        }
        if !full {
            // Cap interiors sit half a step *into* the sweep so each cap faces outward.
            triangulate_cap(rings.first()?, centroids[0].lerp(centroids[1], 0.5), &mut mesh.triangles);
            triangulate_cap(
                rings.last()?,
                centroids[steps].lerp(centroids[steps - 1], 0.5),
                &mut mesh.triangles,
            );
        }
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// Real BREP solid of revolution via the kernel: each profile revolved with
/// `BRepPrimAPI_MakeRevol` (or a helical ThruSections when pitch is non-zero), multiple
/// profiles fused. `None` when any face/axis is degenerate or the kernel can't build it
/// (callers fall back to [`revolve_mesh`]).
pub fn occt_revolution_shape(
    doc: &Document,
    rev: &crate::model::Revolution,
) -> Option<crate::kernel::Shape> {
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let angle_rad = revolve_effective_angle(rev).to_radians() as f64;
    let pitch = rev.pitch_mm as f64;
    let mut fused: Option<crate::kernel::Shape> = None;
    for face in &rev.faces {
        let shape =
            occt_face_revolve_solid(doc, face, origin, dir, angle_rad, rev.symmetric, pitch)?;
        fused = Some(match fused {
            None => shape,
            Some(acc) => acc.boolean(&shape, crate::kernel::BoolOp::Fuse)?,
        });
    }
    fused
}

/// BREP solid for revolving a single face about an axis (#263), mirroring [`occt_face_solid`]:
/// a `Boolean` face revolves each operand and applies the same boolean to the swept solids, so a
/// concentric-ring (annulus) profile revolves into a hollow solid of revolution. Leaf faces
/// revolve their single boundary loop directly. Non-zero `pitch` makes a helix (#1242).
fn occt_face_revolve_solid(
    doc: &Document,
    face: &ExtrudeFace,
    origin: Vec3,
    dir: Vec3,
    angle_rad: f64,
    symmetric: bool,
    pitch: f64,
) -> Option<crate::kernel::Shape> {
    if let ExtrudeFace::Boolean { op, a, b } = face {
        let sa = occt_face_revolve_solid(doc, a, origin, dir, angle_rad, symmetric, pitch)?;
        let sb = occt_face_revolve_solid(doc, b, origin, dir, angle_rad, symmetric, pitch)?;
        let boolop = match op {
            crate::model::BooleanOp::Difference => crate::kernel::BoolOp::Cut,
            crate::model::BooleanOp::Intersection => crate::kernel::BoolOp::Common,
        };
        return sa.boolean(&sb, boolop);
    }
    let (profile, _normal) = face_profile_world(doc, face)?;
    if profile.len() < 3 {
        return None;
    }
    crate::kernel::Shape::revolve(&profile, origin, dir, angle_rad, symmetric, pitch)
}

/// The revolutions fusing into (`false`) or cutting (`true`) `body_index`.
pub fn revolutions_targeting(
    doc: &Document,
    body_index: crate::model::BodyKey,
) -> Vec<(crate::model::RevolutionKey, bool)> {
    doc.revolutions
        .iter()
        .filter_map(|(ri, r)| match &r.mode {
            crate::model::RevolveMode::AddTo(bodies) if bodies.contains(&body_index) => {
                Some((ri, false))
            }
            crate::model::RevolveMode::Cut(bodies) if bodies.contains(&body_index) => {
                Some((ri, true))
            }
            _ => None,
        })
        .collect()
}

/// Ordered world-space polyline of a sweep's picked path lines (#sweep): each
/// line is sampled bezier-aware, the segments are chained tip-to-tail regardless of pick
/// order, and the chain is oriented to start at the end nearer the profile plane. `None`
/// when a path line died, the segments don't form one connected chain, or the result
/// degenerates below two distinct points.
pub fn sweep_path_polyline(
    doc: &Document,
    fp: &crate::model::Sweep,
) -> Option<Vec<Vec3>> {
    /// Endpoint-matching tolerance (mm): path segments picked from a sketch chain share
    /// exact endpoints; the slack only absorbs float noise from the sketch solver.
    const TOL: f32 = 1e-2;
    let mut segs: Vec<Vec<Vec3>> = Vec::new();
    for &li in &fp.path {
        let line = doc.lines.get(li)?;
        if !crate::document_lifecycle::line_alive(doc, li) {
            return None;
        }
        let pts = crate::face::line_world_polyline(doc, line)?;
        if pts.len() >= 2 {
            segs.push(pts);
        }
    }
    if segs.is_empty() {
        return None;
    }
    let mut chain = segs.remove(0);
    while !segs.is_empty() {
        let head = *chain.first()?;
        let tail = *chain.last()?;
        let mut attached = false;
        for i in 0..segs.len() {
            let s_first = *segs[i].first()?;
            let s_last = *segs[i].last()?;
            if s_first.distance(tail) < TOL {
                chain.extend(segs.remove(i).into_iter().skip(1));
            } else if s_last.distance(tail) < TOL {
                let mut s = segs.remove(i);
                s.reverse();
                chain.extend(s.into_iter().skip(1));
            } else if s_last.distance(head) < TOL {
                let mut s = segs.remove(i);
                s.pop();
                s.extend(chain);
                chain = s;
            } else if s_first.distance(head) < TOL {
                let mut s = segs.remove(i);
                s.reverse();
                s.pop();
                s.extend(chain);
                chain = s;
            } else {
                continue;
            }
            attached = true;
            break;
        }
        if !attached {
            // A leftover segment touches neither chain end: the path isn't one chain.
            return None;
        }
    }
    // Sweep from the end nearer the profile plane, so the solid grows away from the faces.
    let (profile, normal) = fp.faces.first().and_then(|f| face_profile_world(doc, f))?;
    let p0 = *profile.first()?;
    if ((*chain.last()? - p0).dot(normal)).abs() < ((*chain.first()? - p0).dot(normal)).abs() {
        chain.reverse();
    }
    // Drop zero-length steps so every window has a real tangent.
    chain.dedup_by(|a, b| a.distance(*b) < 1e-5);
    (chain.len() >= 2).then_some(chain)
}

/// Per-point sweep frames along `path` (#sweep): parallel-transport rotations that
/// carry the profile plane onto each point's tangent without accumulating twist. The
/// first frame turns the profile normal (flipped to face along the path if needed) onto
/// the starting tangent; each following frame adds only the tangent-to-tangent turn.
fn sweep_path_frames(path: &[Vec3], profile_normal: Vec3) -> Vec<glam::Quat> {
    let n = path.len();
    let seg_dir = |i: usize| (path[i + 1] - path[i]).normalize_or_zero();
    let mut tangents: Vec<Vec3> = (0..n)
        .map(|i| {
            if i == 0 {
                seg_dir(0)
            } else if i == n - 1 {
                seg_dir(n - 2)
            } else {
                (seg_dir(i - 1) + seg_dir(i)).normalize_or_zero()
            }
        })
        .collect();
    for i in 0..n {
        // A doubled point or a hairpin corner averages to zero; coast on the neighbor.
        if tangents[i].length_squared() < 1e-8 {
            tangents[i] = if i > 0 { tangents[i - 1] } else { Vec3::Z };
        }
    }
    let n0 = if profile_normal.dot(tangents[0]) < 0.0 {
        -profile_normal
    } else {
        profile_normal
    };
    let mut q = glam::Quat::from_rotation_arc(n0.normalize_or_zero(), tangents[0]);
    let mut frames = Vec::with_capacity(n);
    frames.push(q);
    for i in 1..n {
        q = glam::Quat::from_rotation_arc(tangents[i - 1], tangents[i]) * q;
        frames.push(q);
    }
    frames
}

/// Hand-rolled sweep mesh for a sweep (#sweep) — the no-kernel fallback and
/// the live ghost preview: each profile ring is carried to every path point on
/// parallel-transport frames, walls are stitched between consecutive rings, and both end
/// profiles are capped.
pub fn sweep_mesh(doc: &Document, fp: &crate::model::Sweep) -> Option<SolidMesh> {
    let path = sweep_path_polyline(doc, fp)?;
    let anchor = path[0];
    let mut mesh = SolidMesh::default();
    for face in &fp.faces {
        let (profile, normal) = face_profile_world(doc, face)?;
        if profile.len() < 3 {
            return None;
        }
        let frames = sweep_path_frames(&path, normal);
        let rings: Vec<Vec<Vec3>> = path
            .iter()
            .zip(&frames)
            .map(|(&p, &q)| profile.iter().map(|&v| p + q * (v - anchor)).collect())
            .collect();
        // Orientation reference: the transported profile centroid — a point locally inside
        // the solid at every sweep step (same trick as [`revolve_mesh`]).
        let centroid = profile.iter().copied().sum::<Vec3>() / profile.len() as f32;
        let centroids: Vec<Vec3> = path
            .iter()
            .zip(&frames)
            .map(|(&p, &q)| p + q * (centroid - anchor))
            .collect();
        let n = profile.len();
        let steps = rings.len() - 1;
        for (i, w) in rings.windows(2).enumerate() {
            let (ra, rb) = (&w[0], &w[1]);
            let interior = (centroids[i] + centroids[i + 1]) * 0.5;
            for k in 0..n {
                let k1 = (k + 1) % n;
                push_oriented(&mut mesh.triangles, [ra[k], ra[k1], rb[k1]], interior);
                push_oriented(&mut mesh.triangles, [ra[k], rb[k1], rb[k]], interior);
            }
        }
        // Cap interiors sit half a step *into* the sweep so each cap faces outward.
        triangulate_cap(rings.first()?, centroids[0].lerp(centroids[1], 0.5), &mut mesh.triangles);
        triangulate_cap(
            rings.last()?,
            centroids[steps].lerp(centroids[steps - 1], 0.5),
            &mut mesh.triangles,
        );
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// Real BREP swept solid via the kernel (#sweep): each profile piped along the
/// path wire, multiple profiles fused. `None` when any face/path is degenerate or the
/// kernel can't build it (callers fall back to [`sweep_mesh`]).
pub fn occt_sweep_shape(
    doc: &Document,
    fp: &crate::model::Sweep,
) -> Option<crate::kernel::Shape> {
    let path = sweep_path_polyline(doc, fp)?;
    // A curved segment anywhere makes the whole spine a smooth spline; an all-straight
    // chain keeps its sharp corners.
    let smooth = fp
        .path
        .iter()
        .any(|&li| doc.lines.get(li).is_some_and(|l| l.bezier.is_some()));
    // A spline is fitted through the sample points with uniform parameterization, so
    // straight segments (2 samples) mixed with curved ones (25) would wiggle at the
    // density jump — resample evenly by arc length first.
    let path = if smooth { resample_polyline_by_arc_length(&path, 64) } else { path };
    let mut fused: Option<crate::kernel::Shape> = None;
    for face in &fp.faces {
        let shape = occt_face_sweep_solid(doc, face, &path, smooth)?;
        fused = Some(match fused {
            None => shape,
            Some(acc) => acc.boolean(&shape, crate::kernel::BoolOp::Fuse)?,
        });
    }
    fused
}

/// BREP solid for sweeping a single face along the path, mirroring [`occt_face_solid`]:
/// a `Boolean` face sweeps each operand and applies the same boolean to the swept solids,
/// so an annulus profile sweeps into a tube. Leaf faces sweep their boundary loop.
fn occt_face_sweep_solid(
    doc: &Document,
    face: &ExtrudeFace,
    path: &[Vec3],
    smooth: bool,
) -> Option<crate::kernel::Shape> {
    if let ExtrudeFace::Boolean { op, a, b } = face {
        let sa = occt_face_sweep_solid(doc, a, path, smooth)?;
        let sb = occt_face_sweep_solid(doc, b, path, smooth)?;
        let boolop = match op {
            crate::model::BooleanOp::Difference => crate::kernel::BoolOp::Cut,
            crate::model::BooleanOp::Intersection => crate::kernel::BoolOp::Common,
        };
        return sa.boolean(&sb, boolop);
    }
    let (profile, _normal) = face_profile_world(doc, face)?;
    if profile.len() < 3 {
        return None;
    }
    crate::kernel::Shape::sweep(&profile, path, smooth)
}

thread_local! {
    /// Single-slot memo for [`preview_sweep_cut_meshes`]: `(key, meshes)`. The draft only
    /// changes on a pick (no gizmo drag), so idle frames are free.
    static SWEEP_CUT_PREVIEW_CACHE: std::cell::RefCell<((u64, u64), Vec<(crate::model::BodyKey, SolidMesh)>)> =
        std::cell::RefCell::new(((0, 0), Vec::new()));
}

/// Cut-result meshes for the in-progress sweep cut preview: each target body of the
/// draft's `Cut` list meshed from a scratch document with the draft sweep appended, so the
/// preview shows the finished carve (mirroring the extrude cut preview, #142). Bodies the
/// scratch build can't mesh are simply absent. Cached per `(document, draft)` state.
pub fn preview_sweep_cut_meshes(
    doc: &Document,
    fp: &crate::model::Sweep,
) -> Vec<(crate::model::BodyKey, SolidMesh)> {
    let crate::model::SweepMode::Cut(bodies) = &fp.mode else {
        return Vec::new();
    };
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{fp:?}").hash(&mut h);
    let key = (document_mesh_fingerprint(doc), h.finish());
    SWEEP_CUT_PREVIEW_CACHE.with(|cache| {
        if cache.borrow().0 == key {
            return cache.borrow().1.clone();
        }
        let mut scratch = doc.clone();
        scratch.sweeps.insert(fp.clone());
        let meshes: Vec<(crate::model::BodyKey, SolidMesh)> = bodies
            .iter()
            .filter_map(|&bi| body_solid_mesh_uncached(&scratch, bi).map(|m| (bi, m)))
            .collect();
        *cache.borrow_mut() = (key, meshes.clone());
        meshes
    })
}

/// Resample a polyline to `n + 1` points evenly spaced along its arc length. Keeps the
/// endpoints exact; interior points interpolate on the original segments.
fn resample_polyline_by_arc_length(path: &[Vec3], n: usize) -> Vec<Vec3> {
    let total: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
    if total <= 1e-6 || path.len() < 2 {
        return path.to_vec();
    }
    let mut out = Vec::with_capacity(n + 1);
    out.push(path[0]);
    let mut seg = 0usize;
    let mut seg_start_len = 0.0f32;
    let mut seg_len = path[0].distance(path[1]);
    for i in 1..n {
        let target = total * i as f32 / n as f32;
        while seg_start_len + seg_len < target && seg + 2 < path.len() {
            seg_start_len += seg_len;
            seg += 1;
            seg_len = path[seg].distance(path[seg + 1]);
        }
        let t = if seg_len > 1e-9 { (target - seg_start_len) / seg_len } else { 0.0 };
        out.push(path[seg].lerp(path[seg + 1], t.clamp(0.0, 1.0)));
    }
    out.push(*path.last().unwrap());
    out
}

/// The sweeps fusing into (`false`) or cutting (`true`) `body_index`.
pub fn sweeps_targeting(
    doc: &Document,
    body_index: crate::model::BodyKey,
) -> Vec<(crate::model::SweepKey, bool)> {
    doc.sweeps
        .iter()
        .filter_map(|(fi, f)| match &f.mode {
            crate::model::SweepMode::AddTo(bodies) if bodies.contains(&body_index) => {
                Some((fi, false))
            }
            crate::model::SweepMode::Cut(bodies) if bodies.contains(&body_index) => {
                Some((fi, true))
            }
            _ => None,
        })
        .collect()
}

/// Ruled loft mesh through the given cross sections (in order): each section's boundary is
/// resampled to a common ring size, rings are aligned (consistent winding, twist-minimizing
/// start offset), consecutive rings are stitched with wall quads, and the end sections are
/// capped. A hand-rolled mesh like the no-kernel edge-treatment fallback — the OCCT
/// `ThruSections` surface loft is a documented follow-up.
pub fn loft_mesh(doc: &Document, loft: &crate::model::Loft) -> Option<SolidMesh> {
    let rings = loft_rings(doc, loft)?;
    let centroid = |ring: &Vec<Vec3>| ring.iter().copied().sum::<Vec3>() / ring.len() as f32;
    let interior = rings.iter().map(centroid).sum::<Vec3>() / rings.len() as f32;
    let mut triangles = Vec::new();
    for w in rings.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let n = a.len();
        for k in 0..n {
            let k1 = (k + 1) % n;
            push_oriented(&mut triangles, [a[k], a[k1], b[k1]], interior);
            push_oriented(&mut triangles, [a[k], b[k1], b[k]], interior);
        }
    }
    triangulate_cap(rings.first()?, interior, &mut triangles);
    triangulate_cap(rings.last()?, interior, &mut triangles);
    (!triangles.is_empty()).then_some(SolidMesh { triangles })
}

/// The loft's aligned cross-section rings: each section resampled to a common ring size,
/// wound consistently along the blend axis, and twist-minimized against its predecessor —
/// shared by the mesh and kernel paths.
fn loft_rings(doc: &Document, loft: &crate::model::Loft) -> Option<Vec<Vec<Vec3>>> {
    const RING: usize = CIRCLE_SEGMENTS;
    let mut rings: Vec<Vec<Vec3>> = Vec::new();
    for section in &loft.sections {
        let (profile, _normal) = face_profile_world(doc, &section.face)?;
        if profile.len() < 3 {
            return None;
        }
        rings.push(resample_loop(&profile, RING));
    }
    if rings.len() < 2 {
        return None;
    }

    // Consistent winding: orient every ring so its area normal points along the direction
    // to the next ring's centroid (the loft's local axis).
    let centroid = |ring: &Vec<Vec3>| ring.iter().copied().sum::<Vec3>() / ring.len() as f32;
    for i in 0..rings.len() {
        let c = centroid(&rings[i]);
        let axis = if i + 1 < rings.len() {
            centroid(&rings[i + 1]) - c
        } else {
            c - centroid(&rings[i - 1])
        };
        let normal: Vec3 = (0..rings[i].len())
            .map(|k| {
                let a = rings[i][k] - c;
                let b = rings[i][(k + 1) % rings[i].len()] - c;
                a.cross(b)
            })
            .sum();
        if normal.dot(axis) < 0.0 {
            rings[i].reverse();
        }
    }

    // Twist minimization: rotate each ring's start index to best match the previous ring.
    for i in 1..rings.len() {
        let prev = rings[i - 1].clone();
        let ring = &mut rings[i];
        let n = ring.len();
        let best = (0..n)
            .min_by(|&a, &b| {
                let cost = |offset: usize| -> f32 {
                    (0..n).map(|k| (ring[(k + offset) % n] - prev[k]).length_squared()).sum()
                };
                cost(a).partial_cmp(&cost(b)).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(0);
        ring.rotate_left(best);
    }

    Some(rings)
}

/// Real BREP loft via the kernel (#479): consecutive aligned rings become pairwise ruled
/// `ThruSections` solids, fused — geometrically the same ruled blend as [`loft_mesh`],
/// but a kernel shape that booleans can add/cut with. `None` when any section is
/// degenerate or the kernel can't build a segment (callers fall back to the mesh).
pub fn occt_loft_shape(
    doc: &Document,
    loft: &crate::model::Loft,
) -> Option<crate::kernel::Shape> {
    let rings = loft_rings(doc, loft)?;
    let mut fused: Option<crate::kernel::Shape> = None;
    for w in rings.windows(2) {
        let segment = crate::kernel::Shape::loft(&w[0], &w[1])?;
        fused = Some(match fused {
            None => segment,
            Some(acc) => acc.boolean(&segment, crate::kernel::BoolOp::Fuse)?,
        });
    }
    fused
}

/// The lofts fusing into (`false`) or cutting (`true`) `body_index` (#479).
pub fn lofts_targeting(
    doc: &Document,
    body_index: crate::model::BodyKey,
) -> Vec<(crate::model::LoftKey, bool)> {
    doc.lofts
        .iter()
        .filter_map(|(li, l)| match &l.mode {
            crate::model::LoftMode::AddTo(bodies) if bodies.contains(&body_index) => {
                Some((li, false))
            }
            crate::model::LoftMode::Cut(bodies) if bodies.contains(&body_index) => {
                Some((li, true))
            }
            _ => None,
        })
        .collect()
}

/// Resample a closed loop to exactly `count` points, evenly spaced by arc length.
fn resample_loop(points: &[Vec3], count: usize) -> Vec<Vec3> {
    let n = points.len();
    let mut lengths = Vec::with_capacity(n);
    let mut total = 0.0f32;
    for i in 0..n {
        let seg = (points[(i + 1) % n] - points[i]).length();
        lengths.push(seg);
        total += seg;
    }
    if total < 1e-9 {
        return vec![points[0]; count];
    }
    let mut out = Vec::with_capacity(count);
    let mut seg = 0usize;
    let mut seg_start = 0.0f32;
    for k in 0..count {
        let target = total * k as f32 / count as f32;
        while seg + 1 < n && seg_start + lengths[seg] < target {
            seg_start += lengths[seg];
            seg += 1;
        }
        let t = if lengths[seg] < 1e-9 {
            0.0
        } else {
            ((target - seg_start) / lengths[seg]).clamp(0.0, 1.0)
        };
        out.push(points[seg] + (points[(seg + 1) % n] - points[seg]) * t);
    }
    out
}

/// The loft cross sections the current selection resolves to (in blend order): a selected
/// circle is its own section; a selected line contributes the closed loop containing it.
/// Sections are ordered along the principal direction through their centroids so the blend
/// sequence matches the geometry, not the selection click order.
pub fn loft_sections_from_selection(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
) -> Vec<crate::model::LoftSection> {
    let mut sections: Vec<crate::model::LoftSection> = Vec::new();
    for element in selection.iter() {
        if let Some(section) = loft_section_from_element(doc, element) {
            if !sections.contains(&section) {
                sections.push(section);
            }
        }
    }
    order_loft_sections(doc, sections)
}

/// The loft cross section a picked scene element resolves to: a circle is its own
/// section; a line contributes the closed loop containing it. `None` for anything
/// else (construction geometry, open chains, non-sketch elements).
pub fn loft_section_from_element(
    doc: &Document,
    element: crate::hierarchy::SceneElement,
) -> Option<crate::model::LoftSection> {
    use crate::hierarchy::SceneElement;
    match element {
        SceneElement::Circle(ci) => {
            let circle = doc.circles.get(ci).filter(|c| !c.construction)?;
            Some(crate::model::LoftSection {
                sketch: circle.sketch,
                face: ExtrudeFace::Circle(ci),
            })
        }
        SceneElement::Line(li) => {
            let line = doc.lines.get(li).filter(|l| !l.construction)?;
            crate::polygon::closed_line_loops(doc, line.sketch)
                .into_iter()
                .find(|lines| lines.contains(&li))
                .map(|lines| crate::model::LoftSection {
                    sketch: line.sketch,
                    face: ExtrudeFace::Polygon(lines),
                })
        }
        _ => None,
    }
}

/// The sketch entities that make up a loft cross section, so a picked section can show its
/// selection highlight in the viewport (#202): a circle section is its circle, a line-loop
/// section is every line in the loop.
pub fn loft_section_scene_elements(
    section: &crate::model::LoftSection,
) -> Vec<crate::hierarchy::SceneElement> {
    extrude_face_scene_elements(&section.face)
}

/// The scene element a picked profile face **is** (#952) — its analytic-face identity, which
/// is what an element picker holds. Distinct from [`extrude_face_scene_elements`], which
/// returns the constituent geometry to *highlight*, not the face's identity.
///
/// A loft section needs no element of its own: it is a profile plus its sketch, and the sketch
/// follows from the profile, so this names it too.
pub fn extrude_face_scene_element(face: &ExtrudeFace) -> crate::hierarchy::SceneElement {
    crate::hierarchy::SceneElement::from_face_id(face.face_id())
}

/// The scene elements a picked profile face maps to, for folding a tool's picked faces into
/// the render selection so they highlight like selected geometry (#303): a circle face is its
/// circle, a polygon face is its boundary lines, a text glyph is its whole text.
pub fn extrude_face_scene_elements(
    face: &ExtrudeFace,
) -> Vec<crate::hierarchy::SceneElement> {
    use crate::hierarchy::SceneElement;
    match face {
        ExtrudeFace::Circle(ci) => vec![SceneElement::Circle(*ci)],
        ExtrudeFace::Polygon(lines) => lines.iter().map(|li| SceneElement::Line(*li)).collect(),
        ExtrudeFace::TextGlyph { text, .. } => vec![SceneElement::SketchText(*text)],
        // Neither a boolean combination nor a plane region (#993) is bounded by geometry the
        // sketch owns outright — a region runs partly along the host face's own outline — so
        // there is nothing to fold into the selection.
        ExtrudeFace::Boolean { .. } | ExtrudeFace::SketchRegion { .. } => Vec::new(),
    }
}

/// Order loft sections along the principal direction (the vector between the two
/// most-distant section centroids), so the loft blends through space monotonically
/// regardless of pick order.
pub fn order_loft_sections(
    doc: &Document,
    sections: Vec<crate::model::LoftSection>,
) -> Vec<crate::model::LoftSection> {
    let centroids: Vec<Option<Vec3>> = sections
        .iter()
        .map(|s| {
            face_profile_world(doc, &s.face)
                .map(|(p, _)| p.iter().copied().sum::<Vec3>() / p.len().max(1) as f32)
        })
        .collect();
    let mut axis = None;
    let mut best = 0.0f32;
    for i in 0..centroids.len() {
        for j in (i + 1)..centroids.len() {
            if let (Some(a), Some(b)) = (centroids[i], centroids[j]) {
                let d = (b - a).length_squared();
                if d > best {
                    best = d;
                    axis = Some((a, (b - a).normalize_or_zero()));
                }
            }
        }
    }
    if let Some((origin, dir)) = axis {
        let mut keyed: Vec<(f32, crate::model::LoftSection)> = sections
            .into_iter()
            .zip(centroids)
            .map(|(s, c)| (c.map(|c| (c - origin).dot(dir)).unwrap_or(0.0), s))
            .collect();
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        keyed.into_iter().map(|(_, s)| s).collect()
    } else {
        sections
    }
}

/// Resolve a `SceneElement::BodyFace`'s quantized centroid+normal key back to the coplanar
/// triangle group it names on the body's current solid mesh (#555). `None` when a rebuild
/// has moved the face so no group matches the key anymore.
pub fn body_face_triangles(
    doc: &Document,
    body: crate::model::BodyKey,
    centroid: [i32; 3],
    normal: [i32; 3],
) -> Option<Vec<[Vec3; 3]>> {
    let solid = body_solid_mesh_for_face_key(doc, body)?;
    face_group_matching(&solid, centroid, normal)
}

/// Mesh used to resolve a [`FaceId::BodyMeshFace`] key. Post-op add/cut extrusions
/// (#1168/#1338) change the tessellation (and would recurse if a cut's own sketch sits
/// on this body), so face keys are matched against the base solid they were captured on.
fn body_solid_mesh_for_face_key(
    doc: &Document,
    body: crate::model::BodyKey,
) -> Option<SolidMesh> {
    let src = &doc.bodies.get(body)?.source;
    let base = match src {
        crate::model::BodySource::Boolean {
            op,
            solid,
            add,
            cut,
        } if !add.is_empty() || !cut.is_empty() => occt_boolean_output_shape(doc, *op, *solid),
        crate::model::BodySource::Shelled {
            op,
            target,
            add,
            cut,
        } if !add.is_empty() || !cut.is_empty() => occt_shelled_output_shape(doc, *op, *target),
        crate::model::BodySource::Moved {
            op,
            target,
            add,
            cut,
        } if !add.is_empty() || !cut.is_empty() => occt_moved_output_shape(doc, *op, *target),
        crate::model::BodySource::Mirrored {
            op,
            target,
            add,
            cut,
        } if !add.is_empty() || !cut.is_empty() => occt_mirrored_output_shape(doc, *op, *target),
        crate::model::BodySource::Repeated {
            op,
            target,
            instance,
            add,
            cut,
        } if !add.is_empty() || !cut.is_empty() => {
            occt_repeated_output_shape(doc, *op, *target, *instance)
        }
        crate::model::BodySource::Sliced {
            op,
            target,
            piece,
            add,
            cut,
        } if !add.is_empty() || !cut.is_empty() => {
            occt_sliced_output_shape(doc, *op, *target, *piece)
        }
        crate::model::BodySource::EdgeTreated {
            op,
            target,
            add,
            cut,
        } if !add.is_empty() || !cut.is_empty() => occt_edge_treated_output_shape(doc, *op, *target),
        _ => return body_solid_mesh(doc, body),
    };
    let tris = base?.tessellate(OCCT_DEFLECTION as f64);
    (!tris.is_empty()).then_some(SolidMesh { triangles: tris })
}

/// The coplanar-triangle group of `solid` whose quantized centroid+normal match the key —
/// the matching half of [`body_face_triangles`], callable on an already-obtained mesh (so
/// resolution inside the mesh cache's borrow can use the uncached mesher, #738).
pub fn face_group_matching(
    solid: &SolidMesh,
    centroid: [i32; 3],
    normal: [i32; 3],
) -> Option<Vec<[Vec3; 3]>> {
    let q = crate::hierarchy::quantize_body_point;
    crate::gpu_viewport::solid_mesh_coplanar_faces(solid)
        .into_iter()
        .find(|tris| {
            let count = (tris.len() * 3).max(1) as f32;
            let c = tris.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
            let n = (tris[0][1] - tris[0][0])
                .cross(tris[0][2] - tris[0][0])
                .normalize_or_zero();
            q(c) == centroid && q(n) == normal
        })
}

/// A face group's centre — the average of its triangle vertices, the same formula its
/// quantized selection key stores (#555/#738).
pub fn face_group_center(tris: &[[Vec3; 3]]) -> Vec3 {
    let count = (tris.len() * 3).max(1) as f32;
    tris.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count
}

/// A face group's **area centroid** (#1080): the area-weighted mean of its triangles' own
/// centroids, which is the geometric centre of the region whatever way it was triangulated.
///
/// [`face_group_center`] averages triangle *vertices* instead, so a vertex shared by several
/// triangles is counted several times and the answer drifts with the mesh — on a square plate
/// with a central hole, a couple of tenths of a millimetre off the true middle. That average
/// stays the face's **key** (it is what every stored `BodyFace`/`OnFace` reference is matched
/// by, and changing it would invalidate them), but nothing that has to be *accurate* should
/// use it. Mating does.
pub fn face_group_area_centroid(tris: &[[Vec3; 3]]) -> Vec3 {
    let mut weighted = Vec3::ZERO;
    let mut total = 0.0f32;
    for t in tris {
        let area = (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5;
        weighted += (t[0] + t[1] + t[2]) / 3.0 * area;
        total += area;
    }
    if total <= f32::EPSILON {
        return face_group_center(tris);
    }
    weighted / total
}

/// The stored offset for "the middle of this face" (#1080): the **area** centroid expressed
/// in the face's own axes, relative to the key's vertex-average centroid. `[0, 0]` when the
/// face doesn't resolve, which is the same fallback every other face lookup takes.
///
/// Naming a face without naming a point on it — what a script's `face = {}` does, and what a
/// pane pick does before its point lands — means its middle, and the middle has to be the
/// accurate one or seating a peg in a hole comes out visibly off-centre.
pub fn face_middle_uv(
    doc: &Document,
    body: crate::model::BodyKey,
    centroid: [i32; 3],
    normal: [i32; 3],
) -> [i32; 2] {
    let Some(tris) = body_face_triangles(doc, body, centroid, normal) else {
        return [0, 0];
    };
    face_world_uv(&tris, face_group_area_centroid(&tris))
}

/// A face group's own in-plane axes (#1074): [`crate::construction::plane_basis`] of its
/// normal, so the frame is the same one a sketch on that face would get and a point keeps
/// meaning the same thing across rebuilds.
pub fn face_group_basis(tris: &[[Vec3; 3]]) -> (Vec3, Vec3) {
    let n = (tris[0][1] - tris[0][0])
        .cross(tris[0][2] - tris[0][0])
        .normalize_or_zero();
    crate::construction::plane_basis(n)
}

/// The world point at a quantized in-plane offset from a face group's centre (#1074).
pub fn face_uv_world(tris: &[[Vec3; 3]], uv: [i32; 2]) -> Vec3 {
    let (u, v) = face_group_basis(tris);
    face_group_center(tris) + u * (uv[0] as f32 / 100.0) + v * (uv[1] as f32 / 100.0)
}

/// The points a Face Snap mate can land on within a face (#1083): its **corners**, the
/// **midpoint of each boundary edge**, and the face's **centre**. A rectangular face
/// therefore offers the nine points a user reaches for; a round one offers its rim's
/// vertices, their midpoints, and the centre.
///
/// Collinear boundary vertices — a mesh may split a straight edge into several — are dropped,
/// so a rectangle's edge gives one midpoint rather than one per triangle it happens to span.
pub fn face_snap_points(tris: &[[Vec3; 3]]) -> Vec<Vec3> {
    let boundary = crate::construction::coplanar_face_boundary(tris);
    if boundary.is_empty() {
        return vec![face_group_center(tris)];
    }
    // Chain the boundary segments into a loop so "corner" means a real turn, not a mesh seam.
    let mut loops: Vec<Vec3> = Vec::new();
    let mut remaining = boundary.clone();
    let mut current = remaining.pop().map(|(a, b)| {
        loops.push(a);
        b
    });
    while let Some(at) = current {
        loops.push(at);
        let next = remaining
            .iter()
            .position(|(a, b)| (*a - at).length() < 1e-3 || (*b - at).length() < 1e-3)
            .map(|i| {
                let (a, b) = remaining.swap_remove(i);
                if (a - at).length() < 1e-3 { b } else { a }
            });
        match next {
            Some(p) if (p - loops[0]).length() > 1e-3 => current = Some(p),
            _ => break,
        }
    }
    // Drop points that merely continue a straight run: only real corners count.
    let n = loops.len();
    let corners: Vec<Vec3> = (0..n)
        .filter(|&i| {
            let prev = loops[(i + n - 1) % n];
            let next = loops[(i + 1) % n];
            let a = (loops[i] - prev).normalize_or_zero();
            let b = (next - loops[i]).normalize_or_zero();
            a.cross(b).length() > 1e-3
        })
        .map(|i| loops[i])
        .collect();
    let corners = if corners.len() >= 3 { corners } else { loops };

    let mut points = corners.clone();
    for i in 0..corners.len() {
        points.push((corners[i] + corners[(i + 1) % corners.len()]) * 0.5);
    }
    // The **accurate** centre (#1080), not the vertex average the key is built from: seating
    // a peg in a hole is the commonest thing this point is for, and it has to land dead on.
    points.push(face_group_area_centroid(tris));
    points
}

/// Whether a world point lies **on** a face group (#1075): in its plane, and inside one of
/// its triangles. The tolerance is the same 0.01 mm the quantized body-point keys round to,
/// so a point derived from a mesh vertex of this face always counts as on it.
pub fn face_group_contains(tris: &[[Vec3; 3]], world: Vec3) -> bool {
    let (u, v) = face_group_basis(tris);
    let n = u.cross(v).normalize_or_zero();
    let origin = face_group_center(tris);
    if (world - origin).dot(n).abs() > 0.01 {
        return false;
    }
    let flat = |p: Vec3| ((p - origin).dot(u), (p - origin).dot(v));
    let p = flat(world);
    tris.iter()
        .any(|t| crate::polygon::point_in_triangle_2d(p, flat(t[0]), flat(t[1]), flat(t[2])))
}

/// The quantized in-plane offset of a world point from a face group's centre (#1074) — the
/// inverse of [`face_uv_world`], for turning a pick into a stored reference.
pub fn face_world_uv(tris: &[[Vec3; 3]], world: Vec3) -> [i32; 2] {
    let (u, v) = face_group_basis(tris);
    let d = world - face_group_center(tris);
    [(d.dot(u) * 100.0).round() as i32, (d.dot(v) * 100.0).round() as i32]
}

/// World bounds of the current selection (#164):/// World bounds of the current selection (#164): union of every selected element's own
/// geometry (a body's solid, an extrusion's solid, a line/circle's sampled points, a point's
/// position). `None` when nothing in the selection has world extent (then zoom-to-fit falls
/// back to the whole document).
pub fn selection_world_bounds(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
) -> Option<(Vec3, Vec3)> {
    use crate::hierarchy::SceneElement;
    let mut bounds: Option<(Vec3, Vec3)> = None;
    let mut extend = |p: Vec3| {
        bounds = Some(match bounds {
            Some((min, max)) => (min.min(p), max.max(p)),
            None => (p, p),
        });
    };
    for element in selection.iter() {
        match element {
            // A drawing item is on a page, not in the model, so it contributes no bounds.
            SceneElement::DrawingElement { .. } => {}
            SceneElement::RepeatOp(op) => {
                let outputs = doc
                    .repeat_ops
                    .get(op)
                    .map(|o| o.outputs.clone())
                    .unwrap_or_default();
                for bi in outputs {
                    if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                        extend(min);
                        extend(max);
                    }
                }
            }
            SceneElement::MoveOp(op) => {
                let outputs = doc
                    .move_ops
                    .get(op)
                    .map(|o| o.outputs.clone())
                    .unwrap_or_default();
                for bi in outputs {
                    if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                        extend(min);
                        extend(max);
                    }
                }
            }
            SceneElement::MirrorOp(op) => {
                let outputs = doc
                    .mirror_ops
                    .get(op)
                    .map(|o| o.outputs.clone())
                    .unwrap_or_default();
                for bi in outputs {
                    if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                        extend(min);
                        extend(max);
                    }
                }
            }
            SceneElement::BooleanOp(op) => {
                let outputs = doc
                    .boolean_ops
                    .get(op)
                    .map(|o| o.outputs.clone())
                    .unwrap_or_default();
                for bi in outputs {
                    if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                        extend(min);
                        extend(max);
                    }
                }
            }
            SceneElement::SliceOp(op) => {
                let outputs = doc
                    .slice_ops
                    .get(op)
                    .map(|o| o.outputs.clone())
                    .unwrap_or_default();
                for bi in outputs {
                    if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                        extend(min);
                        extend(max);
                    }
                }
            }
            SceneElement::ShellOp(op) => {
                let outputs = doc
                    .shell_ops
                    .get(op)
                    .map(|o| o.outputs.clone())
                    .unwrap_or_default();
                for bi in outputs {
                    if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                        extend(min);
                        extend(max);
                    }
                }
            }
            SceneElement::EdgeTreatmentOp(op) => {
                let outputs = doc
                    .edge_treatment_ops
                    .get(op)
                    .map(|o| o.outputs.clone())
                    .unwrap_or_default();
                for bi in outputs {
                    if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                        extend(min);
                        extend(max);
                    }
                }
            }
            SceneElement::Revolution(op) => {
                // The revolved solid's body is linked by `BodySource::Revolve` (NewBody mode).
                for bi in doc.bodies.keys().collect::<Vec<_>>() {
                    if doc.bodies[bi].source == crate::model::BodySource::Revolve(op) {
                        if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds())
                        {
                            extend(min);
                            extend(max);
                        }
                    }
                }
            }
            SceneElement::Shape(op) => {
                // A shape's body is linked by `BodySource::Primitive` or a Solid whose base
                // is that shape after an add-to-body / cut (#909/#1104).
                for bi in doc.bodies.keys().collect::<Vec<_>>() {
                    if doc.bodies[bi].source.primitive_base() == Some(op) {
                        if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds())
                        {
                            extend(min);
                            extend(max);
                        }
                    }
                }
            }
            SceneElement::SweepOp(op) => {
                // The swept solid's body is linked by `BodySource::Sweep` (NewBody mode).
                for bi in doc.bodies.keys().collect::<Vec<_>>() {
                    if doc.bodies[bi].source == crate::model::BodySource::Sweep(op) {
                        if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds())
                        {
                            extend(min);
                            extend(max);
                        }
                    }
                }
            }
            SceneElement::Body(bi) => {
                if let Some((min, max)) = body_solid_mesh(doc, bi).and_then(|m| m.bounds()) {
                    extend(min);
                    extend(max);
                }
            }
            SceneElement::Extrusion(ei) => {
                if let Some((min, max)) = doc
                    .extrusions
                    .get(ei)
                    .and_then(|e| extrusion_mesh(doc, e))
                    .and_then(|m| m.bounds())
                {
                    extend(min);
                    extend(max);
                }
            }
            SceneElement::Line(li) => {
                if let Some((line, frame)) = doc
                    .lines
                    .get(li)
                    
                    .and_then(|l| Some((l, sketch_geometry_frame(doc, l.sketch)?)))
                {
                    for (u, v) in line.sample_local(crate::model::BEZIER_SEGMENTS) {
                        extend(local_to_world(&frame, u, v));
                    }
                }
            }
            SceneElement::Circle(ci) => {
                if let Some((circle, frame)) = doc
                    .circles
                    .get(ci)
                    .and_then(|c| Some((c, sketch_geometry_frame(doc, c.sketch)?)))
                {
                    for i in 0..CIRCLE_SEGMENTS {
                        let a = i as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                        extend(local_to_world(
                            &frame,
                            circle.cx + circle.r * a.cos(),
                            circle.cy + circle.r * a.sin(),
                        ));
                    }
                }
            }
            SceneElement::Point(point) => {
                if let Some(p) = crate::construction::point_world_position(doc, point) {
                    extend(p);
                }
            }
            SceneElement::BodyEdge { a, b, .. } => {
                extend(crate::hierarchy::dequantize_body_point(a));
                extend(crate::hierarchy::dequantize_body_point(b));
            }
            SceneElement::BodyVertex { p, .. } => {
                extend(crate::hierarchy::dequantize_body_point(p));
            }
            // A body face (#555): the whole coplanar triangle group, so framing the selection
            // (and auto-zoom's selection watch) takes in the entire face — with the centroid as
            // a fallback should a rebuild have moved the face out from under its key.
            SceneElement::BodyFace { body, centroid, normal } => {
                match body_face_triangles(doc, body, centroid, normal) {
                    Some(triangles) => {
                        for tri in &triangles {
                            for p in tri {
                                extend(*p);
                            }
                        }
                    }
                    None => extend(crate::hierarchy::dequantize_body_point(centroid)),
                }
            }
            // A cylinder frames its whole round wall; its centre line, the axis segment
            // that wall spans (#1013).
            SceneElement::BodyCylinder { body, origin, dir, radius } => {
                match body_cylinder_matching(doc, body, origin, dir, radius) {
                    Some(cyl) => {
                        for tri in &cyl.triangles {
                            for p in tri {
                                extend(*p);
                            }
                        }
                    }
                    None => extend(crate::hierarchy::dequantize_body_point(origin)),
                }
            }
            SceneElement::BodyAxis { body, origin, dir } => {
                match body_axis_segment(doc, body, origin, dir) {
                    Some((a, b)) => {
                        extend(a);
                        extend(b);
                    }
                    None => extend(crate::hierarchy::dequantize_body_point(origin)),
                }
            }
            // A unit instance frames its placed evaluated meshes (#723).
            SceneElement::UnitInstance(index) => {
                for solid in crate::units::placed_instance_meshes(doc, index) {
                    if let Some((min, max)) = solid.bounds() {
                        extend(min);
                        extend(max);
                    }
                }
            }
            // A joint frames the parts it joins (#891).
            SceneElement::Joint(ji) => {
                if let Some(joint) = doc.joints.get(ji) {
                    for member in &joint.members {
                        match *member {
                            crate::model::JointRef::Body(bi) => {
                                if let Some((min, max)) =
                                    body_solid_mesh(doc, bi).and_then(|m| m.bounds())
                                {
                                    extend(min);
                                    extend(max);
                                }
                            }
                            crate::model::JointRef::UnitInstance(ui) => {
                                for solid in crate::units::placed_instance_meshes(doc, ui) {
                                    if let Some((min, max)) = solid.bounds() {
                                        extend(min);
                                        extend(max);
                                    }
                                }
                            }
                            crate::model::JointRef::Component(_) => {}
                        }
                    }
                }
            }
            SceneElement::Sketch(_)
            | SceneElement::ConstructionPlane(_)
            | SceneElement::Constraint(_)
            | SceneElement::FaceEdge(_)
            | SceneElement::Origin
            | SceneElement::GlobalAxis(_)
            // An analytic face's bounds come from the geometry it is defined against, which is
            // selected and framed as its own element.
            | SceneElement::SketchFace(_)
            // A snap point's bounds are a single point on a body that frames itself.
            | SceneElement::MovePoint(_)
            // An analytic edge's bounds come from the extrusion that owns it.
            | SceneElement::ExtrusionEdge { .. }
            | SceneElement::PrimitiveEdge { .. }
            // A repeat instance's face is framed by the repeat that produced it.
            | SceneElement::RepeatedFace { .. }
            // The in-sketch repeat's own bounds come from its duplicated lines/circles, which are
            // selected/framed as their own elements; the op node itself contributes nothing here.
            | SceneElement::SketchRepeatOp(_)
            | SceneElement::SketchOffsetOp(_)
            | SceneElement::SketchMirrorOp(_)
            | SceneElement::SketchVertexTreatmentOp(_)
            | SceneElement::SketchSliceOp(_)
            | SceneElement::SketchText(_)
            | SceneElement::Component(_)
            | SceneElement::Image(_) => {}
        }
    }
    bounds
}

/// Fingerprint of every document input body meshing reads (#162/#1027).
///
/// An integer on the document, bumped by [`Document::bump_mesh_rev`] whenever an
/// `AppState::apply` changes geometry. Cache probes are an integer compare — not a full
/// document JSON serialize.
///
/// A freshly loaded document has `mesh_rev == 0` until the first edit; open/load set it to
/// a non-zero baseline so idle frames after open stay cheap. Tests that build documents
/// without going through `apply` leave it at 0 and fall back to the structural hash so
/// their direct field writes still invalidate caches.
pub(crate) fn document_mesh_fingerprint(doc: &Document) -> u64 {
    if doc.mesh_rev == 0 {
        return structural_mesh_fingerprint(doc);
    }
    doc.mesh_rev
}

/// Structural hash of geometry — the path this counter replaced. Only used while
/// `mesh_rev` is still 0 (test fixtures that never go through `apply`).
fn structural_mesh_fingerprint(doc: &Document) -> u64 {
    use std::hash::Hasher;
    struct HashWriter(std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(std::collections::hash_map::DefaultHasher::new());
    serde_json::to_writer(
        &mut writer,
        &(
            &doc.lines,
            &doc.circles,
            &doc.sketches,
            &doc.construction_planes,
            &doc.extrusions,
            &doc.bodies,
            &doc.parameters,
            &doc.repeat_ops,
            &doc.move_ops,
            &doc.slice_ops,
            &doc.boolean_ops,
            &doc.revolutions,
            &doc.sweeps,
            &doc.lofts,
            &doc.units,
            &doc.unit_instances,
        ),
    )
    .ok();
    for mesh in doc.imported_meshes.values() {
        std::io::Write::write_all(&mut writer, mesh.source_name.as_bytes()).ok();
        writer.0.write_usize(mesh.triangles.len());
        writer.0.write_usize(mesh.step_bytes.as_ref().map(|b| b.len()).unwrap_or(0));
    }
    writer.0.finish()
}

/// Probe counts for persisted mesh cache (#1343). `warmed` is how many bodies
/// [`warm_persisted_meshes`] seeded; `hits`/`misses` count [`body_solid_mesh_unposed`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshCacheStats {
    pub warmed: u32,
    pub hits: u32,
    pub misses: u32,
}

/// Last committed un-posed mesh for a body, keyed by that body's structural
/// fingerprint (includes OCCT version). Incremental writes persist these.
#[derive(Clone, Debug)]
pub struct CommittedMesh {
    pub body: crate::model::BodyKey,
    pub fingerprint: u64,
    pub mesh: SolidMesh,
}

thread_local! {
    static MESH_CACHE_STATS: std::cell::RefCell<MeshCacheStats> =
        std::cell::RefCell::new(MeshCacheStats::default());
    static COMMITTED_MESHES: std::cell::RefCell<HashMap<crate::model::BodyKey, CommittedMesh>> =
        std::cell::RefCell::new(HashMap::new());
    /// Per-thread memo for [`body_solid_mesh`] (#162): `(document fingerprint, body → mesh)`.
    /// The kernel rebuild is expensive (an extrude-to-slanted-plane does OCCT booleans), and
    /// one frame calls `body_solid_mesh` several times per body (scene build, hover picking,
    /// occlusion, the selection aura) — without this the viewer visibly slows down. Any
    /// change to the fingerprinted geometry clears the memo.
    static BODY_MESH_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, Option<SolidMesh>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
    /// Per-thread memo for [`occt_body_shape`] (#1337): the BREP history (shell, slice,
    /// combine, …) is far more expensive than tessellation. Cut previews clone the cached
    /// solid and subtract the tool instead of rebuilding the part every frame.
    ///
    /// Keyed by `(mesh fingerprint, structural extra)` so a trial `doc.clone()` that
    /// keeps `mesh_rev` but rewrites geometry (feasibility checks) cannot hit the
    /// pre-edit solid.
    static BODY_SHAPE_CACHE: std::cell::RefCell<(
        u64,
        u64,
        HashMap<crate::model::BodyKey, Option<crate::kernel::Shape>>,
    )> = std::cell::RefCell::new((0, 0, HashMap::new()));
}

/// OCCT version mixed into [`body_cache_fingerprint`] and stored on each cache row.
pub fn cache_occt_version() -> String {
    crate::kernel::occt_version().unwrap_or_else(|| "none".to_string())
}

/// Per-body cache key: this body's source tree + parameters + OCCT version.
/// Independent of `mesh_rev`, so a saved fingerprint still matches after open.
pub fn body_cache_fingerprint(doc: &Document, body: crate::model::BodyKey) -> u64 {
    use std::hash::{Hash, Hasher};
    struct HashWriter(std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(std::collections::hash_map::DefaultHasher::new());
    cache_occt_version().hash(&mut writer.0);
    serde_json::to_writer(&mut writer, &doc.parameters).ok();
    let mut seen = std::collections::HashSet::new();
    hash_body_deps(doc, body, &mut writer, &mut seen);
    writer.0.finish()
}

fn hash_json(writer: &mut impl std::io::Write, v: &impl serde::Serialize) {
    serde_json::to_writer(writer, v).ok();
}

fn hash_body_deps(
    doc: &Document,
    body: crate::model::BodyKey,
    writer: &mut impl std::io::Write,
    seen: &mut std::collections::HashSet<crate::model::BodyKey>,
) {
    if !seen.insert(body) {
        return;
    }
    let Some(b) = doc.bodies.get(body) else {
        return;
    };
    hash_json(writer, b);
    match &b.source {
        crate::model::BodySource::Extrusion(k) => hash_extrusion(doc, *k, writer),
        crate::model::BodySource::Extrusions(ks) => {
            for k in ks {
                hash_extrusion(doc, *k, writer);
            }
        }
        crate::model::BodySource::Imported(k) => {
            if let Some(m) = doc.imported_meshes.get(*k) {
                hash_json(writer, &m.source_name);
                let _ = std::io::Write::write(
                    writer,
                    &(m.triangles.len() as u64).to_le_bytes(),
                );
                let _ = std::io::Write::write(
                    writer,
                    &(m.step_bytes.as_ref().map(|s| s.len()).unwrap_or(0) as u64)
                        .to_le_bytes(),
                );
            }
        }
        crate::model::BodySource::Loft(k) => {
            if let Some(v) = doc.lofts.get(*k) {
                hash_json(writer, v);
                for section in &v.sections {
                    hash_sketch(doc, section.sketch, writer);
                }
            }
        }
        crate::model::BodySource::Revolve(k) => {
            if let Some(v) = doc.revolutions.get(*k) {
                hash_json(writer, v);
                hash_sketch(doc, v.sketch, writer);
            }
        }
        crate::model::BodySource::Primitive(k) => {
            if let Some(v) = doc.primitives.get(*k) {
                hash_json(writer, v);
            }
        }
        crate::model::BodySource::Sweep(k) => {
            if let Some(v) = doc.sweeps.get(*k) {
                hash_json(writer, v);
                hash_sketch(doc, v.sketch, writer);
            }
        }
        crate::model::BodySource::Boolean { op, .. } => {
            if let Some(v) = doc.boolean_ops.get(*op) {
                hash_json(writer, v);
                for &inp in v.a.iter().chain(v.b.iter()) {
                    hash_body_deps(doc, inp, writer, seen);
                }
            }
        }
        crate::model::BodySource::Repeated { op, .. } => {
            if let Some(v) = doc.repeat_ops.get(*op) {
                hash_json(writer, v);
                for &t in &v.targets {
                    hash_body_deps(doc, t, writer, seen);
                }
            }
        }
        crate::model::BodySource::Moved { op, .. } => {
            if let Some(v) = doc.move_ops.get(*op) {
                hash_json(writer, v);
                for &t in &v.targets {
                    hash_body_deps(doc, t, writer, seen);
                }
            }
        }
        crate::model::BodySource::Mirrored { op, .. } => {
            if let Some(v) = doc.mirror_ops.get(*op) {
                hash_json(writer, v);
                for &t in &v.targets {
                    hash_body_deps(doc, t, writer, seen);
                }
            }
        }
        crate::model::BodySource::Sliced { op, .. } => {
            if let Some(v) = doc.slice_ops.get(*op) {
                hash_json(writer, v);
                for &t in &v.targets {
                    hash_body_deps(doc, t, writer, seen);
                }
            }
        }
        crate::model::BodySource::Shelled { op, .. } => {
            if let Some(v) = doc.shell_ops.get(*op) {
                hash_json(writer, v);
                for &t in &v.targets {
                    hash_body_deps(doc, t, writer, seen);
                }
            }
        }
        crate::model::BodySource::EdgeTreated { op, .. } => {
            if let Some(v) = doc.edge_treatment_ops.get(*op) {
                hash_json(writer, v);
                for &t in &v.targets {
                    hash_body_deps(doc, t, writer, seen);
                }
            }
        }
        crate::model::BodySource::Solid { base, add, cut } => {
            if let Some(pk) = base {
                if let Some(v) = doc.primitives.get(*pk) {
                    hash_json(writer, v);
                }
            }
            for k in add.iter().chain(cut.iter()) {
                hash_extrusion(doc, *k, writer);
            }
        }
        crate::model::BodySource::UnitInstance(k) | crate::model::BodySource::UnitCut { instance: k, .. } => {
            if let Some(inst) = doc.unit_instances.get(*k) {
                hash_json(writer, inst);
                if let Some(unit) = doc.units.get(inst.unit) {
                    hash_json(writer, unit);
                }
            }
        }
    }
    for k in b.source.extrusion_indices() {
        hash_extrusion(doc, *k, writer);
    }
    for k in b.source.cut_extrusion_indices() {
        hash_extrusion(doc, *k, writer);
    }
}

fn hash_extrusion(
    doc: &Document,
    key: crate::model::ExtrusionKey,
    writer: &mut impl std::io::Write,
) {
    let Some(ex) = doc.extrusions.get(key) else {
        return;
    };
    hash_json(writer, ex);
    hash_sketch(doc, ex.sketch, writer);
}

fn hash_sketch(doc: &Document, sketch: crate::model::SketchId, writer: &mut impl std::io::Write) {
    if let Some(s) = doc.sketches.get(sketch) {
        hash_json(writer, s);
        if let crate::model::FaceId::ConstructionPlane(p) = s.face {
            if let Some(plane) = doc.construction_planes.get(p) {
                hash_json(writer, plane);
            }
        }
    }
    for (k, line) in doc.lines.iter() {
        if line.sketch == sketch {
            hash_json(writer, &(k, line));
        }
    }
    for (k, c) in doc.circles.iter() {
        if c.sketch == sketch {
            hash_json(writer, &(k, c));
        }
    }
    for (k, c) in doc.constraints.iter() {
        if c.sketch == sketch {
            hash_json(writer, &(k, c));
        }
    }
    for (k, t) in doc.sketch_texts.iter() {
        if t.sketch == sketch {
            hash_json(writer, &(k, t));
        }
    }
}

pub fn mesh_cache_stats() -> MeshCacheStats {
    MESH_CACHE_STATS.with(|s| *s.borrow())
}

pub fn reset_mesh_cache_stats() {
    MESH_CACHE_STATS.with(|s| *s.borrow_mut() = MeshCacheStats::default());
}

pub fn committed_meshes() -> Vec<CommittedMesh> {
    COMMITTED_MESHES.with(|m| m.borrow().values().cloned().collect())
}

pub fn clear_committed_meshes() {
    COMMITTED_MESHES.with(|m| m.borrow_mut().clear());
}

/// Drop in-memory tessellation / BREP memos and the committed-mesh store (#1343).
pub fn clear_all_mesh_caches() {
    BODY_MESH_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.0 = 0;
        c.1.clear();
    });
    BODY_SHAPE_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.0 = 0;
        c.1 = 0;
        c.2.clear();
    });
    clear_committed_meshes();
}

fn record_committed_mesh(doc: &Document, body: crate::model::BodyKey, mesh: &SolidMesh) {
    if mesh.triangles.is_empty() {
        return;
    }
    let fingerprint = body_cache_fingerprint(doc, body);
    COMMITTED_MESHES.with(|m| {
        m.borrow_mut().insert(
            body,
            CommittedMesh {
                body,
                fingerprint,
                mesh: mesh.clone(),
            },
        );
    });
}

/// Seed [`BODY_MESH_CACHE`] and the committed-mesh store from persisted rows
/// whose fingerprint still matches. Misses stay out so that body rebuilds alone.
pub fn warm_persisted_meshes(
    doc: &Document,
    rows: &[(crate::model::BodyKey, u64, SolidMesh)],
) -> u32 {
    let fingerprint = document_mesh_fingerprint(doc);
    let mut warmed = 0u32;
    BODY_MESH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        for (body, fp, mesh) in rows {
            if *fp != body_cache_fingerprint(doc, *body) {
                continue;
            }
            cache.1.insert(*body, Some(mesh.clone()));
            record_committed_mesh(doc, *body, mesh);
            warmed += 1;
        }
    });
    MESH_CACHE_STATS.with(|s| s.borrow_mut().warmed += warmed);
    warmed
}

/// Everything the **posed** presentation depends on beyond the un-posed geometry (#897):
/// the joints themselves (their positions change per drag frame) and component
/// membership (a joint can drive a whole component). Kept separate from
/// [`document_mesh_fingerprint`] so dragging a joint never invalidates the expensive
/// kernel meshes — only the cheap posed copies. Joint fields are hashed directly
/// (no JSON) so a drag frame stays cheap (#1027).
pub(crate) fn document_pose_fingerprint(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    document_mesh_fingerprint(doc).hash(&mut h);
    for j in doc.joints.values() {
        j.base.hash(&mut h);
        j.position.hash(&mut h);
        j.position2.hash(&mut h);
        j.position3.hash(&mut h);
        j.rest.hash(&mut h);
        j.rest2.hash(&mut h);
        j.rest3.hash(&mut h);
        // Members and kind: who moves, and how.
        std::mem::discriminant(&j.kind).hash(&mut h);
        for m in &j.members {
            // JointRef is a small enum of indices.
            format!("{m:?}").hash(&mut h);
        }
        // The placement and the frame both affect an in-progress joint's posed presentation.
        format!("{:?}", j.placement).hash(&mut h);
        format!("{:?}", j.frame).hash(&mut h);
    }
    for (i, c) in doc.components.iter() {
        i.hash(&mut h);
        c.parent.hash(&mut h);
    }
    for entry in &doc.component_members {
        format!("{entry:?}").hash(&mut h);
    }
    h.finish()
}

/// The cached **un-posed** mesh when the cache is free, else a fresh uncached build —
/// safe both outside and *inside* the mesh cache's own borrow (a Move's transform
/// resolves its snap points from within it, #650). What feature inputs and the joint
/// frame resolvers read; the posed presentation is [`body_solid_mesh`].
pub(crate) fn body_solid_mesh_unposed(doc: &Document, body_index: crate::model::BodyKey) -> Option<SolidMesh> {
    let fingerprint = document_mesh_fingerprint(doc);
    let outcome = BODY_MESH_CACHE.with(|cache| match cache.try_borrow_mut() {
        Ok(mut cache) => {
            if cache.0 != fingerprint {
                cache.0 = fingerprint;
                cache.1.clear();
            }
            if let Some(mesh) = cache.1.get(&body_index) {
                MESH_CACHE_STATS.with(|s| s.borrow_mut().hits += 1);
                return Some(mesh.clone());
            }
            MESH_CACHE_STATS.with(|s| s.borrow_mut().misses += 1);
            let mesh = body_solid_mesh_uncached(doc, body_index);
            cache.1.insert(body_index, mesh.clone());
            if let Some(ref m) = mesh {
                record_committed_mesh(doc, body_index, m);
            }
            Some(mesh)
        }
        Err(_) => None,
    });
    match outcome {
        Some(mesh) => mesh,
        None => {
            MESH_CACHE_STATS.with(|s| s.borrow_mut().misses += 1);
            let mesh = body_solid_mesh_uncached(doc, body_index);
            if let Some(ref m) = mesh {
                record_committed_mesh(doc, body_index, m);
            }
            mesh
        }
    }
}

thread_local! {
    /// Per-thread memo for smooth vertex normals (#1037), keyed by the pose fingerprint so
    /// it invalidates exactly when [`body_solid_mesh`] does. Normals cost a hash of every
    /// corner to compute, which is fine once per edit and far too much once per frame.
    static BODY_NORMALS_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, Option<std::rc::Rc<Vec<[Vec3; 3]>>>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
}

/// Smooth per-vertex normals for a body's current mesh (#1037), memoized per document state.
/// `None` for a body with no mesh. Keyed off the posed fingerprint, so a jointed body that
/// moves gets normals for where it actually is.
/// Shared rather than cloned: the viewport asks for these every frame, and a large
/// imported mesh's normals are megabytes.
pub fn body_smooth_normals(
    doc: &Document,
    body_index: crate::model::BodyKey,
) -> Option<std::rc::Rc<Vec<[Vec3; 3]>>> {
    let fingerprint = document_pose_fingerprint(doc);
    let cached = BODY_NORMALS_CACHE.with(|cache| match cache.try_borrow_mut() {
        Ok(mut cache) => {
            if cache.0 != fingerprint {
                cache.0 = fingerprint;
                cache.1.clear();
            }
            // `Some(value)` is a hit — the value itself may legitimately be `None`.
            cache.1.get(&body_index).cloned()
        }
        Err(_) => None,
    });
    if let Some(hit) = cached {
        return hit;
    }
    let normals = body_solid_mesh(doc, body_index).map(|m| std::rc::Rc::new(smooth_normals(&m)));
    BODY_NORMALS_CACHE.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut() {
            cache.1.insert(body_index, normals.clone());
        }
    });
    normals
}

thread_local! {
    /// Per-thread memo for the **posed** meshes (#893/#897): keyed by the pose
    /// fingerprint, whose misses cost one rigid transform of the cached un-posed mesh —
    /// never a kernel rebuild. This is what makes dragging a joint through its motion
    /// interactive.
    static POSED_BODY_MESH_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, Option<SolidMesh>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
}

/// Build the solid mesh for a single body (by index), or `None` if the body is deleted,
/// missing, or its source feature produces no geometry. Memoized per document state (#162).
/// Joints (#893) pose the driven body here, at the presentation seam: this is what the
/// viewport, exports, and measures read, while feature inputs (booleans, moves) keep
/// reading the un-jointed geometry — a joint is an assembly relationship, not a modelling
/// operation.
pub fn body_solid_mesh(doc: &Document, body_index: crate::model::BodyKey) -> Option<SolidMesh> {
    let unposed = body_solid_mesh_unposed(doc, body_index);
    if doc.joints.is_empty() {
        return unposed;
    }
    let fingerprint = document_pose_fingerprint(doc);
    POSED_BODY_MESH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        if let Some(mesh) = cache.1.get(&body_index) {
            return mesh.clone();
        }
        let mesh = unposed.map(|m| crate::joints::posed_mesh(doc, body_index, m));
        cache.1.insert(body_index, mesh.clone());
        mesh
    })
}

/// A body's kernel solid **with its joint pose applied** (#893) — what STEP export and
/// anything else presenting the assembly should read, while feature inputs keep the
/// un-jointed [`occt_body_shape`].
pub fn posed_body_shape(doc: &Document, body_index: crate::model::BodyKey) -> Option<crate::kernel::Shape> {
    let shape = occt_body_shape(doc, body_index)?;
    match crate::joints::body_joint_pose(doc, body_index) {
        Some(pose) => shape.transformed(&mat4_to_rows_3x4(&pose)),
        None => Some(shape),
    }
}

thread_local! {
    /// Per-thread memo for the mesh **analyses** the pick/hover path runs every frame (#845):
    /// coplanar face groups and feature edges are derived purely from a body's mesh, so they
    /// live and die with the same document fingerprint the mesh cache uses. Recomputing them
    /// per body per frame is what made a document with engraved text lag while zooming — the
    /// cursor hovering the model re-derived every face group of every body, every frame.
    static BODY_FACE_GROUP_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, std::rc::Rc<Vec<Vec<[Vec3; 3]>>>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
    static BODY_EDGE_CHAIN_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, std::rc::Rc<Vec<Vec<(Vec3, Vec3)>>>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
    static BODY_FEATURE_EDGE_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, std::rc::Rc<Vec<(Vec3, Vec3)>>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
}

/// A finely faceted tube, for tests in other modules that need real mesh bulk (#1026).
#[cfg(test)]
pub fn tests_tube(centre: Vec3, radius: f32, height: f32) -> Vec<[Vec3; 3]> {
    tests::tube(centre, radius, height, CIRCLE_SEGMENTS)
}

/// A body's coplanar face groups, memoized per document state (#845).
pub fn body_face_groups(doc: &Document, body_index: crate::model::BodyKey) -> std::rc::Rc<Vec<Vec<[Vec3; 3]>>> {
    let fingerprint = document_pose_fingerprint(doc);
    // The mesh itself comes from its own cache; take it before borrowing this one.
    let mesh = body_solid_mesh(doc, body_index);
    BODY_FACE_GROUP_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        if let Some(groups) = cache.1.get(&body_index) {
            return groups.clone();
        }
        let groups = std::rc::Rc::new(
            mesh.map(|m| crate::gpu_viewport::solid_mesh_coplanar_faces(&m))
                .unwrap_or_default(),
        );
        cache.1.insert(body_index, groups.clone());
        groups
    })
}

/// A body's feature-edge **chains** (#626), memoized per document state (#845): the pick and
/// hover paths walk these every frame, and rebuilding them per body per frame is what made a
/// heavy document lag.
pub fn body_edge_chains(doc: &Document, body_index: crate::model::BodyKey) -> std::rc::Rc<Vec<Vec<(Vec3, Vec3)>>> {
    let fingerprint = document_pose_fingerprint(doc);
    let mesh = body_solid_mesh(doc, body_index);
    BODY_EDGE_CHAIN_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        if let Some(chains) = cache.1.get(&body_index) {
            return chains.clone();
        }
        let chains = std::rc::Rc::new(
            mesh.map(|m| crate::gpu_viewport::solid_mesh_edge_chains(&m))
                .unwrap_or_default(),
        );
        cache.1.insert(body_index, chains.clone());
        chains
    })
}

/// A **cylindrical** surface of a body (#1013): a hole's wall, a boss, a round shaft. The
/// mesher facets a circular profile finely enough (`CIRCLE_SEGMENTS`) that the whole wall
/// already comes back as one group from `solid_mesh_coplanar_faces` — the strips merge under
/// the crease threshold — but that group is no plane, and calling it a flat face gives it a
/// nonsense normal. This names it for what it is, and gives its **centre line** an identity:
/// the thing "put this hole on that shaft" is actually about.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyCylinder {
    /// The middle of the axis segment the surface spans.
    pub origin: Vec3,
    /// The axis direction, signed canonically so the same surface always keys the same way.
    pub dir: Vec3,
    pub radius: f32,
    /// Half the axial extent, so the drawn axis matches the surface's own length.
    pub half_length: f32,
    pub triangles: Vec<[Vec3; 3]>,
}

/// How far a group's points may stray from the fitted cylinder, as a fraction of the radius.
const CYLINDER_FIT_TOLERANCE: f32 = 0.03;
/// How far a group's normals must fan out before it can be a cylinder rather than a plane.
const CYLINDER_MIN_FAN_COS: f32 = 0.5; // 60°
/// How closely the surface's normal must follow the direction straight out from its axis.
/// This is what tells a cylinder from a faceted prism: a box's four walls fit a circle
/// through their corners perfectly, but across each wall the normal wanders tens of degrees
/// off radial, where a finely faceted round wall never leaves a few.
const CYLINDER_RADIAL_COS: f32 = 0.985; // 10°
/// How far apart consecutive facets may sit round the axis. A round wall only reaches the
/// mesher as one group because its facets fall inside the 15° crease threshold; anything
/// coarser is a prism whose walls happen to have their corners on a circle. The single
/// widest gap is ignored, so a half-round wall is still round where it exists.
const CYLINDER_MAX_FACET_STEP_DEG: f32 = 20.0;

/// Fit a cylinder to one coplanar-triangle group (#1013), or `None` if the group is flat, too
/// faceted to be round, or doesn't fit a cylinder at all.
///
/// The normals of a cylinder all lie in the plane square to its axis, so two of them that
/// differ enough give the axis outright; the radius and centre then come from a plain
/// least-squares circle through the points projected onto that plane.
pub fn fit_cylinder(tris: &[[Vec3; 3]]) -> Option<BodyCylinder> {
    if tris.len() < 6 {
        return None;
    }
    let normals: Vec<Vec3> = tris
        .iter()
        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero())
        .filter(|n| n.length_squared() > 0.5)
        .collect();
    let first = *normals.first()?;
    // The normal furthest from the first one: with a full turn of wall that is the opposite
    // side, so pair it with a quarter-turn one instead — any two that differ enough do.
    let mut dir = Vec3::ZERO;
    let mut best_fan = 1.0f32;
    for n in &normals {
        let cos = first.dot(*n);
        let cross = first.cross(*n);
        if cross.length() > 0.1 && cos < best_fan {
            best_fan = cos;
            dir = cross.normalize();
        }
    }
    if best_fan > CYLINDER_MIN_FAN_COS || dir.length_squared() < 0.5 {
        return None;
    }
    // Every normal must be square to that axis, or the surface isn't a cylinder.
    if normals.iter().any(|n| n.dot(dir).abs() > 0.05) {
        return None;
    }
    let dir = canonical_axis_direction(dir);
    let e1 = dir.any_orthonormal_vector();
    let e2 = dir.cross(e1).normalize_or_zero();
    let points: Vec<Vec3> = tris.iter().flat_map(|t| t.iter().copied()).collect();
    let flat: Vec<glam::Vec2> = points
        .iter()
        .map(|p| glam::Vec2::new(p.dot(e1), p.dot(e2)))
        .collect();
    let (centre, radius) = fit_circle(&flat)?;
    // Reject anything that isn't actually round — a box's four walls fit a circle through
    // their corners perfectly well, which is exactly what the tolerance is here to catch.
    let rms = (flat
        .iter()
        .map(|p| ((*p - centre).length() - radius).powi(2))
        .sum::<f32>()
        / flat.len() as f32)
        .sqrt();
    if radius <= 1e-4 || rms > radius * CYLINDER_FIT_TOLERANCE {
        return None;
    }
    // Every facet must face straight out from the axis, or it's a flat wall of a prism.
    let axis_point = e1 * centre.x + e2 * centre.y;
    let mut angles = Vec::with_capacity(tris.len());
    for (t, n) in tris.iter().zip(&normals) {
        let c = (t[0] + t[1] + t[2]) / 3.0;
        let radial = c - axis_point - dir * (c - axis_point).dot(dir);
        let radial = radial.normalize_or_zero();
        if radial.length_squared() < 0.5 || radial.dot(*n).abs() < CYLINDER_RADIAL_COS {
            return None;
        }
        angles.push(radial.dot(e2).atan2(radial.dot(e1)).to_degrees());
    }
    // ...and the facets must be closely enough spaced round the axis to be round at all.
    angles.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut gaps: Vec<f32> = angles
        .windows(2)
        .map(|w| w[1] - w[0])
        .chain(std::iter::once(360.0 - (angles[angles.len() - 1] - angles[0])))
        .collect();
    gaps.sort_by(|a, b| b.partial_cmp(a).unwrap());
    if gaps.get(1).copied().unwrap_or(360.0) > CYLINDER_MAX_FACET_STEP_DEG {
        return None;
    }
    let along: Vec<f32> = points.iter().map(|p| p.dot(dir)).collect();
    let lo = along.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = along.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let origin = axis_point + dir * ((lo + hi) * 0.5);
    Some(BodyCylinder {
        origin,
        dir,
        radius,
        half_length: (hi - lo) * 0.5,
        triangles: tris.to_vec(),
    })
}

/// An axis direction with a settled sign, so the same line always keys the same way whichever
/// way round it was derived.
pub fn canonical_axis_direction(dir: Vec3) -> Vec3 {
    let d = dir.normalize_or_zero();
    let (ax, ay, az) = (d.x.abs(), d.y.abs(), d.z.abs());
    let lead = if ax >= ay && ax >= az {
        d.x
    } else if ay >= az {
        d.y
    } else {
        d.z
    };
    if lead < 0.0 { -d } else { d }
}

/// Least-squares circle through 2-D points (the standard algebraic fit): centre and radius,
/// or `None` when the points are collinear.
fn fit_circle(points: &[glam::Vec2]) -> Option<(glam::Vec2, f32)> {
    let n = points.len() as f32;
    if n < 3.0 {
        return None;
    }
    let mean = points.iter().copied().sum::<glam::Vec2>() / n;
    let (mut sxx, mut sxy, mut syy, mut sxz, mut syz) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
    for p in points {
        let (u, v) = (p.x - mean.x, p.y - mean.y);
        let z = u * u + v * v;
        sxx += u * u;
        sxy += u * v;
        syy += v * v;
        sxz += u * z;
        syz += v * z;
    }
    let det = sxx * syy - sxy * sxy;
    if det.abs() < 1e-9 {
        return None;
    }
    let cu = (sxz * syy - syz * sxy) / (2.0 * det);
    let cv = (syz * sxx - sxz * sxy) / (2.0 * det);
    let centre = glam::Vec2::new(cu, cv) + mean;
    let radius = (points
        .iter()
        .map(|p| (*p - centre).length())
        .sum::<f32>())
        / n;
    Some((centre, radius))
}

thread_local! {
    /// World bounds per body, and per coplanar face group (#1026). The pick/hover path runs
    /// every frame the camera moves, and without these it projects **every triangle of every
    /// body** to answer "what is under the cursor" — which is why zooming over a large
    /// document lagged while orbiting (which suppresses hover) did not.
    static BODY_BOUNDS_CACHE: std::cell::RefCell<(
        u64,
        std::rc::Rc<HashMap<crate::model::BodyKey, Option<(Vec3, Vec3)>>>,
    )> = std::cell::RefCell::new((0, std::rc::Rc::new(HashMap::new())));
    static BODY_FACE_GROUP_BOUNDS_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, std::rc::Rc<Vec<(Vec3, Vec3)>>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
}

/// The world bounds of `triangles`, or `None` when there are none.
pub fn triangle_bounds(triangles: &[[Vec3; 3]]) -> Option<(Vec3, Vec3)> {
    let mut points = triangles.iter().flat_map(|t| t.iter());
    let first = *points.next()?;
    Some(points.fold((first, first), |(min, max), p| (min.min(*p), max.max(*p))))
}

/// **Every** body's world bounding box at once, indexed by body, memoized per document state
/// (#1026).
///
/// Deliberately batched. Every cached mesh accessor keys on
/// [`document_pose_fingerprint`] (an integer revision plus a cheap joint hash, #1027) — so
/// asking per body inside a loop costs one full document hash per body per frame, which on a
/// large document dwarfs the triangle walk this was meant to avoid. The pick walks fetch this
/// once and then index it.
pub fn body_world_bounds_all(
    doc: &Document,
) -> std::rc::Rc<HashMap<crate::model::BodyKey, Option<(Vec3, Vec3)>>> {
    let fingerprint = document_pose_fingerprint(doc);
    {
        let hit = BODY_BOUNDS_CACHE.with(|cache| {
            let cache = cache.borrow();
            (cache.0 == fingerprint).then(|| cache.1.clone())
        });
        if let Some(bounds) = hit {
            return bounds;
        }
    }
    // Built outside the cache's borrow: meshing a body re-enters these caches.
    let bounds = std::rc::Rc::new(
        doc.bodies
            .keys()
            .map(|bi| (bi, body_solid_mesh(doc, bi).and_then(|m| m.bounds())))
            .collect::<HashMap<_, _>>(),
    );
    BODY_BOUNDS_CACHE.with(|cache| {
        *cache.borrow_mut() = (fingerprint, bounds.clone());
    });
    bounds
}

/// One world bounding box per coplanar face group, in the same order as
/// [`body_face_groups`], memoized per document state (#1026).
pub fn body_face_group_bounds(doc: &Document, body_index: crate::model::BodyKey) -> std::rc::Rc<Vec<(Vec3, Vec3)>> {
    let fingerprint = document_pose_fingerprint(doc);
    let groups = body_face_groups(doc, body_index);
    BODY_FACE_GROUP_BOUNDS_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        if let Some(bounds) = cache.1.get(&body_index) {
            return bounds.clone();
        }
        let bounds = std::rc::Rc::new(
            groups
                .iter()
                .map(|tris| triangle_bounds(tris).unwrap_or((Vec3::ZERO, Vec3::ZERO)))
                .collect::<Vec<_>>(),
        );
        cache.1.insert(body_index, bounds.clone());
        bounds
    })
}

thread_local! {
    /// A body's fitted cylinders, keyed like every other mesh analysis (#845/#1013): the
    /// pick and hover paths ask for these every frame.
    static BODY_CYLINDER_CACHE: std::cell::RefCell<(u64, HashMap<crate::model::BodyKey, std::rc::Rc<Vec<BodyCylinder>>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
}

/// A body's cylindrical surfaces in its **own** coordinates (#1013), un-posed and uncached —
/// what a mate resolves against, exactly like the vertex and face keys beside it. The posed
/// [`body_cylinders`] can't serve here: it goes through the posed mesh cache, which is being
/// filled by the very joint resolution asking the question.
pub fn body_cylinders_unposed(doc: &Document, body_index: crate::model::BodyKey) -> Vec<BodyCylinder> {
    let Some(mesh) = body_solid_mesh_unposed(doc, body_index) else {
        return Vec::new();
    };
    crate::gpu_viewport::solid_mesh_coplanar_faces(&mesh)
        .iter()
        .filter_map(|tris| fit_cylinder(tris))
        .collect()
}

/// The world segment a body's cylinder axis spans, in the body's own coordinates (#1013) —
/// the un-posed twin of [`body_axis_segment`], for mate resolution.
pub fn body_axis_segment_unposed(
    doc: &Document,
    body: crate::model::BodyKey,
    origin: [i32; 3],
    dir: [i32; 3],
) -> Option<(Vec3, Vec3)> {
    let q = crate::hierarchy::quantize_body_point;
    let cyl = body_cylinders_unposed(doc, body)
        .into_iter()
        .find(|c| q(c.origin) == origin && q(c.dir) == dir)?;
    Some((
        cyl.origin - cyl.dir * cyl.half_length,
        cyl.origin + cyl.dir * cyl.half_length,
    ))
}

/// A body's cylindrical surfaces, memoized per document state (#1013).
pub fn body_cylinders(doc: &Document, body_index: crate::model::BodyKey) -> std::rc::Rc<Vec<BodyCylinder>> {
    let fingerprint = document_pose_fingerprint(doc);
    let groups = body_face_groups(doc, body_index);
    BODY_CYLINDER_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        if let Some(found) = cache.1.get(&body_index) {
            return found.clone();
        }
        let found =
            std::rc::Rc::new(groups.iter().filter_map(|tris| fit_cylinder(tris)).collect());
        cache.1.insert(body_index, std::rc::Rc::clone(&found));
        found
    })
}

/// Re-find a body's cylindrical surface from a quantized key (#1013) — the round-wall twin of
/// [`face_group_matching`], so a hole follows its geometry and simply stops resolving when a
/// rebuild takes it away.
pub fn body_cylinder_matching(
    doc: &Document,
    body: crate::model::BodyKey,
    origin: [i32; 3],
    dir: [i32; 3],
    radius: i32,
) -> Option<BodyCylinder> {
    let q = crate::hierarchy::quantize_body_point;
    body_cylinders(doc, body)
        .iter()
        .find(|c| {
            q(c.origin) == origin
                && q(c.dir) == dir
                && q(Vec3::splat(c.radius))[0] == radius
        })
        .cloned()
}

/// The world segment a body's cylinder axis spans (#1013): the centre line, as long as the
/// surface it belongs to.
pub fn body_axis_segment(
    doc: &Document,
    body: crate::model::BodyKey,
    origin: [i32; 3],
    dir: [i32; 3],
) -> Option<(Vec3, Vec3)> {
    let q = crate::hierarchy::quantize_body_point;
    let cyl = body_cylinders(doc, body)
        .iter()
        .find(|c| q(c.origin) == origin && q(c.dir) == dir)
        .cloned()?;
    Some((
        cyl.origin - cyl.dir * cyl.half_length,
        cyl.origin + cyl.dir * cyl.half_length,
    ))
}

/// The scene element a fitted cylinder is (#1013).
pub fn cylinder_scene_element(body: crate::model::BodyKey, cyl: &BodyCylinder) -> crate::hierarchy::SceneElement {
    let q = crate::hierarchy::quantize_body_point;
    crate::hierarchy::SceneElement::BodyCylinder {
        body,
        origin: q(cyl.origin),
        dir: q(cyl.dir),
        radius: q(Vec3::splat(cyl.radius))[0],
    }
}

/// A body's **flat** face groups (#1013): the coplanar groups that aren't cylinders, which is
/// what a face pick, a face key and a mating plane all mean by "a face".
pub fn body_flat_face_groups(doc: &Document, body_index: crate::model::BodyKey) -> Vec<Vec<[Vec3; 3]>> {
    body_face_groups(doc, body_index)
        .iter()
        .filter(|tris| fit_cylinder(tris).is_none())
        .cloned()
        .collect()
}

/// A body's feature edges (mesh boundaries and creases), memoized per document state (#845).
pub fn body_feature_edges(doc: &Document, body_index: crate::model::BodyKey) -> std::rc::Rc<Vec<(Vec3, Vec3)>> {
    let fingerprint = document_pose_fingerprint(doc);
    let mesh = body_solid_mesh(doc, body_index);
    BODY_FEATURE_EDGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        if let Some(edges) = cache.1.get(&body_index) {
            return edges.clone();
        }
        let edges = std::rc::Rc::new(
            mesh.map(|m| crate::gpu_viewport::solid_mesh_unique_edges(&m))
                .unwrap_or_default(),
        );
        cache.1.insert(body_index, edges.clone());
        edges
    })
}

/// Build a body's solid mesh **without** consulting or populating [`BODY_MESH_CACHE`]. Used for
/// the in-progress-edit descendant preview (#260), which meshes a throwaway scratch document each
/// frame — routing that through the cache would evict the real document's warm meshes every frame
/// (the two docs fingerprint differently), forcing a full rebuild of the whole scene.
pub fn body_solid_mesh_uncached_pub(doc: &Document, body_index: crate::model::BodyKey) -> Option<SolidMesh> {
    body_solid_mesh_uncached(doc, body_index)
}

fn body_solid_mesh_uncached(doc: &Document, body_index: crate::model::BodyKey) -> Option<SolidMesh> {
    let body = doc.bodies.get(body_index)?;
    // A primitive shape (#909): meshed analytically, no sketch behind it.
    if let crate::model::BodySource::Primitive(pi) = body.source {
        let shape = doc.primitives.get(pi)?;
        return crate::primitives::mesh(doc, shape);
    }
    if let Some(key) = body.source.imported_mesh_key() {
        let imported = doc.imported_meshes.get(key)?;
        return (!imported.triangles.is_empty()).then(|| SolidMesh {
            triangles: imported.triangles.clone(),
        });
    }

    // An imported unit's materialized body (#724): its instance's evaluated meshes,
    // placed, merged into one solid so all body machinery (snap points, edge dimensions,
    // face picks, export) sees ordinary triangles.
    if let crate::model::BodySource::UnitInstance(instance) = body.source {
        let triangles: Vec<[Vec3; 3]> = crate::units::placed_instance_meshes(doc, instance)
            .into_iter()
            .flat_map(|m| m.triangles)
            .collect();
        return (!triangles.is_empty()).then_some(SolidMesh { triangles });
    }
    // A unit with cuts (#726): kernel-only — subtract the tools from the unit's fused
    // solid. When the kernel can't build it, fall back to the intact placed unit mesh
    // (the cut is dropped and `kernel_fallback_cut_warning` says so, like any body).
    if let crate::model::BodySource::UnitCut { instance, .. } = body.source {
        if let Some(shape) = occt_body_shape(doc, body_index) {
            let tris = shape.tessellate(OCCT_DEFLECTION as f64);
            if !tris.is_empty() {
                return Some(SolidMesh { triangles: tris });
            }
        }
        let triangles: Vec<[Vec3; 3]> = crate::units::placed_instance_meshes(doc, instance)
            .into_iter()
            .flat_map(|m| m.triangles)
            .collect();
        return (!triangles.is_empty()).then_some(SolidMesh { triangles });
    }

    if let crate::model::BodySource::Repeated {
        op,
        target,
        instance,
        ref add,
        ref cut,
    } = body.source
    {
        // Pure repeat meshes by transforming the input; fused add/cut go through
        // `occt_body_shape` below so a post-repeat cut shows up (#1345).
        if add.is_empty() && cut.is_empty() {
            let rp = doc.repeat_ops.get(op)?;
            let &input = rp.targets.get(target)?;
            if input == body_index {
                return None;
            }
            let m = repeat_instance_transform(doc, rp, instance)?;
            let source = body_solid_mesh_uncached(doc, input)?;
            let triangles = source
                .triangles
                .iter()
                .map(|tri| {
                    [
                        m.transform_point3(tri[0]),
                        m.transform_point3(tri[1]),
                        m.transform_point3(tri[2]),
                    ]
                })
                .collect();
            return Some(SolidMesh { triangles });
        }
    }
    if let crate::model::BodySource::Moved {
        op,
        target,
        ref add,
        ref cut,
    } = body.source
    {
        if add.is_empty() && cut.is_empty() {
            let mv = doc.move_ops.get(op)?;
            let &input = mv.targets.get(target)?;
            if input == body_index {
                return None;
            }
            let m = move_op_transform(doc, mv)?;
            // The uncached inner fn: this runs inside the mesh cache's own borrow, so going
            // through the cached wrapper would double-borrow the RefCell.
            let source = body_solid_mesh_uncached(doc, input)?;
            let triangles = source
                .triangles
                .iter()
                .map(|tri| {
                    [
                        m.transform_point3(tri[0]),
                        m.transform_point3(tri[1]),
                        m.transform_point3(tri[2]),
                    ]
                })
                .collect();
            return Some(SolidMesh { triangles });
        }
    }
    if let crate::model::BodySource::Mirrored {
        op,
        target,
        ref add,
        ref cut,
    } = body.source
    {
        if add.is_empty() && cut.is_empty() {
            let mr = doc.mirror_ops.get(op)?;
            let &input = mr.targets.get(target)?;
            if input == body_index {
                return None;
            }
            // Join/Cut outputs are a real boolean against the source (#639), so they come from the
            // kernel and tessellate — like Boolean and Slice outputs. A plain reflection stays on
            // the cheap transform path so the lean build still mirrors.
            if mr.mode.consumes_input() {
                let shape = occt_mirrored_output_shape(doc, op, target)?;
                let tris = shape.tessellate(OCCT_DEFLECTION as f64);
                return (!tris.is_empty()).then_some(SolidMesh { triangles: tris });
            }
            let m = mirror_op_transform(doc, mr)?;
            let source = body_solid_mesh_uncached(doc, input)?;
            // A reflection flips handedness, so reverse each triangle's winding (swap two
            // vertices) to keep its outward normal pointing out.
            let triangles = source
                .triangles
                .iter()
                .map(|tri| {
                    [
                        m.transform_point3(tri[0]),
                        m.transform_point3(tri[2]),
                        m.transform_point3(tri[1]),
                    ]
                })
                .collect();
            return Some(SolidMesh { triangles });
        }
    }
    if let crate::model::BodySource::Boolean {
        op,
        solid,
        ref add,
        ref cut,
    } = body.source
    {
        // Pure boolean meshes directly; fused add/cut go through `occt_body_shape` below
        // so a post-combine cut shows up in the viewport (#1338).
        if add.is_empty() && cut.is_empty() {
            let shape = occt_boolean_output_shape(doc, op, solid)?;
            let tris = shape.tessellate(OCCT_DEFLECTION as f64);
            return (!tris.is_empty()).then_some(SolidMesh { triangles: tris });
        }
    }
    if let crate::model::BodySource::EdgeTreated {
        op,
        target,
        ref add,
        ref cut,
    } = body.source
    {
        // Pure treated meshes directly; fused add/cut go through `occt_body_shape` below
        // so a post-fillet cut shows up (#1345).
        if add.is_empty() && cut.is_empty() {
            // Kernel first: primitive-hosted edges have no extrusion to splice onto (#1329),
            // and an extrusion-hosted op still prefers the true BREP when the kernel can
            // build it. Mesh-bevel fallback keeps the no-kernel path working.
            if let Some(shape) = occt_edge_treated_output_shape(doc, op, target) {
                let tris = shape.tessellate(OCCT_DEFLECTION as f64);
                if !tris.is_empty() {
                    return Some(SolidMesh { triangles: tris });
                }
            }
            let (clone, input) = edge_treated_input_doc(doc, op, target)?;
            return body_solid_mesh_uncached(&clone, input);
        }
    }
    if let crate::model::BodySource::Sliced {
        op,
        target,
        piece,
        ref add,
        ref cut,
    } = body.source
    {
        // Pure slice fragments mesh directly; fused add/cut go through `occt_body_shape`
        // below so a post-slice cut shows up (#1345).
        if add.is_empty() && cut.is_empty() {
            let shape = occt_sliced_output_shape(doc, op, target, piece)?;
            let tris = shape.tessellate(OCCT_DEFLECTION as f64);
            return (!tris.is_empty()).then_some(SolidMesh { triangles: tris });
        }
    }
    if let crate::model::BodySource::Shelled { op, target, ref add, ref cut } = body.source {
        // Pure hollow meshes directly; fused add/cut go through `occt_body_shape` below
        // so the boss/cut shows up in the viewport (#1168).
        if add.is_empty() && cut.is_empty() {
            let shape = occt_shelled_output_shape(doc, op, target)?;
            let tris = shape.tessellate(OCCT_DEFLECTION as f64);
            return (!tris.is_empty()).then_some(SolidMesh { triangles: tris });
        }
    }
    // Helical revolve (#1249): do *not* shortcut to the density-capped lathe mesh.
    // The kernel builds a smooth helix-pipe BREP; tessellate that (adaptive deflection
    // floor keeps multi-turn coils interactive). STEP export and the viewport then
    // share the same curved solid — like cylinders/spheres, not a permanent low-poly.
    // Fuse the body's added extrusions into one real solid via OCCT and subtract its cut
    // extrusions (#86/#35) when they're all kernel-representable; otherwise fall back to
    // per-extrusion meshing below. The hand-rolled fallback cannot perform a solid
    // subtraction, so when the kernel fails on a cut-bearing body the additive geometry
    // renders alone — `kernel_fallback_cut_warning` surfaces exactly that case.
    if let Some(shape) = occt_body_shape(doc, body_index) {
        let tris = shape.tessellate(OCCT_DEFLECTION as f64);
        if !tris.is_empty() {
            return Some(SolidMesh { triangles: tris });
        }
    }
    // Kernel path failed (or lean build): the additive fallback. A revolve-sourced body
    // meshes its lathe; cut revolutions are ignored here, like cut extrusions (the
    // fallback warning covers both).
    if let crate::model::BodySource::Revolve(ri) = body.source {
        let rev = doc.revolutions.get(ri)?;
        return revolve_mesh(doc, rev);
    }
    if let crate::model::BodySource::Sweep(fi) = body.source {
        let fp = doc.sweeps.get(fi)?;
        return sweep_mesh(doc, fp);
    }
    if let crate::model::BodySource::Loft(li) = body.source {
        let loft = doc.lofts.get(li)?;
        return loft_mesh(doc, loft);
    }
    let mut mesh = SolidMesh::default();
    // A `Solid` body's primitive base (#1104): its mesh joins the additive extrusions in
    // the fallback (no boolean — the kernel path above does the real fuse; this only runs
    // when the kernel is off or fails).
    if let crate::model::BodySource::Solid { base: Some(base), .. } = body.source {
        if let Some(shape) = doc.primitives.get(base).and_then(|p| crate::primitives::mesh(doc, p)) {
            mesh.triangles.extend(shape.triangles);
        }
    }
    for &ei in body.source.extrusion_indices() {
        let Some(extrusion) = doc.extrusions.get(ei) else {
            continue;
        };
        if let Some(solid) = extrusion_mesh(doc, extrusion) {
            let triangles = match extrusion_host_unpose(doc, extrusion) {
                Some(inv) => solid
                    .triangles
                    .iter()
                    .map(|tri| {
                        [
                            inv.transform_point3(tri[0]),
                            inv.transform_point3(tri[1]),
                            inv.transform_point3(tri[2]),
                        ]
                    })
                    .collect(),
                None => solid.triangles,
            };
            mesh.triangles.extend(triangles);
        }
    }
    for (ri, is_cut) in revolutions_targeting(doc, body_index) {
        if is_cut {
            continue;
        }
        if let Some(solid) = revolve_mesh(doc, &doc.revolutions[ri]) {
            mesh.triangles.extend(solid.triangles);
        }
    }
    for (fi, is_cut) in sweeps_targeting(doc, body_index) {
        if is_cut {
            continue;
        }
        if let Some(solid) = sweep_mesh(doc, &doc.sweeps[fi]) {
            mesh.triangles.extend(solid.triangles);
        }
    }
    for (li, is_cut) in lofts_targeting(doc, body_index) {
        if is_cut {
            continue;
        }
        if let Some(solid) = loft_mesh(doc, &doc.lofts[li]) {
            mesh.triangles.extend(solid.triangles);
        }
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// Cache key for an in-progress extrusion preview (#386): the document's mesh fingerprint plus
/// a hash of the preview extrusion itself (and the target body, for cuts). One entry suffices —
/// there is at most one live preview at a time — and it makes idle frames free: the expensive
/// kernel rebuild only reruns when the drag actually changes something.
fn preview_cache_key(doc: &Document, extrusion: &Extrusion, body_index: crate::model::BodyKey) -> (u64, u64) {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{extrusion:?}").hash(&mut h);
    body_index.hash(&mut h);
    (document_mesh_fingerprint(doc), h.finish())
}

thread_local! {
    static PREVIEW_MESH_CACHE: std::cell::RefCell<Option<((u64, u64), Option<SolidMesh>)>> =
        const { std::cell::RefCell::new(None) };
    static PREVIEW_CUT_MESH_CACHE: std::cell::RefCell<Option<((u64, u64), Option<SolidMesh>)>> =
        const { std::cell::RefCell::new(None) };
    static CUT_TOOL_BITES_CACHE: std::cell::RefCell<Option<((u64, u64), Option<bool>)>> =
        const { std::cell::RefCell::new(None) };
}

/// True when the extrusion contains text-glyph faces (#386): its kernel build is one solid per
/// glyph plus a boolean per counter — far too slow to rebuild every frame of a gizmo drag.
fn has_text_faces(extrusion: &Extrusion) -> bool {
    extrusion
        .faces
        .iter()
        .any(|f| matches!(f, ExtrudeFace::TextGlyph { .. }))
}

/// Preview-quality mesh for the in-progress extrusion (#386): the same geometry
/// [`extrusion_mesh`] builds, but cached per (document, preview) so idle frames don't rebuild,
/// and routed to the fast tessellated mesher for **text** — dragging an engraving's gizmo
/// through per-glyph kernel booleans every frame was unusably laggy. The commit still builds
/// the real kernel solid.
pub fn preview_extrusion_mesh(doc: &Document, extrusion: &Extrusion) -> Option<SolidMesh> {
    // No body: a fresh preview belongs to nothing yet, so key it on a slot no body has.
    let key = preview_cache_key(doc, extrusion, crate::arena::Key::from_bits(u64::MAX));
    PREVIEW_MESH_CACHE.with(|cache| {
        if let Some((cached_key, mesh)) = cache.borrow().as_ref() {
            if *cached_key == key {
                return mesh.clone();
            }
        }
        let mesh = if has_text_faces(extrusion) {
            let distance = effective_distance(doc, extrusion);
            if extrusion.faces.is_empty() || distance.abs() < 1e-4 {
                None
            } else {
                extrusion_mesh_tessellated(doc, extrusion, distance)
            }
        } else {
            extrusion_mesh(doc, extrusion)
        };
        *cache.borrow_mut() = Some((key, mesh.clone()));
        mesh
    })
}

/// Does `cut`'s tool solid (built **without** the cut overshoot) actually overlap
/// `body_index`'s solid (#380)? A cut whose tool misses the body — e.g. a scripted positive
/// distance on a side face, which extrudes along the outward normal — used to commit as a
/// silent no-op. `None` when the kernel can't answer (non-`occt` build, unbuildable tool or
/// body), in which case callers skip the check.
pub fn cut_tool_bites(doc: &Document, body_index: crate::model::BodyKey, cut: &Extrusion) -> Option<bool> {
    let distance = effective_distance(doc, cut);
    if cut.faces.is_empty() || distance.abs() < 1e-4 {
        return None;
    }
    let key = preview_cache_key(doc, cut, body_index);
    CUT_TOOL_BITES_CACHE.with(|cache| {
        if let Some((cached_key, bites)) = cache.borrow().as_ref() {
            if *cached_key == key {
                return *bites;
            }
        }
        let bites = (|| {
            let tool = occt_extrusion_shape_for_host(doc, cut, distance)?;
            let body = occt_body_shape(doc, body_index)?;
            let common = body.boolean(&tool, crate::kernel::BoolOp::Common)?;
            // Volume of the BREP — tessellating a complex Common just to measure
            // overlap was both slow and could come back empty (#1337).
            Some(common.volume().is_some_and(|v| v.abs() > 1e-3))
        })();
        *cache.borrow_mut() = Some((key, bites));
        bites
    })
}

/// Live preview mesh of `body_index`'s solid with `cut` additionally subtracted — what the
/// body will look like once an in-progress cut extrusion is committed (#142). Clones the
/// document to splice `cut` in as one more cut extrusion without mutating the real doc, so the
/// caller can render the finished-cut shape translucently in place of the intact body. `None`
/// (caller keeps the intact body and its additive-block preview) when the kernel is absent,
/// the body is imported/deleted, the cut is degenerate, the cut is **text** (#386 — a
/// per-glyph boolean chain per frame made the drag unusably laggy; text cuts preview as the
/// additive block instead), or the kernel can't build the result. Cached per
/// (document, cut, body) so unchanged frames are free.
pub fn preview_cut_body_mesh(doc: &Document, body_index: crate::model::BodyKey, cut: &Extrusion) -> Option<SolidMesh> {
    {
        let body = doc.bodies.get(body_index)?;
        if body.source.imported_mesh_key().is_some() {
            return None;
        }
        if cut.faces.is_empty() || effective_distance(doc, cut).abs() < 1e-4 {
            return None;
        }
        if has_text_faces(cut) {
            return None;
        }
        let key = preview_cache_key(doc, cut, body_index);
        PREVIEW_CUT_MESH_CACHE.with(|cache| {
            if let Some((cached_key, mesh)) = cache.borrow().as_ref() {
                if *cached_key == key {
                    return mesh.clone();
                }
            }
            let mut clone = doc.clone();
            let cut_index = clone.extrusions.insert(cut.clone());
            let mut cut_indices = body.source.cut_extrusion_indices().to_vec();
            cut_indices.push(cut_index);
            let mesh = occt_body_mesh(&clone, body.source.extrusion_indices(), &cut_indices)
                // That path rebuilds the body from its extrusions, which only works for
                // extrusion-sourced bodies. A body that came out of a fillet, a boolean, a
                // move… has no add list, so the preview used to vanish exactly where cuts are
                // most common: drilling into an already-finished part (#805). Fall back to
                // subtracting the tool from whatever solid the body actually is.
                .or_else(|| {
                    let target = occt_body_shape(doc, body_index)?;
                    let result = occt_subtract_cut_extrusions(&clone, target, &[cut_index])?;
                    let tris = result.tessellate(OCCT_PREVIEW_DEFLECTION as f64);
                    (!tris.is_empty()).then_some(SolidMesh { triangles: tris })
                });
            *cache.borrow_mut() = Some((key, mesh.clone()));
            mesh
        })
    }
}


/// Combined solid mesh of every non-deleted body in the document (the geometry an STL/OBJ
/// export should contain). Bodies are concatenated into one triangle soup.
pub fn document_solid_mesh(doc: &Document) -> SolidMesh {
    // #146: fuse the kernel-representable bodies into one real union so that where bodies
    // *intersect*, the overlap merges into a single watertight surface instead of exporting as
    // two interpenetrating shells with internal walls. Disjoint bodies simply co-exist in the
    // fused compound (identical output to concatenation for them). Triangle-only imports
    // (STL, faceted STEP) have no kernel shape and are concatenated on top; a STEP import
    // that kept its BREP (#1029) joins the union. If any non-import body isn't representable
    // the whole union is unreliable and we fall back to plain concatenation.
    // Shadow bodies are never deliverables (#1218) — skip them in both paths.
    //
    // #1286: only accept the fused OCCT mesh when it is a closed manifold. OCCT's face
    // triangulation can emit degenerate pole triangles (a BREP sphere is the usual
    // offender), and a non-watertight STL is worse for slicers than interpenetrating shells.
    // Fall back to each body's own mesh (hand-rolled primitives, already-validated
    // extrusions) which is watertight per body.
    if let Some(mesh) = occt_document_union_mesh(doc) {
        if mesh_is_watertight(&mesh) {
            return mesh;
        }
    }
    document_solid_mesh_concat(doc)
}

/// Per-body solid meshes concatenated into one triangle soup (no boolean union). Shadow
/// bodies are skipped (#1218). Used as the document-export fallback when the OCCT union is
/// unavailable or not watertight (#1286).
fn document_solid_mesh_concat(doc: &Document) -> SolidMesh {
    let mut mesh = SolidMesh::default();
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        if let Some(solid) = body_solid_mesh(doc, bi) {
            mesh.triangles.extend(solid.triangles);
        }
    }
    mesh
}

/// Fuse every kernel-representable body into one unioned solid and tessellate it, appending any
/// triangle-only imports (they have no kernel shape). `None` — so the caller falls back to
/// plain per-body concatenation — when a non-imported body isn't kernel-representable or the
/// fuse fails to build/tessellate. See [`document_solid_mesh`] (#146). STEP imports that kept
/// their BREP (#1029) join the fuse.
fn occt_document_union_mesh(doc: &Document) -> Option<SolidMesh> {
    use crate::kernel::BoolOp;
    let mut fused: Option<crate::kernel::Shape> = None;
    let mut imported_triangles: Vec<[Vec3; 3]> = Vec::new();
    let mut saw_kernel_body = false;
    for (bi, body) in doc.bodies.iter() {
        // Shadow bodies are consumed inputs / references — not deliverables (#1218).
        if body.shadow {
            continue;
        }
        if body.source.imported_mesh_key().is_some() {
            // Prefer BREP when the import still has it (#1029); otherwise concatenate triangles.
            if let Some(shape) = occt_body_shape(doc, bi) {
                saw_kernel_body = true;
                fused = Some(match fused {
                    None => shape,
                    Some(acc) => acc.boolean(&shape, BoolOp::Fuse)?,
                });
            } else if let Some(solid) = body_solid_mesh(doc, bi) {
                imported_triangles.extend(solid.triangles);
            }
            continue;
        }
        // A non-imported body that the kernel can't represent means the union would silently
        // drop or mangle it — bail so the caller concatenates instead.
        let shape = occt_body_shape(doc, bi)?;
        saw_kernel_body = true;
        fused = Some(match fused {
            None => shape,
            Some(acc) => acc.boolean(&shape, BoolOp::Fuse)?,
        });
    }
    let mut triangles = Vec::new();
    if let Some(shape) = fused {
        triangles = shape.tessellate(OCCT_DEFLECTION as f64);
        // A fuse of real kernel bodies that tessellates to nothing is a kernel failure, not an
        // empty document — fall back rather than exporting nothing.
        if saw_kernel_body && triangles.is_empty() {
            return None;
        }
    }
    triangles.extend(imported_triangles);
    Some(SolidMesh { triangles })
}

/// The `(point, normal)` plane an extrusion's top cap should lie in, when its target defines
/// one. A vertex target or a plain typed distance has no such plane.
pub fn target_top_plane(doc: &Document, extrusion: &Extrusion) -> Option<(Vec3, Vec3)> {
    match extrusion.target.as_ref()? {
        ExtrudeTarget::Face(face) => face_plane(doc, face),
        ExtrudeTarget::Plane(index) => {
            let plane = doc.construction_planes.get(*index)?;
            Some((plane.origin, plane.normal))
        }
        ExtrudeTarget::BodyFace(face_id) => body_face_plane(doc, face_id),
        ExtrudeTarget::RepeatedFace { face, op, instance } => {
            repeated_face_plane(doc, face, *op, *instance)
        }
        ExtrudeTarget::Vertex(_) => None,
    }
}

/// The `(point, normal)` plane of a 3D body face target (#126) — another (or the same)
/// extrusion's cap or side wall, unlike [`face_plane`] which only handles flat sketch
/// profiles. `sketch_frame` already resolves the plane of any `FaceId`, cap/side included.
pub fn body_face_plane(doc: &Document, face_id: &crate::model::FaceId) -> Option<(Vec3, Vec3)> {
    let frame = sketch_frame(doc, face_id.clone())?;
    Some((frame.origin, frame.normal))
}

/// Where a base profile vertex `v` lands when extruded along `dir`. With a target plane each
/// vertex slides until it meets that plane, so the whole top cap lies in it (full contact even
/// when the plane is slanted); otherwise the vertex is offset uniformly by `uniform`.
pub fn extruded_top_point(
    doc: &Document,
    extrusion: &Extrusion,
    dir: Vec3,
    v: Vec3,
    uniform: f32,
) -> Vec3 {
    if let Some((p, n)) = target_top_plane(doc, extrusion) {
        if let Some(t) = plane_axis_distance(v, dir, p, n) {
            return v + dir * t;
        }
    }
    v + dir * uniform
}

/// Start/end offsets along the normal for an extrusion of signed `distance` (#504).
/// Non-symmetric: `[0, distance]`. Symmetric (no target): `[-|d|/2, +|d|/2]` with the
/// sign of `distance` applied so flipping the gizmo still flips the axis.
pub fn extrusion_end_offsets(_doc: &Document, extrusion: &Extrusion, distance: f32) -> (f32, f32) {
    if extrusion.symmetric && extrusion.target.is_none() {
        let half = distance.abs() * 0.5;
        let sign = if distance < 0.0 { -1.0 } else { 1.0 };
        (-half * sign, half * sign)
    } else {
        (0.0, distance)
    }
}

/// Base-plane point for a profile vertex under the current extrusion (symmetric shifts
/// the start off the sketch plane).
pub fn extruded_base_point(
    doc: &Document,
    extrusion: &Extrusion,
    dir: Vec3,
    v: Vec3,
    distance: f32,
) -> Vec3 {
    let (start, _) = extrusion_end_offsets(doc, extrusion, distance);
    v + dir * start
}

/// Free-end (top) point for a profile vertex — symmetric ends at `+|d|/2`; otherwise
/// the same as [`extruded_top_point`] (including slanted targets).
pub fn extruded_free_end_point(
    doc: &Document,
    extrusion: &Extrusion,
    dir: Vec3,
    v: Vec3,
    distance: f32,
) -> Vec3 {
    let (_, end) = extrusion_end_offsets(doc, extrusion, distance);
    if extrusion.symmetric && extrusion.target.is_none() {
        v + dir * end
    } else {
        extruded_top_point(doc, extrusion, dir, v, end)
    }
}

/// Angle taper is limited to (−90°, 89°]. −90° collapses immediately (no solid).
pub const TAPER_ANGLE_MIN_DEG: f32 = -90.0;
/// Hard ceiling: 90° and above are not a draft (tan blows up).
pub const TAPER_ANGLE_MAX_DEG: f32 = 89.0;
/// Max per-side end-face offset (mm). 10 m is huge for a part but still tessellates.
pub const TAPER_MAX_OFFSET_MM: f32 = 10_000.0;

/// Result of [`clamp_extrude_taper`]: the value to store, and an optional warning.
#[derive(Clone, Debug, PartialEq)]
pub struct TaperClamp {
    pub value: f32,
    pub warning: Option<String>,
}

/// Tightest angle (degrees) a taper may use at this extrude height without exceeding
/// [`TAPER_MAX_OFFSET_MM`]. Never above [`TAPER_ANGLE_MAX_DEG`].
pub fn taper_angle_max_for_height(height_abs: f32) -> f32 {
    if height_abs <= 1e-6 {
        return TAPER_ANGLE_MAX_DEG;
    }
    let size_max = (TAPER_MAX_OFFSET_MM / height_abs).atan().to_degrees();
    size_max.min(TAPER_ANGLE_MAX_DEG)
}

/// Clamp an extrude taper so the angle stays in range and the solid stays a reasonable size.
///
/// Angle mode: [−90°, 89°], then further reduced so `height * tan(angle)` ≤ [`TAPER_MAX_OFFSET_MM`].
/// Distance mode: the per-side offset is capped at [`TAPER_MAX_OFFSET_MM`].
pub fn clamp_extrude_taper(
    taper: f32,
    mode: ExtrudeTaperMode,
    height_abs: f32,
) -> TaperClamp {
    match mode {
        ExtrudeTaperMode::Distance => {
            if taper > TAPER_MAX_OFFSET_MM {
                TaperClamp {
                    value: TAPER_MAX_OFFSET_MM,
                    warning: Some(format!(
                        "Taper would make a huge solid; limited to {} mm",
                        TAPER_MAX_OFFSET_MM as i32
                    )),
                }
            } else {
                TaperClamp {
                    value: taper,
                    warning: None,
                }
            }
        }
        ExtrudeTaperMode::Angle => {
            let mut v = taper;
            let mut warning = None;
            if v > TAPER_ANGLE_MAX_DEG {
                warning = Some("Taper angle is limited to 89°".to_string());
                v = TAPER_ANGLE_MAX_DEG;
            } else if v < TAPER_ANGLE_MIN_DEG {
                warning = Some("Taper angle is limited to -90°".to_string());
                v = TAPER_ANGLE_MIN_DEG;
            }
            if v > 0.0 && height_abs > 1e-6 {
                let max_angle = taper_angle_max_for_height(height_abs);
                if v > max_angle + 1e-3 {
                    warning = Some(format!(
                        "Taper would make a huge solid; limited to {max_angle:.1}°"
                    ));
                    v = max_angle;
                }
            }
            TaperClamp {
                value: v,
                warning,
            }
        }
    }
}

/// In-plane solid-growth offset (mm) for a free end of height `|h|` under this extrusion's
/// taper (#1243). Positive grows the end face; negative shrinks it.
pub fn taper_offset_at_height(extrusion: &Extrusion, height_abs: f32) -> f32 {
    match extrusion.taper_mode {
        ExtrudeTaperMode::Distance => extrusion.taper.min(TAPER_MAX_OFFSET_MM),
        ExtrudeTaperMode::Angle => {
            // Keep tan finite. −90° is handled as an immediate collapse in [`taper_end_plan`].
            let deg = extrusion.taper.clamp(-89.0, TAPER_ANGLE_MAX_DEG);
            let offset = height_abs * deg.to_radians().tan();
            offset.clamp(-TAPER_MAX_OFFSET_MM, TAPER_MAX_OFFSET_MM)
        }
    }
}

/// Signed area of a 2D loop (positive = CCW).
fn uv_signed_area(poly: &[(f32, f32)]) -> f32 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..poly.len() {
        let (x1, y1) = poly[i];
        let (x2, y2) = poly[(i + 1) % poly.len()];
        a += x1 * y2 - x2 * y1;
    }
    a * 0.5
}

fn uv_centroid(poly: &[(f32, f32)]) -> (f32, f32) {
    if poly.is_empty() {
        return (0.0, 0.0);
    }
    let n = poly.len() as f32;
    let (sx, sy) = poly.iter().fold((0.0, 0.0), |(sx, sy), &(u, v)| (sx + u, sy + v));
    (sx / n, sy / n)
}

/// Offset a closed UV loop by `d` millimetres (positive grows the enclosed area). When the
/// offset would invert or collapse the loop, returns the centroid as a degenerate triangle
/// so callers can loft to a point without inventing inverted geometry (#1243).
pub fn offset_uv_loop(poly: &[(f32, f32)], d: f32) -> Vec<(f32, f32)> {
    if poly.len() < 3 {
        return poly.to_vec();
    }
    if d.abs() < 1e-9 {
        return poly.to_vec();
    }
    let orig_area = uv_signed_area(poly).abs();
    if orig_area < 1e-8 {
        let c = uv_centroid(poly);
        return vec![c, c, c];
    }
    let sources: Vec<crate::offset::OffsetSource> = (0..poly.len())
        .map(|i| {
            let (au, av) = poly[i];
            let (bu, bv) = poly[(i + 1) % poly.len()];
            crate::offset::OffsetSource {
                // Synthetic ids — offset only needs them to order outputs; they are not
                // live document lines.
                id: crate::arena::Key::from_bits((i as u64) << 32),
                a: Vec2::new(au, av),
                b: Vec2::new(bu, bv),
                bezier: None,
            }
        })
        .collect();
    let out = crate::offset::offset_segments(&sources, d);
    if out.len() != poly.len() {
        let c = uv_centroid(poly);
        return vec![c, c, c];
    }
    let result: Vec<(f32, f32)> = out.iter().map(|s| (s.a.x, s.a.y)).collect();
    let new_area = uv_signed_area(&result).abs();
    // Collapsed or inverted: don't ship inverted geometry (#1243).
    if new_area < 1e-6 || new_area > orig_area * 50.0 && d < 0.0 {
        let c = uv_centroid(poly);
        return vec![c, c, c];
    }
    // Inward offset that flipped winding counts as collapsed.
    if d < 0.0 && uv_signed_area(&result).signum() != uv_signed_area(poly).signum() && new_area > 1e-6
    {
        let c = uv_centroid(poly);
        return vec![c, c, c];
    }
    result
}

/// Largest inward offset (positive number) the outer loop tolerates before collapsing to a
/// point — used to cut the extrude height under a negative angle taper (#1243).
pub fn max_inward_offset_uv(poly: &[(f32, f32)]) -> f32 {
    if poly.len() < 3 {
        return 0.0;
    }
    // Binary-search the largest |d| whose offset still has positive area.
    let mut lo = 0.0f32;
    let mut hi = {
        // Upper bound: half the larger bbox side.
        let (mut min_u, mut max_u) = (f32::MAX, f32::MIN);
        let (mut min_v, mut max_v) = (f32::MAX, f32::MIN);
        for &(u, v) in poly {
            min_u = min_u.min(u);
            max_u = max_u.max(u);
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
        ((max_u - min_u).max(max_v - min_v) * 0.5).max(1e-3)
    };
    // Confirm hi collapses; if not, grow it.
    for _ in 0..8 {
        let o = offset_uv_loop(poly, -hi);
        if uv_signed_area(&o).abs() < 1e-5 {
            break;
        }
        hi *= 2.0;
        if hi > 1.0e6 {
            break;
        }
    }
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        let o = offset_uv_loop(poly, -mid);
        if uv_signed_area(&o).abs() < 1e-5 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    lo
}

/// Whether a UV loop is effectively a single point (collapsed taper end).
fn uv_loop_is_point(poly: &[(f32, f32)]) -> bool {
    if poly.is_empty() {
        return true;
    }
    let c = uv_centroid(poly);
    poly.iter()
        .all(|&(u, v)| (u - c.0).hypot(v - c.1) < 1e-4)
}

/// Plan base/top in-plane solid offsets and the (possibly shortened) end offsets along the
/// normal for one face under taper (#1243).
///
/// Returns `(start, end, base_uv_offset, top_uv_offset)` where the UV offsets are solid-growth
/// amounts applied to the outer loop (holes use the opposite sign).
///
/// For a **symmetric** extrude the free ends both get the taper offset (cap size); the solid
/// body itself is built as two half-extrusions so the sketch mid-plane stays the profile size
/// (#1268) — this plan alone is for end caps / rings, not a single base→top loft of the body.
fn taper_end_plan(
    doc: &Document,
    extrusion: &Extrusion,
    face: &ExtrudeFace,
    distance: f32,
) -> (f32, f32, f32, f32) {
    let (start0, end0) = extrusion_end_offsets(doc, extrusion, distance);
    let taper = extrusion.taper;
    if taper.abs() < 1e-12 {
        return (start0, end0, 0.0, 0.0);
    }
    let half = |h: f32| h.abs();
    // Per free end: distance from the sketch plane to that end.
    let base_h = half(start0);
    let top_h = half(end0);
    let mut base_off = if extrusion.symmetric && extrusion.target.is_none() {
        taper_offset_at_height(extrusion, base_h)
    } else {
        0.0 // non-symmetric start face stays the profile
    };
    let mut top_off = taper_offset_at_height(extrusion, top_h);

    // Angle taper: cut height when a negative draft would collapse before the free end.
    if extrusion.taper_mode == ExtrudeTaperMode::Angle && taper < 0.0 {
        let sketch = crate::actions::extrude_face_sketch(doc, face);
        let region = sketch.and_then(|s| extrude_face_uv_region(doc, s, face));
        if let Some(region) = region {
            let max_in = max_inward_offset_uv(&region.outer);
            let tan = (-taper).clamp(0.0, TAPER_ANGLE_MAX_DEG).to_radians().tan().max(1e-9);
            let max_h = max_in / tan;
            // Shorten each free end that would overshoot.
            if extrusion.symmetric && extrusion.target.is_none() {
                let h = max_h.min(half(end0 - start0) * 0.5);
                let sign = if distance < 0.0 { -1.0 } else { 1.0 };
                let start = -h * sign;
                let end = h * sign;
                base_off = taper_offset_at_height(extrusion, h);
                top_off = base_off;
                return (start, end, base_off, top_off);
            }
            // Only the free end moves; collapse height measured from the start plane.
            let dir = if end0 >= start0 { 1.0 } else { -1.0 };
            let full = (end0 - start0).abs();
            let h = max_h.min(full);
            let end = start0 + dir * h;
            top_off = taper_offset_at_height(extrusion, h);
            return (start0, end, base_off, top_off);
        }
    }
    // Distance mode: clamp offset so the end doesn't invert (stays a point at worst).
    if let Some(sketch) = crate::actions::extrude_face_sketch(doc, face) {
        if let Some(region) = extrude_face_uv_region(doc, sketch, face) {
            let max_in = max_inward_offset_uv(&region.outer);
            if base_off < -max_in {
                base_off = -max_in;
            }
            if top_off < -max_in {
                top_off = -max_in;
            }
        }
    }
    (start0, end0, base_off, top_off)
}

/// UV outer (+ holes) at a given solid-growth offset for `face` (#1243).
fn tapered_uv_region(
    doc: &Document,
    face: &ExtrudeFace,
    solid_offset: f32,
) -> Option<(Vec<(f32, f32)>, Vec<Vec<(f32, f32)>>)> {
    let sketch = crate::actions::extrude_face_sketch(doc, face)?;
    let region = extrude_face_uv_region(doc, sketch, face)?;
    if solid_offset.abs() < 1e-9 {
        return Some((region.outer, region.holes));
    }
    // Circles: exact radius change (add to radius, not diameter) — cleaner than faceted offset.
    if let ExtrudeFace::Circle(i) = face {
        if region.holes.is_empty() {
            let c = doc.circles.get(*i)?;
            let r = (c.r + solid_offset).max(0.0);
            let (cx, cy) = (c.cx, c.cy);
            if r < 1e-6 {
                return Some((vec![(cx, cy), (cx, cy), (cx, cy)], Vec::new()));
            }
            let outer = (0..CIRCLE_SEGMENTS)
                .map(|k| {
                    let a = k as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                    (cx + r * a.cos(), cy + r * a.sin())
                })
                .collect();
            return Some((outer, Vec::new()));
        }
    }
    let outer = offset_uv_loop(&region.outer, solid_offset);
    // Holes move the opposite way: solid growth shrinks holes; solid shrink grows them.
    let mut holes: Vec<Vec<(f32, f32)>> = region
        .holes
        .iter()
        .map(|h| offset_uv_loop(h, -solid_offset))
        .filter(|h| !uv_loop_is_point(h) && uv_signed_area(h).abs() > 1e-6)
        // Keep only holes that still sit inside the outer boundary.
        .filter(|h| loop_strictly_inside(h, &outer) || uv_loop_is_point(&outer))
        .collect();
    // When holes grow into each other under a negative taper, merge pairwise (#1243).
    if solid_offset < 0.0 && holes.len() > 1 {
        let mut merged: Vec<Vec<(f32, f32)>> = Vec::new();
        for h in holes {
            let mut absorbed = false;
            for m in merged.iter_mut() {
                if loops_overlap(&h, m) {
                    if let Some(u) = crate::polygon_boolean::face_boolean(
                        m,
                        &h,
                        crate::model::BooleanOp::Intersection,
                    ) {
                        // Union of hole areas ≈ not of intersection of solids; use difference
                        // of nothing… fall back: keep the larger loop when they overlap hard.
                        let _ = u;
                    }
                    // Prefer the larger hole when they overlap (joined void).
                    if uv_signed_area(&h).abs() > uv_signed_area(m).abs() {
                        *m = h.clone();
                    }
                    absorbed = true;
                    break;
                }
            }
            if !absorbed {
                merged.push(h);
            }
        }
        holes = merged;
    }
    Some((outer, holes))
}

/// Project a UV loop to world at a given axial offset along `normal` from the sketch origin
/// of each point (points already include any in-plane taper).
fn uv_loop_at_height(
    frame: &SketchFrame,
    loop_uv: &[(f32, f32)],
    normal: Vec3,
    axial: f32,
) -> Vec<Vec3> {
    loop_uv
        .iter()
        .map(|&(u, v)| local_to_world(frame, u, v) + normal * axial)
        .collect()
}

/// Base and free-end loops for a profile face under this extrusion's distance/target/taper.
fn extrusion_profile_rings(
    doc: &Document,
    extrusion: &Extrusion,
    face: &ExtrudeFace,
    distance: f32,
) -> Option<(Vec<Vec3>, Vec<Vec3>, Vec3)> {
    // Fast path: no taper — preserve previous world-space projection (handles slanted targets).
    if extrusion.taper.abs() < 1e-12 {
        let (profile0, n) = face_profile_world(doc, face)?;
        if profile0.len() < 3 {
            return None;
        }
        let base: Vec<Vec3> = profile0
            .iter()
            .map(|p| extruded_base_point(doc, extrusion, n, *p, distance))
            .collect();
        let top: Vec<Vec3> = profile0
            .iter()
            .map(|p| extruded_free_end_point(doc, extrusion, n, *p, distance))
            .collect();
        return Some((base, top, n));
    }

    let sketch = crate::actions::extrude_face_sketch(doc, face)?;
    let frame = sketch_geometry_frame(doc, sketch)?;
    let normal = frame.normal;
    let (start, end, base_off, top_off) = taper_end_plan(doc, extrusion, face, distance);

    let (base_uv, _) = tapered_uv_region(doc, face, base_off)?;
    let (top_uv, _) = tapered_uv_region(doc, face, top_off)?;
    // Match vertex counts for loft: if one end collapsed to a point, replicate the centroid
    // to the other end's vertex count so side walls form a pyramid.
    let (base_uv, top_uv) = match (uv_loop_is_point(&base_uv), uv_loop_is_point(&top_uv)) {
        (true, false) => {
            let c = uv_centroid(&base_uv);
            (vec![c; top_uv.len().max(3)], top_uv)
        }
        (false, true) => {
            let c = uv_centroid(&top_uv);
            (base_uv.clone(), vec![c; base_uv.len().max(3)])
        }
        (true, true) => {
            let c = uv_centroid(&base_uv);
            (vec![c; 3], vec![c; 3])
        }
        (false, false) => {
            if base_uv.len() == top_uv.len() {
                (base_uv, top_uv)
            } else {
                let n = base_uv.len().max(top_uv.len()).max(3);
                (resample_uv_loop(&base_uv, n), resample_uv_loop(&top_uv, n))
            }
        }
    };

    let base = uv_loop_at_height(&frame, &base_uv, normal, start);
    let top = uv_loop_at_height(&frame, &top_uv, normal, end);
    Some((base, top, normal))
}

/// Evenly resample a closed UV loop to `n` vertices (linear along edges).
fn resample_uv_loop(poly: &[(f32, f32)], n: usize) -> Vec<(f32, f32)> {
    if poly.len() < 2 || n < 3 {
        return poly.to_vec();
    }
    let mut edge_lens = Vec::with_capacity(poly.len());
    let mut total = 0.0f32;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let len = (a.0 - b.0).hypot(a.1 - b.1);
        edge_lens.push(len);
        total += len;
    }
    if total < 1e-9 {
        let c = uv_centroid(poly);
        return vec![c; n];
    }
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        let mut t = total * (k as f32 / n as f32);
        for i in 0..poly.len() {
            let len = edge_lens[i];
            if t <= len || i + 1 == poly.len() {
                let f = if len > 1e-9 { t / len } else { 0.0 };
                let a = poly[i];
                let b = poly[(i + 1) % poly.len()];
                out.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
                break;
            }
            t -= len;
        }
    }
    out
}

/// Base/top hole loops for a face under taper, parallel to [`extrusion_profile_rings`].
fn extrusion_hole_rings(
    doc: &Document,
    extrusion: &Extrusion,
    face: &ExtrudeFace,
    distance: f32,
) -> Vec<(Vec<Vec3>, Vec<Vec3>)> {
    let Some(sketch) = crate::actions::extrude_face_sketch(doc, face) else {
        return Vec::new();
    };
    let Some(frame) = sketch_geometry_frame(doc, sketch) else {
        return Vec::new();
    };
    let normal = frame.normal;
    let (start, end, base_off, top_off) = taper_end_plan(doc, extrusion, face, distance);
    let Some((_, base_holes)) = tapered_uv_region(doc, face, base_off) else {
        return Vec::new();
    };
    let Some((_, top_holes)) = tapered_uv_region(doc, face, top_off) else {
        return Vec::new();
    };
    // Pair by index; drop holes that collapsed on either end alone by lofting to a point.
    let n = base_holes.len().max(top_holes.len());
    let mut out = Vec::new();
    for i in 0..n {
        let b = base_holes.get(i).cloned().unwrap_or_default();
        let t = top_holes.get(i).cloned().unwrap_or_default();
        if b.is_empty() && t.is_empty() {
            continue;
        }
        let (bu, tu) = match (b.is_empty() || uv_loop_is_point(&b), t.is_empty() || uv_loop_is_point(&t))
        {
            (true, true) => continue,
            (true, false) => {
                let c = uv_centroid(&t);
                (vec![c; t.len().max(3)], t)
            }
            (false, true) => {
                let c = uv_centroid(&b);
                (b.clone(), vec![c; b.len().max(3)])
            }
            (false, false) if b.len() == t.len() => (b, t),
            (false, false) => {
                let m = b.len().max(t.len()).max(3);
                (resample_uv_loop(&b, m), resample_uv_loop(&t, m))
            }
        };
        let base = uv_loop_at_height(&frame, &bu, normal, start);
        let top = uv_loop_at_height(&frame, &tu, normal, end);
        out.push((base, top));
    }
    out
}

/// The effective signed depth: derived from `target`'s extended plane when set, else `distance`.
/// For a symmetric extrusion this is still the *total* height (end-to-end).
///
/// Under a negative **angle** taper that collapses the profile before the typed depth, the
/// returned value is the shortened solid height (#1243).
pub fn effective_distance(doc: &Document, extrusion: &Extrusion) -> f32 {
    let raw = if let Some(target) = &extrusion.target {
        if let Some((base, normal)) = faces_anchor(doc, &extrusion.faces) {
            if let Some(d) = target_distance(doc, base, normal, target) {
                d
            } else {
                extrusion.distance
            }
        } else {
            extrusion.distance
        }
    } else {
        extrusion.distance
    };
    // Angle collapse shortens the solid — report the actual height of the first face.
    if extrusion.taper_mode == ExtrudeTaperMode::Angle
        && extrusion.taper < 0.0
        && extrusion.target.is_none()
    {
        if let Some(face) = extrusion.faces.first() {
            let (start, end, _, _) = taper_end_plan(doc, extrusion, face, raw);
            let sign = if raw < 0.0 { -1.0 } else { 1.0 };
            return (end - start).abs() * sign;
        }
    }
    raw
}

/// Signed distance along `normal` from `base` to where the axis reaches `target`'s plane.
pub fn target_distance(
    doc: &Document,
    base: Vec3,
    normal: Vec3,
    target: &ExtrudeTarget,
) -> Option<f32> {
    match target {
        ExtrudeTarget::Vertex(point) => {
            let world = constraint_point_world(doc, point.clone())?;
            Some((world - base).dot(normal))
        }
        ExtrudeTarget::Face(face) => {
            let (p, n) = face_plane(doc, face)?;
            plane_axis_distance(base, normal, p, n)
        }
        ExtrudeTarget::Plane(index) => {
            let plane = doc.construction_planes.get(*index)?;
            plane_axis_distance(base, normal, plane.origin, plane.normal)
        }
        ExtrudeTarget::BodyFace(face_id) => {
            let (p, n) = body_face_plane(doc, face_id)?;
            plane_axis_distance(base, normal, p, n)
        }
        ExtrudeTarget::RepeatedFace { face, op, instance } => {
            let (p, n) = repeated_face_plane(doc, face, *op, *instance)?;
            plane_axis_distance(base, normal, p, n)
        }
    }
}

/// The plane of a repeated instance's face (#452): the source face's plane translated
/// along the repeat axis by that instance's offset.
pub fn repeated_face_plane(
    doc: &Document,
    face: &crate::model::FaceId,
    op: crate::model::RepeatOpKey,
    instance: usize,
) -> Option<(Vec3, Vec3)> {
    let rep = doc.repeat_ops.get(op)?;
    // `repeat_offsets` lists the copies only; instance 0 is the original body.
    let m = repeat_instance_transform(doc, rep, instance)?;
    let (p, n) = body_face_plane(doc, face)?;
    Some((m.transform_point3(p), m.transform_vector3(n).normalize_or_zero()))
}

/// Distance along `dir` from `base` to the plane (`point`, `plane_normal`).
fn plane_axis_distance(base: Vec3, dir: Vec3, point: Vec3, plane_normal: Vec3) -> Option<f32> {
    let denom = dir.dot(plane_normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    Some((point - base).dot(plane_normal) / denom)
}

fn face_plane(doc: &Document, face: &ExtrudeFace) -> Option<(Vec3, Vec3)> {
    let (center, normal) = face_center_world(doc, face)?;
    Some((center, normal))
}

pub fn constraint_point_world(doc: &Document, point: crate::model::ConstraintPoint) -> Option<Vec3> {
    // A face's own vertex is already a world-space point (#26/#27) — no sketch frame to
    // project through, unlike the other variants below.
    if let crate::model::ConstraintPoint::FaceVertex { face, index } = &point {
        return face_boundary_loop_world(doc, face)?.get(*index).copied();
    }
    let sketch = match &point {
        crate::model::ConstraintPoint::LineEndpoint { line, .. } => doc.lines.get(*line)?.sketch,
        crate::model::ConstraintPoint::CircleCenter(circle) => doc.circles.get(*circle)?.sketch,
        crate::model::ConstraintPoint::TextAnchor { text, .. } => {
            doc.sketch_texts.get(*text)?.sketch
        }
        crate::model::ConstraintPoint::ImageCalibrationPoint { image, index } => {
            // The image lives on a plane, not in a sketch: resolve directly in world space.
            let img = doc.tracing_images.get(*image)?;
            let (u, v) = crate::model::image_calibration_point_uv(img, *index)?;
            let frame = crate::face::sketch_frame(
                doc,
                crate::model::FaceId::ConstructionPlane(img.plane),
            )?;
            return Some(frame.origin + frame.u_axis * u + frame.v_axis * v);
        }
        crate::model::ConstraintPoint::FaceVertex { .. } => unreachable!("handled above"),
    };
    let frame = sketch_geometry_frame(doc, sketch)?;
    let (u, v) = point_uv(doc, sketch, point).ok()?;
    Some(local_to_world(&frame, u, v))
}

/// Gizmo anchor for a set of coplanar faces: the centroid of their centers and the plane
/// normal (the extrusion direction).
pub fn faces_anchor(doc: &Document, faces: &[ExtrudeFace]) -> Option<(Vec3, Vec3)> {
    let mut sum = Vec3::ZERO;
    let mut count = 0u32;
    let mut normal = Vec3::ZERO;
    for face in faces {
        if let Some(center) = face_center_world(doc, face) {
            sum += center.0;
            normal = center.1;
            count += 1;
        }
    }
    (count > 0).then(|| (sum / count as f32, normal))
}

/// World center and normal of a face.
fn face_center_world(doc: &Document, face: &ExtrudeFace) -> Option<(Vec3, Vec3)> {
    match face {
        ExtrudeFace::Circle(i) => {
            let circle = doc.circles.get(*i)?;
            let frame = sketch_geometry_frame(doc, circle.sketch)?;
            Some((local_to_world(&frame, circle.cx, circle.cy), frame.normal))
        }
        ExtrudeFace::Polygon(lines) => {
            let (profile, normal) = polygon_profile_world(doc, lines)?;
            let centroid = profile.iter().copied().sum::<Vec3>() / profile.len() as f32;
            Some((centroid, normal))
        }
        ExtrudeFace::Boolean { .. }
        | ExtrudeFace::TextGlyph { .. }
        | ExtrudeFace::SketchRegion { .. } => {
            let (profile, normal) = face_profile_world(doc, face)?;
            let centroid = profile.iter().copied().sum::<Vec3>() / profile.len() as f32;
            Some((centroid, normal))
        }
    }
}

/// One sketch-text glyph region (#285) in the sketch's UV frame: the glyph's outer loop and its
/// hole loops, already placed by the text's `origin`/`rotation`. `None` if the text or glyph is
/// missing.
fn text_glyph_region_uv(
    doc: &Document,
    text_index: crate::model::SketchTextKey,
    glyph_index: usize,
) -> Option<(Vec<(f32, f32)>, Vec<Vec<(f32, f32)>>)> {
    let t = doc.sketch_texts.get(text_index)?;
    let regions = crate::text::group_glyphs(&t.contours);
    let region = regions.get(glyph_index)?;
    let (sin, cos) = t.rotation.sin_cos();
    let xf = |&(x, y): &(f32, f32)| {
        (x * cos - y * sin + t.origin.0, x * sin + y * cos + t.origin.1)
    };
    let outer: Vec<(f32, f32)> = region.outer.iter().map(xf).collect();
    let holes: Vec<Vec<(f32, f32)>> =
        region.holes.iter().map(|h| h.iter().map(xf).collect()).collect();
    Some((outer, holes))
}

/// World-space boundary loop (CCW in the face frame) and outward normal of a face.
/// Split an extrude's faces into the solids they actually make (#837): profiles that touch —
/// one nested inside another (a hole in its own wall) or overlapping — belong to one solid;
/// profiles sharing nothing are separate solids. Faces whose profile no longer resolves are
/// kept in the first group so nothing is silently dropped.
pub fn disjoint_face_groups(doc: &Document, faces: &[ExtrudeFace]) -> Vec<Vec<ExtrudeFace>> {
    let profiles: Vec<Option<(Vec<Vec3>, Vec3)>> = faces
        .iter()
        .map(|f| face_profile_world(doc, f))
        .collect();
    // Union-find over "these two profiles touch".
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let root = find(parent, parent[i]);
            parent[i] = root;
        }
        parent[i]
    }
    for i in 0..faces.len() {
        for j in (i + 1)..faces.len() {
            // Every glyph of one sketch text is part of the same label, however far apart
            // the letters sit — a label extrudes as one thing.
            let same_text = matches!(
                (&faces[i], &faces[j]),
                (
                    ExtrudeFace::TextGlyph { text: a, .. },
                    ExtrudeFace::TextGlyph { text: b, .. },
                ) if a == b
            );
            let touch = same_text
                || match (&profiles[i], &profiles[j]) {
                    (Some((a, normal)), Some((b, _))) => profiles_touch(a, b, *normal),
                    // An unresolvable profile joins its neighbours rather than splitting off.
                    _ => true,
                };
            if touch {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut groups: Vec<(usize, Vec<ExtrudeFace>)> = Vec::new();
    for (i, face) in faces.iter().enumerate() {
        let root = find(&mut parent, i);
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, set)) => set.push(face.clone()),
            None => groups.push((root, vec![face.clone()])),
        }
    }
    groups.into_iter().map(|(_, set)| set).collect()
}

/// Whether two coplanar profiles share any area or boundary: either encloses a vertex of the
/// other, or their edges cross.
fn profiles_touch(a: &[Vec3], b: &[Vec3], normal: Vec3) -> bool {
    let (u, v) = crate::construction::plane_basis(normal.normalize_or_zero());
    let flat = |p: &[Vec3]| -> Vec<(f32, f32)> {
        p.iter().map(|w| (w.dot(u), w.dot(v))).collect()
    };
    let (a, b) = (flat(a), flat(b));
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    if a.iter().any(|p| crate::polygon::point_in_polygon_2d(*p, &b))
        || b.iter().any(|p| crate::polygon::point_in_polygon_2d(*p, &a))
    {
        return true;
    }
    for i in 0..a.len() {
        let a0 = a[i];
        let a1 = a[(i + 1) % a.len()];
        for j in 0..b.len() {
            let b0 = b[j];
            let b1 = b[(j + 1) % b.len()];
            if segments_cross(a0, a1, b0, b1) {
                return true;
            }
        }
    }
    false
}

fn segments_cross(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> bool {
    let cross = |o: (f32, f32), p: (f32, f32), q: (f32, f32)| {
        (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
    };
    let d1 = cross(a, b, c);
    let d2 = cross(a, b, d);
    let d3 = cross(c, d, a);
    let d4 = cross(c, d, b);
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

pub fn face_profile_world(doc: &Document, face: &ExtrudeFace) -> Option<(Vec<Vec3>, Vec3)> {
    match face {
        ExtrudeFace::Circle(index) => {
            let circle = doc.circles.get(*index)?;
            let frame = sketch_geometry_frame(doc, circle.sketch)?;
            let profile = circle_profile_world(&frame, circle.cx, circle.cy, circle.r);
            Some((profile, frame.normal))
        }
        ExtrudeFace::Polygon(lines) => polygon_profile_world(doc, lines),
        ExtrudeFace::Boolean { .. } => boolean_profile_world(doc, face),
        ExtrudeFace::TextGlyph { text, glyph } => {
            let sketch = doc.sketch_texts.get(*text)?.sketch;
            let frame = sketch_geometry_frame(doc, sketch)?;
            let (outer, _holes) = text_glyph_region_uv(doc, *text, *glyph)?;
            let profile = outer.into_iter().map(|(u, v)| local_to_world(&frame, u, v)).collect();
            Some((profile, frame.normal))
        }
        // A plane region (#993) is recomputed from the live sketch every time, so it follows
        // edits like any other profile — and stops resolving if the cuts move out from under it.
        ExtrudeFace::SketchRegion { sketch, .. } => {
            let frame = sketch_geometry_frame(doc, *sketch)?;
            let region = sketch_region_uv(doc, face)?;
            let profile = region
                .into_iter()
                .map(|(u, v)| local_to_world(&frame, u, v))
                .collect();
            Some((profile, frame.normal))
        }
    }
}

/// World-space boundary loop and outward normal of a `Boolean`-combined face (#16/#62):
/// resolves `a`/`b`'s loops in their shared sketch's UV frame (recursively, in case they're
/// themselves `Boolean`), runs [`crate::polygon_boolean::face_boolean`] (OCCT in kernel
/// builds, #88), and projects the resulting loop back to world space through that same
/// frame. `None` if the sketch/frame can't be resolved, or the boolean result isn't a single
/// simple polygon loop (see `polygon_boolean`'s module docs for the deliberate scope limits).
fn boolean_profile_world(doc: &Document, face: &ExtrudeFace) -> Option<(Vec<Vec3>, Vec3)> {
    let sketch = crate::actions::extrude_face_sketch(doc, face)?;
    let frame = sketch_geometry_frame(doc, sketch)?;
    // Use the region resolver so an annulus (concentric-ring) face resolves to its outer
    // boundary rather than being rejected (#268). Callers wanting the hole loops use
    // [`face_region_world`]; this outer loop is what picking, targets, and validation need.
    let region = extrude_face_uv_region(doc, sketch, face)?;
    let profile = region.outer.into_iter().map(|(u, v)| local_to_world(&frame, u, v)).collect();
    Some((profile, frame.normal))
}

/// The boundary loop of `face`, in `sketch`'s local UV frame (not world space) — used for the
/// 2D polygon-boolean overlap detection and click resolution in [`overlapping_partner`] and
/// [`resolve_boolean_click`] (#16/#62), and to build [`boolean_profile_world`]. `None` if
/// `face` doesn't belong to `sketch`, its underlying geometry is missing/deleted, or (for
/// `Boolean`) the combination doesn't reduce to a single simple loop.
pub fn extrude_face_uv_loop(
    doc: &Document,
    sketch: crate::model::SketchId,
    face: &ExtrudeFace,
) -> Option<Vec<(f32, f32)>> {
    match face {
        ExtrudeFace::Circle(i) => {
            let circle = doc.circles.get(*i)?;
            if circle.sketch != sketch {
                return None;
            }
            Some(
                (0..CIRCLE_SEGMENTS)
                    .map(|k| {
                        let a = k as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                        (circle.cx + circle.r * a.cos(), circle.cy + circle.r * a.sin())
                    })
                    .collect(),
            )
        }
        ExtrudeFace::SketchRegion { sketch: s, .. } => {
            // A plane region (#993) is already a sketch-local loop.
            (*s == sketch).then(|| sketch_region_uv(doc, face)).flatten()
        }
        ExtrudeFace::Polygon(lines) => {
            let first = doc.lines.get(*lines.first()?)?;
            if first.sketch != sketch {
                return None;
            }
            crate::polygon::loop_vertices_uv(doc, sketch, lines)
        }
        ExtrudeFace::Boolean { op, a, b } => {
            let loop_a = extrude_face_uv_loop(doc, sketch, a)?;
            let loop_b = extrude_face_uv_loop(doc, sketch, b)?;
            crate::polygon_boolean::face_boolean(&loop_a, &loop_b, *op)
        }
        ExtrudeFace::TextGlyph { text, glyph } => {
            if doc.sketch_texts.get(*text)?.sketch != sketch {
                return None;
            }
            text_glyph_region_uv(doc, *text, *glyph).map(|(outer, _)| outer)
        }
    }
}

/// A sketch face resolved to a *fillable region* (#268/#263): one outer boundary loop plus zero
/// or more interior **hole** loops. A plain rect/circle/polygon is a hole-free region; a
/// `Boolean { Difference }` whose subtrahend lies strictly inside the minuend is an **annulus** —
/// the minuend's loop as `outer` with the subtrahend's loop as a `hole`. Coordinates are in the
/// same space (UV here) as the inputs. This is what the mesh and kernel builders consume so a
/// concentric-ring profile becomes a true face-with-hole instead of being rejected (as the
/// single-loop [`extrude_face_uv_loop`] does for annuli).
#[derive(Clone, Debug, PartialEq)]
pub struct UvRegion {
    pub outer: Vec<(f32, f32)>,
    pub holes: Vec<Vec<(f32, f32)>>,
}

/// Resolve `face` into a [`UvRegion`] (outer loop + hole loops) in `sketch`'s UV frame.
/// Everything reduces to a single outer loop except a difference with a strictly-contained
/// subtrahend, which yields a hole. Nested holes compose (a difference of an already-holed
/// region keeps its holes and adds the new one).
pub fn extrude_face_uv_region(
    doc: &Document,
    sketch: crate::model::SketchId,
    face: &ExtrudeFace,
) -> Option<UvRegion> {
    // A text glyph carries its own outer + counters (holes) directly (#285).
    if let ExtrudeFace::TextGlyph { text, glyph } = face {
        if doc.sketch_texts.get(*text)?.sketch != sketch {
            return None;
        }
        let (outer, holes) = text_glyph_region_uv(doc, *text, *glyph)?;
        return Some(UvRegion { outer, holes });
    }
    if let ExtrudeFace::Boolean { op: crate::model::BooleanOp::Difference, a, b } = face {
        let region_a = extrude_face_uv_region(doc, sketch, a)?;
        if let Some(loop_b) = extrude_face_uv_loop(doc, sketch, b) {
            // The subtrahend is a clean hole only when it sits strictly inside the minuend's
            // outer boundary and clear of any existing hole; otherwise it's a boundary-crossing
            // difference, handled by the single-loop boolean below.
            if loop_strictly_inside(&loop_b, &region_a.outer)
                && region_a
                    .holes
                    .iter()
                    .all(|h| !loops_overlap(&loop_b, h))
            {
                let mut holes = region_a.holes;
                holes.push(loop_b);
                return Some(UvRegion { outer: region_a.outer, holes });
            }
        }
    }
    // Non-annulus faces (raw shapes, unions/intersections, crossing differences) reduce to a
    // single hole-free loop.
    let outer = extrude_face_uv_loop(doc, sketch, face)?;
    Some(UvRegion { outer, holes: Vec::new() })
}

/// True when every vertex of `inner` lies inside the `outer` polygon — a sufficient test for
/// "strictly contained" given both loops are simple and non-touching (the annulus case).
fn loop_strictly_inside(inner: &[(f32, f32)], outer: &[(f32, f32)]) -> bool {
    !inner.is_empty()
        && inner
            .iter()
            .all(|&p| crate::polygon::point_in_polygon_2d(p, outer))
}

/// Loose overlap test between two loops: any vertex of one inside the other. Used to keep a new
/// hole from landing on top of an existing hole.
fn loops_overlap(x: &[(f32, f32)], y: &[(f32, f32)]) -> bool {
    x.iter().any(|&p| crate::polygon::point_in_polygon_2d(p, y))
        || y.iter().any(|&p| crate::polygon::point_in_polygon_2d(p, x))
}

/// World-space [`UvRegion`] for `face`: outer boundary + hole loops projected through the
/// sketch frame, plus the face normal. The hole-aware analogue of [`face_profile_world`]
/// (which returns only the outer loop). `None` if the sketch/frame or geometry can't resolve.
pub fn face_region_world(doc: &Document, face: &ExtrudeFace) -> Option<(Vec<Vec3>, Vec<Vec<Vec3>>, Vec3)> {
    let sketch = crate::actions::extrude_face_sketch(doc, face)?;
    let frame = sketch_geometry_frame(doc, sketch)?;
    let region = extrude_face_uv_region(doc, sketch, face)?;
    let outer = region.outer.iter().map(|&(u, v)| local_to_world(&frame, u, v)).collect();
    let holes = region
        .holes
        .iter()
        .map(|h| h.iter().map(|&(u, v)| local_to_world(&frame, u, v)).collect())
        .collect();
    Some((outer, holes, frame.normal))
}

/// Every raw (non-`Boolean`) extrude face belonging to `sketch`: each rect, circle, and
/// closed line-loop polygon (#66) whose owning sketch is `sketch`.
fn raw_faces_in_sketch(doc: &Document, sketch: crate::model::SketchId) -> Vec<ExtrudeFace> {
    let mut out = Vec::new();
    for (i, c) in doc.circles.iter() {
        if c.sketch == sketch {
            out.push(ExtrudeFace::Circle(i));
        }
    }
    for lines in crate::polygon::closed_line_loops(doc, sketch) {
        out.push(ExtrudeFace::Polygon(lines));
    }
    out
}

/// If exactly one other raw shape in `face`'s sketch has nonzero-area overlap with it — and no
/// third shape also overlaps that pair — that shape; else `None`. This is the "exactly two
/// overlapping shapes" gate for #16/#62's boolean-region click resolution (see scope note in
/// SPEC.md): a sketch with three or more mutually-overlapping shapes falls back to today's
/// whole-shape picking instead of attempting an N-way arrangement.
pub fn overlapping_partner(
    doc: &Document,
    sketch: crate::model::SketchId,
    face: &ExtrudeFace,
) -> Option<ExtrudeFace> {
    let loop_a = extrude_face_uv_loop(doc, sketch, face)?;
    let mut overlaps: Vec<ExtrudeFace> = Vec::new();
    for other in raw_faces_in_sketch(doc, sketch) {
        if &other == face {
            continue;
        }
        let Some(loop_b) = extrude_face_uv_loop(doc, sketch, &other) else {
            continue;
        };
        // `face_boolean`'s own near-zero-area rejection means `Some` here already implies
        // genuine, nonzero-area overlap — no separate area check needed.
        if crate::polygon_boolean::face_boolean(&loop_a, &loop_b, crate::model::BooleanOp::Intersection)
            .is_some()
        {
            overlaps.push(other);
            if overlaps.len() > 1 {
                return None;
            }
        }
    }
    (overlaps.len() == 1).then(|| overlaps.remove(0))
}

/// Resolve a click at local UV point `point` against `face` and its unique overlapping
/// `other` into the right atomic boolean region (#16/#62): inside both -> `Intersection`,
/// inside only one -> that one minus the other, inside neither -> `None` (falls back to
/// whole-shape picking of `face` itself).
pub fn resolve_boolean_click(
    doc: &Document,
    sketch: crate::model::SketchId,
    face: &ExtrudeFace,
    other: &ExtrudeFace,
    point: (f32, f32),
) -> Option<ExtrudeFace> {
    let loop_a = extrude_face_uv_loop(doc, sketch, face)?;
    let loop_b = extrude_face_uv_loop(doc, sketch, other)?;
    let in_a = crate::polygon::point_in_polygon_2d(point, &loop_a);
    let in_b = crate::polygon::point_in_polygon_2d(point, &loop_b);
    match (in_a, in_b) {
        (true, true) => Some(ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Intersection,
            a: Box::new(face.clone()),
            b: Box::new(other.clone()),
        }),
        (true, false) => Some(ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(face.clone()),
            b: Box::new(other.clone()),
        }),
        (false, true) => Some(ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(other.clone()),
            b: Box::new(face.clone()),
        }),
        (false, false) => None,
    }
}

/// World-space boundary loop and outward normal of a closed polygon, given its ordered
/// line indices (#66). `None` if any line is missing/deleted or the loop isn't closed.
fn polygon_profile_world(doc: &Document, lines: &[crate::model::LineKey]) -> Option<(Vec<Vec3>, Vec3)> {
    let first = doc.lines.get(*lines.first()?)?;
    if lines.iter().any(|&li| !doc.lines.contains(li)) {
        return None;
    }
    let frame = sketch_geometry_frame(doc, first.sketch)?;
    let vertices_uv = crate::polygon::loop_vertices_uv(doc, first.sketch, lines)?;
    let profile = vertices_uv
        .into_iter()
        .map(|(u, v)| local_to_world(&frame, u, v))
        .collect();
    Some((profile, frame.normal))
}

/// World-space boundary loop of an extrusion cap. `top` selects the free end;
/// otherwise the base end (sketch plane, or `−|d|/2` when symmetric, #504).
pub fn cap_polygon_world(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    profile: &ExtrudeFace,
    top: bool,
) -> Option<Vec<Vec3>> {
    let ext = doc.extrusions.get(extrusion)?;
    if !ext.faces.contains(profile) {
        return None;
    }
    let distance = effective_distance(doc, ext);
    let (base, free, _) = extrusion_profile_rings(doc, ext, profile, distance)?;
    Some(if top { free } else { base })
}

/// The hole loops of a cap face, in world space at the base/top cap position (#519). A
/// boolean-difference (inset border) or text-glyph face has an outer ring plus one or more
/// holes; the outer ring comes from [`cap_polygon_world`], and these are the openings inside
/// it. Empty for a plain simply-connected face. Each hole vertex is lifted to the cap by the
/// same base/free-end mapping [`cap_polygon_world`] applies to the outer ring, so the two
/// stay coplanar.
pub fn cap_hole_loops_world(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    profile: &ExtrudeFace,
    top: bool,
) -> Vec<Vec<Vec3>> {
    let Some(ext) = doc.extrusions.get(extrusion) else {
        return Vec::new();
    };
    if !ext.faces.contains(profile) {
        return Vec::new();
    }
    let distance = effective_distance(doc, ext);
    let Some((_, holes0, normal)) = face_region_world(doc, profile) else {
        return Vec::new();
    };
    holes0
        .into_iter()
        .map(|h| {
            h.into_iter()
                .map(|p| {
                    if top {
                        extruded_free_end_point(doc, ext, normal, p, distance)
                    } else {
                        extruded_base_point(doc, ext, normal, p, distance)
                    }
                })
                .collect()
        })
        .collect()
}

/// Number of flat, sketchable side walls of a profile (rectangles have 4, polygons have
/// one per edge; circular profiles are curved and have none).
/// True when `edge` names the circular cap rim of a Circle-profile face (#177): the one
/// continuous edge where a cylinder's wall meets its base/top cap. Rims are identified as
/// `Cap {{ edge: 0, top }}` — a circle profile has exactly one boundary "edge" per cap.
pub fn is_circle_cap_rim(face: &ExtrudeFace, edge: ExtrusionEdgeRef) -> bool {
    matches!(face, ExtrudeFace::Circle(_))
        && matches!(edge, ExtrusionEdgeRef::Cap { edge: 0, .. })
}

pub fn side_face_count(profile: &ExtrudeFace) -> usize {
    match profile {
        ExtrudeFace::Circle(_) => 0,
        ExtrudeFace::Polygon(lines) => lines.len(),
        // The resolved edge count depends on the boolean-clipped geometry (Document state),
        // which this function has no access to; sketching on a boolean-derived extrusion's
        // flat side walls isn't offered (documented limitation, mirrors `Circle`'s curved
        // walls above) — the extrusion mesh itself is unaffected (`extrusion_mesh` walks the
        // resolved profile loop directly, not through this count).
        ExtrudeFace::Boolean { .. } | ExtrudeFace::TextGlyph { .. } => 0,
        // A region's boundary is derived, not a fixed list of profile lines, so its side walls
        // are not analytically addressable — the same limitation `Boolean` has.
        ExtrudeFace::SketchRegion { .. } => 0,
    }
}

/// World-space quad of an extrusion side wall, swept by `edge` of a polygonal profile.
/// Ordered `[base_a, base_b, top_b, top_a]`. `None` for circular profiles, out-of-range
/// edges, or a deleted/foreign extrusion.
///
/// `edge` addresses the profile's lines **analytically** (#178): `edge` is a profile-line
/// index (`0..lines.len()`), so `edge` k is the flat wall of line k regardless of how the
/// curved lines between it are faceted. A curved line has no flat wall, so it resolves to
/// `None` — like a circular profile's curved wall. For an all-straight profile this is
/// identical to the old faceted addressing (each straight line is exactly one faceted edge).
pub fn side_quad_world(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    profile: &ExtrudeFace,
    edge: usize,
) -> Option<[Vec3; 4]> {
    let ext = doc.extrusions.get(extrusion)?;
    if !ext.faces.contains(profile) || edge >= side_face_count(profile) {
        return None;
    }
    let ExtrudeFace::Polygon(lines) = profile else {
        return None;
    };
    // A curved line's swept wall isn't a flat, sketchable face — skip it (mirrors circles).
    if doc.lines.get(*lines.get(edge)?)?.is_curved() {
        return None;
    }
    let first = doc.lines.get(*lines.first()?)?;
    let frame = sketch_geometry_frame(doc, first.sketch)?;
    let corners = crate::polygon::loop_corner_vertices_uv(doc, first.sketch, lines)?;
    let n = corners.len();
    if edge >= n {
        return None;
    }
    let a0 = local_to_world(&frame, corners[edge].0, corners[edge].1);
    let b0 = {
        let (u, v) = corners[(edge + 1) % n];
        local_to_world(&frame, u, v)
    };
    let normal = frame.normal;
    // Base/free ends follow symmetric offsets and (possibly slanted) targets (#504).
    let distance = effective_distance(doc, ext);
    let a = extruded_base_point(doc, ext, normal, a0, distance);
    let b = extruded_base_point(doc, ext, normal, b0, distance);
    let top_a = extruded_free_end_point(doc, ext, normal, a0, distance);
    let top_b = extruded_free_end_point(doc, ext, normal, b0, distance);
    Some([a, b, top_b, top_a])
}

/// Ordered world-space boundary loop of an extrusion-backed body face (#26/#27): dispatches to
/// [`cap_polygon_world`] for `FaceId::ExtrudeCap` and [`side_quad_world`] for
/// `FaceId::ExtrudeSide`, reusing the same analytic geometry sketch-on-face already relies on.
/// `None` for any other `FaceId` variant (construction planes, 2D shapes) — this only serves
/// extrusion body faces, and imported STL/STEP bodies have no `FaceId` of this shape at all.
pub fn face_boundary_loop_world(doc: &Document, face: &FaceId) -> Option<Vec<Vec3>> {
    match face {
        // A unit's flat face (#725): the inner face's loop, placed by the instance.
        FaceId::UnitFace { instance, face } => {
            crate::units::unit_face_world_polygon(doc, *instance, face)
        }
        // Repeated instance face (#1116): source loop placed by the instance transform.
        FaceId::RepeatedFace { face, op, instance } => {
            let poly = face_boundary_loop_world(doc, face)?;
            let rep = doc.repeat_ops.get(*op)?;
            let m = repeat_instance_transform(doc, rep, *instance)?;
            Some(poly.into_iter().map(|p| m.transform_point3(p)).collect())
        }
        FaceId::ExtrudeCap {
            extrusion,
            profile,
            top,
        } => cap_polygon_world(doc, *extrusion, profile, *top),
        FaceId::ExtrudeSide {
            extrusion,
            profile,
            edge,
        } => side_quad_world(doc, *extrusion, profile, *edge as usize).map(|quad| quad.to_vec()),
        FaceId::RevolveCap {
            revolution,
            profile,
            end,
        } => revolve_cap_polygon_world(doc, *revolution, profile, *end).map(|(poly, _)| poly),
        FaceId::RevolveSide {
            revolution,
            profile,
            edge,
        } => revolve_side_geom(doc, *revolution, profile, *edge as usize).map(|(poly, _, _)| poly),
        FaceId::Circle(_)
        | FaceId::Polygon(_)
        | FaceId::ConstructionPlane(_) => None,
        FaceId::PrimitiveFace { primitive, face } => {
            let shape = doc.primitives.get(*primitive)?;
            let poly = crate::primitives::face_polygon(doc, shape, *face)?;
            let body = crate::model::body_index_for_primitive(doc, *primitive)?;
            Some(pose_loop_if_jointed(doc, body, poly))
        }
        FaceId::BodyMeshFace {
            body,
            centroid,
            normal,
        } => {
            let tris = body_face_triangles(doc, *body, *centroid, *normal)?;
            // Ordered outline (#1219/#1220): triangle-visit order draws diagonals and
            // crossing lines on the hover border. Chain the true coplanar boundary instead.
            let loop_pts = crate::construction::coplanar_face_boundary_loop(&tris);
            (loop_pts.len() >= 3).then_some(loop_pts)
        }
    }
}

fn pose_loop_if_jointed(
    doc: &Document,
    body: crate::model::BodyKey,
    pts: Vec<Vec3>,
) -> Vec<Vec3> {
    match crate::joints::body_joint_pose(doc, body) {
        Some(m) => pts.into_iter().map(|p| m.transform_point3(p)).collect(),
        None => pts,
    }
}

/// Inverse of the joint pose the extrusion's sketch host carries, if any (#1358).
/// Features on a jointed body are built where the part is drawn; fuse/cut against the
/// un-jointed host needs them back in modelling space.
fn extrusion_host_unpose(doc: &Document, extrusion: &Extrusion) -> Option<glam::Mat4> {
    let face = doc.sketch_face(extrusion.sketch)?;
    let body = crate::model::body_index_for_face(doc, &face)?;
    crate::joints::body_joint_pose(doc, body).map(|m| m.inverse())
}

fn occt_extrusion_shape_for_host(
    doc: &Document,
    extrusion: &Extrusion,
    distance: f32,
) -> Option<crate::kernel::Shape> {
    let shape = occt_extrusion_shape(doc, extrusion, distance)?;
    match extrusion_host_unpose(doc, extrusion) {
        Some(inv) => shape.transformed(&mat4_to_rows_3x4(&inv)),
        None => Some(shape),
    }
}

fn circle_profile_world(frame: &SketchFrame, cx: f32, cy: f32, r: f32) -> Vec<Vec3> {
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let a = i as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
            local_to_world(frame, cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Emit caps + side walls for a simple (possibly concave) profile, given its base loop and
/// the matching `top` loop (one top vertex per base vertex, so the top cap may be slanted).
/// Hand-rolled (non-kernel) mesh for extruding a face **with holes** (#268): hole-aware caps
/// (via [`crate::polygon::triangulate_planar_with_holes`]) plus outer *and* inner side walls, so
/// a ring/annulus renders as a hollow tube in the fallback mesher too. `holes_base`/`holes_top`
/// are the hole loops projected to the base and (possibly slanted) top, matching `profile`/`top`.
fn extrude_region(
    profile: &[Vec3],
    top: &[Vec3],
    holes_base: &[Vec<Vec3>],
    holes_top: &[Vec<Vec3>],
    triangles: &mut Vec<[Vec3; 3]>,
) {
    let n = profile.len();
    if n < 3 || top.len() != n {
        return;
    }
    let normal = (profile[1] - profile[0])
        .cross(profile[2] - profile[0])
        .normalize_or_zero();
    // Caps: base wound inward (reversed), top wound outward — matching `extrude_profile`.
    let base_cap =
        crate::polygon::triangulate_planar_with_holes(profile, holes_base, normal);
    for [a, b, c] in base_cap {
        triangles.push([a, c, b]);
    }
    let top_cap = crate::polygon::triangulate_planar_with_holes(top, holes_top, normal);
    for [a, b, c] in top_cap {
        triangles.push([a, b, c]);
    }
    // Outer side walls (one quad per edge).
    for i in 0..n {
        let j = (i + 1) % n;
        triangles.push([profile[i], profile[j], top[j]]);
        triangles.push([profile[i], top[j], top[i]]);
    }
    // Inner (hole) side walls, wound opposite so they face into the cavity.
    for (hb, ht) in holes_base.iter().zip(holes_top) {
        let m = hb.len();
        if m < 3 || ht.len() != m {
            continue;
        }
        for i in 0..m {
            let j = (i + 1) % m;
            triangles.push([hb[j], hb[i], ht[i]]);
            triangles.push([hb[j], ht[i], ht[j]]);
        }
    }
}

fn extrude_profile(profile: &[Vec3], top: &[Vec3], triangles: &mut Vec<[Vec3; 3]>) {
    let n = profile.len();
    if n < 3 || top.len() != n {
        return;
    }

    let normal = (profile[1] - profile[0])
        .cross(profile[2] - profile[0])
        .normalize_or_zero();
    let cap_tris = crate::polygon::triangulate_planar(profile, normal);
    for &[a, b, c] in &cap_tris {
        triangles.push([profile[a], profile[c], profile[b]]);
    }
    for &[a, b, c] in &cap_tris {
        triangles.push([top[a], top[b], top[c]]);
    }
    // Side walls (one quad per edge).
    for i in 0..n {
        let j = (i + 1) % n;
        triangles.push([profile[i], profile[j], top[j]]);
        triangles.push([profile[i], top[j], top[i]]);
    }
}

// --- 3D edge chamfer/fillet (#77) ---------------------------------------------------------
//
// A mesh-bevel approximation of a solid-edge chamfer/fillet, scoped to the two edge families
// with a clean analytic definition on a `Rect`/`Polygon` profile: a vertical side-wall-to-
// side-wall edge, and a side-wall-to-cap edge (see `ExtrusionEdgeRef`). There's no BREP kernel
// here (SPEC §3.4/§10), so this doesn't attempt a true tangent-continuous curved surface, and
// it doesn't attempt to blend a shared corner where 3+ treated edges would meet — see
// `edge_treatment_conflicts`.

/// Number of segments used to facet a fillet edge-treatment bevel. Reuses
/// [`crate::model::BEZIER_SEGMENTS`] directly: an edge-treatment fillet is the same
/// cubic-bezier-approximated arc a sketch-vertex fillet uses
/// ([`crate::model::vertex_treatment_geometry`]), just embedded in 3D via [`corner_bevel_3d`]
/// and swept along the edge, so the same faceting density is the natural, consistent choice
/// (mirrors how [`CIRCLE_SEGMENTS`] is this module's own precedent for curve faceting).
pub const EDGE_TREATMENT_FILLET_SEGMENTS: usize = crate::model::BEZIER_SEGMENTS;

/// Truncated points (and, for a fillet, bridging-arc tangent-handle control points) for a
/// chamfer/fillet corner cut at 3D vertex `v`, generalizing
/// [`crate::model::vertex_treatment_geometry`] to arbitrary (non-coplanar) 3D directions.
///
/// `a` and `b` are `v`'s two real neighboring points — the same corner triangle the 2D version
/// takes, just embedded in 3D. Any two rays from a shared point span a flat 2D subspace, so
/// this is an *exact* embedding (angles and distances are preserved, not approximated): `v`,
/// `a`, and `b` are mapped into an orthonormal 2D basis of that subspace, the existing 2D
/// vertex-treatment math runs unchanged, and the results are mapped back into 3D.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerBevel3d {
    /// Truncated point along `v` → `a`.
    pub p1: Vec3,
    /// Truncated point along `v` → `b`.
    pub p2: Vec3,
    /// `Some` for a fillet (bridging arc's tangent-handle control points); `None` for a
    /// chamfer (the bridge is the straight segment `p1`–`p2`).
    pub arc: Option<[Vec3; 2]>,
}

/// Computes a [`CornerBevel3d`] at 3D vertex `v`, given its two real neighboring points `a`/`b`.
/// `None` when `amount` isn't positive, either adjacent edge is degenerate, or `v`/`a`/`b` are
/// collinear (no real corner to bevel) — same failure cases as
/// [`crate::model::vertex_treatment_geometry`], which this delegates the actual math to.
pub fn corner_bevel_3d(v: Vec3, a: Vec3, b: Vec3, kind: VertexTreatmentKind, amount: f32) -> Option<CornerBevel3d> {
    let da = a - v;
    let dist_a = da.length();
    let db = b - v;
    let dist_b = db.length();
    if dist_a < 1e-6 || dist_b < 1e-6 {
        return None;
    }
    let e1 = da / dist_a;
    let e2 = (db - e1 * db.dot(e1)).normalize_or_zero();
    if e2.length_squared() < 1e-8 {
        return None; // v, a, b are collinear: no real corner.
    }
    let a_local = (dist_a, 0.0);
    let b_local = (db.dot(e1), db.dot(e2));
    let geom = vertex_treatment_geometry((0.0, 0.0), a_local, b_local, kind, amount)?;
    let to_world = |p: (f32, f32)| v + e1 * p.0 + e2 * p.1;
    Some(CornerBevel3d {
        p1: to_world(geom.p1),
        p2: to_world(geom.p2),
        arc: geom.bezier.map(|[h0, h1]| [to_world(h0), to_world(h1)]),
    })
}

fn cubic_bezier_point_3d(p0: Vec3, c0: Vec3, c1: Vec3, p1: Vec3, t: f32) -> Vec3 {
    let mt = 1.0 - t;
    p0 * (mt * mt * mt) + c0 * (3.0 * mt * mt * t) + c1 * (3.0 * mt * t * t) + p1 * (t * t * t)
}

/// Discretized points tracing a corner bevel from `p1` to `p2`: just the two endpoints for a
/// chamfer (a straight cut), or [`EDGE_TREATMENT_FILLET_SEGMENTS`]` + 1` points sampled from
/// the bridging arc for a fillet.
pub fn sample_corner_bevel(bevel: &CornerBevel3d, kind: VertexTreatmentKind) -> Vec<Vec3> {
    match (kind, bevel.arc) {
        (VertexTreatmentKind::Fillet, Some([h0, h1])) => (0..=EDGE_TREATMENT_FILLET_SEGMENTS)
            .map(|i| {
                cubic_bezier_point_3d(
                    bevel.p1,
                    h0,
                    h1,
                    bevel.p2,
                    i as f32 / EDGE_TREATMENT_FILLET_SEGMENTS as f32,
                )
            })
            .collect(),
        _ => vec![bevel.p1, bevel.p2],
    }
}

/// Which ring (base or top cap) an [`ExtrusionEdgeRef`] touches at a given profile vertex.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum EdgeRing {
    Base,
    Top,
}

/// The `(vertex, ring)` pairs an edge treatment claims on its face's `n`-vertex profile loop.
/// A `Vertical` edge claims its one vertex on both rings (it runs the full height, base to
/// top); a `Cap` edge claims both its endpoint vertices, but only on the ring it touches.
fn touched_vertex_rings(edge: ExtrusionEdgeRef, n: usize) -> [(usize, EdgeRing); 2] {
    match edge {
        ExtrusionEdgeRef::Vertical { edge, .. } => {
            let v = if n == 0 { 0 } else { (edge + 1) % n };
            [(v, EdgeRing::Base), (v, EdgeRing::Top)]
        }
        ExtrusionEdgeRef::Cap { edge, top, .. } => {
            let ring = if top { EdgeRing::Top } else { EdgeRing::Base };
            let e2 = if n == 0 { 0 } else { (edge + 1) % n };
            [(edge, ring), (e2, ring)]
        }
    }
}

/// Whether adding an edge treatment on `new` would make it share a `(vertex, ring)` with an
/// *different* edge already treated on the same face in `existing` — a vertex miter, which
/// this mesh-bevel approximation doesn't attempt to blend (SPEC §3.4: reject rather than try
/// to combine three-or-more bevels at a shared corner). Re-treating the exact same edge (e.g.
/// dragging its amount again) is not a conflict with itself.
pub fn edge_treatment_conflicts(existing: &[EdgeTreatment], new: ExtrusionEdgeRef, n: usize) -> bool {
    if n == 0 {
        return false;
    }
    let new_touch = touched_vertex_rings(new, n);
    existing.iter().any(|t| {
        t.edge.face() == new.face()
            && t.edge != new
            && touched_vertex_rings(t.edge, n)
                .iter()
                .any(|p| new_touch.contains(p))
    })
}

/// Whether `edge` names a currently-treatable analytic edge of `solid` (#77/#1329).
pub fn treatable_edge_exists(
    doc: &Document,
    solid: TreatableSolid,
    edge: ExtrusionEdgeRef,
) -> bool {
    match solid {
        TreatableSolid::Extrusion(extrusion) => extrusion_edge_exists(doc, extrusion, edge),
        TreatableSolid::Primitive(primitive) => primitive_edge_exists(doc, primitive, edge),
    }
}

/// Whether `edge` names a currently-treatable analytic edge: `extrusion` exists and isn't
/// deleted, `edge.face()` indexes one of its faces, that face has an analytic (`Rect`/
/// `Polygon`, at least 3 sides) profile — a `Circle` profile has none, see
/// [`side_face_count`] — and `edge`'s own index is in range.
pub fn extrusion_edge_exists(doc: &Document, extrusion: crate::model::ExtrusionKey, edge: ExtrusionEdgeRef) -> bool {
    let Some(ext) = doc.extrusions.get(extrusion) else {
        return false;
    };
    let Some(face) = ext.faces.get(edge.face()) else {
        return false;
    };
    // Circle cap rims (#177) are treatable in kernel builds — the chamfer/fillet is a real
    // BREP operation on the rim circle; there's no mesh-bevel fallback for them.
    if is_circle_cap_rim(face, edge) {
        return true;
    }
    let n = side_face_count(face);
    if n < 3 {
        return false;
    }
    match edge {
        ExtrusionEdgeRef::Vertical { edge, .. } | ExtrusionEdgeRef::Cap { edge, .. } => edge < n,
    }
}

/// World-space endpoints of every currently-treatable analytic edge in the document (#77): for
/// each non-deleted extrusion's `Rect`/`Polygon` faces, every vertical side edge and every
/// Resolve a geometry-keyed selected body edge (`SceneElement::BodyEdge`, #156) to the
/// analytic `(extrusion, ExtrusionEdgeRef)` the chamfer/fillet tool operates on (#157/#165):
/// match its quantized endpoints against [`treatable_edges`], in either direction. `None`
/// when the selected mesh edge isn't an analytic treatable edge (e.g. a circle-profile wall
/// seam, an imported-mesh edge, or a boolean-result edge).
pub fn treatable_edge_for_selection(
    doc: &Document,
    body: crate::model::BodyKey,
    a: [i32; 3],
    b: [i32; 3],
) -> Option<(TreatableSolid, ExtrusionEdgeRef)> {
    let q = crate::hierarchy::quantize_body_point;
    for (solid, edge, ea, eb) in treatable_edges(doc) {
        if live_body_for_treatable_solid(doc, solid) != Some(body) {
            continue;
        }
        let (qa, qb) = (q(ea), q(eb));
        if (qa == a && qb == b) || (qa == b && qb == a) {
            return Some((solid, edge));
        }
    }
    None
}

/// The subset of a scene selection the chamfer/fillet tool can operate on (#157/#165): every
/// selected body edge that resolves to an analytic treatable edge, in selection-iteration
/// order (deduplicated by the resolver's identity).
pub fn treatable_edges_in_selection(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
) -> Vec<(TreatableSolid, ExtrusionEdgeRef)> {
    let mut out: Vec<(TreatableSolid, ExtrusionEdgeRef)> = Vec::new();
    for element in selection.iter() {
        if let crate::hierarchy::SceneElement::BodyEdge { body, a, b } = element {
            if let Some(resolved) = treatable_edge_for_selection(doc, body, a, b) {
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
        }
    }
    out
}

/// side/cap edge (see [`ExtrusionEdgeRef`]). The chamfer/fillet tool picks from this list
/// directly (rather than the generic mesh-feature-edge extraction used for construction-plane
/// referencing, #31) when no sketch is open, since it needs the structured edge reference, not
/// just two raw points.
pub fn treatable_edges(
    doc: &Document,
) -> Vec<(TreatableSolid, ExtrusionEdgeRef, Vec3, Vec3)> {
    let mut out = Vec::new();
    for (ei, ext) in doc.extrusions.iter() {
        for (fi, face) in ext.faces.iter().enumerate() {
            let n = side_face_count(face);
            if n < 3 {
                // Circle profiles have no polygonal side edges, but their cap rims are
                // treatable in a kernel build (#177): emit each rim as its chord segments,
                // all naming the same `Cap {{ edge: 0 }}` reference, so segment-based
                // picking works on the whole circle.
                if matches!(face, ExtrudeFace::Circle(_)) {
                    let distance = effective_distance(doc, ext);
                    if let Some((base, top, _)) = extrusion_profile_rings(doc, ext, face, distance) {
                        let m = base.len();
                        for k in 0..m {
                            let k2 = (k + 1) % m;
                            out.push((
                                TreatableSolid::Extrusion(ei),
                                ExtrusionEdgeRef::Cap { face: fi, edge: 0, top: false },
                                base[k],
                                base[k2],
                            ));
                            out.push((
                                TreatableSolid::Extrusion(ei),
                                ExtrusionEdgeRef::Cap { face: fi, edge: 0, top: true },
                                top[k],
                                top[k2],
                            ));
                        }
                    }
                }
                continue;
            }
            let distance = effective_distance(doc, ext);
            let Some((base, top, _)) = extrusion_profile_rings(doc, ext, face, distance) else {
                continue;
            };
            for edge in 0..n {
                let v = (edge + 1) % n;
                out.push((
                    TreatableSolid::Extrusion(ei),
                    ExtrusionEdgeRef::Vertical { face: fi, edge },
                    base[v],
                    top[v],
                ));
                let e2 = (edge + 1) % n;
                out.push((
                    TreatableSolid::Extrusion(ei),
                    ExtrusionEdgeRef::Cap { face: fi, edge, top: false },
                    base[edge],
                    base[e2],
                ));
                out.push((
                    TreatableSolid::Extrusion(ei),
                    ExtrusionEdgeRef::Cap { face: fi, edge, top: true },
                    top[edge],
                    top[e2],
                ));
            }
        }
    }
    // Shape-tool primitives (#1329): a cuboid is a 12-edge box; a cylinder offers its two
    // circular rims. Same `ExtrusionEdgeRef` addresses a rectangular extrusion uses.
    for (pi, shape) in doc.primitives.iter() {
        push_primitive_treatable_edges(doc, pi, shape, &mut out);
    }
    out
}

fn push_primitive_treatable_edges(
    doc: &Document,
    pi: crate::model::PrimitiveKey,
    shape: &crate::model::Primitive,
    out: &mut Vec<(TreatableSolid, ExtrusionEdgeRef, Vec3, Vec3)>,
) {
    let Some(r) = crate::primitives::resolve(doc, shape) else {
        return;
    };
    let solid = TreatableSolid::Primitive(pi);
    match shape.kind {
        crate::model::PrimitiveKind::Cuboid => {
            let base = r.cuboid_base();
            let lift = r.normal * r.height;
            let top: [Vec3; 4] = std::array::from_fn(|i| base[i] + lift);
            for edge in 0..4 {
                let v = (edge + 1) % 4;
                out.push((
                    solid,
                    ExtrusionEdgeRef::Vertical { face: 0, edge },
                    base[v],
                    top[v],
                ));
                out.push((
                    solid,
                    ExtrusionEdgeRef::Cap { face: 0, edge, top: false },
                    base[edge],
                    base[v],
                ));
                out.push((
                    solid,
                    ExtrusionEdgeRef::Cap { face: 0, edge, top: true },
                    top[edge],
                    top[v],
                ));
            }
        }
        crate::model::PrimitiveKind::Cylinder => {
            // Same chord-segment trick as a circle-profile extrusion (#177): pickers work
            // on the tessellated rim, all naming Cap { edge: 0 }.
            const N: usize = crate::primitives::RADIAL_SEGMENTS;
            for top in [false, true] {
                let center = if top {
                    r.origin + r.normal * r.height
                } else {
                    r.origin
                };
                let mut ring = Vec::with_capacity(N);
                for i in 0..N {
                    let a = (i as f32) / (N as f32) * std::f32::consts::TAU;
                    ring.push(center + r.u * r.radius * a.cos() + r.v * r.radius * a.sin());
                }
                for k in 0..N {
                    let k2 = (k + 1) % N;
                    out.push((
                        solid,
                        ExtrusionEdgeRef::Cap { face: 0, edge: 0, top },
                        ring[k],
                        ring[k2],
                    ));
                }
            }
        }
        crate::model::PrimitiveKind::Sphere => {}
    }
}

fn primitive_edge_exists(
    doc: &Document,
    primitive: crate::model::PrimitiveKey,
    edge: ExtrusionEdgeRef,
) -> bool {
    let Some(shape) = doc.primitives.get(primitive) else {
        return false;
    };
    let Some(r) = crate::primitives::resolve(doc, shape) else {
        return false;
    };
    match (shape.kind, edge) {
        (crate::model::PrimitiveKind::Cuboid, ExtrusionEdgeRef::Vertical { edge, .. }) => {
            edge < 4 && r.width > 1e-4 && r.depth > 1e-4 && r.height > 1e-4
        }
        (crate::model::PrimitiveKind::Cuboid, ExtrusionEdgeRef::Cap { edge, .. }) => {
            edge < 4 && r.width > 1e-4 && r.depth > 1e-4 && r.height > 1e-4
        }
        (
            crate::model::PrimitiveKind::Cylinder,
            ExtrusionEdgeRef::Cap { edge: 0, .. },
        ) => r.radius > 1e-4 && r.height > 1e-4,
        _ => false,
    }
}

/// Endpoints the kernel matcher uses for a primitive edge. Cuboid edges are straight
/// (the two vertices). A cylinder rim is one closed circle — two diametrically opposite
/// points, same convention as a circle-profile extrusion (#177).
pub(crate) fn primitive_edge_kernel_endpoints(
    doc: &Document,
    primitive: crate::model::PrimitiveKey,
    edge: ExtrusionEdgeRef,
) -> Option<(Vec3, Vec3)> {
    let shape = doc.primitives.get(primitive)?;
    let r = crate::primitives::resolve(doc, shape)?;
    match (shape.kind, edge) {
        (crate::model::PrimitiveKind::Cuboid, ExtrusionEdgeRef::Vertical { edge, .. })
            if edge < 4 =>
        {
            let v = (edge + 1) % 4;
            let base = r.cuboid_base();
            Some((base[v], base[v] + r.normal * r.height))
        }
        (crate::model::PrimitiveKind::Cuboid, ExtrusionEdgeRef::Cap { edge, top, .. })
            if edge < 4 =>
        {
            let base = r.cuboid_base();
            let lift = r.normal * r.height;
            let e2 = (edge + 1) % 4;
            if top {
                Some((base[edge] + lift, base[e2] + lift))
            } else {
                Some((base[edge], base[e2]))
            }
        }
        (
            crate::model::PrimitiveKind::Cylinder,
            ExtrusionEdgeRef::Cap { edge: 0, top, .. },
        ) => {
            let center = if top {
                r.origin + r.normal * r.height
            } else {
                r.origin
            };
            let a = center + r.u * r.radius;
            let b = center - r.u * r.radius;
            Some((a, b))
        }
        _ => None,
    }
}

fn primitive_edge_anchor(
    doc: &Document,
    primitive: crate::model::PrimitiveKey,
    edge: ExtrusionEdgeRef,
) -> Option<(Vec3, Vec3)> {
    let shape = doc.primitives.get(primitive)?;
    let r = crate::primitives::resolve(doc, shape)?;
    match (shape.kind, edge) {
        (crate::model::PrimitiveKind::Cuboid, ExtrusionEdgeRef::Vertical { edge, .. })
            if edge < 4 =>
        {
            let v = (edge + 1) % 4;
            let base = r.cuboid_base();
            let mid = (base[v] + base[v] + r.normal * r.height) * 0.5;
            let axis = r.origin + r.normal * r.height * 0.5;
            let inward = (axis - mid).normalize_or_zero();
            (inward.length_squared() > 1e-8).then_some((mid, inward))
        }
        (crate::model::PrimitiveKind::Cuboid, ExtrusionEdgeRef::Cap { edge, top, .. })
            if edge < 4 =>
        {
            let base = r.cuboid_base();
            let lift = r.normal * r.height;
            let e2 = (edge + 1) % 4;
            let (a, b) = if top {
                (base[edge] + lift, base[e2] + lift)
            } else {
                (base[edge], base[e2])
            };
            let mid = (a + b) * 0.5;
            let ring_center = if top {
                r.origin + lift
            } else {
                r.origin
            };
            let in_plane = (ring_center - mid).normalize_or_zero();
            let toward_other = if top { -r.normal } else { r.normal };
            let bisector = (in_plane + toward_other).normalize_or_zero();
            (bisector.length_squared() > 1e-8).then_some((mid, bisector))
        }
        (
            crate::model::PrimitiveKind::Cylinder,
            ExtrusionEdgeRef::Cap { edge: 0, top, .. },
        ) => {
            let center = if top {
                r.origin + r.normal * r.height
            } else {
                r.origin
            };
            let mid = center + r.u * r.radius;
            let radial = -r.u;
            let toward_other = if top { -r.normal } else { r.normal };
            let bisector = (radial + toward_other).normalize_or_zero();
            (bisector.length_squared() > 1e-8).then_some((mid, bisector))
        }
        _ => None,
    }
}

/// Live ghost of an in-progress chamfer/fillet on a Shape-tool primitive (#1329):
/// the primitive's kernel solid with the picked edges treated.
pub fn primitive_treatment_preview_mesh(
    doc: &Document,
    primitive: crate::model::PrimitiveKey,
    edges: &[(TreatableSolid, ExtrusionEdgeRef)],
    kind: VertexTreatmentKind,
    amount: f32,
) -> Option<SolidMesh> {
    if amount <= 0.0 {
        return None;
    }
    let body = live_body_for_treatable_solid(doc, TreatableSolid::Primitive(primitive))?;
    let mut shape = occt_body_shape(doc, body)?;
    let mut fillet_edges = Vec::new();
    let mut fillet_radii = Vec::new();
    let mut chamfer_edges = Vec::new();
    let mut chamfer_dists = Vec::new();
    for (solid, edge) in edges {
        if *solid != TreatableSolid::Primitive(primitive) {
            continue;
        }
        let endpoints = primitive_edge_kernel_endpoints(doc, primitive, *edge)?;
        match kind {
            VertexTreatmentKind::Fillet => {
                fillet_edges.push(endpoints);
                fillet_radii.push(amount);
            }
            VertexTreatmentKind::Chamfer => {
                chamfer_edges.push(endpoints);
                chamfer_dists.push(amount);
            }
        }
    }
    if !fillet_edges.is_empty() {
        shape = shape.fillet(&fillet_edges, &fillet_radii)?;
    }
    if !chamfer_edges.is_empty() {
        shape = shape.chamfer(&chamfer_edges, &chamfer_dists)?;
    }
    let tris = shape.tessellate(OCCT_DEFLECTION as f64);
    (!tris.is_empty()).then_some(SolidMesh { triangles: tris })
}

fn primitive_edge_would_bevel(
    doc: &Document,
    primitive: crate::model::PrimitiveKey,
    edge: ExtrusionEdgeRef,
    amount: f32,
) -> bool {
    if !(amount > 0.0) || !primitive_edge_exists(doc, primitive, edge) {
        return false;
    }
    let Some(shape) = doc.primitives.get(primitive) else {
        return false;
    };
    let Some(r) = crate::primitives::resolve(doc, shape) else {
        return false;
    };
    match shape.kind {
        crate::model::PrimitiveKind::Cuboid => {
            amount < r.width * 0.5 && amount < r.depth * 0.5 && amount < r.height
        }
        crate::model::PrimitiveKind::Cylinder => amount < r.radius && amount < r.height,
        crate::model::PrimitiveKind::Sphere => false,
    }
}

/// World-space origin (edge midpoint) and inward-bisector normal for the chamfer/fillet
/// gizmo, for either an extrusion or a Shape-tool primitive (#1329).
pub fn treatable_edge_anchor(
    doc: &Document,
    solid: TreatableSolid,
    edge: ExtrusionEdgeRef,
) -> Option<(Vec3, Vec3)> {
    match solid {
        TreatableSolid::Extrusion(ei) => extrusion_edge_anchor(doc, ei, edge),
        TreatableSolid::Primitive(pi) => primitive_edge_anchor(doc, pi, edge),
    }
}

/// World-space origin (edge midpoint) and normal (inward bisector of the edge's two adjacent
/// faces, pointing into the material so pulling the gizmo away from the edge increases the
/// amount) for the 3D edge chamfer/fillet gizmo — the 3D analogue of `vertex_treatment_anchor`
/// in `main.rs`. `None` if the edge no longer resolves (deleted extrusion, out-of-range index,
/// or degenerate geometry).
pub fn extrusion_edge_anchor(doc: &Document, extrusion: crate::model::ExtrusionKey, edge: ExtrusionEdgeRef) -> Option<(Vec3, Vec3)> {
    let ext = doc.extrusions.get(extrusion)?;
    let face = ext.faces.get(edge.face())?;
    // Circle cap rim (#177): anchor at a rim point, pointing diagonally outward (radial +
    // cap normal) like the polygonal cap-edge bisector below.
    if is_circle_cap_rim(face, edge) {
        let ExtrusionEdgeRef::Cap { top: is_top, .. } = edge else {
            return None;
        };
        let distance = effective_distance(doc, ext);
        let (base, top, normal) = extrusion_profile_rings(doc, ext, face, distance)?;
        let ring = if is_top { &top } else { &base };
        let m = ring.len();
        if m < 3 {
            return None;
        }
        let center = ring.iter().copied().sum::<Vec3>() / m as f32;
        let radial = (ring[0] - center).normalize_or_zero();
        let cap_out = (normal * if is_top { distance.signum() } else { -distance.signum() })
            .normalize_or_zero();
        let bisector = (radial + cap_out).normalize_or_zero();
        if bisector.length_squared() < 1e-8 {
            return None;
        }
        return Some((ring[0], bisector));
    }
    let n = side_face_count(face);
    if n < 3 {
        return None;
    }
    let distance = effective_distance(doc, ext);
    let (base, top, _) = extrusion_profile_rings(doc, ext, face, distance)?;
    match edge {
        ExtrusionEdgeRef::Vertical { edge, .. } => {
            if edge >= n {
                return None;
            }
            let v = (edge + 1) % n;
            let prev = (v + n - 1) % n;
            let next = (v + 1) % n;
            let dir_a = (base[prev] - base[v]).normalize_or_zero();
            let dir_b = (base[next] - base[v]).normalize_or_zero();
            let bisector = (dir_a + dir_b).normalize_or_zero();
            if bisector.length_squared() < 1e-8 {
                return None;
            }
            Some(((base[v] + top[v]) * 0.5, bisector))
        }
        ExtrusionEdgeRef::Cap { edge, top: is_top, .. } => {
            if edge >= n {
                return None;
            }
            let e2 = (edge + 1) % n;
            let (ring, other_ring) = if is_top { (&top, &base) } else { (&base, &top) };
            let edge_dir = (ring[e2] - ring[edge]).normalize_or_zero();
            if edge_dir.length_squared() < 1e-8 {
                return None;
            }
            let prev = (edge + n - 1) % n;
            let raw = ring[prev] - ring[edge];
            let inward = (raw - edge_dir * raw.dot(edge_dir)).normalize_or_zero();
            let wall_dir = (other_ring[edge] - ring[edge]).normalize_or_zero();
            let bisector = (inward + wall_dir).normalize_or_zero();
            if bisector.length_squared() < 1e-8 {
                return None;
            }
            Some(((ring[edge] + ring[e2]) * 0.5, bisector))
        }
    }
}

/// Whether `kind`/`amount` would actually produce a non-degenerate bevel at `edge` right now —
/// i.e. [`corner_bevel_3d`] succeeds at every vertex the edge touches. Used to give a precise
/// "corner is degenerate" rejection (mirroring [`crate::model::vertex_treatment_geometry`]'s
/// own failure mode for the 2D case) before [`crate::actions::Action::CommitEdgeTreatments`]
/// stores the treatment, rather than relying on the mesh builder's silent per-treatment
/// fallback (which never panics, but also never reports *why* an edge didn't visibly change).
pub fn treatable_edge_would_bevel(
    doc: &Document,
    solid: TreatableSolid,
    edge: ExtrusionEdgeRef,
    kind: VertexTreatmentKind,
    amount: f32,
) -> bool {
    match solid {
        TreatableSolid::Extrusion(ei) => edge_treatment_would_bevel(doc, ei, edge, kind, amount),
        TreatableSolid::Primitive(pi) => primitive_edge_would_bevel(doc, pi, edge, amount),
    }
}

pub fn edge_treatment_would_bevel(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    edge: ExtrusionEdgeRef,
    kind: VertexTreatmentKind,
    amount: f32,
) -> bool {
    if !(amount > 0.0) {
        return false;
    }
    let Some(ext) = doc.extrusions.get(extrusion) else {
        return false;
    };
    let Some(face) = ext.faces.get(edge.face()) else {
        return false;
    };
    // A circle cap rim (#177) has no polygonal corner to test; sanity-bound the amount by
    // the cylinder's radius and height — the kernel feasibility trial does the real check.
    if is_circle_cap_rim(face, edge) {
        if let ExtrudeFace::Circle(ci) = face {
            let radius = doc.circles.get(*ci).map(|c| c.r).unwrap_or(0.0);
            let height = effective_distance(doc, ext).abs();
            return amount < radius && amount < height;
        }
        return false;
    }
    let n = side_face_count(face);
    if n < 3 {
        return false;
    }
    let distance = effective_distance(doc, ext);
    let Some((base, top, _)) = extrusion_profile_rings(doc, ext, face, distance) else {
        return false;
    };
    match edge {
        ExtrusionEdgeRef::Vertical { edge, .. } => {
            if edge >= n {
                return false;
            }
            let v = (edge + 1) % n;
            let prev = (v + n - 1) % n;
            let next = (v + 1) % n;
            corner_bevel_3d(base[v], base[prev], base[next], kind, amount).is_some()
                && corner_bevel_3d(top[v], top[prev], top[next], kind, amount).is_some()
        }
        ExtrusionEdgeRef::Cap { edge, top: is_top, .. } => {
            if edge >= n {
                return false;
            }
            let e2 = (edge + 1) % n;
            let (ring, other_ring) = if is_top { (&top, &base) } else { (&base, &top) };
            let edge_dir = (ring[e2] - ring[edge]).normalize_or_zero();
            if edge_dir.length_squared() < 1e-8 {
                return false;
            }
            let prev = (edge + n - 1) % n;
            let next = (e2 + 1) % n;
            let inward_at = |vertex: usize, neighbor: usize| -> Option<Vec3> {
                let raw = ring[neighbor] - ring[vertex];
                let rejected = raw - edge_dir * raw.dot(edge_dir);
                (rejected.length_squared() > 1e-8).then(|| rejected.normalize_or_zero())
            };
            let Some(inward1) = inward_at(edge, prev) else {
                return false;
            };
            let Some(inward2) = inward_at(e2, next) else {
                return false;
            };
            let reach1 = (ring[edge] - ring[prev]).length().max(amount * 4.0);
            let reach2 = (ring[e2] - ring[next]).length().max(amount * 4.0);
            let a1 = ring[edge] + inward1 * reach1;
            let a2 = ring[e2] + inward2 * reach2;
            corner_bevel_3d(ring[edge], a1, other_ring[edge], kind, amount).is_some()
                && corner_bevel_3d(ring[e2], a2, other_ring[e2], kind, amount).is_some()
        }
    }
}

/// Returns a clone of `extrusion`'s source extrusion with `treatment` applied (replacing any
/// existing treatment of the same edge, so re-dragging an already-treated edge updates it in
/// place rather than stacking a duplicate). Used both for the live interactive preview (a ghost
/// extrusion fed straight into `extrusion_mesh`, never touching `doc` until commit) and by
/// [`crate::actions::Action::CommitEdgeTreatments`] to build the value it stores.
pub fn extrusion_with_edge_treatment(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    treatment: EdgeTreatment,
) -> Option<Extrusion> {
    extrusion_with_edge_treatments(doc, extrusion, [treatment])
}

/// [`extrusion_with_edge_treatment`] over a whole set (#166): the ghost preview of a
/// multi-edge chamfer/fillet splices every in-progress treatment into the clone at once.
pub fn extrusion_with_edge_treatments(
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    treatments: impl IntoIterator<Item = EdgeTreatment>,
) -> Option<Extrusion> {
    let mut ext = doc.extrusions.get(extrusion)?.clone();
    // #1322: include treatments already committed on this extrusion's live body so a
    // second-fillet preview is the already-filleted solid plus the new round.
    if let Some(live) = live_body_for_treated_extrusion(doc, extrusion) {
        for t in edge_treatments_leading_to(doc, live, extrusion) {
            ext.edge_treatments.retain(|e| e.edge != t.edge);
            ext.edge_treatments.push(t);
        }
    }
    for treatment in treatments {
        ext.edge_treatments.retain(|t| t.edge != treatment.edge);
        ext.edge_treatments.push(treatment);
    }
    Some(ext)
}

/// Pushes `tri` oriented so its normal points away from `interior` (a rough interior reference
/// point of the solid) — used throughout the edge-treatment mesh builder below so new geometry
/// doesn't need its winding hand-derived per call site; a triangle's *shape* still has to be
/// right, but which of its two windings gets emitted is corrected here uniformly.
fn push_oriented(triangles: &mut Vec<[Vec3; 3]>, tri: [Vec3; 3], interior: Vec3) {
    let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
    let centroid = (tri[0] + tri[1] + tri[2]) / 3.0;
    if normal.dot(centroid - interior) < 0.0 {
        triangles.push([tri[0], tri[2], tri[1]]);
    } else {
        triangles.push(tri);
    }
}

/// Ear-clips a (possibly non-convex) boundary loop into cap triangles, oriented outward from
/// `interior`. Degenerate (near-zero-area / too-short) boundaries are silently skipped.
fn triangulate_cap(boundary: &[Vec3], interior: Vec3, triangles: &mut Vec<[Vec3; 3]>) {
    if boundary.len() < 3 {
        return;
    }
    let normal = (boundary[1] - boundary[0])
        .cross(boundary[2] - boundary[0])
        .normalize_or_zero();
    if normal.length_squared() < 1e-8 {
        return;
    }
    for &[a, b, c] in &crate::polygon::triangulate_planar(boundary, normal) {
        push_oriented(triangles, [boundary[a], boundary[b], boundary[c]], interior);
    }
}

/// Applies one cap-edge treatment (base or top ring, whichever `ring` is) at polygon edge
/// `edge` (between profile vertices `edge` and `edge + 1`).
///
/// Physically this is subtracting a uniform-cross-section prism (triangular for a chamfer, a
/// quarter-round for a fillet) that runs the *entire* length of the treated edge — so the two
/// endpoint vertices (`edge` and `edge + 1`), which are corners of the *original* box, are cut
/// away entirely: they don't appear anywhere in the treated mesh anymore. That has three
/// knock-on effects, each handled here:
/// 1. The cap ring's boundary loses that vertex, replaced by the single inset point `p1`
///    (spliced into `ring_corners`, consumed by [`triangulate_cap`]).
/// 2. The treated wall itself (`edge`) starts (or ends) at the single raised point `p2`
///    instead (recorded in `wall_own_start`/`wall_own_end`, keyed by the wall/edge index).
/// 3. Each *untreated* neighboring wall that used to share that corner vertex — wall
///    `edge - 1` at the `edge` end, wall `edge + 1` at the `edge + 1` end — loses its own
///    corner too: since the prism's cross-section is the *same* at every point along the
///    treated edge (including right at its ends), the neighboring wall's flat face is
///    "notched" by that same cross-section where the two meet, so the neighbor's corner must
///    be replaced by the *full* sampled bevel run (not just its two endpoints — for a fillet
///    the notch is genuinely curved, since the neighbor wall is flat and the removed material
///    follows the arc all the way to the very end of the treated edge). These are recorded in
///    `neighbor_notch_end`/`neighbor_notch_start`, consumed by the main wall loop in
///    [`extrude_profile_with_treatments`], which triangulates each wall's own (possibly
///    notched, `n`-gon) boundary via [`triangulate_cap`] rather than assuming a plain quad.
///
/// The samples for the neighbor's notch are exactly the bevel face's own end cross-section, so
/// the neighbor wall and the new bevel face share that boundary exactly — no T-junction, no
/// gap, and no extra "return" triangle is needed (the sharp corner point is simply gone).
#[allow(clippy::too_many_arguments)]
fn apply_cap_edge_treatment(
    ring: &[Vec3],
    other_ring: &[Vec3],
    edge: usize,
    kind: VertexTreatmentKind,
    amount: f32,
    n: usize,
    // Whether `ring` is the *top* cap: the wall loop in `extrude_profile_with_treatments`
    // visits the top ring in the opposite spatial sense to the base ring (base_start -> ... ->
    // top_end -> top_start -> close), so a top-ring notch's sample order needs to be the
    // mirror image of a base-ring notch's to still read "outward edge toward the wall level,
    // inward toward the cap level" consistently around that loop.
    ring_is_top: bool,
    ring_corners: &mut [Vec<Vec3>],
    wall_own_start: &mut HashMap<usize, Vec3>,
    wall_own_end: &mut HashMap<usize, Vec3>,
    neighbor_notch_end: &mut HashMap<usize, Vec<Vec3>>,
    neighbor_notch_start: &mut HashMap<usize, Vec<Vec3>>,
    interior: Vec3,
    triangles: &mut Vec<[Vec3; 3]>,
) {
    let e2 = (edge + 1) % n;
    let edge_dir = (ring[e2] - ring[edge]).normalize_or_zero();
    if edge_dir.length_squared() < 1e-8 {
        return;
    }
    // Inward direction within the ring's plane, perpendicular to the treated edge: the
    // direction toward each endpoint's *other* neighbor on the ring, with the component along
    // the treated edge itself removed. Exact for a rectangle; a reasonable approximation for a
    // general (possibly non-right-angle) polygon profile.
    let prev = (edge + n - 1) % n;
    let next = (e2 + 1) % n;
    let inward_at = |vertex: usize, neighbor: usize| -> Option<Vec3> {
        let raw = ring[neighbor] - ring[vertex];
        let rejected = raw - edge_dir * raw.dot(edge_dir);
        (rejected.length_squared() > 1e-8).then(|| rejected.normalize_or_zero())
    };
    let Some(inward1) = inward_at(edge, prev) else {
        return;
    };
    let Some(inward2) = inward_at(e2, next) else {
        return;
    };
    // A synthetic "far point" along the inward direction, just to give `corner_bevel_3d` a
    // sensible clamp bound (its own adjacent cap edge's length, or 4x the amount if that's
    // somehow shorter) — there's no *real* adjacent vertex in this direction to clamp against.
    let reach1 = (ring[edge] - ring[prev]).length().max(amount * 4.0);
    let reach2 = (ring[e2] - ring[next]).length().max(amount * 4.0);
    let a1 = ring[edge] + inward1 * reach1;
    let a2 = ring[e2] + inward2 * reach2;

    let Some(bevel1) = corner_bevel_3d(ring[edge], a1, other_ring[edge], kind, amount) else {
        return;
    };
    let Some(bevel2) = corner_bevel_3d(ring[e2], a2, other_ring[e2], kind, amount) else {
        return;
    };
    let samples1 = sample_corner_bevel(&bevel1, kind); // ordered cap-level (p1) -> wall-level (p2)
    let samples2 = sample_corner_bevel(&bevel2, kind);

    ring_corners[edge] = vec![bevel1.p1];
    ring_corners[e2] = vec![bevel2.p1];
    wall_own_start.insert(edge, bevel1.p2);
    wall_own_end.insert(edge, bevel2.p2);
    // Base-ring notches read forward at the wall's *end* slot and reversed at its *start*
    // slot (see the doc comment above); a top-ring notch is visited in the mirrored spatial
    // sense by the wall loop, so it needs the opposite of each.
    let (mut end_samples, mut start_samples) = (samples1.clone(), samples2.clone());
    if ring_is_top {
        end_samples.reverse();
    } else {
        start_samples.reverse();
    }
    neighbor_notch_end.insert(prev, end_samples);
    neighbor_notch_start.insert(e2, start_samples);

    // Bevel face: a quad strip (one quad for a chamfer) between the cap-level samples and the
    // wall-level samples — the corner geometry repeats uniformly along a straight prism edge,
    // so corresponding sample indices at the two endpoints line up into a valid, non-twisting
    // strip.
    let m = samples1.len().min(samples2.len());
    for k in 0..m.saturating_sub(1) {
        let (c1a, c1b) = (samples1[k], samples1[k + 1]);
        let (c2a, c2b) = (samples2[k], samples2[k + 1]);
        push_oriented(triangles, [c1a, c2a, c2b], interior);
        push_oriented(triangles, [c1a, c2b, c1b], interior);
    }
}

/// Emits caps + side walls for a profile with one or more [`EdgeTreatment`]s applied (#77),
/// generalizing [`extrude_profile`]. `treatments` must already be filtered to this face.
///
/// The core idea: represent each cap ring not as `n` points but as `n` *lists* of points (one
/// per profile vertex, normally a singleton), and each side wall not as a fixed quad but as a
/// general boundary loop triangulated via [`triangulate_cap`]. A vertical-edge treatment
/// replaces its one vertex's contribution with a short bevel run (`[p1, ...arc, p2]`) on *both*
/// rings — the ordinary per-edge wall loop picks that run's endpoints straight up, and a
/// separate pass stitches the small bevel walls between consecutive points of the run itself.
/// A cap-edge treatment instead cuts its two endpoint vertices away entirely — physically, it's
/// subtracting a uniform-cross-section prism that runs the whole length of the edge, so those
/// corner points genuinely don't exist in the result anymore — replacing each with the single
/// inset cap-ring point, the treated wall's own single raised point, and a *notch* (the bevel's
/// full sample run, not just its endpoints) spliced into each untreated neighboring wall that
/// used to share that corner; see [`apply_cap_edge_treatment`] for the full derivation. A given
/// analytic edge conflicting with another at a shared vertex (a vertex miter) is rejected
/// before it ever reaches here — see [`edge_treatment_conflicts`] — so this function doesn't
/// attempt to resolve that itself; if the document somehow holds conflicting treatments anyway
/// it applies them in order, later ones winning at a shared vertex, rather than panicking.
fn extrude_profile_with_treatments(
    base: &[Vec3],
    top: &[Vec3],
    treatments: &[&EdgeTreatment],
    triangles: &mut Vec<[Vec3; 3]>,
) {
    let n = base.len();
    if n < 3 || top.len() != n {
        return;
    }

    let mut vertical: HashMap<usize, (VertexTreatmentKind, f32)> = HashMap::new();
    let mut cap_bottom: HashMap<usize, (VertexTreatmentKind, f32)> = HashMap::new();
    let mut cap_top: HashMap<usize, (VertexTreatmentKind, f32)> = HashMap::new();
    for t in treatments {
        if t.amount <= 0.0 {
            continue;
        }
        match t.edge {
            ExtrusionEdgeRef::Vertical { edge, .. } if edge < n => {
                vertical.insert((edge + 1) % n, (t.kind, t.amount));
            }
            ExtrusionEdgeRef::Cap { edge, top: is_top, .. } if edge < n => {
                if is_top {
                    cap_top.insert(edge, (t.kind, t.amount));
                } else {
                    cap_bottom.insert(edge, (t.kind, t.amount));
                }
            }
            _ => {}
        }
    }
    if vertical.is_empty() && cap_bottom.is_empty() && cap_top.is_empty() {
        extrude_profile(base, top, triangles);
        return;
    }

    let interior = (base.iter().chain(top.iter()).copied().sum::<Vec3>()) / (2 * n) as f32;

    let mut base_corners: Vec<Vec<Vec3>> = Vec::with_capacity(n);
    let mut top_corners: Vec<Vec<Vec3>> = Vec::with_capacity(n);
    for v in 0..n {
        let expanded = vertical.get(&v).and_then(|&(kind, amount)| {
            let prev = (v + n - 1) % n;
            let next = (v + 1) % n;
            let bevel_b = corner_bevel_3d(base[v], base[prev], base[next], kind, amount)?;
            let bevel_t = corner_bevel_3d(top[v], top[prev], top[next], kind, amount)?;
            Some((sample_corner_bevel(&bevel_b, kind), sample_corner_bevel(&bevel_t, kind)))
        });
        match expanded {
            Some((sb, st)) => {
                base_corners.push(sb);
                top_corners.push(st);
            }
            None => {
                base_corners.push(vec![base[v]]);
                top_corners.push(vec![top[v]]);
            }
        }
    }

    // Wall-corner overrides are keyed by the *wall/edge* index, not by the shared vertex: a
    // vertex can be an endpoint of an untreated neighboring wall too, which needs a different
    // treatment (a full notch tracing the bevel, not just its raised corner point — see
    // `apply_cap_edge_treatment`'s doc comment) than the treated wall's own corner.
    let mut base_wall_own_start: HashMap<usize, Vec3> = HashMap::new();
    let mut base_wall_own_end: HashMap<usize, Vec3> = HashMap::new();
    let mut base_notch_end: HashMap<usize, Vec<Vec3>> = HashMap::new();
    let mut base_notch_start: HashMap<usize, Vec<Vec3>> = HashMap::new();
    let mut top_wall_own_start: HashMap<usize, Vec3> = HashMap::new();
    let mut top_wall_own_end: HashMap<usize, Vec3> = HashMap::new();
    let mut top_notch_end: HashMap<usize, Vec<Vec3>> = HashMap::new();
    let mut top_notch_start: HashMap<usize, Vec<Vec3>> = HashMap::new();
    for (&edge, &(kind, amount)) in &cap_bottom {
        apply_cap_edge_treatment(
            base,
            top,
            edge,
            kind,
            amount,
            n,
            false,
            &mut base_corners,
            &mut base_wall_own_start,
            &mut base_wall_own_end,
            &mut base_notch_end,
            &mut base_notch_start,
            interior,
            triangles,
        );
    }
    for (&edge, &(kind, amount)) in &cap_top {
        apply_cap_edge_treatment(
            top,
            base,
            edge,
            kind,
            amount,
            n,
            true,
            &mut top_corners,
            &mut top_wall_own_start,
            &mut top_wall_own_end,
            &mut top_notch_end,
            &mut top_notch_start,
            interior,
            triangles,
        );
    }

    let base_loop: Vec<Vec3> = base_corners.iter().flatten().copied().collect();
    let top_loop: Vec<Vec3> = top_corners.iter().flatten().copied().collect();
    triangulate_cap(&base_loop, interior, triangles);
    triangulate_cap(&top_loop, interior, triangles);

    // Main walls: one per original polygon edge. Ordinarily a plain quad, but a wall next to a
    // treated cap edge gets one (or both) of its corners replaced: a full point (raised/lowered)
    // if *this* wall is itself the treated one, or a full notch run (see doc comment on
    // `apply_cap_edge_treatment`) if it's the untreated neighbor of a treatment at that corner.
    // Triangulated as a general polygon (usually 4 points, more when notched) via
    // `triangulate_cap`, since a double-notched wall isn't a simple quad anymore.
    for e in 0..n {
        let e2 = (e + 1) % n;
        let mut wall_loop = Vec::with_capacity(4);
        match base_wall_own_start.get(&e) {
            Some(&p) => wall_loop.push(p),
            None => match base_notch_start.get(&e) {
                Some(samples) => wall_loop.extend(samples.iter().copied()),
                None => wall_loop.push(*base_corners[e].last().unwrap()),
            },
        }
        match base_wall_own_end.get(&e) {
            Some(&p) => wall_loop.push(p),
            None => match base_notch_end.get(&e) {
                Some(samples) => wall_loop.extend(samples.iter().copied()),
                None => wall_loop.push(*base_corners[e2].first().unwrap()),
            },
        }
        match top_wall_own_end.get(&e) {
            Some(&p) => wall_loop.push(p),
            None => match top_notch_end.get(&e) {
                Some(samples) => wall_loop.extend(samples.iter().copied()),
                None => wall_loop.push(*top_corners[e2].first().unwrap()),
            },
        }
        match top_wall_own_start.get(&e) {
            Some(&p) => wall_loop.push(p),
            None => match top_notch_start.get(&e) {
                Some(samples) => wall_loop.extend(samples.iter().copied()),
                None => wall_loop.push(*top_corners[e].last().unwrap()),
            },
        }
        triangulate_cap(&wall_loop, interior, triangles);
    }

    // Vertical-treatment mini-walls: consecutive pairs within one vertex's own expanded run
    // (its bevel face — a flat quad for a chamfer, a faceted strip for a fillet).
    for v in 0..n {
        let sb = &base_corners[v];
        let st = &top_corners[v];
        if sb.len() < 2 || st.len() != sb.len() {
            continue;
        }
        for k in 0..sb.len() - 1 {
            push_oriented(triangles, [sb[k], sb[k + 1], st[k + 1]], interior);
            push_oriented(triangles, [sb[k], st[k + 1], st[k]], interior);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::retain_ground_plane_only;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::sketch_key_for_slot as skey;
    use crate::model::sketch_text_key_for_slot as tkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use crate::model::body_key_for_slot as bkey;
    use crate::model::move_op_key_for_slot as mopkey;
    use super::*;
    use crate::model::{Circle, Document, FaceId, Line};

    /// A tessellated tube of `sides` strips about +Z: what a circular extrusion's wall is.
    pub(crate) fn tube(centre: Vec3, radius: f32, height: f32, sides: usize) -> Vec<[Vec3; 3]> {
        let mut tris = Vec::new();
        for i in 0..sides {
            let a = (i as f32) / sides as f32 * std::f32::consts::TAU;
            let b = ((i + 1) as f32) / sides as f32 * std::f32::consts::TAU;
            let p = |ang: f32, z: f32| {
                centre + Vec3::new(radius * ang.cos(), radius * ang.sin(), z)
            };
            tris.push([p(a, 0.0), p(b, 0.0), p(b, height)]);
            tris.push([p(a, 0.0), p(b, height), p(a, height)]);
        }
        tris
    }

    /// #1037: a tessellated cylinder wall smooths — every corner normal points radially
    /// outward from the axis, not along its own flat facet, so the wall shades as a curve.
    #[test]
    fn a_round_wall_gets_radial_smooth_normals() {
        let centre = Vec3::new(3.0, -2.0, 0.0);
        let mesh = SolidMesh {
            triangles: tube(centre, 4.0, 12.0, CIRCLE_SEGMENTS),
        };
        let normals = smooth_normals(&mesh);
        assert_eq!(normals.len(), mesh.triangles.len());
        for (tri, ns) in mesh.triangles.iter().zip(normals.iter()) {
            for (corner, n) in tri.iter().zip(ns.iter()) {
                // The true normal of a cylinder wall is the radial direction, z-free.
                let radial = Vec3::new(corner.x - centre.x, corner.y - centre.y, 0.0)
                    .normalize_or_zero();
                assert!(
                    n.dot(radial) > 0.999,
                    "corner {corner} got {n}, expected the radial {radial}"
                );
            }
        }
    }

    /// #1037: the corollary — smoothing must not round the edges that are really there.
    /// A cube's corner normals stay on their own faces, because neighbouring faces meet at
    /// 90°, well past the crease angle.
    #[test]
    fn a_cube_keeps_its_edges_sharp() {
        let (mut doc, sketch) = sketch_doc();
        let face = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        doc.extrusions.insert(extrusion(sketch, vec![face], 10.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            name: None,
            material: None,
            shadow: false,
        });
        let mesh = body_solid_mesh(&doc, bkey(0)).expect("the cube meshes");
        let normals = smooth_normals(&mesh);
        for (tri, ns) in mesh.triangles.iter().zip(normals.iter()) {
            let flat = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
            for n in ns {
                assert!(
                    n.dot(flat).abs() > 0.999,
                    "a cube corner should keep its face normal {flat}, got {n}"
                );
            }
        }
        // And every normal is axis-aligned — nothing got averaged around an edge.
        for n in normals.iter().flatten() {
            let axis_aligned = [Vec3::X, Vec3::Y, Vec3::Z]
                .iter()
                .any(|a| n.dot(*a).abs() > 0.999);
            assert!(axis_aligned, "{n} is not a box face normal");
        }
    }

    /// #1037: normals are always three per triangle, so the renderer can index them
    /// positionally without bounds checks per corner.
    #[test]
    fn smooth_normals_are_unit_length_and_parallel_to_the_triangles() {
        let mesh = SolidMesh {
            triangles: tube(Vec3::ZERO, 5.0, 8.0, 16),
        };
        let normals = smooth_normals(&mesh);
        assert_eq!(normals.len(), mesh.triangles.len());
        for n in normals.iter().flatten() {
            assert!((n.length() - 1.0).abs() < 1e-4, "{n} is not unit length");
        }
        // A degenerate mesh yields no normals rather than panicking.
        assert!(smooth_normals(&SolidMesh::default()).is_empty());
    }

    /// #1013: a round wall reads as a cylinder — its axis, radius and length recovered from
    /// the mesh alone, so an imported part gets one as readily as a modelled hole.
    #[test]
    fn a_round_wall_fits_a_cylinder() {
        let tris = tube(Vec3::new(3.0, -2.0, 5.0), 4.0, 12.0, CIRCLE_SEGMENTS);
        let cyl = fit_cylinder(&tris).expect("a tessellated tube is a cylinder");
        assert!((cyl.dir - Vec3::Z).length() < 1e-3, "axis is {}", cyl.dir);
        assert!((cyl.radius - 4.0).abs() < 0.02, "radius is {}", cyl.radius);
        assert!((cyl.half_length - 6.0).abs() < 1e-3, "half length is {}", cyl.half_length);
        // The axis runs through the middle of the wall it was fitted to.
        assert!(
            (cyl.origin - Vec3::new(3.0, -2.0, 11.0)).length() < 0.02,
            "axis passes through {}",
            cyl.origin
        );
    }

    /// #1013: what isn't round isn't a cylinder — a box's walls fit a circle through their
    /// corners perfectly well, and a faceted prism is a set of flat faces, not a hole.
    #[test]
    fn flat_and_faceted_walls_are_not_cylinders() {
        // Four walls of a box, as one group: the corners lie on a circle, the walls don't.
        let box_walls = tube(Vec3::ZERO, 5.0, 10.0, 4);
        assert!(fit_cylinder(&box_walls).is_none(), "a box is not a cylinder");
        let octagon = tube(Vec3::ZERO, 5.0, 10.0, 8);
        assert!(fit_cylinder(&octagon).is_none(), "an octagonal prism is not a cylinder");
        // A single flat face has no fan of normals at all.
        let flat = vec![
            [Vec3::ZERO, Vec3::X, Vec3::Y],
            [Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y],
        ];
        assert!(fit_cylinder(&flat).is_none(), "a flat face is not a cylinder");
    }

    /// #1013: the axis direction keys the same way whichever end it was derived from, so two
    /// picks of the same hole compare equal.
    #[test]
    fn a_cylinder_axis_has_a_settled_sign() {
        let up = tube(Vec3::ZERO, 4.0, 10.0, CIRCLE_SEGMENTS);
        let down: Vec<[Vec3; 3]> = up.iter().map(|t| [t[0], t[2], t[1]]).collect();
        let a = fit_cylinder(&up).unwrap();
        let b = fit_cylinder(&down).unwrap();
        assert!((a.dir - b.dir).length() < 1e-5, "{} vs {}", a.dir, b.dir);
        assert!((a.origin - b.origin).length() < 1e-3);
    }

    /// #669: the optional B pair turns the bodies about end point A so that start B lands on
    /// end B, and end B is confined to the sphere start B can actually reach.
    /// #920: the cursor ray meets the constraint sphere — the near side when it crosses,
    /// the nearest point on it when it misses — and the direction rounds to the angle grid.
    #[test]
    fn ray_sphere_and_angle_rounding() {
        let centre = Vec3::new(10.0, 0.0, 0.0);
        // Straight at the sphere from -X: the near face.
        let hit = ray_sphere_point(Vec3::ZERO, Vec3::X, centre, 4.0).expect("a hit");
        assert!((hit - Vec3::new(6.0, 0.0, 0.0)).length() < 1e-3, "near side, got {hit}");
        // A miss still lands on the sphere, on the side the ray passes.
        let miss = ray_sphere_point(Vec3::new(0.0, 20.0, 0.0), Vec3::X, centre, 4.0)
            .expect("the nearest point");
        assert!(((miss - centre).length() - 4.0).abs() < 1e-3, "on the sphere, got {miss}");
        assert!(miss.y > 0.0, "on the side the ray went by, got {miss}");
        // Rounding: 40° of azimuth snaps to 45° at a 45° step.
        let dir = Vec3::new(40.0_f32.to_radians().cos(), 40.0_f32.to_radians().sin(), 0.0);
        let snapped = snap_direction_to_angle(dir, 45.0);
        let azimuth = snapped.y.atan2(snapped.x).to_degrees();
        assert!((azimuth - 45.0).abs() < 1e-2, "rounded to 45°, got {azimuth}");
        // A zero step leaves the direction where it is.
        let free = snap_direction_to_angle(dir, 0.0);
        assert!((free - dir.normalize()).length() < 1e-4);
    }

    /// #919: the sweeps to a hovered End-B candidate — the bearing turned in the ground
    /// plane and the lift out of it, each arc starting where the last one left off.
    #[test]
    fn direction_sweeps_report_azimuth_and_elevation() {
        let pivot = Vec3::new(1.0, 2.0, 3.0);
        // Due +Y at the pivot's height: a quarter turn of azimuth, no lift.
        let sweeps = move_direction_sweeps(pivot, pivot + Vec3::new(0.0, 10.0, 0.0));
        assert_eq!(sweeps.len(), 1, "no elevation arc when it's flat");
        assert!((sweeps[0].1 - 90.0).abs() < 1e-3, "90° of azimuth, got {}", sweeps[0].1);
        // 45° up along +X: no azimuth turn, 45° of lift.
        let up = Vec3::new(1.0, 0.0, 1.0).normalize() * 10.0;
        let sweeps = move_direction_sweeps(pivot, pivot + up);
        assert_eq!(sweeps.len(), 2);
        assert!(sweeps[0].1.abs() < 1e-3, "no azimuth, got {}", sweeps[0].1);
        assert!((sweeps[1].1 - 45.0).abs() < 1e-2, "45° of lift, got {}", sweeps[1].1);
        // Every arc starts at the pivot's radius and ends on the target.
        let end = *sweeps[1].0.last().unwrap();
        assert!(
            ((end - pivot).length() - 7.5).abs() < 1e-2,
            "the lift arc is drawn at 0.75 r, got {}",
            (end - pivot).length()
        );
        // Straight up: one arc, 90°.
        let sweeps = move_direction_sweeps(pivot, pivot + Vec3::Z * 4.0);
        assert_eq!(sweeps.len(), 1);
        assert!((sweeps[0].1 - 90.0).abs() < 1e-3);
    }

    /// #919: end point C's sweep is the signed spin from the no-spin position.
    #[test]
    fn spin_circle_sweeps_from_the_reference() {
        let circle = SpinCircle {
            center: Vec3::ZERO,
            axis: Vec3::Z,
            reference: Vec3::X,
            radius: 5.0,
        };
        let (arc, degrees) = circle.sweep_to(Vec3::new(0.0, 5.0, 0.0)).expect("a sweep");
        assert!((degrees - 90.0).abs() < 1e-3, "a quarter turn, got {degrees}");
        assert!(arc.len() > 2 && arc[0].x > 0.0, "it starts at the reference");
        let (_, back) = circle.sweep_to(Vec3::new(0.0, -5.0, 0.0)).expect("a sweep");
        assert!((back + 90.0).abs() < 1e-3, "the other way is negative, got {back}");
        // A target on the axis has no bearing to sweep to.
        assert!(circle.sweep_to(Vec3::new(0.0, 0.0, 9.0)).is_none());
    }

    /// #918: the angle grid on the sphere — 90° gives the six axis directions, 45° gives
    /// 26 (two poles and three rings of eight), and every spot sits on the sphere.
    #[test]
    fn angle_snap_sphere_candidates_count_and_lie_on_the_sphere() {
        let centre = Vec3::new(5.0, -2.0, 1.0);
        let ninety = snap_angle_sphere_candidates(centre, 10.0, 90.0);
        assert_eq!(ninety.len(), 6, "the six axis directions");
        for p in &ninety {
            assert!(((*p - centre).length() - 10.0).abs() < 1e-3, "on the sphere: {p}");
        }
        assert_eq!(snap_angle_sphere_candidates(centre, 10.0, 45.0).len(), 26);
        assert_eq!(snap_angle_sphere_candidates(centre, 10.0, 30.0).len(), 62);
        // No spacing, no grid — the geometry-derived spots stand alone.
        assert!(snap_angle_sphere_candidates(centre, 10.0, 0.0).is_empty());
    }

    /// #918: the same spacing around the end-C circle, starting at the zero-spin position.
    #[test]
    fn angle_snap_circle_candidates_ring_the_axis() {
        let centre = Vec3::ZERO;
        let spots = snap_angle_circle_candidates(centre, Vec3::Z, Vec3::X, 4.0, 45.0);
        assert_eq!(spots.len(), 8);
        assert!((spots[0] - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-3, "starts at the reference");
        for p in &spots {
            assert!((p.length() - 4.0).abs() < 1e-3, "on the circle: {p}");
            assert!(p.z.abs() < 1e-3, "in its plane: {p}");
        }
        assert_eq!(
            snap_angle_circle_candidates(centre, Vec3::Z, Vec3::X, 4.0, 90.0).len(),
            4
        );
    }

    /// #914: with end points A and B fixed, end point C rides a circle — four quarter-turn
    /// spots on it, the first carrying the start-side geometry over with no extra spin.
    #[test]
    fn spin_candidates_ring_the_axis_a_quarter_turn_apart() {
        use crate::model::MovePointRef;
        let mut doc = Document::default();
        // One body is enough: the points are read by position, not by which body they name.
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let at = |p: [f32; 3]| {
            Some(MovePointRef::OnEdge {
                body: bkey(0),
                p: crate::hierarchy::quantize_body_point(Vec3::from_array(p)),
            })
        };
        // Start: A at the origin, B 10 up +X, C 4 out along +Y from A.
        // End: A at (0, 0, 100), B 10 along +X from it — the same axis, moved.
        let (start_a, start_b, start_c) =
            (at([0.0, 0.0, 0.0]), at([10.0, 0.0, 0.0]), at([0.0, 4.0, 0.0]));
        let (end_a, end_b) = (at([0.0, 0.0, 100.0]), at([10.0, 0.0, 100.0]));
        let circle = snap_spin_candidates(
            &doc,
            start_a.as_ref(),
            start_b.as_ref(),
            start_c.as_ref(),
            end_a.as_ref(),
            end_b.as_ref(),
        )
        .expect("A, B and C give a circle");
        let (center, spots) = (circle.center, circle.spots(90.0));
        assert!(
            (center - Vec3::new(0.0, 0.0, 100.0)).length() < 1e-3,
            "C is perpendicular to the axis, so its circle centres on end A: {center}"
        );
        assert_eq!(spots.len(), 4, "four spots");
        for p in &spots {
            assert!(
                ((*p - center).length() - 4.0).abs() < 1e-3,
                "each sits at C's radius, got {p}"
            );
            // The circle's plane is perpendicular to the +X axis, through end A.
            assert!(p.x.abs() < 1e-3, "and in the circle's plane, got {p}");
        }
        // The first is the no-extra-spin position: straight over from start C.
        assert!(
            (spots[0] - Vec3::new(0.0, 4.0, 100.0)).length() < 1e-3,
            "the first spot carries C over unspun, got {}",
            spots[0]
        );
        // A quarter turn about +X takes +Y to +Z.
        assert!(
            (spots[1] - Vec3::new(0.0, 0.0, 104.0)).length() < 1e-3,
            "a quarter turn about the axis, got {}",
            spots[1]
        );
    }

    /// #914: a start point C sitting **on** the axis can't be spun anywhere, so nothing is
    /// offered rather than four coincident spots.
    #[test]
    fn spin_candidates_refuse_a_point_on_the_axis() {
        use crate::model::MovePointRef;
        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let at = |p: [f32; 3]| {
            Some(MovePointRef::OnEdge {
                body: bkey(0),
                p: crate::hierarchy::quantize_body_point(Vec3::from_array(p)),
            })
        };
        assert!(snap_spin_candidates(
            &doc,
            at([0.0, 0.0, 0.0]).as_ref(),
            at([10.0, 0.0, 0.0]).as_ref(),
            at([5.0, 0.0, 0.0]).as_ref(),
            at([0.0, 0.0, 100.0]).as_ref(),
            at([10.0, 0.0, 100.0]).as_ref(),
        )
        .is_none());
    }

    #[test]
    fn snap_b_pair_rotates_start_b_onto_end_b() {
        use crate::hierarchy::quantize_body_point as q;
        use crate::model::{MoveOperation, MovePointRef, MoveTranslateMode};
        // One triangle body with corners at the origin, +10X and +10Y.
        let mut doc = Document::default();
        let (o, x, y) = (
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
        );
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[o, x, y]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let vertex = |p: Vec3| Some(MovePointRef::Vertex { body: bkey(0), p: q(p) });

        // Start A = origin, end A = origin: no translation, so the rotation stands alone.
        // Start B = +10X; end B = +10Y is exactly 10 from the pivot, so it's reachable.
        let op = MoveOperation {
            keep_inputs: false,
            targets: vec![bkey(0)],
            translate_mode: MoveTranslateMode::PointSnap,
            start_point_a: vertex(o),
            end_point_a: vertex(o),
            start_point_b: vertex(x),
            end_point_b: vertex(y),
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: Vec::new(),
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        };
        assert!(op.has_snap_rotation());
        let m = move_op_transform(&doc, &op).expect("transform");
        let landed = m.transform_point3(x);
        assert!(
            (landed - y).length() < 1e-3,
            "start B should land on end B, got {landed:?}"
        );
        // The pivot (end point A) doesn't move.
        let held = m.transform_point3(o);
        assert!(held.length() < 1e-3, "the pivot holds, got {held:?}");

        // Without the B pair it's a pure translation — no turn.
        let translate_only = MoveOperation { start_point_b: None, end_point_b: None, ..op.clone() };
        assert!(!translate_only.has_snap_rotation());
        let m = move_op_transform(&doc, &translate_only).expect("transform");
        assert!((m.transform_point3(x) - x).length() < 1e-3, "nothing turns");

        // The constraint sphere: radius = |startA - startB| = 10 about end point A.
        assert_eq!(
            snap_rotation_radius(&doc, op.start_point_a.as_ref(), op.start_point_b.as_ref()),
            Some(10.0)
        );
        let reachable = |p: Vec3| {
            snap_rotation_reachable(
                &doc,
                op.start_point_a.as_ref(),
                op.start_point_b.as_ref(),
                op.end_point_a.as_ref(),
                p,
            )
        };
        assert!(reachable(y), "10 from the pivot is on the sphere");
        assert!(reachable(Vec3::new(0.0, 0.0, 10.0)), "any direction, same radius");
        assert!(!reachable(Vec3::new(0.0, 40.0, 0.0)), "too far to reach");
        assert!(!reachable(Vec3::new(0.0, 2.0, 0.0)), "too close to reach");
    }

    /// The optional C pair pins the spin about `end A → end B` that the B pair leaves free,
    /// so the placement is fully determined rather than free to roll.
    #[test]
    fn snap_c_pair_pins_the_spin_b_leaves_free() {
        use crate::hierarchy::quantize_body_point as q;
        use crate::model::{MoveOperation, MovePointRef, MoveTranslateMode};
        // A body with the origin, +10X and +10Z as corners, so C has something off the
        // start A → start B line to aim with.
        let mut doc = Document::default();
        let (o, x, z) = (
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 10.0),
        );
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[o, x, z]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let vertex = |p: Vec3| Some(MovePointRef::Vertex { body: bkey(0), p: q(p) });
        // Targets are picked as points *on* geometry rather than corners (what the end-point
        // pickers hand back), so they can sit anywhere in space.
        let at = |p: Vec3| Some(MovePointRef::OnEdge { body: bkey(0), p: q(p) });

        // A holds the origin still and B keeps +10X where it is, so B's turn is the identity
        // and the bodies are free to spin about the X axis — exactly the ambiguity C fixes.
        let base = MoveOperation {
            keep_inputs: false,
            targets: vec![bkey(0)],
            translate_mode: MoveTranslateMode::PointSnap,
            start_point_a: vertex(o),
            end_point_a: vertex(o),
            start_point_b: vertex(x),
            end_point_b: at(x),
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: Vec::new(),
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        };
        assert!(base.has_snap_rotation() && !base.has_snap_roll());
        // With B alone, +10Z stays put — the spin is undecided, so nothing turns.
        let m = move_op_transform(&doc, &base).expect("transform");
        assert!((m.transform_point3(z) - z).length() < 1e-3, "B alone leaves the spin free");

        // C says +10Z should end up at +10Y: a quarter turn about the X axis.
        let y = Vec3::new(0.0, 10.0, 0.0);
        let op = MoveOperation {
            start_point_c: vertex(z),
            end_point_c: at(y),
            ..base.clone()
        };
        assert!(op.has_snap_roll());
        let (axis, angle) = move_snap_roll_axis_angle(&doc, &op).expect("roll");
        assert!((axis - Vec3::X).length() < 1e-4, "spins about end A → end B, got {axis:?}");
        // A quarter turn, negative about +X: the right-hand rule takes +Y to +Z, and this
        // goes the other way.
        assert!(
            (angle + std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "a quarter turn back about +X, got {angle}"
        );
        let m = move_op_transform(&doc, &op).expect("transform");
        let landed = m.transform_point3(z);
        assert!((landed - y).length() < 1e-3, "start C should land on end C, got {landed:?}");
        // A and B still hold what they were pinning.
        assert!(m.transform_point3(o).length() < 1e-3, "the pivot holds");
        assert!((m.transform_point3(x) - x).length() < 1e-3, "end B holds");

        // Only C's direction *about* the axis counts: a target further out along the same
        // bearing asks for the same turn, since distance is A's and B's to decide.
        let far = MoveOperation {
            end_point_c: at(Vec3::new(5.0, 40.0, 0.0)),
            ..op.clone()
        };
        let (_, far_angle) = move_snap_roll_axis_angle(&doc, &far).expect("roll");
        assert!((far_angle - angle).abs() < 1e-4, "same bearing, same turn");

        // A C point on the axis itself has no bearing to line up, so there's no spin to
        // derive — the move falls back to what B alone gives.
        let on_axis = MoveOperation { start_point_c: vertex(x), ..op.clone() };
        assert!(move_snap_roll_axis_angle(&doc, &on_axis).is_none());
        let m = move_op_transform(&doc, &on_axis).expect("transform");
        assert!((m.transform_point3(z) - z).length() < 1e-3, "no spin derived, none applied");

        // #1078: the same turn asked for as an **angle** instead of a target point. It needs
        // no third point at all — the spin is simply that many degrees about the axis, which
        // is the whole reason to offer it.
        let by_angle = MoveOperation { roll_angle: "-90".to_string(), ..base.clone() };
        assert!(by_angle.has_snap_roll_angle() && !by_angle.has_snap_roll());
        let (axis, a) = move_snap_roll_axis_angle(&doc, &by_angle).expect("roll");
        assert!((axis - Vec3::X).length() < 1e-4, "{axis:?}");
        assert!((a + std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{a}");
        let landed = move_op_transform(&doc, &by_angle).expect("transform").transform_point3(z);
        assert!((landed - y).length() < 1e-3, "an angle places it exactly as the point did");

        // A picked end point C wins over a typed angle: it says where the part should face,
        // where a number only says how far to turn it.
        let both = MoveOperation { roll_angle: "17".to_string(), ..op.clone() };
        assert!(!both.has_snap_roll_angle(), "the point form answers");
        let landed = move_op_transform(&doc, &both).expect("transform").transform_point3(z);
        assert!((landed - y).length() < 1e-3, "still C's quarter turn, not 17°");
    }

    /// #670: the reachable end-point-B spots are where body edges cross the constraint
    /// sphere. Bodies being moved don't offer any — start B has to land on something that
    /// stays put.
    #[test]
    fn snap_rotation_candidates_are_edge_sphere_crossings() {
        let mut doc = Document::default();
        // A single edge running along X from (-10, 0, 0) to (10, 0, 0), as a degenerate
        // triangle so the mesh has that edge.
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[
                Vec3::new(-10.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(0.0, 6.0, 0.0),
            ]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });

        // A sphere of radius 5 about the origin crosses that edge at ±5 along X.
        let found = snap_rotation_candidates(&doc, &[], Vec3::ZERO, 5.0);
        let xs: Vec<f32> = found
            .iter()
            .filter(|(_, p)| p.y.abs() < 1e-3)
            .map(|(_, p)| p.x)
            .collect();
        assert!(
            xs.iter().any(|x| (x - 5.0).abs() < 1e-3)
                && xs.iter().any(|x| (x + 5.0).abs() < 1e-3),
            "expected crossings at ±5 along X, got {xs:?}"
        );
        assert!(found.iter().all(|(bi, _)| *bi == bkey(0)), "each candidate names its body");
        // Every candidate really is on the sphere.
        for (_, p) in &found {
            assert!((p.length() - 5.0).abs() < 1e-3, "{p:?} is off the sphere");
        }

        // A body that's being moved offers nothing, and a sphere that misses entirely too.
        assert!(snap_rotation_candidates(&doc, &[bkey(0)], Vec3::ZERO, 5.0).is_empty());
        assert!(snap_rotation_candidates(&doc, &[], Vec3::new(0.0, 0.0, 100.0), 5.0).is_empty());
        // A degenerate radius offers nothing rather than dividing by zero.
        assert!(snap_rotation_candidates(&doc, &[], Vec3::ZERO, 0.0).is_empty());
    }

    /// The rotation's axis+angle drive the white preview arc: sweeping the translated
    /// start B by the reported angle about the reported axis lands it exactly on end B.
    #[test]
    fn snap_rotation_axis_angle_sweeps_start_b_onto_end_b() {
        use crate::model::{MovePointRef, MoveOperation, MoveTranslateMode};
        let q = crate::hierarchy::quantize_body_point;
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[
                Vec3::ZERO,
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(0.0, 10.0, 0.0),
            ]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let op = MoveOperation {
            keep_inputs: false,
            targets: vec![bkey(0)],
            translate_mode: MoveTranslateMode::PointSnap,
            start_point_a: Some(MovePointRef::Vertex { body: bkey(0), p: q(Vec3::ZERO) }),
            end_point_a: Some(MovePointRef::Vertex { body: bkey(0), p: q(Vec3::new(10.0, 0.0, 0.0)) }),
            start_point_b: Some(MovePointRef::Vertex { body: bkey(0), p: q(Vec3::new(10.0, 0.0, 0.0)) }),
            end_point_b: Some(MovePointRef::OnEdge { body: bkey(0), p: q(Vec3::new(10.0, 10.0, 0.0)) }),
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: Vec::new(),
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        };
        let (axis, angle) = move_snap_rotation_axis_angle(&doc, &op).unwrap();
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "quarter turn, got {angle}");
        let pivot = Vec3::new(10.0, 0.0, 0.0);
        let p0 = Vec3::new(10.0, 0.0, 0.0) + move_op_translation(&doc, &op).unwrap();
        let swept = pivot + glam::Quat::from_axis_angle(axis, angle) * (p0 - pivot);
        assert!(
            (swept - Vec3::new(10.0, 10.0, 0.0)).length() < 1e-3,
            "the full sweep lands on end B, got {swept:?}"
        );
    }

    /// #745: edges whose line passes through end point A extend straight out to the
    /// sphere, offering mid-air landing spots along the edge's direction — even with a
    /// radius larger than the edge itself. Edges that miss the pivot offer nothing.
    #[test]
    fn snap_rotation_axis_candidates_extend_edges_through_the_pivot() {
        let mut doc = Document::default();
        // Two edges through the origin (along X and along Y) and one that misses it.
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[
                Vec3::new(-10.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(0.0, 10.0, 0.0),
            ]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });

        // Radius 40 dwarfs every edge: nothing crosses the sphere on-edge, but the X
        // edge's line passes through the origin and lands spots at ±40. The triangle's
        // other two edges miss the origin, so X is the only qualifying direction.
        let found = snap_rotation_axis_candidates(&doc, &[], Vec3::ZERO, 40.0);
        assert_eq!(found.len(), 2, "one qualifying edge line, two ends: {found:?}");
        assert!(
            found.iter().any(|(_, p)| (*p - Vec3::new(40.0, 0.0, 0.0)).length() < 1e-3)
                && found.iter().any(|(_, p)| (*p - Vec3::new(-40.0, 0.0, 0.0)).length() < 1e-3),
            "expected mid-air spots at ±40 along X, got {found:?}"
        );
        for (_, p) in &found {
            assert!((p.length() - 40.0).abs() < 1e-3, "{p:?} is off the sphere");
        }
        // Moving bodies and degenerate radii offer nothing.
        assert!(snap_rotation_axis_candidates(&doc, &[bkey(0)], Vec3::ZERO, 40.0).is_empty());
        assert!(snap_rotation_axis_candidates(&doc, &[], Vec3::ZERO, 0.0).is_empty());
    }

    /// #648/#650: a Snap move only overrides the X/Y/Z expressions once **both** points are
    /// picked — while one is missing (or there are no bodies at all, as for a plane or image
    /// move) the expressions still drive it, so the tool stays usable mid-pick.
    #[test]
    fn snap_move_falls_back_to_expressions_until_both_points_are_picked() {
        use crate::model::{MovePointRef, MoveOperation, MoveTranslateMode};
        let doc = Document::default();
        let base = MoveOperation {
            keep_inputs: false,
            targets: Vec::new(),
            translate_mode: MoveTranslateMode::PointSnap,
            start_point_a: None,
            end_point_a: None,
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: "7".to_string(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: Vec::new(),
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        };
        assert!(!base.has_snap_translation());
        assert_eq!(
            move_op_translation(&doc, &base),
            Some(Vec3::new(7.0, 0.0, 0.0)),
            "no points yet: the expressions still drive it"
        );
        // One point isn't enough either.
        let half = MoveOperation {
            start_point_a: Some(MovePointRef::Vertex { body: bkey(0), p: [0; 3] }),
            ..base.clone()
        };
        assert!(!half.has_snap_translation());
        assert_eq!(move_op_translation(&doc, &half), Some(Vec3::new(7.0, 0.0, 0.0)));
        // With both, the snap takes over — and points that no longer resolve contribute
        // nothing rather than killing the op.
        let full = MoveOperation {
            end_point_a: Some(MovePointRef::Vertex { body: bkey(1), p: [100, 0, 0] }),
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            ..half
        };
        assert!(full.has_snap_translation());
        assert_eq!(move_op_translation(&doc, &full), Some(Vec3::ZERO));
    }

    /// #949: the axis a candidate end point B would turn the bodies about — a quarter turn in
    /// the XY plane goes about Z, one in XZ about Y — so the dots can be coloured by it.
    #[test]
    fn the_axis_a_candidate_end_b_turns_about() {
        use crate::model::MovePointRef;
        let q = crate::hierarchy::quantize_body_point;
        let mut doc = Document::default();
        // One triangle body with corners at the origin, +10X and +10Y.
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[Vec3::ZERO, Vec3::X * 10.0, Vec3::Y * 10.0]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        // A pair pins the origin in place; start B points along +X from it.
        let start_a = MovePointRef::Vertex { body: bkey(0), p: q(Vec3::ZERO) };
        let start_b = MovePointRef::Vertex { body: bkey(0), p: q(Vec3::X * 10.0) };
        let axis = |target: Vec3| {
            snap_rotation_axis_toward(&doc, Some(&start_a), Some(&start_b), Some(&start_a), target)
        };
        // +X → +Y is a quarter turn about +Z; +X → +Z is one about −Y.
        assert!((axis(Vec3::Y * 10.0).unwrap() - Vec3::Z).length() < 1e-4);
        assert!((axis(Vec3::Z * 10.0).unwrap() + Vec3::Y).length() < 1e-4);
        // Straight ahead is no turn at all, so there's no axis to colour by.
        assert_eq!(axis(Vec3::X * 10.0), None);
        // A missing point leaves it unresolved rather than guessing.
        assert_eq!(
            snap_rotation_axis_toward(&doc, Some(&start_a), None, Some(&start_a), Vec3::Y),
            None
        );
    }

    /// #946: the world origin is a Move point of its own — it resolves to (0, 0, 0) with no
    /// body behind it, so a body's corner can be snapped onto the origin.
    #[test]
    fn the_world_origin_is_a_move_point() {
        use crate::model::{MoveOperation, MovePointRef, MoveTranslateMode};
        let q = crate::hierarchy::quantize_body_point;
        let empty = Document::default();
        let origin = MovePointRef::Origin;
        assert_eq!(origin.body(), None, "the origin belongs to no body");
        assert_eq!(move_point_world(&empty, &origin), Some(Vec3::ZERO));
        // An empty document has no bodies at all, and the origin still resolves — a corner
        // of a body that isn't there doesn't.
        assert_eq!(
            move_point_world(&empty, &MovePointRef::Vertex { body: bkey(0), p: [0; 3] }),
            None
        );

        // A move onto it snaps the source corner to (0, 0, 0).
        let mut doc = Document::default();
        let corner = Vec3::new(40.0, 40.0, 0.0);
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[corner, corner + Vec3::X * 10.0, corner + Vec3::Y * 10.0]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let op = MoveOperation {
            keep_inputs: false,
            targets: vec![bkey(0)],
            translate_mode: MoveTranslateMode::PointSnap,
            start_point_a: Some(MovePointRef::Vertex { body: bkey(0), p: q(corner) }),
            end_point_a: Some(MovePointRef::Origin),
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: Vec::new(),
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        };
        assert!(op.has_snap_translation());
        assert_eq!(
            move_op_translation(&doc, &op),
            Some(Vec3::new(-40.0, -40.0, 0.0))
        );
    }

    /// #644: the distance gizmo hangs off the targets' **start** plane along the axis, centred
    /// on them in the other two directions, so the handle sits at `anchor + dir * distance`.
    #[test]
    fn repeat_gizmo_anchor_sits_on_the_start_plane() {
        let mut doc = Document::default();
        // A triangle spanning x in [10, 30], y in [0, 4], flat at z = 0.
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: vec![[
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(30.0, 0.0, 0.0),
                Vec3::new(20.0, 6.0, 0.0),
            ]],
            source_name: "tri".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        let (anchor, dir) = repeat_gizmo_anchor(&doc, &[bkey(0)], crate::model::RevolveAxis::X)
            .expect("anchor resolves");
        assert_eq!(dir, Vec3::X);
        // Along X the anchor pins to the minimum (10); across it, the centroid (y = 2).
        assert!((anchor.x - 10.0).abs() < 1e-4, "start plane, got {anchor:?}");
        assert!((anchor.y - 2.0).abs() < 1e-4, "centroid across the axis, got {anchor:?}");
        // No targets, or an axis that can't resolve, gives no gizmo.
        assert!(repeat_gizmo_anchor(&doc, &[], crate::model::RevolveAxis::X).is_none());
        assert!(repeat_gizmo_anchor(&doc, &[bkey(0)], crate::model::RevolveAxis::Line(lkey(9))).is_none());
    }

    /// #643: a body feature edge resolves as an axis (origin `a`, unit direction `a → b`) and
    /// goes dead with the body it was picked on.
    #[test]
    fn axis_world_resolves_a_body_edge() {
        let mut doc = Document::default();
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let axis = crate::model::RevolveAxis::BodyEdge {
            body: bkey(0),
            a: Vec3::new(1.0, 2.0, 3.0),
            b: Vec3::new(1.0, 7.0, 3.0),
        };
        let (origin, dir) = axis_world(&doc, axis).expect("live body resolves");
        assert_eq!(origin, Vec3::new(1.0, 2.0, 3.0));
        assert!((dir - Vec3::Y).length() < 1e-6, "unit direction along a → b, got {dir:?}");
        // A degenerate edge has no direction.
        assert!(axis_world(
            &doc,
            crate::model::RevolveAxis::BodyEdge { body: bkey(0), a: Vec3::ZERO, b: Vec3::ZERO }
        )
        .is_none());
        // A deleted body takes its edges with it.
        doc.bodies.remove(bkey(0));
        assert!(axis_world(&doc, axis).is_none());
        assert!(axis_world(&doc, crate::model::RevolveAxis::BodyEdge {
            body: bkey(9),
            a: Vec3::ZERO,
            b: Vec3::X
        })
        .is_none());
    }

    /// #260: descendants walk forward through operations — a body feeding a boolean whose output
    /// feeds a move op yields both downstream bodies, but not unrelated bodies.
    #[test]
    fn descendant_bodies_walks_downstream_operations() {
        let mut doc = Document::default();
        for _ in 0..5 {
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Imported(crate::arena::Key::from_bits(0)),
                material: None,
                name: None,
                shadow: false,
            });
        }
        // body0 + body1 -> boolean -> body2; body2 -> move -> body3. body4 is unrelated.
        doc.boolean_ops.insert(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Combine,
            a: vec![bkey(0)],
            b: vec![bkey(1)],
            keep_b: false,
            outputs: vec![bkey(2)],
            name: None,
        });
        doc.move_ops.insert(crate::model::MoveOperation {
            keep_inputs: false,
            translate_mode: Default::default(),
            start_point_a: None,
            end_point_a: None,
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            targets: vec![bkey(2)],
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: vec![bkey(3)],
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        });

        let d = descendant_bodies(&doc, &[bkey(0)]);
        assert!(d.contains(&bkey(2)), "boolean output is downstream of body 0");
        assert!(d.contains(&bkey(3)), "moved output is downstream transitively");
        assert!(!d.contains(&bkey(0)) && !d.contains(&bkey(1)), "seeds/siblings aren't descendants");
        assert!(!d.contains(&bkey(4)), "unrelated body isn't a descendant");
    }

    fn sketch_doc() -> (Document, crate::model::SketchId) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        (doc, sketch)
    }

    /// #840: a curved path carries the copies along its bend, spaced by arc length.
    #[test]
    fn a_curved_path_carries_the_copies_along_it() {
        use crate::model::{Line, RepeatMode, RepeatOperation, RevolveAxis};
        let (mut doc, sketch) = sketch_doc();
        // A quarter-circle-ish bend from (0,0) to (40,40).
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 40.0, 40.0));
        doc.lines[lkey(0)].bezier = Some([(40.0, 0.0), (40.0, 0.0)]);
        assert!(doc.lines[lkey(0)].is_curved());
        let op = RepeatOperation {
            targets: Vec::new(),
            plane_targets: vec![pkey(0)],
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            axis: RevolveAxis::Line(lkey(0)),
            path_circle: None,
            around_axis: false,
            flip: false,
            mode: RepeatMode::CountGap,
            count: "4".to_string(),
            spacing: "15".to_string(),
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            name: None,
        };
        let path = repeat_path_polyline(&doc, op.axis).expect("a curved path");
        assert!(path.len() > 2, "a curve samples to a polyline");

        let offsets = repeat_offsets(&doc, &op).expect("offsets");
        assert_eq!(offsets, vec![15.0, 30.0, 45.0], "spaced by arc length");

        // Each copy sits on the curve, not on the straight line between its ends.
        let m = repeat_instance_transform(&doc, &op, 1).expect("transform");
        let p = m.transform_point3(Vec3::ZERO);
        assert!(p.x > 0.0 && p.y > 0.0, "moved along the bend, got {p:?}");
        assert!(
            p.x > p.y,
            "the bend leans along +X first, so the first copy is right of the chord: {p:?}"
        );
        // Arc length really is the spacing: consecutive copies are ~15mm apart along the curve.
        let a = repeat_instance_transform(&doc, &op, 1).unwrap().transform_point3(Vec3::ZERO);
        let b = repeat_instance_transform(&doc, &op, 2).unwrap().transform_point3(Vec3::ZERO);
        let straight = (b - a).length();
        assert!(straight > 10.0 && straight < 15.1, "chord under the 15mm arc, got {straight}");

        // A curved path is followed, never turned about, even if the flag is set.
        let turned = RepeatOperation { around_axis: true, ..op.clone() };
        assert_eq!(repeat_offsets(&doc, &turned), repeat_offsets(&doc, &op));

        // A straight line is not a path polyline — it keeps the along-the-axis maths.
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        assert!(repeat_path_polyline(&doc, RevolveAxis::Line(lkey(1))).is_none());
        assert!(repeat_path_polyline(&doc, RevolveAxis::Z).is_none());

        // A circle is a path too (#840): the copies ride round its circumference.
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 30.0, 0.0));
        let ring = repeat_path_polyline_of(&doc, RevolveAxis::Z, Some(rkey(0))).expect("a circle path");
        assert!(ring.len() > 8, "sampled round");
        assert!(
            (ring.first().unwrap() - ring.last().unwrap()).length() < 1e-3,
            "the ring closes"
        );
        let circumference: f32 = ring.windows(2).map(|p| (p[1] - p[0]).length()).sum();
        let exact = std::f32::consts::TAU * 30.0;
        assert!(
            (circumference - exact).abs() < exact * 0.01,
            "sampled circumference ≈ 2πr, got {circumference}"
        );
    }

    /// #989: a path has two directions and picking one says nothing about which you meant, so
    /// `flip` runs the pattern the other way. It reverses all three kinds of step, and
    /// `repeat_offset_transform` is the only place it has to be applied — every preview, ghost
    /// and output goes through there.
    #[test]
    fn flip_runs_the_pattern_the_other_way_along_every_kind_of_path() {
        use crate::model::{Line, RepeatMode, RepeatOperation, RevolveAxis};
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let op = |axis: RevolveAxis, around: bool, flip: bool| RepeatOperation {
            targets: Vec::new(),
            plane_targets: vec![pkey(0)],
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            axis,
            path_circle: None,
            around_axis: around,
            flip,
            mode: RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: if around { "90".to_string() } else { "10".to_string() },
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            name: None,
        };
        let step1 = |doc: &Document, o: &RepeatOperation| {
            repeat_instance_transform(doc, o, 1)
                .expect("transform")
                .transform_point3(Vec3::ZERO)
        };

        // Sliding along a straight axis: the copies march the opposite way.
        let along = step1(&doc, &op(RevolveAxis::X, false, false));
        let along_flipped = step1(&doc, &op(RevolveAxis::X, false, true));
        assert!(along.x > 0.0, "unflipped runs +X, got {along:?}");
        assert!(
            (along_flipped + along).length() < 1e-4,
            "flipped is the exact negation, got {along_flipped:?} against {along:?}"
        );

        // Turning about it: the sense of the turn reverses. A point off the axis lands on the
        // mirrored side.
        let probe = Vec3::new(10.0, 0.0, 0.0);
        let turn = |doc: &Document, flip: bool| {
            repeat_instance_transform(doc, &op(RevolveAxis::Z, true, flip), 1)
                .expect("transform")
                .transform_point3(probe)
        };
        let (turned, turned_flipped) = (turn(&doc, false), turn(&doc, true));
        assert!(turned.y > 1.0, "a +90° turn about Z sends +X to +Y, got {turned:?}");
        assert!(
            turned_flipped.y < -1.0,
            "flipped it turns the other way, got {turned_flipped:?}"
        );

        // A curved path is followed from the other end rather than stepped backwards off its
        // start — so the copies stay on the path either way.
        doc.lines.insert(Line {
            bezier: Some([(20.0, 0.0), (40.0, 20.0)]),
            ..Line::from_local_endpoints(sketch, 0.0, 0.0, 40.0, 40.0)
        });
        let curved = |doc: &Document, flip: bool| {
            let mut o = op(RevolveAxis::Line(lkey(0)), false, flip);
            o.spacing = "15".to_string();
            step1(doc, &o)
        };
        let (curve, curve_flipped) = (curved(&doc, false), curved(&doc, true));
        assert!(curve.x > 0.0, "unflipped leaves the start along the bend, got {curve:?}");
        // From the far end the first step heads back toward the origin, so it moves the other
        // way in Y — and it is a real point on the curve, not an extrapolation past an end.
        assert!(
            curve_flipped.y < 0.0 && curve_flipped.x < 0.0,
            "flipped follows the path back from its far end, got {curve_flipped:?}"
        );
    }

    /// #845: the pick/hover path's per-body mesh analyses are memoized on the same document
    /// fingerprint the mesh cache uses — repeated calls reuse the work, and any geometry
    /// change invalidates it.
    #[test]
    fn body_mesh_analyses_are_cached_and_invalidated() {
        let (mut doc, _sketch, ext) = box_doc(); // 10x10 footprint, 5 tall
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });

        let first = body_face_groups(&doc, bkey(0));
        let again = body_face_groups(&doc, bkey(0));
        assert!(std::rc::Rc::ptr_eq(&first, &again), "the second call reuses the first");
        assert_eq!(first.len(), 6, "a box has six faces");
        assert!(!body_feature_edges(&doc, bkey(0)).is_empty());
        assert!(!body_edge_chains(&doc, bkey(0)).is_empty());

        // Changing the geometry invalidates it: the taller box's faces are recomputed.
        let before = first.clone();
        doc.extrusions[xkey(0)].distance = 40.0;
        let after = body_face_groups(&doc, bkey(0));
        assert!(!std::rc::Rc::ptr_eq(&before, &after), "a geometry edit rebuilds the groups");
        let height = |groups: &Vec<Vec<[Vec3; 3]>>| {
            groups
                .iter()
                .flatten()
                .flatten()
                .map(|p| p.z)
                .fold(f32::MIN, f32::max)
        };
        assert!(height(&after) > height(&before), "the rebuilt groups are the taller box");
    }

    /// #839: a rotational repeat turns its copies about the axis — six 60° steps put the
    /// last copy at 300°, and the transform rotates rather than slides.
    #[test]
    fn a_rotational_repeat_turns_its_copies_about_the_axis() {
        use crate::model::{RepeatMode, RepeatOperation, RevolveAxis};
        let doc = Document::default();
        let op = |around: bool, spacing: &str| RepeatOperation {
            targets: Vec::new(),
            plane_targets: vec![pkey(0)],
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            axis: RevolveAxis::Z,
            path_circle: None,
            around_axis: around,
            flip: false,
            mode: RepeatMode::CountGap,
            count: "6".to_string(),
            spacing: spacing.to_string(),
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            name: None,
        };
        let angles = repeat_offsets(&doc, &op(true, "60deg")).expect("angles");
        assert_eq!(angles.len(), 5, "6 instances = the original plus 5 copies");
        assert!((angles[0] - 60.0).abs() < 1e-3, "{angles:?}");
        assert!((angles[4] - 300.0).abs() < 1e-3, "{angles:?}");

        // The instance transform is a turn about the axis, not a slide along it.
        let m = repeat_instance_transform(&doc, &op(true, "90deg"), 1).expect("transform");
        let p = m.transform_point3(Vec3::new(10.0, 0.0, 0.0));
        assert!((p - Vec3::new(0.0, 10.0, 0.0)).length() < 1e-3, "got {p:?}");

        // The same op along the axis still slides.
        let m = repeat_instance_transform(&doc, &op(false, "10"), 1).expect("transform");
        let p = m.transform_point3(Vec3::new(10.0, 0.0, 0.0));
        assert!((p - Vec3::new(10.0, 0.0, 10.0)).length() < 1e-3, "got {p:?}");
    }

    /// #837: an extrude's faces split into the solids they make — profiles that touch (nested
    /// or overlapping) stay together, ones that share nothing come apart.
    #[test]
    fn disjoint_face_groups_splits_profiles_that_dont_touch() {
        let (mut doc, sketch) = sketch_doc();
        // Two circles far apart, plus one nested inside the first (a hole in its wall).
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 4.0, 0.0));
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 40.0, 0.0, 5.0, 0.0));
        let faces = vec![
            ExtrudeFace::Circle(rkey(0)),
            ExtrudeFace::Circle(rkey(1)),
            ExtrudeFace::Circle(rkey(2)),
        ];
        let groups = disjoint_face_groups(&doc, &faces);
        assert_eq!(groups.len(), 2, "the ring is one solid, the far circle another: {groups:?}");
        assert!(groups.iter().any(|g| g.len() == 2), "the nested pair stays together");
        assert!(groups.iter().any(|g| g == &[ExtrudeFace::Circle(rkey(2))]));

        // One profile alone is one group, and overlapping profiles are one solid.
        assert_eq!(disjoint_face_groups(&doc, &faces[..1]).len(), 1);
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 44.0, 0.0, 5.0, 0.0));
        let overlapping = vec![ExtrudeFace::Circle(rkey(2)), ExtrudeFace::Circle(rkey(3))];
        assert_eq!(disjoint_face_groups(&doc, &overlapping).len(), 1, "overlapping profiles fuse");
    }

    /// #835: the extent the in-sketch repeat measures its gap/distance from — how far the
    /// picked entities themselves reach along the repeat direction.
    #[test]
    fn sketch_repeat_extent_spans_the_picked_entities_along_the_direction() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(crate::model::Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 20.0, 0.0, 3.0, 0.0));
        let op = |dir_u: f32,
                  dir_v: f32,
                  lines: Vec<crate::model::LineKey>,
                  circles: Vec<crate::model::CircleKey>| {
            crate::model::SketchRepeatOperation {
                sketch,
                line_targets: lines,
                circle_targets: circles,
                dir_u,
                dir_v,
                mode: crate::model::RepeatMode::CountGap,
                count: "3".to_string(),
                spacing: "5".to_string(),
                length: String::new(),
                line_outputs: Vec::new(),
                circle_outputs: Vec::new(),
                name: None,
            }
        };
        // Along U the line spans 0..10.
        let e = sketch_repeat_extent(&doc, &op(1.0, 0.0, vec![lkey(0)], Vec::new())).unwrap();
        assert!((e - 10.0).abs() < 1e-3, "got {e}");
        // Along V it's edge-on: no extent.
        let e = sketch_repeat_extent(&doc, &op(0.0, 1.0, vec![lkey(0)], Vec::new())).unwrap();
        assert!(e.abs() < 1e-3, "got {e}");
        // The circle contributes its radius either side of its centre.
        let e = sketch_repeat_extent(&doc, &op(1.0, 0.0, Vec::new(), vec![rkey(0)])).unwrap();
        assert!((e - 6.0).abs() < 1e-3, "got {e}");
        // Both together span 0..23.
        let e = sketch_repeat_extent(&doc, &op(1.0, 0.0, vec![lkey(0)], vec![rkey(0)])).unwrap();
        assert!((e - 23.0).abs() < 1e-3, "got {e}");
        // Nothing picked, or a degenerate direction, has no extent to measure.
        assert!(sketch_repeat_extent(&doc, &op(1.0, 0.0, Vec::new(), Vec::new())).is_none());
        assert!(sketch_repeat_extent(&doc, &op(0.0, 0.0, vec![lkey(0)], Vec::new())).is_none());
    }

    /// #260: the live-edit descendant preview relies on [`body_solid_mesh_uncached_pub`] being a
    /// pure function of the document, so writing an in-progress edit into a scratch clone flows
    /// through to a downstream body's geometry. Here a moved body follows its move op's `tx`.
    #[test]
    fn uncached_mesh_follows_scratch_doc_edit() {
        let (mut doc, _sketch, ext) = box_doc();
        doc.extrusions.insert(ext);
        // body 0: the extruded box; body 1: a moved copy of it.
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        doc.move_ops.insert(crate::model::MoveOperation {
            keep_inputs: false,
            translate_mode: Default::default(),
            start_point_a: None,
            end_point_a: None,
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            targets: vec![bkey(0)],
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: "0mm".to_string(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: vec![bkey(1)],
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Moved {
                op: mopkey(0),
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });

        let before = body_solid_mesh_uncached_pub(&doc, bkey(1)).and_then(|m| m.bounds()).unwrap();
        // Simulate an in-progress move-gizmo drag on a scratch clone: shift tx by 20mm.
        let mut scratch = doc.clone();
        scratch.move_ops.values_mut().nth(0).unwrap().tx = "20mm".to_string();
        let after = body_solid_mesh_uncached_pub(&scratch, scratch.body_at(1).unwrap()).and_then(|m| m.bounds()).unwrap();

        assert!(
            (after.0.x - before.0.x - 20.0).abs() < 1e-3,
            "moved body's mesh must follow the scratch edit's tx (before {:?}, after {:?})",
            before.0,
            after.0,
        );
    }

    /// Drop a rectangle (four lines + a closed-loop polygon face) and return its `Polygon`
    /// profile — the rectangle profile every extrude test used to build from a `Rect`.
    fn rect_profile(
        doc: &mut Document,
        sketch: crate::model::SketchId,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> ExtrudeFace {
        let lines = crate::construction::add_line_rectangle(doc, sketch, x, y, w, h, [false; 4]);
        ExtrudeFace::Polygon(lines.to_vec())
    }

    /// A body built from a 10x10x5 box (extrusion 0) with a 4x4 column (extrusion 1, centered)
    /// cut through it (#35): source `Solid { add: [0], cut: [1] }`.
    fn cut_body_doc() -> Document {
        let (mut doc, sketch) = sketch_doc();
        let outer = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let inner = rect_profile(&mut doc, sketch, 3.0, 3.0, 4.0, 4.0);
        doc.extrusions.insert(extrusion(sketch, vec![outer], 5.0));
        doc.extrusions.insert(extrusion(sketch, vec![inner], 5.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![xkey(0)],
                cut: vec![xkey(1)],
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc
    }

    fn box_doc() -> (Document, crate::model::SketchId, Extrusion) {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let ext = extrusion(sketch, vec![profile], 5.0);
        (doc, sketch, ext)
    }

    /// A selected body face contributes its whole coplanar triangle group to the selection
    /// bounds, so framing it (zoom-to-selection, auto-zoom's selection watch) takes in the
    /// entire face — not just the centroid point its selection key stores.
    #[test]
    fn selection_bounds_cover_a_body_faces_full_extent() {
        let (mut doc, _sketch, ext) = box_doc(); // 10x10 footprint, 5 tall
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let solid = body_solid_mesh(&doc, bkey(0)).unwrap();
        let cap = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid)
            .into_iter()
            .find(|tris| {
                (tris[0][1] - tris[0][0])
                    .cross(tris[0][2] - tris[0][0])
                    .normalize_or_zero()
                    .z
                    > 0.9
            })
            .unwrap();
        let count = (cap.len() * 3) as f32;
        let centroid = cap.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
        let normal = (cap[0][1] - cap[0][0])
            .cross(cap[0][2] - cap[0][0])
            .normalize_or_zero();
        let q = crate::hierarchy::quantize_body_point;
        let mut selection = crate::selection::SceneSelection::default();
        selection.insert(crate::hierarchy::SceneElement::BodyFace {
            body: bkey(0),
            centroid: q(centroid),
            normal: q(normal),
        });
        let (min, max) = selection_world_bounds(&doc, &selection).unwrap();
        assert!((max.x - min.x - 10.0).abs() < 1e-3, "covers the cap's x extent");
        assert!((max.y - min.y - 10.0).abs() < 1e-3, "covers the cap's y extent");
        assert!(max.z - min.z < 1e-3, "the cap is flat");
    }

    /// #1379: shifting a Free-move selection's bounds by the live translation shifts every
    /// translation-handle origin and the rotation-ring centre by the same amount — the gizmos
    /// travel with the preview rather than staying anchored on the original body.
    #[test]
    fn free_move_gizmos_travel_with_the_live_translation() {
        let (mut doc, _sketch, ext) = box_doc(); // 10x10 footprint, 5 tall
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let bodies = [bkey(0)];
        let shift = glam::Vec3::new(3.0, -2.0, 7.0);

        // The translation arrow origins ride the AABB, so translating the bounds carries them.
        let (min, max) = free_move_targets_bounds(&doc, &bodies, &[]).unwrap();
        let at_rest = free_move_translation_handles(min, max);
        let travelled =
            free_move_translation_handles(min + shift, max + shift);
        for (rest, moved) in at_rest.iter().zip(travelled.iter()) {
            assert!(
                (moved.origin - (rest.origin + shift)).length() < 1e-4,
                "arrow {} should follow the preview, origin {:?} vs {:?}",
                moved.axis,
                moved.origin,
                rest.origin + shift
            );
            assert_eq!(rest.axis, moved.axis);
            assert!((rest.outward - moved.outward).length() < 1e-4);
        }

        // And the rotation-ring centre follows the preview too (radius is unchanged by a pure
        // translation).
        let (rest_center, rest_radius) = free_move_rotation_ring(min, max);
        let (moved_center, moved_radius) =
            free_move_rotation_ring(min + shift, max + shift);
        assert!((moved_center - (rest_center + shift)).length() < 1e-4);
        assert!((moved_radius - rest_radius).abs() < 1e-4);
    }

    /// #1413: Free-move rotation rings each get a deterministic handle reference, and the
    /// three starting positions spread around the body rather than overlapping.
    #[test]
    fn free_move_rotation_handles_are_distinct_and_follow_the_preview() {
        let (mut doc, _sketch, ext) = box_doc(); // 10x10 footprint, 5 tall
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let bodies = [bkey(0)];
        let (min, max) = free_move_targets_bounds(&doc, &bodies, &[]).unwrap();

        // The three base references are pairwise-distinct.
        let (b0, b1, b2) = (free_move_rotation_base_dir(0), free_move_rotation_base_dir(1), free_move_rotation_base_dir(2));
        assert_ne!(b0, b1);
        assert_ne!(b0, b2);
        assert_ne!(b1, b2);

        // At rest, the three handles sit at distinct spots around the ring.
        let zero = glam::Vec3::ZERO;
        let rest = free_move_rotation_handles(&doc, min, max, zero, "", "", "").unwrap();
        for i in 0..3 {
            for j in (i + 1)..3 {
                assert!(
                    (rest[i] - rest[j]).length() > 1e-3,
                    "handles {i} and {j} overlap at rest: {rest:?}"
                );
            }
        }

        // #1414: rotating one ring (here 90° about Z) rotates all three handles along with the
        // preview, including the ring being turned — every base direction follows the composed
        // Free turn. Each handle ends at the ring centre plus Q·(rest offset).
        let rz90 = free_move_rotation_handles(&doc, min, max, zero, "", "", "90").unwrap();
        let about_z = glam::Quat::from_rotation_z(90f32.to_radians());
        let c = (min + max) * 0.5 + zero;
        for i in 0..3 {
            let expected = c + about_z * (rest[i] - c);
            assert!(
                rz90[i].distance(expected) < 1e-3,
                "ring {i} handle should follow the preview turn: {} vs {expected}",
                rz90[i]
            );
        }
        // The handles whose bases aren't on the turned axis (X ring's +Y and Z ring's +X) move
        // with the body; the Y ring's base (+Z) sits on the Z axis and rightly stays put.
        assert!((rz90[0] - rest[0]).length() > 1e-3, "X ring handle should have moved");
        assert!((rz90[2] - rest[2]).length() > 1e-3, "Z ring handle should have moved");
    }

    /// #1415: the Free-move turn expressions stay signed — a negative typed turn reads back
    /// negative rather than wrapping to a 0-360 value.
    #[test]
    fn free_move_turns_stay_signed() {
        let (mut doc, _sketch, ext) = box_doc();
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let bodies = [bkey(0)];
        let (min, max) = free_move_targets_bounds(&doc, &bodies, &[]).unwrap();
        // A -5° typed turn must evaluate to -5°, not 355°.
        let got = crate::value::eval_angle_rad_in_doc("-5", &doc).unwrap().to_degrees();
        assert!((got - -5.0).abs() < 1e-3, "typed -5° should stay -5°, got {got}");
        let handles = free_move_rotation_handles(&doc, min, max, glam::Vec3::ZERO, "", "", "-5").unwrap();
        // And the labelled handle position still follows the signed turn.
        let q = glam::Quat::from_rotation_z((-5f32).to_radians());
        let c = (min + max) * 0.5;
        let expected = c + q * free_move_rotation_base_dir(2) * free_move_rotation_ring(min, max).1;
        assert!(handles[2].distance(expected) < 1e-3, "Z handle should sit at the signed -5° position");
    }

    /// #186: a repeat's fill length can be bound to a target's extended plane (like an
    /// The extracted spacing-mode math (#222) is input-only and covers every mode: count×gap
    /// steps by extent+gap; the fit modes divide the span; the fill modes count how many fit.
    #[test]
    fn spacing_offsets_covers_every_mode() {
        use crate::model::RepeatMode;
        let f = super::spacing_offsets;
        // Count × gap: extent 10, gap 5 → step 15; 3 instances → offsets 15, 30.
        assert_eq!(f(RepeatMode::CountGap, 10.0, Some(3), Some(5.0), None), Some(vec![15.0, 30.0]));
        // Count fit-to-end: 3 instances across L=40 with extent 10 → step (40-10)/2 = 15.
        assert_eq!(f(RepeatMode::CountFitEnds, 10.0, Some(3), None, Some(40.0)), Some(vec![15.0, 30.0]));
        // Count fit start-to-start: 3 instances across span 40 → step 20.
        assert_eq!(f(RepeatMode::CountFitCenters, 0.0, Some(3), None, Some(40.0)), Some(vec![20.0, 40.0]));
        // Count-fit with < 2 instances is just the original (empty extras).
        assert_eq!(f(RepeatMode::CountFitEnds, 10.0, Some(1), None, Some(40.0)), Some(Vec::new()));
        // Fill by gap: L=40, extent 10, gap 5 → step 15 → n = floor((40-10)/15)+1 = 3.
        assert_eq!(f(RepeatMode::FillGap, 10.0, None, Some(5.0), Some(40.0)), Some(vec![15.0, 30.0]));
        // Fill by pitch: L=40, pitch 10 → n = floor((40-0)/10)+1 = 5 with extent 0.
        assert_eq!(f(RepeatMode::FillPitch, 0.0, None, Some(10.0), Some(40.0)), Some(vec![10.0, 20.0, 30.0, 40.0]));
        // Missing inputs / degenerate steps don't evaluate.
        assert_eq!(f(RepeatMode::CountGap, 10.0, None, Some(5.0), None), None);
        assert_eq!(f(RepeatMode::CountGap, 10.0, Some(3), None, None), None);

        // #257 new modes:
        // Count × pitch: 3 instances at pitch 15 → offsets 15, 30 (extent doesn't matter).
        assert_eq!(f(RepeatMode::CountPitch, 10.0, Some(3), Some(15.0), None), Some(vec![15.0, 30.0]));
        // Fill span by gap: span 40, extent 10, gap 5 → step 15 → n = floor(40/15)+1 = 3.
        assert_eq!(f(RepeatMode::FillGapSpan, 10.0, None, Some(5.0), Some(40.0)), Some(vec![15.0, 30.0]));
        // Fill span by pitch: span 40, pitch 20 → n = floor(40/20)+1 = 3 → offsets 20, 40.
        assert_eq!(f(RepeatMode::FillPitchSpan, 0.0, None, Some(20.0), Some(40.0)), Some(vec![20.0, 40.0]));
    }

    /// extrusion's "up to face"), so `L` is the along-axis distance to that plane and follows
    /// it — overriding the `length` expression.
    #[test]
    fn repeat_fill_length_follows_a_face_target() {
        use crate::model::{Body, BodySource, ExtrudeTarget, RepeatMode, RepeatOperation, RevolveAxis};
        let (mut doc, sketch, ext) = box_doc(); // 10x10x5 box, x∈[0,10]
        let _ = sketch;
        doc.extrusions.insert(ext);
        doc.bodies.insert(Body {
            source: BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![] },
            material: None,
            name: None,
            shadow: false,
        });
        // A target plane at x = 30, normal +X (an X-facing wall the repeat fills up to).
        doc.construction_planes.insert(crate::construction::plane_from_definition(
            &crate::construction::definition_from_reference(
                &crate::construction::PlaneReference::Face {
                    origin: glam::Vec3::new(30.0, 0.0, 0.0),
                    normal: glam::Vec3::X,
                    label: "wall".to_string(),
                },
                0.0,
                0.0,
            ),
            crate::model::ConstructionPlaneParent::Root,
        ));
        let plane_index = doc.construction_planes.keys().last().unwrap();

        let mut op = RepeatOperation {
            targets: vec![bkey(0)],
            plane_targets: Vec::new(),
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            axis: RevolveAxis::X,
            path_circle: None,
            around_axis: false,
            flip: false,
            mode: RepeatMode::FillPitch,
            count: String::new(),
            spacing: "10".to_string(),
            length: "999".to_string(), // deliberately wrong; the target must win
            length_target: Some(ExtrudeTarget::Plane(plane_index)),
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            name: None,
        };
        // L = 30 (x=0 start → x=30 plane), pitch 10, extent 10 → n = ((30-10)/10)+1 = 3.
        assert_eq!(repeat_offsets(&doc, &op), Some(vec![10.0, 20.0]));

        // Move the plane out to x = 50: L follows → n = ((50-10)/10)+1 = 5 → 4 extra instances.
        doc.construction_planes[plane_index].origin = glam::Vec3::new(50.0, 0.0, 0.0);
        assert_eq!(repeat_offsets(&doc, &op), Some(vec![10.0, 20.0, 30.0, 40.0]));

        // Clearing the target falls back to the (wrong) expression → many instances.
        op.length_target = None;
        let fallback = repeat_offsets(&doc, &op).expect("expression length");
        assert!(fallback.len() > 4, "expression length 999 should place many instances");
    }

    /// #146: exporting a document with two *intersecting* bodies unions them, so the exported
    /// mesh's volume is the union (no double-counted overlap), not the sum of the two.
    #[test]
    fn document_solid_mesh_unions_intersecting_bodies() {
        let (mut doc, sketch) = sketch_doc();
        // Two 10x10x5 boxes overlapping in x∈[5,10]: union volume 500+500-250 = 750.
        let a = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let b = rect_profile(&mut doc, sketch, 5.0, 0.0, 10.0, 10.0);
        doc.extrusions.insert(extrusion(sketch, vec![a], 5.0));
        doc.extrusions.insert(extrusion(sketch, vec![b], 5.0));
        for ei in 0..2 {
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Extrusion(xkey(ei)),
                material: None,
                name: None,
                shadow: false,
            });
        }
        let vol = mesh_signed_volume(&document_solid_mesh(&doc)).abs();
        assert!(
            (vol - 750.0).abs() < 5.0,
            "expected union volume ~750, got {vol} (concatenation would be ~1000)"
        );
        assert!(
            mesh_is_watertight(&document_solid_mesh(&doc)),
            "unioned export mesh must be watertight (#1286)"
        );
    }

    /// #1286: whole-document STL export must be a closed manifold. A BREP sphere's OCCT
    /// tessellation can emit degenerate pole triangles; the export path must not ship those.
    #[test]
    fn document_solid_mesh_of_a_sphere_is_watertight() {
        let mut doc = Document::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Sphere);
        shape.radius = "10".into();
        let pi = doc.primitives.insert(shape);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        let mesh = document_solid_mesh(&doc);
        assert!(!mesh.is_empty(), "sphere should produce triangles");
        assert!(
            mesh_is_watertight(&mesh),
            "document export of a sphere must be watertight (#1286); got {} tris",
            mesh.triangles.len()
        );
        // Divergence-theorem volume should match a solid ball (hand mesh ≈ 2% of exact).
        let vol = mesh_signed_volume(&mesh).abs();
        let exact = 4.0 / 3.0 * std::f32::consts::PI * 1000.0;
        assert!(
            (vol - exact).abs() / exact < 0.05,
            "sphere export volume {vol} vs exact {exact}"
        );
    }

    /// #1286: ASCII STL of a whole document (sphere) round-trips as a watertight mesh —
    /// the bytes a slicer/printer actually reads.
    #[test]
    fn stl_export_of_document_sphere_is_watertight() {
        let mut doc = Document::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Sphere);
        shape.radius = "10".into();
        let pi = doc.primitives.insert(shape);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        let mesh = document_solid_mesh(&doc);
        let text = crate::stl::write_ascii_stl("bearcad", &mesh);
        let parsed = crate::stl::parse_ascii_stl(&text).expect("round-trip parse");
        let reimport = SolidMesh {
            triangles: parsed.iter().map(|t| t.vertices).collect(),
        };
        assert!(
            mesh_is_watertight(&reimport),
            "re-parsed STL must stay watertight (#1286); {} tris",
            reimport.triangles.len()
        );
    }


    /// #263/#268: two concentric circles resolve a click in the ring (inside the outer,
    /// outside the inner) to `Difference(outer, inner)` — the ring face — and a click in the
    /// inner disc to `Intersection` (the inner face).
    #[test]
    fn concentric_circles_resolve_a_ring_and_an_inner_face() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0)); // outer
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 4.0, 0.0)); // inner
        let outer = ExtrudeFace::Circle(rkey(0));
        let inner = ExtrudeFace::Circle(rkey(1));

        // The outer circle's unique overlapping partner is the inner one.
        assert_eq!(overlapping_partner(&doc, sketch, &outer), Some(inner.clone()));

        // A point in the ring (radius 7) → Difference(outer − inner) = the ring.
        let ring = resolve_boolean_click(&doc, sketch, &outer, &inner, (7.0, 0.0));
        assert!(
            matches!(
                ring,
                Some(ExtrudeFace::Boolean { op: crate::model::BooleanOp::Difference, .. })
            ),
            "ring click should resolve to a Difference face, got {ring:?}"
        );

        // A point in the inner disc (radius 1) → Intersection = the inner disc.
        let center = resolve_boolean_click(&doc, sketch, &outer, &inner, (1.0, 0.0));
        assert!(matches!(
            center,
            Some(ExtrudeFace::Boolean { op: crate::model::BooleanOp::Intersection, .. })
        ));
    }

    /// #268/#263: the concentric-ring (annulus) face resolves to a fillable region — its outer
    /// loop with the inner circle as a hole — so `face_region_world` reports one hole.
    #[test]
    fn ring_face_resolves_to_a_holed_region() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 4.0, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(rkey(0))),
            b: Box::new(ExtrudeFace::Circle(rkey(1))),
        };
        // Previously rejected (annulus) — now the outer loop resolves via the region.
        let (outer, holes, _n) = face_region_world(&doc, &ring).expect("ring region");
        assert!(outer.len() >= 3, "outer boundary present");
        assert_eq!(holes.len(), 1, "inner circle becomes one hole");
    }

    /// #285: extruding the glyph 'o' builds a hollow ring — its counter (hole) comes out, so the
    /// volume is well below a solid fill of the glyph's outer boundary. Skips without a font.
    #[test]
    fn extruding_letter_o_is_hollow() {
        let family = ["Helvetica", "Arial", "DejaVu Sans", "Liberation Sans"]
            .into_iter()
            .find(|f| crate::text::font_bytes(f, false, false).is_some());
        let Some(family) = family else { return };
        let (mut doc, sketch) = sketch_doc();
        let (shaped, bytes) =
            crate::text::shape_with_system_font(family, false, false, 20.0, "o").expect("shape o");
        doc.sketch_texts.insert(crate::model::SketchText {
            sketch,
            text: "o".to_string(),
            font_family: family.to_string(),
            bold: false,
            italic: false,
            underline: false,
            size: 20.0,
            size_expr: "20".to_string(),
            origin: (0.0, 0.0),
            rotation: 0.0,
            wrap_width: None,
            baseline_line: None,
            contours: shaped.contours,
            font_bytes: bytes,
            pin: None,
            name: None,
        });
        let glyph_face = ExtrudeFace::TextGlyph { text: tkey(0), glyph: 0 };
        // Solid fill of just the outer boundary, for comparison.
        let (outer, holes, _n) = face_region_world(&doc, &glyph_face).expect("region");
        assert_eq!(holes.len(), 1, "o has a counter hole");
        let outer_area = {
            let mut a = 0.0f32;
            let n = outer.len();
            for i in 0..n {
                let j = (i + 1) % n;
                a += outer[i].x * outer[j].y - outer[j].x * outer[i].y;
            }
            a.abs() * 0.5
        };
        doc.extrusions.insert(extrusion(sketch, vec![glyph_face], 5.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("o mesh")).abs();
        let solid_fill = outer_area * 5.0;
        assert!(
            vol > 1.0 && vol < solid_fill * 0.85,
            "hollow 'o' volume {vol} should be well under the solid-fill {solid_fill}",
        );
    }

    /// #268: extruding the concentric ring builds a **tube** — outer cylinder minus inner
    /// cylinder — with volume π(R² − r²)·h, not the full disc π·R²·h.
    #[test]
    fn ring_extrusion_is_a_hollow_tube() {
        let (mut doc, sketch) = sketch_doc();
        let (big_r, small_r, h) = (10.0_f32, 4.0_f32, 20.0_f32);
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, big_r, 0.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, small_r, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(rkey(0))),
            b: Box::new(ExtrudeFace::Circle(rkey(1))),
        };
        doc.extrusions.insert(extrusion(sketch, vec![ring], h));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("tube mesh")).abs();
        let expected = std::f32::consts::PI * (big_r * big_r - small_r * small_r) * h;
        assert!(
            (vol - expected).abs() / expected < 0.02,
            "tube volume {vol} should be ~{expected} (π(R²−r²)h), not the full disc",
        );
    }

    /// #519: hovering an annular (boolean-difference) cap for extrusion must report its hole
    /// so the highlight cuts the opening out instead of filling across it. `cap_hole_loops_world`
    /// returns the ring's one hole, lifted to the requested cap along the extrusion normal.
    #[test]
    fn cap_hole_loops_report_the_ring_hole_at_the_cap() {
        let (mut doc, sketch) = sketch_doc();
        let h = 20.0_f32;
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 4.0, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(rkey(0))),
            b: Box::new(ExtrudeFace::Circle(rkey(1))),
        };
        doc.extrusions.insert(extrusion(sketch, vec![ring.clone()], h));

        let base = cap_hole_loops_world(&doc, xkey(0), &ring, false);
        let top = cap_hole_loops_world(&doc, xkey(0), &ring, true);
        assert_eq!(base.len(), 1, "the ring has one hole on the base cap");
        assert_eq!(top.len(), 1, "and one hole on the top cap");

        // The two caps' holes are the same ring, separated by the extrusion height along z.
        let base_z = base[0][0].z;
        let top_z = top[0][0].z;
        assert!(
            (base_z - top_z).abs() - h < 0.05,
            "top hole should sit ~{h} above the base hole (base z={base_z}, top z={top_z})",
        );

        // A simply-connected cap (a plain disc) has no holes.
        doc.extrusions.insert(extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], h));
        let disc = cap_hole_loops_world(&doc, xkey(1), &ExtrudeFace::Circle(rkey(0)), true);
        assert!(disc.is_empty(), "a solid disc cap reports no holes");
    }

    /// #177: chamfering a cylinder's top rim through the kernel removes an annular ring
    /// (~perimeter * d^2/2 for a 45-degree chamfer).
    #[test]
    fn circle_boss_rim_chamfer_removes_a_ring() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut ext = extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 20.0);
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: true },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        let cylinder = std::f32::consts::PI * 100.0 * 20.0;
        let ring = std::f32::consts::PI * 2.0 * (10.0 - 2.0 / 3.0) * 2.0;
        let expected = cylinder - ring;
        assert!(
            (vol - expected).abs() < 30.0,
            "expected ~{expected} (rim chamfered), got {vol} (untreated would be ~{cylinder})"
        );
    }

    /// #177: fillet works on circular rims too — a cut hole's rim fillets into a
    /// rounded-over lead-in (removes the (1 - pi/4) corner ring), through the same
    /// post-subtraction body path as chamfer countersinks.
    #[test]
    fn cut_hole_rim_fillet_rounds_the_hole_edge() {
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, -10.0, -10.0, 20.0, 20.0);
        doc.extrusions.insert(extrusion(sketch, vec![plate], 5.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 2.5, 0.0));
        let mut hole = extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 6.0);
        hole.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            kind: VertexTreatmentKind::Fillet,
            amount: 1.0,
        });
        doc.extrusions.insert(hole);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![xkey(1)] },
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        let plain = 2000.0 - std::f32::consts::PI * 2.5 * 2.5 * 5.0;
        // Rounded-over ring: (1 - pi/4) r^2 cross-section revolved near the hole radius.
        let ring = (1.0 - std::f32::consts::FRAC_PI_4)
            * 2.0
            * std::f32::consts::PI
            * (2.5 + 0.223);
        let expected = plain - ring;
        assert!(
            (vol - expected).abs() < 3.0,
            "expected ~{expected} (rounded hole edge), got {vol} (plain would be ~{plain})"
        );
    }

    /// #220: repeating a cut extrusion replays the hole along the axis — a plate with one hole
    /// repeated ×3 loses three holes' worth of material, not one.
    #[test]
    fn repeat_cut_extrusion_punches_n_holes() {
        use crate::model::{RepeatMode, RepeatOperation, RevolveAxis};
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, -10.0, -10.0, 20.0, 20.0); // 20×20×5
        doc.extrusions.insert(extrusion(sketch, vec![plate], 5.0));
        // A 2.5mm-radius hole at x = -6.
        doc.circles.insert(Circle::from_local_center_radius(sketch, -6.0, 0.0, 2.5, 0.0));
        doc.extrusions.insert(extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 6.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![xkey(1)] },
            material: None,
            name: None,
            shadow: false,
        });
        let one_hole = 2000.0 - std::f32::consts::PI * 2.5 * 2.5 * 5.0;
        assert!((mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).unwrap()).abs() - one_hole).abs() < 3.0);

        // Replay the hole (extrusion 1) ×3 along X at 6mm gap → holes at x = -6, 0, +6.
        doc.repeat_ops.insert(RepeatOperation {
            targets: Vec::new(),
            plane_targets: Vec::new(),
            extrusion_targets: vec![xkey(1)],
            sketch_targets: Vec::new(),
            axis: RevolveAxis::X,
            path_circle: None,
            around_axis: false,
            flip: false,
            mode: RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "6".to_string(),
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            name: None,
        });
        let three_holes = 2000.0 - 3.0 * std::f32::consts::PI * 2.5 * 2.5 * 5.0;
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).unwrap()).abs();
        assert!(
            (vol - three_holes).abs() < 6.0,
            "expected ~{three_holes} (3 holes), got {vol} (one hole is ~{one_hole})"
        );
    }

    /// #220: repeating an *add* extrusion fuses the solid at each offset — one box becomes three
    /// disjoint boxes (union volume triples).
    #[test]
    fn repeat_add_extrusion_grows_n_bodies() {
        use crate::model::{RepeatMode, RepeatOperation, RevolveAxis};
        let (mut doc, sketch) = sketch_doc();
        let box_face = rect_profile(&mut doc, sketch, 0.0, 0.0, 4.0, 4.0); // 4×4
        doc.extrusions.insert(extrusion(sketch, vec![box_face], 5.0)); // ×5 = 80
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![] },
            material: None,
            name: None,
            shadow: false,
        });
        assert!((mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).unwrap()).abs() - 80.0).abs() < 1.0);

        // Replay the add ×3 along X at 10mm gap → boxes at x = 0, 10, 20 (disjoint).
        doc.repeat_ops.insert(RepeatOperation {
            targets: Vec::new(),
            plane_targets: Vec::new(),
            extrusion_targets: vec![xkey(0)],
            sketch_targets: Vec::new(),
            axis: RevolveAxis::X,
            path_circle: None,
            around_axis: false,
            flip: false,
            mode: RepeatMode::CountGap,
            count: "3".to_string(),
            spacing: "10".to_string(),
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            name: None,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).unwrap()).abs();
        assert!((vol - 240.0).abs() < 3.0, "expected ~240 (3 boxes), got {vol}");
    }

    /// Ancestor→descendant propagation: a body moved by a parameter expression follows edits to
    /// that parameter. Regression guard for the mesh-cache fingerprint — `doc.parameters` /
    /// `doc.move_ops` must be part of it, or the moved body keeps a stale cached mesh.
    #[test]
    fn parameter_edit_propagates_to_a_moved_descendant() {
        use crate::model::{Body, BodySource, MoveOperation, Parameter};
        let (mut doc, _sketch, ext) = box_doc(); // box x ∈ [0, 10]
        doc.extrusions.insert(ext);
        doc.bodies.insert(Body {
            source: BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![] },
            material: None,
            name: None,
            shadow: true, // consumed by the move
        });
        doc.parameters.insert(Parameter {
            name: "gap".to_string(),
            expression: "10".to_string(),
            primary: false,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        doc.move_ops.insert(MoveOperation {
            keep_inputs: false,
            translate_mode: Default::default(),
            start_point_a: None,
            end_point_a: None,
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            targets: vec![bkey(0)],
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: "gap".to_string(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: vec![bkey(1)],
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        });
        doc.bodies.insert(Body {
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
        let min_x = |doc: &Document, bi: crate::model::BodyKey| {
            body_solid_mesh(doc, bi)
                .unwrap()
                .triangles
                .iter()
                .flat_map(|t| t.iter())
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min)
        };
        // The moved copy starts at x = 0 + gap(10).
        assert!((min_x(&doc, bkey(1)) - 10.0).abs() < 1e-3, "moved by gap = 10");
        // Editing the parameter the move references must propagate to the descendant body.
        doc.parameters.values_mut().next().unwrap().expression = "25".to_string();
        assert!(
            (min_x(&doc, bkey(1)) - 25.0).abs() < 1e-3,
            "descendant follows the parameter edit (fingerprint includes parameters/move_ops)"
        );
    }

    /// #177: a chamfer on a *cut* circle extrusion's rim carves a countersink into the
    /// body it cuts — more material removed than the plain hole.
    #[test]
    fn cut_hole_rim_chamfer_countersinks_the_body() {
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, -10.0, -10.0, 20.0, 20.0);
        doc.extrusions.insert(extrusion(sketch, vec![plate], 5.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 2.5, 0.0));
        let mut hole = extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 6.0);
        hole.edge_treatments.push(EdgeTreatment {
            // The hole prism runs z 0..6 through the 5mm plate; its base rim (z=0) is the
            // plate's bottom surface rim.
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            kind: VertexTreatmentKind::Chamfer,
            amount: 1.0,
        });
        doc.extrusions.insert(hole);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![xkey(1)] },
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        let plain = 2000.0 - std::f32::consts::PI * 2.5 * 2.5 * 5.0;
        let countersink = std::f32::consts::PI * 2.0 * (2.5 + 1.0 / 3.0) * 0.5;
        let expected = plain - countersink;
        assert!(
            (vol - expected).abs() < 4.0,
            "expected ~{expected} (countersunk), got {vol} (plain hole would be ~{plain})"
        );
    }

    /// #177: circle cap rims surface as treatable edges (kernel builds), one shared edge
    /// reference per rim.
    #[test]
    fn treatable_edges_include_circle_cap_rims() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.extrusions.insert(extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 6.0));
        let edges = treatable_edges(&doc);
        let tops: Vec<_> = edges
            .iter()
            .filter(|(_, e, _, _)| {
                matches!(e, ExtrusionEdgeRef::Cap { edge: 0, top: true, .. })
            })
            .collect();
        let bases: Vec<_> = edges
            .iter()
            .filter(|(_, e, _, _)| {
                matches!(e, ExtrusionEdgeRef::Cap { edge: 0, top: false, .. })
            })
            .collect();
        assert_eq!(tops.len(), CIRCLE_SEGMENTS);
        assert_eq!(bases.len(), CIRCLE_SEGMENTS);
        assert!(edges
            .iter()
            .all(|(_, e, _, _)| !matches!(e, ExtrusionEdgeRef::Vertical { .. })));
        assert!(extrusion_edge_exists(
            &doc,
            xkey(0),
            ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: true }
        ));
    }

    /// A cut extrusion carrying several faces (two holes cut in one operation) must
    /// subtract all of them — it used to fall off the kernel path entirely, silently
    /// dropping every hole (additive-only fallback).
    #[test]
    fn multi_face_cut_extrusion_subtracts_every_face() {
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, 0.0, 0.0, 50.0, 40.0);
        doc.extrusions.insert(extrusion(sketch, vec![plate], 5.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 35.0, 10.0, 2.5, 0.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 35.0, 30.0, 2.5, 0.0));
        doc.extrusions.insert(extrusion(
            sketch,
            vec![ExtrudeFace::Circle(rkey(0)), ExtrudeFace::Circle(rkey(1))],
            6.0,
        ));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![xkey(1)] },
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        let expected = 10000.0 - 2.0 * std::f32::consts::PI * 2.5 * 2.5 * 5.0;
        assert!(
            (vol - expected).abs() < 20.0,
            "expected ~{expected} (both holes cut), got {vol}"
        );
    }

    /// #142: the live cut preview meshes the target body with the in-progress extrusion already
    /// subtracted, so its volume is less than the intact body's — i.e. it shows the finished
    /// hole, not an additive block.
    #[test]
    fn preview_cut_body_mesh_removes_material() {
        let (mut doc, sketch) = sketch_doc();
        let outer = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        doc.extrusions.insert(extrusion(sketch, vec![outer], 5.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let intact = body_solid_mesh(&doc, bkey(0)).expect("intact box");
        let intact_vol = mesh_signed_volume(&intact).abs();

        // A 4x4 column overlapping the box, extruded through it — the pending cut.
        let hole = rect_profile(&mut doc, sketch, 3.0, 3.0, 4.0, 4.0);
        let cut = extrusion(sketch, vec![hole], 5.0);
        let preview = preview_cut_body_mesh(&doc, bkey(0), &cut).expect("cut preview");
        let preview_vol = mesh_signed_volume(&preview).abs();

        assert!(
            preview_vol < intact_vol - 1.0,
            "cut preview should remove material: {preview_vol} vs {intact_vol}"
        );
        // The pending cut must not have been committed into the real document.
        assert_eq!(doc.extrusions.len(), 1, "preview must not mutate the doc");
    }

    /// #126: an extrusion can target another (already-committed) extrusion's cap face —
    /// not just a construction plane or a flat sketch profile.
    #[test]
    fn body_face_target_reaches_another_extrusions_cap() {
        let (mut doc, sketch) = sketch_doc();
        let base_profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        doc.extrusions.insert(extrusion(sketch, vec![base_profile.clone()], 8.0));

        let second_profile = rect_profile(&mut doc, sketch, 20.0, 0.0, 10.0, 10.0);
        let mut second = extrusion(sketch, vec![second_profile], 3.0);
        second.target = Some(ExtrudeTarget::BodyFace(FaceId::ExtrudeCap {
            extrusion: xkey(0),
            profile: base_profile,
            top: true,
        }));
        doc.extrusions.insert(second);

        let depth = effective_distance(&doc, &doc.extrusions[xkey(1)]);
        assert!(
            (depth - 8.0).abs() < 1e-3,
            "should reach extrusion 0's top cap at z=8, got {depth}"
        );
    }

    /// A body-face target that doesn't resolve (unknown extrusion index) must not silently
    /// fall back to the typed distance's *wrong* value — `target_distance` returns `None` so
    /// `effective_distance` falls back to the plain `distance` field.
    #[test]
    fn body_face_target_with_unknown_extrusion_falls_back_to_typed_distance() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile.clone()], 3.0);
        ext.target = Some(ExtrudeTarget::BodyFace(FaceId::ExtrudeCap {
            extrusion: xkey(99),
            profile,
            top: true,
        }));
        doc.extrusions.insert(ext);
        let depth = effective_distance(&doc, &doc.extrusions[xkey(0)]);
        assert!((depth - 3.0).abs() < 1e-3, "should fall back to distance=3, got {depth}");
    }

    #[test]
    fn line_rectangle_extrudes_to_a_box_of_expected_volume() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 4.0);
        let ext = extrusion(sketch, vec![profile], 6.0);
        let mesh = extrusion_mesh(&doc, &ext).unwrap();
        // A 10x4x6 box: 12 triangles, 240 mm^3, spanning its footprint.
        assert_eq!(mesh.triangles.len(), 12);
        let (min, max) = mesh.bounds().unwrap();
        assert!((max.x - min.x - 10.0).abs() < 1e-4);
        assert!((max.y - min.y - 4.0).abs() < 1e-4);
        assert!((max.z - min.z - 6.0).abs() < 1e-4);
    }

    fn test_revolution(
        sketch: crate::model::SketchId,
        faces: Vec<ExtrudeFace>,
        angle: f32,
        symmetric: bool,
        mode: crate::model::RevolveMode,
    ) -> crate::model::Revolution {
        crate::model::Revolution {
            sketch,
            faces,
            axis: crate::model::RevolveAxis::Y,
            angle_deg: angle,
            pitch_mm: 0.0,
            symmetric,
            mode,
            name: None,
        }
    }

    /// A 10x10 square at x 10..20 revolved 360 degrees around the global Y axis is a
    /// washer: pi * (20^2 - 10^2) * 10.
    #[test]
    fn revolve_full_sweep_makes_a_ring() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 10.0);
        let rev = doc.revolutions.insert(test_revolution(
            sketch,
            vec![profile],
            360.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Revolve(rev),
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        let expected = std::f32::consts::PI * (400.0 - 100.0) * 10.0;
        assert!(
            (vol - expected).abs() < expected * 0.02,
            "expected ~{expected}, got {vol}"
        );
    }

    /// #621: a partial revolve's flat profile caps resolve world polygons (start in the
    /// profile plane, end rotated by the sweep angle), and the axis-perpendicular profile
    /// edges sweep flat annular sides with axis-aligned, outward sketch-frame normals.
    #[test]
    fn revolve_flat_faces_resolve_polygons_and_frames() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 10.0);
        let rev = doc.revolutions.insert(test_revolution(
            sketch,
            vec![profile.clone()],
            90.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        // Start cap: the profile itself, in the sketch (z = 0) plane.
        let (start, _) = revolve_cap_polygon_world(&doc, rev, &profile, false).expect("start cap");
        assert!(start.iter().all(|p| p.z.abs() < 1e-3));
        // End cap: the profile rotated 90° about +Y — (x, y, 0) lands on (0, y, −x).
        let (end, _) = revolve_cap_polygon_world(&doc, rev, &profile, true).expect("end cap");
        assert!(end.iter().all(|p| p.x.abs() < 1e-3 && p.z < 0.0));
        // Exactly the two constant-height rect edges sweep flat sides; their frames'
        // normals run along the axis, pointing away from the profile.
        let flats: Vec<(usize, SketchFrame)> = (0..revolve_side_count(&profile))
            .filter_map(|e| revolve_side_geom(&doc, rev, &profile, e).map(|(_, f, _)| (e, f)))
            .collect();
        assert_eq!(flats.len(), 2, "two axis-perpendicular edges sweep flat sides");
        for (_, frame) in &flats {
            assert!(frame.normal.cross(Vec3::Y).length() < 1e-4);
        }
        let normal_ys: Vec<f32> = flats.iter().map(|(_, f)| f.normal.y).collect();
        assert!(normal_ys.contains(&-1.0) && normal_ys.contains(&1.0));
        // A full sweep closes on itself: no caps, but the flat washer sides remain.
        doc.revolutions[rev].angle_deg = 360.0;
        assert!(revolve_cap_polygon_world(&doc, rev, &profile, false).is_none());
        assert!(revolve_side_geom(&doc, rev, &profile, flats[0].0).is_some());
    }

    /// #626: the tessellated rims of a full revolve chain into whole curves — each circular
    /// rim is ONE chain, so picking any facet selects the entire circle.
    #[test]
    fn revolve_rim_segments_chain_into_whole_curves() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 10.0);
        let rev = doc.revolutions.insert(test_revolution(
            sketch,
            vec![profile],
            360.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Revolve(rev),
            material: None,
            name: None,
            shadow: false,
        });
        let solid = body_solid_mesh(&doc, bkey(0)).expect("mesh");
        let chains = crate::gpu_viewport::solid_mesh_edge_chains(&solid);
        // The ring's only feature edges are its 4 circular rims (inner/outer × both flat
        // ends) — each must gather into a single many-segment chain.
        assert_eq!(chains.len(), 4, "expected 4 rim chains, got {}", chains.len());
        for chain in &chains {
            assert!(chain.len() >= 8, "a rim chain should span many facets");
        }
        // Any single facet expands back to its whole rim, and every facet of a chain maps
        // to the same canonical identity segment.
        let (a, b) = chains[0][0];
        let expanded = crate::gpu_viewport::body_edge_curve_chain(&solid, a, b);
        assert_eq!(expanded.len(), chains[0].len());
        let canon = crate::gpu_viewport::chain_canonical_segment(&chains[0]);
        let (a2, b2) = chains[0][chains[0].len() / 2];
        let canon2 = crate::gpu_viewport::chain_canonical_segment(
            &crate::gpu_viewport::body_edge_curve_chain(&solid, a2, b2),
        );
        assert_eq!(canon, canon2);
    }

    /// #263: revolving the concentric-ring (annulus) face 360° about the Y axis makes a hollow
    /// tube-torus — outer torus minus inner torus. By Pappus, volume = 2π·d·π·(R² − r²) with
    /// d the centre's distance from the axis.
    #[test]
    fn revolve_ring_face_makes_a_hollow_torus() {
        let (mut doc, sketch) = sketch_doc();
        let (d, big_r, small_r) = (20.0_f32, 5.0_f32, 2.0_f32);
        doc.circles.insert(Circle::from_local_center_radius(sketch, d, 0.0, big_r, 0.0));
        doc.circles.insert(Circle::from_local_center_radius(sketch, d, 0.0, small_r, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(rkey(0))),
            b: Box::new(ExtrudeFace::Circle(rkey(1))),
        };
        let rev = doc.revolutions.insert(test_revolution(
            sketch,
            vec![ring],
            360.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Revolve(rev),
            material: None,
            name: None,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("torus mesh")).abs();
        let expected = std::f32::consts::TAU
            * d
            * std::f32::consts::PI
            * (big_r * big_r - small_r * small_r);
        assert!(
            (vol - expected).abs() / expected < 0.03,
            "hollow torus volume {vol} should be ~{expected} (2π·d·π(R²−r²))",
        );
    }

    /// #1242: a non-zero pitch advances the profile along the axis — after one full turn
    /// the end cap sits `pitch` past the start, so the solid's axial span grows by pitch
    /// (on top of the profile's own height). That's a spring coil.
    #[test]
    fn revolve_with_pitch_advances_along_the_axis() {
        let (mut doc, sketch) = sketch_doc();
        // Profile at x 10..20, y 0..5 — axial height 5 along Y.
        let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 5.0);
        let mut rev = test_revolution(
            sketch,
            vec![profile],
            360.0,
            false,
            crate::model::RevolveMode::NewBody,
        );
        rev.pitch_mm = 20.0;
        let mesh = revolve_mesh(&doc, &rev).expect("helical mesh");
        let (min, max) = mesh.bounds().expect("bounds");
        // Pure revolve would span y 0..5; with pitch 20 the end sits at y 20..25.
        assert!(
            (min.y - 0.0).abs() < 0.5,
            "start of helix near y=0, got min.y={}",
            min.y
        );
        assert!(
            (max.y - 25.0).abs() < 1.0,
            "end of helix near y=25 (5 profile + 20 pitch), got max.y={}",
            max.y
        );
        // Multi-turn: 2.5 revolutions with pitch 10 → axial travel 25.
        rev.angle_deg = 900.0; // 2.5 turns
        rev.pitch_mm = 10.0;
        let mesh2 = revolve_mesh(&doc, &rev).expect("multi-turn helix");
        let (min2, max2) = mesh2.bounds().expect("bounds");
        let span = max2.y - min2.y;
        // profile height 5 + 2.5 * 10 pitch = 30
        assert!(
            (span - 30.0).abs() < 2.0,
            "2.5 turns × pitch 10 + height 5 ≈ 30, got span {span}"
        );
    }

    /// #1248/#1249: multi-turn helical revolve (fixture spring, 7200°) stays
    /// interactive (tessellation not 100k+) while the viewport mesh comes from
    /// the smooth OCCT helix BREP — not the density-capped lathe shortcut.
    #[test]
    fn issue_1248_helical_revolve_stays_density_capped() {
        let bytes = include_bytes!("../tests/fixtures/issue_1248.json");
        let mut doc = crate::storage::from_json_bytes(bytes).expect("load");
        doc.bump_mesh_rev();
        let bi = doc.bodies.keys().next().expect("body");
        let rev = doc.revolutions.values().next().expect("revolve").clone();
        assert!(
            rev.pitch_mm.abs() > 1.0 && rev.angle_deg.abs() > 3600.0,
            "fixture should be a multi-turn helical spring"
        );

        assert_eq!(
            revolve_mesh_steps(360.0, 0.0),
            CIRCLE_SEGMENTS,
            "pure revolve keeps CIRCLE_SEGMENTS density"
        );

        // Viewport path must use the kernel solid (smooth helix pipe), not the
        // hand-rolled lathe that #1248 temporarily preferred for speed (#1249).
        let shape = occt_body_shape(&doc, bi).expect("kernel spring BREP");
        let occt_tris = shape.tessellate(OCCT_DEFLECTION as f64);
        let mesh = body_solid_mesh_uncached_pub(&doc, bi).expect("mesh");
        let n_tris = mesh.triangles.len();
        assert_eq!(
            n_tris,
            occt_tris.len(),
            "viewport mesh must be the OCCT tessellation of the smooth spring BREP, \
             not the density-capped lathe (lathe would differ from kernel tri count)"
        );
        assert!(
            n_tris < 40_000,
            "multi-turn spring viewport mesh must stay interactive, got {n_tris} tris"
        );
        // Geometry still spans ~20 turns of pitch along the axis.
        let (min, max) = mesh.bounds().expect("bounds");
        let span = (max - min).max_element();
        assert!(
            span > 400.0,
            "spring should still be long (~20×pitch), span={span}"
        );

        // Cached mesh path: fingerprint-stable reads return the same density (no
        // re-tessellate). View navigation only clones the memoized mesh.
        let again = body_solid_mesh(&doc, bi).expect("cached");
        assert_eq!(again.triangles.len(), n_tris);
        for _ in 0..5 {
            let hit = body_solid_mesh(&doc, bi).expect("hit");
            assert_eq!(hit.triangles.len(), n_tris);
        }
    }

    /// #1249: a pure helical revolve body's viewport mesh is the adaptive
    /// tessellation of a curved BREP — denser than the coarse lathe, and STEP
    /// export goes through real kernel BREP (not FACETED_BREP of the lathe).
    #[test]
    fn issue_1249_helical_revolve_viewport_uses_smooth_brep() {
        let (mut doc, sketch) = sketch_doc();
        // Profile at x 10..15, y 0..4 — small coil wire.
        let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 5.0, 4.0);
        let mut rev = test_revolution(
            sketch,
            vec![profile],
            1080.0, // 3 turns
            false,
            crate::model::RevolveMode::NewBody,
        );
        rev.pitch_mm = 10.0;
        let ri = doc.revolutions.insert(rev.clone());
        let bi = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Revolve(ri),
            name: None,
            material: None,
            shadow: false,
        });
        doc.bump_mesh_rev();

        let lathe = revolve_mesh(&doc, &rev).expect("lathe");
        let mesh = body_solid_mesh_uncached_pub(&doc, bi).expect("body mesh");
        // Smooth BREP tessellation is not the fixed-step lathe (different topology).
        assert_ne!(
            mesh.triangles.len(),
            lathe.triangles.len(),
            "viewport must not use the density-capped lathe for helical revolves"
        );
        // Kernel shape is what STEP export writes as real BREP.
        let shape = occt_body_shape(&doc, bi).expect("STEP path BREP");
        let vol = shape.volume().expect("volume");
        assert!(vol > 1.0, "spring BREP volume, got {vol}");
        let (min, max) = mesh.bounds().expect("bounds");
        // profile height 4 + 3 × pitch 10 = 34
        let span = max.y - min.y;
        assert!(
            (span - 34.0).abs() < 3.0,
            "3-turn spring axial span ~34, got {span}"
        );
    }


    /// #1242: Gap mode stores pitch = gap + profile height; Offset mode stores pitch as-is.
    #[test]
    fn revolve_gap_offset_converts_through_profile_height() {
        let mut state = crate::actions::AppState::default();
        let sketch = state
            .doc
            .add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        let lines = crate::construction::add_line_rectangle(
            &mut state.doc,
            sketch,
            10.0,
            0.0,
            10.0,
            10.0,
            [false; 4],
        );
        let mut cr = crate::actions::CreatingRevolve {
            sketch: Some(sketch),
            faces: vec![ExtrudeFace::Polygon(lines.to_vec())],
            axis: Some(crate::model::RevolveAxis::Y),
            gap_is_offset: true,
            pitch_live: 15.0,
            gap_text: "15".to_string(),
            gap_user_edited: true,
            ..crate::actions::CreatingRevolve::default()
        };
        assert!((cr.evaluated_pitch_mm(&state.doc) - 15.0).abs() < 1e-3);
        // Flip to Gap: shown value becomes pitch − height = 15 − 10 = 5.
        cr.toggle_gap_mode(&state.doc);
        assert!(!cr.gap_is_offset);
        assert!(
            (cr.evaluated_pitch_mm(&state.doc) - 15.0).abs() < 1e-3,
            "pitch preserved"
        );
        // Type a clear gap of 2 → pitch becomes 12.
        cr.gap_text = "2".to_string();
        cr.gap_user_edited = true;
        assert!((cr.evaluated_pitch_mm(&state.doc) - 12.0).abs() < 1e-3);
        // Angle ↔ Revolutions: 720° ↔ 2 turns.
        cr.angle_live = 720.0;
        cr.user_edited = false;
        cr.angle_is_revolutions = false;
        cr.refresh_angle_text_from_live();
        cr.toggle_angle_mode(&state.doc);
        assert!(cr.angle_is_revolutions);
        assert!((cr.evaluated_angle_deg(&state.doc) - 720.0).abs() < 1e-3);
        cr.text = "1.5".to_string();
        cr.user_edited = true;
        assert!((cr.evaluated_angle_deg(&state.doc) - 540.0).abs() < 1e-3);
    }

    /// A 90-degree sweep is a quarter of the ring, symmetric or not.
    #[test]
    fn revolve_partial_sweep_is_proportional_and_symmetric_matches() {
        let expected = std::f32::consts::PI * 300.0 * 10.0 / 4.0;
        for symmetric in [false, true] {
            let (mut doc, sketch) = sketch_doc();
            let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 10.0);
            let rev = doc.revolutions.insert(test_revolution(
                sketch,
                vec![profile],
                90.0,
                symmetric,
                crate::model::RevolveMode::NewBody,
            ));
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Revolve(rev),
                material: None,
                name: None,
                shadow: false,
            });
            let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
            assert!(
                (vol - expected).abs() < expected * 0.02,
                "symmetric={symmetric}: expected ~{expected}, got {vol}"
            );
        }
    }

    /// #revolve cut mode: a revolved ring subtracted from a plate leaves a circular groove.
    #[test]
    fn revolve_cut_carves_the_targeted_body() {
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, -30.0, -30.0, 60.0, 60.0);
        doc.extrusions.insert(extrusion(sketch, vec![plate], 5.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        // Cut tool: a rect profile (x -20..20, y 3..6 in the ground plane) revolved 360
        // degrees around the global X axis — a tube of inner radius 3, outer radius 6,
        // length 40, centered on the X axis. It pierces the plate (z 0..5), so the cut
        // carves a half-buried channel through it.
        let tube = rect_profile(&mut doc, sketch, -20.0, 3.0, 40.0, 3.0);
        let rev = doc.revolutions.insert(crate::model::Revolution {
            sketch,
            faces: vec![tube],
            axis: crate::model::RevolveAxis::X,
            angle_deg: 360.0,
            pitch_mm: 0.0,
            symmetric: false,
            mode: crate::model::RevolveMode::Cut(vec![bkey(0)]),
            name: None,
        });
        let _ = rev;
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        // Removed material = plate ∩ tube: for the z 0..5 slab of an annulus r 3..6 around
        // the X axis over 40 of length. Assert a meaningful bite rather than the exact
        // integral: well below the intact plate, well above nothing.
        let plain = 60.0 * 60.0 * 5.0;
        assert!(
            vol < plain - 100.0 && vol > plain * 0.5,
            "cut should remove a channel: got {vol} vs plain {plain}"
        );
    }

    /// A vertical construction plane (normal Y, u→X, v→Z) for sweep tests: sketch
    /// lines drawn on it run through the ground plane rather than in it.
    fn vertical_path_sketch(doc: &mut Document) -> crate::model::SketchId {
        doc.construction_planes.insert(crate::model::ConstructionPlane {
            origin: Vec3::ZERO,
            normal: Vec3::Y,
            u_axis: Vec3::X,
            v_axis: Vec3::Z,
            parent: crate::model::ConstructionPlaneParent::Root,
            definition: crate::face::default_xy_plane_definition(),
            repeat_instance: None,
            name: None,
            extent: crate::model::PlaneExtent::default(),
        });
        doc.add_sketch(FaceId::ConstructionPlane(doc.construction_planes.keys().last().unwrap()))
    }

    /// #sweep: a 10x10 profile swept along a straight 30mm path normal to its plane
    /// is a plain box — the fallback sweep mesh closes to the exact prism volume.
    #[test]
    fn sweep_straight_path_makes_a_box() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let path_sketch = vertical_path_sketch(&mut doc);
        doc.lines.insert(Line::from_local_endpoints(path_sketch, 5.0, 0.0, 5.0, 30.0));
        let fp = crate::model::Sweep {
            sketch,
            faces: vec![profile],
            path: vec![*doc.lines.keys().collect::<Vec<_>>().iter().rev().nth(0).unwrap()],
            mode: crate::model::SweepMode::NewBody,
            name: None,
        };
        let vol = mesh_signed_volume(&sweep_mesh(&doc, &fp).expect("sweep mesh")).abs();
        assert!(
            (vol - 3000.0).abs() < 30.0,
            "10x10 profile along a straight 30mm path should be ~3000, got {vol}"
        );
    }

    /// #sweep: segments picked out of order chain tip-to-tail, and the chain starts
    /// at the end on the profile plane. An L path (up 20, across 15) picked far-leg-first.
    #[test]
    fn sweep_chains_out_of_order_segments() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, -2.0, -2.0, 4.0, 4.0);
        let ps = vertical_path_sketch(&mut doc);
        doc.lines.insert(Line::from_local_endpoints(ps, 0.0, 20.0, 15.0, 20.0));
        doc.lines.insert(Line::from_local_endpoints(ps, 0.0, 0.0, 0.0, 20.0));
        let fp = crate::model::Sweep {
            sketch,
            faces: vec![profile],
            path: vec![
                *doc.lines.keys().collect::<Vec<_>>().iter().rev().nth(1).unwrap(),
                doc.lines.keys().last().unwrap(),
            ],
            mode: crate::model::SweepMode::NewBody,
            name: None,
        };
        let path = sweep_path_polyline(&doc, &fp).expect("chained polyline");
        assert!(
            path.first().unwrap().z.abs() < 1e-3,
            "path must start on the profile plane, starts at {:?}",
            path.first().unwrap()
        );
        assert!(
            (*path.last().unwrap() - Vec3::new(15.0, 0.0, 20.0)).length() < 1e-3,
            "path must end at the far leg's tip, ends at {:?}",
            path.last().unwrap()
        );
        // The swept solid closes: a 4x4 section over the ~35mm L, corner effects aside.
        let vol = mesh_signed_volume(&sweep_mesh(&doc, &fp).expect("sweep mesh")).abs();
        assert!(vol > 300.0 && vol < 700.0, "L-sweep volume plausible, got {vol}");
    }

    /// #sweep: a disconnected extra segment refuses to chain (no silent gaps).
    #[test]
    fn sweep_rejects_a_disconnected_path() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, -2.0, -2.0, 4.0, 4.0);
        let ps = vertical_path_sketch(&mut doc);
        doc.lines.insert(Line::from_local_endpoints(ps, 0.0, 0.0, 0.0, 20.0));
        doc.lines.insert(Line::from_local_endpoints(ps, 40.0, 0.0, 40.0, 20.0));
        let fp = crate::model::Sweep {
            sketch,
            faces: vec![profile],
            path: vec![
                *doc.lines.keys().collect::<Vec<_>>().iter().rev().nth(1).unwrap(),
                doc.lines.keys().last().unwrap(),
            ],
            mode: crate::model::SweepMode::NewBody,
            name: None,
        };
        assert!(sweep_path_polyline(&doc, &fp).is_none());
        assert!(sweep_mesh(&doc, &fp).is_none());
    }

    /// #sweep: a sweep in Cut mode carves its swept column out of the targeted body
    /// (kernel path, mirroring `revolve_cut_carves_the_targeted_body`).
    #[test]
    fn sweep_cut_carves_the_targeted_body() {
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, -30.0, -30.0, 60.0, 60.0);
        doc.extrusions.insert(extrusion(sketch, vec![plate], 5.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        // Cut tool: a 4x4 profile swept straight through the plate (z -10..10).
        let bit = rect_profile(&mut doc, sketch, -2.0, -2.0, 4.0, 4.0);
        let ps = vertical_path_sketch(&mut doc);
        doc.lines.insert(Line::from_local_endpoints(ps, 0.0, -10.0, 0.0, 10.0));
        doc.sweeps.insert(crate::model::Sweep {
            sketch,
            faces: vec![bit],
            path: vec![*doc.lines.keys().collect::<Vec<_>>().iter().rev().nth(0).unwrap()],
            mode: crate::model::SweepMode::Cut(vec![bkey(0)]),
            name: None,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        let plain = 60.0 * 60.0 * 5.0;
        let expected = plain - 4.0 * 4.0 * 5.0;
        assert!(
            (vol - expected).abs() < 40.0,
            "cut should remove the swept column: got {vol}, expected {expected}"
        );
    }

    /// #479: a loft in Cut mode carves its blended solid out of the targeted body via
    /// the kernel (pairwise ruled ThruSections, fused, then subtracted).
    #[test]
    fn loft_cut_carves_the_targeted_body() {
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, -30.0, -30.0, 60.0, 60.0);
        doc.extrusions.insert(extrusion(sketch, vec![plate], 5.0));
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        // Cut tool: two circles on planes below and above the plate loft into a frustum
        // column punching through it.
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 3.0, 0.0));
        doc.construction_planes.insert(crate::construction::plane_from_definition(
            &crate::construction::definition_from_reference(
                &crate::construction::PlaneReference::Face {
                    origin: glam::Vec3::ZERO,
                    normal: glam::Vec3::Z,
                    label: "Ground".to_string(),
                },
                10.0,
                0.0,
            ),
            crate::model::ConstructionPlaneParent::Root,
        ));
        let top = doc.add_sketch(FaceId::ConstructionPlane(doc.construction_planes.keys().last().unwrap()));
        doc.circles.insert(Circle::from_local_center_radius(top, 0.0, 0.0, 3.0, 0.0));
        doc.lofts.insert(crate::model::Loft {
            sections: vec![
                crate::model::LoftSection { sketch, face: ExtrudeFace::Circle(rkey(0)) },
                crate::model::LoftSection { sketch: top, face: ExtrudeFace::Circle(rkey(1)) },
            ],
            mode: crate::model::LoftMode::Cut(vec![bkey(0)]),
            name: None,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("mesh")).abs();
        let plain = 60.0 * 60.0 * 5.0;
        // The cylinder-ish column removes ~pi*r^2*h through the 5mm plate.
        let expected = plain - std::f32::consts::PI * 3.0 * 3.0 * 5.0;
        assert!(
            (vol - expected).abs() < 20.0,
            "loft cut should remove the column: got {vol}, expected ~{expected}"
        );
    }

    /// Two equal circles on planes 10mm apart loft into a closed prism whose signed
    /// volume matches the swept n-gon (~pi*r^2*h), proving the walls and caps close up.
    /// #399: a loft between circles sketched at the same off-origin (u, v) on Ground and an
    /// offset plane is a straight (vertical) frustum — the offset plane's basis matches
    /// Ground's, so the second ring keeps its in-plane offset instead of collapsing to the
    /// plane centre and leaning the solid.
    #[test]
    fn loft_stays_straight_for_off_origin_sections() {
        let mut doc = Document::default();
        retain_ground_plane_only(&mut doc);
        doc.construction_planes.insert(crate::construction::plane_from_face(
            30.0,
            Vec3::ZERO,
            Vec3::Z,
        ));
        let s0 = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(s0, -30.0, 0.0, 6.0, 0.0));
        let s1 = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(1)));
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(s1, -30.0, 0.0, 3.0, 0.0));
        let loft = crate::model::Loft {
            sections: vec![
                crate::model::LoftSection { sketch: s0, face: ExtrudeFace::Circle(rkey(0)) },
                crate::model::LoftSection { sketch: s1, face: ExtrudeFace::Circle(rkey(1)) },
            ],
            mode: crate::model::LoftMode::NewBody,
            name: None,
        };
        let mesh = loft_mesh(&doc, &loft).expect("loft builds");
        let (min, max) = mesh.bounds().unwrap();
        assert!(
            (min.x + 36.0).abs() < 0.2 && (max.x + 24.0).abs() < 0.2,
            "x spans the r=6 ring at -30, got {min:?}..{max:?}"
        );
        assert!(
            min.y.abs() <= 6.2 && max.y.abs() <= 6.2,
            "y stays within the bottom ring radius (no lean), got {min:?}..{max:?}"
        );
        assert!((max.z - 30.0).abs() < 0.2, "reaches the offset plane");
    }

    #[test]
    fn loft_mesh_between_two_circles_closes_with_expected_volume() {
        let mut doc = Document::default();
        retain_ground_plane_only(&mut doc);
        let bottom = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles.insert(Circle::from_local_center_radius(bottom, 0.0, 0.0, 5.0, 0.0));
        doc.construction_planes.insert(crate::construction::plane_from_definition(
            &crate::construction::definition_from_reference(
                &crate::construction::PlaneReference::Face {
                    origin: glam::Vec3::ZERO,
                    normal: glam::Vec3::Z,
                    label: "Ground".to_string(),
                },
                10.0,
                0.0,
            ),
            crate::model::ConstructionPlaneParent::Root,
        ));
        let top = doc.add_sketch(FaceId::ConstructionPlane(pkey(1)));
        doc.circles.insert(Circle::from_local_center_radius(top, 0.0, 0.0, 5.0, 0.0));

        let loft = crate::model::Loft {
            sections: vec![
                crate::model::LoftSection { sketch: bottom, face: ExtrudeFace::Circle(rkey(0)) },
                crate::model::LoftSection { sketch: top, face: ExtrudeFace::Circle(rkey(1)) },
            ],
            mode: crate::model::LoftMode::NewBody,
            name: None,
        };
        let mesh = loft_mesh(&doc, &loft).expect("two closed sections should loft");
        // Cross section is the inscribed n-gon of the r=5 circle, so slightly under pi*25.
        let ngon_area = 0.5 * CIRCLE_SEGMENTS as f32 * 25.0
            * (2.0 * std::f32::consts::PI / CIRCLE_SEGMENTS as f32).sin();
        let expected = ngon_area * 10.0;
        let vol = mesh_signed_volume(&mesh).abs();
        assert!(
            (vol - expected).abs() < expected * 0.01,
            "expected ~{expected}, got {vol}"
        );
    }

    /// A single section (or an open profile) can't loft.
    #[test]
    fn loft_mesh_requires_two_sections() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let loft = crate::model::Loft {
            sections: vec![crate::model::LoftSection {
                sketch,
                face: ExtrudeFace::Circle(rkey(0)),
            }],
            mode: crate::model::LoftMode::NewBody,
            name: None,
        };
        assert!(loft_mesh(&doc, &loft).is_none());
    }

    /// A picked loft section maps back to the sketch entities that should show a selection
    /// highlight (#202): a circle is itself; a line loop is every line in the loop.
    #[test]
    fn loft_section_scene_elements_covers_circle_and_polygon() {
        use crate::hierarchy::SceneElement;
        let circle = crate::model::LoftSection {
            sketch: skey(0),
            face: ExtrudeFace::Circle(rkey(3)),
        };
        assert_eq!(
            loft_section_scene_elements(&circle),
            vec![SceneElement::Circle(rkey(3))]
        );
        let polygon = crate::model::LoftSection {
            sketch: skey(0),
            face: ExtrudeFace::Polygon(vec![lkey(4), lkey(5), lkey(6)]),
        };
        assert_eq!(
            loft_section_scene_elements(&polygon),
            vec![
                SceneElement::Line(lkey(4)),
                SceneElement::Line(lkey(5)),
                SceneElement::Line(lkey(6)),
            ]
        );
    }

    /// Sections are re-ordered along the loft's principal direction, so pick order
    /// (here: top, bottom, middle) doesn't tangle the blend.
    #[test]
    fn order_loft_sections_sorts_along_principal_direction() {
        let mut doc = Document::default();
        let mut sketches = Vec::new();
        for (i, z) in [(0usize, 0.0f32), (1, 10.0), (2, 5.0)] {
            let plane_idx = if z == 0.0 {
                doc.ground_plane().unwrap()
            } else {
                doc.construction_planes.insert(crate::construction::plane_from_definition(
                    &crate::construction::definition_from_reference(
                        &crate::construction::PlaneReference::Face {
                            origin: glam::Vec3::ZERO,
                            normal: glam::Vec3::Z,
                            label: "Ground".to_string(),
                        },
                        z,
                        0.0,
                    ),
                    crate::model::ConstructionPlaneParent::Root,
                ));
                doc.construction_planes.keys().last().unwrap()
            };
            let sketch = doc.add_sketch(FaceId::ConstructionPlane(plane_idx));
            doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
            sketches.push((i, sketch));
        }
        // Pick order: top (z=10), bottom (z=0), middle (z=5).
        let sections = vec![
            crate::model::LoftSection { sketch: sketches[1].1, face: ExtrudeFace::Circle(rkey(1)) },
            crate::model::LoftSection { sketch: sketches[0].1, face: ExtrudeFace::Circle(rkey(0)) },
            crate::model::LoftSection { sketch: sketches[2].1, face: ExtrudeFace::Circle(rkey(2)) },
        ];
        let ordered = order_loft_sections(&doc, sections);
        let order: Vec<_> = ordered
            .iter()
            .map(|s| match s.face {
                ExtrudeFace::Circle(ci) => ci.index() as usize,
                _ => usize::MAX,
            })
            .collect();
        // Circle i sits at z = [0, 10, 5][i]; either monotonic direction is fine.
        assert!(
            order == vec![0, 2, 1] || order == vec![1, 2, 0],
            "expected monotonic order along z, got {order:?}"
        );
    }

    fn extrusion(sketch: crate::model::SketchId, faces: Vec<ExtrudeFace>, distance: f32) -> Extrusion {
        Extrusion {
            sketch,
            faces,
            distance,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        }
    }

    /// #1243: distance taper of +5 on a 10×10 square makes a 20×20 end face (5 per side).
    #[test]
    fn taper_distance_positive_grows_end_face_by_per_side_amount() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 10.0);
        ext.taper = 5.0;
        ext.taper_mode = ExtrudeTaperMode::Distance;
        let mesh = extrusion_mesh(&doc, &ext).expect("tapered mesh");
        let (min, max) = mesh.bounds().expect("bounds");
        assert!(
            (max.x - min.x - 20.0).abs() < 0.5 && (max.y - min.y - 20.0).abs() < 0.5,
            "end face should span 20×20, got {}×{}",
            max.x - min.x,
            max.y - min.y
        );
        assert!((max.z - min.z - 10.0).abs() < 0.5, "height still 10, got {}", max.z - min.z);
        // Base stays 10×10: the solid's min corner is at the larger end's -5, so overall
        // xy is 20×20; volume of a frustum of squares 10→20 over h=10 is h/3*(A1+A2+sqrt(A1*A2)).
        let vol = mesh_signed_volume(&mesh).abs();
        let expected = 10.0 / 3.0 * (100.0 + 400.0 + (100.0f32 * 400.0).sqrt());
        assert!(
            (vol - expected).abs() < expected * 0.05,
            "frustum volume expected ~{expected}, got {vol}"
        );
    }

    /// #1243: distance taper of −5 on a 10×10 square collapses the end to a point (pyramid).
    #[test]
    fn taper_distance_negative_collapses_to_a_point() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 10.0);
        ext.taper = -5.0;
        ext.taper_mode = ExtrudeTaperMode::Distance;
        let mesh = extrusion_mesh(&doc, &ext).expect("pyramid mesh");
        let (min, max) = mesh.bounds().expect("bounds");
        // Base 10×10, tip at centre; overall xy still 10×10, height 10.
        assert!((max.x - min.x - 10.0).abs() < 0.5, "xy width {}", max.x - min.x);
        assert!((max.z - min.z - 10.0).abs() < 0.5, "height {}", max.z - min.z);
        let vol = mesh_signed_volume(&mesh).abs();
        let expected = 100.0 * 10.0 / 3.0; // pyramid
        assert!(
            (vol - expected).abs() < expected * 0.08,
            "pyramid volume expected ~{expected}, got {vol}"
        );
    }

    /// #1243: more negative than collapse still yields a point end (no invert), full height.
    #[test]
    fn taper_distance_past_collapse_stays_a_point() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 10.0);
        ext.taper = -20.0;
        ext.taper_mode = ExtrudeTaperMode::Distance;
        let mesh = extrusion_mesh(&doc, &ext).expect("still a solid");
        let vol = mesh_signed_volume(&mesh).abs();
        assert!(vol > 50.0 && vol < 400.0, "clamped pyramid-ish volume, got {vol}");
        assert!((effective_distance(&doc, &ext) - 10.0).abs() < 1e-3, "distance mode does not cut height");
    }

    /// #1243: circle taper adds to the radius, not the diameter.
    #[test]
    fn taper_distance_on_circle_adds_to_radius() {
        let (mut doc, sketch) = sketch_doc();
        let ck = doc
            .circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut ext = extrusion(sketch, vec![ExtrudeFace::Circle(ck)], 10.0);
        ext.taper = 5.0;
        ext.taper_mode = ExtrudeTaperMode::Distance;
        let mesh = extrusion_mesh(&doc, &ext).expect("cone frustum");
        let (min, max) = mesh.bounds().expect("bounds");
        // Bottom r=10, top r=15 → overall diameter 30.
        let xy = (max.x - min.x).max(max.y - min.y);
        assert!(
            (xy - 30.0).abs() < 1.0,
            "top diameter should be 30 (r=15), got span {xy}"
        );
    }

    /// #1243: −45° angle taper on a 10×10×10 extrude collapses at height 5.
    #[test]
    fn taper_angle_negative_45_collapses_at_half_height() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 10.0);
        ext.taper = -45.0;
        ext.taper_mode = ExtrudeTaperMode::Angle;
        let mesh = extrusion_mesh(&doc, &ext).expect("angle pyramid");
        let (min, max) = mesh.bounds().expect("bounds");
        assert!(
            (max.z - min.z - 5.0).abs() < 0.6,
            "height cut to 5, got {}",
            max.z - min.z
        );
        let eff = effective_distance(&doc, &ext);
        assert!((eff.abs() - 5.0).abs() < 0.6, "effective distance ~5, got {eff}");
        let vol = mesh_signed_volume(&mesh).abs();
        let expected = 100.0 * 5.0 / 3.0;
        assert!(
            (vol - expected).abs() < expected * 0.1,
            "pyramid vol ~{expected}, got {vol}"
        );
    }

    /// #1243: 0° angle taper is a plain prism.
    #[test]
    fn taper_angle_zero_is_prism() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 10.0);
        ext.taper = 0.0;
        ext.taper_mode = ExtrudeTaperMode::Angle;
        let mesh = extrusion_mesh(&doc, &ext).expect("prism");
        let (min, max) = mesh.bounds().expect("bounds");
        assert!((max.x - min.x - 10.0).abs() < 0.2);
        assert!((max.z - min.z - 10.0).abs() < 0.2);
    }

    /// #1243: +45° angle taper on height 10 grows each side by 10 (tan45=1).
    #[test]
    fn taper_angle_positive_45_flares_out() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 10.0);
        ext.taper = 45.0;
        ext.taper_mode = ExtrudeTaperMode::Angle;
        let mesh = extrusion_mesh(&doc, &ext).expect("flare");
        let (min, max) = mesh.bounds().expect("bounds");
        assert!(
            (max.x - min.x - 30.0).abs() < 1.0,
            "10 + 2×10 = 30 end span, got {}",
            max.x - min.x
        );
        assert!((max.z - min.z - 10.0).abs() < 0.5);
    }

    /// #1352: angle tapers outside (−90°, 89°] clamp to the bound and warn.
    #[test]
    fn clamp_extrude_taper_angle_bounds() {
        let over = clamp_extrude_taper(180.0, ExtrudeTaperMode::Angle, 20.0);
        assert!(
            (over.value - TAPER_ANGLE_MAX_DEG).abs() < 1e-4,
            "180° should become 89°, got {}",
            over.value
        );
        assert!(over.warning.is_some(), "over-max angle should warn");

        let under = clamp_extrude_taper(-180.0, ExtrudeTaperMode::Angle, 20.0);
        assert!(
            (under.value - TAPER_ANGLE_MIN_DEG).abs() < 1e-4,
            "−180° should become −90°, got {}",
            under.value
        );
        assert!(under.warning.is_some(), "under-min angle should warn");

        let ok = clamp_extrude_taper(45.0, ExtrudeTaperMode::Angle, 20.0);
        assert!((ok.value - 45.0).abs() < 1e-4);
        assert!(ok.warning.is_none(), "in-range angle should not warn");

        let at_max = clamp_extrude_taper(89.0, ExtrudeTaperMode::Angle, 20.0);
        assert!((at_max.value - 89.0).abs() < 1e-4);
        assert!(
            at_max.warning.is_none(),
            "89° on a short extrude is allowed, got {:?}",
            at_max.warning
        );
    }

    /// #1352: a long extrude at 89° still flares too far; clamp the offset and warn.
    #[test]
    fn clamp_extrude_taper_caps_huge_offset() {
        // 1000 mm high at 89° → offset ≈ 57 m. Cap at TAPER_MAX_OFFSET_MM.
        let r = clamp_extrude_taper(89.0, ExtrudeTaperMode::Angle, 1000.0);
        let offset = 1000.0 * r.value.to_radians().tan();
        assert!(
            offset <= TAPER_MAX_OFFSET_MM + 1.0,
            "offset should be capped at {} mm, got {offset} (angle {})",
            TAPER_MAX_OFFSET_MM,
            r.value
        );
        assert!(r.value < 89.0, "angle should drop below 89°, got {}", r.value);
        assert!(r.warning.is_some(), "huge flare should warn");

        let dist = clamp_extrude_taper(1_000_000.0, ExtrudeTaperMode::Distance, 20.0);
        assert!(
            (dist.value - TAPER_MAX_OFFSET_MM).abs() < 1e-3,
            "distance taper should cap at {} mm, got {}",
            TAPER_MAX_OFFSET_MM,
            dist.value
        );
        assert!(dist.warning.is_some());
    }

    /// #504: a symmetric extrude of total height `d` spans `[-d/2, +d/2]` along the normal.
    #[test]
    fn symmetric_extrusion_spans_both_sides_of_sketch_plane() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 20.0);
        ext.symmetric = true;
        let (start, end) = extrusion_end_offsets(&doc, &ext, 20.0);
        assert!((start - (-10.0)).abs() < 1e-4, "start={start}");
        assert!((end - 10.0).abs() < 1e-4, "end={end}");
        let mesh = extrusion_mesh(&doc, &ext).expect("symmetric mesh");
        let (min, max) = mesh.bounds().expect("bounds");
        assert!(
            (min.z + 10.0).abs() < 0.5 && (max.z - 10.0).abs() < 0.5,
            "solid should span z≈[-10,10], min={min:?} max={max:?}"
        );
        let base_pt = extruded_base_point(&doc, &ext, glam::Vec3::Z, glam::Vec3::ZERO, 20.0);
        let top_pt = extruded_free_end_point(&doc, &ext, glam::Vec3::Z, glam::Vec3::ZERO, 20.0);
        assert!((base_pt.z + 10.0).abs() < 1e-4, "base z={}", base_pt.z);
        assert!((top_pt.z - 10.0).abs() < 1e-4, "top z={}", top_pt.z);
    }

    /// #1268: symmetric + distance taper keeps the sketch plane at the original profile size.
    /// Both free ends shrink/grow by the taper; the solid is two frustums joined at mid,
    /// not a single end-to-end taper (or a prism of the tapered size).
    #[test]
    fn taper_symmetric_distance_keeps_midplane_at_sketch_size() {
        let (mut doc, sketch) = sketch_doc();
        // 10×10 square, total height 20 (±10), taper −2 per side → ends 6×6, mid 10×10.
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 20.0);
        ext.symmetric = true;
        ext.taper = -2.0;
        ext.taper_mode = ExtrudeTaperMode::Distance;
        let mesh = extrusion_mesh(&doc, &ext).expect("symmetric taper mesh");
        let (min, max) = mesh.bounds().expect("bounds");
        assert!(
            (min.z + 10.0).abs() < 0.5 && (max.z - 10.0).abs() < 0.5,
            "height still ±10, min={min:?} max={max:?}"
        );
        // Overall xy is the larger of mid (10) and ends (6) → 10×10.
        assert!(
            (max.x - min.x - 10.0).abs() < 0.5 && (max.y - min.y - 10.0).abs() < 0.5,
            "overall xy should be the mid-plane 10×10, got {}×{}",
            max.x - min.x,
            max.y - min.y
        );
        // Two frustums 10×10 → 6×6 over h=10 each.
        // V_one = h/3*(A1+A2+√(A1*A2)) = 10/3*(100+36+60) = 1960/3 ≈ 653.333
        let vol = mesh_signed_volume(&mesh).abs();
        let expected = 2.0 * (10.0 / 3.0) * (100.0 + 36.0 + (100.0f32 * 36.0).sqrt());
        assert!(
            (vol - expected).abs() < expected * 0.08,
            "double-frustum volume expected ~{expected}, got {vol} \
             (a tapered-size prism would be ~720; a single end-to-end frustum differs too)"
        );
        // Mid-plane cross-section of the solid should hit the sketch extent (~10).
        let mid_span = mesh_xy_span_near_z(&mesh, 0.0, 0.3);
        assert!(
            (mid_span - 10.0).abs() < 0.8,
            "mid-plane solid span should be ~10 (sketch size), got {mid_span}"
        );
        // Ends should be the tapered 6×6.
        let top_span = mesh_xy_span_near_z(&mesh, 10.0, 0.3);
        let bot_span = mesh_xy_span_near_z(&mesh, -10.0, 0.3);
        assert!(
            (top_span - 6.0).abs() < 0.8,
            "top end span should be ~6, got {top_span}"
        );
        assert!(
            (bot_span - 6.0).abs() < 0.8,
            "bottom end span should be ~6, got {bot_span}"
        );
    }

    /// #1268: symmetric + positive distance taper flares both ends; mid stays sketch size.
    #[test]
    fn taper_symmetric_distance_positive_flares_both_ends() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile], 20.0);
        ext.symmetric = true;
        ext.taper = 5.0;
        ext.taper_mode = ExtrudeTaperMode::Distance;
        let mesh = extrusion_mesh(&doc, &ext).expect("flare mesh");
        let (min, max) = mesh.bounds().expect("bounds");
        // Ends 20×20, mid 10×10 → overall xy 20.
        assert!(
            (max.x - min.x - 20.0).abs() < 0.5,
            "overall xy should be end span 20, got {}",
            max.x - min.x
        );
        let mid_span = mesh_xy_span_near_z(&mesh, 0.0, 0.3);
        assert!(
            (mid_span - 10.0).abs() < 0.8,
            "mid-plane should stay sketch size 10, got {mid_span}"
        );
        let vol = mesh_signed_volume(&mesh).abs();
        // Two frustums 10×10 → 20×20 over h=10: h/3*(100+400+200)= 10/3*700 ≈ 2333.33 each → 4666.67
        let expected = 2.0 * (10.0 / 3.0) * (100.0 + 400.0 + (100.0f32 * 400.0).sqrt());
        assert!(
            (vol - expected).abs() < expected * 0.08,
            "double-frustum volume expected ~{expected}, got {vol}"
        );
    }

    /// Max XY span of the mesh at plane z = z0: vertices within `tol`, plus intersections of
    /// triangle edges that straddle the plane (OCCT fuse often drops coplanar mid faces, so the
    /// waist may only appear as side-wall edge crossings).
    fn mesh_xy_span_near_z(mesh: &SolidMesh, z0: f32, tol: f32) -> f32 {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        let mut any = false;
        let mut consider = |p: glam::Vec3| {
            any = true;
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        };
        for tri in &mesh.triangles {
            for p in tri {
                if (p.z - z0).abs() <= tol {
                    consider(*p);
                }
            }
            // Edge–plane hits for edges that straddle z0.
            for i in 0..3 {
                let a = tri[i];
                let b = tri[(i + 1) % 3];
                let da = a.z - z0;
                let db = b.z - z0;
                if da * db < 0.0 {
                    let t = da / (da - db);
                    consider(a + (b - a) * t);
                }
            }
        }
        if !any {
            return 0.0;
        }
        (max_x - min_x).max(max_y - min_y)
    }

    /// #504/#548: the extrude-to-face distance to a **symmetric** extrusion's cap reaches its
    /// real position — half the height to either side of the sketch plane — not the full height.
    /// (Extruding a rectangle up to a symmetric cylinder's face used to overshoot by d/2.)
    #[test]
    fn extrude_target_to_a_symmetric_cap_is_half_the_height() {
        use crate::model::{ExtrudeTarget, FaceId};
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let mut ext = extrusion(sketch, vec![profile.clone()], 20.0);
        ext.symmetric = true;
        let ei = doc.extrusions.insert(ext);
        let base = glam::Vec3::ZERO;
        let normal = glam::Vec3::Z;
        // The sketch sits on z = 0; the symmetric caps are at +10 and -10, not +20 and 0.
        let top = FaceId::ExtrudeCap { extrusion: ei, profile: profile.clone(), top: true };
        let d_top = target_distance(&doc, base, normal, &ExtrudeTarget::BodyFace(top)).unwrap();
        assert!((d_top - 10.0).abs() < 1e-3, "top cap at +d/2, got {d_top}");
        let bot = FaceId::ExtrudeCap { extrusion: ei, profile, top: false };
        let d_bot = target_distance(&doc, base, normal, &ExtrudeTarget::BodyFace(bot)).unwrap();
        assert!((d_bot + 10.0).abs() < 1e-3, "base cap at -d/2, got {d_bot}");
    }

    /// #200: a cut tool built with overshoot extends past both ends by `2 * overshoot`, so
    /// its caps clear any body face they would otherwise sit exactly on (which leaves a
    /// coincident seam face — a wall that renders capped even though the material is gone).
    #[test]
    fn cut_tool_overshoots_past_both_ends() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let ext = extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 20.0);
        let flush = occt_extrusion_shape(&doc, &ext, 20.0).unwrap().volume().unwrap();
        let overshot = occt_extrusion_shape_overshoot(&doc, &ext, 20.0, 0.05)
            .unwrap()
            .volume()
            .unwrap();
        // Extra volume = the cylinder cross-section times the 2 * 0.05 mm of added length.
        let expected_extra = std::f64::consts::PI * 25.0 * 0.10;
        assert!(
            (overshot - flush - expected_extra).abs() < 1.0,
            "flush={flush} overshot={overshot} expected_extra={expected_extra}"
        );
    }

    #[test]
    fn face_boundary_loop_world_none_for_construction_plane() {
        let doc = Document::default();
        assert!(face_boundary_loop_world(&doc, &FaceId::ConstructionPlane(pkey(0))).is_none());
    }

    #[test]
    fn closed_line_loop_extrudes_to_a_prism_mesh() {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, LineEnd};

        let (mut doc, sketch) = sketch_doc();
        doc.lines.insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines.insert(Line::from_local_endpoints(sketch, 10.0, 0.0, 5.0, 8.0));
        doc.lines.insert(Line::from_local_endpoints(sketch, 5.0, 8.0, 0.0, 0.0));
        let coincident = |a, b| Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(a),
                b: ConstraintEntity::Point(b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        };
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        doc.constraints.insert(coincident(point(lkey(0), LineEnd::End), point(lkey(1), LineEnd::Start)));
        doc.constraints.insert(coincident(point(lkey(1), LineEnd::End), point(lkey(2), LineEnd::Start)));
        doc.constraints.insert(coincident(point(lkey(2), LineEnd::End), point(lkey(0), LineEnd::Start)));

        let loops = crate::polygon::closed_line_loops(&doc, sketch);
        assert_eq!(loops.len(), 1);
        let ext = extrusion(sketch, vec![ExtrudeFace::Polygon(loops[0].clone())], 6.0);
        let mesh = extrusion_mesh(&doc, &ext).unwrap();
        // A triangular prism: 1 (bottom fan) + 1 (top fan) + 3 sides * 2 = 8 triangles.
        assert_eq!(mesh.triangles.len(), 8);
        let (min, max) = mesh.bounds().unwrap();
        assert!((min.z).abs() < 1e-4 && (max.z - 6.0).abs() < 1e-4, "z [{},{}]", min.z, max.z);
    }

    /// The `docs-site/screenshots/letter-b.lua` geometry: the outer silhouette of a blocky
    /// capital "B" (a straight left spine, two right-side bumps with a waist notch between
    /// them) traced with the Line tool, closed into a loop with `Coincident` constraints, and
    /// extruded 12 mm. This locks the docs example against regressions with robust *geometric*
    /// invariants — volume ≈ (2D outline area) × depth, bounding box, and a sane triangle
    /// count — rather than a brittle golden mesh. Kernel-agnostic on purpose (plain polygon
    /// extrusion needs no OCCT), so it must pass with and without the `occt` feature.
    const LETTER_B_DEPTH: f32 = 12.0;

    /// Build the letter-B outer silhouette from `docs-site/screenshots/letter-b.lua`: a
    /// straight left spine and two rounded lobes formed by **bezier curves** (#54), with a
    /// waist notch, traced with the Line tool (letter coords: x = width, y = height) and
    /// closed into one loop with `Coincident` constraints. Returns the closed loop's ordered
    /// line indices. Shape must match `segs[]` in the script (the script also rotates it into
    /// the sketch's (u, v) for the top view, which doesn't change area/volume).
    fn push_letter_b_outline(
        doc: &mut Document,
        sketch: crate::model::SketchId,
    ) -> Vec<crate::model::LineKey> {
        use crate::model::{
            Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, Line, LineEnd,
        };
        // (start, end, optional bezier handles [near start, near end]) in letter coords. The
        // two lobe curves meet at the single waist point (18, 36).
        let segs: [((f32, f32), (f32, f32), Option<[(f32, f32); 2]>); 4] = [
            ((0.0, 0.0), (0.0, 72.0), None),
            ((0.0, 72.0), (18.0, 36.0), Some([(54.0, 72.0), (50.0, 42.0)])),
            ((18.0, 36.0), (14.0, 0.0), Some([(50.0, 30.0), (58.0, -2.0)])),
            ((14.0, 0.0), (0.0, 0.0), None),
        ];
        let n = segs.len();
        let mut keys = Vec::new();
        for (a, b, bez) in segs {
            let mut line = Line::from_local_endpoints(sketch, a.0, a.1, b.0, b.1);
            line.bezier = bez;
            keys.push(doc.lines.insert(line));
        }
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        for i in 0..n {
            doc.constraints.insert(Constraint {
                sketch,
                kind: ConstraintKind::Coincident {
                    a: ConstraintEntity::Point(point(keys[i], LineEnd::End)),
                    b: ConstraintEntity::Point(point(keys[(i + 1) % n], LineEnd::Start)),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
            });
        }
        let loops = crate::polygon::closed_line_loops(doc, sketch);
        assert_eq!(loops.len(), 1, "the B outline should be a single closed loop");
        assert_eq!(loops[0].len(), n, "loop should use all {n} segments");
        loops[0].clone()
    }

    /// Build a "D"-shaped counter profile (matching `draw_d_counter` in the letter-B script):
    /// a flat left edge at x=`lx` spanning `cy` ± `hh`, plus a rounded right edge (two
    /// cubic-bezier quarter-arcs, kappa control offset) bulging to x=`lx + w`, closed into one
    /// loop. Returns its `ExtrudeFace::Polygon`. Its area is a half-ellipse: π·w·hh / 2.
    fn push_d_profile(
        doc: &mut Document,
        sketch: crate::model::SketchId,
        lx: f32,
        cy: f32,
        w: f32,
        hh: f32,
    ) -> ExtrudeFace {
        use crate::model::{
            Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, Line, LineEnd,
        };
        const K: f32 = 0.552_284_75;
        let (ty, by, rx) = (cy + hh, cy - hh, lx + w);
        let (kx, ky) = (K * w, K * hh);
        // (start, end, optional [ctrl-near-start, ctrl-near-end]) in letter coords.
        let parts: [((f32, f32), (f32, f32), Option<[(f32, f32); 2]>); 3] = [
            ((lx, by), (lx, ty), None),                                 // flat left edge
            ((lx, ty), (rx, cy), Some([(lx + kx, ty), (rx, cy + ky)])), // top-right arc
            ((rx, cy), (lx, by), Some([(rx, cy - ky), (lx + kx, by)])), // bottom-right arc
        ];
        let mut keys = Vec::new();
        for (p0, p1, bez) in parts {
            let mut line = Line::from_local_endpoints(sketch, p0.0, p0.1, p1.0, p1.1);
            line.bezier = bez;
            keys.push(doc.lines.insert(line));
        }
        let base = keys[0];
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        for k in 0..3 {
            doc.constraints.insert(Constraint {
                sketch,
                kind: ConstraintKind::Coincident {
                    a: ConstraintEntity::Point(point(keys[k], LineEnd::End)),
                    b: ConstraintEntity::Point(point(keys[(k + 1) % 3], LineEnd::Start)),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
            });
        }
        let loop_ = crate::polygon::closed_line_loops(doc, sketch)
            .into_iter()
            .find(|l| l.contains(&base))
            .expect("D counter forms a closed loop");
        ExtrudeFace::Polygon(loop_)
    }

    /// The letter-B outline extrudes to a valid solid. Kernel-agnostic (plain polygon
    /// extrusion needs no OCCT), so it must pass with and without the `occt` feature. The
    /// bezier lobes make an exact area fiddly, so this locks the docs example with robust
    /// invariants — one closed loop, a non-empty watertight solid, and a bounded volume /
    /// bounding box — rather than a brittle golden number.
    #[test]
    fn letter_b_outline_extrudes_to_the_expected_solid() {
        let (mut doc, sketch) = sketch_doc();
        let loop_ = push_letter_b_outline(&mut doc, sketch);

        let ext = extrusion(sketch, vec![ExtrudeFace::Polygon(loop_)], LETTER_B_DEPTH);
        let mesh = extrusion_mesh(&doc, &ext).expect("B extrudes to a solid mesh");
        assert!(!mesh.is_empty(), "extruded B mesh must be non-empty");

        let volume = mesh_signed_volume(&mesh).abs();
        assert!(volume.is_finite() && volume > 0.0, "B volume {volume}");

        // The bezier lobes bulge beyond the straight chords, so the true area exceeds the
        // chord polygon's (900 mm^2) yet stays within the bounding box — bound the volume
        // between those, times depth.
        let (min, max) = mesh.bounds().unwrap();
        let bbox_area = (max.x - min.x) * (max.y - min.y);
        assert!(volume > 900.0 * LETTER_B_DEPTH, "B volume {volume} below chord lower bound");
        assert!(volume < bbox_area * LETTER_B_DEPTH, "B volume {volume} exceeds bbox");

        // Full letter height (~72, with a little bezier overshoot) extruded DEPTH into z; the
        // lobes give a sane width. z is an exact straight prism.
        assert!((70.0..=76.0).contains(&(max.y - min.y)), "y span {}", max.y - min.y);
        assert!((40.0..=60.0).contains(&(max.x - min.x)), "x span {}", max.x - min.x);
        assert!(
            min.z.abs() < 1e-3 && (max.z - LETTER_B_DEPTH).abs() < 1e-3,
            "z span [{}, {}]",
            min.z,
            max.z
        );
        assert!(mesh.triangles.len() >= 10, "triangle count {}", mesh.triangles.len());
    }

    /// The `docs-site/screenshots/letter-b.lua` full geometry: the outer "B" silhouette
    /// extruded to a solid, then the two counter holes (upper + lower bowls) punched clean
    /// through it as **cut** extrusions (`body = "cut"` / `BodySource::Solid { add, cut }`,
    /// #35). Needs the kernel to perform the boolean subtraction, so it's `occt`-only. The
    /// expected volume is self-checked: (outer_area − upper_hole − lower_hole) × depth, every
    /// area computed by shoelace from the same coordinates the script draws.
    /// The full letter-B: the curved outer silhouette extruded to a solid, then the two
    /// counter holes punched clean through as **cut** extrusions (`BodySource::Solid { add,
    /// cut }`, #35). Needs the kernel for the boolean subtraction, so it's `occt`-only. The
    /// curved outer area is fiddly to compute exactly, so this isolates the holes: compare the
    /// no-cut solid to the cut solid and assert the removed volume equals the two D counters'
    /// area (π·w·hh / 2 each — a half-ellipse) × depth (the curved outer area cancels).
    #[test]
    fn occt_letter_b_with_two_counters_cuts_to_the_expected_volume() {
        // D counters (letter coords): flat-left x = lx, center y = cy, width w, half-height hh
        // — must match upper_d/lower_d in docs-site/screenshots/letter-b.lua.
        const UPPER: (f32, f32, f32, f32) = (10.0, 54.0, 24.0, 9.0);
        const LOWER: (f32, f32, f32, f32) = (10.0, 16.0, 26.0, 9.0);

        let (mut doc, sketch) = sketch_doc();
        let outer = ExtrudeFace::Polygon(push_letter_b_outline(&mut doc, sketch));

        // No-cut solid volume (curved outer × depth) — the isolation baseline.
        let outer_only = extrusion(sketch, vec![outer.clone()], LETTER_B_DEPTH);
        let outer_vol =
            mesh_signed_volume(&extrusion_mesh(&doc, &outer_only).expect("outer B mesh")).abs();

        // Two D counter profiles cut through the full thickness.
        let upper = push_d_profile(&mut doc, sketch, UPPER.0, UPPER.1, UPPER.2, UPPER.3);
        let lower = push_d_profile(&mut doc, sketch, LOWER.0, LOWER.1, LOWER.2, LOWER.3);
        doc.extrusions.insert(extrusion(sketch, vec![outer], LETTER_B_DEPTH)); // 0: the B
        doc.extrusions.insert(extrusion(sketch, vec![upper], LETTER_B_DEPTH)); // 1: upper cut
        doc.extrusions.insert(extrusion(sketch, vec![lower], LETTER_B_DEPTH)); // 2: lower cut
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Solid { base: None, add: vec![xkey(0)],
                cut: vec![xkey(1), xkey(2)],
            },
            material: None,
            name: Some("B".to_string()),
            shadow: false,
        });
        let cut_vol = mesh_signed_volume(&body_solid_mesh(&doc, bkey(0)).expect("occt B mesh")).abs();

        // Each D removes ≈ (π·w·hh / 2) × depth (a half-ellipse, tessellated), fully enclosed
        // in its bowl => a clean through-hole, independent of the curved outer area.
        use std::f32::consts::PI;
        let removed = outer_vol - cut_vol;
        let expected_removed =
            (PI * UPPER.2 * UPPER.3 / 2.0 + PI * LOWER.2 * LOWER.3 / 2.0) * LETTER_B_DEPTH;
        assert!(
            (removed - expected_removed).abs() < expected_removed * 0.03,
            "D counters removed {removed} mm^3, expected ~{expected_removed} (π·w·hh/2 × depth)"
        );
        assert!(cut_vol > 0.0 && cut_vol < outer_vol, "cut {cut_vol}, outer {outer_vol}");
    }

    #[test]
    fn occt_cut_body_subtracts_overlapping_extrusion_volume() {
        // A 10x10x5 box (500 mm^3) with a 4x4 column cut clean through it removes 4*4*5 = 80,
        // leaving ~420. The result is meshed via the kernel's Cut boolean (#35); its
        // divergence-theorem volume should match.
        let doc = cut_body_doc();
        let mesh = body_solid_mesh(&doc, bkey(0)).expect("occt cut-body mesh");
        let volume = mesh_signed_volume(&mesh).abs();
        assert!(
            (volume - 420.0).abs() < 5.0,
            "cut-body volume {volume}, expected ~420"
        );
    }

    #[test]
    fn circle_extrudes_to_a_cylinder_mesh() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles
            .insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let ext = extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 8.0);
        let mesh = extrusion_mesh(&doc, &ext).unwrap();
        // The kernel tessellates a *true* cylinder (triangle count varies with the
        // mesher); the hand-rolled fallback path is covered by its own tests.
        assert!(!mesh.triangles.is_empty());
        let (min, max) = mesh.bounds().unwrap();
        assert!((max.z - 8.0).abs() < 1e-4 && min.z.abs() < 1e-4);
        // Radius 5 → diameter 10 in x and y.
        assert!((max.x - min.x - 10.0).abs() < 0.1 && (max.y - min.y - 10.0).abs() < 0.1);
    }

    #[test]
    fn circle_extruded_to_a_slanted_plane_is_a_closed_solid() {
        // #582: a circle extruded up to a *diagonal* target plane takes the loft path (its top ring
        // is slanted, not a pure translation). The result must still be a watertight, capped solid,
        // not an open tube ("pipe").
        use crate::construction::{definition_from_reference, plane_from_definition, PlaneReference};
        use crate::model::ConstructionPlaneParent;
        let (mut doc, sketch) = sketch_doc();
        doc.construction_planes.insert(plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Axis {
                    origin: Vec3::new(0.0, 0.0, 40.0),
                    direction: Vec3::X,
                    label: "X".to_string(),
                },
                0.0,
                45.0,
            ),
            ConstructionPlaneParent::Root,
        ));
        let target_plane = doc.construction_planes.keys().last().unwrap();
        doc.circles
            .insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let mut ext = extrusion(sketch, vec![ExtrudeFace::Circle(rkey(0))], 40.0);
        ext.target = Some(crate::model::ExtrudeTarget::Plane(target_plane));
        let mesh = extrusion_mesh(&doc, &ext).expect("mesh built");
        assert_watertight(&mesh);
    }

    // --- 3D edge chamfer/fillet (#77) ---------------------------------------------------

    /// Every edge of a closed mesh should be shared by exactly two triangles (a manifold,
    /// watertight solid) — the strongest generic check available for a hand-derived mesh-bevel
    /// algorithm without visualizing it. Coordinates are snapped to a millimetre/1000 grid so
    /// two triangles' shared edge compares equal despite unrelated floating-point paths.
    fn assert_watertight(mesh: &SolidMesh) {
        use std::collections::HashMap;
        let key = |p: Vec3| {
            (
                (p.x * 1000.0).round() as i64,
                (p.y * 1000.0).round() as i64,
                (p.z * 1000.0).round() as i64,
            )
        };
        let mut edge_count: HashMap<((i64, i64, i64), (i64, i64, i64)), u32> = HashMap::new();
        for tri in &mesh.triangles {
            for i in 0..3 {
                let a = key(tri[i]);
                let b = key(tri[(i + 1) % 3]);
                assert_ne!(a, b, "degenerate zero-length edge in {tri:?}");
                let e = if a <= b { (a, b) } else { (b, a) };
                *edge_count.entry(e).or_insert(0) += 1;
            }
        }
        for (e, c) in &edge_count {
            assert_eq!(*c, 2, "edge {e:?} used by {c} triangle(s), expected exactly 2 (not watertight)");
        }
    }

    #[test]
    fn corner_bevel_3d_matches_2d_math_when_embedded_flat() {
        // v=(0,0,0), a=(10,0,0), b=(0,10,0): a right-angle corner in the XY plane, chamfer 3 —
        // should match `vertex_treatment_geometry`'s (v=(0,0), a=(10,0), b=(0,10)) exactly.
        let bevel = corner_bevel_3d(
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
            VertexTreatmentKind::Chamfer,
            3.0,
        )
        .unwrap();
        assert!((bevel.p1 - Vec3::new(3.0, 0.0, 0.0)).length() < 1e-4, "{:?}", bevel.p1);
        assert!((bevel.p2 - Vec3::new(0.0, 3.0, 0.0)).length() < 1e-4, "{:?}", bevel.p2);
        assert!(bevel.arc.is_none());
    }

    #[test]
    fn corner_bevel_3d_fillet_has_arc_and_is_none_when_degenerate() {
        let bevel = corner_bevel_3d(
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
            VertexTreatmentKind::Fillet,
            2.0,
        )
        .unwrap();
        assert!(bevel.arc.is_some());
        let samples = sample_corner_bevel(&bevel, VertexTreatmentKind::Fillet);
        assert_eq!(samples.len(), EDGE_TREATMENT_FILLET_SEGMENTS + 1);
        assert!((samples[0] - bevel.p1).length() < 1e-4);
        assert!((*samples.last().unwrap() - bevel.p2).length() < 1e-4);

        // Collinear v/a/b: no real corner.
        assert!(corner_bevel_3d(
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(-5.0, 0.0, 0.0),
            VertexTreatmentKind::Chamfer,
            1.0,
        )
        .is_none());
    }

    // The next several tests assert mesh-bevel-specific triangle counts and removed
    // volumes — they exercise `extrusion_mesh_tessellated` (the hand-rolled fallback
    // and live-preview mesher) directly, since the kernel path builds true BREP
    // fillets/chamfers (#77) with a different tessellation and (true-arc vs
    // faceted-bezier) removed volume; the OCCT path has its own tests below.
    #[test]
    fn vertical_edge_chamfer_is_watertight_and_adds_expected_triangles() {
        let (doc, _sketch, mut ext) = box_doc();
        // Vertical edge index 0 sits at profile vertex 1 (see `ExtrusionEdgeRef::Vertical`).
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        let mesh = extrusion_mesh_tessellated(&doc, &ext, ext.distance).unwrap();
        assert_watertight(&mesh);
        // Untreated box: 12 triangles. One chamfered vertical corner: caps grow from a
        // quadrilateral (2 tri) to a pentagon (3 tri) each = +2, plus a 2-triangle bevel wall.
        assert_eq!(mesh.triangles.len(), 12 + 2 + 2);
        // The treated corner is cut back, so nothing should reach the original sharp corner
        // at local (10, 0) (profile vertex 1) anymore.
        let cut_corner = Vec3::new(10.0, 0.0, 0.0);
        assert!(mesh.triangles.iter().flatten().all(|p| (*p - cut_corner).length() > 1e-3));
    }

    #[test]
    fn vertical_edge_fillet_is_watertight_and_adds_expected_triangles() {
        let (doc, _sketch, mut ext) = box_doc();
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Fillet,
            amount: 2.0,
        });
        let mesh = extrusion_mesh_tessellated(&doc, &ext, ext.distance).unwrap();
        assert_watertight(&mesh);
        let m = EDGE_TREATMENT_FILLET_SEGMENTS; // arc has m+1 points, m segments
        let cap_points = 3 + (m + 1); // 3 untouched corners + the filleted corner's run
        let cap_tris_each = cap_points - 2;
        let expected = cap_tris_each * 2 // bottom + top caps
            + 4 * 2 // the 4 original-edge main walls (unchanged count)
            + m * 2; // the fillet's own faceted bevel wall
        assert_eq!(mesh.triangles.len(), expected);
    }

    #[test]
    fn cap_edge_chamfer_is_watertight_and_removes_expected_volume() {
        let (doc, _sketch, mut ext) = box_doc();
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        let mesh = extrusion_mesh_tessellated(&doc, &ext, ext.distance).unwrap();
        assert_watertight(&mesh);
        // Cap stays a quad (just repositioned, +0); the two neighboring walls each gain one
        // extra triangle from their notch (4 points -> 3 triangles instead of 2, +1 each);
        // plus the bevel's own quad (2 tri). The two corner points cut away entirely (see
        // `apply_cap_edge_treatment`'s doc comment) don't add cap points back.
        assert_eq!(mesh.triangles.len(), 12 + 1 + 1 + 2);
        // Nothing should touch the original sharp bottom-front edge (z = 0, y = 0) anymore.
        assert!(mesh
            .triangles
            .iter()
            .flatten()
            .all(|p| !(p.y.abs() < 1e-3 && p.z.abs() < 1e-3)));
        // A 10x10x5 box (volume 500) with a 2mm chamfer shaved off one 10mm-long bottom edge
        // removes a triangular-prism sliver of volume 0.5 * 2 * 2 * 10 = 20.
        let volume = mesh_signed_volume(&mesh);
        assert!((volume - 480.0).abs() < 1.0, "volume {volume}");
    }

    #[test]
    fn cap_edge_fillet_on_top_is_watertight_and_removes_expected_volume() {
        let (doc, _sketch, mut ext) = box_doc();
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 2, top: true },
            kind: VertexTreatmentKind::Fillet,
            amount: 1.5,
        });
        let mesh = extrusion_mesh_tessellated(&doc, &ext, ext.distance).unwrap();
        assert_watertight(&mesh);
        // A quarter-circle-ish fillet of radius 1.5 shaves roughly (1 - pi/4) * r^2 * length
        // off the box (500) along the 10mm top edge.
        let removed = (1.0 - std::f32::consts::FRAC_PI_4) * 1.5 * 1.5 * 10.0;
        let volume = mesh_signed_volume(&mesh);
        assert!((volume - (500.0 - removed)).abs() < 0.5, "volume {volume}, removed ~{removed}");
    }

    #[test]
    fn multiple_non_conflicting_treatments_combine_and_stay_watertight() {
        let (doc, _sketch, mut ext) = box_doc();
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        // Edge 2 (opposite side) doesn't touch vertex 1, so it's independent.
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 2, top: false },
            kind: VertexTreatmentKind::Fillet,
            amount: 1.0,
        });
        let mesh = extrusion_mesh_tessellated(&doc, &ext, ext.distance).unwrap();
        assert_watertight(&mesh);
        let volume = mesh_signed_volume(&mesh);
        assert!(volume > 400.0 && volume < 500.0, "volume {volume}");
    }

    // --- OCCT path (#77): true BREP fillets/chamfers replace the mesh-bevel above. ---
    // These don't hard-code triangle counts (OCCT tessellation differs); instead they
    // check the treated solid is watertight (its mesh's divergence-theorem volume
    // matches OCCT's own exact solid volume) and that a treatment removed a sane, small
    // amount of material. Roundness of a fillet can't be verified in a headless env.

    #[test]
    fn occt_vertical_edge_fillet_is_watertight_and_removes_material() {
        let (doc, _sketch, base) = box_doc();
        let dist = effective_distance(&doc, &base);
        let untreated = occt_extrusion_shape(&doc, &base, dist).unwrap().volume().unwrap();

        let mut ext = base;
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Fillet,
            amount: 2.0,
        });
        let solid_vol = occt_extrusion_shape(&doc, &ext, dist).unwrap().volume().unwrap();
        let mesh = extrusion_mesh(&doc, &ext).unwrap();
        let mesh_vol = mesh_signed_volume(&mesh).abs() as f64;
        assert!(mesh_vol.is_finite() && mesh_vol > 0.0, "mesh vol {mesh_vol}");
        // Watertight: the closed mesh's divergence-theorem volume matches the exact solid.
        assert!(
            (mesh_vol - solid_vol).abs() < solid_vol * 2e-2,
            "mesh vol {mesh_vol} vs solid vol {solid_vol}"
        );
        // A fillet removes only a small sliver of the 10x10x5 box.
        assert!(
            solid_vol < untreated && solid_vol > untreated * 0.9,
            "solid {solid_vol}, untreated {untreated}"
        );
    }

    #[test]
    fn occt_cap_edge_chamfer_is_watertight_and_removes_material() {
        let (doc, _sketch, base) = box_doc();
        let dist = effective_distance(&doc, &base);
        let untreated = occt_extrusion_shape(&doc, &base, dist).unwrap().volume().unwrap();

        let mut ext = base;
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        let solid_vol = occt_extrusion_shape(&doc, &ext, dist).unwrap().volume().unwrap();
        let mesh = extrusion_mesh(&doc, &ext).unwrap();
        let mesh_vol = mesh_signed_volume(&mesh).abs() as f64;
        assert!(mesh_vol.is_finite() && mesh_vol > 0.0, "mesh vol {mesh_vol}");
        assert!(
            (mesh_vol - solid_vol).abs() < solid_vol * 2e-2,
            "mesh vol {mesh_vol} vs solid vol {solid_vol}"
        );
        // A 2mm chamfer off one 10mm bottom edge removes a ~20mm^3 triangular prism.
        assert!(
            solid_vol < untreated && solid_vol > untreated * 0.9,
            "solid {solid_vol}, untreated {untreated}"
        );
    }

    #[test]
    fn nonpositive_amount_treatment_is_ignored() {
        let (doc, _sketch, mut ext) = box_doc();
        let untreated = extrusion_mesh(&doc, &ext).unwrap().triangles.len();
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Chamfer,
            amount: 0.0,
        });
        assert_eq!(extrusion_mesh(&doc, &ext).unwrap().triangles.len(), untreated);
    }

    /// #157/#165: a Select-mode body-edge selection resolves to the analytic treatable edge
    /// the chamfer/fillet tool needs — matched by quantized endpoints in either direction,
    /// and filtered down from a whole `SceneSelection`.
    #[test]
    fn selected_body_edges_resolve_to_treatable_edges() {
        use crate::hierarchy::{quantize_body_point, SceneElement};

        let (doc, _sketch, ext) = box_doc();
        let mut doc = doc;
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });

        let edges = treatable_edges(&doc);
        let (expect_ei, expect_edge, a, b) = edges[0].clone();
        let (qa, qb) = (quantize_body_point(a), quantize_body_point(b));

        // Forward and reversed endpoint order both resolve to the same analytic edge.
        assert_eq!(
            treatable_edge_for_selection(&doc, bkey(0), qa, qb),
            Some((expect_ei, expect_edge)),
        );
        assert_eq!(
            treatable_edge_for_selection(&doc, bkey(0), qb, qa),
            Some((expect_ei, expect_edge)),
        );
        // A different body index does not match.
        assert_eq!(treatable_edge_for_selection(&doc, bkey(7), qa, qb), None);
        // An edge that isn't in the analytic list resolves to None.
        assert_eq!(
            treatable_edge_for_selection(&doc, bkey(0), [123456, 0, 0], [123456, 100, 0]),
            None
        );

        // Selection filter: two selected edges (one duplicated via reversal) plus a
        // non-edge element yield exactly the resolved unique edges.
        let mut selection = crate::selection::SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::BodyEdge { body: bkey(0), a: qa, b: qb },
            true,
        );
        crate::selection::click_scene_selection(&mut selection, SceneElement::Body(bkey(0)), true);
        let resolved = treatable_edges_in_selection(&doc, &selection);
        assert_eq!(resolved, vec![(expect_ei, expect_edge)]);
    }

    /// #1077: Face Snap turns the moving face to meet the target face and lands its point on
    /// the target's. By default the two normals end up **opposed**, so the surfaces touch —
    /// which is what "put this face on that face" nearly always means; the flip points them
    /// the same way instead.
    #[test]
    fn face_snap_puts_a_face_on_a_face() {
        use crate::model::{MoveOperation, MoveTranslateMode};
        // Two boxes: A at the origin, B 40mm along +X. Both 10x10x5.
        let (mut doc, sketch) = sketch_doc();
        for (x, slot) in [(0.0f32, 0usize), (40.0, 1)] {
            let profile = rect_profile(&mut doc, sketch, x, 0.0, 10.0, 10.0);
            doc.extrusions.insert(extrusion(sketch, vec![profile], 5.0));
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Extrusion(xkey(slot)),
                material: None,
                name: None,
                shadow: false,
            });
        }
        // Pick a face of a body by its outward normal, as a point at its centre.
        let cap = |body: usize, want: Vec3| {
            let solid = body_solid_mesh(&doc, bkey(body)).expect("box mesh");
            let tris = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid)
                .into_iter()
                .find(|t| {
                    let n = (t[0][1] - t[0][0]).cross(t[0][2] - t[0][0]).normalize_or_zero();
                    n.dot(want) > 0.9
                })
                .expect("a face with that normal");
            let q = crate::hierarchy::quantize_body_point;
            crate::model::MovePointRef::OnFace {
                body: bkey(body),
                centroid: q(face_group_center(&tris)),
                normal: q((tris[0][1] - tris[0][0]).cross(tris[0][2] - tris[0][0]).normalize_or_zero()),
                uv: [0, 0],
            }
        };
        let op = |flip: bool, spin: &str| MoveOperation {
            keep_inputs: false,
            targets: vec![bkey(0)],
            translate_mode: MoveTranslateMode::FaceSnap,
            // A's top cap (+Z) onto B's left wall (-X), so the turn is a real quarter turn.
            start_point_a: Some(cap(0, Vec3::Z)),
            end_point_a: Some(cap(1, -Vec3::X)),
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            face_flip: flip,
            face_spin: spin.to_string(),
            rx: String::new(),
            ry: String::new(),
            rz: String::new(),
            outputs: Vec::new(),
            name: None,
            roll_angle: String::new(),
            face_offset: String::new(),
        };

        // A's top cap centre (5, 5, 5) lands on B's left wall centre (40, 5, 2.5).
        let target = Vec3::new(40.0, 5.0, 2.5);
        let m = move_op_transform(&doc, &op(false, "")).expect("a placement");
        let mate = m.transform_point3(Vec3::new(5.0, 5.0, 5.0));
        assert!((mate - target).length() < 1e-3, "{mate:?}");
        // The moving face's +Z normal now **opposes** the target's -X, so the surfaces touch.
        let n = m.transform_vector3(Vec3::Z).normalize_or_zero();
        assert!((n - Vec3::X).length() < 1e-3, "the surfaces touch: {n:?}");

        // Flipped, the normals point the same way instead and the part sits behind the face.
        let flipped = move_op_transform(&doc, &op(true, "")).expect("a placement");
        let n = flipped.transform_vector3(Vec3::Z).normalize_or_zero();
        assert!((n - -Vec3::X).length() < 1e-3, "flipped: {n:?}");

        // A gap (#1079) holds the part off along the target's normal — clearance, whichever
        // way round it ended up.
        let gapped = move_op_transform(&doc, &MoveOperation {
            face_offset: "3".to_string(),
            ..op(false, "")
        })
        .expect("a placement");
        let mate = gapped.transform_point3(Vec3::new(5.0, 5.0, 5.0));
        // The fixed face's normal is -X, so 3 mm of gap sits 3 mm along -X of the target.
        assert!((mate - (target - Vec3::X * 3.0)).length() < 1e-3, "{mate:?}");

        // The spin turns about the target's normal, through the mate point — which stays put.
        let spun = move_op_transform(&doc, &op(false, "90")).expect("a placement");
        let mate = spun.transform_point3(Vec3::new(5.0, 5.0, 5.0));
        assert!((mate - target).length() < 1e-3, "the mate point holds: {mate:?}");
        let plain = m.transform_point3(Vec3::ZERO);
        let turned = spun.transform_point3(Vec3::ZERO);
        assert!((plain - turned).length() > 1.0, "a 90° spin moves the far corner");
    }

    /// #1076: Free mode grows a rotation — X/Y/Z turns typed alongside the X/Y/Z amounts —
    /// and it acts about the moving part's **own centre**, so typing a turn spins the part
    /// where it stands rather than swinging it around the world origin. And In place is the
    /// identity by definition, whatever else is filled in.
    #[test]
    fn free_mode_turns_the_part_about_its_own_centre() {
        use crate::model::{MoveOperation, MoveTranslateMode};
        let (mut doc, _sketch, ext) = box_doc(); // 10x10 footprint at the origin, 5 tall
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let op = |mode: MoveTranslateMode, rz: &str, tx: &str| MoveOperation {
            keep_inputs: false,
            targets: vec![bkey(0)],
            translate_mode: mode,
            start_point_a: None,
            end_point_a: None,
            start_point_b: None,
            end_point_b: None,
            start_point_c: None,
            end_point_c: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            instance_targets: Vec::new(),
            tx: tx.to_string(),
            ty: String::new(),
            tz: String::new(),
            rx: String::new(),
            ry: String::new(),
            rz: rz.to_string(),
            outputs: Vec::new(),
            name: None,
            face_flip: false,
            face_spin: String::new(),
            roll_angle: String::new(),
            face_offset: String::new(),
        };

        // #1233: Free-move translation handles sit on all six faces of the tight AABB.
        {
            let min = Vec3::new(0.0, 0.0, 0.0);
            let max = Vec3::new(10.0, 20.0, 30.0);
            let handles = free_move_translation_handles(min, max);
            assert_eq!(handles.len(), 6);
            // One pair per axis, on opposite faces, both pointing outward.
            for axis in 0..3 {
                let pair: Vec<_> = handles.iter().filter(|h| h.axis == axis).collect();
                assert_eq!(pair.len(), 2, "axis {axis}");
                assert!(
                    (pair[0].outward + pair[1].outward).length() < 1e-5,
                    "opposite outward normals on axis {axis}"
                );
                let dir = free_move_axis_dir(axis);
                // Face centres sit on the AABB faces, not at the box centre.
                for h in &pair {
                    let along = (h.origin - (min + max) * 0.5).dot(h.outward);
                    assert!(along > 0.0, "handle outside centre along outward: {h:?}");
                    assert!(
                        (h.outward.dot(dir)).abs() > 0.99,
                        "outward is ±axis for {h:?}"
                    );
                }
            }
            // +X face centre is at max.x, mid y/z.
            let plus_x = handles.iter().find(|h| h.outward == Vec3::X).unwrap();
            assert!((plus_x.origin - Vec3::new(10.0, 10.0, 15.0)).length() < 1e-4);
            let minus_z = handles.iter().find(|h| h.outward == -Vec3::Z).unwrap();
            assert!((minus_z.origin - Vec3::new(5.0, 10.0, 0.0)).length() < 1e-4);
        }
        // #1234: rotation ring is centred on the AABB and clears its corners.
        {
            let min = Vec3::new(0.0, 0.0, 0.0);
            let max = Vec3::new(10.0, 20.0, 30.0);
            let (center, radius) = free_move_rotation_ring(min, max);
            assert!((center - Vec3::new(5.0, 10.0, 15.0)).length() < 1e-4);
            let half_diag = ((max - min) * 0.5).length();
            assert!(radius > half_diag, "ring clears the corners: {radius} vs {half_diag}");
        }

        // No turn typed: a plain translation, exactly as before (#648).
        let plain = move_op_transform(&doc, &op(MoveTranslateMode::Free, "", "10")).unwrap();
        assert_eq!(plain, glam::Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)));

        // 90° about Z, no translation. The box spans 0..10 in x and y, so its centre is
        // (5, 5); turning about that centre leaves the footprint where it is.
        let turned = move_op_transform(&doc, &op(MoveTranslateMode::Free, "90", "")).unwrap();
        let corner = turned.transform_point3(Vec3::ZERO);
        assert!((corner - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-3, "{corner:?}");
        let far = turned.transform_point3(Vec3::new(10.0, 10.0, 0.0));
        assert!((far - Vec3::new(0.0, 10.0, 0.0)).length() < 1e-3, "{far:?}");
        // A turn about the world origin would have thrown the corner somewhere else entirely.
        assert!(
            (corner - glam::Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)
                .transform_point3(Vec3::new(10.0, 0.0, 0.0)))
            .length()
                > 1.0
        );

        // In place ignores every amount typed: the mate is the identity (#1076).
        assert_eq!(
            move_op_transform(&doc, &op(MoveTranslateMode::InPlace, "90", "10")),
            Some(glam::Mat4::IDENTITY)
        );
    }

    /// #1080: the centre a mate lands on is the face's **area** centroid — the same point
    /// whatever way the mesh triangulated it. The face's *key* averages triangle vertices,
    /// which counts a shared vertex once per triangle, so on a plate with a hole it drifts a
    /// couple of tenths off the true middle and a peg seats visibly off-centre.
    #[test]
    fn a_faces_mating_centre_is_its_area_centroid_not_its_vertex_average() {
        // A square, triangulated lopsidedly: three triangles, so the shared corner (0, 0) is
        // counted three times by a vertex average and drags it off centre.
        let sq = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let tris = vec![
            [sq(0.0, 0.0), sq(10.0, 0.0), sq(10.0, 5.0)],
            [sq(0.0, 0.0), sq(10.0, 5.0), sq(10.0, 10.0)],
            [sq(0.0, 0.0), sq(10.0, 10.0), sq(0.0, 10.0)],
        ];
        let exact = face_group_area_centroid(&tris);
        assert!(
            (exact - sq(5.0, 5.0)).length() < 1e-4,
            "the area centroid is the square's real middle, got {exact:?}"
        );
        // The vertex average is not — which is why the key is not what a mate lands on.
        let averaged = face_group_center(&tris);
        assert!(
            (averaged - sq(5.0, 5.0)).length() > 0.5,
            "the vertex average drifts with the triangulation, got {averaged:?}"
        );
        // And the centre among the nine candidates is the accurate one.
        let centre = *face_snap_points(&tris).last().expect("the centre candidate");
        assert!((centre - exact).length() < 1e-4, "{centre:?}");
    }

    /// #1083: a rectangular face offers exactly nine points to mate on — its four corners,
    /// the midpoint of each edge, and its centre. A mesh splits a straight edge into several
    /// triangles, so the boundary has more vertices than the face has corners; only the real
    /// turns count, or a rectangle would offer a midpoint per triangle instead of per edge.
    #[test]
    fn a_rectangular_face_offers_nine_points_to_mate_on() {
        let (mut doc, _sketch, ext) = box_doc(); // 10x10 footprint, 5 tall
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let solid = body_solid_mesh(&doc, bkey(0)).unwrap();
        let cap = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid)
            .into_iter()
            .find(|t| {
                (t[0][1] - t[0][0]).cross(t[0][2] - t[0][0]).normalize_or_zero().z > 0.9
            })
            .expect("the top cap");

        let points = face_snap_points(&cap);
        assert_eq!(points.len(), 9, "four corners, four edge midpoints, the centre: {points:?}");
        let has = |p: Vec3| points.iter().any(|q| (*q - p).length() < 1e-3);
        // The four corners.
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
            assert!(has(Vec3::new(x, y, 5.0)), "corner ({x}, {y}) missing from {points:?}");
        }
        // The four edge midpoints — the ones that were missing entirely.
        for (x, y) in [(5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (0.0, 5.0)] {
            assert!(has(Vec3::new(x, y, 5.0)), "midpoint ({x}, {y}) missing from {points:?}");
        }
        // And the centre.
        assert!(has(Vec3::new(5.0, 5.0, 5.0)), "the face centre is missing from {points:?}");
    }

    /// #1074: a point on a face is stored as an offset in the **face's own axes**, resolved
    /// against the live triangle group — so the same offset names the same corner of the face
    /// wherever that face has got to. A world-space position would name a fixed spot instead,
    /// and a raised face would slide out from under it.
    #[test]
    fn a_point_on_a_face_names_the_same_spot_after_the_face_moves() {
        let (mut doc, _sketch, ext) = box_doc(); // 10x10 footprint, 5 tall
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let solid = body_solid_mesh(&doc, bkey(0)).unwrap();
        let cap = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid)
            .into_iter()
            .find(|tris| {
                (tris[0][1] - tris[0][0])
                    .cross(tris[0][2] - tris[0][0])
                    .normalize_or_zero()
                    .z
                    > 0.9
            })
            .expect("the top cap");
        let q = crate::hierarchy::quantize_body_point;
        let key = |uv: [i32; 2]| crate::model::MovePointRef::OnFace {
            body: bkey(0),
            centroid: q(face_group_center(&cap)),
            normal: q((cap[0][1] - cap[0][0]).cross(cap[0][2] - cap[0][0]).normalize_or_zero()),
            uv,
        };

        // The centre is uv = [0, 0] — the #738 face-centre point, unchanged.
        let center = move_point_world(&doc, &key([0, 0])).expect("the face centre");
        assert!((center - face_group_center(&cap)).length() < 1e-3, "{center:?}");

        // A corner of the cap, expressed in the face's axes, is that corner in the world.
        let corner = cap.iter().flat_map(|t| t.iter()).copied().fold(
            Vec3::new(f32::MIN, f32::MIN, 0.0),
            |a, b| Vec3::new(a.x.max(b.x), a.y.max(b.y), b.z),
        );
        let uv = face_world_uv(&cap, corner);
        assert_ne!(uv, [0, 0], "a corner is not the centre");
        let resolved = move_point_world(&doc, &key(uv)).expect("the corner");
        assert!((resolved - corner).length() < 1e-2, "{resolved:?} vs {corner:?}");

        // Now raise the box. The cap's centroid key moves with it (that is how every
        // face-keyed reference works), so re-derive the key — what is under test is that the
        // *offset* still lands on the same corner, now 4 mm higher.
        doc.extrusions[xkey(0)].distance = 9.0;
        let solid = body_solid_mesh(&doc, bkey(0)).unwrap();
        let moved_cap = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid)
            .into_iter()
            .find(|tris| {
                (tris[0][1] - tris[0][0])
                    .cross(tris[0][2] - tris[0][0])
                    .normalize_or_zero()
                    .z
                    > 0.9
            })
            .expect("the raised cap");
        let raised_key = crate::model::MovePointRef::OnFace {
            body: bkey(0),
            centroid: q(face_group_center(&moved_cap)),
            normal: q((moved_cap[0][1] - moved_cap[0][0])
                .cross(moved_cap[0][2] - moved_cap[0][0])
                .normalize_or_zero()),
            uv,
        };
        let raised = move_point_world(&doc, &raised_key).expect("the corner, raised");
        assert!((raised.z - 9.0).abs() < 1e-2, "the point rode up with its face: {raised:?}");
        assert!(
            (raised.x - resolved.x).abs() < 1e-2 && (raised.y - resolved.y).abs() < 1e-2,
            "and stayed the same corner: {raised:?} vs {resolved:?}"
        );
    }

    /// #162: `body_solid_mesh` is memoized on document geometry — an in-place mutation
    /// (no shape_order change, e.g. editing the extrusion distance) must still invalidate
    /// the cache and produce the new solid.
    #[test]
    fn body_mesh_cache_invalidates_on_in_place_geometry_edits() {
        let (mut doc, _sketch, ext) = box_doc();
        doc.extrusions.insert(ext);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Extrusion(xkey(0)),
            material: None,
            name: None,
            shadow: false,
        });
        let before = body_solid_mesh(&doc, bkey(0)).expect("box mesh");
        let (_, before_max) = before.bounds().unwrap();
        // Cached call returns the same mesh.
        assert_eq!(body_solid_mesh(&doc, bkey(0)).unwrap(), before);

        doc.extrusions[xkey(0)].distance = 9.0;
        let after = body_solid_mesh(&doc, bkey(0)).expect("re-meshed box");
        let (_, after_max) = after.bounds().unwrap();
        assert!(
            (after_max.z - 9.0).abs() < 1e-3 && (before_max.z - 5.0).abs() < 1e-3,
            "cache must invalidate on distance edit: before z {} after z {}",
            before_max.z,
            after_max.z
        );
    }

    #[test]
    fn treatable_edges_enumerates_verticals_and_caps_for_rect_none_for_circle() {
        let (doc, _sketch, ext) = box_doc();
        let mut doc = doc;
        doc.extrusions.insert(ext);
        let edges = treatable_edges(&doc);
        // 4 vertical + 4 bottom cap + 4 top cap = 12 for a rectangular profile.
        assert_eq!(edges.len(), 12);
        assert!(edges
            .iter()
            .all(|(ei, _, _, _)| *ei == TreatableSolid::Extrusion(xkey(0))));

        let (mut cdoc, csketch) = sketch_doc();
        cdoc.circles
            .insert(Circle::from_local_center_radius(csketch, 0.0, 0.0, 5.0, 0.0));
        cdoc.extrusions
            .insert(extrusion(csketch, vec![ExtrudeFace::Circle(rkey(0))], 6.0));
        // Circle profiles have no polygonal edges; their two cap rims are treatable
        // (#177), emitted as chord segments naming Cap { edge: 0 }.
        let circle_edges = treatable_edges(&cdoc);
        assert!(!circle_edges.is_empty());
        assert!(circle_edges
            .iter()
            .all(|(_, e, _, _)| matches!(e, ExtrusionEdgeRef::Cap { edge: 0, .. })));
    }

    /// #1329: a Shape-tool cuboid is a 12-edge box — the same vertical + cap topology a
    /// rectangular extrusion exposes. The fillet/chamfer tool must offer those edges.
    #[test]
    fn treatable_edges_include_cuboid_primitive_edges() {
        let mut doc = Document::default();
        let mut shape = crate::model::Primitive::new(crate::model::PrimitiveKind::Cuboid);
        shape.width = "40".into();
        shape.depth = "50".into();
        shape.height = "22".into();
        let pi = doc.primitives.insert(shape);
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        assert_eq!(pi, crate::model::primitive_key_for_slot(0));
        let edges = treatable_edges(&doc);
        // 4 vertical + 4 bottom cap + 4 top cap.
        assert_eq!(edges.len(), 12, "a cuboid has 12 treatable edges, got {}", edges.len());
        let verticals = edges
            .iter()
            .filter(|(_, e, _, _)| matches!(e, ExtrusionEdgeRef::Vertical { .. }))
            .count();
        let tops = edges
            .iter()
            .filter(|(_, e, _, _)| matches!(e, ExtrusionEdgeRef::Cap { top: true, .. }))
            .count();
        let bases = edges
            .iter()
            .filter(|(_, e, _, _)| matches!(e, ExtrusionEdgeRef::Cap { top: false, .. }))
            .count();
        assert_eq!(verticals, 4);
        assert_eq!(tops, 4);
        assert_eq!(bases, 4);

        // A selected mesh edge of the cuboid must resolve to that analytic edge.
        let (_, edge, a, b) = edges
            .iter()
            .find(|(_, e, _, _)| matches!(e, ExtrusionEdgeRef::Cap { top: true, edge: 0, .. }))
            .cloned()
            .expect("top cap edge 0");
        let q = crate::hierarchy::quantize_body_point;
        assert!(
            treatable_edge_for_selection(&doc, bkey(0), q(a), q(b)).is_some(),
            "cuboid top edge must resolve from a body-edge pick"
        );
        let _ = (pi, edge);
    }

    /// #1329: the reported document is a lone cuboid; every one of its 12 box edges is
    /// treatable, including the top front edge the screenshot is hovering.
    #[test]
    fn issue_1329_cuboid_edges_are_treatable() {
        let bytes = include_bytes!("../tests/fixtures/issue_1329.json");
        let doc = crate::storage::from_json_bytes(bytes).expect("load issue 1329");
        assert_eq!(doc.primitives.len(), 1);
        assert!(doc.extrusions.is_empty());
        let edges = treatable_edges(&doc);
        assert_eq!(
            edges.len(),
            12,
            "the reported cuboid must expose 12 treatable edges, got {}",
            edges.len()
        );

        // The hovered top-front edge in the screenshot is a top cap — filleting it must
        // produce a live treated body with less volume than the sharp cuboid.
        let (solid, edge, _, _) = edges
            .iter()
            .copied()
            .find(|(_, e, _, _)| matches!(e, ExtrusionEdgeRef::Cap { top: true, .. }))
            .expect("top cap edge");
        let mut state = crate::actions::AppState::default();
        state.doc = doc;
        let before = body_solid_mesh(&state.doc, bkey(0)).expect("sharp cuboid");
        let v0 = mesh_signed_volume(&before).abs();
        assert!(matches!(
            state.apply(crate::actions::Action::CommitEdgeTreatments {
                edges: vec![(solid, edge)],
                kind: crate::model::VertexTreatmentKind::Fillet,
                amount: 3.0,
            }),
            crate::actions::ActionResult::Ok
        ));
        assert_eq!(state.doc.edge_treatment_ops.len(), 1);
        let live = state.doc.edge_treatment_ops.values().nth(0).unwrap().outputs[0];
        let after = body_solid_mesh(&state.doc, live).expect("filleted cuboid");
        let v1 = mesh_signed_volume(&after).abs();
        assert!(v1 < v0 - 1.0, "fillet must cut the reported cuboid: {v1} vs {v0}");
    }

    /// #1337 fixture: combined shelled/sliced body plus the circle sketched on it.
    fn issue_1337_cut_setup() -> (Document, crate::model::BodyKey, Extrusion) {
        let bytes = include_bytes!("../tests/fixtures/issue_1337.json");
        let mut doc = crate::storage::from_json_bytes(bytes).expect("load issue 1337");
        doc.bump_mesh_rev();
        let body = doc
            .bodies
            .iter()
            .find_map(|(k, b)| (!b.shadow).then_some(k))
            .expect("live combined body");
        let circle = doc.circles.keys().next().expect("circle on the body face");
        let sketch = doc.circles[circle].sketch;
        let mut cut = Extrusion {
            sketch,
            faces: vec![ExtrudeFace::Circle(circle)],
            distance: 177.0,
            target: None,
            expression: String::new(),
            symmetric: false,
            name: None,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        };
        // Same inward flip the extrude tool applies (`resolve_cut_direction` / #805).
        if cut_tool_bites(&doc, body, &cut) == Some(false) {
            cut.distance = -cut.distance;
        }
        (doc, body, cut)
    }

    /// #1337: cutting the reported body with the sketched circle must remove material.
    #[test]
    fn issue_1337_circle_cut_preview_removes_material() {
        let (doc, body, cut) = issue_1337_cut_setup();
        let intact = body_solid_mesh(&doc, body).expect("intact combined body");
        let intact_vol = mesh_signed_volume(&intact).abs();
        let preview = preview_cut_body_mesh(&doc, body, &cut).expect("cut preview");
        let preview_vol = mesh_signed_volume(&preview).abs();
        assert!(
            preview_vol < intact_vol - 1.0,
            "circle cut should remove material: {preview_vol} vs {intact_vol}"
        );
        assert!(
            cut_tool_bites(&doc, body, &cut) == Some(true),
            "177 mm circle extrude must overlap the reported body"
        );
    }

    /// #1337: idle frames (same cut, same document) must not re-run kernel booleans.
    /// The live cut preview used to rebuild the shelled/sliced/combined solid and
    /// boolean it on every frame — including `cut_tool_bites` — so the tool felt
    /// frozen while just looking at the gizmo.
    #[test]
    fn issue_1337_idle_cut_preview_does_not_reboolean() {
        let (doc, body, cut) = issue_1337_cut_setup();
        crate::kernel::reset_boolean_call_count();
        let t0 = crate::time::Instant::now();
        let first = preview_cut_body_mesh(&doc, body, &cut).expect("first preview");
        let _ = cut_tool_bites(&doc, body, &cut);
        let first_ms = t0.elapsed().as_millis();
        let after_first = crate::kernel::boolean_call_count();
        assert!(after_first > 0, "first preview must run the kernel boolean");

        let t1 = crate::time::Instant::now();
        let second = preview_cut_body_mesh(&doc, body, &cut).expect("cached preview");
        let _ = cut_tool_bites(&doc, body, &cut);
        let idle_ms = t1.elapsed().as_millis();
        let after_idle = crate::kernel::boolean_call_count();
        eprintln!(
            "issue_1337 idle: first={first_ms}ms booleans={after_first}, idle={idle_ms}ms extra={}",
            after_idle - after_first
        );
        assert_eq!(
            first.triangles.len(),
            second.triangles.len(),
            "cached preview must match"
        );
        assert_eq!(
            after_idle, after_first,
            "idle cut-preview frames must not re-run kernel booleans (was {} extra)",
            after_idle - after_first
        );
    }

    /// #1337: dragging the cut distance must not rebuild the target body's history.
    /// Only the tool solid and the cut/common against the memoized body should run.
    #[test]
    fn issue_1337_changing_cut_distance_reuses_target_body() {
        let (doc, body, mut cut) = issue_1337_cut_setup();
        crate::kernel::reset_boolean_call_count();
        let t0 = crate::time::Instant::now();
        let _ = preview_cut_body_mesh(&doc, body, &cut).expect("preview 177");
        let _ = cut_tool_bites(&doc, body, &cut);
        let first_ms = t0.elapsed().as_millis();
        let after_first = crate::kernel::boolean_call_count();

        cut.distance = 180.0_f32.copysign(cut.distance);
        crate::kernel::reset_boolean_call_count();
        let t1 = crate::time::Instant::now();
        let preview = preview_cut_body_mesh(&doc, body, &cut).expect("preview 180");
        let bites = cut_tool_bites(&doc, body, &cut);
        let drag_ms = t1.elapsed().as_millis();
        let extra = crate::kernel::boolean_call_count();
        eprintln!(
            "issue_1337 drag: first={first_ms}ms booleans={after_first}, drag={drag_ms}ms booleans={extra}"
        );
        assert!(bites == Some(true), "nudged cut still overlaps");
        assert!(
            !preview.triangles.is_empty(),
            "nudged cut must still produce a mesh"
        );
        assert!(
            extra <= 2,
            "changing cut distance must reuse the target body (at most Common+Cut), got {extra} booleans"
        );
    }

    /// #1338: the reported document has a tapered circle extrusion that never landed as a
    /// cut on the combined body. Editing it to Cut must apply the hole and leave the body
    /// open to further cuts.
    #[test]
    fn issue_1338_pending_cut_on_combined_body_applies() {
        let bytes = include_bytes!("../tests/fixtures/issue_1338.json");
        let doc = crate::storage::from_json_bytes(bytes).expect("load issue 1338");
        let ei = doc.extrusions.keys().next().expect("pending extrusion");
        assert!(
            crate::model::body_index_for_extrusion(&doc, ei).is_none(),
            "the reported extrusion is an orphan — it never attached to a body"
        );
        let live = doc
            .bodies
            .iter()
            .find_map(|(k, b)| {
                (!b.shadow && matches!(b.source, crate::model::BodySource::Boolean { .. }))
                    .then_some(k)
            })
            .expect("live combined body");
        let v0 = mesh_signed_volume(&body_solid_mesh(&doc, live).expect("combined mesh")).abs();

        let mut state = crate::actions::AppState::default();
        state.doc = doc;
        assert!(matches!(
            state.apply(crate::actions::Action::EditExtrusion { index: ei }),
            crate::actions::ActionResult::Ok
        ));
        assert!(matches!(
            state.apply(crate::actions::Action::SetExtrudeBodyMode {
                mode: crate::actions::ExtrudeBodyMode::Cut(live),
            }),
            crate::actions::ActionResult::Ok
        ));
        assert!(matches!(
            state.apply(crate::actions::Action::CommitExtrusion),
            crate::actions::ActionResult::Ok
        ));
        assert_eq!(
            state.doc.bodies[live].source.cut_extrusion_indices(),
            [ei],
            "pending cut must attach to the combined body"
        );
        let v1 = mesh_signed_volume(&body_solid_mesh(&state.doc, live).expect("cut mesh")).abs();
        assert!(v1 < v0 - 1.0, "pending cut must remove material: {v1} vs {v0}");
    }

    #[test]
    fn extrusion_edge_anchor_points_at_edge_midpoint() {
        let (mut doc, _sketch, ext) = box_doc();
        doc.extrusions.insert(ext);
        // Vertical edge 0 -> profile vertex 1 = local (10, 0); base z=0, top z=5.
        let (origin, normal) =
            extrusion_edge_anchor(&doc, xkey(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 0 })
                .unwrap();
        assert!((origin - Vec3::new(10.0, 0.0, 2.5)).length() < 1e-3, "{origin:?}");
        assert!(normal.length() > 0.9 && normal.length() < 1.1);

        // A removed extrusion, a stale key, and an out-of-range edge index all resolve to
        // `None` (#1055).
        let mut gone = doc.clone();
        gone.extrusions.remove(xkey(0));
        assert!(
            extrusion_edge_anchor(&gone, xkey(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 0 })
                .is_none()
        );
        assert!(extrusion_edge_anchor(&doc, xkey(7), ExtrusionEdgeRef::Vertical { face: 0, edge: 0 })
            .is_none());
        assert!(
            extrusion_edge_anchor(&doc, xkey(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 9 })
                .is_none()
        );
    }

    #[test]
    fn edge_treatment_conflicts_detects_shared_vertex_not_the_same_edge() {
        let n = 4;
        let existing = vec![EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 }, // touches vertex 1
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        }];
        // Cap edge 0 touches vertices 0 and 1 (base ring) -> shares vertex 1 with the vertical.
        assert!(edge_treatment_conflicts(
            &existing,
            ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            n
        ));
        // Cap edge 1 touches vertices 1 and 2 -> also shares vertex 1.
        assert!(edge_treatment_conflicts(
            &existing,
            ExtrusionEdgeRef::Cap { face: 0, edge: 1, top: false },
            n
        ));
        // Vertical edge 1 touches vertex 2 only -> no conflict.
        assert!(!edge_treatment_conflicts(
            &existing,
            ExtrusionEdgeRef::Vertical { face: 0, edge: 1 },
            n
        ));
        // A top-cap edge sharing the same vertex on a *different* ring doesn't conflict, since
        // the existing vertical treatment already reserves both rings at vertex 1 — wait, it
        // does conflict (vertical reserves top too): edge 0's top-cap also touches vertex 1.
        assert!(edge_treatment_conflicts(
            &existing,
            ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: true },
            n
        ));
        // Re-treating the exact same edge is not a conflict with itself.
        assert!(!edge_treatment_conflicts(
            &existing,
            ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            n
        ));
        // A different face entirely never conflicts.
        assert!(!edge_treatment_conflicts(
            &existing,
            ExtrusionEdgeRef::Cap { face: 1, edge: 0, top: false },
            n
        ));
    }

    #[test]
    fn extrusion_edge_exists_checks_range_and_profile_kind() {
        let (doc, _sketch, ext) = box_doc();
        let mut doc = doc;
        doc.extrusions.insert(ext.clone());
        assert!(extrusion_edge_exists(&doc, xkey(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 3 }));
        assert!(!extrusion_edge_exists(&doc, xkey(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 4 }));
        assert!(!extrusion_edge_exists(&doc, xkey(5), ExtrusionEdgeRef::Vertical { face: 0, edge: 0 }));
        assert!(!extrusion_edge_exists(&doc, xkey(0), ExtrusionEdgeRef::Vertical { face: 1, edge: 0 }));
        doc.extrusions.remove(xkey(0));
        assert!(!extrusion_edge_exists(&doc, xkey(0), ExtrusionEdgeRef::Vertical { face: 0, edge: 0 }));
    }

    #[test]
    fn extrusion_with_edge_treatment_replaces_same_edge_rather_than_stacking() {
        let (doc, _sketch, ext) = box_doc();
        let mut doc = doc;
        doc.extrusions.insert(ext);
        let edge = ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        let once = extrusion_with_edge_treatment(
            &doc,
            xkey(0),
            EdgeTreatment { edge, kind: VertexTreatmentKind::Chamfer, amount: 1.0 },
        )
        .unwrap();
        doc.extrusions[xkey(0)] = once;
        let twice = extrusion_with_edge_treatment(
            &doc,
            xkey(0),
            EdgeTreatment { edge, kind: VertexTreatmentKind::Fillet, amount: 3.0 },
        )
        .unwrap();
        assert_eq!(twice.edge_treatments.len(), 1);
        assert_eq!(twice.edge_treatments[0].kind, VertexTreatmentKind::Fillet);
        assert_eq!(twice.edge_treatments[0].amount, 3.0);
    }

    /// #103: the commit-time kernel trial — a fillet the kernel can build passes, an
    /// oversized one (radius >> the 10x10x5 box) fails, and a base extrusion the kernel
    /// can't represent at all (here: two faces) is left to the mesh-bevel fallback (trial
    /// passes, it has nothing to validate against).
    #[test]
    fn occt_edge_treatments_feasible_rejects_only_what_the_kernel_cannot_build() {
        let (mut doc, sketch, ext) = box_doc();
        doc.extrusions.insert(ext);
        let edge = ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        let small = extrusion_with_edge_treatment(
            &doc,
            xkey(0),
            EdgeTreatment { edge, kind: VertexTreatmentKind::Fillet, amount: 2.0 },
        )
        .unwrap();
        assert!(occt_edge_treatments_feasible(&doc, xkey(0), &small));
        let oversized = extrusion_with_edge_treatment(
            &doc,
            xkey(0),
            EdgeTreatment { edge, kind: VertexTreatmentKind::Fillet, amount: 500.0 },
        )
        .unwrap();
        assert!(!occt_edge_treatments_feasible(&doc, xkey(0), &oversized));

        // A two-face extrusion is kernel-representable too (each face's prism fused), so
        // the feasibility trial still applies: the oversized fillet is rejected on it.
        let second = rect_profile(&mut doc, sketch, 20.0, 20.0, 10.0, 10.0);
        let extra_face = second.clone();
        doc.extrusions[xkey(0)].faces.push(extra_face);
        let candidate = extrusion_with_edge_treatment(
            &doc,
            xkey(0),
            EdgeTreatment { edge, kind: VertexTreatmentKind::Fillet, amount: 500.0 },
        )
        .unwrap();
        assert!(!occt_edge_treatments_feasible(&doc, xkey(0), &candidate));
    }

    /// #103 part 2: [`kernel_fallback_cut_warning`] fires exactly when a cut-bearing body
    /// can't be built by the kernel (so the additive-only fallback would silently drop the
    /// cuts), and stays quiet for healthy bodies or bodies without cuts.
    #[test]
    fn kernel_fallback_cut_warning_fires_only_for_kernel_infeasible_cut_bodies() {
        let mut doc = cut_body_doc();
        assert_eq!(kernel_fallback_cut_warning(&doc), None, "healthy cut body: no warning");
        doc.extrusions[xkey(0)].edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Fillet,
            amount: 500.0,
        });
        let warning = kernel_fallback_cut_warning(&doc).expect("infeasible cut body warns");
        assert!(warning.contains("cuts are not shown"), "{warning}");
        // Without cuts there's nothing to silently drop: no warning even though the body
        // still falls back to the mesh-bevel path.
        doc.bodies.values_mut().nth(0).unwrap().source = crate::model::BodySource::Solid { base: None, add: vec![xkey(0)], cut: vec![] };
        assert_eq!(kernel_fallback_cut_warning(&doc), None);
    }
}



/// The sketch-local loop of an [`ExtrudeFace::SketchRegion`] (#993): the region of the hosted
/// sketch's plane that contains the profile's seed point. `None` when the sketch's lines no
/// longer divide the face, or none of the regions they make still contains the seed.
pub fn sketch_region_uv(doc: &Document, face: &ExtrudeFace) -> Option<Vec<(f32, f32)>> {
    let ExtrudeFace::SketchRegion { sketch, seed_u, seed_v } = face else {
        return None;
    };
    let seed = crate::model::sketch_region_seed_point(*seed_u, *seed_v);
    crate::polygon::sketch_plane_regions(doc, *sketch)
        .into_iter()
        .find(|region| crate::polygon::point_in_polygon_2d(seed, region))
}
