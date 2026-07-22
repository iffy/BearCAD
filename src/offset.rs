//! Pure 2D offset geometry for the sketch Offset operation: parallel copies of line
//! chains (mitered at shared endpoints), cubic-bezier curves (#494), and concentric
//! copies of circles, all in sketch-local (u, v) coordinates.
//!
//! Sign convention: **positive grows** — a closed loop offsets outward, a circle's
//! radius increases; an open chain offsets to the left of its first segment's stored
//! direction (the GUI's push-pull gizmo makes the side visible either way).

use glam::Vec2;

/// A source segment to offset, tagged with its caller-side id (line index).
///
/// When `bezier` is set, the segment is a cubic curve with handles near `a` and `b`
/// (same convention as [`crate::model::Line::bezier`]). Curved sources are offset as
/// independent parallel curves (#494), not chord-mitered as straight segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffsetSource {
    pub id: usize,
    pub a: Vec2,
    pub b: Vec2,
    pub bezier: Option<[Vec2; 2]>,
}

/// An offset output segment, in the same stored orientation as its source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffsetSegment {
    pub id: usize,
    pub a: Vec2,
    pub b: Vec2,
    pub bezier: Option<[Vec2; 2]>,
}

/// Endpoints within this distance chain together (sketch coords are millimetres).
const JOIN_EPS: f32 = 1e-3;
/// A collapsed circle keeps this minimum radius so output indices stay stable.
pub const MIN_CIRCLE_RADIUS: f32 = 0.01;
/// Miter points farther than this many |distance| from the joint fall back to the
/// unmitered offset point (degenerate near-parallel joints).
const MITER_LIMIT: f32 = 100.0;

/// Offset a circle's radius; collapsed circles clamp to [`MIN_CIRCLE_RADIUS`].
pub fn offset_circle_radius(r: f32, distance: f32) -> f32 {
    (r + distance).max(MIN_CIRCLE_RADIUS)
}

fn key(p: Vec2) -> (i64, i64) {
    (
        (p.x / JOIN_EPS).round() as i64,
        (p.y / JOIN_EPS).round() as i64,
    )
}

/// One source segment as walked along a chain (`flipped` when the walk traverses it
/// against its stored orientation).
#[derive(Clone, Copy, Debug)]
struct Walked {
    index: usize,
    flipped: bool,
}

impl Walked {
    fn tail(&self, sources: &[OffsetSource]) -> Vec2 {
        let s = sources[self.index];
        if self.flipped { s.b } else { s.a }
    }
    fn head(&self, sources: &[OffsetSource]) -> Vec2 {
        let s = sources[self.index];
        if self.flipped { s.a } else { s.b }
    }
}

/// Group the sources into chains of segments connected end-to-end. Only points where
/// exactly two segments meet chain together — a T-junction breaks the chain there.
/// Returns (ordered walk, closed).
fn chains(sources: &[OffsetSource]) -> Vec<(Vec<Walked>, bool)> {
    use std::collections::HashMap;
    let mut at_point: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in sources.iter().enumerate() {
        at_point.entry(key(s.a)).or_default().push(i);
        at_point.entry(key(s.b)).or_default().push(i);
    }
    let joinable = |p: Vec2| at_point.get(&key(p)).is_some_and(|v| v.len() == 2);
    let other_at = |p: Vec2, not: usize| -> Option<usize> {
        let v = at_point.get(&key(p))?;
        if v.len() != 2 {
            return None;
        }
        v.iter().copied().find(|&i| i != not)
    };

    let mut used = vec![false; sources.len()];
    let mut result = Vec::new();
    // Open chains first: start from segments with a free (non-joinable) endpoint.
    for start in 0..sources.len() {
        if used[start] {
            continue;
        }
        let s = sources[start];
        let flipped = if !joinable(s.a) {
            false
        } else if !joinable(s.b) {
            true
        } else {
            continue; // interior of a chain or part of a cycle
        };
        let mut walk = vec![Walked { index: start, flipped }];
        used[start] = true;
        loop {
            let last = *walk.last().unwrap();
            let head = last.head(sources);
            let Some(next) = other_at(head, last.index).filter(|&i| !used[i]) else {
                break;
            };
            let flipped = key(sources[next].b) == key(head);
            used[next] = true;
            walk.push(Walked { index: next, flipped });
        }
        result.push((walk, false));
    }
    // Remaining segments are cycles (every endpoint joinable) or isolated pieces.
    for start in 0..sources.len() {
        if used[start] {
            continue;
        }
        let mut walk = vec![Walked { index: start, flipped: false }];
        used[start] = true;
        loop {
            let last = *walk.last().unwrap();
            let head = last.head(sources);
            let Some(next) = other_at(head, last.index).filter(|&i| !used[i]) else {
                break;
            };
            let flipped = key(sources[next].b) == key(head);
            used[next] = true;
            walk.push(Walked { index: next, flipped });
        }
        let closed = walk.len() > 1
            && key(walk.last().unwrap().head(sources)) == key(walk[0].tail(sources));
        result.push((walk, closed));
    }
    result
}

/// Signed area of the walked chain's polygon (positive = counter-clockwise).
fn signed_area(walk: &[Walked], sources: &[OffsetSource]) -> f32 {
    let mut area = 0.0;
    for w in walk {
        let a = w.tail(sources);
        let b = w.head(sources);
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

/// Intersection of two infinite lines (p + t·d); None when near-parallel.
fn line_intersection(p1: Vec2, d1: Vec2, p2: Vec2, d2: Vec2) -> Option<Vec2> {
    let cross = d1.perp_dot(d2);
    if cross.abs() < 1e-6 {
        return None;
    }
    let t = (p2 - p1).perp_dot(d2) / cross;
    Some(p1 + d1 * t)
}

/// Offset a cubic bezier by `distance` to the left of its start→end orientation
/// (or right when `distance` is negative). First-order parallel-curve approx: move
/// endpoints along end normals and keep handle lengths along the end tangents (#494).
pub fn offset_cubic_bezier(
    p0: Vec2,
    c0: Vec2,
    c1: Vec2,
    p1: Vec2,
    distance: f32,
) -> (Vec2, Vec2, Vec2, Vec2) {
    let t0 = (c0 - p0).normalize_or_zero();
    let t1 = (p1 - c1).normalize_or_zero();
    // Fall back to the chord when a handle collapses onto its endpoint.
    let t0 = if t0 == Vec2::ZERO {
        (p1 - p0).normalize_or_zero()
    } else {
        t0
    };
    let t1 = if t1 == Vec2::ZERO {
        (p1 - p0).normalize_or_zero()
    } else {
        t1
    };
    let n0 = Vec2::new(-t0.y, t0.x);
    let n1 = Vec2::new(-t1.y, t1.x);
    let len0 = (c0 - p0).length();
    let len1 = (p1 - c1).length();
    let op0 = p0 + n0 * distance;
    let op1 = p1 + n1 * distance;
    let oc0 = op0 + t0 * len0;
    let oc1 = op1 - t1 * len1;
    (op0, oc0, oc1, op1)
}

/// Offset every source segment by `distance`, mitering shared endpoints so chains and
/// closed loops stay connected. Outputs keep their source's stored orientation and
/// come back in source order. Curved (bezier) sources are offset as independent
/// parallel curves rather than chord-mitered (#494).
pub fn offset_segments(sources: &[OffsetSource], distance: f32) -> Vec<OffsetSegment> {
    let mut out: Vec<Option<OffsetSegment>> = vec![None; sources.len()];

    // Straight sources: existing chain + miter path.
    let straight: Vec<(usize, OffsetSource)> = sources
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, s)| s.bezier.is_none())
        .collect();
    if !straight.is_empty() {
        let straight_only: Vec<OffsetSource> = straight.iter().map(|(_, s)| *s).collect();
        let index_map: Vec<usize> = straight.iter().map(|(i, _)| *i).collect();
        for (walk, closed) in chains(&straight_only) {
            let flip = if closed && signed_area(&walk, &straight_only) > 0.0 {
                -1.0
            } else {
                1.0
            };
            let mut pieces: Vec<(Vec2, Vec2, Vec2)> = Vec::with_capacity(walk.len());
            for w in &walk {
                let a = w.tail(&straight_only);
                let b = w.head(&straight_only);
                let dir = (b - a).normalize_or_zero();
                let normal = Vec2::new(-dir.y, dir.x) * flip;
                let shift = normal * distance;
                pieces.push((a + shift, b + shift, dir));
            }
            let n = pieces.len();
            let joints = if closed { n } else { n.saturating_sub(1) };
            for j in 0..joints {
                let i0 = j;
                let i1 = (j + 1) % n;
                let (a0, b0, d0) = pieces[i0];
                let (a1, _b1, d1) = pieces[i1];
                let miter = line_intersection(a0, d0, a1, d1)
                    .filter(|m| (*m - b0).length() <= MITER_LIMIT * distance.abs().max(JOIN_EPS));
                if let Some(m) = miter {
                    pieces[i0].1 = m;
                    pieces[i1].0 = m;
                }
            }
            for (w, (a, b, _)) in walk.iter().zip(pieces) {
                let (a, b) = if w.flipped { (b, a) } else { (a, b) };
                let src_i = index_map[w.index];
                out[src_i] = Some(OffsetSegment {
                    id: sources[src_i].id,
                    a,
                    b,
                    bezier: None,
                });
            }
        }
    }

    // Curved sources: independent parallel-curve offset (no chord fallback).
    for (i, s) in sources.iter().enumerate() {
        let Some([c0, c1]) = s.bezier else {
            continue;
        };
        let (a, oc0, oc1, b) = offset_cubic_bezier(s.a, c0, c1, s.b, distance);
        out[i] = Some(OffsetSegment {
            id: s.id,
            a,
            b,
            bezier: Some([oc0, oc1]),
        });
    }

    out.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(id: usize, a: (f32, f32), b: (f32, f32)) -> OffsetSource {
        OffsetSource {
            id,
            a: Vec2::new(a.0, a.1),
            b: Vec2::new(b.0, b.1),
            bezier: None,
        }
    }

    fn src_curve(
        id: usize,
        a: (f32, f32),
        c0: (f32, f32),
        c1: (f32, f32),
        b: (f32, f32),
    ) -> OffsetSource {
        OffsetSource {
            id,
            a: Vec2::new(a.0, a.1),
            b: Vec2::new(b.0, b.1),
            bezier: Some([Vec2::new(c0.0, c0.1), Vec2::new(c1.0, c1.1)]),
        }
    }

    fn close(p: Vec2, q: (f32, f32)) -> bool {
        (p - Vec2::new(q.0, q.1)).length() < 1e-3
    }

    #[test]
    fn single_line_offsets_to_its_left() {
        let out = offset_segments(&[src(0, (0.0, 0.0), (10.0, 0.0))], 2.0);
        assert_eq!(out.len(), 1);
        assert!(close(out[0].a, (0.0, 2.0)) && close(out[0].b, (10.0, 2.0)), "{out:?}");
        let out = offset_segments(&[src(0, (0.0, 0.0), (10.0, 0.0))], -2.0);
        assert!(close(out[0].a, (0.0, -2.0)) && close(out[0].b, (10.0, -2.0)), "{out:?}");
    }

    #[test]
    fn l_chain_miters_the_shared_corner() {
        // (0,0)→(10,0) then up to (10,10); positive = left of the first segment (+v),
        // mitered corner at the intersection of the two offset lines.
        let out = offset_segments(
            &[src(0, (0.0, 0.0), (10.0, 0.0)), src(1, (10.0, 0.0), (10.0, 10.0))],
            2.0,
        );
        assert_eq!(out.len(), 2);
        assert!(close(out[0].a, (0.0, 2.0)), "{out:?}");
        assert!(close(out[0].b, (8.0, 2.0)), "corner should miter, {out:?}");
        assert!(close(out[1].a, (8.0, 2.0)) && close(out[1].b, (8.0, 10.0)), "{out:?}");
    }

    #[test]
    fn reversed_segment_keeps_its_stored_orientation() {
        // Second line stored head-first; the chain still miters and the output keeps
        // the stored a/b order.
        let out = offset_segments(
            &[src(0, (0.0, 0.0), (10.0, 0.0)), src(1, (10.0, 10.0), (10.0, 0.0))],
            2.0,
        );
        assert!(close(out[1].a, (8.0, 10.0)) && close(out[1].b, (8.0, 2.0)), "{out:?}");
    }

    #[test]
    fn closed_square_grows_outward_for_positive_distance() {
        // CCW square; positive must offset outward regardless of winding.
        let square = [
            src(0, (0.0, 0.0), (10.0, 0.0)),
            src(1, (10.0, 0.0), (10.0, 10.0)),
            src(2, (10.0, 10.0), (0.0, 10.0)),
            src(3, (0.0, 10.0), (0.0, 0.0)),
        ];
        let out = offset_segments(&square, 1.0);
        assert_eq!(out.len(), 4);
        assert!(close(out[0].a, (-1.0, -1.0)) && close(out[0].b, (11.0, -1.0)), "{out:?}");
        assert!(close(out[2].a, (11.0, 11.0)) && close(out[2].b, (-1.0, 11.0)), "{out:?}");
        let out = offset_segments(&square, -1.0);
        assert!(close(out[0].a, (1.0, 1.0)) && close(out[0].b, (9.0, 1.0)), "shrink {out:?}");
    }

    #[test]
    fn clockwise_square_also_grows_outward_for_positive_distance() {
        let square = [
            src(0, (0.0, 0.0), (0.0, 10.0)),
            src(1, (0.0, 10.0), (10.0, 10.0)),
            src(2, (10.0, 10.0), (10.0, 0.0)),
            src(3, (10.0, 0.0), (0.0, 0.0)),
        ];
        let out = offset_segments(&square, 1.0);
        assert!(close(out[0].a, (-1.0, -1.0)) && close(out[0].b, (-1.0, 11.0)), "{out:?}");
    }

    #[test]
    fn t_junction_breaks_the_chain() {
        // Three segments meeting at (10,0): no miter there, everyone offsets flat.
        let out = offset_segments(
            &[
                src(0, (0.0, 0.0), (10.0, 0.0)),
                src(1, (10.0, 0.0), (20.0, 0.0)),
                src(2, (10.0, 0.0), (10.0, 10.0)),
            ],
            2.0,
        );
        assert_eq!(out.len(), 3);
        assert!(close(out[0].b, (10.0, 2.0)), "{out:?}");
        assert!(close(out[1].a, (10.0, 2.0)), "{out:?}");
    }

    #[test]
    fn circle_radius_grows_shrinks_and_clamps() {
        assert!((offset_circle_radius(5.0, 2.0) - 7.0).abs() < 1e-6);
        assert!((offset_circle_radius(5.0, -2.0) - 3.0).abs() < 1e-6);
        assert!((offset_circle_radius(5.0, -9.0) - MIN_CIRCLE_RADIUS).abs() < 1e-6);
    }

    /// #494: a cubic-bezier source must produce a curved offset (handles preserved),
    /// not a straight chord/"chamfer-style" segment.
    #[test]
    fn curved_line_offset_keeps_bezier_handles() {
        // Horizontal-ish S-curve: endpoints on y=0, handles pull up then down.
        let out = offset_segments(
            &[src_curve(0, (0.0, 0.0), (10.0, 20.0), (30.0, 20.0), (40.0, 0.0))],
            5.0,
        );
        assert_eq!(out.len(), 1);
        let seg = &out[0];
        let bezier = seg.bezier.expect("offset of a curve must stay a curve");
        // Endpoints moved, not coincident with the source chord offset alone.
        assert!(
            (seg.a - Vec2::new(0.0, 0.0)).length() > 1.0,
            "start should move off the source, got {:?}",
            seg.a
        );
        assert!(
            (bezier[0] - seg.a).length() > 1.0 && (bezier[1] - seg.b).length() > 1.0,
            "handles must remain off the endpoints so the offset is curved: {seg:?}"
        );
        // A pure chord offset of the endpoints would be a straight line; the mid
        // sample of the offset curve must leave that chord.
        let mid = {
            let t = 0.5f32;
            let p0 = seg.a;
            let p1 = bezier[0];
            let p2 = bezier[1];
            let p3 = seg.b;
            let u = 1.0 - t;
            p0 * u.powi(3) + p1 * 3.0 * u.powi(2) * t + p2 * 3.0 * u * t.powi(2) + p3 * t.powi(3)
        };
        let chord_mid = (seg.a + seg.b) * 0.5;
        assert!(
            (mid - chord_mid).length() > 2.0,
            "offset must bow (not be a straight chamfer), mid={mid:?} chord_mid={chord_mid:?}"
        );
    }
}
