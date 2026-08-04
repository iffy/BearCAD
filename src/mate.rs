//! What a mate pick names (#1013/#1018/#1021): a face, a datum plane, an edge, a world axis,
//! a hole's centre line or a point, resolved against the live model.
//!
//! Where a joint's parts *start out* is no longer worked out here (#1079). A joint's
//! placement is an ordinary move, solved by [`crate::extrude::move_op_transform`] exactly as
//! the Move tool's own is — a mate always was a move, so it is one. What survives is the
//! vocabulary: resolving a pick, and the **direction** one contributes to a joint's frame.

use crate::model::{Document, MateRef};
use glam::Vec3;

/// A mate pick resolved against the live model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MateGeom {
    /// A face or datum plane: a point on it plus its outward normal.
    Plane { origin: Vec3, normal: Vec3 },
    /// A straight edge or world axis.
    Line { origin: Vec3, dir: Vec3 },
    Point(Vec3),
}

/// The **direction** a mate reference contributes to a joint's frame (#1079): a face or datum
/// plane gives its normal, an edge or world axis its own direction, a hole its centre line. A
/// point has no direction, so it is `None` — which is why the frame's axis inputs refuse one.
pub fn mate_ref_direction(doc: &Document, r: &MateRef) -> Option<Vec3> {
    match resolve(doc, r)? {
        MateGeom::Plane { normal, .. } => (normal.length_squared() > 0.5).then_some(normal),
        MateGeom::Line { dir, .. } => (dir.length_squared() > 0.5).then_some(dir),
        MateGeom::Point(_) => None,
    }
}

/// Resolve a mate pick in un-posed world space — body-local keys re-found on the live mesh,
/// world-fixed references (a datum plane, a world axis, the origin) as they are. `None` when
/// the reference no longer resolves, which mates as identity (#1019).
pub fn resolve(doc: &Document, r: &MateRef) -> Option<MateGeom> {
    match r {
        MateRef::Face { body, centroid, normal } => {
            doc.bodies.get(*body)?;
            let solid = crate::extrude::body_solid_mesh_unposed(doc, *body)?;
            let tris = crate::extrude::face_group_matching(&solid, *centroid, *normal)?;
            let origin = crate::extrude::face_group_center(&tris);
            let n = (tris[0][1] - tris[0][0])
                .cross(tris[0][2] - tris[0][0])
                .normalize_or_zero();
            (n.length_squared() > 0.5).then_some(MateGeom::Plane { origin, normal: n })
        }
        MateRef::Plane(i) => {
            let p = doc.construction_planes.get(*i)?;
            Some(MateGeom::Plane {
                origin: p.origin,
                normal: p.normal.normalize_or_zero(),
            })
        }
        MateRef::Edge { body, a, b } => {
            let (p0, p1) = crate::parameters::body_edge_world_segment(doc, *body, *a, *b)?;
            let dir = (p1 - p0).normalize_or_zero();
            (dir.length_squared() > 0.5).then_some(MateGeom::Line { origin: p0, dir })
        }
        MateRef::Axis(a) => Some(MateGeom::Line {
            origin: Vec3::ZERO,
            dir: a.direction(),
        }),
        // A hole's or a shaft's centre line (#1013) — what "line these up" usually means.
        MateRef::HoleAxis { body, origin, dir } => {
            let (a, b) = crate::extrude::body_axis_segment_unposed(doc, *body, *origin, *dir)?;
            let d = (b - a).normalize_or_zero();
            (d.length_squared() > 0.5).then_some(MateGeom::Line {
                origin: (a + b) * 0.5,
                dir: d,
            })
        }
        MateRef::Point(p) => crate::extrude::move_point_world(doc, p).map(MateGeom::Point),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::model::{Body, BodySource, ImportedMesh, JointKind, JointLimits, JointRef};

    /// A `size` cube with its low corner at `origin` — six real faces, so face keys resolve.
    pub fn cube_tris(origin: Vec3, size: Vec3) -> Vec<[Vec3; 3]> {
        let (a, b) = (origin, origin + size);
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let quad = |p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3| {
            vec![[p0, p1, p2], [p0, p2, p3]]
        };
        let mut t = Vec::new();
        // -Z and +Z
        t.extend(quad(v(a.x, a.y, a.z), v(a.x, b.y, a.z), v(b.x, b.y, a.z), v(b.x, a.y, a.z)));
        t.extend(quad(v(a.x, a.y, b.z), v(b.x, a.y, b.z), v(b.x, b.y, b.z), v(a.x, b.y, b.z)));
        // -Y and +Y
        t.extend(quad(v(a.x, a.y, a.z), v(b.x, a.y, a.z), v(b.x, a.y, b.z), v(a.x, a.y, b.z)));
        t.extend(quad(v(a.x, b.y, a.z), v(a.x, b.y, b.z), v(b.x, b.y, b.z), v(b.x, b.y, a.z)));
        // -X and +X
        t.extend(quad(v(a.x, a.y, a.z), v(a.x, a.y, b.z), v(a.x, b.y, b.z), v(a.x, b.y, a.z)));
        t.extend(quad(v(b.x, a.y, a.z), v(b.x, b.y, a.z), v(b.x, b.y, b.z), v(b.x, a.y, b.z)));
        t
    }

    pub fn cube_body(doc: &mut Document, origin: Vec3, size: Vec3) -> crate::model::BodyKey {
        let mesh = doc.imported_meshes.insert(ImportedMesh {
            triangles: cube_tris(origin, size),
            source_name: format!("part{}", doc.imported_meshes.len()),
            step_bytes: None,
        });
        doc.bodies.insert(Body {
            source: BodySource::Imported(mesh),
            name: None,
            material: None,
            shadow: false,
        })
    }

    /// The face of `body` whose middle is nearest `near` — how the tests name a face without
    /// hand-quantizing a key.
    pub fn face_ref(doc: &Document, body: crate::model::BodyKey, near: Vec3) -> MateRef {
        let solid = crate::extrude::body_solid_mesh_unposed(doc, body).unwrap();
        let groups = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid);
        let best = groups
            .iter()
            .min_by(|a, b| {
                let d = |t: &Vec<[Vec3; 3]>| {
                    (crate::extrude::face_group_center(t) - near).length()
                };
                d(a).partial_cmp(&d(b)).unwrap()
            })
            .unwrap();
        let q = crate::hierarchy::quantize_body_point;
        let n = (best[0][1] - best[0][0]).cross(best[0][2] - best[0][0]).normalize();
        MateRef::Face {
            body,
            centroid: q(crate::extrude::face_group_center(best)),
            normal: q(n),
        }
    }

    pub fn joint(members: Vec<JointRef>, kind: JointKind) -> crate::model::Joint {
        crate::model::Joint {
            members,
            base: 0,
            kind,
            placement: Default::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: JointLimits::default(),
            name: None,
            frame: Default::default(),
        }
    }
}
