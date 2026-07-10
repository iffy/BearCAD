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

/// The orientation of an aligned child projection (#296) placed in `dir` relative to a parent
/// showing `parent`. Derived by the glass-box unfolding: the child shares one of the parent's
/// screen axes and rotates 90° about it. `None` if the result isn't one of the six orthographic
/// views (e.g. the parent is Isometric), so alignment is offered only for orthographic parents.
pub fn aligned_child_orientation(
    parent: DrawingOrientation,
    dir: crate::model::AlignDir,
) -> Option<DrawingOrientation> {
    use crate::model::AlignDir;
    let (r, u) = view_axes(parent);
    let o = r.cross(u); // "into the page" for this view basis
    let (cr, cu) = match dir {
        AlignDir::Below => (r, -o),
        AlignDir::Above => (r, o),
        AlignDir::Right => (-o, u),
        AlignDir::Left => (o, u),
    };
    orientation_from_axes(cr, cu)
}

/// The on-page position of a view (#296), resolving an aligned child's shared axis to its
/// parent's so the two always line up regardless of which was dragged. Non-aligned views (and
/// children whose parent is gone) return their own stored `(pos_x, pos_y)`.
pub fn resolved_view_pos(doc: &Document, drawing: usize, view: usize) -> (f32, f32) {
    let Some(d) = doc.drawings.get(drawing).filter(|d| !d.deleted) else {
        return (0.5, 0.5);
    };
    let Some(v) = d.views.get(view) else {
        return (0.5, 0.5);
    };
    match (v.aligned_parent, v.aligned_dir) {
        (Some(p), Some(dir)) if p != view => {
            if let Some(parent) = d.views.get(p) {
                // Resolve the parent recursively so chains of aligned views stay consistent.
                let (px, py) = resolved_view_pos(doc, drawing, p);
                let _ = parent;
                if dir.shares_pos_x() {
                    return (px, v.pos_y);
                } else {
                    return (v.pos_x, py);
                }
            }
            (v.pos_x, v.pos_y)
        }
        _ => (v.pos_x, v.pos_y),
    }
}

/// A view's effective print scale (#296/#300): an aligned child inherits its parent's scale
/// (walking the chain), so a whole aligned group prints at one scale. Non-aligned views use
/// their own `scale`.
pub fn resolved_view_scale(doc: &Document, drawing: usize, view: usize) -> Option<String> {
    let d = doc.drawings.get(drawing).filter(|d| !d.deleted)?;
    let v = d.views.get(view)?;
    match v.aligned_parent {
        Some(p) if p != view && d.views.get(p).is_some() => {
            resolved_view_scale(doc, drawing, p)
        }
        _ => v.scale.clone(),
    }
}

/// Match a `(right, up)` axis pair back to one of the six orthographic [`DrawingOrientation`]s.
fn orientation_from_axes(right: Vec3, up: Vec3) -> Option<DrawingOrientation> {
    use DrawingOrientation as O;
    const ALL: [O; 6] = [O::Front, O::Back, O::Left, O::Right, O::Top, O::Bottom];
    ALL.into_iter().find(|&o| {
        let (r, u) = view_axes(o);
        (r - right).length() < 1e-3 && (u - up).length() < 1e-3
    })
}

fn edge_key(a: Vec3, b: Vec3) -> crate::model::DrawingEdgeKey {
    crate::model::normalized_edge_key(
        crate::hierarchy::quantize_body_point(a),
        crate::hierarchy::quantize_body_point(b),
    )
}

/// A circle detected among a view's projected feature edges (#313): a tessellated curve
/// (cylinder rim, extruded-circle boundary) that should render as one smooth circle and carry
/// a single diameter dimension rather than a dimension per short segment.
pub struct DetectedCircle {
    pub center: glam::Vec2,
    pub radius: f32,
}

/// Classify a view's projected 2D feature edges (#313): find tessellated circles (closed loops
/// of short segments that fit a circle) and report which edges belong to them, so the renderer
/// can draw them smooth and dimension only the diameter. Straight edges are everything else.
pub fn classify_projected_circles(edges: &[(glam::Vec2, glam::Vec2)]) -> Vec<DetectedCircle> {
    use std::collections::HashMap;
    // Quantize endpoints (0.001 mm) so shared vertices merge into one index.
    let q = |p: glam::Vec2| (((p.x * 1000.0).round()) as i64, ((p.y * 1000.0).round()) as i64);
    let mut index_of: HashMap<(i64, i64), usize> = HashMap::new();
    let mut verts: Vec<glam::Vec2> = Vec::new();
    let vid = |p: glam::Vec2, index_of: &mut HashMap<(i64, i64), usize>, verts: &mut Vec<glam::Vec2>| {
        *index_of.entry(q(p)).or_insert_with(|| {
            verts.push(p);
            verts.len() - 1
        })
    };
    // Edge endpoints as vertex indices; per-vertex degree and adjacency. Overlapping edges
    // (e.g. a cylinder's top and bottom rim projecting onto the same circle in a face-on view)
    // are deduplicated so shared vertices keep degree 2 rather than 4.
    let mut e_verts: Vec<(usize, usize)> = Vec::with_capacity(edges.len());
    let mut seen_pairs: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for &(a, b) in edges {
        let ia = vid(a, &mut index_of, &mut verts);
        let ib = vid(b, &mut index_of, &mut verts);
        let pair = if ia <= ib { (ia, ib) } else { (ib, ia) };
        if ia != ib && !seen_pairs.insert(pair) {
            continue; // a duplicate/overlapping edge
        }
        e_verts.push((ia, ib));
    }
    let n = verts.len();
    let mut degree = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n]; // adjacent edge indices
    for (ei, &(a, b)) in e_verts.iter().enumerate() {
        if a == b {
            continue;
        }
        degree[a] += 1;
        degree[b] += 1;
        adj[a].push(ei);
        adj[b].push(ei);
    }
    // Connected components over edges where every touched vertex has degree exactly 2 — a clean
    // cycle (a tessellated circle). Any degree != 2 disqualifies the component.
    let mut seen = vec![false; e_verts.len()];
    let mut circles = Vec::new();
    for start in 0..e_verts.len() {
        if seen[start] || e_verts[start].0 == e_verts[start].1 {
            continue;
        }
        // BFS over connected edges.
        let mut stack = vec![start];
        let mut comp_edges = Vec::new();
        let mut comp_verts: Vec<usize> = Vec::new();
        let mut clean = true;
        seen[start] = true;
        while let Some(ei) = stack.pop() {
            comp_edges.push(ei);
            for &v in [e_verts[ei].0, e_verts[ei].1].iter() {
                if degree[v] != 2 {
                    clean = false;
                }
                comp_verts.push(v);
                for &ne in &adj[v] {
                    if !seen[ne] {
                        seen[ne] = true;
                        stack.push(ne);
                    }
                }
            }
        }
        comp_verts.sort_unstable();
        comp_verts.dedup();
        // A circle: a clean cycle with enough segments and a tight radius fit around the centroid.
        if !clean || comp_edges.len() < 8 || comp_verts.len() != comp_edges.len() {
            continue;
        }
        let center = comp_verts.iter().map(|&v| verts[v]).sum::<glam::Vec2>()
            / comp_verts.len() as f32;
        let radii: Vec<f32> = comp_verts.iter().map(|&v| (verts[v] - center).length()).collect();
        let mean_r = radii.iter().sum::<f32>() / radii.len() as f32;
        if mean_r < 1e-3 {
            continue;
        }
        let max_dev = radii.iter().map(|r| (r - mean_r).abs()).fold(0.0f32, f32::max);
        if max_dev <= mean_r * 0.06 {
            circles.push(DetectedCircle { center, radius: mean_r });
        }
    }
    circles
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

/// The architectural dimension-line geometry for one edge (#294), all in the view's projected
/// 2D mm space. `a`/`b` are the edge endpoints; `outward` is the unit perpendicular pointing
/// away from the geometry centroid; `offset` is how far out along `outward` the dimension line
/// sits. Both the editor and the exports build their strokes from this so they never drift.
pub struct DimLineGeometry {
    /// The two extension lines (edge endpoint → just past the dimension line).
    pub extensions: [(glam::Vec2, glam::Vec2); 2],
    /// The dimension line itself (endpoint to endpoint, parallel to the edge).
    pub line: (glam::Vec2, glam::Vec2),
    /// Two arrowhead triangles (three points each) at the dimension line's ends.
    pub arrows: [[glam::Vec2; 3]; 2],
    /// Where the measurement label sits (centre of the dimension line, nudged outward).
    pub label_pos: glam::Vec2,
}

/// Build [`DimLineGeometry`] for an edge from `a` to `b`, offset `outward * offset` from it.
/// `arrow` is the arrowhead length in the same units, so callers can size features to the
/// drawing (a proportional fraction of the projected extent keeps them readable at any scale).
pub fn dimension_line_geometry(
    a: glam::Vec2,
    b: glam::Vec2,
    outward: glam::Vec2,
    offset: f32,
    arrow: f32,
) -> DimLineGeometry {
    let da = a + outward * offset;
    let db = b + outward * offset;
    let along = (db - da).normalize_or_zero();
    // Arrowheads point outward from the line centre toward each end.
    let head = |tip: glam::Vec2, dir: glam::Vec2| {
        let base = tip - dir * arrow;
        let side = glam::Vec2::new(-dir.y, dir.x) * (arrow * 0.4);
        [tip, base + side, base - side]
    };
    DimLineGeometry {
        // Extension lines start a hair off the edge and overshoot the dimension line a touch.
        extensions: [
            (a + outward * (arrow * 0.4), da + outward * (arrow * 0.7)),
            (b + outward * (arrow * 0.4), db + outward * (arrow * 0.7)),
        ],
        line: (da, db),
        arrows: [head(da, -along), head(db, along)],
        label_pos: (da + db) * 0.5 + outward * (arrow * 1.6),
    }
}

/// Whether a projected segment lies on one of the detected circles (#313): both endpoints
/// within tolerance of that circle's radius from its centre.
pub fn segment_on_circle(a: glam::Vec2, b: glam::Vec2, circles: &[DetectedCircle]) -> bool {
    circles.iter().any(|c| {
        let tol = c.radius * 0.08 + 1e-3;
        ((a - c.center).length() - c.radius).abs() < tol
            && ((b - c.center).length() - c.radius).abs() < tol
    })
}

/// The outward unit perpendicular for an edge's dimension line: the side of the edge facing
/// away from the geometry centroid `center` (#294), so labels sit outside the part.
pub fn dimension_outward(a: glam::Vec2, b: glam::Vec2, center: glam::Vec2) -> glam::Vec2 {
    let seg = b - a;
    let mut perp = glam::Vec2::new(-seg.y, seg.x).normalize_or_zero();
    if perp == glam::Vec2::ZERO {
        perp = glam::Vec2::new(0.0, -1.0);
    }
    let mid = (a + b) * 0.5;
    if perp.dot(mid - center) < 0.0 {
        perp = -perp;
    }
    perp
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
    /// A stroked (unfilled) circle outline — a smooth detected curve (#313).
    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32);
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
    for (vi, view) in drawing.views.iter().enumerate() {
        // Aligned children (#296) resolve their shared axis to the parent's.
        let (px, py) = resolved_view_pos(doc, index, vi);
        let cell_x = px * width - cell_w * 0.5;
        let cell_y = py * height - cell_h * 0.5;
        canvas.rect(cell_x + 3.0, cell_y + 3.0, cell_w - 6.0, cell_h - 6.0, None, Some(GRAY), 1.0);
        let source = match view.sketch {
            Some(si) => crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Sketch(si)),
            None => crate::names::node_label(doc, crate::hierarchy::HierarchyNode::Body(view.body)),
        };
        // An aligned child inherits its parent's scale (#296/#300).
        let scale_text = resolved_view_scale(doc, index, vi);
        let scale_suffix = scale_text
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let label = format!("{source} — {}{scale_suffix}", view.orientation.label());
        canvas.text(cell_x + CELL_PAD, cell_y + 20.0, 11.0, Anchor::Start, &label);
        render_view_geometry(
            canvas,
            doc,
            view,
            scale_text.as_deref(),
            cell_x,
            cell_y,
            cell_w,
            cell_h,
            unit,
        );
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn render_view_geometry<C: Canvas>(
    canvas: &mut C,
    doc: &Document,
    view: &DrawingView,
    scale_text: Option<&str>,
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
    let scale = match scale_text.and_then(crate::model::parse_drawing_scale) {
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

    // Detect tessellated circles (#313): render them smooth and skip their many short segments
    // in both the stroke and the per-edge dimensions; each circle gets one diameter dimension.
    let circles = classify_projected_circles(&proj);

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
        // A segment lying on a detected circle is drawn as part of the smooth circle instead.
        if segment_on_circle(*a, *b, &circles) {
            continue;
        }
        let (sa, sb) = (to_screen(*a), to_screen(*b));
        canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, 1.2);
    }
    // Smooth detected circles.
    for c in &circles {
        let sc = to_screen(c.center);
        canvas.circle(sc.x, sc.y, c.radius * scale, BLACK, 1.2);
    }

    // Length dimensions (#294): architectural dimension lines — extension lines, an offset
    // dimension line with arrowheads, and the measured length centred on it. Sizes are a
    // fraction of the projected extent so they read at any scale; a per-edge override
    // (dimension_offsets) pushes the line further out.
    let diag = extent.length().max(1.0);
    let default_gap = diag * 0.05;
    let arrow = diag * 0.025;
    // A single diameter dimension per detected circle (#313), replacing its segments' dims.
    for c in &circles {
        let dir = glam::Vec2::new(0.70710677, -0.70710677); // 45° so it clears the extents
        let a = c.center - dir * c.radius;
        let b = c.center + dir * c.radius;
        let sa = to_screen(a);
        let sb = to_screen(b);
        let mid = (sa + sb) * 0.5;
        canvas.line(sa.x, sa.y, sb.x, sb.y, BLACK, 0.8);
        // Diameter value in the view's world scale (radius is already world mm).
        let d = crate::value::format_length_display_in(c.radius * 2.0, unit);
        canvas.text(mid.x, mid.y - 3.0, 11.0, Anchor::Middle, &format!("⌀{d}"));
    }
    for (i, (a, b)) in proj.iter().enumerate() {
        let (wa, wb) = world_edges[i];
        let key = edge_key(wa, wb);
        // An edge-on edge projects to a point — nothing meaningful to dimension here (#294) —
        // and circle segments are covered by the single diameter dimension above (#313).
        if !view.dimensioned_edges.contains(&key)
            || (*b - *a).length() < 1e-3
            || segment_on_circle(*a, *b, &circles)
        {
            continue;
        }
        let outward = dimension_outward(*a, *b, bbox_center);
        let extra = view
            .dimension_offsets
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, o)| *o)
            .unwrap_or(0.0);
        let geom = dimension_line_geometry(*a, *b, outward, default_gap + extra, arrow);
        let stroke_line = |canvas: &mut C, p: glam::Vec2, q: glam::Vec2| {
            let (sp, sq) = (to_screen(p), to_screen(q));
            canvas.line(sp.x, sp.y, sq.x, sq.y, BLACK, 0.8);
        };
        for (p, q) in geom.extensions {
            stroke_line(canvas, p, q);
        }
        stroke_line(canvas, geom.line.0, geom.line.1);
        for tri in geom.arrows {
            let pts: Vec<(f32, f32)> = tri
                .iter()
                .map(|p| {
                    let s = to_screen(*p);
                    (s.x, s.y)
                })
                .collect();
            canvas.poly(&pts, BLACK);
        }
        let lp = to_screen(geom.label_pos);
        canvas.text(
            lp.x,
            lp.y,
            11.0,
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

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32) {
        self.body.push_str(&format!(
            "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"none\" stroke=\"{}\" \
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
            'Ø' | '⌀' => out.push(0xD8), // diameter → WinAnsi Ø (Latin O with stroke)
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

    fn circle(&mut self, cx: f32, cy: f32, r: f32, color: Rgb, width: f32) {
        // Four cubic Bézier arcs (kappa ≈ 0.5523) approximate a circle; y flips to PDF space.
        let k = 0.552_284_75 * r;
        let cy = self.height - cy;
        self.set_stroke(color);
        let mut path = format!("{width:.2} w\n{:.2} {cy:.2} m ", cx + r);
        // Right → top → left → bottom, counter-clockwise in PDF's y-up space.
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx + r, cy + k, cx + k, cy + r, cx, cy + r));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx - k, cy + r, cx - r, cy + k, cx - r, cy));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx - r, cy - k, cx - k, cy - r, cx, cy - r));
        path.push_str(&format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", cx + k, cy - r, cx + r, cy - k, cx + r, cy));
        path.push_str("S\n");
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

    /// #313: a tessellated circle loop is detected (centre/radius); a run of straight edges is
    /// not, so it stays a set of dimensionable segments.
    #[test]
    fn detects_a_tessellated_circle_but_not_straight_edges() {
        // A 32-gon of radius 10 centred at (5, 3).
        let n = 32;
        let c = glam::Vec2::new(5.0, 3.0);
        let r = 10.0;
        let pts: Vec<glam::Vec2> = (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                c + glam::Vec2::new(a.cos(), a.sin()) * r
            })
            .collect();
        let mut edges: Vec<(glam::Vec2, glam::Vec2)> = (0..n)
            .map(|i| (pts[i], pts[(i + 1) % n]))
            .collect();
        // Plus a separate square (straight edges), which must NOT be a circle.
        let sq = [
            glam::Vec2::new(40.0, 0.0),
            glam::Vec2::new(50.0, 0.0),
            glam::Vec2::new(50.0, 10.0),
            glam::Vec2::new(40.0, 10.0),
        ];
        for i in 0..4 {
            edges.push((sq[i], sq[(i + 1) % 4]));
        }
        let circles = classify_projected_circles(&edges);
        assert_eq!(circles.len(), 1, "one circle detected (the 32-gon, not the square)");
        assert!((circles[0].center - c).length() < 0.2);
        assert!((circles[0].radius - r).abs() < 0.2);
        // The square's edges are not on the circle.
        for i in 0..4 {
            assert!(!segment_on_circle(sq[i], sq[(i + 1) % 4], &circles));
        }
    }

    /// #296: a Front parent's aligned children follow the issue's mapping — down→Bottom,
    /// up→Top, right→Right, left→Left — and an isometric parent has no orthographic child.
    #[test]
    fn aligned_children_of_front_follow_the_screen_direction() {
        use crate::model::AlignDir;
        use DrawingOrientation as O;
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Below), Some(O::Bottom));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Above), Some(O::Top));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Right), Some(O::Right));
        assert_eq!(aligned_child_orientation(O::Front, AlignDir::Left), Some(O::Left));
        // The four upright views (up = +Z) neighbour each other around the vertical axis, so
        // their left/right children are always canonical orthographic views.
        for parent in [O::Front, O::Back, O::Left, O::Right] {
            assert!(aligned_child_orientation(parent, AlignDir::Right).is_some(), "{parent:?}");
            assert!(aligned_child_orientation(parent, AlignDir::Left).is_some(), "{parent:?}");
        }
        // Directions whose unfolded view would need a rolled (non-canonical) up simply have no
        // aligned child, and an isometric parent never resolves — the tool just won't offer it.
        assert_eq!(aligned_child_orientation(O::Isometric, AlignDir::Below), None);
    }

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
                dimension_offsets: Vec::new(),
                aligned_parent: None,
                aligned_dir: None,
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
