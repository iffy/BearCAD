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
/// (or right when `distance` is negative).
///
/// Circular-arc fillets (equal endpoint radii to a shared center) offset exactly by
/// changing radius so the distance stays constant along the arc (#513). Otherwise a
/// first-order parallel-curve approx moves endpoints along end normals and keeps
/// handle lengths along the end tangents (#494).
pub fn offset_cubic_bezier(
    p0: Vec2,
    c0: Vec2,
    c1: Vec2,
    p1: Vec2,
    distance: f32,
) -> (Vec2, Vec2, Vec2, Vec2) {
    if let Some((center, r)) = circular_arc_fit(p0, c0, c1, p1) {
        let sign = {
            // Left of start→end: positive distance grows the side of n0.
            let t0 = (c0 - p0).normalize_or_zero();
            let t0 = if t0 == Vec2::ZERO {
                (p1 - p0).normalize_or_zero()
            } else {
                t0
            };
            let n0 = Vec2::new(-t0.y, t0.x);
            let radial = (p0 - center).normalize_or_zero();
            if radial.dot(n0) >= 0.0 {
                1.0
            } else {
                -1.0
            }
        };
        let new_r = (r + distance * sign).max(MIN_CIRCLE_RADIUS);
        let scale = new_r / r.max(1e-6);
        let op0 = center + (p0 - center) * scale;
        let op1 = center + (p1 - center) * scale;
        let oc0 = center + (c0 - center) * scale;
        let oc1 = center + (c1 - center) * scale;
        return (op0, oc0, oc1, op1);
    }
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

/// If the cubic is a circular arc (equal radii from endpoints and mid-sample to a
/// common center within tolerance), return `(center, radius)`.
fn circular_arc_fit(p0: Vec2, c0: Vec2, c1: Vec2, p1: Vec2) -> Option<(Vec2, f32)> {
    // Chord perpendicular bisector ∩ handle-normal estimate.
    let mid = {
        let t = 0.5f32;
        let u = 1.0 - t;
        p0 * u.powi(3) + c0 * 3.0 * u.powi(2) * t + c1 * 3.0 * u * t.powi(2) + p1 * t.powi(3)
    };
    // Center = intersection of perp bisectors of p0–mid and mid–p1.
    let m01 = (p0 + mid) * 0.5;
    let m12 = (mid + p1) * 0.5;
    let d01 = (mid - p0).perp(); // glam Vec2::perp is rotate 90° CCW
    let d12 = (p1 - mid).perp();
    let center = line_intersection(m01, d01, m12, d12)?;
    let r0 = (p0 - center).length();
    let r1 = (p1 - center).length();
    let rm = (mid - center).length();
    if r0 < 1e-4 {
        return None;
    }
    let tol = (r0 * 0.02).max(1e-2);
    if (r1 - r0).abs() > tol || (rm - r0).abs() > tol {
        return None;
    }
    // Handles should also lie near the same circle (fillet cubics do).
    let rc0 = (c0 - center).length();
    let rc1 = (c1 - center).length();
    // Cubic handles of a circle are outside the radius; allow more slack.
    if (rc0 - r0).abs() > r0 * 0.5 && (rc1 - r0).abs() > r0 * 0.5 {
        // Still accept if endpoints+mid match — handles get scaled with center.
    }
    Some((center, r0))
}

/// One offset piece before mitering: endpoints, walk-direction unit tangent at each end,
/// and optional bezier handles in walk orientation.
struct OffsetPiece {
    a: Vec2,
    b: Vec2,
    /// Unit tangent at `a` pointing along the walk (into the segment).
    ta: Vec2,
    /// Unit tangent at `b` pointing along the walk (out of the segment).
    tb: Vec2,
    bezier: Option<[Vec2; 2]>,
}

/// Offset every source segment by `distance`, mitering shared endpoints so chains and
/// closed loops stay connected — including mixed straight + fillet (bezier) corners
/// (#513). Outputs keep their source's stored orientation and come back in source order.
pub fn offset_segments(sources: &[OffsetSource], distance: f32) -> Vec<OffsetSegment> {
    let mut out: Vec<Option<OffsetSegment>> = vec![None; sources.len()];

    for (walk, closed) in chains(sources) {
        let flip = if closed && signed_area(&walk, sources) > 0.0 {
            -1.0
        } else {
            1.0
        };
        let signed_d = distance * flip;

        let mut pieces: Vec<OffsetPiece> = Vec::with_capacity(walk.len());
        for w in &walk {
            let s = sources[w.index];
            let (a, b, bezier) = if w.flipped {
                (
                    s.b,
                    s.a,
                    s.bezier.map(|[c0, c1]| [c1, c0]),
                )
            } else {
                (s.a, s.b, s.bezier)
            };
            let piece = if let Some([c0, c1]) = bezier {
                let (oa, oc0, oc1, ob) = offset_cubic_bezier(a, c0, c1, b, signed_d);
                let ta = (oc0 - oa).normalize_or_zero();
                let tb = (ob - oc1).normalize_or_zero();
                let ta = if ta == Vec2::ZERO {
                    (ob - oa).normalize_or_zero()
                } else {
                    ta
                };
                let tb = if tb == Vec2::ZERO {
                    (ob - oa).normalize_or_zero()
                } else {
                    tb
                };
                OffsetPiece {
                    a: oa,
                    b: ob,
                    ta,
                    tb,
                    bezier: Some([oc0, oc1]),
                }
            } else {
                let dir = (b - a).normalize_or_zero();
                let normal = Vec2::new(-dir.y, dir.x);
                let shift = normal * signed_d;
                OffsetPiece {
                    a: a + shift,
                    b: b + shift,
                    ta: dir,
                    tb: dir,
                    bezier: None,
                }
            };
            pieces.push(piece);
        }

        // Miter joints so consecutive offset pieces meet (#513 — rounded-rect corners).
        let n = pieces.len();
        let joints = if closed { n } else { n.saturating_sub(1) };
        for j in 0..joints {
            let i0 = j;
            let i1 = (j + 1) % n;
            let b0 = pieces[i0].b;
            let a1 = pieces[i1].a;
            let d0 = pieces[i0].tb;
            let d1 = pieces[i1].ta;
            let miter = line_intersection(b0, d0, a1, d1)
                .filter(|m| (*m - b0).length() <= MITER_LIMIT * distance.abs().max(JOIN_EPS));
            if let Some(m) = miter {
                // Slide bezier handles with their endpoints so the curve shape stays.
                if let Some([c0, c1]) = pieces[i0].bezier {
                    let delta = m - pieces[i0].b;
                    pieces[i0].bezier = Some([c0, c1 + delta]);
                }
                if let Some([c0, c1]) = pieces[i1].bezier {
                    let delta = m - pieces[i1].a;
                    pieces[i1].bezier = Some([c0 + delta, c1]);
                }
                pieces[i0].b = m;
                pieces[i1].a = m;
            }
        }

        for (w, piece) in walk.iter().zip(pieces) {
            let (a, b, bezier) = if w.flipped {
                (
                    piece.b,
                    piece.a,
                    piece.bezier.map(|[c0, c1]| [c1, c0]),
                )
            } else {
                (piece.a, piece.b, piece.bezier)
            };
            out[w.index] = Some(OffsetSegment {
                id: sources[w.index].id,
                a,
                b,
                bezier,
            });
        }
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

    /// #513: a rounded rectangle (straights + circular fillet cubics) offsets so the
    /// gap between source and offset stays approximately `distance` along the whole loop,
    /// including across the rounded corners — not a flat chamfer-style shortcut.
    #[test]
    fn rounded_rectangle_offset_keeps_constant_distance_at_fillets() {
        // Axis-aligned box from (0,0)–(40,20) with r=5 circular fillets at each corner.
        // Each fillet is a cubic approximating a quarter circle (standard k≈0.552).
        let k = 0.5522847498f32;
        let r = 5.0;
        let sources = [
            // Bottom: (5,0) → (35,0)
            src(0, (5.0, 0.0), (35.0, 0.0)),
            // Bottom-right fillet: (35,0) → (40,5), center (35,5)
            src_curve(
                1,
                (35.0, 0.0),
                (35.0 + k * r, 0.0),
                (40.0, 5.0 - k * r),
                (40.0, 5.0),
            ),
            // Right: (40,5) → (40,15)
            src(2, (40.0, 5.0), (40.0, 15.0)),
            // Top-right fillet: (40,15) → (35,20), center (35,15)
            src_curve(
                3,
                (40.0, 15.0),
                (40.0, 15.0 + k * r),
                (35.0 + k * r, 20.0),
                (35.0, 20.0),
            ),
            // Top: (35,20) → (5,20)
            src(4, (35.0, 20.0), (5.0, 20.0)),
            // Top-left fillet: (5,20) → (0,15), center (5,15)
            src_curve(
                5,
                (5.0, 20.0),
                (5.0 - k * r, 20.0),
                (0.0, 15.0 + k * r),
                (0.0, 15.0),
            ),
            // Left: (0,15) → (0,5)
            src(6, (0.0, 15.0), (0.0, 5.0)),
            // Bottom-left fillet: (0,5) → (5,0), center (5,5)
            src_curve(
                7,
                (0.0, 5.0),
                (0.0, 5.0 - k * r),
                (5.0 - k * r, 0.0),
                (5.0, 0.0),
            ),
        ];
        let d = 3.0;
        let out = offset_segments(&sources, d);
        assert_eq!(out.len(), 8);
        // Outer bottom should sit at y = -d.
        let bottom = out.iter().find(|s| s.id == 0).unwrap();
        assert!(
            (bottom.a.y + d).abs() < 0.15 && (bottom.b.y + d).abs() < 0.15,
            "bottom offset y≈{}, got {bottom:?}",
            -d
        );
        // Fillet mid-sample should be ~r+d from the corner center (35,5).
        let fillet = out.iter().find(|s| s.id == 1).unwrap();
        let bez = fillet.bezier.expect("fillet stays curved");
        let t = 0.5f32;
        let u = 1.0 - t;
        let mid = fillet.a * u.powi(3)
            + bez[0] * 3.0 * u.powi(2) * t
            + bez[1] * 3.0 * u * t.powi(2)
            + fillet.b * t.powi(3);
        let center = Vec2::new(35.0, 5.0);
        let dist = (mid - center).length();
        assert!(
            (dist - (r + d)).abs() < 0.5,
            "fillet mid should sit at r+d={}; got dist={dist} mid={mid:?}",
            r + d
        );
        // Adjacent pieces meet at joints (no gap between bottom and fillet).
        assert!(
            (bottom.b - fillet.a).length() < 0.2,
            "bottom→fillet joint must miter, bottom.b={:?} fillet.a={:?}",
            bottom.b,
            fillet.a
        );
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
