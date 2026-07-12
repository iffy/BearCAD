//! Technical drawings (#180): view projection and vector export, independent of the egui
//! drawing pane so it can be unit-tested and reused for print/PDF output. A drawing renders
//! black-on-white to either **SVG** (prints to PDF via any browser/OS print dialog) or a
//! direct single-page **PDF** — both drive the identical layout through the [`Canvas`] trait,
//! so the two exports never drift.

use crate::model::{Document, DrawingOrientation, DrawingView};
use glam::Vec3;

/// In-plane `(right, up)` world axes a drawing view projects onto: a point `p` maps to
/// `(p·right, p·up)`. The six orthographic directions plus a standard isometric view.
pub fn view_axes(orientation: DrawingOrientation) -> (Vec3, Vec3) {
    use DrawingOrientation as O;
    match orientation {
        O::Front => (Vec3::X, Vec3::Z),
        O::Back => (-Vec3::X, Vec3::Z),
        O::Right => (Vec3::Y, Vec3::Z),
        O::Left => (-Vec3::Y, Vec3::Z),
        O::Top => (Vec3::X, -Vec3::Y),
        O::Bottom => (Vec3::X, Vec3::Y),
        O::Isometric => {
            let out = Vec3::new(1.0, 1.0, 1.0).normalize();
            let right = Vec3::Z.cross(out).normalize();
            let up = out.cross(right).normalize();
            (right, up)
        }
        // A diagonal edge view (#339): the camera looks along the average of its two faces'
        // into-page directions (the 45° bisector), with world +Z as up (Gram-Schmidt'd square to
        // that direction — no cube edge points straight up, so this is always well-defined).
        O::Edge(e) => {
            let (fa, fb) = e.faces();
            let (ra, ua) = view_axes(fa);
            let (rb, ub) = view_axes(fb);
            let out = (ra.cross(ua) + rb.cross(ub)).normalize();
            let up = (Vec3::Z - Vec3::Z.dot(out) * out).normalize();
            let right = up.cross(out).normalize();
            (right, up)
        }
        // A corner three-quarter view (#344): the camera looks along the average of its three
        // faces' into-page directions (the corner's diagonal), world +Z up. No corner points
        // straight up, so the Gram-Schmidt up is always well-defined.
        O::Corner(c) => {
            let (fa, fb, fc) = c.faces();
            let face_out = |f| {
                let (r, u) = view_axes(f);
                r.cross(u)
            };
            let out = (face_out(fa) + face_out(fb) + face_out(fc)).normalize();
            let up = (Vec3::Z - Vec3::Z.dot(out) * out).normalize();
            let right = up.cross(out).normalize();
            (right, up)
        }
        // A free (arbitrary) angle (#345): use the stored basis directly, re-orthonormalised
        // defensively so a slightly-off basis still projects cleanly.
        O::Free { right, up } => {
            let r = Vec3::from_array(right).normalize_or(Vec3::X);
            let u0 = Vec3::from_array(up).normalize_or(Vec3::Z);
            let out = r.cross(u0).normalize_or(Vec3::Y);
            let u = out.cross(r).normalize_or(Vec3::Z);
            (r, u)
        }
    }
}

fn dequant(q: [i32; 3]) -> Vec3 {
    Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) / 100.0
}

/// The orientation of an aligned child projection (#296) placed in `dir` relative to a parent
/// showing `parent`. Derived by the glass-box unfolding: the child shares one of the parent's
/// screen axes and rotates 90° about it. `None` if the result isn't one of the six orthographic
/// views (e.g. the parent is Isometric), so alignment is offered only for orthographic parents.
pub fn aligned_child_orientation(
    parent: DrawingOrientation,
    dir: crate::model::AlignDir,
) -> Option<DrawingOrientation> {
    use crate::model::AlignDir;
    use DrawingOrientation as O;
    // Only the six straight-on views unfold into aligned children; iso/edge/corner parents don't.
    if !matches!(parent, O::Front | O::Back | O::Left | O::Right | O::Top | O::Bottom) {
        return None;
    }
    let (r, u) = view_axes(parent);
    let o = r.cross(u); // "into the page" for this view basis
    let (cr, cu) = match dir {
        AlignDir::Below => (r, -o),
        AlignDir::Above => (r, o),
        AlignDir::Right => (-o, u),
        AlignDir::Left => (o, u),
    };
    // The unfolded child may be a *rotated* face view (e.g. a Top base's Left/Right children),
    // which isn't an axis-aligned canonical orientation (#351). Its true rotated basis comes from
    // `resolved_view_axes`; here we just pick the nearest face by view direction for its label, so
    // all four directions are offerable rather than only the ones that happen to stay canonical.
    orientation_from_axes(cr, cu).or_else(|| nearest_face_by_view_dir(cr.cross(cu)))
}

/// The orthographic orientations an aligned child may take while staying **in line** with its
/// base (#332): rotating the view about the screen axis the two share keeps the alignment intact.
/// A horizontally-placed child (Left/Right) shares the parent's vertical (up) axis, so it can be
/// any view whose up axis matches the parent's (Front/Back/Left/Right for a Front parent); a
/// vertically-placed child (Above/Below) shares the horizontal (right) axis. The parent's own
/// derived child orientation is always included. Empty for a non-orthographic (Isometric) parent.
pub fn aligned_inline_orientations(
    parent: DrawingOrientation,
    dir: crate::model::AlignDir,
) -> Vec<DrawingOrientation> {
    let (pr, pu) = view_axes(parent);
    // Which parent screen axis the child shares depends on the drag direction.
    let shared = if dir.shares_pos_x() { pr } else { pu };
    let axis_matches = |a: Vec3, b: Vec3| a.dot(b).abs() > 0.9;
    DrawingOrientation::ALL
        .iter()
        .copied()
        .filter(|o| !matches!(o, DrawingOrientation::Isometric))
        .filter(|o| {
            let (r, u) = view_axes(*o);
            if dir.shares_pos_x() {
                axis_matches(r, shared)
            } else {
                axis_matches(u, shared)
            }
        })
        .collect()
}

/// The projection basis `(right, up)` a view actually renders with (#357). A non-aligned view uses
/// `view_axes(orientation)`; an **aligned child** uses the glass-box **unfolding** of its parent's
/// basis about their shared screen axis, so it stays lined up *and correctly rotated* for any base
/// orientation — e.g. a Top base yields Front below, Back above, and rotated Left/Right to the
/// sides (#351). `views` is the drawing's view list; the parent is looked up by `aligned_parent`
/// (recursively, so chains stay consistent).
pub fn resolved_view_axes(views: &[DrawingView], view: &DrawingView) -> (Vec3, Vec3) {
    use crate::model::AlignDir;
    if let (Some(p), Some(dir)) = (view.aligned_parent, view.aligned_dir) {
        if let Some(parent) = views.get(p) {
            if !std::ptr::eq(parent, view) {
                let (pr, pu) = resolved_view_axes(views, parent);
                let po = pr.cross(pu); // parent's "into the page"
                return match dir {
                    AlignDir::Below => (pr, -po),
                    AlignDir::Above => (pr, po),
                    AlignDir::Right => (-po, pu),
                    AlignDir::Left => (po, pu),
                };
            }
        }
    }
    view_axes(view.orientation)
}

/// The on-page position of a view (#296), resolving an aligned child's shared axis to its
/// parent's so the two always line up regardless of which was dragged. Non-aligned views (and
/// children whose parent is gone) return their own stored `(pos_x, pos_y)`.
pub fn resolved_view_pos(doc: &Document, drawing: usize, view: usize) -> (f32, f32) {
    let Some(d) = doc.drawings.get(drawing).filter(|d| !d.deleted) else {
        return (0.5, 0.5);
    };
    let Some(v) = d.views.get(view) else {
        return (0.5, 0.5);
    };
    match (v.aligned_parent, v.aligned_dir) {
        (Some(p), Some(dir)) if p != view => {
            if let Some(parent) = d.views.get(p) {
                // Resolve the parent recursively so chains of aligned views stay consistent.
                let (px, py) = resolved_view_pos(doc, drawing, p);
                let _ = parent;
                if dir.shares_pos_x() {
                    return (px, v.pos_y);
                } else {
                    return (v.pos_x, py);
                }
            }
            (v.pos_x, v.pos_y)
        }
        _ => (v.pos_x, v.pos_y),
    }
}

/// A view's effective print scale (#296/#300): an aligned child inherits its parent's scale
/// (walking the chain), so a whole aligned group prints at one scale. Non-aligned views use
/// their own `scale`.
pub fn resolved_view_scale(doc: &Document, drawing: usize, view: usize) -> Option<String> {
    let d = doc.drawings.get(drawing).filter(|d| !d.deleted)?;
    let v = d.views.get(view)?;
    match v.aligned_parent {
        Some(p) if p != view && d.views.get(p).is_some() => {
            resolved_view_scale(doc, drawing, p)
        }
        _ => v.scale.clone(),
    }
}

/// The projected `(right, up)` bounding box of a view's geometry, or `None` if it has none.
fn view_projected_bbox(
    doc: &Document,
    views: &[DrawingView],
    view: usize,
) -> Option<(glam::Vec2, glam::Vec2)> {
    let v = views.get(view)?;
    let world_edges = drawing_view_dimensionable_edges(doc, views, v);
    if world_edges.is_empty() {
        return None;
    }
    let (right, up) = resolved_view_axes(views, v);
    let (mut min, mut max) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for (a, b) in &world_edges {
        for p in [a, b] {
            let pr = glam::Vec2::new(p.dot(right), p.dot(up));
            min = min.min(pr);
            max = max.max(pr);
        }
    }
    Some((min, max))
}

/// A view's own projected extent (size), floored to a tiny positive value per axis.
fn view_projected_extent(doc: &Document, views: &[DrawingView], view: usize) -> glam::Vec2 {
    match view_projected_bbox(doc, views, view) {
        Some((min, max)) => (max - min).max(glam::Vec2::splat(1e-3)),
        None => glam::Vec2::splat(1.0),
    }
}

/// Auto-fit scale for a view within an `area_w`×`area_h` card, filling `fit` of it. An aligned
/// child inherits its parent's auto-fit scale (walking to the aligned root) so a whole aligned
/// group renders at one size — a prerequisite for their edges lining up (#364).
pub fn view_autofit_scale(
    doc: &Document,
    views: &[DrawingView],
    view: usize,
    area_w: f32,
    area_h: f32,
    fit: f32,
) -> f32 {
    if let Some(v) = views.get(view) {
        if let Some(p) = v.aligned_parent {
            if p != view && views.get(p).is_some() {
                return view_autofit_scale(doc, views, p, area_w, area_h, fit);
            }
        }
    }
    let e = view_projected_extent(doc, views, view);
    (area_w / e.x).min(area_h / e.y) * fit
}

/// The bbox center to render a view's geometry about. An aligned child adopts its parent's center
/// along their **shared** projected axis (horizontal for above/below, vertical for left/right) so
/// the part's edges line up across the aligned group, not just the view cards (#364).
pub fn view_render_center(doc: &Document, views: &[DrawingView], view: usize) -> glam::Vec2 {
    let (min, max) =
        view_projected_bbox(doc, views, view).unwrap_or((glam::Vec2::ZERO, glam::Vec2::ZERO));
    let mut center = (min + max) * 0.5;
    if let Some(v) = views.get(view) {
        if let (Some(p), Some(dir)) = (v.aligned_parent, v.aligned_dir) {
            if p != view && views.get(p).is_some() {
                let parent_center = view_render_center(doc, views, p);
                if dir.shares_pos_x() {
                    center.x = parent_center.x;
                } else {
                    center.y = parent_center.y;
                }
            }
        }
    }
    center
}

/// Match a `(right, up)` axis pair back to one of the six orthographic [`DrawingOrientation`]s.
fn orientation_from_axes(right: Vec3, up: Vec3) -> Option<DrawingOrientation> {
    use DrawingOrientation as O;
    const ALL: [O; 6] = [O::Front, O::Back, O::Left, O::Right, O::Top, O::Bottom];
    ALL.into_iter().find(|&o| {
        let (r, u) = view_axes(o);
        (r - right).length() < 1e-3 && (u - up).length() < 1e-3
    })
}

/// The straight-on face whose view direction (into the page) best matches `view_dir` — used to
/// **label** a rotated aligned child (#351) by the face it looks at, when its unfolded basis isn't
/// an axis-aligned canonical orientation.
fn nearest_face_by_view_dir(view_dir: Vec3) -> Option<DrawingOrientation> {
    use DrawingOrientation as O;
    const ALL: [O; 6] = [O::Front, O::Back, O::Left, O::Right, O::Top, O::Bottom];
    ALL.into_iter().max_by(|&a, &b| {
        let dir = |o| {
            let (r, u) = view_axes(o);
            r.cross(u).dot(view_dir)
        };
        dir(a).partial_cmp(&dir(b)).unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn edge_key(a: Vec3, b: Vec3) -> crate::model::DrawingEdgeKey {
    crate::model::normalized_edge_key(
        crate::hierarchy::quantize_body_point(a),
        crate::hierarchy::quantize_body_point(b),
    )
}

/// A circle detected among a view's feature edges (#313), in **world** space so it can be
/// classified once and projected per view: a tessellated curve (cylinder rim, extruded-circle
/// boundary) drawn as one smooth circle (or a foreshortened line when edge-on) with a single
/// diameter dimension rather than a dimension per short segment.
#[derive(Clone, Copy, Debug)]
pub struct WorldCircle {
    pub center: Vec3,
    pub radius: f32,
    /// Unit normal of the circle's plane.
    pub normal: Vec3,
}

/// How a [`WorldCircle`] appears in a particular orthographic view (#313/#319).
pub enum ProjectedCircle {
    /// The circle faces the viewer (roughly): a round outline.
    Round { center: glam::Vec2, radius: f32 },
    /// The circle is (near) edge-on: it projects to a line — the foreshortened diameter.
    EdgeOn { a: glam::Vec2, b: glam::Vec2 },
}

/// Classify a view's world feature edges (#313): find tessellated circles (clean degree-2
/// cycles that fit a planar circle) so the renderers can draw them smooth and dimension only
/// the diameter, in any orientation. Straight edges are everything else.
pub fn classify_world_circles(edges: &[(Vec3, Vec3)]) -> Vec<WorldCircle> {
    use std::collections::HashMap;
    // Quantize endpoints (0.01 mm) so shared vertices merge into one index.
    let q = |p: Vec3| {
        (
            (p.x * 100.0).round() as i64,
            (p.y * 100.0).round() as i64,
            (p.z * 100.0).round() as i64,
        )
    };
    let mut index_of: HashMap<(i64, i64, i64), usize> = HashMap::new();
    let mut verts: Vec<Vec3> = Vec::new();
    let mut vid = |p: Vec3| {
        *index_of.entry(q(p)).or_insert_with(|| {
            verts.push(p);
            verts.len() - 1
        })
    };
    let mut e_verts: Vec<(usize, usize)> = Vec::with_capacity(edges.len());
    let mut seen_pairs: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for &(a, b) in edges {
        let (ia, ib) = (vid(a), vid(b));
        let pair = if ia <= ib { (ia, ib) } else { (ib, ia) };
        if ia != ib && !seen_pairs.insert(pair) {
            continue;
        }
        e_verts.push((ia, ib));
    }
    let n = verts.len();
    let mut degree = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (ei, &(a, b)) in e_verts.iter().enumerate() {
        if a == b {
            continue;
        }
        degree[a] += 1;
        degree[b] += 1;
        adj[a].push(ei);
        adj[b].push(ei);
    }
    let mut seen = vec![false; e_verts.len()];
    let mut circles = Vec::new();
    for start in 0..e_verts.len() {
        if seen[start] || e_verts[start].0 == e_verts[start].1 {
            continue;
        }
        let mut stack = vec![start];
        let mut comp_edges = Vec::new();
        let mut comp_verts: Vec<usize> = Vec::new();
        let mut clean = true;
        seen[start] = true;
        while let Some(ei) = stack.pop() {
            comp_edges.push(ei);
            for &v in [e_verts[ei].0, e_verts[ei].1].iter() {
                if degree[v] != 2 {
                    clean = false;
                }
                comp_verts.push(v);
                for &ne in &adj[v] {
                    if !seen[ne] {
                        seen[ne] = true;
                        stack.push(ne);
                    }
                }
            }
        }
        comp_verts.sort_unstable();
        comp_verts.dedup();
        if !clean || comp_edges.len() < 8 || comp_verts.len() != comp_edges.len() {
            continue;
        }
        let center =
            comp_verts.iter().map(|&v| verts[v]).sum::<Vec3>() / comp_verts.len() as f32;
        let radii: Vec<f32> = comp_verts.iter().map(|&v| (verts[v] - center).length()).collect();
        let mean_r = radii.iter().sum::<f32>() / radii.len() as f32;
        if mean_r < 1e-2 {
            continue;
        }
        let max_dev = radii.iter().map(|r| (r - mean_r).abs()).fold(0.0f32, f32::max);
        if max_dev > mean_r * 0.06 {
            continue;
        }
        // Plane normal from the summed fan cross products (consistent for a planar loop).
        let mut normal = Vec3::ZERO;
        for w in comp_verts.windows(2) {
            normal += (verts[w[0]] - center).cross(verts[w[1]] - center);
        }
        let normal = normal.normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
        // Require coplanarity (all vertices near the plane).
        let coplanar = comp_verts
            .iter()
            .all(|&v| (verts[v] - center).dot(normal).abs() <= mean_r * 0.06);
        if coplanar {
            circles.push(WorldCircle { center, radius: mean_r, normal });
        }
    }
    circles
}

/// Project a world circle into a view's 2D space (#313/#319): round when it faces the viewer,
/// a foreshortened line when edge-on.
pub fn project_world_circle(c: &WorldCircle, right: Vec3, up: Vec3) -> ProjectedCircle {
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    let c2 = project(c.center);
    // Two orthonormal in-plane axes.
    let seed = if c.normal.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = (seed - c.normal * seed.dot(c.normal)).normalize();
    let v = c.normal.cross(u);
    let pu = project(c.center + u * c.radius) - c2;
    let pv = project(c.center + v * c.radius) - c2;
    let (major, minor) = if pu.length() >= pv.length() { (pu, pv) } else { (pv, pu) };
    if minor.length() < 0.15 * c.radius {
        ProjectedCircle::EdgeOn { a: c2 - major, b: c2 + major }
    } else {
        ProjectedCircle::Round { center: c2, radius: major.length() }
    }
}

/// Whether a projected 2D segment lies on one of the projected circles (#313), so it's drawn as
/// part of the smooth circle/edge-on line instead of a straight stroke or dimension.
pub fn projected_segment_on_circle(a: glam::Vec2, b: glam::Vec2, pcs: &[ProjectedCircle]) -> bool {
    pcs.iter().any(|pc| match pc {
        ProjectedCircle::Round { center, radius } => {
            let tol = radius * 0.08 + 1e-2;
            ((a - *center).length() - radius).abs() < tol
                && ((b - *center).length() - radius).abs() < tol
        }
        ProjectedCircle::EdgeOn { a: la, b: lb } => {
            let d = *lb - *la;
            let len2 = d.length_squared().max(1e-6);
            let tol = d.length() * 0.08 + 1e-2;
            let on = |p: glam::Vec2| {
                let t = ((p - *la).dot(d) / len2).clamp(0.0, 1.0);
                (p - (*la + d * t)).length() < tol
            };
            on(a) && on(b)
        }
    })
}

/// PDF points per millimetre (1 pt = 1/72 in): exports are sized in points so the PDF page
/// physically matches the drawing's configured mm page (#298).
const PT_PER_MM: f32 = 72.0 / 25.4;
/// A placed view card's size as a fraction of the page — the same 0.42 the editor uses, so the
/// export lays out where the editor showed it (#297).
const CELL_FRAC: f32 = 0.42;
/// Padding inside a view card between its border and the projected geometry.
const CELL_PAD: f32 = 12.0;

/// Stroke width for the model's projected edges and detected circles (#327). Kept clearly
/// heavier than the dimension/extension lines so the part outline reads as the primary geometry.
pub const MODEL_STROKE: f32 = 1.6;
/// Stroke width for dimension lines, their extension lines, and diameter lines (#327) — thinner
/// than [`MODEL_STROKE`] so annotations sit visually beneath the model outline.
pub const DIM_STROKE: f32 = 0.6;

/// The world-space feature edges a drawing view projects (#278): a body's solid-mesh unique
/// edges, or — when the view's `sketch` is set — that sketch's line/circle geometry. Shared by
/// the editor pane and the SVG/PDF export so both draw the same thing.
pub fn drawing_view_world_edges(doc: &Document, view: &DrawingView) -> Vec<(Vec3, Vec3)> {
    if let Some(si) = view.sketch {
        let mut edges = Vec::new();
        for line in doc.lines.iter().filter(|l| !l.deleted && l.sketch == si) {
            if let Some(pts) = crate::face::line_world_polyline(doc, line) {
                for w in pts.windows(2) {
                    edges.push((w[0], w[1]));
                }
            }
        }
        for circle in doc.circles.iter().filter(|c| !c.deleted && c.sketch == si) {
            if let Some(pts) = crate::face::circle_world_perimeter(doc, circle, 48) {
                for w in pts.windows(2) {
                    edges.push((w[0], w[1]));
                }
            }
        }
        edges
    } else {
        // Crease/feature edges only — the view-dependent silhouette (#319) is added later, in
        // the stroke geometry, so it doesn't interfere with circle detection (#313).
        crate::extrude::body_solid_mesh(doc, view.body)
            .map(|mesh| crate::gpu_viewport::solid_mesh_unique_edges(&mesh))
            .unwrap_or_default()
    }
}

/// The view-dependent silhouette edges of a body view (#319): a cylinder's straight sides and
/// other smooth-surface outlines that aren't crease edges. Empty for sketch views.
pub fn drawing_view_silhouette_edges(
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
) -> Vec<(Vec3, Vec3)> {
    if view.sketch.is_some() {
        return Vec::new();
    }
    let Some(mesh) = crate::extrude::body_solid_mesh(doc, view.body) else {
        return Vec::new();
    };
    let (right, up) = resolved_view_axes(views, view);
    crate::gpu_viewport::solid_mesh_silhouette_edges(&mesh, right.cross(up))
}

/// The edges a view can dimension (#334): its crease/feature edges plus the view-dependent
/// silhouette edges (a cylinder's straight sides), so the **length** of a smooth extrusion — which
/// has no crease edge down its side — can be dimensioned like any straight edge. Silhouette edges
/// are deduped against the crease set by quantized endpoints. Circle detection deliberately stays
/// on the crease-only [`drawing_view_world_edges`] (#319), so this is used only for dimensioning.
pub fn drawing_view_dimensionable_edges(
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
) -> Vec<(Vec3, Vec3)> {
    let mut edges = drawing_view_world_edges(doc, view);
    let mut seen: std::collections::HashSet<crate::model::DrawingEdgeKey> = edges
        .iter()
        .map(|(a, b)| {
            crate::model::normalized_edge_key(
                crate::hierarchy::quantize_body_point(*a),
                crate::hierarchy::quantize_body_point(*b),
            )
        })
        .collect();
    for (a, b) in drawing_view_silhouette_edges(doc, views, view) {
        let key = crate::model::normalized_edge_key(
            crate::hierarchy::quantize_body_point(a),
            crate::hierarchy::quantize_body_point(b),
        );
        if seen.insert(key) {
            edges.push((a, b));
        }
    }
    edges
}

/// The architectural dimension-line geometry for one edge (#294), all in the view's projected
/// 2D mm space. `a`/`b` are the edge endpoints; `outward` is the unit perpendicular pointing
/// away from the geometry centroid; `offset` is how far out along `outward` the dimension line
/// sits. Both the editor and the exports build their strokes from this so they never drift.
pub struct DimLineGeometry {
    /// The two extension lines (edge endpoint → just past the dimension line).
    pub extensions: [(glam::Vec2, glam::Vec2); 2],
    /// The dimension line itself (endpoint to endpoint, parallel to the edge).
    pub line: (glam::Vec2, glam::Vec2),
    /// Two arrowhead triangles (three points each) at the dimension line's ends.
    pub arrows: [[glam::Vec2; 3]; 2],
}

/// Build [`DimLineGeometry`] for an edge from `a` to `b`, offset `outward * offset` from it.
/// `arrow` is the arrowhead length in the same units, so callers can size features to the
/// drawing (a proportional fraction of the projected extent keeps them readable at any scale).
pub fn dimension_line_geometry(
    a: glam::Vec2,
    b: glam::Vec2,
    outward: glam::Vec2,
    offset: f32,
    arrow: f32,
) -> DimLineGeometry {
    let da = a + outward * offset;
    let db = b + outward * offset;
    let along = (db - da).normalize_or_zero();
    // Arrowheads point outward from the line centre toward each end.
    let head = |tip: glam::Vec2, dir: glam::Vec2| {
        let base = tip - dir * arrow;
        let side = glam::Vec2::new(-dir.y, dir.x) * (arrow * 0.4);
        [tip, base + side, base - side]
    };
    DimLineGeometry {
        // Extension lines start a hair off the edge and overshoot the dimension line a touch.
        extensions: [
            (a + outward * (arrow * 0.4), da + outward * (arrow * 0.7)),
            (b + outward * (arrow * 0.4), db + outward * (arrow * 0.7)),
        ],
        line: (da, db),
        arrows: [head(da, -along), head(db, along)],
    }
}

/// Plan per-dimension **extra offsets** (beyond the default gap) so dimension lines and their
/// number labels don't overlap each other (#321): parallel dimensions whose lines would land at
/// the same distance and whose spans overlap are pushed out onto successive "tiers", the way CAD
/// stacks parallel dimensions. Input is one `(a, b, outward)` per dimension in projected mm;
/// output is the extra offset for each, in the same order. Greedy interval colouring per
/// parallel group, longest-span dimensions taking the innermost tier.
pub fn plan_dimension_tiers(dims: &[(glam::Vec2, glam::Vec2, glam::Vec2)], gap: f32) -> Vec<f32> {
    let n = dims.len();
    // Per-dimension: line direction, span [s0,s1] along it, and the signed distance of the
    // dimension line from the origin along `outward` (its "height", so parallel lines at the
    // same height on the same side are the ones that can collide).
    struct Info {
        dir: glam::Vec2,
        outward: glam::Vec2,
        s0: f32,
        s1: f32,
        height: f32,
        len: f32,
    }
    let info: Vec<Info> = dims
        .iter()
        .map(|&(a, b, outward)| {
            let seg = b - a;
            let len = seg.length().max(1e-6);
            let dir = seg / len;
            let s0 = a.dot(dir);
            let s1 = b.dot(dir);
            let (s0, s1) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
            // Dimension line sits at the edge midpoint pushed out by the default gap.
            let mid = (a + b) * 0.5 + outward * gap;
            Info { dir, outward, s0, s1, height: mid.dot(outward), len }
        })
        .collect();

    // Process longest first so big datums stay innermost; assign the lowest free tier.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| info[j].len.total_cmp(&info[i].len));
    let mut tier = vec![0usize; n];
    let mut placed: Vec<usize> = Vec::new(); // indices already assigned
    for &i in &order {
        let mut t = 0;
        'search: loop {
            for &j in &placed {
                if tier[j] != t {
                    continue;
                }
                // Same tier: collide if parallel, same side, near the same height, spans overlap.
                let parallel = info[i].dir.dot(info[j].dir).abs() > 0.99;
                let same_side = info[i].outward.dot(info[j].outward) > 0.9;
                let same_height = (info[i].height - info[j].height).abs() < gap * 0.5;
                let overlap = info[i].s0 < info[j].s1 - 1e-3 && info[j].s0 < info[i].s1 - 1e-3;
                if parallel && same_side && same_height && overlap {
                    t += 1;
                    continue 'search;
                }
            }
            break;
        }
        tier[i] = t;
        placed.push(i);
    }
    // Each tier steps out by ~1.4 gaps so a label on the inner line clears the outer one.
    tier.iter().map(|&t| t as f32 * gap * 1.4).collect()
}

/// The rotation (radians, clockwise in screen space) that makes a label along direction `dir`
/// always read **left-to-right or bottom-to-top** (#322): the angle is normalized into
/// `[-90°, 90°)`, so a downward vertical reads upward (−90°) rather than top-to-bottom, and a
/// down-to-the-right slope reads top-left → bottom-right.
pub fn readable_text_angle(dir: glam::Vec2) -> f32 {
    let mut angle = dir.y.atan2(dir.x);
    while angle >= std::f32::consts::FRAC_PI_2 {
        angle -= std::f32::consts::PI;
    }
    while angle < -std::f32::consts::FRAC_PI_2 {
        angle += std::f32::consts::PI;
    }
    angle
}

/// The outward unit perpendicular for an edge's dimension line: the side of the edge facing
/// away from the geometry centroid `center` (#294), so labels sit outside the part.
pub fn dimension_outward(a: glam::Vec2, b: glam::Vec2, center: glam::Vec2) -> glam::Vec2 {
    let seg = b - a;
    let mut perp = glam::Vec2::new(-seg.y, seg.x).normalize_or_zero();
    if perp == glam::Vec2::ZERO {
        perp = glam::Vec2::new(0.0, -1.0);
    }
    let mid = (a + b) * 0.5;
    if perp.dot(mid - center) < 0.0 {
        perp = -perp;
    }
    perp
}

/// Projected 2D geometry for a drawing view under its display style (#301), shared by the
/// editor pane and the SVG/PDF export.
pub struct StyledViewGeometry {
    /// Back-to-front shaded triangles (projected 2D + a 0..1 grey, 1 = white) — `Shaded` only.
    pub tris: Vec<([glam::Vec2; 3], f32)>,
    /// The edge segments to stroke: every feature edge for `Wireframe`; only the visible
    /// runs (hidden lines removed) for `Visible`/`Shaded`.
    pub segments: Vec<(glam::Vec2, glam::Vec2)>,
}

/// Project a view's geometry under its display style (#301). Sketch views have no solid to
/// occlude or shade, so they always render as plain wireframe.
pub fn styled_view_geometry(
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
) -> StyledViewGeometry {
    use crate::model::DrawingViewStyle;
    let (right, up) = resolved_view_axes(views, view);
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    // Crease edges plus the view-dependent silhouette (#319) so smooth-surface outlines (a
    // cylinder's straight sides) are stroked; circle detection/dimensioning use crease edges
    // only, so the silhouette here doesn't affect them.
    let mut edges = drawing_view_world_edges(doc, view);
    edges.extend(drawing_view_silhouette_edges(doc, views, view));
    let wireframe = || StyledViewGeometry {
        tris: Vec::new(),
        segments: edges.iter().map(|(a, b)| (project(*a), project(*b))).collect(),
    };
    if view.sketch.is_some() || view.style == DrawingViewStyle::Wireframe {
        return wireframe();
    }
    let Some(mesh) = crate::extrude::body_solid_mesh(doc, view.body) else {
        return wireframe();
    };
    // Depth grows toward the viewer along the view's out-of-page axis.
    let toward = right.cross(up);
    let Some((lo, hi)) = mesh.bounds() else {
        return wireframe();
    };
    let eps = (hi - lo).length().max(1e-3) * 2e-3;

    // Projected triangles with per-vertex depth, for point-occlusion tests.
    struct ProjTri {
        p: [glam::Vec2; 3],
        d: [f32; 3],
        /// Twice the signed area of the projected triangle; ~0 = edge-on, skipped.
        area2: f32,
    }
    let tris: Vec<ProjTri> = mesh
        .triangles
        .iter()
        .map(|t| {
            let p = [project(t[0]), project(t[1]), project(t[2])];
            let area2 = (p[1] - p[0]).perp_dot(p[2] - p[0]);
            ProjTri { p, d: [t[0].dot(toward), t[1].dot(toward), t[2].dot(toward)], area2 }
        })
        .filter(|t| t.area2.abs() > 1e-6)
        .collect();
    // Whether some face is strictly in front of `(point, depth)`.
    let occluded = |point: glam::Vec2, depth: f32| -> bool {
        tris.iter().any(|t| {
            // Barycentric coordinates of `point` in the projected triangle.
            let w0 = (t.p[1] - point).perp_dot(t.p[2] - point) / t.area2;
            let w1 = (t.p[2] - point).perp_dot(t.p[0] - point) / t.area2;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                return false;
            }
            w0 * t.d[0] + w1 * t.d[1] + w2 * t.d[2] > depth + eps
        })
    };

    // Sample each edge and keep the visible runs (hidden-line removal).
    const SAMPLES: usize = 32;
    let mut segments = Vec::new();
    for (a, b) in &edges {
        let mut run_start: Option<f32> = None;
        let mut push_run = |from: f32, to: f32| {
            let wa = a.lerp(*b, from);
            let wb = a.lerp(*b, to);
            segments.push((project(wa), project(wb)));
        };
        for i in 0..SAMPLES {
            let t = (i as f32 + 0.5) / SAMPLES as f32;
            let w = a.lerp(*b, t);
            let visible = !occluded(project(w), w.dot(toward));
            match (visible, run_start) {
                (true, None) => run_start = Some(i as f32 / SAMPLES as f32),
                (false, Some(s)) => {
                    push_run(s, i as f32 / SAMPLES as f32);
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = run_start {
            push_run(s, 1.0);
        }
    }

    // Shaded fills: front faces painted back-to-front, greyed by how squarely they face a
    // fixed key light up-and-left of the viewer.
    let mut fills = Vec::new();
    if view.style == DrawingViewStyle::Shaded {
        let light = (toward * 1.2 - right * 0.35 + up * 0.55).normalize();
        let mut shaded: Vec<(f32, [glam::Vec2; 3], f32)> = mesh
            .triangles
            .iter()
            .filter_map(|t| {
                let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                if n == Vec3::ZERO || n.dot(toward) <= 0.0 {
                    return None; // back or degenerate face
                }
                let shade = 0.62 + 0.33 * n.dot(light).max(0.0);
                let depth = (t[0] + t[1] + t[2]).dot(toward) / 3.0;
                Some((depth, [project(t[0]), project(t[1]), project(t[2])], shade))
            })
            .collect();
        shaded.sort_by(|a, b| a.0.total_cmp(&b.0));
        fills = shaded.into_iter().map(|(_, p, s)| (p, s)).collect();
    }

    StyledViewGeometry { tris: fills, segments }
}

/// An 8-bit RGB paint.
#[derive(Clone, Copy, PartialEq)]
struct Rgb(u8, u8, u8);
const BLACK: Rgb = Rgb(0, 0, 0);
const WHITE: Rgb = Rgb(255, 255, 255);

/// Horizontal text alignment relative to the given `x`.
#[derive(Clone, Copy)]
enum Anchor {
    Start,
    Middle,
}

/// A 2D vector-drawing sink in top-left (SVG-style) coordinates: `y` grows downward and text
/// `y` is the baseline. The PDF backend flips to bottom-left internally. Both backends render
/// the same [`render_drawing`] output.
trait Canvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32);
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32);
    /// A filled polygon (shaded view faces, #301).
    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb);
    /// A stroked (unfilled) circle outline — a smooth detected curve (#313).
    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32);
    /// Text is always black in a drawing; `size` is the font size in px.
    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str);
    /// Text rotated `angle` radians clockwise about `(x, y)` — dimension labels running along
    /// their dimension line (#314). Backends override; the default draws it unrotated.
    fn text_rot(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str, angle: f32) {
        let _ = angle;
        self.text(x, y, size, anchor, content);
    }
}

/// Approximate rendered width (device units) of a dimension label in the drawing's Helvetica —
/// ~0.55 em per glyph, matching the PDF backend's centring estimate (#314).
pub fn text_device_width(size: f32, content: &str) -> f32 {
    0.55 * size * content.chars().count() as f32
}

/// Where and how to draw a dimension's label (#314): `(pos, angle_radians)`. If the text fits
/// along the dimension line it runs centred along it (rotated, kept upright); otherwise it's
/// placed just beyond the line's far end, horizontal, so it never overlaps the arrows.
/// Everything is in device units (screen px for the editor, points for export).
pub fn dimension_label_layout(
    a: glam::Vec2,
    b: glam::Vec2,
    outward: glam::Vec2,
    text_w: f32,
    gap: f32,
) -> (glam::Vec2, f32) {
    let along = b - a;
    let len = along.length();
    let dir = if len > 1e-3 { along / len } else { glam::Vec2::new(1.0, 0.0) };
    let mid = (a + b) * 0.5;
    if text_w + gap <= len {
        (mid + outward * gap, readable_text_angle(dir))
    } else {
        // Too short: sit horizontally just past the far end, on the outward side.
        (b + dir * (text_w * 0.5 + gap) + outward * gap, 0.0)
    }
}

/// The page size (width, height) in PDF points for a drawing — its configured mm page (#298),
/// landscape US-Letter by default — or `None` if the index is missing/deleted.
fn page_dims(doc: &Document, index: usize) -> Option<(f32, f32)> {
    let drawing = doc.drawings.get(index).filter(|d| !d.deleted)?;
    Some((
        drawing.page_width_mm * PT_PER_MM,
        drawing.page_height_mm * PT_PER_MM,
    ))
}

/// Draw a whole drawing into `canvas`, WYSIWYG with the editor (#297): each view is a card
/// centred at its `pos_x`/`pos_y` page fraction, sized like the editor's cards, on the
/// drawing's configured page. The title sits in the top margin.
fn render_drawing<C: Canvas>(doc: &Document, index: usize, canvas: &mut C) -> Option<()> {
    let drawing = doc.drawings.get(index).filter(|d| !d.deleted)?;
    let (width, height) = page_dims(doc, index)?;
    let unit = doc.default_length_unit;

    canvas.rect(0.0, 0.0, width, height, Some(WHITE), None, 0.0);
    // The title is a normal, deletable text annotation created with the drawing (#335), rendered
    // in the annotation loop below just like any other note — the export no longer stamps its own
    // title into the top margin (that never appeared in the WYSIWYG editor).

    let cell_w = width * CELL_FRAC;
    let cell_h = height * CELL_FRAC;
    for (vi, view) in drawing.views.iter().enumerate() {
        // Aligned children (#296) resolve their shared axis to the parent's.
        let (px, py) = resolved_view_pos(doc, index, vi);
        let cell_x = px * width - cell_w * 0.5;
        let cell_y = py * height - cell_h * 0.5;
        // No card border in exports (#337): the grey rectangle is an editor-only affordance for
        // selecting/dragging a view; a printed drawing shows just the projection and its caption.
        let source = match view.sketch {
            Some(si) => crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Sketch(si)),
            None => crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Body(view.body)),
        };
        // An aligned child inherits its parent's scale (#296/#300).
        let scale_text = resolved_view_scale(doc, index, vi);
        let scale_suffix = scale_text
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let label = format!("{source} — {}{scale_suffix}", view.orientation.label());
        canvas.text(cell_x + CELL_PAD, cell_y + 20.0, 11.0, Anchor::Start, &label);
        render_view_geometry(
            canvas,
            doc,
            &drawing.views,
            view,
            vi,
            scale_text.as_deref(),
            cell_x,
            cell_y,
            cell_w,
            cell_h,
            unit,
        );
    }

    // Free text annotations (#312): wrapped to their box, positioned by page fraction.
    for ann in &drawing.annotations {
        if ann.deleted {
            continue;
        }
        let font = (ann.size_frac * height).clamp(4.0, 400.0);
        let x = ann.pos_x * width;
        let y = ann.pos_y * height + font; // baseline of the first line
        let wrap = ann.wrap_frac.map(|w| (w * width).max(font));
        let line_h = font * 1.25;
        // Substitute {expr} variable fields against the document's parameters (#338).
        let rendered = crate::value::interpolate_text(&ann.text, doc);
        for (i, line) in wrap_text_lines(&rendered, font, wrap).iter().enumerate() {
            canvas.text(x, y + i as f32 * line_h, font, Anchor::Start, line);
        }
    }
    Some(())
}

/// Word-wrap `text` to `wrap_width` device units (`None` = no wrap), splitting on explicit
/// newlines too (#312). Uses the same ~0.55em glyph estimate as the PDF centring.
fn wrap_text_lines(text: &str, font: f32, wrap_width: Option<f32>) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        match wrap_width {
            None => out.push(para.to_string()),
            Some(w) => {
                let mut line = String::new();
                for word in para.split(' ') {
                    let candidate = if line.is_empty() {
                        word.to_string()
                    } else {
                        format!("{line} {word}")
                    };
                    if !line.is_empty() && text_device_width(font, &candidate) > w {
                        out.push(std::mem::take(&mut line));
                        line = word.to_string();
                    } else {
                        line = candidate;
                    }
                }
                out.push(line);
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_view_geometry<C: Canvas>(
    canvas: &mut C,
    doc: &Document,
    views: &[DrawingView],
    view: &DrawingView,
    view_index: usize,
    scale_text: Option<&str>,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
    unit: crate::value::LengthUnit,
) {
    // Crease edges drive circle detection (#319); the dimensionable set also carries silhouette
    // edges so a smooth extrusion's length can be dimensioned (#334).
    let crease_edges = drawing_view_world_edges(doc, view);
    let world_edges = drawing_view_dimensionable_edges(doc, views, view);
    if world_edges.is_empty() {
        return;
    }
    let (right, up) = resolved_view_axes(views, view);
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    let proj: Vec<(glam::Vec2, glam::Vec2)> = world_edges
        .iter()
        .map(|(a, b)| (project(*a), project(*b)))
        .collect();

    let (mut min, mut max) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for (a, b) in &proj {
        min = min.min(*a).min(*b);
        max = max.max(*a).max(*b);
    }
    let extent = (max - min).max(glam::Vec2::splat(1e-3));
    // The caption strip takes the top of the card; geometry fits below it.
    let caption_h = 26.0;
    let area_w = cell_w - 2.0 * CELL_PAD;
    let area_h = cell_h - caption_h - 2.0 * CELL_PAD;
    // A set print scale (#300) draws at exactly `factor` page-mm per model-mm (points on the
    // export canvas); otherwise auto-fit to the card.
    let _ = extent;
    let scale = match scale_text.and_then(crate::model::parse_drawing_scale) {
        Some(factor) => factor * PT_PER_MM,
        // Aligned children share their parent's auto-fit scale so edges line up (#364).
        None => view_autofit_scale(doc, views, view_index, area_w, area_h, 0.9),
    };
    // Aligned children align to their parent along the shared edge (#364), not just their card.
    let bbox_center = view_render_center(doc, views, view_index);
    let area_center =
        glam::Vec2::new(cell_x + cell_w * 0.5, cell_y + caption_h + CELL_PAD + area_h * 0.5);
    // Model +up maps to screen -y (y grows downward).
    let to_screen = |p: glam::Vec2| {
        let d = (p - bbox_center) * scale;
        glam::Vec2::new(area_center.x + d.x, area_center.y - d.y)
    };

    // Detect tessellated circles (#313) in world space and project them for this view: round
    // when face-on, a foreshortened line when edge-on (#319). Their segments are drawn as the
    // smooth circle/line and dimensioned once (the diameter), not per short segment.
    let world_circles = classify_world_circles(&crease_edges);
    let pcircles: Vec<ProjectedCircle> = world_circles
        .iter()
        .map(|c| project_world_circle(c, right, up))
        .collect();

    // Strokes (and shaded fills) come from the view's display style (#301); the fit above
    // always uses the full wireframe bbox so switching styles never re-scales the view.
    let styled = styled_view_geometry(doc, views, view);
    for (pts, shade) in &styled.tris {
        let level = (shade.clamp(0.0, 1.0) * 255.0) as u8;
        let fill = Rgb(level, level, level);
        let s: Vec<(f32, f32)> = pts
            .iter()
            .map(|p| {
                let sp = to_screen(*p);
                (sp.x, sp.y)
            })
            .collect();
        canvas.poly(&s, fill);
    }
    for (a, b) in &styled.segments {
        // A segment lying on a detected circle is drawn as part of the smooth circle instead.
        if projected_segment_on_circle(*a, *b, &pcircles) {
            continue;
        }
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, MODEL_STROKE);
    }
    // Smooth detected circles (round) or their foreshortened diameter line (edge-on).
    for pc in &pcircles {
        match pc {
            ProjectedCircle::Round { center, radius } => {
                let sc = to_screen(*center);
                canvas.circle(sc.x, sc.y, radius * scale, BLACK, MODEL_STROKE);
            }
            ProjectedCircle::EdgeOn { a, b } => {
                let (sa, sb) = (to_screen(*a), to_screen(*b));
                canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, MODEL_STROKE);
            }
        }
    }

    // Length dimensions (#294): architectural dimension lines — extension lines, an offset
    // dimension line with arrowheads, and the measured length centred on it. Sizes are a
    // fraction of the projected extent so they read at any scale; a per-edge override
    // (dimension_offsets) pushes the line further out.
    let diag = extent.length().max(1.0);
    let default_gap = diag * 0.05;
    let arrow = diag * 0.025;
    // A single diameter dimension per detected circle (#313), replacing its segments' dims — but
    // only for circles whose diameter is shown (#342), so Show/Hide all controls them too.
    for (wc, pc) in world_circles.iter().zip(&pcircles) {
        if !view
            .dimensioned_circles
            .contains(&crate::hierarchy::quantize_body_point(wc.center))
        {
            continue;
        }
        let label = format!("Ø{}", crate::value::format_length_display_in(wc.radius * 2.0, unit));
        match pc {
            // Face-on: a diameter line across the circle with the value beside it.
            ProjectedCircle::Round { center, radius } => {
                let dir = glam::Vec2::new(0.70710677, -0.70710677);
                let (a, b) = (*center - dir * *radius, *center + dir * *radius);
                let (sa, sb) = (to_screen(a), to_screen(b));
                canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, DIM_STROKE);
                let mid = (sa + sb) * 0.5;
                canvas.text_rot(mid.x, mid.y, 11.0, Anchor::Middle, &label, readable_text_angle(sb - sa));
            }
            // Edge-on (looks like a line, #320): a normal linear dimension — extension lines,
            // an offset dimension line with arrowheads, and the value running along it.
            ProjectedCircle::EdgeOn { a, b } => {
                let outward = dimension_outward(*a, *b, bbox_center);
                let geom = dimension_line_geometry(*a, *b, outward, default_gap, arrow);
                let sl = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
                    let (sp, sq) = (to_screen(p), to_screen(q));
                    canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
                };
                for (p, q) in geom.extensions {
                    sl(canvas, p, q);
                }
                sl(canvas, geom.line.0, geom.line.1);
                for tri in geom.arrows {
                    let pts: Vec<(f32, f32)> =
                        tri.iter().map(|p| { let s = to_screen(*p); (s.x, s.y) }).collect();
                    canvas.poly(&pts, BLACK);
                }
                let (sla, slb) = (to_screen(geom.line.0), to_screen(geom.line.1));
                let out_screen =
                    (to_screen(geom.line.0 + outward) - to_screen(geom.line.0)).normalize_or_zero();
                let (lp, ang) = dimension_label_layout(sla, slb, out_screen, text_device_width(11.0, &label), 5.0);
                canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, ang);
            }
        }
    }
    for (i, (a, b)) in proj.iter().enumerate() {
        let (wa, wb) = world_edges[i];
        let key = edge_key(wa, wb);
        // An edge-on edge projects to a point — nothing meaningful to dimension here (#294) —
        // and circle segments are covered by the single diameter dimension above (#313).
        if !view.dimensioned_edges.contains(&key)
            || (*b - *a).length() < 1e-3
            || projected_segment_on_circle(*a, *b, &pcircles)
        {
            continue;
        }
        let outward = dimension_outward(*a, *b, bbox_center);
        let extra = view
            .dimension_offsets
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, o)| *o)
            .unwrap_or(0.0);
        let geom = dimension_line_geometry(*a, *b, outward, default_gap + extra, arrow);
        let stroke_line = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
            let (sp, sq) = (to_screen(p), to_screen(q));
            canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, DIM_STROKE);
        };
        for (p, q) in geom.extensions {
            stroke_line(canvas, p, q);
        }
        stroke_line(canvas, geom.line.0, geom.line.1);
        for tri in geom.arrows {
            let pts: Vec<(f32, f32)> = tri
                .iter()
                .map(|p| {
                    let s = to_screen(*p);
                    (s.x, s.y)
                })
                .collect();
            canvas.poly(&pts, BLACK);
        }
        // The label runs along the dimension line, or sits past its end if too short (#314).
        let label = crate::value::format_length_display_in((wa - wb).length(), unit);
        let (sa, sb) = (to_screen(geom.line.0), to_screen(geom.line.1));
        let out_screen = (to_screen(geom.line.0 + outward) - to_screen(geom.line.0)).normalize();
        let (lp, ang) = dimension_label_layout(
            sa,
            sb,
            out_screen,
            text_device_width(11.0, &label),
            5.0,
        );
        canvas.text_rot(lp.x, lp.y, 11.0, Anchor::Middle, &label, ang);
    }

    // Angle dimensions: the degree value at (or near) the two edges' corner.
    for (k1, k2) in &view.angle_dims {
        let (a0, a1) = (dequant(k1.0), dequant(k1.1));
        let (b0, b1) = (dequant(k2.0), dequant(k2.1));
        let d1 = (a1 - a0).normalize_or_zero();
        let d2 = (b1 - b0).normalize_or_zero();
        if d1.length_squared() < 0.5 || d2.length_squared() < 0.5 {
            continue;
        }
        let angle = d1.angle_between(d2).to_degrees();
        let shared = [k1.0, k1.1]
            .into_iter()
            .find(|e| *e == k2.0 || *e == k2.1)
            .map(dequant);
        let anchor = shared.unwrap_or_else(|| ((a0 + a1) * 0.5 + (b0 + b1) * 0.5) * 0.5);
        let sp = to_screen(project(anchor));
        canvas.text(sp.x, sp.y - 12.0, 12.0, Anchor::Middle, &format!("{angle:.0}°"));
    }
}

// ----- SVG backend -----

struct SvgCanvas {
    body: String,
}

fn svg_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn svg_color(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

impl Canvas for SvgCanvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32) {
        let fill = fill.map(svg_color).unwrap_or_else(|| "none".to_string());
        let stroke_attr = match stroke {
            Some(c) => format!(" stroke=\"{}\" stroke-width=\"{stroke_w}\"", svg_color(c)),
            None => String::new(),
        };
        self.body.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"{fill}\"{stroke_attr}/>\n"
        ));
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        self.body.push_str(&format!(
            "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{}\" \
             stroke-width=\"{width}\"/>\n",
            svg_color(color)
        ));
    }

    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb) {
        // Stroked with its own fill so adjacent shaded triangles don't show hairline seams.
        let points: Vec<String> = pts.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
        self.body.push_str(&format!(
            "<polygon points=\"{}\" fill=\"{fill}\" stroke=\"{fill}\" stroke-width=\"0.6\"/>\n",
            points.join(" "),
            fill = svg_color(fill)
        ));
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32) {
        self.body.push_str(&format!(
            "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"none\" stroke=\"{}\" \
             stroke-width=\"{width}\"/>\n",
            svg_color(color)
        ));
    }

    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str) {
        let anchor = match anchor {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
        };
        self.body.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"sans-serif\" font-size=\"{size}\" \
             fill=\"black\" text-anchor=\"{anchor}\">{}</text>\n",
            svg_esc(content)
        ));
    }

    fn text_rot(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str, angle: f32) {
        let anchor = match anchor {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
        };
        let deg = angle.to_degrees();
        self.body.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"sans-serif\" font-size=\"{size}\" \
             fill=\"black\" text-anchor=\"{anchor}\" transform=\"rotate({deg:.2} {x:.1} {y:.1})\">{}</text>\n",
            svg_esc(content)
        ));
    }
}

/// Render one drawing to a self-contained black-on-white SVG document. `None` if the drawing
/// index is missing or deleted.
pub fn drawing_to_svg(doc: &Document, index: usize) -> Option<String> {
    let (width, height) = page_dims(doc, index)?;
    let mut canvas = SvgCanvas { body: String::new() };
    render_drawing(doc, index, &mut canvas)?;
    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">\n"
    ));
    s.push_str(&canvas.body);
    s.push_str("</svg>\n");
    Some(s)
}

// ----- PDF backend -----

/// Accumulates a PDF content stream. PDF space is bottom-left origin, so `y` is flipped from
/// the top-left [`Canvas`] coordinates using the page `height`.
struct PdfCanvas {
    ops: Vec<u8>,
    height: f32,
}

impl PdfCanvas {
    fn new(height: f32) -> Self {
        PdfCanvas { ops: Vec::new(), height }
    }
    fn push(&mut self, s: &str) {
        self.ops.extend_from_slice(s.as_bytes());
    }
    fn set_fill(&mut self, c: Rgb) {
        self.push(&format!("{:.3} {:.3} {:.3} rg\n", c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0));
    }
    fn set_stroke(&mut self, c: Rgb) {
        self.push(&format!("{:.3} {:.3} {:.3} RG\n", c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0));
    }
}

/// Escape a string into PDF WinAnsi document bytes (the font is Helvetica/WinAnsiEncoding).
fn pdf_text_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in s.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push(b'\\');
                out.push(ch as u8);
            }
            '°' => out.push(0xB0),  // WinAnsi degree sign
            '—' | '–' => out.push(0x97), // em/en dash → WinAnsi em dash
            'Ø' | '⌀' => out.push(0xD8), // diameter → WinAnsi Ø (Latin O with stroke)
            c if (c as u32) < 128 => out.push(c as u8),
            _ => out.push(b'?'),
        }
    }
    out
}

impl Canvas for PdfCanvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32) {
        // Top-left (x, y) with height h → PDF bottom-left corner is (x, H - y - h).
        let py = self.height - y - h;
        self.push(&format!("{x:.2} {py:.2} {w:.2} {h:.2} re\n"));
        match (fill, stroke) {
            (Some(f), Some(s)) => {
                self.set_fill(f);
                self.set_stroke(s);
                self.push(&format!("{stroke_w:.2} w\nB\n"));
            }
            (Some(f), None) => {
                self.set_fill(f);
                self.push("f\n");
            }
            (None, Some(s)) => {
                self.set_stroke(s);
                self.push(&format!("{stroke_w:.2} w\nS\n"));
            }
            (None, None) => self.push("n\n"),
        }
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        let (py1, py2) = (self.height - y1, self.height - y2);
        self.set_stroke(color);
        self.push(&format!("{width:.2} w\n{x1:.2} {py1:.2} m {x2:.2} {py2:.2} l S\n"));
    }

    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb) {
        let Some(((x0, y0), rest)) = pts.split_first() else {
            return;
        };
        // Fill *and* stroke with the same grey so adjacent shaded triangles don't show
        // hairline seams between them.
        self.set_fill(fill);
        self.set_stroke(fill);
        let mut path = format!("0.6 w\n{x0:.2} {:.2} m ", self.height - y0);
        for (x, y) in rest {
            path.push_str(&format!("{x:.2} {:.2} l ", self.height - y));
        }
        path.push_str("h b\n");
        self.push(&path);
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32) {
        // Four cubic Bézier arcs (kappa ≈ 0.5523) approximate a circle; y flips to PDF space.
        let k = 0.552_284_75 * r;
        let cy = self.height - cy;
        self.set_stroke(color);
        let mut path = format!("{width:.2} w\n{:.2} {cy:.2} m ", cx + r);
        // Right → top → left → bottom, counter-clockwise in PDF's y-up space.
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx + r, cy + k, cx + k, cy + r, cx, cy + r));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx - k, cy + r, cx - r, cy + k, cx - r, cy));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx - r, cy - k, cx - k, cy - r, cx, cy - r));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx + k, cy - r, cx + r, cy - k, cx + r, cy));
        path.push_str("S\n");
        self.push(&path);
    }

    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str) {
        // Helvetica averages ~0.5em per glyph; good enough to center dimension labels.
        let width = 0.5 * size * content.chars().count() as f32;
        let tx = match anchor {
            Anchor::Start => x,
            Anchor::Middle => x - width * 0.5,
        };
        let py = self.height - y;
        self.set_fill(BLACK);
        self.push(&format!("BT /F1 {size:.2} Tf {tx:.2} {py:.2} Td ("));
        let bytes = pdf_text_bytes(content);
        self.ops.extend_from_slice(&bytes);
        self.push(") Tj ET\n");
    }

    fn text_rot(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str, angle: f32) {
        // Rotate about (x, y) via the text matrix. Screen angle is clockwise (y-down); PDF is
        // y-up, so negate. Centre by shifting half the text width along the rotated baseline.
        let width = 0.5 * size * content.chars().count() as f32;
        let half = match anchor {
            Anchor::Middle => width * 0.5,
            Anchor::Start => 0.0,
        };
        let a = -angle;
        let (c, s) = (a.cos(), a.sin());
        let py = self.height - y;
        let tx = x - half * c;
        let ty = py - half * s;
        self.set_fill(BLACK);
        self.push(&format!(
            "BT /F1 {size:.2} Tf {c:.4} {s:.4} {:.4} {c:.4} {tx:.2} {ty:.2} Tm (",
            -s
        ));
        let bytes = pdf_text_bytes(content);
        self.ops.extend_from_slice(&bytes);
        self.push(") Tj ET\n");
    }
}

/// Render one drawing to a self-contained single-page PDF (black-on-white, Helvetica text).
/// `None` if the drawing index is missing or deleted.
pub fn drawing_to_pdf(doc: &Document, index: usize) -> Option<Vec<u8>> {
    let (width, height) = page_dims(doc, index)?;
    let mut canvas = PdfCanvas::new(height);
    render_drawing(doc, index, &mut canvas)?;
    Some(assemble_pdf(width, height, &canvas.ops))
}

/// Wrap a content stream in a minimal single-page PDF document (catalog, pages, one page with
/// a Helvetica font, the content stream), with a correct cross-reference table.
fn assemble_pdf(width: f32, height: f32, content: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");

    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, &mut offsets, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        &mut offsets,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.2} {height:.2}] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    // Content stream object (4).
    {
        offsets.push(out.len());
        out.extend_from_slice(b"4 0 obj\n");
        out.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        out.extend_from_slice(content);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }
    obj(
        &mut out,
        &mut offsets,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );

    // Cross-reference table + trailer.
    let xref_pos = out.len();
    let count = offsets.len() + 1;
    out.extend_from_slice(format!("xref\n0 {count}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Drawing, DrawingView};

    /// #345: a free-angle orientation projects with its stored basis, so a free basis equal to a
    /// preset's reproduces that preset exactly (the convention `view_cube::free_basis` is chosen so
    /// a spun Front pose == the Front projection).
    #[test]
    fn free_orientation_uses_its_stored_basis() {
        use crate::model::DrawingOrientation as O;
        let (fr, fu) = view_axes(O::Front);
        let free = O::Free { right: fr.to_array(), up: fu.to_array() };
        let (r, u) = view_axes(free);
        assert!((r - fr).length() < 1e-5 && (u - fu).length() < 1e-5, "free == Front basis");
        // A denormalised / non-orthogonal stored basis is re-orthonormalised, not trusted blindly.
        let sloppy = O::Free { right: [2.0, 0.0, 0.0], up: [0.3, 0.0, 4.0] };
        let (r, u) = view_axes(sloppy);
        assert!((r.length() - 1.0).abs() < 1e-4 && (u.length() - 1.0).abs() < 1e-4);
        assert!(r.dot(u).abs() < 1e-4, "re-orthonormalised");
    }

    /// #351: an aligned child unfolds from its parent's basis for *any* base orientation, so a Top
    /// base yields Front below, Back above, and rotated Left/Right to the sides — all four
    /// directions offerable, and each rendered with the correct (possibly rotated) basis.
    #[test]
    fn aligned_children_unfold_for_a_top_base() {
        use crate::model::{AlignDir, DrawingOrientation as O};
        // All four directions are offered from a Top base (not just Below).
        for dir in [AlignDir::Below, AlignDir::Above, AlignDir::Left, AlignDir::Right] {
            assert!(aligned_child_orientation(O::Top, dir).is_some(), "{dir:?} offered");
        }
        assert_eq!(aligned_child_orientation(O::Top, AlignDir::Below), Some(O::Front));

        // The rendered bases come from resolved_view_axes unfolding the Top parent (X, -Y).
        let parent = DrawingView {
            body: 0, sketch: None, orientation: O::Top,
            dimensioned_edges: Vec::new(), angle_dims: Vec::new(), dimension_offsets: Vec::new(),
            dimensioned_circles: Vec::new(), aligned_parent: None, aligned_dir: None,
            scale: None, style: Default::default(), pos_x: 0.5, pos_y: 0.5,
        };
        let child = |dir| DrawingView {
            aligned_parent: Some(0), aligned_dir: Some(dir), ..parent.clone()
        };
        let views = |dir| vec![parent.clone(), child(dir)];
        // Top parent basis = (X, -Y), into-page = X×(-Y) = -Z.
        let vb = views(AlignDir::Below);
        assert_eq!(resolved_view_axes(&vb, &vb[1]), (Vec3::X, Vec3::Z), "below → Front basis");
        let va = views(AlignDir::Above);
        assert_eq!(resolved_view_axes(&va, &va[1]), (Vec3::X, -Vec3::Z), "above → rotated Back");
        let vr = views(AlignDir::Right);
        assert_eq!(resolved_view_axes(&vr, &vr[1]), (Vec3::Z, -Vec3::Y), "right → rotated Right");
        let vl = views(AlignDir::Left);
        assert_eq!(resolved_view_axes(&vl, &vl[1]), (-Vec3::Z, -Vec3::Y), "left → rotated Left");
    }

    /// #332: an aligned child dragged to the side of a Front parent can be re-oriented to any of
    /// the four views that share the vertical axis (Front/Back/Left/Right), and one dragged above
    /// or below to the four sharing the horizontal axis (Front/Back/Top/Bottom).
    #[test]
    fn aligned_inline_orientations_stay_in_line() {
        use crate::model::{AlignDir, DrawingOrientation as O};
        let side = aligned_inline_orientations(O::Front, AlignDir::Right);
        for o in [O::Front, O::Back, O::Left, O::Right] {
            assert!(side.contains(&o), "{o:?} should be an in-line side view");
        }
        // The diagonal vertical-edge views (#339) share the vertical axis too, so they're in-line.
        use crate::model::EdgeView;
        for e in [EdgeView::FrontRight, EdgeView::BackRight, EdgeView::BackLeft, EdgeView::FrontLeft] {
            assert!(side.contains(&O::Edge(e)), "{e:?} should be an in-line diagonal");
        }
        assert!(!side.contains(&O::Top) && !side.contains(&O::Bottom));
        assert!(!side.contains(&O::Isometric));
        assert!(!side.contains(&O::Edge(EdgeView::FrontTop)), "tilted edges aren't in-line here");

        let stack = aligned_inline_orientations(O::Front, AlignDir::Below);
        for o in [O::Front, O::Back, O::Top, O::Bottom] {
            assert!(stack.contains(&o), "{o:?} should be an in-line stacked view");
        }
        assert!(!stack.contains(&O::Left) && !stack.contains(&O::Right));
    }

    /// #339: every edge view projects with a valid orthonormal basis, and a vertical-edge view
    /// (Front-Right) is the 45° rotation of Front about the vertical axis.
    #[test]
    fn edge_view_bases_are_orthonormal() {
        use crate::model::{DrawingOrientation as O, EdgeView};
        for e in EdgeView::ALL {
            let (r, u) = view_axes(O::Edge(*e));
            assert!((r.length() - 1.0).abs() < 1e-4, "{e:?} right unit");
            assert!((u.length() - 1.0).abs() < 1e-4, "{e:?} up unit");
            assert!(r.dot(u).abs() < 1e-4, "{e:?} right ⟂ up");
        }
        // Front is (right=X, up=Z); Front-Right rotates 45° about Z → right=(X+Y)/√2, up=Z.
        let (r, u) = view_axes(O::Edge(EdgeView::FrontRight));
        let inv = 1.0 / 2.0_f32.sqrt();
        assert!((r - glam::Vec3::new(inv, inv, 0.0)).length() < 1e-4, "got {r:?}");
        assert!((u - glam::Vec3::Z).length() < 1e-4, "got {u:?}");
    }

    /// #344: every corner view projects with a valid orthonormal basis, and distinct corners give
    /// distinct views (not one fixed isometric).
    #[test]
    fn corner_view_bases_are_orthonormal_and_distinct() {
        use crate::model::{CornerView, DrawingOrientation as O};
        let mut outs = Vec::new();
        for c in CornerView::ALL {
            let (r, u) = view_axes(O::Corner(*c));
            assert!((r.length() - 1.0).abs() < 1e-4, "{c:?} right unit");
            assert!((u.length() - 1.0).abs() < 1e-4, "{c:?} up unit");
            assert!(r.dot(u).abs() < 1e-4, "{c:?} right ⟂ up");
            outs.push(r.cross(u));
        }
        // The eight corner view directions are all different.
        for i in 0..outs.len() {
            for j in (i + 1)..outs.len() {
                assert!((outs[i] - outs[j]).length() > 0.1, "corners {i},{j} share a view");
            }
        }
    }

    /// #314: a label that fits runs centred along the dimension line (angle matches, kept
    /// upright); one too wide sits past the far end, horizontal.
    #[test]
    fn dimension_label_runs_along_or_beside_the_line() {
        use std::f32::consts::FRAC_PI_2;
        let out = glam::Vec2::new(0.0, -1.0);
        // A long horizontal line: label fits, angle ~0, centred on the midpoint.
        let (pos, ang) = dimension_label_layout(
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(100.0, 0.0),
            out,
            30.0,
            5.0,
        );
        assert!(ang.abs() < 1e-3, "horizontal line → horizontal label");
        assert!((pos.x - 50.0).abs() < 1e-3, "centred along the line");
        // A downward vertical line (screen y grows down): the label reads bottom-to-top
        // (angle −90°), never top-to-bottom (#322).
        let (_, ang_v) = dimension_label_layout(
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(0.0, 100.0),
            glam::Vec2::new(1.0, 0.0),
            30.0,
            5.0,
        );
        assert!((ang_v + FRAC_PI_2).abs() < 1e-3, "downward vertical → reads bottom-to-top (−90°)");
        // The reverse direction reads the same way.
        assert!(
            (readable_text_angle(glam::Vec2::new(0.0, -1.0)) + FRAC_PI_2).abs() < 1e-3,
            "upward vertical also reads bottom-to-top"
        );
        // A down-to-the-right slope is allowed to read top-left → bottom-right (positive angle).
        assert!(readable_text_angle(glam::Vec2::new(1.0, 1.0)) > 0.0);
        // A short line: label can't fit, so it sits past the far end (x > line end), horizontal.
        let (pos_s, ang_s) = dimension_label_layout(
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(4.0, 0.0),
            out,
            30.0,
            5.0,
        );
        assert!(ang_s.abs() < 1e-3, "short line → horizontal label");
        assert!(pos_s.x > 4.0, "label sits past the far end");
    }

    /// #321: two parallel dimensions on the same side whose spans overlap land on different
    /// tiers (different offsets); a non-overlapping pair shares the innermost tier.
    #[test]
    fn overlapping_parallel_dimensions_get_staggered() {
        let out = glam::Vec2::new(0.0, -1.0);
        // Two horizontal dimensions on the same side, spans overlapping in x.
        let dims = vec![
            (glam::Vec2::new(0.0, 0.0), glam::Vec2::new(10.0, 0.0), out),
            (glam::Vec2::new(2.0, 0.0), glam::Vec2::new(8.0, 0.0), out),
        ];
        let offs = plan_dimension_tiers(&dims, 1.0);
        assert!(
            (offs[0] - offs[1]).abs() > 1e-3,
            "overlapping parallel dims should be on different tiers: {offs:?}"
        );
        // Two horizontal dims on the same side but non-overlapping spans → same tier (0).
        let dims2 = vec![
            (glam::Vec2::new(0.0, 0.0), glam::Vec2::new(10.0, 0.0), out),
            (glam::Vec2::new(20.0, 0.0), glam::Vec2::new(30.0, 0.0), out),
        ];
        let offs2 = plan_dimension_tiers(&dims2, 1.0);
        assert!((offs2[0]).abs() < 1e-4 && (offs2[1]).abs() < 1e-4, "non-overlapping share tier 0");
    }

    /// #313/#319: a tessellated circle in a plane is detected in 3D (centre/radius/normal); a
    /// run of straight edges is not. Projected face-on it's Round, edge-on it's a line.
    #[test]
    fn detects_a_world_circle_and_projects_it() {
        // A 32-gon of radius 10 in the XY plane (normal +Z), centred at (5, 3, 0).
        let n = 32;
        let c = Vec3::new(5.0, 3.0, 0.0);
        let r = 10.0;
        let pts: Vec<Vec3> = (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                c + Vec3::new(a.cos(), a.sin(), 0.0) * r
            })
            .collect();
        let mut edges: Vec<(Vec3, Vec3)> = (0..n).map(|i| (pts[i], pts[(i + 1) % n])).collect();
        // Plus a separate straight square in a different place — not a circle.
        let sq = [
            Vec3::new(40.0, 0.0, 0.0),
            Vec3::new(50.0, 0.0, 0.0),
            Vec3::new(50.0, 10.0, 0.0),
            Vec3::new(40.0, 10.0, 0.0),
        ];
        for i in 0..4 {
            edges.push((sq[i], sq[(i + 1) % 4]));
        }
        let circles = classify_world_circles(&edges);
        assert_eq!(circles.len(), 1, "one circle (the 32-gon, not the square)");
        assert!((circles[0].radius - r).abs() < 0.3);
        // Looking down +Z (Top view: right=X, up=-Y) the circle faces us → Round.
        match project_world_circle(&circles[0], Vec3::X, -Vec3::Y) {
            ProjectedCircle::Round { radius, .. } => assert!((radius - r).abs() < 0.3),
            _ => panic!("face-on circle should project Round"),
        }
        // Looking along the plane (Front view: right=X, up=Z) it's edge-on → a line.
        match project_world_circle(&circles[0], Vec3::X, Vec3::Z) {
            ProjectedCircle::EdgeOn { a, b } => assert!(((a - b).length() - 2.0 * r).abs() < 0.5),
            _ => panic!("edge-on circle should project EdgeOn"),
        }
    }

    /// #296: a Front parent's aligned children follow the issue's mapping — down→Bottom,
    /// up→Top, right→Right, left→Left — and an isometric parent has no orthographic child.
    #[test]
    fn aligned_children_of_front_follow_the_screen_direction() {
        use crate::model::AlignDir;
        use DrawingOrientation as O;
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Below), Some(O::Bottom));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Above), Some(O::Top));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Right), Some(O::Right));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Left), Some(O::Left));
        // The four upright views (up = +Z) neighbour each other around the vertical axis, so
        // their left/right children are always canonical orthographic views.
        for parent in [O::Front, O::Back, O::Left, O::Right] {
            assert!(aligned_child_orientation(parent, AlignDir::Right).is_some(), "{parent:?}");
            assert!(aligned_child_orientation(parent, AlignDir::Left).is_some(), "{parent:?}");
        }
        // Directions whose unfolded view would need a rolled (non-canonical) up simply have no
        // aligned child, and an isometric parent never resolves — the tool just won't offer it.
        assert_eq!(aligned_child_orientation(O::Isometric, AlignDir::Below), None);
    }

    fn doc_with_drawing() -> Document {
        let mut doc = Document::default();
        doc.drawings.push(Drawing {
            name: Some("Plate".to_string()),
            views: vec![DrawingView {
                body: 0,
                sketch: None,
                orientation: DrawingOrientation::Front,
                dimensioned_edges: Vec::new(),
                angle_dims: Vec::new(),
                dimension_offsets: Vec::new(),
                dimensioned_circles: Vec::new(),
                aligned_parent: None,
                aligned_dir: None,
                scale: None,
                style: Default::default(),
                pos_x: 0.5,
                pos_y: 0.5,
            }],
            // The title now renders as a normal text annotation, added with the drawing (#335),
            // not a baked-in export stamp — mirror that here.
            annotations: vec![crate::model::DrawingAnnotation {
                text: "Plate".to_string(),
                pos_x: 0.045,
                pos_y: 0.02,
                size_frac: 0.028,
                wrap_frac: None,
                deleted: false,
            }],
            deleted: false,
            ..Default::default()
        });
        doc
    }

    #[test]
    fn svg_export_is_a_document() {
        let svg = drawing_to_svg(&doc_with_drawing(), 0).unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Plate"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn pdf_export_is_a_single_page_document() {
        let pdf = drawing_to_pdf(&doc_with_drawing(), 0).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"), "has a PDF header");
        assert!(pdf.ends_with(b"%%EOF\n"), "ends at EOF marker");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Type /Catalog"));
        assert!(text.contains("/BaseFont /Helvetica"));
        assert!(text.contains("startxref"));
        // The title text is emitted into the content stream.
        assert!(text.contains("(Plate) Tj"));

        // Cross-reference integrity, checked against the RAW bytes (the content stream can
        // carry non-UTF-8 WinAnsi bytes, so string indices wouldn't match byte offsets):
        // startxref points at the `xref` table and every listed offset lands on its
        // `N 0 obj` — the easy thing to get wrong in a hand-rolled PDF.
        let start = parse_startxref(&pdf);
        assert_eq!(&pdf[start..start + 4], b"xref");
        let offsets = parse_xref_offsets(&pdf[start..]);
        assert_eq!(offsets.len(), 5, "five objects in the xref");
        for (i, off) in offsets.iter().enumerate() {
            let expect = format!("{} 0 obj", i + 1);
            assert!(
                pdf[*off..].starts_with(expect.as_bytes()),
                "xref offset {off} should point at '{expect}'"
            );
        }
    }

    /// The `startxref` byte offset from a PDF's trailer.
    fn parse_startxref(pdf: &[u8]) -> usize {
        let needle = b"startxref";
        let pos = pdf.windows(needle.len()).rposition(|w| w == needle).unwrap()
            + needle.len();
        let rest = &pdf[pos..];
        let digits: Vec<u8> = rest
            .iter()
            .skip_while(|b| b.is_ascii_whitespace())
            .take_while(|b| b.is_ascii_digit())
            .copied()
            .collect();
        String::from_utf8(digits).unwrap().parse().unwrap()
    }

    /// The object byte offsets listed in an xref table (the ` n ` entries, skipping the free
    /// object 0).
    fn parse_xref_offsets(table: &[u8]) -> Vec<usize> {
        String::from_utf8_lossy(table)
            .lines()
            .filter_map(|l| {
                let l = l.trim_end();
                (l.len() == 18 && l.ends_with(" n")).then(|| l[..10].parse().unwrap())
            })
            .collect()
    }

    /// #298: the exported page is the drawing's configured mm page in PDF points — the
    /// default is landscape US-Letter, 792 × 612 pt.
    #[test]
    fn pdf_page_matches_the_configured_page_size() {
        let doc = doc_with_drawing();
        let pdf = drawing_to_pdf(&doc, 0).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("/MediaBox [0 0 792.00 612.00]"),
            "default landscape-letter MediaBox, got: {}",
            text.lines().find(|l| l.contains("MediaBox")).unwrap_or("<none>")
        );

        let mut doc = doc;
        doc.drawings[0].page_width_mm = 210.0; // portrait A4
        doc.drawings[0].page_height_mm = 297.0;
        let pdf = drawing_to_pdf(&doc, 0).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        let media = text.lines().find(|l| l.contains("MediaBox")).unwrap().to_string();
        assert!(
            media.contains("[0 0 595.") && media.contains(" 841."),
            "A4 MediaBox in points, got: {media}"
        );
    }

    /// #297: exports are WYSIWYG — a view's card lands at its `pos_x`/`pos_y` page fraction,
    /// so two views placed apart export apart (not into a fixed grid).
    #[test]
    fn svg_places_views_at_their_page_positions() {
        let mut doc = doc_with_drawing();
        let mut second = doc.drawings[0].views[0].clone();
        doc.drawings[0].views[0].pos_x = 0.25;
        doc.drawings[0].views[0].pos_y = 0.3;
        second.pos_x = 0.75;
        second.pos_y = 0.7;
        doc.drawings[0].views.push(second);
        let svg = drawing_to_svg(&doc, 0).unwrap();
        let (page_w, page_h) = page_dims(&doc, 0).unwrap();
        // Exports have no card border (#337); each view's caption text is placed at
        // (cell_x + CELL_PAD, cell_y + 20), so its position pins the card.
        let cell_w = page_w * CELL_FRAC;
        let cell_h = page_h * CELL_FRAC;
        for (px, py) in [(0.25f32, 0.3f32), (0.75, 0.7)] {
            let x = px * page_w - cell_w * 0.5 + CELL_PAD;
            let y = py * page_h - cell_h * 0.5 + 20.0;
            let needle = format!("<text x=\"{x:.1}\" y=\"{y:.1}\"");
            assert!(svg.contains(&needle), "expected a view caption at {needle}");
        }
    }

    #[test]
    fn missing_drawing_has_no_export() {
        let doc = Document::default();
        assert!(drawing_to_svg(&doc, 0).is_none());
        assert!(drawing_to_pdf(&doc, 0).is_none());
    }
}
