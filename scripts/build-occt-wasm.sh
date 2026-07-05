#!/usr/bin/env bash
# Build the OCCT geometry kernel for the BROWSER: OCCT static libs compiled with
# Emscripten, then the BearCAD kernel shim (cpp/bearcad_kernel.cpp) linked into a
# standalone `kernel.js` + `kernel.wasm` ES module in web/kernel/.
#
# The web app itself is wasm32-unknown-unknown (eframe/wasm-bindgen), which can't link
# Emscripten C++ directly — so the kernel ships as a *second* wasm module and the app
# calls its 16-function C API through a small JS bridge (web/kernel-bridge.js,
# src/kernel/wasm.rs). Same shim source, same API, different module boundary.
#
# Usage:
#   scripts/build-occt-wasm.sh          # needs emcc (emscripten) + cmake on PATH
#
# Outputs:
#   third_party/OCCT/occt-install-wasm/   OCCT static libs (cached in CI)
#   web/kernel/kernel.js, kernel.wasm     the linked kernel module

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
occt_src="$repo_root/third_party/OCCT"
occt_build="$occt_src/occt-build-wasm"
occt_install="$occt_src/occt-install-wasm"

command -v emcc >/dev/null 2>&1 || { echo "error: emcc (emscripten) not found on PATH" >&2; exit 1; }

jobs="$( (getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4) )"

if [ ! -f "$occt_install/lib/libTKernel.a" ]; then
  # The OCCT *sources* (submodule) and cmake are only needed to build the static libs.
  # With a populated install prefix (e.g. restored from CI cache), the link step below
  # is self-sufficient — CI skips the submodule fetch entirely on a cache hit.
  if [ ! -f "$occt_src/CMakeLists.txt" ]; then
    echo "error: OCCT submodule missing at $occt_src" >&2
    echo "       run: git submodule update --init --depth 1 third_party/OCCT" >&2
    exit 1
  fi

  command -v cmake >/dev/null 2>&1 || { echo "error: cmake not found on PATH" >&2; exit 1; }
  # emcmake wraps cmake with the Emscripten toolchain file.
  command -v emcmake >/dev/null 2>&1 || { echo "error: emcmake not found on PATH" >&2; exit 1; }

  echo ">> Configuring OCCT for wasm (static, modeling + STEP, single-threaded) ..."
  emcmake cmake -S "$occt_src" -B "$occt_build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$occt_install" \
    -DCMAKE_CXX_FLAGS="-ffunction-sections -fdata-sections -fexceptions" \
    -DCMAKE_C_FLAGS="-ffunction-sections -fdata-sections -fexceptions" \
    -DBUILD_LIBRARY_TYPE=Static \
    -DBUILD_MODULE_FoundationClasses=ON \
    -DBUILD_MODULE_ModelingData=ON \
    -DBUILD_MODULE_ModelingAlgorithms=ON \
    -DBUILD_MODULE_Visualization=OFF \
    -DBUILD_MODULE_ApplicationFramework=OFF \
    -DBUILD_MODULE_DataExchange=ON \
    -DBUILD_MODULE_Draw=OFF \
    -DBUILD_MODULE_DETools=OFF \
    -DUSE_FREETYPE=OFF \
    -DUSE_TK=OFF \
    -DUSE_TCL=OFF \
    -DUSE_VTK=OFF \
    -DUSE_FREEIMAGE=OFF \
    -DUSE_RAPIDJSON=OFF \
    -DUSE_OPENGL=OFF \
    -DUSE_GLES2=OFF \
    -DUSE_PTHREADS=OFF \
    -DBUILD_DOC_Overview=OFF

  echo ">> Building OCCT (wasm) with $jobs jobs (this takes a while) ..."
  cmake --build "$occt_build" --config Release -j "$jobs"

  echo ">> Installing OCCT (wasm) into $occt_install ..."
  cmake --install "$occt_build" --config Release
else
  echo ">> OCCT wasm libs already present in $occt_install (delete to rebuild)"
fi

echo ">> Linking the BearCAD kernel module (kernel.js/kernel.wasm) ..."
mkdir -p "$repo_root/web/kernel"

# The 16 extern "C" entry points the app bridge calls, plus malloc/free for
# passing arrays through the module heap.
exports='["_bearcad_kernel_box_volume","_bearcad_kernel_occt_version","_bearcad_shape_prism","_bearcad_shape_loft","_bearcad_shape_revolve","_bearcad_shape_cylinder","_bearcad_shape_boolean","_bearcad_shape_fillet","_bearcad_shape_chamfer","_bearcad_shape_volume","_bearcad_shape_tessellate","_bearcad_tri_free","_bearcad_shape_free","_bearcad_face_boolean_loop","_bearcad_pts_free","_bearcad_shape_write_step","_bearcad_read_step","_malloc","_free"]'

emcc "$repo_root/cpp/bearcad_kernel.cpp" \
  -I"$occt_install/include/opencascade" \
  -L"$occt_install/lib" \
  -lTKDESTEP -lTKDE -lTKXSBase -lTKMesh -lTKFillet -lTKOffset -lTKBool -lTKBO \
  -lTKShHealing -lTKPrim -lTKTopAlgo -lTKGeomAlgo -lTKBRep -lTKGeomBase -lTKG3d \
  -lTKG2d -lTKMath -lTKernel \
  -O2 -fexceptions \
  -sMODULARIZE=1 -sEXPORT_ES6=1 -sEXPORT_NAME=BearcadKernel \
  -sALLOW_MEMORY_GROWTH=1 \
  -sENVIRONMENT=web \
  -sFILESYSTEM=1 -sFORCE_FILESYSTEM=1 \
  -sEXPORTED_FUNCTIONS="$exports" \
  -sEXPORTED_RUNTIME_METHODS='["ccall","cwrap","HEAPF64","HEAPU8","UTF8ToString","stringToNewUTF8","FS"]' \
  -o "$repo_root/web/kernel/kernel.js"

echo
echo "Built:"
ls -la "$repo_root/web/kernel/"
