//! Extrusions: turning coplanar sketch faces into 3D solid meshes.
//!
//! Stage 1 builds the data-driven solid geometry (a prism/cylinder per face) from an
//! [`Extrusion`]. Rendering and the interactive tool layer build on top of this.
// The mesh API is exercised by tests and consumed by the (next-stage) GPU renderer.
#![allow(dead_code)]

use crate::face::{local_to_world, sketch_frame, sketch_geometry_frame, SketchFrame};
use crate::geometric_constraints::point_uv;
use crate::model::{
    vertex_treatment_geometry, Document, EdgeTreatment, ExtrudeFace, ExtrudeTarget, Extrusion,
    ExtrusionEdgeRef, FaceId, VertexTreatmentKind,
};
use glam::Vec3;
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
    for (i, body) in doc.bodies.iter().enumerate() {
        if body.deleted {
            continue;
        }
        if let Some((min, max)) = body_solid_mesh(doc, i).and_then(|m| m.bounds()) {
            extend(min);
            extend(max);
        }
    }
    // Construction geometry is scaffolding, not "the model" — zoom-to-fit (#164) frames
    // only real geometry.
    for line in doc.lines.iter().filter(|l| !l.deleted && !l.construction) {
        if let Some(frame) = sketch_geometry_frame(doc, line.sketch) {
            for (u, v) in line.sample_local(crate::model::BEZIER_SEGMENTS) {
                extend(local_to_world(&frame, u, v));
            }
        }
    }
    for circle in doc.circles.iter().filter(|c| !c.deleted && !c.construction) {
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
    let mut mesh = SolidMesh::default();
    for (face_index, face) in extrusion.faces.iter().enumerate() {
        if let Some((profile, top, normal)) = extrusion_profile_rings(doc, extrusion, face, distance)
        {
            // A face with holes (annulus, #268) has no edge treatments in the fallback path;
            // build it as a hollow region (hole-aware caps + inner walls).
            let holes0: Vec<Vec<Vec3>> = face_region_world(doc, face)
                .map(|(_, holes, _)| holes)
                .unwrap_or_default();
            if !holes0.is_empty() {
                let holes: Vec<Vec<Vec3>> = holes0
                    .iter()
                    .map(|h| {
                        h.iter()
                            .map(|p| extruded_base_point(doc, extrusion, normal, *p, distance))
                            .collect()
                    })
                    .collect();
                let holes_top: Vec<Vec<Vec3>> = holes0
                    .iter()
                    .map(|h| {
                        h.iter()
                            .map(|p| extruded_free_end_point(doc, extrusion, normal, *p, distance))
                            .collect()
                    })
                    .collect();
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
    // A leaf face with holes (a text glyph's counters, #285): subtract each hole's prism so the
    // glyph extrudes hollow. Boolean faces get their holes via the recursion above instead.
    if let Some((_, holes, hnormal)) = face_region_world(doc, face) {
        for hole0 in holes {
            if hole0.len() < 3 {
                continue;
            }
            let mut hole: Vec<Vec3> = hole0
                .iter()
                .map(|p| extruded_base_point(doc, extrusion, hnormal, *p, distance))
                .collect();
            let mut htop: Vec<Vec3> = hole0
                .iter()
                .map(|p| extruded_free_end_point(doc, extrusion, hnormal, *p, distance))
                .collect();
            if overshoot > 1e-6 {
                let u = (htop[0] - hole[0]).normalize_or_zero();
                hole = hole.iter().map(|p| *p - u * overshoot).collect();
                htop = htop.iter().map(|t| *t + u * overshoot).collect();
            }
            let hdir = htop[0] - hole[0];
            let hole_solid = crate::kernel::Shape::prism(&hole, hdir)?;
            shape = shape.boolean(&hole_solid, crate::kernel::BoolOp::Cut)?;
        }
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
    indices: &[usize],
) -> Option<Option<crate::kernel::Shape>> {
    use crate::kernel::BoolOp;
    let mut fused: Option<crate::kernel::Shape> = None;
    for &ei in indices {
        let extrusion = doc.extrusions.get(ei)?;
        if extrusion.deleted {
            continue;
        }
        let distance = effective_distance(doc, extrusion);
        if extrusion.faces.is_empty() || distance.abs() < 1e-4 {
            continue;
        }
        let shape = occt_extrusion_shape(doc, extrusion, distance)?;
        // Placements this add contributes: the base, plus one per repeat-op replay offset (#220
        // add-replay) — an add extrusion targeted by a repeat op is fused again at each instance,
        // growing N bumps instead of one.
        let mut placements: Vec<glam::Mat4> = vec![glam::Mat4::IDENTITY];
        for op in doc.repeat_ops.iter() {
            if op.deleted || !op.extrusion_targets.contains(&ei) {
                continue;
            }
            if let (Some((_, dir)), Some(offsets)) =
                (axis_world(doc, op.axis), repeat_offsets(doc, op))
            {
                for off in offsets {
                    placements.push(glam::Mat4::from_translation(dir * off));
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
    add_indices: &[usize],
    cut_indices: &[usize],
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
    add_indices: &[usize],
    cut_indices: &[usize],
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    let mut solid = occt_fused_extrusions(doc, add_indices)??;
    // Subtract each cut extrusion's solid. A cut that isn't kernel-representable aborts to the
    // fallback (returns None); a cut contributing no geometry is a no-op.
    for &ei in cut_indices {
        let extrusion = doc.extrusions.get(ei)?;
        if extrusion.deleted {
            continue;
        }
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
        solid = solid.boolean(&cut, BoolOp::Cut)?;
        // Repeat-operation replay (#220): any non-deleted repeat op that targets this cut
        // extrusion subtracts the same tool again at each instance offset along its axis —
        // punching N holes rather than copying a solid.
        for op in doc.repeat_ops.iter() {
            if op.deleted || !op.extrusion_targets.contains(&ei) {
                continue;
            }
            if let (Some((_, dir)), Some(offsets)) =
                (axis_world(doc, op.axis), repeat_offsets(doc, op))
            {
                for off in offsets {
                    let m = glam::Mat4::from_translation(dir * off);
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

/// The body's real OCCT BREP solid (adds fused, cuts subtracted), *before* tessellation —
/// used by STEP export (#65) to write genuine BREP rather than tessellated triangles. `None`
/// for a deleted/missing body, an imported-mesh body (no kernel solid), or a body whose
/// geometry isn't fully kernel-representable (the caller then falls back to the mesh path).
pub fn occt_body_shape(doc: &Document, body_index: usize) -> Option<crate::kernel::Shape> {
    let body = doc.bodies.get(body_index)?;
    if body.deleted || body.source.imported_mesh_index().is_some() {
        return None;
    }
    let mut solid = match body.source {
        crate::model::BodySource::Revolve(ri) => {
            occt_revolution_shape(doc, doc.revolutions.get(ri).filter(|r| !r.deleted)?)?
        }
        crate::model::BodySource::Sweep(fi) => {
            occt_sweep_shape(doc, doc.sweeps.get(fi).filter(|f| !f.deleted)?)?
        }
        crate::model::BodySource::Loft(li) => {
            occt_loft_shape(doc, doc.lofts.get(li).filter(|l| !l.deleted)?)?
        }
        crate::model::BodySource::Boolean { op, solid } => {
            return occt_boolean_output_shape(doc, op, solid);
        }
        crate::model::BodySource::Moved { op, target } => {
            return occt_moved_output_shape(doc, op, target);
        }
        crate::model::BodySource::Mirrored { op, target } => {
            return occt_mirrored_output_shape(doc, op, target);
        }
        crate::model::BodySource::Repeated { op, target, instance } => {
            return occt_repeated_output_shape(doc, op, target, instance);
        }
        crate::model::BodySource::Sliced { op, target, piece } => {
            return occt_sliced_output_shape(doc, op, target, piece);
        }
        crate::model::BodySource::EdgeTreated { op, target } => {
            return occt_edge_treated_output_shape(doc, op, target);
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
/// body's live mesh. `None` once the mesh no longer has that corner/edge.
pub fn move_point_world(doc: &Document, point: &crate::model::MovePointRef) -> Option<Vec3> {
    match point {
        crate::model::MovePointRef::Vertex { body, p } => {
            crate::parameters::body_vertex_world_position(doc, *body, *p)
        }
        crate::model::MovePointRef::EdgeMidpoint { body, a, b } => {
            let (p0, p1) = crate::parameters::body_edge_world_segment(doc, *body, *a, *b)?;
            Some((p0 + p1) * 0.5)
        }
    }
}

/// A move's translation vector (#648/#650): in `Snap` mode the offset that lands the source
/// point on the target point, otherwise the `tx`/`ty`/`tz` expressions. A snap with either
/// point missing or unresolvable contributes no translation, so the op stays valid while the
/// user is still picking.
pub fn move_op_translation(doc: &Document, op: &crate::model::MoveOperation) -> Option<Vec3> {
    if op.has_snap_translation() {
        let (source, target) = (op.source_point.as_ref()?, op.target_point.as_ref()?);
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

pub fn move_op_transform(doc: &Document, op: &crate::model::MoveOperation) -> Option<glam::Mat4> {
    let t = move_op_translation(doc, op)?;
    let mut m = glam::Mat4::from_translation(t);
    if let Some(axis) = op.axis {
        let angle_rad = if op.angle.trim().is_empty() {
            0.0
        } else {
            crate::value::eval_angle_rad_in_doc(&op.angle, doc)?
        };
        if angle_rad.abs() > 1e-9 {
            let (axis_origin, dir) = axis_world(doc, axis)?;
            // The move turns about its picked rotation point when it has one (#651) — the
            // axis then only supplies a direction. Without one it turns about the axis itself.
            let origin = op
                .rotation_pivot()
                .and_then(|p| move_point_world(doc, p))
                .unwrap_or(axis_origin);
            let rot = glam::Mat4::from_translation(origin)
                * glam::Mat4::from_axis_angle(dir, angle_rad)
                * glam::Mat4::from_translation(-origin);
            m *= rot;
        }
    }
    Some(m)
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
            let alive = doc.bodies.get(body).is_some_and(|b| !b.deleted);
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
    op_index: usize,
    target: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.move_ops.get(op_index).filter(|o| !o.deleted)?;
    let &input = op.targets.get(target)?;
    if op.outputs.contains(&input) {
        return None;
    }
    let shape = occt_body_shape(doc, input)?;
    let m = move_op_transform(doc, op)?;
    shape.transformed(&mat4_to_rows_3x4(&m))
}

/// A clone of `doc` with the edge-treatment op's treatments spliced onto the target input
/// body's extrusions, together with that input body's index (#531). Building or meshing the
/// input body in this clone then reuses the whole extrusion chamfer/fillet machinery — so an
/// edge-treatment op's output is exactly its input body, beveled. `None` when the op or target
/// is gone, or an output body was fed back as its own input.
fn edge_treated_input_doc(
    doc: &Document,
    op_index: usize,
    target: usize,
) -> Option<(Document, usize)> {
    let op = doc.edge_treatment_ops.get(op_index).filter(|o| !o.deleted)?;
    let &input = op.targets.get(target)?;
    if op.outputs.contains(&input) {
        return None;
    }
    let mut clone = doc.clone();
    for te in op.edges.iter().filter(|e| e.target == target) {
        if let Some(ext) = clone.extrusions.get_mut(te.extrusion) {
            ext.edge_treatments.push(crate::model::EdgeTreatment {
                edge: te.edge,
                kind: op.kind,
                amount: op.amount,
            });
        }
    }
    Some((clone, input))
}

/// The BREP solid of one edge-treatment output (#531): the input body's shape built with the
/// op's chamfer/fillet edges spliced onto its extrusions.
fn occt_edge_treated_output_shape(
    doc: &Document,
    op_index: usize,
    target: usize,
) -> Option<crate::kernel::Shape> {
    let (clone, input) = edge_treated_input_doc(doc, op_index, target)?;
    occt_body_shape(&clone, input)
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
    op_index: usize,
    target: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.mirror_ops.get(op_index).filter(|o| !o.deleted)?;
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
pub fn sketch_repeat_offsets(
    doc: &Document,
    op: &crate::model::SketchRepeatOperation,
) -> Option<Vec<f32>> {
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
        let l = doc.lines.get(li).filter(|l| !l.deleted)?;
        extend(l.x0 * du + l.y0 * dv);
        extend(l.x1 * du + l.y1 * dv);
    }
    for &ci in &op.circle_targets {
        let c = doc.circles.get(ci).filter(|c| !c.deleted)?;
        let center = c.cx * du + c.cy * dv;
        extend(center - c.r);
        extend(center + c.r);
    }
    if !min_p.is_finite() || !max_p.is_finite() {
        return None;
    }
    let extent = (max_p - min_p).max(0.0);
    let eval = |expr: &str| -> Option<f32> {
        (!expr.trim().is_empty())
            .then(|| crate::value::eval_length_mm_in_doc(expr, doc))
            .flatten()
    };
    let count = || -> Option<usize> {
        let n = crate::value::eval_parameter_in_doc(&op.count, doc).and_then(|v| match v {
            crate::value::EvaluatedParameter::LengthMm(n) => Some(n),
            crate::value::EvaluatedParameter::AngleRad(_) => None,
        })?;
        (n >= 1.0).then_some((n.round() as usize).min(MAX_REPEAT_INSTANCES))
    };
    spacing_offsets(op.mode, extent, count(), eval(&op.spacing), eval(&op.length))
}

/// Every body strictly **downstream** of `seeds` (#260): bodies produced by an operation that
/// consumes a seed body, transitively. Used to fade the descendants of an operation being edited.
pub fn descendant_bodies(doc: &Document, seeds: &[usize]) -> std::collections::HashSet<usize> {
    use std::collections::{HashSet, VecDeque};
    let mut result = HashSet::new();
    let mut queue: VecDeque<usize> = seeds.iter().copied().collect();
    let mut visited: HashSet<usize> = seeds.iter().copied().collect();
    while let Some(bi) = queue.pop_front() {
        let mut outs: Vec<usize> = Vec::new();
        for op in doc.boolean_ops.iter().filter(|o| !o.deleted) {
            if op.a.contains(&bi) || op.b.contains(&bi) {
                outs.extend(op.outputs.iter().copied());
            }
        }
        for op in doc.move_ops.iter().filter(|o| !o.deleted) {
            if op.targets.contains(&bi) {
                outs.extend(op.outputs.iter().copied());
            }
        }
        for op in doc.repeat_ops.iter().filter(|o| !o.deleted) {
            if op.targets.contains(&bi) {
                outs.extend(op.outputs.iter().copied());
            }
        }
        for op in doc.slice_ops.iter().filter(|o| !o.deleted) {
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
    targets: &[usize],
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

pub fn repeat_offsets(doc: &Document, op: &crate::model::RepeatOperation) -> Option<Vec<f32>> {
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
        let n = crate::value::eval_parameter_in_doc(&op.count, doc).and_then(|v| match v {
            crate::value::EvaluatedParameter::LengthMm(n) => Some(n),
            crate::value::EvaluatedParameter::AngleRad(_) => None,
        })?;
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
    op_index: usize,
    target: usize,
    instance: usize,
) -> Option<crate::kernel::Shape> {
    let op = doc.repeat_ops.get(op_index).filter(|o| !o.deleted)?;
    let &input = op.targets.get(target)?;
    let (_, dir) = axis_world(doc, op.axis)?;
    let offsets = repeat_offsets(doc, op)?;
    let offset = *offsets.get(instance.checked_sub(1)?)?;
    let shape = occt_body_shape(doc, input)?;
    let t = dir * offset;
    let m = glam::Mat4::from_translation(t);
    shape.transformed(&mat4_to_rows_3x4(&m))
}

/// The half-space cutting solid for one slice cutter: a large prism built on the cutter's
/// plane, occupying the `+normal` side. With `extend_infinite` the profile is a big square
/// covering the target; otherwise it's the cutter face's own boundary (a planar body face),
/// so the cut only reaches material within that footprint. Construction planes have no
/// finite boundary and always cut as infinite planes.
fn occt_slice_halfspace(
    doc: &Document,
    cutter: &FaceId,
    extend_infinite: bool,
    target: usize,
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

/// The ordered fragments one slice target splits into: start from the input body's solid(s)
/// and, for each cutter, replace every current piece with its two sides of the cutter's
/// half-space, dropping empty results. Deterministic order (common side before cut side, in
/// cutter order) keeps output-body mapping stable across edits.
fn occt_slice_pieces(doc: &Document, op_index: usize, target_pos: usize) -> Option<Vec<crate::kernel::Shape>> {
    use crate::kernel::BoolOp;
    const MIN_PIECE_VOLUME: f64 = 1e-6;
    let op = doc.slice_ops.get(op_index).filter(|o| !o.deleted)?;
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
    for cutter in &op.cutters {
        let Some(hs) = occt_slice_halfspace(doc, cutter, op.extend_infinite, input) else {
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

/// The BREP solid of one slice fragment: piece `piece` of target `target`. The target's
/// *last* fragment absorbs any extra solids a rebuild produced (fused into one shape), so
/// the body list stays stable while geometry changes underneath — same contract as boolean
/// outputs.
fn occt_sliced_output_shape(
    doc: &Document,
    op_index: usize,
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
fn slice_target_body_count(doc: &Document, op_index: usize, target: usize) -> usize {
    doc.bodies
        .iter()
        .filter(|b| {
            !b.deleted
                && matches!(
                    b.source,
                    crate::model::BodySource::Sliced { op, target: t, .. }
                        if op == op_index && t == target
                )
        })
        .count()
}

/// Number of fragments a slice target currently produces (commit-time output sizing).
pub fn slice_piece_count(doc: &Document, op_index: usize, target: usize) -> Option<usize> {
    Some(occt_slice_pieces(doc, op_index, target)?.len())
}


/// The whole (possibly multi-solid) OCCT result of one boolean operation: A-side bodies
/// fused, then combined with the fused B side per the operation's algebra. Difference
/// (symmetric) is (A∪B) − (A∩B). `None` when any input body isn't kernel-representable.
fn occt_boolean_result_shape(
    doc: &Document,
    op_index: usize,
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    let op = doc.boolean_ops.get(op_index).filter(|o| !o.deleted)?;
    let fuse_all = |list: &[usize]| -> Option<crate::kernel::Shape> {
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
    op_index: usize,
    ordinal: usize,
) -> Option<crate::kernel::Shape> {
    use crate::kernel::BoolOp;
    let op = doc.boolean_ops.get(op_index).filter(|o| !o.deleted)?;
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

/// Number of solids a boolean operation currently produces (commit-time output sizing).
pub fn boolean_result_solid_count(doc: &Document, op_index: usize) -> Option<usize> {
    Some(occt_boolean_result_shape(doc, op_index)?.solids().len())
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
    extrusion: usize,
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
        .position(|b| !b.deleted && b.source.owns_extrusion(extrusion))
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
    for (i, body) in doc.bodies.iter().enumerate() {
        let cut_by_revolve = revolutions_targeting(doc, i).iter().any(|(_, cut)| *cut);
        let cut_by_sweep = sweeps_targeting(doc, i).iter().any(|(_, cut)| *cut);
        let cut_by_loft = lofts_targeting(doc, i).iter().any(|(_, cut)| *cut);
        if body.deleted
            || body.source.imported_mesh_index().is_some()
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
                .unwrap_or_else(|| format!("body {i}"));
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

/// Effective sweep angle in degrees, clamped to a sane range (360 = full revolution).
pub fn revolve_effective_angle(rev: &crate::model::Revolution) -> f32 {
    rev.angle_deg.clamp(0.1, 360.0)
}

/// World polygon of a partial revolve's flat start/end side (#621): the profile rotated
/// to the sweep's start (`end = false`) or end (`end = true`) angle, plus the outward
/// face normal (the sweep-tangent direction at the cap, pointing out of the solid).
/// `None` for a full 360° sweep — that closes on itself and has no flat sides.
pub fn revolve_cap_polygon_world(
    doc: &Document,
    revolution: usize,
    profile: &crate::model::ExtrudeFace,
    end: bool,
) -> Option<(Vec<Vec3>, Vec3)> {
    let rev = doc.revolutions.get(revolution)?;
    if rev.deleted || !rev.faces.contains(profile) {
        return None;
    }
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let angle = revolve_effective_angle(rev);
    if angle >= 359.99 {
        return None;
    }
    let start = if rev.symmetric { -angle / 2.0 } else { 0.0 };
    let cap_angle = if end { start + angle } else { start };
    let q = glam::Quat::from_axis_angle(dir, cap_angle.to_radians());
    let (pts, _) = face_profile_world(doc, profile)?;
    if pts.len() < 3 {
        return None;
    }
    let poly: Vec<Vec3> = pts.iter().map(|p| origin + q * (*p - origin)).collect();
    // Sweep tangent at the rotated centroid (direction of increasing angle, right-hand
    // rule about `dir`): the end cap faces along it, the start cap opposite.
    let centroid = poly.iter().copied().sum::<Vec3>() / poly.len() as f32;
    let tangent = dir.cross(centroid - origin).normalize_or_zero();
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
    revolution: usize,
    profile: &ExtrudeFace,
    edge: usize,
) -> Option<(Vec<Vec3>, SketchFrame, Vec3)> {
    let rev = doc.revolutions.get(revolution)?;
    if rev.deleted || !rev.faces.contains(profile) {
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
    let full = angle >= 359.99;
    let steps = (((CIRCLE_SEGMENTS as f32) * angle / 360.0).ceil() as usize).max(8);
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
    revolution: usize,
    profile: &ExtrudeFace,
    edge: usize,
) -> Option<(f32, f32)> {
    let rev = doc.revolutions.get(revolution)?;
    if revolve_effective_angle(rev) < 359.99 {
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

/// Hand-rolled lathe mesh for a revolution (the no-kernel fallback and the live ghost
/// preview): each profile is swept around the axis in angular steps, walls stitched
/// between consecutive rotated rings, with the start/end profile faces capped for a
/// partial sweep (a full 360-degree sweep closes on itself and needs no caps).
pub fn revolve_mesh(doc: &Document, rev: &crate::model::Revolution) -> Option<SolidMesh> {
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let angle = revolve_effective_angle(rev);
    let full = angle >= 359.99;
    let start = if rev.symmetric { -angle / 2.0 } else { 0.0 };
    let mut mesh = SolidMesh::default();
    for face in &rev.faces {
        let (profile, _normal) = face_profile_world(doc, face)?;
        if profile.len() < 3 {
            return None;
        }
        let steps = (((CIRCLE_SEGMENTS as f32) * angle / 360.0).ceil() as usize).max(8);
        let rings: Vec<Vec<Vec3>> = (0..=steps)
            .map(|i| {
                let a = (start + angle * i as f32 / steps as f32).to_radians();
                let q = glam::Quat::from_axis_angle(dir, a);
                profile.iter().map(|p| origin + q * (*p - origin)).collect()
            })
            .collect();
        // Orientation reference: the *rotated profile centroid* at each sweep step — a
        // point locally inside the solid, which stays correct for washer-like profiles
        // that don't contain the axis (a single on-axis reference flips the inner wall).
        let centroid = profile.iter().copied().sum::<Vec3>() / profile.len() as f32;
        let centroids: Vec<Vec3> = (0..=steps)
            .map(|i| {
                let a = (start + angle * i as f32 / steps as f32).to_radians();
                origin + glam::Quat::from_axis_angle(dir, a) * (centroid - origin)
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
/// `BRepPrimAPI_MakeRevol`, multiple profiles fused. `None` when any face/axis is
/// degenerate or the kernel can't build it (callers fall back to [`revolve_mesh`]).
pub fn occt_revolution_shape(
    doc: &Document,
    rev: &crate::model::Revolution,
) -> Option<crate::kernel::Shape> {
    let (origin, dir) = revolve_axis_world(doc, rev)?;
    let angle_rad = revolve_effective_angle(rev).to_radians() as f64;
    let mut fused: Option<crate::kernel::Shape> = None;
    for face in &rev.faces {
        let shape = occt_face_revolve_solid(doc, face, origin, dir, angle_rad, rev.symmetric)?;
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
/// revolve their single boundary loop directly.
fn occt_face_revolve_solid(
    doc: &Document,
    face: &ExtrudeFace,
    origin: Vec3,
    dir: Vec3,
    angle_rad: f64,
    symmetric: bool,
) -> Option<crate::kernel::Shape> {
    if let ExtrudeFace::Boolean { op, a, b } = face {
        let sa = occt_face_revolve_solid(doc, a, origin, dir, angle_rad, symmetric)?;
        let sb = occt_face_revolve_solid(doc, b, origin, dir, angle_rad, symmetric)?;
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
    crate::kernel::Shape::revolve(&profile, origin, dir, angle_rad, symmetric)
}

/// The revolutions fusing into (`false`) or cutting (`true`) `body_index`.
pub fn revolutions_targeting(
    doc: &Document,
    body_index: usize,
) -> Vec<(usize, bool)> {
    doc.revolutions
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.deleted)
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
    static SWEEP_CUT_PREVIEW_CACHE: std::cell::RefCell<((u64, u64), Vec<(usize, SolidMesh)>)> =
        std::cell::RefCell::new(((0, 0), Vec::new()));
}

/// Cut-result meshes for the in-progress sweep cut preview: each target body of the
/// draft's `Cut` list meshed from a scratch document with the draft sweep appended, so the
/// preview shows the finished carve (mirroring the extrude cut preview, #142). Bodies the
/// scratch build can't mesh are simply absent. Cached per `(document, draft)` state.
pub fn preview_sweep_cut_meshes(
    doc: &Document,
    fp: &crate::model::Sweep,
) -> Vec<(usize, SolidMesh)> {
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
        scratch.sweeps.push(fp.clone());
        let meshes: Vec<(usize, SolidMesh)> = bodies
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
pub fn sweeps_targeting(doc: &Document, body_index: usize) -> Vec<(usize, bool)> {
    doc.sweeps
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.deleted)
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
pub fn lofts_targeting(doc: &Document, body_index: usize) -> Vec<(usize, bool)> {
    doc.lofts
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.deleted)
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
            let circle = doc.circles.get(ci).filter(|c| !c.deleted && !c.construction)?;
            Some(crate::model::LoftSection {
                sketch: circle.sketch,
                face: ExtrudeFace::Circle(ci),
            })
        }
        SceneElement::Line(li) => {
            let line = doc.lines.get(li).filter(|l| !l.deleted && !l.construction)?;
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
        ExtrudeFace::Boolean { .. } => Vec::new(),
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
                for bi in 0..doc.bodies.len() {
                    if doc.bodies[bi].source == crate::model::BodySource::Revolve(op) {
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
                for bi in 0..doc.bodies.len() {
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
                    .filter(|e| !e.deleted)
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
                    .filter(|l| !l.deleted)
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
                    .filter(|c| !c.deleted)
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
            // A body face (#555): its centroid is the only stored point; enough to frame toward it.
            SceneElement::BodyFace { centroid, .. } => {
                extend(crate::hierarchy::dequantize_body_point(centroid));
            }
            SceneElement::Sketch(_)
            | SceneElement::ConstructionPlane(_)
            | SceneElement::Constraint(_)
            | SceneElement::FaceEdge(_)
            | SceneElement::Origin
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

/// Fingerprint of every document input body meshing reads (#162): sketch geometry, planes,
/// extrusions, and body sources are hashed structurally (via their serde encodings, streamed
/// straight into a hasher — no allocation of the encoded form); imported meshes are
/// append-only after load, so their name + triangle count suffices. Two documents with equal
/// fingerprints mesh identically, which keys [`body_solid_mesh`]'s cache.
fn document_mesh_fingerprint(doc: &Document) -> u64 {
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
            // Every downstream feature whose output body geometry is a function of its inputs must
            // be in the fingerprint, so editing an ancestor (or a parameter one of them evaluates
            // live) invalidates the descendant's cached mesh and forces a rebuild. Ops that
            // evaluate expressions on the fly — moves (`move_op_transform` reads `tx`/angle),
            // repeats (`repeat_offsets`), etc. — otherwise leave stale caches, since their input
            // parameters live in `doc.parameters` (not the op struct) and the op's expression
            // *string* doesn't change when a parameter it references does.
            &doc.parameters,
            &doc.repeat_ops,
            &doc.move_ops,
            &doc.slice_ops,
            &doc.boolean_ops,
            &doc.revolutions,
            &doc.sweeps,
            &doc.lofts,
        ),
    )
    .ok();
    for mesh in &doc.imported_meshes {
        std::io::Write::write_all(&mut writer, mesh.source_name.as_bytes()).ok();
        writer.0.write_usize(mesh.triangles.len());
    }
    writer.0.finish()
}

thread_local! {
    /// Per-thread memo for [`body_solid_mesh`] (#162): `(document fingerprint, body → mesh)`.
    /// The kernel rebuild is expensive (an extrude-to-slanted-plane does OCCT booleans), and
    /// one frame calls `body_solid_mesh` several times per body (scene build, hover picking,
    /// occlusion, the selection aura) — without this the viewer visibly slows down. Any
    /// change to the fingerprinted geometry clears the memo.
    static BODY_MESH_CACHE: std::cell::RefCell<(u64, HashMap<usize, Option<SolidMesh>>)> =
        std::cell::RefCell::new((0, HashMap::new()));
}

/// Build the solid mesh for a single body (by index), or `None` if the body is deleted,
/// missing, or its source feature produces no geometry. Memoized per document state (#162);
/// see [`BODY_MESH_CACHE`].
pub fn body_solid_mesh(doc: &Document, body_index: usize) -> Option<SolidMesh> {
    let fingerprint = document_mesh_fingerprint(doc);
    BODY_MESH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            cache.0 = fingerprint;
            cache.1.clear();
        }
        if let Some(mesh) = cache.1.get(&body_index) {
            return mesh.clone();
        }
        let mesh = body_solid_mesh_uncached(doc, body_index);
        cache.1.insert(body_index, mesh.clone());
        mesh
    })
}

/// Build a body's solid mesh **without** consulting or populating [`BODY_MESH_CACHE`]. Used for
/// the in-progress-edit descendant preview (#260), which meshes a throwaway scratch document each
/// frame — routing that through the cache would evict the real document's warm meshes every frame
/// (the two docs fingerprint differently), forcing a full rebuild of the whole scene.
pub fn body_solid_mesh_uncached_pub(doc: &Document, body_index: usize) -> Option<SolidMesh> {
    body_solid_mesh_uncached(doc, body_index)
}

fn body_solid_mesh_uncached(doc: &Document, body_index: usize) -> Option<SolidMesh> {
    let body = doc.bodies.get(body_index)?;
    if body.deleted {
        return None;
    }
    if let Some(idx) = body.source.imported_mesh_index() {
        let imported = doc.imported_meshes.get(idx)?;
        return (!imported.triangles.is_empty()).then(|| SolidMesh {
            triangles: imported.triangles.clone(),
        });
    }

    if let crate::model::BodySource::Repeated { op, target, instance } = body.source {
        let rp = doc.repeat_ops.get(op).filter(|o| !o.deleted)?;
        let &input = rp.targets.get(target)?;
        if input == body_index {
            return None;
        }
        let (_, dir) = axis_world(doc, rp.axis)?;
        let offsets = repeat_offsets(doc, rp)?;
        let offset = *offsets.get(instance.checked_sub(1)?)?;
        let source = body_solid_mesh_uncached(doc, input)?;
        let t = dir * offset;
        let triangles = source
            .triangles
            .iter()
            .map(|tri| [tri[0] + t, tri[1] + t, tri[2] + t])
            .collect();
        return Some(SolidMesh { triangles });
    }
    if let crate::model::BodySource::Moved { op, target } = body.source {
        let mv = doc.move_ops.get(op).filter(|o| !o.deleted)?;
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
    if let crate::model::BodySource::Mirrored { op, target } = body.source {
        let mr = doc.mirror_ops.get(op).filter(|o| !o.deleted)?;
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
    if let crate::model::BodySource::Boolean { op, solid } = body.source {
        // Boolean outputs are kernel-computed; shadow inputs keep their own meshes.
        {
            let shape = occt_boolean_output_shape(doc, op, solid)?;
            let tris = shape.tessellate(OCCT_DEFLECTION as f64);
            return (!tris.is_empty()).then_some(SolidMesh { triangles: tris });
        }
    }
    if let crate::model::BodySource::EdgeTreated { op, target } = body.source {
        // A beveled output is exactly its input body meshed with the op's treatments spliced
        // in — reusing the extrusion chamfer/fillet path (mesh or kernel), so the default
        // (kernel-off) build still bevels.
        let (clone, input) = edge_treated_input_doc(doc, op, target)?;
        return body_solid_mesh_uncached(&clone, input);
    }
    if let crate::model::BodySource::Sliced { op, target, piece } = body.source {
        // Slice fragments are kernel-computed; shadow inputs keep their own meshes.
        {
            let shape = occt_sliced_output_shape(doc, op, target, piece)?;
            let tris = shape.tessellate(OCCT_DEFLECTION as f64);
            return (!tris.is_empty()).then_some(SolidMesh { triangles: tris });
        }
    }
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
        let rev = doc.revolutions.get(ri).filter(|r| !r.deleted)?;
        return revolve_mesh(doc, rev);
    }
    if let crate::model::BodySource::Sweep(fi) = body.source {
        let fp = doc.sweeps.get(fi).filter(|f| !f.deleted)?;
        return sweep_mesh(doc, fp);
    }
    if let crate::model::BodySource::Loft(li) = body.source {
        let loft = doc.lofts.get(li).filter(|l| !l.deleted)?;
        return loft_mesh(doc, loft);
    }
    let mut mesh = SolidMesh::default();
    for &ei in body.source.extrusion_indices() {
        let Some(extrusion) = doc.extrusions.get(ei) else {
            continue;
        };
        if extrusion.deleted {
            continue;
        }
        if let Some(solid) = extrusion_mesh(doc, extrusion) {
            mesh.triangles.extend(solid.triangles);
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
fn preview_cache_key(doc: &Document, extrusion: &Extrusion, body_index: usize) -> (u64, u64) {
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
    let key = preview_cache_key(doc, extrusion, usize::MAX);
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
pub fn cut_tool_bites(doc: &Document, body_index: usize, cut: &Extrusion) -> Option<bool> {
    {
        let distance = effective_distance(doc, cut);
        if cut.faces.is_empty() || distance.abs() < 1e-4 {
            return None;
        }
        let tool = occt_extrusion_shape(doc, cut, distance)?;
        let body = occt_body_shape(doc, body_index)?;
        let common = body.boolean(&tool, crate::kernel::BoolOp::Common)?;
        let mesh = SolidMesh { triangles: common.tessellate(OCCT_DEFLECTION as f64) };
        Some(mesh_signed_volume(&mesh).abs() > 1e-3)
    }
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
pub fn preview_cut_body_mesh(doc: &Document, body_index: usize, cut: &Extrusion) -> Option<SolidMesh> {
    {
        let body = doc.bodies.get(body_index)?;
        if body.deleted || body.source.imported_mesh_index().is_some() {
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
            let cut_index = clone.extrusions.len();
            clone.extrusions.push(cut.clone());
            let mut cut_indices = body.source.cut_extrusion_indices().to_vec();
            cut_indices.push(cut_index);
            let mesh = occt_body_mesh(&clone, body.source.extrusion_indices(), &cut_indices);
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
    // fused compound (identical output to concatenation for them). Imported meshes have no
    // kernel shape, so they're concatenated on top; if any kernel body isn't representable the
    // whole union is unreliable and we fall back to plain concatenation.
    if let Some(mesh) = occt_document_union_mesh(doc) {
        return mesh;
    }
    let mut mesh = SolidMesh::default();
    for bi in 0..doc.bodies.len() {
        if let Some(solid) = body_solid_mesh(doc, bi) {
            mesh.triangles.extend(solid.triangles);
        }
    }
    mesh
}

/// Fuse every kernel-representable body into one unioned solid and tessellate it, appending any
/// imported-mesh bodies' triangles (they have no kernel shape). `None` — so the caller falls
/// back to plain per-body concatenation — when a non-imported body isn't kernel-representable
/// or the fuse fails to build/tessellate. See [`document_solid_mesh`] (#146).
fn occt_document_union_mesh(doc: &Document) -> Option<SolidMesh> {
    use crate::kernel::BoolOp;
    let mut fused: Option<crate::kernel::Shape> = None;
    let mut imported_triangles: Vec<[Vec3; 3]> = Vec::new();
    let mut saw_kernel_body = false;
    for (bi, body) in doc.bodies.iter().enumerate() {
        if body.deleted {
            continue;
        }
        if body.source.imported_mesh_index().is_some() {
            if let Some(solid) = body_solid_mesh(doc, bi) {
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
fn body_face_plane(doc: &Document, face_id: &crate::model::FaceId) -> Option<(Vec3, Vec3)> {
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

/// Base and free-end loops for a profile face under this extrusion's distance/target.
fn extrusion_profile_rings(
    doc: &Document,
    extrusion: &Extrusion,
    face: &ExtrudeFace,
    distance: f32,
) -> Option<(Vec<Vec3>, Vec<Vec3>, Vec3)> {
    let (profile0, normal) = face_profile_world(doc, face)?;
    if profile0.len() < 3 {
        return None;
    }
    let base: Vec<Vec3> = profile0
        .iter()
        .map(|p| extruded_base_point(doc, extrusion, normal, *p, distance))
        .collect();
    let top: Vec<Vec3> = profile0
        .iter()
        .map(|p| extruded_free_end_point(doc, extrusion, normal, *p, distance))
        .collect();
    Some((base, top, normal))
}

/// The effective signed depth: derived from `target`'s extended plane when set, else `distance`.
/// For a symmetric extrusion this is still the *total* height (end-to-end).
pub fn effective_distance(doc: &Document, extrusion: &Extrusion) -> f32 {
    if let Some(target) = &extrusion.target {
        if let Some((base, normal)) = faces_anchor(doc, &extrusion.faces) {
            if let Some(d) = target_distance(doc, base, normal, target) {
                return d;
            }
        }
    }
    extrusion.distance
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
    op: usize,
    instance: usize,
) -> Option<(Vec3, Vec3)> {
    let rep = doc.repeat_ops.get(op).filter(|o| !o.deleted)?;
    let (_, dir) = axis_world(doc, rep.axis)?;
    // `repeat_offsets` lists the copies only; instance 0 is the original body.
    let offsets = repeat_offsets(doc, rep)?;
    let offset = *offsets.get(instance.checked_sub(1)?)?;
    let (p, n) = body_face_plane(doc, face)?;
    Some((p + dir * offset, n))
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
            let img = doc.tracing_images.get(*image).filter(|i| !i.deleted)?;
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
        ExtrudeFace::Boolean { .. } | ExtrudeFace::TextGlyph { .. } => {
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
    text_index: usize,
    glyph_index: usize,
) -> Option<(Vec<(f32, f32)>, Vec<Vec<(f32, f32)>>)> {
    let t = doc.sketch_texts.get(text_index).filter(|t| !t.deleted)?;
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
pub fn face_profile_world(doc: &Document, face: &ExtrudeFace) -> Option<(Vec<Vec3>, Vec3)> {
    match face {
        ExtrudeFace::Circle(index) => {
            let circle = doc.circles.get(*index)?;
            if circle.deleted {
                return None;
            }
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
            if circle.deleted || circle.sketch != sketch {
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
        ExtrudeFace::Polygon(lines) => {
            let first = doc.lines.get(*lines.first()?)?;
            if first.deleted || first.sketch != sketch {
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
    for (i, c) in doc.circles.iter().enumerate() {
        if !c.deleted && c.sketch == sketch {
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
fn polygon_profile_world(doc: &Document, lines: &[usize]) -> Option<(Vec<Vec3>, Vec3)> {
    let first = doc.lines.get(*lines.first()?)?;
    if first.deleted || lines.iter().any(|&li| doc.lines.get(li).is_none_or(|l| l.deleted)) {
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
    extrusion: usize,
    profile: &ExtrudeFace,
    top: bool,
) -> Option<Vec<Vec3>> {
    let ext = doc.extrusions.get(extrusion)?;
    if ext.deleted || !ext.faces.contains(profile) {
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
    extrusion: usize,
    profile: &ExtrudeFace,
    top: bool,
) -> Vec<Vec<Vec3>> {
    let Some(ext) = doc.extrusions.get(extrusion) else {
        return Vec::new();
    };
    if ext.deleted || !ext.faces.contains(profile) {
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
    extrusion: usize,
    profile: &ExtrudeFace,
    edge: usize,
) -> Option<[Vec3; 4]> {
    let ext = doc.extrusions.get(extrusion)?;
    if ext.deleted || !ext.faces.contains(profile) || edge >= side_face_count(profile) {
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

/// Whether `edge` names a currently-treatable analytic edge: `extrusion` exists and isn't
/// deleted, `edge.face()` indexes one of its faces, that face has an analytic (`Rect`/
/// `Polygon`, at least 3 sides) profile — a `Circle` profile has none, see
/// [`side_face_count`] — and `edge`'s own index is in range.
pub fn extrusion_edge_exists(doc: &Document, extrusion: usize, edge: ExtrusionEdgeRef) -> bool {
    let Some(ext) = doc.extrusions.get(extrusion) else {
        return false;
    };
    if ext.deleted {
        return false;
    }
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
    body: usize,
    a: [i32; 3],
    b: [i32; 3],
) -> Option<(usize, ExtrusionEdgeRef)> {
    let q = crate::hierarchy::quantize_body_point;
    for (extrusion, edge, ea, eb) in treatable_edges(doc) {
        if crate::model::body_index_for_extrusion(doc, extrusion) != Some(body) {
            continue;
        }
        let (qa, qb) = (q(ea), q(eb));
        if (qa == a && qb == b) || (qa == b && qb == a) {
            return Some((extrusion, edge));
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
) -> Vec<(usize, ExtrusionEdgeRef)> {
    let mut out: Vec<(usize, ExtrusionEdgeRef)> = Vec::new();
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
pub fn treatable_edges(doc: &Document) -> Vec<(usize, ExtrusionEdgeRef, Vec3, Vec3)> {
    let mut out = Vec::new();
    for (ei, ext) in doc.extrusions.iter().enumerate() {
        if ext.deleted {
            continue;
        }
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
                                ei,
                                ExtrusionEdgeRef::Cap { face: fi, edge: 0, top: false },
                                base[k],
                                base[k2],
                            ));
                            out.push((
                                ei,
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
                out.push((ei, ExtrusionEdgeRef::Vertical { face: fi, edge }, base[v], top[v]));
                let e2 = (edge + 1) % n;
                out.push((
                    ei,
                    ExtrusionEdgeRef::Cap { face: fi, edge, top: false },
                    base[edge],
                    base[e2],
                ));
                out.push((
                    ei,
                    ExtrusionEdgeRef::Cap { face: fi, edge, top: true },
                    top[edge],
                    top[e2],
                ));
            }
        }
    }
    out
}

/// World-space origin (edge midpoint) and normal (inward bisector of the edge's two adjacent
/// faces, pointing into the material so pulling the gizmo away from the edge increases the
/// amount) for the 3D edge chamfer/fillet gizmo — the 3D analogue of `vertex_treatment_anchor`
/// in `main.rs`. `None` if the edge no longer resolves (deleted extrusion, out-of-range index,
/// or degenerate geometry).
pub fn extrusion_edge_anchor(doc: &Document, extrusion: usize, edge: ExtrusionEdgeRef) -> Option<(Vec3, Vec3)> {
    let ext = doc.extrusions.get(extrusion)?;
    if ext.deleted {
        return None;
    }
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
/// own failure mode for the 2D case) before [`crate::actions::Action::CommitEdgeTreatment`]
/// stores the treatment, rather than relying on the mesh builder's silent per-treatment
/// fallback (which never panics, but also never reports *why* an edge didn't visibly change).
pub fn edge_treatment_would_bevel(
    doc: &Document,
    extrusion: usize,
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
    if ext.deleted {
        return false;
    }
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
/// [`crate::actions::Action::CommitEdgeTreatment`] to build the value it stores.
pub fn extrusion_with_edge_treatment(
    doc: &Document,
    extrusion: usize,
    treatment: EdgeTreatment,
) -> Option<Extrusion> {
    extrusion_with_edge_treatments(doc, extrusion, [treatment])
}

/// [`extrusion_with_edge_treatment`] over a whole set (#166): the ghost preview of a
/// multi-edge chamfer/fillet splices every in-progress treatment into the clone at once.
pub fn extrusion_with_edge_treatments(
    doc: &Document,
    extrusion: usize,
    treatments: impl IntoIterator<Item = EdgeTreatment>,
) -> Option<Extrusion> {
    let mut ext = doc.extrusions.get(extrusion)?.clone();
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
    use super::*;
    use crate::model::{Circle, Document, FaceId, Line};

    /// #648/#650: a Snap move only overrides the X/Y/Z expressions once **both** points are
    /// picked — while one is missing (or there are no bodies at all, as for a plane or image
    /// move) the expressions still drive it, so the tool stays usable mid-pick.
    #[test]
    fn snap_move_falls_back_to_expressions_until_both_points_are_picked() {
        use crate::model::{MovePointRef, MoveOperation, MoveTranslateMode};
        let doc = Document::default();
        let base = MoveOperation {
                        rotate_mode: Default::default(),
            rotation_point: None,
            targets: Vec::new(),
            translate_mode: MoveTranslateMode::Snap,
            source_point: None,
            target_point: None,
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            tx: "7".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
            outputs: Vec::new(),
            name: None,
            deleted: false,
        };
        assert!(!base.has_snap_translation());
        assert_eq!(
            move_op_translation(&doc, &base),
            Some(Vec3::new(7.0, 0.0, 0.0)),
            "no points yet: the expressions still drive it"
        );
        // One point isn't enough either.
        let half = MoveOperation {
                        rotate_mode: Default::default(),
            rotation_point: None,
            source_point: Some(MovePointRef::Vertex { body: 0, p: [0; 3] }),
            ..base.clone()
        };
        assert!(!half.has_snap_translation());
        assert_eq!(move_op_translation(&doc, &half), Some(Vec3::new(7.0, 0.0, 0.0)));
        // With both, the snap takes over — and points that no longer resolve contribute
        // nothing rather than killing the op.
        let full = MoveOperation {
                        rotate_mode: Default::default(),
            rotation_point: None,
            target_point: Some(MovePointRef::Vertex { body: 1, p: [100, 0, 0] }),
            ..half
        };
        assert!(full.has_snap_translation());
        assert_eq!(move_op_translation(&doc, &full), Some(Vec3::ZERO));
    }

    /// #644: the distance gizmo hangs off the targets' **start** plane along the axis, centred
    /// on them in the other two directions, so the handle sits at `anchor + dir * distance`.
    #[test]
    fn repeat_gizmo_anchor_sits_on_the_start_plane() {
        let mut doc = Document::default();
        // A triangle spanning x in [10, 30], y in [0, 4], flat at z = 0.
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles: vec![[
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(30.0, 0.0, 0.0),
                Vec3::new(20.0, 6.0, 0.0),
            ]],
            source_name: "tri".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let (anchor, dir) = repeat_gizmo_anchor(&doc, &[0], crate::model::RevolveAxis::X)
            .expect("anchor resolves");
        assert_eq!(dir, Vec3::X);
        // Along X the anchor pins to the minimum (10); across it, the centroid (y = 2).
        assert!((anchor.x - 10.0).abs() < 1e-4, "start plane, got {anchor:?}");
        assert!((anchor.y - 2.0).abs() < 1e-4, "centroid across the axis, got {anchor:?}");
        // No targets, or an axis that can't resolve, gives no gizmo.
        assert!(repeat_gizmo_anchor(&doc, &[], crate::model::RevolveAxis::X).is_none());
        assert!(repeat_gizmo_anchor(&doc, &[0], crate::model::RevolveAxis::Line(9)).is_none());
    }

    /// #643: a body feature edge resolves as an axis (origin `a`, unit direction `a → b`) and
    /// goes dead with the body it was picked on.
    #[test]
    fn axis_world_resolves_a_body_edge() {
        let mut doc = Document::default();
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let axis = crate::model::RevolveAxis::BodyEdge {
            body: 0,
            a: Vec3::new(1.0, 2.0, 3.0),
            b: Vec3::new(1.0, 7.0, 3.0),
        };
        let (origin, dir) = axis_world(&doc, axis).expect("live body resolves");
        assert_eq!(origin, Vec3::new(1.0, 2.0, 3.0));
        assert!((dir - Vec3::Y).length() < 1e-6, "unit direction along a → b, got {dir:?}");
        // A degenerate edge has no direction.
        assert!(axis_world(
            &doc,
            crate::model::RevolveAxis::BodyEdge { body: 0, a: Vec3::ZERO, b: Vec3::ZERO }
        )
        .is_none());
        // A deleted body takes its edges with it.
        doc.bodies[0].deleted = true;
        assert!(axis_world(&doc, axis).is_none());
        assert!(axis_world(&doc, crate::model::RevolveAxis::BodyEdge {
            body: 9,
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
            doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Imported(0),
                name: None,
                deleted: false,
                shadow: false,
            });
        }
        // body0 + body1 -> boolean -> body2; body2 -> move -> body3. body4 is unrelated.
        doc.boolean_ops.push(crate::model::BooleanOperation {
            kind: crate::model::BooleanOpKind::Combine,
            a: vec![0],
            b: vec![1],
            keep_b: false,
            outputs: vec![2],
            name: None,
            deleted: false,
        });
        doc.move_ops.push(crate::model::MoveOperation {
                        rotate_mode: Default::default(),
            rotation_point: None,
            translate_mode: Default::default(),
            source_point: None,
            target_point: None,
            targets: vec![2],
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            tx: String::new(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
            outputs: vec![3],
            name: None,
            deleted: false,
        });

        let d = descendant_bodies(&doc, &[0]);
        assert!(d.contains(&2), "boolean output is downstream of body 0");
        assert!(d.contains(&3), "moved output is downstream transitively");
        assert!(!d.contains(&0) && !d.contains(&1), "seeds/siblings aren't descendants");
        assert!(!d.contains(&4), "unrelated body isn't a descendant");
    }

    fn sketch_doc() -> (Document, crate::model::SketchId) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        (doc, sketch)
    }

    /// #260: the live-edit descendant preview relies on [`body_solid_mesh_uncached_pub`] being a
    /// pure function of the document, so writing an in-progress edit into a scratch clone flows
    /// through to a downstream body's geometry. Here a moved body follows its move op's `tx`.
    #[test]
    fn uncached_mesh_follows_scratch_doc_edit() {
        let (mut doc, _sketch, ext) = box_doc();
        doc.extrusions.push(ext);
        // body 0: the extruded box; body 1: a moved copy of it.
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        doc.move_ops.push(crate::model::MoveOperation {
                        rotate_mode: Default::default(),
            rotation_point: None,
            translate_mode: Default::default(),
            source_point: None,
            target_point: None,
            targets: vec![0],
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            tx: "0mm".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
            outputs: vec![1],
            name: None,
            deleted: false,
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Moved { op: 0, target: 0 },
            name: None,
            deleted: false,
            shadow: false,
        });

        let before = body_solid_mesh_uncached_pub(&doc, 1).and_then(|m| m.bounds()).unwrap();
        // Simulate an in-progress move-gizmo drag on a scratch clone: shift tx by 20mm.
        let mut scratch = doc.clone();
        scratch.move_ops[0].tx = "20mm".to_string();
        let after = body_solid_mesh_uncached_pub(&scratch, 1).and_then(|m| m.bounds()).unwrap();

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
        doc.extrusions.push(extrusion(sketch, vec![outer], 5.0));
        doc.extrusions.push(extrusion(sketch, vec![inner], 5.0));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid {
                add: vec![0],
                cut: vec![1],
            },
            name: None,
            deleted: false,
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
        doc.extrusions.push(ext);
        doc.bodies.push(Body {
            source: BodySource::Solid { add: vec![0], cut: vec![] },
            name: None,
            deleted: false,
            shadow: false,
        });
        // A target plane at x = 30, normal +X (an X-facing wall the repeat fills up to).
        doc.construction_planes.push(crate::construction::plane_from_definition(
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
        let plane_index = doc.construction_planes.len() - 1;

        let mut op = RepeatOperation {
            targets: vec![0],
            plane_targets: Vec::new(),
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            axis: RevolveAxis::X,
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
            deleted: false,
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
        doc.extrusions.push(extrusion(sketch, vec![a], 5.0));
        doc.extrusions.push(extrusion(sketch, vec![b], 5.0));
        for ei in 0..2 {
            doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Extrusion(ei),
                name: None,
                deleted: false,
                shadow: false,
            });
        }
        let vol = mesh_signed_volume(&document_solid_mesh(&doc)).abs();
        assert!(
            (vol - 750.0).abs() < 5.0,
            "expected union volume ~750, got {vol} (concatenation would be ~1000)"
        );
    }


    /// #263/#268: two concentric circles resolve a click in the ring (inside the outer,
    /// outside the inner) to `Difference(outer, inner)` — the ring face — and a click in the
    /// inner disc to `Intersection` (the inner face).
    #[test]
    fn concentric_circles_resolve_a_ring_and_an_inner_face() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0)); // outer
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 4.0, 0.0)); // inner
        let outer = ExtrudeFace::Circle(0);
        let inner = ExtrudeFace::Circle(1);

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
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 4.0, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(0)),
            b: Box::new(ExtrudeFace::Circle(1)),
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
        doc.sketch_texts.push(crate::model::SketchText {
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
            deleted: false,
        });
        let glyph_face = ExtrudeFace::TextGlyph { text: 0, glyph: 0 };
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
        doc.extrusions.push(extrusion(sketch, vec![glyph_face], 5.0));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("o mesh")).abs();
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
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, big_r, 0.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, small_r, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(0)),
            b: Box::new(ExtrudeFace::Circle(1)),
        };
        doc.extrusions.push(extrusion(sketch, vec![ring], h));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("tube mesh")).abs();
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
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 4.0, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(0)),
            b: Box::new(ExtrudeFace::Circle(1)),
        };
        doc.extrusions.push(extrusion(sketch, vec![ring.clone()], h));

        let base = cap_hole_loops_world(&doc, 0, &ring, false);
        let top = cap_hole_loops_world(&doc, 0, &ring, true);
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
        doc.extrusions.push(extrusion(sketch, vec![ExtrudeFace::Circle(0)], h));
        let disc = cap_hole_loops_world(&doc, 1, &ExtrudeFace::Circle(0), true);
        assert!(disc.is_empty(), "a solid disc cap reports no holes");
    }

    /// #177: chamfering a cylinder's top rim through the kernel removes an annular ring
    /// (~perimeter * d^2/2 for a 45-degree chamfer).
    #[test]
    fn circle_boss_rim_chamfer_removes_a_ring() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        let mut ext = extrusion(sketch, vec![ExtrudeFace::Circle(0)], 20.0);
        ext.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: true },
            kind: VertexTreatmentKind::Chamfer,
            amount: 2.0,
        });
        doc.extrusions.push(ext);
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.extrusions.push(extrusion(sketch, vec![plate], 5.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 2.5, 0.0));
        let mut hole = extrusion(sketch, vec![ExtrudeFace::Circle(0)], 6.0);
        hole.edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            kind: VertexTreatmentKind::Fillet,
            amount: 1.0,
        });
        doc.extrusions.push(hole);
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid { add: vec![0], cut: vec![1] },
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.extrusions.push(extrusion(sketch, vec![plate], 5.0));
        // A 2.5mm-radius hole at x = -6.
        doc.circles.push(Circle::from_local_center_radius(sketch, -6.0, 0.0, 2.5, 0.0));
        doc.extrusions.push(extrusion(sketch, vec![ExtrudeFace::Circle(0)], 6.0));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid { add: vec![0], cut: vec![1] },
            name: None,
            deleted: false,
            shadow: false,
        });
        let one_hole = 2000.0 - std::f32::consts::PI * 2.5 * 2.5 * 5.0;
        assert!((mesh_signed_volume(&body_solid_mesh(&doc, 0).unwrap()).abs() - one_hole).abs() < 3.0);

        // Replay the hole (extrusion 1) ×3 along X at 6mm gap → holes at x = -6, 0, +6.
        doc.repeat_ops.push(RepeatOperation {
            targets: Vec::new(),
            plane_targets: Vec::new(),
            extrusion_targets: vec![1],
            sketch_targets: Vec::new(),
            axis: RevolveAxis::X,
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
            deleted: false,
        });
        let three_holes = 2000.0 - 3.0 * std::f32::consts::PI * 2.5 * 2.5 * 5.0;
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).unwrap()).abs();
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
        doc.extrusions.push(extrusion(sketch, vec![box_face], 5.0)); // ×5 = 80
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid { add: vec![0], cut: vec![] },
            name: None,
            deleted: false,
            shadow: false,
        });
        assert!((mesh_signed_volume(&body_solid_mesh(&doc, 0).unwrap()).abs() - 80.0).abs() < 1.0);

        // Replay the add ×3 along X at 10mm gap → boxes at x = 0, 10, 20 (disjoint).
        doc.repeat_ops.push(RepeatOperation {
            targets: Vec::new(),
            plane_targets: Vec::new(),
            extrusion_targets: vec![0],
            sketch_targets: Vec::new(),
            axis: RevolveAxis::X,
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
            deleted: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).unwrap()).abs();
        assert!((vol - 240.0).abs() < 3.0, "expected ~240 (3 boxes), got {vol}");
    }

    /// Ancestor→descendant propagation: a body moved by a parameter expression follows edits to
    /// that parameter. Regression guard for the mesh-cache fingerprint — `doc.parameters` /
    /// `doc.move_ops` must be part of it, or the moved body keeps a stale cached mesh.
    #[test]
    fn parameter_edit_propagates_to_a_moved_descendant() {
        use crate::model::{Body, BodySource, MoveOperation, Parameter};
        let (mut doc, _sketch, ext) = box_doc(); // box x ∈ [0, 10]
        doc.extrusions.push(ext);
        doc.bodies.push(Body {
            source: BodySource::Solid { add: vec![0], cut: vec![] },
            name: None,
            deleted: false,
            shadow: true, // consumed by the move
        });
        doc.parameters.push(Parameter {
            name: "gap".to_string(),
            expression: "10".to_string(),
            deleted: false,
            source: None,
        });
        doc.move_ops.push(MoveOperation {
                        rotate_mode: Default::default(),
            rotation_point: None,
            translate_mode: Default::default(),
            source_point: None,
            target_point: None,
            targets: vec![0],
            plane_targets: Vec::new(),
            image_targets: Vec::new(),
            tx: "gap".to_string(),
            ty: String::new(),
            tz: String::new(),
            axis: None,
            angle: String::new(),
            outputs: vec![1],
            name: None,
            deleted: false,
        });
        doc.bodies.push(Body {
            source: BodySource::Moved { op: 0, target: 0 },
            name: None,
            deleted: false,
            shadow: false,
        });
        let min_x = |doc: &Document, bi: usize| {
            body_solid_mesh(doc, bi)
                .unwrap()
                .triangles
                .iter()
                .flat_map(|t| t.iter())
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min)
        };
        // The moved copy starts at x = 0 + gap(10).
        assert!((min_x(&doc, 1) - 10.0).abs() < 1e-3, "moved by gap = 10");
        // Editing the parameter the move references must propagate to the descendant body.
        doc.parameters[0].expression = "25".to_string();
        assert!(
            (min_x(&doc, 1) - 25.0).abs() < 1e-3,
            "descendant follows the parameter edit (fingerprint includes parameters/move_ops)"
        );
    }

    /// #177: a chamfer on a *cut* circle extrusion's rim carves a countersink into the
    /// body it cuts — more material removed than the plain hole.
    #[test]
    fn cut_hole_rim_chamfer_countersinks_the_body() {
        let (mut doc, sketch) = sketch_doc();
        let plate = rect_profile(&mut doc, sketch, -10.0, -10.0, 20.0, 20.0);
        doc.extrusions.push(extrusion(sketch, vec![plate], 5.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 2.5, 0.0));
        let mut hole = extrusion(sketch, vec![ExtrudeFace::Circle(0)], 6.0);
        hole.edge_treatments.push(EdgeTreatment {
            // The hole prism runs z 0..6 through the 5mm plate; its base rim (z=0) is the
            // plate's bottom surface rim.
            edge: ExtrusionEdgeRef::Cap { face: 0, edge: 0, top: false },
            kind: VertexTreatmentKind::Chamfer,
            amount: 1.0,
        });
        doc.extrusions.push(hole);
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid { add: vec![0], cut: vec![1] },
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        doc.extrusions.push(extrusion(sketch, vec![ExtrudeFace::Circle(0)], 6.0));
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
            0,
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
        doc.extrusions.push(extrusion(sketch, vec![plate], 5.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 35.0, 10.0, 2.5, 0.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 35.0, 30.0, 2.5, 0.0));
        doc.extrusions.push(extrusion(
            sketch,
            vec![ExtrudeFace::Circle(0), ExtrudeFace::Circle(1)],
            6.0,
        ));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid { add: vec![0], cut: vec![1] },
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.extrusions.push(extrusion(sketch, vec![outer], 5.0));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let intact = body_solid_mesh(&doc, 0).expect("intact box");
        let intact_vol = mesh_signed_volume(&intact).abs();

        // A 4x4 column overlapping the box, extruded through it — the pending cut.
        let hole = rect_profile(&mut doc, sketch, 3.0, 3.0, 4.0, 4.0);
        let cut = extrusion(sketch, vec![hole], 5.0);
        let preview = preview_cut_body_mesh(&doc, 0, &cut).expect("cut preview");
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
        doc.extrusions.push(extrusion(sketch, vec![base_profile.clone()], 8.0));

        let second_profile = rect_profile(&mut doc, sketch, 20.0, 0.0, 10.0, 10.0);
        let mut second = extrusion(sketch, vec![second_profile], 3.0);
        second.target = Some(ExtrudeTarget::BodyFace(FaceId::ExtrudeCap {
            extrusion: 0,
            profile: base_profile,
            top: true,
        }));
        doc.extrusions.push(second);

        let depth = effective_distance(&doc, &doc.extrusions[1]);
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
            extrusion: 99,
            profile,
            top: true,
        }));
        doc.extrusions.push(ext);
        let depth = effective_distance(&doc, &doc.extrusions[0]);
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
            symmetric,
            mode,
            name: None,
            deleted: false,
        }
    }

    /// A 10x10 square at x 10..20 revolved 360 degrees around the global Y axis is a
    /// washer: pi * (20^2 - 10^2) * 10.
    #[test]
    fn revolve_full_sweep_makes_a_ring() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 10.0);
        doc.revolutions.push(test_revolution(
            sketch,
            vec![profile],
            360.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Revolve(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.revolutions.push(test_revolution(
            sketch,
            vec![profile.clone()],
            90.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        // Start cap: the profile itself, in the sketch (z = 0) plane.
        let (start, _) = revolve_cap_polygon_world(&doc, 0, &profile, false).expect("start cap");
        assert!(start.iter().all(|p| p.z.abs() < 1e-3));
        // End cap: the profile rotated 90° about +Y — (x, y, 0) lands on (0, y, −x).
        let (end, _) = revolve_cap_polygon_world(&doc, 0, &profile, true).expect("end cap");
        assert!(end.iter().all(|p| p.x.abs() < 1e-3 && p.z < 0.0));
        // Exactly the two constant-height rect edges sweep flat sides; their frames'
        // normals run along the axis, pointing away from the profile.
        let flats: Vec<(usize, SketchFrame)> = (0..revolve_side_count(&profile))
            .filter_map(|e| revolve_side_geom(&doc, 0, &profile, e).map(|(_, f, _)| (e, f)))
            .collect();
        assert_eq!(flats.len(), 2, "two axis-perpendicular edges sweep flat sides");
        for (_, frame) in &flats {
            assert!(frame.normal.cross(Vec3::Y).length() < 1e-4);
        }
        let normal_ys: Vec<f32> = flats.iter().map(|(_, f)| f.normal.y).collect();
        assert!(normal_ys.contains(&-1.0) && normal_ys.contains(&1.0));
        // A full sweep closes on itself: no caps, but the flat washer sides remain.
        doc.revolutions[0].angle_deg = 360.0;
        assert!(revolve_cap_polygon_world(&doc, 0, &profile, false).is_none());
        assert!(revolve_side_geom(&doc, 0, &profile, flats[0].0).is_some());
    }

    /// #626: the tessellated rims of a full revolve chain into whole curves — each circular
    /// rim is ONE chain, so picking any facet selects the entire circle.
    #[test]
    fn revolve_rim_segments_chain_into_whole_curves() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 10.0);
        doc.revolutions.push(test_revolution(
            sketch,
            vec![profile],
            360.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Revolve(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let solid = body_solid_mesh(&doc, 0).expect("mesh");
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
        doc.circles.push(Circle::from_local_center_radius(sketch, d, 0.0, big_r, 0.0));
        doc.circles.push(Circle::from_local_center_radius(sketch, d, 0.0, small_r, 0.0));
        let ring = ExtrudeFace::Boolean {
            op: crate::model::BooleanOp::Difference,
            a: Box::new(ExtrudeFace::Circle(0)),
            b: Box::new(ExtrudeFace::Circle(1)),
        };
        doc.revolutions.push(test_revolution(
            sketch,
            vec![ring],
            360.0,
            false,
            crate::model::RevolveMode::NewBody,
        ));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Revolve(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("torus mesh")).abs();
        let expected = std::f32::consts::TAU
            * d
            * std::f32::consts::PI
            * (big_r * big_r - small_r * small_r);
        assert!(
            (vol - expected).abs() / expected < 0.03,
            "hollow torus volume {vol} should be ~{expected} (2π·d·π(R²−r²))",
        );
    }

    /// A 90-degree sweep is a quarter of the ring, symmetric or not.
    #[test]
    fn revolve_partial_sweep_is_proportional_and_symmetric_matches() {
        let expected = std::f32::consts::PI * 300.0 * 10.0 / 4.0;
        for symmetric in [false, true] {
            let (mut doc, sketch) = sketch_doc();
            let profile = rect_profile(&mut doc, sketch, 10.0, 0.0, 10.0, 10.0);
            doc.revolutions.push(test_revolution(
                sketch,
                vec![profile],
                90.0,
                symmetric,
                crate::model::RevolveMode::NewBody,
            ));
            doc.bodies.push(crate::model::Body {
                source: crate::model::BodySource::Revolve(0),
                name: None,
                deleted: false,
                shadow: false,
            });
            let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.extrusions.push(extrusion(sketch, vec![plate], 5.0));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        // Cut tool: a rect profile (x -20..20, y 3..6 in the ground plane) revolved 360
        // degrees around the global X axis — a tube of inner radius 3, outer radius 6,
        // length 40, centered on the X axis. It pierces the plate (z 0..5), so the cut
        // carves a half-buried channel through it.
        let tube = rect_profile(&mut doc, sketch, -20.0, 3.0, 40.0, 3.0);
        doc.revolutions.push(crate::model::Revolution {
            sketch,
            faces: vec![tube],
            axis: crate::model::RevolveAxis::X,
            angle_deg: 360.0,
            symmetric: false,
            mode: crate::model::RevolveMode::Cut(vec![0]),
            name: None,
            deleted: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.construction_planes.push(crate::model::ConstructionPlane {
            origin: Vec3::ZERO,
            normal: Vec3::Y,
            u_axis: Vec3::X,
            v_axis: Vec3::Z,
            parent: crate::model::ConstructionPlaneParent::Root,
            definition: crate::face::default_xy_plane_definition(),
            repeat_instance: None,
            name: None,
            deleted: false,
        });
        doc.add_sketch(FaceId::ConstructionPlane(doc.construction_planes.len() - 1))
    }

    /// #sweep: a 10x10 profile swept along a straight 30mm path normal to its plane
    /// is a plain box — the fallback sweep mesh closes to the exact prism volume.
    #[test]
    fn sweep_straight_path_makes_a_box() {
        let (mut doc, sketch) = sketch_doc();
        let profile = rect_profile(&mut doc, sketch, 0.0, 0.0, 10.0, 10.0);
        let path_sketch = vertical_path_sketch(&mut doc);
        doc.lines.push(Line::from_local_endpoints(path_sketch, 5.0, 0.0, 5.0, 30.0));
        let fp = crate::model::Sweep {
            sketch,
            faces: vec![profile],
            path: vec![doc.lines.len() - 1],
            mode: crate::model::SweepMode::NewBody,
            name: None,
            deleted: false,
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
        doc.lines.push(Line::from_local_endpoints(ps, 0.0, 20.0, 15.0, 20.0));
        doc.lines.push(Line::from_local_endpoints(ps, 0.0, 0.0, 0.0, 20.0));
        let fp = crate::model::Sweep {
            sketch,
            faces: vec![profile],
            path: vec![doc.lines.len() - 2, doc.lines.len() - 1],
            mode: crate::model::SweepMode::NewBody,
            name: None,
            deleted: false,
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
        doc.lines.push(Line::from_local_endpoints(ps, 0.0, 0.0, 0.0, 20.0));
        doc.lines.push(Line::from_local_endpoints(ps, 40.0, 0.0, 40.0, 20.0));
        let fp = crate::model::Sweep {
            sketch,
            faces: vec![profile],
            path: vec![doc.lines.len() - 2, doc.lines.len() - 1],
            mode: crate::model::SweepMode::NewBody,
            name: None,
            deleted: false,
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
        doc.extrusions.push(extrusion(sketch, vec![plate], 5.0));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        // Cut tool: a 4x4 profile swept straight through the plate (z -10..10).
        let bit = rect_profile(&mut doc, sketch, -2.0, -2.0, 4.0, 4.0);
        let ps = vertical_path_sketch(&mut doc);
        doc.lines.push(Line::from_local_endpoints(ps, 0.0, -10.0, 0.0, 10.0));
        doc.sweeps.push(crate::model::Sweep {
            sketch,
            faces: vec![bit],
            path: vec![doc.lines.len() - 1],
            mode: crate::model::SweepMode::Cut(vec![0]),
            name: None,
            deleted: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.extrusions.push(extrusion(sketch, vec![plate], 5.0));
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        // Cut tool: two circles on planes below and above the plate loft into a frustum
        // column punching through it.
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 3.0, 0.0));
        doc.construction_planes.push(crate::construction::plane_from_definition(
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
        let top = doc.add_sketch(FaceId::ConstructionPlane(doc.construction_planes.len() - 1));
        doc.circles.push(Circle::from_local_center_radius(top, 0.0, 0.0, 3.0, 0.0));
        doc.lofts.push(crate::model::Loft {
            sections: vec![
                crate::model::LoftSection { sketch, face: ExtrudeFace::Circle(0) },
                crate::model::LoftSection { sketch: top, face: ExtrudeFace::Circle(1) },
            ],
            mode: crate::model::LoftMode::Cut(vec![0]),
            name: None,
            deleted: false,
        });
        let vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("mesh")).abs();
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
        doc.construction_planes.push(crate::construction::plane_from_face(
            30.0,
            Vec3::ZERO,
            Vec3::Z,
        ));
        let s0 = doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.circles
            .push(crate::model::Circle::from_local_center_radius(s0, -30.0, 0.0, 6.0, 0.0));
        let s1 = doc.add_sketch(crate::model::FaceId::ConstructionPlane(1));
        doc.circles
            .push(crate::model::Circle::from_local_center_radius(s1, -30.0, 0.0, 3.0, 0.0));
        let loft = crate::model::Loft {
            sections: vec![
                crate::model::LoftSection { sketch: s0, face: ExtrudeFace::Circle(0) },
                crate::model::LoftSection { sketch: s1, face: ExtrudeFace::Circle(1) },
            ],
            mode: crate::model::LoftMode::NewBody,
            name: None,
            deleted: false,
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
        let bottom = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles.push(Circle::from_local_center_radius(bottom, 0.0, 0.0, 5.0, 0.0));
        doc.construction_planes.push(crate::construction::plane_from_definition(
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
        let top = doc.add_sketch(FaceId::ConstructionPlane(1));
        doc.circles.push(Circle::from_local_center_radius(top, 0.0, 0.0, 5.0, 0.0));

        let loft = crate::model::Loft {
            sections: vec![
                crate::model::LoftSection { sketch: bottom, face: ExtrudeFace::Circle(0) },
                crate::model::LoftSection { sketch: top, face: ExtrudeFace::Circle(1) },
            ],
            mode: crate::model::LoftMode::NewBody,
            name: None,
            deleted: false,
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let loft = crate::model::Loft {
            sections: vec![crate::model::LoftSection {
                sketch,
                face: ExtrudeFace::Circle(0),
            }],
            mode: crate::model::LoftMode::NewBody,
            name: None,
            deleted: false,
        };
        assert!(loft_mesh(&doc, &loft).is_none());
    }

    /// A picked loft section maps back to the sketch entities that should show a selection
    /// highlight (#202): a circle is itself; a line loop is every line in the loop.
    #[test]
    fn loft_section_scene_elements_covers_circle_and_polygon() {
        use crate::hierarchy::SceneElement;
        let circle = crate::model::LoftSection {
            sketch: 0,
            face: ExtrudeFace::Circle(3),
        };
        assert_eq!(
            loft_section_scene_elements(&circle),
            vec![SceneElement::Circle(3)]
        );
        let polygon = crate::model::LoftSection {
            sketch: 0,
            face: ExtrudeFace::Polygon(vec![4, 5, 6]),
        };
        assert_eq!(
            loft_section_scene_elements(&polygon),
            vec![
                SceneElement::Line(4),
                SceneElement::Line(5),
                SceneElement::Line(6),
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
                0
            } else {
                doc.construction_planes.push(crate::construction::plane_from_definition(
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
                doc.construction_planes.len() - 1
            };
            let sketch = doc.add_sketch(FaceId::ConstructionPlane(plane_idx));
            doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
            sketches.push((i, sketch));
        }
        // Pick order: top (z=10), bottom (z=0), middle (z=5).
        let sections = vec![
            crate::model::LoftSection { sketch: sketches[1].1, face: ExtrudeFace::Circle(1) },
            crate::model::LoftSection { sketch: sketches[0].1, face: ExtrudeFace::Circle(0) },
            crate::model::LoftSection { sketch: sketches[2].1, face: ExtrudeFace::Circle(2) },
        ];
        let ordered = order_loft_sections(&doc, sections);
        let order: Vec<_> = ordered
            .iter()
            .map(|s| match s.face {
                ExtrudeFace::Circle(ci) => ci,
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
            deleted: false,
            edge_treatments: Vec::new(),
        }
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
        doc.extrusions.push(ext);
        let ei = doc.extrusions.len() - 1;
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
        doc.circles.push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let ext = extrusion(sketch, vec![ExtrudeFace::Circle(0)], 20.0);
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
        assert!(face_boundary_loop_world(&doc, &FaceId::ConstructionPlane(0)).is_none());
    }

    #[test]
    fn closed_line_loop_extrudes_to_a_prism_mesh() {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, LineEnd};

        let (mut doc, sketch) = sketch_doc();
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(Line::from_local_endpoints(sketch, 10.0, 0.0, 5.0, 8.0));
        doc.lines.push(Line::from_local_endpoints(sketch, 5.0, 8.0, 0.0, 0.0));
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
        doc.constraints.push(coincident(point(0, LineEnd::End), point(1, LineEnd::Start)));
        doc.constraints.push(coincident(point(1, LineEnd::End), point(2, LineEnd::Start)));
        doc.constraints.push(coincident(point(2, LineEnd::End), point(0, LineEnd::Start)));

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
    fn push_letter_b_outline(doc: &mut Document, sketch: crate::model::SketchId) -> Vec<usize> {
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
        for (a, b, bez) in segs {
            let mut line = Line::from_local_endpoints(sketch, a.0, a.1, b.0, b.1);
            line.bezier = bez;
            doc.lines.push(line);
        }
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        for i in 0..n {
            doc.constraints.push(Constraint {
                sketch,
                kind: ConstraintKind::Coincident {
                    a: ConstraintEntity::Point(point(i, LineEnd::End)),
                    b: ConstraintEntity::Point(point((i + 1) % n, LineEnd::Start)),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
                deleted: false,
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
        let base = doc.lines.len();
        for (p0, p1, bez) in parts {
            let mut line = Line::from_local_endpoints(sketch, p0.0, p0.1, p1.0, p1.1);
            line.bezier = bez;
            doc.lines.push(line);
        }
        let point = |line, end| ConstraintPoint::LineEndpoint { line, end };
        for k in 0..3 {
            doc.constraints.push(Constraint {
                sketch,
                kind: ConstraintKind::Coincident {
                    a: ConstraintEntity::Point(point(base + k, LineEnd::End)),
                    b: ConstraintEntity::Point(point(base + (k + 1) % 3, LineEnd::Start)),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
                deleted: false,
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
        doc.extrusions.push(extrusion(sketch, vec![outer], LETTER_B_DEPTH)); // 0: the B
        doc.extrusions.push(extrusion(sketch, vec![upper], LETTER_B_DEPTH)); // 1: upper cut
        doc.extrusions.push(extrusion(sketch, vec![lower], LETTER_B_DEPTH)); // 2: lower cut
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Solid { add: vec![0], cut: vec![1, 2] },
            name: Some("B".to_string()),
            deleted: false,
            shadow: false,
        });
        let cut_vol = mesh_signed_volume(&body_solid_mesh(&doc, 0).expect("occt B mesh")).abs();

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
        let mesh = body_solid_mesh(&doc, 0).expect("occt cut-body mesh");
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
            .push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let ext = extrusion(sketch, vec![ExtrudeFace::Circle(0)], 8.0);
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
        doc.construction_planes.push(plane_from_definition(
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
        let target_plane = doc.construction_planes.len() - 1;
        doc.circles
            .push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 5.0, 0.0));
        let mut ext = extrusion(sketch, vec![ExtrudeFace::Circle(0)], 40.0);
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
        doc.extrusions.push(ext);
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });

        let edges = treatable_edges(&doc);
        let (expect_ei, expect_edge, a, b) = edges[0].clone();
        let (qa, qb) = (quantize_body_point(a), quantize_body_point(b));

        // Forward and reversed endpoint order both resolve to the same analytic edge.
        assert_eq!(
            treatable_edge_for_selection(&doc, 0, qa, qb),
            Some((expect_ei, expect_edge)),
        );
        assert_eq!(
            treatable_edge_for_selection(&doc, 0, qb, qa),
            Some((expect_ei, expect_edge)),
        );
        // A different body index does not match.
        assert_eq!(treatable_edge_for_selection(&doc, 7, qa, qb), None);
        // An edge that isn't in the analytic list resolves to None.
        assert_eq!(
            treatable_edge_for_selection(&doc, 0, [123456, 0, 0], [123456, 100, 0]),
            None
        );

        // Selection filter: two selected edges (one duplicated via reversal) plus a
        // non-edge element yield exactly the resolved unique edges.
        let mut selection = crate::selection::SceneSelection::default();
        crate::selection::click_scene_selection(
            &mut selection,
            SceneElement::BodyEdge { body: 0, a: qa, b: qb },
            true,
        );
        crate::selection::click_scene_selection(&mut selection, SceneElement::Body(0), true);
        let resolved = treatable_edges_in_selection(&doc, &selection);
        assert_eq!(resolved, vec![(expect_ei, expect_edge)]);
    }

    /// #162: `body_solid_mesh` is memoized on document geometry — an in-place mutation
    /// (no shape_order change, e.g. editing the extrusion distance) must still invalidate
    /// the cache and produce the new solid.
    #[test]
    fn body_mesh_cache_invalidates_on_in_place_geometry_edits() {
        let (mut doc, _sketch, ext) = box_doc();
        doc.extrusions.push(ext);
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Extrusion(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        let before = body_solid_mesh(&doc, 0).expect("box mesh");
        let (_, before_max) = before.bounds().unwrap();
        // Cached call returns the same mesh.
        assert_eq!(body_solid_mesh(&doc, 0).unwrap(), before);

        doc.extrusions[0].distance = 9.0;
        let after = body_solid_mesh(&doc, 0).expect("re-meshed box");
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
        doc.extrusions.push(ext);
        let edges = treatable_edges(&doc);
        // 4 vertical + 4 bottom cap + 4 top cap = 12 for a rectangular profile.
        assert_eq!(edges.len(), 12);
        assert!(edges.iter().all(|(ei, _, _, _)| *ei == 0));

        let (mut cdoc, csketch) = sketch_doc();
        cdoc.circles
            .push(Circle::from_local_center_radius(csketch, 0.0, 0.0, 5.0, 0.0));
        cdoc.extrusions
            .push(extrusion(csketch, vec![ExtrudeFace::Circle(0)], 6.0));
        // Circle profiles have no polygonal edges; their two cap rims are treatable
        // (#177), emitted as chord segments naming Cap { edge: 0 }.
        let circle_edges = treatable_edges(&cdoc);
        assert!(!circle_edges.is_empty());
        assert!(circle_edges
            .iter()
            .all(|(_, e, _, _)| matches!(e, ExtrusionEdgeRef::Cap { edge: 0, .. })));
    }

    #[test]
    fn extrusion_edge_anchor_points_at_edge_midpoint() {
        let (mut doc, _sketch, ext) = box_doc();
        doc.extrusions.push(ext);
        // Vertical edge 0 -> profile vertex 1 = local (10, 0); base z=0, top z=5.
        let (origin, normal) =
            extrusion_edge_anchor(&doc, 0, ExtrusionEdgeRef::Vertical { face: 0, edge: 0 })
                .unwrap();
        assert!((origin - Vec3::new(10.0, 0.0, 2.5)).length() < 1e-3, "{origin:?}");
        assert!(normal.length() > 0.9 && normal.length() < 1.1);

        // A deleted extrusion, an out-of-range extrusion index, and an out-of-range edge index
        // all resolve to `None`.
        doc.extrusions[0].deleted = true;
        assert!(
            extrusion_edge_anchor(&doc, 0, ExtrusionEdgeRef::Vertical { face: 0, edge: 0 })
                .is_none()
        );
        doc.extrusions[0].deleted = false;
        assert!(extrusion_edge_anchor(&doc, 7, ExtrusionEdgeRef::Vertical { face: 0, edge: 0 })
            .is_none());
        assert!(
            extrusion_edge_anchor(&doc, 0, ExtrusionEdgeRef::Vertical { face: 0, edge: 9 })
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
        let (doc, _sketch, mut ext) = box_doc();
        let mut doc = doc;
        doc.extrusions.push(ext.clone());
        assert!(extrusion_edge_exists(&doc, 0, ExtrusionEdgeRef::Vertical { face: 0, edge: 3 }));
        assert!(!extrusion_edge_exists(&doc, 0, ExtrusionEdgeRef::Vertical { face: 0, edge: 4 }));
        assert!(!extrusion_edge_exists(&doc, 5, ExtrusionEdgeRef::Vertical { face: 0, edge: 0 }));
        assert!(!extrusion_edge_exists(&doc, 0, ExtrusionEdgeRef::Vertical { face: 1, edge: 0 }));
        ext.deleted = true;
        doc.extrusions[0] = ext;
        assert!(!extrusion_edge_exists(&doc, 0, ExtrusionEdgeRef::Vertical { face: 0, edge: 0 }));
    }

    #[test]
    fn extrusion_with_edge_treatment_replaces_same_edge_rather_than_stacking() {
        let (doc, _sketch, ext) = box_doc();
        let mut doc = doc;
        doc.extrusions.push(ext);
        let edge = ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        let once = extrusion_with_edge_treatment(
            &doc,
            0,
            EdgeTreatment { edge, kind: VertexTreatmentKind::Chamfer, amount: 1.0 },
        )
        .unwrap();
        doc.extrusions[0] = once;
        let twice = extrusion_with_edge_treatment(
            &doc,
            0,
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
        doc.extrusions.push(ext);
        let edge = ExtrusionEdgeRef::Vertical { face: 0, edge: 0 };
        let small = extrusion_with_edge_treatment(
            &doc,
            0,
            EdgeTreatment { edge, kind: VertexTreatmentKind::Fillet, amount: 2.0 },
        )
        .unwrap();
        assert!(occt_edge_treatments_feasible(&doc, 0, &small));
        let oversized = extrusion_with_edge_treatment(
            &doc,
            0,
            EdgeTreatment { edge, kind: VertexTreatmentKind::Fillet, amount: 500.0 },
        )
        .unwrap();
        assert!(!occt_edge_treatments_feasible(&doc, 0, &oversized));

        // A two-face extrusion is kernel-representable too (each face's prism fused), so
        // the feasibility trial still applies: the oversized fillet is rejected on it.
        let second = rect_profile(&mut doc, sketch, 20.0, 20.0, 10.0, 10.0);
        let extra_face = second.clone();
        doc.extrusions[0].faces.push(extra_face);
        let candidate = extrusion_with_edge_treatment(
            &doc,
            0,
            EdgeTreatment { edge, kind: VertexTreatmentKind::Fillet, amount: 500.0 },
        )
        .unwrap();
        assert!(!occt_edge_treatments_feasible(&doc, 0, &candidate));
    }

    /// #103 part 2: [`kernel_fallback_cut_warning`] fires exactly when a cut-bearing body
    /// can't be built by the kernel (so the additive-only fallback would silently drop the
    /// cuts), and stays quiet for healthy bodies or bodies without cuts.
    #[test]
    fn kernel_fallback_cut_warning_fires_only_for_kernel_infeasible_cut_bodies() {
        let mut doc = cut_body_doc();
        assert_eq!(kernel_fallback_cut_warning(&doc), None, "healthy cut body: no warning");
        doc.extrusions[0].edge_treatments.push(EdgeTreatment {
            edge: ExtrusionEdgeRef::Vertical { face: 0, edge: 0 },
            kind: VertexTreatmentKind::Fillet,
            amount: 500.0,
        });
        let warning = kernel_fallback_cut_warning(&doc).expect("infeasible cut body warns");
        assert!(warning.contains("cuts are not shown"), "{warning}");
        // Without cuts there's nothing to silently drop: no warning even though the body
        // still falls back to the mesh-bevel path.
        doc.bodies[0].source = crate::model::BodySource::Solid { add: vec![0], cut: vec![] };
        assert_eq!(kernel_fallback_cut_warning(&doc), None);
    }
}

