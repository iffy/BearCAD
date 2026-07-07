//! Technical drawings (#180): view projection and vector (SVG) export, independent of the
//! egui drawing pane so it can be unit-tested and reused for print/PDF output. An exported
//! SVG is black-on-white and prints to PDF through any browser/OS print dialog.

use crate::model::{Document, DrawingOrientation, DrawingView};
use glam::Vec3;

/// In-plane `(right, up)` world axes a drawing view projects onto: a point `p` maps to
/// `(p·right, p·up)`. The six orthographic directions plus a standard isometric view.
pub fn view_axes(orientation: DrawingOrientation) -> (Vec3, Vec3) {
    use DrawingOrientation as O;
    match orientation {
        O::Front => (Vec3::X, Vec3::Z),
        O::Back => (-Vec3::X, Vec3::Z),
        O::Right => (Vec3::Y, Vec3::Z),
        O::Left => (-Vec3::Y, Vec3::Z),
        O::Top => (Vec3::X, -Vec3::Y),
        O::Bottom => (Vec3::X, Vec3::Y),
        O::Isometric => {
            let out = Vec3::new(1.0, 1.0, 1.0).normalize();
            let right = Vec3::Z.cross(out).normalize();
            let up = out.cross(right).normalize();
            (right, up)
        }
    }
}

fn dequant(q: [i32; 3]) -> Vec3 {
    Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) / 100.0
}

fn edge_key(a: Vec3, b: Vec3) -> crate::model::DrawingEdgeKey {
    crate::model::normalized_edge_key(
        crate::hierarchy::quantize_body_point(a),
        crate::hierarchy::quantize_body_point(b),
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

const CELL_W: f32 = 520.0;
const CELL_H: f32 = 380.0;
const HEADER: f32 = 48.0;
const MARGIN: f32 = 30.0;

/// Render one drawing to a self-contained black-on-white SVG document. `None` if the drawing
/// index is missing or deleted.
pub fn drawing_to_svg(doc: &Document, index: usize) -> Option<String> {
    let drawing = doc.drawings.get(index).filter(|d| !d.deleted)?;
    let n = drawing.views.len();
    let cols = if n <= 1 { 1 } else { 2 };
    let rows = n.div_ceil(cols).max(1);
    let width = cols as f32 * CELL_W;
    let height = HEADER + rows as f32 * CELL_H;
    let unit = doc.default_length_unit;

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">\n"
    ));
    s.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" fill=\"white\"/>\n"
    ));
    let title = drawing
        .name
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Drawing {index}"));
    s.push_str(&format!(
        "<text x=\"{MARGIN}\" y=\"30\" font-family=\"sans-serif\" font-size=\"20\" \
         fill=\"black\">{}</text>\n",
        esc(&title)
    ));

    for (vi, view) in drawing.views.iter().enumerate() {
        let (col, row) = (vi % cols, vi / cols);
        let cell_x = col as f32 * CELL_W;
        let cell_y = HEADER + row as f32 * CELL_H;
        s.push_str(&format!(
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"none\" \
             stroke=\"#c8c8c8\" stroke-width=\"1\"/>\n",
            cell_x + 3.0,
            cell_y + 3.0,
            CELL_W - 6.0,
            CELL_H - 6.0
        ));
        s.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" font-family=\"sans-serif\" font-size=\"13\" \
             fill=\"black\">{}</text>\n",
            cell_x + 12.0,
            cell_y + 22.0,
            esc(&format!(
                "{} — {}",
                crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Body(view.body)),
                view.orientation.label()
            ))
        ));
        push_view_geometry(&mut s, doc, view, cell_x, cell_y, unit);
    }

    s.push_str("</svg>\n");
    Some(s)
}

fn push_view_geometry(
    s: &mut String,
    doc: &Document,
    view: &DrawingView,
    cell_x: f32,
    cell_y: f32,
    unit: crate::value::LengthUnit,
) {
    let world_edges = match crate::extrude::body_solid_mesh(doc, view.body) {
        Some(mesh) => crate::gpu_viewport::solid_mesh_unique_edges(&mesh),
        None => return,
    };
    if world_edges.is_empty() {
        return;
    }
    let (right, up) = view_axes(view.orientation);
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    let proj: Vec<(glam::Vec2, glam::Vec2)> = world_edges
        .iter()
        .map(|(a, b)| (project(*a), project(*b)))
        .collect();

    let (mut min, mut max) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for (a, b) in &proj {
        min = min.min(*a).min(*b);
        max = max.max(*a).max(*b);
    }
    let extent = (max - min).max(glam::Vec2::splat(1e-3));
    let area_w = CELL_W - 2.0 * MARGIN;
    let area_h = CELL_H - MARGIN - 40.0;
    let scale = (area_w / extent.x).min(area_h / extent.y) * 0.9;
    let bbox_center = (min + max) * 0.5;
    let area_center = glam::Vec2::new(cell_x + CELL_W * 0.5, cell_y + 40.0 + area_h * 0.5);
    // Model +up maps to SVG -y (y grows downward).
    let to_screen = |p: glam::Vec2| {
        let d = (p - bbox_center) * scale;
        glam::Vec2::new(area_center.x + d.x, area_center.y - d.y)
    };

    for (a, b) in &proj {
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        s.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"black\" \
             stroke-width=\"1.2\"/>\n",
            sa.x, sa.y, sb.x, sb.y
        ));
    }

    // Length dimensions: the measured length beside the edge midpoint.
    for (i, (a, b)) in proj.iter().enumerate() {
        let (wa, wb) = world_edges[i];
        if !view.dimensioned_edges.contains(&edge_key(wa, wb)) {
            continue;
        }
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        let mid = (sa + sb) * 0.5;
        let seg = sb - sa;
        let perp = if seg.length() > 1e-3 {
            glam::Vec2::new(-seg.y, seg.x).normalize()
        } else {
            glam::Vec2::new(0.0, -1.0)
        };
        let pos = mid + perp * 12.0;
        s.push_str(&text_svg(
            pos.x,
            pos.y,
            &crate::value::format_length_display_in((wa - wb).length(), unit),
        ));
    }

    // Angle dimensions: the degree value at (or near) the two edges' corner.
    for (k1, k2) in &view.angle_dims {
        let (a0, a1) = (dequant(k1.0), dequant(k1.1));
        let (b0, b1) = (dequant(k2.0), dequant(k2.1));
        let d1 = (a1 - a0).normalize_or_zero();
        let d2 = (b1 - b0).normalize_or_zero();
        if d1.length_squared() < 0.5 || d2.length_squared() < 0.5 {
            continue;
        }
        let angle = d1.angle_between(d2).to_degrees();
        let shared = [k1.0, k1.1]
            .into_iter()
            .find(|e| *e == k2.0 || *e == k2.1)
            .map(dequant);
        let anchor = shared.unwrap_or_else(|| ((a0 + a1) * 0.5 + (b0 + b1) * 0.5) * 0.5);
        let sp = to_screen(project(anchor));
        s.push_str(&text_svg(sp.x, sp.y - 12.0, &format!("{angle:.0}°")));
    }
}

fn text_svg(x: f32, y: f32, content: &str) -> String {
    format!(
        "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"sans-serif\" font-size=\"12\" \
         fill=\"black\" text-anchor=\"middle\">{}</text>\n",
        esc(content)
    )
}
