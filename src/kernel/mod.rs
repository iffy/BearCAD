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
            pitch: f64,
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
        pub fn bearcad_shape_sphere(
            cx: f64,
            cy: f64,
            cz: f64,
            radius: f64,
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
        pub fn bearcad_shape_shell(
            s: *const BearcadShape,
            faces: *const f64,
            n_faces: c_ulong,
            thickness: f64,
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
        pub fn bearcad_shape_clone(shape: *const BearcadShape) -> *mut BearcadShape;
        pub fn bearcad_shape_split_solids(
            shape: *const BearcadShape,
            out_count: *mut c_ulong,
        ) -> *mut *mut BearcadShape;
        pub fn bearcad_handles_free(handles: *mut *mut BearcadShape);
        pub fn bearcad_shape_transform(
            shape: *const BearcadShape,
            m: *const f64,
        ) -> *mut BearcadShape;

        pub fn bearcad_shape_write_step(
            s: *const BearcadShape,
            path: *const c_char,
            name: *const c_char,
        ) -> c_int;
        pub fn bearcad_shapes_write_step(
            shapes: *const *const BearcadShape,
            count: c_int,
            path: *const c_char,
            name: *const c_char,
        ) -> c_int;
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

// Per-thread so parallel tests that also boolean don't inflate #1337's count.
#[cfg(test)]
thread_local! {
    static BOOLEAN_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn note_boolean_call() {
    #[cfg(test)]
    BOOLEAN_CALLS.with(|c| c.set(c.get() + 1));
}

/// Kernel boolean calls since the last [`reset_boolean_call_count`].
#[cfg(test)]
pub fn boolean_call_count() -> u64 {
    BOOLEAN_CALLS.with(|c| c.get())
}

/// Reset [`boolean_call_count`]. Tests only.
#[cfg(test)]
pub fn reset_boolean_call_count() {
    BOOLEAN_CALLS.with(|c| c.set(0));
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

    /// A true BREP sphere (#936): the revolve path can't build one — a half-disc profile
    /// touches its own axis at both poles and OCCT refuses it — so the primitive is used.
    pub fn sphere(center: glam::Vec3, radius: f64) -> Option<Shape> {
        if radius <= 0.0 {
            return None;
        }
        let raw = unsafe {
            ffi::bearcad_shape_sphere(
                center.x as f64,
                center.y as f64,
                center.z as f64,
                radius,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Revolve a closed planar profile around `axis` (through `origin`) by `angle_rad`
    /// (#revolve). `symmetric` sweeps half the angle to each side of the profile plane.
    /// Non-zero `pitch` (axial travel per full 2π turn) makes a helix for springs (#1242).
    /// Signed `angle_rad` is allowed (negative reverses the turn). `None` on a degenerate
    /// profile/axis or kernel failure.
    pub fn revolve(
        profile: &[glam::Vec3],
        origin: glam::Vec3,
        axis: glam::Vec3,
        angle_rad: f64,
        symmetric: bool,
        pitch: f64,
    ) -> Option<Shape> {
        if profile.len() < 3 || axis.length_squared() < 1e-12 || angle_rad.abs() < 1e-12 {
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
                pitch,
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
        note_boolean_call();
        let code = match op {
            BoolOp::Fuse => 0,
            BoolOp::Cut => 1,
            BoolOp::Common => 2,
        };
        let raw = unsafe { ffi::bearcad_shape_boolean(self.raw, other.raw, code) };
        (!raw.is_null()).then_some(Shape { raw })
    }

    /// Cheap handle copy so a memoized solid can be reused without rebuilding
    /// the BREP history (#1337). `None` if the kernel refused the copy.
    pub fn try_clone(&self) -> Option<Shape> {
        let raw = unsafe { ffi::bearcad_shape_clone(self.raw) };
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

    /// Hollow this solid (Shell tool, #1156): remove the listed faces and leave walls of
    /// `thickness` mm (positive, applied inward). Each face is `(point_on_face, outward_normal)`.
    /// An empty face list makes a closed hollow. `None` on failure or non-positive thickness.
    pub fn shell(&self, open_faces: &[(glam::Vec3, glam::Vec3)], thickness: f32) -> Option<Shape> {
        if thickness <= 0.0 {
            return None;
        }
        let mut flat = Vec::with_capacity(open_faces.len() * 6);
        for (p, n) in open_faces {
            flat.extend_from_slice(&[
                p.x as f64, p.y as f64, p.z as f64, n.x as f64, n.y as f64, n.z as f64,
            ]);
        }
        let raw = unsafe {
            ffi::bearcad_shape_shell(
                self.raw,
                if flat.is_empty() {
                    std::ptr::null()
                } else {
                    flat.as_ptr()
                },
                open_faces.len() as std::os::raw::c_ulong,
                thickness as f64,
            )
        };
        (!raw.is_null()).then_some(Shape { raw })
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
    /// surfaces), via OCCT's `STEPControl_Writer` (#65), naming the part `name` so it
    /// arrives titled in the recipient's CAD tool (#1656). `true` on success; `false`
    /// on a kernel/write error or a path that isn't valid UTF-8 or contains a NUL.
    pub fn write_step_named(&self, path: &std::path::Path, name: &str) -> bool {
        let Some(s) = path.to_str() else {
            return false;
        };
        let Ok(c) = std::ffi::CString::new(s) else {
            return false;
        };
        // A NUL or a control character in a body name is not worth failing the export
        // over — drop what can't ride along and keep the rest.
        let cleaned: String = name
            .chars()
            .filter(|c| *c != '\0' && !c.is_control())
            .collect();
        let label = std::ffi::CString::new(cleaned).unwrap_or_default();
        let rc = unsafe { ffi::bearcad_shape_write_step(self.raw, c.as_ptr(), label.as_ptr()) };
        rc == 0
    }

    /// Write several shapes into one STEP file (#1938) — one real BREP solid per shape,
    /// rather than the faceted single shell a multi-body export used to collapse to.
    /// `true` on success.
    pub fn write_step_many(
        shapes: &[Shape],
        path: &std::path::Path,
        name: &str,
    ) -> bool {
        if shapes.is_empty() {
            return false;
        }
        let Some(s) = path.to_str() else {
            return false;
        };
        let Ok(c) = std::ffi::CString::new(s) else {
            return false;
        };
        let cleaned: String = name
            .chars()
            .filter(|c| *c != '\0' && !c.is_control())
            .collect();
        let label = std::ffi::CString::new(cleaned).unwrap_or_default();
        let raws: Vec<*const ffi::BearcadShape> =
            shapes.iter().map(|s| s.raw as *const ffi::BearcadShape).collect();
        let rc = unsafe {
            ffi::bearcad_shapes_write_step(
                raws.as_ptr(),
                raws.len() as std::os::raw::c_int,
                c.as_ptr(),
                label.as_ptr(),
            )
        };
        rc == 0
    }

    /// [`Shape::write_step_named`] with no part name (OCCT's own defaults).
    #[cfg(test)]
    pub fn write_step(&self, path: &std::path::Path) -> bool {
        self.write_step_named(path, "")
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

    /// #1468: a sketch on a tessellated mesh face is planar in f32, not in OCCT's 1e-7
    /// confusion. MakeFace must still build the prism.
    #[test]
    fn prism_from_a_slightly_nonplanar_quad_still_builds() {
        let profile = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 2.0e-5),
            Vec3::new(10.0, 8.0, 0.0),
            Vec3::new(0.0, 8.0, -1.0e-5),
        ];
        let sh = Shape::prism(&profile, Vec3::new(0.0, 0.0, 4.0)).expect("prism built");
        let v = sh.volume().expect("volume");
        assert!((v - 320.0).abs() < 1.0, "volume {v}");
    }

    #[test]
    fn shape_try_clone_preserves_volume() {
        let sh = Shape::prism(&square(0.0, 0.0, 2.0, 3.0), Vec3::new(0.0, 0.0, 4.0))
            .expect("prism built");
        let cloned = sh.try_clone().expect("clone");
        assert!((cloned.volume().unwrap() - 24.0).abs() < 1e-6);
        assert!((sh.volume().unwrap() - 24.0).abs() < 1e-6);
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

    /// #1615: tessellating a block with a through hole must yield the BREP volume
    /// (block minus cylinder), not a mesh that only subtracts a sliver of the hole.
    #[test]
    fn through_hole_tessellation_volume_matches_brep() {
        let boxy =
            Shape::prism(&square(0.0, 0.0, 40.0, 40.0), Vec3::new(0.0, 0.0, 10.0)).unwrap();
        let hole = Shape::cylinder(Vec3::new(20.0, 20.0, -1.0), Vec3::Z, 5.0, 12.0).unwrap();
        let cut = boxy.boolean(&hole, BoolOp::Cut).expect("through hole");
        let brep = cut.volume().expect("brep volume");
        let expected = 40.0 * 40.0 * 10.0 - std::f64::consts::PI * 25.0 * 10.0;
        assert!(
            (brep - expected).abs() < 1e-3,
            "BREP through-hole volume {brep} expected {expected}"
        );
        let tris = cut.tessellate(0.05);
        let mesh = mesh_volume(&tris) as f64;
        assert!(
            (mesh - expected).abs() < 20.0,
            "mesh through-hole volume {mesh} expected {expected} (brep {brep})"
        );
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

    /// #1156: shell a 10×10×10 cube with the top face open and 1 mm walls → volume ≈
    /// outer − inner, with the open top leaving no top wall.
    #[test]
    fn shell_open_top_of_a_cube_has_the_expected_volume() {
        let cube = Shape::prism(&square(0.0, 0.0, 10.0, 10.0), Vec3::new(0.0, 0.0, 10.0)).unwrap();
        let open = (Vec3::new(5.0, 5.0, 10.0), Vec3::new(0.0, 0.0, 1.0));
        let shelled = cube.shell(&[open], 1.0).expect("shell built");
        let v = shelled.volume().expect("volume");
        // Outer 10³ minus the inner cavity: 8×8×9 (floor+4 walls at 1 mm; open top).
        let expected = 1000.0 - 8.0 * 8.0 * 9.0;
        assert!(
            (v - expected).abs() < 1.0,
            "shelled volume {v}, expected ~{expected}"
        );
    }

    /// #1163: a closed shell (no open faces) is the wall remainder — outer solid minus the
    /// inset cavity — not the cavity solid alone. Volume, bounds, and triangle count must
    /// match a hollow box, not the hole that was cut out.
    #[test]
    fn shell_closed_cube_is_the_wall_remainder_not_the_cavity() {
        let cube = Shape::prism(&square(0.0, 0.0, 10.0, 10.0), Vec3::new(0.0, 0.0, 10.0)).unwrap();
        let shelled = cube.shell(&[], 1.0).expect("closed shell built");
        let v = shelled.volume().expect("volume");
        // Outer 10³ − inner 8³ = 488 (walls). The cavity alone is 512 — the bug.
        let expected_walls = 1000.0 - 8.0 * 8.0 * 8.0;
        assert!(
            (v - expected_walls).abs() < 1.0,
            "closed shell volume {v}, expected walls ~{expected_walls} (not the 512 cavity)"
        );
        let tris = shelled.tessellate(0.2);
        // Outer 6 faces + inner 6 faces → more than a single solid box's 12 tris.
        assert!(
            tris.len() >= 24,
            "hollow shell should tessellate both outer and inner faces, got {} tris",
            tris.len()
        );
        let mut min = glam::Vec3::splat(f32::MAX);
        let mut max = glam::Vec3::splat(f32::MIN);
        for tri in &tris {
            for p in tri {
                min = min.min(*p);
                max = max.max(*p);
            }
        }
        // Outer bounds stay the original cube; a cavity-only solid would be inset to 1..9.
        assert!(
            min.x <= 1e-3 && min.y <= 1e-3 && min.z <= 1e-3,
            "walls must reach the outer surface, min={min:?}"
        );
        assert!(
            (max.x - 10.0).abs() <= 1e-3
                && (max.y - 10.0).abs() <= 1e-3
                && (max.z - 10.0).abs() <= 1e-3,
            "walls must reach the outer surface, max={max:?}"
        );
    }

    #[test]
    fn shell_rejects_non_positive_thickness() {
        let cube = Shape::prism(&square(0.0, 0.0, 1.0, 1.0), Vec3::new(0.0, 0.0, 1.0)).unwrap();
        assert!(cube.shell(&[], 0.0).is_none());
        assert!(cube.shell(&[], -1.0).is_none());
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

    /// #1033: a cutter whose surface passes exactly through a corner of the body it cuts —
    /// what snapping a sphere onto a box corner with the Move tool produces — still cuts.
    /// OCCT's default tolerance finds no intersection at all in that tangency and hands
    /// back the uncut body; the fuzzy retry is what recovers the cut.
    #[test]
    fn a_cutter_touching_a_corner_exactly_still_cuts() {
        // A 40×40×40 box, and a sphere centred above its top face whose radius is exactly
        // the distance to the box's (40, 40, 40) corner — so its surface passes through it.
        let boxy = Shape::prism(&square(0.0, 0.0, 40.0, 40.0), Vec3::new(0.0, 0.0, 40.0)).unwrap();
        let centre = Vec3::new(20.0, 20.0, 55.0);
        let corner = Vec3::new(40.0, 40.0, 40.0);
        let radius = (corner - centre).length() as f64;

        let sphere = Shape::sphere(centre, radius).unwrap();
        let cut = boxy.boolean(&sphere, BoolOp::Cut).unwrap().volume().unwrap();
        let intact = boxy.volume().unwrap();

        // The sphere reaches 15 mm below the top face, so it must take a real bite.
        assert!(
            cut < intact - 1.0,
            "the cut removed nothing: {cut} vs the intact {intact}"
        );
        // And what it removes is the part of the sphere inside the box, nothing more.
        let common = boxy.boolean(&sphere, BoolOp::Common).unwrap().volume().unwrap();
        assert!(
            ((intact - cut) - common).abs() < intact * 1e-6,
            "removed {} but the overlap is {common}",
            intact - cut
        );
    }

    /// The fuzzy retry must not invent an intersection between solids that really are
    /// apart: a cut by a cutter that misses returns the body whole, not a failure.
    #[test]
    fn a_cutter_that_misses_leaves_the_body_whole() {
        let boxy = Shape::prism(&square(0.0, 0.0, 40.0, 40.0), Vec3::new(0.0, 0.0, 40.0)).unwrap();
        // Centred 60 mm above the top face with a 15 mm radius: nowhere near it.
        let sphere = Shape::sphere(Vec3::new(20.0, 20.0, 100.0), 15.0).unwrap();

        let cut = boxy.boolean(&sphere, BoolOp::Cut).unwrap().volume().unwrap();
        let intact = boxy.volume().unwrap();
        assert!((cut - intact).abs() < 1e-6, "cut {cut} vs intact {intact}");

        let common = boxy.boolean(&sphere, BoolOp::Common).unwrap().volume().unwrap();
        assert!(common < 1e-6, "disjoint solids share no volume, got {common}");
    }

    /// #1355: cutting a fully-enclosed interior solid out of a larger one must leave a
    /// cavity, not the unsliced outer body. Outer 20³, inner 4³ floating inside.
    #[test]
    fn cutting_a_fully_enclosed_solid_leaves_a_cavity() {
        let outer = Shape::prism(&square(0.0, 0.0, 20.0, 20.0), Vec3::new(0.0, 0.0, 20.0)).unwrap();
        // [8,12]×[8,12]×[8,12] — no shared faces with the outer box.
        let inner_base = [
            Vec3::new(8.0, 8.0, 8.0),
            Vec3::new(12.0, 8.0, 8.0),
            Vec3::new(12.0, 12.0, 8.0),
            Vec3::new(8.0, 12.0, 8.0),
        ];
        let inner = Shape::prism(&inner_base, Vec3::new(0.0, 0.0, 4.0)).unwrap();
        let cut = outer.boolean(&inner, BoolOp::Cut).expect("enclosed cut must build");
        let vol = cut.volume().expect("cavity volume");
        assert!(
            (vol - 7936.0).abs() < 1.0,
            "enclosed cut should be 20³−4³ = 7936, got {vol}"
        );
        let solids = cut.solids();
        assert_eq!(solids.len(), 1, "a cavity is one solid with an inner shell");
    }

    /// #1355: A−B is empty when A sits fully inside B — the kernel must not invent a solid.
    #[test]
    fn cutting_a_body_wholly_inside_the_cutter_is_empty() {
        let inner = Shape::prism(&square(0.0, 0.0, 4.0, 4.0), Vec3::new(0.0, 0.0, 4.0)).unwrap();
        let outer = Shape::prism(&square(0.0, 0.0, 50.0, 50.0), Vec3::new(0.0, 0.0, 50.0)).unwrap();
        let cut = inner.boolean(&outer, BoolOp::Cut);
        let vol = cut.as_ref().and_then(|s| s.volume()).unwrap_or(0.0);
        let n = cut.as_ref().map(|s| s.solids().len()).unwrap_or(0);
        assert!(
            vol < 1e-6 && n == 0,
            "A wholly inside B must yield no solid, got vol={vol} solids={n}"
        );
    }

    /// #1356: intersect of disjoint solids is empty — no solid, not a zero-volume leftover.
    #[test]
    fn common_of_disjoint_boxes_is_empty() {
        let a = Shape::prism(&square(0.0, 0.0, 10.0, 10.0), Vec3::new(0.0, 0.0, 10.0)).unwrap();
        let b = Shape::prism(&square(50.0, 50.0, 60.0, 60.0), Vec3::new(0.0, 0.0, 10.0)).unwrap();
        let common = a.boolean(&b, BoolOp::Common);
        let vol = common.as_ref().and_then(|s| s.volume()).unwrap_or(0.0);
        let n = common.as_ref().map(|s| s.solids().len()).unwrap_or(0);
        assert!(
            vol < 1e-6 && n == 0,
            "disjoint common must yield no solid, got vol={vol} solids={n}"
        );
    }

    /// #1248/#1249: multi-turn helical revolve must tessellate leanly enough for
    /// interactive orbit while still being a *smooth* BREP (helix pipe), not a
    /// coarse ruled-strip loft that exports faceted STEP.
    #[test]
    fn multi_turn_helical_revolve_tessellates_leanly() {
        // Rectangular profile offset from the X axis (matches a spring coil sketch).
        let profile = [
            Vec3::new(0.0, 50.0, 0.0),
            Vec3::new(20.0, 50.0, 0.0),
            Vec3::new(20.0, 60.0, 0.0),
            Vec3::new(0.0, 60.0, 0.0),
        ];
        let origin = Vec3::ZERO;
        let axis = Vec3::X;
        // 20 turns × 30 mm pitch — same order as the issue-1248 fixture.
        let angle = 20.0 * std::f64::consts::TAU;
        let pitch = 30.0;
        let sh = Shape::revolve(&profile, origin, axis, angle, false, pitch)
            .expect("helical revolve");
        let vol = sh.volume().expect("volume");
        assert!(vol > 1.0, "spring must have solid volume, got {vol}");
        let tris = sh.tessellate(0.05);
        assert!(
            !tris.is_empty() && tris.len() < 40_000,
            "multi-turn helix must not explode under default deflection, got {} tris",
            tris.len()
        );
    }

    /// #1249: helical revolve BREP must be curved (refines under tighter deflection),
    /// not planar ruled strips whose triangle count is deflection-invariant.
    #[test]
    fn helical_revolve_brep_has_curved_surfaces() {
        let profile = [
            Vec3::new(0.0, 20.0, 0.0),
            Vec3::new(5.0, 20.0, 0.0),
            Vec3::new(5.0, 24.0, 0.0),
            Vec3::new(0.0, 24.0, 0.0),
        ];
        // Compact 2-turn spring so the adaptive deflection floor stays tiny.
        let sh = Shape::revolve(
            &profile,
            Vec3::ZERO,
            Vec3::Y,
            2.0 * std::f64::consts::TAU,
            false,
            10.0,
        )
        .expect("helical revolve");
        let coarse = sh.tessellate(2.0).len();
        let fine = sh.tessellate(0.05).len();
        assert!(
            fine > coarse && fine as f64 > (coarse as f64) * 1.4,
            "smooth helical surfaces must refine under tighter deflection \
             (coarse={coarse}, fine={fine}); planar ruled strips would not"
        );
        let vol = sh.volume().expect("volume");
        assert!(vol > 1.0, "spring volume, got {vol}");
    }
}
