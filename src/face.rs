//! Sketch faces and parent/child dependencies between faces and sketch entities.

use crate::model::{
    ConstructionPlane, ConstructionPlaneParent, Document, FaceId, Line, PlaneAnchor,
    PlaneDefinition, Rect, SketchId,
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
        FaceId::Rect(i) => {
            let rect = doc.rects.get(i)?;
            let face = doc.sketch_face(rect.sketch)?;
            let parent = sketch_frame(doc, face)?;
            let origin = local_to_world(&parent, rect.x, rect.y);
            Some(SketchFrame {
                origin,
                u_axis: parent.u_axis,
                v_axis: parent.v_axis,
                normal: parent.normal,
            })
        }
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

/// Camera up vector so the sketch plane's v-axis points up on screen and u-axis points right.
pub fn sketch_view_up(view_direction: Vec3, frame: &SketchFrame) -> Vec3 {
    // `view_direction` points from the face toward the eye; `look_at_rh` uses the opposite.
    let look_forward = (-view_direction).normalize_or_zero();
    let mut up = frame.v_axis.normalize_or_zero();
    if up.length_squared() < 1e-8 {
        return Vec3::Z;
    }
    // Match `Mat4::look_at_rh`: right = cross(look_forward, up_hint).
    let right = look_forward.cross(up).normalize_or_zero();
    if right.dot(frame.u_axis) < 0.0 {
        up = -up;
    }
    up
}

pub fn rect_world_corners(doc: &Document, rect: &Rect) -> Option<[Vec3; 4]> {
    let frame = sketch_geometry_frame(doc, rect.sketch)?;
    Some([
        local_to_world(&frame, rect.x, rect.y),
        local_to_world(&frame, rect.x + rect.w, rect.y),
        local_to_world(&frame, rect.x + rect.w, rect.y + rect.h),
        local_to_world(&frame, rect.x, rect.y + rect.h),
    ])
}

pub fn line_world_endpoints(doc: &Document, line: &Line) -> Option<(Vec3, Vec3)> {
    let frame = sketch_geometry_frame(doc, line.sketch)?;
    Some((
        local_to_world(&frame, line.x0, line.y0),
        local_to_world(&frame, line.x1, line.y1),
    ))
}

pub fn rect_center_world(doc: &Document, rect: &Rect) -> Option<Vec3> {
    let frame = sketch_geometry_frame(doc, rect.sketch)?;
    Some(local_to_world(
        &frame,
        rect.x + rect.w * 0.5,
        rect.y + rect.h * 0.5,
    ))
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

fn sketch_local_bounds(doc: &Document, sketch: SketchId) -> Option<SketchZoomBounds> {
    let mut bounds: Option<SketchZoomBounds> = None;
    for rect in &doc.rects {
        if rect.sketch != sketch {
            continue;
        }
        let next = SketchZoomBounds::from_uv_rect(rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
        bounds = Some(match bounds {
            Some(b) => SketchZoomBounds::union(b, next),
            None => next,
        });
    }
    for line in &doc.lines {
        if line.sketch != sketch {
            continue;
        }
        let next = SketchZoomBounds::from_uv_rect(line.x0, line.y0, line.x1, line.y1);
        bounds = Some(match bounds {
            Some(b) => SketchZoomBounds::union(b, next),
            None => next,
        });
    }
    bounds
}

/// Resolve camera target, view direction, and optional zoom bounds for sketch mode.
pub fn sketch_camera_target(doc: &Document, sketch: SketchId) -> Option<SketchCameraTarget> {
    let face = doc.sketch_face(sketch)?;
    let frame = sketch_frame(doc, face)?;
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
        FaceId::Rect(i) => {
            let rect = doc.rects.get(i)?;
            let mut zoom = SketchZoomBounds::from_uv_rect(
                rect.x,
                rect.y,
                rect.x + rect.w,
                rect.y + rect.h,
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
        FaceId::Rect(i) => format!("Rectangle face {i}"),
    }
}

/// Pick a sketchable face (rectangle or construction plane) under the cursor.
pub fn pick_sketch_face(
    screen: eframe::egui::Pos2,
    project: &impl Fn(Vec3) -> Option<eframe::egui::Pos2>,
    doc: &Document,
) -> Option<FaceId> {
    let mut best: Option<(FaceId, f32)> = None;

    let mut consider = |face: FaceId, corners: [Vec3; 4]| {
        let pts: Option<Vec<eframe::egui::Pos2>> =
            corners.iter().map(|&c| project(c)).collect();
        let Some(pts) = pts else { return };
        let quad = [pts[0], pts[1], pts[2], pts[3]];
        let dist = if point_in_screen_quad(screen, quad) {
            0.0
        } else {
            dist_point_to_quad_edges(screen, quad)
        };
        if dist <= crate::construction::FACE_PICK_MARGIN_PX {
            if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                best = Some((face, dist));
            }
        }
    };

    for (i, plane) in doc.construction_planes.iter().enumerate().rev() {
        let corners = crate::construction::plane_corners(plane, crate::construction::PLANE_DISPLAY_HALF);
        consider(FaceId::ConstructionPlane(i), corners);
    }

    for (i, rect) in doc.rects.iter().enumerate().rev() {
        if let Some(corners) = rect_world_corners(doc, rect) {
            consider(FaceId::Rect(i), corners);
        }
    }

    best.map(|(face, _)| face)
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
    fn rect_face_frame_follows_parent_plane() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.rects.push(Rect::from_local_corners(
            sketch,
            5.0,
            5.0,
            15.0,
            15.0,
        ));
        let frame = sketch_frame(&doc, FaceId::Rect(0)).unwrap();
        assert!((frame.origin.x - 5.0).abs() < 1e-4);
        assert!((frame.origin.y - 5.0).abs() < 1e-4);
    }

    #[test]
    fn child_rect_is_offset_on_parent_face() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.rects.push(Rect::from_local_corners(s0, 0.0, 0.0, 10.0, 10.0));
        let s1 = doc.add_sketch(FaceId::Rect(0));
        doc.rects.push(Rect::from_local_corners(s1, 2.0, 3.0, 5.0, 6.0));
        let corners = rect_world_corners(&doc, &doc.rects[1]).unwrap();
        assert!((corners[0].x - 2.0).abs() < 1e-4);
        assert!((corners[0].y - 3.0).abs() < 1e-4);
    }

    #[test]
    fn has_children_detects_dependents() {
        let mut doc = Document::default();
        assert!(!doc.has_children(FaceId::ConstructionPlane(0)));
        doc.sketches.push(Sketch {
            face: FaceId::ConstructionPlane(0),
        });
        assert!(doc.has_children(FaceId::ConstructionPlane(0)));
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
    fn sketch_camera_plane_with_children_requests_zoom() {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.rects.push(Rect::from_local_corners(
            sketch,
            10.0,
            20.0,
            90.0,
            60.0,
        ));
        let target = sketch_camera_target(&doc, sketch).unwrap();
        let zoom = target.zoom.expect("children should request zoom");
        assert!((zoom.center_u - 50.0).abs() < 1e-4);
        assert!((zoom.center_v - 40.0).abs() < 1e-4);
        assert!((zoom.half_u - 40.0).abs() < 1e-4);
        assert!((zoom.half_v - 20.0).abs() < 1e-4);
    }

    #[test]
    fn sketch_camera_rect_face_includes_face_and_children() {
        let mut doc = Document::default();
        let s0 = doc.add_sketch(FaceId::ConstructionPlane(0));
        doc.rects.push(Rect::from_local_corners(s0, 0.0, 0.0, 20.0, 20.0));
        let s1 = doc.add_sketch(FaceId::Rect(0));
        doc.rects.push(Rect::from_local_corners(
            s1,
            2.0,
            2.0,
            18.0,
            18.0,
        ));
        doc.lines.push(Line::from_local_endpoints(
            s1,
            5.0,
            5.0,
            15.0,
            10.0,
        ));
        let target = sketch_camera_target(&doc, s1).unwrap();
        let zoom = target.zoom.unwrap();
        assert!(zoom.half_u >= 8.0);
        assert!(zoom.half_v >= 8.0);
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
        cam.set_view_up(Some(sketch_view_up(view_direction, &frame)));
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