# Plan: Rust Sketch Constraint Solver (Option C)

Build a native MIT/Apache-2.0 geometric constraint solver in Rust to replace the
procedural `apply_*` constraint code. The solver minimizes constraint residuals
with Levenberg–Marquardt (fully/over-constrained) and least-squares (under-constrained),
matching the approach described in [SolveSpace's technology page](https://solvespace.com/tech.pl).

**Status:** In progress  
**Decision:** Option C — build, not adopt SolveSpace or PlaneGCS  
**Spec reference:** SPEC.md §6.3 (was TBD)

---

## 1. Goals

| Goal | Success criterion |
|------|-------------------|
| Simultaneous solve | Compound constraints (e.g. ⊥ + point-line distance + drag) never fight each other |
| Replace procedural solver | `solve_document_constraints_with_pins` delegates to `sketch_solver` |
| Full test coverage | Every constraint kind has unit + integration tests; existing ~50 constraint/drag tests keep passing |
| Scriptable | Same solve path for GUI, `cargo test`, and Lua (`le3.solve()` or implicit on constraint add) |
| Deterministic | Same document + parameters → same geometry (fixed LM damping, fixed iteration order) |
| DOF reporting | `SolveResult::dof_remaining` and `conflicting` for UI (Phase 4) |

**Out of scope for v1 of this plan:** 3D constraints, arcs/splines as solver entities, incremental Jacobian updates.

---

## 2. Architecture

```
Document (ConstraintKind, Line, Rect, Circle)
        │
        ▼
 sketch_solver::bridge::sync_sketch()     ← build solver graph from one sketch
        │
        ▼
 sketch_solver::System                    ← variables + residual list + weights
        │
        ▼
 sketch_solver::newton::solve_lm()        ← LM / least-squares iterate
        │
        ▼
 sketch_solver::bridge::apply_solution()  ← write UV back to Document
```

### 2.1 Degrees of freedom

| Entity | Solver variables | Notes |
|--------|------------------|-------|
| Point | `u`, `v` | Line endpoints, rect corners, circle centres share point IDs |
| Circle | `radius` | Centre is a point; diameter constraints fix radius |
| Line | — | Derived from two endpoint points |
| Rect | — | Four corner points derived from `(x,y,w,h)` in bridge; edges are line pairs |

Rectangles decompose to **four corner points** in the solver. `ConstraintLine::RectEdge` maps to
the appropriate corner pair. Width/height constraints become point-to-point or edge-length residuals.

### 2.2 Residual formulation

Each constraint contributes one or more scalar residuals that should be zero. Examples:

| Constraint | Residual(s) |
|------------|-------------|
| Coincident (pt–pt) | `pu − qu`, `pv − qv` |
| Coincident (pt–line) | signed distance from point to infinite line |
| Horizontal | `y_end − y_start` |
| Vertical | `x_end − x_start` |
| Parallel | `dx_a·dy_b − dy_a·dx_b` (cross product) |
| Perpendicular | `dx_a·dx_b + dy_a·dy_b` (dot product) |
| Line length | `‖p₁−p₀‖ − L` |
| Point–point distance | `‖mover−anchor‖ − L` (anchor fixed via weight or removed DOF) |
| Point–line distance | signed perpendicular offset − `side·L` |
| Line–line distance | signed offset of movable line midpoint from reference − `side·L` |
| Angle | `atan2(movable) − atan2(ref) − sign·θ` (wrapped) |
| Midpoint | `pu − (x₀+x₁)/2`, `pv − (y₀+y₁)/2` |
| Circle diameter | `2r − D` |
| Rect width/height | distance between opposite corners − value |

Residuals are **weighted** (`weight: f64`) so drag-pins can use very large weight (equivalent to
fixing variables) without a separate code path.

### 2.3 Solve modes

```rust
pub enum SolveMode {
    /// Normal constraint satisfaction.
    Normal,
    /// Interactive drag: high-weight residuals pin dragged point(s).
    Drag { pinned: Vec<(ConstraintPoint, (f32, f32))> },
}
```

- **Fully/over-constrained:** LM with adaptive λ until `‖r‖∞ < ε` or max iterations.
- **Under-constrained:** same LM but accept best-effort minimum `‖r‖²` (penalty metric); dragged
  DOFs get extra weight so the solver changes other geometry first (SolveSpace-style).

### 2.4 Public API

```rust
pub struct SolveResult {
    pub success: bool,
    pub iterations: u32,
    pub residual_norm: f64,
    pub dof_remaining: i32,
    pub failed_constraints: Vec<ConstraintId>,
}

pub fn solve_sketch(doc: &mut Document, sketch: SketchId) -> Result<SolveResult, String>;
pub fn solve_sketch_with_drag(
    doc: &mut Document,
    sketch: SketchId,
    pins: &[(ConstraintPoint, (f32, f32))],
) -> Result<SolveResult, String>;
```

`solve_document_constraints` / `solve_document_constraints_with_pins` become thin wrappers.

---

## 3. Module layout

```
src/sketch_solver/
  mod.rs          Public API, SolveResult, re-exports
  system.rs       Variable registry, System { vars, residuals, fixed_mask }
  residuals.rs    Residual trait, per-constraint eval + analytic Jacobian rows
  entities.rs     SolverEntityId, point/line/circle handles
  newton.rs       Dense LM solver (nalgebra-free: hand-rolled small matrices)
  bridge.rs       Document ↔ System sync, expression evaluation hooks
  dof.rs          Rank analysis on Jacobian (Phase 4)
```

Add `mod sketch_solver;` in `main.rs`. No new Cargo dependencies for v1 (dense `Vec<f64>` math).

---

## 4. Implementation phases

### Phase 1 — Core math kernel (week 1)

**Deliverables**
- [ ] `System`: allocate variables, set/get values, mark fixed
- [ ] `Residual` trait: `fn evaluate(&self, vars: &[f64]) -> f64` and `fn jacobian_row(&self, vars, row: &mut [(VarId, f64)])`
- [ ] Residuals: horizontal, vertical, coincident (pt–pt), line length, parallel, perpendicular
- [ ] `solve_lm(system, config) -> SolveReport`
- [ ] Pin/fixed variables excluded from Jacobian columns

**Tests** (`sketch_solver` unit tests)
| Test | Asserts |
|------|---------|
| `horizontal_line_solves` | Endpoints level after solve |
| `vertical_line_solves` | Endpoints aligned in u |
| `coincident_points_merge` | Two points same UV |
| `line_length_enforced` | Distance = L ± ε |
| `parallel_lines_align` | Cross product ≈ 0 |
| `perpendicular_lines_align` | Dot product ≈ 0 |
| `compound_perpendicular_and_distance` | User's bug scenario in isolation (no Document) |
| `lm_converges_from_poor_initial` | Noisy start still converges |
| `fixed_variable_honored` | Pinned u/v unchanged |

---

### Phase 2 — Document bridge (week 2)

**Deliverables**
- [ ] `bridge::build_system(doc, sketch) -> System`
- [ ] Map `ConstraintPoint` / `ConstraintLine` → solver entity IDs (stable per solve)
- [ ] Rect corners: read/write through `Rect::{x,y,w,h}` after solve
- [ ] Circle: centre point + radius variable
- [ ] Evaluate `expression` via existing `eval_length_mm_in_doc` / `eval_angle_rad_in_doc`
- [ ] `bridge::apply_solution(doc, sketch, &system)`

**Tests** (integration, new `sketch_solver/bridge_tests.rs` or in `bridge.rs`)
| Test | Asserts |
|------|---------|
| `bridge_round_trip_line` | Build → apply → same geometry |
| `bridge_rect_edge_parallel` | Rect top ∥ line |
| `bridge_point_line_distance` | 50 mm offset |
| `bridge_angle_constraint` | Angle dim satisfied |
| `bridge_midpoint` | Point at line centre |
| `bridge_coincident_point_on_line` | Point projects onto line |
| `replaces_procedural_parallel` | Same result as old solver on fixture sketches |

---

### Phase 3 — Wire into application (week 3)

**Deliverables**
- [ ] Replace body of `solve_document_constraints_with_pins` with `sketch_solver` call
- [ ] `vertex_drag::drag_point` uses `SolveMode::Drag` (remove pin-restore heuristics)
- [ ] `parameters.rs` / `storage.rs` unchanged call sites
- [ ] Lua: ensure constraint add + drag still scriptable (no new API required if solve is implicit)
- [ ] Delete procedural code: `apply_parallel`, `apply_perpendicular`, `orient_line_with_pins`,
      `apply_geometric_constraints_with_pins` multi-pass loop, `distance_target_moves_pinned_point`
- [ ] Keep `geometric_constraints.rs` for **UI eligibility** and `add_geometric_constraint_from_selection`

**Tests** (existing suites must pass)
| Suite | Count |
|-------|-------|
| `constraints.rs` tests | ~20 |
| `geometric_constraints.rs` tests | ~17 |
| `vertex_drag.rs` tests | ~15 |
| New regression | `drag_vertex_preserves_perpendicular_with_rect_point_line_distance` |

**Gate:** `cargo test` all green; `cargo run --exit` launches.

---

### Phase 4 — DOF & diagnostics (week 4)

**Deliverables**
- [ ] `dof.rs`: SVD or QR rank of Jacobian → `dof_remaining`
- [ ] Failed solve → `failed_constraints` via identifying largest residual contributors
- [ ] `can_drag_point` / `can_drag_line` use DOF count instead of ad hoc `line_vertex_drag_blocked`
- [ ] UI hook (optional): expose DOF in Elements pane or status bar

**Tests**
| Test | Asserts |
|------|---------|
| `underconstrained_square` | dof > 0 |
| `fully_constrained_triangle` | dof = 0 |
| `overconstrained_detected` | solve fails or reports conflict |
| `drag_blocked_when_fully_constrained` | `can_drag_point` false |

---

### Phase 5 — Hardening & spec update (week 5+)

**Deliverables**
- [ ] Determinism test: solve same sketch 100× → bit-identical `f32` UVs
- [ ] Performance budget: 100-constraint sketch < 5 ms (release)
- [ ] Update SPEC.md §6.3: solver = native Rust LM
- [ ] Property tests for residual signs (`side` disambiguation preserved)

---

## 5. Migration strategy

1. **Parallel run (Phase 2–3 only):** `#[cfg(test)]` compare procedural vs new solver on fixtures; remove flag once parity proven.
2. **Hard cutover (Phase 3):** Delete procedural apply functions in one PR after tests pass.
3. **No schema migration:** `Constraint` / `ConstraintKind` unchanged; only solve backend changes.

---

## 6. Numerical defaults

```rust
pub struct SolverConfig {
    pub max_iterations: u32,       // 50
    pub tolerance: f64,            // 1e-8
    pub lm_lambda_init: f64,       // 1e-3
    pub lm_lambda_up: f64,         // 10.0
    pub lm_lambda_down: f64,       // 0.3
    pub drag_pin_weight: f64,      // 1e6 — soft pin during drag
}
```

Use `f64` internally; round-trip to `f32` document coords at bridge boundary.

---

## 7. Risk register

| Risk | Mitigation |
|------|------------|
| LM fails to converge on under-constrained drags | High drag weight; fall back to best-effort minimum ‖r‖² |
| Rect corner sharing inconsistent | Single canonical corner order in bridge (match `face::rect_world_corners_in_frame`) |
| Angle constraint periodicity | Wrap residual to (−π, π]; use captured `rotation_sign` |
| Performance on large sketches | Dense Jacobian OK to ~200 DOFs; profile before optimizing |
| Regression during cutover | Keep full test suite; Phase 2 parity tests |

---

## 8. Definition of done

- [ ] All phases 1–4 complete
- [ ] `cargo test` ≥ 564 tests passing (count may grow)
- [ ] Procedural solver code removed
- [ ] User's perpendicular + point-line distance + drag scenario passes
- [ ] SPEC.md §6.3 updated
- [ ] `plan.md` status → Done

---

## 9. Work log

| Date | Phase | Notes |
|------|-------|-------|
| 2026-06-19 | Plan | Option C approved; plan written |
| 2026-06-19 | 1 | Started: `sketch_solver` module, System, LM kernel |
| 2026-06-19 | 1 | Phase 1 core: 9 unit tests passing (horizontal, vertical, coincident, length, parallel, ⊥, compound+drag, fixed vars, poor initial) |