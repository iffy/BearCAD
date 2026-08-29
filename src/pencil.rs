//! Drawing by hand (#1805/#1809): the strokes, tones and hatching that the `LoosePencil` and
//! `ColorPencil` shading modes and the drawings workbench's pencil style share.
//!
//! Everything here is deliberately **repeatable**: a wobble re-rolled every frame would make
//! the whole drawing crawl as the camera moved, which is unusable. Each stroke's shape comes
//! from a hash of its own endpoints, so the same edge is drawn the same way from every angle
//! and on every redraw, and two edges that meet at a corner still wobble differently.

use eframe::egui::Color32;
use glam::{Vec2, Vec3};

/// The face a pencil drawing's lettering is set in (#1830): Klee One, a hand-lettered
/// Japanese/Latin face under the SIL Open Font License (see `assets/fonts/KleeOne-OFL.txt`).
///
/// A technical caption set in the same clean sans every other style uses undoes the drawn look
/// of the view beneath it. The face is bundled rather than looked up on the system, so a
/// drawing letters the same on every machine and in every export.
pub const LABEL_FONT_FAMILY: &str = "Klee One";
pub const LABEL_FONT: &[u8] = include_bytes!("assets/fonts/KleeOne-Regular.ttf");

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
/// Colored pencil for the world axes: the same three hues, muted to sit on paper.
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

/// Laying color on a face with a colored pencil (#1818/#1825).
///
/// Not a flat fill — that reads as paint. The color is *scribbled* in: ruled strokes this far
/// apart, run past the outline a little and broken by gaps of bare paper, the way a hand
/// filling a shape quickly does. The spacing is one number and not a function of the light
/// (#1825): a colored pencil drawing gets its form from its outlines, so every side of a
/// solid is laid on the same, exactly as the plain pencil mode does it.
pub const PENCIL_SCRIBBLE_SPACING_MM: f32 = 1.5;
pub const PENCIL_SHADE_WIDTH_PX: f32 = 2.8;
/// Coverage of one scribble stroke. Light: the tone comes from laying many side by side.
pub const PENCIL_SHADE_ALPHA: f32 = 0.8;
/// How much of the gap to its neighbour a ruled fill's wobble may use up (#1826). A quarter
/// leaves the lines clearly apart; the default cap is nearly half a section hatch's spacing,
/// which filled the face in solid.
pub const RULED_WOBBLE_OF_SPACING: f32 = 0.25;

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

/// How a face's own color reads when it is *scribbled* on with a colored pencil (#1818): the
/// body color deepened a little toward graphite, so a stroke reads as pencil pressure rather
/// than as a brighter version of the fill underneath it.
pub fn shading_tone(base: Color32) -> Color32 {
    mix(base, PENCIL_GRAPHITE, 0.16)
}

/// One ruled span, as the pieces a quick scribble actually leaves (#1825).
///
/// A hand filling a shape lifts and re-lands, and runs past the outline on the way — so the
/// line is not continuous, the paper shows through, and the color sits a little outside the
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

/// Blend two colors, `t` of the way from `a` to `b`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp =
        |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

// ---------------------------------------------------------------------------------------
// Watercolor (#1829). The same drawing as the pencil styles — same paper, same hand-drawn
// outlines, same hatched contact shadow — with the color laid on as a *wash* instead of a
// scribble. A wash is not a scribble with the gaps filled in: it covers, it pools unevenly
// where the water gathered, it runs past the line it was painted inside, and it dries darker
// at the edge of the puddle. Those four things are what these constants buy.

/// How far a wash is taken toward the paper. Much less than the pencil ground: a wash covers,
/// where a colored pencil only grazes the tooth of the paper.
pub const WASH_MIX: f32 = 0.42;
/// The rim a drying wash leaves where pigment gathers against the edge — the single most
/// recognisable thing about watercolor. A deeper draw of the same color.
pub const WASH_EDGE_MIX: f32 = 0.16;
/// Width of that rim and how strongly it reads. Laid down as a few thin passes rather than one
/// thick stroke: a thick round-capped line beads at every joint of its own wobble, which reads
/// as bubbles along the edge instead of pigment gathered against it.
pub const WASH_EDGE_WIDTH_PX: f32 = 3.2;
pub const WASH_EDGE_ALPHA: f32 = 0.4;
pub const WASH_EDGE_PASSES: usize = 3;
/// Pooling within the wash: soft, low-opacity bands where the pigment settled deeper. Close
/// enough together to overlap, so the tone builds smoothly instead of reading as stripes.
pub const WASH_POOL_SPACING_MM: f32 = 3.0;
pub const WASH_POOL_WIDTH_PX: f32 = 5.5;
pub const WASH_POOL_ALPHA: f32 = 0.22;
/// …and only about half of them land, so the pooling is uneven rather than even.
pub const WASH_POOL_COVERAGE: f32 = 0.55;

/// The body of a wash (#1829): the color it dries to across the flat.
pub fn wash_tone(base: Color32) -> Color32 {
    mix(base, PENCIL_PAPER, WASH_MIX)
}

/// The rim it dries to where it gathered against an edge (#1829) — the same color, drawn
/// deeper. Never mixed toward graphite: watercolor stays its own hue as it darkens, which is
/// what separates it from a pencil pressed harder.
pub fn wash_edge_tone(base: Color32) -> Color32 {
    mix(base, PENCIL_PAPER, WASH_EDGE_MIX)
}

/// The tone a patch of wash the water left dry keeps (#1839): the color a long way toward the
/// paper, but not bare paper — a dry patch is where the wash thinned out, not a hole cut in it.
pub fn wash_dry_tone(base: Color32) -> Color32 {
    mix(base, PENCIL_PAPER, 0.55)
}

/// The tone a wash reaches where the pigment pooled a little deeper (#1829). Between the body
/// of the wash and the rim: on a printed page the marks are opaque, so they cannot build tone
/// by overlapping the way the viewport's translucent ones do — the color has to be right on
/// the first pass.
pub fn wash_pool_tone(base: Color32) -> Color32 {
    mix(wash_tone(base), wash_edge_tone(base), 0.4)
}

/// Where a wash pooled, as spans along one ruled line (#1829). Same repeatable hand as
/// [`scribble`], but far fewer and much longer pieces: a puddle, not a stroke.
pub fn pooling(a: Vec3, b: Vec3, pass: usize) -> Vec<(Vec3, Vec3)> {
    let along = b - a;
    let length = along.length();
    if length < 1e-6 {
        return Vec::new();
    }
    let dir = along / length;
    let mut at = 0.0f32;
    let mut out = Vec::new();
    for i in 0..PENCIL_SCRIBBLE_MAX_PIECES {
        if at >= length {
            break;
        }
        let s = seed(a, b, i, pass);
        // Long pieces: a pool spans a good part of the face it settled on.
        let piece = length * (0.35 + 0.4 * noise(s).abs());
        let next = (at + piece).min(length);
        if noise(s ^ 0x2545_F491).abs() < WASH_POOL_COVERAGE && next - at > 1e-4 {
            out.push((a + dir * at, a + dir * next));
        }
        at = next + length * 0.1 * noise(s ^ 0x1B87_3593).abs();
    }
    out
}

/// Splotches (#1839): the wash's own shape, which is not the shape it was painted inside.
///
/// A wash does not dry evenly and it does not stop at the line. Water carries the pigment
/// into pools that dry a good deal deeper than the body of the wash, leaves patches the
/// brush never wetted at all, and runs past the outline wherever the paper let it. Painting
/// a flat with one tone reads as printing, not painting — these are the marks laid over that
/// tone to make it read as a wash.
///
/// Lattice step between splotches, in world mm — one splotch per cell, wandering within it.
pub const WASH_SPLOTCH_SPACING_MM: f32 = 4.0;
/// A splotch's radius, as a share of that step, and how far it varies. The spread matters as
/// much as the size: patches all of a size read as spots, not as a wash drying unevenly.
const WASH_SPLOTCH_RADIUS: f32 = 0.42;
const WASH_SPLOTCH_RADIUS_JITTER: f32 = 0.75;
/// How far from round a splotch may be pulled: water spreads along the paper's grain, so a
/// patch is longer one way than the other. It only ever narrows — a splotch's reach is
/// capped against the outline, and stretching it would take it back over the line.
const WASH_SPLOTCH_STRETCH: f32 = 0.55;
/// How far a splotch wanders from its lattice point, as a share of the step.
const WASH_SPLOTCH_JITTER: f32 = 0.4;
/// Roughly how many splotches are places the water left dry — nearly bare paper inside the
/// shape, rather than pigment gathered deeper.
const WASH_DRY_SHARE: f32 = 0.3;
/// How ragged a splotch's rim is: the share of its radius the edge may pull in by.
const WASH_SPLOTCH_RAGGED: f32 = 0.34;
/// How far past the outline a splotch may reach, in world mm. A wash goes outside the line —
/// by a brush-width, not by a finger.
const WASH_SPILL_MM: f32 = 1.2;
/// …and however close to the edge it started, a splotch keeps at least this much of a mark.
const WASH_SPLOTCH_MIN_MM: f32 = 0.5;
/// A guard: an enormous flat must not ask for an unbounded number of splotches.
const WASH_SPLOTCH_MAX: usize = 500;

/// One patch of a drying wash: where it sits in the flat's plane, how far it reaches, and
/// whether it is pigment gathered deeper or paper the water never covered.
#[derive(Clone, Copy, Debug)]
pub struct Splotch {
    pub center: Vec3,
    pub radius: f32,
    /// A patch the wash left dry — thinned nearly to the paper, not deeper pigment.
    pub dry: bool,
}

/// The flat's own outline in the frame's 2D: the edges only one of its triangles owns.
fn flat_rim(flat: &[[Vec2; 3]]) -> Vec<(Vec2, Vec2)> {
    let key = |p: Vec2| [(p.x * 1000.0).round() as i64, (p.y * 1000.0).round() as i64];
    let mut seen: std::collections::HashMap<([i64; 2], [i64; 2]), (Vec2, Vec2, u32)> =
        std::collections::HashMap::new();
    for tri in flat {
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            let (ka, kb) = (key(a), key(b));
            let k = if ka <= kb { (ka, kb) } else { (kb, ka) };
            seen.entry(k).or_insert((a, b, 0)).2 += 1;
        }
    }
    seen.into_values().filter(|(_, _, n)| *n == 1).map(|(a, b, _)| (a, b)).collect()
}

/// How far a point of the plane is from the flat's outline.
fn distance_to_rim(rim: &[(Vec2, Vec2)], p: Vec2) -> f32 {
    rim.iter().fold(f32::MAX, |best, (a, b)| {
        let d = *b - *a;
        let t = (p - *a).dot(d) / d.length_squared().max(1e-9);
        best.min((p - (*a + d * t.clamp(0.0, 1.0))).length())
    })
}

/// Whether a point of the frame's plane lies within the flat, in the frame's own 2D.
fn inside_flat(flat: &[[Vec2; 3]], p: Vec2) -> bool {
    flat.iter().any(|t| {
        let area2 = (t[1] - t[0]).perp_dot(t[2] - t[0]);
        if area2.abs() < 1e-9 {
            return false;
        }
        let w0 = (t[1] - p).perp_dot(t[2] - p) / area2;
        let w1 = (t[2] - p).perp_dot(t[0] - p) / area2;
        let w2 = 1.0 - w0 - w1;
        w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4
    })
}

/// The splotches a wash dries into on one flat (#1839).
///
/// Centres come off a *world* lattice, like the hatch lines do, so a face that grows or moves
/// keeps its splotches where they were instead of having them crawl; each one's wander, size
/// and wetness are hashed from its own lattice cell, so a view repaints identically.
///
/// A splotch is kept when its centre lands on the flat, and it is *not* clipped to it: one
/// sitting near an edge runs a millimetre or two past the outline, which is the wash going
/// outside the line, and a dry one there leaves the color short of it.
pub fn wash_splotches(frame: &HatchFrame, flat: &[[Vec3; 3]]) -> Vec<Splotch> {
    let flat2: Vec<[Vec2; 3]> = flat
        .iter()
        .map(|tri| std::array::from_fn(|i| frame.to_2d(tri[i])))
        .collect();
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for tri in &flat2 {
        for p in tri {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return Vec::new();
    }
    let rim = flat_rim(&flat2);
    let step = WASH_SPLOTCH_SPACING_MM;
    let cell = |v: f32| (v / step).floor() as i32;
    let (x0, x1) = (cell(lo.x) - 1, cell(hi.x) + 1);
    let (y0, y1) = (cell(lo.y) - 1, cell(hi.y) + 1);
    let mut out = Vec::new();
    for gx in x0..=x1 {
        for gy in y0..=y1 {
            if out.len() >= WASH_SPLOTCH_MAX {
                return out;
            }
            let s = cell_seed(gx, gy);
            let center = Vec2::new(
                (gx as f32 + 0.5) * step + noise(s) * step * WASH_SPLOTCH_JITTER,
                (gy as f32 + 0.5) * step + noise(s ^ 0x68E3_1DA4) * step * WASH_SPLOTCH_JITTER,
            );
            let radius = step
                * (WASH_SPLOTCH_RADIUS
                    + WASH_SPLOTCH_RADIUS_JITTER * noise(s ^ 0xB5297A4D).abs());
            let dry = noise(s ^ 0x1B56_C4E9).abs() < WASH_DRY_SHARE;
            if !inside_flat(&flat2, center) {
                continue;
            }
            // Held to a brush-width past the outline: a wash goes outside the line, it does
            // not wander off the shape.
            let radius = radius
                .min(distance_to_rim(&rim, center) + WASH_SPILL_MM)
                .max(WASH_SPLOTCH_MIN_MM);
            out.push(Splotch { center: frame.to_3d(center), radius, dry });
        }
    }
    out
}

/// How many points a splotch's rim is traced with — enough for a wet edge, few enough that a
/// face full of them stays cheap to paint.
pub const WASH_SPLOTCH_RIM_POINTS: usize = 14;

/// The rim of one splotch (#1839), as a closed ring of points in the frame's plane: a disc
/// pulled in by its own amount at every step round, so a patch dries ragged rather than
/// round. Star-shaped about its centre, so a renderer can fan it into triangles.
pub fn splotch_outline(frame: &HatchFrame, splotch: &Splotch) -> Vec<Vec3> {
    let center = frame.to_2d(splotch.center);
    let s = seed(splotch.center, splotch.center, 0, splotch.dry as usize);
    // Stretched along its own direction, so the patch is longer one way than the other.
    let lean = noise(s ^ 0x27D4_EB2F) * std::f32::consts::PI;
    let (ls, lc) = lean.sin_cos();
    let narrow = 1.0 - WASH_SPLOTCH_STRETCH * noise(s ^ 0x1656_67B1).abs();
    (0..WASH_SPLOTCH_RIM_POINTS)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / WASH_SPLOTCH_RIM_POINTS as f32;
            // Two turns of wobble round the rim, so neighbouring points pull in together and
            // the edge undulates instead of spiking.
            let n = noise(s ^ (i as u32).wrapping_mul(0x9E37_79B9));
            let m = noise(s ^ ((i as u32 / 2).wrapping_mul(0x85EB_CA6B)));
            let r = splotch.radius * (1.0 - WASH_SPLOTCH_RAGGED * (0.5 * n.abs() + 0.5 * m.abs()));
            let (x, y) = (a.cos() * r, a.sin() * r * narrow);
            frame.to_3d(center + Vec2::new(x * lc - y * ls, x * ls + y * lc))
        })
        .collect()
}

/// A seed for one cell of the splotch lattice.
fn cell_seed(x: i32, y: i32) -> u32 {
    let mut h = 0x811C_9DC5u32;
    for v in [x, y] {
        h = (h ^ v as u32).wrapping_mul(0x0100_0193);
    }
    h
}

/// The ground tone a colored-pencil body is filled with (#1825): its own color taken most of
/// the way to the paper. It is only there so a near face hides a far one — what the eye reads
/// as the color is the scribble laid over it, and the gaps in that scribble have to read as
/// bare paper, not as a lighter wash of the same color.
pub fn scribble_ground(base: Color32) -> Color32 {
    mix(base, PENCIL_PAPER, 0.88)
}

/// How a body's own color reads in colored pencil (#1812): the fill it is laid on with,
/// and the darker tone of the same color its outline is drawn in.
///
/// A colored pencil does not cover the paper — the fill is the body color let a long way
/// down toward [`PENCIL_PAPER`]. The outline is the same color pressed harder, mixed toward
/// graphite so it still reads as a drawn line rather than a bright edge.
pub fn color_tones(base: Color32) -> (Color32, Color32) {
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
    // The free-hand cap is per-axis, as it always was, so this keeps the hand it had.
    stroke_within(a, b, pass, PENCIL_WOBBLE_MAX_MM * std::f32::consts::SQRT_2)
}

/// [`stroke_inside`] with the wobble held under `max_wobble_mm` (#1826) — that is the furthest
/// the line may stray from straight, resultant and all.
///
/// A hand's wander has to stay well inside the gap to its neighbour. Ruled fills — a section
/// hatch, say — sit a few millimetres apart, and the default cap is nearly half of that: every
/// line crossed the next and the whole face filled in solid.
pub fn stroke_within(a: Vec3, b: Vec3, pass: usize, max_wobble_mm: f32) -> Vec<Vec3> {
    let along = b - a;
    let length = along.length();
    if length < 1e-6 {
        return vec![a, b];
    }
    let dir = along / length;
    let helper = if dir.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
    let u = dir.cross(helper).normalize_or_zero();
    let v = dir.cross(u);
    // The wobble goes on in two perpendicular directions at once, so the *resultant* can reach
    // √2 times either one. Divide it out, or a cap meant to keep a ruled fill inside its lane
    // lets it wander 40% past (#1826).
    let wobble = (length * PENCIL_WOBBLE)
        .min(max_wobble_mm.max(0.0) / std::f32::consts::SQRT_2);
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

/// The flat-paper form of [`stroke_within`] (#1827): bowed, staying inside its own ends, and
/// with the wobble held under `max_wobble_mm` so a ruled fill's lines do not cross each other.
pub fn stroke_2d_within(a: Vec2, b: Vec2, pass: usize, max_wobble_mm: f32) -> Vec<Vec2> {
    stroke_within(
        Vec3::new(a.x, a.y, 0.0),
        Vec3::new(b.x, b.y, 0.0),
        pass,
        max_wobble_mm,
    )
    .into_iter()
    .map(|p| Vec2::new(p.x, p.y))
    .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: f32) -> Vec<[Vec3; 3]> {
        let p = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        vec![
            [p(0.0, 0.0), p(size, 0.0), p(size, size)],
            [p(0.0, 0.0), p(size, size), p(0.0, size)],
        ]
    }

    /// #1839: a wash dries in splotches — pools of deeper pigment and patches the water left
    /// dry — and they do not line up with the shape they were painted inside: each one is
    /// free to run past the outline, by a brush-width and no more.
    #[test]
    fn a_wash_dries_in_splotches_that_ignore_the_outline() {
        let frame = HatchFrame { origin: Vec3::ZERO, u: Vec3::X, v: Vec3::Y };
        let flat = square(40.0);
        let splotches = wash_splotches(&frame, &flat);
        assert!(splotches.len() > 20, "a 40 mm square dries in many patches, got {}", splotches.len());
        assert!(splotches.iter().any(|s| s.dry), "some of it the water never covered");
        assert!(splotches.iter().any(|s| !s.dry), "and some of it is pigment gathered deeper");

        let mut outside = 0;
        for splotch in &splotches {
            for p in splotch_outline(&frame, splotch) {
                let over = [-p.x, -p.y, p.x - 40.0, p.y - 40.0]
                    .into_iter()
                    .fold(f32::MIN, f32::max);
                if over > 0.05 {
                    outside += 1;
                }
                assert!(
                    over < WASH_SPILL_MM + 0.01,
                    "a splotch reached {over} mm past the outline"
                );
            }
        }
        assert!(outside > 0, "and the wash goes outside the line somewhere");

        // Repeatable: a wash that re-rolled itself every frame would crawl as the camera moved.
        let again = wash_splotches(&frame, &flat);
        assert_eq!(splotches.len(), again.len());
        for (a, b) in splotches.iter().zip(&again) {
            assert_eq!((a.center, a.radius, a.dry), (b.center, b.radius, b.dry));
        }
    }

    /// The tones a wash dries to run from the dry patches, through its body, to the pools and
    /// the rim — each a step deeper in the body's own color, never greyed toward graphite.
    #[test]
    fn a_washs_tones_deepen_in_its_own_color() {
        let blue = Color32::from_rgb(60, 110, 200);
        let steps = [wash_dry_tone(blue), wash_tone(blue), wash_pool_tone(blue), wash_edge_tone(blue)];
        for pair in steps.windows(2) {
            assert!(
                pair[1].r() < pair[0].r() && pair[1].b() <= pair[0].b(),
                "{:?} should be a deeper draw than {:?}",
                pair[1],
                pair[0]
            );
        }
        for tone in steps {
            assert!(tone.b() > tone.r(), "every step stays blue, got {tone:?}");
        }
    }
}
