//! Sketch faces and parent/child dependencies between faces and sketch entities.

use crate::model::{
    Circle, ConstructionPlane, ConstructionPlaneParent, Document, FaceId, Line, PlaneAnchor,
    PlaneDefinition, SketchId,
};
use glam::Vec3;

/// Local (u, v) coordinate frame of a sketchable face in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SketchFrame {
    pub origin: Vec3,
    pub u_axis: Vec3,
    pub v_axis: Vec3,
    pub normal: Vec3,
}

/// Default definition for the datum XY construction plane.
pub fn default_xy_plane_definition() -> PlaneDefinition {
    PlaneDefinition {
        anchor: PlaneAnchor::Face {
            origin: Vec3::ZERO,
            normal: Vec3::Z,
            label: "Ground".to_string(),
        },
        offset_mm: 0.0,
        angle_deg: 0.0,
    }
}

/// Default XY ground construction plane for new documents.
pub fn default_xy_plane() -> ConstructionPlane {
    ConstructionPlane {
        origin: Vec3::ZERO,
        normal: Vec3::Z,
        u_axis: Vec3::X,
        v_axis: Vec3::Y,
        parent: ConstructionPlaneParent::Root,
        definition: default_xy_plane_definition(),
        repeat_instance: None,
        name: None,
        extent: crate::model::PlaneExtent::default(),
    }
}

/// How far the datum planes of a fresh document reach (mm), into their +u/+v quadrant.
pub const DATUM_PLANE_SIZE_MM: f32 = 100.0;

/// How far the datum planes stand clear of the origin (mm) — the same gap on every one, so
/// the three of them frame the origin instead of boxing it in (#838).
pub const DATUM_PLANE_GAP_MM: f32 = 5.0;

/// The three datum planes a new document opens with (#833): XY, XZ and YZ, each occupying
/// the positive quadrant of its own plane so the origin corner is shared and none of them
/// hides the others' geometry.
pub fn default_datum_planes() -> crate::arena::Arena<ConstructionPlane> {
    let extent = crate::model::PlaneExtent::quadrant(DATUM_PLANE_SIZE_MM, DATUM_PLANE_GAP_MM);
    let plane = |normal: Vec3, u: Vec3, v: Vec3, label: &str| ConstructionPlane {
        origin: Vec3::ZERO,
        normal,
        u_axis: u,
        v_axis: v,
        parent: ConstructionPlaneParent::Root,
        definition: PlaneDefinition {
            anchor: PlaneAnchor::Face {
                origin: Vec3::ZERO,
                normal,
                label: label.to_string(),
            },
            offset_mm: 0.0,
            angle_deg: 0.0,
        },
        repeat_instance: None,
        name: Some(label.to_string()),
        extent,
    };
    let mut planes = crate::arena::Arena::new();
    // XY goes in first and the "Ground" anchor label every existing document uses is its.
    planes.insert(ConstructionPlane {
        name: Some("XY".to_string()),
        extent,
        ..default_xy_plane()
    });
    planes.insert(plane(Vec3::Y, Vec3::X, Vec3::Z, "XZ"));
    planes.insert(plane(Vec3::X, Vec3::Y, Vec3::Z, "YZ"));
    planes
}

/// Resolve the world-space sketch frame for a face.
pub fn sketch_frame(doc: &Document, face: FaceId) -> Option<SketchFrame> {
    match face {
        FaceId::ConstructionPlane(i) => {
            let plane = doc.construction_planes.get(i)?;
            Some(SketchFrame {
                origin: plane.origin,
                u_axis: plane.u_axis,
                v_axis: plane.v_axis,
                normal: plane.normal,
            })
        }
        // A unit's flat face (#725): the inner face's frame in the instance's rebuilt
        // document, placed by its transform — `None` (→ unhealthy sketch) once the
        // instance is deleted or a re-sync removed the face.
        FaceId::UnitFace { instance, face } => {
            if doc.unit_instances.get(instance).is_none() {
                return None;
            }
            let eval = crate::units::evaluate_instance(doc, instance)?;
            let inner = sketch_frame(&eval.document, *face)?;
            let m = crate::units::instance_transform(doc, instance);
            Some(SketchFrame {
                origin: m.transform_point3(inner.origin),
                u_axis: m.transform_vector3(inner.u_axis).normalize_or_zero(),
                v_axis: m.transform_vector3(inner.v_axis).normalize_or_zero(),
                normal: m.transform_vector3(inner.normal).normalize_or_zero(),
            })
        }
        // A repeated body's face (#1116): the source face's frame placed by the instance
        // transform — same rule as a unit face.
        FaceId::RepeatedFace { face, op, instance } => {
            let rep = doc.repeat_ops.get(op)?;
            let m = crate::extrude::repeat_instance_transform(doc, rep, instance)?;
            let inner = sketch_frame(doc, *face)?;
            Some(SketchFrame {
                origin: m.transform_point3(inner.origin),
                u_axis: m.transform_vector3(inner.u_axis).normalize_or_zero(),
                v_axis: m.transform_vector3(inner.v_axis).normalize_or_zero(),
                normal: m.transform_vector3(inner.normal).normalize_or_zero(),
            })
        }
        FaceId::Circle(i) => {
            let circle = doc.circles.get(i)?;
            let face = doc.sketch_face(circle.sketch)?;
            let parent = sketch_frame(doc, face)?;
            let origin = local_to_world(&parent, circle.cx, circle.cy);
            Some(SketchFrame {
                origin,
                u_axis: parent.u_axis,
                v_axis: parent.v_axis,
                normal: parent.normal,
            })
        }
        FaceId::Polygon(ref lines) => {
            let first_line = doc.lines.get(*lines.first()?)?;
            let sketch = first_line.sketch;
            let face = doc.sketch_face(sketch)?;
            let parent = sketch_frame(doc, face)?;
            let (u, v) = *crate::polygon::loop_vertices_uv(doc, sketch, lines)?.first()?;
            let origin = local_to_world(&parent, u, v);
            Some(SketchFrame {
                origin,
                u_axis: parent.u_axis,
                v_axis: parent.v_axis,
                normal: parent.normal,
            })
        }
        FaceId::ExtrudeCap {
            extrusion,
            profile,
            top,
        } => {
            let ext = doc.extrusions.get(extrusion)?;
            if !ext.faces.contains(&profile) {
                return None;
            }
            let base = sketch_frame(doc, profile.face_id())?;
            // A top cap that meets a slanted target plane lies in that plane, so derive its
            // frame from the actual (slanted) cap polygon rather than a parallel offset.
            if top && crate::extrude::target_top_plane(doc, ext).is_some() {
                let poly = crate::extrude::cap_polygon_world(doc, extrusion, &profile, true)?;
                return frame_from_polygon(&poly, base.normal);
            }
            // Otherwise the cap shares the profile's in-plane axes, shifted along the extrusion
            // normal to its actual end. A **symmetric** extrusion spans [−d/2, +d/2] about the
            // sketch plane, so its caps sit half a distance to either side — not at 0 and d
            // (#504/#548). Using the real end offsets keeps snap-to-cap targets (extrude-to-face)
            // and sketching-on-cap aligned with the built geometry.
            let (start, end) =
                crate::extrude::extrusion_end_offsets(doc, ext, crate::extrude::effective_distance(doc, ext));
            let dist = if top { end } else { start };
            Some(SketchFrame {
                origin: base.origin + base.normal * dist,
                u_axis: base.u_axis,
                v_axis: base.v_axis,
                normal: base.normal,
            })
        }
        FaceId::ExtrudeSide {
            extrusion,
            profile,
            edge,
        } => {
            let quad = crate::extrude::side_quad_world(doc, extrusion, &profile, edge as usize)?;
            let (poly, plane_normal) = crate::extrude::face_profile_world(doc, &profile)?;
            let (a, b) = (quad[0], quad[1]);
            let u_axis = (b - a).normalize_or_zero();
            if u_axis.length_squared() < 1e-8 {
                return None;
            }
            // Outward wall normal, derived from the profile's winding: a loop winding CCW
            // about the sketch normal keeps its interior to the left of each edge, so
            // edge × normal points away from the solid (CW winding flips it). Unlike a
            // centroid heuristic this is exact for non-convex profiles, whose centroid can
            // sit on the wrong side of an inner edge — that made the frame left-handed as
            // seen from outside, mirroring sketch content on concave walls (#362).
            let mut normal = u_axis.cross(plane_normal).normalize_or_zero();
            if normal.length_squared() < 1e-8 {
                return None;
            }
            // Origin-independent polygon area vector (Σ pᵢ × pᵢ₊₁): along the sketch
            // normal for a CCW loop, opposite for CW.
            let mut area = Vec3::ZERO;
            for i in 0..poly.len() {
                area += poly[i].cross(poly[(i + 1) % poly.len()]);
            }
            if area.dot(plane_normal) < 0.0 {
                normal = -normal;
            }
            // (u, v, normal) right-handed: v = normal × u keeps u × v == normal.
            let v_axis = normal.cross(u_axis).normalize_or_zero();
            Some(SketchFrame {
                origin: a,
                u_axis,
                v_axis,
                normal,
            })
        }
        FaceId::RevolveCap {
            revolution,
            ref profile,
            end,
        } => {
            let (poly, outward) =
                crate::extrude::revolve_cap_polygon_world(doc, revolution, profile, end)?;
            frame_from_polygon(&poly, outward)
        }
        FaceId::RevolveSide {
            revolution,
            ref profile,
            edge,
        } => crate::extrude::revolve_side_geom(doc, revolution, profile, edge as usize)
            .map(|(_, frame, _)| frame),
        FaceId::PrimitiveFace { primitive, face } => {
            let shape = doc.primitives.get(primitive)?;
            let frame = crate::primitives::face_frame(doc, shape, face)?;
            Some(with_body_joint_pose(
                doc,
                crate::model::body_index_for_primitive(doc, primitive),
                frame,
            ))
        }
        // Live mesh face (#1173): frame from the coplanar triangle group, normal matching
        // the quantized key so a sketch on a shell's inner wall sits on that wall.
        FaceId::BodyMeshFace {
            body,
            centroid,
            normal,
        } => {
            let tris = crate::extrude::body_face_triangles(doc, body, centroid, normal)?;
            if tris.is_empty() {
                return None;
            }
            let mut n = (tris[0][1] - tris[0][0])
                .cross(tris[0][2] - tris[0][0])
                .normalize_or_zero();
            if n.length_squared() < 1e-8 {
                return None;
            }
            let key_n = crate::hierarchy::dequantize_body_point(normal).normalize_or_zero();
            if n.dot(key_n) < 0.0 {
                n = -n;
            }
            let origin = crate::extrude::face_group_center(&tris);
            let u_axis = crate::primitives::plane_u_axis(n);
            let v_axis = n.cross(u_axis).normalize_or_zero();
            Some(SketchFrame {
                origin,
                u_axis,
                v_axis,
                normal: n,
            })
        }
    }
}

/// Build a sketch frame from a planar world-space polygon: origin at the first vertex, U along
/// the first edge, and a normal flipped to agree with `reference_normal` (so a slanted cap keeps
/// the same facing as its base). Returns `None` for degenerate polygons.
fn frame_from_polygon(poly: &[Vec3], reference_normal: Vec3) -> Option<SketchFrame> {
    if poly.len() < 3 {
        return None;
    }
    let origin = poly[0];
    let mut normal = (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
    if normal.length_squared() < 1e-8 {
        return None;
    }
    if normal.dot(reference_normal) < 0.0 {
        normal = -normal;
    }
    // U along the first edge, made orthogonal to the (possibly flipped) normal.
    let mut u_axis = poly[1] - poly[0];
    u_axis = (u_axis - normal * u_axis.dot(normal)).normalize_or_zero();
    if u_axis.length_squared() < 1e-8 {
        return None;
    }
    // v = normal × u keeps (u, v, normal) right-handed with u × v == normal.
    let v_axis = normal.cross(u_axis).normalize_or_zero();
    Some(SketchFrame {
        origin,
        u_axis,
        v_axis,
        normal,
    })
}

/// Apply a body's joint pose to a modelling-space sketch frame (#1358): features on a
/// jointed part are picked and drawn where the part sits, then un-posed when fused back
/// onto the un-jointed host solid.
fn with_body_joint_pose(
    doc: &Document,
    body: Option<crate::model::BodyKey>,
    frame: SketchFrame,
) -> SketchFrame {
    let Some(body) = body else {
        return frame;
    };
    let Some(m) = crate::joints::body_joint_pose(doc, body) else {
        return frame;
    };
    SketchFrame {
        origin: m.transform_point3(frame.origin),
        u_axis: m.transform_vector3(frame.u_axis).normalize_or_zero(),
        v_axis: m.transform_vector3(frame.v_axis).normalize_or_zero(),
        normal: m.transform_vector3(frame.normal).normalize_or_zero(),
    }
}

/// Resolve the world-space frame for geometry in a sketch.
pub fn sketch_geometry_frame(doc: &Document, sketch: SketchId) -> Option<SketchFrame> {
    let face = doc.sketch_face(sketch)?;
    sketch_frame(doc, face)
}

pub fn world_to_local(frame: &SketchFrame, p: Vec3) -> (f32, f32) {
    let rel = p - frame.origin;
    (rel.dot(frame.u_axis), rel.dot(frame.v_axis))
}

pub fn local_to_world(frame: &SketchFrame, u: f32, v: f32) -> Vec3 {
    frame.origin + frame.u_axis * u + frame.v_axis * v
}

fn camera_up_from_look_at_hint(look_forward: Vec3, up_hint: Vec3) -> Vec3 {
    let mut right = look_forward.cross(up_hint);
    if right.length_squared() < 1e-8 {
        return up_hint.normalize_or_zero();
    }
    right = right.normalize();
    right.cross(look_forward).normalize_or_zero()
}

fn axis_screen_vec(axis: Vec3, look_forward: Vec3, up_hint: Vec3) -> glam::Vec2 {
    let right = look_forward.cross(up_hint).normalize_or_zero();
    if right.length_squared() < 1e-8 {
        return glam::Vec2::ZERO;
    }
    let up = right.cross(look_forward).normalize_or_zero();
    glam::Vec2::new(axis.dot(right), -axis.dot(up))
}

fn axes_match_sketch_convention(u_screen: glam::Vec2, v_screen: glam::Vec2) -> bool {
    let u_right = u_screen.x > 0.0 && u_screen.x.abs() >= u_screen.y.abs();
    let v_up = v_screen.y < 0.0 && v_screen.y.abs() >= v_screen.x.abs();
    u_right && v_up
}

fn sketch_view_up_score(
    u_screen_before: glam::Vec2,
    v_screen_before: glam::Vec2,
    u_screen_after: glam::Vec2,
    v_screen_after: glam::Vec2,
) -> f32 {
    // Minimal apparent roll (#577): how far the plane's u/v axes rotate on screen versus the
    // current view. Entering a sketch takes the **shortest** orientation change rather than snapping
    // to a fixed u-right/v-up convention (that forced a big spin on the ground plane). With the
    // sketch axes now drawn and selectable, orientation no longer has to encode which way is
    // "horizontal". A tiny nudge still favours the convention, but only to break near-ties — never
    // enough to override a real roll difference.
    let du = (u_screen_after - u_screen_before).length_squared();
    let dv = (v_screen_after - v_screen_before).length_squared();
    let mut score = du + dv;
    if !axes_match_sketch_convention(u_screen_after, v_screen_after) {
        score += 0.05;
    }
    score
}

/// Camera up hint that places the sketch plane's u/v axes on the screen axes with the
/// smallest roll change from the current view.
pub fn sketch_view_up(
    view_direction: Vec3,
    frame: &SketchFrame,
    current_look_forward: Vec3,
    current_up_hint: Vec3,
) -> Vec3 {
    // `view_direction` points from the face toward the eye; `look_at_rh` uses the opposite.
    let target_look = (-view_direction).normalize_or_zero();
    let current_look = current_look_forward.normalize_or_zero();
    let current_up_hint = current_up_hint.normalize_or_zero();
    let u = frame.u_axis.normalize_or_zero();
    let v = frame.v_axis.normalize_or_zero();
    if u.length_squared() < 1e-8 || v.length_squared() < 1e-8 {
        return Vec3::Z;
    }

    let u_screen_before = axis_screen_vec(u, current_look, current_up_hint);
    let v_screen_before = axis_screen_vec(v, current_look, current_up_hint);
    let mut best_hint = v;
    let mut best_score = f32::MAX;

    // For a near-vertical face (e.g. the side wall of a solid) there is a natural
    // "up": world +Z. Orient the sketch so the ground falls to the bottom of the
    // screen rather than rolling sideways to preserve the previous view. Faces that
    // are horizontal or only mildly tilted have little in-plane vertical component,
    // so they keep the roll-preservation behavior. A vertical wall's in-plane
    // vertical component is ~1; the 0.9 cutoff admits faces within ~25° of vertical.
    let plane_normal = (-target_look).normalize_or_zero();
    let world_up_in_plane = Vec3::Z - plane_normal * Vec3::Z.dot(plane_normal);
    let prefer_world_up = world_up_in_plane.length() > 0.9;

    for hint in [u, -u, v, -v] {
        let right = target_look.cross(hint).normalize_or_zero();
        if right.length_squared() < 1e-8 {
            continue;
        }

        let cam_up = camera_up_from_look_at_hint(target_look, hint);
        let u_h = u.dot(right).abs();
        let u_v = u.dot(cam_up).abs();
        let v_h = v.dot(right).abs();
        let v_v = v.dot(cam_up).abs();
        const AXIS_EPS: f32 = 0.05;
        let u_axis_aligned = (u_h > AXIS_EPS) ^ (u_v > AXIS_EPS);
        let v_axis_aligned = (v_h > AXIS_EPS) ^ (v_v > AXIS_EPS);
        if !u_axis_aligned || !v_axis_aligned || u_h + u_v < 0.9 || v_h + v_v < 0.9 {
            continue;
        }
        if (u_h > AXIS_EPS) == (v_h > AXIS_EPS) {
            continue;
        }

        let score = if prefer_world_up {
            // Smaller is better: pick the orientation whose screen-up points most
            // toward world +Z, keeping the ground at the bottom of the view.
            -cam_up.dot(Vec3::Z)
        } else {
            // Take the axis-aligned orientation that rotates the on-screen content the least
            // (#577), so entering a sketch is the shortest move rather than a forced spin.
            let u_screen_after = axis_screen_vec(u, target_look, hint);
            let v_screen_after = axis_screen_vec(v, target_look, hint);
            sketch_view_up_score(
                u_screen_before,
                v_screen_before,
                u_screen_after,
                v_screen_after,
            )
        };
        if score < best_score {
            best_score = score;
            best_hint = hint;
        }
    }

    if best_score < f32::MAX {
        return best_hint;
    }

    let mut up = v;
    let right = target_look.cross(up).normalize_or_zero();
    if right.dot(u) < 0.0 {
        up = -up;
    }
    up
}

pub fn line_world_endpoints(doc: &Document, line: &Line) -> Option<(Vec3, Vec3)> {
    let frame = sketch_geometry_frame(doc, line.sketch)?;
    Some((
        local_to_world(&frame, line.x0, line.y0),
        local_to_world(&frame, line.x1, line.y1),
    ))
}

/// World-space polyline approximation of a line, sampled with
/// [`crate::model::BEZIER_SEGMENTS`] segments for a curved line, or just its two endpoints
/// for a straight one.
pub fn line_world_polyline(doc: &Document, line: &Line) -> Option<Vec<Vec3>> {
    let frame = sketch_geometry_frame(doc, line.sketch)?;
    Some(
        line.sample_local(crate::model::BEZIER_SEGMENTS)
            .into_iter()
            .map(|(u, v)| local_to_world(&frame, u, v))
            .collect(),
    )
}


pub fn circle_world_center(doc: &Document, circle: &Circle) -> Option<Vec3> {
    let frame = sketch_geometry_frame(doc, circle.sketch)?;
    Some(local_to_world(&frame, circle.cx, circle.cy))
}

/// Rim-to-rim diameter segment through the circle center.
pub fn circle_world_diameter_endpoints(doc: &Document, circle: &Circle) -> Option<(Vec3, Vec3)> {
    let frame = sketch_geometry_frame(doc, circle.sketch)?;
    let du = circle.diameter_dim_angle.cos() * circle.r;
    let dv = circle.diameter_dim_angle.sin() * circle.r;
    Some((
        local_to_world(&frame, circle.cx - du, circle.cy - dv),
        local_to_world(&frame, circle.cx + du, circle.cy + dv),
    ))
}

/// Sampled world-space points around a circle perimeter (closed loop).
pub fn circle_world_perimeter(doc: &Document, circle: &Circle, segments: usize) -> Option<Vec<Vec3>> {
    let frame = sketch_geometry_frame(doc, circle.sketch)?;
    let segments = segments.max(8);
    let mut pts = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = i as f32 / segments as f32 * std::f32::consts::TAU;
        let u = circle.cx + circle.r * t.cos();
        let v = circle.cy + circle.r * t.sin();
        pts.push(local_to_world(&frame, u, v));
    }
    Some(pts)
}

/// Axis-aligned bounds in a face's local (u, v) coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SketchZoomBounds {
    pub center_u: f32,
    pub center_v: f32,
    pub half_u: f32,
    pub half_v: f32,
}

/// Camera framing parameters when entering sketch mode on a sketch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SketchCameraTarget {
    pub target: glam::Vec3,
    /// Outward face normal; the camera picks ±this to stay on the visible side.
    pub face_normal: glam::Vec3,
    pub zoom: Option<SketchZoomBounds>,
}

impl SketchZoomBounds {
    fn from_uv_rect(u0: f32, v0: f32, u1: f32, v1: f32) -> Self {
        let u_min = u0.min(u1);
        let u_max = u0.max(u1);
        let v_min = v0.min(v1);
        let v_max = v0.max(v1);
        let half_u = ((u_max - u_min) * 0.5).max(1.0);
        let half_v = ((v_max - v_min) * 0.5).max(1.0);
        Self {
            center_u: (u_min + u_max) * 0.5,
            center_v: (v_min + v_max) * 0.5,
            half_u,
            half_v,
        }
    }

    fn union(a: Self, b: Self) -> Self {
        let u_min = (a.center_u - a.half_u).min(b.center_u - b.half_u);
        let u_max = (a.center_u + a.half_u).max(b.center_u + b.half_u);
        let v_min = (a.center_v - a.half_v).min(b.center_v - b.half_v);
        let v_max = (a.center_v + a.half_v).max(b.center_v + b.half_v);
        Self::from_uv_rect(u_min, v_min, u_max, v_max)
    }

    pub fn world_corners(&self, frame: &SketchFrame) -> [Vec3; 4] {
        [
            local_to_world(
                frame,
                self.center_u - self.half_u,
                self.center_v - self.half_v,
            ),
            local_to_world(
                frame,
                self.center_u + self.half_u,
                self.center_v - self.half_v,
            ),
            local_to_world(
                frame,
                self.center_u + self.half_u,
                self.center_v + self.half_v,
            ),
            local_to_world(
                frame,
                self.center_u - self.half_u,
                self.center_v + self.half_v,
            ),
        ]
    }
}

fn extend_sketch_bounds(bounds: &mut Option<SketchZoomBounds>, u0: f32, v0: f32, u1: f32, v1: f32) {
    let next = SketchZoomBounds::from_uv_rect(u0, v0, u1, v1);
    *bounds = Some(match bounds.take() {
        Some(existing) => SketchZoomBounds::union(existing, next),
        None => next,
    });
}

/// Axis-aligned zoom bounds for all geometry in a sketch (lines and circles).
fn sketch_local_bounds(doc: &Document, sketch: SketchId) -> Option<SketchZoomBounds> {
    let mut bounds = None;
    for line in doc.lines.values() {
        if line.sketch == sketch {
            extend_sketch_bounds(&mut bounds, line.x0, line.y0, line.x1, line.y1);
        }
    }
    for circle in doc.circles.values() {
        if circle.sketch == sketch {
            extend_sketch_bounds(
                &mut bounds,
                circle.cx - circle.r,
                circle.cy - circle.r,
                circle.cx + circle.r,
                circle.cy + circle.r,
            );
        }
    }
    bounds
}

/// Resolve camera target, view direction, and optional zoom bounds for sketch mode.
pub fn sketch_camera_target(doc: &Document, sketch: SketchId) -> Option<SketchCameraTarget> {
    let face = doc.sketch_face(sketch)?;
    let frame = sketch_frame(doc, face.clone())?;
    let face_normal = frame.normal;

    match face {
        FaceId::ConstructionPlane(_) => {
            if let Some(zoom) = sketch_local_bounds(doc, sketch) {
                let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
                Some(SketchCameraTarget {
                    target,
                    face_normal,
                    zoom: Some(zoom),
                })
            } else {
                Some(SketchCameraTarget {
                    target: frame.origin,
                    face_normal,
                    zoom: None,
                })
            }
        }
        FaceId::Circle(i) => {
            let circle = doc.circles.get(i)?;
            let mut zoom = SketchZoomBounds::from_uv_rect(
                circle.cx - circle.r,
                circle.cy - circle.r,
                circle.cx + circle.r,
                circle.cy + circle.r,
            );
            if let Some(children) = sketch_local_bounds(doc, sketch) {
                zoom = SketchZoomBounds::union(zoom, children);
            }
            let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
            Some(SketchCameraTarget {
                target,
                face_normal,
                zoom: Some(zoom),
            })
        }
        FaceId::Polygon(ref lines) => {
            let vertices = crate::polygon::loop_vertices_uv(doc, sketch, lines)?;
            let mut zoom: Option<SketchZoomBounds> = None;
            for (u, v) in vertices {
                extend_sketch_bounds(&mut zoom, u, v, u, v);
            }
            if let Some(children) = sketch_local_bounds(doc, sketch) {
                zoom = Some(match zoom {
                    Some(z) => SketchZoomBounds::union(z, children),
                    None => children,
                });
            }
            let zoom = zoom?;
            let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
            Some(SketchCameraTarget {
                target,
                face_normal,
                zoom: Some(zoom),
            })
        }
        FaceId::ExtrudeCap {
            extrusion,
            profile,
            top,
        } => {
            let poly = crate::extrude::cap_polygon_world(doc, extrusion, &profile, top)?;
            let mut zoom: Option<SketchZoomBounds> = None;
            for p in &poly {
                let (u, v) = world_to_local(&frame, *p);
                extend_sketch_bounds(&mut zoom, u, v, u, v);
            }
            if let Some(children) = sketch_local_bounds(doc, sketch) {
                zoom = Some(match zoom {
                    Some(z) => SketchZoomBounds::union(z, children),
                    None => children,
                });
            }
            let zoom = zoom?;
            let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
            Some(SketchCameraTarget {
                target,
                face_normal,
                zoom: Some(zoom),
            })
        }
        FaceId::ExtrudeSide {
            extrusion,
            profile,
            edge,
        } => {
            let quad = crate::extrude::side_quad_world(doc, extrusion, &profile, edge as usize)?;
            let mut zoom: Option<SketchZoomBounds> = None;
            for p in &quad {
                let (u, v) = world_to_local(&frame, *p);
                extend_sketch_bounds(&mut zoom, u, v, u, v);
            }
            if let Some(children) = sketch_local_bounds(doc, sketch) {
                zoom = Some(match zoom {
                    Some(z) => SketchZoomBounds::union(z, children),
                    None => children,
                });
            }
            let zoom = zoom?;
            let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
            Some(SketchCameraTarget {
                target,
                face_normal,
                zoom: Some(zoom),
            })
        }
        // A unit's flat face (#725): frame the face's placed boundary polygon.
        FaceId::UnitFace { instance, ref face } => {
            let poly = crate::units::unit_face_world_polygon(doc, instance, face)?;
            let mut zoom: Option<SketchZoomBounds> = None;
            for p in &poly {
                let (u, v) = world_to_local(&frame, *p);
                extend_sketch_bounds(&mut zoom, u, v, u, v);
            }
            if let Some(children) = sketch_local_bounds(doc, sketch) {
                zoom = Some(match zoom {
                    Some(z) => SketchZoomBounds::union(z, children),
                    None => children,
                });
            }
            let zoom = zoom?;
            let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
            Some(SketchCameraTarget {
                target,
                face_normal,
                zoom: Some(zoom),
            })
        }
        // A repeated face (#1116): frame the placed boundary polygon, like a unit face.
        FaceId::RepeatedFace { .. } => {
            let poly = crate::extrude::face_boundary_loop_world(doc, &face)?;
            let mut zoom: Option<SketchZoomBounds> = None;
            for p in &poly {
                let (u, v) = world_to_local(&frame, *p);
                extend_sketch_bounds(&mut zoom, u, v, u, v);
            }
            if let Some(children) = sketch_local_bounds(doc, sketch) {
                zoom = Some(match zoom {
                    Some(z) => SketchZoomBounds::union(z, children),
                    None => children,
                });
            }
            let zoom = zoom?;
            let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
            Some(SketchCameraTarget {
                target,
                face_normal,
                zoom: Some(zoom),
            })
        }
        FaceId::RevolveCap { .. }
        | FaceId::RevolveSide { .. }
        | FaceId::PrimitiveFace { .. }
        | FaceId::BodyMeshFace { .. } => {
            let poly = match face {
                FaceId::RevolveCap {
                    revolution,
                    ref profile,
                    end,
                } => {
                    crate::extrude::revolve_cap_polygon_world(doc, revolution, profile, end)?.0
                }
                FaceId::RevolveSide {
                    revolution,
                    ref profile,
                    edge,
                } => {
                    crate::extrude::revolve_side_geom(doc, revolution, profile, edge as usize)?.0
                }
                FaceId::PrimitiveFace { primitive, face } => {
                    let shape = doc.primitives.get(primitive)?;
                    crate::primitives::face_polygon(doc, shape, face)?
                }
                FaceId::BodyMeshFace {
                    body,
                    centroid,
                    normal,
                } => {
                    let tris = crate::extrude::body_face_triangles(doc, body, centroid, normal)?;
                    tris.iter().flat_map(|t| t.iter().copied()).collect()
                }
                _ => unreachable!(),
            };
            let mut zoom: Option<SketchZoomBounds> = None;
            for p in &poly {
                let (u, v) = world_to_local(&frame, *p);
                extend_sketch_bounds(&mut zoom, u, v, u, v);
            }
            if let Some(children) = sketch_local_bounds(doc, sketch) {
                zoom = Some(match zoom {
                    Some(z) => SketchZoomBounds::union(z, children),
                    None => children,
                });
            }
            let zoom = zoom?;
            let target = local_to_world(&frame, zoom.center_u, zoom.center_v);
            Some(SketchCameraTarget {
                target,
                face_normal,
                zoom: Some(zoom),
            })
        }
    }
}

pub fn sketch_label(doc: &Document, sketch: SketchId) -> String {
    let face = doc
        .sketch_face(sketch)
        .map(|face| face_label(doc, face))
        .unwrap_or_else(|| "unknown face".to_string());
    format!("Sketch {} on {face}", sketch.index())
}

/// Every sketchable analytic face of a live body (#1156): extrusion caps/sides, revolve
/// flats, and primitive flats. Used by Shell open-face resolution from a mesh BodyFace.
pub fn analytic_faces_of_body(doc: &Document, body: crate::model::BodyKey) -> Vec<FaceId> {
    let Some(body_rec) = doc.bodies.get(body) else {
        return Vec::new();
    };
    let mut faces: Vec<FaceId> = Vec::new();
    for &ei in body_rec.source.extrusion_indices() {
        let Some(ext) = doc.extrusions.get(ei) else {
            continue;
        };
        for profile in &ext.faces {
            for top in [true, false] {
                faces.push(FaceId::ExtrudeCap {
                    extrusion: ei,
                    profile: profile.clone(),
                    top,
                });
            }
            for edge in 0..crate::extrude::side_face_count(profile) {
                faces.push(FaceId::ExtrudeSide {
                    extrusion: ei,
                    profile: profile.clone(),
                    edge: edge as u8,
                });
            }
        }
    }
    if let crate::model::BodySource::Primitive(pi) = body_rec.source {
        if let Some(shape) = doc.primitives.get(pi) {
            for face in crate::primitives::flat_faces(shape) {
                faces.push(FaceId::PrimitiveFace {
                    primitive: pi,
                    face,
                });
            }
        }
    }
    if let crate::model::BodySource::Revolve(ri) = body_rec.source {
        if let Some(rev) = doc.revolutions.get(ri) {
            for profile in &rev.faces {
                for end in [true, false] {
                    faces.push(FaceId::RevolveCap {
                        revolution: ri,
                        profile: profile.clone(),
                        end,
                    });
                }
                for edge in 0..crate::extrude::side_face_count(profile) {
                    faces.push(FaceId::RevolveSide {
                        revolution: ri,
                        profile: profile.clone(),
                        edge: edge as u8,
                    });
                }
            }
        }
    }
    // Solid with primitive base (#1104): include the base's flat faces.
    if let crate::model::BodySource::Solid {
        base: Some(pi), ..
    } = body_rec.source
    {
        if let Some(shape) = doc.primitives.get(pi) {
            for face in crate::primitives::flat_faces(shape) {
                faces.push(FaceId::PrimitiveFace {
                    primitive: pi,
                    face,
                });
            }
        }
    }
    faces
}

pub fn face_label(_doc: &Document, face: FaceId) -> String {
    match face {
        FaceId::ConstructionPlane(i) => format!("Construction plane {}", i.index()),
        FaceId::Circle(i) => format!("Circle face {}", i.index()),
        FaceId::Polygon(lines) => format!("Polygon face ({} lines)", lines.len()),
        FaceId::ExtrudeCap {
            extrusion, top, ..
        } => {
            let end = if top { "top" } else { "bottom" };
            format!("Extrusion {} {end} face", extrusion.index())
        }
        FaceId::ExtrudeSide {
            extrusion, edge, ..
        } => format!("Extrusion {} side face {edge}", extrusion.index()),
        FaceId::RevolveCap {
            revolution, end, ..
        } => {
            let side = if end { "end" } else { "start" };
            format!("Revolution {} {side} face", revolution.index())
        }
        FaceId::RevolveSide {
            revolution, edge, ..
        } => format!("Revolution {} side face {edge}", revolution.index()),
        FaceId::UnitFace { instance, .. } => {
            format!("Unit instance {} face", instance.index())
        }
        FaceId::RepeatedFace { instance, .. } => {
            format!("Repeated instance {instance} face")
        }
        FaceId::PrimitiveFace { primitive, face } => {
            let which = match face {
                crate::model::PrimitiveFace::CuboidBottom => "bottom",
                crate::model::PrimitiveFace::CuboidTop => "top",
                crate::model::PrimitiveFace::CuboidSide { edge } => {
                    return format!("Primitive {} side face {edge}", primitive.index());
                }
                crate::model::PrimitiveFace::CylinderBottom => "bottom cap",
                crate::model::PrimitiveFace::CylinderTop => "top cap",
            };
            format!("Primitive {} {which} face", primitive.index())
        }
        FaceId::BodyMeshFace { body, centroid, .. } => format!(
            "Body {} face at ({:.3}, {:.3}, {:.3})",
            body.index(),
            centroid[0] as f32 / 1000.0,
            centroid[1] as f32 / 1000.0,
            centroid[2] as f32 / 1000.0
        ),
    }
}

/// Screen-distance band within which two face picks count as "the same depth
/// under the cursor", so the nearer (camera-facing) one is preferred. This is
/// what keeps a hovered solid from selecting its hidden back face.
const FACE_PICK_DEPTH_TIE_PX: f32 = 0.5;

/// A pick candidate: which face, how far the cursor is from it on screen, how far it is from
/// the eye, and — for sketch profiles — how big it is. `area` is `INFINITY` for a body's own
/// face, which is never compared by size.
struct FacePick {
    face: FaceId,
    dist: f32,
    depth: f32,
    area: f32,
}

fn consider_face_pick_sized(best: &mut Option<FacePick>, candidate: FacePick) {
    if candidate.dist > crate::construction::FACE_PICK_MARGIN_PX {
        return;
    }
    let better = match best.as_ref() {
        None => true,
        Some(b) => {
            if candidate.dist < b.dist - FACE_PICK_DEPTH_TIE_PX {
                true
            } else if candidate.dist > b.dist + FACE_PICK_DEPTH_TIE_PX {
                false
            } else if candidate.area.is_finite() && b.area.is_finite() {
                // Two sketch profiles on top of each other (a hole inside a plate outline,
                // #822): the **smaller** one is what the cursor is aiming at. Depth can't
                // tell them apart — they're coplanar.
                candidate.area < b.area
            } else if candidate.face.is_construction_plane() != b.face.is_construction_plane() {
                // Real geometry beats a datum plane under the same cursor, even when the
                // plane is nearer the camera (#844): the planes are translucent references,
                // and a click on a body's face means that face.
                !candidate.face.is_construction_plane()
            } else {
                // Essentially the same screen distance (e.g. cursor inside both the
                // front and back face of a solid): prefer the one nearer the camera.
                candidate.depth < b.depth
            }
        }
    };
    if better {
        *best = Some(candidate);
    }
}

/// World area of a closed profile polygon, for the smaller-wins rule above.
fn polygon_world_area(points: &[Vec3]) -> f32 {
    if points.len() < 3 {
        return f32::INFINITY;
    }
    let mut sum = Vec3::ZERO;
    for i in 0..points.len() {
        sum += points[i].cross(points[(i + 1) % points.len()]);
    }
    sum.length() * 0.5
}

/// The exact face a sketch profile candidate (`Circle`/`Polygon`) was drawn on, if any.
fn sketch_host_face(doc: &Document, face: &FaceId) -> Option<FaceId> {
    let sketch = match face {
        FaceId::Circle(i) => doc.circles.get(*i)?.sketch,
        FaceId::Polygon(lines) => doc.lines.get(lines.first().copied()?)?.sketch,
        FaceId::ConstructionPlane(_)
        | FaceId::ExtrudeCap { .. }
        | FaceId::ExtrudeSide { .. }
        | FaceId::RevolveCap { .. }
        | FaceId::RevolveSide { .. }
        | FaceId::UnitFace { .. }
        | FaceId::PrimitiveFace { .. }
        | FaceId::RepeatedFace { .. }
        | FaceId::BodyMeshFace { .. } => return None,
    };
    doc.sketches.get(sketch).map(|s| s.face.clone())
}

/// True when some sketch profile under the cursor is drawn directly on `candidate`, at
/// essentially the same screen distance (#117): a rectangle or circle sketched on a solid's
/// face is coincident with that face, so its centroid can be farther from the eye than the
/// (larger) host face's — the depth tie-break in [`consider_face_pick`] would then wrongly
/// let the plain face win the pick, silently discarding the sketch (`Extrude` only picks
/// `Circle`/`Polygon` faces). A sketch drawn on a face is always meant to be picked over the
/// bare face beneath it, so skip the depth compare entirely once we know — by construction,
/// not by geometry — that they're the same surface.
///
/// Checked against **every** hit profile rather than just the current best (#822): with a
/// third face also under the cursor (the bracket's own profile lying on the ground, say)
/// the best could be that unrelated one, and the host face then beat the hole the user was
/// aiming at.
fn sketch_shadows(hosts: &[(FaceId, f32)], candidate: &FaceId, dist: f32) -> bool {
    hosts.iter().any(|(host, host_dist)| {
        host == candidate && (dist - host_dist).abs() <= FACE_PICK_DEPTH_TIE_PX
    })
}

/// Whether `face` was cut away as an open face of a committed shell (#1165). Those faces no
/// longer exist on the hollowed result and must not win sketch-face hover/pick.
fn is_shell_open_face(doc: &Document, face: &FaceId) -> bool {
    doc.shell_ops
        .iter()
        .any(|(_, op)| op.open_faces.iter().any(|f| f == face))
}

/// Whether a sketchable face still belongs to **live** geometry (#1219): analytic faces of
/// a shadow body (consumed by fuse/slice/boolean) must not highlight or accept a new sketch
/// — the live cut/merge result is what the user sees. Construction planes are always live.
/// Sketch profiles (circle/polygon) follow the host face they were drawn on.
fn sketch_face_is_live(doc: &Document, face: &FaceId) -> bool {
    match face {
        FaceId::ConstructionPlane(_) => true,
        FaceId::Circle(ci) => {
            let Some(circle) = doc.circles.get(*ci) else {
                return false;
            };
            if circle.shadow {
                return false;
            }
            doc.sketch_face(circle.sketch)
                .is_some_and(|host| sketch_face_is_live(doc, &host))
        }
        FaceId::Polygon(lines) => {
            let Some(&first) = lines.first() else {
                return false;
            };
            let Some(line) = doc.lines.get(first) else {
                return false;
            };
            if line.shadow {
                return false;
            }
            doc.sketch_face(line.sketch)
                .is_some_and(|host| sketch_face_is_live(doc, &host))
        }
        other => match crate::model::body_index_for_face(doc, other) {
            // No owning body (shouldn't reach here for the arms above) — keep it.
            None => true,
            Some(bi) => doc.bodies.get(bi).is_some_and(|b| !b.shadow),
        },
    }
}

/// Map a mesh face key (quantized centroid + normal) to an analytic [`FaceId`] on that body
/// when one matches (#1156/#1173). Outer shell walls map back to their primitive/extrude
/// faces; inner walls (and other non-analytic flats) return `None`.
pub fn analytic_face_from_mesh(
    doc: &Document,
    body: crate::model::BodyKey,
    centroid: [i32; 3],
    normal: [i32; 3],
) -> Option<FaceId> {
    let fingerprint = crate::extrude::document_pose_fingerprint(doc);
    let key = (body, centroid, normal);
    let cached = ANALYTIC_FACE_CACHE.with(|cache| match cache.try_borrow_mut() {
        Ok(mut cache) => {
            if cache.0 != fingerprint {
                cache.0 = fingerprint;
                cache.1.clear();
            }
            cache.1.get(&key).cloned()
        }
        Err(_) => None,
    });
    if let Some(hit) = cached {
        return hit;
    }
    let found = analytic_face_from_mesh_uncached(doc, body, centroid, normal);
    ANALYTIC_FACE_CACHE.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut() {
            cache.1.insert(key, found.clone());
        }
    });
    found
}

fn analytic_face_from_mesh_uncached(
    doc: &Document,
    body: crate::model::BodyKey,
    centroid: [i32; 3],
    normal: [i32; 3],
) -> Option<FaceId> {
    let q = crate::hierarchy::quantize_body_point;
    for face in analytic_faces_of_body(doc, body) {
        let Some(frame) = sketch_frame(doc, face.clone()) else {
            continue;
        };
        let c = crate::extrude::face_boundary_loop_world(doc, &face)
            .map(|pts| pts.iter().copied().sum::<Vec3>() / pts.len().max(1) as f32)
            .unwrap_or(frame.origin);
        let n = frame.normal.normalize_or_zero();
        if q(c) == centroid && (q(n) == normal || q(-n) == normal) {
            return Some(face);
        }
    }
    None
}

thread_local! {
    /// Hover maps every nearby mesh group back to an analytic face (#1466).
    /// The match walks sketch frames; memoize it on the pose fingerprint.
    static ANALYTIC_FACE_CACHE: std::cell::RefCell<(
        u64,
        std::collections::HashMap<
            (crate::model::BodyKey, [i32; 3], [i32; 3]),
            Option<FaceId>,
        >,
    )> = std::cell::RefCell::new((0, std::collections::HashMap::new()));
}

/// Sketchable [`FaceId`] for a coplanar mesh-face triangle group under the cursor (#1173):
/// the matching analytic face when one exists, else a [`FaceId::BodyMeshFace`] so shell
/// inner walls (and other non-analytic flats) can host a sketch.
fn sketch_face_id_for_mesh_group(
    doc: &Document,
    body: crate::model::BodyKey,
    triangles: &[[Vec3; 3]],
) -> Option<FaceId> {
    if triangles.is_empty() {
        return None;
    }
    let centroid = crate::extrude::face_group_center(triangles);
    let normal = (triangles[0][1] - triangles[0][0])
        .cross(triangles[0][2] - triangles[0][0])
        .normalize_or_zero();
    if normal.length_squared() < 1e-8 {
        return None;
    }
    let q = crate::hierarchy::quantize_body_point;
    let (qc, qn) = (q(centroid), q(normal));
    if let Some(analytic) = analytic_face_from_mesh(doc, body, qc, qn) {
        // Don't re-offer open faces the shell removed.
        if !is_shell_open_face(doc, &analytic) {
            return Some(analytic);
        }
    }
    Some(FaceId::BodyMeshFace {
        body,
        centroid: qc,
        normal: qn,
    })
}

/// Screen-space pick of a body mesh face group: 0 when the cursor is inside any projected
/// triangle, else the min edge distance — plus the world point under the cursor for depth.
fn mesh_face_pick_distance(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    triangles: &[[Vec3; 3]],
) -> Option<(f32, Vec3)> {
    if triangles.is_empty() {
        return None;
    }
    let mut best_dist = f32::MAX;
    let mut at = crate::extrude::face_group_center(triangles);
    let mut inside = false;
    for tri in triangles {
        let (Some(a), Some(b), Some(c)) = (project(tri[0]), project(tri[1]), project(tri[2]))
        else {
            continue;
        };
        if point_in_tri(screen, a, b, c) {
            inside = true;
            // Barycentric world point under the cursor.
            let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
            if area.abs() > 1e-6 {
                let w0 = ((b.x - screen.x) * (c.y - screen.y)
                    - (c.x - screen.x) * (b.y - screen.y))
                    / area;
                let w1 = ((c.x - screen.x) * (a.y - screen.y)
                    - (a.x - screen.x) * (c.y - screen.y))
                    / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                    at = tri[0] * w0 + tri[1] * w1 + tri[2] * w2;
                }
            }
            best_dist = 0.0;
            break;
        }
        let edge = dist_point_to_segment_px(screen, a, b)
            .min(dist_point_to_segment_px(screen, b, c))
            .min(dist_point_to_segment_px(screen, c, a));
        if edge < best_dist {
            best_dist = edge;
        }
    }
    if !inside && best_dist == f32::MAX {
        return None;
    }
    Some((if inside { 0.0 } else { best_dist }, at))
}

fn centroid(points: &[Vec3]) -> Vec3 {
    if points.is_empty() {
        return Vec3::ZERO;
    }
    points.iter().copied().sum::<Vec3>() / points.len() as f32
}

/// The world point on a planar face that lands under the cursor (#844): the polygon is
/// triangulated, the triangle the cursor sits in is found in screen space, and its world
/// corners are blended by the same weights. Falls back to the centroid when the cursor is
/// outside the face (a near miss picked up by the edge distance).
///
/// This is what depth comparisons need: the *centroid's* distance says nothing about which
/// face is in front under the cursor, and a big datum plane's centroid could beat a small
/// body face the cursor was actually over.
fn face_point_under_cursor(
    screen: eframe::egui::Pos2,
    projected: &[eframe::egui::Pos2],
    poly: &[Vec3],
) -> Option<Vec3> {
    if poly.len() < 3 || projected.len() != poly.len() {
        return None;
    }
    let normal = (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
    for [a, b, c] in crate::polygon::triangulate_planar(poly, normal) {
        let (pa, pb, pc) = (projected[a], projected[b], projected[c]);
        let area = (pb.x - pa.x) * (pc.y - pa.y) - (pc.x - pa.x) * (pb.y - pa.y);
        if area.abs() < 1e-6 {
            continue;
        }
        let w0 = ((pb.x - screen.x) * (pc.y - screen.y) - (pc.x - screen.x) * (pb.y - screen.y))
            / area;
        let w1 = ((pc.x - screen.x) * (pa.y - screen.y) - (pa.x - screen.x) * (pc.y - screen.y))
            / area;
        let w2 = 1.0 - w0 - w1;
        if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
            return Some(poly[a] * w0 + poly[b] * w1 + poly[c] * w2);
        }
    }
    None
}

fn quad_face_pick_distance(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    corners: [Vec3; 4],
) -> Option<(f32, Vec3)> {
    let pts: Option<Vec<eframe::egui::Pos2>> = corners.iter().map(|&c| project(c)).collect();
    let pts = pts?;
    let quad = [pts[0], pts[1], pts[2], pts[3]];
    let dist = if point_in_screen_quad(screen, quad) {
        0.0
    } else {
        dist_point_to_quad_edges(screen, quad)
    };
    // The point under the cursor, not the middle of the face — that's what decides which of
    // two overlapping faces is in front (#844).
    let at = face_point_under_cursor(screen, &pts, &corners).unwrap_or_else(|| centroid(&corners));
    Some((dist, at))
}

/// Pick a sketchable face (rectangle, circle, or construction plane) under the cursor.
pub fn pick_sketch_face(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    eye: Vec3,
) -> Option<FaceId> {
    let mut best: Option<FacePick> = None;
    let depth = |p: Vec3| (p - eye).length();
    // Host faces of every sketch profile hit this pass, so those hosts step aside for them.
    let mut shadowed_hosts: Vec<(FaceId, f32)> = Vec::new();
    let mut note_host = |face: &FaceId, dist: f32, doc: &Document| {
        if dist <= crate::construction::FACE_PICK_MARGIN_PX {
            if let Some(host) = sketch_host_face(doc, face) {
                shadowed_hosts.push((host, dist));
            }
        }
    };

    for (i, circle) in doc.circles.iter().collect::<Vec<_>>().into_iter().rev() {
        if let Some((dist, c)) = circle_face_pick_distance(screen, doc, circle, project) {
            let face = FaceId::Circle(i);
            if !sketch_face_is_live(doc, &face) {
                continue;
            }
            note_host(&face, dist, doc);
            consider_face_pick_sized(
                &mut best,
                FacePick {
                    face,
                    dist,
                    depth: depth(c),
                    area: std::f32::consts::PI * circle.r * circle.r,
                },
            );
        }
    }

    // Closed loops of plain lines (#66).
    for sketch in doc.sketches.keys().collect::<Vec<_>>().into_iter().rev() {
        for lines in crate::polygon::closed_line_loops(doc, sketch) {
            if let Some((poly, _)) = crate::extrude::face_profile_world(
                doc,
                &crate::model::ExtrudeFace::Polygon(lines.clone()),
            ) {
                if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                    let face = FaceId::Polygon(lines);
                    if !sketch_face_is_live(doc, &face) {
                        continue;
                    }
                    note_host(&face, dist, doc);
                    consider_face_pick_sized(
                        &mut best,
                        FacePick {
                            face,
                            dist,
                            depth: depth(c),
                            area: polygon_world_area(&poly),
                        },
                    );
                }
            }
        }
    }

    // Planar caps of extruded bodies (so sketches can be placed on them). Tested
    // before construction planes since a solid cap occludes the datum plane.
    for (ei, extrusion) in doc.extrusions.iter().collect::<Vec<_>>().into_iter().rev() {
        for profile in &extrusion.faces {
            for top in [true, false] {
                if let Some((dist, c)) =
                    cap_face_pick_distance(screen, project, doc, ei, profile.clone(), top)
                {
                    let candidate = FaceId::ExtrudeCap {
                        extrusion: ei,
                        profile: profile.clone(),
                        top,
                    };
                    if !is_shell_open_face(doc, &candidate)
                        && sketch_face_is_live(doc, &candidate)
                        && !sketch_shadows(&shadowed_hosts, &candidate, dist)
                    {
                        consider_face_pick_sized(
                        &mut best,
                        FacePick { face: candidate, dist, depth: depth(c), area: f32::INFINITY },
                    );
                    }
                }
            }
            // Flat side walls (rectangular profiles) are sketchable too.
            for edge in 0..crate::extrude::side_face_count(profile) {
                if let Some((dist, c)) =
                    side_face_pick_distance(screen, project, doc, ei, profile.clone(), edge)
                {
                    let candidate = FaceId::ExtrudeSide {
                        extrusion: ei,
                        profile: profile.clone(),
                        edge: edge as u8,
                    };
                    if !is_shell_open_face(doc, &candidate)
                        && sketch_face_is_live(doc, &candidate)
                        && !sketch_shadows(&shadowed_hosts, &candidate, dist)
                    {
                        consider_face_pick_sized(
                        &mut best,
                        FacePick { face: candidate, dist, depth: depth(c), area: f32::INFINITY },
                    );
                    }
                }
            }
        }
    }

    // Flat sides of revolves are sketchable too (#621): a partial sweep's start/end
    // profile caps, and the flat washer faces swept by axis-perpendicular profile edges.
    // Reverse slot order, so the most recently created revolve wins a tie the way the
    // positional pass did.
    for (ri, rev) in doc.revolutions.iter().collect::<Vec<_>>().into_iter().rev() {
        for profile in &rev.faces {
            for end in [false, true] {
                let Some((poly, _)) =
                    crate::extrude::revolve_cap_polygon_world(doc, ri, profile, end)
                else {
                    continue;
                };
                if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                    let candidate = FaceId::RevolveCap {
                        revolution: ri,
                        profile: profile.clone(),
                        end,
                    };
                    if !is_shell_open_face(doc, &candidate)
                        && sketch_face_is_live(doc, &candidate)
                        && !sketch_shadows(&shadowed_hosts, &candidate, dist)
                    {
                        consider_face_pick_sized(
                        &mut best,
                        FacePick { face: candidate, dist, depth: depth(c), area: f32::INFINITY },
                    );
                    }
                }
            }
            for edge in 0..crate::extrude::revolve_side_count(profile) {
                let Some((poly, _, _)) = crate::extrude::revolve_side_geom(doc, ri, profile, edge)
                else {
                    continue;
                };
                if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                    let candidate = FaceId::RevolveSide {
                        revolution: ri,
                        profile: profile.clone(),
                        edge: edge as u8,
                    };
                    if !is_shell_open_face(doc, &candidate)
                        && sketch_face_is_live(doc, &candidate)
                        && !sketch_shadows(&shadowed_hosts, &candidate, dist)
                    {
                        consider_face_pick_sized(
                        &mut best,
                        FacePick { face: candidate, dist, depth: depth(c), area: f32::INFINITY },
                    );
                    }
                }
            }
        }
    }

    // Flat faces of imported units are sketchable (#725): each live instance's analytic
    // inner faces, placed by its transform.
    for instance in doc.unit_instances.keys().collect::<Vec<_>>() {
        let Some(eval) = crate::units::evaluate_instance(doc, instance) else {
            continue;
        };
        for inner_face in crate::units::inner_face_ids(&eval.document) {
            let Some(poly) = crate::units::unit_face_world_polygon(doc, instance, &inner_face)
            else {
                continue;
            };
            if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                let candidate = FaceId::UnitFace {
                    instance,
                    face: Box::new(inner_face),
                };
                if sketch_face_is_live(doc, &candidate)
                    && !sketch_shadows(&shadowed_hosts, &candidate, dist)
                {
                    consider_face_pick_sized(
                        &mut best,
                        FacePick { face: candidate, dist, depth: depth(c), area: f32::INFINITY },
                    );
                }
            }
        }
    }

    // Flat faces of primitive shapes (#1103): a cuboid's six faces and a cylinder's two
    // caps are sketchable, exactly like an extrusion's caps and side walls, so a shape
    // placed by the Shape tool is indistinguishable from one extruded from a sketch.
    for (pi, shape) in doc.primitives.iter().collect::<Vec<_>>().into_iter().rev() {
        for face in crate::primitives::flat_faces(shape) {
            let Some(poly) = crate::primitives::face_polygon(doc, shape, face) else {
                continue;
            };
            // A degenerate/zero-sized primitive has no face polygon to hit.
            let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) else {
                continue;
            };
            let candidate = FaceId::PrimitiveFace { primitive: pi, face };
            if !is_shell_open_face(doc, &candidate)
                && sketch_face_is_live(doc, &candidate)
                && !sketch_shadows(&shadowed_hosts, &candidate, dist)
            {
                consider_face_pick_sized(
                    &mut best,
                    FacePick { face: candidate, dist, depth: depth(c), area: f32::INFINITY },
                );
            }
        }
    }

    // Faces of repeated body instances (#1116/#1119): each copy's flat faces, placed by the
    // instance transform — extrusion caps/sides **and** Shape-tool primitive faces, so a
    // repeated cuboid is pickable for sketch/extrude/revolve just like the original.
    for (op_index, op) in doc.repeat_ops.iter().collect::<Vec<_>>().into_iter().rev() {
        let Some(offsets) = crate::extrude::repeat_offsets(doc, op) else {
            continue;
        };
        for &body in &op.targets {
            let Some(body_rec) = doc.bodies.get(body) else {
                continue;
            };
            // Source analytic faces of this target body (extrusion- or primitive-backed).
            let mut source_faces: Vec<FaceId> = Vec::new();
            for &ei in body_rec.source.extrusion_indices() {
                let Some(ext) = doc.extrusions.get(ei) else {
                    continue;
                };
                for profile in &ext.faces {
                    for top in [true, false] {
                        source_faces.push(FaceId::ExtrudeCap {
                            extrusion: ei,
                            profile: profile.clone(),
                            top,
                        });
                    }
                    for edge in 0..crate::extrude::side_face_count(profile) {
                        source_faces.push(FaceId::ExtrudeSide {
                            extrusion: ei,
                            profile: profile.clone(),
                            edge: edge as u8,
                        });
                    }
                }
            }
            if let crate::model::BodySource::Primitive(pi) = body_rec.source {
                if let Some(shape) = doc.primitives.get(pi) {
                    for face in crate::primitives::flat_faces(shape) {
                        source_faces.push(FaceId::PrimitiveFace {
                            primitive: pi,
                            face,
                        });
                    }
                }
            }
            for (i, &_offset) in offsets.iter().enumerate() {
                let instance = i + 1;
                let Some(m) = crate::extrude::repeat_instance_transform(doc, op, instance) else {
                    continue;
                };
                for source in &source_faces {
                    let Some(poly) = crate::extrude::face_boundary_loop_world(doc, source) else {
                        continue;
                    };
                    let pts: Vec<Vec3> = poly.iter().map(|&p| m.transform_point3(p)).collect();
                    let Some((dist, c)) = polygon_face_pick_distance(screen, project, &pts) else {
                        continue;
                    };
                    let candidate = FaceId::RepeatedFace {
                        face: Box::new(source.clone()),
                        op: op_index,
                        instance,
                    };
                    if sketch_face_is_live(doc, &candidate)
                        && !sketch_shadows(&shadowed_hosts, &candidate, dist)
                    {
                        consider_face_pick_sized(
                            &mut best,
                            FacePick {
                                face: candidate,
                                dist,
                                depth: depth(c),
                                area: f32::INFINITY,
                            },
                        );
                    }
                }
            }
        }
    }

    for (i, plane) in doc.construction_planes.iter().collect::<Vec<_>>().into_iter().rev() {
        // A deleted plane is not there to be picked (#1051) — it is gone from the arena, so
        // this loop cannot reach it at all.
        let corners = crate::construction::plane_corners(plane);
        if let Some((dist, c)) = quad_face_pick_distance(screen, project, corners) {
            let candidate = FaceId::ConstructionPlane(i);
            if !sketch_shadows(&shadowed_hosts, &candidate, dist) {
                consider_face_pick_sized(
                        &mut best,
                        FacePick { face: candidate, dist, depth: depth(c), area: f32::INFINITY },
                    );
            }
        }
    }

    // Live mesh faces of every body (#1173): a shell's inner wall, a boolean face, an
    // imported flat — anything that has no (or a different) analytic identity. Ranked by
    // the same screen-distance + eye-depth rules as the analytics above, so the surface
    // under the cursor nearest the camera wins over a parallel outer face behind it.
    // Bounds-reject bodies first so a large document does not walk every mesh every frame.
    let bounds = crate::extrude::body_world_bounds_all(doc);
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        if !bounds.get(&bi).copied().flatten().is_some_and(|b| {
            crate::construction::screen_bounds_hit(screen, project, b, 0.0)
        }) {
            continue;
        }
        let group_bounds = crate::extrude::body_face_group_bounds(doc, bi);
        let groups = crate::extrude::body_face_groups(doc, bi);
        for (gi, triangles) in groups.iter().enumerate() {
            // Cylindrical walls are not sketchable flats.
            if crate::extrude::fit_cylinder(triangles).is_some() {
                continue;
            }
            if !group_bounds.get(gi).is_some_and(|b| {
                crate::construction::screen_bounds_hit(screen, project, *b, 0.0)
            }) {
                continue;
            }
            let Some((dist, c)) = mesh_face_pick_distance(screen, project, triangles) else {
                continue;
            };
            let Some(candidate) = sketch_face_id_for_mesh_group(doc, bi, triangles) else {
                continue;
            };
            if !sketch_face_is_live(doc, &candidate) {
                continue;
            }
            if sketch_shadows(&shadowed_hosts, &candidate, dist) {
                continue;
            }
            consider_face_pick_sized(
                &mut best,
                FacePick {
                    face: candidate,
                    dist,
                    depth: depth(c),
                    area: f32::INFINITY,
                },
            );
        }
    }

    best.map(|pick| pick.face)
}

/// Every sketchable analytic face within `radius` px of the cursor (#625): the same
/// candidates [`pick_sketch_face`] chooses among — sketch profiles (circles and closed
/// line loops), extrusion caps/side walls, revolve flat faces, and construction planes —
/// but **all** of them, not just the best, and with no occlusion preference, for the
/// Selection Exploder crowd. Returns `(face, world centroid, screen distance)` triples.
pub fn sketch_faces_near(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    radius: f32,
) -> Vec<(FaceId, Vec3, f32)> {
    let mut out: Vec<(FaceId, Vec3, f32)> = Vec::new();
    let mut push = |face: FaceId, centroid: Vec3, dist: f32| {
        // Shell open faces no longer exist on the hollowed body (#1165).
        // Shadow-body analytic faces are not live sketch targets (#1219).
        if dist <= radius && !is_shell_open_face(doc, &face) && sketch_face_is_live(doc, &face) {
            out.push((face, centroid, dist));
        }
    };

    for (i, circle) in doc.circles.iter() {
        if let Some((dist, c)) = circle_face_pick_distance(screen, doc, circle, project) {
            push(FaceId::Circle(i), c, dist);
        }
    }
    for sketch in doc.sketches.keys().collect::<Vec<_>>() {
        for lines in crate::polygon::closed_line_loops(doc, sketch) {
            if let Some((poly, _)) = crate::extrude::face_profile_world(
                doc,
                &crate::model::ExtrudeFace::Polygon(lines.clone()),
            ) {
                if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                    push(FaceId::Polygon(lines), c, dist);
                }
            }
        }
    }
    for (ei, extrusion) in doc.extrusions.iter() {
        for profile in &extrusion.faces {
            for top in [true, false] {
                if let Some((dist, c)) =
                    cap_face_pick_distance(screen, project, doc, ei, profile.clone(), top)
                {
                    push(
                        FaceId::ExtrudeCap {
                            extrusion: ei,
                            profile: profile.clone(),
                            top,
                        },
                        c,
                        dist,
                    );
                }
            }
            for edge in 0..crate::extrude::side_face_count(profile) {
                if let Some((dist, c)) =
                    side_face_pick_distance(screen, project, doc, ei, profile.clone(), edge)
                {
                    push(
                        FaceId::ExtrudeSide {
                            extrusion: ei,
                            profile: profile.clone(),
                            edge: edge as u8,
                        },
                        c,
                        dist,
                    );
                }
            }
        }
    }
    for (ri, rev) in doc.revolutions.iter() {
        for profile in &rev.faces {
            for end in [false, true] {
                if let Some((poly, _)) =
                    crate::extrude::revolve_cap_polygon_world(doc, ri, profile, end)
                {
                    if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                        push(
                            FaceId::RevolveCap {
                                revolution: ri,
                                profile: profile.clone(),
                                end,
                            },
                            c,
                            dist,
                        );
                    }
                }
            }
            for edge in 0..crate::extrude::revolve_side_count(profile) {
                if let Some((poly, _, anchor)) = crate::extrude::revolve_side_geom(doc, ri, profile, edge)
                {
                    if let Some((dist, _)) = polygon_face_pick_distance(screen, project, &poly) {
                        // The on-face anchor, not the boundary centroid — a full washer's
                        // centroid sits in its hole, where a redirected pick would miss.
                        push(
                            FaceId::RevolveSide {
                                revolution: ri,
                                profile: profile.clone(),
                                edge: edge as u8,
                            },
                            anchor,
                            dist,
                        );
                    }
                }
            }
        }
    }
    for (i, plane) in doc.construction_planes.iter() {
        let corners = crate::construction::plane_corners(plane);
        if let Some((dist, c)) = quad_face_pick_distance(screen, project, corners) {
            push(FaceId::ConstructionPlane(i), c, dist);
        }
    }
    // Primitive shape faces (#1103), mirroring `pick_sketch_face`.
    for (pi, shape) in doc.primitives.iter() {
        for face in crate::primitives::flat_faces(shape) {
            let Some(poly) = crate::primitives::face_polygon(doc, shape, face) else {
                continue;
            };
            if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                push(FaceId::PrimitiveFace { primitive: pi, face }, c, dist);
            }
        }
    }
    // Live mesh faces (#1173): shell inners and other non-analytic flats, so the Selection
    // Exploder can fan them out when more than one surface sits under the cursor.
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        for triangles in crate::extrude::body_face_groups(doc, bi).iter() {
            if crate::extrude::fit_cylinder(triangles).is_some() {
                continue;
            }
            let Some((dist, c)) = mesh_face_pick_distance(screen, project, triangles) else {
                continue;
            };
            let Some(candidate) = sketch_face_id_for_mesh_group(doc, bi, triangles) else {
                continue;
            };
            // Analytic faces already pushed above — don't double-list the same outer wall.
            if matches!(candidate, FaceId::BodyMeshFace { .. }) {
                push(candidate, c, dist);
            }
        }
    }
    out
}

/// Nearest planar body face (#144) under the cursor across all 3D bodies, for 3D hover/selection.
/// Mirrors [`pick_sketch_face`]'s screen-space containment test plus eye-depth ordering, but over
/// a solid mesh's coplanar-triangle groups (`solid_mesh_coplanar_faces`) rather than sketch
/// profiles — so any face of any body, including boolean-cut and imported ones, can be picked.
pub fn pick_body_face(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    eye: Vec3,
) -> Option<crate::construction::PickTargetKind> {
    pick_body_face_where(screen, project, doc, eye, |_| true)
}

/// [`pick_body_face`] restricted to the bodies `allow` accepts (#1336).
///
/// The filter belongs *inside* the search: a body in front that the pick cannot take
/// (the part being moved, while choosing where it lands) would otherwise win and then
/// be rejected, leaving the face behind it unpickable.
pub fn pick_body_face_where(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    eye: Vec3,
    allow: impl Fn(crate::model::BodyKey) -> bool,
) -> Option<crate::construction::PickTargetKind> {
    let mut best: Option<(crate::construction::PickTargetKind, f32)> = None;
    // Reject the whole body, then each face, on screen-space bounds before touching a single
    // triangle (#1026). This runs every frame the camera moves, and testing every triangle of
    // every body is what made zooming over a large document lag. The bounds come batched
    // because the per-body cached accessors each re-hash the whole document.
    let bounds = crate::extrude::body_world_bounds_all(doc);
    for (bi, body) in doc.bodies.iter() {
        if body.shadow || !allow(bi) {
            continue;
        }
        if !bounds.get(&bi).copied().flatten().is_some_and(|b| {
            crate::construction::screen_bounds_hit(screen, project, b, 0.0)
        }) {
            continue;
        }
        let group_bounds = crate::extrude::body_face_group_bounds(doc, bi);
        // Walk groups by reference — a hole wall can be hundreds of triangles (#1141). Cloning
        // every group every hover frame (including misses that fail the bounds test) was the
        // dominant cost when the cursor sat over a body with circular cuts.
        let groups = crate::extrude::body_face_groups(doc, bi);
        for (gi, triangles) in groups.iter().enumerate() {
            if !group_bounds.get(gi).is_some_and(|b| {
                crate::construction::screen_bounds_hit(screen, project, *b, 0.0)
            }) {
                continue;
            }
            let inside = triangles.iter().any(|tri| {
                matches!(
                    (project(tri[0]), project(tri[1]), project(tri[2])),
                    (Some(a), Some(b), Some(c)) if point_in_tri(screen, a, b, c)
                )
            });
            if !inside {
                continue;
            }
            let count = (triangles.len() * 3).max(1) as f32;
            let centroid =
                triangles.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
            let depth = (centroid - eye).length();
            if best.as_ref().is_none_or(|(_, d)| depth < *d) {
                // A round wall is a cylinder, not a face (#1013): it has no one normal, so
                // calling it flat gives it a nonsense plane. Fit against the borrowed group
                // (no clone); only a flat face pays for cloning its triangles into the pick.
                let kind = match crate::extrude::fit_cylinder(triangles) {
                    Some(cylinder) => crate::construction::PickTargetKind::BodyCylinder {
                        body: bi,
                        cylinder: Box::new(cylinder),
                    },
                    None => {
                        let normal = (triangles[0][1] - triangles[0][0])
                            .cross(triangles[0][2] - triangles[0][0])
                            .normalize_or_zero();
                        crate::construction::PickTargetKind::BodyFace {
                            body: bi,
                            triangles: triangles.clone(),
                            normal,
                        }
                    }
                };
                best = Some((kind, depth));
            }
        }
    }
    best.map(|(kind, _)| kind)
}

/// Every body face whose projected area is within `radius` px of the cursor (#555/#556) — front
/// and back, not just the nearest ray-hit face [`pick_body_face`] returns. For each non-deleted,
/// non-shadow body, each coplanar-triangle group (`solid_mesh_coplanar_faces`) is measured by the
/// minimum screen distance from the cursor to its projected triangles: 0 when the cursor is inside
/// any projected triangle, else the min distance to the projected triangle edges. This catches a
/// narrow face seen edge-on — a thin projected sliver between its two edges — that no single-face
/// ray hit would report. Returns each face's `PickTargetKind::BodyFace`, its world centroid (the
/// exploder's connecting-line anchor), and that screen distance (for nearest-first ordering).
pub fn body_faces_near(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    radius: f32,
) -> Vec<(crate::construction::PickTargetKind, Vec3, f32)> {
    let mut out: Vec<(crate::construction::PickTargetKind, Vec3, f32)> = Vec::new();
    for (bi, body) in doc.bodies.iter() {
        if body.shadow {
            continue;
        }
        // By reference until a group is near enough to keep (#1141).
        for triangles in crate::extrude::body_face_groups(doc, bi).iter() {
            let mut dist = f32::MAX;
            for tri in triangles {
                let (Some(a), Some(b), Some(c)) =
                    (project(tri[0]), project(tri[1]), project(tri[2]))
                else {
                    continue;
                };
                if point_in_tri(screen, a, b, c) {
                    dist = 0.0;
                    break;
                }
                let edge = dist_point_to_segment_px(screen, a, b)
                    .min(dist_point_to_segment_px(screen, b, c))
                    .min(dist_point_to_segment_px(screen, c, a));
                dist = dist.min(edge);
            }
            if dist > radius {
                continue;
            }
            let count = (triangles.len() * 3).max(1) as f32;
            let centroid =
                triangles.iter().flat_map(|t| t.iter()).copied().sum::<Vec3>() / count;
            let normal = (triangles[0][1] - triangles[0][0])
                .cross(triangles[0][2] - triangles[0][0])
                .normalize_or_zero();
            out.push((
                crate::construction::PickTargetKind::BodyFace {
                    body: bi,
                    triangles: triangles.clone(),
                    normal,
                },
                centroid,
                dist,
            ));
        }
    }
    out
}

/// Screen-space pick distance to an extrusion cap polygon (0 inside).
fn cap_face_pick_distance(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    profile: crate::model::ExtrudeFace,
    top: bool,
) -> Option<(f32, Vec3)> {
    let poly = crate::extrude::cap_polygon_world(doc, extrusion, &profile, top)?;
    polygon_face_pick_distance(screen, project, &poly)
}

/// Screen-space pick distance to an extrusion side wall (0 inside).
fn side_face_pick_distance(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    extrusion: crate::model::ExtrusionKey,
    profile: crate::model::ExtrudeFace,
    edge: usize,
) -> Option<(f32, Vec3)> {
    let quad = crate::extrude::side_quad_world(doc, extrusion, &profile, edge)?;
    polygon_face_pick_distance(screen, project, &quad)
}

/// Screen-space pick distance to a planar world-space polygon (0 inside, else
/// nearest edge), paired with the polygon's world centroid for depth ordering.
fn polygon_face_pick_distance(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    poly: &[Vec3],
) -> Option<(f32, Vec3)> {
    let pts: Option<Vec<eframe::egui::Pos2>> = poly.iter().map(|&p| project(p)).collect();
    let pts = pts?;
    if pts.len() < 3 {
        return None;
    }
    // The point under the cursor when there is one, else the middle of the face (#844).
    let c = face_point_under_cursor(screen, &pts, poly).unwrap_or_else(|| centroid(poly));
    let normal = (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
    let inside = crate::polygon::triangulate_planar(poly, normal)
        .into_iter()
        .any(|[a, b, c]| point_in_tri(screen, pts[a], pts[b], pts[c]));
    if inside {
        return Some((0.0, c));
    }
    let mut edge = f32::MAX;
    for i in 0..pts.len() {
        let j = (i + 1) % pts.len();
        edge = edge.min(dist_point_to_segment_px(screen, pts[i], pts[j]));
    }
    Some((edge, c))
}

fn circle_face_pick_distance(
    screen: eframe::egui::Pos2,
    doc: &Document,
    circle: &Circle,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
) -> Option<(f32, Vec3)> {
    let center = circle_world_center(doc, circle)?;
    let frame = sketch_geometry_frame(doc, circle.sketch)?;
    let rim = local_to_world(&frame, circle.cx + circle.r, circle.cy);
    let center_sp = project(center)?;
    let rim_sp = project(rim)?;
    let radius = (rim_sp - center_sp).length();
    if radius < 1e-3 {
        return None;
    }
    let d = (screen - center_sp).length();
    Some((if d <= radius { 0.0 } else { d - radius }, center))
}

fn point_in_screen_quad(p: eframe::egui::Pos2, quad: [eframe::egui::Pos2; 4]) -> bool {
    point_in_tri(p, quad[0], quad[1], quad[2]) || point_in_tri(p, quad[0], quad[2], quad[3])
}

fn point_in_tri(p: eframe::egui::Pos2, a: eframe::egui::Pos2, b: eframe::egui::Pos2, c: eframe::egui::Pos2) -> bool {
    let v0 = c - a;
    let v1 = b - a;
    let v2 = p - a;
    let dot00 = v0.dot(v0);
    let dot01 = v0.dot(v1);
    let dot02 = v0.dot(v2);
    let dot11 = v1.dot(v1);
    let dot12 = v1.dot(v2);
    let denom = dot00 * dot11 - dot01 * dot01;
    if denom.abs() < 1e-8 {
        return false;
    }
    let inv = 1.0 / denom;
    let u = (dot11 * dot02 - dot01 * dot12) * inv;
    let v = (dot00 * dot12 - dot01 * dot02) * inv;
    // Inclusive edges with a small epsilon — matches `polygon::point_in_triangle_2d`.
    // Exact boundary hits (e.g. rectangle diagonal under a true top view) otherwise
    // drop out of both fan triangles and leave only the construction plane to pick.
    const EPS: f32 = 1e-4;
    u >= -EPS && v >= -EPS && (u + v) <= 1.0 + EPS
}

fn dist_point_to_quad_edges(p: eframe::egui::Pos2, quad: [eframe::egui::Pos2; 4]) -> f32 {
    let edges = [(0, 1), (1, 2), (2, 3), (3, 0)];
    edges
        .iter()
        .map(|&(i, j)| dist_point_to_segment_px(p, quad[i], quad[j]))
        .fold(f32::MAX, f32::min)
}

fn dist_point_to_segment_px(p: eframe::egui::Pos2, a: eframe::egui::Pos2, b: eframe::egui::Pos2) -> f32 {
    let ab = b - a;
    if ab.length_sq() < 1e-4 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}


#[cfg(test)]
mod pick_tests {
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::circle_key_for_slot as rkey;
    use super::*;

    /// #822: a hole drawn inside a bigger profile on the same plane wins the pick — the
    /// cursor is inside both, and the smaller shape is the one being aimed at. (Depth can't
    /// separate coplanar profiles, and the bigger one's centroid is often nearer the eye.)
    #[test]
    fn a_smaller_coplanar_profile_wins_the_face_pick() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        // A 40×30 rectangle as four lines, with a small circle inside it.
        for (x0, y0, x1, y1) in [
            (0.0, 0.0, 40.0, 0.0),
            (40.0, 0.0, 40.0, 30.0),
            (40.0, 30.0, 0.0, 30.0),
            (0.0, 30.0, 0.0, 0.0),
        ] {
            doc.lines
                .insert(crate::model::Line::from_local_endpoints(sketch, x0, y0, x1, y1));
        }
        doc.circles
            .insert(crate::model::Circle::from_local_center_radius(sketch, 20.0, 15.0, 3.0, 0.0));

        let cam = crate::camera::Camera::default();
        let viewport =
            eframe::egui::Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |w: Vec3| cam.project(w, viewport, &vp);
        let centre = local_to_world(&sketch_geometry_frame(&doc, sketch).unwrap(), 20.0, 15.0);
        let at = project(centre).expect("the circle's centre projects");

        assert_eq!(
            pick_sketch_face(at, &project, &doc, cam.eye()),
            Some(FaceId::Circle(rkey(0))),
            "the hole, not the rectangle around it"
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::model::line_key_for_slot as lkey;
    use crate::model::plane_key_for_slot as pkey;
    use crate::model::retain_ground_plane_only;
    use crate::model::circle_key_for_slot as rkey;
    use crate::model::extrusion_key_for_slot as xkey;
    use super::*;
    use crate::model::Sketch;

    /// #833: a new document opens with the three datum planes, XY first (so plane 0 stays
    /// the ground plane everything else assumes), each one sitting in a single quadrant.
    #[test]
    fn default_document_has_the_three_datum_planes() {
        let doc = Document::default();
        assert_eq!(doc.construction_planes.len(), 3);
        assert!((doc.construction_planes[pkey(0)].normal.z - 1.0).abs() < 1e-4);
        assert!((doc.construction_planes[pkey(1)].normal.y - 1.0).abs() < 1e-4);
        assert!((doc.construction_planes[pkey(2)].normal.x - 1.0).abs() < 1e-4);
        for plane in doc.construction_planes.values() {
            assert_eq!(
                plane.extent,
                crate::model::PlaneExtent::quadrant(DATUM_PLANE_SIZE_MM, DATUM_PLANE_GAP_MM)
            );
            // #838: every plane stands the same distance clear of the origin.
            assert_eq!(plane.extent.u_min, DATUM_PLANE_GAP_MM);
            assert_eq!(plane.extent.v_min, DATUM_PLANE_GAP_MM);
        }
        assert!(doc.shape_order.is_empty());
    }

    #[test]
    fn sketch_on_plane_stores_local_coordinates() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let frame = sketch_geometry_frame(&doc, sketch).unwrap();
        let p = local_to_world(&frame, 10.0, 20.0);
        let (u, v) = world_to_local(&frame, p);
        assert!((u - 10.0).abs() < 1e-4);
        assert!((v - 20.0).abs() < 1e-4);
    }

    #[test]
    fn circle_face_frame_origin_is_center() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(Circle::from_local_center_radius(sketch, 5.0, 7.0, 10.0, 0.0));
        let frame = sketch_frame(&doc, FaceId::Circle(rkey(0))).unwrap();
        assert!((frame.origin.x - 5.0).abs() < 1e-4);
        assert!((frame.origin.y - 7.0).abs() < 1e-4);
    }

    #[test]
    fn child_sketch_on_circle_face_uses_center_origin() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(Circle::from_local_center_radius(s0, 10.0, 10.0, 5.0, 0.0));
        let s1 = doc.add_sketch(FaceId::Circle(rkey(0)));
        let frame = sketch_geometry_frame(&doc, s1).unwrap();
        let p = local_to_world(&frame, 2.0, 3.0);
        assert!((p.x - 12.0).abs() < 1e-4);
        assert!((p.y - 13.0).abs() < 1e-4);
    }

    /// #844: a datum plane between the camera and a body doesn't steal the click — the
    /// body's face is what a click on the body means.
    #[test]
    fn a_body_face_beats_a_datum_plane_in_front_of_it() {
        let mut doc = doc_with_extruded_box();
        // A plane parked *nearer the camera* than the box, covering it on screen.
        let mut plane = crate::construction::plane_from_face(0.0, Vec3::ZERO, Vec3::Z);
        plane.origin = Vec3::new(0.0, 0.0, 40.0);
        retain_ground_plane_only(&mut doc);
        doc.construction_planes.insert(plane);
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let eye = Vec3::new(5.0, 5.0, 500.0);

        let face = pick_sketch_face(eframe::egui::pos2(10.0, 10.0), &project, &doc, eye);
        assert!(
            matches!(face, Some(FaceId::ExtrudeCap { .. }) | Some(FaceId::Polygon(_))),
            "expected the body's own face, got {face:?}"
        );

        // Away from the body, the plane is still perfectly pickable.
        let off = pick_sketch_face(eframe::egui::pos2(-40.0, -40.0), &project, &doc, eye);
        assert_eq!(off, Some(FaceId::ConstructionPlane(pkey(1))), "got {off:?}");
    }

    /// #1051: a deleted construction plane is gone — it must not go on being hovered,
    /// selected, or (since the Shape tool anchors through this) catching shapes aimed at
    /// whatever is behind it.
    #[test]
    fn a_deleted_construction_plane_is_not_pickable() {
        let mut doc = Document::default();
        retain_ground_plane_only(&mut doc);
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let eye = Vec3::new(0.0, 0.0, 100.0);
        let at = eframe::egui::pos2(20.0, 20.0);

        assert_eq!(
            pick_sketch_face(at, &project, &doc, eye),
            Some(FaceId::ConstructionPlane(pkey(0))),
            "the live plane is pickable to begin with"
        );
        assert!(
            sketch_faces_near(at, &project, &doc, 40.0)
                .iter()
                .any(|(face, ..)| *face == FaceId::ConstructionPlane(pkey(0))),
            "and reaches the crowd"
        );

        let ground = doc.ground_plane().unwrap();
        doc.construction_planes.remove(ground);
        assert_eq!(
            pick_sketch_face(at, &project, &doc, eye),
            None,
            "a removed plane is not pickable"
        );
        assert!(
            !sketch_faces_near(at, &project, &doc, 40.0)
                .iter()
                .any(|(face, ..)| *face == FaceId::ConstructionPlane(pkey(0))),
            "nor does it reach the crowd"
        );
    }

    #[test]
    fn pick_sketch_face_finds_circle_interior() {
        let mut doc = Document::default();
        // Only the ground plane matters here; the other two datum planes (#833) project
        // edge-on under this flattening test projection and would tie for the click.
        retain_ground_plane_only(&mut doc);
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 20.0, 0.0));
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let face = pick_sketch_face(eframe::egui::pos2(5.0, 0.0), &project, &doc, Vec3::new(0.0, 0.0, 100.0));
        assert_eq!(face, Some(FaceId::Circle(rkey(0))));
    }

    #[test]
    fn sketch_camera_circle_face_includes_face_and_children() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(Circle::from_local_center_radius(s0, 0.0, 0.0, 20.0, 0.0));
        let s1 = doc.add_sketch(FaceId::Circle(rkey(0)));
        doc.lines
            .insert(Line::from_local_endpoints(s1, -5.0, -5.0, 5.0, 5.0));
        let target = sketch_camera_target(&doc, s1).unwrap();
        let zoom = target.zoom.unwrap();
        assert!(zoom.half_u >= 5.0);
        assert!(zoom.half_v >= 5.0);
    }

    fn doc_with_extruded_box() -> Document {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let rect_lines =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 20.0, 20.0, [false; 4]);
        doc.extrusions.insert(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 10.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        doc
    }

    fn doc_with_imported_box() -> Document {
        // A unit-scaled 10x10x10 box as an imported-mesh body (#144), so `pick_body_face` has a
        // real body with coplanar faces to resolve without needing the extrusion kernel.
        let c = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(10.0, 10.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::new(10.0, 0.0, 10.0),
            Vec3::new(10.0, 10.0, 10.0),
            Vec3::new(0.0, 10.0, 10.0),
        ];
        let quad = |a: usize, b: usize, d: usize, e: usize| {
            vec![[c[a], c[b], c[d]], [c[a], c[d], c[e]]]
        };
        let mut triangles = Vec::new();
        for face in [
            quad(0, 1, 2, 3),
            quad(4, 5, 6, 7),
            quad(0, 1, 5, 4),
            quad(1, 2, 6, 5),
            quad(2, 3, 7, 6),
            quad(3, 0, 4, 7),
        ] {
            triangles.extend(face);
        }
        let mut doc = Document::default();
        let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles,
            source_name: "box".to_string(),
                    step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(mesh),
            material: None,
            name: None,
            shadow: false,
        });
        doc
    }

    /// A closed box's triangles, for tests that need a face to aim at.
    fn box_triangles(origin: Vec3, size: Vec3) -> Vec<[Vec3; 3]> {
        let (a, b) = (origin, origin + size);
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let quad = |p0, p1, p2, p3| vec![[p0, p1, p2], [p0, p2, p3]];
        let mut t = Vec::new();
        t.extend(quad(v(a.x, a.y, a.z), v(a.x, b.y, a.z), v(b.x, b.y, a.z), v(b.x, a.y, a.z)));
        t.extend(quad(v(a.x, a.y, b.z), v(b.x, a.y, b.z), v(b.x, b.y, b.z), v(a.x, b.y, b.z)));
        t.extend(quad(v(a.x, a.y, a.z), v(b.x, a.y, a.z), v(b.x, a.y, b.z), v(a.x, a.y, b.z)));
        t.extend(quad(v(a.x, b.y, a.z), v(a.x, b.y, b.z), v(b.x, b.y, b.z), v(b.x, b.y, a.z)));
        t.extend(quad(v(a.x, a.y, a.z), v(a.x, a.y, b.z), v(a.x, b.y, b.z), v(a.x, b.y, a.z)));
        t.extend(quad(v(b.x, a.y, a.z), v(b.x, b.y, a.z), v(b.x, b.y, b.z), v(b.x, a.y, b.z)));
        t
    }

    /// #1026: hover picking must not touch a body's triangles when the cursor is nowhere
    /// near it. This runs every frame the camera moves, so its cost has to scale with the
    /// number of *bodies*, not the number of triangles in the document.
    #[test]
    fn a_far_cursor_rejects_bodies_before_their_triangles() {
        // Twenty finely faceted cylinders spread across the ground.
        let mut doc = Document::default();
        for i in 0..20 {
            let (x, y) = ((i % 5) as f32 * 40.0, (i / 5) as f32 * 40.0);
            let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
                triangles: crate::extrude::tests_tube(glam::Vec3::new(x, y, 0.0), 8.0, 10.0),
                source_name: format!("part{i}"),
                step_bytes: None,
            });
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Imported(mesh),
                name: None,
                material: None,
                shadow: false,
            });
        }
        // Plus one solid box, whose face is unambiguous to aim at.
        let box_mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
            triangles: box_triangles(Vec3::new(300.0, 300.0, 0.0), Vec3::splat(20.0)),
            source_name: "box".to_string(),
            step_bytes: None,
        });
        doc.bodies.insert(crate::model::Body {
            source: crate::model::BodySource::Imported(box_mesh),
            name: None,
            material: None,
            shadow: false,
        });
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let eye = Vec3::new(0.0, 0.0, 500.0);

        // Rejecting on bounds must not reject a real hit: the box's middle still picks it.
        assert!(
            pick_body_face(eframe::egui::pos2(310.0, 310.0), &project, &doc, eye).is_some(),
            "a cursor over a body still picks it"
        );

        // Far off in empty space: nothing, and — the reason for the change — cheaply.
        let far = eframe::egui::pos2(5000.0, 5000.0);
        assert!(
            pick_body_face(far, &project, &doc, eye).is_none(),
            "a cursor in empty space picks nothing"
        );
        let started = crate::time::Instant::now();
        for _ in 0..200 {
            let _ = pick_body_face(far, &project, &doc, eye);
        }
        let each = started.elapsed() / 200;
        // Generous by three orders of magnitude against a debug build on a slow machine: the
        // assertion is "this rejects instead of walking the mesh", not a benchmark.
        assert!(
            each < std::time::Duration::from_millis(2),
            "a far-away pick should reject on bounds, took {each:?} per call"
        );
    }

    #[test]
    fn pick_body_face_prefers_the_camera_facing_face() {
        // Top-down projection: the top (z=10) and bottom (z=0) faces both project onto the same
        // square, so the cursor at the center is inside both. The visible top face must win.
        let doc = doc_with_imported_box();
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let kind = pick_body_face(
            eframe::egui::pos2(5.0, 5.0),
            &project,
            &doc,
            Vec3::new(5.0, 5.0, 100.0),
        )
        .expect("cursor over the box should pick a face");
        match kind {
            crate::construction::PickTargetKind::BodyFace { triangles, .. } => {
                assert!(
                    triangles.iter().flatten().all(|p| (p.z - 10.0).abs() < 1e-4),
                    "should pick the near top face (z=10), got {triangles:?}"
                );
            }
            other => panic!("expected a body face, got {other:?}"),
        }
    }

    #[test]
    fn pick_body_face_where_skips_a_front_body() {
        // Two stacked boxes: the front one covers the back. Skipping the front body
        // inside the search is what lets a Move destination pick the face behind it (#1336).
        let mut doc = Document::default();
        let insert = |doc: &mut Document, origin: Vec3, name: &str| {
            let mesh = doc.imported_meshes.insert(crate::model::ImportedMesh {
                triangles: box_triangles(origin, Vec3::new(10.0, 10.0, 10.0)),
                source_name: name.to_string(),
                step_bytes: None,
            });
            doc.bodies.insert(crate::model::Body {
                source: crate::model::BodySource::Imported(mesh),
                material: None,
                name: None,
                shadow: false,
            })
        };
        let back = insert(&mut doc, Vec3::ZERO, "back");
        let front = insert(&mut doc, Vec3::new(0.0, 0.0, 20.0), "front");
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let eye = Vec3::new(5.0, 5.0, 100.0);
        let cursor = eframe::egui::pos2(5.0, 5.0);
        let all = pick_body_face(cursor, &project, &doc, eye).expect("a face");
        assert!(
            matches!(all, crate::construction::PickTargetKind::BodyFace { body, .. } if body == front),
            "the uncovered pick takes the front body, got {all:?}"
        );
        let through = pick_body_face_where(cursor, &project, &doc, eye, |bi| bi != front)
            .expect("the back face");
        assert!(
            matches!(through, crate::construction::PickTargetKind::BodyFace { body, .. } if body == back),
            "skipping the front body must take the one behind it, got {through:?}"
        );
    }

    #[test]
    fn pick_body_face_misses_outside_the_body() {
        let doc = doc_with_imported_box();
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        assert!(pick_body_face(
            eframe::egui::pos2(99.0, 99.0),
            &project,
            &doc,
            Vec3::new(5.0, 5.0, 100.0),
        )
        .is_none());
    }

    #[test]
    fn pick_sketch_face_finds_extrusion_cap() {
        let mut doc = doc_with_extruded_box();
        retain_ground_plane_only(&mut doc);
        // Offset screen x by height so the top cap (z=10) separates from the base
        // rect; click where only the lifted top cap projects.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x + p.z, p.y));
        let face = pick_sketch_face(eframe::egui::pos2(25.0, 10.0), &project, &doc, Vec3::new(0.0, 0.0, 100.0));
        assert!(
            matches!(
                face,
                Some(FaceId::ExtrudeCap {
                    extrusion: _,
                    top: true,
                    ..
                })
            ),
            "clicking the lifted top cap should pick it, got {face:?}"
        );
    }

    #[test]
    fn pick_prefers_the_camera_facing_cap_not_the_hidden_one() {
        // Top-down orthographic projection: both the top cap (z=10) and the bottom
        // cap (z=0) of the box project onto the same screen rectangle, so the cursor
        // at the center is inside both. The visible (camera-facing) cap must win.
        let doc = doc_with_extruded_box();
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let cursor = eframe::egui::pos2(10.0, 10.0);

        // Eye above the box: the near top cap must be picked, never the hidden
        // bottom cap (z=0) which faces away from the camera.
        let from_above = pick_sketch_face(cursor, &project, &doc, Vec3::new(10.0, 10.0, 100.0));
        assert!(
            matches!(from_above, Some(FaceId::ExtrudeCap { top: true, .. })),
            "looking down should pick the visible top cap, got {from_above:?}"
        );
    }

    #[test]
    fn circular_profiles_have_no_flat_side_walls() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        doc.circles
            .insert(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.extrusions.insert(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Circle(rkey(0))],
            distance: 8.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
        let profile = crate::model::ExtrudeFace::Circle(rkey(0));
        assert_eq!(crate::extrude::side_face_count(&profile), 0);
        assert!(crate::extrude::side_quad_world(&doc, xkey(0), &profile, 0).is_none());
    }

    #[test]
    fn pick_sketch_face_finds_extrusion_side_wall() {
        let mut doc = doc_with_extruded_box();
        retain_ground_plane_only(&mut doc);
        // Project to the XZ plane so the y=0 side wall shows as a 20x10 rectangle.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.z));
        let face = pick_sketch_face(eframe::egui::pos2(10.0, 5.0), &project, &doc, Vec3::new(0.0, 0.0, 100.0));
        assert!(
            matches!(face, Some(FaceId::ExtrudeSide { extrusion, .. }) if extrusion == xkey(0)),
            "clicking a side wall should pick it, got {face:?}"
        );
    }

    /// #1173: looking into a hollowed cuboid at the inner back wall must pick that wall, not
    /// the outer face of the same wall buried a thickness behind it along the pick ray.
    #[test]
    fn pick_sketch_face_prefers_shell_inner_wall_over_outer_behind_it() {
        use crate::model::{
            Body, BodySource, Primitive, PrimitiveFace, PrimitiveKind, ShellOperation,
        };
        // Real shell via OCCT — 40³ cuboid, open +Y side and top, 4 mm walls.
        // Outer back wall at y = −20 (normal −Y); inner back wall at y = −16 (normal +Y).
        let mut doc = Document::default();
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "40".to_string();
        shape.depth = "40".to_string();
        shape.height = "40".to_string();
        let pi = doc.primitives.insert(shape);
        let input = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: true,
        });
        let open_top = FaceId::PrimitiveFace {
            primitive: pi,
            face: PrimitiveFace::CuboidTop,
        };
        let open_side = FaceId::PrimitiveFace {
            primitive: pi,
            face: PrimitiveFace::CuboidSide { edge: 2 },
        };
        let op = doc.shell_ops.insert(ShellOperation {
            targets: vec![input],
            open_faces: vec![open_top, open_side],
            thickness: "4".to_string(),
            outputs: Vec::new(),
            name: None,
        });
        let out = doc.bodies.insert(Body {
            source: BodySource::Shelled {
                op,
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.shell_ops[op].outputs = vec![out];
        // Kernel must produce the hollow mesh for body face groups.
        assert!(
            crate::extrude::body_solid_mesh(&doc, out).is_some_and(|m| !m.triangles.is_empty()),
            "shelled body must tessellate"
        );

        // Looking along −Y into the open side: screen is XZ.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.z));
        let eye = Vec3::new(0.0, 100.0, 20.0);
        // Cursor at the middle of the back wall.
        let hit = pick_sketch_face(eframe::egui::pos2(0.0, 20.0), &project, &doc, eye)
            .expect("inner back wall under cursor should pick a face");
        let outer_back = FaceId::PrimitiveFace {
            primitive: pi,
            face: PrimitiveFace::CuboidSide { edge: 0 },
        };
        assert_ne!(
            hit, outer_back,
            "must not pick the outer back face behind the cavity wall"
        );
        // The front surface is the mesh inner wall: either BodyMeshFace near y = −16, or at
        // least a face whose frame sits closer to the eye than the outer wall.
        let frame = sketch_frame(&doc, hit.clone()).expect("picked face has a frame");
        let outer_frame = sketch_frame(&doc, outer_back).expect("outer face frame");
        let hit_depth = (frame.origin - eye).length();
        let outer_depth = (outer_frame.origin - eye).length();
        assert!(
            hit_depth < outer_depth - 1.0,
            "picked face origin {frame:?} should be closer to the eye than outer {outer_frame:?} \
             (depths {hit_depth} vs {outer_depth})"
        );
        match hit {
            FaceId::BodyMeshFace { body, .. } => {
                assert_eq!(body, out, "mesh face belongs to the shelled body");
            }
            other => {
                // Analytic is fine only if it is not the outer wall (e.g. a future inner id).
                assert!(
                    !matches!(
                        other,
                        FaceId::PrimitiveFace {
                            face: PrimitiveFace::CuboidSide { edge: 0 },
                            ..
                        }
                    ),
                    "unexpected pick {other:?}"
                );
            }
        }
    }

    /// #1103: a cuboid drawn by the Shape tool (a primitive, no sketch behind it) has the
    /// #1165: faces removed by a committed shell (open faces) must not win sketch-face hover
    /// or pick — the hole is gone, so the original analytic face is not a valid target.
    #[test]
    fn pick_sketch_face_skips_shell_open_faces() {
        use crate::model::{
            Body, BodySource, Primitive, PrimitiveFace, PrimitiveKind, ShellOperation,
        };
        let mut doc = Document::default();
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "20".to_string();
        shape.depth = "20".to_string();
        shape.height = "10".to_string();
        let pi = doc.primitives.insert(shape);
        let input = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: true, // consumed by the shell
        });
        let open = FaceId::PrimitiveFace {
            primitive: pi,
            face: PrimitiveFace::CuboidTop,
        };
        let op = doc.shell_ops.insert(ShellOperation {
            targets: vec![input],
            open_faces: vec![open.clone()],
            thickness: "1".to_string(),
            outputs: Vec::new(),
            name: None,
        });
        let out = doc.bodies.insert(Body {
            source: BodySource::Shelled {
                op,
                target: 0,
                add: Vec::new(),
                cut: Vec::new(),
            },
            material: None,
            name: None,
            shadow: false,
        });
        doc.shell_ops[op].outputs = vec![out];

        // Top-down over the (removed) top face — must not resolve to that open face.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let hit = pick_sketch_face(
            eframe::egui::pos2(0.0, 0.0),
            &project,
            &doc,
            Vec3::new(0.0, 0.0, 100.0),
        );
        assert!(
            hit.as_ref() != Some(&open),
            "open face of a shell must not be pickable, got {hit:?}"
        );
        // A remaining face (the bottom) is still sketchable.
        let bottom = FaceId::PrimitiveFace {
            primitive: pi,
            face: PrimitiveFace::CuboidBottom,
        };
        // Eye below the box looking up — bottom face is nearest.
        let from_below = pick_sketch_face(
            eframe::egui::pos2(0.0, 0.0),
            &project,
            &doc,
            Vec3::new(0.0, 0.0, -100.0),
        );
        assert_eq!(
            from_below,
            Some(bottom),
            "remaining non-open faces stay pickable, got {from_below:?}"
        );
    }

    /// same sketchable analytic faces as an extruded box — its top cap and side walls can
    /// be picked to sketch on, just like an extrusion's.
    #[test]
    fn pick_sketch_face_finds_primitive_cuboid_faces() {
        use crate::model::{Body, BodySource, Primitive, PrimitiveKind};
        let mut doc = Document::default();
        // A 20x20x10 cuboid resting on the ground at the world origin: base at z=0,
        // top at z=10, spanning x,y in [-10,10].
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "20".to_string();
        shape.depth = "20".to_string();
        shape.height = "10".to_string();
        let pi = doc.primitives.insert(shape);
        doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        // Top-down: the top cap (z=10) projects to [-10,10]² on screen.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let top = pick_sketch_face(
            eframe::egui::pos2(0.0, 0.0),
            &project,
            &doc,
            Vec3::new(0.0, 0.0, 100.0),
        );
        assert!(
            matches!(top, Some(FaceId::PrimitiveFace { primitive, face })
                if primitive == pi
                    && matches!(face, crate::model::PrimitiveFace::CuboidTop)),
            "clicking the top of a primitive cuboid should pick it, got {top:?}"
        );
        // A sketch on that face resolves a frame on the top cap's plane (z=10, normal +Z).
        let frame = sketch_frame(&doc, top.unwrap()).expect("primitive top face has a frame");
        assert!((frame.normal - Vec3::Z).length() < 1e-4, "top cap faces +Z");
        assert!((frame.origin.z - 10.0).abs() < 1e-3, "top cap sits at z=10, got {}", frame.origin);
        // The side wall: project to XZ so the y=-10 wall shows as a 20x10 rectangle.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.z));
        let side = pick_sketch_face(
            eframe::egui::pos2(0.0, 5.0),
            &project,
            &doc,
            Vec3::new(0.0, -100.0, 0.0),
        );
        assert!(
            matches!(side, Some(FaceId::PrimitiveFace { primitive, face: crate::model::PrimitiveFace::CuboidSide { .. } })
                if primitive == pi),
            "clicking a side wall should pick it, got {side:?}"
        );
    }

    /// #1119: a Shape-tool cuboid that has been linearly repeated exposes each copy's
    /// flat faces as `RepeatedFace` so revolve/extrude can take them, not just the original.
    #[test]
    fn pick_sketch_face_finds_repeated_primitive_cuboid_faces() {
        use crate::model::{
            Body, BodySource, Primitive, PrimitiveKind, RepeatMode, RepeatOperation, RevolveAxis,
        };
        let mut doc = Document::default();
        let mut shape = Primitive::new(PrimitiveKind::Cuboid);
        shape.width = "20".to_string();
        shape.depth = "20".to_string();
        shape.height = "10".to_string();
        let pi = doc.primitives.insert(shape);
        let body = doc.bodies.insert(Body {
            source: BodySource::Primitive(pi),
            material: None,
            name: None,
            shadow: false,
        });
        // Count=2, gap=40 along +X: instance 1 steps by extent+gap. Cuboid width 20 → step 60;
        // top face centre lands near x=60 (original top is at origin).
        let op = doc.repeat_ops.insert(RepeatOperation {
            targets: vec![body],
            plane_targets: Vec::new(),
            extrusion_targets: Vec::new(),
            sketch_targets: Vec::new(),
            sketch_plane_outputs: Vec::new(),
            sketch_outputs: Vec::new(),
            axis: RevolveAxis::X,
            path_circle: None,
            around_axis: false,
            flip: false,
            mode: RepeatMode::CountGap,
            count: "2".to_string(),
            spacing: "40".to_string(),
            length: String::new(),
            length_target: None,
            outputs: Vec::new(),
            plane_outputs: Vec::new(),
            name: None,
        });
        let offsets = crate::extrude::repeat_offsets(&doc, doc.repeat_ops.get(op).unwrap())
            .expect("repeat offsets");
        assert!(!offsets.is_empty(), "expected at least one copy offset");
        let copy_x = offsets[0];
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let hit = pick_sketch_face(
            eframe::egui::pos2(copy_x, 0.0),
            &project,
            &doc,
            Vec3::new(copy_x, 0.0, 100.0),
        );
        assert!(
            matches!(
                &hit,
                Some(FaceId::RepeatedFace {
                    face,
                    op: hit_op,
                    instance: 1,
                }) if *hit_op == op
                    && matches!(
                        face.as_ref(),
                        FaceId::PrimitiveFace {
                            face: crate::model::PrimitiveFace::CuboidTop,
                            ..
                        }
                    )
            ),
            "clicking a repeated cuboid's top should pick RepeatedFace, got {hit:?}"
        );
    }

    /// Interaction `revolve_axis_click` / CI: under a true top view (#1183) the geometric
    /// center of a ground rectangle must pick the polygon profile, not a construction plane.
    #[test]
    fn pick_sketch_face_finds_rectangle_center_under_true_top_view() {
        // Full default document (all three datum planes), matching the live app.
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 20.0, 10.0, [false; 4]);

        let mut cam = crate::camera::Camera::default();
        let (yaw, pitch) = crate::camera::StandardView::Top.yaw_pitch();
        cam.yaw = yaw;
        cam.pitch = pitch;
        cam.target = glam::Vec3::new(10.0, -2.5, 0.0);
        cam.distance = 38.347;
        let viewport = eframe::egui::Rect::from_min_size(
            eframe::egui::pos2(0.0, 0.0),
            eframe::egui::vec2(1280.0, 800.0),
        );
        let vp = cam.view_proj(viewport);
        let project = |p: Vec3| cam.project(p, viewport, &vp);
        let eye = cam.eye();

        let center = project(glam::Vec3::new(10.0, 5.0, 0.0)).expect("center projects");
        let face = pick_sketch_face(center, &project, &doc, eye);
        assert!(
            matches!(face, Some(FaceId::Polygon(_))),
            "center of a ground rectangle under true top view must pick the profile, got {face:?}"
        );

        let near = project(glam::Vec3::new(5.0, 2.5, 0.0)).expect("near-center projects");
        let face_near = pick_sketch_face(near, &project, &doc, eye);
        assert!(
            matches!(face_near, Some(FaceId::Polygon(_))),
            "near-center should pick the profile, got {face_near:?}"
        );
    }

    #[test]
    fn pick_prefers_a_sketch_profile_over_the_solid_face_it_sits_on() {
        // #117: drawing a rectangle on a solid's face and then trying to hover/extrude it
        // silently failed. Root cause: a sketch profile coincident with its host face ties
        // on screen distance (both "inside" at the click), and the old depth tie-break
        // compared each shape's own centroid distance to the eye — which diverges from the
        // wall's centroid whenever the sketch isn't centered on its host face, letting the
        // (unextrudable) bare face win the pick outright.
        let mut doc = doc_with_extruded_box();
        let profile = crate::model::ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]);
        let host = FaceId::ExtrudeSide {
            extrusion: xkey(0),
            profile: profile.clone(),
            edge: 0,
        };
        let wall = crate::extrude::side_quad_world(&doc, xkey(0), &profile, 0).expect("wall face exists");
        let (a, b, d) = (wall[0], wall[1], wall[3]);
        let world_pt = |s: f32, t: f32| a + (b - a) * s + (d - a) * t;

        // A small sketch tucked in one corner of the wall (s,t in [0.05, 0.25]) — off-center
        // from the wall's own centroid (s = t = 0.5).
        let child_sketch = doc.add_sketch(host);
        let frame = sketch_geometry_frame(&doc, child_sketch).expect("frame for child sketch");
        let (u0, v0) = world_to_local(&frame, world_pt(0.05, 0.05));
        let (u1, v1) = world_to_local(&frame, world_pt(0.25, 0.25));
        let child_lines = crate::construction::add_line_rectangle(
            &mut doc,
            child_sketch,
            u0.min(u1),
            v0.min(v1),
            (u1 - u0).abs(),
            (v1 - v0).abs(),
            [false; 4],
        )
        .to_vec();

        // Project world -> screen by dropping y (the wall's constant coordinate) so both
        // the wall and the child sketch project into a consistent 2D layout.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.z));
        let click = project(world_pt(0.15, 0.15)).unwrap();

        // Eye near the wall's own centroid, not the sketch's corner: this is what made the
        // bare wall look "closer" than the sketch under the old centroid-depth compare.
        let eye = world_pt(0.5, 0.5) + Vec3::new(0.0, -100.0, 0.0);

        let face = pick_sketch_face(click, &project, &doc, eye);
        assert_eq!(
            face,
            Some(FaceId::Polygon(child_lines)),
            "clicking a sketch drawn on a solid's face must pick the sketch, not the bare face, got {face:?}"
        );
    }

    /// Push `vertices` as a closed loop of lines into `sketch` (with the coincident
    /// constraints that make it a recognized loop), returning the line keys.
    fn add_line_loop(
        doc: &mut Document,
        sketch: SketchId,
        vertices: &[(f32, f32)],
    ) -> Vec<crate::model::LineKey> {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, LineEnd};
        let n = vertices.len();
        let keys: Vec<crate::model::LineKey> = (0..n)
            .map(|i| {
                let (u0, v0) = vertices[i];
                let (u1, v1) = vertices[(i + 1) % n];
                doc.lines.insert(Line::from_local_endpoints(sketch, u0, v0, u1, v1))
            })
            .collect();
        for i in 0..n {
            doc.constraints.insert(Constraint {
                sketch,
                kind: ConstraintKind::Coincident {
                    a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                        line: keys[i],
                        end: LineEnd::End,
                    }),
                    b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                        line: keys[(i + 1) % n],
                        end: LineEnd::Start,
                    }),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
            });
        }
        keys
    }

    fn extrude_loop(doc: &mut Document, sketch: SketchId, lines: Vec<crate::model::LineKey>) {
        doc.extrusions.insert(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(lines)],
            distance: 10.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            taper: 0.0,
            taper_mode: crate::model::ExtrudeTaperMode::Distance,
            taper_expression: String::new(),
            edge_treatments: Vec::new(),
        });
    }

    /// Every side-wall frame's normal must point out of the solid and its (u, v, normal)
    /// triad must be right-handed — checked by mapping the frame's outward offset back to
    /// the profile plane and asserting it lands *outside* the profile polygon.
    fn assert_side_frames_outward(
        doc: &Document,
        vertices: &[(f32, f32)],
        lines: &[crate::model::LineKey],
    ) {
        let profile = crate::model::ExtrudeFace::Polygon(lines.to_vec());
        for edge in 0..vertices.len() {
            let frame = sketch_frame(
                doc,
                FaceId::ExtrudeSide {
                    extrusion: xkey(0),
                    profile: profile.clone(),
                    edge: edge as u8,
                },
            )
            .unwrap_or_else(|| panic!("frame for edge {edge}"));
            // Right-handed frame: u × v == normal.
            assert!(
                frame.u_axis.cross(frame.v_axis).dot(frame.normal) > 0.99,
                "edge {edge}: (u, v, normal) must stay right-handed"
            );
            // Outward: nudging the wall midpoint along the normal exits the profile.
            let (u0, v0) = vertices[edge];
            let (u1, v1) = vertices[(edge + 1) % vertices.len()];
            let mid = glam::Vec2::new((u0 + u1) * 0.5, (v0 + v1) * 0.5);
            let plane = sketch_frame(doc, FaceId::ConstructionPlane(pkey(0))).unwrap();
            let world_mid = local_to_world(&plane, mid.x, mid.y) + frame.normal * 0.1;
            let (pu, pv) = world_to_local(&plane, world_mid);
            assert!(
                !point_in_polygon_2d((pu, pv), vertices),
                "edge {edge}: normal {:?} points into the profile interior",
                frame.normal
            );
        }
    }

    fn point_in_polygon_2d(p: (f32, f32), vertices: &[(f32, f32)]) -> bool {
        let mut inside = false;
        for i in 0..vertices.len() {
            let a = vertices[i];
            let b = vertices[(i + 1) % vertices.len()];
            if (a.1 > p.1) != (b.1 > p.1)
                && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0
            {
                inside = !inside;
            }
        }
        inside
    }

    /// #362: on a non-convex (L-shaped) profile the old centroid heuristic flipped the
    /// frame of the inner walls (the two edges flanking the concave corner) inward,
    /// making the frame left-handed seen from outside — sketch text on those walls
    /// rendered mirrored. The winding-derived normal must point outward on every wall.
    #[test]
    fn concave_side_walls_get_outward_right_handed_frames() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        // CCW L-profile; the concave corner is at (10, 10).
        let vertices = [
            (0.0, 0.0),
            (30.0, 0.0),
            (30.0, 10.0),
            (10.0, 10.0),
            (10.0, 30.0),
            (0.0, 30.0),
        ];
        let lines = add_line_loop(&mut doc, sketch, &vertices);
        extrude_loop(&mut doc, sketch, lines.clone());
        assert_side_frames_outward(&doc, &vertices, &lines);

        // The two inner walls specifically: edge 2 (y = 10, material below) faces +Y and
        // edge 3 (x = 10, material to the left) faces +X.
        let profile = crate::model::ExtrudeFace::Polygon(lines);
        let f2 = sketch_frame(
            &doc,
            FaceId::ExtrudeSide { extrusion: xkey(0), profile: profile.clone(), edge: 2 },
        )
        .unwrap();
        assert!(f2.normal.dot(Vec3::Y) > 0.99, "edge 2 outward is +Y, got {:?}", f2.normal);
        let f3 = sketch_frame(
            &doc,
            FaceId::ExtrudeSide { extrusion: xkey(0), profile, edge: 3 },
        )
        .unwrap();
        assert!(f3.normal.dot(Vec3::X) > 0.99, "edge 3 outward is +X, got {:?}", f3.normal);
    }

    /// A clockwise-wound profile must get the same outward walls as a CCW one — the
    /// winding sign feeds the normal derivation, not the result.
    #[test]
    fn clockwise_profiles_still_get_outward_side_frames() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let vertices = [(0.0, 0.0), (0.0, 20.0), (20.0, 20.0), (20.0, 0.0)];
        let lines = add_line_loop(&mut doc, sketch, &vertices);
        extrude_loop(&mut doc, sketch, lines.clone());
        assert_side_frames_outward(&doc, &vertices, &lines);
    }

    /// Convex profiles keep the frames they always had (the box case): outward normals,
    /// v-axis along the extrusion.
    #[test]
    fn convex_side_frames_unchanged_by_winding_derivation() {
        let doc = doc_with_extruded_box();
        let profile = crate::model::ExtrudeFace::Polygon(vec![lkey(0), lkey(1), lkey(2), lkey(3)]);
        let expected = [-Vec3::Y, Vec3::X, Vec3::Y, -Vec3::X];
        for edge in 0..4u8 {
            let frame = sketch_frame(
                &doc,
                FaceId::ExtrudeSide {
                    extrusion: xkey(0),
                    profile: profile.clone(),
                    edge,
                },
            )
            .unwrap();
            assert!(
                frame.normal.dot(expected[edge as usize]) > 0.99,
                "edge {edge}: expected {:?}, got {:?}",
                expected[edge as usize],
                frame.normal
            );
            assert!(
                frame.v_axis.dot(Vec3::Z) > 0.99,
                "edge {edge}: v-axis should run up the extrusion, got {:?}",
                frame.v_axis
            );
        }
    }

    #[test]
    fn has_children_detects_dependents() {
        let mut doc = Document::default();
        assert!(!doc.has_children(&FaceId::ConstructionPlane(pkey(0))));
        doc.sketches.insert(Sketch {
            face: FaceId::ConstructionPlane(pkey(0)),
            name: None,
            length_unit: None,
            angle_unit: None,
        });
        assert!(doc.has_children(&FaceId::ConstructionPlane(pkey(0))));
    }

    #[test]
    fn sketch_camera_empty_plane_orients_without_zoom() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(0)));
        let target = sketch_camera_target(&doc, sketch).unwrap();
        assert!(target.zoom.is_none());
        assert!(target.target.length_squared() < 1e-8);
        assert!((target.face_normal.z - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sketch_view_up_from_isometric_takes_minimal_roll() {
        use crate::camera::Camera;

        // Entering the ground (XY) sketch from the default isometric view keeps the plane's u/v
        // axes screen-aligned but takes the **shortest** roll (#577) — it no longer spins around to
        // force the u-right/v-up convention. The chosen orientation must roll no more than that
        // convention pick would have.
        let cam = Camera::default();
        let frame = SketchFrame {
            origin: Vec3::ZERO,
            u_axis: Vec3::X,
            v_axis: Vec3::Y,
            normal: Vec3::Z,
        };
        let view_dir = cam.visible_face_view_direction(Vec3::ZERO, Vec3::Z);
        let current_look = (Vec3::ZERO - cam.eye()).normalize_or_zero();
        let current_up = cam.view_up_hint();
        let target_look = (-view_dir).normalize_or_zero();

        let hint = sketch_view_up(view_dir, &frame, current_look, current_up);

        // The axes stay screen-aligned: u and v each land on a screen axis and stay perpendicular.
        let u_screen = axis_screen_vec(frame.u_axis, target_look, hint);
        let v_screen = axis_screen_vec(frame.v_axis, target_look, hint);
        let axis_aligned = |s: glam::Vec2| s.x.abs() < 0.05 || s.y.abs() < 0.05;
        assert!(axis_aligned(u_screen) && axis_aligned(v_screen), "u={u_screen:?} v={v_screen:?}");
        assert_ne!(
            u_screen.x.abs() > u_screen.y.abs(),
            v_screen.x.abs() > v_screen.y.abs(),
            "u and v must lie on different screen axes"
        );

        // The pick rolls no more than forcing the +Y-up convention would have.
        let u_before = axis_screen_vec(frame.u_axis, current_look, current_up);
        let v_before = axis_screen_vec(frame.v_axis, current_look, current_up);
        let roll = |h: Vec3| {
            let ua = axis_screen_vec(frame.u_axis, target_look, h);
            let va = axis_screen_vec(frame.v_axis, target_look, h);
            (ua - u_before).length_squared() + (va - v_before).length_squared()
        };
        assert!(
            roll(hint) <= roll(Vec3::Y) + 1e-4,
            "minimal-roll pick must not roll more than the convention pick"
        );
    }

    #[test]
    fn sketch_view_up_prefers_minimal_roll_flip() {
        let frame = SketchFrame {
            origin: Vec3::ZERO,
            u_axis: Vec3::X,
            v_axis: Vec3::Y,
            normal: Vec3::Z,
        };
        let hint = sketch_view_up(Vec3::Z, &frame, -Vec3::Z, Vec3::Y);
        assert!(
            hint.dot(Vec3::Y) > 0.0,
            "already aligned with +Y should keep +Y hint, got {hint:?}"
        );
    }

    #[test]
    fn sketch_view_up_on_vertical_wall_keeps_ground_at_the_bottom() {
        // A side wall whose in-plane axes are u along world +X and v along world
        // +Z (a vertical wall facing -Y). Regardless of how the camera was rolled
        // before, the sketch should orient so world up (+Z, our v axis) points up
        // on screen, putting the ground at the bottom.
        let frame = SketchFrame {
            origin: Vec3::ZERO,
            u_axis: Vec3::X,
            v_axis: Vec3::Z,
            normal: -Vec3::Y,
        };
        // view_direction points from the face toward the eye (outward normal, -Y).
        let view_direction = -Vec3::Y;
        // Start from a rolled-sideways view (current up pointing along +X).
        let hint = sketch_view_up(view_direction, &frame, Vec3::Y, Vec3::X);
        assert!(
            hint.dot(Vec3::Z) > 0.9,
            "vertical wall sketch should orient world +Z up, got {hint:?}"
        );
    }

    #[test]
    fn sketch_view_up_aligns_plane_axes_with_screen() {
        use crate::camera::Camera;
        use crate::construction::{
            definition_from_reference, plane_from_definition, PlaneReference,
        };
        use crate::model::ConstructionPlaneParent;
        use eframe::egui::{Pos2, Rect};

        let mut doc = Document::default();
        doc.construction_planes.insert(plane_from_definition(
            &definition_from_reference(
                &PlaneReference::Axis {
                    origin: Vec3::ZERO,
                    direction: Vec3::X,
                    label: "X axis".to_string(),
                },
                0.0,
                45.0,
            ),
            ConstructionPlaneParent::Root,
        ));
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(pkey(1)));
        let frame = sketch_frame(&doc, FaceId::ConstructionPlane(pkey(1))).unwrap();
        let mut cam = Camera::default();
        cam.target = frame.origin;
        cam.distance = 200.0;
        let view_direction =
            cam.visible_face_view_direction(frame.origin, frame.normal);
        let look_forward = (cam.target - cam.eye()).normalize_or_zero();
        let hint = sketch_view_up(
            view_direction,
            &frame,
            look_forward,
            cam.view_up_hint(),
        );
        cam.set_view_up(Some(hint));
        let (yaw, pitch) = Camera::view_direction_to_yaw_pitch(view_direction);
        cam.yaw = yaw;
        cam.pitch = pitch;

        let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let base = cam.project(frame.origin, viewport, &vp).unwrap();
        let above = cam
            .project(frame.origin + frame.v_axis * 10.0, viewport, &vp)
            .unwrap();
        let right = cam
            .project(frame.origin + frame.u_axis * 10.0, viewport, &vp)
            .unwrap();

        // After the minimal-roll redesign (#577) the axes need not follow the u-right/v-up
        // convention, but they must still be **screen-aligned**: each projects along a screen axis
        // (not diagonal) and the two lie on different axes (stay perpendicular on screen).
        let u_dir = right - base;
        let v_dir = above - base;
        let aligned = |d: egui::Vec2| {
            let len = d.length();
            len > 1e-3 && (d.x.abs() < 0.1 * len || d.y.abs() < 0.1 * len)
        };
        assert!(aligned(u_dir), "u should be screen-aligned, got {u_dir:?}");
        assert!(aligned(v_dir), "v should be screen-aligned, got {v_dir:?}");
        assert_ne!(
            u_dir.x.abs() > u_dir.y.abs(),
            v_dir.x.abs() > v_dir.y.abs(),
            "u and v must lie on different screen axes"
        );
        let _ = sketch;
    }



    /// #1219: analytic faces of a shadow body must not win sketch-face picks over the
    /// live cut pieces that replaced it.
    #[test]
    fn issue_1219_shadow_primitive_face_is_not_sketchable() {
        use crate::camera::Camera;
        use eframe::egui::{Pos2, Rect};

        let bytes = include_bytes!("../tests/fixtures/issue_1219.json");
        let doc = crate::storage::from_json_bytes(bytes).expect("load");
        // Body 0 is the pure cuboid primitive and is a shadow (consumed by the solid).
        assert!(doc.bodies.values().nth(0).unwrap().shadow);
        let b5 = doc.bodies.keys().nth(5).unwrap();
        let mesh5 = crate::extrude::body_solid_mesh(&doc, b5).expect("body 5 mesh");
        let (min, max) = mesh5.bounds().unwrap();
        let center = (min + max) * 0.5;

        let mut cam = Camera::default();
        cam.target = center;
        cam.distance = 400.0;
        cam.yaw = 0.3;
        cam.pitch = -1.1;
        let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);

        // Sample a triangle of body 5 that currently resolves as the shadow cuboid's analytic face.
        let mut saw_body5 = false;
        for tri in mesh5.triangles.iter().step_by(15) {
            let p = (tri[0] + tri[1] + tri[2]) / 3.0;
            let Some(sp) = project(p) else { continue };
            let Some(face) = pick_sketch_face(sp, &project, &doc, cam.eye()) else {
                continue;
            };
            match face {
                FaceId::PrimitiveFace { primitive, .. } => {
                    let body = crate::model::body_index_for_primitive(&doc, primitive);
                    let shadow = body
                        .and_then(|bi| doc.bodies.get(bi))
                        .is_some_and(|b| b.shadow);
                    assert!(
                        !shadow,
                        "sketch pick must not land on a shadow body's primitive face: {face:?}"
                    );
                }
                FaceId::ExtrudeCap { extrusion, .. } | FaceId::ExtrudeSide { extrusion, .. } => {
                    let body = crate::model::body_index_for_extrusion(&doc, extrusion);
                    let shadow = body
                        .and_then(|bi| doc.bodies.get(bi))
                        .is_some_and(|b| b.shadow);
                    assert!(
                        !shadow,
                        "sketch pick must not land on a shadow extrusion face: {face:?}"
                    );
                }
                FaceId::BodyMeshFace { body, .. } if body == b5 => saw_body5 = true,
                _ => {}
            }
        }
        assert!(
            saw_body5,
            "at least one sample over body 5 should pick a live mesh face of body 5"
        );
    }


    /// #1220: a mesh face's highlight border is the true outline, not the triangulation
    /// visit order (which draws diagonals / crossing lines).
    #[test]
    fn issue_1220_body_mesh_face_boundary_is_true_outline() {
        let bytes = include_bytes!("../tests/fixtures/issue_1219.json");
        let doc = crate::storage::from_json_bytes(bytes).expect("load");
        let quant = |v: glam::Vec3| {
            (
                (v.x * 1000.0).round() as i64,
                (v.y * 1000.0).round() as i64,
                (v.z * 1000.0).round() as i64,
            )
        };
        let mut checked = 0usize;
        for (bi, body) in doc.bodies.iter() {
            if body.shadow {
                continue;
            }
            let Some(mesh) = crate::extrude::body_solid_mesh(&doc, bi) else {
                continue;
            };
            for tris in crate::gpu_viewport::solid_mesh_coplanar_faces(&mesh) {
                if tris.len() < 2 {
                    continue;
                }
                if crate::extrude::fit_cylinder(&tris).is_some() {
                    continue;
                }
                let true_boundary = crate::construction::coplanar_face_boundary(&tris);
                if true_boundary.len() < 3 {
                    continue;
                }
                let loop_pts = crate::construction::coplanar_face_boundary_loop(&tris);
                if loop_pts.len() < 3 {
                    // Degenerate / non-manifold outline — skip; highlight falls back to edges.
                    continue;
                }
                let bset: std::collections::HashSet<_> = true_boundary
                    .iter()
                    .map(|(a, b)| {
                        let (ka, kb) = (quant(*a), quant(*b));
                        if ka <= kb {
                            (ka, kb)
                        } else {
                            (kb, ka)
                        }
                    })
                    .collect();
                let n = loop_pts.len();
                let mut bad = 0usize;
                for i in 0..n {
                    let a = loop_pts[i];
                    let b = loop_pts[(i + 1) % n];
                    let (ka, kb) = (quant(a), quant(b));
                    let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
                    if !bset.contains(&key) {
                        bad += 1;
                    }
                }
                assert_eq!(
                    bad, 0,
                    "body {:?} tris={} boundary={} loop={} has {bad} non-boundary edges",
                    bi,
                    tris.len(),
                    true_boundary.len(),
                    n
                );
                // face_boundary_loop_world must agree (this is what the highlight uses).
                let centroid = crate::extrude::face_group_center(&tris);
                let normal = (tris[0][1] - tris[0][0])
                    .cross(tris[0][2] - tris[0][0])
                    .normalize_or_zero();
                let q = crate::hierarchy::quantize_body_point;
                let face = FaceId::BodyMeshFace {
                    body: bi,
                    centroid: q(centroid),
                    normal: q(normal),
                };
                if let Some(via) = crate::extrude::face_boundary_loop_world(&doc, &face) {
                    assert!(
                        via.len() >= 3,
                        "face_boundary_loop_world returned too-short loop for body {:?}",
                        bi
                    );
                    let mut via_bad = 0usize;
                    for i in 0..via.len() {
                        let a = via[i];
                        let b = via[(i + 1) % via.len()];
                        let (ka, kb) = (quant(a), quant(b));
                        let key = if ka <= kb { (ka, kb) } else { (kb, ka) };
                        if !bset.contains(&key) {
                            via_bad += 1;
                        }
                    }
                    assert_eq!(
                        via_bad, 0,
                        "face_boundary_loop_world has crossing edges on body {:?}",
                        bi
                    );
                }
                checked += 1;
            }
        }
        assert!(checked >= 5, "expected several multi-triangle faces, got {checked}");
    }

    /// #1221: Select-tool pick over a live cut body prefers that body, not a datum plane.
    #[test]
    fn issue_1221_cut_body_beats_construction_plane() {
        use crate::camera::Camera;
        use eframe::egui::{Pos2, Rect};

        let bytes = include_bytes!("../tests/fixtures/issue_1221.json");
        let doc = crate::storage::from_json_bytes(bytes).expect("load");
        let b5 = doc.bodies.keys().nth(5).unwrap();
        assert!(!doc.bodies[b5].shadow, "body 5 is the live cut piece");
        let mesh5 = crate::extrude::body_solid_mesh(&doc, b5).expect("body 5 mesh");
        let (min, max) = mesh5.bounds().unwrap();
        let center = (min + max) * 0.5;

        let mut cam = Camera::default();
        cam.target = center;
        cam.distance = 350.0;
        cam.yaw = 0.4;
        cam.pitch = -0.9;
        let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let project = |w: glam::Vec3| cam.project(w, viewport, &vp);
        let visibility = crate::hierarchy::ElementVisibility::default();
        let occ = crate::construction::PickOcclusion::new(&doc, &visibility, cam.eye());

        let mut body_wins = 0usize;
        let mut plane_wins = 0usize;
        for tri in mesh5.triangles.iter().step_by(20) {
            let p = (tri[0] + tri[1] + tri[2]) / 3.0;
            let Some(sp) = project(p) else { continue };
            // Only sample where a body face is actually under the cursor.
            let Some(face_kind) = pick_body_face(sp, &project, &doc, cam.eye()) else {
                continue;
            };
            let crate::construction::PickTargetKind::BodyFace { body, .. } = face_kind else {
                continue;
            };
            if body != b5 {
                continue;
            }
            let gp = cam.ground_point(sp, viewport, &vp);
            let Some(t) = crate::construction::resolve_pick_target(
                sp,
                &project,
                gp,
                &doc,
                Some(&occ),
            ) else {
                continue;
            };
            match &t.kind {
                crate::construction::PickTargetKind::ConstructionPlane(_) => plane_wins += 1,
                crate::construction::PickTargetKind::BodyFace { body, .. } if *body == b5 => {
                    body_wins += 1;
                }
                crate::construction::PickTargetKind::BodyEdge { body, .. }
                | crate::construction::PickTargetKind::BodyVertex { body, .. }
                    if *body == b5 =>
                {
                    body_wins += 1;
                }
                // Sketch lines floating on consumed geometry can still rank above a face —
                // they are a separate problem; for this test we only care that the plane loses.
                other => {
                    assert!(
                        !matches!(other, crate::construction::PickTargetKind::ConstructionPlane(_)),
                        "plane must not win over body 5, got {other:?}"
                    );
                    body_wins += 1; // edge/vertex/line still not the plane
                }
            }
        }
        assert!(
            body_wins > 0,
            "expected samples over body 5 where a body pick wins, plane_wins={plane_wins}"
        );
        assert_eq!(
            plane_wins, 0,
            "construction plane must not beat body 5 under the body's own surface"
        );
    }



}