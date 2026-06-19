//! Degrees-of-freedom analysis from the constraint Jacobian.

use super::system::{System, VarId};

const RANK_EPS: f64 = 1e-8;

/// Build the weighted Jacobian for free variables: `m` equations × `n` free columns.
pub fn build_jacobian(system: &System) -> (Vec<f64>, usize, usize, Vec<VarId>) {
    let free = system.free_vars();
    let n_eq = system.equations.len();
    let n_free = free.len();
    if n_eq == 0 || n_free == 0 {
        return (Vec::new(), n_eq, n_free, free);
    }

    let free_index: std::collections::HashMap<usize, usize> = free
        .iter()
        .enumerate()
        .map(|(i, var)| (var.0, i))
        .collect();

    let mut jacobian = vec![0.0f64; n_eq * n_free];
    let mut row_buf: Vec<(VarId, f64)> = Vec::new();

    for (row, equation) in system.equations.iter().enumerate() {
        equation.jacobian_row(system, &mut row_buf);
        for (var, deriv) in &row_buf {
            if let Some(&col) = free_index.get(&var.0) {
                jacobian[row * n_free + col] = *deriv;
            }
        }
    }

    (jacobian, n_eq, n_free, free)
}

/// Rank of a dense matrix stored row-major (`rows` × `cols`).
pub fn matrix_rank(matrix: &[f64], rows: usize, cols: usize) -> usize {
    if rows == 0 || cols == 0 {
        return 0;
    }
    let mut a = matrix.to_vec();
    let mut rank = 0usize;
    let mut pivot_row = 0usize;

    for col in 0..cols {
        let mut pivot = None;
        for row in pivot_row..rows {
            if a[row * cols + col].abs() > RANK_EPS {
                pivot = Some(row);
                break;
            }
        }
        let Some(pivot) = pivot else {
            continue;
        };
        if pivot != pivot_row {
            for c in 0..cols {
                a.swap(pivot * cols + c, pivot_row * cols + c);
            }
        }
        let pivot_val = a[pivot_row * cols + col];
        for row in (pivot_row + 1)..rows {
            let factor = a[row * cols + col] / pivot_val;
            if factor.abs() <= RANK_EPS {
                continue;
            }
            for c in col..cols {
                a[row * cols + c] -= factor * a[pivot_row * cols + c];
            }
            a[row * cols + col] = 0.0;
        }
        rank += 1;
        pivot_row += 1;
        if pivot_row == rows {
            break;
        }
    }
    rank
}

/// Remaining degrees of freedom: `n_free_vars - rank(J)`.
pub fn dof_remaining(system: &System) -> i32 {
    let (jacobian, rows, cols, _) = build_jacobian(system);
    if cols == 0 {
        return 0;
    }
    let rank = matrix_rank(&jacobian, rows, cols);
    (cols as i32) - (rank as i32)
}

/// Whether any variable in `vars` participates in the Jacobian null space.
pub fn vars_can_move_together(system: &System, vars: &[VarId]) -> bool {
    let movable: Vec<VarId> = vars
        .iter()
        .copied()
        .filter(|var| !system.fixed[var.0])
        .collect();
    if movable.is_empty() {
        return false;
    }
    let (jacobian, rows, cols, free) = build_jacobian(system);
    if cols == 0 || dof_remaining(system) <= 0 {
        return false;
    }
    let target_cols: Vec<usize> = movable
        .iter()
        .filter_map(|var| free.iter().position(|v| *v == *var))
        .collect();
    if target_cols.is_empty() {
        return false;
    }
    null_space_touches_columns(&jacobian, rows, cols, &target_cols)
}

fn null_space_touches_columns(
    matrix: &[f64],
    rows: usize,
    cols: usize,
    target_cols: &[usize],
) -> bool {
    let mut a = matrix.to_vec();
    let mut pivot_cols = vec![false; cols];
    let mut pivot_row = 0usize;

    for col in 0..cols {
        let mut pivot = None;
        for row in pivot_row..rows {
            if a[row * cols + col].abs() > RANK_EPS {
                pivot = Some(row);
                break;
            }
        }
        let Some(pivot) = pivot else {
            continue;
        };
        pivot_cols[col] = true;
        if pivot != pivot_row {
            for c in 0..cols {
                a.swap(pivot * cols + c, pivot_row * cols + c);
            }
        }
        let pivot_val = a[pivot_row * cols + col];
        for row in 0..rows {
            if row == pivot_row {
                continue;
            }
            let factor = a[row * cols + col] / pivot_val;
            if factor.abs() <= RANK_EPS {
                continue;
            }
            for c in col..cols {
                a[row * cols + c] -= factor * a[pivot_row * cols + c];
            }
            a[row * cols + col] = 0.0;
        }
        pivot_row += 1;
        if pivot_row == rows {
            break;
        }
    }

    for free_col in 0..cols {
        if pivot_cols[free_col] {
            continue;
        }
        let mut basis = vec![0.0; cols];
        basis[free_col] = 1.0;
        for row in 0..rows {
            let mut pivot_col = None;
            for col in 0..cols {
                if pivot_cols[col] && a[row * cols + col].abs() > RANK_EPS {
                    pivot_col = Some(col);
                    break;
                }
            }
            let Some(pivot_col) = pivot_col else {
                continue;
            };
            basis[pivot_col] = -a[row * cols + free_col];
        }
        for &target in target_cols {
            if basis[target].abs() > RANK_EPS {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch_solver::residuals::{Equation, DEFAULT_WEIGHT};

    #[test]
    fn underconstrained_square_has_positive_dof() {
        let mut sys = System::new();
        let (x0, y0) = sys.add_point(0.0, 0.0, true);
        let (x1, y1) = sys.add_point(10.0, 0.0, false);
        let (_x2, _y2) = sys.add_point(10.0, 10.0, false);
        let (_x3, _y3) = sys.add_point(0.0, 10.0, false);
        sys.add_equation(Equation::Horizontal {
            y0,
            y1,
            weight: DEFAULT_WEIGHT,
        });
        sys.add_equation(Equation::LineLength {
            x0,
            y0,
            x1,
            y1,
            length: 10.0,
            weight: DEFAULT_WEIGHT,
        });
        assert!(dof_remaining(&sys) > 0);
    }

    #[test]
    fn fully_constrained_triangle_has_zero_dof() {
        let mut sys = System::new();
        let (x0, y0) = sys.add_point(0.0, 0.0, true);
        let (x1, y1) = sys.add_point(10.0, 0.0, true);
        let (x2, y2) = sys.add_point(5.0, 8.0, false);
        sys.add_equation(Equation::LineLength {
            x0,
            y0,
            x1,
            y1,
            length: 10.0,
            weight: DEFAULT_WEIGHT,
        });
        sys.add_equation(Equation::LineLength {
            x0,
            y0,
            x1: x2,
            y1: y2,
            length: 8.0,
            weight: DEFAULT_WEIGHT,
        });
        sys.add_equation(Equation::LineLength {
            x0: x1,
            y0: y1,
            x1: x2,
            y1: y2,
            length: 8.0,
            weight: DEFAULT_WEIGHT,
        });
        assert_eq!(dof_remaining(&sys), 0);
        assert!(!vars_can_move_together(&sys, &[x2, y2]));
    }

    #[test]
    fn conflicting_lengths_fail_to_converge() {
        use crate::sketch_solver::newton::{solve_lm, SolverConfig};

        let mut sys = System::new();
        let (x0, y0) = sys.add_point(0.0, 0.0, true);
        let (x1, y1) = sys.add_point(10.0, 0.0, false);
        sys.add_equation(Equation::LineLength {
            x0,
            y0,
            x1,
            y1,
            length: 10.0,
            weight: DEFAULT_WEIGHT,
        });
        sys.add_equation(Equation::LineLength {
            x0,
            y0,
            x1,
            y1,
            length: 12.0,
            weight: DEFAULT_WEIGHT,
        });
        let report = solve_lm(&mut sys, SolverConfig::default());
        assert!(!report.success);
        assert!(report.residual_norm > 1e-3);
    }

    #[test]
    fn fixed_var_cannot_move() {
        let mut sys = System::new();
        let (x0, y0) = sys.add_point(0.0, 0.0, true);
        let (x1, y1) = sys.add_point(10.0, 0.0, false);
        sys.add_equation(Equation::Horizontal {
            y0,
            y1,
            weight: DEFAULT_WEIGHT,
        });
        assert!(!vars_can_move_together(&sys, &[x0, y0]));
        assert!(vars_can_move_together(&sys, &[x1, y1]));
    }
}