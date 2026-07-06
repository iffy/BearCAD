// Bridge between the BearCAD app module (wasm32-unknown-unknown, wasm-bindgen) and the
// OCCT kernel module (C++, compiled separately with Emscripten — see
// scripts/build-occt-wasm.sh). The hosting page loads the kernel first and stores its
// instance on `globalThis.bearcadKernel`; every function here degrades gracefully (null /
// 0 handle) when the kernel failed to load, which the Rust side reports as "kernel not
// available" — the same lean-build fallback story as native.
//
// Handles are the kernel module's C heap pointers, passed around as plain numbers.
// Arrays are copied between the app's memory (wasm-bindgen hands us typed-array views)
// and the kernel module's heap.

function M() {
  return globalThis.bearcadKernel || null;
}

// Read a NUL-terminated C string from the kernel heap. `slice` (not `subarray`) matters:
// the kernel's memory is growable, and TextDecoder refuses views over resizable buffers —
// emscripten's own UTF8ToString hits exactly that in Chrome.
function cString(m, ptr) {
  if (!ptr) return "";
  const heap = m.HEAPU8;
  let end = ptr;
  while (heap[end]) end++;
  return new TextDecoder().decode(heap.slice(ptr, end));
}

function copyF64In(m, arr) {
  const ptr = m._malloc(arr.length * 8);
  m.HEAPF64.set(arr, ptr / 8);
  return ptr;
}

export function kernel_available() {
  return !!M();
}

export function kernel_box_volume(dx, dy, dz) {
  const m = M();
  return m ? m._bearcad_kernel_box_volume(dx, dy, dz) : -1.0;
}

export function kernel_occt_version() {
  const m = M();
  if (!m) return "";
  return cString(m, m._bearcad_kernel_occt_version());
}

export function kernel_prism(xyz, dx, dy, dz) {
  const m = M();
  if (!m) return 0;
  const ptr = copyF64In(m, xyz);
  const h = m._bearcad_shape_prism(ptr, xyz.length / 3, dx, dy, dz);
  m._free(ptr);
  return h;
}

export function kernel_cylinder(cx, cy, cz, ax, ay, az, radius, height) {
  const m = M();
  if (!m) return 0;
  return m._bearcad_shape_cylinder(cx, cy, cz, ax, ay, az, radius, height);
}

export function kernel_loft(bottom, top) {
  const m = M();
  if (!m) return 0;
  const bp = copyF64In(m, bottom);
  const tp = copyF64In(m, top);
  const h = m._bearcad_shape_loft(bp, tp, bottom.length / 3);
  m._free(bp);
  m._free(tp);
  return h;
}

export function kernel_revolve(xyz, ox, oy, oz, ax, ay, az, angleRad, symmetric) {
  const m = M();
  if (!m) return 0;
  const ptr = copyF64In(m, xyz);
  const h = m._bearcad_shape_revolve(ptr, xyz.length / 3, ox, oy, oz, ax, ay, az, angleRad,
                                     symmetric ? 1 : 0);
  m._free(ptr);
  return h;
}

export function kernel_boolean(a, b, op) {
  const m = M();
  if (!m) return 0;
  return m._bearcad_shape_boolean(a, b, op);
}

export function kernel_fillet(h, edges, radii) {
  const m = M();
  if (!m) return 0;
  const ep = copyF64In(m, edges);
  const rp = copyF64In(m, radii);
  const out = m._bearcad_shape_fillet(h, ep, rp, radii.length);
  m._free(ep);
  m._free(rp);
  return out;
}

export function kernel_chamfer(h, edges, dists) {
  const m = M();
  if (!m) return 0;
  const ep = copyF64In(m, edges);
  const dp = copyF64In(m, dists);
  const out = m._bearcad_shape_chamfer(h, ep, dp, dists.length);
  m._free(ep);
  m._free(dp);
  return out;
}

export function kernel_volume(h) {
  const m = M();
  return m ? m._bearcad_shape_volume(h) : -1.0;
}

export function kernel_tessellate(h, deflection) {
  const m = M();
  if (!m) return new Float64Array(0);
  // _bearcad_shape_tessellate(shape, deflection, out_count*) -> double* (9 per triangle)
  const countPtr = m._malloc(4);
  const triPtr = m._bearcad_shape_tessellate(h, deflection, countPtr);
  const count = m.HEAPU32 ? m.HEAPU32[countPtr / 4] : new Uint32Array(m.HEAPU8.buffer)[countPtr / 4];
  m._free(countPtr);
  if (!triPtr || !count) return new Float64Array(0);
  const doubles = m.HEAPF64.slice(triPtr / 8, triPtr / 8 + count * 9);
  m._bearcad_tri_free(triPtr);
  return doubles;
}

// Split a shape into its individual solids; returns a Float64Array of new shape
// handles (empty when none / kernel missing).
export function kernel_split_solids(h) {
  const m = M();
  if (!m) return new Float64Array(0);
  const countPtr = m._malloc(4);
  const arrPtr = m._bearcad_shape_split_solids(h, countPtr);
  const count = new Uint32Array(m.HEAPU8.buffer)[countPtr / 4];
  m._free(countPtr);
  if (!arrPtr || !count) return new Float64Array(0);
  const handles = new Uint32Array(m.HEAPU8.buffer).slice(arrPtr / 4, arrPtr / 4 + count);
  m._bearcad_handles_free(arrPtr);
  return Float64Array.from(handles);
}

export function kernel_shape_free(h) {
  const m = M();
  if (m && h) m._bearcad_shape_free(h);
}

export function kernel_face_boolean_loop(a, b, op) {
  const m = M();
  if (!m) return null;
  const ap = copyF64In(m, a);
  const bp = copyF64In(m, b);
  const countPtr = m._malloc(4);
  const outPtr = m._bearcad_face_boolean_loop(ap, a.length / 2, bp, b.length / 2, op, countPtr);
  const count = new Uint32Array(m.HEAPU8.buffer)[countPtr / 4];
  m._free(ap);
  m._free(bp);
  m._free(countPtr);
  if (!outPtr || !count) return null;
  const doubles = m.HEAPF64.slice(outPtr / 8, outPtr / 8 + count * 2);
  m._bearcad_pts_free(outPtr);
  return doubles;
}

// SolveSpace constraint solve (libslvs, linked into this kernel module). Flat f64
// arrays in (see cpp/bearcad_slvs.cpp for the row layouts); returns a Float64Array
// packed as [result, dof, nfaileds, ...failedHandles, ...paramVals], or null when the
// kernel module isn't loaded.
export function kernel_slvs_solve(params, entities, constraints, dragged) {
  const m = M();
  if (!m) return null;
  const nparams = params.length / 3;
  const nconstraints = constraints.length / 13;
  const pp = copyF64In(m, params);
  const ep = copyF64In(m, entities);
  const cp = copyF64In(m, constraints);
  const dp = copyF64In(m, dragged);
  const valsPtr = m._malloc(nparams * 8);
  const failedPtr = m._malloc(Math.max(nconstraints, 1) * 4);
  const nfailedsPtr = m._malloc(4);
  const dofPtr = m._malloc(4);
  const result = m._bearcad_slvs_solve(
    pp, nparams, ep, entities.length / 14, cp, nconstraints, dp, dragged.length,
    valsPtr, failedPtr, Math.max(nconstraints, 1), nfailedsPtr, dofPtr);
  const ints = new Int32Array(m.HEAPU8.buffer);
  const nfaileds = ints[nfailedsPtr / 4];
  const dof = ints[dofPtr / 4];
  const failed = new Uint32Array(m.HEAPU8.buffer).slice(failedPtr / 4, failedPtr / 4 + nfaileds);
  const vals = m.HEAPF64.slice(valsPtr / 8, valsPtr / 8 + nparams);
  m._free(pp); m._free(ep); m._free(cp); m._free(dp);
  m._free(valsPtr); m._free(failedPtr); m._free(nfailedsPtr); m._free(dofPtr);
  const out = new Float64Array(3 + nfaileds + nparams);
  out[0] = result;
  out[1] = dof;
  out[2] = nfaileds;
  out.set(Float64Array.from(failed), 3);
  out.set(vals, 3 + nfaileds);
  return out;
}

export function kernel_write_step(h) {
  const m = M();
  if (!m) return null;
  const path = "/bearcad_out.step";
  const pathPtr = m.stringToNewUTF8(path);
  const ret = m._bearcad_shape_write_step(h, pathPtr);
  m._free(pathPtr);
  if (ret !== 0) return null;
  try {
    const bytes = m.FS.readFile(path);
    m.FS.unlink(path);
    return bytes;
  } catch (_) {
    return null;
  }
}

export function kernel_read_step(bytes) {
  const m = M();
  if (!m) return 0;
  const path = "/bearcad_in.step";
  try {
    m.FS.writeFile(path, bytes);
  } catch (_) {
    return 0;
  }
  const pathPtr = m.stringToNewUTF8(path);
  const h = m._bearcad_read_step(pathPtr);
  m._free(pathPtr);
  try {
    m.FS.unlink(path);
  } catch (_) {}
  return h;
}
