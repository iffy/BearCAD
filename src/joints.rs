//! Joint kinematics (#893): turn each joint's kind + frames + position into a rigid
//! transform and apply it to the driven side **in place** — no output bodies, the same
//! shape as a Move's plane/image/instance targets.
//!
//! Joints form a tree rooted at whatever is grounded (a member no joint drives). Poses
//! resolve in dependency order so a chain (A→B→C) composes; a joint that would close a
//! loop, or drive a part another joint already drives, is skipped and reported through
//! document health.

use crate::model::{Document, Joint, JointKind, JointRef};
use glam::{Mat3, Mat4, Vec3};
use std::collections::HashMap;

/// The outcome of resolving every live joint: a world pose per driven member, plus the
/// joints that couldn't apply and why (cycles, double-driven parts).
#[derive(Default, Debug, Clone)]
pub struct JointResolution {
    poses: HashMap<JointRef, Mat4>,
    /// Joint index → the reason it couldn't apply.
    pub errors: Vec<(crate::model::JointKey, String)>,
}

impl JointResolution {
    /// The pose a member carries, if any joint drives it (directly — component/instance
    /// containment is [`body_joint_pose`]'s job).
    pub fn member_pose(&self, member: JointRef) -> Option<Mat4> {
        self.poses.get(&member).copied()
    }
}

/// A resolved mating frame: an origin and a right-handed orthonormal basis. `x` is the
/// primary axis (slide direction / turn axis), `y` the secondary.
struct Frame {
    origin: Vec3,
    x: Vec3,
    y: Vec3,
    z: Vec3,
}

fn frame_mat(f: &Frame) -> Mat4 {
    Mat4::from_cols(
        f.x.extend(0.0),
        f.y.extend(0.0),
        f.z.extend(0.0),
        f.origin.extend(1.0),
    )
}

/// The frame a joint's freedoms act in (#1021/#1079): the joint's **own** `frame` where one
/// is set, else the mating plane it used to be derived from.
///
/// It used to be derived from the mate alone — the primary axis *was* the mating normal, and
/// there was no way to say otherwise. That only worked because a mate was always a face on a
/// face; now that a joint's placement is an ordinary move (#1068), three of the four move
/// modes name no plane, so the frame is the user's to set (#1079). The mate seeds it and the
/// pane can change it.
///
/// The fallback keeps the old rule exactly: the primary axis is the mating normal — a part
/// spun, tilted or screwed on a face turns about the face it sits on — except for the two
/// kinds whose slide is travel rather than lift. A Slider and a PinSlot take the in-plane
/// direction instead, because a part flush on a face slides **along** it, not off it.
fn mate_frame(doc: &Document, joint: &Joint, p: &crate::mate::Placement) -> Frame {
    let stored = joint_frame_axes(doc, &joint.frame);
    // The origin defaults to the mate's landing point — where the moving face ended up — not
    // to the axis's own reference point: choosing an axis says which way the part moves, not
    // where the mate put it. Picking an origin outright overrides that.
    let origin = match &joint.frame.origin {
        Some(point) => crate::extrude::move_point_world(doc, point).unwrap_or(p.origin),
        None => p.origin,
    };
    let (n, d) = match stored {
        Some((primary, secondary)) => (primary, secondary),
        None => (p.normal, p.along),
    };
    // A frame the user set says which axis is which outright; only the mate-derived fallback
    // still swaps them by kind.
    let (x, y) = match (stored.is_some(), &joint.kind) {
        (false, JointKind::Slider) | (false, JointKind::PinSlot) => (d, n),
        (false, _) => (n, d),
        (true, _) => (n, d),
    };
    let y = (y - x * y.dot(x)).normalize_or_zero();
    let y = if y.length_squared() > 0.5 { y } else { x.any_orthonormal_vector() };
    Frame { origin, x, y, z: x.cross(y).normalize_or_zero() }
}

/// A stored [`crate::model::JointFrame`]'s two axes (#1079). `None` when no primary axis is
/// set or it no longer resolves — a frame without one has nothing to say, so the mate's own
/// plane answers instead.
fn joint_frame_axes(doc: &Document, frame: &crate::model::JointFrame) -> Option<(Vec3, Vec3)> {
    let primary = crate::mate::mate_ref_direction(doc, frame.primary.as_ref()?)?;
    let secondary = frame
        .secondary
        .as_ref()
        .and_then(|r| crate::mate::mate_ref_direction(doc, r))
        .unwrap_or_else(|| primary.any_orthonormal_vector());
    Some((primary, secondary))
}

/// Solve a joint's mate against the base side's pose (#1021). `None` when the face pair is
/// unset or no longer resolves — the joint then mates as identity, which is exactly the
/// join-in-place behaviour (#891): parts that already touch stay put.
fn solve_mate(doc: &Document, joint: &Joint, base_pose: Mat4) -> Option<crate::mate::Placement> {
    crate::mate::placement(doc, &crate::mate::settled(joint), base_pose)
}

/// A joint's resolved travel bounds (#896): mm for the slide, radians for the turn.
/// `None` leaves that end open.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResolvedJointLimits {
    pub slide_min: Option<f32>,
    pub slide_max: Option<f32>,
    pub turn_min: Option<f32>,
    pub turn_max: Option<f32>,
}

impl ResolvedJointLimits {
    pub fn clamp_slide(&self, v: f32) -> f32 {
        let v = self.slide_max.map_or(v, |m| v.min(m));
        self.slide_min.map_or(v, |m| v.max(m))
    }

    pub fn clamp_turn(&self, v: f32) -> f32 {
        let v = self.turn_max.map_or(v, |m| v.min(m));
        self.turn_min.map_or(v, |m| v.max(m))
    }
}

/// Resolve a joint's limits (#896): expressions evaluate against document parameters; a
/// slide stop picked as geometry resolves to wherever the joint's axis meets the target's
/// extended plane (the "extrude to object" idea), recomputed as the model changes — a
/// geometry stop wins over the expression on the same end.
pub fn resolve_limits(doc: &Document, joint: &Joint) -> ResolvedJointLimits {
    resolve_limits_posed(doc, joint, base_pose_of(doc, joint))
}

/// The pose the joint's base side carries, for resolving a mate against where the fixed part
/// actually sits (#1021). Identity for an ungrounded base.
fn base_pose_of(doc: &Document, joint: &Joint) -> Mat4 {
    joint
        .base_member()
        .and_then(|m| joint_resolution(doc).member_pose(m))
        .unwrap_or(Mat4::IDENTITY)
}

fn resolve_limits_posed(doc: &Document, joint: &Joint, base_pose: Mat4) -> ResolvedJointLimits {
    let len = |e: &str| {
        if e.trim().is_empty() {
            None
        } else {
            crate::value::eval_length_mm_in_doc(e, doc)
        }
    };
    let ang = |e: &str| {
        if e.trim().is_empty() {
            None
        } else {
            crate::value::eval_angle_rad_in_doc(e, doc)
        }
    };
    let stop = |target: &Option<crate::model::ExtrudeTarget>| -> Option<f32> {
        let target = target.as_ref()?;
        let frame = mate_frame(doc, joint, &solve_mate(doc, joint, base_pose)?);
        crate::extrude::target_distance(doc, frame.origin, frame.x, target)
    };
    ResolvedJointLimits {
        slide_min: stop(&joint.limits.slide_min_target).or_else(|| len(&joint.limits.slide_min)),
        slide_max: stop(&joint.limits.slide_max_target).or_else(|| len(&joint.limits.slide_max)),
        turn_min: ang(&joint.limits.turn_min),
        turn_max: ang(&joint.limits.turn_max),
    }
}

/// The motion the joint's position expressions ask for, in the mating frame's own
/// coordinates (`x` = primary axis), clamped to its limits (#896). Empty expressions read
/// as zero, so a fresh joint sits at its captured pose. `None` when an expression doesn't
/// evaluate.
fn motion(doc: &Document, joint: &Joint, base_pose: Mat4) -> Option<Mat4> {
    let limits = resolve_limits_posed(doc, joint, base_pose);
    let len = |e: &str| {
        if e.trim().is_empty() {
            Some(0.0)
        } else {
            crate::value::eval_length_mm_in_doc(e, doc)
        }
    };
    let ang = |e: &str| {
        if e.trim().is_empty() {
            Some(0.0)
        } else {
            crate::value::eval_angle_rad_in_doc(e, doc)
        }
    };
    let slide = |e: &str| len(e).map(|v| limits.clamp_slide(v));
    let turn = |e: &str| ang(e).map(|v| limits.clamp_turn(v));
    let rot_x = |a: f32| Mat4::from_mat3(Mat3::from_rotation_x(a));
    let rot_y = |a: f32| Mat4::from_mat3(Mat3::from_rotation_y(a));
    let rot_z = |a: f32| Mat4::from_mat3(Mat3::from_rotation_z(a));
    Some(match &joint.kind {
        JointKind::Rigid => Mat4::IDENTITY,
        JointKind::Slider => Mat4::from_translation(Vec3::X * slide(&joint.position)?),
        JointKind::Revolute => rot_x(turn(&joint.position)?),
        JointKind::Cylindrical => {
            Mat4::from_translation(Vec3::X * slide(&joint.position)?)
                * rot_x(turn(&joint.position2)?)
        }
        // Slide across the plane perpendicular to the primary axis, spin about it. The
        // slide limits bound both in-plane freedoms.
        JointKind::Planar => {
            Mat4::from_translation(
                Vec3::Y * slide(&joint.position)? + Vec3::Z * slide(&joint.position2)?,
            ) * rot_x(turn(&joint.position3)?)
        }
        JointKind::Ball => {
            rot_x(turn(&joint.position)?)
                * rot_y(turn(&joint.position2)?)
                * rot_z(turn(&joint.position3)?)
        }
        // Slide along the primary axis while turning about the secondary one.
        JointKind::PinSlot => {
            Mat4::from_translation(Vec3::X * slide(&joint.position)?)
                * rot_y(turn(&joint.position2)?)
        }
        // The turn drives the travel: one full turn advances by the lead. The slide
        // limits bound the travel too, turned back into an angle through the lead.
        JointKind::Screw { lead } => {
            let mut a = turn(&joint.position)?;
            let lead_mm = len(lead)?;
            if lead_mm.abs() > 1e-6 {
                let travel_angle = |travel: f32| travel / lead_mm * std::f32::consts::TAU;
                if let Some(max) = limits.slide_max {
                    a = a.min(travel_angle(max));
                }
                if let Some(min) = limits.slide_min {
                    a = a.max(travel_angle(min));
                }
            }
            let travel = a / std::f32::consts::TAU * lead_mm;
            Mat4::from_translation(Vec3::X * travel) * rot_x(a)
        }
    })
}

/// The transform a joint imposes on its driven side, given the base side's own pose: put the
/// driven part where the mate says (#1021), then move it through the joint's freedoms about
/// the mate's frame. Falls back to the base pose alone when the mate is unset or no longer
/// resolves — the identity mate that keeps already-placed parts in place.
fn joint_transform(doc: &Document, joint: &Joint, base_pose: Mat4) -> Mat4 {
    let Some(placed) = solve_mate(doc, joint, base_pose) else {
        return base_pose;
    };
    let Some(m) = motion(doc, joint, base_pose) else {
        return placed.transform;
    };
    let frame = frame_mat(&mate_frame(doc, joint, &placed));
    frame * m * frame.inverse() * placed.transform
}

/// Resolve every live joint's pose, in dependency order (#893). A joint is ready once the
/// joint driving its base (if any) has resolved; whatever never becomes ready closes a
/// loop and is reported instead of applied.
pub fn resolve_joint_poses(doc: &Document) -> JointResolution {
    let mut out = JointResolution::default();
    let live: Vec<(crate::model::JointKey, &Joint)> = doc
        .joints
        .iter()
        .filter(|(_, j)| j.members.len() >= 2)
        .collect();
    // Who drives each member: a part can only be driven by one joint — a tree, not a web.
    let mut driver_of: HashMap<JointRef, crate::model::JointKey> = HashMap::new();
    let mut skipped: Vec<crate::model::JointKey> = Vec::new();
    for &(ji, joint) in &live {
        let mut clash = None;
        for m in joint.driven_members() {
            if let Some(&other) = driver_of.get(&m) {
                clash = Some((m, other));
                break;
            }
        }
        if let Some((_, other)) = clash {
            out.errors.push((
                ji,
                format!(
                    "A part this joint drives is already driven by {}",
                    crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Joint(other)),
                ),
            ));
            skipped.push(ji);
            continue;
        }
        for m in joint.driven_members() {
            driver_of.insert(m, ji);
        }
    }
    // Kahn-style: a joint resolves once its base's driver (if any) has.
    let mut done: HashMap<crate::model::JointKey, bool> = HashMap::new();
    let mut progressed = true;
    while progressed {
        progressed = false;
        for &(ji, joint) in &live {
            if done.contains_key(&ji) || skipped.contains(&ji) {
                continue;
            }
            let Some(base) = joint.base_member() else { continue };
            let base_ready = match driver_of.get(&base) {
                None => true,
                Some(&driver) => driver != ji && *done.get(&driver).unwrap_or(&false),
            };
            if !base_ready {
                continue;
            }
            let base_pose = out.poses.get(&base).copied().unwrap_or(Mat4::IDENTITY);
            if motion(doc, joint, base_pose).is_none() {
                out.errors.push((
                    ji,
                    "A joint position expression cannot be evaluated".to_string(),
                ));
            }
            let pose = joint_transform(doc, joint, base_pose);
            for m in joint.driven_members() {
                out.poses.insert(m, pose);
            }
            done.insert(ji, true);
            progressed = true;
        }
    }
    for &(ji, _) in &live {
        if !done.contains_key(&ji) && !skipped.contains(&ji) {
            out.errors
                .push((ji, "This joint would close a loop of joints".to_string()));
        }
    }
    out
}

thread_local! {
    /// Memoized [`resolve_joint_poses`] keyed by the document mesh fingerprint — pose
    /// lookups run per body per frame, the resolution itself only on document change.
    static POSE_CACHE: std::cell::RefCell<(u64, std::rc::Rc<JointResolution>)> =
        std::cell::RefCell::new((0, std::rc::Rc::new(JointResolution::default())));
}

/// The document's resolved joint poses, memoized per document state.
pub fn joint_resolution(doc: &Document) -> std::rc::Rc<JointResolution> {
    if doc.joints.is_empty() {
        return std::rc::Rc::new(JointResolution::default());
    }
    let fingerprint = crate::extrude::document_pose_fingerprint(doc);
    POSE_CACHE.with(|cache| {
        {
            let cache = cache.borrow();
            if cache.0 == fingerprint {
                return cache.1.clone();
            }
        }
        let resolved = std::rc::Rc::new(resolve_joint_poses(doc));
        *cache.borrow_mut() = (fingerprint, resolved.clone());
        resolved
    })
}

/// The joint pose a body carries, if any (#893): driven directly, as the body of a driven
/// unit instance, or by living inside a driven component (nearest component wins).
/// `None` means the body sits where its features put it.
pub fn body_joint_pose(doc: &Document, body_index: crate::model::BodyKey) -> Option<Mat4> {
    if doc.joints.is_empty() {
        return None;
    }
    let res = joint_resolution(doc);
    if let Some(m) = res.member_pose(JointRef::Body(body_index)) {
        return Some(m);
    }
    let body = doc.bodies.get(body_index)?;
    match body.source {
        crate::model::BodySource::UnitInstance(ui)
        | crate::model::BodySource::UnitCut { instance: ui, .. } => {
            if let Some(m) = res.member_pose(JointRef::UnitInstance(ui)) {
                return Some(m);
            }
        }
        _ => {}
    }
    let owning = crate::hierarchy::owning_component(
        doc,
        &crate::hierarchy::SceneElement::Body(body_index),
    )?;
    for c in doc.component_chain(owning) {
        if let Some(m) = res.member_pose(JointRef::Component(c)) {
            return Some(m);
        }
    }
    None
}

/// The live bodies a joint member stands for (#894): the body itself, a unit instance's
/// materialized bodies, or every body living inside a component (nearest-chain match,
/// like [`body_joint_pose`]).
pub fn member_bodies(doc: &Document, member: JointRef) -> Vec<crate::model::BodyKey> {
    match member {
        JointRef::Body(bi) => doc
            .bodies
            .get(bi)
            .map(|_| vec![bi])
            .unwrap_or_default(),
        JointRef::UnitInstance(ui) => doc
            .bodies
            .iter()
            .filter(|(_, b)| {
                matches!(
                        b.source,
                        crate::model::BodySource::UnitInstance(i)
                        | crate::model::BodySource::UnitCut { instance: i, .. }
                        if i == ui
                    )
            })
            .map(|(i, _)| i)
            .collect(),
        JointRef::Component(ci) => doc
            .bodies
            .iter()
            .filter(|(bi, _)| {
                crate::hierarchy::owning_component(
                    doc,
                    &crate::hierarchy::SceneElement::Body(*bi),
                )
                .is_some_and(|owner| doc.component_chain(owner).contains(&ci))
            })
            .map(|(i, _)| i)
            .collect(),
    }
}

/// What a Select-tool drag on a body drives (#897).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyDragTarget {
    /// The body isn't part of any joint — the drag means nothing here.
    Free,
    /// The body is jointed but held: nothing up its chain has freedom to move.
    Grounded,
    /// The nearest joint up the chain with freedom — a hinge swings, a slider slides.
    Joint(crate::model::JointKey),
}

/// Resolve a Select-tool drag on `body` to the joint it should move (#897): the joint
/// driving the body (directly, as a unit instance's body, or through its component),
/// walking up through rigid groups to the nearest freedom. A part that's jointed but has
/// no freedom anywhere up its chain is grounded and refuses the drag.
pub fn body_drag_joint(doc: &Document, body: crate::model::BodyKey) -> BodyDragTarget {
    let mut refs = vec![JointRef::Body(body)];
    if let Some(b) = doc.bodies.get(body) {
        match b.source {
            crate::model::BodySource::UnitInstance(ui)
            | crate::model::BodySource::UnitCut { instance: ui, .. } => {
                refs.push(JointRef::UnitInstance(ui));
            }
            _ => {}
        }
    }
    if let Some(owning) = crate::hierarchy::owning_component(
        doc,
        &crate::hierarchy::SceneElement::Body(body),
    ) {
        for c in doc.component_chain(owning) {
            refs.push(JointRef::Component(c));
        }
    }
    let live = |j: &Joint| j.members.len() >= 2;
    let driver_of = |member: JointRef| -> Option<crate::model::JointKey> {
        doc.joints
            .iter()
            .find(|(_, j)| live(j) && j.driven_members().any(|m| m == member))
            .map(|(i, _)| i)
    };
    let participates = refs
        .iter()
        .any(|r| doc.joints.values().any(|j| live(j) && j.members.contains(r)));
    let Some(mut ji) = refs.iter().find_map(|r| driver_of(*r)) else {
        return if participates {
            BodyDragTarget::Grounded
        } else {
            BodyDragTarget::Free
        };
    };
    // Walk up through rigid ties to the nearest joint that actually moves.
    for _ in 0..doc.joints.len() + 1 {
        let Some(joint) = doc.joints.get(ji) else {
            return BodyDragTarget::Grounded;
        };
        if !matches!(joint.kind, JointKind::Rigid) {
            return BodyDragTarget::Joint(ji);
        }
        let Some(base) = joint.base_member() else {
            return BodyDragTarget::Grounded;
        };
        match driver_of(base) {
            Some(next) => ji = next,
            None => return BodyDragTarget::Grounded,
        }
    }
    BodyDragTarget::Grounded
}

/// The joint's mating frame in world space with the base side's pose applied (#897):
/// origin plus the x/y/z axes the drag projects onto. `None` when the frame is unset —
/// a frameless joint has no axis to drag along.
pub fn posed_joint_frame(
    doc: &Document,
    ji: crate::model::JointKey,
) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    let joint = doc.joints.get(ji)?;
    let base_pose = base_pose_of(doc, joint);
    let frame = mate_frame(doc, joint, &solve_mate(doc, joint, base_pose)?);
    Some((frame.origin, frame.x, frame.y, frame.z))
}

/// A joint's three position slots evaluated to numbers (#897): mm for slide slots,
/// degrees for turn slots — the values a drag starts from and writes back.
pub fn evaluated_positions(doc: &Document, joint: &Joint) -> (f32, f32, f32) {
    let len = |e: &str| {
        if e.trim().is_empty() {
            0.0
        } else {
            crate::value::eval_length_mm_in_doc(e, doc).unwrap_or(0.0)
        }
    };
    let deg = |e: &str| {
        if e.trim().is_empty() {
            0.0
        } else {
            crate::value::eval_angle_rad_in_doc(e, doc)
                .map(|r| r.to_degrees())
                .unwrap_or(0.0)
        }
    };
    match &joint.kind {
        JointKind::Rigid => (0.0, 0.0, 0.0),
        JointKind::Slider => (len(&joint.position), 0.0, 0.0),
        JointKind::Revolute | JointKind::Screw { .. } => (deg(&joint.position), 0.0, 0.0),
        JointKind::Cylindrical | JointKind::PinSlot => {
            (len(&joint.position), deg(&joint.position2), 0.0)
        }
        JointKind::Planar => (
            len(&joint.position),
            len(&joint.position2),
            deg(&joint.position3),
        ),
        JointKind::Ball => (
            deg(&joint.position),
            deg(&joint.position2),
            deg(&joint.position3),
        ),
    }
}

/// One round trip of the create/edit preview sweep (#895), seconds.
pub const JOINT_SWEEP_PERIOD_SECS: f64 = 6.0;

/// The preview sweep's position values at `time` (#895): each of the joint's freedoms
/// glides back and forth through its range — between its limits where they're set, a
/// sensible default span where they aren't (±20 mm, ±30°, or the full turn for a
/// revolute with no limits). Slow, eased (a cosine glide, not a linear scrub), looping.
/// Returns the three position-slot values in the units their expressions read (mm or
/// degrees, per kind); slots the kind doesn't use come back zero. `None` for rigid —
/// there is nothing to sweep.
pub fn sweep_positions(doc: &Document, joint: &Joint, time: f64) -> Option<(f32, f32, f32)> {
    let limits = resolve_limits(doc, joint);
    // Eased ping-pong: 0 → 1 → 0 over one period, easing out at both ends.
    let t = 0.5 - 0.5 * (time * std::f64::consts::TAU / JOINT_SWEEP_PERIOD_SECS).cos() as f32;
    let span = |lo: Option<f32>, hi: Option<f32>, default_lo: f32, default_hi: f32| {
        let lo = lo.unwrap_or(default_lo);
        let hi = hi.unwrap_or(default_hi);
        lo + (hi - lo) * t
    };
    let slide = || span(limits.slide_min, limits.slide_max, -20.0, 20.0);
    let turn_deg = || {
        span(
            limits.turn_min.map(|r| r.to_degrees()),
            limits.turn_max.map(|r| r.to_degrees()),
            -30.0,
            30.0,
        )
    };
    Some(match &joint.kind {
        JointKind::Rigid => return None,
        JointKind::Slider => (slide(), 0.0, 0.0),
        // The full turn for a revolute with no limits at all.
        JointKind::Revolute => {
            if limits.turn_min.is_none() && limits.turn_max.is_none() {
                (360.0 * t, 0.0, 0.0)
            } else {
                (turn_deg(), 0.0, 0.0)
            }
        }
        // Both freedoms sweep together, so the coupling reads.
        JointKind::Cylindrical => (slide(), turn_deg(), 0.0),
        JointKind::Planar => (slide(), slide(), turn_deg()),
        JointKind::Ball => (turn_deg(), turn_deg(), turn_deg()),
        JointKind::PinSlot => (slide(), turn_deg(), 0.0),
        // The turn drives the travel; sweep the angle between whatever bounds bite —
        // turn limits first, else the slide limits turned into angles by the lead,
        // else a full turn either way.
        JointKind::Screw { lead } => {
            let lead_mm = crate::value::eval_length_mm_in_doc(lead, doc).unwrap_or(0.0);
            let angle = if limits.turn_min.is_some() || limits.turn_max.is_some() {
                turn_deg()
            } else if lead_mm.abs() > 1e-6
                && (limits.slide_min.is_some() || limits.slide_max.is_some())
            {
                span(
                    limits.slide_min.map(|s| s / lead_mm * 360.0),
                    limits.slide_max.map(|s| s / lead_mm * 360.0),
                    -360.0,
                    360.0,
                )
            } else {
                span(None, None, -360.0, 360.0)
            };
            (angle, 0.0, 0.0)
        }
    })
}

/// The pose an in-progress joint would impose on its driven side (#894): the committed
/// assembly's base pose composed with the probe's mate — what the tool's ghost shows.
/// `None` when it works out to identity (nothing to ghost).
pub fn preview_pose(doc: &Document, joint: &Joint) -> Option<Mat4> {
    joint.base_member()?;
    let base_pose = base_pose_of(doc, joint);
    let pose = joint_transform(doc, joint, base_pose);
    if pose.abs_diff_eq(Mat4::IDENTITY, 1e-5) {
        None
    } else {
        Some(pose)
    }
}

/// Apply a body's joint pose to its solid mesh, if it carries one.
pub fn posed_mesh(
    doc: &Document,
    body_index: crate::model::BodyKey,
    mesh: crate::extrude::SolidMesh,
) -> crate::extrude::SolidMesh {
    let Some(pose) = body_joint_pose(doc, body_index) else {
        return mesh;
    };
    crate::extrude::SolidMesh {
        triangles: mesh
            .triangles
            .iter()
            .map(|tri| {
                [
                    pose.transform_point3(tri[0]),
                    pose.transform_point3(tri[1]),
                    pose.transform_point3(tri[2]),
                ]
            })
            .collect(),
    }
}


#[cfg(test)]
mod tests {
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::joint_key_for_slot as jkey;
    use super::*;
    use crate::mate::tests::{cube_body, face_ref, joint};
    use crate::model::{JointLimits, JointMate, MateLineUp, MateRef};

    /// The two cubes every test mates: a 10 mm block at the origin and a 4 mm block parked
    /// well away from it, so a placement that fires is unmistakable.
    fn two_cubes(doc: &mut Document) -> (crate::model::BodyKey, crate::model::BodyKey) {
        let fixed = cube_body(doc, Vec3::ZERO, Vec3::splat(10.0));
        let moving = cube_body(doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(4.0));
        (fixed, moving)
    }

    /// The moving cube's underside onto the fixed cube's top: lands it at z = 10, keeping
    /// where it sits in the plane.
    fn stack_mate(doc: &Document, fixed: crate::model::BodyKey, moving: crate::model::BodyKey) -> JointMate {
        JointMate {
            moving_face: Some(face_ref(doc, moving, Vec3::new(42.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            ..Default::default()
        }
    }

    /// A line-up row aiming both parts' near edges along +X, which is what gives a slider a
    /// direction to travel in.
    fn along_x_row(fixed: crate::model::BodyKey, moving: crate::model::BodyKey, moving_lo: Vec3, fixed_lo: Vec3) -> MateLineUp {
        let q = crate::hierarchy::quantize_body_point;
        MateLineUp {
            moving: Some(MateRef::Edge {
                body: moving,
                a: q(moving_lo),
                b: q(moving_lo + Vec3::X * 4.0),
            }),
            fixed: Some(MateRef::Edge {
                body: fixed,
                a: q(fixed_lo),
                b: q(fixed_lo + Vec3::X * 10.0),
            }),
        }
    }

    /// #893/#1021: a joint with no mate places nothing — joining parts already in place
    /// moves them not at all.
    #[test]
    fn mateless_joint_is_a_no_op() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        doc.joints.insert(joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Revolute,
        ));
        let before = crate::extrude::body_solid_mesh_uncached_pub(&doc, b).unwrap();
        let after = posed_mesh(&doc, b, before.clone());
        assert_eq!(before.triangles, after.triangles);
        assert!(resolve_joint_poses(&doc).errors.is_empty());
    }

    /// #893/#1021: the face pair places the part, then the slider carries it along the
    /// line-up row's direction — the mate is the starting position, the kind is the motion.
    #[test]
    fn slider_travels_along_the_line_up() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(vec![JointRef::Body(a), JointRef::Body(b)], JointKind::Slider);
        j.mate = stack_mate(&doc, a, b);
        j.mate.line_up.push(along_x_row(a, b, Vec3::new(40.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 10.0)));
        j.position = "7".to_string();
        doc.joints.insert(j);
        let pose = body_joint_pose(&doc, b).expect("the driven body carries a pose");
        let landed = pose.transform_point3(Vec3::new(40.0, 0.0, 0.0));
        assert!(
            (landed - Vec3::new(47.0, 0.0, 10.0)).length() < 1e-3,
            "landed at {landed}"
        );
        assert!(body_joint_pose(&doc, a).is_none(), "the base is held");
    }

    /// #893/#1021: a revolute turns about the mating normal — a part spun on the face it
    /// sits on. 90° about +Z sends +X to +Y about the landing point.
    #[test]
    fn revolute_turns_about_the_mating_normal() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Revolute,
        );
        j.mate = stack_mate(&doc, a, b);
        j.position = "90".to_string();
        doc.joints.insert(j);
        let pose = body_joint_pose(&doc, b).unwrap();
        // The face's middle lands at (42, 2, 10) and is the turn's centre; a point 2 mm
        // along +X of it swings to 2 mm along +Y.
        let moved = pose.transform_point3(Vec3::new(44.0, 2.0, 0.0));
        assert!(
            (moved - Vec3::new(42.0, 4.0, 10.0)).length() < 1e-3,
            "swung to {moved}"
        );
    }

    /// #1079: the frame is the joint's **own**, not the mate's, once one is set. A Revolute
    /// mated flat on a face turns about that face's normal by default — but say the axis is
    /// +X and it turns about +X instead, without touching where the mate put the part.
    #[test]
    fn a_joints_own_frame_overrides_the_mating_normal() {
        use crate::model::{JointFrame, MateRef};
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Revolute,
        );
        j.mate = stack_mate(&doc, a, b);
        j.position = "90".to_string();
        // The mate lands the moving face's middle at (42, 2, 10); the frame's origin stays
        // there, so only the axis changes.
        j.frame = JointFrame {
            origin: None,
            primary: Some(MateRef::Axis(crate::construction::GlobalAxis::X)),
            secondary: None,
        };
        let ji = doc.joints.insert(j);
        let pose = body_joint_pose(&doc, b).unwrap();
        // The mate still lands the moving face's middle at (42, 2, 10), and that is still the
        // turn's centre — choosing an axis says which way the part moves, not where the mate
        // put it. The un-posed (44, 4, 0) lifts to (44, 4, 10), which is (2, 2, 0) from the
        // centre; a quarter turn about +X takes (y, z) = (2, 0) to (0, 2), landing it at
        // (44, 2, 12). About the mating normal it would have stayed at z = 10.
        let moved = pose.transform_point3(Vec3::new(44.0, 4.0, 0.0));
        assert!(
            (moved - Vec3::new(44.0, 2.0, 12.0)).length() < 1e-3,
            "turned about +X, not the mating normal: {moved}"
        );
        // And the frame the rest of the tool reads agrees.
        let (_, x, _, _) = posed_joint_frame(&doc, ji).unwrap();
        assert!((x - Vec3::X).length() < 1e-3, "{x:?}");

        // Clearing it hands the axis back to the mate.
        doc.joints[ji].frame = JointFrame::default();
        let (_, x, _, _) = posed_joint_frame(&doc, ji).unwrap();
        assert!((x - Vec3::Z).length() < 1e-3, "back to the mating normal: {x:?}");
    }

    /// #893: a screw couples travel to turn by its lead — a full turn advances one lead
    /// along the mating normal.
    #[test]
    fn screw_couples_turn_to_travel() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Screw { lead: "2".to_string() },
        );
        j.mate = stack_mate(&doc, a, b);
        j.position = "360".to_string();
        doc.joints.insert(j);
        let pose = body_joint_pose(&doc, b).unwrap();
        let moved = pose.transform_point3(Vec3::new(42.0, 2.0, 0.0));
        assert!(
            (moved - Vec3::new(42.0, 2.0, 12.0)).length() < 1e-3,
            "advanced to {moved}"
        );
    }

    /// #896: expression limits clamp the slide — a position past the max stops there.
    #[test]
    fn slider_clamps_to_expression_limits() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(vec![JointRef::Body(a), JointRef::Body(b)], JointKind::Slider);
        j.mate = stack_mate(&doc, a, b);
        j.mate.line_up.push(along_x_row(a, b, Vec3::new(40.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 10.0)));
        j.position = "7".to_string();
        j.limits.slide_max = "4".to_string();
        j.limits.slide_min = "-1".to_string();
        doc.joints.insert(j);
        let x = |doc: &Document| {
            resolve_joint_poses(doc)
                .member_pose(JointRef::Body(b))
                .unwrap()
                .transform_point3(Vec3::new(40.0, 0.0, 0.0))
                .x
        };
        assert!((x(&doc) - 44.0).abs() < 1e-3, "clamped to the max");
        doc.joints.values_mut().next().unwrap().position = "-9".to_string();
        assert!((x(&doc) - 39.0).abs() < 1e-3, "clamped to the min");
    }

    /// #896: turn limits clamp a revolute — 110° one way and not at all the other.
    #[test]
    fn revolute_clamps_to_turn_limits() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Revolute,
        );
        j.mate = stack_mate(&doc, a, b);
        j.position = "90".to_string();
        j.limits.turn_min = "0".to_string();
        j.limits.turn_max = "45".to_string();
        doc.joints.insert(j);
        let arm = |doc: &Document| {
            resolve_joint_poses(doc)
                .member_pose(JointRef::Body(b))
                .unwrap()
                .transform_point3(Vec3::new(44.0, 2.0, 0.0))
                - Vec3::new(42.0, 2.0, 10.0)
        };
        let a45 = 45f32.to_radians();
        let expected = Vec3::new(2.0 * a45.cos(), 2.0 * a45.sin(), 0.0);
        assert!((arm(&doc) - expected).length() < 1e-3, "clamped to 45°");
        doc.joints.values_mut().next().unwrap().position = "-30".to_string();
        assert!(
            (arm(&doc) - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-3,
            "no travel the other way"
        );
    }

    /// #896: a slide stop picked as geometry — the travel ends where the joint's axis meets
    /// the target's plane, recomputed as the model changes.
    #[test]
    fn slide_stop_resolves_against_a_plane() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        // Park the XY datum plane 18 mm up: the part lands at z = 10 and its slide along the
        // mating normal must stop 8 mm later.
        doc.construction_planes[pkey(0)].origin = Vec3::new(0.0, 0.0, 18.0);
        doc.construction_planes[pkey(0)].normal = Vec3::Z;
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Cylindrical,
        );
        j.mate = stack_mate(&doc, a, b);
        j.position = "20".to_string();
        j.limits.slide_max_target = Some(crate::model::ExtrudeTarget::Plane(pkey(0)));
        doc.joints.insert(j);
        let limits = resolve_limits(&doc, &doc.joints.values().nth(0).unwrap());
        assert!(
            limits.slide_max.is_some_and(|m| (m - 8.0).abs() < 1e-3),
            "stop resolves to 8 mm, got {limits:?}"
        );
        let moved = body_joint_pose(&doc, b)
            .unwrap()
            .transform_point3(Vec3::new(42.0, 2.0, 0.0));
        assert!((moved.z - 18.0).abs() < 1e-3, "clamped at the plane, got {moved}");
    }

    /// #896: a screw's slide limit bounds the coupled travel through its lead.
    #[test]
    fn screw_slide_limit_bounds_the_turn() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Screw { lead: "2".to_string() },
        );
        j.mate = stack_mate(&doc, a, b);
        j.position = "720".to_string();
        j.limits.slide_max = "1".to_string();
        doc.joints.insert(j);
        let moved = body_joint_pose(&doc, b)
            .unwrap()
            .transform_point3(Vec3::new(42.0, 2.0, 0.0));
        assert!(
            (moved.z - 11.0).abs() < 1e-3,
            "travel bounded to the slide max, got {moved}"
        );
    }

    /// #897: a Select-tool drag resolves to the joint it should move — the driver with
    /// freedom, walking up through rigid ties — and grounded/free parts read as such.
    #[test]
    fn body_drag_resolves_through_the_chain() {
        let mut doc = Document::default();
        let (ground, b) = two_cubes(&mut doc);
        let c = cube_body(&mut doc, Vec3::new(60.0, 0.0, 0.0), Vec3::splat(4.0));
        let free = cube_body(&mut doc, Vec3::new(90.0, 0.0, 0.0), Vec3::splat(4.0));
        let mut hinge = joint(
            vec![JointRef::Body(ground), JointRef::Body(b)],
            JointKind::Revolute,
        );
        hinge.mate = stack_mate(&doc, ground, b);
        doc.joints.insert(hinge);
        doc.joints.insert(joint(
            vec![JointRef::Body(b), JointRef::Body(c)],
            JointKind::Rigid,
        ));
        assert_eq!(body_drag_joint(&doc, b), BodyDragTarget::Joint(jkey(0)), "driven directly");
        assert_eq!(
            body_drag_joint(&doc, c),
            BodyDragTarget::Joint(jkey(0)),
            "a rigid tie walks up to the hinge"
        );
        assert_eq!(
            body_drag_joint(&doc, ground),
            BodyDragTarget::Grounded,
            "the held side refuses"
        );
        assert_eq!(body_drag_joint(&doc, free), BodyDragTarget::Free, "unjointed is free");
        // The posed drag frame sits at the mate's landing point, aimed along its normal.
        let (origin, axis, _, _) = posed_joint_frame(&doc, jkey(0)).expect("a frame to drag on");
        assert!((origin - Vec3::new(42.0, 2.0, 10.0)).length() < 1e-3, "origin {origin}");
        assert!((axis - Vec3::Z).length() < 1e-3, "axis {axis}");
    }

    /// #897: the drag reads its start values from the position expressions, in the units
    /// the pane shows (mm / degrees).
    #[test]
    fn drag_start_values_evaluate_per_kind() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Cylindrical,
        );
        j.position = "7".to_string();
        j.position2 = "45".to_string();
        let (slide, turn, _) = evaluated_positions(&doc, &j);
        assert!((slide - 7.0).abs() < 1e-4);
        assert!((turn - 45.0).abs() < 1e-4);
    }

    /// #895: the preview sweep glides between the limits where they're set — the ends of
    /// the period land exactly on them — and a limitless revolute sweeps the full turn.
    #[test]
    fn sweep_glides_between_the_limits() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let mut j = joint(vec![JointRef::Body(a), JointRef::Body(b)], JointKind::Slider);
        j.limits.slide_min = "-1".to_string();
        j.limits.slide_max = "4".to_string();
        let (lo, _, _) = sweep_positions(&doc, &j, 0.0).unwrap();
        assert!((lo - -1.0).abs() < 1e-3, "starts at the min, got {lo}");
        let (hi, _, _) = sweep_positions(&doc, &j, JOINT_SWEEP_PERIOD_SECS / 2.0).unwrap();
        assert!((hi - 4.0).abs() < 1e-3, "reaches the max half way, got {hi}");
        j.kind = JointKind::Revolute;
        j.limits = JointLimits::default();
        let (turn, _, _) = sweep_positions(&doc, &j, JOINT_SWEEP_PERIOD_SECS / 2.0).unwrap();
        assert!((turn - 360.0).abs() < 1e-3, "a free revolute sweeps the full turn, got {turn}");
        j.kind = JointKind::Rigid;
        assert!(sweep_positions(&doc, &j, 1.0).is_none(), "rigid has nothing to sweep");
    }

    /// #893/#1021: a chain A→B→C composes — C's mate resolves against B's **posed**
    /// geometry, so it lines up where B actually ended up rather than where it was drawn.
    #[test]
    fn chained_joints_compose_in_order() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let c = cube_body(&mut doc, Vec3::new(60.0, 0.0, 0.0), Vec3::splat(4.0));
        let mut ab = joint(vec![JointRef::Body(a), JointRef::Body(b)], JointKind::Slider);
        ab.mate = stack_mate(&doc, a, b);
        ab.mate
            .line_up
            .push(along_x_row(a, b, Vec3::new(40.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 10.0)));
        ab.position = "2".to_string();
        let q = crate::hierarchy::quantize_body_point;
        let mut bc = joint(vec![JointRef::Body(b), JointRef::Body(c)], JointKind::Slider);
        bc.mate = JointMate {
            moving_face: Some(face_ref(&doc, c, Vec3::new(62.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(&doc, b, Vec3::new(42.0, 2.0, 4.0))),
            line_up: vec![MateLineUp {
                moving: Some(MateRef::Edge {
                    body: c,
                    a: q(Vec3::new(60.0, 0.0, 0.0)),
                    b: q(Vec3::new(64.0, 0.0, 0.0)),
                }),
                fixed: Some(MateRef::Edge {
                    body: b,
                    a: q(Vec3::new(40.0, 0.0, 4.0)),
                    b: q(Vec3::new(44.0, 0.0, 4.0)),
                }),
            }],
            ..Default::default()
        };
        bc.position = "3".to_string();
        // Deliberately out of dependency order: the B→C joint is authored first.
        doc.joints.insert(bc);
        doc.joints.insert(ab);
        let res = resolve_joint_poses(&doc);
        assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
        // B lands on A's top and slides 2 along +X.
        let pb = res.member_pose(JointRef::Body(b)).unwrap();
        let b_at = pb.transform_point3(Vec3::new(40.0, 0.0, 0.0));
        assert!((b_at - Vec3::new(42.0, 0.0, 10.0)).length() < 1e-3, "B at {b_at}");
        // C lands on B's *posed* top (z = 14) and slides 3 more.
        let pc = res.member_pose(JointRef::Body(c)).unwrap();
        let c_at = pc.transform_point3(Vec3::new(60.0, 0.0, 0.0));
        assert!((c_at - Vec3::new(63.0, 0.0, 14.0)).length() < 1e-3, "C at {c_at}");
    }

    /// #893: a loop of joints resolves nothing and reports every joint in the loop.
    #[test]
    fn joint_cycle_is_reported_not_resolved() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        doc.joints.insert(joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Rigid,
        ));
        doc.joints.insert(joint(
            vec![JointRef::Body(b), JointRef::Body(a)],
            JointKind::Rigid,
        ));
        let res = resolve_joint_poses(&doc);
        assert_eq!(res.errors.len(), 2, "both loop joints report: {:?}", res.errors);
        let health = crate::document_health::recompute_document_health(&doc);
        assert!(health
            .element_reasons
            .get(&crate::hierarchy::SceneElement::Joint(jkey(0)))
            .or_else(|| health
                .element_reasons
                .get(&crate::hierarchy::SceneElement::Joint(jkey(1))))
            .is_some());
    }

    /// #893: a part two joints both drive keeps the first and reports the second.
    #[test]
    fn double_driven_part_reports_the_second_joint() {
        let mut doc = Document::default();
        let (a, b) = two_cubes(&mut doc);
        let c = cube_body(&mut doc, Vec3::new(60.0, 0.0, 0.0), Vec3::splat(4.0));
        doc.joints.insert(joint(
            vec![JointRef::Body(a), JointRef::Body(c)],
            JointKind::Rigid,
        ));
        let second = doc.joints.insert(joint(
            vec![JointRef::Body(b), JointRef::Body(c)],
            JointKind::Rigid,
        ));
        let res = resolve_joint_poses(&doc);
        assert_eq!(res.errors.len(), 1);
        assert_eq!(res.errors[0].0, second, "the later joint is the one refused");
    }

    /// #893: a rigid group's driven members ride the base's pose — the whole group swings
    /// when the base is itself driven through a revolute.
    #[test]
    fn rigid_group_rides_its_base_through_a_chain() {
        let mut doc = Document::default();
        let (ground, b) = two_cubes(&mut doc);
        let c = cube_body(&mut doc, Vec3::new(60.0, 0.0, 0.0), Vec3::splat(4.0));
        doc.joints.insert(joint(
            vec![JointRef::Body(b), JointRef::Body(c)],
            JointKind::Rigid,
        ));
        let mut hinge = joint(
            vec![JointRef::Body(ground), JointRef::Body(b)],
            JointKind::Revolute,
        );
        hinge.mate = stack_mate(&doc, ground, b);
        hinge.position = "90".to_string();
        doc.joints.insert(hinge);
        let res = resolve_joint_poses(&doc);
        assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
        let pb = res.member_pose(JointRef::Body(b)).unwrap();
        let pc = res.member_pose(JointRef::Body(c)).unwrap();
        assert!(
            pb.abs_diff_eq(pc, 1e-5),
            "the rigid tie carries the hinge's pose whole"
        );
        // C sat 18 mm along +X of the hinge's landing point; the 90° swing puts it 18 mm
        // along +Y of it.
        let c_at = pc.transform_point3(Vec3::new(60.0, 2.0, 0.0));
        assert!(
            (c_at - Vec3::new(42.0, 20.0, 10.0)).length() < 1e-3,
            "C at {c_at}"
        );
    }
}
