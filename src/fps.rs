//! First-person (FPS) mode (#91): walk on the ground plane with WASD + mouse look,
//! Space to jump, double-tap Space to toggle Minecraft-style flying, and weapon-style
//! tool switching (1-9 slots, wheel cycles). The controller owns the player's eye and
//! look angles and *writes* the orbit [`Camera`](crate::camera::Camera) every frame
//! (`target = eye + look`, unit distance), so rendering, picking, and every gizmo keep
//! working unchanged — the crosshair at the viewport center is the pointer.
//!
//! The document is millimeters, so the player is person-scale in mm.

use glam::Vec3;

/// Eye height above the ground plane (standing), mm.
pub const EYE_HEIGHT: f32 = 1700.0;
/// Walking speed, mm/s.
pub const WALK_SPEED: f32 = 4300.0;
/// Flying speed, mm/s (horizontal and vertical).
pub const FLY_SPEED: f32 = 9000.0;
/// Gravity, mm/s².
pub const GRAVITY: f32 = 9810.0;
/// Initial upward speed of a jump, mm/s (~0.9 m apex).
pub const JUMP_SPEED: f32 = 4300.0;
/// Mouse-look sensitivity, radians per pixel of pointer motion.
pub const LOOK_SENSITIVITY: f32 = 0.0025;
/// Two Space presses within this window (seconds) toggle flying.
pub const DOUBLE_TAP_WINDOW: f32 = 0.35;
/// Look pitch clamp, radians — inside the camera's own ±88° pole clamp.
const PITCH_RANGE: f32 = 1.53;
/// Where the orbit camera's `target` sits along the look ray, mm. Arbitrary but
/// non-degenerate; also the natural "focus" distance for plane/face picking.
const LOOK_TARGET_DISTANCE: f32 = 1000.0;

/// Smallest player [`FpsController::scale`] (#120): eye height 17 mm, small enough to work
/// at mm-detail scale.
pub const MIN_SCALE: f32 = 0.01;
/// Largest player scale (#120): eye height 170 m, for walking building-sized models.
pub const MAX_SCALE: f32 = 100.0;
/// Each `[`/`]` keypress multiplies or divides the current scale by this factor.
pub const SCALE_STEP: f32 = 2.0;

/// Per-frame movement intent, decoded from held keys by the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FpsInput {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    /// Space edge (just pressed) — jump, or double-tap to toggle flying.
    pub jump_pressed: bool,
    /// Space held — ascend while flying.
    pub ascend: bool,
    /// Shift held — descend while flying.
    pub descend: bool,
}

/// The player: eye position, look angles, and vertical physics state.
#[derive(Clone, Debug, PartialEq)]
pub struct FpsController {
    /// Eye position in world space (z is height above the ground plane).
    pub eye: Vec3,
    /// Look azimuth around +Z, radians (0 = +X, counter-clockwise from above).
    pub yaw: f32,
    /// Look elevation, radians, clamped to ±[`PITCH_RANGE`].
    pub pitch: f32,
    /// Vertical speed while airborne (walking mode), mm/s.
    pub vertical_speed: f32,
    /// Minecraft-style flying: no gravity, Space/Shift ascend/descend.
    pub flying: bool,
    /// Seconds since the previous Space press, while within the double-tap window.
    space_tap_age: Option<f32>,
    /// Uniform scale on the player's spatial constants — eye height, movement/jump speed,
    /// gravity, and look-target distance (#120): shrink to work at mm-detail scale, grow to
    /// cover building/meter-scale models quickly. 1.0 is the human-scale baseline the other
    /// constants in this module are tuned for; angular quantities (yaw/pitch/sensitivity)
    /// are unaffected. Clamped to [`MIN_SCALE`, `MAX_SCALE`].
    pub scale: f32,
}

impl FpsController {
    /// Enter FPS mode from wherever the orbit camera is looking: keep the horizontal
    /// position and look direction, but stand on the ground plane.
    pub fn enter(cam: &crate::camera::Camera) -> Self {
        let look = (cam.target - cam.eye()).normalize_or_zero();
        let look = if look.length_squared() < 0.5 { Vec3::X } else { look };
        let mut eye = cam.eye();
        eye.z = EYE_HEIGHT;
        FpsController {
            eye,
            yaw: look.y.atan2(look.x),
            pitch: look.z.clamp(-1.0, 1.0).asin().clamp(-PITCH_RANGE, PITCH_RANGE),
            vertical_speed: 0.0,
            flying: false,
            space_tap_age: None,
            scale: 1.0,
        }
    }

    /// Set the player's scale directly (#120), clamped to [`MIN_SCALE`, `MAX_SCALE`]. Eye
    /// height and vertical speed scale with it — by the same ratio — so a grounded player
    /// stays exactly grounded and momentum stays proportional across the change.
    pub fn set_scale(&mut self, scale: f32) {
        let new_scale = scale.clamp(MIN_SCALE, MAX_SCALE);
        let ratio = new_scale / self.scale;
        self.eye.z *= ratio;
        self.vertical_speed *= ratio;
        self.scale = new_scale;
    }

    /// Multiply the current scale by `factor` (e.g. [`SCALE_STEP`] to grow, its reciprocal
    /// to shrink); see [`Self::set_scale`].
    pub fn scale_by(&mut self, factor: f32) {
        self.set_scale(self.scale * factor);
    }

    /// Unit look direction.
    pub fn look_dir(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * cy, cp * sy, sp)
    }

    /// Ground-projected forward (what W walks along, even while looking up/down).
    pub fn ground_forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, sy, 0.0)
    }

    /// Ground-projected right (what D strafes along).
    pub fn ground_right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(sy, -cy, 0.0)
    }

    /// Standing on the ground (not flying, not mid-jump).
    pub fn on_ground(&self) -> bool {
        !self.flying && self.eye.z <= EYE_HEIGHT * self.scale + 1e-3 && self.vertical_speed <= 0.0
    }

    /// Mouse-look by a raw pointer delta in pixels (right/down positive).
    pub fn look_by_pixels(&mut self, dx: f32, dy: f32) {
        self.look_by_angles(-dx * LOOK_SENSITIVITY, -dy * LOOK_SENSITIVITY);
    }

    /// Turn by explicit angles, radians (positive = turn left / look up).
    pub fn look_by_angles(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-PITCH_RANGE, PITCH_RANGE);
    }

    /// Integrate one frame of movement.
    pub fn tick(&mut self, dt: f32, input: FpsInput) {
        let dt = dt.clamp(0.0, 0.1); // a hitched frame must not launch the player
        let eye_height = EYE_HEIGHT * self.scale;
        let walk_speed = WALK_SPEED * self.scale;
        let fly_speed = FLY_SPEED * self.scale;
        let jump_speed = JUMP_SPEED * self.scale;
        let gravity = GRAVITY * self.scale;
        if input.jump_pressed {
            if self.space_tap_age.is_some() {
                // Double-tap: toggle flying (landing resumes gravity from rest).
                self.flying = !self.flying;
                self.vertical_speed = 0.0;
                self.space_tap_age = None;
            } else {
                if !self.flying && self.on_ground() {
                    self.vertical_speed = jump_speed;
                }
                self.space_tap_age = Some(0.0);
            }
        } else if let Some(age) = &mut self.space_tap_age {
            *age += dt;
            if *age > DOUBLE_TAP_WINDOW {
                self.space_tap_age = None;
            }
        }

        let axis = |pos: bool, neg: bool| (pos as i8 - neg as i8) as f32;
        let wish = self.ground_forward() * axis(input.forward, input.back)
            + self.ground_right() * axis(input.right, input.left);
        let speed = if self.flying { fly_speed } else { walk_speed };
        self.eye += wish.normalize_or_zero() * speed * dt;

        if self.flying {
            self.eye.z += axis(input.ascend, input.descend) * fly_speed * dt;
            // Flew into the ground: land and go back to walking. Strictly below, so
            // hovering exactly at eye height doesn't count as touching down.
            if self.eye.z < eye_height {
                self.eye.z = eye_height;
                self.flying = false;
                self.vertical_speed = 0.0;
            }
        } else {
            self.vertical_speed -= gravity * dt;
            self.eye.z += self.vertical_speed * dt;
            // Land only while falling: a fresh jump starts at exactly ground height
            // with upward speed, and must not be clamped back down the same frame.
            if self.eye.z <= eye_height && self.vertical_speed <= 0.0 {
                self.eye.z = eye_height;
                self.vertical_speed = 0.0;
            }
        }
    }

    /// Write the player's view into the orbit camera (see module docs).
    pub fn apply_to_camera(&self, cam: &mut crate::camera::Camera) {
        cam.set_first_person(self.eye, self.look_dir(), LOOK_TARGET_DISTANCE * self.scale);
    }
}

/// Weapon-style tool slots (#91): number keys 1-9 pick, the wheel cycles the full list.
pub const TOOL_SLOTS: &[crate::actions::Tool] = &[
    crate::actions::Tool::Select,
    crate::actions::Tool::Sketch,
    crate::actions::Tool::Rectangle,
    crate::actions::Tool::Line,
    crate::actions::Tool::Circle,
    crate::actions::Tool::Extrude,
    crate::actions::Tool::Dimension,
    crate::actions::Tool::Constraint,
    crate::actions::Tool::ConstructionPlane,
    crate::actions::Tool::Chamfer,
    crate::actions::Tool::Fillet,
];

/// The next/previous tool in the slot cycle (wheel up = previous, down = next).
pub fn cycle_tool(current: crate::actions::Tool, step: i32) -> crate::actions::Tool {
    let len = TOOL_SLOTS.len() as i32;
    let index = TOOL_SLOTS
        .iter()
        .position(|t| *t == current)
        .map(|i| i as i32)
        .unwrap_or(0);
    TOOL_SLOTS[((index + step).rem_euclid(len)) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grounded() -> FpsController {
        FpsController {
            eye: Vec3::new(0.0, 0.0, EYE_HEIGHT),
            yaw: 0.0,
            pitch: 0.0,
            vertical_speed: 0.0,
            flying: false,
            space_tap_age: None,
            scale: 1.0,
        }
    }

    #[test]
    fn walking_moves_along_ground_forward_even_while_looking_up() {
        let mut p = grounded();
        p.pitch = 1.0; // looking well above the horizon
        p.tick(
            1.0,
            FpsInput {
                forward: true,
                ..Default::default()
            },
        );
        assert!((p.eye.x - WALK_SPEED * 0.1).abs() < 1e-2, "x={}", p.eye.x); // dt clamped to 0.1
        assert!((p.eye.z - EYE_HEIGHT).abs() < 1e-3, "walking must stay on the ground");
    }

    #[test]
    fn strafing_is_perpendicular_to_forward() {
        let mut p = grounded();
        p.yaw = std::f32::consts::FRAC_PI_2; // facing +Y
        p.tick(
            0.1,
            FpsInput {
                right: true,
                ..Default::default()
            },
        );
        assert!(p.eye.x > 1.0, "strafing right while facing +Y moves +X, got {:?}", p.eye);
        assert!(p.eye.y.abs() < 1e-2);
    }

    #[test]
    fn jump_rises_then_gravity_brings_the_player_back_down() {
        let mut p = grounded();
        p.tick(
            0.01,
            FpsInput {
                jump_pressed: true,
                ..Default::default()
            },
        );
        assert!(p.eye.z > EYE_HEIGHT, "jump should leave the ground");
        let apex_ceiling = EYE_HEIGHT + JUMP_SPEED * JUMP_SPEED / (2.0 * GRAVITY) + 1.0;
        let mut apex = p.eye.z;
        for _ in 0..200 {
            p.tick(0.01, FpsInput::default());
            apex = apex.max(p.eye.z);
        }
        assert!(apex <= apex_ceiling, "apex {apex} above ballistic ceiling {apex_ceiling}");
        assert!(p.on_ground(), "gravity should land the player, z={}", p.eye.z);
    }

    #[test]
    fn jumping_midair_does_nothing() {
        let mut p = grounded();
        p.tick(0.01, FpsInput { jump_pressed: true, ..Default::default() });
        // Wait past the double-tap window, then press Space again mid-air.
        for _ in 0..40 {
            p.tick(0.01, FpsInput::default());
        }
        assert!(p.eye.z > EYE_HEIGHT && !p.on_ground());
        let v_before = p.vertical_speed;
        p.tick(0.01, FpsInput { jump_pressed: true, ..Default::default() });
        assert!(p.vertical_speed <= v_before, "mid-air jump must not add lift");
        assert!(!p.flying);
    }

    #[test]
    fn double_space_toggles_flying_and_space_shift_steer_altitude() {
        let mut p = grounded();
        p.tick(0.01, FpsInput { jump_pressed: true, ..Default::default() });
        p.tick(0.01, FpsInput { jump_pressed: true, ..Default::default() });
        assert!(p.flying, "double-tap Space should start flying");

        let z0 = p.eye.z;
        p.tick(0.1, FpsInput { ascend: true, ..Default::default() });
        assert!(p.eye.z > z0, "Space held should ascend while flying");
        let z1 = p.eye.z;
        p.tick(0.05, FpsInput { descend: true, ..Default::default() });
        assert!(p.eye.z < z1, "Shift held should descend while flying");
        for _ in 0..50 {
            p.tick(0.1, FpsInput::default());
        }
        assert!((p.eye.z - z1 + FLY_SPEED * 0.05).abs() < 1.0, "flying must not fall");
    }

    #[test]
    fn descending_into_the_ground_lands_and_stops_flying() {
        let mut p = grounded();
        p.flying = true;
        p.eye.z = EYE_HEIGHT + 100.0;
        for _ in 0..10 {
            p.tick(0.05, FpsInput { descend: true, ..Default::default() });
        }
        assert!(!p.flying, "touching down should end flying");
        assert!((p.eye.z - EYE_HEIGHT).abs() < 1e-3);
    }

    #[test]
    fn look_pitch_is_clamped_short_of_the_poles() {
        let mut p = grounded();
        p.look_by_angles(0.3, 10.0);
        assert!(p.pitch <= PITCH_RANGE);
        p.look_by_angles(0.0, -20.0);
        assert!(p.pitch >= -PITCH_RANGE);
        assert!((p.yaw - 0.3).abs() < 1e-6, "yaw is unclamped");
    }

    #[test]
    fn mouse_right_turns_right_and_camera_matches_look() {
        let mut p = grounded();
        p.look_by_pixels(100.0, 0.0);
        assert!(p.yaw < 0.0, "mouse right should turn clockwise (yaw decreases)");

        let mut cam = crate::camera::Camera::default();
        p.apply_to_camera(&mut cam);
        assert!((cam.eye() - p.eye).length() < 1e-2, "camera eye should sit at the player eye");
        let cam_look = (cam.target - cam.eye()).normalize();
        assert!((cam_look - p.look_dir()).length() < 1e-3, "camera look should match");
    }

    #[test]
    fn tool_slot_cycle_wraps_both_ways() {
        use crate::actions::Tool;
        assert_eq!(cycle_tool(Tool::Select, 1), Tool::Sketch);
        assert_eq!(cycle_tool(Tool::Select, -1), Tool::Fillet);
        assert_eq!(cycle_tool(Tool::Fillet, 1), Tool::Select);
        let mut tool = Tool::Select;
        for _ in 0..TOOL_SLOTS.len() {
            tool = cycle_tool(tool, 1);
        }
        assert_eq!(tool, Tool::Select, "a full cycle returns to the start");
    }

    #[test]
    fn scaling_up_keeps_a_grounded_player_grounded_at_the_new_eye_height() {
        let mut p = grounded();
        p.set_scale(10.0);
        assert!((p.eye.z - EYE_HEIGHT * 10.0).abs() < 1e-2);
        assert!(p.on_ground(), "rescaling a grounded player must leave them grounded");
    }

    #[test]
    fn scaling_down_keeps_a_midair_player_proportionally_placed() {
        let mut p = grounded();
        p.eye.z = EYE_HEIGHT * 2.0; // twice normal eye height off the ground
        p.set_scale(0.5);
        // Halving scale should also halve the eye's height above ground.
        assert!(
            (p.eye.z - EYE_HEIGHT).abs() < 1e-2,
            "eye.z should scale by the same ratio as scale, got {}",
            p.eye.z
        );
    }

    #[test]
    fn scale_is_clamped_to_the_documented_range() {
        let mut p = grounded();
        p.set_scale(1e6);
        assert_eq!(p.scale, MAX_SCALE);
        p.set_scale(-5.0);
        assert_eq!(p.scale, MIN_SCALE);
    }

    #[test]
    fn scale_by_multiplies_the_current_scale() {
        let mut p = grounded();
        p.scale_by(SCALE_STEP);
        assert!((p.scale - SCALE_STEP).abs() < 1e-4);
        p.scale_by(1.0 / SCALE_STEP);
        assert!((p.scale - 1.0).abs() < 1e-4, "growing then shrinking should round-trip");
    }

    #[test]
    fn a_shrunk_player_walks_and_jumps_proportionally_slower() {
        let mut small = grounded();
        small.set_scale(0.1);
        small.tick(0.1, FpsInput { forward: true, ..Default::default() });
        let mut normal = grounded();
        normal.tick(0.1, FpsInput { forward: true, ..Default::default() });
        assert!(
            (small.eye.x - normal.eye.x * 0.1).abs() < 1e-2,
            "walking distance should scale with player size: small={} normal={}",
            small.eye.x,
            normal.eye.x
        );
    }

    #[test]
    fn a_grown_player_jump_apex_scales_with_size() {
        let mut giant = grounded();
        giant.set_scale(4.0);
        giant.tick(0.01, FpsInput { jump_pressed: true, ..Default::default() });
        let mut apex = giant.eye.z;
        for _ in 0..400 {
            giant.tick(0.01, FpsInput::default());
            apex = apex.max(giant.eye.z);
        }
        let ground = EYE_HEIGHT * 4.0;
        let jump_height = apex - ground;
        // At normal scale the apex is JUMP_SPEED^2 / (2*GRAVITY) above the ground; since
        // both jump speed and gravity scale by the same factor, apex height scales linearly.
        let expected = 4.0 * JUMP_SPEED * JUMP_SPEED / (2.0 * GRAVITY);
        // Euler-integrated physics (dt = 0.01) drifts a few percent from the closed-form
        // ballistic apex even at scale 1 (see `jump_rises_then_gravity_brings_the_player_back_down`);
        // a relative tolerance keeps this test about the *scaling*, not integrator error.
        assert!(
            (jump_height - expected).abs() / expected < 0.05,
            "jump height should scale linearly with player size: got {jump_height}, expected {expected}"
        );
    }
}
