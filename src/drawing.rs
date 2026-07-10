//! Technical drawings (#180): view projection and vector export, independent of the egui
//! drawing pane so it can be unit-tested and reused for print/PDF output. A drawing renders
//! black-on-white to either **SVG** (prints to PDF via any browser/OS print dialog) or a
//! direct single-page **PDF** — both drive the identical layout through the [`Canvas`] trait,
//! so the two exports never drift.

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

/// PDF points per millimetre (1 pt = 1/72 in): exports are sized in points so the PDF page
/// physically matches the drawing's configured mm page (#298).
const PT_PER_MM: f32 = 72.0 / 25.4;
/// A placed view card's size as a fraction of the page — the same 0.42 the editor uses, so the
/// export lays out where the editor showed it (#297).
const CELL_FRAC: f32 = 0.42;
/// Padding inside a view card between its border and the projected geometry.
const CELL_PAD: f32 = 12.0;

/// The world-space feature edges a drawing view projects (#278): a body's solid-mesh unique
/// edges, or — when the view's `sketch` is set — that sketch's line/circle geometry. Shared by
/// the editor pane and the SVG/PDF export so both draw the same thing.
pub fn drawing_view_world_edges(doc: &Document, view: &DrawingView) -> Vec<(Vec3, Vec3)> {
    if let Some(si) = view.sketch {
        let mut edges = Vec::new();
        for line in doc.lines.iter().filter(|l| !l.deleted && l.sketch == si) {
            if let Some(pts) = crate::face::line_world_polyline(doc, line) {
                for w in pts.windows(2) {
                    edges.push((w[0], w[1]));
                }
            }
        }
        for circle in doc.circles.iter().filter(|c| !c.deleted && c.sketch == si) {
            if let Some(pts) = crate::face::circle_world_perimeter(doc, circle, 48) {
                for w in pts.windows(2) {
                    edges.push((w[0], w[1]));
                }
            }
        }
        edges
    } else {
        crate::extrude::body_solid_mesh(doc, view.body)
            .map(|mesh| crate::gpu_viewport::solid_mesh_unique_edges(&mesh))
            .unwrap_or_default()
    }
}

/// Projected 2D geometry for a drawing view under its display style (#301), shared by the
/// editor pane and the SVG/PDF export.
pub struct StyledViewGeometry {
    /// Back-to-front shaded triangles (projected 2D + a 0..1 grey, 1 = white) — `Shaded` only.
    pub tris: Vec<([glam::Vec2; 3], f32)>,
    /// The edge segments to stroke: every feature edge for `Wireframe`; only the visible
    /// runs (hidden lines removed) for `Visible`/`Shaded`.
    pub segments: Vec<(glam::Vec2, glam::Vec2)>,
}

/// Project a view's geometry under its display style (#301). Sketch views have no solid to
/// occlude or shade, so they always render as plain wireframe.
pub fn styled_view_geometry(doc: &Document, view: &DrawingView) -> StyledViewGeometry {
    use crate::model::DrawingViewStyle;
    let (right, up) = view_axes(view.orientation);
    let project = |p: Vec3| glam::Vec2::new(p.dot(right), p.dot(up));
    let edges = drawing_view_world_edges(doc, view);
    let wireframe = || StyledViewGeometry {
        tris: Vec::new(),
        segments: edges.iter().map(|(a, b)| (project(*a), project(*b))).collect(),
    };
    if view.sketch.is_some() || view.style == DrawingViewStyle::Wireframe {
        return wireframe();
    }
    let Some(mesh) = crate::extrude::body_solid_mesh(doc, view.body) else {
        return wireframe();
    };
    // Depth grows toward the viewer along the view's out-of-page axis.
    let toward = right.cross(up);
    let Some((lo, hi)) = mesh.bounds() else {
        return wireframe();
    };
    let eps = (hi - lo).length().max(1e-3) * 2e-3;

    // Projected triangles with per-vertex depth, for point-occlusion tests.
    struct ProjTri {
        p: [glam::Vec2; 3],
        d: [f32; 3],
        /// Twice the signed area of the projected triangle; ~0 = edge-on, skipped.
        area2: f32,
    }
    let tris: Vec<ProjTri> = mesh
        .triangles
        .iter()
        .map(|t| {
            let p = [project(t[0]), project(t[1]), project(t[2])];
            let area2 = (p[1] - p[0]).perp_dot(p[2] - p[0]);
            ProjTri { p, d: [t[0].dot(toward), t[1].dot(toward), t[2].dot(toward)], area2 }
        })
        .filter(|t| t.area2.abs() > 1e-6)
        .collect();
    // Whether some face is strictly in front of `(point, depth)`.
    let occluded = |point: glam::Vec2, depth: f32| -> bool {
        tris.iter().any(|t| {
            // Barycentric coordinates of `point` in the projected triangle.
            let w0 = (t.p[1] - point).perp_dot(t.p[2] - point) / t.area2;
            let w1 = (t.p[2] - point).perp_dot(t.p[0] - point) / t.area2;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                return false;
            }
            w0 * t.d[0] + w1 * t.d[1] + w2 * t.d[2] > depth + eps
        })
    };

    // Sample each edge and keep the visible runs (hidden-line removal).
    const SAMPLES: usize = 32;
    let mut segments = Vec::new();
    for (a, b) in &edges {
        let mut run_start: Option<f32> = None;
        let mut push_run = |from: f32, to: f32| {
            let wa = a.lerp(*b, from);
            let wb = a.lerp(*b, to);
            segments.push((project(wa), project(wb)));
        };
        for i in 0..SAMPLES {
            let t = (i as f32 + 0.5) / SAMPLES as f32;
            let w = a.lerp(*b, t);
            let visible = !occluded(project(w), w.dot(toward));
            match (visible, run_start) {
                (true, None) => run_start = Some(i as f32 / SAMPLES as f32),
                (false, Some(s)) => {
                    push_run(s, i as f32 / SAMPLES as f32);
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = run_start {
            push_run(s, 1.0);
        }
    }

    // Shaded fills: front faces painted back-to-front, greyed by how squarely they face a
    // fixed key light up-and-left of the viewer.
    let mut fills = Vec::new();
    if view.style == DrawingViewStyle::Shaded {
        let light = (toward * 1.2 - right * 0.35 + up * 0.55).normalize();
        let mut shaded: Vec<(f32, [glam::Vec2; 3], f32)> = mesh
            .triangles
            .iter()
            .filter_map(|t| {
                let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
                if n == Vec3::ZERO || n.dot(toward) <= 0.0 {
                    return None; // back or degenerate face
                }
                let shade = 0.62 + 0.33 * n.dot(light).max(0.0);
                let depth = (t[0] + t[1] + t[2]).dot(toward) / 3.0;
                Some((depth, [project(t[0]), project(t[1]), project(t[2])], shade))
            })
            .collect();
        shaded.sort_by(|a, b| a.0.total_cmp(&b.0));
        fills = shaded.into_iter().map(|(_, p, s)| (p, s)).collect();
    }

    StyledViewGeometry { tris: fills, segments }
}

/// An 8-bit RGB paint.
#[derive(Clone, Copy, PartialEq)]
struct Rgb(u8, u8, u8);
const BLACK: Rgb = Rgb(0, 0, 0);
const WHITE: Rgb = Rgb(255, 255, 255);
const GRAY: Rgb = Rgb(200, 200, 200);

/// Horizontal text alignment relative to the given `x`.
#[derive(Clone, Copy)]
enum Anchor {
    Start,
    Middle,
}

/// A 2D vector-drawing sink in top-left (SVG-style) coordinates: `y` grows downward and text
/// `y` is the baseline. The PDF backend flips to bottom-left internally. Both backends render
/// the same [`render_drawing`] output.
trait Canvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32);
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32);
    /// A filled polygon (shaded view faces, #301).
    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb);
    /// Text is always black in a drawing; `size` is the font size in px.
    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str);
}

/// The page size (width, height) in PDF points for a drawing — its configured mm page (#298),
/// landscape US-Letter by default — or `None` if the index is missing/deleted.
fn page_dims(doc: &Document, index: usize) -> Option<(f32, f32)> {
    let drawing = doc.drawings.get(index).filter(|d| !d.deleted)?;
    Some((
        drawing.page_width_mm * PT_PER_MM,
        drawing.page_height_mm * PT_PER_MM,
    ))
}

/// Draw a whole drawing into `canvas`, WYSIWYG with the editor (#297): each view is a card
/// centred at its `pos_x`/`pos_y` page fraction, sized like the editor's cards, on the
/// drawing's configured page. The title sits in the top margin.
fn render_drawing<C: Canvas>(doc: &Document, index: usize, canvas: &mut C) -> Option<()> {
    let drawing = doc.drawings.get(index).filter(|d| !d.deleted)?;
    let (width, height) = page_dims(doc, index)?;
    let unit = doc.default_length_unit;

    canvas.rect(0.0, 0.0, width, height, Some(WHITE), None, 0.0);
    let margin = (drawing.margin_mm * PT_PER_MM).clamp(0.0, width.min(height) * 0.4);
    let title = drawing
        .name
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Drawing {index}"));
    canvas.text(margin.max(12.0), (margin * 0.7).max(16.0), 14.0, Anchor::Start, &title);

    let cell_w = width * CELL_FRAC;
    let cell_h = height * CELL_FRAC;
    for view in &drawing.views {
        let cell_x = view.pos_x * width - cell_w * 0.5;
        let cell_y = view.pos_y * height - cell_h * 0.5;
        canvas.rect(cell_x + 3.0, cell_y + 3.0, cell_w - 6.0, cell_h - 6.0, None, Some(GRAY), 1.0);
        let source = match view.sketch {
            Some(si) => crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Sketch(si)),
            None => crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Body(view.body)),
        };
        let scale_suffix = view
            .scale
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let label = format!("{source} — {}{scale_suffix}", view.orientation.label());
        canvas.text(cell_x + CELL_PAD, cell_y + 20.0, 11.0, Anchor::Start, &label);
        render_view_geometry(canvas, doc, view, cell_x, cell_y, cell_w, cell_h, unit);
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn render_view_geometry<C: Canvas>(
    canvas: &mut C,
    doc: &Document,
    view: &DrawingView,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
    unit: crate::value::LengthUnit,
) {
    let world_edges = drawing_view_world_edges(doc, view);
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
    // The caption strip takes the top of the card; geometry fits below it.
    let caption_h = 26.0;
    let area_w = cell_w - 2.0 * CELL_PAD;
    let area_h = cell_h - caption_h - 2.0 * CELL_PAD;
    // A set print scale (#300) draws at exactly `factor` page-mm per model-mm (points on the
    // export canvas); otherwise auto-fit to the card.
    let scale = match view.scale.as_deref().and_then(crate::model::parse_drawing_scale) {
        Some(factor) => factor * PT_PER_MM,
        None => (area_w / extent.x).min(area_h / extent.y) * 0.9,
    };
    let bbox_center = (min + max) * 0.5;
    let area_center =
        glam::Vec2::new(cell_x + cell_w * 0.5, cell_y + caption_h + CELL_PAD + area_h * 0.5);
    // Model +up maps to screen -y (y grows downward).
    let to_screen = |p: glam::Vec2| {
        let d = (p - bbox_center) * scale;
        glam::Vec2::new(area_center.x + d.x, area_center.y - d.y)
    };

    // Strokes (and shaded fills) come from the view's display style (#301); the fit above
    // always uses the full wireframe bbox so switching styles never re-scales the view.
    let styled = styled_view_geometry(doc, view);
    for (pts, shade) in &styled.tris {
        let level = (shade.clamp(0.0, 1.0) * 255.0) as u8;
        let fill = Rgb(level, level, level);
        let s: Vec<(f32, f32)> = pts
            .iter()
            .map(|p| {
                let sp = to_screen(*p);
                (sp.x, sp.y)
            })
            .collect();
        canvas.poly(&s, fill);
    }
    for (a, b) in &styled.segments {
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, 1.2);
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
        canvas.text(
            pos.x,
            pos.y,
            12.0,
            Anchor::Middle,
            &crate::value::format_length_display_in((wa - wb).length(), unit),
        );
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
        canvas.text(sp.x, sp.y - 12.0, 12.0, Anchor::Middle, &format!("{angle:.0}°"));
    }
}

// ----- SVG backend -----

struct SvgCanvas {
    body: String,
}

fn svg_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn svg_color(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

impl Canvas for SvgCanvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32) {
        let fill = fill.map(svg_color).unwrap_or_else(|| "none".to_string());
        let stroke_attr = match stroke {
            Some(c) => format!(" stroke=\"{}\" stroke-width=\"{stroke_w}\"", svg_color(c)),
            None => String::new(),
        };
        self.body.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" fill=\"{fill}\"{stroke_attr}/>\n"
        ));
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        self.body.push_str(&format!(
            "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{}\" \
             stroke-width=\"{width}\"/>\n",
            svg_color(color)
        ));
    }

    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb) {
        // Stroked with its own fill so adjacent shaded triangles don't show hairline seams.
        let points: Vec<String> = pts.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
        self.body.push_str(&format!(
            "<polygon points=\"{}\" fill=\"{fill}\" stroke=\"{fill}\" stroke-width=\"0.6\"/>\n",
            points.join(" "),
            fill = svg_color(fill)
        ));
    }

    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str) {
        let anchor = match anchor {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
        };
        self.body.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"sans-serif\" font-size=\"{size}\" \
             fill=\"black\" text-anchor=\"{anchor}\">{}</text>\n",
            svg_esc(content)
        ));
    }
}

/// Render one drawing to a self-contained black-on-white SVG document. `None` if the drawing
/// index is missing or deleted.
pub fn drawing_to_svg(doc: &Document, index: usize) -> Option<String> {
    let (width, height) = page_dims(doc, index)?;
    let mut canvas = SvgCanvas { body: String::new() };
    render_drawing(doc, index, &mut canvas)?;
    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">\n"
    ));
    s.push_str(&canvas.body);
    s.push_str("</svg>\n");
    Some(s)
}

// ----- PDF backend -----

/// Accumulates a PDF content stream. PDF space is bottom-left origin, so `y` is flipped from
/// the top-left [`Canvas`] coordinates using the page `height`.
struct PdfCanvas {
    ops: Vec<u8>,
    height: f32,
}

impl PdfCanvas {
    fn new(height: f32) -> Self {
        PdfCanvas { ops: Vec::new(), height }
    }
    fn push(&mut self, s: &str) {
        self.ops.extend_from_slice(s.as_bytes());
    }
    fn set_fill(&mut self, c: Rgb) {
        self.push(&format!("{:.3} {:.3} {:.3} rg\n", c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0));
    }
    fn set_stroke(&mut self, c: Rgb) {
        self.push(&format!("{:.3} {:.3} {:.3} RG\n", c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0));
    }
}

/// Escape a string into PDF WinAnsi document bytes (the font is Helvetica/WinAnsiEncoding).
fn pdf_text_bytes(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in s.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push(b'\\');
                out.push(ch as u8);
            }
            '°' => out.push(0xB0),  // WinAnsi degree sign
            '—' | '–' => out.push(0x97), // em/en dash → WinAnsi em dash
            c if (c as u32) < 128 => out.push(c as u8),
            _ => out.push(b'?'),
        }
    }
    out
}

impl Canvas for PdfCanvas {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Rgb>, stroke: Option<Rgb>, stroke_w: f32) {
        // Top-left (x, y) with height h → PDF bottom-left corner is (x, H - y - h).
        let py = self.height - y - h;
        self.push(&format!("{x:.2} {py:.2} {w:.2} {h:.2} re\n"));
        match (fill, stroke) {
            (Some(f), Some(s)) => {
                self.set_fill(f);
                self.set_stroke(s);
                self.push(&format!("{stroke_w:.2} w\nB\n"));
            }
            (Some(f), None) => {
                self.set_fill(f);
                self.push("f\n");
            }
            (None, Some(s)) => {
                self.set_stroke(s);
                self.push(&format!("{stroke_w:.2} w\nS\n"));
            }
            (None, None) => self.push("n\n"),
        }
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Rgb, width: f32) {
        let (py1, py2) = (self.height - y1, self.height - y2);
        self.set_stroke(color);
        self.push(&format!("{width:.2} w\n{x1:.2} {py1:.2} m {x2:.2} {py2:.2} l S\n"));
    }

    fn poly(&mut self, pts: &[(f32, f32)], fill: Rgb) {
        let Some(((x0, y0), rest)) = pts.split_first() else {
            return;
        };
        // Fill *and* stroke with the same grey so adjacent shaded triangles don't show
        // hairline seams between them.
        self.set_fill(fill);
        self.set_stroke(fill);
        let mut path = format!("0.6 w\n{x0:.2} {:.2} m ", self.height - y0);
        for (x, y) in rest {
            path.push_str(&format!("{x:.2} {:.2} l ", self.height - y));
        }
        path.push_str("h b\n");
        self.push(&path);
    }

    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str) {
        // Helvetica averages ~0.5em per glyph; good enough to center dimension labels.
        let width = 0.5 * size * content.chars().count() as f32;
        let tx = match anchor {
            Anchor::Start => x,
            Anchor::Middle => x - width * 0.5,
        };
        let py = self.height - y;
        self.set_fill(BLACK);
        self.push(&format!("BT /F1 {size:.2} Tf {tx:.2} {py:.2} Td ("));
        let bytes = pdf_text_bytes(content);
        self.ops.extend_from_slice(&bytes);
        self.push(") Tj ET\n");
    }
}

/// Render one drawing to a self-contained single-page PDF (black-on-white, Helvetica text).
/// `None` if the drawing index is missing or deleted.
pub fn drawing_to_pdf(doc: &Document, index: usize) -> Option<Vec<u8>> {
    let (width, height) = page_dims(doc, index)?;
    let mut canvas = PdfCanvas::new(height);
    render_drawing(doc, index, &mut canvas)?;
    Some(assemble_pdf(width, height, &canvas.ops))
}

/// Wrap a content stream in a minimal single-page PDF document (catalog, pages, one page with
/// a Helvetica font, the content stream), with a correct cross-reference table.
fn assemble_pdf(width: f32, height: f32, content: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");

    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, &mut offsets, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    obj(
        &mut out,
        &mut offsets,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.2} {height:.2}] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        )
        .as_bytes(),
    );
    // Content stream object (4).
    {
        offsets.push(out.len());
        out.extend_from_slice(b"4 0 obj\n");
        out.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        out.extend_from_slice(content);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }
    obj(
        &mut out,
        &mut offsets,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );

    // Cross-reference table + trailer.
    let xref_pos = out.len();
    let count = offsets.len() + 1;
    out.extend_from_slice(format!("xref\n0 {count}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Drawing, DrawingView};

    fn doc_with_drawing() -> Document {
        let mut doc = Document::default();
        doc.drawings.push(Drawing {
            name: Some("Plate".to_string()),
            views: vec![DrawingView {
                body: 0,
                sketch: None,
                orientation: DrawingOrientation::Front,
                dimensioned_edges: Vec::new(),
                angle_dims: Vec::new(),
                scale: None,
                style: Default::default(),
                pos_x: 0.5,
                pos_y: 0.5,
            }],
            deleted: false,
            ..Default::default()
        });
        doc
    }

    #[test]
    fn svg_export_is_a_document() {
        let svg = drawing_to_svg(&doc_with_drawing(), 0).unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("Plate"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn pdf_export_is_a_single_page_document() {
        let pdf = drawing_to_pdf(&doc_with_drawing(), 0).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"), "has a PDF header");
        assert!(pdf.ends_with(b"%%EOF\n"), "ends at EOF marker");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Type /Catalog"));
        assert!(text.contains("/BaseFont /Helvetica"));
        assert!(text.contains("startxref"));
        // The title text is emitted into the content stream.
        assert!(text.contains("(Plate) Tj"));

        // Cross-reference integrity, checked against the RAW bytes (the content stream can
        // carry non-UTF-8 WinAnsi bytes, so string indices wouldn't match byte offsets):
        // startxref points at the `xref` table and every listed offset lands on its
        // `N 0 obj` — the easy thing to get wrong in a hand-rolled PDF.
        let start = parse_startxref(&pdf);
        assert_eq!(&pdf[start..start + 4], b"xref");
        let offsets = parse_xref_offsets(&pdf[start..]);
        assert_eq!(offsets.len(), 5, "five objects in the xref");
        for (i, off) in offsets.iter().enumerate() {
            let expect = format!("{} 0 obj", i + 1);
            assert!(
                pdf[*off..].starts_with(expect.as_bytes()),
                "xref offset {off} should point at '{expect}'"
            );
        }
    }

    /// The `startxref` byte offset from a PDF's trailer.
    fn parse_startxref(pdf: &[u8]) -> usize {
        let needle = b"startxref";
        let pos = pdf.windows(needle.len()).rposition(|w| w == needle).unwrap()
            + needle.len();
        let rest = &pdf[pos..];
        let digits: Vec<u8> = rest
            .iter()
            .skip_while(|b| b.is_ascii_whitespace())
            .take_while(|b| b.is_ascii_digit())
            .copied()
            .collect();
        String::from_utf8(digits).unwrap().parse().unwrap()
    }

    /// The object byte offsets listed in an xref table (the ` n ` entries, skipping the free
    /// object 0).
    fn parse_xref_offsets(table: &[u8]) -> Vec<usize> {
        String::from_utf8_lossy(table)
            .lines()
            .filter_map(|l| {
                let l = l.trim_end();
                (l.len() == 18 && l.ends_with(" n")).then(|| l[..10].parse().unwrap())
            })
            .collect()
    }

    /// #298: the exported page is the drawing's configured mm page in PDF points — the
    /// default is landscape US-Letter, 792 × 612 pt.
    #[test]
    fn pdf_page_matches_the_configured_page_size() {
        let doc = doc_with_drawing();
        let pdf = drawing_to_pdf(&doc, 0).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("/MediaBox [0 0 792.00 612.00]"),
            "default landscape-letter MediaBox, got: {}",
            text.lines().find(|l| l.contains("MediaBox")).unwrap_or("<none>")
        );

        let mut doc = doc;
        doc.drawings[0].page_width_mm = 210.0; // portrait A4
        doc.drawings[0].page_height_mm = 297.0;
        let pdf = drawing_to_pdf(&doc, 0).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        let media = text.lines().find(|l| l.contains("MediaBox")).unwrap().to_string();
        assert!(
            media.contains("[0 0 595.") && media.contains(" 841."),
            "A4 MediaBox in points, got: {media}"
        );
    }

    /// #297: exports are WYSIWYG — a view's card lands at its `pos_x`/`pos_y` page fraction,
    /// so two views placed apart export apart (not into a fixed grid).
    #[test]
    fn svg_places_views_at_their_page_positions() {
        let mut doc = doc_with_drawing();
        let mut second = doc.drawings[0].views[0].clone();
        doc.drawings[0].views[0].pos_x = 0.25;
        doc.drawings[0].views[0].pos_y = 0.3;
        second.pos_x = 0.75;
        second.pos_y = 0.7;
        doc.drawings[0].views.push(second);
        let svg = drawing_to_svg(&doc, 0).unwrap();
        let (page_w, page_h) = page_dims(&doc, 0).unwrap();
        // Card rects are the only GRAY-stroked rects; their x = pos_x*W - cell_w/2 + 3.
        let cell_w = page_w * CELL_FRAC;
        let cell_h = page_h * CELL_FRAC;
        for (px, py) in [(0.25f32, 0.3f32), (0.75, 0.7)] {
            let x = px * page_w - cell_w * 0.5 + 3.0;
            let y = py * page_h - cell_h * 0.5 + 3.0;
            let needle = format!("<rect x=\"{x:.1}\" y=\"{y:.1}\"");
            assert!(svg.contains(&needle), "expected a card at {needle}");
        }
    }

    #[test]
    fn missing_drawing_has_no_export() {
        let doc = Document::default();
        assert!(drawing_to_svg(&doc, 0).is_none());
        assert!(drawing_to_pdf(&doc, 0).is_none());
    }
}
