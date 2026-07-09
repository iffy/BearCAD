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

const CELL_W: f32 = 520.0;
const CELL_H: f32 = 380.0;
const HEADER: f32 = 48.0;
const MARGIN: f32 = 30.0;

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
    /// Text is always black in a drawing; `size` is the font size in px.
    fn text(&mut self, x: f32, y: f32, size: f32, anchor: Anchor, content: &str);
}

/// The page size (width, height) for a drawing, or `None` if the index is missing/deleted.
fn page_dims(doc: &Document, index: usize) -> Option<(f32, f32)> {
    let drawing = doc.drawings.get(index).filter(|d| !d.deleted)?;
    let n = drawing.views.len();
    let cols = if n <= 1 { 1 } else { 2 };
    let rows = n.div_ceil(cols).max(1);
    Some((cols as f32 * CELL_W, HEADER + rows as f32 * CELL_H))
}

/// Draw a whole drawing (title, per-view cells, projected edges, dimensions) into `canvas`.
fn render_drawing<C: Canvas>(doc: &Document, index: usize, canvas: &mut C) -> Option<()> {
    let drawing = doc.drawings.get(index).filter(|d| !d.deleted)?;
    let n = drawing.views.len();
    let cols = if n <= 1 { 1 } else { 2 };
    let (width, height) = page_dims(doc, index)?;
    let unit = doc.default_length_unit;

    canvas.rect(0.0, 0.0, width, height, Some(WHITE), None, 0.0);
    let title = drawing
        .name
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Drawing {index}"));
    canvas.text(MARGIN, 30.0, 20.0, Anchor::Start, &title);

    for (vi, view) in drawing.views.iter().enumerate() {
        let (col, row) = (vi % cols, vi / cols);
        let cell_x = col as f32 * CELL_W;
        let cell_y = HEADER + row as f32 * CELL_H;
        canvas.rect(cell_x + 3.0, cell_y + 3.0, CELL_W - 6.0, CELL_H - 6.0, None, Some(GRAY), 1.0);
        let label = format!(
            "{} — {}",
            crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Body(view.body)),
            view.orientation.label()
        );
        canvas.text(cell_x + 12.0, cell_y + 22.0, 13.0, Anchor::Start, &label);
        render_view_geometry(canvas, doc, view, cell_x, cell_y, unit);
    }
    Some(())
}

fn render_view_geometry<C: Canvas>(
    canvas: &mut C,
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
    // Model +up maps to screen -y (y grows downward).
    let to_screen = |p: glam::Vec2| {
        let d = (p - bbox_center) * scale;
        glam::Vec2::new(area_center.x + d.x, area_center.y - d.y)
    };

    for (a, b) in &proj {
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
                orientation: DrawingOrientation::Front,
                dimensioned_edges: Vec::new(),
                angle_dims: Vec::new(),
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

    #[test]
    fn missing_drawing_has_no_export() {
        let doc = Document::default();
        assert!(drawing_to_svg(&doc, 0).is_none());
        assert!(drawing_to_pdf(&doc, 0).is_none());
    }
}
