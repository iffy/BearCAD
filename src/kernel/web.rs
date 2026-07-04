//! Web (wasm32) backend for the OCCT kernel: the same safe API as the native FFI, but the
//! kernel is a *second* wasm module (OCCT + the C++ shim compiled with Emscripten — see
//! `scripts/build-occt-wasm.sh`) reached through the JS bridge `web/kernel-bridge.js`.
//! Shape handles are the kernel module's heap pointers, carried here as plain `u32`s.
//!
//! If the hosting page failed to load the kernel module, every bridge call degrades (null
//! results), and this backend reports "not available" — the same graceful story as a
//! native build without `--features occt`.

use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(module = "/web/kernel-bridge.js")]
extern "C" {
    fn kernel_available() -> bool;
    fn kernel_box_volume(dx: f64, dy: f64, dz: f64) -> f64;
    fn kernel_occt_version() -> String;
    fn kernel_prism(xyz: &[f64], dx: f64, dy: f64, dz: f64) -> u32;
    fn kernel_cylinder(
        cx: f64,
        cy: f64,
        cz: f64,
        ax: f64,
        ay: f64,
        az: f64,
        radius: f64,
        height: f64,
    ) -> u32;
    fn kernel_loft(bottom: &[f64], top: &[f64]) -> u32;
    fn kernel_boolean(a: u32, b: u32, op: i32) -> u32;
    fn kernel_fillet(h: u32, edges: &[f64], radii: &[f64]) -> u32;
    fn kernel_chamfer(h: u32, edges: &[f64], dists: &[f64]) -> u32;
    fn kernel_volume(h: u32) -> f64;
    fn kernel_tessellate(h: u32, deflection: f64) -> Vec<f64>;
    fn kernel_shape_free(h: u32);
    fn kernel_face_boolean_loop(a: &[f64], b: &[f64], op: i32) -> Option<Vec<f64>>;
    fn kernel_write_step(h: u32) -> Option<Vec<u8>>;
    fn kernel_read_step(bytes: &[u8]) -> u32;
}

pub fn box_volume(dx: f64, dy: f64, dz: f64) -> Option<f64> {
    if !kernel_available() {
        return None;
    }
    let v = kernel_box_volume(dx, dy, dz);
    (v >= 0.0).then_some(v)
}

pub fn occt_version() -> Option<String> {
    let v = kernel_occt_version();
    (!v.is_empty()).then_some(v)
}

/// See the native `face_boolean_loop` — same contract, bridged.
pub fn face_boolean_loop(
    a: &[(f32, f32)],
    b: &[(f32, f32)],
    op: crate::model::BooleanOp,
) -> Option<Vec<(f32, f32)>> {
    if a.len() < 3 || b.len() < 3 || !kernel_available() {
        return None;
    }
    let flat = |pts: &[(f32, f32)]| -> Vec<f64> {
        pts.iter().flat_map(|&(x, y)| [x as f64, y as f64]).collect()
    };
    let code = match op {
        crate::model::BooleanOp::Difference => 1,
        crate::model::BooleanOp::Intersection => 2,
    };
    let doubles = kernel_face_boolean_loop(&flat(a), &flat(b), code)?;
    let out: Vec<(f32, f32)> = doubles
        .chunks_exact(2)
        .map(|c| (c[0] as f32, c[1] as f32))
        .collect();
    (out.len() >= 3).then_some(out)
}

/// An owned kernel-module BREP solid (see the native `Shape` for semantics).
pub struct Shape {
    handle: u32,
}

fn flat_points(pts: &[glam::Vec3]) -> Vec<f64> {
    pts.iter()
        .flat_map(|p| [p.x as f64, p.y as f64, p.z as f64])
        .collect()
}

fn flat_edges(edges: &[(glam::Vec3, glam::Vec3)]) -> Vec<f64> {
    edges
        .iter()
        .flat_map(|(a, b)| {
            [
                a.x as f64, a.y as f64, a.z as f64, b.x as f64, b.y as f64, b.z as f64,
            ]
        })
        .collect()
}

impl Shape {
    fn from_handle(handle: u32) -> Option<Shape> {
        (handle != 0).then_some(Shape { handle })
    }

    pub fn prism(profile: &[glam::Vec3], dir: glam::Vec3) -> Option<Shape> {
        if profile.len() < 3 || !kernel_available() {
            return None;
        }
        Self::from_handle(kernel_prism(
            &flat_points(profile),
            dir.x as f64,
            dir.y as f64,
            dir.z as f64,
        ))
    }

    pub fn cylinder(center: glam::Vec3, axis: glam::Vec3, radius: f64, height: f64) -> Option<Shape> {
        if radius <= 0.0 || height <= 0.0 || axis.length_squared() < 1e-12 || !kernel_available() {
            return None;
        }
        Self::from_handle(kernel_cylinder(
            center.x as f64,
            center.y as f64,
            center.z as f64,
            axis.x as f64,
            axis.y as f64,
            axis.z as f64,
            radius,
            height,
        ))
    }

    pub fn loft(bottom: &[glam::Vec3], top: &[glam::Vec3]) -> Option<Shape> {
        if bottom.len() < 3 || bottom.len() != top.len() || !kernel_available() {
            return None;
        }
        Self::from_handle(kernel_loft(&flat_points(bottom), &flat_points(top)))
    }

    pub fn boolean(&self, other: &Shape, op: super::BoolOp) -> Option<Shape> {
        let code = match op {
            super::BoolOp::Fuse => 0,
            super::BoolOp::Cut => 1,
            super::BoolOp::Common => 2,
        };
        Self::from_handle(kernel_boolean(self.handle, other.handle, code))
    }

    pub fn fillet(&self, edges: &[(glam::Vec3, glam::Vec3)], radii: &[f32]) -> Option<Shape> {
        if edges.is_empty() || edges.len() != radii.len() {
            return None;
        }
        let r: Vec<f64> = radii.iter().map(|&x| x as f64).collect();
        Self::from_handle(kernel_fillet(self.handle, &flat_edges(edges), &r))
    }

    pub fn chamfer(&self, edges: &[(glam::Vec3, glam::Vec3)], dists: &[f32]) -> Option<Shape> {
        if edges.is_empty() || edges.len() != dists.len() {
            return None;
        }
        let d: Vec<f64> = dists.iter().map(|&x| x as f64).collect();
        Self::from_handle(kernel_chamfer(self.handle, &flat_edges(edges), &d))
    }

    pub fn volume(&self) -> Option<f64> {
        let v = kernel_volume(self.handle);
        (v >= 0.0).then_some(v)
    }

    pub fn tessellate(&self, deflection: f64) -> Vec<[glam::Vec3; 3]> {
        let doubles = kernel_tessellate(self.handle, deflection);
        doubles
            .chunks_exact(9)
            .map(|c| {
                [
                    glam::Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32),
                    glam::Vec3::new(c[3] as f32, c[4] as f32, c[5] as f32),
                    glam::Vec3::new(c[6] as f32, c[7] as f32, c[8] as f32),
                ]
            })
            .collect()
    }

    /// Path-based STEP IO doesn't exist in the browser; use the byte variants.
    pub fn write_step(&self, _path: &std::path::Path) -> bool {
        false
    }

    pub fn read_step(_path: &std::path::Path) -> Option<Shape> {
        None
    }

    /// STEP file contents for this shape (real BREP via the kernel's writer).
    pub fn write_step_bytes(&self) -> Option<Vec<u8>> {
        kernel_write_step(self.handle)
    }

    /// Read a STEP file's contents into a shape (real BREP via the kernel's reader).
    pub fn read_step_bytes(bytes: &[u8]) -> Option<Shape> {
        if !kernel_available() {
            return None;
        }
        Self::from_handle(kernel_read_step(bytes))
    }
}

impl Drop for Shape {
    fn drop(&mut self) {
        kernel_shape_free(self.handle);
    }
}
