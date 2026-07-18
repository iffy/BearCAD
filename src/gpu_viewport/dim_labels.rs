//! GPU mesh builders for committed sketch dimension labels.

use crate::dimensions::{
    planar_label_frame, LinearDimensionWorldGeom, PlanarLabelView, LABEL_FONT_SIZE,
};
use eframe::egui::{Color32, FontId, Pos2};
use egui::Context;
use glam::Vec3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTextVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

fn color32_to_gpu(color: Color32) -> [f32; 4] {
    let [r, g, b, a] = color.to_array();
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

#[derive(Clone, Debug)]
pub struct ViewportDimLabel {
    pub world_geom: LinearDimensionWorldGeom,
    pub color: Color32,
    pub text_vertices: Vec<GpuTextVertex>,
    pub text_indices: Vec<u32>,
    pub draw_dimension_lines: bool,
}

/// Tessellate a dimension label into world-space textured vertices.
///
/// Glyphs are laid out **in the dimension's plane** (#454) through
/// [`planar_label_frame`]: orthonormal in-plane axes with one uniform scale, so the
/// text sits flat with its dimension lines and arrows, foreshortening naturally with
/// the plane under perspective but never shearing.
pub fn build_planar_label_mesh<Project>(
    ctx: &Context,
    world: &LinearDimensionWorldGeom,
    view: &PlanarLabelView,
    label: &str,
    color: Color32,
    project: &Project,
) -> (Vec<GpuTextVertex>, Vec<u32>)
where
    Project: Fn(Vec3) -> Option<Pos2>,
{
    let galley = ctx.fonts(|fonts| {
        fonts.layout_no_wrap(
            label.to_owned(),
            FontId::proportional(LABEL_FONT_SIZE),
            color,
        )
    });
    let size = galley.size();
    if size.x < 1e-4 || size.y < 1e-4 {
        return (Vec::new(), Vec::new());
    }
    let Some(frame) = planar_label_frame(world, view, size, project) else {
        return (Vec::new(), Vec::new());
    };
    let to_eye = (view.eye - world.label_center).normalize_or_zero();
    // Lift the glyphs slightly toward the eye so they never z-fight the face they
    // annotate.
    let depth_bias = to_eye * 0.25;

    let font_tex_size = ctx.fonts(|fonts| fonts.font_image_size());
    let uv_norm = egui::Vec2::new(
        1.0 / font_tex_size[0] as f32,
        1.0 / font_tex_size[1] as f32,
    );

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for row in &galley.rows {
        if row.visuals.mesh.is_empty() {
            continue;
        }
        let index_base = vertices.len() as u32;
        for (i, vertex) in row.visuals.mesh.vertices.iter().enumerate() {
            let mut world_pos = frame.glyph_point(vertex.pos.to_vec2());
            world_pos += depth_bias;
            let mut glyph_color = vertex.color;
            if glyph_color == Color32::PLACEHOLDER {
                glyph_color = color;
            } else if row.visuals.glyph_vertex_range.contains(&i) {
                glyph_color = color;
            }
            let uv = vertex.uv.to_vec2() * uv_norm;
            vertices.push(GpuTextVertex {
                position: world_pos.to_array(),
                uv: [uv.x, uv.y],
                color: color32_to_gpu(glyph_color),
            });
        }
        indices.extend(
            row.visuals
                .mesh
                .indices
                .iter()
                .map(|index| index + index_base),
        );
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::dimensions::linear_dimension_world_geom;
    use egui::Pos2;

    fn test_project(cam: &Camera, viewport: egui::Rect) -> impl Fn(Vec3) -> Option<Pos2> + '_ {
        let vp = cam.view_proj(viewport);
        move |w: Vec3| cam.project(w, viewport, &vp)
    }

    fn test_world() -> LinearDimensionWorldGeom {
        linear_dimension_world_geom(
            Vec3::new(-40.0, 10.0, 0.0),
            Vec3::new(40.0, 10.0, 0.0),
            Vec3::Y,
            5.0,
            1.0,
            2.0,
        )
    }

    fn build(cam: &Camera, world: &LinearDimensionWorldGeom) -> (Vec<GpuTextVertex>, Vec<u32>) {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let project = test_project(cam, viewport);
        let view = PlanarLabelView::from_camera_and_plane(cam, Vec3::Z);
        build_planar_label_mesh(&ctx, world, &view, "80.0 mm", Color32::WHITE, &project)
    }

    #[test]
    fn build_planar_label_mesh_emits_textured_vertices() {
        let cam = Camera::default();
        let (vertices, indices) = build(&cam, &test_world());
        assert!(!vertices.is_empty());
        assert!(!indices.is_empty());
        assert_eq!(indices.len() % 3, 0);
    }

    /// #454: the glyphs live in the dimension's plane (modulo the small toward-eye
    /// depth lift), flat with the dimension lines and arrows.
    #[test]
    fn label_mesh_lies_in_dimension_plane() {
        let mut cam = Camera::default();
        cam.orbit(egui::vec2(120.0, 45.0));
        let (vertices, _) = build(&cam, &test_world());
        assert!(!vertices.is_empty());
        for vertex in &vertices {
            assert!(
                vertex.position[2].abs() < 0.5,
                "glyph vertex should sit in the z = 0 sketch plane, got z {}",
                vertex.position[2]
            );
        }
    }

    /// #454: viewed face-on, the label reprojects at text size no matter how far out
    /// the camera zooms — the in-plane layout scales with the view, it doesn't skew.
    #[test]
    fn label_mesh_reprojects_at_label_size_when_zoomed_far_out() {
        let mut cam = Camera::default();
        cam.pitch = std::f32::consts::FRAC_PI_2 - 0.01;
        cam.yaw = 0.0;
        cam.distance = 3000.0;
        let world = test_world();
        let (vertices, _) = build(&cam, &world);
        assert!(!vertices.is_empty());
        let viewport = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let vp = cam.view_proj(viewport);
        let mut min = egui::Vec2::splat(f32::MAX);
        let mut max = egui::Vec2::splat(f32::MIN);
        for vertex in &vertices {
            let world_pos = Vec3::from_array(vertex.position);
            let screen = cam
                .project(world_pos, viewport, &vp)
                .expect("label vertex should project");
            min = min.min(screen.to_vec2());
            max = max.max(screen.to_vec2());
        }
        // The label follows the projected dimension line, which may run vertically at
        // this view — compare the box's short and long extents, not screen x/y.
        let size = max - min;
        let (short, long) = (size.x.min(size.y), size.x.max(size.y));
        assert!(
            (8.0..=30.0).contains(&short),
            "label should reproject at text height regardless of zoom, got {size:?}"
        );
        assert!(
            (20.0..=120.0).contains(&long),
            "label should reproject at text width regardless of zoom, got {size:?}"
        );
    }
}
