# Plan: Rust Sketch Constraint Solver (Option C)

Build a native MIT/Apache-2.0 geometric constraint solver in Rust to replace the
procedural `apply_*` constraint code. The solver minimizes constraint residuals
with Levenberg–Marquardt (fully/over-constrained) and least-squares (under-constrained),
matching the approach described in [SolveSpace's technology page](https://solvespace.com/tech.pl).

**Status:** Done  
**Decision:** Option C — build, not adopt SolveSpace or PlaneGCS  
**Spec reference:** SPEC.md §6.3

---

## 1. Goals

| Goal | Success criterion |
|------|-------------------|
| Simultaneous solve | Compound constraints (e.g. ⊥ + point-line distance + drag) never fight each other |
| Replace procedural solver | `solve_document_constraints_with_pins` delegates to `sketch_solver` |
| Full test coverage | Every constraint kind has unit + integration tests; existing ~50 constraint/drag tests keep passing |
| Scriptable | Same solve path for GUI, `cargo test`, and Lua (`le3.solve()` or implicit on constraint add) |
| Deterministic | Same document + parameters → same geometry (fixed LM damping, fixed iteration order) |
| DOF reporting | `SolveReport::dof_remaining` and `failed_constraints` for UI (Phase 4) |

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
the appropriate corner pair. Width/height constraints become edge-length + horizontal/vertical residuals.

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
| Rect width/height | anchor corner + horizontal/vertical edges + edge length |
| Pin (drag/hold) | `var − target` (high weight) |

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
pub struct SolveReport {
    pub success: bool,
    pub iterations: u32,
    pub residual_norm: f64,
    pub dof_remaining: i32,
}

pub fn solve_document_sketches(doc: &mut Document, pins: &[(ConstraintPoint, (f32, f32))]) -> Result<(), String>;
pub fn sketch_dof_remaining(doc: &Document, sketch: SketchId) -> Result<i32, String>;
pub fn sketch_degrees_of_freedom(doc: &Document, sketch: SketchId) -> Result<i32, String>;
```

`solve_document_constraints` / `solve_document_constraints_with_pins` are thin wrappers.

---

## 3. Module layout

```
src/sketch_solver/
  mod.rs          Public API, unit tests
  system.rs       Variable registry, System { vars, residuals, fixed_mask }
  residuals.rs    Equation enum, per-constraint eval + analytic Jacobian rows
  newton.rs       Dense LM solver (hand-rolled small matrices)
  bridge.rs       Document ↔ System sync, expression evaluation hooks
  dof.rs          Jacobian rank → dof_remaining, drag eligibility
```

Add `mod sketch_solver;` in `main.rs`. No new Cargo dependencies for v1 (dense `Vec<f64>` math).

---

## 4. Implementation phases

### Phase 1 — Core math kernel ✅

- `System`, `Equation` residuals, `solve_lm`, 9 unit tests

### Phase 2 — Document bridge ✅

- `SketchBridge`, `solve_document_sketches`, 8 integration tests

### Phase 3 — Wire into application ✅

- `solve_document_constraints_with_pins` → `sketch_solver`
- Procedural `apply_*` solver removed from `geometric_constraints.rs`
- `cargo test` 585 passing; `cargo run -- --exit` launches

### Phase 4 — DOF & diagnostics ✅

- `dof.rs`: Jacobian rank → `dof_remaining`
- `sketch_degrees_of_freedom()` scriptable API
- `sketch_line_vertex_drag_blocked()` replaces ad hoc `line_vertex_drag_blocked`
- `SolveReport::dof_remaining` and `failed_constraints` populated after solve
- `sketch_conflicting_constraints()` / `le3.sketch_conflicts()` scriptable API

### Phase 5 — Hardening & spec update ✅

- Determinism test: `solve_is_deterministic_for_same_sketch` (100× bit-identical)
- Performance test: `solve_perf_100_constraints_under_5ms` (release, `#[ignore]`)
- SPEC.md §6.3 updated

---

## 5. Migration strategy

1. **Hard cutover (Phase 3):** Procedural apply functions deleted; tests prove parity.
2. **No schema migration:** `Constraint` / `ConstraintKind` unchanged; only solve backend changes.

---

## 6. Numerical defaults

```rust
pub struct SolverConfig {
    pub max_iterations: u32,       // 100
    pub tolerance: f64,            // 1e-6
    pub lm_lambda_init: f64,       // 1e-3
    pub lm_lambda_up: f64,         // 10.0
    pub lm_lambda_down: f64,       // 0.3
    pub drag_pin_weight: f64,      // 1e6 — soft pin during drag
}
```

Use `f64` internally; round-trip to `f32` document coords at bridge boundary.

---

## 7. Definition of done

- [x] All phases 1–5 complete
- [x] `cargo test` 585 tests passing
- [x] Procedural solver code removed
- [x] Perpendicular + point-line distance + drag scenario passes
- [x] SPEC.md §6.3 updated
- [x] `plan.md` status → Done

---

## 8. Work log

| Date | Phase | Notes |
|------|-------|-------|
| 2026-06-19 | Plan | Option C approved; plan written |
| 2026-06-19 | 1 | Core LM kernel: 9 unit tests |
| 2026-06-19 | 2 | Document bridge: 8 integration tests |
| 2026-06-19 | 3 | Wired into app; procedural solver removed; rect width+height via edge constraints |
| 2026-06-19 | 4 | `dof.rs`, `sketch_degrees_of_freedom`, DOF-based drag blocking |
| 2026-06-19 | 5 | Determinism test, perf test, SPEC §6.3, plan done |