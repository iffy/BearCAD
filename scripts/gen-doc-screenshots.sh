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
# Selection / sharding (#1297):
#   BEARCAD_SCREENSHOT_MODE=full|missing|affected|incremental
#   BEARCAD_SCREENSHOT_ONLY=scene1,scene2     # explicit allow-list (overrides mode)
#   BEARCAD_SCREENSHOT_SHARD=1/4              # run only this shard of the selection
#   BEARCAD_SCREENSHOT_SKIP_EXISTING=1        # skip scenes that already have a PNG
#   BEARCAD_SCREENSHOT_LIST_ONLY=1            # print selected scenes and exit
#   BEARCAD_SKIP_SWATCHES=1                   # skip style-swatch cargo test
#   BEARCAD_SCREENSHOT_BASE=origin/master     # git base for affected mode
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

# Deterministic framing: pin the window instead of letting it maximize. Without
# this the shot depends on the machine — a desktop maximizes to the whole (often
# retina) display, while the CI runner has no window manager and stays at the
# 960x640 default, so what CI deploys is framed nothing like what the author
# reviewed locally. Anything sized in points (the exploder's loupes, toolbars,
# labels) then covers a very different share of the viewport. A fixed logical
# size gives both the same composition; the PNG comes out at that size times the
# display scale factor (2x on retina), so a desktop just renders it sharper.
export BEARCAD_WINDOW="${BEARCAD_WINDOW:-1600x900}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SCRIPT_DIR="docs-site/screenshots"
OUT_DIR="${BEARCAD_SCREENSHOT_OUT:-docs-site/static/img/screenshots}"
# Per-screenshot wall-clock budget (seconds). The app also self-terminates via
# its own --timeout; this outer bound is a belt-and-suspenders guard in case the
# process fails to exit at all.
PER_SHOT_TIMEOUT="${BEARCAD_SHOT_TIMEOUT:-60}"
CARGO_FLAGS="${BEARCAD_CARGO_FLAGS:-}"
LIST_ONLY="${BEARCAD_SCREENSHOT_LIST_ONLY:-0}"
SKIP_EXISTING="${BEARCAD_SCREENSHOT_SKIP_EXISTING:-0}"
SKIP_SWATCHES="${BEARCAD_SKIP_SWATCHES:-0}"
SHARD="${BEARCAD_SCREENSHOT_SHARD:-}"
ONLY="${BEARCAD_SCREENSHOT_ONLY:-}"
MODE="${BEARCAD_SCREENSHOT_MODE:-full}"

# --- Locate a `timeout`-style command (optional; absent on stock macOS) --------
TIMEOUT_CMD=""
if command -v timeout >/dev/null 2>&1; then
  TIMEOUT_CMD="timeout"
elif command -v gtimeout >/dev/null 2>&1; then
  TIMEOUT_CMD="gtimeout"
fi

# --- Wrap the run in xvfb on Linux so it renders headlessly --------------------
# The virtual screen has to be roomier than the pinned window, otherwise the
# window is clipped to xvfb's 1280x1024 default and the framing drifts again.
WINDOW_SIZE="${BEARCAD_WINDOW%%@*}"   # BEARCAD_WINDOW may carry an @x,y position
SCREEN_W=$(( ${WINDOW_SIZE%%x*} + 128 ))
SCREEN_H=$(( ${WINDOW_SIZE##*x} + 128 ))
XVFB_PREFIX=()
case "$(uname -s)" in
  Linux)
    if command -v xvfb-run >/dev/null 2>&1; then
      XVFB_PREFIX=(xvfb-run -a -s "-screen 0 ${SCREEN_W}x${SCREEN_H}x24")
    else
      echo "warning: xvfb-run not found on Linux; rendering will likely fail." >&2
    fi
    ;;
esac

# --- Build the app (release) unless told to reuse an existing binary -----------
BIN="target/release/bearcad"
if [[ "$LIST_ONLY" == "1" ]]; then
  : # selection only — no binary needed
elif [[ "${BEARCAD_SKIP_BUILD:-0}" == "1" ]]; then
  echo "Skipping build (BEARCAD_SKIP_BUILD=1); using $BIN"
else
  echo "Building bearcad (release) ${CARGO_FLAGS:+with flags: $CARGO_FLAGS}..."
  # shellcheck disable=SC2086
  cargo build --release $CARGO_FLAGS
fi

if [[ "$LIST_ONLY" != "1" && ! -x "$BIN" ]]; then
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

# --- Which scene owns which PNG ------------------------------------------------
# A scene writes <name>.png, or <name>-<variant>.png when it takes several shots
# (a tool's Context pane in each of its modes). That makes the `<name>-*` glob
# ambiguous as soon as another scene's name starts with `<name>-`: chamfer.lua's
# glob also matches chamfer-sketch.lua's shot. Ownership is the **longest** scene
# name the file's stem matches, so the more specific scene keeps its own files —
# otherwise a scene wipes its neighbor's shots on the way in and, having already
# had its turn, the neighbour never regenerates them.
scene_names=()
for script in "${scripts[@]}"; do
  scene="$(basename "$script" .lua)"
  [[ "$scene" == _* ]] && continue
  scene_names+=("$scene")
done

# owns <scene> <png-path> — true when <scene> is that PNG's owner.
owns() {
  local scene="$1" stem best="" candidate
  stem="$(basename "$2" .png)"
  for candidate in ${scene_names[@]+"${scene_names[@]}"}; do
    if [[ "$stem" == "$candidate" || "$stem" == "$candidate"-* ]] \
      && (( ${#candidate} > ${#best} )); then
      best="$candidate"
    fi
  done
  [[ "$best" == "$scene" ]]
}

has_png() {
  local name="$1" png
  for png in "$OUT_DIR/$name.png" "$OUT_DIR/$name"-*.png; do
    [[ -s "$png" ]] || continue
    owns "$name" "$png" && return 0
  done
  return 1
}

# --- Select scenes -------------------------------------------------------------
selected=()

if [[ -n "$ONLY" ]]; then
  # Explicit allow-list: comma or whitespace separated basenames (with or without .lua).
  ONLY="${ONLY//,/ }"
  # shellcheck disable=SC2206
  only_list=($ONLY)
  for raw in ${only_list[@]+"${only_list[@]}"}; do
    name="${raw%.lua}"
    name="$(basename "$name")"
    [[ -z "$name" ]] && continue
    found=0
    for s in ${scene_names[@]+"${scene_names[@]}"}; do
      if [[ "$s" == "$name" ]]; then
        found=1
        break
      fi
    done
    if [[ "$found" -eq 0 ]]; then
      echo "error: unknown scene in BEARCAD_SCREENSHOT_ONLY: $name" >&2
      exit 1
    fi
    selected+=("$name")
  done
else
  # Delegate to the selector (full / missing / affected).
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    selected+=("$line")
  done <<EOF
$(
  BEARCAD_SCREENSHOT_MODE="$MODE" \
  BEARCAD_SCREENSHOT_OUT="$OUT_DIR" \
  BEARCAD_SCREENSHOT_BASE="${BEARCAD_SCREENSHOT_BASE:-origin/master}" \
  "$ROOT/scripts/select-doc-screenshots.sh"
)
EOF
fi

# Optional: drop scenes that already have a PNG (used after restoring goldens).
if [[ "$SKIP_EXISTING" == "1" ]]; then
  filtered=()
  for name in ${selected[@]+"${selected[@]}"}; do
    if has_png "$name"; then
      echo "skip (exists): $name"
    else
      filtered+=("$name")
    fi
  done
  selected=()
  for name in ${filtered[@]+"${filtered[@]}"}; do
    selected+=("$name")
  done
fi

# Optional: shard the selection across parallel CI jobs (1/4 … 4/4).
if [[ -n "$SHARD" ]]; then
  case "$SHARD" in
    */*)
      shard_i="${SHARD%%/*}"
      shard_n="${SHARD##*/}"
      ;;
    *)
      echo "error: BEARCAD_SCREENSHOT_SHARD must look like N/M (got '$SHARD')" >&2
      exit 2
      ;;
  esac
  if ! [[ "$shard_i" =~ ^[0-9]+$ && "$shard_n" =~ ^[0-9]+$ ]]; then
    echo "error: BEARCAD_SCREENSHOT_SHARD must look like N/M (got '$SHARD')" >&2
    exit 2
  fi
  if (( shard_n < 1 || shard_i < 1 || shard_i > shard_n )); then
    echo "error: invalid shard '$SHARD' (need 1 <= N <= M)" >&2
    exit 2
  fi
  # Stable order, then take index ≡ (shard_i-1) (mod shard_n).
  sorted="$(printf '%s\n' ${selected[@]+"${selected[@]}"} | LC_ALL=C sort)"
  selected=()
  idx=0
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    if (( idx % shard_n == shard_i - 1 )); then
      selected+=("$name")
    fi
    idx=$((idx + 1))
  done <<EOF
$sorted
EOF
  echo "Shard $SHARD: ${#selected[@]} scene(s)" >&2
fi

if [[ "$LIST_ONLY" == "1" ]]; then
  printf '%s\n' ${selected[@]+"${selected[@]}"}
  exit 0
fi

if [[ ${#selected[@]} -eq 0 ]]; then
  echo "No screenshot scenes selected (mode=$MODE); nothing to generate."
fi

# --- Style swatches (#160) ------------------------------------------------------
# Drawn directly from the renderer's color constants into PNGs — no GPU or display
# needed, so this works everywhere the tests build. Only shard 1 (or a non-sharded
# run) should do this in CI.
if [[ "$SKIP_SWATCHES" != "1" ]]; then
  run_swatches=1
  if [[ -n "$SHARD" ]]; then
    shard_i="${SHARD%%/*}"
    if [[ "$shard_i" != "1" ]]; then
      run_swatches=0
    fi
  fi
  if [[ "$run_swatches" == "1" ]]; then
    echo "==> style swatches -> $OUT_DIR/styles/"
    # shellcheck disable=SC2086
    cargo test --release $CARGO_FLAGS generate_style_swatches -- --ignored
  else
    echo "Skipping style swatches (shard $SHARD; only shard 1 runs them)"
  fi
fi

if [[ ${#selected[@]} -eq 0 ]]; then
  echo "Screenshots generated: 0 ok, 0 failed (selection empty)."
  exit 0
fi

# --- Run each selected script and check its PNG --------------------------------
failed=()
succeeded=()
for name in "${selected[@]}"; do
  script="$SCRIPT_DIR/$name.lua"
  if [[ ! -f "$script" ]]; then
    echo "error: missing script $script" >&2
    failed+=("$name")
    continue
  fi

  # A script writes either <name>.png or, when one scene yields several shots (a
  # tool's Context pane in each of its modes), <name>-<variant>.png.
  out_png="$OUT_DIR/$name.png"
  for png in "$out_png" "$OUT_DIR/$name"-*.png; do
    owns "$name" "$png" && rm -f "$png"
  done
  echo "==> $script -> $OUT_DIR/$name*.png"

  # Give the app a self-timeout a little under the outer budget so it exits on
  # its own where possible (cleaner than an external kill).
  app_timeout=$(( PER_SHOT_TIMEOUT > 10 ? PER_SHOT_TIMEOUT - 5 : PER_SHOT_TIMEOUT ))

  run=("${XVFB_PREFIX[@]}" "$BIN" --script "$script" --exit --timeout "$app_timeout")
  if [[ -n "$TIMEOUT_CMD" ]]; then
    BEARCAD_SCREENSHOT_OUT="$OUT_DIR" "$TIMEOUT_CMD" "$PER_SHOT_TIMEOUT" "${run[@]}" || true
  else
    BEARCAD_SCREENSHOT_OUT="$OUT_DIR" "${run[@]}" || true
  fi

  produced=()
  for png in "$out_png" "$OUT_DIR/$name"-*.png; do
    [[ -s "$png" ]] || continue
    owns "$name" "$png" || continue
    produced+=("$png")
  done
  if [[ ${#produced[@]} -gt 0 ]]; then
    for png in "${produced[@]}"; do
      echo "    ok $(basename "$png") ($(wc -c <"$png" | tr -d ' ') bytes)"
    done
    succeeded+=("$name")
  else
    echo "    FAILED: no non-empty PNG produced" >&2
    failed+=("$name")
  fi
done

# --- Report --------------------------------------------------------------------
echo
echo "Screenshots generated: ${#succeeded[@]} ok, ${#failed[@]} failed (mode=$MODE${SHARD:+ shard=$SHARD})."
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

echo "Selected screenshots written to $OUT_DIR/"
