//! Sketch constraint solving.
//!
//! The numeric solve is SolveSpace's libslvs (`slvs.rs`) on every target. The native
//! equation system in `system.rs`/`residuals.rs` remains for *analysis only*: DOF/rank
//! (`dof.rs`), drag-movability, and fully-constrained styling.

mod bridge;
mod dof;
pub(crate) mod slvs;
mod residuals;
mod system;

pub use bridge::{
    fully_constrained_lines, sketch_conflicting_constraints, sketch_dof_remaining,
    sketch_fully_constrained_lines, sketch_line_vertex_drag_blocked, sketch_point_movable,
    solve_document_sketches,
};
