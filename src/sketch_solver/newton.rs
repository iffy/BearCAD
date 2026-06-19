//! Dense Levenberg–Marquardt solver for sketch constraint systems.

use super::system::{System, VarId};

/// Numerical parameters for the LM solver.
#[derive(Clone, Copy, Debug)]
pub struct SolverConfig {
    pub max_iterations: u32,
    pub tolerance: f64,
    pub lm_lambda_init: f64,
    pub lm_lambda_up: f64,
    pub lm_lambda_down: f64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            tolerance: 1e-6,
            lm_lambda_init: 1e-3,
            lm_lambda_up: 10.0,
            lm_lambda_down: 0.3,
        }
    }
}

/// Outcome of a solve attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct SolveReport {
    pub success: bool,
    pub iterations: u32,
    pub residual_norm: f64,
    pub dof_remaining: i32,
    /// Document constraint indices with the largest residuals after a failed solve.
    pub failed_constraints: Vec<usize>,
}

pub fn solve_lm(system: &mut System, config: SolverConfig) -> SolveReport {
    let free = system.free_vars();
    if free.is_empty() {
        let norm = system.residual_norm_inf();
        return SolveReport {
            success: norm <= config.tolerance,
            iterations: 0,
            residual_norm: norm,
            dof_remaining: 0,
            failed_constraints: Vec::new(),
        };
    }

    let mut lambda = config.lm_lambda_init;
    let mut iterations = 0u32;
    let mut norm = system.residual_norm_inf();

    while iterations < config.max_iterations && norm > config.tolerance {
        let step = compute_lm_step(system, &free, lambda);
        if step.is_none() {
            break;
        }
        let step = step.unwrap();

        let saved: Vec<f64> = system.values.clone();
        for (i, var) in free.iter().enumerate() {
            system.values[var.0] += step[i];
        }

        let new_norm = system.residual_norm_inf();
        if new_norm < norm {
            norm = new_norm;
            lambda = (lambda * config.lm_lambda_down).max(1e-12);
        } else {
            system.values = saved;
            lambda *= config.lm_lambda_up;
        }
        iterations += 1;
    }

    SolveReport {
        success: norm <= config.tolerance,
        iterations,
        residual_norm: norm,
        dof_remaining: 0,
        failed_constraints: Vec::new(),
    }
}

fn compute_lm_step(system: &System, free: &[VarId], lambda: f64) -> Option<Vec<f64>> {
    let n_eq = system.equations.len();
    let n_free = free.len();
    if n_eq == 0 || n_free == 0 {
        return Some(vec![0.0; n_free]);
    }

    let mut jacobian = vec![0.0f64; n_eq * n_free];
    let mut residuals = vec![0.0f64; n_eq];
    let mut row_buf: Vec<(VarId, f64)> = Vec::new();

    let free_index: std::collections::HashMap<usize, usize> = free
        .iter()
        .enumerate()
        .map(|(i, var)| (var.0, i))
        .collect();

    for (row, equation) in system.equations.iter().enumerate() {
        residuals[row] = equation.residual(system);
        equation.jacobian_row(system, &mut row_buf);
        for (var, deriv) in &row_buf {
            if let Some(&col) = free_index.get(&var.0) {
                jacobian[row * n_free + col] = *deriv;
            }
        }
    }

    let jtj = build_jtj(&jacobian, n_eq, n_free);
    let mut jtr = build_jtr(&jacobian, &residuals, n_eq, n_free);

    let mut a = jtj;
    for i in 0..n_free {
        a[i * n_free + i] += lambda;
        a[i * n_free + i] = a[i * n_free + i].max(1e-12);
    }

    solve_symmetric_positive(&mut a, &mut jtr, n_free)?;
    let step: Vec<f64> = jtr.iter().map(|x| -x).collect();
    Some(step)
}

fn build_jtj(jacobian: &[f64], n_eq: usize, n_free: usize) -> Vec<f64> {
    let mut jtj = vec![0.0f64; n_free * n_free];
    for row in 0..n_eq {
        for i in 0..n_free {
            let ji = jacobian[row * n_free + i];
            for j in 0..=i {
                jtj[i * n_free + j] += ji * jacobian[row * n_free + j];
            }
        }
    }
    for i in 0..n_free {
        for j in 0..i {
            jtj[j * n_free + i] = jtj[i * n_free + j];
        }
    }
    jtj
}

fn build_jtr(jacobian: &[f64], residuals: &[f64], n_eq: usize, n_free: usize) -> Vec<f64> {
    let mut jtr = vec![0.0f64; n_free];
    for row in 0..n_eq {
        let r = residuals[row];
        for col in 0..n_free {
            jtr[col] += jacobian[row * n_free + col] * r;
        }
    }
    jtr
}

/// Solve `A x = b` in place; `b` becomes `x`. Returns None if singular.
fn solve_symmetric_positive(a: &mut [f64], b: &mut [f64], n: usize) -> Option<()> {
    // Cholesky decomposition for small dense systems.
    let mut l = vec![0.0f64; n * n];
    for i in 0..n {
        for j in 0..=i {
            let mut sum = a[i * n + j];
            for k in 0..j {
                sum -= l[i * n + k] * l[j * n + k];
            }
            if i == j {
                if sum <= 1e-15 {
                    return None;
                }
                l[i * n + j] = sum.sqrt();
            } else {
                l[i * n + j] = sum / l[j * n + j];
            }
        }
    }

    for i in 0..n {
        let mut sum = b[i];
        for k in 0..i {
            sum -= l[i * n + k] * b[k];
        }
        b[i] = sum / l[i * n + i];
    }
    for i in (0..n).rev() {
        let mut sum = b[i];
        for k in (i + 1)..n {
            sum -= l[k * n + i] * b[k];
        }
        b[i] = sum / l[i * n + i];
    }
    Some(())
}