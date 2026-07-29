//! Joint kinematics (#893): turn each joint's kind + frames + position into a rigid
//! transform and apply it to the driven side **in place** — no output bodies, the same
//! shape as a Move's plane/image/instance targets.
//!
//! Joints form a tree rooted at whatever is grounded (a member no joint drives). Poses
//! resolve in dependency order so a chain (A→B→C) composes; a joint that would close a
//! loop, or drive a part another joint already drives, is skipped and reported through
//! document health.

use crate::model::{Document, Joint, JointFrame, JointKind, JointRef};
use glam::{Mat3, Mat4, Vec3};
use std::collections::HashMap;

/// The outcome of resolving every live joint: a world pose per driven member, plus the
/// joints that couldn't apply and why (cycles, double-driven parts).
#[derive(Default, Debug, Clone)]
pub struct JointResolution {
    poses: HashMap<JointRef, Mat4>,
    /// Joint index → the reason it couldn't apply.
    pub errors: Vec<(usize, String)>,
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

/// Resolve a picked frame against the live (un-jointed) meshes. `None` when the origin is
/// unset or no longer resolves — the joint then mates as identity, which is exactly the
/// join-in-place behaviour (#891): parts that already touch stay put.
fn resolve_frame(doc: &Document, frame: &JointFrame) -> Option<Frame> {
    let origin = crate::extrude::move_point_world(doc, frame.origin.as_ref()?)?;
    let x = frame
        .axis
        .as_ref()
        .and_then(|p| crate::extrude::move_point_world(doc, p))
        .map(|p| (p - origin).normalize_or_zero())
        .filter(|v| v.length_squared() > 0.5)
        .unwrap_or(Vec3::Z);
    let y = frame
        .orient
        .as_ref()
        .and_then(|p| crate::extrude::move_point_world(doc, p))
        .map(|p| {
            let v = p - origin;
            (v - x * v.dot(x)).normalize_or_zero()
        })
        .filter(|v| v.length_squared() > 0.5)
        .unwrap_or_else(|| x.any_orthonormal_vector());
    let z = x.cross(y).normalize_or_zero();
    Some(Frame { origin, x, y, z })
}

/// The motion the joint's position expressions ask for, in the mating frame's own
/// coordinates (`x` = primary axis). Empty expressions read as zero, so a fresh joint sits
/// at its captured pose. `None` when an expression doesn't evaluate.
fn motion(doc: &Document, joint: &Joint) -> Option<Mat4> {
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
    let rot_x = |a: f32| Mat4::from_mat3(Mat3::from_rotation_x(a));
    let rot_y = |a: f32| Mat4::from_mat3(Mat3::from_rotation_y(a));
    let rot_z = |a: f32| Mat4::from_mat3(Mat3::from_rotation_z(a));
    Some(match &joint.kind {
        JointKind::Rigid => Mat4::IDENTITY,
        JointKind::Slider => Mat4::from_translation(Vec3::X * len(&joint.position)?),
        JointKind::Revolute => rot_x(ang(&joint.position)?),
        JointKind::Cylindrical => {
            Mat4::from_translation(Vec3::X * len(&joint.position)?)
                * rot_x(ang(&joint.position2)?)
        }
        // Slide across the plane perpendicular to the primary axis, spin about it.
        JointKind::Planar => {
            Mat4::from_translation(
                Vec3::Y * len(&joint.position)? + Vec3::Z * len(&joint.position2)?,
            ) * rot_x(ang(&joint.position3)?)
        }
        JointKind::Ball => {
            rot_x(ang(&joint.position)?)
                * rot_y(ang(&joint.position2)?)
                * rot_z(ang(&joint.position3)?)
        }
        // Slide along the primary axis while turning about the secondary one.
        JointKind::PinSlot => {
            Mat4::from_translation(Vec3::X * len(&joint.position)?)
                * rot_y(ang(&joint.position2)?)
        }
        // The turn drives the travel: one full turn advances by the lead.
        JointKind::Screw { lead } => {
            let a = ang(&joint.position)?;
            let travel = a / std::f32::consts::TAU * len(lead)?;
            Mat4::from_translation(Vec3::X * travel) * rot_x(a)
        }
    })
}

/// The transform a joint imposes on its driven side, given the base side's own pose:
/// carry the driven part's mating frame onto the base's (posed) frame, then move it
/// through the joint's freedoms. Falls back to the base pose alone when either frame is
/// unset/unresolvable — the identity mate that keeps already-placed parts in place.
fn joint_transform(doc: &Document, joint: &Joint, base_pose: Mat4, base_is_first: bool) -> Mat4 {
    let mate = match (
        resolve_frame(doc, &joint.frame_a),
        resolve_frame(doc, &joint.frame_b),
        motion(doc, joint),
    ) {
        (Some(fa), Some(fb), Some(m)) => {
            if base_is_first {
                frame_mat(&fa) * m * frame_mat(&fb).inverse()
            } else {
                // The base is the second member: drive the first through the inverse.
                frame_mat(&fb) * m.inverse() * frame_mat(&fa).inverse()
            }
        }
        _ => Mat4::IDENTITY,
    };
    base_pose * mate
}

/// Resolve every live joint's pose, in dependency order (#893). A joint is ready once the
/// joint driving its base (if any) has resolved; whatever never becomes ready closes a
/// loop and is reported instead of applied.
pub fn resolve_joint_poses(doc: &Document) -> JointResolution {
    let mut out = JointResolution::default();
    let live: Vec<(usize, &Joint)> = doc
        .joints
        .iter()
        .enumerate()
        .filter(|(_, j)| !j.deleted && j.members.len() >= 2)
        .collect();
    // Who drives each member: a part can only be driven by one joint — a tree, not a web.
    let mut driver_of: HashMap<JointRef, usize> = HashMap::new();
    let mut skipped: Vec<usize> = Vec::new();
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
    let mut done: HashMap<usize, bool> = HashMap::new();
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
            if motion(doc, joint).is_none() {
                out.errors.push((
                    ji,
                    "A joint position expression cannot be evaluated".to_string(),
                ));
            }
            let base_is_first = joint.base == 0 || joint.base >= joint.members.len();
            let pose = joint_transform(doc, joint, base_pose, base_is_first);
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
    if doc.joints.iter().all(|j| j.deleted) {
        return std::rc::Rc::new(JointResolution::default());
    }
    let fingerprint = crate::extrude::document_mesh_fingerprint(doc);
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
pub fn body_joint_pose(doc: &Document, body_index: usize) -> Option<Mat4> {
    if doc.joints.iter().all(|j| j.deleted) {
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
pub fn member_bodies(doc: &Document, member: JointRef) -> Vec<usize> {
    match member {
        JointRef::Body(bi) => doc
            .bodies
            .get(bi)
            .filter(|b| !b.deleted)
            .map(|_| vec![bi])
            .unwrap_or_default(),
        JointRef::UnitInstance(ui) => doc
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                !b.deleted
                    && matches!(
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
            .enumerate()
            .filter(|(bi, b)| {
                !b.deleted
                    && crate::hierarchy::owning_component(
                        doc,
                        &crate::hierarchy::SceneElement::Body(*bi),
                    )
                    .is_some_and(|owner| doc.component_chain(owner).contains(&ci))
            })
            .map(|(i, _)| i)
            .collect(),
    }
}

/// The pose an in-progress joint would impose on its driven side (#894): the committed
/// assembly's base pose composed with the probe's mate — what the tool's ghost shows.
/// `None` when it works out to identity (nothing to ghost).
pub fn preview_pose(doc: &Document, joint: &Joint) -> Option<Mat4> {
    let base = joint.base_member()?;
    let base_pose = joint_resolution(doc)
        .member_pose(base)
        .unwrap_or(Mat4::IDENTITY);
    let base_is_first = joint.base == 0 || joint.base >= joint.members.len();
    let pose = joint_transform(doc, joint, base_pose, base_is_first);
    if pose.abs_diff_eq(Mat4::IDENTITY, 1e-5) {
        None
    } else {
        Some(pose)
    }
}

/// Apply a body's joint pose to its solid mesh, if it carries one.
pub fn posed_mesh(
    doc: &Document,
    body_index: usize,
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
    use super::*;
    use crate::model::{Body, BodySource, ImportedMesh, Joint, JointLimits, MovePointRef};

    /// A 1 mm right tetrahedron at `origin` — enough mesh for vertex keys to resolve.
    fn tetra(origin: Vec3) -> Vec<[Vec3; 3]> {
        let o = origin;
        let x = origin + Vec3::X;
        let y = origin + Vec3::Y;
        let z = origin + Vec3::Z;
        vec![[o, x, y], [o, x, z], [o, y, z], [x, y, z]]
    }

    fn mesh_body(doc: &mut Document, origin: Vec3) -> usize {
        doc.imported_meshes.push(ImportedMesh {
            triangles: tetra(origin),
            source_name: format!("part{}", doc.imported_meshes.len()),
        });
        doc.bodies.push(Body {
            source: BodySource::Imported(doc.imported_meshes.len() - 1),
            name: None,
            material: None,
            deleted: false,
            shadow: false,
        });
        doc.bodies.len() - 1
    }

    fn joint(members: Vec<JointRef>, kind: JointKind) -> Joint {
        Joint {
            members,
            base: 0,
            kind,
            frame_a: JointFrame::default(),
            frame_b: JointFrame::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: JointLimits::default(),
            name: None,
            deleted: false,
        }
    }

    fn vert(body: usize, p: Vec3) -> Option<MovePointRef> {
        Some(MovePointRef::Vertex { body, p: crate::hierarchy::quantize_body_point(p) })
    }

    /// #893: a joint with no frames set mates as identity — joining parts already in
    /// place moves nothing.
    #[test]
    fn frameless_joint_is_a_no_op() {
        let mut doc = Document::default();
        let a = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::new(5.0, 0.0, 0.0));
        doc.joints.push(joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Revolute,
        ));
        let before = crate::extrude::body_solid_mesh_uncached_pub(&doc, b).unwrap();
        let after = posed_mesh(&doc, b, before.clone());
        assert_eq!(before.triangles, after.triangles);
        assert!(resolve_joint_poses(&doc).errors.is_empty());
    }

    /// #893: a slider carries the driven part along the frame's primary axis by its
    /// position expression, parametrically.
    #[test]
    fn slider_translates_along_its_axis() {
        let mut doc = Document::default();
        let a = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::new(5.0, 0.0, 0.0));
        let mut j = joint(vec![JointRef::Body(a), JointRef::Body(b)], JointKind::Slider);
        // Frame on A: origin at its corner, axis toward +Y. Frame on B: its own corner,
        // same +Y axis (already-in-place mate — the frames coincide in orientation).
        j.frame_a = JointFrame {
            origin: vert(a, Vec3::ZERO),
            axis: vert(a, Vec3::Y),
            orient: None,
        };
        j.frame_b = JointFrame {
            origin: vert(b, Vec3::new(5.0, 0.0, 0.0)),
            axis: vert(b, Vec3::new(5.0, 1.0, 0.0)),
            orient: None,
        };
        j.position = "7".to_string();
        doc.joints.push(j);
        let pose = body_joint_pose(&doc, b).expect("driven body carries a pose");
        // B's frame origin lands on A's origin, then slides 7 mm along +Y.
        let landed = pose.transform_point3(Vec3::new(5.0, 0.0, 0.0));
        assert!(
            (landed - Vec3::new(0.0, 7.0, 0.0)).length() < 1e-4,
            "landed at {landed}"
        );
        assert!(body_joint_pose(&doc, a).is_none(), "the base is held");
    }

    /// #893: a revolute turns the driven part about the frame's axis; 90° about +Z sends
    /// +X to +Y.
    #[test]
    fn revolute_turns_about_its_axis() {
        let mut doc = Document::default();
        let a = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::ZERO);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Revolute,
        );
        j.frame_a = JointFrame {
            origin: vert(a, Vec3::ZERO),
            axis: vert(a, Vec3::Z),
            orient: vert(a, Vec3::X),
        };
        j.frame_b = JointFrame {
            origin: vert(b, Vec3::ZERO),
            axis: vert(b, Vec3::Z),
            orient: vert(b, Vec3::X),
        };
        j.position = "90".to_string();
        doc.joints.push(j);
        let pose = body_joint_pose(&doc, b).unwrap();
        let moved = pose.transform_point3(Vec3::X);
        assert!((moved - Vec3::Y).length() < 1e-4, "+X turned to {moved}");
    }

    /// #893: a screw couples travel to turn by its lead — a full turn advances one lead.
    #[test]
    fn screw_couples_turn_to_travel() {
        let mut doc = Document::default();
        let a = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::ZERO);
        let mut j = joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Screw { lead: "2".to_string() },
        );
        j.frame_a = JointFrame {
            origin: vert(a, Vec3::ZERO),
            axis: vert(a, Vec3::Z),
            orient: vert(a, Vec3::X),
        };
        j.frame_b = j.frame_a.clone();
        j.frame_b.origin = vert(b, Vec3::ZERO);
        j.frame_b.axis = vert(b, Vec3::Z);
        j.frame_b.orient = vert(b, Vec3::X);
        j.position = "360".to_string();
        doc.joints.push(j);
        let pose = body_joint_pose(&doc, b).unwrap();
        let moved = pose.transform_point3(Vec3::ZERO);
        assert!(
            (moved - Vec3::new(0.0, 0.0, 2.0)).length() < 1e-4,
            "advanced to {moved}"
        );
    }

    /// #893: a chain A→B→C composes — C carries B's pose on top of its own slide.
    #[test]
    fn chained_joints_compose_in_order() {
        let mut doc = Document::default();
        let a = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::new(5.0, 0.0, 0.0));
        let c = mesh_body(&mut doc, Vec3::new(10.0, 0.0, 0.0));
        let slide = |from: usize, to: usize, from_o: Vec3, to_o: Vec3, amount: &str| {
            let mut j = joint(
                vec![JointRef::Body(from), JointRef::Body(to)],
                JointKind::Slider,
            );
            j.frame_a = JointFrame {
                origin: vert(from, from_o),
                axis: vert(from, from_o + Vec3::Y),
                orient: None,
            };
            j.frame_b = JointFrame {
                origin: vert(to, to_o),
                axis: vert(to, to_o + Vec3::Y),
                orient: None,
            };
            j.position = amount.to_string();
            j
        };
        // Deliberately out of dependency order: the B→C joint is authored first.
        doc.joints.push(slide(b, c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), "3"));
        doc.joints.push(slide(a, b, Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0), "2"));
        let res = resolve_joint_poses(&doc);
        assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
        // B: lands on A's origin, slides 2 along Y.
        let pb = res.member_pose(JointRef::Body(b)).unwrap();
        let b_at = pb.transform_point3(Vec3::new(5.0, 0.0, 0.0));
        assert!((b_at - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-4, "B at {b_at}");
        // C: lands on B's *posed* frame (0,2,0), slides 3 more along Y.
        let pc = res.member_pose(JointRef::Body(c)).unwrap();
        let c_at = pc.transform_point3(Vec3::new(10.0, 0.0, 0.0));
        assert!((c_at - Vec3::new(0.0, 5.0, 0.0)).length() < 1e-4, "C at {c_at}");
    }

    /// #893: a loop of joints resolves nothing and reports every joint in the loop.
    #[test]
    fn joint_cycle_is_reported_not_resolved() {
        let mut doc = Document::default();
        let a = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::new(5.0, 0.0, 0.0));
        doc.joints.push(joint(
            vec![JointRef::Body(a), JointRef::Body(b)],
            JointKind::Rigid,
        ));
        doc.joints.push(joint(
            vec![JointRef::Body(b), JointRef::Body(a)],
            JointKind::Rigid,
        ));
        let res = resolve_joint_poses(&doc);
        assert_eq!(res.errors.len(), 2, "both loop joints report: {:?}", res.errors);
        // Health surfaces them as invalid elements with the reason.
        let health = crate::document_health::recompute_document_health(&doc);
        assert!(health
            .element_reasons
            .get(&crate::hierarchy::SceneElement::Joint(0))
            .or_else(|| health
                .element_reasons
                .get(&crate::hierarchy::SceneElement::Joint(1)))
            .is_some());
    }

    /// #893: a part two joints both drive keeps the first and reports the second.
    #[test]
    fn double_driven_part_reports_the_second_joint() {
        let mut doc = Document::default();
        let a = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::new(5.0, 0.0, 0.0));
        let c = mesh_body(&mut doc, Vec3::new(10.0, 0.0, 0.0));
        doc.joints.push(joint(
            vec![JointRef::Body(a), JointRef::Body(c)],
            JointKind::Rigid,
        ));
        doc.joints.push(joint(
            vec![JointRef::Body(b), JointRef::Body(c)],
            JointKind::Rigid,
        ));
        let res = resolve_joint_poses(&doc);
        assert_eq!(res.errors.len(), 1);
        assert_eq!(res.errors[0].0, 1, "the later joint is the one refused");
    }

    /// #893: a rigid group's driven members ride the base's pose — and the whole group
    /// swings when the base is itself driven through a revolute.
    #[test]
    fn rigid_group_rides_its_base_through_a_chain() {
        let mut doc = Document::default();
        let ground = mesh_body(&mut doc, Vec3::ZERO);
        let b = mesh_body(&mut doc, Vec3::new(5.0, 0.0, 0.0));
        let c = mesh_body(&mut doc, Vec3::new(10.0, 0.0, 0.0));
        // Tie B and C rigidly (in place), then hinge B off ground by 90° about +Z.
        doc.joints.push(joint(
            vec![JointRef::Body(b), JointRef::Body(c)],
            JointKind::Rigid,
        ));
        let mut hinge = joint(
            vec![JointRef::Body(ground), JointRef::Body(b)],
            JointKind::Revolute,
        );
        hinge.frame_a = JointFrame {
            origin: vert(ground, Vec3::ZERO),
            axis: vert(ground, Vec3::Z),
            orient: vert(ground, Vec3::X),
        };
        hinge.frame_b = JointFrame {
            origin: vert(b, Vec3::new(5.0, 0.0, 0.0)),
            axis: vert(b, Vec3::new(5.0, 0.0, 1.0)),
            orient: vert(b, Vec3::new(6.0, 0.0, 0.0)),
        };
        hinge.position = "90".to_string();
        doc.joints.push(hinge);
        let res = resolve_joint_poses(&doc);
        assert!(res.errors.is_empty(), "errors: {:?}", res.errors);
        let pc = res.member_pose(JointRef::Body(c)).unwrap();
        // C sat 5 mm along +X from B's frame origin; after the 90° swing it sits 5 mm
        // along +Y from A's origin.
        let c_at = pc.transform_point3(Vec3::new(10.0, 0.0, 0.0));
        assert!((c_at - Vec3::new(0.0, 5.0, 0.0)).length() < 1e-4, "C at {c_at}");
    }
}

