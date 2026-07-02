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
        }
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
        !self.flying && self.eye.z <= EYE_HEIGHT + 1e-3 && self.vertical_speed <= 0.0
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
        if input.jump_pressed {
            if self.space_tap_age.is_some() {
                // Double-tap: toggle flying (landing resumes gravity from rest).
                self.flying = !self.flying;
                self.vertical_speed = 0.0;
                self.space_tap_age = None;
            } else {
                if !self.flying && self.on_ground() {
                    self.vertical_speed = JUMP_SPEED;
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
        let speed = if self.flying { FLY_SPEED } else { WALK_SPEED };
        self.eye += wish.normalize_or_zero() * speed * dt;

        if self.flying {
            self.eye.z += axis(input.ascend, input.descend) * FLY_SPEED * dt;
            // Flew into the ground: land and go back to walking. Strictly below, so
            // hovering exactly at eye height doesn't count as touching down.
            if self.eye.z < EYE_HEIGHT {
                self.eye.z = EYE_HEIGHT;
                self.flying = false;
                self.vertical_speed = 0.0;
            }
        } else {
            self.vertical_speed -= GRAVITY * dt;
            self.eye.z += self.vertical_speed * dt;
            // Land only while falling: a fresh jump starts at exactly ground height
            // with upward speed, and must not be clamped back down the same frame.
            if self.eye.z <= EYE_HEIGHT && self.vertical_speed <= 0.0 {
                self.eye.z = EYE_HEIGHT;
                self.vertical_speed = 0.0;
            }
        }
    }

    /// Write the player's view into the orbit camera (see module docs).
    pub fn apply_to_camera(&self, cam: &mut crate::camera::Camera) {
        cam.set_first_person(self.eye, self.look_dir(), LOOK_TARGET_DISTANCE);
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
}
