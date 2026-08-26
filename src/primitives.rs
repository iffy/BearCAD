//! Primitive solids (#909): the cuboids, cylinders and spheres the Create Shape tool places
//! straight into 3D, with no sketch behind them.
//!
//! A [`crate::model::Primitive`] stores an anchor frame — a point, the plane normal it was
//! placed on, and that plane's first in-plane direction — plus its dimensions as
//! **expressions**, so a shape rebuilds parametrically like every other feature. Everything
//! here resolves those expressions against the document and turns the result into geometry:
//! a triangle mesh for the viewport, and (where the kernel is available) a real solid for
//! booleans, fillets and STEP.

use crate::extrude::SolidMesh;
use crate::model::{Document, Primitive, PrimitiveKind};
use glam::Vec3;

/// How many segments a cylinder's wall (and a sphere's equator) is tessellated into.
pub(crate) const RADIAL_SEGMENTS: usize = 64;
/// How many stacks a sphere is tessellated into, pole to pole.
const SPHERE_STACKS: usize = 32;

/// A shape's resolved frame and dimensions, in world mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub kind: PrimitiveKind,
    pub origin: Vec3,
    /// Unit normal — the direction the shape grows along.
    pub normal: Vec3,
    /// Unit in-plane axis (orthogonal to `normal`), and its perpendicular.
    pub u: Vec3,
    pub v: Vec3,
    pub width: f32,
    pub depth: f32,
    pub height: f32,
    pub radius: f32,
}

/// Evaluate one dimension expression against the document's parameters. An empty
/// expression is zero, which reads as "not sized yet" everywhere below.
fn length(doc: &Document, expression: &str) -> f32 {
    if expression.trim().is_empty() {
        return 0.0;
    }
    crate::value::eval_length_mm_in_doc(expression, doc).unwrap_or(0.0)
}

/// A shape's frame and numbers, or `None` when it is degenerate or a dimension it needs is
/// missing/zero.
pub fn resolve(doc: &Document, shape: &Primitive) -> Option<Resolved> {
    let normal = Vec3::from_array(shape.normal).normalize_or_zero();
    if normal.length_squared() < 0.5 {
        return None;
    }
    // Orthogonalize the stored in-plane axis against the normal, falling back to any
    // perpendicular when the two are parallel (or the axis was never set).
    let raw_u = Vec3::from_array(shape.u_axis);
    let mut u = (raw_u - normal * raw_u.dot(normal)).normalize_or_zero();
    if u.length_squared() < 0.5 {
        u = plane_u_axis(normal);
    }
    let v = normal.cross(u).normalize_or_zero();
    let resolved = Resolved {
        kind: shape.kind,
        origin: Vec3::from_array(shape.origin),
        normal,
        u,
        v,
        // Not `.abs()`: a negative dimension is a mistake, and taking its magnitude
        // built a shape whose geometry disagreed with its own stored expression (#1663).
        width: length(doc, &shape.width),
        depth: length(doc, &shape.depth),
        height: length(doc, &shape.height),
        radius: length(doc, &shape.radius),
    };
    let sized = match shape.kind {
        PrimitiveKind::Cuboid => {
            resolved.width > 1e-4 && resolved.depth > 1e-4 && resolved.height > 1e-4
        }
        PrimitiveKind::Cylinder => resolved.radius > 1e-4 && resolved.height > 1e-4,
        PrimitiveKind::Sphere => resolved.radius > 1e-4,
    };
    sized.then_some(resolved)
}

/// The name of the first dimension of `shape` that evaluates negative (#1663), for an
/// error that says what is actually wrong rather than "needs a size".
pub fn negative_dimension(doc: &Document, shape: &Primitive) -> Option<&'static str> {
    let fields: &[(&'static str, &String)] = match shape.kind {
        PrimitiveKind::Cuboid => &[
            ("width", &shape.width),
            ("depth", &shape.depth),
            ("height", &shape.height),
        ],
        PrimitiveKind::Cylinder => &[("radius", &shape.radius), ("height", &shape.height)],
        PrimitiveKind::Sphere => &[("radius", &shape.radius)],
    };
    fields
        .iter()
        .find(|(_, expr)| length(doc, expr) < 0.0)
        .map(|(name, _)| *name)
}

impl Resolved {
    /// The four base corners of a cuboid, counter-clockwise about the normal.
    pub fn cuboid_base(&self) -> [Vec3; 4] {
        let (hu, hv) = (self.u * self.width * 0.5, self.v * self.depth * 0.5);
        [
            self.origin - hu - hv,
            self.origin + hu - hv,
            self.origin + hu + hv,
            self.origin - hu + hv,
        ]
    }

    /// A sphere's centre: one radius up the normal from the point it rests on.
    pub fn sphere_center(&self) -> Vec3 {
        self.origin + self.normal * self.radius
    }
}

/// A shape's triangle mesh: what the viewport draws and every mesh-based measure reads.
pub fn mesh(doc: &Document, shape: &Primitive) -> Option<SolidMesh> {
    let r = resolve(doc, shape)?;
    let triangles = match r.kind {
        PrimitiveKind::Cuboid => cuboid_triangles(&r),
        PrimitiveKind::Cylinder => cylinder_triangles(&r),
        PrimitiveKind::Sphere => sphere_triangles(&r),
    };
    (!triangles.is_empty()).then_some(SolidMesh { triangles })
}

fn quad(out: &mut Vec<[Vec3; 3]>, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
    out.push([a, b, c]);
    out.push([a, c, d]);
}

fn cuboid_triangles(r: &Resolved) -> Vec<[Vec3; 3]> {
    let base = r.cuboid_base();
    let lift = r.normal * r.height;
    let top: Vec<Vec3> = base.iter().map(|p| *p + lift).collect();
    let mut out = Vec::with_capacity(12);
    // Bottom (wound against the normal) and top.
    out.push([base[0], base[2], base[1]]);
    out.push([base[0], base[3], base[2]]);
    out.push([top[0], top[1], top[2]]);
    out.push([top[0], top[2], top[3]]);
    for i in 0..4 {
        let j = (i + 1) % 4;
        quad(&mut out, base[i], base[j], top[j], top[i]);
    }
    out
}

fn cylinder_triangles(r: &Resolved) -> Vec<[Vec3; 3]> {
    let lift = r.normal * r.height;
    let rim: Vec<Vec3> = (0..RADIAL_SEGMENTS)
        .map(|i| {
            let a = i as f32 / RADIAL_SEGMENTS as f32 * std::f32::consts::TAU;
            r.origin + (r.u * a.cos() + r.v * a.sin()) * r.radius
        })
        .collect();
    let mut out = Vec::with_capacity(RADIAL_SEGMENTS * 4);
    let top_center = r.origin + lift;
    for i in 0..RADIAL_SEGMENTS {
        let j = (i + 1) % RADIAL_SEGMENTS;
        // Caps, then the wall quad between the two rims.
        out.push([r.origin, rim[j], rim[i]]);
        out.push([top_center, rim[i] + lift, rim[j] + lift]);
        quad(&mut out, rim[i], rim[j], rim[j] + lift, rim[i] + lift);
    }
    out
}

fn sphere_triangles(r: &Resolved) -> Vec<[Vec3; 3]> {
    let center = r.sphere_center();
    // Latitude runs from the pole against the normal (the resting point) to the pole along it.
    let point = |stack: usize, seg: usize| -> Vec3 {
        let phi = stack as f32 / SPHERE_STACKS as f32 * std::f32::consts::PI;
        let theta = seg as f32 / RADIAL_SEGMENTS as f32 * std::f32::consts::TAU;
        let ring = phi.sin();
        center
            + (r.u * (ring * theta.cos()) + r.v * (ring * theta.sin()) - r.normal * phi.cos())
                * r.radius
    };
    let mut out = Vec::with_capacity(SPHERE_STACKS * RADIAL_SEGMENTS * 2);
    for stack in 0..SPHERE_STACKS {
        for seg in 0..RADIAL_SEGMENTS {
            let next = (seg + 1) % RADIAL_SEGMENTS;
            let (a, b) = (point(stack, seg), point(stack, next));
            let (c, d) = (point(stack + 1, next), point(stack + 1, seg));
            // The two caps degenerate to a triangle at the poles.
            if stack == 0 {
                out.push([a, c, d]);
            } else if stack + 1 == SPHERE_STACKS {
                out.push([a, b, c]);
            } else {
                quad(&mut out, a, b, c, d);
            }
        }
    }
    out
}

/// Where each of a shape's dimension fields sits in 3D (#930): the middle of the edge it
/// measures, so the width/depth/height (or radius) read against the geometry they drive.
/// Uses the raw frame rather than [`resolve`], so half-placed shapes still get anchors.
pub fn field_anchors(doc: &Document, shape: &Primitive) -> Vec<(crate::actions::ShapeDimension, Vec3)> {
    use crate::actions::ShapeDimension as D;
    let origin = Vec3::from_array(shape.origin);
    let normal = Vec3::from_array(shape.normal).normalize_or_zero();
    if normal.length_squared() < 0.5 {
        return Vec::new();
    }
    let raw_u = Vec3::from_array(shape.u_axis);
    let mut u = (raw_u - normal * raw_u.dot(normal)).normalize_or_zero();
    if u.length_squared() < 0.5 {
        u = plane_u_axis(normal);
    }
    let v = normal.cross(u).normalize_or_zero();
    let (w, d, h, r) = (
        length(doc, &shape.width).abs(),
        length(doc, &shape.depth).abs(),
        length(doc, &shape.height).abs(),
        length(doc, &shape.radius).abs(),
    );
    match shape.kind {
        PrimitiveKind::Cuboid => {
            let (hu, hv) = (u * w * 0.5, v * d * 0.5);
            vec![
                // The middle of the base edge each dimension runs along, and the middle of
                // a vertical edge for the height.
                (D::Width, origin - hv),
                (D::Depth, origin + hu),
                (D::Height, origin + hu - hv + normal * h * 0.5),
            ]
        }
        PrimitiveKind::Cylinder => vec![
            (D::Radius, origin + u * r * 0.5),
            (D::Height, origin + u * r + normal * h * 0.5),
        ],
        // The sphere's radius reads across its equator.
        PrimitiveKind::Sphere => vec![(D::Radius, origin + normal * r + u * r * 0.5)],
    }
}

/// Where a shape's ghost sits while it follows the cursor (#929): a cuboid hangs its
/// **corner** on the cursor — its first click places a corner — so the stored base centre
/// is half a diagonal away; a cylinder and a sphere are placed by their centre, and stay
/// on the cursor.
pub fn ghost_origin(
    kind: PrimitiveKind,
    cursor: Vec3,
    u: Vec3,
    v: Vec3,
    width: f32,
    depth: f32,
) -> Vec3 {
    match kind {
        PrimitiveKind::Cuboid => cursor + u * width * 0.5 + v * depth * 0.5,
        PrimitiveKind::Cylinder | PrimitiveKind::Sphere => cursor,
    }
}

/// A bare sphere mesh at a point (#920): the Move tool draws the rotation's constraint
/// sphere with it, translucent, when the angle snap is too fine for dots.
pub fn sphere_mesh(center: Vec3, radius: f32) -> SolidMesh {
    let r = Resolved {
        kind: PrimitiveKind::Sphere,
        origin: center - Vec3::Z * radius,
        normal: Vec3::Z,
        u: Vec3::X,
        v: Vec3::Y,
        width: 0.0,
        depth: 0.0,
        height: 0.0,
        radius,
    };
    SolidMesh { triangles: sphere_triangles(&r) }
}

/// Whether a body is a sphere primitive (#1101): the Select tool treats a sphere as a
/// whole body only — its tessellation vertices are not individual selectable points the
/// way a cuboid's corners are, since none of them is a real feature.
pub fn body_is_sphere(doc: &crate::model::Document, body_index: crate::model::BodyKey) -> bool {
    let body = match doc.bodies.get(body_index) {
        Some(b) => b,
        None => return false,
    };
    match body.source {
        crate::model::BodySource::Primitive(pi) => doc
            .primitives
            .get(pi)
            .is_some_and(|shape| shape.kind == PrimitiveKind::Sphere),
        _ => false,
    }
}

/// The flat faces a primitive shape exposes for sketching (#1103): every one a
/// [`crate::model::FaceId::PrimitiveFace`] can name. A cuboid has six, a cylinder two caps,
/// a sphere none (its surface is curved).
pub fn flat_faces(shape: &Primitive) -> Vec<crate::model::PrimitiveFace> {
    use crate::model::PrimitiveFace as F;
    match shape.kind {
        PrimitiveKind::Cuboid => vec![
            F::CuboidBottom,
            F::CuboidTop,
            F::CuboidSide { edge: 0 },
            F::CuboidSide { edge: 1 },
            F::CuboidSide { edge: 2 },
            F::CuboidSide { edge: 3 },
        ],
        PrimitiveKind::Cylinder => vec![F::CylinderBottom, F::CylinderTop],
        PrimitiveKind::Sphere => Vec::new(),
    }
}

/// The world-space polygon (CCW about the face's outward normal) of one flat face of a
/// primitive shape (#1103), for hit-testing and sketch-frame derivation. `None` for a
/// face the shape doesn't have (a sphere, or a cylinder's curved wall).
pub fn face_polygon(doc: &Document, shape: &Primitive, face: crate::model::PrimitiveFace) -> Option<Vec<Vec3>> {
    use crate::model::PrimitiveFace as F;
    let r = resolve(doc, shape)?;
    match face {
        F::CuboidBottom => Some(r.cuboid_base().into_iter().rev().collect()),
        F::CuboidTop => Some(r.cuboid_base().iter().map(|p| *p + r.normal * r.height).collect()),
        F::CuboidSide { edge } => {
            let i = edge as usize;
            if i >= 4 {
                return None;
            }
            let base = r.cuboid_base();
            let j = (i + 1) % 4;
            let top: Vec<Vec3> = base.iter().map(|p| *p + r.normal * r.height).collect();
            // CCW about the outward normal: bottom edge i→j, then up, then back.
            Some(vec![base[i], base[j], top[j], top[i]])
        }
        F::CylinderBottom | F::CylinderTop => {
            let center = if matches!(face, F::CylinderBottom) {
                r.origin
            } else {
                r.origin + r.normal * r.height
            };
            let mut pts = Vec::with_capacity(RADIAL_SEGMENTS);
            for i in 0..RADIAL_SEGMENTS {
                let a = (i as f32) / (RADIAL_SEGMENTS as f32) * std::f32::consts::TAU;
                pts.push(center + r.u * r.radius * a.cos() + r.v * r.radius * a.sin());
            }
            // The bottom cap's outward normal is -normal, so wind it the other way.
            if matches!(face, F::CylinderBottom) {
                pts.reverse();
            }
            Some(pts)
        }
    }
}

/// The sketch frame for one flat face of a primitive shape (#1103): origin at the first
/// polygon vertex, U along its first edge, and the outward normal. A sketch drawn here
/// follows the primitive through edits to its frame and dimensions.
pub fn face_frame(doc: &Document, shape: &Primitive, face: crate::model::PrimitiveFace) -> Option<crate::face::SketchFrame> {
    let poly = face_polygon(doc, shape, face)?;
    if poly.len() < 3 {
        return None;
    }
    let origin = poly[0];
    let normal = (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
    if normal.length_squared() < 1e-8 {
        return None;
    }
    let mut u_axis = poly[1] - poly[0];
    u_axis = (u_axis - normal * u_axis.dot(normal)).normalize_or_zero();
    if u_axis.length_squared() < 1e-8 {
        return None;
    }
    let v_axis = normal.cross(u_axis).normalize_or_zero();
    Some(crate::face::SketchFrame {
        origin,
        u_axis,
        v_axis,
        normal,
    })
}

/// In-plane width direction the Shape tool uses for a hover/preview cuboid (#1748).
///
/// A construction plane keeps its authored `plane_u`. Every other surface — a primitive
/// face, an extrusion cap, a mesh wall — uses [`plane_u_axis`] so two picks of the **same**
/// plane cannot hang the ghost from opposite corners. A cuboid side's analytic frame
/// follows the polygon's first edge, which on the +Y / −X walls runs *against* the world
/// axis [`plane_u_axis`] returns for the mesh pick; as the pointer moved along that wall
/// the nearer of the two hits jittered and the preview flopped.
pub fn shape_preview_u_axis(normal: Vec3, plane_u: Vec3, construction_plane: bool) -> Vec3 {
    if !construction_plane {
        return plane_u_axis(normal);
    }
    let n = normal.normalize_or_zero();
    let u = (plane_u - n * plane_u.dot(n)).normalize_or_zero();
    if u.length_squared() < 0.5 {
        plane_u_axis(normal)
    } else {
        u
    }
}

/// A **stable** in-plane axis for a face's frame (#1050).
///
/// `Vec3::any_orthonormal_vector` is free to return any perpendicular, and does not agree
/// with itself across normals that describe the same plane. Two horizontal surfaces — the
/// ground, whose frame is explicitly world `X`, and a body's top face — would then disagree,
/// so a shape placed on the face landed rotated 90° against one placed on the ground beside
/// it. This picks the world axis least aligned with `normal` and projects it into the plane,
/// which gives `X` for a `+Z` face, matching the ground.
pub fn plane_u_axis(normal: Vec3) -> Vec3 {
    let n = normal.normalize_or_zero();
    // Fixed preference — X, then Y, then Z — taking the first that is not near-parallel to the
    // normal, rather than the *least* aligned of the three. Picking the minimum ties whenever
    // two axes are both perpendicular, which is exactly the case for a lateral face (its
    // normal is one axis, leaving the other two tied at zero). A mesh normal carries float
    // noise, so the tie broke differently between frames and the ghost visibly swapped which
    // corner it hung from (#1052). A threshold this loose cannot be crossed by that noise.
    const NEAR_PARALLEL: f32 = 0.9;
    let reference = [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .find(|axis| n.dot(*axis).abs() < NEAR_PARALLEL)
        .unwrap_or(Vec3::X);
    let u = (reference - n * reference.dot(n)).normalize_or_zero();
    if u.length_squared() < 0.5 {
        n.any_orthonormal_vector()
    } else {
        u
    }
}

/// A shape's kernel solid, for booleans, edge treatments and STEP export. `None` without a
/// kernel (the mesh above still draws the shape).
pub fn kernel_shape(doc: &Document, shape: &Primitive) -> Option<crate::kernel::Shape> {
    let r = resolve(doc, shape)?;
    match r.kind {
        PrimitiveKind::Cuboid => {
            crate::kernel::Shape::prism(&r.cuboid_base(), r.normal * r.height)
        }
        PrimitiveKind::Cylinder => crate::kernel::Shape::cylinder(
            r.origin,
            r.normal,
            r.radius as f64,
            r.height as f64,
        ),
        // A true BREP sphere (#936): revolving a half-disc fails, because its profile
        // touches the revolution axis at both poles.
        PrimitiveKind::Sphere => {
            crate::kernel::Shape::sphere(r.sphere_center(), r.radius as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PrimitiveKind as K;

    fn doc_with(shape: Primitive) -> Document {
        let mut doc = Document::default();
        doc.primitives.insert(shape);
        doc
    }

    /// A document whose only body is a sphere primitive of the given radius, resting on the
    /// ground (origin at the world origin, growing up +Z). Used by the Select-tool vertex
    /// exclusion tests (#1101).
    fn doc_with_sphere_body(radius: &str) -> (Document, crate::model::BodyKey) {
        let mut doc = Document::default();
        let mut shape = Primitive::new(K::Sphere);
        shape.radius = radius.to_string();
        let pi = doc.primitives.insert(shape);
        let bi = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        (doc, bi)
    }

    fn sized(kind: K, width: &str, depth: &str, height: &str, radius: &str) -> Primitive {
        let mut shape = Primitive::new(kind);
        shape.width = width.to_string();
        shape.depth = depth.to_string();
        shape.height = height.to_string();
        shape.radius = radius.to_string();
        shape
    }

    /// A cuboid is its own box: centred on its anchor in plane, growing along the normal.
    #[test]
    fn a_cuboid_meshes_to_its_dimensions() {
        let shape = sized(K::Cuboid, "40", "20", "10", "");
        let doc = doc_with(shape.clone());
        let mesh = mesh(&doc, &shape).expect("a sized cuboid meshes");
        let (min, max) = mesh.bounds().unwrap();
        assert!((min - Vec3::new(-20.0, -10.0, 0.0)).length() < 1e-3, "min {min}");
        assert!((max - Vec3::new(20.0, 10.0, 10.0)).length() < 1e-3, "max {max}");
        let volume = crate::extrude::mesh_signed_volume(&mesh).abs();
        assert!((volume - 8000.0).abs() < 1.0, "8000 mm³, got {volume}");
    }

    /// A cylinder rests on its anchor plane and grows along the normal.
    #[test]
    fn a_cylinder_meshes_to_its_radius_and_height() {
        let shape = sized(K::Cylinder, "", "", "20", "5");
        let doc = doc_with(shape.clone());
        let mesh = mesh(&doc, &shape).expect("a sized cylinder meshes");
        let (min, max) = mesh.bounds().unwrap();
        assert!((min.z - 0.0).abs() < 1e-3 && (max.z - 20.0).abs() < 1e-3, "{min} {max}");
        let volume = crate::extrude::mesh_signed_volume(&mesh).abs();
        let exact = std::f32::consts::PI * 25.0 * 20.0;
        assert!((volume - exact).abs() / exact < 0.01, "{volume} vs {exact}");
    }

    /// #1050: two surfaces with the same normal must give the same in-plane frame. The
    /// ground's is explicitly world X; `any_orthonormal_vector` used to hand a body's top
    /// face Y instead, so a cuboid dropped on the face landed rotated 90 degrees against one
    /// dropped on the ground beside it.
    #[test]
    fn a_horizontal_face_gets_the_same_axis_as_the_ground() {
        assert!((plane_u_axis(Vec3::Z) - Vec3::X).length() < 1e-5);
        // Upside down is still a horizontal plane.
        assert!((plane_u_axis(-Vec3::Z) - Vec3::X).length() < 1e-5);
    }

    /// #1052: the axis must not flip under the float noise a mesh normal carries. A lateral
    /// face's normal is one world axis, leaving the other two exactly perpendicular — a
    /// "least aligned" rule ties there, and the tie broke differently frame to frame, so the
    /// preview cuboid kept swapping which corner it hung from.
    #[test]
    fn the_plane_axis_does_not_flip_under_normal_noise() {
        for face in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
            let clean = plane_u_axis(face);
            // Noise far larger than a tessellated normal's, in every direction.
            for jitter in [
                Vec3::new(1e-6, -2e-6, 3e-6),
                Vec3::new(-4e-5, 5e-5, -6e-5),
                Vec3::new(7e-4, 8e-4, -9e-4),
                Vec3::new(-1e-3, 1e-3, 1e-3),
            ] {
                let noisy = plane_u_axis((face + jitter).normalize());
                assert!(
                    (noisy - clean).length() < 1e-2,
                    "{face} gave {clean} but {} gave {noisy}",
                    face + jitter
                );
            }
        }
    }

    /// #1748: a cuboid's +Y wall has two in-plane frames — polygon first-edge (−X) from
    /// the analytic pick, world +X from the mesh pick. The Shape preview must pick one
    /// (the world axis) so the ghost does not swap corners as the pointer moves.
    #[test]
    fn shape_preview_u_axis_is_the_world_axis_on_a_body_face() {
        let n = Vec3::Y;
        let analytic_u = -Vec3::X;
        let mesh_u = plane_u_axis(n);
        assert!(
            (mesh_u - Vec3::X).length() < 1e-5,
            "plane_u_axis(+Y) is +X, got {mesh_u:?}"
        );
        let from_analytic = shape_preview_u_axis(n, analytic_u, false);
        let from_mesh = shape_preview_u_axis(n, mesh_u, false);
        assert!(
            (from_analytic - from_mesh).length() < 1e-5,
            "analytic {from_analytic:?} and mesh {from_mesh:?} must agree"
        );
        assert!(
            (from_analytic - Vec3::X).length() < 1e-5,
            "body faces use the world axis, got {from_analytic:?}"
        );
    }

    /// #1748: hanging the cuboid ghost from opposite in-plane axes is the visual flop
    /// (up-right vs down-left). After `shape_preview_u_axis` both picks share a corner.
    #[test]
    fn shape_preview_ghost_does_not_swap_corners_on_a_cuboid_side() {
        let n = Vec3::Y;
        let cursor = Vec3::new(10.0, 20.0, 30.0);
        let (w, d) = (50.0, 40.0);
        let u1 = shape_preview_u_axis(n, -Vec3::X, false);
        let u2 = shape_preview_u_axis(n, Vec3::X, false);
        let o1 = ghost_origin(K::Cuboid, cursor, u1, n.cross(u1), w, d);
        let o2 = ghost_origin(K::Cuboid, cursor, u2, n.cross(u2), w, d);
        assert!(
            (o1 - o2).length() < 1e-4,
            "ghost jumped from {o1:?} to {o2:?}"
        );
    }

    /// A construction plane keeps the axes the user drew it with, even when those are
    /// not the world-axis convention — a 45° plane should still line a cuboid up with it.
    #[test]
    fn shape_preview_u_axis_keeps_a_construction_plane_frame() {
        let n = Vec3::new(1.0, 1.0, 0.0).normalize();
        let u = Vec3::new(-1.0, 1.0, 0.0).normalize();
        let got = shape_preview_u_axis(n, u, true);
        assert!(
            (got - u).length() < 1e-5,
            "construction-plane u is authored, got {got:?}"
        );
        // And it is *not* silently replaced by the world-axis fallback.
        let world = plane_u_axis(n);
        assert!(
            (got - world).length() > 0.5,
            "the authored u should differ from plane_u_axis here"
        );
    }

    /// Every cuboid face: analytic first-edge and mesh `plane_u_axis` must produce the
    /// same preview axis, so a pointer sliding across that face cannot flop the ghost.
    #[test]
    fn every_cuboid_face_has_one_shape_preview_axis() {
        let mut shape = sized(K::Cuboid, "40", "30", "50", "");
        shape.origin = [0.0, 0.0, 0.0];
        let doc = doc_with(shape.clone());
        for face in flat_faces(&shape) {
            let frame = face_frame(&doc, &shape, face).expect("flat cuboid faces have a frame");
            let mesh_u = plane_u_axis(frame.normal);
            let a = shape_preview_u_axis(frame.normal, frame.u_axis, false);
            let b = shape_preview_u_axis(frame.normal, mesh_u, false);
            assert!(
                (a - b).length() < 1e-4,
                "{face:?}: analytic u={:?} and mesh u={mesh_u:?} previewed as {a:?} vs {b:?}",
                frame.u_axis
            );
        }
    }

    /// #1050: the axis stays in the plane, stays unit length, and is stable for any normal —
    /// including the vertical faces where X itself is unusable.
    #[test]
    fn the_plane_axis_is_always_a_unit_vector_in_the_plane() {
        let normals = [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            -Vec3::Y,
            Vec3::new(1.0, 1.0, 0.0).normalize(),
            Vec3::new(0.3, -0.7, 0.5).normalize(),
        ];
        for n in normals {
            let u = plane_u_axis(n);
            assert!((u.length() - 1.0).abs() < 1e-4, "{n} gave {u}");
            assert!(u.dot(n).abs() < 1e-4, "{u} is not in the plane of {n}");
            // Stable: the same normal always answers the same way.
            assert_eq!(u, plane_u_axis(n));
        }
    }

    /// A sphere sits **on** its anchor: the click point is the bottom, the centre one
    /// radius up the normal.
    #[test]
    fn a_sphere_rests_on_its_anchor_point() {
        let shape = sized(K::Sphere, "", "", "", "8");
        let doc = doc_with(shape.clone());
        let mesh = mesh(&doc, &shape).expect("a sized sphere meshes");
        let (min, max) = mesh.bounds().unwrap();
        assert!(min.z.abs() < 1e-2, "the sphere rests on z = 0, got {min}");
        assert!((max.z - 16.0).abs() < 1e-2, "and reaches 2r, got {max}");
        let volume = crate::extrude::mesh_signed_volume(&mesh).abs();
        let exact = 4.0 / 3.0 * std::f32::consts::PI * 512.0;
        assert!((volume - exact).abs() / exact < 0.02, "{volume} vs {exact}");
    }

    /// #1050: a cuboid placed on a face rests **on** that face and grows along its normal,
    /// on any plane — not just the ground. The bottom corner is the cursor, every base corner
    /// lies in the anchor plane, and the whole solid is on the normal's side of it.
    #[test]
    fn a_cuboid_rests_on_its_anchor_plane_and_grows_along_the_normal() {
        for normal in [
            Vec3::Z,
            -Vec3::Z,
            Vec3::X,
            Vec3::new(0.3, -0.7, 0.5).normalize(),
        ] {
            let u = plane_u_axis(normal);
            let v = normal.cross(u).normalize();
            let cursor = Vec3::new(19.2, 19.8, 20.0);
            let (w, d, h) = (12.0, 8.0, 30.0);
            let centre = ghost_origin(K::Cuboid, cursor, u, v, w, d);

            let mut shape = sized(K::Cuboid, "12", "8", "30", "");
            shape.origin = centre.to_array();
            shape.normal = normal.to_array();
            shape.u_axis = u.to_array();
            let doc = doc_with(shape.clone());
            let r = resolve(&doc, &shape).unwrap();

            // The bottom corner is exactly where the cursor was.
            assert!(
                r.cuboid_base().iter().any(|c| (*c - cursor).length() < 1e-3),
                "on {normal} the cursor is not a base corner: {:?}",
                r.cuboid_base()
            );
            // The base lies in the anchor plane — nothing dips below the face.
            for c in r.cuboid_base() {
                assert!(
                    (c - cursor).dot(normal).abs() < 1e-3,
                    "base corner {c} is off the plane of {normal}"
                );
            }
            // And the solid is entirely on the normal's side, reaching exactly its height.
            let mesh = mesh(&doc, &shape).expect("a sized cuboid meshes");
            let heights: Vec<f32> = mesh
                .triangles
                .iter()
                .flatten()
                .map(|p| (*p - cursor).dot(normal))
                .collect();
            let lo = heights.iter().cloned().fold(f32::INFINITY, f32::min);
            let hi = heights.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            assert!(lo > -1e-3, "on {normal} the cuboid dips {lo} below its base");
            assert!((hi - h).abs() < 1e-3, "on {normal} it reaches {hi}, not {h}");
        }
    }

    /// #929: a cuboid's ghost hangs its **corner** on the cursor — its first click places a
    /// corner — while a cylinder and a sphere are placed by their centre.
    #[test]
    fn ghost_origin_hangs_a_cuboid_by_its_corner() {
        let cursor = Vec3::new(10.0, 5.0, 0.0);
        let centre = ghost_origin(K::Cuboid, cursor, Vec3::X, Vec3::Y, 40.0, 20.0);
        assert!(
            (centre - Vec3::new(30.0, 15.0, 0.0)).length() < 1e-4,
            "half a base diagonal from the cursor, got {centre}"
        );
        // The cursor really is a corner of the resulting base rectangle.
        let mut shape = sized(K::Cuboid, "40", "20", "5", "");
        shape.origin = centre.to_array();
        let doc = doc_with(shape.clone());
        let r = resolve(&doc, &shape).unwrap();
        assert!(
            r.cuboid_base().iter().any(|c| (*c - cursor).length() < 1e-4),
            "the cursor is one of {:?}",
            r.cuboid_base()
        );
        for kind in [K::Cylinder, K::Sphere] {
            assert_eq!(
                ghost_origin(kind, cursor, Vec3::X, Vec3::Y, 40.0, 20.0),
                cursor,
                "{kind:?} is placed by its centre"
            );
        }
    }

    /// #930: each dimension's mirror sits on the edge it measures.
    #[test]
    fn field_anchors_sit_on_the_edges_they_measure() {
        use crate::actions::ShapeDimension as D;
        let shape = sized(K::Cuboid, "40", "20", "10", "");
        let doc = doc_with(shape.clone());
        let anchors = field_anchors(&doc, &shape);
        let at = |field: D| anchors.iter().find(|(f, _)| *f == field).map(|(_, p)| *p);
        // Width runs along +X, so its label sits on the -Y base edge's middle.
        assert!((at(D::Width).unwrap() - Vec3::new(0.0, -10.0, 0.0)).length() < 1e-4);
        assert!((at(D::Depth).unwrap() - Vec3::new(20.0, 0.0, 0.0)).length() < 1e-4);
        // Height rides a vertical edge, halfway up.
        assert!((at(D::Height).unwrap() - Vec3::new(20.0, -10.0, 5.0)).length() < 1e-4);

        let cylinder = sized(K::Cylinder, "", "", "12", "5");
        let doc = doc_with(cylinder.clone());
        let anchors = field_anchors(&doc, &cylinder);
        assert_eq!(anchors.len(), 2, "a cylinder shows its radius and height");
        let sphere = sized(K::Sphere, "", "", "", "8");
        let doc = doc_with(sphere.clone());
        assert_eq!(field_anchors(&doc, &sphere).len(), 1, "a sphere shows its radius");
    }

    /// #936: every shape builds a **kernel** solid too, not just a mesh — booleans, edge
    /// treatments and STEP all read that, and a cut with a shape that fails to build lands
    /// an empty body.
    #[test]
    fn every_shape_builds_a_kernel_solid() {
        if crate::kernel::occt_version().is_none() {
            return; // no kernel in this build; the mesh path stands alone
        }
        let cases = [
            (sized(K::Cuboid, "40", "20", "10", ""), 8000.0_f32),
            (
                sized(K::Cylinder, "", "", "20", "5"),
                std::f32::consts::PI * 25.0 * 20.0,
            ),
            (
                sized(K::Sphere, "", "", "", "8"),
                4.0 / 3.0 * std::f32::consts::PI * 512.0,
            ),
        ];
        for (shape, expected) in cases {
            let doc = doc_with(shape.clone());
            let solid = kernel_shape(&doc, &shape)
                .unwrap_or_else(|| panic!("{:?} builds a kernel solid", shape.kind));
            let volume = solid.volume().unwrap_or(0.0) as f32;
            assert!(
                (volume - expected).abs() / expected < 0.02,
                "{:?} kernel volume {volume} vs {expected}",
                shape.kind
            );
        }
    }

    /// A shape with a dimension missing (or zero) has no geometry yet — it isn't an error,
    /// it's a half-placed shape.
    #[test]
    fn an_unsized_shape_has_no_mesh() {
        for shape in [
            sized(K::Cuboid, "40", "20", "", ""),
            sized(K::Cylinder, "", "", "10", ""),
            sized(K::Sphere, "", "", "", "0"),
        ] {
            let doc = doc_with(shape.clone());
            assert!(mesh(&doc, &shape).is_none(), "{:?} has nothing to draw", shape.kind);
        }
    }

    /// Dimensions are expressions, so a shape follows its parameters (#909).
    #[test]
    fn shape_dimensions_are_expressions() {
        let mut doc = Document::default();
        doc.parameters.insert(crate::model::Parameter {
            name: "side".to_string(),
            expression: "12".to_string(),
            primary: true,
            minimum: None,
            maximum: None,
            step: None,
            source: None,
        });
        let shape = sized(K::Cuboid, "side", "side", "side * 2", "");
        doc.primitives.insert(shape.clone());
        let r = resolve(&doc, &shape).expect("the expressions resolve");
        assert_eq!((r.width, r.depth, r.height), (12.0, 12.0, 24.0));
    }

    /// The anchor plane's normal orients the shape: a cuboid on a wall grows sideways.
    #[test]
    fn a_shape_grows_along_its_anchor_normal() {
        let mut shape = sized(K::Cuboid, "10", "10", "6", "");
        shape.origin = [5.0, 0.0, 0.0];
        shape.normal = [1.0, 0.0, 0.0];
        shape.u_axis = [0.0, 1.0, 0.0];
        let doc = doc_with(shape.clone());
        let mesh = mesh(&doc, &shape).expect("meshes");
        let (min, max) = mesh.bounds().unwrap();
        assert!((min.x - 5.0).abs() < 1e-3 && (max.x - 11.0).abs() < 1e-3, "{min} {max}");
    }

    /// #1101: a sphere primitive body is detected as a sphere, so the Select tool can treat
    /// it as whole-body-only.
    #[test]
    fn a_sphere_primitive_body_is_a_sphere() {
        let (doc, bi) = doc_with_sphere_body("10");
        assert!(body_is_sphere(&doc, bi));
    }

    /// #1101: a non-sphere primitive (a cuboid) is not a sphere body, and a body that is not a
    /// primitive at all is not one either.
    #[test]
    fn a_cuboid_primitive_body_is_not_a_sphere() {
        let mut doc = Document::default();
        let shape = sized(K::Cuboid, "10", "10", "10", "");
        let pi = doc.primitives.insert(shape.clone());
        let bi = doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        assert!(!body_is_sphere(&doc, bi));
        // A body that does not exist is not a sphere.
        assert!(!body_is_sphere(&doc, crate::arena::Key::from_bits(u64::MAX)));
    }
}
