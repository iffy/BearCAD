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
const RADIAL_SEGMENTS: usize = 64;
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

/// A shape's frame and numbers, or `None` when it is deleted, degenerate, or a dimension
/// it needs is missing/zero.
pub fn resolve(doc: &Document, shape: &Primitive) -> Option<Resolved> {
    if shape.deleted {
        return None;
    }
    let normal = Vec3::from_array(shape.normal).normalize_or_zero();
    if normal.length_squared() < 0.5 {
        return None;
    }
    // Orthogonalize the stored in-plane axis against the normal, falling back to any
    // perpendicular when the two are parallel (or the axis was never set).
    let raw_u = Vec3::from_array(shape.u_axis);
    let mut u = (raw_u - normal * raw_u.dot(normal)).normalize_or_zero();
    if u.length_squared() < 0.5 {
        u = normal.any_orthonormal_vector();
    }
    let v = normal.cross(u).normalize_or_zero();
    let resolved = Resolved {
        kind: shape.kind,
        origin: Vec3::from_array(shape.origin),
        normal,
        u,
        v,
        width: length(doc, &shape.width).abs(),
        depth: length(doc, &shape.depth).abs(),
        height: length(doc, &shape.height).abs(),
        radius: length(doc, &shape.radius).abs(),
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
        u = normal.any_orthonormal_vector();
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
        doc.primitives.push(shape);
        doc
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
        doc.parameters.push(crate::model::Parameter {
            name: "side".to_string(),
            expression: "12".to_string(),
            deleted: false,
            primary: true,
            source: None,
        });
        let shape = sized(K::Cuboid, "side", "side", "side * 2", "");
        doc.primitives.push(shape.clone());
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
}
