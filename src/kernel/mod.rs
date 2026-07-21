//! Safe Rust surface over the OpenCASCADE (OCCT) geometry kernel.
//!
//! OCCT is an optional, statically-linked native dependency gated behind the
//! `occt` Cargo feature (off by default so the normal build and CI don't need a
//! C++ toolchain or a built OCCT). All `unsafe` FFI lives here; the rest of the
//! app calls the safe functions below and gets a graceful "not available" answer
//! when the kernel wasn't compiled in — see SPEC.md §10.
//!
//! To build with the kernel: see `README.md` ("Building with the OCCT kernel").

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::{face_boolean_loop, slvs_available, slvs_solve, Shape};

#[cfg(not(target_arch = "wasm32"))]
mod ffi {
    use std::os::raw::{c_char, c_int, c_ulong};

    /// Opaque owned BREP shape handle (a heap `TopoDS_Shape` in the shim).
    #[repr(C)]
    pub struct BearcadShape {
        _private: [u8; 0],
    }

    // Must stay ABI-compatible with cpp/bearcad_kernel.hpp.
    unsafe extern "C" {
        pub fn bearcad_kernel_box_volume(dx: f64, dy: f64, dz: f64) -> f64;
        pub fn bearcad_kernel_occt_version() -> *const c_char;

        pub fn bearcad_shape_prism(
            xyz: *const f64,
            n_pts: c_ulong,
            dx: f64,
            dy: f64,
            dz: f64,
        ) -> *mut BearcadShape;
        pub fn bearcad_shape_revolve(
            xyz: *const f64,
            n_pts: c_ulong,
            ox: f64,
            oy: f64,
            oz: f64,
            ax: f64,
            ay: f64,
            az: f64,
            angle_rad: f64,
            symmetric: c_int,
        ) -> *mut BearcadShape;
        pub fn bearcad_shape_loft(
            bottom_xyz: *const f64,
            top_xyz: *const f64,
            n_pts: c_ulong,
        ) -> *mut BearcadShape;
        pub fn bearcad_shape_sweep(
            profile_xyz: *const f64,
            n_profile: c_ulong,
            path_xyz: *const f64,
            n_path: c_ulong,
            smooth: c_int,
        ) -> *mut BearcadShape;
        pub fn bearcad_shape_cylinder(
            cx: f64,
            cy: f64,
            cz: f64,
            ax: f64,
            ay: f64,
            az: f64,
            radius: f64,
            height: f64,
        ) -> *mut BearcadShape;
        pub fn bearcad_shape_boolean(
            a: *const BearcadShape,
            b: *const BearcadShape,
            op: c_int,
        ) -> *mut BearcadShape;
        pub fn bearcad_shape_fillet(
            s: *const BearcadShape,
            edges: *const f64,
            radii: *const f64,
            n: c_ulong,
        ) -> *mut BearcadShape;
        pub fn bearcad_shape_chamfer(
            s: *const BearcadShape,
            edges: *const f64,
            dists: *const f64,
            n: c_ulong,
        ) -> *mut BearcadShape;
        pub fn bearcad_face_boolean_loop(
            a_xy: *const f64,
            a_n: c_ulong,
            b_xy: *const f64,
            b_n: c_ulong,
            op: c_int,
            out_n: *mut c_ulong,
        ) -> *mut f64;
        pub fn bearcad_pts_free(pts: *mut f64);
        pub fn bearcad_shape_volume(shape: *const BearcadShape) -> f64;
        pub fn bearcad_shape_tessellate(
            shape: *const BearcadShape,
            deflection: f64,
            out_tri_count: *mut c_ulong,
        ) -> *mut f64;
        pub fn bearcad_tri_free(tris: *mut f64);
        pub fn bearcad_shape_free(shape: *mut BearcadShape);
        pub fn bearcad_shape_split_solids(
            shape: *const BearcadShape,
            out_count: *mut c_ulong,
        ) -> *mut *mut BearcadShape;
        pub fn bearcad_handles_free(handles: *mut *mut BearcadShape);
        pub fn bearcad_shape_transform(
            shape: *const BearcadShape,
            m: *const f64,
        ) -> *mut BearcadShape;

        pub fn bearcad_shape_write_step(s: *const BearcadShape, path: *const c_char) -> c_int;
        pub fn bearcad_read_step(path: *const c_char) -> *mut BearcadShape;
    }
}

/// Volume of an axis-aligned box, computed by the OCCT kernel. `None` when the
/// kernel isn't compiled in; `None` also on a kernel-side failure (the shim
/// returns a negative sentinel rather than unwinding a C++ exception across FFI).
///
/// Part of the kernel's public API surface; only exercised (by [`selftest`] and
/// the pilot tests) in `occt` builds, hence inert/dead in the default build.
pub fn box_volume(dx: f64, dy: f64, dz: f64) -> Option<f64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let v = unsafe { ffi::bearcad_kernel_box_volume(dx, dy, dz) };
        (v >= 0.0).then_some(v)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::box_volume(dx, dy, dz)
    }
}

/// Linked OCCT version string (e.g. `"8.0.0"`), or `None` when the kernel isn't
/// compiled in. Inert/dead in the default build, like [`box_volume`].
pub fn occt_version() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ptr = unsafe { ffi::bearcad_kernel_occt_version() };
        if ptr.is_null() {
            return None;
        }
        let s = unsafe { std::ffi::CStr::from_ptr(ptr) };
        s.to_str().ok().map(str::to_owned)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::occt_version()
    }
}

/// One-line human-readable kernel status, used by the Help ▸ About message so a
/// user (or a bug report) can tell at a glance whether this build has a real
/// geometry kernel. Doubles as the pilot round-trip self-check: with the kernel
/// linked it actually calls OCCT (build a 1×2×3 box, expect volume ≈ 6).
pub fn selftest() -> String {
    {
        match box_volume(1.0, 2.0, 3.0) {
            Some(v) if (v - 6.0).abs() < 1e-6 => {
                let ver = occt_version().unwrap_or_else(|| "unknown".to_string());
                format!("OCCT kernel {ver}: OK (box self-check passed)")
            }
            Some(v) => format!("OCCT kernel: self-check FAILED (box volume {v} != 6)"),
            None => "OCCT kernel: self-check FAILED (kernel error)".to_string(),
        }
    }
}

/// Boolean operation on two [`Shape`]s. `Fuse` drives body union today; `Cut`
/// and `Common` are exercised by tests and land in app code with extrude
/// cut/intersect mode (#35), hence `allow(dead_code)` for the unused variants.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    /// `a ∪ b`.
    Fuse,
    /// `a − b`.
    Cut,
    /// `a ∩ b`.
    Common,
}

/// Boolean-combine two planar faces given as closed 2D loops (z=0 plane, first
/// point not repeated) and return the result's outer loop, via OCCT
/// (`bearcad_face_boolean_loop`, #88). Same strictness contract as the hand-rolled
/// fallback [`crate::polygon_boolean::polygon_boolean`]: `None` unless the boolean
/// result is exactly one hole-free face (multi-part, annulus, empty, or any OCCT
/// error all reject). Winding of the returned loop is unspecified — the caller
/// ([`crate::polygon_boolean::face_boolean`]) normalizes it.
#[cfg(not(target_arch = "wasm32"))]
pub fn face_boolean_loop(
    a: &[(f32, f32)],
    b: &[(f32, f32)],
    op: crate::model::BooleanOp,
) -> Option<Vec<(f32, f32)>> {
    if a.len() < 3 || b.len() < 3 {
        return None;
    }
    let flat = |pts: &[(f32, f32)]| -> Vec<f64> {
        pts.iter().flat_map(|&(x, y)| [x as f64, y as f64]).collect()
    };
    let fa = flat(a);
    let fb = flat(b);
    // Match bearcad_shape_boolean's op codes: 1 = cut (a − b), 2 = common (a ∩ b).
    let code = match op {
        crate::model::BooleanOp::Difference => 1,
        crate::model::BooleanOp::Intersection => 2,
    };
    let mut count: std::os::raw::c_ulong = 0;
    let ptr = unsafe {
        ffi::bearcad_face_boolean_loop(
            fa.as_ptr(),
            a.len() as std::os::raw::c_ulong,
            fb.as_ptr(),
            b.len() as std::os::raw::c_ulong,
            code,
            &mut count,
        )
    };
    if ptr.is_null() {
        return None;
    }
    let n = count as usize;
    let doubles = unsafe { std::slice::from_raw_parts(ptr, n * 2) };
    let out: Vec<(f32, f32)> = (0..n)
        .map(|i| (doubles[2 * i] as f32, doubles[2 * i + 1] as f32))
        .collect();
    unsafe { ffi::bearcad_pts_free(ptr) };
    (out.len() >= 3).then_some(out)
}

/// An owned OCCT BREP solid. Real geometry, not a mesh: built from profiles,
/// combined with booleans, and only tessellated into triangles at the end for the
/// viewport. Only available in `occt` builds — the migration off the hand-rolled
/// mesh code onto this type is incremental and feature-gated (#86).
#[cfg(not(target_arch = "wasm32"))]
pub struct Shape {
    raw: *mut ffi::BearcadShape,
}

#[cfg(not(target_arch = "wasm32"))]
impl Shape {
    /// Extrude a closed planar profile loop (world-space points, first point not
    /// repeated) along `dir`. `None` on a degenerate profile or kernel failure.
    pub fn prism(profile: &[glam::Vec3], dir: glam::Vec3) -> Option<Shape> {
        if profile.len() < 3 {
            return None;
        }
        let mut xyz = Vec::with_capacity(profile.len() * 3);
        for p in profile {
            xyz.push(p.x as f64);
            xyz.push(p.y as f64);
            xyz.push(p.z as f64);
        }
        let raw = unsafe {
            ffi::bearcad_shape_prism(
                xyz.as_ptr(),
                profile.len() as std::os::raw::c_ulong,
                dir.x as f64,
                dir.y as f64,
                dir.z as f64,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// True BREP cylinder (#177): base circle centered at `center`, extruded `height`
    /// along `axis`. Unlike a faceted [`Shape::prism`] of a sampled circle, its wall is a
    /// real cylindrical surface and its cap rims are single circular edges — which is what
    /// lets rim chamfers/fillets and countersinks work. `None` on degenerate input.
    pub fn cylinder(center: glam::Vec3, axis: glam::Vec3, radius: f64, height: f64) -> Option<Shape> {
        if radius <= 0.0 || height <= 0.0 || axis.length_squared() < 1e-12 {
            return None;
        }
        let raw = unsafe {
            ffi::bearcad_shape_cylinder(
                center.x as f64,
                center.y as f64,
                center.z as f64,
                axis.x as f64,
                axis.y as f64,
                axis.z as f64,
                radius,
                height,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Revolve a closed planar profile around `axis` (through `origin`) by `angle_rad`
    /// (#revolve). `symmetric` sweeps half the angle to each side of the profile plane.
    /// `None` on a degenerate profile/axis or kernel failure.
    pub fn revolve(
        profile: &[glam::Vec3],
        origin: glam::Vec3,
        axis: glam::Vec3,
        angle_rad: f64,
        symmetric: bool,
    ) -> Option<Shape> {
        if profile.len() < 3 || axis.length_squared() < 1e-12 || angle_rad <= 0.0 {
            return None;
        }
        let mut xyz = Vec::with_capacity(profile.len() * 3);
        for p in profile {
            xyz.push(p.x as f64);
            xyz.push(p.y as f64);
            xyz.push(p.z as f64);
        }
        let raw = unsafe {
            ffi::bearcad_shape_revolve(
                xyz.as_ptr(),
                profile.len() as std::os::raw::c_ulong,
                origin.x as f64,
                origin.y as f64,
                origin.z as f64,
                axis.x as f64,
                axis.y as f64,
                axis.z as f64,
                angle_rad,
                symmetric as std::os::raw::c_int,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Sweep a closed planar profile loop (world-space points, first point not repeated)
    /// along a path polyline (#sweep). `smooth` interpolates the path points with a
    /// spline (curved sketch segments); otherwise the spine keeps its sharp polyline
    /// corners. `None` on degenerate input or kernel failure.
    pub fn sweep(profile: &[glam::Vec3], path: &[glam::Vec3], smooth: bool) -> Option<Shape> {
        if profile.len() < 3 || path.len() < 2 {
            return None;
        }
        let flat = |pts: &[glam::Vec3]| -> Vec<f64> {
            pts.iter()
                .flat_map(|p| [p.x as f64, p.y as f64, p.z as f64])
                .collect()
        };
        let pr = flat(profile);
        let pa = flat(path);
        let raw = unsafe {
            ffi::bearcad_shape_sweep(
                pr.as_ptr(),
                profile.len() as std::os::raw::c_ulong,
                pa.as_ptr(),
                path.len() as std::os::raw::c_ulong,
                smooth as std::os::raw::c_int,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Solid lofted between a bottom and top loop in point-for-point
    /// correspondence (same length ≥ 3). Handles a slanted top, unlike
    /// [`Shape::prism`]. `None` on mismatch or kernel failure.
    pub fn loft(bottom: &[glam::Vec3], top: &[glam::Vec3]) -> Option<Shape> {
        if bottom.len() < 3 || bottom.len() != top.len() {
            return None;
        }
        let flat = |pts: &[glam::Vec3]| -> Vec<f64> {
            pts.iter()
                .flat_map(|p| [p.x as f64, p.y as f64, p.z as f64])
                .collect()
        };
        let b = flat(bottom);
        let t = flat(top);
        let raw = unsafe {
            ffi::bearcad_shape_loft(
                b.as_ptr(),
                t.as_ptr(),
                bottom.len() as std::os::raw::c_ulong,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Boolean-combine `self` and `other` into a new shape. `None` on failure.
    pub fn boolean(&self, other: &Shape, op: BoolOp) -> Option<Shape> {
        let code = match op {
            BoolOp::Fuse => 0,
            BoolOp::Cut => 1,
            BoolOp::Common => 2,
        };
        let raw = unsafe { ffi::bearcad_shape_boolean(self.raw, other.raw, code) };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Rigid-transform this shape (Move tool): `m` is a row-major 3x4 rotation+translation.
    pub fn transformed(&self, m: &[f64; 12]) -> Option<Shape> {
        let raw = unsafe { ffi::bearcad_shape_transform(self.raw, m.as_ptr()) };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Split into individual solids (a boolean between disjoint bodies can yield several
    /// disconnected pieces). Empty when the shape holds no solid.
    pub fn solids(&self) -> Vec<Shape> {
        let mut count: std::os::raw::c_ulong = 0;
        let raw = unsafe { ffi::bearcad_shape_split_solids(self.raw, &mut count) };
        if raw.is_null() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let handle = unsafe { *raw.add(i) };
            if !handle.is_null() {
                out.push(Shape { raw: handle });
            }
        }
        unsafe { ffi::bearcad_handles_free(raw) };
        out
    }

    /// Apply true BREP fillets (rounded edges) of the given per-edge `radii` to the
    /// edges of `self` whose two world-space endpoints match each `(a, b)` pair in
    /// `edges` (either order, within a bbox-scaled tolerance). All requested edges go
    /// into one fillet operation. `None` on length mismatch, an unmatched edge, or a
    /// kernel failure — the caller then falls back to the hand-rolled mesher (#77).
    pub fn fillet(&self, edges: &[(glam::Vec3, glam::Vec3)], radii: &[f32]) -> Option<Shape> {
        self.edge_treatment(edges, radii, ffi::bearcad_shape_fillet)
    }

    /// Apply true BREP chamfers (flat symmetric bevels) of the given per-edge `dists`
    /// to the matching edges of `self`. Same matching/fallback contract as
    /// [`Shape::fillet`] (#77).
    pub fn chamfer(&self, edges: &[(glam::Vec3, glam::Vec3)], dists: &[f32]) -> Option<Shape> {
        self.edge_treatment(edges, dists, ffi::bearcad_shape_chamfer)
    }

    /// Shared marshalling for [`Shape::fillet`]/[`Shape::chamfer`]: flatten the edge
    /// endpoint pairs to `[ax,ay,az,bx,by,bz, ...]` (as `prism`/`loft` flatten points)
    /// and the amounts to `f64`, then call the given FFI entry point.
    fn edge_treatment(
        &self,
        edges: &[(glam::Vec3, glam::Vec3)],
        amounts: &[f32],
        f: unsafe extern "C" fn(
            *const ffi::BearcadShape,
            *const f64,
            *const f64,
            std::os::raw::c_ulong,
        ) -> *mut ffi::BearcadShape,
    ) -> Option<Shape> {
        if edges.is_empty() || edges.len() != amounts.len() {
            return None;
        }
        let mut flat = Vec::with_capacity(edges.len() * 6);
        for (a, b) in edges {
            flat.extend_from_slice(&[
                a.x as f64, a.y as f64, a.z as f64, b.x as f64, b.y as f64, b.z as f64,
            ]);
        }
        let amt: Vec<f64> = amounts.iter().map(|&r| r as f64).collect();
        let raw = unsafe {
            f(
                self.raw,
                flat.as_ptr(),
                amt.as_ptr(),
                edges.len() as std::os::raw::c_ulong,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Solid volume, or `None` on a kernel error (negative sentinel).
    /// (Kernel API; exercised by tests, consumed by app code incrementally.)
    #[allow(dead_code)]
    pub fn volume(&self) -> Option<f64> {
        let v = unsafe { ffi::bearcad_shape_volume(self.raw) };
        (v >= 0.0).then_some(v)
    }

    /// Triangulate into outward-oriented triangles (world space) at the given
    /// linear deflection. Empty on failure or an empty shape.
    pub fn tessellate(&self, deflection: f64) -> Vec<[glam::Vec3; 3]> {
        let mut count: std::os::raw::c_ulong = 0;
        let ptr = unsafe { ffi::bearcad_shape_tessellate(self.raw, deflection, &mut count) };
        if ptr.is_null() || count == 0 {
            return Vec::new();
        }
        let n = count as usize;
        let doubles = unsafe { std::slice::from_raw_parts(ptr, n * 9) };
        let mut tris = Vec::with_capacity(n);
        for t in 0..n {
            let b = t * 9;
            let v = |o: usize| {
                glam::Vec3::new(
                    doubles[b + o] as f32,
                    doubles[b + o + 1] as f32,
                    doubles[b + o + 2] as f32,
                )
            };
            tris.push([v(0), v(3), v(6)]);
        }
        unsafe { ffi::bearcad_tri_free(ptr) };
        tris
    }

    /// Write this shape to `path` as a real BREP AP214 STEP file (planar + curved
    /// surfaces), via OCCT's `STEPControl_Writer` (#65). `true` on success; `false`
    /// on a kernel/write error or a path that isn't valid UTF-8 or contains a NUL.
    pub fn write_step(&self, path: &std::path::Path) -> bool {
        let Some(s) = path.to_str() else {
            return false;
        };
        let Ok(c) = std::ffi::CString::new(s) else {
            return false;
        };
        let rc = unsafe { ffi::bearcad_shape_write_step(self.raw, c.as_ptr()) };
        rc == 0
    }

    /// Read the first/combined shape from a STEP file at `path` via OCCT's
    /// `STEPControl_Reader` (#71) — real BREP, curved surfaces included. `None` on a
    /// read failure, an empty file, or a path that isn't valid UTF-8 / contains a NUL.
    pub fn read_step(path: &std::path::Path) -> Option<Shape> {
        let s = path.to_str()?;
        let c = std::ffi::CString::new(s).ok()?;
        let raw = unsafe { ffi::bearcad_read_step(c.as_ptr()) };
        (!raw.is_null()).then_some(Shape { raw })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Shape {
    fn drop(&mut self) {
        unsafe { ffi::bearcad_shape_free(self.raw) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn box_volume_round_trips_through_occt() {
        let v = box_volume(2.0, 3.0, 4.0).expect("kernel available in occt build");
        assert!((v - 24.0).abs() < 1e-6, "box volume {v} != 24");
    }

    #[test]
    fn selftest_passes_when_kernel_linked() {
        assert!(selftest().contains("OK"), "{}", selftest());
    }

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> [Vec3; 4] {
        [
            Vec3::new(x0, y0, 0.0),
            Vec3::new(x1, y0, 0.0),
            Vec3::new(x1, y1, 0.0),
            Vec3::new(x0, y1, 0.0),
        ]
    }

    /// Signed volume of a triangle soup via the divergence theorem — a mesh
    /// integrity check independent of OCCT's own volume computation.
    fn mesh_volume(tris: &[[Vec3; 3]]) -> f32 {
        tris.iter()
            .map(|[a, b, c]| a.dot(b.cross(*c)) / 6.0)
            .sum::<f32>()
            .abs()
    }

    #[test]
    fn prism_from_square_has_expected_volume() {
        let sh = Shape::prism(&square(0.0, 0.0, 1.0, 1.0), Vec3::new(0.0, 0.0, 5.0))
            .expect("prism built");
        assert!((sh.volume().unwrap() - 5.0).abs() < 1e-6);
    }

    #[test]
    fn prism_tessellation_is_watertight_by_volume() {
        let sh = Shape::prism(&square(0.0, 0.0, 2.0, 3.0), Vec3::new(0.0, 0.0, 4.0))
            .expect("prism built");
        let tris = sh.tessellate(0.01);
        assert!(!tris.is_empty());
        // A watertight closed mesh's divergence-theorem volume matches the solid.
        assert!((mesh_volume(&tris) - 24.0).abs() < 1e-3, "mesh vol {}", mesh_volume(&tris));
    }

    #[test]
    fn loft_with_slanted_top_has_average_height_volume() {
        // Unit-square base at z=0; top square with the same x,y but z rising
        // linearly 1→2 across x. Volume = base area (1) × average height (1.5).
        let bottom = square(0.0, 0.0, 1.0, 1.0);
        let top = [
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 2.0),
            Vec3::new(1.0, 1.0, 2.0),
            Vec3::new(0.0, 1.0, 1.0),
        ];
        let sh = Shape::loft(&bottom, &top).expect("loft built");
        assert!((sh.volume().unwrap() - 1.5).abs() < 1e-4, "vol {:?}", sh.volume());
    }

    #[test]
    fn fillet_of_a_cube_vertical_edge_removes_expected_volume() {
        // Unit cube [0,1]^3 as a prism; fillet the vertical edge at corner (1,1).
        let cube = Shape::prism(&square(0.0, 0.0, 1.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let r = 0.2_f32;
        let edge = (Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let filleted = cube.fillet(&[edge], &[r]).expect("fillet applied");
        let v = filleted.volume().unwrap();
        // Rounding a right-angle vertical edge of radius r over height h removes the
        // square-minus-quarter-circle corner: (1 - pi/4) * r^2 * h.
        let removed = (1.0 - std::f64::consts::FRAC_PI_4) * (r as f64).powi(2) * 1.0;
        assert!((v - (1.0 - removed)).abs() < 1e-3, "filleted volume {v}, expected {}", 1.0 - removed);
    }

    #[test]
    fn chamfer_of_a_cube_vertical_edge_removes_expected_volume() {
        let cube = Shape::prism(&square(0.0, 0.0, 1.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        let d = 0.2_f32;
        let edge = (Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let chamfered = cube.chamfer(&[edge], &[d]).expect("chamfer applied");
        let v = chamfered.volume().unwrap();
        // A symmetric chamfer of distance d cuts a right-triangle prism: (d^2 / 2) * h.
        let removed = (d as f64).powi(2) / 2.0 * 1.0;
        assert!((v - (1.0 - removed)).abs() < 1e-3, "chamfered volume {v}, expected {}", 1.0 - removed);
    }

    #[test]
    fn fillet_returns_none_for_unmatched_edge() {
        let cube = Shape::prism(&square(0.0, 0.0, 1.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        // No edge runs between these two points, so matching fails -> None (fallback).
        let bogus = (Vec3::new(5.0, 5.0, 0.0), Vec3::new(6.0, 6.0, 0.0));
        assert!(cube.fillet(&[bogus], &[0.1]).is_none());
        // Length mismatch is also rejected up front.
        let edge = (Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        assert!(cube.fillet(&[edge], &[0.1, 0.2]).is_none());
    }

    #[test]
    fn write_step_then_read_step_round_trips_a_box_by_volume() {
        // Build a 2×3×4 box (volume 24), write it to a temp STEP file, read it back,
        // and assert the re-read solid's volume matches — proving the STEP writer +
        // reader round-trip real BREP.
        let sh = Shape::prism(&square(0.0, 0.0, 2.0, 3.0), Vec3::new(0.0, 0.0, 4.0))
            .expect("prism built");
        let path = std::env::temp_dir()
            .join(format!("bearcad_kernel_step_{}.step", std::process::id()));
        assert!(sh.write_step(&path), "write_step failed");
        let read = Shape::read_step(&path).expect("read_step returned None");
        let v = read.volume().expect("volume");
        assert!((v - 24.0).abs() < 1e-3, "re-read volume {v} != 24");
        // Its tessellation is watertight (divergence-theorem volume matches the box).
        let tris = read.tessellate(0.01);
        assert!(!tris.is_empty());
        assert!((mesh_volume(&tris) - 24.0).abs() < 1e-2, "mesh vol {}", mesh_volume(&tris));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_step_on_a_nonexistent_path_is_none() {
        assert!(Shape::read_step(std::path::Path::new("/nonexistent/bearcad-no.step")).is_none());
    }

    #[test]
    fn booleans_of_two_overlapping_boxes_have_expected_volumes() {
        // Box A: [0,2]×[0,2]×[0,2] (vol 8). Box B: [1,3]×[0,2]×[0,2] (vol 8).
        // Overlap [1,2]×[0,2]×[0,2] = vol 4.
        let a = Shape::prism(&square(0.0, 0.0, 2.0, 2.0), Vec3::new(0.0, 0.0, 2.0)).unwrap();
        let b = Shape::prism(&square(1.0, 0.0, 3.0, 2.0), Vec3::new(0.0, 0.0, 2.0)).unwrap();

        let fuse = a.boolean(&b, BoolOp::Fuse).unwrap().volume().unwrap();
        let cut = a.boolean(&b, BoolOp::Cut).unwrap().volume().unwrap();
        let common = a.boolean(&b, BoolOp::Common).unwrap().volume().unwrap();

        assert!((fuse - 12.0).abs() < 1e-4, "fuse {fuse}");
        assert!((cut - 4.0).abs() < 1e-4, "cut {cut}");
        assert!((common - 4.0).abs() < 1e-4, "common {common}");
    }
}
