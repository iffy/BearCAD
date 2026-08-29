//! Drawing by hand (#1805/#1809): the strokes, tones and hatching that the `LoosePencil` and
//! `ColourPencil` shading modes and the drawings workbench's pencil style share.
//!
//! Everything here is deliberately **repeatable**: a wobble re-rolled every frame would make
//! the whole drawing crawl as the camera moved, which is unusable. Each stroke's shape comes
//! from a hash of its own endpoints, so the same edge is drawn the same way from every angle
//! and on every redraw, and two edges that meet at a corner still wobble differently.

use eframe::egui::Color32;
use glam::{Vec2, Vec3};

/// Paper the `LoosePencil` scene is drawn on: warm white, not the theme's near-black.
pub const PENCIL_PAPER: Color32 = Color32::from_rgb(250, 249, 245);
/// Graphite: a soft blue-black, never pure black — pure black reads as ink, not pencil.
pub const PENCIL_GRAPHITE: Color32 = Color32::from_rgb(58, 60, 68);
/// The faint tone inside a body, so a near edge hides a far one without the fill reading as
/// paint. A hair off the paper, which is what a pencil drawing's enclosed areas look like.
pub const PENCIL_BODY_FILL: Color32 = Color32::from_rgb(240, 239, 234);
/// Ruled guide lines on the paper: the ground grid, barely there.
pub const PENCIL_GRID: Color32 = Color32::from_rgb(222, 220, 212);
pub const PENCIL_GRID_AXIS: Color32 = Color32::from_rgb(196, 193, 184);
/// Coloured pencil for the world axes: the same three hues, muted to sit on paper.
pub const PENCIL_X_AXIS: Color32 = Color32::from_rgb(184, 96, 92);
pub const PENCIL_Y_AXIS: Color32 = Color32::from_rgb(104, 152, 104);
pub const PENCIL_Z_AXIS: Color32 = Color32::from_rgb(96, 122, 176);
/// A pencil outline carries the drawing, so it is heavier than the thin technical wireframe
/// overlay (#1810).
pub const PENCIL_LINE_WIDTH_PX: f32 = 2.1;
/// How many times a stroke is gone over. Two is the hand-drawn tell; more turns to mud.
pub const PENCIL_PASSES: usize = 2;
/// Joints along one stroke. Each one is nudged sideways, so the line bows rather than bends.
pub const PENCIL_STROKE_STEPS: usize = 5;
/// Sideways wobble, as a fraction of the stroke's own length, capped so a long edge bows the
/// same visible amount as a short one rather than swinging wildly.
pub const PENCIL_WOBBLE: f32 = 0.02;
pub const PENCIL_WOBBLE_MAX_MM: f32 = 0.9;
/// How far a stroke runs past the corner it should stop at — the other hand-drawn tell.
pub const PENCIL_OVERSHOOT: f32 = 0.035;
pub const PENCIL_OVERSHOOT_MAX_MM: f32 = 1.6;
/// Spacing of the hatch strokes that stand in for a contact shadow, in world mm.
pub const PENCIL_HATCH_SPACING_MM: f32 = 2.2;
/// Hatch strokes run at this angle in the ground plane — off-axis, the way a hand shades.
pub const PENCIL_HATCH_ANGLE_RAD: f32 = 0.6;
pub const PENCIL_HATCH_WIDTH_PX: f32 = 1.3;
/// Graphite laid down lightly — premultiplied, so it composites as a ~45%-coverage stroke.
pub const PENCIL_HATCH_COLOR: Color32 = Color32::from_rgba_premultiplied(26, 27, 31, 118);

/// Laying colour on a face with a coloured pencil (#1818/#1825).
///
/// Not a flat fill — that reads as paint. The colour is *scribbled* in: ruled strokes this far
/// apart, run past the outline a little and broken by gaps of bare paper, the way a hand
/// filling a shape quickly does. The spacing is one number and not a function of the light
/// (#1825): a coloured pencil drawing gets its form from its outlines, so every side of a
/// solid is laid on the same, exactly as the plain pencil mode does it.
pub const PENCIL_SCRIBBLE_SPACING_MM: f32 = 1.5;
pub const PENCIL_SHADE_WIDTH_PX: f32 = 2.8;
/// Coverage of one scribble stroke. Light: the tone comes from laying many side by side.
pub const PENCIL_SHADE_ALPHA: f32 = 0.8;
/// How far a scribble runs past the end of its span, as a fraction of that span and capped in
/// mm — the "outside the lines" of a quick fill.
pub const PENCIL_SCRIBBLE_OVERSHOOT: f32 = 0.09;
pub const PENCIL_SCRIBBLE_OVERSHOOT_MAX_MM: f32 = 3.0;
/// Roughly how much of a span the pencil actually lands on. The rest is the gaps.
pub const PENCIL_SCRIBBLE_COVERAGE: f32 = 0.68;
/// A landed piece is this fraction of the span, give or take — small enough that a face gets
/// several per line, big enough that it reads as a stroke rather than a dotted line.
const PENCIL_SCRIBBLE_PIECE: f32 = 0.30;
const PENCIL_SCRIBBLE_PIECE_JITTER: f32 = 0.22;
/// Guard: a degenerate span must not ask for an unbounded number of pieces.
const PENCIL_SCRIBBLE_MAX_PIECES: usize = 48;
/// Spacing of the hatch a body's shadow lays on the face it falls on (#1818) — a touch tighter
/// than the ground hatch, so a shadow on a part reads as darker than one on the paper.
pub const PENCIL_CAST_SPACING_MM: f32 = 1.7;
/// …and turned well across the strokes shading that face, so the two read as separate layers
/// rather than as one heavier tone.
pub const PENCIL_CAST_TURN_RAD: f32 = 1.72;

/// How a face's own colour reads when it is *scribbled* on with a coloured pencil (#1818): the
/// body colour deepened a little toward graphite, so a stroke reads as pencil pressure rather
/// than as a brighter version of the fill underneath it.
pub fn shading_tone(base: Color32) -> Color32 {
    mix(base, PENCIL_GRAPHITE, 0.16)
}

/// One ruled span, as the pieces a quick scribble actually leaves (#1825).
///
/// A hand filling a shape lifts and re-lands, and runs past the outline on the way — so the
/// line is not continuous, the paper shows through, and the colour sits a little outside the
/// lines. Every piece is keyed to the span's own endpoints, so the same face scribbles the
/// same way from every angle and on every redraw; re-rolling it per frame would make the whole
/// drawing crawl as the camera moved.
pub fn scribble(a: Vec3, b: Vec3, pass: usize) -> Vec<(Vec3, Vec3)> {
    let along = b - a;
    let length = along.length();
    if length < 1e-6 {
        return Vec::new();
    }
    let dir = along / length;
    let over = (length * PENCIL_SCRIBBLE_OVERSHOOT).min(PENCIL_SCRIBBLE_OVERSHOOT_MAX_MM);
    // `noise` is -1..1; the ends run *out*, never in, so a span never falls short of its own
    // shape — a scribble overshoots, it does not leave a margin.
    let mut at = -over * noise(seed(a, b, usize::MAX, pass)).abs();
    let end = length + over * noise(seed(b, a, usize::MAX, pass)).abs();

    let mut out = Vec::new();
    for i in 0..PENCIL_SCRIBBLE_MAX_PIECES {
        if at >= end {
            break;
        }
        let s = seed(a, b, i, pass);
        let piece =
            length * (PENCIL_SCRIBBLE_PIECE + PENCIL_SCRIBBLE_PIECE_JITTER * noise(s).abs());
        let next = (at + piece).min(end);
        // Whether the pencil was down for this piece. Above the coverage threshold it lifted,
        // which is where the bare paper comes from.
        if noise(s ^ 0x5F35_6495).abs() < PENCIL_SCRIBBLE_COVERAGE && next - at > 1e-4 {
            out.push((a + dir * at, a + dir * next));
        }
        // A short lift before the next piece, sometimes barely any.
        at = next + length * 0.04 * noise(s ^ 0x9E37_79B9).abs();
    }
    out
}

/// Blend two colours, `t` of the way from `a` to `b`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp =
        |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// The ground tone a coloured-pencil body is filled with (#1825): its own colour taken most of
/// the way to the paper. It is only there so a near face hides a far one — what the eye reads
/// as the colour is the scribble laid over it, and the gaps in that scribble have to read as
/// bare paper, not as a lighter wash of the same colour.
pub fn scribble_ground(base: Color32) -> Color32 {
    mix(base, PENCIL_PAPER, 0.88)
}

/// How a body's own colour reads in coloured pencil (#1812): the fill it is laid on with,
/// and the darker tone of the same colour its outline is drawn in.
///
/// A coloured pencil does not cover the paper — the fill is the body colour let a long way
/// down toward [`PENCIL_PAPER`]. The outline is the same colour pressed harder, mixed toward
/// graphite so it still reads as a drawn line rather than a bright edge.
pub fn colour_tones(base: Color32) -> (Color32, Color32) {
    (mix(base, PENCIL_PAPER, 0.72), mix(base, PENCIL_GRAPHITE, 0.55))
}

/// A repeatable number in `-1..1` from an integer seed. The point is repeatability: a wobble
/// re-rolled every frame would make the whole drawing crawl as the camera moved, which is
/// unusable. Keying it to the stroke's own endpoints means the same edge wobbles the same way
/// from every angle, and two edges that meet at a corner wobble differently.
fn noise(seed: u32) -> f32 {
    // A cheap integer hash (Wang/xorshift finalizer), then map the top bits into -1..1.
    let mut h = seed.wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// A seed for one joint of one stroke: the segment's endpoints (quantized to a micron so
/// floating-point noise doesn't reseed it), the joint index, and which pass this is.
fn seed(a: Vec3, b: Vec3, joint: usize, pass: usize) -> u32 {
    let q = |v: f32| (v * 1000.0).round() as i32 as u32;
    let mut h = 0x811C_9DC5u32;
    for value in [a.x, a.y, a.z, b.x, b.y, b.z] {
        h = (h ^ q(value)).wrapping_mul(0x0100_0193);
    }
    (h ^ (joint as u32).wrapping_mul(0x27D4_EB2F)).wrapping_add((pass as u32).wrapping_mul(0x165667B1))
}

/// One hand-drawn pass over the segment `a`–`b`: overshot at both ends and bowed along its
/// length, as a polyline to stroke.
pub fn stroke(a: Vec3, b: Vec3, pass: usize) -> Vec<Vec3> {
    let along = b - a;
    let length = along.length();
    if length < 1e-6 {
        return vec![a, b];
    }
    let dir = along / length;
    // Two perpendiculars, so the wobble can go any way around the segment and still read
    // from whatever angle the camera is at.
    let helper = if dir.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
    let u = dir.cross(helper).normalize_or_zero();
    let v = dir.cross(u);

    let wobble = (length * PENCIL_WOBBLE).min(PENCIL_WOBBLE_MAX_MM);
    let overshoot = (length * PENCIL_OVERSHOOT).min(PENCIL_OVERSHOOT_MAX_MM);
    let start = -overshoot * noise(seed(a, b, usize::MAX, pass)).abs();
    let end = length + overshoot * noise(seed(b, a, usize::MAX, pass)).abs();

    (0..=PENCIL_STROKE_STEPS)
        .map(|joint| {
            let t = joint as f32 / PENCIL_STROKE_STEPS as f32;
            let point = a + dir * (start + (end - start) * t);
            // The ends stay put (a corner is a corner); the middle is free to bow.
            let bow = (std::f32::consts::PI * t).sin();
            let seed = seed(a, b, joint, pass);
            point
                + u * (noise(seed) * wobble * bow)
                + v * (noise(seed ^ 0x5BF0_3635) * wobble * bow)
        })
        .collect()
}

/// An in-plane frame to hatch within (#1818): a point on the plane and two orthonormal
/// in-plane axes. Built from the plane itself — the foot of the perpendicular from the world
/// origin — so the same flat gets the same frame from every angle and on every redraw, which
/// is what keeps the strokes from crawling as the camera moves.
#[derive(Clone, Copy, Debug)]
pub struct HatchFrame {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
}

impl HatchFrame {
    /// The canonical frame for the plane with outward normal `n` through `point`.
    pub fn new(point: Vec3, n: Vec3) -> Self {
        let n = n.normalize_or_zero();
        // A helper axis the normal is least aligned with, so `u` never collapses.
        let helper = if n.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
        let u = n.cross(helper).normalize_or_zero();
        Self { origin: n * n.dot(point), u, v: n.cross(u) }
    }

    fn to_2d(&self, p: Vec3) -> Vec2 {
        let d = p - self.origin;
        Vec2::new(d.dot(self.u), d.dot(self.v))
    }

    fn to_3d(&self, p: Vec2) -> Vec3 {
        self.origin + self.u * p.x + self.v * p.y
    }
}

/// A triangle soup prepared for scanning (#1818): the triangles, plus the scan lines each one
/// can possibly cross. A shadow soup is every triangle of every solid standing over the face,
/// and testing all of them against all of the scan lines is what made the pencil view crawl.
struct ScanBins {
    tris: Vec<[Vec2; 3]>,
    bins: Vec<Vec<u32>>,
}

fn bin_soup(
    soup: &[[Vec2; 3]],
    across: Vec2,
    first_offset: f32,
    spacing: f32,
    lines: usize,
) -> ScanBins {
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); lines];
    for (i, tri) in soup.iter().enumerate() {
        let d: [f32; 3] = std::array::from_fn(|k| tri[k].dot(across));
        let lo = d[0].min(d[1]).min(d[2]);
        let hi = d[0].max(d[1]).max(d[2]);
        let from = ((lo - first_offset) / spacing).ceil().max(0.0) as usize;
        let to = ((hi - first_offset) / spacing).floor();
        if to < 0.0 {
            continue;
        }
        for bin in bins.iter_mut().take((to as usize + 1).min(lines)).skip(from) {
            bin.push(i as u32);
        }
    }
    ScanBins { tris: soup.to_vec(), bins }
}

/// The spans one scan line covers over a prepared soup, merged.
fn scan_spans(soup: &ScanBins, line: usize, across: Vec2, along: Vec2, offset: f32) -> Vec<(f32, f32)> {
    let mut spans: Vec<(f32, f32)> = Vec::new();
    for &i in soup.bins.get(line).map(Vec::as_slice).unwrap_or(&[]) {
        let tri = &soup.tris[i as usize];
        let mut hits: Vec<f32> = Vec::new();
        for e in 0..3 {
            let (p, q) = (tri[e], tri[(e + 1) % 3]);
            let (dp, dq) = (p.dot(across) - offset, q.dot(across) - offset);
            if (dp > 0.0) == (dq > 0.0) || (dp - dq).abs() < 1e-9 {
                continue;
            }
            let t = dp / (dp - dq);
            hits.push((p + (q - p) * t).dot(along));
        }
        if hits.len() >= 2 {
            hits.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            spans.push((hits[0], hits[hits.len() - 1]));
        }
    }
    if spans.is_empty() {
        return spans;
    }
    // Merge so a stroke crosses the whole shape in one go rather than restarting at every
    // internal triangle edge.
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged = vec![spans[0]];
    for &(s, e) in &spans[1..] {
        let last = merged.last_mut().expect("seeded above");
        if s <= last.1 + 1e-3 {
            last.1 = last.1.max(e);
        } else {
            merged.push((s, e));
        }
    }
    merged
}

/// Ruled strokes filling `cover` inside `frame`, `spacing` apart and running at `angle` within
/// the plane, optionally clipped to `clip` (#1818). Both soups are world triangles that lie in
/// — or have already been projected onto — the frame's plane.
///
/// The scan lines sit on a world lattice rather than on wherever `cover` happens to start
/// (#1811), so a shaded area that grows, moves or merges with another keeps to the same ruled
/// lines instead of drifting into a moiré against its neighbour.
pub fn hatch_in_frame(
    frame: &HatchFrame,
    spacing: f32,
    angle: f32,
    cover: &[[Vec3; 3]],
    clip: Option<&[[Vec3; 3]]>,
) -> Vec<(Vec3, Vec3)> {
    if cover.is_empty() || spacing <= 1e-4 {
        return Vec::new();
    }
    let (sin, cos) = angle.sin_cos();
    let along = Vec2::new(cos, sin);
    let across = Vec2::new(-sin, cos);
    let flatten = |soup: &[[Vec3; 3]]| -> Vec<[Vec2; 3]> {
        soup.iter()
            .map(|tri| std::array::from_fn(|i| frame.to_2d(tri[i])))
            .collect()
    };
    let cover = flatten(cover);

    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for tri in &cover {
        for p in tri {
            let d = p.dot(across);
            lo = lo.min(d);
            hi = hi.max(d);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return Vec::new();
    }
    let first = (lo / spacing).ceil();
    // A degenerate or enormous area would ask for an unbounded number of strokes.
    let lines = ((((hi - lo) / spacing).ceil() as i32 + 1).clamp(0, 600)) as usize;
    let first_offset = first * spacing;
    let cover = bin_soup(&cover, across, first_offset, spacing, lines);
    let clip = clip.map(|c| bin_soup(&flatten(c), across, first_offset, spacing, lines));

    let mut out = Vec::new();
    for line in 0..lines {
        let offset = first_offset + line as f32 * spacing;
        if offset > hi {
            break;
        }
        let spans = scan_spans(&cover, line, across, along, offset);
        if spans.is_empty() {
            continue;
        }
        // Clipped to the receiving surface: a cast shadow only marks the face it lands on.
        let spans: Vec<(f32, f32)> = match &clip {
            None => spans,
            Some(clip) => {
                let keep = scan_spans(clip, line, across, along, offset);
                spans
                    .iter()
                    .flat_map(|&(s, e)| {
                        keep.iter().filter_map(move |&(ks, ke)| {
                            let (s, e) = (s.max(ks), e.min(ke));
                            (e - s > 1e-3).then_some((s, e))
                        })
                    })
                    .collect()
            }
        };
        let point = |d: f32| frame.to_3d(across * offset + along * d);
        out.extend(spans.into_iter().map(|(s, e)| (point(s), point(e))));
    }
    out
}

/// Where the hatch strokes standing in for a body's contact shadow start and end (#1805).
/// The caster's ground-plane footprint is scanned by parallel lines `PENCIL_HATCH_SPACING_MM`
/// apart; each line's span across the footprint becomes one stroke, so the hatch fills the
/// shadow's shape rather than a box around it.
pub fn hatch_segments(footprint: &[[Vec3; 3]]) -> Vec<(Vec3, Vec3)> {
    hatch_in_frame(
        &HatchFrame { origin: Vec3::ZERO, u: Vec3::X, v: Vec3::Y },
        PENCIL_HATCH_SPACING_MM,
        PENCIL_HATCH_ANGLE_RAD,
        footprint,
        None,
    )
}

/// One hand-drawn pass that stays **inside** its own ends (#1818): bowed like [`stroke`], but
/// with no overshoot. Shading and shadow strokes are bounded by the outline of the face they
/// fill — letting them run past a corner leaves a fringe of hair around every silhouette.
pub fn stroke_inside(a: Vec3, b: Vec3, pass: usize) -> Vec<Vec3> {
    let along = b - a;
    let length = along.length();
    if length < 1e-6 {
        return vec![a, b];
    }
    let dir = along / length;
    let helper = if dir.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
    let u = dir.cross(helper).normalize_or_zero();
    let v = dir.cross(u);
    let wobble = (length * PENCIL_WOBBLE).min(PENCIL_WOBBLE_MAX_MM);
    (0..=PENCIL_STROKE_STEPS)
        .map(|joint| {
            let t = joint as f32 / PENCIL_STROKE_STEPS as f32;
            let point = a + along * t;
            let bow = (std::f32::consts::PI * t).sin();
            let seed = seed(a, b, joint, pass);
            point
                + u * (noise(seed) * wobble * bow)
                + v * (noise(seed ^ 0x5BF0_3635) * wobble * bow)
        })
        .collect()
}

/// A hand-drawn pass over the 2D segment `a`–`b` (#1809) — the flat-paper form of [`stroke`],
/// for the drawings workbench, where there is no third dimension to bow into.
pub fn stroke_2d(a: Vec2, b: Vec2, pass: usize) -> Vec<Vec2> {
    stroke(Vec3::new(a.x, a.y, 0.0), Vec3::new(b.x, b.y, 0.0), pass)
        .into_iter()
        .map(|p| Vec2::new(p.x, p.y))
        .collect()
}
