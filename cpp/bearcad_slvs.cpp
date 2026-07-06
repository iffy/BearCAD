// Flat-array C shim over SolveSpace's libslvs (third_party/solvespace), the sketch
// constraint solver backend. One ABI for both targets: natively it is compiled by
// build.rs and called directly over FFI (src/sketch_solver/slvs.rs); for the web it is
// linked into the emscripten kernel module (scripts/build-occt-wasm.sh) and reached
// through web/kernel-bridge.js, exactly like the OCCT shim.
//
// The arrays are row-major f64 (wasm-friendly: one HEAPF64 copy each way):
//   params:      [h, group, val]                                        x nparams
//   entities:    [h, group, type, wrkpl, p0, p1, p2, p3, normal, dist,
//                 q0, q1, q2, q3]                                        x nentities
//   constraints: [h, group, type, wrkpl, valA, ptA, ptB, eA, eB, eC, eD,
//                 other, other2]                                         x nconstraints
//   dragged:     [param handle]                                          x ndragged
//
// Outputs: the solved parameter values (same order as `params`), the failing
// constraint handles, and the remaining DOF. Returns the SLVS_RESULT_* code.

#include <slvs.h>

#include <cstring>
#include <vector>

extern "C" int bearcad_slvs_solve(const double *params, int nparams,
                                  const double *entities, int nentities,
                                  const double *constraints, int nconstraints,
                                  const double *dragged, int ndragged,
                                  double *out_vals,
                                  unsigned *out_failed, int max_faileds, int *out_nfaileds,
                                  int *out_dof) {
    std::vector<Slvs_Param> p(nparams);
    for (int i = 0; i < nparams; ++i) {
        const double *r = params + 3 * i;
        p[i].h = (Slvs_hParam)r[0];
        p[i].group = (Slvs_hGroup)r[1];
        p[i].val = r[2];
    }

    std::vector<Slvs_Entity> e(nentities);
    for (int i = 0; i < nentities; ++i) {
        const double *r = entities + 14 * i;
        std::memset(&e[i], 0, sizeof(Slvs_Entity));
        e[i].h = (Slvs_hEntity)r[0];
        e[i].group = (Slvs_hGroup)r[1];
        e[i].type = (int)r[2];
        e[i].wrkpl = (Slvs_hEntity)r[3];
        for (int k = 0; k < 4; ++k) {
            e[i].point[k] = (Slvs_hEntity)r[4 + k];
        }
        e[i].normal = (Slvs_hEntity)r[8];
        e[i].distance = (Slvs_hEntity)r[9];
        for (int k = 0; k < 4; ++k) {
            e[i].param[k] = (Slvs_hParam)r[10 + k];
        }
    }

    std::vector<Slvs_Constraint> c(nconstraints);
    for (int i = 0; i < nconstraints; ++i) {
        const double *r = constraints + 13 * i;
        std::memset(&c[i], 0, sizeof(Slvs_Constraint));
        c[i].h = (Slvs_hConstraint)r[0];
        c[i].group = (Slvs_hGroup)r[1];
        c[i].type = (int)r[2];
        c[i].wrkpl = (Slvs_hEntity)r[3];
        c[i].valA = r[4];
        c[i].ptA = (Slvs_hEntity)r[5];
        c[i].ptB = (Slvs_hEntity)r[6];
        c[i].entityA = (Slvs_hEntity)r[7];
        c[i].entityB = (Slvs_hEntity)r[8];
        c[i].entityC = (Slvs_hEntity)r[9];
        c[i].entityD = (Slvs_hEntity)r[10];
        c[i].other = (int)r[11];
        c[i].other2 = (int)r[12];
    }

    std::vector<Slvs_hParam> drag(ndragged);
    for (int i = 0; i < ndragged; ++i) {
        drag[i] = (Slvs_hParam)dragged[i];
    }

    std::vector<Slvs_hConstraint> failed(nconstraints > 0 ? nconstraints : 1);

    Slvs_System sys;
    std::memset(&sys, 0, sizeof(sys));
    sys.param = p.data();
    sys.params = nparams;
    sys.entity = e.data();
    sys.entities = nentities;
    sys.constraint = c.data();
    sys.constraints = nconstraints;
    sys.dragged = drag.data();
    sys.ndragged = ndragged;
    sys.calculateFaileds = 1;
    sys.failed = failed.data();
    sys.faileds = (int)failed.size();

    // Group 2 is the solve group by convention (group 1 holds the fixed workplane and
    // reference geometry) — see src/sketch_solver/slvs.rs.
    Slvs_Solve(&sys, 2);

    for (int i = 0; i < nparams; ++i) {
        out_vals[i] = p[i].val;
    }
    int nf = sys.faileds;
    if (nf > max_faileds) {
        nf = max_faileds;
    }
    for (int i = 0; i < nf; ++i) {
        out_failed[i] = failed[i];
    }
    *out_nfaileds = nf;
    *out_dof = sys.dof;
    return sys.result;
}
