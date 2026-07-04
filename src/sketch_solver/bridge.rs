//! Bridge between [`Document`] sketch geometry and the numeric solver.

use super::dof::{dof_remaining, vars_can_move_together};
use super::newton::{solve_lm, SolveReport, SolverConfig};
use super::residuals::{
    Equation, DEFAULT_WEIGHT, DRAG_PIN_WEIGHT, GAUGE_HOLD_WEIGHT, REFERENCE_HOLD_WEIGHT,
};
use crate::geometric_constraints::parallel_reference_and_movable;
use super::system::{System, VarId};
use crate::document_lifecycle::constraint_kind_applicable;
use crate::geometric_constraints::point_uv;
use crate::model::{
    ConstraintEntity, ConstraintKind, ConstraintLine, ConstraintPoint, DistanceTarget, Document,
    LineEnd, SketchId,
};
use crate::value::{eval_angle_rad_in_doc, eval_length_mm_in_doc};
use std::collections::{HashMap, HashSet};

/// Solver graph for one sketch, with stable point-variable mapping.
pub struct SketchBridge {
    pub system: System,
    sketch: SketchId,
    point_vars: HashMap<ConstraintPoint, (VarId, VarId)>,
    circle_radius: HashMap<usize, VarId>,
    hold_references: bool,
    constraint_equations: HashMap<usize, Vec<usize>>,
}

impl SketchBridge {
    pub fn from_document(
        doc: &Document,
        sketch: SketchId,
        hold_references: bool,
    ) -> Result<Self, String> {
        let mut bridge = Self {
            system: System::new(),
            sketch,
            point_vars: HashMap::new(),
            circle_radius: HashMap::new(),
            hold_references,
            constraint_equations: HashMap::new(),
        };
        bridge.seed_entities(doc)?;
        bridge.add_constraints(doc)?;
        Ok(bridge)
    }

    pub fn add_drag_pins(
        &mut self,
        doc: &Document,
        pins: &[(ConstraintPoint, (f32, f32))],
    ) {
        let pinned: HashSet<ConstraintPoint> = pins.iter().map(|(point, _)| point.clone()).collect();
        for (point, (u, v)) in pins {
            if let Some((u_id, v_id)) = self.point_vars.get(point).copied() {
                self.system.add_equation(Equation::Pin {
                    var: u_id,
                    target: *u as f64,
                    weight: DRAG_PIN_WEIGHT,
                });
                self.system.add_equation(Equation::Pin {
                    var: v_id,
                    target: *v as f64,
                    weight: DRAG_PIN_WEIGHT,
                });
            }
        }
        if self.hold_references {
            return;
        }
        // Reference geometry must stay put while the movable side is dragged. Holds were
        // dropped for the whole sketch (hold_references = false during drag), so re-pin the
        // reference of each direction/distance constraint when its movable side is the one
        // being dragged. Coincident anchors are intentionally left free so they still follow.
        for constraint in &doc.constraints {
            if constraint.deleted || constraint.sketch != self.sketch {
                continue;
            }
            match &constraint.kind {
                ConstraintKind::Distance {
                    target: DistanceTarget::PointPointDistance { anchor, mover, .. },
                } => {
                    if pinned.contains(mover) && !pinned.contains(anchor) {
                        let _ = self.anchor_point(doc, anchor.clone(), REFERENCE_HOLD_WEIGHT);
                    } else if pinned.contains(anchor) && !pinned.contains(mover) {
                        let _ = self.anchor_point(doc, mover.clone(), REFERENCE_HOLD_WEIGHT);
                    }
                }
                ConstraintKind::Distance {
                    target: DistanceTarget::PointLineDistance { point, line, .. },
                } => self.hold_reference_when_point_dragged(doc, line.clone(), point.clone(), &pinned),
                ConstraintKind::Midpoint { point, line } => {
                    self.hold_reference_when_point_dragged(doc, line.clone(), point.clone(), &pinned)
                }
                ConstraintKind::Distance {
                    target: DistanceTarget::LineLineDistance { line_a, line_b, .. },
                }
                | ConstraintKind::Parallel { line_a, line_b }
                | ConstraintKind::Perpendicular { line_a, line_b }
                | ConstraintKind::Equal { line_a, line_b }
                | ConstraintKind::Angle { line_a, line_b, .. } => {
                    let (reference, movable) =
                        parallel_reference_and_movable(line_a.clone(), line_b.clone());
                    self.hold_reference_when_movable_dragged(doc, reference, movable, &pinned);
                }
                _ => {}
            }
        }
    }

    /// Hold a constraint's reference line if the dragged geometry is the movable line (and the
    /// reference itself isn't being dragged).
    fn hold_reference_when_movable_dragged(
        &mut self,
        doc: &Document,
        reference: ConstraintLine,
        movable: ConstraintLine,
        pinned: &HashSet<ConstraintPoint>,
    ) {
        let reference_points = line_endpoint_points(doc, reference);
        let movable_points = line_endpoint_points(doc, movable);
        let movable_dragged = movable_points.iter().any(|p| pinned.contains(p));
        let reference_dragged = reference_points.iter().any(|p| pinned.contains(p));
        if movable_dragged && !reference_dragged {
            for point in reference_points {
                let _ = self.anchor_point(doc, point, REFERENCE_HOLD_WEIGHT);
            }
        }
    }

    /// Hold a reference line if the dragged geometry is the constrained point (and the line
    /// itself isn't being dragged).
    fn hold_reference_when_point_dragged(
        &mut self,
        doc: &Document,
        line: ConstraintLine,
        point: ConstraintPoint,
        pinned: &HashSet<ConstraintPoint>,
    ) {
        let reference_points = line_endpoint_points(doc, line);
        let reference_dragged = reference_points.iter().any(|p| pinned.contains(p));
        if pinned.contains(&point) && !reference_dragged {
            for reference_point in reference_points {
                let _ = self.anchor_point(doc, reference_point, REFERENCE_HOLD_WEIGHT);
            }
        }
    }

    pub fn solve(&mut self) -> SolveReport {
        let mut report = solve_lm(&mut self.system, SolverConfig::default());
        report.dof_remaining = dof_remaining(&self.system);
        if !report.success {
            report.failed_constraints = self.conflicting_constraints();
        }
        report
    }

    /// Constraint indices sorted by largest residual contribution (failed solves only).
    pub fn conflicting_constraints(&self) -> Vec<usize> {
        let residuals = self.system.residual_values();
        let mut scored: Vec<(usize, f64)> = self
            .constraint_equations
            .iter()
            .map(|(id, equations)| {
                let score = equations
                    .iter()
                    .map(|index| residuals[*index].abs())
                    .fold(0.0f64, f64::max);
                (*id, score)
            })
            .filter(|(_, score)| *score > 1e-9)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().map(|(id, _)| id).collect()
    }

    pub fn point_solver_vars(&mut self, doc: &Document, point: ConstraintPoint) -> Result<(VarId, VarId), String> {
        self.point_vars(doc, point)
    }

    pub fn apply_to_document(&self, doc: &mut Document) -> Result<(), String> {
        for (point, (u_id, v_id)) in &self.point_vars {
            if let ConstraintPoint::LineEndpoint { .. } = point {
                set_point_uv_from_solver(doc, self.sketch, point.clone(), self.system.value(*u_id), self.system.value(*v_id))?;
            }
        }

        for (circle, radius_var) in &self.circle_radius {
            let center = ConstraintPoint::CircleCenter(*circle);
            if let Some((u_id, v_id)) = self.point_vars.get(&center) {
                set_point_uv_from_solver(
                    doc,
                    self.sketch,
                    center,
                    self.system.value(*u_id),
                    self.system.value(*v_id),
                )?;
            }
            let entity = doc
                .circles
                .get_mut(*circle)
                .ok_or_else(|| format!("Circle {circle} not found"))?;
            entity.r = self.system.value(*radius_var) as f32;
        }
        Ok(())
    }

    fn seed_entities(&mut self, doc: &Document) -> Result<(), String> {
        for (index, line) in doc.lines.iter().enumerate() {
            if line.deleted || line.sketch != self.sketch {
                continue;
            }
            self.ensure_line_endpoint(doc, index, LineEnd::Start)?;
            self.ensure_line_endpoint(doc, index, LineEnd::End)?;
        }
        for (index, circle) in doc.circles.iter().enumerate() {
            if circle.deleted || circle.sketch != self.sketch {
                continue;
            }
            let center = ConstraintPoint::CircleCenter(index);
            if !self.point_vars.contains_key(&center) {
                let (u, v) = point_uv(doc, self.sketch, center.clone())?;
                let (u_id, v_id) = self.system.add_point(u as f64, v as f64, false);
                self.point_vars.insert(center, (u_id, v_id));
            }
            let radius_var = self.system.add_var(circle.r as f64, false);
            self.circle_radius.insert(index, radius_var);
        }
        Ok(())
    }

    fn ensure_line_endpoint(
        &mut self,
        doc: &Document,
        line: usize,
        end: LineEnd,
    ) -> Result<(), String> {
        let point = ConstraintPoint::LineEndpoint { line, end };
        if self.point_vars.contains_key(&point) {
            return Ok(());
        }
        let (u, v) = point_uv(doc, self.sketch, point.clone())?;
        let (u_id, v_id) = self.system.add_point(u as f64, v as f64, false);
        self.point_vars.insert(point, (u_id, v_id));
        Ok(())
    }

    fn add_constraints(&mut self, doc: &Document) -> Result<(), String> {
        for (index, constraint) in doc.constraints.iter().enumerate() {
            if constraint.deleted || constraint.sketch != self.sketch {
                continue;
            }
            if !constraint_kind_applicable(doc, &constraint.kind) {
                continue;
            }
            self.add_constraint(doc, index, constraint)?;
        }
        Ok(())
    }

    fn add_constraint(
        &mut self,
        doc: &Document,
        constraint_id: usize,
        constraint: &crate::model::Constraint,
    ) -> Result<(), String> {
        let eq_start = self.system.equations.len();
        let result = self.add_constraint_body(doc, constraint);
        let eq_end = self.system.equations.len();
        if eq_end > eq_start {
            self.constraint_equations
                .insert(constraint_id, (eq_start..eq_end).collect());
        }
        result
    }

    fn add_constraint_body(
        &mut self,
        doc: &Document,
        constraint: &crate::model::Constraint,
    ) -> Result<(), String> {
        match constraint.kind.clone() {
            ConstraintKind::Distance { target } => {
                self.add_distance_constraint(doc, constraint, target)?;
            }
            ConstraintKind::Horizontal { line } => {
                let ((x0, y0), (x1, y1)) = self.line_vars(doc, line)?;
                self.system.add_equation(Equation::Horizontal {
                    y0,
                    y1,
                    weight: DEFAULT_WEIGHT,
                });
                let _ = (x0, x1);
            }
            ConstraintKind::Vertical { line } => {
                let ((x0, y0), (x1, y1)) = self.line_vars(doc, line)?;
                self.system.add_equation(Equation::Vertical {
                    x0,
                    x1,
                    weight: DEFAULT_WEIGHT,
                });
                let _ = (y0, y1);
            }
            ConstraintKind::Parallel { line_a, line_b } => {
                let (reference, movable) = parallel_reference_and_movable(line_a, line_b);
                self.hold_line(doc, reference.clone(), GAUGE_HOLD_WEIGHT)?;
                let a = self.line_vars(doc, reference)?;
                let b = self.line_vars(doc, movable)?;
                let weight = self.direction_product_weight(a, b);
                self.system.add_equation(Equation::Parallel {
                    ax0: a.0.0,
                    ay0: a.0.1,
                    ax1: a.1.0,
                    ay1: a.1.1,
                    bx0: b.0.0,
                    by0: b.0.1,
                    bx1: b.1.0,
                    by1: b.1.1,
                    weight,
                });
            }
            ConstraintKind::Perpendicular { line_a, line_b } => {
                let (reference, movable) = parallel_reference_and_movable(line_a, line_b);
                self.hold_line(doc, reference.clone(), GAUGE_HOLD_WEIGHT)?;
                let a = self.line_vars(doc, reference)?;
                let b = self.line_vars(doc, movable)?;
                let weight = self.direction_product_weight(a, b);
                self.system.add_equation(Equation::Perpendicular {
                    ax0: a.0.0,
                    ay0: a.0.1,
                    ax1: a.1.0,
                    ay1: a.1.1,
                    bx0: b.0.0,
                    by0: b.0.1,
                    bx1: b.1.0,
                    by1: b.1.1,
                    weight,
                });
            }
            ConstraintKind::Equal { line_a, line_b } => {
                let (reference, movable) = parallel_reference_and_movable(line_a, line_b);
                self.hold_line(doc, reference.clone(), GAUGE_HOLD_WEIGHT)?;
                let a = self.line_vars(doc, reference)?;
                let b = self.line_vars(doc, movable)?;
                self.system.add_equation(Equation::EqualLength {
                    ax0: a.0.0,
                    ay0: a.0.1,
                    ax1: a.1.0,
                    ay1: a.1.1,
                    bx0: b.0.0,
                    by0: b.0.1,
                    bx1: b.1.0,
                    by1: b.1.1,
                    weight: DEFAULT_WEIGHT,
                });
            }
            ConstraintKind::Coincident { a, b } => self.add_coincident(doc, a, b)?,
            ConstraintKind::Midpoint { point, line } => {
                self.hold_line(doc, line.clone(), REFERENCE_HOLD_WEIGHT)?;
                let (pu, pv) = self.point_vars(doc, point)?;
                let ((x0, y0), (x1, y1)) = self.line_vars(doc, line)?;
                self.system.add_equation(Equation::MidpointU {
                    px: pu,
                    x0,
                    x1,
                    weight: DEFAULT_WEIGHT,
                });
                self.system.add_equation(Equation::MidpointV {
                    py: pv,
                    y0,
                    y1,
                    weight: DEFAULT_WEIGHT,
                });
            }
            ConstraintKind::Angle {
                line_a,
                line_b,
                rotation_sign,
            } => {
                let Some(angle) = eval_angle_rad_in_doc(&constraint.expression, doc) else {
                    return Ok(());
                };
                if angle <= 0.0 || angle >= std::f32::consts::PI {
                    return Ok(());
                }
                let (reference, movable) = parallel_reference_and_movable(line_a, line_b);
                self.hold_line(doc, reference.clone(), GAUGE_HOLD_WEIGHT)?;
                let a = self.line_vars(doc, reference)?;
                let b = self.line_vars(doc, movable)?;
                self.system.add_equation(Equation::Angle {
                    ax0: a.0.0,
                    ay0: a.0.1,
                    ax1: a.1.0,
                    ay1: a.1.1,
                    bx0: b.0.0,
                    by0: b.0.1,
                    bx1: b.1.0,
                    by1: b.1.1,
                    angle: rotation_sign as f64 * angle as f64,
                    weight: DEFAULT_WEIGHT,
                });
            }
        }
        Ok(())
    }

    fn add_distance_constraint(
        &mut self,
        doc: &Document,
        constraint: &crate::model::Constraint,
        target: DistanceTarget,
    ) -> Result<(), String> {
        let Some(value) = eval_length_mm_in_doc(&constraint.expression, doc) else {
            return Ok(());
        };
        if value <= 0.0 {
            return Ok(());
        }
        let value = value as f64;
        match target {
            DistanceTarget::LineLength(index) => {
                // Gauge weight, not a firm hold: this anchor only expresses "prefer growing
                // the line from its start" for otherwise-free geometry. A firm (1e4) hold
                // here pins every dimensioned line's start point at wherever it sat when the
                // solve began — dimension two or more chained lines of a polygon and those
                // pins contradict each other and the coincident corners, leaving the solver
                // stuck millimetres from an exact solution it can no longer reach.
                self.anchor_point(
                    doc,
                    ConstraintPoint::LineEndpoint {
                        line: index,
                        end: LineEnd::Start,
                    },
                    GAUGE_HOLD_WEIGHT,
                )?;
                let line = ConstraintLine::Line(index);
                let ((x0, y0), (x1, y1)) = self.line_vars(doc, line)?;
                self.system.add_equation(Equation::LineLength {
                    x0,
                    y0,
                    x1,
                    y1,
                    length: value,
                    weight: DEFAULT_WEIGHT,
                });
            }
            DistanceTarget::CircleDiameter(index) => {
                self.anchor_point(doc, ConstraintPoint::CircleCenter(index), REFERENCE_HOLD_WEIGHT)?;
                let radius = self
                    .circle_radius
                    .get(&index)
                    .copied()
                    .ok_or_else(|| format!("Circle {index} not in solver graph"))?;
                self.system.add_equation(Equation::CircleDiameter {
                    radius,
                    diameter: value,
                    weight: DEFAULT_WEIGHT,
                });
            }
            DistanceTarget::LineLineDistance {
                line_a,
                line_b,
                side,
            } => {
                let (reference, movable) = parallel_reference_and_movable(line_a, line_b);
                // A line-line *distance* is a dimensional constraint between two typically
                // separate parallel lines (not shared polygon corners), so its reference stays
                // firm like a point-line distance — unlike the direction/equality relational
                // constraints below, which share corners and must hold weakly (#137).
                self.hold_line(doc, reference.clone(), REFERENCE_HOLD_WEIGHT)?;
                let a = self.line_vars(doc, reference)?;
                let b = self.line_vars(doc, movable)?;
                self.system.add_equation(Equation::LineLineDistance {
                    ax0: a.0.0,
                    ay0: a.0.1,
                    ax1: a.1.0,
                    ay1: a.1.1,
                    bx0: b.0.0,
                    by0: b.0.1,
                    bx1: b.1.0,
                    by1: b.1.1,
                    distance: value,
                    side: side as f64,
                    weight: DEFAULT_WEIGHT,
                });
            }
            DistanceTarget::PointPointDistance {
                anchor,
                mover,
                dir_u: _,
                dir_v: _,
            } => {
                self.hold_point(doc, anchor.clone())?;
                let (ax, ay) = self.point_vars(doc, anchor)?;
                let (mx, my) = self.point_vars(doc, mover)?;
                self.system.add_equation(Equation::PointPointDistance {
                    mx,
                    my,
                    ax,
                    ay,
                    distance: value,
                    weight: DEFAULT_WEIGHT,
                });
            }
            DistanceTarget::PointLineDistance {
                point,
                line,
                side,
            } => {
                self.hold_line(doc, line.clone(), REFERENCE_HOLD_WEIGHT)?;
                let (px, py) = self.point_vars(doc, point)?;
                let ((x0, y0), (x1, y1)) = self.line_vars(doc, line)?;
                self.system.add_equation(Equation::PointLineDistance {
                    px,
                    py,
                    x0,
                    y0,
                    x1,
                    y1,
                    distance: value,
                    side: side as f64,
                    weight: DEFAULT_WEIGHT,
                });
            }
        }
        Ok(())
    }

    /// Hold a constraint's reference line during a full solve (no-op during a drag), so the
    /// dependent geometry moves to it rather than the reference moving.
    ///
    /// `weight` selects how firmly. A metric/dimensional constraint whose reference is a
    /// genuinely separate line the dependent geometry snaps onto (point-line distance,
    /// midpoint, point-on-line coincident, line-line *distance*) passes
    /// `REFERENCE_HOLD_WEIGHT`: the reference's endpoints aren't shared with another held line,
    /// so a firm pin is safe and keeps "the dependent side moves, the reference stays"
    /// predictable.
    ///
    /// A line-vs-line *direction/equality* relational constraint
    /// (`Parallel`/`Perpendicular`/`Equal`/`Angle`) passes `GAUGE_HOLD_WEIGHT` instead (#137).
    /// A firm per-constraint
    /// pin over-constrains any sketch where two *different* relational constraints hold two
    /// *different* lines that share a corner (e.g. `Perpendicular` holding one side of a quad
    /// while `Equal` holds an adjacent one): each pin clamps its own line's copy of the shared
    /// corner to a different place, and both beat the weak (`DEFAULT_WEIGHT`) `Coincident`
    /// equation linking the copies, so the corner tears open at a genuine converged optimum of
    /// an inconsistent weighted system. A weak gauge bias only breaks ties among
    /// otherwise-free degrees of freedom, so the real `Coincident` constraints win and keep
    /// every corner closed, while the reference still carries enough bias to stay put.
    fn hold_line(&mut self, doc: &Document, line: ConstraintLine, weight: f64) -> Result<(), String> {
        if !self.hold_references {
            return Ok(());
        }
        for endpoint in line_endpoint_points(doc, line.clone()) {
            let _ = self.anchor_point(doc, endpoint, weight);
        }
        Ok(())
    }

    fn anchor_point(&mut self, doc: &Document, point: ConstraintPoint, weight: f64) -> Result<(), String> {
        let (u, v) = self.point_vars(doc, point)?;
        self.hold_var(u, weight);
        self.hold_var(v, weight);
        Ok(())
    }

    /// Weight for cross/dot-product equations (`Parallel`/`Perpendicular`) that normalizes
    /// the raw residual by the product of the two line lengths, making it ~sin/cos of the
    /// angle error. Unnormalized, two 50 mm lines yield residuals in the thousands (mm²)
    /// that drown the mm-scale coincident/length equations in the least-squares objective —
    /// ill-scaling that leaves the LM solver in spurious local minima on real sketches.
    /// Lengths are read at system-build time; `.max(1.0)` keeps degenerate (sub-mm) lines
    /// from exploding the weight.
    fn direction_product_weight(
        &self,
        a: ((VarId, VarId), (VarId, VarId)),
        b: ((VarId, VarId), (VarId, VarId)),
    ) -> f64 {
        let val = |id: VarId| self.system.value(id);
        let len = |l: ((VarId, VarId), (VarId, VarId))| {
            (val(l.1.0) - val(l.0.0)).hypot(val(l.1.1) - val(l.0.1))
        };
        let denom = (len(a) * len(b)).max(1.0);
        DEFAULT_WEIGHT / (denom * denom)
    }

    /// Gauge-hold a reference point during a full solve (no-op during a drag). Uses the weak
    /// gauge weight so it stabilises free geometry without fighting real constraints.
    fn hold_point(&mut self, doc: &Document, point: ConstraintPoint) -> Result<(), String> {
        if !self.hold_references {
            return Ok(());
        }
        self.anchor_point(doc, point, GAUGE_HOLD_WEIGHT)
    }

    fn hold_var(&mut self, var: VarId, weight: f64) {
        self.system.add_equation(Equation::Pin {
            var,
            target: self.system.value(var),
            weight,
        });
    }

    fn add_coincident(
        &mut self,
        doc: &Document,
        a: ConstraintEntity,
        b: ConstraintEntity,
    ) -> Result<(), String> {
        match (a, b) {
            (ConstraintEntity::Point(pa), ConstraintEntity::Point(pb)) => {
                use crate::geometric_constraints::coincident_mover_and_anchor;
                let (_mover, anchor) = coincident_mover_and_anchor(pa.clone(), pb.clone());
                self.hold_point(doc, anchor)?;
                let (au, av) = self.point_vars(doc, pa)?;
                let (bu, bv) = self.point_vars(doc, pb)?;
                self.system.add_equation(Equation::CoincidentU {
                    a: au,
                    b: bu,
                    weight: DEFAULT_WEIGHT,
                });
                self.system.add_equation(Equation::CoincidentV {
                    a: av,
                    b: bv,
                    weight: DEFAULT_WEIGHT,
                });
            }
            (ConstraintEntity::Point(point), ConstraintEntity::Line(line))
            | (ConstraintEntity::Line(line), ConstraintEntity::Point(point)) => {
                self.hold_line(doc, line.clone(), REFERENCE_HOLD_WEIGHT)?;
                let (px, py) = self.point_vars(doc, point)?;
                let ((x0, y0), (x1, y1)) = self.line_vars(doc, line)?;
                self.system.add_equation(Equation::PointLineDistance {
                    px,
                    py,
                    x0,
                    y0,
                    x1,
                    y1,
                    distance: 0.0,
                    side: 1.0,
                    weight: DEFAULT_WEIGHT,
                });
            }
            (ConstraintEntity::Point(point), ConstraintEntity::Circle(circle))
            | (ConstraintEntity::Circle(circle), ConstraintEntity::Point(point)) => {
                let center = ConstraintPoint::CircleCenter(circle);
                self.hold_point(doc, center.clone())?;
                let (px, py) = self.point_vars(doc, point)?;
                let (cx, cy) = self.point_vars(doc, center)?;
                let radius = self
                    .circle_radius
                    .get(&circle)
                    .copied()
                    .ok_or_else(|| format!("Circle {circle} not in solver graph"))?;
                // The circle is the reference: hold its radius so the point moves to the
                // perimeter rather than the circle shrinking to meet the point.
                if self.hold_references {
                    self.hold_var(radius, GAUGE_HOLD_WEIGHT);
                }
                self.system.add_equation(Equation::PointOnCircle {
                    px,
                    py,
                    cx,
                    cy,
                    radius,
                    weight: DEFAULT_WEIGHT,
                });
            }
            (ConstraintEntity::Point(point), ConstraintEntity::Origin)
            | (ConstraintEntity::Origin, ConstraintEntity::Point(point)) => {
                // Pin the point to the sketch origin via a fixed (0, 0) helper point.
                let (px, py) = self.point_vars(doc, point)?;
                let (ox, oy) = self.system.add_point(0.0, 0.0, true);
                self.system.add_equation(Equation::CoincidentU {
                    a: px,
                    b: ox,
                    weight: DEFAULT_WEIGHT,
                });
                self.system.add_equation(Equation::CoincidentV {
                    a: py,
                    b: oy,
                    weight: DEFAULT_WEIGHT,
                });
            }
            (ConstraintEntity::Line(_), ConstraintEntity::Line(_))
            | (ConstraintEntity::Circle(_), ConstraintEntity::Circle(_))
            | (ConstraintEntity::Line(_), ConstraintEntity::Circle(_))
            | (ConstraintEntity::Circle(_), ConstraintEntity::Line(_))
            | (ConstraintEntity::Origin, _)
            | (_, ConstraintEntity::Origin) => {
                return Err("Unsupported coincident entity pair".to_string());
            }
        }
        Ok(())
    }

    /// Resolve a point's solver variables, lazily seeding a `FaceVertex` the first time it's
    /// referenced (#26/#27): unlike sketch-native points, a face's own vertex isn't discovered
    /// by `seed_entities` walking `doc.lines`/`doc.circles`, so it's seeded here on
    /// first use instead — as a **fixed** point (mirrors how `add_coincident`'s `Origin` arm
    /// above adds a fixed helper point), since it's not draggable/settable.
    fn point_vars(&mut self, doc: &Document, point: ConstraintPoint) -> Result<(VarId, VarId), String> {
        if let Some(vars) = self.point_vars.get(&point) {
            return Ok(*vars);
        }
        if let ConstraintPoint::FaceVertex { .. } = &point {
            let (u, v) = point_uv(doc, self.sketch, point.clone())?;
            let vars = self.system.add_point(u as f64, v as f64, true);
            self.point_vars.insert(point, vars);
            return Ok(vars);
        }
        Err(format!("Point {point:?} not in solver graph"))
    }

    fn line_vars(
        &mut self,
        doc: &Document,
        line: ConstraintLine,
    ) -> Result<((VarId, VarId), (VarId, VarId)), String> {
        match line {
            ConstraintLine::Line(index) => {
                let start = self.point_vars(
                    doc,
                    ConstraintPoint::LineEndpoint {
                        line: index,
                        end: LineEnd::Start,
                    },
                )?;
                let end = self.point_vars(
                    doc,
                    ConstraintPoint::LineEndpoint {
                        line: index,
                        end: LineEnd::End,
                    },
                )?;
                Ok((start, end))
            }
            // A face's own edge runs between two of its boundary loop's vertices (#26/#27);
            // each resolves (and lazily seeds, if new) through the same `FaceVertex` path above.
            ConstraintLine::FaceEdge { face, index } => {
                let boundary = crate::extrude::face_boundary_loop_world(doc, &face)
                    .ok_or_else(|| "Face boundary not available".to_string())?;
                let n = boundary.len();
                if n == 0 || index >= n {
                    return Err(format!("Face edge {index} out of range"));
                }
                let start = self.point_vars(
                    doc,
                    ConstraintPoint::FaceVertex {
                        face: face.clone(),
                        index,
                    },
                )?;
                let end = self.point_vars(
                    doc,
                    ConstraintPoint::FaceVertex {
                        face,
                        index: (index + 1) % n,
                    },
                )?;
                Ok((start, end))
            }
        }
    }
}

/// Constraint indices with the largest residuals when the sketch fails to solve.
pub fn sketch_conflicting_constraints(
    doc: &Document,
    sketch: SketchId,
) -> Result<Vec<usize>, String> {
    let mut bridge = SketchBridge::from_document(doc, sketch, true)?;
    let report = bridge.solve();
    if report.success {
        return Ok(Vec::new());
    }
    Ok(report.failed_constraints)
}

/// Remaining degrees of freedom for one sketch's constraint system.
pub fn sketch_dof_remaining(doc: &Document, sketch: SketchId) -> Result<i32, String> {
    let bridge = SketchBridge::from_document(doc, sketch, true)?;
    Ok(dof_remaining(&bridge.system))
}

/// Whether a sketch point can still move under the current constraints (reference geometry held).
pub fn sketch_point_movable(
    doc: &Document,
    sketch: SketchId,
    point: ConstraintPoint,
) -> Result<bool, String> {
    let mut bridge = SketchBridge::from_document(doc, sketch, true)?;
    match bridge.point_solver_vars(doc, point) {
        Ok((u, v)) => Ok(vars_can_move_together(&bridge.system, &[u, v])),
        Err(_) => Ok(false),
    }
}

/// Whether a sketch line's endpoints still have any freedom to move.
pub fn sketch_line_vertex_drag_blocked(
    doc: &Document,
    sketch: SketchId,
    line_index: usize,
) -> Result<bool, String> {
    use crate::constraints::find_distance_constraint;
    if find_distance_constraint(doc, DistanceTarget::LineLength(line_index)).is_none() {
        return Ok(false);
    }
    let mut bridge = SketchBridge::from_document(doc, sketch, true)?;
    let start = ConstraintPoint::LineEndpoint {
        line: line_index,
        end: LineEnd::Start,
    };
    let end = ConstraintPoint::LineEndpoint {
        line: line_index,
        end: LineEnd::End,
    };
    let mut line_vars = Vec::new();
    for point in [start, end] {
        if let Ok((u, v)) = bridge.point_solver_vars(doc, point) {
            line_vars.push(u);
            line_vars.push(v);
        }
    }
    Ok(!vars_can_move_together(&bridge.system, &line_vars))
}

/// The lines of one sketch the renderer should style as **fully constrained** (#172).
/// Deliberately mirrors [`sketch_line_vertex_drag_blocked`]'s semantics — a line styles
/// white exactly when the app would refuse to drag it (dimensioned, and its endpoints have
/// no joint freedom under the solve-time gauge holds) — but builds the solver bridge once
/// per sketch instead of once per line.
pub fn sketch_fully_constrained_lines(
    doc: &Document,
    sketch: SketchId,
) -> Result<std::collections::HashSet<usize>, String> {
    use crate::constraints::find_distance_constraint;
    let mut bridge = SketchBridge::from_document(doc, sketch, true)?;
    let mut out = std::collections::HashSet::new();
    for (li, line) in doc.lines.iter().enumerate() {
        if line.deleted || line.sketch != sketch {
            continue;
        }
        if find_distance_constraint(doc, DistanceTarget::LineLength(li)).is_none() {
            continue;
        }
        let mut line_vars = Vec::new();
        for end in [LineEnd::Start, LineEnd::End] {
            let point = ConstraintPoint::LineEndpoint { line: li, end };
            if let Ok((u, v)) = bridge.point_solver_vars(doc, point) {
                line_vars.push(u);
                line_vars.push(v);
            }
        }
        if line_vars.len() == 4 && !vars_can_move_together(&bridge.system, &line_vars) {
            out.insert(li);
        }
    }
    Ok(out)
}

/// All fully-constrained lines across every sketch (#172), memoized per document state —
/// the DOF analysis builds a solver system per sketch, far too heavy to run per line per
/// frame. Any change to sketch geometry or constraints invalidates the memo.
pub fn fully_constrained_lines(doc: &Document) -> std::collections::HashSet<usize> {
    use std::hash::Hasher;
    struct HashWriter(std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(std::collections::hash_map::DefaultHasher::new());
    serde_json::to_writer(
        &mut writer,
        &(&doc.lines, &doc.circles, &doc.constraints, &doc.sketches),
    )
    .ok();
    let fingerprint = writer.0.finish();

    thread_local! {
        static CONSTRAINED: std::cell::RefCell<(u64, std::collections::HashSet<usize>)> =
            std::cell::RefCell::new((0, std::collections::HashSet::new()));
    }
    CONSTRAINED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.0 != fingerprint {
            let mut all = std::collections::HashSet::new();
            let sketches: std::collections::HashSet<SketchId> = doc
                .lines
                .iter()
                .filter(|l| !l.deleted)
                .map(|l| l.sketch)
                .collect();
            for sketch in sketches {
                if let Ok(set) = sketch_fully_constrained_lines(doc, sketch) {
                    all.extend(set);
                }
            }
            *cache = (fingerprint, all);
        }
        cache.1.clone()
    })
}

/// Solve all sketches in `doc`, optionally pinning points during drag.
pub fn solve_document_sketches(
    doc: &mut Document,
    pins: &[(ConstraintPoint, (f32, f32))],
) -> Result<(), String> {
    let sketches = sketches_to_solve(doc, pins);
    for sketch in sketches {
        solve_one_sketch(doc, sketch, pins)?;
    }
    Ok(())
}

fn sketches_to_solve(doc: &Document, pins: &[(ConstraintPoint, (f32, f32))]) -> Vec<SketchId> {
    let mut sketches = HashSet::new();
    for constraint in &doc.constraints {
        if !constraint.deleted {
            sketches.insert(constraint.sketch);
        }
    }
    // Points being dragged are always sketch-native (a `FaceVertex` is fixed, never a drag
    // pin), so `point_sketch` alone — no need for a `point_uv` existence check too — decides
    // which sketch's solve this pin belongs to.
    for (point, _) in pins {
        if let Some(sketch) = point_sketch(doc, point.clone()) {
            sketches.insert(sketch);
        }
    }
    let mut ordered: Vec<SketchId> = sketches.into_iter().collect();
    ordered.sort_unstable();
    ordered
}

/// The two endpoint points that define a constraint line (line endpoints, rect-edge corners, or
/// — #26/#27 — the two boundary-loop vertices of a face's own edge).
fn line_endpoint_points(doc: &Document, line: ConstraintLine) -> Vec<ConstraintPoint> {
    match line {
        ConstraintLine::Line(index) => vec![
            ConstraintPoint::LineEndpoint {
                line: index,
                end: LineEnd::Start,
            },
            ConstraintPoint::LineEndpoint {
                line: index,
                end: LineEnd::End,
            },
        ],
        ConstraintLine::FaceEdge { face, index } => {
            let Some(boundary) = crate::extrude::face_boundary_loop_world(doc, &face) else {
                return Vec::new();
            };
            let n = boundary.len();
            if n == 0 || index >= n {
                return Vec::new();
            }
            vec![
                ConstraintPoint::FaceVertex {
                    face: face.clone(),
                    index,
                },
                ConstraintPoint::FaceVertex {
                    face,
                    index: (index + 1) % n,
                },
            ]
        }
    }
}

fn point_sketch(doc: &Document, point: ConstraintPoint) -> Option<SketchId> {
    match point {
        ConstraintPoint::LineEndpoint { line, .. } => doc.lines.get(line).map(|l| l.sketch),
        ConstraintPoint::CircleCenter(circle) => doc.circles.get(circle).map(|c| c.sketch),
        // A face's own vertex has no owning sketch — it's referenced *from* whichever sketch a
        // constraint projects it into, not owned by one (mirrors `construction::point_sketch`).
        ConstraintPoint::FaceVertex { .. } => None,
    }
}

fn solve_one_sketch(
    doc: &mut Document,
    sketch: SketchId,
    pins: &[(ConstraintPoint, (f32, f32))],
) -> Result<(), String> {
    let sketch_pins: Vec<_> = pins
        .iter()
        .filter(|(point, _)| point_sketch(doc, point.clone()) == Some(sketch))
        .cloned()
        .collect();
    let hold_references = sketch_pins.is_empty();
    let mut bridge = SketchBridge::from_document(doc, sketch, hold_references)?;
    bridge.add_drag_pins(doc, &sketch_pins);
    let _report = bridge.solve();
    bridge.apply_to_document(doc)?;
    crate::model::refit_fillet_arc_handles(doc, sketch);
    Ok(())
}

fn set_point_uv_from_solver(
    doc: &mut Document,
    sketch: SketchId,
    point: ConstraintPoint,
    u: f64,
    v: f64,
) -> Result<(), String> {
    // `sketch` is only meaningful for `FaceVertex` (fixed, so `set_point_uv` always errors on
    // it anyway) — every point this is actually called with (`LineEndpoint`/`CircleCenter`)
    // ignores it.
    crate::geometric_constraints::set_point_uv(doc, sketch, point, u as f32, v as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraints::add_distance_constraint;
    use crate::geometric_constraints::{
        add_geometric_constraint_from_selection, GeometricConstraintType,
    };
    use crate::hierarchy::SceneElement;
    use crate::model::{Constraint, ConstraintKind, Document, FaceId, Line};
    use crate::selection::{click_scene_selection, SceneSelection};

    const EPS: f32 = 1e-2;

    fn sketch_doc() -> (Document, SketchId) {
        let mut doc = Document::default();
        let sketch = doc.add_sketch(FaceId::ConstructionPlane(0));
        (doc, sketch)
    }

    fn solve_bridge(doc: &mut Document, _sketch: SketchId) {
        solve_document_sketches(doc, &[]).expect("solve");
    }

    /// #172: the fully-constrained set contains a line whose endpoints have no remaining
    /// freedom (anchored + oriented + dimensioned), and not a free line.
    #[test]
    fn fully_constrained_set_tracks_line_freedom() {
        use crate::model::{ConstraintEntity, ConstraintPoint, DistanceTarget, LineEnd};

        let (mut doc, sketch) = sketch_doc();
        doc.lines.push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines.push(Line::from_local_endpoints(sketch, 30.0, 5.0, 42.0, 9.0));
        let mut push = |kind: ConstraintKind| {
            doc.constraints.push(Constraint {
                sketch,
                kind,
                expression: String::new(),
                dim_offset: None,
                name: None,
                deleted: false,
            });
        };
        // Line 0: start pinned to the origin, horizontal, length locked → zero freedom.
        push(ConstraintKind::Coincident {
            a: ConstraintEntity::Origin,
            b: ConstraintEntity::Point(ConstraintPoint::LineEndpoint {
                line: 0,
                end: LineEnd::Start,
            }),
        });
        push(ConstraintKind::Horizontal {
            line: crate::model::ConstraintLine::Line(0),
        });
        doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Distance {
                target: DistanceTarget::LineLength(0),
            },
            expression: "10".to_string(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        solve_document_sketches(&mut doc, &[]).expect("solve");

        let set = sketch_fully_constrained_lines(&doc, sketch).expect("dof analysis");
        assert!(set.contains(&0), "anchored+oriented+dimensioned line is fully constrained");
        assert!(!set.contains(&1), "the free line still has freedom");

        // The memoized document-wide wrapper agrees, and refreshes when the dimension is
        // removed (an undimensioned line never styles as fully constrained).
        let all = fully_constrained_lines(&doc);
        assert!(all.contains(&0) && !all.contains(&1));
        let dist = doc
            .constraints
            .iter()
            .position(|c| matches!(c.kind, ConstraintKind::Distance { .. }))
            .unwrap();
        doc.constraints[dist].deleted = true;
        solve_document_sketches(&mut doc, &[]).expect("solve");
        let all = fully_constrained_lines(&doc);
        assert!(!all.contains(&0), "removing the dimension must drop the line from the set");
    }

    /// #137: chaining relational constraints across a closed quad (two `Equal` pairs plus a
    /// `Perpendicular` between two lines that each act as a *different* constraint's held
    /// reference) must not tear the shared corners open — every corner must stay closed after
    /// each incremental solve, matching how the real UI solves after every constraint add.
    #[test]
    fn chained_relational_constraints_keep_quad_corners_closed() {
        use crate::construction::add_line_polygon;
        let (mut doc, sketch) = sketch_doc();
        let idx = add_line_polygon(
            &mut doc,
            sketch,
            &[
                (62.863728, 70.923386),
                (40.238636, 93.450745),
                (67.94635, 115.119255),
                (102.57943, 102.74624),
            ],
        );
        let mut push = |kind: ConstraintKind| {
            doc.constraints.push(Constraint {
                sketch,
                kind,
                expression: String::new(),
                dim_offset: None,
                name: None,
                deleted: false,
            });
            doc.shape_order.push(crate::model::ShapeKind::Constraint);
            solve_document_sketches(&mut doc, &[]).unwrap();
        };
        push(ConstraintKind::Equal { line_a: ConstraintLine::Line(idx[3]), line_b: ConstraintLine::Line(idx[1]) });
        push(ConstraintKind::Equal { line_a: ConstraintLine::Line(idx[2]), line_b: ConstraintLine::Line(idx[0]) });
        push(ConstraintKind::Perpendicular { line_a: ConstraintLine::Line(idx[1]), line_b: ConstraintLine::Line(idx[2]) });

        let mut bridge = SketchBridge::from_document(&doc, sketch, true).unwrap();
        let _ = bridge.solve();

        for i in 0..4 {
            let a = &doc.lines[idx[i]];
            let b = &doc.lines[idx[(i + 1) % 4]];
            let gap = ((a.x1 - b.x0).powi(2) + (a.y1 - b.y0).powi(2)).sqrt();
            assert!(gap < 0.1, "corner {i} opened up by {gap} units");
        }
        let a = &doc.lines[idx[1]];
        let b = &doc.lines[idx[2]];
        let adu = a.x1 - a.x0;
        let adv = a.y1 - a.y0;
        let bdu = b.x1 - b.x0;
        let bdv = b.y1 - b.y0;
        let cos = (adu * bdu + adv * bdv) / (adu.hypot(adv) * bdu.hypot(bdv));
        assert!(cos.abs() < 0.05, "lines 1 and 2 should end up perpendicular, cos={cos}");
    }

    /// Dragging the movable line of a parallel pair must not drag the reference line.
    #[test]
    fn drag_parallel_movable_does_not_move_reference() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 100.0, 0.0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 40.0, 100.0, 40.0));
        doc.shape_order.push(crate::model::ShapeKind::Line);
        doc.shape_order.push(crate::model::ShapeKind::Line);
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Line(0), false);
        click_scene_selection(&mut sel, SceneElement::Line(1), true);
        add_geometric_constraint_from_selection(
            &mut doc,
            sketch,
            GeometricConstraintType::Parallel,
            &sel,
        )
        .unwrap();

        let pins = [(
            ConstraintPoint::LineEndpoint {
                line: 1,
                end: LineEnd::End,
            },
            (100.0_f32, 80.0_f32),
        )];
        solve_document_sketches(&mut doc, &pins).unwrap();

        let a = &doc.lines[0];
        assert!(
            a.x0.abs() < 0.5 && a.y0.abs() < 0.5 && (a.x1 - 100.0).abs() < 0.5 && a.y1.abs() < 0.5,
            "reference line A drifted to ({},{})-({},{})",
            a.x0,
            a.y0,
            a.x1,
            a.y1
        );
    }

    #[test]
    fn bridge_conflicting_constraints_reports_largest_residuals() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Distance {
                target: DistanceTarget::LineLength(0),
            },
            expression: "10mm".to_string(),
            dim_offset: None,
            name: None,
            deleted: false,
        });
        doc.constraints.push(Constraint {
            sketch,
            kind: ConstraintKind::Distance {
                target: DistanceTarget::LineLength(0),
            },
            expression: "12mm".to_string(),
            dim_offset: None,
            name: None,
            deleted: false,
        });

        let mut bridge = SketchBridge::from_document(&doc, sketch, true).unwrap();
        let report = bridge.solve();
        assert!(!report.success);
        assert_eq!(report.failed_constraints.len(), 2);
        assert!(report.failed_constraints.contains(&0));
        assert!(report.failed_constraints.contains(&1));
    }

    #[test]
    #[ignore = "run with `cargo test --release solve_perf -- --ignored`"]
    fn solve_perf_100_constraints_under_5ms() {
        use std::time::Instant;

        let (mut doc, sketch) = sketch_doc();
        for i in 0..50 {
            let y = i as f32 * 5.0;
            doc.lines.push(Line::from_local_endpoints(
                sketch,
                0.0,
                y,
                100.0,
                y + 3.0,
            ));
        }
        for index in 0..doc.lines.len() {
            add_distance_constraint(
                &mut doc,
                sketch,
                DistanceTarget::LineLength(index),
                "100mm".to_string(),
            )
            .unwrap();
        }
        let start = Instant::now();
        solve_document_sketches(&mut doc, &[]).unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 5,
            "solve took {} ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn bridge_sketch_dof_remaining_reports_underconstrained_line() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        assert!(sketch_dof_remaining(&doc, sketch).unwrap() > 0);
    }

    #[test]
    fn bridge_round_trip_line() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        add_distance_constraint(
            &mut doc,
            sketch,
            DistanceTarget::LineLength(0),
            "10mm".to_string(),
        )
        .unwrap();
        doc.lines[0].x1 = 7.0;
        solve_bridge(&mut doc, sketch);
        assert!((doc.lines[0].length() - 10.0).abs() < EPS);
    }

    #[test]
    fn bridge_equal_makes_two_lines_equal_length() {
        let (mut doc, sketch) = sketch_doc();
        // A horizontal line of length 10 and a horizontal line of length 4.
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 5.0, 4.0, 5.0));
        doc.shape_order.push(crate::model::ShapeKind::Line);
        doc.shape_order.push(crate::model::ShapeKind::Line);
        let mut sel = SceneSelection::default();
        click_scene_selection(&mut sel, SceneElement::Line(0), false);
        click_scene_selection(&mut sel, SceneElement::Line(1), true);
        add_geometric_constraint_from_selection(
            &mut doc,
            sketch,
            GeometricConstraintType::Equal,
            &sel,
        )
        .unwrap();
        solve_bridge(&mut doc, sketch);
        assert!(
            (doc.lines[0].length() - doc.lines[1].length()).abs() < EPS,
            "lengths: {} vs {}",
            doc.lines[0].length(),
            doc.lines[1].length()
        );
    }

    #[test]
    fn bridge_point_line_distance() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 5.0, -4.0, 6.0, -4.0));
        doc.shape_order.push(crate::model::ShapeKind::Line);
        doc.shape_order.push(crate::model::ShapeKind::Line);
        let point = ConstraintPoint::LineEndpoint {
            line: 1,
            end: LineEnd::Start,
        };
        add_distance_constraint(
            &mut doc,
            sketch,
            DistanceTarget::PointLineDistance {
                point: point.clone(),
                line: ConstraintLine::Line(0),
                side: 1,
            },
            "3mm".to_string(),
        )
        .unwrap();
        solve_bridge(&mut doc, sketch);
        let (_pu, pv) = point_uv(&doc, sketch, point).unwrap();
        assert!((pv + 3.0).abs() < 0.2, "pv={pv}");
    }

    #[test]
    fn bridge_midpoint() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 4.0, 8.0, 5.0, 9.0));
        let mut sel = SceneSelection::default();
        click_scene_selection(
            &mut sel,
            SceneElement::Point(ConstraintPoint::LineEndpoint {
                line: 1,
                end: LineEnd::Start,
            }),
            false,
        );
        click_scene_selection(&mut sel, SceneElement::Line(0), true);
        add_geometric_constraint_from_selection(
            &mut doc,
            sketch,
            GeometricConstraintType::Midpoint,
            &sel,
        )
        .unwrap();
        solve_bridge(&mut doc, sketch);
        let (pu, pv) = point_uv(
            &doc,
            sketch,
            ConstraintPoint::LineEndpoint {
                line: 1,
                end: LineEnd::Start,
            },
        )
        .unwrap();
        assert!((pu - 5.0).abs() < EPS);
        assert!(pv.abs() < EPS);
    }

    #[test]
    fn bridge_coincident_point_on_line() {
        let (mut doc, sketch) = sketch_doc();
        doc.lines
            .push(Line::from_local_endpoints(sketch, 0.0, 0.0, 10.0, 0.0));
        doc.lines
            .push(Line::from_local_endpoints(sketch, 5.0, 8.0, 6.0, 9.0));
        let mut sel = SceneSelection::default();
        click_scene_selection(
            &mut sel,
            SceneElement::Point(ConstraintPoint::LineEndpoint {
                line: 1,
                end: LineEnd::Start,
            }),
            false,
        );
        click_scene_selection(&mut sel, SceneElement::Line(0), true);
        add_geometric_constraint_from_selection(
            &mut doc,
            sketch,
            GeometricConstraintType::Coincident,
            &sel,
        )
        .unwrap();
        solve_bridge(&mut doc, sketch);
        let (pu, pv) = point_uv(
            &doc,
            sketch,
            ConstraintPoint::LineEndpoint {
                line: 1,
                end: LineEnd::Start,
            },
        )
        .unwrap();
        assert!((pu - 5.0).abs() < EPS);
        assert!(pv.abs() < EPS);
    }

    /// A sloppily drawn closed hexagon "squared up" afterwards by geometric constraints
    /// plus several length dims and an angle dim must solve exactly. This regressed two
    /// ways historically: LineLength dims firmly pinned each dimensioned line's start
    /// point (mutually contradictory once several chained lines carry dims), and the
    /// unnormalized parallel/perpendicular residuals (mm^2-scale) drowned the mm-scale
    /// corner-closing equations, leaving LM in a local minimum ~1 mm off.
    #[test]
    fn sloppy_bracket_squares_up_exactly() {
        use crate::model::{ConstraintEntity, ConstraintLine, ConstraintPoint, ConstraintSign, DistanceTarget, LineEnd};

        let (mut doc, sketch) = sketch_doc();
        // Roughly a 120-degree bracket profile, every segment a little off.
        let pts = [
            (0.0f32, 0.0f32),
            (51.0, 2.5),
            (49.5, 7.8),
            (4.5, 5.5),
            (-17.5, 47.0),
            (-25.5, 43.0),
        ];
        for i in 0..6 {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % 6];
            doc.lines.push(Line::from_local_endpoints(sketch, x0, y0, x1, y1));
        }
        let push = |doc: &mut Document, kind: ConstraintKind, expression: &str| {
            doc.constraints.push(Constraint {
                sketch,
                kind,
                expression: expression.to_string(),
                dim_offset: None,
                name: None,
                deleted: false,
            });
        };
        let end = |line: usize| {
            ConstraintEntity::Point(ConstraintPoint::LineEndpoint { line, end: LineEnd::End })
        };
        let start = |line: usize| {
            ConstraintEntity::Point(ConstraintPoint::LineEndpoint { line, end: LineEnd::Start })
        };
        for i in 0..6 {
            push(&mut doc, ConstraintKind::Coincident { a: end(i), b: start((i + 1) % 6) }, "");
        }
        let line = ConstraintLine::Line;
        push(&mut doc, ConstraintKind::Horizontal { line: line(0) }, "");
        push(&mut doc, ConstraintKind::Parallel { line_a: line(0), line_b: line(2) }, "");
        push(&mut doc, ConstraintKind::Parallel { line_a: line(3), line_b: line(5) }, "");
        push(&mut doc, ConstraintKind::Perpendicular { line_a: line(0), line_b: line(1) }, "");
        push(&mut doc, ConstraintKind::Perpendicular { line_a: line(5), line_b: line(4) }, "");
        push(&mut doc, ConstraintKind::Angle {
            line_a: line(0),
            line_b: line(3),
            rotation_sign: 1 as ConstraintSign,
        }, "120");
        for (index, len) in [(0usize, "50"), (5, "50"), (1, "5"), (4, "5")] {
            push(&mut doc, ConstraintKind::Distance { target: DistanceTarget::LineLength(index) }, len);
        }

        solve_bridge(&mut doc, sketch);

        // Corners closed.
        for i in 0..6 {
            let l = &doc.lines[i];
            let n = &doc.lines[(i + 1) % 6];
            assert!(
                (l.x1 - n.x0).abs() < EPS && (l.y1 - n.y0).abs() < EPS,
                "corner {i} open: ({}, {}) vs ({}, {})", l.x1, l.y1, n.x0, n.y0,
            );
        }
        // Dimensioned lengths exact.
        let len = |i: usize| {
            let l = &doc.lines[i];
            (l.x1 - l.x0).hypot(l.y1 - l.y0)
        };
        assert!((len(0) - 50.0).abs() < EPS, "L0 length {}", len(0));
        assert!((len(5) - 50.0).abs() < EPS, "L5 length {}", len(5));
        assert!((len(1) - 5.0).abs() < EPS, "L1 length {}", len(1));
        assert!((len(4) - 5.0).abs() < EPS, "L4 length {}", len(4));
        // The bend: line 3 at 120 degrees from the horizontal base.
        let l3 = &doc.lines[3];
        let angle = (l3.y1 - l3.y0).atan2(l3.x1 - l3.x0).to_degrees();
        assert!((angle - 120.0).abs() < 0.05, "bend angle {angle}");
    }
}
