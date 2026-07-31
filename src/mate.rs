//! Where a joint's parts start out (#1021): *put this face on that face, then line this up
//! with that.*
//!
//! A mate is a **placement and nothing more** — it works out to a rigid transform that the
//! kind's freedoms then act on top of ([`crate::joints`]). Nothing here constrains how a
//! joint moves.
//!
//! The face pair (#1014) carries the moving part's face onto the fixed one, which leaves
//! three freedoms: two slides in the mating plane and the spin about its normal. Line-up
//! rows (#1015) take those away. Every row's two picks are **projected along the mating
//! normal** onto the mating plane and the relationship applied to the projections, so the
//! pick need not lie in the plane — a hole rim, a boss centre or a far corner all work, and
//! a row can never disturb the face pair.
//!
//! What the face pair and the rows leave undetermined is chosen by **least motion** from
//! where the part already sits, so a part dragged roughly into place doesn't jump across the
//! document and the preview doesn't drift as rows are added.

use crate::model::{Document, Joint, JointMate, MateLineUp, MateRef};
use glam::{Mat3, Mat4, Vec2, Vec3};

/// A mate pick resolved against the live model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MateGeom {
    /// A face or datum plane: a point on it plus its outward normal.
    Plane { origin: Vec3, normal: Vec3 },
    /// A straight edge or world axis.
    Line { origin: Vec3, dir: Vec3 },
    Point(Vec3),
}

/// A resolved pick's representative point — where its mark is drawn.
pub fn geom_point(g: &MateGeom) -> Vec3 {
    g.point()
}

impl MateGeom {
    /// A representative point — what a line-up row projects when the pick isn't a line.
    fn point(&self) -> Vec3 {
        match self {
            MateGeom::Plane { origin, .. } | MateGeom::Line { origin, .. } => *origin,
            MateGeom::Point(p) => *p,
        }
    }
}

/// Resolve a mate pick in un-posed world space — body-local keys re-found on the live mesh,
/// world-fixed references (a datum plane, a world axis, the origin) as they are. `None` when
/// the reference no longer resolves, which mates as identity (#1019).
pub fn resolve(doc: &Document, r: &MateRef) -> Option<MateGeom> {
    match r {
        MateRef::Face { body, centroid, normal } => {
            doc.bodies.get(*body).filter(|b| !b.deleted)?;
            let solid = crate::extrude::body_solid_mesh_unposed(doc, *body)?;
            let tris = crate::extrude::face_group_matching(&solid, *centroid, *normal)?;
            let origin = crate::extrude::face_group_center(&tris);
            let n = (tris[0][1] - tris[0][0])
                .cross(tris[0][2] - tris[0][0])
                .normalize_or_zero();
            (n.length_squared() > 0.5).then_some(MateGeom::Plane { origin, normal: n })
        }
        MateRef::Plane(i) => {
            let p = doc.construction_planes.get(*i).filter(|p| !p.deleted)?;
            Some(MateGeom::Plane {
                origin: p.origin,
                normal: p.normal.normalize_or_zero(),
            })
        }
        MateRef::Edge { body, a, b } => {
            let (p0, p1) = crate::parameters::body_edge_world_segment(doc, *body, *a, *b)?;
            let dir = (p1 - p0).normalize_or_zero();
            (dir.length_squared() > 0.5).then_some(MateGeom::Line { origin: p0, dir })
        }
        MateRef::Axis(a) => Some(MateGeom::Line {
            origin: Vec3::ZERO,
            dir: a.direction(),
        }),
        MateRef::Point(p) => crate::extrude::move_point_world(doc, p).map(MateGeom::Point),
    }
}

/// Resolve a **fixed-side** pick into world space. Geometry on a body rides the base's pose;
/// a datum plane, a world axis or the origin is world-fixed and doesn't (#1018).
fn resolve_fixed(doc: &Document, r: &MateRef, base_pose: Mat4) -> Option<MateGeom> {
    let geom = resolve(doc, r)?;
    if r.body().is_none() {
        return Some(geom);
    }
    Some(match geom {
        MateGeom::Plane { origin, normal } => MateGeom::Plane {
            origin: base_pose.transform_point3(origin),
            normal: base_pose.transform_vector3(normal).normalize_or_zero(),
        },
        MateGeom::Line { origin, dir } => MateGeom::Line {
            origin: base_pose.transform_point3(origin),
            dir: base_pose.transform_vector3(dir).normalize_or_zero(),
        },
        MateGeom::Point(p) => MateGeom::Point(base_pose.transform_point3(p)),
    })
}

/// A solved mate: where the moving part lands, and the frame the kind's freedoms act in.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// World transform for the driven part's un-posed geometry.
    pub transform: Mat4,
    /// The mating plane's landing point — where the moving face's middle ends up.
    pub origin: Vec3,
    /// The mating plane's normal, pointing out of the fixed face.
    pub normal: Vec3,
    /// An in-plane direction the freedoms can aim along: the first line-up row's fixed edge
    /// where there is one, else an arbitrary in-plane axis.
    pub along: Vec3,
    /// How many of the three post-face-pair freedoms the line-up rows leave open (#1016).
    /// Zero means the part is fully placed and no further row is offered.
    pub open_freedoms: usize,
}

/// One line-up row projected into the mating plane's 2-D coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlaneRow {
    moving: PlaneGeom,
    fixed: PlaneGeom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PlaneGeom {
    Point(Vec2),
    Line { origin: Vec2, dir: Vec2 },
}

fn perp(v: Vec2) -> Vec2 {
    Vec2::new(-v.y, v.x)
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Project a resolved pick onto the mating plane. A pick running along the normal projects
/// to a point and orients nothing, so it comes back as a point (and #1016 refuses it).
fn project(geom: MateGeom, origin: Vec3, e1: Vec3, e2: Vec3) -> PlaneGeom {
    let flat = |p: Vec3| Vec2::new((p - origin).dot(e1), (p - origin).dot(e2));
    match geom {
        MateGeom::Point(p) => PlaneGeom::Point(flat(p)),
        MateGeom::Plane { origin: o, .. } => PlaneGeom::Point(flat(o)),
        MateGeom::Line { origin: o, dir } => {
            let d = Vec2::new(dir.dot(e1), dir.dot(e2));
            if d.length() < 1e-3 {
                PlaneGeom::Point(flat(o))
            } else {
                PlaneGeom::Line { origin: flat(o), dir: d.normalize() }
            }
        }
    }
}

/// One residual of the in-plane fit at a given spin: `a · t + b`, where `t` is the in-plane
/// slide. Splitting it this way keeps the fit **linear in the slide** for any fixed spin, so
/// the solve is a 1-D search over the spin with a 2×2 solve inside — no iteration to
/// diverge, and rank-deficient cases fall out as least motion rather than as a failure.
struct Residual {
    a: Vec2,
    b: f32,
}

fn residuals(rows: &[PlaneRow], theta: f32, scale: f32) -> Vec<Residual> {
    let (s, c) = theta.sin_cos();
    let rot = |v: Vec2| Vec2::new(c * v.x - s * v.y, s * v.x + c * v.y);
    let mut out = Vec::new();
    for row in rows {
        match (row.moving, row.fixed) {
            // Their projections coincide: pins both slides, leaves the spin.
            (PlaneGeom::Point(pm), PlaneGeom::Point(pf)) => {
                let d = rot(pm) - pf;
                out.push(Residual { a: Vec2::X, b: d.x });
                out.push(Residual { a: Vec2::Y, b: d.y });
            }
            // The point's projection lands on the line's: pins one slide.
            (PlaneGeom::Point(pm), PlaneGeom::Line { origin: q, dir }) => {
                let n = perp(dir);
                out.push(Residual { a: n, b: n.dot(rot(pm) - q) });
            }
            (PlaneGeom::Line { origin: pm, dir }, PlaneGeom::Point(pf)) => {
                let n = perp(rot(dir));
                out.push(Residual { a: n, b: n.dot(rot(pm) - pf) });
            }
            // Their projections are collinear: pins the spin and the slide across the line.
            (
                PlaneGeom::Line { origin: pm, dir: dm },
                PlaneGeom::Line { origin: q, dir: df },
            ) => {
                let rdm = rot(dm);
                // An edge has no sign, so take whichever way round is the nearer alignment.
                let df = if rdm.dot(df) < 0.0 { -df } else { df };
                out.push(Residual { a: Vec2::ZERO, b: cross2(rdm, df) * scale });
                let n = perp(df);
                out.push(Residual { a: n, b: n.dot(rot(pm) - q) });
            }
        }
    }
    out
}

/// The best in-plane slide for a fixed spin, and what it costs.
///
/// The **least-norm** solution, not merely a solution: what the rows don't determine stays
/// at zero rather than drifting, which is what makes an underdetermined mate hold the part
/// where it already sits. A regularizer would do the same job but would also bias the spin
/// (a bigger turn can shorten the slide), so the rank-deficient case is solved outright.
fn best_slide(rows: &[PlaneRow], theta: f32, scale: f32) -> (Vec2, f32) {
    let rs = residuals(rows, theta, scale);
    let (mut mxx, mut mxy, mut myy) = (0.0f32, 0.0f32, 0.0f32);
    let mut g = Vec2::ZERO;
    for r in &rs {
        mxx += r.a.x * r.a.x;
        mxy += r.a.x * r.a.y;
        myy += r.a.y * r.a.y;
        g += r.a * r.b;
    }
    let trace = mxx + myy;
    let det = mxx * myy - mxy * mxy;
    let t = if det > trace * trace * 1e-6 {
        // Both slides pinned: the exact least-squares answer.
        Vec2::new(
            -(myy * g.x - mxy * g.y) / det,
            -(mxx * g.y - mxy * g.x) / det,
        )
    } else if trace > 1e-9 {
        // One slide pinned: move along that direction only, and not at all across it.
        let u = if mxx >= myy {
            Vec2::new(mxx, mxy)
        } else {
            Vec2::new(mxy, myy)
        };
        let u = u.normalize_or_zero();
        let denom = u.x * (mxx * u.x + mxy * u.y) + u.y * (mxy * u.x + myy * u.y);
        if denom > 1e-9 { -u * (u.dot(g) / denom) } else { Vec2::ZERO }
    } else {
        Vec2::ZERO
    };
    let cost: f32 = rs.iter().map(|r| (r.a.dot(t) + r.b).powi(2)).sum();
    (t, cost)
}

/// Solve the in-plane fit: the spin and slide that satisfy the rows, choosing **least
/// motion** among equally good answers. A coarse sweep over the whole turn finds every
/// basin, so a row asking for 170° is found as readily as one asking for 2°.
fn solve_plane(rows: &[PlaneRow], scale: f32) -> (f32, Vec2) {
    if rows.is_empty() {
        return (0.0, Vec2::ZERO);
    }
    const STEPS: usize = 720;
    let mut best: Option<(f32, f32, Vec2)> = None; // (cost, theta, slide)
    for i in 0..STEPS {
        let mut theta = (i as f32) * std::f32::consts::TAU / STEPS as f32;
        if theta > std::f32::consts::PI {
            theta -= std::f32::consts::TAU;
        }
        // The tolerance is a length, not a ratio — an exactly-satisfiable set of rows costs
        // nothing at every spin it admits, and a ratio can't tell those apart.
        let tol = (scale * scale * 1e-8).max(1e-9);
        // Golden-section refine inside the step, which is where the true minimum sits —
        // but only if it actually buys something. Where the cost is flat (a row that leaves
        // the spin free) the search would otherwise wander off the step and turn the part
        // by a fraction of a degree for nothing.
        let step = std::f32::consts::TAU / STEPS as f32;
        let (mut lo, mut hi) = (theta - step * 0.5, theta + step * 0.5);
        for _ in 0..24 {
            let m1 = lo + (hi - lo) * 0.382;
            let m2 = lo + (hi - lo) * 0.618;
            if best_slide(rows, m1, scale).1 <= best_slide(rows, m2, scale).1 {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let (centre_slide, centre_cost) = best_slide(rows, theta, scale);
        let refined = (lo + hi) * 0.5;
        let (refined_slide, refined_cost) = best_slide(rows, refined, scale);
        let (theta, slide, cost) = if refined_cost < centre_cost - tol {
            (refined, refined_slide, refined_cost)
        } else {
            (theta, centre_slide, centre_cost)
        };
        // Ties go to the smaller turn: the part that barely moves is the one meant.
        let better = match best {
            None => true,
            Some((bc, bt, _)) => {
                cost < bc - tol || (cost < bc + tol && theta.abs() < bt.abs())
            }
        };
        if better {
            best = Some((cost, theta, slide));
        }
    }
    let (_, theta, slide) = best.unwrap_or((0.0, 0.0, Vec2::ZERO));
    (theta, slide)
}

/// How many of the three freedoms the rows leave open, from the rank of the fit's Jacobian
/// at the answer (#1016). Three means the face pair alone; zero means fully placed.
fn open_freedoms(rows: &[PlaneRow], theta: f32, slide: Vec2, scale: f32) -> usize {
    let h = 1e-3;
    let cost_at = |th: f32, t: Vec2| -> Vec<f32> {
        residuals(rows, th, scale)
            .iter()
            .map(|r| r.a.dot(t) + r.b)
            .collect()
    };
    let base = cost_at(theta, slide);
    if base.is_empty() {
        return 3;
    }
    let mut cols: Vec<Vec<f32>> = Vec::new();
    for (dth, dt) in [
        (h, Vec2::ZERO),
        (0.0, Vec2::new(h, 0.0)),
        (0.0, Vec2::new(0.0, h)),
    ] {
        let up = cost_at(theta + dth, slide + dt);
        let down = cost_at(theta - dth, slide - dt);
        cols.push(
            up.iter()
                .zip(&down)
                .map(|(u, d)| (u - d) / (2.0 * h))
                .collect(),
        );
    }
    // Gram matrix of the three columns; its rank is the Jacobian's.
    let dot = |i: usize, j: usize| -> f64 {
        cols[i].iter().zip(&cols[j]).map(|(a, b)| (*a as f64) * (*b as f64)).sum()
    };
    let mut m = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = dot(i, j);
        }
    }
    let norm = (0..3).map(|i| m[i][i]).fold(0.0f64, f64::max).max(1e-9);
    // Gaussian elimination with a relative tolerance — a column that adds nothing to the
    // span is a freedom nothing has pinned.
    let mut rank = 0;
    let mut rows_left: Vec<[f64; 3]> = m.to_vec();
    for col in 0..3 {
        let Some(pivot) = (rank..rows_left.len())
            .max_by(|&a, &b| {
                rows_left[a][col]
                    .abs()
                    .partial_cmp(&rows_left[b][col].abs())
                    .unwrap()
            })
            .filter(|&i| rows_left[i][col].abs() > norm * 1e-6)
        else {
            continue;
        };
        rows_left.swap(rank, pivot);
        for r in rank + 1..rows_left.len() {
            let f = rows_left[r][col] / rows_left[rank][col];
            for c in 0..3 {
                rows_left[r][c] -= f * rows_left[rank][c];
            }
        }
        rank += 1;
    }
    3 - rank
}

/// Solve a joint's mate (#1021): where its driven part lands, and the frame its freedoms act
/// in. `base_pose` is the fixed side's own pose, so a chain lines up against the fixed part
/// where it actually sits. `None` until the face pair is complete and resolves — an
/// unresolved mate places nothing, which is the identity mate that leaves parts put.
pub fn placement(doc: &Document, mate: &JointMate, base_pose: Mat4) -> Option<Placement> {
    let (Some(moving), Some(fixed)) = (mate.moving_face.as_ref(), mate.fixed_face.as_ref())
    else {
        return None;
    };
    let MateGeom::Plane { origin: pm, normal: nm } = resolve(doc, moving)? else {
        return None;
    };
    let MateGeom::Plane { origin: pf, normal: nf } = resolve_fixed(doc, fixed, base_pose)?
    else {
        return None;
    };
    let offset = if mate.offset.trim().is_empty() {
        0.0
    } else {
        crate::value::eval_length_mm_in_doc(&mate.offset, doc)?
    };
    // The surfaces touch by default, so the moving face turns to face the fixed one; flipped,
    // the two normals point the same way.
    let target = if mate.flip { nf } else { -nf };
    let r0 = shortest_rotation(nm, target);
    // Turn about the moving face's own middle and then push it onto the fixed plane: the
    // face centre keeps its in-plane position, which is the least-motion landing (#1014).
    let place0 = Mat4::from_translation(pm) * Mat4::from_mat3(r0) * Mat4::from_translation(-pm);
    let shift = nf * (offset - (pm - pf).dot(nf));
    let place0 = Mat4::from_translation(shift) * place0;
    let landing = pm + shift;

    let e1 = nf.any_orthonormal_vector();
    let e2 = nf.cross(e1).normalize_or_zero();

    // Line-up rows, projected along the mating normal (#1015).
    let mut rows = Vec::new();
    let mut along: Option<Vec3> = None;
    for row in &mate.line_up {
        let (Some(mv), Some(fx)) = (row.moving.as_ref(), row.fixed.as_ref()) else {
            continue;
        };
        let (Some(mg), Some(fg)) = (resolve(doc, mv), resolve_fixed(doc, fx, base_pose)) else {
            continue;
        };
        let mg = match mg {
            MateGeom::Point(p) => MateGeom::Point(place0.transform_point3(p)),
            MateGeom::Line { origin, dir } => MateGeom::Line {
                origin: place0.transform_point3(origin),
                dir: place0.transform_vector3(dir).normalize_or_zero(),
            },
            MateGeom::Plane { origin, normal } => MateGeom::Plane {
                origin: place0.transform_point3(origin),
                normal: place0.transform_vector3(normal).normalize_or_zero(),
            },
        };
        let fixed_plane = project(fg, landing, e1, e2);
        if along.is_none() {
            if let PlaneGeom::Line { dir, .. } = fixed_plane {
                along = Some((e1 * dir.x + e2 * dir.y).normalize_or_zero());
            }
        }
        rows.push(PlaneRow {
            moving: project(mg, landing, e1, e2),
            fixed: fixed_plane,
        });
    }
    let scale = rows
        .iter()
        .flat_map(|r| {
            [r.moving, r.fixed].into_iter().map(|g| match g {
                PlaneGeom::Point(p) | PlaneGeom::Line { origin: p, .. } => p.length(),
            })
        })
        .fold(1.0f32, f32::max);
    let (theta, slide) = solve_plane(&rows, scale);
    let open = open_freedoms(&rows, theta, slide, scale);

    let spin = Mat4::from_translation(landing)
        * Mat4::from_mat3(Mat3::from_axis_angle(nf, theta))
        * Mat4::from_translation(-landing);
    let glide = Mat4::from_translation(e1 * slide.x + e2 * slide.y);
    let transform = glide * spin * place0;

    let origin = landing + e1 * slide.x + e2 * slide.y;
    let along = along
        .map(|d| Mat3::from_axis_angle(nf, theta) * d)
        .filter(|d| d.length_squared() > 0.5)
        .unwrap_or(e1);
    Some(Placement {
        transform,
        origin,
        normal: nf,
        along: (along - nf * along.dot(nf)).normalize_or_zero(),
        open_freedoms: open,
    })
}

/// The smallest rotation carrying `from` onto `to` — no spin of its own, which is what makes
/// the face pair's undetermined turn the least-motion one.
fn shortest_rotation(from: Vec3, to: Vec3) -> Mat3 {
    let (from, to) = (from.normalize_or_zero(), to.normalize_or_zero());
    let d = from.dot(to).clamp(-1.0, 1.0);
    if d > 1.0 - 1e-7 {
        return Mat3::IDENTITY;
    }
    if d < -1.0 + 1e-7 {
        return Mat3::from_axis_angle(from.any_orthonormal_vector(), std::f32::consts::PI);
    }
    Mat3::from_axis_angle(from.cross(to).normalize(), d.acos())
}

/// Whether a line-up row would pin something the rows before it leave open (#1016): the test
/// that keeps the pickers from offering a pick that changes nothing.
pub fn row_pins_something(
    doc: &Document,
    mate: &JointMate,
    base_pose: Mat4,
    row: &MateLineUp,
) -> bool {
    let Some(before) = placement(doc, mate, base_pose) else {
        return false;
    };
    if before.open_freedoms == 0 {
        return false;
    }
    let mut probe = mate.clone();
    probe.line_up.push(*row);
    placement(doc, &probe, base_pose)
        .is_some_and(|after| after.open_freedoms < before.open_freedoms)
}

/// The joint's mate with only its complete rows — what a half-picked row must not disturb.
pub fn settled(joint: &Joint) -> JointMate {
    settled_mate(&joint.mate)
}

/// A mate with only its complete line-up rows.
pub fn settled_mate(mate: &JointMate) -> JointMate {
    let mut mate = mate.clone();
    mate.line_up.retain(|r| r.is_complete());
    mate
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::model::{Body, BodySource, ImportedMesh, JointKind, JointLimits, JointRef};

    /// A `size` cube with its low corner at `origin` — six real faces, so face keys resolve.
    pub fn cube_tris(origin: Vec3, size: Vec3) -> Vec<[Vec3; 3]> {
        let (a, b) = (origin, origin + size);
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let quad = |p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3| {
            vec![[p0, p1, p2], [p0, p2, p3]]
        };
        let mut t = Vec::new();
        // -Z and +Z
        t.extend(quad(v(a.x, a.y, a.z), v(a.x, b.y, a.z), v(b.x, b.y, a.z), v(b.x, a.y, a.z)));
        t.extend(quad(v(a.x, a.y, b.z), v(b.x, a.y, b.z), v(b.x, b.y, b.z), v(a.x, b.y, b.z)));
        // -Y and +Y
        t.extend(quad(v(a.x, a.y, a.z), v(b.x, a.y, a.z), v(b.x, a.y, b.z), v(a.x, a.y, b.z)));
        t.extend(quad(v(a.x, b.y, a.z), v(a.x, b.y, b.z), v(b.x, b.y, b.z), v(b.x, b.y, a.z)));
        // -X and +X
        t.extend(quad(v(a.x, a.y, a.z), v(a.x, a.y, b.z), v(a.x, b.y, b.z), v(a.x, b.y, a.z)));
        t.extend(quad(v(b.x, a.y, a.z), v(b.x, b.y, a.z), v(b.x, b.y, b.z), v(b.x, a.y, b.z)));
        t
    }

    pub fn cube_body(doc: &mut Document, origin: Vec3, size: Vec3) -> usize {
        doc.imported_meshes.push(ImportedMesh {
            triangles: cube_tris(origin, size),
            source_name: format!("part{}", doc.imported_meshes.len()),
        });
        doc.bodies.push(Body {
            source: BodySource::Imported(doc.imported_meshes.len() - 1),
            name: None,
            material: None,
            deleted: false,
            shadow: false,
        });
        doc.bodies.len() - 1
    }

    /// The face of `body` whose middle is nearest `near` — how the tests name a face without
    /// hand-quantizing a key.
    pub fn face_ref(doc: &Document, body: usize, near: Vec3) -> MateRef {
        let solid = crate::extrude::body_solid_mesh_unposed(doc, body).unwrap();
        let groups = crate::gpu_viewport::solid_mesh_coplanar_faces(&solid);
        let best = groups
            .iter()
            .min_by(|a, b| {
                let d = |t: &Vec<[Vec3; 3]>| {
                    (crate::extrude::face_group_center(t) - near).length()
                };
                d(a).partial_cmp(&d(b)).unwrap()
            })
            .unwrap();
        let q = crate::hierarchy::quantize_body_point;
        let n = (best[0][1] - best[0][0]).cross(best[0][2] - best[0][0]).normalize();
        MateRef::Face {
            body,
            centroid: q(crate::extrude::face_group_center(best)),
            normal: q(n),
        }
    }

    pub fn vertex_ref(body: usize, p: Vec3) -> MateRef {
        MateRef::Point(crate::model::MovePointRef::Vertex {
            body,
            p: crate::hierarchy::quantize_body_point(p),
        })
    }

    pub fn joint(members: Vec<JointRef>, kind: JointKind) -> Joint {
        Joint {
            members,
            base: 0,
            kind,
            mate: JointMate::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: JointLimits::default(),
            name: None,
            deleted: false,
        }
    }

    /// #1014: the face pair alone puts the moving face flush on the fixed one, normals
    /// opposed, without sliding the part about in the plane.
    #[test]
    fn face_pair_lands_the_part_flush() {
        let mut doc = Document::default();
        let fixed = cube_body(&mut doc, Vec3::ZERO, Vec3::splat(10.0));
        let moving = cube_body(&mut doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(4.0));
        let mate = JointMate {
            // The moving cube's -Z face onto the fixed cube's +Z face.
            moving_face: Some(face_ref(&doc, moving, Vec3::new(42.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(&doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            ..Default::default()
        };
        let p = placement(&doc, &mate, Mat4::IDENTITY).expect("a complete face pair places");
        // The moving face's middle lands on the fixed plane, keeping its in-plane position.
        let landed = p.transform.transform_point3(Vec3::new(42.0, 2.0, 0.0));
        assert!(
            (landed - Vec3::new(42.0, 2.0, 10.0)).length() < 1e-3,
            "landed at {landed}"
        );
        // Its own outward normal ends up opposed to the fixed face's +Z, so they touch and
        // the part sits on top rather than sinking through.
        let n = p.transform.transform_vector3(-Vec3::Z);
        assert!((n - -Vec3::Z).length() < 1e-3, "normal is {n}");
        assert_eq!(p.open_freedoms, 3, "two slides and the spin are still open");
    }

    /// #1014: the offset holds the part off the face, and the flip turns it the other way.
    #[test]
    fn offset_and_flip_change_where_the_face_lands() {
        let mut doc = Document::default();
        let fixed = cube_body(&mut doc, Vec3::ZERO, Vec3::splat(10.0));
        let moving = cube_body(&mut doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(4.0));
        let mut mate = JointMate {
            moving_face: Some(face_ref(&doc, moving, Vec3::new(42.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(&doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            offset: "3".to_string(),
            ..Default::default()
        };
        let p = placement(&doc, &mate, Mat4::IDENTITY).unwrap();
        let landed = p.transform.transform_point3(Vec3::new(42.0, 2.0, 0.0));
        assert!((landed.z - 13.0).abs() < 1e-3, "held 3 mm off, got {landed}");
        // Flipped, the two normals point the same way — the part hangs under the face
        // instead of standing on it.
        mate.flip = true;
        mate.offset = String::new();
        let p = placement(&doc, &mate, Mat4::IDENTITY).unwrap();
        let n = p.transform.transform_vector3(-Vec3::Z);
        assert!((n - Vec3::Z).length() < 1e-3, "flipped normal is {n}");
    }

    /// #1015: a point line-up row makes the two projections coincide, pinning both slides
    /// and leaving the spin.
    #[test]
    fn a_point_row_pins_both_slides() {
        let mut doc = Document::default();
        let fixed = cube_body(&mut doc, Vec3::ZERO, Vec3::splat(10.0));
        let moving = cube_body(&mut doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(4.0));
        let mate = JointMate {
            moving_face: Some(face_ref(&doc, moving, Vec3::new(42.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(&doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            line_up: vec![MateLineUp {
                // A corner of the moving cube **off** the mating plane (its top, 4 mm up)
                // onto a corner of the fixed cube: the projections are what must meet.
                moving: Some(vertex_ref(moving, Vec3::new(40.0, 0.0, 4.0))),
                fixed: Some(vertex_ref(fixed, Vec3::new(0.0, 0.0, 10.0))),
            }],
            ..Default::default()
        };
        let p = placement(&doc, &mate, Mat4::IDENTITY).unwrap();
        let landed = p.transform.transform_point3(Vec3::new(40.0, 0.0, 4.0));
        assert!(
            (landed.x).abs() < 1e-2 && (landed.y).abs() < 1e-2,
            "projections coincide, got {landed}"
        );
        assert_eq!(p.open_freedoms, 1, "only the spin is left");
    }

    /// #1015: a face plus two point rows fully places a part — three picks past the face
    /// pair is the worst case.
    #[test]
    fn two_point_rows_fully_place_the_part() {
        let mut doc = Document::default();
        let fixed = cube_body(&mut doc, Vec3::ZERO, Vec3::splat(10.0));
        // The same size as the fixed block, so the two corner pairs are consistent — a rigid
        // placement can't stretch the part to bridge a mismatch.
        let moving = cube_body(&mut doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(10.0));
        let mate = JointMate {
            moving_face: Some(face_ref(&doc, moving, Vec3::new(45.0, 5.0, 0.0))),
            fixed_face: Some(face_ref(&doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            line_up: vec![
                MateLineUp {
                    moving: Some(vertex_ref(moving, Vec3::new(40.0, 0.0, 0.0))),
                    fixed: Some(vertex_ref(fixed, Vec3::new(0.0, 0.0, 10.0))),
                },
                MateLineUp {
                    moving: Some(vertex_ref(moving, Vec3::new(50.0, 0.0, 0.0))),
                    fixed: Some(vertex_ref(fixed, Vec3::new(10.0, 0.0, 10.0))),
                },
            ],
            ..Default::default()
        };
        let p = placement(&doc, &mate, Mat4::IDENTITY).unwrap();
        assert_eq!(p.open_freedoms, 0, "nothing is left to pin");
        let a = p.transform.transform_point3(Vec3::new(40.0, 0.0, 0.0));
        assert!((a - Vec3::new(0.0, 0.0, 10.0)).length() < 1e-2, "corner at {a}");
        let b = p.transform.transform_point3(Vec3::new(50.0, 0.0, 0.0));
        assert!(
            (b - Vec3::new(10.0, 0.0, 10.0)).length() < 1e-2,
            "the second corner lands on its own target too, got {b}"
        );
    }

    /// #1015: an edge row makes the projections collinear — it pins the spin, and a part
    /// that has to turn a long way round still finds it.
    #[test]
    fn an_edge_row_pins_the_spin() {
        let mut doc = Document::default();
        let fixed = cube_body(&mut doc, Vec3::ZERO, Vec3::splat(10.0));
        let moving = cube_body(&mut doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(4.0));
        let q = crate::hierarchy::quantize_body_point;
        let mate = JointMate {
            moving_face: Some(face_ref(&doc, moving, Vec3::new(42.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(&doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            line_up: vec![MateLineUp {
                // The moving cube's edge along +Y at its far side, and the fixed cube's
                // edge along +X: lining them up asks for a quarter turn.
                moving: Some(MateRef::Edge {
                    body: moving,
                    a: q(Vec3::new(44.0, 0.0, 0.0)),
                    b: q(Vec3::new(44.0, 4.0, 0.0)),
                }),
                fixed: Some(MateRef::Edge {
                    body: fixed,
                    a: q(Vec3::new(0.0, 0.0, 10.0)),
                    b: q(Vec3::new(10.0, 0.0, 10.0)),
                }),
            }],
            ..Default::default()
        };
        let p = placement(&doc, &mate, Mat4::IDENTITY).unwrap();
        assert_eq!(p.open_freedoms, 1, "the slide along the line is left");
        let a = p.transform.transform_point3(Vec3::new(44.0, 0.0, 0.0));
        let b = p.transform.transform_point3(Vec3::new(44.0, 4.0, 0.0));
        assert!((a.y).abs() < 1e-2 && (b.y).abs() < 1e-2, "collinear: {a} {b}");
        assert!((b - a).normalize().dot(Vec3::X).abs() > 0.999, "aimed along X");
    }

    /// #1016: a row that changes nothing isn't offered — a second point on top of a pinned
    /// one, or a second edge parallel to one already made collinear.
    #[test]
    fn a_row_that_pins_nothing_is_refused() {
        let mut doc = Document::default();
        let fixed = cube_body(&mut doc, Vec3::ZERO, Vec3::splat(10.0));
        let moving = cube_body(&mut doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(4.0));
        let mut mate = JointMate {
            moving_face: Some(face_ref(&doc, moving, Vec3::new(42.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(&doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            ..Default::default()
        };
        let first = MateLineUp {
            moving: Some(vertex_ref(moving, Vec3::new(40.0, 0.0, 0.0))),
            fixed: Some(vertex_ref(fixed, Vec3::new(0.0, 0.0, 10.0))),
        };
        assert!(row_pins_something(&doc, &mate, Mat4::IDENTITY, &first));
        mate.line_up.push(first);
        // The very same pair again pins nothing.
        assert!(!row_pins_something(&doc, &mate, Mat4::IDENTITY, &first));
        // A different corner does — it takes the spin.
        let second = MateLineUp {
            moving: Some(vertex_ref(moving, Vec3::new(44.0, 0.0, 0.0))),
            fixed: Some(vertex_ref(fixed, Vec3::new(10.0, 0.0, 10.0))),
        };
        assert!(row_pins_something(&doc, &mate, Mat4::IDENTITY, &second));
        mate.line_up.push(second);
        // Fully placed: nothing more is offered at all.
        assert!(!row_pins_something(&doc, &mate, Mat4::IDENTITY, &second));
        let third = MateLineUp {
            moving: Some(vertex_ref(moving, Vec3::new(44.0, 4.0, 0.0))),
            fixed: Some(vertex_ref(fixed, Vec3::new(10.0, 10.0, 10.0))),
        };
        assert!(!row_pins_something(&doc, &mate, Mat4::IDENTITY, &third));
    }

    /// #1018: the fixed side takes a datum plane, so the first part of an assembly can be
    /// grounded against the world.
    #[test]
    fn a_datum_plane_grounds_the_first_part() {
        let mut doc = Document::default();
        let moving = cube_body(&mut doc, Vec3::new(0.0, 0.0, 20.0), Vec3::splat(4.0));
        // The XY datum plane at the origin, normal +Z.
        doc.construction_planes[0].origin = Vec3::ZERO;
        doc.construction_planes[0].normal = Vec3::Z;
        let mate = JointMate {
            moving_face: Some(face_ref(&doc, moving, Vec3::new(2.0, 2.0, 20.0))),
            fixed_face: Some(MateRef::Plane(0)),
            ..Default::default()
        };
        let p = placement(&doc, &mate, Mat4::IDENTITY).unwrap();
        let landed = p.transform.transform_point3(Vec3::new(2.0, 2.0, 20.0));
        assert!((landed.z).abs() < 1e-3, "sits on the datum plane, got {landed}");
    }

    /// #1019: a mate whose picks no longer resolve places nothing, so the parts stay put.
    #[test]
    fn an_unresolvable_mate_places_nothing() {
        let mut doc = Document::default();
        let fixed = cube_body(&mut doc, Vec3::ZERO, Vec3::splat(10.0));
        let moving = cube_body(&mut doc, Vec3::new(40.0, 0.0, 0.0), Vec3::splat(4.0));
        let mut mate = JointMate {
            moving_face: Some(face_ref(&doc, moving, Vec3::new(42.0, 2.0, 0.0))),
            fixed_face: Some(face_ref(&doc, fixed, Vec3::new(5.0, 5.0, 10.0))),
            ..Default::default()
        };
        assert!(placement(&doc, &mate, Mat4::IDENTITY).is_some());
        mate.moving_face = Some(MateRef::Face {
            body: moving,
            centroid: [999, 999, 999],
            normal: [0, 0, 1000],
        });
        assert!(placement(&doc, &mate, Mat4::IDENTITY).is_none());
    }
}
