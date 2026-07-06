//! SolveSpace's constraint solver (libslvs) — the sketch solver's numeric backend
//! (`slvs` feature, on by default). Natively it is linked statically (build.rs); on the
//! web it lives inside the emscripten kernel module and is reached through
//! web/kernel-bridge.js. When neither is available (lean `--no-default-features` build,
//! or the kernel module failed to load in the browser), sketch solving falls back to
//! the built-in Levenberg-Marquardt solver in `newton.rs`.
//!
//! Only the *solve* is replaced: the document model, the constraint kinds, the DOF/rank
//! analysis (`dof.rs`), and the drag plumbing stay as they are. This module maps one
//! sketch's geometry and constraints onto a `Slvs_System`, solves group 2, and hands the
//! new point positions back. libslvs converges from the initial guess and favours
//! `dragged` parameters, which naturally covers what the native solver emulates with
//! gauge/reference holds.
//!
//! Known semantic gaps (fine for the experiment, revisit before adopting):
//! - `Angle` maps to SLVS_C_ANGLE, which constrains the *unsigned* angle between the two
//!   line directions (cosine-based); BearCAD's equation is signed. The mirrored solution
//!   is far from the initial guess, so Newton practically never jumps to it.
//! - Signed distances (`side`) map to the sign of the currently measured offset, the
//!   same natural-sign convention the constraint was created with.

use crate::model::{
    ConstraintEntity, ConstraintKind, ConstraintLine, ConstraintPoint, Document,
    DistanceTarget, FaceId, LineEnd, SketchId,
};
use crate::geometric_constraints::point_uv;
use crate::value::{eval_angle_rad_in_doc, eval_length_mm_in_doc};
use std::collections::HashMap;

const GROUP_FIXED: u32 = 1;
const GROUP_SOLVE: u32 = 2;

const SLVS_E_POINT_IN_3D: i32 = 50000;
const SLVS_E_POINT_IN_2D: i32 = 50001;
const SLVS_E_NORMAL_IN_3D: i32 = 60000;
const SLVS_E_DISTANCE: i32 = 70000;
const SLVS_E_WORKPLANE: i32 = 80000;
const SLVS_E_LINE_SEGMENT: i32 = 80001;
const SLVS_E_CIRCLE: i32 = 80003;

const SLVS_C_POINTS_COINCIDENT: i32 = 100000;
const SLVS_C_PT_PT_DISTANCE: i32 = 100001;
const SLVS_C_PT_LINE_DISTANCE: i32 = 100003;
#[allow(dead_code)] // see the coincident mapping for why this is deliberately unused
const SLVS_C_PT_ON_LINE: i32 = 100006;
const SLVS_C_EQUAL_LENGTH_LINES: i32 = 100008;
const SLVS_C_AT_MIDPOINT: i32 = 100018;
const SLVS_C_HORIZONTAL: i32 = 100019;
const SLVS_C_VERTICAL: i32 = 100020;
const SLVS_C_DIAMETER: i32 = 100021;
const SLVS_C_PT_ON_CIRCLE: i32 = 100022;
const SLVS_C_WHERE_DRAGGED: i32 = 100031;
const SLVS_C_ANGLE: i32 = 100024;
const SLVS_C_PARALLEL: i32 = 100025;
const SLVS_C_PERPENDICULAR: i32 = 100026;

const SLVS_RESULT_OKAY: i32 = 0;
/// Handle offset for the second slvs constraint of a two-equation document constraint.
const SECONDARY_HANDLE_BASE: u32 = 1_000_000;
const SLVS_RESULT_REDUNDANT_OKAY: i32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SlvsParam {
    h: u32,
    group: u32,
    val: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SlvsEntity {
    h: u32,
    group: u32,
    type_: i32,
    wrkpl: u32,
    point: [u32; 4],
    normal: u32,
    distance: u32,
    param: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SlvsConstraint {
    h: u32,
    group: u32,
    type_: i32,
    wrkpl: u32,
    val_a: f64,
    pt_a: u32,
    pt_b: u32,
    entity_a: u32,
    entity_b: u32,
    entity_c: u32,
    entity_d: u32,
    other: i32,
    other2: i32,
}

/// Raw solve result, decoded from the shim's flat outputs.
struct RawSolve {
    result: i32,
    dof: i32,
    failed: Vec<u32>,
    /// New parameter values, same order as the input params.
    vals: Vec<f64>,
}

#[cfg(not(target_arch = "wasm32"))]
extern "C" {
    // cpp/bearcad_slvs.cpp, compiled by build.rs.
    fn bearcad_slvs_solve(
        params: *const f64,
        nparams: i32,
        entities: *const f64,
        nentities: i32,
        constraints: *const f64,
        nconstraints: i32,
        dragged: *const f64,
        ndragged: i32,
        out_vals: *mut f64,
        out_failed: *mut u32,
        max_faileds: i32,
        out_nfaileds: *mut i32,
        out_dof: *mut i32,
    ) -> i32;
}

fn flatten_params(params: &[SlvsParam]) -> Vec<f64> {
    let mut out = Vec::with_capacity(params.len() * 3);
    for p in params {
        out.extend_from_slice(&[p.h as f64, p.group as f64, p.val]);
    }
    out
}

fn flatten_entities(entities: &[SlvsEntity]) -> Vec<f64> {
    let mut out = Vec::with_capacity(entities.len() * 14);
    for e in entities {
        out.extend_from_slice(&[
            e.h as f64,
            e.group as f64,
            e.type_ as f64,
            e.wrkpl as f64,
            e.point[0] as f64,
            e.point[1] as f64,
            e.point[2] as f64,
            e.point[3] as f64,
            e.normal as f64,
            e.distance as f64,
            e.param[0] as f64,
            e.param[1] as f64,
            e.param[2] as f64,
            e.param[3] as f64,
        ]);
    }
    out
}

fn flatten_constraints(constraints: &[SlvsConstraint]) -> Vec<f64> {
    let mut out = Vec::with_capacity(constraints.len() * 13);
    for c in constraints {
        out.extend_from_slice(&[
            c.h as f64,
            c.group as f64,
            c.type_ as f64,
            c.wrkpl as f64,
            c.val_a,
            c.pt_a as f64,
            c.pt_b as f64,
            c.entity_a as f64,
            c.entity_b as f64,
            c.entity_c as f64,
            c.entity_d as f64,
            c.other as f64,
            c.other2 as f64,
        ]);
    }
    out
}

/// Run one solve through the shim. `None` when the backend isn't reachable (web build
/// with the kernel module missing).
fn raw_solve(b: &Builder) -> Option<RawSolve> {
    let params = flatten_params(&b.params);
    let entities = flatten_entities(&b.entities);
    let constraints = flatten_constraints(&b.constraints);
    let dragged: Vec<f64> = b.dragged.iter().map(|&h| h as f64).collect();

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut vals = vec![0.0f64; b.params.len()];
        let mut failed = vec![0u32; b.constraints.len().max(1)];
        let mut nfaileds: i32 = 0;
        let mut dof: i32 = 0;
        // libslvs is not thread-safe (its temporary arena is process-global state), so
        // serialize solves. The app only solves from the UI thread; this protects
        // parallel test runs.
        static SOLVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let result = {
            let _guard = SOLVE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            unsafe {
                bearcad_slvs_solve(
                    params.as_ptr(),
                    b.params.len() as i32,
                    entities.as_ptr(),
                    b.entities.len() as i32,
                    constraints.as_ptr(),
                    b.constraints.len() as i32,
                    dragged.as_ptr(),
                    dragged.len() as i32,
                    vals.as_mut_ptr(),
                    failed.as_mut_ptr(),
                    failed.len() as i32,
                    &mut nfaileds,
                    &mut dof,
                )
            }
        };
        failed.truncate(nfaileds.max(0) as usize);
        Some(RawSolve {
            result,
            dof,
            failed,
            vals,
        })
    }
    #[cfg(all(target_arch = "wasm32", feature = "occt"))]
    {
        // Packed as [result, dof, nfaileds, ...failed, ...vals] by web/kernel-bridge.js.
        let out = crate::kernel::slvs_solve(&params, &entities, &constraints, &dragged)?;
        if out.len() < 3 {
            return None;
        }
        let result = out[0] as i32;
        let dof = out[1] as i32;
        let nfaileds = (out[2].max(0.0)) as usize;
        if out.len() < 3 + nfaileds + b.params.len() {
            return None;
        }
        let failed = out[3..3 + nfaileds].iter().map(|&h| h as u32).collect();
        let vals = out[3 + nfaileds..3 + nfaileds + b.params.len()].to_vec();
        Some(RawSolve {
            result,
            dof,
            failed,
            vals,
        })
    }
    #[cfg(all(target_arch = "wasm32", not(feature = "occt")))]
    {
        let _ = (params, entities, constraints, dragged);
        None
    }
}

/// Outcome of one libslvs solve, before anything is written back to the document.
pub struct SlvsOutcome {
    pub success: bool,
    #[allow(dead_code)]
    pub dof: i32,
    /// Document constraint indices libslvs blames for a failed solve.
    pub failed_constraints: Vec<usize>,
    /// New `(point, (u, v))` positions for every free sketch point.
    moved_points: Vec<(ConstraintPoint, (f32, f32))>,
    /// New radii per circle index.
    circle_radii: Vec<(usize, f32)>,
}

impl SlvsOutcome {
    /// Write the solved positions into the document (the counterpart of the native
    /// bridge's `apply_to_document`).
    pub fn apply_to_document(&self, doc: &mut Document, sketch: SketchId) -> Result<(), String> {
        for (point, (u, v)) in &self.moved_points {
            crate::geometric_constraints::set_point_uv(doc, sketch, point.clone(), *u, *v)?;
        }
        for (index, r) in &self.circle_radii {
            if let Some(c) = doc.circles.get_mut(*index) {
                c.r = *r;
            }
        }
        Ok(())
    }
}

/// System under construction for one sketch.
struct Builder<'a> {
    doc: &'a Document,
    sketch: SketchId,
    params: Vec<SlvsParam>,
    entities: Vec<SlvsEntity>,
    constraints: Vec<SlvsConstraint>,
    dragged: Vec<u32>,
    workplane: u32,
    wp_normal: u32,
    /// Free 2D points: `point -> (entity, param_u, param_v)`.
    points: HashMap<ConstraintPoint, (u32, u32, u32)>,
    /// Fixed helper points (origin, face vertices) share the map with a `None` param pair.
    fixed_points: HashMap<ConstraintPoint, u32>,
    origin_point: Option<u32>,
    lines: HashMap<ConstraintLine, u32>,
    circles: HashMap<usize, (u32, u32)>, // index -> (entity, radius param)
}

impl<'a> Builder<'a> {
    fn new(doc: &'a Document, sketch: SketchId) -> Self {
        let mut b = Builder {
            doc,
            sketch,
            params: Vec::new(),
            entities: Vec::new(),
            constraints: Vec::new(),
            dragged: Vec::new(),
            workplane: 0,
            wp_normal: 0,
            points: HashMap::new(),
            fixed_points: HashMap::new(),
            origin_point: None,
            lines: HashMap::new(),
            circles: HashMap::new(),
        };
        // The workplane: sketch-local (u, v) coordinates live directly in it, so it is
        // simply the XY plane at the origin. Everything about it is fixed (group 1).
        let px = b.param(GROUP_FIXED, 0.0);
        let py = b.param(GROUP_FIXED, 0.0);
        let pz = b.param(GROUP_FIXED, 0.0);
        let origin3d = b.entity(SlvsEntity {
            group: GROUP_FIXED,
            type_: SLVS_E_POINT_IN_3D,
            param: [px, py, pz, 0],
            ..Default::default()
        });
        let (qw, qx, qy, qz) = (
            b.param(GROUP_FIXED, 1.0),
            b.param(GROUP_FIXED, 0.0),
            b.param(GROUP_FIXED, 0.0),
            b.param(GROUP_FIXED, 0.0),
        );
        let normal = b.entity(SlvsEntity {
            group: GROUP_FIXED,
            type_: SLVS_E_NORMAL_IN_3D,
            param: [qw, qx, qy, qz],
            ..Default::default()
        });
        let wp = b.entity(SlvsEntity {
            group: GROUP_FIXED,
            type_: SLVS_E_WORKPLANE,
            point: [origin3d, 0, 0, 0],
            normal,
            ..Default::default()
        });
        b.workplane = wp;
        b.wp_normal = normal;
        b
    }

    fn param(&mut self, group: u32, val: f64) -> u32 {
        let h = self.params.len() as u32 + 1;
        self.params.push(SlvsParam { h, group, val });
        h
    }

    fn entity(&mut self, mut e: SlvsEntity) -> u32 {
        e.h = self.entities.len() as u32 + 1;
        self.entities.push(e);
        e.h
    }

    fn point2d(&mut self, group: u32, u: f64, v: f64) -> (u32, u32, u32) {
        let pu = self.param(group, u);
        let pv = self.param(group, v);
        let e = self.entity(SlvsEntity {
            group,
            type_: SLVS_E_POINT_IN_2D,
            wrkpl: self.workplane,
            param: [pu, pv, 0, 0],
            ..Default::default()
        });
        (e, pu, pv)
    }

    /// The slvs entity for a sketch point: free (group 2) for line endpoints and circle
    /// centers, fixed (group 1) for face vertices, which belong to solid geometry.
    fn ensure_point(&mut self, point: &ConstraintPoint) -> Result<u32, String> {
        if let Some((e, _, _)) = self.points.get(point) {
            return Ok(*e);
        }
        if let Some(e) = self.fixed_points.get(point) {
            return Ok(*e);
        }
        let (u, v) = point_uv(self.doc, self.sketch, point.clone())?;
        match point {
            ConstraintPoint::FaceVertex { .. } => {
                let (e, _, _) = self.point2d(GROUP_FIXED, u as f64, v as f64);
                self.fixed_points.insert(point.clone(), e);
                Ok(e)
            }
            _ => {
                let (e, pu, pv) = self.point2d(GROUP_SOLVE, u as f64, v as f64);
                self.points.insert(point.clone(), (e, pu, pv));
                Ok(e)
            }
        }
    }

    fn ensure_origin(&mut self) -> u32 {
        if let Some(e) = self.origin_point {
            return e;
        }
        let (e, _, _) = self.point2d(GROUP_FIXED, 0.0, 0.0);
        self.origin_point = Some(e);
        e
    }

    fn ensure_line(&mut self, line: &ConstraintLine) -> Result<u32, String> {
        if let Some(e) = self.lines.get(line) {
            return Ok(*e);
        }
        let (a, b) = match line {
            ConstraintLine::Line(index) => (
                self.ensure_point(&ConstraintPoint::LineEndpoint {
                    line: *index,
                    end: LineEnd::Start,
                })?,
                self.ensure_point(&ConstraintPoint::LineEndpoint {
                    line: *index,
                    end: LineEnd::End,
                })?,
            ),
            ConstraintLine::FaceEdge { face, index } => {
                let n = face_loop_len(self.doc, face)
                    .ok_or_else(|| "Face edge no longer resolves".to_string())?;
                if n == 0 {
                    return Err("Face edge no longer resolves".to_string());
                }
                (
                    self.ensure_point(&ConstraintPoint::FaceVertex {
                        face: face.clone(),
                        index: *index,
                    })?,
                    self.ensure_point(&ConstraintPoint::FaceVertex {
                        face: face.clone(),
                        index: (*index + 1) % n,
                    })?,
                )
            }
            // A fixed reference line from the origin along the axis direction (#189).
            ConstraintLine::OriginAxis(axis) => {
                let origin = self.ensure_origin();
                let (dx, dy) = match axis {
                    crate::model::SketchAxis::X => (1.0, 0.0),
                    crate::model::SketchAxis::Y => (0.0, 1.0),
                };
                let (dir, _, _) = self.point2d(GROUP_FIXED, dx, dy);
                (origin, dir)
            }
        };
        let group = if matches!(
            line,
            ConstraintLine::FaceEdge { .. } | ConstraintLine::OriginAxis(_)
        ) {
            GROUP_FIXED
        } else {
            GROUP_SOLVE
        };
        let e = self.entity(SlvsEntity {
            group,
            type_: SLVS_E_LINE_SEGMENT,
            wrkpl: self.workplane,
            point: [a, b, 0, 0],
            ..Default::default()
        });
        self.lines.insert(line.clone(), e);
        Ok(e)
    }

    fn ensure_circle(&mut self, index: usize) -> Result<u32, String> {
        if let Some((e, _)) = self.circles.get(&index) {
            return Ok(*e);
        }
        let circle = self
            .doc
            .circles
            .get(index)
            .filter(|c| !c.deleted)
            .ok_or_else(|| format!("Circle {index} not found"))?;
        let center = self.ensure_point(&ConstraintPoint::CircleCenter(index))?;
        // The radius is only a solver unknown when a diameter dimension drives it;
        // otherwise it is whatever the user set, and constraints like point-on-circle
        // must move the point, not quietly resize the circle.
        let has_diameter_dim = self.doc.constraints.iter().any(|c| {
            !c.deleted
                && c.sketch == self.sketch
                && matches!(
                    c.kind,
                    ConstraintKind::Distance {
                        target: DistanceTarget::CircleDiameter(i)
                    } if i == index
                )
        });
        let r_group = if has_diameter_dim { GROUP_SOLVE } else { GROUP_FIXED };
        let r = self.param(r_group, circle.r as f64);
        let dist = self.entity(SlvsEntity {
            group: r_group,
            type_: SLVS_E_DISTANCE,
            wrkpl: self.workplane,
            param: [r, 0, 0, 0],
            ..Default::default()
        });
        let e = self.entity(SlvsEntity {
            group: GROUP_SOLVE,
            type_: SLVS_E_CIRCLE,
            wrkpl: self.workplane,
            point: [center, 0, 0, 0],
            normal: self.wp_normal,
            distance: dist,
            ..Default::default()
        });
        self.circles.insert(index, (e, r));
        Ok(e)
    }

    fn constraint(&mut self, doc_index: usize, mut c: SlvsConstraint) {
        // Handle = document constraint index + 1, so a failed handle maps straight back.
        c.h = doc_index as u32 + 1;
        c.group = GROUP_SOLVE;
        c.wrkpl = self.workplane;
        self.constraints.push(c);
    }

    /// Second slvs constraint for a document constraint that expands to two equations;
    /// handles live `SECONDARY_HANDLE_BASE` above the primary range.
    fn secondary_constraint(&mut self, doc_index: usize, mut c: SlvsConstraint) {
        c.h = SECONDARY_HANDLE_BASE + doc_index as u32 + 1;
        c.group = GROUP_SOLVE;
        c.wrkpl = self.workplane;
        self.constraints.push(c);
    }

    /// Map one document constraint. Unknown/unevaluable constraints are skipped, exactly
    /// like the native bridge skips dimensions whose expressions don't evaluate.
    fn add_constraint(&mut self, doc_index: usize, kind: &ConstraintKind, expression: &str) -> Result<(), String> {
        match kind {
            ConstraintKind::Distance { target } => {
                self.add_distance(doc_index, target, expression)?
            }
            ConstraintKind::Coincident { a, b } => self.add_coincident(doc_index, a, b)?,
            ConstraintKind::Horizontal { line } => {
                let e = self.ensure_line(line)?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_HORIZONTAL,
                    entity_a: e,
                    ..Default::default()
                });
            }
            ConstraintKind::Vertical { line } => {
                let e = self.ensure_line(line)?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_VERTICAL,
                    entity_a: e,
                    ..Default::default()
                });
            }
            ConstraintKind::Parallel { line_a, line_b } => {
                let (a, b) = (self.ensure_line(line_a)?, self.ensure_line(line_b)?);
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PARALLEL,
                    entity_a: a,
                    entity_b: b,
                    ..Default::default()
                });
            }
            ConstraintKind::Perpendicular { line_a, line_b } => {
                let (a, b) = (self.ensure_line(line_a)?, self.ensure_line(line_b)?);
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PERPENDICULAR,
                    entity_a: a,
                    entity_b: b,
                    ..Default::default()
                });
            }
            ConstraintKind::Equal { line_a, line_b } => {
                let (a, b) = (self.ensure_line(line_a)?, self.ensure_line(line_b)?);
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_EQUAL_LENGTH_LINES,
                    entity_a: a,
                    entity_b: b,
                    ..Default::default()
                });
            }
            ConstraintKind::Midpoint { point, line } => {
                let p = self.ensure_point(point)?;
                let l = self.ensure_line(line)?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_AT_MIDPOINT,
                    pt_a: p,
                    entity_a: l,
                    ..Default::default()
                });
            }
            ConstraintKind::Angle {
                line_a,
                line_b,
                rotation_sign: _,
            } => {
                let Some(angle) = eval_angle_rad_in_doc(expression, self.doc) else {
                    return Ok(());
                };
                if angle <= 0.0 || angle >= std::f32::consts::PI {
                    return Ok(());
                }
                let (a, b) = (self.ensure_line(line_a)?, self.ensure_line(line_b)?);
                // SLVS_C_ANGLE constrains the unsigned angle between the two direction
                // vectors (see the module comment on signedness).
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_ANGLE,
                    entity_a: a,
                    entity_b: b,
                    val_a: (angle as f64).to_degrees(),
                    ..Default::default()
                });
            }
        }
        Ok(())
    }

    fn add_distance(
        &mut self,
        doc_index: usize,
        target: &DistanceTarget,
        expression: &str,
    ) -> Result<(), String> {
        let Some(value) = eval_length_mm_in_doc(expression, self.doc) else {
            return Ok(());
        };
        if value <= 0.0 {
            return Ok(());
        }
        let value = value as f64;
        match target {
            DistanceTarget::LineLength(index) => {
                let a = self.ensure_point(&ConstraintPoint::LineEndpoint {
                    line: *index,
                    end: LineEnd::Start,
                })?;
                let b = self.ensure_point(&ConstraintPoint::LineEndpoint {
                    line: *index,
                    end: LineEnd::End,
                })?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PT_PT_DISTANCE,
                    pt_a: a,
                    pt_b: b,
                    val_a: value,
                    ..Default::default()
                });
            }
            DistanceTarget::CircleDiameter(index) => {
                let c = self.ensure_circle(*index)?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_DIAMETER,
                    entity_a: c,
                    val_a: value,
                    ..Default::default()
                });
            }
            DistanceTarget::PointPointDistance { anchor, mover, .. } => {
                let a = self.ensure_point(anchor)?;
                let b = self.ensure_point(mover)?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PT_PT_DISTANCE,
                    pt_a: a,
                    pt_b: b,
                    val_a: value,
                    ..Default::default()
                });
            }
            DistanceTarget::PointLineDistance { point, line, .. } => {
                let p = self.ensure_point(point)?;
                let l = self.ensure_line(line)?;
                // slvs point–line distance is signed in a workplane; keep the sign the
                // geometry currently has (the natural sign the constraint was made with).
                let signed = self
                    .measured_point_line_signed(point, line)
                    .unwrap_or(1.0);
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PT_LINE_DISTANCE,
                    pt_a: p,
                    entity_a: l,
                    val_a: value * signed.signum(),
                    ..Default::default()
                });
            }
            DistanceTarget::LineLineDistance { line_a, line_b, .. } => {
                // Two lines a distance apart: both endpoints of the movable line sit at
                // the signed distance from the reference line (which also keeps them
                // parallel, matching the native two-equation formulation). The second
                // equation uses the offset handle range; both map back to `doc_index`.
                let (start, end) = line_endpoints(line_b);
                let (Some(start), Some(end)) = (start, end) else {
                    return Ok(());
                };
                let l = self.ensure_line(line_a)?;
                let signed = self
                    .measured_point_line_signed(&start, line_a)
                    .unwrap_or(1.0);
                let p0 = self.ensure_point(&start)?;
                let p1 = self.ensure_point(&end)?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PT_LINE_DISTANCE,
                    pt_a: p0,
                    entity_a: l,
                    val_a: value * signed.signum(),
                    ..Default::default()
                });
                self.secondary_constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PT_LINE_DISTANCE,
                    pt_a: p1,
                    entity_a: l,
                    val_a: value * signed.signum(),
                    ..Default::default()
                });
            }
        }
        Ok(())
    }

    fn add_coincident(
        &mut self,
        doc_index: usize,
        a: &ConstraintEntity,
        b: &ConstraintEntity,
    ) -> Result<(), String> {
        use ConstraintEntity as E;
        match (a, b) {
            (E::Point(pa), E::Point(pb)) => {
                let (ea, eb) = (self.ensure_point(pa)?, self.ensure_point(pb)?);
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_POINTS_COINCIDENT,
                    pt_a: ea,
                    pt_b: eb,
                    ..Default::default()
                });
            }
            (E::Point(p), E::Origin) | (E::Origin, E::Point(p)) => {
                let ep = self.ensure_point(p)?;
                let eo = self.ensure_origin();
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_POINTS_COINCIDENT,
                    pt_a: ep,
                    pt_b: eo,
                    ..Default::default()
                });
            }
            (E::Point(p), E::Line(l)) | (E::Line(l), E::Point(p)) => {
                let ep = self.ensure_point(p)?;
                let el = self.ensure_line(l)?;
                // Not SLVS_C_PT_ON_LINE: that formulation adds an internal line-parameter
                // initialized to 0, which yanks the point toward the line's *start* on the
                // next solve instead of keeping it where it projects. A zero point-line
                // distance is the same geometric statement without the helper param.
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PT_LINE_DISTANCE,
                    pt_a: ep,
                    entity_a: el,
                    val_a: 0.0,
                    ..Default::default()
                });
            }
            (E::Point(p), E::Circle(c)) | (E::Circle(c), E::Point(p)) => {
                let ep = self.ensure_point(p)?;
                let ec = self.ensure_circle(*c)?;
                self.constraint(doc_index, SlvsConstraint {
                    type_: SLVS_C_PT_ON_CIRCLE,
                    pt_a: ep,
                    entity_a: ec,
                    ..Default::default()
                });
            }
            // Line–line / circle–circle coincidence isn't produced by the UI; skip.
            _ => {}
        }
        Ok(())
    }

    fn measured_point_line_signed(
        &self,
        point: &ConstraintPoint,
        line: &ConstraintLine,
    ) -> Option<f64> {
        let (p_u, p_v) = point_uv(self.doc, self.sketch, point.clone()).ok()?;
        let (a, b) = line_endpoints_uv(self.doc, self.sketch, line)?;
        let d = (b.0 - a.0, b.1 - a.1);
        let len = (d.0 * d.0 + d.1 * d.1).sqrt();
        if len < 1e-9 {
            return None;
        }
        // libslvs's PointLineDistance builds its direction as (a - b), so its signed
        // distance is the negation of the usual (b - a) cross product.
        Some((-(d.0 * (p_v - a.1) - d.1 * (p_u - a.0)) / len) as f64)
    }
}

fn face_loop_len(doc: &Document, face: &FaceId) -> Option<usize> {
    crate::extrude::face_boundary_loop_world(doc, face).map(|l| l.len())
}

fn line_endpoints(line: &ConstraintLine) -> (Option<ConstraintPoint>, Option<ConstraintPoint>) {
    match line {
        ConstraintLine::Line(index) => (
            Some(ConstraintPoint::LineEndpoint {
                line: *index,
                end: LineEnd::Start,
            }),
            Some(ConstraintPoint::LineEndpoint {
                line: *index,
                end: LineEnd::End,
            }),
        ),
        ConstraintLine::FaceEdge { .. } | ConstraintLine::OriginAxis(_) => (None, None),
    }
}

fn line_endpoints_uv(
    doc: &Document,
    sketch: SketchId,
    line: &ConstraintLine,
) -> Option<((f32, f32), (f32, f32))> {
    match line {
        ConstraintLine::Line(index) => {
            let l = doc.lines.get(*index).filter(|l| !l.deleted)?;
            Some(((l.x0, l.y0), (l.x1, l.y1)))
        }
        ConstraintLine::FaceEdge { face, index } => {
            let n = face_loop_len(doc, face)?;
            if n == 0 {
                return None;
            }
            let a = point_uv(
                doc,
                sketch,
                ConstraintPoint::FaceVertex {
                    face: face.clone(),
                    index: *index,
                },
            )
            .ok()?;
            let b = point_uv(
                doc,
                sketch,
                ConstraintPoint::FaceVertex {
                    face: face.clone(),
                    index: (*index + 1) % n,
                },
            )
            .ok()?;
            Some((a, b))
        }
        ConstraintLine::OriginAxis(axis) => Some(match axis {
            crate::model::SketchAxis::X => ((0.0, 0.0), (1.0, 0.0)),
            crate::model::SketchAxis::Y => ((0.0, 0.0), (0.0, 1.0)),
        }),
    }
}

/// Build the system for `sketch`, solve group 2 with libslvs, and return the outcome.
/// Nothing is written back; call [`SlvsOutcome::apply_to_document`] for that.
pub fn solve_sketch(
    doc: &Document,
    sketch: SketchId,
    pins: &[(ConstraintPoint, (f32, f32))],
) -> Result<SlvsOutcome, String> {
    let mut b = Builder::new(doc, sketch);

    // Seed every sketch point so under-constrained geometry still round-trips (and so a
    // pin can reference a point no constraint mentions).
    for (index, line) in doc.lines.iter().enumerate() {
        if line.deleted || line.sketch != sketch {
            continue;
        }
        for end in [LineEnd::Start, LineEnd::End] {
            b.ensure_point(&ConstraintPoint::LineEndpoint { line: index, end })?;
        }
    }
    for (index, circle) in doc.circles.iter().enumerate() {
        if circle.deleted || circle.sketch != sketch {
            continue;
        }
        b.ensure_circle(index)?;
    }

    for (doc_index, constraint) in doc.constraints.iter().enumerate() {
        if constraint.deleted || constraint.sketch != sketch {
            continue;
        }
        if !crate::document_lifecycle::constraint_kind_applicable(doc, &constraint.kind) {
            continue;
        }
        b.add_constraint(doc_index, &constraint.kind, &constraint.expression)?;
    }

    // Drag pins: move the parameter values to the drag target and mark them dragged, so
    // libslvs favours keeping them there — its native equivalent of the drag-pin weights.
    let mut dragged: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for (point, (u, v)) in pins {
        if let Some((_, pu, pv)) = b.points.get(point).copied() {
            b.params[pu as usize - 1].val = *u as f64;
            b.params[pv as usize - 1].val = *v as f64;
            dragged.insert(pu);
            dragged.insert(pv);
        }
    }

    // Reference-hold policy, ported from the native bridge (#137): each asymmetric
    // constraint has a side that should stay put (the anchor point, the reference line,
    // a dimensioned line's start), which the native solver expresses as weighted holds.
    // Here those params join `dragged`, libslvs's favour-unchanged mechanism. Points the
    // user is interactively dragging keep their pin value instead.
    //
    // During an interactive drag the policy narrows, exactly like the native bridge:
    // blanket holds would bias so many params that Newton struggles to converge, so only
    // the references whose *movable* side is being dragged are held.
    let pinned: std::collections::HashSet<&ConstraintPoint> =
        pins.iter().map(|(p, _)| p).collect();
    let dragging = !pins.is_empty();
    let line_pinned = |line: &ConstraintLine| -> bool {
        let (a, z) = line_endpoints(line);
        [a, z]
            .into_iter()
            .flatten()
            .any(|p| pinned.contains(&p))
    };
    let point_pinned = |p: &ConstraintPoint| pinned.contains(p);
    // Whether the reference side of a (reference, movable) pair should be held.
    let should_hold_pair = |reference: &ConstraintLine, movable_dragged: bool| -> bool {
        if !dragging {
            return true;
        }
        movable_dragged && !line_pinned(reference)
    };
    let mut hold_point = |b: &Builder, point: &ConstraintPoint, dragged: &mut std::collections::HashSet<u32>| {
        if pinned.contains(point) {
            return;
        }
        if let Some((_, pu, pv)) = b.points.get(point) {
            dragged.insert(*pu);
            dragged.insert(*pv);
        }
    };
    let hold_line = |b: &Builder, line: &ConstraintLine, dragged: &mut std::collections::HashSet<u32>,
                     hold_point: &mut dyn FnMut(&Builder, &ConstraintPoint, &mut std::collections::HashSet<u32>)| {
        let (a, z) = line_endpoints(line);
        for p in [a, z].into_iter().flatten() {
            hold_point(b, &p, dragged);
        }
    };
    let mut held_centers: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for constraint in doc.constraints.iter() {
        if constraint.deleted || constraint.sketch != sketch {
            continue;
        }
        use crate::geometric_constraints::parallel_reference_and_movable;
        match &constraint.kind {
            ConstraintKind::Distance { target } => match target {
                DistanceTarget::LineLength(index) => {
                    if !dragging {
                        hold_point(
                            &b,
                            &ConstraintPoint::LineEndpoint {
                                line: *index,
                                end: LineEnd::Start,
                            },
                            &mut dragged,
                        )
                    }
                }
                DistanceTarget::PointPointDistance { anchor, mover, .. } => {
                    if !dragging || (point_pinned(mover) && !point_pinned(anchor)) {
                        hold_point(&b, anchor, &mut dragged)
                    }
                }
                DistanceTarget::PointLineDistance { point, line, .. } => {
                    if should_hold_pair(line, point_pinned(point)) {
                        hold_line(&b, line, &mut dragged, &mut hold_point)
                    }
                }
                DistanceTarget::LineLineDistance { line_a, line_b, .. } => {
                    let (reference, movable) =
                        parallel_reference_and_movable(line_a.clone(), line_b.clone());
                    if should_hold_pair(&reference, line_pinned(&movable)) {
                        hold_line(&b, &reference, &mut dragged, &mut hold_point)
                    }
                }
                DistanceTarget::CircleDiameter(_) => {}
            },
            ConstraintKind::Parallel { line_a, line_b }
            | ConstraintKind::Perpendicular { line_a, line_b }
            | ConstraintKind::Equal { line_a, line_b }
            | ConstraintKind::Angle { line_a, line_b, .. } => {
                let (reference, movable) =
                    parallel_reference_and_movable(line_a.clone(), line_b.clone());
                if should_hold_pair(&reference, line_pinned(&movable)) {
                    hold_line(&b, &reference, &mut dragged, &mut hold_point)
                }
            }
            ConstraintKind::Midpoint { point, line } => {
                if should_hold_pair(line, point_pinned(point)) {
                    hold_line(&b, line, &mut dragged, &mut hold_point)
                }
            }
            ConstraintKind::Coincident { a, b: cb } => {
                use ConstraintEntity as E;
                match (a, cb) {
                    (E::Point(p), E::Line(l)) | (E::Line(l), E::Point(p)) => {
                        if should_hold_pair(l, point_pinned(p)) {
                            hold_line(&b, l, &mut dragged, &mut hold_point)
                        }
                    }
                    (E::Point(_), E::Circle(c)) | (E::Circle(c), E::Point(_)) => {
                        if !dragging {
                            hold_point(&b, &ConstraintPoint::CircleCenter(*c), &mut dragged);
                            // `dragged` only *favours* stillness; a point being pulled onto
                            // the perimeter from far away still bleeds a visible fraction
                            // of the motion into the centre. Lock it outright — BearCAD's
                            // semantic is that the point projects and the circle stays.
                            held_centers.insert(*c);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    for c in held_centers {
        let center = ConstraintPoint::CircleCenter(c);
        if pinned.contains(&center) {
            continue;
        }
        if let Some((e, _, _)) = b.points.get(&center).copied() {
            let h = SECONDARY_HANDLE_BASE * 2 + c as u32 + 1;
            b.constraints.push(SlvsConstraint {
                h,
                group: GROUP_SOLVE,
                type_: SLVS_C_WHERE_DRAGGED,
                wrkpl: b.workplane,
                pt_a: e,
                ..Default::default()
            });
        }
    }
    b.dragged = dragged.into_iter().collect();

    let raw = raw_solve(&b)
        .ok_or_else(|| "libslvs backend unavailable (kernel module not loaded)".to_string())?;

    if std::env::var_os("SLVS_DEBUG").is_some() {
        eprintln!(
            "SLVSDBG result={} dof={} faileds={}",
            raw.result,
            raw.dof,
            raw.failed.len()
        );
    }
    let success = raw.result == SLVS_RESULT_OKAY || raw.result == SLVS_RESULT_REDUNDANT_OKAY;
    let failed_constraints = if success {
        Vec::new()
    } else {
        let mut indices: Vec<usize> = raw
            .failed
            .iter()
            .filter(|&&h| h > 0)
            .filter(|&&h| h <= SECONDARY_HANDLE_BASE * 2)
            .map(|&h| {
                let h = if h > SECONDARY_HANDLE_BASE {
                    h - SECONDARY_HANDLE_BASE
                } else {
                    h
                };
                h as usize - 1
            })
            .collect();
        indices.sort_unstable();
        indices.dedup();
        indices
    };

    let moved_points = b
        .points
        .iter()
        .map(|(point, (_, pu, pv))| {
            (
                point.clone(),
                (
                    raw.vals[*pu as usize - 1] as f32,
                    raw.vals[*pv as usize - 1] as f32,
                ),
            )
        })
        .collect();
    let circle_radii = b
        .circles
        .iter()
        .map(|(index, (_, r))| (*index, raw.vals[*r as usize - 1] as f32))
        .collect();

    Ok(SlvsOutcome {
        success,
        dof: raw.dof,
        failed_constraints,
        moved_points,
        circle_radii,
    })
}
