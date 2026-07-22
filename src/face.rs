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
        deleted: false,
    }
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
            if ext.deleted || !ext.faces.contains(&profile) {
                return None;
            }
            let base = sketch_frame(doc, profile.face_id())?;
            // A top cap that meets a slanted target plane lies in that plane, so derive its
            // frame from the actual (slanted) cap polygon rather than a parallel offset.
            if top && crate::extrude::target_top_plane(doc, ext).is_some() {
                let poly = crate::extrude::cap_polygon_world(doc, extrusion, &profile, true)?;
                return frame_from_polygon(&poly, base.normal);
            }
            // Otherwise the cap shares the profile's in-plane axes, shifted along the
            // extrusion normal to the base or offset end.
            let dist = if top {
                crate::extrude::effective_distance(doc, ext)
            } else {
                0.0
            };
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

fn axis_screen_preserve_weight(screen: glam::Vec2) -> f32 {
    let len = screen.length();
    if len < 1e-6 {
        0.0
    } else if screen.x > 0.0 {
        // Already pointing right on screen — keep it there.
        screen.x / len
    } else if screen.y < 0.0 {
        // Already pointing up on screen (egui y-down).
        screen.y.abs() / len
    } else {
        0.0
    }
}

fn axes_match_sketch_convention(u_screen: glam::Vec2, v_screen: glam::Vec2) -> bool {
    let u_right = u_screen.x > 0.0 && u_screen.x.abs() >= u_screen.y.abs();
    let v_up = v_screen.y < 0.0 && v_screen.y.abs() >= v_screen.x.abs();
    u_right && v_up
}

fn axis_is_screen_horizontal(screen: glam::Vec2) -> bool {
    screen.x.abs() > screen.y.abs()
}

fn sketch_view_up_score(
    u_screen_before: glam::Vec2,
    v_screen_before: glam::Vec2,
    u_screen_after: glam::Vec2,
    v_screen_after: glam::Vec2,
) -> f32 {
    let use_minimal_roll =
        axis_is_screen_horizontal(u_screen_before) && axis_is_screen_horizontal(v_screen_before);
    if use_minimal_roll {
        let delta_u = u_screen_after - u_screen_before;
        let delta_v = v_screen_after - v_screen_before;
        let u_preserve = axis_screen_preserve_weight(u_screen_before);
        let v_preserve = axis_screen_preserve_weight(v_screen_before);
        let mut score = (1.0 + 3.0 * u_preserve) * delta_u.length_squared()
            + (1.0 + 3.0 * v_preserve) * delta_v.length_squared()
            - 2.0 * u_preserve * u_screen_after.dot(u_screen_before)
            - 2.0 * v_preserve * v_screen_after.dot(v_screen_before);
        if !axes_match_sketch_convention(u_screen_after, v_screen_after) {
            score += 0.2;
        }
        score
    } else if axes_match_sketch_convention(u_screen_after, v_screen_after) {
        0.0
    } else {
        1.0
    }
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
            let u_screen_after = axis_screen_vec(u, target_look, hint);
            let v_screen_after = axis_screen_vec(v, target_look, hint);
            // The plane's u-axis pointing screen-right and v-axis screen-up is
            // authoritative (#187): a Horizontal constraint fixes a line along u and a
            // Vertical constraint along v, so this is the only orientation where those
            // constraints read horizontal/vertical on screen. Roll-preservation only
            // breaks ties among convention-matching orientations (there is normally just
            // one, so it rarely matters), never overrides the convention.
            // Far larger than any roll score, so a convention-matching orientation
            // always wins; roll only orders ties among matching orientations.
            let convention_penalty =
                if axes_match_sketch_convention(u_screen_after, v_screen_after) {
                    0.0
                } else {
                    1000.0
                };
            convention_penalty
                + sketch_view_up_score(
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
    for line in &doc.lines {
        if line.sketch == sketch {
            extend_sketch_bounds(&mut bounds, line.x0, line.y0, line.x1, line.y1);
        }
    }
    for circle in &doc.circles {
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
    }
}

pub fn sketch_label(doc: &Document, sketch: SketchId) -> String {
    let face = doc
        .sketch_face(sketch)
        .map(|face| face_label(doc, face))
        .unwrap_or_else(|| "unknown face".to_string());
    format!("Sketch {sketch} on {face}")
}

pub fn face_label(_doc: &Document, face: FaceId) -> String {
    match face {
        FaceId::ConstructionPlane(i) => format!("Construction plane {i}"),
        FaceId::Circle(i) => format!("Circle face {i}"),
        FaceId::Polygon(lines) => format!("Polygon face ({} lines)", lines.len()),
        FaceId::ExtrudeCap {
            extrusion, top, ..
        } => {
            let end = if top { "top" } else { "bottom" };
            format!("Extrusion {extrusion} {end} face")
        }
        FaceId::ExtrudeSide {
            extrusion, edge, ..
        } => format!("Extrusion {extrusion} side face {edge}"),
    }
}

/// Screen-distance band within which two face picks count as "the same depth
/// under the cursor", so the nearer (camera-facing) one is preferred. This is
/// what keeps a hovered solid from selecting its hidden back face.
const FACE_PICK_DEPTH_TIE_PX: f32 = 0.5;

fn consider_face_pick(
    best: &mut Option<(FaceId, f32, f32)>,
    face: FaceId,
    dist: f32,
    depth: f32,
) {
    if dist > crate::construction::FACE_PICK_MARGIN_PX {
        return;
    }
    let better = match best.as_ref() {
        None => true,
        Some((_, best_dist, best_depth)) => {
            if dist < best_dist - FACE_PICK_DEPTH_TIE_PX {
                true
            } else if dist > best_dist + FACE_PICK_DEPTH_TIE_PX {
                false
            } else {
                // Essentially the same screen distance (e.g. cursor inside both the
                // front and back face of a solid): prefer the one nearer the camera.
                depth < *best_depth
            }
        }
    };
    if better {
        *best = Some((face, dist, depth));
    }
}

/// The exact face a sketch profile candidate (`Circle`/`Polygon`) was drawn on, if any.
fn sketch_host_face(doc: &Document, face: &FaceId) -> Option<FaceId> {
    let sketch = match face {
        FaceId::Circle(i) => doc.circles.get(*i)?.sketch,
        FaceId::Polygon(lines) => doc.lines.get(lines.first().copied()?)?.sketch,
        FaceId::ConstructionPlane(_) | FaceId::ExtrudeCap { .. } | FaceId::ExtrudeSide { .. } => {
            return None
        }
    };
    doc.sketches.get(sketch).map(|s| s.face.clone())
}

/// True when `best` is a sketch profile drawn directly on `candidate`, at essentially the
/// same screen distance (#117): a rectangle sketched on a solid's face is coincident with
/// that face, so its centroid can be farther from the eye than the (larger) host face's —
/// the depth tie-break in [`consider_face_pick`] would then wrongly let the plain face win
/// the pick, silently discarding the sketch (`Extrude` only picks `Circle`/`Polygon` faces).
/// A sketch drawn on a face is always meant to be picked over the bare face beneath it, so
/// skip the depth compare entirely once we know — by construction, not by geometry — that
/// they're the same surface.
fn sketch_shadows(best: &Option<(FaceId, f32, f32)>, candidate: &FaceId, dist: f32, doc: &Document) -> bool {
    let Some((best_face, best_dist, _)) = best else {
        return false;
    };
    if (dist - best_dist).abs() > FACE_PICK_DEPTH_TIE_PX {
        return false;
    }
    sketch_host_face(doc, best_face).as_ref() == Some(candidate)
}

fn centroid(points: &[Vec3]) -> Vec3 {
    if points.is_empty() {
        return Vec3::ZERO;
    }
    points.iter().copied().sum::<Vec3>() / points.len() as f32
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
    Some((dist, centroid(&corners)))
}

/// Pick a sketchable face (rectangle, circle, or construction plane) under the cursor.
pub fn pick_sketch_face(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    eye: Vec3,
) -> Option<FaceId> {
    let mut best: Option<(FaceId, f32, f32)> = None;
    let depth = |p: Vec3| (p - eye).length();

    for (i, circle) in doc.circles.iter().enumerate().rev() {
        if let Some((dist, c)) = circle_face_pick_distance(screen, doc, circle, project) {
            consider_face_pick(&mut best, FaceId::Circle(i), dist, depth(c));
        }
    }

    // Closed loops of plain lines (#66).
    for sketch in (0..doc.sketches.len()).rev() {
        for lines in crate::polygon::closed_line_loops(doc, sketch) {
            if let Some((poly, _)) = crate::extrude::face_profile_world(
                doc,
                &crate::model::ExtrudeFace::Polygon(lines.clone()),
            ) {
                if let Some((dist, c)) = polygon_face_pick_distance(screen, project, &poly) {
                    consider_face_pick(&mut best, FaceId::Polygon(lines), dist, depth(c));
                }
            }
        }
    }

    // Planar caps of extruded bodies (so sketches can be placed on them). Tested
    // before construction planes since a solid cap occludes the datum plane.
    for (ei, extrusion) in doc.extrusions.iter().enumerate().rev() {
        if extrusion.deleted {
            continue;
        }
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
                    if !sketch_shadows(&best, &candidate, dist, doc) {
                        consider_face_pick(&mut best, candidate, dist, depth(c));
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
                    if !sketch_shadows(&best, &candidate, dist, doc) {
                        consider_face_pick(&mut best, candidate, dist, depth(c));
                    }
                }
            }
        }
    }

    for (i, plane) in doc.construction_planes.iter().enumerate().rev() {
        let corners = crate::construction::plane_corners(plane, crate::construction::PLANE_DISPLAY_HALF);
        if let Some((dist, c)) = quad_face_pick_distance(screen, project, corners) {
            let candidate = FaceId::ConstructionPlane(i);
            if !sketch_shadows(&best, &candidate, dist, doc) {
                consider_face_pick(&mut best, candidate, dist, depth(c));
            }
        }
    }

    best.map(|(face, _, _)| face)
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
    let mut best: Option<(crate::construction::PickTargetKind, f32)> = None;
    for (bi, body) in doc.bodies.iter().enumerate() {
        if body.deleted || body.shadow {
            continue;
        }
        let Some(solid) = crate::extrude::body_solid_mesh(doc, bi) else {
            continue;
        };
        for triangles in crate::gpu_viewport::solid_mesh_coplanar_faces(&solid) {
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
                let normal = (triangles[0][1] - triangles[0][0])
                    .cross(triangles[0][2] - triangles[0][0])
                    .normalize_or_zero();
                best = Some((
                    crate::construction::PickTargetKind::BodyFace {
                        body: bi,
                        triangles,
                        normal,
                    },
                    depth,
                ));
            }
        }
    }
    best.map(|(kind, _)| kind)
}

/// Screen-space pick distance to an extrusion cap polygon (0 inside).
fn cap_face_pick_distance(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
    extrusion: usize,
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
    extrusion: usize,
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
    let c = centroid(poly);
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
    u >= 0.0 && v >= 0.0 && (u + v) <= 1.0
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
mod tests {
    use super::*;
    use crate::model::Sketch;

    #[test]
    fn default_document_has_xy_construction_plane() {
        let doc = Document::default();
        assert_eq!(doc.construction_planes.len(), 1);
        assert!((doc.construction_planes[0].normal.z - 1.0).abs() < 1e-4);
        assert!(doc.shape_order.is_empty());
    }

    #[test]
    fn sketch_on_plane_stores_local_coordinates() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let frame = sketch_geometry_frame(&doc, sketch).unwrap();
        let p = local_to_world(&frame, 10.0, 20.0);
        let (u, v) = world_to_local(&frame, p);
        assert!((u - 10.0).abs() < 1e-4);
        assert!((v - 20.0).abs() < 1e-4);
    }

    #[test]
    fn circle_face_frame_origin_is_center() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles
            .push(Circle::from_local_center_radius(sketch, 5.0, 7.0, 10.0, 0.0));
        let frame = sketch_frame(&doc, FaceId::Circle(0)).unwrap();
        assert!((frame.origin.x - 5.0).abs() < 1e-4);
        assert!((frame.origin.y - 7.0).abs() < 1e-4);
    }

    #[test]
    fn child_sketch_on_circle_face_uses_center_origin() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles
            .push(Circle::from_local_center_radius(s0, 10.0, 10.0, 5.0, 0.0));
        let s1 = doc.add_sketch(FaceId::Circle(0));
        let frame = sketch_geometry_frame(&doc, s1).unwrap();
        let p = local_to_world(&frame, 2.0, 3.0);
        assert!((p.x - 12.0).abs() < 1e-4);
        assert!((p.y - 13.0).abs() < 1e-4);
    }

    #[test]
    fn pick_sketch_face_finds_circle_interior() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles
            .push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 20.0, 0.0));
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.y));
        let face = pick_sketch_face(eframe::egui::pos2(5.0, 0.0), &project, &doc, Vec3::new(0.0, 0.0, 100.0));
        assert_eq!(face, Some(FaceId::Circle(0)));
    }

    #[test]
    fn sketch_camera_circle_face_includes_face_and_children() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles
            .push(Circle::from_local_center_radius(s0, 0.0, 0.0, 20.0, 0.0));
        let s1 = doc.add_sketch(FaceId::Circle(0));
        doc.lines
            .push(Line::from_local_endpoints(s1, -5.0, -5.0, 5.0, 5.0));
        let target = sketch_camera_target(&doc, s1).unwrap();
        let zoom = target.zoom.unwrap();
        assert!(zoom.half_u >= 5.0);
        assert!(zoom.half_v >= 5.0);
    }

    fn doc_with_extruded_box() -> Document {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let rect_lines =
            crate::construction::add_line_rectangle(&mut doc, sketch, 0.0, 0.0, 20.0, 20.0, [false; 4]);
        doc.extrusions.push(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(rect_lines.to_vec())],
            distance: 10.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            deleted: false,
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
        doc.imported_meshes.push(crate::model::ImportedMesh {
            triangles,
            source_name: "box".to_string(),
        });
        doc.bodies.push(crate::model::Body {
            source: crate::model::BodySource::Imported(0),
            name: None,
            deleted: false,
            shadow: false,
        });
        doc
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
        let doc = doc_with_extruded_box();
        // Offset screen x by height so the top cap (z=10) separates from the base
        // rect; click where only the lifted top cap projects.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x + p.z, p.y));
        let face = pick_sketch_face(eframe::egui::pos2(25.0, 10.0), &project, &doc, Vec3::new(0.0, 0.0, 100.0));
        assert!(
            matches!(
                face,
                Some(FaceId::ExtrudeCap {
                    extrusion: 0,
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.circles
            .push(Circle::from_local_center_radius(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.extrusions.push(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Circle(0)],
            distance: 8.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            deleted: false,
            edge_treatments: Vec::new(),
        });
        let profile = crate::model::ExtrudeFace::Circle(0);
        assert_eq!(crate::extrude::side_face_count(&profile), 0);
        assert!(crate::extrude::side_quad_world(&doc, 0, &profile, 0).is_none());
    }

    #[test]
    fn pick_sketch_face_finds_extrusion_side_wall() {
        let doc = doc_with_extruded_box();
        // Project to the XZ plane so the y=0 side wall shows as a 20x10 rectangle.
        let project = |p: Vec3| Some(eframe::egui::Pos2::new(p.x, p.z));
        let face = pick_sketch_face(eframe::egui::pos2(10.0, 5.0), &project, &doc, Vec3::new(0.0, 0.0, 100.0));
        assert!(
            matches!(face, Some(FaceId::ExtrudeSide { extrusion: 0, .. })),
            "clicking a side wall should pick it, got {face:?}"
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
        let profile = crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3]);
        let host = FaceId::ExtrudeSide {
            extrusion: 0,
            profile: profile.clone(),
            edge: 0,
        };
        let wall = crate::extrude::side_quad_world(&doc, 0, &profile, 0).expect("wall face exists");
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
    /// constraints that make it a recognized loop), returning the line indices.
    fn add_line_loop(doc: &mut Document, sketch: SketchId, vertices: &[(f32, f32)]) -> Vec<usize> {
        use crate::model::{Constraint, ConstraintEntity, ConstraintKind, ConstraintPoint, LineEnd};
        let base = doc.lines.len();
        let n = vertices.len();
        for i in 0..n {
            let (u0, v0) = vertices[i];
            let (u1, v1) = vertices[(i + 1) % n];
            doc.lines
                .push(Line::from_local_endpoints(sketch, u0, v0, u1, v1));
        }
        for i in 0..n {
            doc.constraints.push(Constraint {
                sketch,
                kind: ConstraintKind::Coincident {
                    a: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                        line: base + i,
                        end: LineEnd::End,
                    }),
                    b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                        line: base + (i + 1) % n,
                        end: LineEnd::Start,
                    }),
                },
                expression: String::new(),
                dim_offset: None,
                name: None,
                deleted: false,
            });
        }
        (base..doc.lines.len()).collect()
    }

    fn extrude_loop(doc: &mut Document, sketch: SketchId, lines: Vec<usize>) {
        doc.extrusions.push(crate::model::Extrusion {
            sketch,
            faces: vec![crate::model::ExtrudeFace::Polygon(lines)],
            distance: 10.0,
            target: None,
            expression: String::new(),
            name: None,
            symmetric: false,
            deleted: false,
            edge_treatments: Vec::new(),
        });
    }

    /// Every side-wall frame's normal must point out of the solid and its (u, v, normal)
    /// triad must be right-handed — checked by mapping the frame's outward offset back to
    /// the profile plane and asserting it lands *outside* the profile polygon.
    fn assert_side_frames_outward(doc: &Document, vertices: &[(f32, f32)], lines: &[usize]) {
        let profile = crate::model::ExtrudeFace::Polygon(lines.to_vec());
        for edge in 0..vertices.len() {
            let frame = sketch_frame(
                doc,
                FaceId::ExtrudeSide {
                    extrusion: 0,
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
            let plane = sketch_frame(doc, FaceId::ConstructionPlane(0)).unwrap();
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
            FaceId::ExtrudeSide { extrusion: 0, profile: profile.clone(), edge: 2 },
        )
        .unwrap();
        assert!(f2.normal.dot(Vec3::Y) > 0.99, "edge 2 outward is +Y, got {:?}", f2.normal);
        let f3 = sketch_frame(
            &doc,
            FaceId::ExtrudeSide { extrusion: 0, profile, edge: 3 },
        )
        .unwrap();
        assert!(f3.normal.dot(Vec3::X) > 0.99, "edge 3 outward is +X, got {:?}", f3.normal);
    }

    /// A clockwise-wound profile must get the same outward walls as a CCW one — the
    /// winding sign feeds the normal derivation, not the result.
    #[test]
    fn clockwise_profiles_still_get_outward_side_frames() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
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
        let profile = crate::model::ExtrudeFace::Polygon(vec![0, 1, 2, 3]);
        let expected = [-Vec3::Y, Vec3::X, Vec3::Y, -Vec3::X];
        for edge in 0..4u8 {
            let frame = sketch_frame(
                &doc,
                FaceId::ExtrudeSide {
                    extrusion: 0,
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
        assert!(!doc.has_children(&FaceId::ConstructionPlane(0)));
        doc.sketches.push(Sketch {
            face: FaceId::ConstructionPlane(0),
            name: None,
            deleted: false,
            length_unit: None,
            angle_unit: None,
        });
        assert!(doc.has_children(&FaceId::ConstructionPlane(0)));
    }

    #[test]
    fn sketch_camera_empty_plane_orients_without_zoom() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        let target = sketch_camera_target(&doc, sketch).unwrap();
        assert!(target.zoom.is_none());
        assert!(target.target.length_squared() < 1e-8);
        assert!((target.face_normal.z - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sketch_view_up_from_isometric_puts_u_axis_right_v_axis_up() {
        use crate::camera::Camera;

        // Entering the ground (XY) sketch from the default isometric view must orient the
        // plane's u-axis (+X) screen-right and v-axis (+Y) screen-up, so a Horizontal
        // constraint (line along u) reads horizontal and a Vertical constraint (along v)
        // reads vertical (#187) — not the prior roll-preserving "u-axis down" pick.
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
        // The v-axis (+Y) becomes screen-up.
        assert!(
            hint.dot(Vec3::Y) > 0.9,
            "isometric entry should pick +Y (v-axis) up, got {hint:?}"
        );
        // And the u-axis (+X) lands screen-right with the v-axis screen-up.
        let u_screen = axis_screen_vec(frame.u_axis, target_look, hint);
        let v_screen = axis_screen_vec(frame.v_axis, target_look, hint);
        assert!(
            axes_match_sketch_convention(u_screen, v_screen),
            "u should be screen-right and v screen-up: u={u_screen:?} v={v_screen:?}"
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
        doc.construction_planes.push(plane_from_definition(
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
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(1));
        let frame = sketch_frame(&doc, FaceId::ConstructionPlane(1)).unwrap();
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

        assert!(
            above.y < base.y,
            "positive v should point up on screen (smaller egui y)"
        );
        assert!(
            right.x > base.x,
            "positive u should point right on screen"
        );
        let _ = sketch;
    }
}