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
        .filter(|(i, l)| l.sketch == sketch && line_alive(doc, *i))
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
/// is stored as that line's `Start`/`End`.
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

    let mut vertices = Vec::with_capacity(lines.len());
    for i in 0..lines.len() {
        let prev = (i + lines.len() - 1) % lines.len();
        let (prev_start, prev_end) = keys[prev];
        let (start, end) = keys[i];
        let entry_uv = if start == prev_start || start == prev_end {
            let line = doc.lines.get(lines[i])?;
            (line.x0, line.y0)
        } else if end == prev_start || end == prev_end {
            let line = doc.lines.get(lines[i])?;
            (line.x1, line.y1)
        } else {
            return None;
        };
        vertices.push(entry_uv);
    }
    Some(vertices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Constraint, ConstraintEntity, ConstraintKind, Line};

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
}
