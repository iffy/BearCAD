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
    if let Some((_, snap)) = best_mid {
        return Some(snap);
    }
    best_on_line.map(|(_, snap)| snap)
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
        SnapTarget::OnLine(line) | SnapTarget::OnLineExtension(line) => ConstraintKind::Coincident {
            a: ConstraintEntity::Point(point),
            b: ConstraintEntity::Line(line),
        },
        SnapTarget::NormalAtMidpoint(_) => unreachable!(
            "NormalAtMidpoint is handled by add_normal_at_midpoint_constraint, not snap_constraint_kind"
        ),
    }
}

/// Whether the constraint a snap would add is already present (avoids duplicates).
pub fn snap_constraint_already_present(
    doc: &Document,
    point: ConstraintPoint,
    target: SnapTarget,
) -> bool {
    let kind = snap_constraint_kind(point, target);
    doc.constraints
        .values()
        .any(|c| constraint_equivalent(&c.kind, &kind))
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
    // The body face the sketch sits on (#26/#27, #139) is *not* exposed as FaceVertex
    // corners here (#1395): a sketch drawn on an extrusion cap/side wall snaps to that face's
    // boundary edges as FaceEdge **lines** (see `sketch_lines`) instead. Snapping to a face
    // corner would create a Coincident(point, vertex) constraint that fixes the point to that
    // specific corner. When the body reshapes, that corner may not move (e.g. the corner of a
    // rectangle on the y=0 edge, while width changes the y-extent), and the sketch would stay
    // put — confusing for users who snapped to "the edge". `sketch_lines` still offers the
    // FaceEdge OnLine snap, which pins the point to the edge line and tracks the face as it
    // reshapes.
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
    if let Some((face, count)) = sketched_on_face_boundary(doc, sketch) {
        for index in 0..count {
            lines.push(ConstraintLine::FaceEdge {
                face: face.clone(),
                index,
            });
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

    /// #139/#1395: a sketch drawn on a body's own extrusion cap face can snap to, and
    /// constrain against, that face's own boundary edges — so the face's edge lines appear as
    /// snap candidates (`sketch_lines`) and are reachable via `find_snap`. Since #1395 the
    /// face's corners no longer snap as FaceVertex *points*; they are only offered as part of
    /// a FaceEdge line (so the snap pins to the edge, not a single possibly-fixed corner).
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

        // The cap face no longer offers its corners as snap-able vertices (#1395).
        let verts = sketch_vertices(&doc, cap);
        assert!(
            verts.iter().all(|p| !matches!(p, ConstraintPoint::FaceVertex { .. })),
            "face corners must not be exposed as vertex snaps, got {verts:?}"
        );
        // ...but it still offers its 4 boundary edges as lines.
        let face_edges = sketch_lines(&doc, cap)
            .iter()
            .filter(|l| matches!(l, ConstraintLine::FaceEdge { .. }))
            .count();
        assert_eq!(face_edges, 4, "cap face should expose its 4 edges");

        // A cursor sitting on a face corner's projected location snaps onto the face's edge
        // line (OnLine), not a FaceVertex corner (#1395).
        let cap_face = FaceId::ExtrudeCap {
            extrusion: xkey(0),
            profile: profile.clone(),
            top: true,
        };
        let v0 = ConstraintPoint::FaceVertex {
            face: cap_face.clone(),
            index: 1,
        };
        let (u, v) = point_uv(&doc, cap, v0).unwrap();
        let snap = find_snap(&doc, cap, (u, v), 1.0, &[]).unwrap();
        assert_eq!(
            snap.target,
            SnapTarget::OnLine(ConstraintLine::FaceEdge { face: cap_face, index: 0 })
        );
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
