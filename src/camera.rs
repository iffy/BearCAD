//! Orbit camera and 3D→2D projection for the viewport.
//!
//! Until the wgpu/OCCT 3D pipeline lands (SPEC §1, §10), the viewport is drawn
//! by projecting world-space geometry to screen with egui's 2D painter. This
//! module owns the camera state and the project/unproject math.
//!
//! World convention: **Z is up, the ground plane is XY** (z = 0).

use egui::{Pos2, Rect};
use glam::{Mat4, Vec3, Vec4};

/// A look-at orbit camera parameterised in spherical coordinates around a
/// `target` point.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Point the camera orbits and looks at, in world space.
    pub target: Vec3,
    /// Azimuth around the up (Z) axis, radians.
    pub yaw: f32,
    /// Elevation above the ground plane, radians. Clamped away from straight
    /// down/up so the look-at `up` vector never degenerates.
    pub pitch: f32,
    /// Distance from `target` to the eye.
    pub distance: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: 0.8,
            pitch: 0.6,
            distance: 400.0,
            fov_y: 45f32.to_radians(),
        }
    }
}

const PITCH_LIMIT: f32 = 1.54; // ~88°, just shy of the singularity at 90°.

impl Camera {
    /// Eye position in world space.
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + self.distance * Vec3::new(cp * cy, cp * sy, sp)
    }

    /// Orbit by a screen-space drag delta (in points).
    pub fn orbit(&mut self, delta: egui::Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch + delta.y * 0.01).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Pan: slide the look-at `target` (and therefore the eye) in the camera's
    /// view plane by a screen-space drag delta. Scaled so the point under the
    /// cursor tracks it regardless of zoom level.
    pub fn pan(&mut self, delta: egui::Vec2, viewport_height: f32) {
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(Vec3::Z).normalize();
        let up = right.cross(forward).normalize();
        let world_per_px =
            2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_height.max(1.0);
        self.target += (-right * delta.x + up * delta.y) * world_per_px;
    }

    /// Dolly in/out from a scroll amount (positive = zoom in).
    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (1.0 - scroll * 0.001)).clamp(2.0, 50_000.0);
    }

    /// Combined view-projection matrix for the given viewport rectangle.
    pub fn view_proj(&self, viewport: Rect) -> Mat4 {
        let aspect = (viewport.width() / viewport.height().max(1.0)).max(0.01);
        let proj = Mat4::perspective_rh(self.fov_y, aspect, 0.1, 100_000.0);
        let view = Mat4::look_at_rh(self.eye(), self.target, Vec3::Z);
        proj * view
    }

    /// Project a world point to a screen position, or `None` if it is behind
    /// the camera.
    pub fn project(&self, world: Vec3, viewport: Rect, vp: &Mat4) -> Option<Pos2> {
        let clip = *vp * world.extend(1.0);
        if clip.w <= 1e-4 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        let x = viewport.min.x + (ndc.x * 0.5 + 0.5) * viewport.width();
        let y = viewport.min.y + (1.0 - (ndc.y * 0.5 + 0.5)) * viewport.height();
        Some(Pos2::new(x, y))
    }

    /// Cast a ray from the screen pixel onto the ground plane (z = 0) and return
    /// the hit point, or `None` if the ray misses (points at/above the horizon).
    pub fn ground_point(&self, screen: Pos2, viewport: Rect, vp: &Mat4) -> Option<Vec3> {
        let inv = vp.inverse();
        let ndc_x = ((screen.x - viewport.min.x) / viewport.width()) * 2.0 - 1.0;
        let ndc_y = (1.0 - (screen.y - viewport.min.y) / viewport.height()) * 2.0 - 1.0;

        let near = inv * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;

        let dir = far - near;
        if dir.z.abs() < 1e-6 {
            return None; // Ray parallel to the ground.
        }
        let t = -near.z / dir.z;
        if t < 0.0 {
            return None; // Ground is behind the camera.
        }
        Some(near + dir * t)
    }
}
