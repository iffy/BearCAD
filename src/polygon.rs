//! Closed-polygon face detection (#66): any set of plain `Line` entities that connect
//! end-to-end into a closed loop (via `Coincident` point constraints) can be used as a
//! face, the same way a `Rect` or `Circle` profile can.

use crate::document_lifecycle::line_alive;
use crate::model::{ConstraintPoint, Document, LineEnd, SketchId};
use crate::vertex_drag::coincident_group;

/// Canonical id for the vertex group a line endpoint belongs to: the lexicographically
/// smallest `(line, is_end)` among every `LineEndpoint` transitively coincident with it
/// (via `Coincident` constraints). Two endpoints share a vertex iff this key matches.
fn vertex_key(doc: &Document, sketch: SketchId, line: usize, end: LineEnd) -> (usize, bool) {
    coincident_group(doc, sketch, ConstraintPoint::LineEndpoint { line, end })
        .into_iter()
        .filter_map(|p| match p {
            ConstraintPoint::LineEndpoint { line, end } => {
                Some((line, matches!(end, LineEnd::End)))
            }
            _ => None,
        })
        .min()
        .unwrap_or((line, matches!(end, LineEnd::End)))
}

/// Every closed loop of connected `Line`s in `sketch`, as ordered line indices.
///
/// A loop is any simple cycle in the graph whose nodes are vertex groups and whose edges
/// are lines (no line repeated within a loop). Loops are deduped by their line-index set
/// (so the same polygon found by walking it in either direction, or starting from a
/// different line, is reported once), and returned in a deterministic order: sorted by
/// their lowest-numbered line, then by length.
pub fn closed_line_loops(doc: &Document, sketch: SketchId) -> Vec<Vec<usize>> {
    let lines: Vec<usize> = doc
        .lines
        .iter()
        .enumerate()
        // Shadow lines (#224, consumed by an in-sketch slice) keep existing for editing but no
        // longer form faces — their split fragments do.
        .filter(|(i, l)| l.sketch == sketch && !l.shadow && line_alive(doc, *i))
        .map(|(i, _)| i)
        .collect();
    if lines.len() < 3 {
        return Vec::new();
    }

    // For each line, the vertex key at its start and end.
    let endpoints: std::collections::HashMap<usize, ((usize, bool), (usize, bool))> = lines
        .iter()
        .map(|&i| {
            (
                i,
                (
                    vertex_key(doc, sketch, i, LineEnd::Start),
                    vertex_key(doc, sketch, i, LineEnd::End),
                ),
            )
        })
        .collect();

    // Lines incident to each vertex key, paired with which of their own endpoints sits there.
    let mut incident: std::collections::HashMap<(usize, bool), Vec<(usize, bool)>> =
        std::collections::HashMap::new();
    for (&line, &(start_key, end_key)) in &endpoints {
        incident.entry(start_key).or_default().push((line, false));
        incident.entry(end_key).or_default().push((line, true));
    }

    let mut found: Vec<Vec<usize>> = Vec::new();
    let mut seen_sets: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();

    for &start_line in &lines {
        // Walk from `start_line`'s end vertex, looking for a path back to its start vertex.
        let mut path = vec![start_line];
        let mut used: std::collections::HashSet<usize> = std::collections::HashSet::new();
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

    found.sort_by(|a, b| {
        let min_a = *a.iter().min().unwrap();
        let min_b = *b.iter().min().unwrap();
        min_a.cmp(&min_b).then(a.len().cmp(&b.len()))
    });
    found
}

fn walk(
    incident: &std::collections::HashMap<(usize, bool), Vec<(usize, bool)>>,
    endpoints: &std::collections::HashMap<usize, ((usize, bool), (usize, bool))>,
    current: (usize, bool),
    path: &mut Vec<usize>,
    used: &mut std::collections::HashSet<usize>,
    found: &mut Vec<Vec<usize>>,
    seen_sets: &mut std::collections::HashSet<Vec<usize>>,
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
                let mut set: Vec<usize> = path.clone();
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
pub fn loop_vertices_uv(doc: &Document, sketch: SketchId, lines: &[usize]) -> Option<Vec<(f32, f32)>> {
    if lines.len() < 3 {
        return None;
    }
    let keys: Vec<((usize, bool), (usize, bool))> = lines
        .iter()
        .map(|&i| {
            (
                vertex_key(doc, sketch, i, LineEnd::Start),
                vertex_key(doc, sketch, i, LineEnd::End),
            )
        })
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
    lines: &[usize],
) -> Option<Vec<(f32, f32)>> {
    if lines.len() < 3 {
        return None;
    }
    let keys: Vec<((usize, bool), (usize, bool))> = lines
        .iter()
        .map(|&i| {
            (
                vertex_key(doc, sketch, i, LineEnd::Start),
                vertex_key(doc, sketch, i, LineEnd::End),
            )
        })
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
pub(crate) fn point_in_polygon_2d(p: (f32, f32), vertices: &[(f32, f32)]) -> bool {
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
    use super::*;
    use crate::model::{Constraint, ConstraintEntity, ConstraintKind, Line};

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
            deleted: false,
        }
    }

    fn line(sketch: SketchId, x0: f32, y0: f32, x1: f32, y1: f32) -> Line {
        Line::from_local_endpoints(sketch, x0, y0, x1, y1)
    }

    fn point(line: usize, end: LineEnd) -> ConstraintPoint {
        ConstraintPoint::LineEndpoint { line, end }
    }

    #[test]
    fn three_lines_closed_into_a_triangle_form_one_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        // Three lines, each one's end coincident with the next one's start, closing back.
        doc.lines.push(line(0, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(line(0, 10.0, 0.0, 5.0, 8.0));
        doc.lines.push(line(0, 5.0, 8.0, 0.0, 0.0));
        doc.constraints.push(coincident(
            0,
            point(0, LineEnd::End),
            point(1, LineEnd::Start),
        ));
        doc.constraints.push(coincident(
            0,
            point(1, LineEnd::End),
            point(2, LineEnd::Start),
        ));
        doc.constraints.push(coincident(
            0,
            point(2, LineEnd::End),
            point(0, LineEnd::Start),
        ));

        let loops = closed_line_loops(&doc, 0);
        assert_eq!(loops.len(), 1);
        let mut sorted = loops[0].clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2]);
    }

    #[test]
    fn open_chain_of_lines_has_no_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines.push(line(0, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(line(0, 10.0, 0.0, 5.0, 8.0));
        doc.constraints.push(coincident(
            0,
            point(0, LineEnd::End),
            point(1, LineEnd::Start),
        ));

        assert!(closed_line_loops(&doc, 0).is_empty());
    }

    #[test]
    fn unconnected_lines_form_no_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines.push(line(0, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(line(0, 100.0, 0.0, 110.0, 0.0));
        doc.lines.push(line(0, 200.0, 0.0, 210.0, 0.0));

        assert!(closed_line_loops(&doc, 0).is_empty());
    }

    #[test]
    fn deleted_line_does_not_participate_in_a_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines.push(line(0, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(line(0, 10.0, 0.0, 5.0, 8.0));
        doc.lines.push(line(0, 5.0, 8.0, 0.0, 0.0));
        doc.lines[2].deleted = true;
        doc.constraints.push(coincident(
            0,
            point(0, LineEnd::End),
            point(1, LineEnd::Start),
        ));
        doc.constraints.push(coincident(
            0,
            point(1, LineEnd::End),
            point(2, LineEnd::Start),
        ));
        doc.constraints.push(coincident(
            0,
            point(2, LineEnd::End),
            point(0, LineEnd::Start),
        ));

        assert!(closed_line_loops(&doc, 0).is_empty());
    }

    #[test]
    fn four_lines_closed_into_a_quad_form_one_loop() {
        let mut doc = Document::default();
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        doc.lines.push(line(0, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(line(0, 10.0, 0.0, 10.0, 10.0));
        doc.lines.push(line(0, 10.0, 10.0, 0.0, 10.0));
        doc.lines.push(line(0, 0.0, 10.0, 0.0, 0.0));
        for i in 0..4 {
            doc.constraints.push(coincident(
                0,
                point(i, LineEnd::End),
                point((i + 1) % 4, LineEnd::Start),
            ));
        }

        let loops = closed_line_loops(&doc, 0);
        assert_eq!(loops.len(), 1);
        let mut sorted = loops[0].clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3]);
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
        doc.add_sketch(crate::model::FaceId::ConstructionPlane(0));
        // Outer quad edges 0..3
        doc.lines.push(line(0, 0.0, 0.0, 10.0, 0.0)); // A-B
        doc.lines.push(line(0, 10.0, 0.0, 10.0, 10.0)); // B-C
        doc.lines.push(line(0, 10.0, 10.0, 0.0, 10.0)); // C-D
        doc.lines.push(line(0, 0.0, 10.0, 0.0, 0.0)); // D-A
        // Inner concave loop edges 4..7
        doc.lines.push(line(0, 0.0, 0.0, 10.0, 5.0)); // A-P
        doc.lines.push(line(0, 10.0, 5.0, 6.0, 8.0)); // P-E
        doc.lines.push(line(0, 6.0, 8.0, 2.0, 6.0)); // E-F
        doc.lines.push(line(0, 2.0, 6.0, 0.0, 0.0)); // F-A
        doc.constraints.push(coincident(0, point(0, LineEnd::End), point(1, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(1, LineEnd::End), point(2, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(2, LineEnd::End), point(3, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(3, LineEnd::End), point(0, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(4, LineEnd::End), point(1, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(4, LineEnd::Start), point(0, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(5, LineEnd::End), point(6, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(6, LineEnd::End), point(7, LineEnd::Start)));
        doc.constraints.push(coincident(0, point(7, LineEnd::End), point(4, LineEnd::Start)));

        let loops = closed_line_loops(&doc, 0);
        assert!(loops.len() >= 2, "expected outer and inner loops, got {loops:?}");
        let inner = loops
            .iter()
            .find(|l| l.len() == 4 && l.contains(&4))
            .expect("inner concave loop");
        let uv = loop_vertices_uv(&doc, 0, inner).unwrap();
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
