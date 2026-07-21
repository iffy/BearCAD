#!/usr/bin/env bash
# Generate the documentation screenshots.
#
# For each Lua script in docs-site/screenshots/*.lua this builds a small,
# deterministic scene and captures a PNG into docs-site/static/img/screenshots/
# (Docusaurus serves that as /img/screenshots/<name>.png). The scripts read
# their output directory from the BEARCAD_SCREENSHOT_OUT environment variable.
#
# Usage:
#   scripts/gen-doc-screenshots.sh                  # release build (default)
#   BEARCAD_SKIP_BUILD=1 scripts/gen-doc-screenshots.sh   # reuse existing binary
#
# Rendering requirements: capturing a screenshot needs a real rendered GPU
# frame. This works on a normal desktop (a machine with a working display/GPU)
# and on CI Linux runners that provide a software Vulkan driver under xvfb
# (mesa-vulkan-drivers + xvfb, as the CI smoke test uses). In a headless
# environment without any of that the capture never resolves and the per-script
# timeout force-exits with no PNG; this script then reports that script as
# failed and exits non-zero.
set -uo pipefail

# Deterministic captures: never show the update badge (#427) in doc screenshots.
export BEARCAD_NO_UPDATE_CHECK=1

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SCRIPT_DIR="docs-site/screenshots"
OUT_DIR="docs-site/static/img/screenshots"
# Per-screenshot wall-clock budget (seconds). The app also self-terminates via
# its own --timeout; this outer bound is a belt-and-suspenders guard in case the
# process fails to exit at all.
PER_SHOT_TIMEOUT="${BEARCAD_SHOT_TIMEOUT:-60}"
CARGO_FLAGS="${BEARCAD_CARGO_FLAGS:-}"

# --- Locate a `timeout`-style command (optional; absent on stock macOS) --------
TIMEOUT_CMD=""
if command -v timeout >/dev/null 2>&1; then
  TIMEOUT_CMD="timeout"
elif command -v gtimeout >/dev/null 2>&1; then
  TIMEOUT_CMD="gtimeout"
fi

# --- Wrap the run in xvfb on Linux so it renders headlessly --------------------
XVFB_PREFIX=()
case "$(uname -s)" in
  Linux)
    if command -v xvfb-run >/dev/null 2>&1; then
      XVFB_PREFIX=(xvfb-run -a)
    else
      echo "warning: xvfb-run not found on Linux; rendering will likely fail." >&2
    fi
    ;;
esac

# --- Build the app (release) unless told to reuse an existing binary -----------
BIN="target/release/bearcad"
if [[ "${BEARCAD_SKIP_BUILD:-0}" == "1" ]]; then
  echo "Skipping build (BEARCAD_SKIP_BUILD=1); using $BIN"
else
  echo "Building bearcad (release) ${CARGO_FLAGS:+with flags: $CARGO_FLAGS}..."
  # shellcheck disable=SC2086
  cargo build --release $CARGO_FLAGS
fi

if [[ ! -x "$BIN" ]]; then
  echo "error: $BIN not found or not executable." >&2
  exit 1
fi

# --- Gather the scripts --------------------------------------------------------
shopt -s nullglob
scripts=("$SCRIPT_DIR"/*.lua)
if [[ ${#scripts[@]} -eq 0 ]]; then
  echo "error: no screenshot scripts found in $SCRIPT_DIR/" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

# --- Style swatches (#160) ------------------------------------------------------
# Drawn directly from the renderer's color constants into PNGs — no GPU or display
# needed, so this works everywhere the tests build.
echo "==> style swatches -> $OUT_DIR/styles/"
cargo test --release $CARGO_FLAGS generate_style_swatches -- --ignored

# --- Run each script and check its PNG -----------------------------------------
failed=()
succeeded=()
for script in "${scripts[@]}"; do
  name="$(basename "$script" .lua)"
  # Skip helper/partial files by convention (names starting with '_').
  [[ "$name" == _* ]] && continue

  out_png="$OUT_DIR/$name.png"
  rm -f "$out_png"
  echo "==> $script -> $out_png"

  # Give the app a self-timeout a little under the outer budget so it exits on
  # its own where possible (cleaner than an external kill).
  app_timeout=$(( PER_SHOT_TIMEOUT > 10 ? PER_SHOT_TIMEOUT - 5 : PER_SHOT_TIMEOUT ))

  run=("${XVFB_PREFIX[@]}" "$BIN" --script "$script" --exit --timeout "$app_timeout")
  if [[ -n "$TIMEOUT_CMD" ]]; then
    BEARCAD_SCREENSHOT_OUT="$OUT_DIR" "$TIMEOUT_CMD" "$PER_SHOT_TIMEOUT" "${run[@]}" || true
  else
    BEARCAD_SCREENSHOT_OUT="$OUT_DIR" "${run[@]}" || true
  fi

  if [[ -s "$out_png" ]]; then
    echo "    ok ($(wc -c <"$out_png" | tr -d ' ') bytes)"
    succeeded+=("$name")
  else
    echo "    FAILED: no non-empty PNG produced" >&2
    failed+=("$name")
  fi
done

# --- Report --------------------------------------------------------------------
echo
echo "Screenshots generated: ${#succeeded[@]} ok, ${#failed[@]} failed."
if [[ ${#succeeded[@]} -gt 0 ]]; then
  echo "  ok:     ${succeeded[*]}"
fi
if [[ ${#failed[@]} -gt 0 ]]; then
  echo "  failed: ${failed[*]}" >&2
  echo "One or more screenshots were not produced (needs a render-capable" >&2
  echo "environment: a real display/GPU, or CI Linux with xvfb + a software" >&2
  echo "Vulkan driver)." >&2
  exit 1
fi

echo "All screenshots written to $OUT_DIR/"
