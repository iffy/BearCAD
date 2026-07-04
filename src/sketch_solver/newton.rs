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
            // 100 stalled real sketches mid-convergence (a sloppily drawn closed profile
            // being squared up by parallel/perpendicular/angle constraints needs the loop
            // to reflow through many small LM steps, and every rejected trial step also
            // burns an iteration). These systems are tiny (tens of variables), so a
            // deeper budget costs microseconds and only when actually needed.
            max_iterations: 600,
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

/// When lambda has been driven this high by consecutive rejected steps, the descent is
/// parked at a stationary point (J^T r ≈ 0) and further iterations are wasted.
const LAMBDA_STALL: f64 = 1e10;

/// Restart attempts after a stalled descent. Squaring up a sloppily drawn closed profile
/// (parallel/perpendicular/angle constraints landing on freehand geometry) routinely puts
/// the least-squares objective in a local minimum that is *not* a solution even though one
/// exists nearby; a deterministic jitter of the free variables and a re-descent escapes it.
const STALL_RESTARTS: u32 = 6;

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

    let mut iterations = 0u32;
    let mut norm = descend(system, &free, config, &mut iterations);
    if norm <= config.tolerance {
        return report(true, iterations, norm);
    }

    // Stalled short of a solution: retry from deterministically jittered starts, keeping
    // the best configuration seen. Genuinely conflicting sketches stay failed (each
    // descent parks quickly once lambda blows up), just with a better local minimum.
    let mut best_values = system.values.clone();
    let mut best_norm = norm;
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..STALL_RESTARTS {
        if iterations >= config.max_iterations {
            break;
        }
        system.values = best_values.clone();
        // Jitter each free variable by up to ±4x the residual norm: enough to hop out of
        // the basin without losing the overall shape the user drew.
        let scale = (best_norm * 4.0).max(config.tolerance * 10.0);
        for var in &free {
            rng = xorshift64(rng);
            let unit = (rng >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
            system.values[var.0] += (unit * 2.0 - 1.0) * scale;
        }
        norm = descend(system, &free, config, &mut iterations);
        if norm < best_norm {
            best_norm = norm;
            best_values = system.values.clone();
        }
        if best_norm <= config.tolerance {
            break;
        }
    }
    system.values = best_values;
    report(best_norm <= config.tolerance, iterations, best_norm)
}

fn report(success: bool, iterations: u32, norm: f64) -> SolveReport {
    SolveReport {
        success,
        iterations,
        residual_norm: norm,
        dof_remaining: 0,
        failed_constraints: Vec::new(),
    }
}

fn xorshift64(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// One LM descent from the system's current values. Returns the final inf-norm; the
/// shared iteration counter enforces `config.max_iterations` across restarts.
fn descend(system: &mut System, free: &[VarId], config: SolverConfig, iterations: &mut u32) -> f64 {
    let mut lambda = config.lm_lambda_init;
    let mut norm = system.residual_norm_inf();
    // Steps are accepted on the squared 2-norm — the objective LM actually descends.
    // Judging acceptance on the inf-norm (as this loop once did) deadlocks whenever the
    // best global step nudges the single worst residual up: every trial gets rejected,
    // lambda climbs, and the solve stalls short of an answer that exists. The inf-norm
    // is still what `success` measures, so "solved" continues to mean every individual
    // constraint is within tolerance.
    let mut sq_sum = residual_sq_sum(system);

    while *iterations < config.max_iterations && norm > config.tolerance && lambda < LAMBDA_STALL
    {
        let step = compute_lm_step(system, free, lambda);
        if step.is_none() {
            break;
        }
        let step = step.unwrap();

        let saved: Vec<f64> = system.values.clone();
        for (i, var) in free.iter().enumerate() {
            system.values[var.0] += step[i];
        }

        let new_sq_sum = residual_sq_sum(system);
        if new_sq_sum < sq_sum {
            sq_sum = new_sq_sum;
            norm = system.residual_norm_inf();
            lambda = (lambda * config.lm_lambda_down).max(1e-12);
        } else {
            system.values = saved;
            lambda *= config.lm_lambda_up;
        }
        *iterations += 1;
    }
    norm
}

fn residual_sq_sum(system: &System) -> f64 {
    system
        .equations
        .iter()
        .map(|eq| {
            let r = eq.residual(system);
            r * r
        })
        .sum()
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