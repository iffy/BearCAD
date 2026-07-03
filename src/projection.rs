//! Associative projections of external 3D geometry into sketches (#140).
//!
//! Pressing **Y** with body edges selected (or a whole body/extrusion) while a sketch is
//! open creates one construction-style [`Line`](crate::model::Line) per source edge, carrying
//! a [`ProjectionSource`](crate::model::ProjectionSource). Every geometry recompute calls
//! [`refresh_projections`], which re-resolves each source edge and rewrites the projected
//! line's endpoints — so projections follow their source **associatively**. Sources are
//! geometry-keyed (mesh edges have no stable topological name): when a rebuild moves or
//! removes the source edge, the projection keeps its last resolved shape as a static
//! fallback instead of dangling.

use crate::model::{Document, ProjectionSource, SketchId};
use glam::Vec3;

/// Resolve a projection source to its current world-space segment, or `None` when the
/// source geometry no longer exists (deleted body, or the keyed edge no longer matches
/// after a rebuild).
pub fn resolve_projection_source(doc: &Document, source: ProjectionSource) -> Option<(Vec3, Vec3)> {
    match source {
        ProjectionSource::BodyEdge { body, a, b } => {
            let mesh = crate::extrude::body_solid_mesh(doc, body)?;
            let q = crate::hierarchy::quantize_body_point;
            for (ea, eb) in crate::gpu_viewport::solid_mesh_unique_edges(&mesh) {
                let (qa, qb) = (q(ea), q(eb));
                if (qa == a && qb == b) || (qa == b && qb == a) {
                    return Some((ea, eb));
                }
            }
            None
        }
    }
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
pub fn refresh_projections(doc: &mut Document) {
    let updates: Vec<(usize, (f32, f32), (f32, f32))> = doc
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.deleted)
        .filter_map(|(li, line)| {
            let source = line.projection?;
            let (wa, wb) = resolve_projection_source(doc, source)?;
            let a = project_world_point_into_sketch(doc, line.sketch, wa)?;
            let b = project_world_point_into_sketch(doc, line.sketch, wb)?;
            Some((li, a, b))
        })
        .collect();
    for (li, (x0, y0), (x1, y1)) in updates {
        let line = &mut doc.lines[li];
        line.x0 = x0;
        line.y0 = y0;
        line.x1 = x1;
        line.y1 = y1;
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
            _ => {}
        }
    }
    out
}
