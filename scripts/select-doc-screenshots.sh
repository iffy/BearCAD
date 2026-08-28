#!/usr/bin/env bash
# Select which documentation screenshot scenes to run.
#
# Prints scene basenames (no .lua) one per line. Used by gen-doc-screenshots.sh
# and CI to avoid regenerating every PNG on every push (#1297).
#
# Modes (BEARCAD_SCREENSHOT_MODE or --mode):
#   full        — every scene (master / nightly default)
#   missing     — scenes with no non-empty owned PNG in OUT_DIR
#   affected    — missing + scenes whose script or mapped deps changed
#   incremental — same as affected (PR / non-master pushes)
#
# Usage:
#   scripts/select-doc-screenshots.sh
#   BEARCAD_SCREENSHOT_MODE=affected scripts/select-doc-screenshots.sh
#   scripts/select-doc-screenshots.sh --mode full
#   scripts/select-doc-screenshots.sh --mode affected --base origin/master
#
# Env:
#   BEARCAD_SCREENSHOT_MODE   full|missing|affected|incremental (default: full)
#   BEARCAD_SCREENSHOT_BASE   git ref for affected diff (default: origin/master)
#   BEARCAD_SCREENSHOT_OUT    PNG dir (default: docs-site/static/img/screenshots)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SCRIPT_DIR="docs-site/screenshots"
OUT_DIR="${BEARCAD_SCREENSHOT_OUT:-docs-site/static/img/screenshots}"
MODE="${BEARCAD_SCREENSHOT_MODE:-full}"
BASE="${BEARCAD_SCREENSHOT_BASE:-origin/master}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode) MODE="$2"; shift 2 ;;
    --base) BASE="$2"; shift 2 ;;
    --out)  OUT_DIR="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,25p' "$0"
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

case "$MODE" in
  full|missing|affected|incremental) ;;
  *)
    echo "error: unknown mode '$MODE' (want full|missing|affected|incremental)" >&2
    exit 2
    ;;
esac
if [[ "$MODE" == "incremental" ]]; then
  MODE="affected"
fi

shopt -s nullglob
scripts=("$SCRIPT_DIR"/*.lua)
if [[ ${#scripts[@]} -eq 0 ]]; then
  echo "error: no screenshot scripts in $SCRIPT_DIR/" >&2
  exit 1
fi

scene_names=()
for script in "${scripts[@]}"; do
  scene="$(basename "$script" .lua)"
  [[ "$scene" == _* ]] && continue
  scene_names+=("$scene")
done

# owns <scene> <png-path> — longest scene-name match wins (see gen-doc-screenshots.sh).
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

# True when the scene already has at least one non-empty owned PNG.
has_png() {
  local name="$1" png
  for png in "$OUT_DIR/$name.png" "$OUT_DIR/$name"-*.png; do
    [[ -s "$png" ]] || continue
    owns "$name" "$png" && return 0
  done
  return 1
}

emit_all() {
  local s
  for s in ${scene_names[@]+"${scene_names[@]}"}; do
    printf '%s\n' "$s"
  done
}

if [[ "$MODE" == "full" ]]; then
  emit_all
  exit 0
fi

# --- missing -----------------------------------------------------------------
missing=()
for s in ${scene_names[@]+"${scene_names[@]}"}; do
  if ! has_png "$s"; then
    missing+=("$s")
  fi
done

if [[ "$MODE" == "missing" ]]; then
  if [[ ${#missing[@]} -eq 0 ]]; then
    exit 0
  fi
  printf '%s\n' "${missing[@]}"
  exit 0
fi

# --- affected (+ always include missing) -------------------------------------
# Paths that force the full suite: renderer, theme, harness, kernel/link surface.
FULL_FORCE_REGEX='^(src/gpu_viewport/|src/gpu_view_cube/|src/theme\.rs|src/icons\.rs|src/main\.rs|src/script\.rs|src/lua_script\.rs|src/web_lua\.rs|cpp/|scripts/gen-doc-screenshots\.sh|scripts/select-doc-screenshots\.sh|Cargo\.toml|Cargo\.lock)'

# Print path regexes for a scene (beyond the generic rules applied below).
deps_for_scene() {
  local scene="$1" stem="$1"
  stem="${stem#pane-}"
  stem="${stem%-sketch}"
  stem="${stem%-scene}"
  stem="${stem%-pairs}"
  stem="${stem%-kinds}"
  stem="${stem%-views}"
  stem="${stem%-pane}"

  printf '%s\n' "docs-site/screenshots/${scene}\\.lua"
  printf '%s\n' "docs-site/screenshots/assets/"

  case "$scene" in
    pane-settings)
      printf '%s\n' "src/settings\\.rs"
      ;;
    command-palette)
      printf '%s\n' "src/command_palette\\.rs" "src/menu_command\\.rs"
      ;;
    elements-pane)
      printf '%s\n' "src/hierarchy\\.rs" "src/selection\\.rs"
      ;;
    exploder)
      printf '%s\n' "src/element_picker\\.rs" "src/selection\\.rs" "src/touch_loupe\\.rs"
      ;;
    joint-kinds|pane-joint)
      printf '%s\n' "src/joints\\.rs" "src/joint_viewport\\.rs" "src/mate\\.rs"
      ;;
    snap-pairs|move|pane-move)
      printf '%s\n' "src/snapping\\.rs" "src/mate\\.rs" "src/selection\\.rs"
      ;;
    letter-b)
      printf '%s\n' "src/kernel/" "cpp/"
      ;;
    tracing|pane-tracing)
      printf '%s\n' "src/face\\.rs" "src/drawing\\.rs"
      ;;
    aligned-views|drawing|pane-drawing-*)
      printf '%s\n' "src/drawing\\.rs" "src/projection\\.rs"
      ;;
    dimension|pane-dimension|constraint|pane-constraint)
      printf '%s\n' "src/dimensions\\.rs" "src/constraints\\.rs" "src/geometric_constraints\\.rs" "src/sketch_solver/"
      ;;
    construction-plane|pane-construction-plane)
      printf '%s\n' "src/construction\\.rs"
      ;;
    *)
      if [[ -f "src/${stem}.rs" ]]; then
        printf '%s\n' "src/${stem}\\.rs"
      fi
      if [[ -d "src/${stem}" ]]; then
        printf '%s\n' "src/${stem}/"
      fi
      case "$stem" in
        rect|rectangle|circle|line|sketch)
          printf '%s\n' "src/primitives\\.rs" "src/polygon\\.rs" "src/model\\.rs"
          ;;
        fillet|chamfer)
          printf '%s\n' "src/model\\.rs" "src/opsigs\\.rs"
          ;;
        extrude|revolve|sweep|loft|slice|combine|offset|repeat|shape|text|shell)
          printf '%s\n' "src/extrude\\.rs" "src/model\\.rs" "src/opsigs\\.rs" "src/offset\\.rs" "src/text\\.rs"
          ;;
      esac
      ;;
  esac
}

# Collect changed files vs BASE. If the ref is missing, fall back to full.
changed_files=""
if git rev-parse --verify "$BASE" >/dev/null 2>&1; then
  changed_files="$(git diff --name-only "$BASE"...HEAD 2>/dev/null \
    || git diff --name-only "$BASE" HEAD 2>/dev/null \
    || true)"
else
  echo "warning: base ref '$BASE' not found; selecting full suite" >&2
  emit_all
  exit 0
fi

if [[ -z "${changed_files//[$'\n']/}" ]]; then
  if [[ ${#missing[@]} -eq 0 ]]; then
    exit 0
  fi
  printf '%s\n' "${missing[@]}"
  exit 0
fi

if printf '%s\n' "$changed_files" | grep -E -q "$FULL_FORCE_REGEX"; then
  echo "select: full-force path changed; selecting all scenes" >&2
  emit_all
  exit 0
fi

if ! printf '%s\n' "$changed_files" | grep -E -q '^(docs-site/screenshots/|src/|cpp/|scripts/(gen-doc-screenshots|select-doc-screenshots)\.sh)'; then
  echo "select: no visual-affecting paths in diff; only missing scenes" >&2
  if [[ ${#missing[@]} -eq 0 ]]; then
    exit 0
  fi
  printf '%s\n' "${missing[@]}"
  exit 0
fi

selected=()
# Seed with missing.
for s in ${missing[@]+"${missing[@]}"}; do
  selected+=("$s")
done

in_selected() {
  local want="$1" x
  for x in ${selected[@]+"${selected[@]}"}; do
    [[ "$x" == "$want" ]] && return 0
  done
  return 1
}

for s in ${scene_names[@]+"${scene_names[@]}"}; do
  in_selected "$s" && continue
  # Portable: process deps without process substitution / mapfile.
  deps="$(deps_for_scene "$s")"
  hit=0
  while IFS= read -r re; do
    [[ -z "$re" ]] && continue
    if printf '%s\n' "$changed_files" | grep -E -q "$re"; then
      hit=1
      break
    fi
  done <<EOF
$deps
EOF
  if [[ "$hit" -eq 1 ]]; then
    selected+=("$s")
  fi
done

if [[ ${#selected[@]} -eq 0 ]]; then
  exit 0
fi
printf '%s\n' "${selected[@]}"
