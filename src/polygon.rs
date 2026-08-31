//! Closed-polygon face detection (#66): any set of plain `Line` entities that connect
//! end-to-end into a closed loop (via `Coincident` point constraints, or simply by
//! geometrically coinciding, #1791) can be used as a face, the same way a `Rect` or
//! `Circle` profile can.

use crate::document_lifecycle::line_alive;
use crate::model::{ConstraintPoint, Document, LineEnd, SketchId};
use crate::vertex_drag::coincident_group;

/// How close two unconstrained endpoints must be (sketch units, mm) to count as the
/// same vertex (#1791). Scripted chains type endpoints exactly; a micron tolerates
/// float noise without ever merging visibly distinct corners.
const GEOMETRIC_MERGE_TOLERANCE: f32 = 1e-3;

/// Canonical identity of a sketch vertex: either a `Coincident`-joined group of line
/// endpoints (keyed by its canonical member, so it survives drags that temporarily
/// separate the endpoints) or a geometric position cluster of otherwise-unconnected
/// endpoints that coincide within [`GEOMETRIC_MERGE_TOLERANCE`] (#1791).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum VertexId {
    Constrained(crate::model::LineKey, bool),
    Free(usize),
}

/// Endpoint → [`VertexId`] map for one sketch, built once per query. Endpoints joined
/// by `Coincident` constraints keep their group identity; the rest are clustered by
/// position so an exact-touching line chain forms a loop without constraints (#1791).
struct VertexIndex {
    ids: std::collections::HashMap<(crate::model::LineKey, bool), VertexId>,
}

impl VertexIndex {
    fn build(doc: &Document, sketch: SketchId) -> Self {
        let endpoints: Vec<(crate::model::LineKey, LineEnd)> = doc
            .lines
            .iter()
            .filter(|(_, l)| l.sketch == sketch)
            .flat_map(|(i, _)| [(i, LineEnd::Start), (i, LineEnd::End)])
            .collect();
        let mut ids = std::collections::HashMap::new();
        let mut free: Vec<(crate::model::LineKey, LineEnd)> = Vec::new();
        for &(line, end) in &endpoints {
            let group: Vec<_> = coincident_group(doc, sketch, ConstraintPoint::LineEndpoint { line, end })
                .into_iter()
                .filter_map(|p| match p {
                    ConstraintPoint::LineEndpoint { line, end } => Some((line, end)),
                    _ => None,
                })
                .collect();
            if group.len() > 1 {
                let (key_line, key_end) = group
                    .iter()
                    .copied()
                    .min_by_key(|&(l, e)| (l, is_end(e)))
                    .unwrap();
                ids.insert((line, is_end(end)), VertexId::Constrained(key_line, is_end(key_end)));
            } else {
                free.push((line, end));
            }
        }
        // Cluster the unconstrained endpoints by proximity: each endpoint joins the
        // first cluster within tolerance, else starts a new one. Sketches are tiny,
        // so the linear scan is fine.
        let mut clusters: Vec<(f32, f32)> = Vec::new();
        for (line, end) in free {
            let l = &doc.lines[line];
            let (x, y) = match end {
                LineEnd::Start => (l.x0, l.y0),
                LineEnd::End => (l.x1, l.y1),
            };
            let id = match clusters.iter().position(|&(cx, cy)| {
                (x - cx).hypot(y - cy) <= GEOMETRIC_MERGE_TOLERANCE
            }) {
                Some(c) => c,
                None => {
                    clusters.push((x, y));
                    clusters.len() - 1
                }
            };
            ids.insert((line, is_end(end)), VertexId::Free(id));
        }
        VertexIndex { ids }
    }

    fn id(&self, line: crate::model::LineKey, end: LineEnd) -> VertexId {
        self.ids
            .get(&(line, is_end(end)))
            .copied()
            .unwrap_or(VertexId::Constrained(line, is_end(end)))
    }
}

fn is_end(end: LineEnd) -> bool {
    matches!(end, LineEnd::End)
}

/// Every closed loop of connected `Line`s in `sketch`, as ordered line indices.
///
/// A loop is any simple cycle in the graph whose nodes are vertex groups and whose edges
/// are lines (no line repeated within a loop). Loops are deduped by their line-index set
/// (so the same polygon found by walking it in either direction, or starting from a
/// different line, is reported once), and returned in a deterministic order: sorted by
/// their lowest-numbered line, then by length.
/// Closed sketch profiles a script can extrude: circles, line loops, text glyphs, and
/// hosted-plane regions (#1888).
pub fn sketch_profiles(
    doc: &Document,
    sketch: SketchId,
) -> Vec<crate::model::ExtrudeFace> {
    let mut faces = Vec::new();
    for (k, c) in doc.circles.iter() {
        if c.sketch == sketch && !c.construction {
            faces.push(crate::model::ExtrudeFace::Circle(k));
        }
    }
    for lines in closed_line_loops(doc, sketch) {
        faces.push(crate::model::ExtrudeFace::Polygon(lines));
    }
    for (k, t) in doc.sketch_texts.iter() {
        if t.sketch == sketch {
            let n = crate::text::group_glyphs(&t.contours).len();
            for glyph in 0..n {
                faces.push(crate::model::ExtrudeFace::TextGlyph { text: k, glyph });
            }
        }
    }
    for region in sketch_plane_regions(doc, sketch) {
        if region.is_empty() {
            continue;
        }
        let n = region.len() as f32;
        let u = region.iter().map(|p| p.0).sum::<f32>() / n;
        let v = region.iter().map(|p| p.1).sum::<f32>() / n;
        let (seed_u, seed_v) = crate::model::sketch_region_seed(u, v);
        faces.push(crate::model::ExtrudeFace::SketchRegion {
            sketch,
            seed_u,
            seed_v,
        });
    }
    faces
}

pub fn closed_line_loops(doc: &Document, sketch: SketchId) -> Vec<Vec<crate::model::LineKey>> {
    let lines: Vec<crate::model::LineKey> = doc
        .lines
        .iter()
        // Shadow lines (#224, consumed by an in-sketch slice) keep existing for editing but no
        // longer form faces — their split fragments do.
        .filter(|(_, l)| l.sketch == sketch && !l.shadow)
        .map(|(i, _)| i)
        .collect();
    if lines.len() < 3 {
        return Vec::new();
    }
    let index = VertexIndex::build(doc, sketch);

    // For each line, the vertex id at its start and end.
    let endpoints: std::collections::HashMap<crate::model::LineKey, (VertexId, VertexId)> = lines
        .iter()
        .map(|&i| {
            (
                i,
                (
                    index.id(i, LineEnd::Start),
                    index.id(i, LineEnd::End),
                ),
            )
        })
        .collect();

    // Lines incident to each vertex id, paired with which of their own endpoints sits there.
    let mut incident: std::collections::HashMap<VertexId, Vec<(crate::model::LineKey, bool)>> =
        std::collections::HashMap::new();
    for (&line, &(start_key, end_key)) in &endpoints {
        incident.entry(start_key).or_default().push((line, false));
        incident.entry(end_key).or_default().push((line, true));
    }

    let mut found: Vec<Vec<crate::model::LineKey>> = Vec::new();
    let mut seen_sets: std::collections::HashSet<Vec<crate::model::LineKey>> = std::collections::HashSet::new();

    for &start_line in &lines {
        // Walk from `start_line`'s end vertex, looking for a path back to its start vertex.
        let mut path = vec![start_line];
        let mut used: std::collections::HashSet<crate::model::LineKey> =
            std::collections::HashSet::new();
        used.insert(start_line);
        let (_, first_end_key) = endpoints[&start_line];
        walk(
            &incident,
            &endpoints,
            first_end_key,
            &mut path,
            &mut used,
            &mut found,
            &mut seen_sets,
        );
    }

    // Two filters make the cycle enumeration behave like planar-face extraction after a slice
    // (#238), while staying a no-op for ordinary sketches:
    //  1. Vertex-simple: drop self-touching cycles that pass through the same vertex twice (e.g. a
    //     naive cycle running "straight through" a cut point, enclosing both faces at once).
    //  2. Minimal face: drop a loop that another alive line subdivides — an internal **chord**
    //     connecting two of its *non-adjacent* boundary vertices. This is exactly the cut chord
    //     relative to the reconstructed outer boundary, so the un-split perimeter is rejected in
    //     favour of the two half-faces. A disjoint nested shape shares no vertices with the outer
    //     loop, so it never triggers this — nested faces still resolve normally.
    let ordered: Vec<Option<Vec<VertexId>>> = found
        .iter()
        .map(|lines| loop_shared_vertices(doc, sketch, lines))
        .collect();
    let mut keep = Vec::with_capacity(found.len());
    for (lines, verts) in found.into_iter().zip(ordered) {
        let Some(verts) = verts else { continue }; // not vertex-simple
        if !loop_is_minimal_face(doc, sketch, &lines, &verts) {
            continue;
        }
        keep.push(lines);
    }
    let mut found = keep;

    found.sort_by(|a, b| {
        let min_a = *a.iter().min().unwrap();
        let min_b = *b.iter().min().unwrap();
        min_a.cmp(&min_b).then(a.len().cmp(&b.len()))
    });
    found
}

/// The ordered coincidence-keyed vertices around a closed line loop (vertex `i` is shared by
/// `lines[i]` and `lines[i+1]`), or `None` if the loop isn't vertex-simple — it revisits a vertex,
/// or consecutive lines don't actually share one. Vertices are keyed exactly as
/// [`closed_line_loops`] walks them.
fn loop_shared_vertices(
    doc: &Document,
    sketch: SketchId,
    lines: &[crate::model::LineKey],
) -> Option<Vec<VertexId>> {
    let n = lines.len();
    if n < 3 {
        return None;
    }
    let index = VertexIndex::build(doc, sketch);
    let keys: Vec<(VertexId, VertexId)> = lines
        .iter()
        .map(|&i| (index.id(i, LineEnd::Start), index.id(i, LineEnd::End)))
        .collect();
    let mut shared = Vec::with_capacity(n);
    for i in 0..n {
        let j = (i + 1) % n;
        let (s0, e0) = keys[i];
        let (s1, e1) = keys[j];
        let v = if e0 == s1 || e0 == e1 {
            e0
        } else if s0 == s1 || s0 == e1 {
            s0
        } else {
            return None;
        };
        shared.push(v);
    }
    let mut sorted = shared.clone();
    sorted.sort_unstable();
    sorted.dedup();
    (sorted.len() == n).then_some(shared)
}

/// Whether `lines` (with ordered boundary vertices `verts`) is a **minimal** face — no other alive,
/// non-shadow line of the sketch bridges two of its *non-adjacent* boundary vertices (which would
/// subdivide it). Used to reject a slice's reconstructed outer perimeter in favour of the two
/// halves the cut chord makes (#238).
fn loop_is_minimal_face(
    doc: &Document,
    sketch: SketchId,
    lines: &[crate::model::LineKey],
    verts: &[VertexId],
) -> bool {
    let n = verts.len();
    let pos: std::collections::HashMap<VertexId, usize> =
        verts.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let index = VertexIndex::build(doc, sketch);
    for (li, l) in doc.lines.iter() {
        if l.sketch != sketch || l.shadow || lines.contains(&li) {
            continue;
        }
        let a = index.id(li, LineEnd::Start);
        let b = index.id(li, LineEnd::End);
        if let (Some(&ia), Some(&ib)) = (pos.get(&a), pos.get(&b)) {
            let adjacent = ia == ib
                || (ia + 1) % n == ib
                || (ib + 1) % n == ia;
            if !adjacent {
                return false; // an internal chord subdivides this loop
            }
        }
    }
    true
}

fn walk(
    incident: &std::collections::HashMap<VertexId, Vec<(crate::model::LineKey, bool)>>,
    endpoints: &std::collections::HashMap<crate::model::LineKey, (VertexId, VertexId)>,
    current: VertexId,
    path: &mut Vec<crate::model::LineKey>,
    used: &mut std::collections::HashSet<crate::model::LineKey>,
    found: &mut Vec<Vec<crate::model::LineKey>>,
    seen_sets: &mut std::collections::HashSet<Vec<crate::model::LineKey>>,
) {
    if path.len() > 64 {
        // Defensive bound against pathological inputs; real sketches are tiny.
        return;
    }
    let Some(candidates) = incident.get(&current) else {
        return;
    };
    for &(next_line, at_end) in candidates {
        if next_line == *path.last().unwrap() {
            continue;
        }
        if next_line == path[0] {
            // Back to the start: only a real loop once we've used at least 3 lines.
            if path.len() >= 3 {
                let mut set: Vec<crate::model::LineKey> = path.clone();
                set.sort_unstable();
                if seen_sets.insert(set) {
                    found.push(path.clone());
                }
            }
            continue;
        }
        if used.contains(&next_line) {
            continue;
        }
        let (start_key, end_key) = endpoints[&next_line];
        let next_vertex = if at_end { start_key } else { end_key };
        path.push(next_line);
        used.insert(next_line);
        walk(
            incident, endpoints, next_vertex, path, used, found, seen_sets,
        );
        used.remove(&next_line);
        path.pop();
    }
}

/// The boundary vertices (local sketch coordinates) of a closed loop, in order: vertex `i`
/// is the endpoint of `lines[i]` shared with `lines[i - 1]` (wrapping around) — i.e. each
/// line is walked in whichever direction continues the loop, regardless of which endpoint
/// is stored as that line's `Start`/`End`. A curved (bezier) line contributes its entry
/// point plus intermediate sampled points (its exit point is the next line's entry point),
/// so the returned vertex count can exceed `lines.len()`.
///
/// Returns `None` if the lines don't actually form a closed loop (consecutive lines, with
/// wraparound, must share a vertex via a `Coincident` constraint).
pub fn loop_vertices_uv(
    doc: &Document,
    sketch: SketchId,
    lines: &[crate::model::LineKey],
) -> Option<Vec<(f32, f32)>> {
    if lines.len() < 3 {
        return None;
    }
    let index = VertexIndex::build(doc, sketch);
    let keys: Vec<(VertexId, VertexId)> = lines
        .iter()
        .map(|&i| (index.id(i, LineEnd::Start), index.id(i, LineEnd::End)))
        .collect();

    let mut vertices = Vec::new();
    for i in 0..lines.len() {
        let prev = (i + lines.len() - 1) % lines.len();
        let (prev_start, prev_end) = keys[prev];
        let (start, end) = keys[i];
        let reversed = if start == prev_start || start == prev_end {
            false
        } else if end == prev_start || end == prev_end {
            true
        } else {
            return None;
        };
        let line = doc.lines.get(lines[i])?;
        let mut sampled = line.sample_local(crate::model::BEZIER_SEGMENTS);
        if reversed {
            sampled.reverse();
        }
        sampled.pop(); // the exit point is the next line's entry point
        vertices.extend(sampled);
    }
    Some(vertices)
}

/// The loop's **corner** vertices — one per line, in loop order — without faceting curves
/// (#178). Where [`loop_vertices_uv`] samples every bezier along the boundary, this returns
/// only the analytic corners: `corner[i]` is the oriented start of `lines[i]`, so `lines[i]`'s
/// span is `corner[i] -> corner[(i+1) % n]`. This is what lets a side wall be addressed by its
/// profile-line index rather than a curve's faceted sub-edge index.
pub fn loop_corner_vertices_uv(
    doc: &Document,
    sketch: SketchId,
    lines: &[crate::model::LineKey],
) -> Option<Vec<(f32, f32)>> {
    if lines.len() < 3 {
        return None;
    }
    let index = VertexIndex::build(doc, sketch);
    let keys: Vec<(VertexId, VertexId)> = lines
        .iter()
        .map(|&i| (index.id(i, LineEnd::Start), index.id(i, LineEnd::End)))
        .collect();

    let mut corners = Vec::with_capacity(lines.len());
    for i in 0..lines.len() {
        let prev = (i + lines.len() - 1) % lines.len();
        let (prev_start, prev_end) = keys[prev];
        let (start, end) = keys[i];
        let reversed = if start == prev_start || start == prev_end {
            false
        } else if end == prev_start || end == prev_end {
            true
        } else {
            return None;
        };
        let line = doc.lines.get(lines[i])?;
        corners.push(if reversed {
            (line.x1, line.y1)
        } else {
            (line.x0, line.y0)
        });
    }
    Some(corners)
}

/// Ear-clipping triangulation of a simple (possibly concave) 2D polygon. `vertices` are
/// ordered boundary points; returns `n - 2` triangles as index triples into `vertices`.
pub fn triangulate_uv(vertices: &[(f32, f32)]) -> Vec<[usize; 3]> {
    let n = vertices.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }

    let ccw = signed_area_2d(vertices) > 0.0;
    let mut indices: Vec<usize> = (0..n).collect();
    let mut triangles = Vec::with_capacity(n - 2);

    let mut guard = 0;
    while indices.len() > 3 {
        if guard > n * n {
            break;
        }
        guard += 1;
        let mut ear_found = false;
        let len = indices.len();
        for i in 0..len {
            let prev = indices[(i + len - 1) % len];
            let curr = indices[i];
            let next = indices[(i + 1) % len];
            if !is_convex_vertex_2d(vertices[prev], vertices[curr], vertices[next], ccw) {
                continue;
            }
            let tri = [vertices[prev], vertices[curr], vertices[next]];
            let contains_other = indices.iter().any(|&idx| {
                idx != prev
                    && idx != curr
                    && idx != next
                    && point_in_triangle_2d(vertices[idx], tri[0], tri[1], tri[2])
            });
            if contains_other {
                continue;
            }
            triangles.push([prev, curr, next]);
            indices.remove(i);
            ear_found = true;
            break;
        }
        if !ear_found {
            break;
        }
    }
    if indices.len() == 3 {
        triangles.push([indices[0], indices[1], indices[2]]);
    }
    triangles
}

/// Triangulate a planar polygon **with holes** (#268) in world space, returning world-space
/// triangles (not indices). Each hole is spliced into the outer loop by a zero-width *bridge*
/// (the classic hole-elimination technique — connect the hole's rightmost vertex to a visible
/// outer vertex, walk the hole, and bridge back), reducing the region to one weakly-simple loop
/// that ear-clipping handles. The outer loop is normalised to CCW and holes to CW so the merged
/// loop stays consistently wound. With no holes this is just [`triangulate_planar`] mapped to
/// world points.
pub fn triangulate_planar_with_holes(
    outer: &[glam::Vec3],
    holes: &[Vec<glam::Vec3>],
    normal: glam::Vec3,
) -> Vec<[glam::Vec3; 3]> {
    if outer.len() < 3 {
        return Vec::new();
    }
    if holes.is_empty() {
        return triangulate_planar(outer, normal)
            .into_iter()
            .map(|[a, b, c]| [outer[a], outer[b], outer[c]])
            .collect();
    }
    let n = normal.normalize_or_zero();
    let u_axis = (if n.z.abs() < 0.9 { glam::Vec3::Z.cross(n) } else { glam::Vec3::X.cross(n) })
        .normalize_or_zero();
    let v_axis = n.cross(u_axis).normalize_or_zero();
    let origin = outer[0];
    let to_uv = |p: glam::Vec3| {
        let r = p - origin;
        (r.dot(u_axis), r.dot(v_axis))
    };

    // Outer loop CCW.
    let mut loop_uv: Vec<(f32, f32)> = outer.iter().map(|&p| to_uv(p)).collect();
    let mut loop_w: Vec<glam::Vec3> = outer.to_vec();
    if signed_area_2d(&loop_uv) < 0.0 {
        loop_uv.reverse();
        loop_w.reverse();
    }

    // Holes CW, bridged rightmost-first so each bridge lands on already-merged geometry.
    let mut prepared: Vec<(Vec<(f32, f32)>, Vec<glam::Vec3>)> = holes
        .iter()
        .filter(|h| h.len() >= 3)
        .map(|h| {
            let mut huv: Vec<(f32, f32)> = h.iter().map(|&p| to_uv(p)).collect();
            let mut hw = h.clone();
            if signed_area_2d(&huv) > 0.0 {
                huv.reverse();
                hw.reverse();
            }
            (huv, hw)
        })
        .collect();
    prepared.sort_by(|a, b| {
        let am = a.0.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        let bm = b.0.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        bm.partial_cmp(&am).unwrap_or(std::cmp::Ordering::Equal)
    });

    for (huv, hw) in prepared {
        // Rightmost hole vertex.
        let m = (0..huv.len())
            .max_by(|&i, &j| huv[i].0.partial_cmp(&huv[j].0).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        let mp = huv[m];
        // Closest loop vertex to its right — visible for the convex-ish outer loops we build from
        // (rects, circles). Falls back to the nearest overall vertex if none is to the right.
        let mut best: Option<(usize, f32)> = None;
        for (i, &lp) in loop_uv.iter().enumerate() {
            let d = (lp.0 - mp.0).powi(2) + (lp.1 - mp.1).powi(2);
            let to_right = lp.0 >= mp.0 - 1e-6;
            let rank = if to_right { d } else { d + 1e9 };
            if best.map_or(true, |(_, bd)| rank < bd) {
                best = Some((i, rank));
            }
        }
        let p = best.map(|(i, _)| i).unwrap_or(0);

        let mut new_uv = Vec::with_capacity(loop_uv.len() + huv.len() + 2);
        let mut new_w = Vec::with_capacity(loop_w.len() + hw.len() + 2);
        new_uv.extend_from_slice(&loop_uv[..=p]);
        new_w.extend_from_slice(&loop_w[..=p]);
        for k in 0..huv.len() {
            let idx = (m + k) % huv.len();
            new_uv.push(huv[idx]);
            new_w.push(hw[idx]);
        }
        new_uv.push(huv[m]);
        new_w.push(hw[m]);
        new_uv.push(loop_uv[p]);
        new_w.push(loop_w[p]);
        new_uv.extend_from_slice(&loop_uv[p + 1..]);
        new_w.extend_from_slice(&loop_w[p + 1..]);
        loop_uv = new_uv;
        loop_w = new_w;
    }

    // Ear-clip the merged loop. Unlike the shared [`triangulate_uv`], the bridges introduce
    // pairs of coincident vertices (the zero-width slit), so the "does any other vertex fall in
    // this ear?" test must be *strict* — a vertex sitting exactly on the ear's corner (its bridge
    // twin) mustn't block the ear, or clipping stalls.
    ear_clip_with_bridges(&loop_uv)
        .into_iter()
        .map(|[a, b, c]| [loop_w[a], loop_w[b], loop_w[c]])
        .collect()
}

/// Ear-clipping for a (weakly-simple) loop that may contain bridge slits — i.e. pairs of
/// coincident vertices. The containment test is strict-interior so a bridge twin coinciding with
/// an ear corner doesn't veto the ear.
fn ear_clip_with_bridges(vertices: &[(f32, f32)]) -> Vec<[usize; 3]> {
    let n = vertices.len();
    if n < 3 {
        return Vec::new();
    }
    let ccw = signed_area_2d(vertices) > 0.0;
    let mut indices: Vec<usize> = (0..n).collect();
    let mut triangles = Vec::with_capacity(n - 2);
    let strict_inside = |p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)| -> bool {
        // Barycentric with a strict margin; points on/near a vertex or edge don't count.
        let v0 = (c.0 - a.0, c.1 - a.1);
        let v1 = (b.0 - a.0, b.1 - a.1);
        let v2 = (p.0 - a.0, p.1 - a.1);
        let d00 = v0.0 * v0.0 + v0.1 * v0.1;
        let d01 = v0.0 * v1.0 + v0.1 * v1.1;
        let d02 = v0.0 * v2.0 + v0.1 * v2.1;
        let d11 = v1.0 * v1.0 + v1.1 * v1.1;
        let d12 = v1.0 * v2.0 + v1.1 * v2.1;
        let denom = d00 * d11 - d01 * d01;
        if denom.abs() < 1e-12 {
            return false;
        }
        let inv = 1.0 / denom;
        let u = (d11 * d02 - d01 * d12) * inv;
        let v = (d00 * d12 - d01 * d02) * inv;
        u > 1e-5 && v > 1e-5 && (u + v) < 1.0 - 1e-5
    };
    let mut guard = 0;
    while indices.len() > 3 {
        if guard > n * n {
            break;
        }
        guard += 1;
        let mut ear_found = false;
        let len = indices.len();
        for i in 0..len {
            let prev = indices[(i + len - 1) % len];
            let curr = indices[i];
            let next = indices[(i + 1) % len];
            if !is_convex_vertex_2d(vertices[prev], vertices[curr], vertices[next], ccw) {
                continue;
            }
            let tri = [vertices[prev], vertices[curr], vertices[next]];
            let contains_other = indices.iter().any(|&idx| {
                idx != prev
                    && idx != curr
                    && idx != next
                    && strict_inside(vertices[idx], tri[0], tri[1], tri[2])
            });
            if contains_other {
                continue;
            }
            triangles.push([prev, curr, next]);
            indices.remove(i);
            ear_found = true;
            break;
        }
        if !ear_found {
            break;
        }
    }
    if indices.len() == 3 {
        triangles.push([indices[0], indices[1], indices[2]]);
    }
    triangles
}

/// Triangulate a simple planar polygon in world space (same winding as the boundary loop).
pub fn triangulate_planar(vertices: &[glam::Vec3], normal: glam::Vec3) -> Vec<[usize; 3]> {
    if vertices.len() < 3 {
        return Vec::new();
    }
    let uv = project_planar_uv(vertices, normal);
    triangulate_uv(&uv)
}

fn project_planar_uv(vertices: &[glam::Vec3], normal: glam::Vec3) -> Vec<(f32, f32)> {
    let n = normal.normalize_or_zero();
    let mut u_axis = if n.z.abs() < 0.9 {
        glam::Vec3::Z.cross(n)
    } else {
        glam::Vec3::X.cross(n)
    };
    u_axis = u_axis.normalize_or_zero();
    let v_axis = n.cross(u_axis).normalize_or_zero();
    let origin = vertices[0];
    vertices
        .iter()
        .map(|p| {
            let rel = *p - origin;
            (rel.dot(u_axis), rel.dot(v_axis))
        })
        .collect()
}

fn signed_area_2d(vertices: &[(f32, f32)]) -> f32 {
    let mut area = 0.0;
    for i in 0..vertices.len() {
        let j = (i + 1) % vertices.len();
        area += vertices[i].0 * vertices[j].1 - vertices[j].0 * vertices[i].1;
    }
    area * 0.5
}

fn is_convex_vertex_2d(prev: (f32, f32), curr: (f32, f32), next: (f32, f32), ccw: bool) -> bool {
    let cross = (curr.0 - prev.0) * (next.1 - prev.1) - (curr.1 - prev.1) * (next.0 - prev.0);
    if ccw {
        cross > 1e-6
    } else {
        cross < -1e-6
    }
}

pub(crate) fn point_in_triangle_2d(
    p: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
) -> bool {
    let v0 = (c.0 - a.0, c.1 - a.1);
    let v1 = (b.0 - a.0, b.1 - a.1);
    let v2 = (p.0 - a.0, p.1 - a.1);
    let dot00 = v0.0 * v0.0 + v0.1 * v0.1;
    let dot01 = v0.0 * v1.0 + v0.1 * v1.1;
    let dot02 = v0.0 * v2.0 + v0.1 * v2.1;
    let dot11 = v1.0 * v1.0 + v1.1 * v1.1;
    let dot12 = v1.0 * v2.0 + v1.1 * v2.1;
    let denom = dot00 * dot11 - dot01 * dot01;
    if denom.abs() < 1e-8 {
        return false;
    }
    let inv = 1.0 / denom;
    let u = (dot11 * dot02 - dot01 * dot12) * inv;
    let v = (dot00 * dot12 - dot01 * dot02) * inv;
    u >= -1e-4 && v >= -1e-4 && (u + v) <= 1.0 + 1e-4
}

/// Even-odd (ray-casting) point-in-polygon test; winding-independent. Used both by tests and,
/// at runtime, to resolve which atomic boolean region (#16/#62) a click landed in.
pub fn point_in_polygon_2d(p: (f32, f32), vertices: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = vertices.len();
    for i in 0..n {
        let j = (i + 1) % n;
        let (xi, yi) = vertices[i];
        let (xj, yj) = vertices[j];
        let intersects = (yi > p.1) != (yj > p.1)
            && p.0 < (xj - xi) * (p.1 - yi) / (yj - yi) + xi;
        if intersects {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::sketch_key_for_slot as skey;
    use super::*;
    use crate::model::{Constraint, ConstraintEntity, ConstraintKind, Line};

    /// #993: the reported case. A square face ruled by two lines across it reads as three
    /// regions to anyone looking at it, and neither line closes a loop with anything — the
    /// regions are bounded by the *face's own outline*, which the sketch never drew.
    #[test]
    fn two_lines_across_a_face_make_three_regions() {
        // A 10x10 outline, cut by two horizontal lines at y = 3 and y = 7.
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut segments: Vec<((f32, f32), (f32, f32))> = (0..4)
            .map(|i| (square[i], square[(i + 1) % 4]))
            .collect();
        segments.push(((0.0, 3.0), (10.0, 3.0)));
        segments.push(((0.0, 7.0), (10.0, 7.0)));

        let regions = planar_regions(&segments);
        assert_eq!(regions.len(), 3, "two cuts across a face make three regions");
        let areas: Vec<f32> = regions.iter().map(|r| signed_area(r)).collect();
        assert!(
            areas.iter().all(|a| *a > 0.0),
            "every region is wound counter-clockwise, got {areas:?}"
        );
        let total: f32 = areas.iter().sum();
        assert!(
            (total - 100.0).abs() < 0.01,
            "the regions tile the face exactly, got {total}"
        );
        // Sorted by lowest corner, so they come out bottom-to-top: 30, 40, 30.
        for (got, want) in areas.iter().zip([30.0, 40.0, 30.0]) {
            assert!((got - want).abs() < 0.01, "region areas {areas:?}");
        }
    }

    /// A cut that stops short of the far side divides nothing — it leaves one region, and a
    /// dangling edge must not spawn a degenerate sliver.
    #[test]
    fn a_cut_that_does_not_reach_across_divides_nothing() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut segments: Vec<((f32, f32), (f32, f32))> = (0..4)
            .map(|i| (square[i], square[(i + 1) % 4]))
            .collect();
        segments.push(((0.0, 5.0), (6.0, 5.0)));
        let regions = planar_regions(&segments);
        assert_eq!(regions.len(), 1, "a partial cut leaves the face whole, got {regions:?}");
        assert!((signed_area(&regions[0]) - 100.0).abs() < 0.01);
    }

    /// Crossing cuts make four quadrants — the split has to happen at the crossing, not just
    /// where a line meets the outline.
    #[test]
    fn crossing_cuts_make_four_regions() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let mut segments: Vec<((f32, f32), (f32, f32))> = (0..4)
            .map(|i| (square[i], square[(i + 1) % 4]))
            .collect();
        segments.push(((0.0, 5.0), (10.0, 5.0)));
        segments.push(((5.0, 0.0), (5.0, 10.0)));
        let regions = planar_regions(&segments);
        assert_eq!(regions.len(), 4, "a cross makes quadrants");
        let total: f32 = regions.iter().map(|r| signed_area(r)).sum();
        assert!((total - 100.0).abs() < 0.01, "and they tile the face");
        assert!(
            regions.iter().all(|r| (signed_area(r) - 25.0).abs() < 0.01),
            "each quadrant is a quarter"
        );
    }

    /// #268: a square with a square hole triangulates to the annulus area (outer − hole), and no
    /// triangle covers the hole's interior.
    #[test]
    fn triangulate_with_holes_leaves_the_hole_empty() {
        use glam::Vec3;
        let outer = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(10.0, 10.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
        ];
        let hole = vec![
            Vec3::new(4.0, 4.0, 0.0),
            Vec3::new(6.0, 4.0, 0.0),
            Vec3::new(6.0, 6.0, 0.0),
            Vec3::new(4.0, 6.0, 0.0),
        ];
        let tris = triangulate_planar_with_holes(&outer, &[hole], Vec3::Z);
        let area: f32 = tris
            .iter()
            .map(|[a, b, c]| (b - a).cross(c - a).length() * 0.5)
            .sum();
        assert!((area - 96.0).abs() < 1e-2, "annulus area should be 100 − 4 = 96, got {area}");
        // The hole centre (5,5) must not be inside any emitted triangle.
        let center = (5.0_f32, 5.0_f32);
        let covered = tris.iter().any(|[a, b, c]| {
            point_in_triangle_2d(center, (a.x, a.y), (b.x, b.y), (c.x, c.y))
        });
        assert!(!covered, "no triangle should cover the hole interior");
    }

    fn coincident(sketch: SketchId, a: ConstraintPoint, b: ConstraintPoint) -> Constraint {
        Constraint {
            sketch,
            kind: ConstraintKind::Coincident {
                a: ConstraintEntity::Point(a),
                b: ConstraintEntity::Point(b),
            },
            expression: String::new(),
            dim_offset: None,
            name: None,
        }
    }

    fn line(sketch: SketchId, x0: f32, y0: f32, x1: f32, y1: f32) -> Line {
        Line::from_local_endpoints(sketch, x0, y0, x1, y1)
    }

    fn point(line: crate::model::LineKey, end: LineEnd) -> ConstraintPoint {
        ConstraintPoint::LineEndpoint { line, end }
    }

    #[test]
    fn three_lines_closed_into_a_triangle_form_one_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        // Three lines, each one's end coincident with the next one's start, closing back.
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 0.0));
        doc.lines.insert(line(skey(0), 10.0, 0.0, 5.0, 8.0));
        doc.lines.insert(line(skey(0), 5.0, 8.0, 0.0, 0.0));
        doc.constraints.insert(coincident(
            skey(0),
            point(lkey(0), LineEnd::End),
            point(lkey(1), LineEnd::Start),
        ));
        doc.constraints.insert(coincident(
            skey(0),
            point(lkey(1), LineEnd::End),
            point(lkey(2), LineEnd::Start),
        ));
        doc.constraints.insert(coincident(
            skey(0),
            point(lkey(2), LineEnd::End),
            point(lkey(0), LineEnd::Start),
        ));

        let loops = closed_line_loops(&doc, skey(0));
        assert_eq!(loops.len(), 1);
        let mut sorted = loops[0].clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![lkey(0), lkey(1), lkey(2)]);
    }

    /// #238: two triangles meeting at a single shared vertex (a bowtie) are two faces, not three —
    /// the 6-line cycle that runs through the shared vertex twice is self-touching, not a face, and
    /// the vertex-simplicity filter drops it. This is the same shape a face-slice produces at a cut
    /// point, so getting it right is what makes Option-A face slicing detect exactly two loops.
    #[test]
    fn two_triangles_sharing_a_vertex_are_two_faces_not_three() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        // Triangle 1: P0(0,0) P1(10,0) V(5,5).
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 0.0)); // 0
        doc.lines.insert(line(skey(0), 10.0, 0.0, 5.0, 5.0)); // 1
        doc.lines.insert(line(skey(0), 5.0, 5.0, 0.0, 0.0)); // 2
        // Triangle 2: V(5,5) P3(10,10) P4(0,10).
        doc.lines.insert(line(skey(0), 5.0, 5.0, 10.0, 10.0)); // 3
        doc.lines.insert(line(skey(0), 10.0, 10.0, 0.0, 10.0)); // 4
        doc.lines.insert(line(skey(0), 0.0, 10.0, 5.0, 5.0)); // 5
        let joins = [
            (lkey(0), LineEnd::End, lkey(1), LineEnd::Start),
            (lkey(1), LineEnd::End, lkey(2), LineEnd::Start),
            (lkey(2), LineEnd::End, lkey(0), LineEnd::Start),
            (lkey(3), LineEnd::End, lkey(4), LineEnd::Start),
            (lkey(4), LineEnd::End, lkey(5), LineEnd::Start),
            (lkey(5), LineEnd::End, lkey(3), LineEnd::Start),
            // Glue the four endpoints that all sit at the shared apex V.
            (lkey(1), LineEnd::End, lkey(3), LineEnd::Start),
            (lkey(2), LineEnd::Start, lkey(5), LineEnd::End),
        ];
        for (la, ea, lb, eb) in joins {
            doc.constraints
                .insert(coincident(skey(0), point(la, ea), point(lb, eb)));
        }
        let loops = closed_line_loops(&doc, skey(0));
        assert_eq!(loops.len(), 2, "bowtie is two faces, got {loops:?}");
    }

    #[test]
    fn open_chain_of_lines_has_no_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 0.0));
        doc.lines.insert(line(skey(0), 10.0, 0.0, 5.0, 8.0));
        doc.constraints.insert(coincident(
            skey(0),
            point(lkey(0), LineEnd::End),
            point(lkey(1), LineEnd::Start),
        ));

        assert!(closed_line_loops(&doc, skey(0)).is_empty());
    }

    #[test]
    fn unconnected_lines_form_no_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 0.0));
        doc.lines.insert(line(skey(0), 100.0, 0.0, 110.0, 0.0));
        doc.lines.insert(line(skey(0), 200.0, 0.0, 210.0, 0.0));

        assert!(closed_line_loops(&doc, skey(0)).is_empty());
    }

    #[test]
    fn deleted_line_does_not_participate_in_a_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 0.0));
        doc.lines.insert(line(skey(0), 10.0, 0.0, 5.0, 8.0));
        doc.lines.insert(line(skey(0), 5.0, 8.0, 0.0, 0.0));
        doc.lines.remove(lkey(2));
        doc.constraints.insert(coincident(
            skey(0),
            point(lkey(0), LineEnd::End),
            point(lkey(1), LineEnd::Start),
        ));
        doc.constraints.insert(coincident(
            skey(0),
            point(lkey(1), LineEnd::End),
            point(lkey(2), LineEnd::Start),
        ));
        doc.constraints.insert(coincident(
            skey(0),
            point(lkey(2), LineEnd::End),
            point(lkey(0), LineEnd::Start),
        ));

        assert!(closed_line_loops(&doc, skey(0)).is_empty());
    }

    #[test]
    fn four_lines_closed_into_a_quad_form_one_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 0.0));
        doc.lines.insert(line(skey(0), 10.0, 0.0, 10.0, 10.0));
        doc.lines.insert(line(skey(0), 10.0, 10.0, 0.0, 10.0));
        doc.lines.insert(line(skey(0), 0.0, 10.0, 0.0, 0.0));
        for i in 0..4 {
            doc.constraints.insert(coincident(
                skey(0),
                point(lkey(i), LineEnd::End),
                point(lkey((i + 1) % 4), LineEnd::Start),
            ));
        }

        let loops = closed_line_loops(&doc, skey(0));
        assert_eq!(loops.len(), 1);
        let mut sorted = loops[0].clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![lkey(0), lkey(1), lkey(2), lkey(3)]);
    }

    #[test]
    fn concave_polygon_triangulation_stays_inside_boundary() {
        // L-shaped hexagon: convex fan from the first vertex fills the missing notch.
        let pts = vec![
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 1.0),
            (1.0, 1.0),
            (1.0, 4.0),
            (0.0, 4.0),
        ];
        let tris = triangulate_uv(&pts);
        assert_eq!(tris.len(), 4);
        for [a, b, c] in &tris {
            let centroid = (
                (pts[*a].0 + pts[*b].0 + pts[*c].0) / 3.0,
                (pts[*a].1 + pts[*b].1 + pts[*c].1) / 3.0,
            );
            assert!(
                point_in_polygon_2d(centroid, &pts),
                "centroid {centroid:?} outside polygon"
            );
        }
        let leak = (2.0, 2.0);
        assert!(!point_in_polygon_2d(leak, &pts), "notch point should lie outside the L");
        for [a, b, c] in &tris {
            assert!(!point_in_triangle_2d(leak, pts[*a], pts[*b], pts[*c]));
        }
    }

    #[test]
    fn concave_loop_inside_a_split_quad_is_detected_and_triangulated() {
        // Outer quad A-B-C-D with a concave inner loop A-P-E-F-A where P lies on edge B-C.
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        // Outer quad edges 0..3
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 0.0)); // A-B
        doc.lines.insert(line(skey(0), 10.0, 0.0, 10.0, 10.0)); // B-C
        doc.lines.insert(line(skey(0), 10.0, 10.0, 0.0, 10.0)); // C-D
        doc.lines.insert(line(skey(0), 0.0, 10.0, 0.0, 0.0)); // D-A
        // Inner concave loop edges 4..7
        doc.lines.insert(line(skey(0), 0.0, 0.0, 10.0, 5.0)); // A-P
        doc.lines.insert(line(skey(0), 10.0, 5.0, 6.0, 8.0)); // P-E
        doc.lines.insert(line(skey(0), 6.0, 8.0, 2.0, 6.0)); // E-F
        doc.lines.insert(line(skey(0), 2.0, 6.0, 0.0, 0.0)); // F-A
        doc.constraints.insert(coincident(skey(0), point(lkey(0), LineEnd::End), point(lkey(1), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(1), LineEnd::End), point(lkey(2), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(2), LineEnd::End), point(lkey(3), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(3), LineEnd::End), point(lkey(0), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(4), LineEnd::End), point(lkey(1), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(4), LineEnd::Start), point(lkey(0), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(5), LineEnd::End), point(lkey(6), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(6), LineEnd::End), point(lkey(7), LineEnd::Start)));
        doc.constraints.insert(coincident(skey(0), point(lkey(7), LineEnd::End), point(lkey(4), LineEnd::Start)));

        let loops = closed_line_loops(&doc, skey(0));
        assert!(loops.len() >= 2, "expected outer and inner loops, got {loops:?}");
        let inner = loops
            .iter()
            .find(|l| l.len() == 4 && l.contains(&lkey(4)))
            .expect("inner concave loop");
        let uv = loop_vertices_uv(&doc, skey(0), inner).unwrap();
        assert_eq!(uv.len(), 4);
        let tris = triangulate_uv(&uv);
        assert_eq!(tris.len(), 2);
        for [a, b, c] in &tris {
            let centroid = (
                (uv[*a].0 + uv[*b].0 + uv[*c].0) / 3.0,
                (uv[*a].1 + uv[*b].1 + uv[*c].1) / 3.0,
            );
            assert!(
                point_in_polygon_2d(centroid, &uv),
                "inner face centroid {centroid:?} leaked outside loop"
            );
        }
    }
}

/// The regions a hosted sketch's plane is divided into (#993), each as a closed polygon in
/// **sketch-local** coordinates, wound counter-clockwise.
///
/// A sketch drawn *on a face* has a boundary it did not draw: the face's own outline. Two lines
/// ruled across a box's top cap read to anyone as three regions, but neither line closes a loop
/// with anything, so `closed_line_loops` — which only ever sees the sketch's own lines — finds
/// none, and the whole cap stays the only thing to extrude.
///
/// This builds the **planar arrangement** of the host face's boundary together with the sketch's
/// own solid lines: every segment is split wherever another crosses it, and the minimal faces of
/// the resulting graph are the regions. Returns empty for a sketch with no host face, and for one
/// whose lines divide nothing — a single undivided region is not a division, and offering it
/// would just duplicate the face itself.
///
/// Construction geometry is scaffolding and never bounds a region; nor do shadow or dead lines.
pub fn sketch_plane_regions(doc: &Document, sketch: SketchId) -> Vec<Vec<(f32, f32)>> {
    let Some(frame) = crate::face::sketch_geometry_frame(doc, sketch) else {
        return Vec::new();
    };
    let Some(host) = doc.sketch_face(sketch) else {
        return Vec::new();
    };
    let Some(boundary) = crate::extrude::face_boundary_loop_world(doc, &host) else {
        return Vec::new();
    };
    // The host outline, brought into the sketch's own frame.
    let outline: Vec<(f32, f32)> = boundary
        .iter()
        .map(|p| crate::face::world_to_local(&frame, *p))
        .collect();
    if outline.len() < 3 {
        return Vec::new();
    }
    let mut segments: Vec<((f32, f32), (f32, f32))> = Vec::new();
    for i in 0..outline.len() {
        segments.push((outline[i], outline[(i + 1) % outline.len()]));
    }
    // The sketch's own lines, curves sampled to polylines so a bend bounds a region like
    // anything else.
    let mut cutters = 0usize;
    for (li, line) in doc.lines.iter() {
        if line.sketch != sketch || line.construction || line.shadow || !line_alive(doc, li) {
            continue;
        }
        let points = line.sample_local(crate::model::BEZIER_SEGMENTS);
        for w in points.windows(2) {
            segments.push((w[0], w[1]));
        }
        cutters += 1;
    }
    if cutters == 0 {
        return Vec::new();
    }
    let regions = planar_regions(&segments);
    // One region is just the face over again.
    if regions.len() < 2 {
        return Vec::new();
    }
    regions
}

/// How close two points must be to count as the same vertex of an arrangement, in sketch units.
const ARRANGEMENT_EPS: f32 = 1e-4;

/// The minimal faces of the planar graph `segments` make, each wound counter-clockwise.
///
/// Segments are split at every crossing first, so they only ever meet at endpoints; the faces
/// then fall out of the standard "walk the most-clockwise turn at each vertex" traversal. The
/// unbounded outer face comes out wound the other way, which is how it's told from the rest.
fn planar_regions(segments: &[((f32, f32), (f32, f32))]) -> Vec<Vec<(f32, f32)>> {
    let split = split_at_crossings(segments);
    // Weld coincident endpoints into shared vertices.
    let mut points: Vec<(f32, f32)> = Vec::new();
    let index_of = |p: (f32, f32), points: &mut Vec<(f32, f32)>| -> usize {
        for (i, q) in points.iter().enumerate() {
            if (p.0 - q.0).abs() <= ARRANGEMENT_EPS && (p.1 - q.1).abs() <= ARRANGEMENT_EPS {
                return i;
            }
        }
        points.push(p);
        points.len() - 1
    };
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for (a, b) in split {
        let (ia, ib) = (index_of(a, &mut points), index_of(b, &mut points));
        if ia != ib && !edges.contains(&(ia, ib)) && !edges.contains(&(ib, ia)) {
            edges.push((ia, ib));
        }
    }
    if edges.is_empty() {
        return Vec::new();
    }
    // Two directed half-edges per edge; `2i` runs a→b and `2i+1` runs b→a.
    let half = |e: usize, rev: bool| e * 2 + usize::from(rev);
    let from = |h: usize| {
        let (a, b) = edges[h / 2];
        if h % 2 == 0 { a } else { b }
    };
    let to = |h: usize| {
        let (a, b) = edges[h / 2];
        if h % 2 == 0 { b } else { a }
    };
    let angle = |h: usize| {
        let (p, q) = (points[from(h)], points[to(h)]);
        (q.1 - p.1).atan2(q.0 - p.0)
    };
    // Half-edges leaving each vertex, sorted by direction.
    let mut out_of: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for e in 0..edges.len() {
        out_of[edges[e].0].push(half(e, false));
        out_of[edges[e].1].push(half(e, true));
    }
    for outs in out_of.iter_mut() {
        outs.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
    }
    let mut visited = vec![false; edges.len() * 2];
    let mut faces: Vec<Vec<(f32, f32)>> = Vec::new();
    for start in 0..edges.len() * 2 {
        if visited[start] {
            continue;
        }
        let mut loop_points: Vec<(f32, f32)> = Vec::new();
        let mut h = start;
        loop {
            if visited[h] {
                break;
            }
            visited[h] = true;
            loop_points.push(points[from(h)]);
            // Turn as sharply clockwise as possible at the far end: step onto the twin, then
            // take the outgoing half-edge just before it in angle order. That is what walks the
            // face on one consistent side and makes the faces minimal.
            let twin = h ^ 1;
            let v = from(twin);
            let outs = &out_of[v];
            let at = outs.iter().position(|&x| x == twin).unwrap_or(0);
            h = outs[(at + outs.len() - 1) % outs.len()];
            if h == start {
                break;
            }
        }
        if loop_points.len() >= 3 && signed_area(&loop_points) > ARRANGEMENT_EPS {
            faces.push(loop_points);
        }
    }
    // Deterministic order: by the lowest corner each region reaches.
    faces.sort_by(|a, b| {
        let key = |f: &Vec<(f32, f32)>| {
            f.iter().fold((f32::MAX, f32::MAX), |acc, p| {
                (acc.0.min(p.0), acc.1.min(p.1))
            })
        };
        let (ka, kb) = (key(a), key(b));
        ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
    faces
}

/// Twice the signed area of a closed polygon — positive when wound counter-clockwise.
fn signed_area(poly: &[(f32, f32)]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum::<f32>()
        / 2.0
}

/// Split every segment wherever another crosses or touches it, so the pieces meet only at
/// endpoints — the precondition the face walk needs.
fn split_at_crossings(
    segments: &[((f32, f32), (f32, f32))],
) -> Vec<((f32, f32), (f32, f32))> {
    let mut out = Vec::new();
    for (i, &(a, b)) in segments.iter().enumerate() {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len_sq = dx * dx + dy * dy;
        if len_sq <= ARRANGEMENT_EPS * ARRANGEMENT_EPS {
            continue;
        }
        // Parameters along this segment where something touches it, ends included.
        let mut ts: Vec<f32> = vec![0.0, 1.0];
        for (j, &(c, d)) in segments.iter().enumerate() {
            if i == j {
                continue;
            }
            for t in crossing_params(a, b, c, d) {
                if t > ARRANGEMENT_EPS && t < 1.0 - ARRANGEMENT_EPS {
                    ts.push(t);
                }
            }
        }
        ts.sort_by(f32::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() <= ARRANGEMENT_EPS);
        for w in ts.windows(2) {
            let p = (a.0 + dx * w[0], a.1 + dy * w[0]);
            let q = (a.0 + dx * w[1], a.1 + dy * w[1]);
            if (p.0 - q.0).abs() > ARRANGEMENT_EPS || (p.1 - q.1).abs() > ARRANGEMENT_EPS {
                out.push((p, q));
            }
        }
    }
    out
}

/// Where segment `cd` meets segment `ab`, as parameters along `ab`. Collinear overlaps
/// contribute `cd`'s own endpoints, so a segment lying along another still splits it.
fn crossing_params(
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    d: (f32, f32),
) -> Vec<f32> {
    let (rx, ry) = (b.0 - a.0, b.1 - a.1);
    let (sx, sy) = (d.0 - c.0, d.1 - c.1);
    let denom = rx * sy - ry * sx;
    let (qpx, qpy) = (c.0 - a.0, c.1 - a.1);
    if denom.abs() > 1e-9 {
        let t = (qpx * sy - qpy * sx) / denom;
        let u = (qpx * ry - qpy * rx) / denom;
        if (-ARRANGEMENT_EPS..=1.0 + ARRANGEMENT_EPS).contains(&u) {
            return vec![t];
        }
        return Vec::new();
    }
    // Parallel: only a collinear one can touch, and then its endpoints are the split points.
    if (qpx * ry - qpy * rx).abs() > 1e-6 {
        return Vec::new();
    }
    let len_sq = rx * rx + ry * ry;
    [c, d]
        .iter()
        .map(|p| ((p.0 - a.0) * rx + (p.1 - a.1) * ry) / len_sq)
        .collect()
}
