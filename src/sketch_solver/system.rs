//! Variable registry and equation list for the sketch constraint solver.

use super::residuals::Equation;

/// One scalar degree of freedom in the solver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VarId(pub usize);

/// Mutable constraint system: scalar variables plus residual equations.
#[derive(Clone, Debug)]
pub struct System {
    pub values: Vec<f64>,
    pub fixed: Vec<bool>,
    pub equations: Vec<Equation>,
}

impl System {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            fixed: Vec::new(),
            equations: Vec::new(),
        }
    }

    pub fn add_var(&mut self, initial: f64, fixed: bool) -> VarId {
        let id = VarId(self.values.len());
        self.values.push(initial);
        self.fixed.push(fixed);
        id
    }

    pub fn add_point(&mut self, u: f64, v: f64, fixed: bool) -> (VarId, VarId) {
        let u_id = self.add_var(u, fixed);
        let v_id = self.add_var(v, fixed);
        (u_id, v_id)
    }

    #[allow(dead_code)]
    pub fn set_fixed(&mut self, var: VarId, fixed: bool) {
        self.fixed[var.0] = fixed;
    }

    #[allow(dead_code)]
    pub fn set_value(&mut self, var: VarId, value: f64) {
        self.values[var.0] = value;
    }

    pub fn value(&self, var: VarId) -> f64 {
        self.values[var.0]
    }

    pub fn add_equation(&mut self, equation: Equation) {
        self.equations.push(equation);
    }

    pub fn free_vars(&self) -> Vec<VarId> {
        self.fixed
            .iter()
            .enumerate()
            .filter_map(|(i, fixed)| (!fixed).then_some(VarId(i)))
            .collect()
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}