//! Associative projections of external 3D geometry into sketches (#140).
//!
//! Pressing **Y** with body edges selected (or a whole body/extrusion) while a sketch is
//! open creates one construction-style [`Line`](crate::model::Line) per source edge, carrying
//! a [`ProjectionSource`](crate::model::ProjectionSource) and drawn solid cyan (#1186).
//! Every geometry recompute calls [`refresh_projections`], which re-resolves each source
//! edge and rewrites the projected line's endpoints — so projections follow their source
//! **associatively**. Sources are geometry-keyed (mesh edges have no stable topological
//! name): when a rebuild moves or removes the source edge, the projection keeps its last
//! resolved shape as a static fallback instead of dangling.

use crate::model::{Document, ProjectionSource, SketchId};
use glam::Vec3;

/// Resolve a projection source to its current world-space segment, or `None` when the
/// source geometry no longer exists (deleted body, or the keyed edge no longer matches
/// after a rebuild). `sketch` is the projection's target: a plane source's segment lies
/// where the plane crosses that sketch's plane (#983), so the target is part of the answer.
pub fn resolve_projection_source(
    doc: &Document,
    sketch: SketchId,
    source: &ProjectionSource,
) -> Option<(Vec3, Vec3)> {
    match source {
        // Shared with derived-parameter edge length (#647/#1188): exact key match, then
        // transform-aware re-find for Repeat/Move/Mirror instances whose world keys slid.
        ProjectionSource::BodyEdge { body, a, b } => {
            crate::parameters::body_edge_world_segment(doc, *body, *a, *b)
        }
        // A unit face's boundary edge (#725): analytic, so it re-resolves after the
        // instance's overrides change instead of going stale like a quantized key.
        ProjectionSource::UnitEdge { instance, face, edge } => {
            crate::units::unit_edge_world_segment(doc, *instance, face, *edge)
        }
        ProjectionSource::Plane { plane } => plane_sketch_intersection(doc, sketch, *plane),
    }
}

/// Where a construction plane crosses `sketch`'s plane (#983): a world segment along the two
/// infinite planes' intersection line, spanning the source plane's drawn rectangle (its
/// corners' shadow on the line) — so the reference line sits where the user sees the planes
/// meet, even when the drawn rectangle itself floats clear of the sketch plane (the datum
/// planes' quadrant gap). `None` for a deleted plane, parallel/coincident planes, or a
/// degenerate span.
pub fn plane_sketch_intersection(
    doc: &Document,
    sketch: SketchId,
    plane: crate::model::ConstructionPlaneKey,
) -> Option<(Vec3, Vec3)> {
    let source = doc.construction_planes.get(plane)?;
    let frame = crate::face::sketch_geometry_frame(doc, sketch)?;
    let d = source.normal.cross(frame.normal);
    if d.length_squared() < 1e-8 {
        return None;
    }
    // A point on both planes: for planes n1·p=k1, n2·p=k2 with d = n1×n2,
    // p0 = ((k1·n2 − k2·n1) × d) / |d|².
    let (k1, k2) = (
        source.normal.dot(source.origin),
        frame.normal.dot(frame.origin),
    );
    let p0 = (frame.normal * k1 - source.normal * k2).cross(d) / d.length_squared();
    let dir = d.normalize();
    let mut t_min = f32::MAX;
    let mut t_max = f32::MIN;
    for corner in crate::construction::plane_corners(source) {
        let t = (corner - p0).dot(dir);
        t_min = t_min.min(t);
        t_max = t_max.max(t);
    }
    (t_max - t_min > 1e-3).then(|| (p0 + dir * t_min, p0 + dir * t_max))
}

/// Project a world-space point onto `sketch`'s plane (along the plane normal) and return it
/// in sketch-local coordinates.
pub fn project_world_point_into_sketch(
    doc: &Document,
    sketch: SketchId,
    world: Vec3,
) -> Option<(f32, f32)> {
    let frame = crate::face::sketch_geometry_frame(doc, sketch)?;
    // `world_to_local` drops the out-of-plane component, which *is* the projection along
    // the plane normal.
    Some(crate::face::world_to_local(&frame, world))
}

/// Re-resolve every projected line's source and rewrite its endpoints (#140). Called from
/// `recompute_document_geometry` so projections track their sources through any edit.
/// Unresolvable sources leave the line untouched (static fallback).
///
/// When a [`ProjectionSource::BodyEdge`] re-resolves (including via the transform-aware
/// fallback for moved repeat instances, #1188), the stored quantized keys are rewritten to
/// the live endpoints so the next recompute hits exact match.
pub fn refresh_projections(doc: &mut Document) {
    let updates: Vec<(
        crate::model::LineKey,
        (f32, f32),
        (f32, f32),
        Option<([i32; 3], [i32; 3])>,
    )> = doc
        .lines
        .iter()
        .filter_map(|(li, line)| {
            let source = line.projection.as_ref()?;
            let (wa, wb) = resolve_projection_source(doc, line.sketch, source)?;
            let a = project_world_point_into_sketch(doc, line.sketch, wa)?;
            let b = project_world_point_into_sketch(doc, line.sketch, wb)?;
            let new_keys = matches!(source, ProjectionSource::BodyEdge { .. }).then(|| {
                let q = crate::hierarchy::quantize_body_point;
                let (ka, kb) = (q(wa), q(wb));
                if ka <= kb {
                    (ka, kb)
                } else {
                    (kb, ka)
                }
            });
            Some((li, a, b, new_keys))
        })
        .collect();
    for (li, (x0, y0), (x1, y1), new_keys) in updates {
        let line = &mut doc.lines[li];
        line.x0 = x0;
        line.y0 = y0;
        line.x1 = x1;
        line.y1 = y1;
        if let (Some((ka, kb)), Some(ProjectionSource::BodyEdge { a, b, .. })) =
            (new_keys, line.projection.as_mut())
        {
            *a = ka;
            *b = kb;
        }
    }
}

/// The source edges a projection request covers (#140), resolved from the scene selection:
/// each selected body edge projects individually; a selected body or extrusion projects all
/// of its solid's feature edges.
pub fn projection_sources_from_selection(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
) -> Vec<ProjectionSource> {
    use crate::hierarchy::SceneElement;
    let q = crate::hierarchy::quantize_body_point;
    let mut out: Vec<ProjectionSource> = Vec::new();
    let mut push = |source: ProjectionSource| {
        if !out.contains(&source) {
            out.push(source);
        }
    };
    for element in selection.iter() {
        match element {
            SceneElement::BodyEdge { body, a, b } => {
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                push(ProjectionSource::BodyEdge { body, a, b });
            }
            SceneElement::Body(body) => {
                if let Some(mesh) = crate::extrude::body_solid_mesh(doc, body) {
                    for (ea, eb) in crate::gpu_viewport::solid_mesh_unique_edges(&mesh) {
                        let (qa, qb) = (q(ea), q(eb));
                        let (qa, qb) = if qa <= qb { (qa, qb) } else { (qb, qa) };
                        push(ProjectionSource::BodyEdge { body, a: qa, b: qb });
                    }
                }
            }
            SceneElement::Extrusion(ei) => {
                if let Some(body) = crate::model::body_index_for_extrusion(doc, ei) {
                    if let Some(mesh) = crate::extrude::body_solid_mesh(doc, body) {
                        for (ea, eb) in crate::gpu_viewport::solid_mesh_unique_edges(&mesh) {
                            let (qa, qb) = (q(ea), q(eb));
                            let (qa, qb) = if qa <= qb { (qa, qb) } else { (qb, qa) };
                            push(ProjectionSource::BodyEdge { body, a: qa, b: qb });
                        }
                    }
                }
            }
            // A construction plane (#983) projects as the line where it crosses the sketch.
            SceneElement::ConstructionPlane(plane) => {
                push(ProjectionSource::Plane { plane });
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::model::plane_key_for_slot as pkey;
    use super::*;

    /// #983: a datum plane's projection into a sketch runs along the two planes'
    /// intersection — every point on both planes — spanning the source's drawn extent even
    /// though the drawn rectangle floats a gap clear of the sketch plane.
    #[test]
    fn plane_intersection_spans_the_source_extent_on_the_sketch_plane() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(crate::model::FaceId::ConstructionPlane(pkey(0)));
        // YZ (index 2, normal X) crosses the ground sketch along the world Y axis.
        let (a, b) = plane_sketch_intersection(&doc, sketch, pkey(2)).expect("YZ crosses the ground");
        for p in [a, b] {
            assert!(p.x.abs() < 1e-4 && p.z.abs() < 1e-4, "on both planes: {p:?}");
        }
        assert!((a - b).length() > 1.0, "a real span, not a degenerate point");
        // The sketch's own plane is parallel to itself: nothing to intersect.
        assert!(plane_sketch_intersection(&doc, sketch, pkey(0)).is_none());
    }
}
