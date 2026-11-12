//! Sketch snapping: while drawing or dragging, snap a cursor point to nearby geometry
//! (other vertices, line midpoints, or anywhere along a line) and, when the user leaves a
//! point on a snap, record the constraint that should pin it there.

use crate::geometric_constraints::{line_uv_endpoints, point_uv};
use crate::model::{
    ConstraintEntity, ConstraintKind, ConstraintLine, ConstraintPoint, Document, LineEnd,
    SketchAxis, SketchId,
};

/// What a snapped point latched onto, and the constraint to add if it is left there.
#[derive(Clone, Debug, PartialEq)]
pub enum SnapTarget {
    /// Coincident with another vertex.
    Vertex(ConstraintPoint),
    /// Coincident with the sketch origin (local UV `(0, 0)`).
    Origin,
    /// On the midpoint of a line/edge.
    Midpoint(ConstraintLine),
    /// Anywhere along a line/edge.
    OnLine(ConstraintLine),
    /// On the *infinite extension* of a line/edge (inference snapping): the point lies on the
    /// edge's line but beyond its endpoints. Pinned with a point-on-line coincidence, just like
    /// [`SnapTarget::OnLine`], so the point stays collinear with the edge.
    OnLineExtension(ConstraintLine),
    /// On the rim of a **circular** body face the sketch sits on (#1858). The face's
    /// boundary is stored as 48 chords; those are never offered as snap lines, so the rim
    /// snaps as one true circle instead of a ring of chord midpoints. Pinned with a
    /// point-on-circle coincidence against the face's analytic centre and radius.
    OnFaceCircle(crate::model::FaceId),
    /// On the rim of a sketch circle (#1977). Pinned with a point-on-circle coincidence.
    OnCircle(crate::model::CircleKey),
    /// Where two curves (lines, origin axes, circles) cross (#1977). Pinned with a
    /// point-on-curve coincidence against each.
    Intersection(ConstraintEntity, ConstraintEntity),
    /// The infinite line normal to `line`, through its midpoint (inference snapping: only
    /// reachable after first touching that line's midpoint — see `AppState`'s remembered anchor,
    /// mirroring `OnLineExtension`'s vertex-touch mechanic for #21). Committing this snap invents
    /// a construction line to carry the constraint (see `crate::actions::add_snap_constraint`).
    NormalAtMidpoint(ConstraintLine),
}

/// A resolved snap: where to place the point and what it latched onto.
#[derive(Clone, Debug, PartialEq)]
pub struct Snap {
    pub uv: (f32, f32),
    pub target: SnapTarget,
}

/// Find the best snap for `query` within `radius` (sketch units), ignoring `exclude` vertices
/// (and the lines/rects that own them, so a dragged point never snaps to its own geometry).
/// Vertices win over midpoints, which win over generic on-line snaps; ties break on distance.
pub fn find_snap(
    doc: &Document,
    sketch: SketchId,
    query: (f32, f32),
    radius: f32,
    exclude: &[ConstraintPoint],
) -> Option<Snap> {
    if radius <= 0.0 {
        return None;
    }
    let radius_sq = radius * radius;

    // Nearest vertex (highest priority).
    let mut best_vertex: Option<(f32, Snap)> = None;
    for point in sketch_vertices(doc, sketch) {
        if exclude.contains(&point) {
            continue;
        }
        let Ok((u, v)) = point_uv(doc, sketch, point.clone()) else {
            continue;
        };
        let d2 = dist_sq(query, (u, v));
        if d2 <= radius_sq && best_vertex.as_ref().is_none_or(|(b, _)| d2 < *b) {
            best_vertex = Some((
                d2,
                Snap {
                    uv: (u, v),
                    target: SnapTarget::Vertex(point),
                },
            ));
        }
    }
    // The sketch origin is always snappable as a vertex. A real vertex at the same
    // distance wins (it produces a richer point-to-point coincidence).
    {
        let d2 = dist_sq(query, (0.0, 0.0));
        if d2 <= radius_sq && best_vertex.as_ref().is_none_or(|(b, _)| d2 < *b) {
            best_vertex = Some((
                d2,
                Snap {
                    uv: (0.0, 0.0),
                    target: SnapTarget::Origin,
                },
            ));
        }
    }
    if let Some((_, snap)) = best_vertex {
        return Some(snap);
    }

    let excluded_lines: Vec<ConstraintLine> = exclude
        .iter()
        .flat_map(owning_lines)
        .collect();
    let excluded_circles: Vec<crate::model::CircleKey> = exclude
        .iter()
        .filter_map(|p| match p {
            ConstraintPoint::CircleCenter(c) => Some(*c),
            _ => None,
        })
        .collect();

    // Crossings of two curves outrank midpoints and on-curve snaps (#1977).
    if let Some(snap) =
        nearest_intersection(doc, sketch, query, radius, &excluded_lines, &excluded_circles)
    {
        return Some(snap);
    }

    // Nearest line midpoint (next priority).
    let mut best_mid: Option<(f32, Snap)> = None;
    let mut best_on_line: Option<(f32, Snap)> = None;
    for line in sketch_lines(doc, sketch) {
        if excluded_lines.contains(&line) {
            continue;
        }
        let Ok(((x0, y0), (x1, y1))) = line_uv_endpoints(doc, sketch, line.clone()) else {
            continue;
        };
        let mid = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let dm = dist_sq(query, mid);
        if dm <= radius_sq && best_mid.as_ref().is_none_or(|(b, _)| dm < *b) {
            best_mid = Some((
                dm,
                Snap {
                    uv: mid,
                    target: SnapTarget::Midpoint(line.clone()),
                },
            ));
        }

        if let Some(foot) = project_onto_segment(query, (x0, y0), (x1, y1)) {
            let dl = dist_sq(query, foot);
            if dl <= radius_sq && best_on_line.as_ref().is_none_or(|(b, _)| dl < *b) {
                best_on_line = Some((
                    dl,
                    Snap {
                        uv: foot,
                        target: SnapTarget::OnLine(line),
                    },
                ));
            }
        }
    }
    // The sketch's own origin axes are always snappable (point-on-axis, #189): the X axis is
    // the line v = 0, the Y axis is u = 0. Treated at on-line priority, so a real vertex,
    // the origin, and line midpoints still win.
    for (axis, foot, d2) in [
        (SketchAxis::X, (query.0, 0.0), query.1 * query.1),
        (SketchAxis::Y, (0.0, query.1), query.0 * query.0),
    ] {
        if d2 <= radius_sq && best_on_line.as_ref().is_none_or(|(b, _)| d2 < *b) {
            best_on_line = Some((
                d2,
                Snap {
                    uv: foot,
                    target: SnapTarget::OnLine(ConstraintLine::OriginAxis(axis)),
                },
            ));
        }
    }
    // Sketch circles' rims (#1977).
    for (key, circle) in doc.circles.iter() {
        if circle.sketch != sketch || excluded_circles.contains(&key) {
            continue;
        }
        let d = dist_sq(query, (circle.cx, circle.cy)).sqrt();
        if d < 1e-6 {
            continue;
        }
        let scale = circle.r / d;
        let foot = (
            circle.cx + (query.0 - circle.cx) * scale,
            circle.cy + (query.1 - circle.cy) * scale,
        );
        let d2 = dist_sq(query, foot);
        if d2 <= radius_sq && best_on_line.as_ref().is_none_or(|(b, _)| d2 < *b) {
            best_on_line = Some((
                d2,
                Snap {
                    uv: foot,
                    target: SnapTarget::OnCircle(key),
                },
            ));
        }
    }
    // The rim of a circular body face the sketch sits on (#1858): the true circle, so the
    // point lands exactly on it rather than on one of the 48 chords that approximate it.
    if let Some((face, (cu, cv), radius)) = sketched_on_face_circle(doc, sketch) {
        let d = dist_sq(query, (cu, cv)).sqrt();
        if d > 1e-6 {
            let scale = radius / d;
            let foot = (cu + (query.0 - cu) * scale, cv + (query.1 - cv) * scale);
            let d2 = dist_sq(query, foot);
            if d2 <= radius_sq && best_on_line.as_ref().is_none_or(|(b, _)| d2 < *b) {
                best_on_line = Some((
                    d2,
                    Snap {
                        uv: foot,
                        target: SnapTarget::OnFaceCircle(face),
                    },
                ));
            }
        }
    }
    if let Some((_, snap)) = best_mid {
        return Some(snap);
    }
    best_on_line.map(|(_, snap)| snap)
}

enum Curve {
    Segment(ConstraintEntity, (f32, f32), (f32, f32)),
    Axis(SketchAxis),
    Circle(crate::model::CircleKey, (f32, f32), f32),
}

impl Curve {
    fn entity(&self) -> ConstraintEntity {
        match self {
            Curve::Segment(e, ..) => e.clone(),
            Curve::Axis(a) => ConstraintEntity::Line(ConstraintLine::OriginAxis(*a)),
            Curve::Circle(k, ..) => ConstraintEntity::Circle(*k),
        }
    }

    fn distance_to(&self, p: (f32, f32)) -> f32 {
        match self {
            Curve::Segment(_, a, b) => project_onto_segment(p, *a, *b)
                .map_or(f32::INFINITY, |f| dist_sq(p, f).sqrt()),
            Curve::Axis(SketchAxis::X) => p.1.abs(),
            Curve::Axis(SketchAxis::Y) => p.0.abs(),
            Curve::Circle(_, c, r) => (dist_sq(p, *c).sqrt() - r).abs(),
        }
    }
}

/// Points where two curves cross. Segments are finite; axes are infinite.
fn curve_crossings(a: &Curve, b: &Curve) -> Vec<(f32, f32)> {
    const BIG: f32 = 1.0e6;
    let as_line = |c: &Curve| match c {
        Curve::Segment(_, p, q) => Some((*p, *q, false)),
        Curve::Axis(SketchAxis::X) => Some(((-BIG, 0.0), (BIG, 0.0), true)),
        Curve::Axis(SketchAxis::Y) => Some(((0.0, -BIG), (0.0, BIG), true)),
        Curve::Circle(..) => None,
    };
    match (as_line(a), as_line(b), a, b) {
        (Some((p, q, _)), Some((r, s, _)), ..) => {
            let (d1, d2) = ((q.0 - p.0, q.1 - p.1), (s.0 - r.0, s.1 - r.1));
            let den = d1.0 * d2.1 - d1.1 * d2.0;
            if den.abs() < 1e-9 {
                return Vec::new();
            }
            let t = ((r.0 - p.0) * d2.1 - (r.1 - p.1) * d2.0) / den;
            let u = ((r.0 - p.0) * d1.1 - (r.1 - p.1) * d1.0) / den;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                vec![(p.0 + d1.0 * t, p.1 + d1.1 * t)]
            } else {
                Vec::new()
            }
        }
        (Some((p, q, _)), None, _, Curve::Circle(_, c, r))
        | (None, Some((p, q, _)), Curve::Circle(_, c, r), _) => {
            // f64: an axis spans ±1e6, where f32 loses the quadratic's cancellation.
            let (px, py, qx, qy) = (p.0 as f64, p.1 as f64, q.0 as f64, q.1 as f64);
            let d = (qx - px, qy - py);
            let f = (px - c.0 as f64, py - c.1 as f64);
            let r = *r as f64;
            let qa = d.0 * d.0 + d.1 * d.1;
            let qb = 2.0 * (f.0 * d.0 + f.1 * d.1);
            let qc = f.0 * f.0 + f.1 * f.1 - r * r;
            let disc = qb * qb - 4.0 * qa * qc;
            if qa < 1e-12 || disc < 0.0 {
                return Vec::new();
            }
            let root = disc.sqrt();
            [(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)]
                .into_iter()
                .filter(|t| (0.0..=1.0).contains(t))
                .map(|t| ((px + d.0 * t) as f32, (py + d.1 * t) as f32))
                .collect()
        }
        (None, None, Curve::Circle(_, c1, r1), Curve::Circle(_, c2, r2)) => {
            let d = dist_sq(*c1, *c2).sqrt();
            if d < 1e-6 || d > r1 + r2 || d < (r1 - r2).abs() {
                return Vec::new();
            }
            let x = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
            let h = (r1 * r1 - x * x).max(0.0).sqrt();
            let ux = ((c2.0 - c1.0) / d, (c2.1 - c1.1) / d);
            let m = (c1.0 + ux.0 * x, c1.1 + ux.1 * x);
            vec![
                (m.0 - ux.1 * h, m.1 + ux.0 * h),
                (m.0 + ux.1 * h, m.1 - ux.0 * h),
            ]
        }
        _ => Vec::new(),
    }
}

/// The crossing of two curves nearest `query` within `radius` (#1977).
fn nearest_intersection(
    doc: &Document,
    sketch: SketchId,
    query: (f32, f32),
    radius: f32,
    excluded_lines: &[ConstraintLine],
    excluded_circles: &[crate::model::CircleKey],
) -> Option<Snap> {
    let mut curves = Vec::new();
    for line in sketch_lines(doc, sketch) {
        if excluded_lines.contains(&line) {
            continue;
        }
        if let Ok((p, q)) = line_uv_endpoints(doc, sketch, line.clone()) {
            curves.push(Curve::Segment(ConstraintEntity::Line(line), p, q));
        }
    }
    curves.push(Curve::Axis(SketchAxis::X));
    curves.push(Curve::Axis(SketchAxis::Y));
    for (key, c) in doc.circles.iter() {
        if c.sketch == sketch && !excluded_circles.contains(&key) {
            curves.push(Curve::Circle(key, (c.cx, c.cy), c.r));
        }
    }
    // Only curves passing near the query can cross near it.
    curves.retain(|c| c.distance_to(query) <= radius * 2.0);
    let mut best: Option<(f32, Snap)> = None;
    for i in 0..curves.len() {
        for j in (i + 1)..curves.len() {
            for pt in curve_crossings(&curves[i], &curves[j]) {
                let d2 = dist_sq(query, pt);
                if d2 <= radius * radius && best.as_ref().is_none_or(|(b, _)| d2 < *b) {
                    best = Some((
                        d2,
                        Snap {
                            uv: pt,
                            target: SnapTarget::Intersection(curves[i].entity(), curves[j].entity()),
                        },
                    ));
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

/// The constraint that pins `point` to `target` when a snapped point is left in place.
///
/// [`SnapTarget::NormalAtMidpoint`] has no single `ConstraintKind` that fits this signature — it
/// needs to invent a construction line first — so it is special-cased before this function is
/// called (see `crate::actions::AppState::add_snap_constraint`) and must never reach here.
pub fn snap_constraint_kind(point: ConstraintPoint, target: SnapTarget) -> ConstraintKind {
    match target {
        SnapTarget::Vertex(other) => ConstraintKind::Coincident {
            a: ConstraintEntity::Point(point),
            b: ConstraintEntity::Point(other),
        },
        SnapTarget::Origin => ConstraintKind::Coincident {
            a: ConstraintEntity::Point(point),
            b: ConstraintEntity::Origin,
        },
        SnapTarget::Midpoint(line) => ConstraintKind::Midpoint { point, line },
        SnapTarget::OnFaceCircle(face) => ConstraintKind::Coincident {
            a: ConstraintEntity::Point(point),
            b: ConstraintEntity::FaceCircle { face },
        },
        SnapTarget::OnLine(line) | SnapTarget::OnLineExtension(line) => ConstraintKind::Coincident {
            a: ConstraintEntity::Point(point),
            b: ConstraintEntity::Line(line),
        },
        SnapTarget::OnCircle(circle) => ConstraintKind::Coincident {
            a: ConstraintEntity::Point(point),
            b: ConstraintEntity::Circle(circle),
        },
        SnapTarget::Intersection(..) => unreachable!(
            "Intersection is pinned by snap_constraint_kinds, not snap_constraint_kind"
        ),
        SnapTarget::NormalAtMidpoint(_) => unreachable!(
            "NormalAtMidpoint is handled by add_normal_at_midpoint_constraint, not snap_constraint_kind"
        ),
    }
}

/// Every constraint that pins `point` to `target`: one for most snaps, two for an
/// [`SnapTarget::Intersection`] (#1977). Not for `NormalAtMidpoint`.
pub fn snap_constraint_kinds(point: ConstraintPoint, target: SnapTarget) -> Vec<ConstraintKind> {
    match target {
        SnapTarget::Intersection(a, b) => [a, b]
            .into_iter()
            .map(|entity| ConstraintKind::Coincident {
                a: ConstraintEntity::Point(point.clone()),
                b: entity,
            })
            .collect(),
        other => vec![snap_constraint_kind(point, other)],
    }
}

/// Whether the constraint a snap would add is already present (avoids duplicates).
pub fn snap_constraint_already_present(
    doc: &Document,
    point: ConstraintPoint,
    target: SnapTarget,
) -> bool {
    snap_constraint_kinds(point, target).iter().all(|kind| {
        doc.constraints
            .values()
            .any(|c| constraint_equivalent(&c.kind, kind))
    })
}

fn constraint_equivalent(a: &ConstraintKind, b: &ConstraintKind) -> bool {
    match (a, b) {
        (
            ConstraintKind::Coincident { a: a1, b: b1 },
            ConstraintKind::Coincident { a: a2, b: b2 },
        ) => (entity_eq(a1, a2) && entity_eq(b1, b2)) || (entity_eq(a1, b2) && entity_eq(b1, a2)),
        (
            ConstraintKind::Midpoint { point: p1, line: l1 },
            ConstraintKind::Midpoint { point: p2, line: l2 },
        ) => p1 == p2 && l1 == l2,
        _ => false,
    }
}

fn entity_eq(a: &ConstraintEntity, b: &ConstraintEntity) -> bool {
    match (a, b) {
        (ConstraintEntity::Point(p), ConstraintEntity::Point(q)) => p == q,
        (ConstraintEntity::Line(l), ConstraintEntity::Line(m)) => l == m,
        (ConstraintEntity::Circle(c), ConstraintEntity::Circle(d)) => c == d,
        (ConstraintEntity::Origin, ConstraintEntity::Origin) => true,
        _ => false,
    }
}

/// Lines/edges owned by a vertex (so dragging it never snaps to its own geometry).
fn owning_lines(point: &ConstraintPoint) -> Vec<ConstraintLine> {
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => vec![ConstraintLine::Line(*line)],
        ConstraintPoint::CircleCenter(_)
        | ConstraintPoint::FaceVertex { .. }
        | ConstraintPoint::FaceCircleCenter { .. }
        | ConstraintPoint::TextAnchor { .. }
        | ConstraintPoint::ImageCalibrationPoint { .. }
        | ConstraintPoint::ImageAnchor { .. }
        | ConstraintPoint::Origin => Vec::new(),
    }
}

/// Every vertex in the document (line endpoints, rect corners, circle centers), across sketches.
pub fn all_sketch_vertices(doc: &Document) -> Vec<ConstraintPoint> {
    let mut points = Vec::new();
    for (index, _line) in doc.lines.iter() {
        points.push(ConstraintPoint::LineEndpoint {
            line: index,
            end: LineEnd::Start,
        });
        points.push(ConstraintPoint::LineEndpoint {
            line: index,
            end: LineEnd::End,
        });
    }
    for (index, _circle) in doc.circles.iter() {
        points.push(ConstraintPoint::CircleCenter(index));
    }
    for (index, _text) in doc.sketch_texts.iter() {
        for anchor in crate::model::TextAnchor::ALL {
            points.push(ConstraintPoint::TextAnchor { text: index, anchor });
        }
    }
    points
}

/// All snap-able vertices in a sketch (line endpoints, circle centers).
pub fn sketch_vertices(doc: &Document, sketch: SketchId) -> Vec<ConstraintPoint> {
    let mut points = Vec::new();
    for (index, line) in doc.lines.iter() {
        if line.sketch != sketch {
            continue;
        }
        points.push(ConstraintPoint::LineEndpoint {
            line: index,
            end: LineEnd::Start,
        });
        points.push(ConstraintPoint::LineEndpoint {
            line: index,
            end: LineEnd::End,
        });
    }
    for (index, circle) in doc.circles.iter() {
        if circle.sketch != sketch {
            continue;
        }
        points.push(ConstraintPoint::CircleCenter(index));
    }
    // A text's nine anchor points (#408): snapping a dragged vertex onto one holds it there
    // with a coincident constraint, like any other vertex.
    for (index, text) in doc.sketch_texts.iter() {
        if text.sketch != sketch {
            continue;
        }
        for anchor in crate::model::TextAnchor::ALL {
            points.push(ConstraintPoint::TextAnchor { text: index, anchor });
        }
    }
    // A calibrated image's reference points (#425) and box anchors (#1589), for images
    // on this sketch's plane. Calibration first so they win a tie with a coincident
    // top/bottom-middle box point.
    for (index, img) in doc.tracing_images.iter() {
        if doc.sketch_face(sketch) != Some(crate::model::FaceId::ConstructionPlane(img.plane)) {
            continue;
        }
        for i in 0..2 {
            if crate::model::image_calibration_point_uv(img, i).is_some() {
                points.push(ConstraintPoint::ImageCalibrationPoint { image: index, index: i });
            }
        }
        for anchor in crate::model::TextAnchor::ALL {
            points.push(ConstraintPoint::ImageAnchor { image: index, anchor });
        }
    }
    // A circular body face's analytic centre (#1858): unlike a polygon corner (rejected
    // below), the centre of a circle is stable and meaningful — it stays the centre however
    // the body's radius changes — so it is a first-class vertex snap. A cap sketched on an
    // extruded circle already has its UV origin there, and that origin wins the tie.
    if let Some(face) = doc.sketch_face(sketch) {
        if crate::extrude::face_circle_world(doc, &face).is_some() {
            points.push(ConstraintPoint::FaceCircleCenter { face });
        }
    }
    // The corners of the body face the sketch sits on (#26/#27, #139, #1973). A FaceVertex
    // resolves from the face's live boundary, so a point snapped to a corner follows that
    // corner as the body reshapes. Round faces have no corners — their boundary is chords.
    if let Some((face, count)) = sketched_on_face_boundary(doc, sketch) {
        if crate::extrude::face_circle_world(doc, &face).is_none() {
            for index in 0..count {
                points.push(ConstraintPoint::FaceVertex { face: face.clone(), index });
            }
        }
    }
    points
}

/// All snap-able line segments in a sketch (its own lines plus, when it sits on a body face,
/// that face's own boundary edges — #26/#27, #139).
pub fn sketch_lines(doc: &Document, sketch: SketchId) -> Vec<ConstraintLine> {
    let mut lines = Vec::new();
    for (index, line) in doc.lines.iter() {
        if line.sketch != sketch {
            continue;
        }
        lines.push(ConstraintLine::Line(index));
    }
    // A circular face's boundary is 48 chords, not 48 edges (#1858) — offering them would
    // litter the rim with chord midpoints. `find_snap` snaps to the true circle instead.
    if let Some((face, count)) = sketched_on_face_boundary(doc, sketch) {
        if crate::extrude::face_circle_world(doc, &face).is_none() {
            for index in 0..count {
                lines.push(ConstraintLine::FaceEdge {
                    face: face.clone(),
                    index,
                });
            }
        }
    }
    for (index, img) in doc.tracing_images.iter() {
        if doc.sketch_face(sketch) != Some(crate::model::FaceId::ConstructionPlane(img.plane)) {
            continue;
        }
        for edge in crate::model::ImageEdge::ALL {
            lines.push(ConstraintLine::ImageEdge { image: index, edge });
        }
    }
    lines
}

/// The `FaceId` and boundary-loop length of the body face a sketch is drawn on, or `None` if
/// the sketch sits on a construction plane (no analytic boundary) or the face no longer
/// resolves. Backs referencing the sketched-on face's own vertices/edges (#139).
fn sketched_on_face_boundary(doc: &Document, sketch: SketchId) -> Option<(crate::model::FaceId, usize)> {
    let face = doc.sketch_face(sketch)?;
    let boundary = crate::extrude::face_boundary_loop_world(doc, &face)?;
    (!boundary.is_empty()).then_some((face, boundary.len()))
}

/// The analytic circle of the body face a sketch sits on, in that sketch's UV (#1858):
/// the face, its centre, and its radius. `None` when the face isn't round.
fn sketched_on_face_circle(
    doc: &Document,
    sketch: SketchId,
) -> Option<(crate::model::FaceId, (f32, f32), f32)> {
    let face = doc.sketch_face(sketch)?;
    let (center, radius) = crate::extrude::face_circle_world(doc, &face)?;
    let frame = crate::face::sketch_geometry_frame(doc, sketch)?;
    Some((face, crate::face::world_to_local(&frame, center), radius))
}

fn dist_sq(a: (f32, f32), b: (f32, f32)) -> f32 {
    let du = a.0 - b.0;
    let dv = a.1 - b.1;
    du * du + dv * dv
}

/// Foot of the perpendicular from `p` onto segment `a`–`b`, or `None` if it falls outside it
/// or the segment is degenerate.
fn project_onto_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> Option<(f32, f32)> {
    let du = b.0 - a.0;
    let dv = b.1 - a.1;
    let len_sq = du * du + dv * dv;
    if len_sq < 1e-12 {
        return None;
    }
    let t = ((p.0 - a.0) * du + (p.1 - a.1) * dv) / len_sq;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    Some((a.0 + du * t, a.1 + dv * t))
}

/// Foot of the perpendicular from `p` onto the *infinite line* through `a`–`b` (unclamped), and
/// the parameter `t` along `a`→`b` (`t<0` or `t>1` means the foot is beyond the segment). `None`
/// for a degenerate segment.
fn project_onto_infinite_line(
    p: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
) -> Option<((f32, f32), f32)> {
    let du = b.0 - a.0;
    let dv = b.1 - a.1;
    let len_sq = du * du + dv * dv;
    if len_sq < 1e-12 {
        return None;
    }
    let t = ((p.0 - a.0) * du + (p.1 - a.1) * dv) / len_sq;
    Some(((a.0 + du * t, a.1 + dv * t), t))
}

/// Inference ("extension") snap: project `query` onto the infinite extension of each anchor
/// line/edge, returning the nearest whose perpendicular distance is within `perp_tol` and whose
/// foot lies *beyond* the segment (the on-segment region is already covered by [`find_snap`]'s
/// `OnLine`). `exclude` drops anchors owned by points being dragged. Sketch units throughout.
pub fn find_extension_snap(
    doc: &Document,
    sketch: SketchId,
    anchors: &[ConstraintLine],
    query: (f32, f32),
    perp_tol: f32,
    exclude: &[ConstraintPoint],
) -> Option<Snap> {
    if perp_tol <= 0.0 {
        return None;
    }
    let perp_tol_sq = perp_tol * perp_tol;
    let excluded_lines: Vec<ConstraintLine> = exclude
        .iter()
        .flat_map(owning_lines)
        .collect();

    let mut best: Option<(f32, Snap)> = None;
    for line in anchors {
        let line = line.clone();
        if excluded_lines.contains(&line) {
            continue;
        }
        let Ok(((x0, y0), (x1, y1))) = line_uv_endpoints(doc, sketch, line.clone()) else {
            continue;
        };
        let Some((foot, t)) = project_onto_infinite_line(query, (x0, y0), (x1, y1)) else {
            continue;
        };
        // Only the extension beyond the segment endpoints is an "extension" snap.
        if (0.0..=1.0).contains(&t) {
            continue;
        }
        let d2 = dist_sq(query, foot);
        if d2 <= perp_tol_sq && best.as_ref().is_none_or(|(b, _)| d2 < *b) {
            best = Some((
                d2,
                Snap {
                    uv: foot,
                    target: SnapTarget::OnLineExtension(line),
                },
            ));
        }
    }
    best.map(|(_, snap)| snap)
}

/// Edges incident to a vertex — its extension guides when the cursor hovers it (see #21).
pub fn vertex_extension_anchors(point: ConstraintPoint) -> Vec<ConstraintLine> {
    owning_lines(&point)
}

/// Inference snap onto the infinite line perpendicular to `anchor`, through its midpoint —
/// reachable only once `anchor` has been "touched" (see #41). Unlike `find_extension_snap`,
/// there's no segment to exclude a middle region from: the whole perpendicular line is fair
/// game, since it doesn't coincide with `anchor` itself.
pub fn find_normal_at_midpoint_snap(
    doc: &Document,
    sketch: SketchId,
    anchor: Option<ConstraintLine>,
    query: (f32, f32),
    perp_tol: f32,
    exclude: &[ConstraintPoint],
) -> Option<Snap> {
    let anchor = anchor?;
    if perp_tol <= 0.0 {
        return None;
    }
    let excluded_lines: Vec<ConstraintLine> = exclude
        .iter()
        .flat_map(owning_lines)
        .collect();
    if excluded_lines.contains(&anchor) {
        return None;
    }
    let Ok(((x0, y0), (x1, y1))) = line_uv_endpoints(doc, sketch, anchor.clone()) else {
        return None;
    };
    let mid = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let perp_b = (mid.0 - dy, mid.1 + dx);
    let (foot, _t) = project_onto_infinite_line(query, mid, perp_b)?;
    let d2 = dist_sq(query, foot);
    if d2 <= perp_tol * perp_tol {
        Some(Snap {
            uv: foot,
            target: SnapTarget::NormalAtMidpoint(anchor),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use super::*;
    use crate::model::{Document, FaceId, Line};

    const EPS: f32 = 1e-4;

    fn sketch_doc() -> (Document, SketchId) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        (doc, sketch)
    }

    /// #1977: a sketch circle's rim is a snap target, pinned with a point-on-circle
    /// coincidence.
    #[test]
    fn snaps_onto_a_sketch_circles_rim() {
        let (mut doc, sketch) = sketch_doc();
        doc.circles.insert(crate::model::Circle::from_local_center_radius(sketch, 30.0, 30.0, 10.0, 0.0));
        let snap = find_snap(&doc, sketch, (40.4, 30.5), 1.0, &[]).expect("snap to rim");
        assert_eq!(snap.target, SnapTarget::OnCircle(rkey(0)));
        let on_rim = ((snap.uv.0 - 30.0).powi(2) + (snap.uv.1 - 30.0).powi(2)).sqrt();
        assert!((on_rim - 10.0).abs() < 1e-3, "{:?}", snap.uv);
        assert!(matches!(
            snap_constraint_kind(ConstraintPoint::CircleCenter(rkey(1)), snap.target),
            ConstraintKind::Coincident { b: ConstraintEntity::Circle(c), .. } if c == rkey(0)
        ));
        // The circle's own centre never snaps to its own rim.
        assert!(find_snap(&doc, sketch, (40.4, 30.5), 1.0, &[ConstraintPoint::CircleCenter(rkey(0))]).is_none());
    }

    /// #1977: where a circle crosses another curve (here the Y axis), that crossing is a
    /// snap point of its own, pinned to both curves.
    #[test]
    fn snaps_to_where_a_circle_crosses_an_axis() {
        use crate::model::SketchAxis;
        let (mut doc, sketch) = sketch_doc();
        doc.circles.insert(crate::model::Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        let snap = find_snap(&doc, sketch, (0.4, 10.4), 1.0, &[]).expect("snap to crossing");
        let SnapTarget::Intersection(a, b) = &snap.target else {
            panic!("expected an intersection, got {:?}", snap.target);
        };
        assert!(
            [a, b].contains(&&ConstraintEntity::Circle(rkey(0)))
                && [a, b].contains(&&ConstraintEntity::Line(ConstraintLine::OriginAxis(SketchAxis::Y))),
            "{a:?} {b:?}"
        );
        assert!(snap.uv.0.abs() < 1e-3 && (snap.uv.1 - 10.0).abs() < 1e-3, "{:?}", snap.uv);
    }

    #[test]
    fn snaps_onto_the_origin_axes() {
        use crate::model::SketchAxis;
        let (doc, sketch) = sketch_doc();
        // Near the X axis (small v, away from the origin): snaps onto it at v = 0.
        let sx = find_snap(&doc, sketch, (20.0, 0.3), 1.0, &[]).expect("snap to X axis");
        assert_eq!(
            sx.target,
            SnapTarget::OnLine(ConstraintLine::OriginAxis(SketchAxis::X))
        );
        assert!(sx.uv.1.abs() < EPS && (sx.uv.0 - 20.0).abs() < EPS);

        // Near the Y axis (small u): snaps onto it at u = 0.
        let sy = find_snap(&doc, sketch, (0.3, 15.0), 1.0, &[]).expect("snap to Y axis");
        assert_eq!(
            sy.target,
            SnapTarget::OnLine(ConstraintLine::OriginAxis(SketchAxis::Y))
        );
        assert!(sy.uv.0.abs() < EPS && (sy.uv.1 - 15.0).abs() < EPS);
    }

    #[test]
    fn snaps_to_nearby_vertex() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let snap = find_snap(&doc, sketch, (10.4, 0.3), 1.0, &[]).unwrap();
        assert_eq!(
            snap.target,
            SnapTarget::Vertex(ConstraintPoint::LineEndpoint {
                line: lkey(0),
                end: LineEnd::End,
            })
        );
        assert!((snap.uv.0 - 10.0).abs() < EPS && snap.uv.1.abs() < EPS);
    }

    #[test]
    fn vertex_beats_midpoint_and_on_line() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        // Near the start vertex but also near the line; the vertex must win.
        let snap = find_snap(&doc, sketch, (0.3, 0.2), 2.0, &[]).unwrap();
        assert!(matches!(snap.target, SnapTarget::Vertex(_)));
    }

    #[test]
    fn snaps_to_line_midpoint() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        // Near the midpoint (5,0) but away from both endpoints.
        let snap = find_snap(&doc, sketch, (5.2, 0.3), 1.0, &[]).unwrap();
        assert_eq!(snap.target, SnapTarget::Midpoint(ConstraintLine::Line(lkey(0))));
        assert!((snap.uv.0 - 5.0).abs() < EPS && snap.uv.1.abs() < EPS);
    }

    #[test]
    fn snaps_onto_line_when_not_near_vertex_or_midpoint() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let snap = find_snap(&doc, sketch, (2.0, 0.3), 1.0, &[]).unwrap();
        assert_eq!(snap.target, SnapTarget::OnLine(ConstraintLine::Line(lkey(0))));
        assert!((snap.uv.0 - 2.0).abs() < EPS && snap.uv.1.abs() < EPS);
    }

    /// #139/#1395/#1973: a sketch drawn on a body's own extrusion cap face can snap to, and
    /// constrain against, that face's own corners and boundary edges. A cursor on a corner
    /// snaps to the corner (FaceVertex, which tracks the face as it reshapes); a cursor along
    /// an edge snaps onto that edge line (FaceEdge).
    #[test]
    fn sketched_on_face_boundary_is_snappable() {
        use crate::model::{ExtrudeFace, Extrusion, FaceId};
        let (mut doc, base) = sketch_doc();
        let lines = crate::construction::add_line_rectangle(&mut doc, base, 0.0, 0.0, 10.0, 4.0, [false; 4]);
        let profile = ExtrudeFace::Polygon(lines.to_vec());
        doc.extrusions.insert(Extrusion {
            sketch: base,
            faces: vec![profile.clone()],
            distance: 6.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        let cap = doc.add_sketch(FaceId::ExtrudeCap {
            extrusion: xkey(0),
            profile: profile.clone(),
            top: true,
        });
        let cap_face = FaceId::ExtrudeCap {
            extrusion: xkey(0),
            profile: profile.clone(),
            top: true,
        };

        let corners = sketch_vertices(&doc, cap)
            .iter()
            .filter(|p| matches!(p, ConstraintPoint::FaceVertex { .. }))
            .count();
        assert_eq!(corners, 4, "cap face should expose its 4 corners");
        let face_edges = sketch_lines(&doc, cap)
            .iter()
            .filter(|l| matches!(l, ConstraintLine::FaceEdge { .. }))
            .count();
        assert_eq!(face_edges, 4, "cap face should expose its 4 edges");

        // A cursor on a corner snaps to that corner (#1973).
        let v1 = ConstraintPoint::FaceVertex { face: cap_face.clone(), index: 1 };
        let (u, v) = point_uv(&doc, cap, v1.clone()).unwrap();
        let snap = find_snap(&doc, cap, (u + 0.2, v + 0.2), 1.0, &[]).unwrap();
        assert_eq!(snap.target, SnapTarget::Vertex(v1));
        assert!(dist_sq(snap.uv, (u, v)) < 1e-8);

        // A cursor along an edge, away from corners and the midpoint, snaps onto the edge line.
        let v0 = ConstraintPoint::FaceVertex { face: cap_face.clone(), index: 0 };
        let (u0, v0uv) = point_uv(&doc, cap, v0).unwrap();
        let q = (u0 + (u - u0) * 0.25, v0uv + (v - v0uv) * 0.25);
        let snap = find_snap(&doc, cap, q, 1.0, &[]).unwrap();
        assert_eq!(
            snap.target,
            SnapTarget::OnLine(ConstraintLine::FaceEdge { face: cap_face, index: 0 })
        );
    }

    /// #1858: a sketch on a *circular* body face must treat that face as a circle, not as the
    /// 48-segment mesh approximation its boundary loop is stored as. None of those chords are
    /// offered as snap lines (they'd litter the rim with 48 midpoint snaps); instead the face
    /// offers its analytic centre as a vertex and its true rim as an on-circle snap.
    #[test]
    fn circular_face_snaps_as_a_circle_not_a_mesh() {
        use crate::model::{Circle, ExtrudeFace, Extrusion, FaceId};
        let (mut doc, base) = sketch_doc();
        doc.circles
            .insert(Circle::from_local_center_radius(base, 0.0, 0.0, 10.0, 0.0));
        let profile = ExtrudeFace::Circle(rkey(0));
        doc.extrusions.insert(Extrusion {
            sketch: base,
            faces: vec![profile.clone()],
            distance: 6.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        let cap_face = FaceId::ExtrudeCap {
            extrusion: xkey(0),
            profile,
            top: true,
        };
        let cap = doc.add_sketch(cap_face.clone());

        // No faceted boundary edges at all — the rim is a circle, not 48 chords.
        let face_edges = sketch_lines(&doc, cap)
            .iter()
            .filter(|l| matches!(l, ConstraintLine::FaceEdge { .. }))
            .count();
        assert_eq!(face_edges, 0, "a circular face must not expose mesh chords");

        // The face's centre is a real snap-able vertex. (On an extruded circle's cap the
        // sketch's own origin already sits there, and wins the tie — but the analytic centre
        // is offered too, for round faces whose frame origin is elsewhere.)
        let centre = ConstraintPoint::FaceCircleCenter { face: cap_face.clone() };
        assert!(
            sketch_vertices(&doc, cap).contains(&centre),
            "circular face should offer its centre"
        );
        let (cu, cv) = point_uv(&doc, cap, centre).unwrap();
        assert!(cu.abs() < EPS && cv.abs() < EPS, "centre at UV origin, got {cu},{cv}");
        let snap = find_snap(&doc, cap, (0.3, 0.2), 1.0, &[]).expect("snap to centre");
        assert!(matches!(
            snap.target,
            SnapTarget::Origin | SnapTarget::Vertex(ConstraintPoint::FaceCircleCenter { .. })
        ));
        assert!(snap.uv.0.abs() < EPS && snap.uv.1.abs() < EPS);

        // The rim snaps onto the *exact* circle (radius 10), not onto a chord (which would
        // sit ~0.02 mm inside it), and pins with a point-on-circle coincidence.
        // (off the origin axes, which are on-line snaps too)
        let q = 10.3 / 2f32.sqrt();
        let snap = find_snap(&doc, cap, (q, q), 1.0, &[]).expect("snap to rim");
        assert_eq!(snap.target, SnapTarget::OnFaceCircle(cap_face.clone()));
        assert!(
            (snap.uv.0.hypot(snap.uv.1) - 10.0).abs() < EPS,
            "rim snap must land on the true circle, got {:?}",
            snap.uv
        );
        assert!(matches!(
            snap_constraint_kind(ConstraintPoint::CircleCenter(rkey(0)), snap.target),
            ConstraintKind::Coincident {
                b: ConstraintEntity::FaceCircle { face },
                ..
            } if face == cap_face
        ));
    }

    #[test]
    fn excludes_dragged_point_and_its_line() {
        let (mut doc, sketch) = sketch_doc();
        // Off both origin axes (v = 5) so only own-geometry exclusion is under test.
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 5.0, 10.0, 5.0));
        let dragged = ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::End,
        };
        // Querying right at the dragged end must not snap to itself or its own line.
        let snap = find_snap(&doc, sketch, (10.0, 5.0), 1.0, &[dragged]);
        assert!(snap.is_none(), "should not snap to own geometry: {snap:?}");
    }

    #[test]
    fn real_vertex_beats_origin_when_closer() {
        let (mut doc, sketch) = sketch_doc();
        // A line endpoint very near the query, closer than the origin.
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.4, 0.4, 10.0, 0.0));
        let snap = find_snap(&doc, sketch, (0.45, 0.45), 1.0, &[]).unwrap();
        assert!(matches!(snap.target, SnapTarget::Vertex(_)));
    }

    #[test]
    fn extension_snaps_to_infinite_line_beyond_endpoint() {
        let (mut doc, sketch) = sketch_doc();
        // A horizontal segment from (0,0) to (10,0); the query at (15, 0.2) is past the End
        // endpoint but close to the line's extension.
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchors = vec![ConstraintLine::Line(lkey(0))];
        let snap = find_extension_snap(&doc, sketch, &anchors, (15.0, 0.2), 1.0, &[]).unwrap();
        assert_eq!(snap.target, SnapTarget::OnLineExtension(ConstraintLine::Line(lkey(0))));
        // Snapped onto the line (v=0) at the queried u.
        assert!((snap.uv.0 - 15.0).abs() < EPS && snap.uv.1.abs() < EPS);
        // Leaving the point there pins it on the (infinite) line.
        let point = ConstraintPoint::CircleCenter(rkey(0));
        assert!(matches!(
            snap_constraint_kind(point, snap.target),
            ConstraintKind::Coincident {
                b: ConstraintEntity::Line(ConstraintLine::Line(l)),
                ..
            } if l == lkey(0)
        ));
    }

    #[test]
    fn extension_snap_ignores_on_segment_region() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchors = vec![ConstraintLine::Line(lkey(0))];
        // Query is within the segment span — handled by `find_snap`'s OnLine, not extension.
        assert!(find_extension_snap(&doc, sketch, &anchors, (5.0, 0.2), 1.0, &[]).is_none());
    }

    #[test]
    fn extension_snap_rejects_far_perpendicular_distance() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchors = vec![ConstraintLine::Line(lkey(0))];
        // Far above the extension line: outside perpendicular tolerance.
        assert!(find_extension_snap(&doc, sketch, &anchors, (15.0, 5.0), 1.0, &[]).is_none());
    }

    #[test]
    fn extension_snap_excludes_dragged_points_own_edges() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchors = vec![ConstraintLine::Line(lkey(0))];
        let dragged = ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::End,
        };
        assert!(find_extension_snap(&doc, sketch, &anchors, (15.0, 0.2), 1.0, &[dragged]).is_none());
    }

    #[test]
    fn normal_at_midpoint_snaps_onto_perpendicular_line() {
        let (mut doc, sketch) = sketch_doc();
        // Horizontal segment (0,0)-(10,0); midpoint (5,0). The perpendicular through the
        // midpoint is the vertical line u=5.
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchor = Some(ConstraintLine::Line(lkey(0)));
        let snap = find_normal_at_midpoint_snap(&doc, sketch, anchor, (5.2, 4.0), 1.0, &[]).unwrap();
        assert_eq!(
            snap.target,
            SnapTarget::NormalAtMidpoint(ConstraintLine::Line(lkey(0)))
        );
        assert!((snap.uv.0 - 5.0).abs() < EPS && (snap.uv.1 - 4.0).abs() < EPS);
    }

    #[test]
    fn normal_at_midpoint_rejects_no_anchor() {
        let (doc, _sketch) = sketch_doc();
        assert!(find_normal_at_midpoint_snap(&doc, _sketch, None, (5.0, 4.0), 1.0, &[]).is_none());
    }

    #[test]
    fn normal_at_midpoint_rejects_far_perpendicular_distance() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchor = Some(ConstraintLine::Line(lkey(0)));
        // Far from the perpendicular line u=5.
        assert!(find_normal_at_midpoint_snap(&doc, sketch, anchor, (15.0, 4.0), 1.0, &[]).is_none());
    }

    #[test]
    fn normal_at_midpoint_excludes_anchors_own_line() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchor = Some(ConstraintLine::Line(lkey(0)));
        let dragged = ConstraintPoint::LineEndpoint {
            line: lkey(0),
            end: LineEnd::End,
        };
        assert!(
            find_normal_at_midpoint_snap(&doc, sketch, anchor, (5.2, 4.0), 1.0, &[dragged]).is_none()
        );
    }

    #[test]
    fn normal_at_midpoint_rejects_zero_tolerance() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .insert(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        let anchor = Some(ConstraintLine::Line(lkey(0)));
        assert!(find_normal_at_midpoint_snap(&doc, sketch, anchor, (5.0, 0.0), 0.0, &[]).is_none());
    }
}

