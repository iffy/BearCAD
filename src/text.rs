//! Font-outline extraction for the Text tool (#282).
//!
//! Turns a string + font selection into sketch-plane geometry: each glyph's outline becomes one
//! or more closed contours (flattened to polylines, in millimetres), laid out along the baseline
//! by each glyph's advance. Outer contours and holes (the counter of `o`, `a`, …) are both
//! returned; callers tell them apart by signed area / containment.
//!
//! [`fontdb`] enumerates and selects system fonts by family + style and yields the raw font bytes
//! (which the document embeds so text stays reproducible on a machine that lacks the font, like a
//! PDF). [`ttf_parser`] walks the selected face's glyph outlines.

use std::sync::{Mutex, OnceLock};

/// A closed glyph contour in millimetres (baseline at y=0, y up), first point not repeated.
pub type Contour = Vec<(f32, f32)>;

/// The shaped result of a string: every glyph contour of every line, already positioned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapedText {
    pub contours: Vec<Contour>,
}

/// Curve-flattening resolution: segments per quadratic/cubic bezier. Glyphs are small, so a
/// modest fixed count keeps outlines smooth without exploding the vertex count.
const BEZIER_STEPS: usize = 8;

/// Process-wide system font database, loaded once (enumeration is slow).
fn font_db() -> &'static Mutex<fontdb::Database> {
    static DB: OnceLock<Mutex<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Mutex::new(db)
    })
}

/// Sorted, de-duplicated list of installed font family names — for the family chooser (#282d).
pub fn system_font_families() -> Vec<String> {
    let db = font_db().lock().unwrap();
    let mut names: Vec<String> = db
        .faces()
        .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The raw bytes of the installed font best matching `family` + weight/italic, or `None` if no
/// font matches. These bytes are what the document embeds for portability.
pub fn font_bytes(family: &str, bold: bool, italic: bool) -> Option<Vec<u8>> {
    let db = font_db().lock().unwrap();
    let query = fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight: if bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
        stretch: fontdb::Stretch::Normal,
        style: if italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
    };
    let id = db.query(&query)?;
    db.with_face_data(id, |data, _index| data.to_vec())
}

/// Extract the outline of `text` set in the given font bytes at `size_mm` (cap-to-baseline scale
/// follows the font's units-per-em). Returns every glyph contour in millimetres, positioned along
/// the baseline (newlines start a new line below). `None` if the bytes aren't a parseable face.
pub fn outline_text(font_bytes: &[u8], size_mm: f32, text: &str) -> Option<ShapedText> {
    outline_text_wrapped(font_bytes, size_mm, text, None)
}

/// Like [`outline_text`], but when `wrap_width` is `Some(w)` (mm) the text **word-wraps** to
/// that width (#282): words that would overflow the line start a new line below, growing the
/// block downward. Explicit newlines still force line breaks. `None` = no wrapping (a single
/// growing line per input line, as before).
pub fn outline_text_wrapped(
    font_bytes: &[u8],
    size_mm: f32,
    text: &str,
    wrap_width: Option<f32>,
) -> Option<ShapedText> {
    let face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    let upem = face.units_per_em() as f32;
    if upem <= 0.0 {
        return None;
    }
    let scale = size_mm / upem;
    let line_height = (face.ascender() as f32 - face.descender() as f32 + face.line_gap() as f32) * scale;
    let advance = |ch: char| {
        let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
        face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale
    };
    let word_width = |w: &str| w.chars().map(advance).sum::<f32>();

    let mut out = ShapedText::default();
    let mut pen_x = 0.0f32;
    let mut pen_y = 0.0f32;
    let mut emit = |pen_x: &mut f32, pen_y: f32, ch: char| {
        let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
        let mut builder = OutlineCollector::new(scale, *pen_x, pen_y);
        face.outline_glyph(gid, &mut builder);
        builder.finish();
        out.contours.extend(builder.contours);
        *pen_x += advance(ch);
    };

    for (li, line) in text.split('\n').enumerate() {
        if li > 0 {
            pen_x = 0.0;
            pen_y -= line_height;
        }
        match wrap_width.filter(|w| *w > 0.0) {
            None => {
                for ch in line.chars() {
                    emit(&mut pen_x, pen_y, ch);
                }
            }
            Some(w) => {
                // Word-wrap: break before a word that would overflow (unless the line is empty).
                let mut first_on_line = true;
                for word in split_keep_spaces(line) {
                    let ww = word_width(&word);
                    let is_space = word.chars().all(|c| c.is_whitespace());
                    if !first_on_line && !is_space && pen_x + ww > w {
                        pen_x = 0.0;
                        pen_y -= line_height;
                        first_on_line = true;
                    }
                    // Drop leading spaces at the very start of a wrapped line.
                    if first_on_line && is_space {
                        continue;
                    }
                    for ch in word.chars() {
                        emit(&mut pen_x, pen_y, ch);
                    }
                    first_on_line = false;
                }
            }
        }
    }
    Some(out)
}

/// Split a line into alternating word / whitespace runs, preserving the spaces so advances line
/// up (#282). Used by the word-wrapping layout.
fn split_keep_spaces(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_space = None;
    for ch in line.chars() {
        let is_space = ch.is_whitespace();
        match cur_space {
            Some(s) if s == is_space => cur.push(ch),
            _ => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push(ch);
                cur_space = Some(is_space);
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Convenience: select a system font by family/style and outline `text`, returning the contours
/// *and* the embeddable font bytes. `None` if no matching font is installed or it can't be parsed.
pub fn shape_with_system_font(
    family: &str,
    bold: bool,
    italic: bool,
    size_mm: f32,
    text: &str,
) -> Option<(ShapedText, Vec<u8>)> {
    shape_with_system_font_wrapped(family, bold, italic, size_mm, text, None)
}

/// Like [`shape_with_system_font`], but honoring an optional `wrap_width` (mm) for word-wrap.
pub fn shape_with_system_font_wrapped(
    family: &str,
    bold: bool,
    italic: bool,
    size_mm: f32,
    text: &str,
    wrap_width: Option<f32>,
) -> Option<(ShapedText, Vec<u8>)> {
    let bytes = font_bytes(family, bold, italic)?;
    let shaped = outline_text_wrapped(&bytes, size_mm, text, wrap_width)?;
    Some((shaped, bytes))
}

/// [`ttf_parser::OutlineBuilder`] that flattens each glyph contour to a polyline in mm, offset to
/// the current pen position. Font outlines are y-up, matching the sketch's local frame.
struct OutlineCollector {
    scale: f32,
    ox: f32,
    oy: f32,
    contours: Vec<Contour>,
    current: Contour,
    last: (f32, f32),
}

impl OutlineCollector {
    fn new(scale: f32, ox: f32, oy: f32) -> Self {
        Self { scale, ox, oy, contours: Vec::new(), current: Vec::new(), last: (0.0, 0.0) }
    }
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (self.ox + x * self.scale, self.oy + y * self.scale)
    }
    fn flush(&mut self) {
        if self.current.len() >= 3 {
            self.contours.push(std::mem::take(&mut self.current));
        } else {
            self.current.clear();
        }
    }
    fn finish(&mut self) {
        self.flush();
    }
}

impl ttf_parser::OutlineBuilder for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        let p = self.map(x, y);
        self.last = p;
        self.current.push(p);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.last = p;
        self.current.push(p);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last;
        let c = self.map(x1, y1);
        let p1 = self.map(x, y);
        for k in 1..=BEZIER_STEPS {
            let t = k as f32 / BEZIER_STEPS as f32;
            let mt = 1.0 - t;
            let px = mt * mt * p0.0 + 2.0 * mt * t * c.0 + t * t * p1.0;
            let py = mt * mt * p0.1 + 2.0 * mt * t * c.1 + t * t * p1.1;
            self.current.push((px, py));
        }
        self.last = p1;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last;
        let c1 = self.map(x1, y1);
        let c2 = self.map(x2, y2);
        let p1 = self.map(x, y);
        for k in 1..=BEZIER_STEPS {
            let t = k as f32 / BEZIER_STEPS as f32;
            let mt = 1.0 - t;
            let px = mt * mt * mt * p0.0
                + 3.0 * mt * mt * t * c1.0
                + 3.0 * mt * t * t * c2.0
                + t * t * t * p1.0;
            let py = mt * mt * mt * p0.1
                + 3.0 * mt * mt * t * c1.1
                + 3.0 * mt * t * t * c2.1
                + t * t * t * p1.1;
            self.current.push((px, py));
        }
        self.last = p1;
    }
    fn close(&mut self) {
        self.flush();
    }
}

/// One fillable glyph region: an outer boundary loop plus its interior holes (counters).
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRegion {
    pub outer: Contour,
    pub holes: Vec<Contour>,
}

/// Group a text's flat contour list into glyph regions (#285): the larger-area loops are outer
/// boundaries; each smaller loop nests as a hole of the *smallest* outer that contains it (its
/// representative point falls inside). This turns `o`/`a`/`e` counters into holes so the glyph
/// extrudes hollow.
pub fn group_glyphs(contours: &[Contour]) -> Vec<GlyphRegion> {
    // Outer loops = those not contained in any other loop; sort by decreasing area so a hole is
    // matched to the tightest enclosing outer.
    let mut idx: Vec<usize> = (0..contours.len()).filter(|&i| contours[i].len() >= 3).collect();
    idx.sort_by(|&a, &b| {
        contour_signed_area(&contours[b])
            .abs()
            .partial_cmp(&contour_signed_area(&contours[a]).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut regions: Vec<GlyphRegion> = Vec::new();
    for &i in &idx {
        let rep = contour_point(&contours[i]);
        // Find the smallest existing region whose outer contains this loop's representative point.
        let mut best: Option<usize> = None;
        for (ri, region) in regions.iter().enumerate() {
            if point_in_contour(rep, &region.outer) {
                let smaller = best
                    .map(|b| contour_signed_area(&region.outer).abs()
                        < contour_signed_area(&regions[b].outer).abs())
                    .unwrap_or(true);
                if smaller {
                    best = Some(ri);
                }
            }
        }
        match best {
            Some(ri) => regions[ri].holes.push(contours[i].clone()),
            None => regions.push(GlyphRegion { outer: contours[i].clone(), holes: Vec::new() }),
        }
    }
    regions
}

/// A representative interior-ish point of a contour (its centroid); good enough for the nesting
/// test on convex-ish glyph loops.
fn contour_point(contour: &[(f32, f32)]) -> (f32, f32) {
    let n = contour.len().max(1) as f32;
    let (sx, sy) = contour.iter().fold((0.0, 0.0), |(ax, ay), &(x, y)| (ax + x, ay + y));
    (sx / n, sy / n)
}

/// Even-odd point-in-polygon test (winding-independent).
fn point_in_contour(p: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > p.1) != (yj > p.1)
            && p.0 < (xj - xi) * (p.1 - yi) / (yj - yi) + xi
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Signed area (shoelace) of a contour; sign encodes winding (used to tell outer loops from holes).
pub fn contour_signed_area(contour: &[(f32, f32)]) -> f32 {
    let n = contour.len();
    let mut a = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        a += contour[i].0 * contour[j].1 - contour[j].0 * contour[i].1;
    }
    a * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain text font's bytes, or skip (CI without fonts). Prefers common sans/serif families so
    /// the glyph shapes are predictable; falls back to the first parseable installed font.
    fn any_font() -> Option<Vec<u8>> {
        for fam in ["Helvetica", "Arial", "Times New Roman", "DejaVu Sans", "Liberation Sans"] {
            if let Some(b) = font_bytes(fam, false, false) {
                if ttf_parser::Face::parse(&b, 0).is_ok() {
                    return Some(b);
                }
            }
        }
        for fam in system_font_families() {
            if let Some(b) = font_bytes(&fam, false, false) {
                if ttf_parser::Face::parse(&b, 0).is_ok() {
                    return Some(b);
                }
            }
        }
        None
    }

    /// #282: word-wrapping to a narrow width pushes later words onto new lines, so the block
    /// gets taller (more negative min-y) and no wider than the wrap width.
    #[test]
    fn wrapping_breaks_words_onto_new_lines() {
        let Some(bytes) = any_font() else {
            eprintln!("no system fonts; skipping");
            return;
        };
        let text = "one two three four five";
        let unwrapped = outline_text(&bytes, 8.0, text).expect("unwrapped");
        // Width of the whole phrase on one line.
        let max_x = |s: &ShapedText| {
            s.contours.iter().flatten().map(|p| p.0).fold(f32::MIN, f32::max)
        };
        let min_y = |s: &ShapedText| {
            s.contours.iter().flatten().map(|p| p.1).fold(f32::MAX, f32::min)
        };
        let full_width = max_x(&unwrapped);
        assert!(full_width > 20.0, "phrase should be reasonably wide");
        // Wrap to a third of that: it must break onto multiple lines (taller, narrower).
        let wrapped =
            outline_text_wrapped(&bytes, 8.0, text, Some(full_width / 3.0)).expect("wrapped");
        assert!(
            max_x(&wrapped) <= full_width * 0.7 + 1.0,
            "wrapped block is narrower ({} vs {full_width})",
            max_x(&wrapped)
        );
        assert!(
            min_y(&wrapped) < min_y(&unwrapped) - 1.0,
            "wrapped block is taller (grows downward)"
        );
    }

    #[test]
    fn capital_h_has_one_contour_with_size() {
        let Some(bytes) = any_font() else {
            eprintln!("no system fonts; skipping");
            return;
        };
        let shaped = outline_text(&bytes, 10.0, "H").expect("outline H");
        assert_eq!(shaped.contours.len(), 1, "H is a single contour");
        let ys: Vec<f32> = shaped.contours[0].iter().map(|p| p.1).collect();
        let height = ys.iter().cloned().fold(f32::MIN, f32::max)
            - ys.iter().cloned().fold(f32::MAX, f32::min);
        // A 10mm cap-height-ish letter should be a few mm tall, well under the em size.
        assert!(height > 2.0 && height < 12.0, "H height {height} out of range");
    }

    #[test]
    fn letter_o_has_a_hole() {
        let Some(bytes) = any_font() else {
            eprintln!("no system fonts; skipping");
            return;
        };
        let shaped = outline_text(&bytes, 10.0, "o").expect("outline o");
        assert_eq!(shaped.contours.len(), 2, "o is an outer ring plus a counter (hole)");
        // The two contours wind oppositely (outer vs hole).
        let a0 = contour_signed_area(&shaped.contours[0]);
        let a1 = contour_signed_area(&shaped.contours[1]);
        assert!(a0 * a1 < 0.0, "outer and hole wind oppositely ({a0}, {a1})");
    }

    #[test]
    fn group_glyphs_makes_o_a_ring() {
        let Some(bytes) = any_font() else {
            return;
        };
        let shaped = outline_text(&bytes, 10.0, "o").expect("o");
        let regions = group_glyphs(&shaped.contours);
        assert_eq!(regions.len(), 1, "o is one glyph region");
        assert_eq!(regions[0].holes.len(), 1, "with one counter (hole)");
    }

    #[test]
    fn advance_lays_glyphs_left_to_right() {
        let Some(bytes) = any_font() else {
            return;
        };
        let one = outline_text(&bytes, 10.0, "H").expect("H");
        let two = outline_text(&bytes, 10.0, "HH").expect("HH");
        let max_x = |s: &ShapedText| {
            s.contours.iter().flatten().map(|p| p.0).fold(f32::MIN, f32::max)
        };
        assert!(max_x(&two) > max_x(&one) + 1.0, "second H sits to the right of the first");
    }
}
