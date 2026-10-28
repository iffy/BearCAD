# Eigen (vendored headers)

The `Eigen/` header tree and license files from <https://gitlab.com/libeigen/eigen> at
commit `3147391d946bb4b6c68edd901f2add6ac1f31f8c` — the commit SolveSpace pins as its
`extlib/eigen` submodule. Vendored so builds don't depend on gitlab.com, which often
refuses clones under load. Eigen is MPL-2.0 (`COPYING.MPL2`).

To update: check out the commit SolveSpace pins, copy its `Eigen/` and `COPYING.*` here,
and update the commit above.
